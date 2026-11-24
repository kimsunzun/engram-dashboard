//! Step-9b file operations — inherent [`LibraryService`] methods answering crate types: delete (into the item's own mount's
//! `.trash`), restore (back out of it), and move and rename — on one volume a no-replace rename, across volumes a copy
//! whose original then goes into its trash —, and copy (a staged copy placed in the hold, no reference fix). Each runs
//! plan → execute → fix references (ADR-0335) under one `Change` hold (ADR-0409 M1).
//!
//! ## Rules (invariants)
//!
//! - **Per item** (ADR-0409): every given path answers on its own ([`Outcome`]) — done, skipped ([`Skip`]: a path given
//!   with a folder above it — only the folder counts —, a move into the folder it is in, a taken name with `on_conflict`
//!   skip), a conflict, or failed with a code — and what finished stays finished. Refused
//!   per item before the hold: a spelling that is no library path or holds a dot or temp name (a path under `.trash`
//!   included) `item-path-invalid`, a mount's top `virtual-root-read-only`, the built-in mount `mount-read-only`, a mount
//!   that cannot be read `library-unavailable` (walked again first — ADR-0400 decision 6). Inside the hold, before any
//!   disk step: the mount judged again from the recorded state, the link admission (ADR-0365 — `path-through-link`), and
//!   a no-follow stat (absent `not-found`).
//! - **The wait before the ticket** (ADR-0409 decision 14): delete, restore and copy always may touch `.trash`, a move
//!   with `overwrite` or with an item crossing volumes; those run through [`LibraryService::change_held_at`] with the
//!   waiting count — raised just before the ticket (after a move's probes and a move's or a copy's copies), lowered after
//!   the hold or on any earlier exit — and take the trash lock once (a delete at its first item, a restore before its
//!   per-mount loop, a move or a copy at its first displaced entry, original or staging), holding it to the end of their
//!   trash moves only (it is released before any preset file or agent is touched).
//! - **Delete** (ADR-0409 decisions 5–8): the plan names who refers to what goes — the preset files listing a deleted path
//!   (a folder: anything inside it; a lone `SKILL.md` — a file the catalog holds as a skill's, never a folder of that
//!   name: its skill folder or itself — ADR-0339) that are not deleted themselves, and the agents attaching a deleted path
//!   (never for a `SKILL.md`); its digest pins those and the items. Referenced and no `yes` → one
//!   [`DELETE_UNCONFIRMED`] for the call with the references and the two forms; `yes` keeps them (they read as not
//!   found until restored); `yes` + `detach_refs` must carry the plan's digest; `detach_refs` without `yes` is no `yes`
//!   (ADR-0339 M108 — every delete item is of a detachable shape, so a delete never refuses `detach_refs`). The same
//!   judgement runs again inside the hold before any disk step: a plan that changed answers the conflict again with the
//!   fresh plan, nothing written (ADR-0405 decision 1 · ADR-0409 decision 2). In the hold, per mount: `.trash` checked and
//!   made without following once (unusable → every item there `trash-unusable`), one bundle id per operation (a taken id
//!   in a later mount draws a new one there), each item moved by the store's no-replace trash step into
//!   `.trash/<bundle>/<its path in the mount, as the catalog spells it when that is the same entry>`, each mount's record
//!   published after the trash moves (a failed publication leaves the moves standing — the bundle reads by its files), a
//!   bundle that received no root pruned. Then the
//!   catalog range leaves and the parent folder is re-classified from disk ([`LibraryService::reclassify`] — a folder
//!   that lost its `SKILL.md` is no skill at once). With `detach_refs`, kept to the confirmed plan (the hold's
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
//!   follows the delete's `yes` / `detach_refs` / digest rule with [`SKILL_UNCONFIRMED`], judged at the call and again
//!   in the hold; so does a skill folder a merge would leave without its `SKILL.md` (the file moved, an entry left
//!   behind — planned from no-follow listings and stats at the call and again in the hold, only for an item holding a
//!   skill, and detached only when the folder is no skill after the disk step and did not merge away whole); a merge
//!   moves a skill's `SKILL.md` last and keeps it (left `skill-unconfirmed`) when an entry stayed behind that the
//!   hold's plan did not name, unless it would stay anyway (it then answers its own reason); `detach_refs` on a call
//!   with none of these is [`DETACH_REFS_INVALID`] (ADR-0339 M108 · M110). In the hold, per item: both mounts judged
//!   from the recorded state, the item and the destination admitted (ADR-0365), the item stat'ed without following, the
//!   destination folder a real folder (never made — ADR-0335 decision 7), then one no-replace rename — an item whose
//!   source an earlier item of the call displaced or moved another entry onto fails `not-found`. A taken name without
//!   `on_conflict` is the item's conflict `name-exists` (a rename never has one); same-name folders merge entry by entry
//!   by the same rules and a source folder emptied so is removed empty-only (one left holding anything stays; one whose
//!   removal fails is named in `left`); `Skip` leaves the entry; `KeepBoth` takes `name (2)`, `name (3)` …; `Overwrite`
//!   moves the entry there into its mount's `.trash` (bundle, record — a path displaced twice goes into a second
//!   bundle; a file onto a folder or the reverse [`KIND_MISMATCH`]) and renames the item onto the name, renaming the
//!   displaced entry back when that fails. The catalog follows in the same hold: the displaced ranges leave, each moved
//!   range is re-keyed by the path-key prefix mapper, what the catalog still holds in a merge's emptied source folder
//!   leaves, the moved files whose kind may have changed and the `SKILL.md` of a skill at a moved folder's new path are
//!   re-read (through no link — ADR-0365), and every folder whose direct children changed is re-classified (a skill
//!   gained or lost at once). Then the references, without a plan confirmation (ADR-0409 decision 4): every preset file
//!   of every readable mount that names a moved path is rewritten whole with the old → new mapping, its pairs applied
//!   in move order (a touched list keeps the first place of each path — ADR-0339; a merge's emptied source folder maps
//!   only itself, and only when its `SKILL.md` became the destination's), with `detach_refs` the confirmed plan's files
//!   also drop the lines naming the gone skill (no agent step); then the change number is raised and the agents are
//!   re-pointed once through `ProfileEditPort::rewrite_preset_paths` over the current roster. The answer carries the
//!   mapping (each row marked whole-range or exact, to apply in order), the files written and skipped and the git
//!   working trees their folders lie in (a folder behind a link: none).
//! - **Move across volumes** (ADR-0409 decisions 5 and 12–14, M13, M14 · ADR-0400 decision 25): before the plan, a
//!   volume probe per (item's folder, destination folder) pair — none for a rename in its own folder — tells which
//!   items cross (an empty folder made under a base temp name beside the item, renamed without replacing into the
//!   destination folder, removed empty-only; `CrossesDevices` = another volume, any other failure fails the item). For
//!   such an item both mounts' `.trash` must be usable (`trash-unusable`), and a taken destination name is answered
//!   before any copy — no `on_conflict` `name-exists`, skip skipped, a kind mismatch under overwrite, and a same-name
//!   folder, which is never merged across volumes, [`CROSSES_VOLUMES`]. After the plan, outside every lock, the item is
//!   copied into a staging under a base temp name in the destination folder (`LibraryStore::copy_tree`: create-new,
//!   every entry, a link inside refuses [`LINKS_INSIDE`] with nothing copied, tags before bytes, each file read only
//!   through a handle checked to be the file listed, hashes while copying, modified times and permissions kept); then
//!   the waiting count and the ticket. In the hold: the root generation the copy was made under
//!   ([`TARGET_MOUNT_CHANGED`]), both `.trash` judged again, the item listed again and compared entry by entry and its
//!   racy files hashed again ([`SOURCE_CHANGED`], the item gone included) — a change keeps both, the staging named by
//!   its virtual path as made (`staging`) and its disk path (`staging_disk`), and so does a hash that cannot be read —;
//!   the staging placed by the no-replace rename and `on_conflict` as an item on one volume (overwrite: the entry there
//!   into its trash first, renamed back when the staging cannot take its name); then the original into its own mount's
//!   `.trash` (the answer's `original`), and the pair followed by the catalog and the references as on one volume. A
//!   copy that failed part-way (the answer naming the entry it stopped at — `not_copied`), or a staging that does not
//!   take its place (skipped: [`Skip::TakenDuringCopy`] names it), goes into the destination mount's `.trash` as a
//!   staging row (`staging_row`); one that cannot go there stays, named, with the trash step's refusal (`staging_kept`
//!   — `trash-not-reserved` …), as does a staging when an overwrite's entry could not be renamed back. A refusal of the
//!   whole call inside the hold — a stale `root_generation`, which the hold compares itself once anything was copied,
//!   or the `SKILL.md` judgement — first does the same with every staging (one whose mount is no longer registered at
//!   the folder it was copied into stays) and names them in its `args` (`stagings`). An original that cannot be trashed
//!   leaves its copy in place: no pair, the copy walked into the catalog after the hold, both paths answered (`copy`).
//!   An item on one volume whose rename in the hold answers another volume (the probe went stale) fails
//!   [`TARGET_MOUNT_CHANGED`].
//! - **Copy** (ADR-0409 decisions 3, 12 and 14, M3, M14 · ADR-0416): every copy — one inside a mount too — goes the way
//!   of a move across volumes, without a volume probe, a source compare or a reference fix. Before the ticket, per
//!   item: the entrance rules above for the destination; the item a library path below a mount's top (a mount's top and
//!   `engram` itself `virtual-root-read-only`) or, from the built-in mount, one of its items (`builtin::file_of` — the
//!   copy takes that file's name) or its virtual folder `engram/settings` (copied as a folder holding its item's file),
//!   each file reached from the built-in folder down without following (a link `path-through-link`), anything else
//!   there `not-found` and a built-in mount that cannot be read `library-unavailable`; the destination folder a real
//!   folder (missing or a file `not-found` naming the folder, a link `path-through-link`); a destination inside the
//!   item [`INTO_ITSELF`] (as a move judges it); the destination mount's `.trash` usable (`trash-unusable` — a staging
//!   that does not take its place goes there); a taken destination name answered as a move across volumes answers it,
//!   except that a same-name folder is not merged [`COPY_MERGE_REFUSED`]. A copy into its item's own folder (by path
//!   key, else the folders' identities, else their canonical paths, else — nothing answering — taken as its own folder;
//!   judged once here after the generation the hold compares was read) takes the copy name instead (` - Copy`, then ` -
//!   Copy (2)` … — [`copy_name`], whatever `on_conflict` says). A stale `root_generation` refuses before anything is
//!   copied. Then the staging copy as a move across volumes makes it ([`LINKS_INSIDE`], `not_copied`), the waiting
//!   count and the ticket. In the hold: the root generation (the whole call, as a move's: every staging trashed or kept
//!   and named in `stagings`), each item's root generation since its copy ([`TARGET_MOUNT_CHANGED`], kept), a failed
//!   copy (a staging row), the destination checked as a move's and its `.trash` judged again; then the no-replace
//!   rename of the staging onto its name — `on_conflict` as a move across volumes, an overwrite's entry into its mount's
//!   `.trash` first — or onto the first free copy name; a staging that does not take its place goes into the
//!   destination mount's `.trash` as a staging row (skipped: [`Skip::TakenDuringCopy`]). The folder a copy landed in is
//!   re-classified in the hold and an overwrite's displaced range leaves the catalog; the copy itself enters the
//!   catalog by a folder-scoped walk after the hold, before the answer (ADR-0409 M3). Nothing refers to a copy and the
//!   original stays, so no reference or agent moves; the change number is raised as by every hold that applies
//!   something (`Gate::change_held_at`).
//! - **Stop rules** (ADR-0335, relink's): a per-file refusal skips the file; a read or write failure stops before the
//!   agents (the answer carries it); a file in a mount that cannot be read is skipped `library-unavailable`, a mount
//!   never walked this run is one skipped row at its top. Nothing is journaled: an interrupted detach leaves kept
//!   references, an interrupted move lines that read as not found until one relink of the old path fixes them (for a
//!   merge's exact folder pair a relink re-points by prefix, so entries left under the old folder follow too — rare).
//! - **Restore** (ADR-0409 decisions 5, 6 and 9): per row `{mount, bundle, path}` — a bundle name that does not parse
//!   as an id is foreign (`not-found`); the trash folder and the bundle are checked without following (a check that
//!   fails other than by absence `library-read-failed`); the row is found in the record's existing roots (a staging
//!   root `not-restorable`, a path inside a root `inside-root`, an `other` row `not-restorable`, a root behind a link
//!   `path-through-link`) or, without a readable record, among its files and empty folders (a temp name
//!   `not-restorable`; a bundle's files are walked once per call). The destination — the same path in the same mount,
//!   spelled as the bundle holds it — is checked name by name without following before anything is made (a link on the
//!   way `path-through-link`), missing parents are made with `create_dir` one name at a time (removed again,
//!   empty-only, when the move fails), and the row is moved back by the no-replace rename: a taken name is that row's
//!   conflict — with keep-both it goes back as `name (2)`, `name (3)`… — and nothing is ever overwritten. The bundle
//!   shell is pruned after each row. The catalog gains each restored range by a folder-scoped walk after the hold —
//!   from the topmost folder the restore made, so those folders are listed too (a scan classifies it — a restored
//!   `SKILL.md` makes its folder a skill), before the answer.
// ADR-0335
// ADR-0339
// ADR-0358
// ADR-0365
// ADR-0377
// ADR-0400 decisions 6, 18, 23 and 25
// ADR-0405
// ADR-0409
// ADR-0414
// ADR-0416

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
use engram_dashboard_base::file::temp_path;
use engram_dashboard_platform::fs::{path_key, same_path, PathKey};
use serde_json::{json, Value};

use crate::agent_edit::port_refused;
use crate::builtin::file_of;
use crate::catalog::{
    is_racy, key, makes_skill, preset_refs, skill_file_name, within, Catalog, ItemContent,
    ItemKind, ScanItem, ScanResult,
};
use crate::create::NAME_EXISTS;
use crate::edit::{as_read, lib_path, list_mut, read_path, ITEM_LISTS, LIBRARY_WRITE_FAILED};
use crate::format::read::ITEM_PATH_INVALID;
use crate::gate::NOT_FOUND;
use crate::launch::LIBRARY_READ_FAILED;
use crate::mounts::{
    admits_inside, builtin_rest, in_builtin, is_builtin, lies_within, virtual_path, DiskProbe,
    FolderProbe, Mount, MountName, MountPath, BUILTIN_MOUNT,
};
use crate::path::{AssetRef, BuiltinId, LibPath};
use crate::relink::{RelinkSkip, RelinkStop};
use crate::rename::{lines_digest, GitTree};
use crate::resolve::ref_key;
use crate::store::{
    tree_path, CopiedEntry, CopyFailure, EntryKind, LibraryStore, NameWalk, ScanEntry, ScanListing,
    TreeCopy, TreeEntry, NO_REPLACE_UNSUPPORTED,
};
use crate::trash::{
    below, check_trash_folder, ensure_trash_folder, fallback_rows, make_bundle, prune_shell,
    publish_record, read_record, row_path, trash_unusable, BundleId, FallbackRow, RecordRead,
    RecordRoot, RootState, TrashHeld, TrashUnavailable, TRASH_NOT_RESERVED,
};
use crate::{
    mount_read_only, mount_unavailable, refuse_unusable_now, stale_root_generation,
    virtual_root_read_only, Hold, LibraryRefusal, LibraryService, MountsNow, RefusalClass,
    LIBRARY_READ_CAP, PATH_THROUGH_LINK,
};

/// The code of a delete whose items are referenced, refused for the whole call (`Conflict`; `args` = `{plan,
/// plan_digest, forms}` — the referencing preset files and agents, and the two ways to go on: `yes` keeps the
/// references, `yes` + `detach_refs` + that `plan_digest` removes them).
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

/// The code of `detach_refs` on a move or rename with no item of a detachable shape — no lone `SKILL.md` leaving its
/// skill folder and no merge leaving a skill folder without its `SKILL.md` (`InvalidArgument`, `args` = `{}`).
// ADR-0339 M108
pub const DETACH_REFS_INVALID: &str = "detach-refs-invalid";

/// The code of a move onto a same-name folder on another volume (`args` = `{path, to}`) — that item's conflict, nothing
/// copied or moved: a merge is not done across volumes.
// ADR-0409 decision 12
pub const CROSSES_VOLUMES: &str = "crosses-volumes";

/// The code of a move across volumes or a copy whose item holds links (`args` = `{path, links}` — their virtual
/// paths): nothing copied or moved.
// ADR-0365
pub const LINKS_INSIDE: &str = "links-inside";

/// The code of a move across volumes whose item changed between its copy and its hold — an entry added or removed, a
/// kind or a tag changed, a racy file's bytes (`args` = `{path, staging}`): the item and its copy are both kept.
// ADR-0409 decision 12
pub const SOURCE_CHANGED: &str = "source-changed";

/// The code of a move or a copy whose mounts changed under it (`args` = `{path, to}`): a move across volumes or a copy
/// whose hold finds the mount registrations changed since its copy (the root generation moved — the copy is kept,
/// `staging` names it), or a rename in the hold answering that its destination lies on another volume although the
/// volume probe found one volume.
// ADR-0377
// ADR-0409 decision 13
pub const TARGET_MOUNT_CHANGED: &str = "target-mount-changed";

/// The code of a copy onto a same-name folder (`args` = `{path, to}`) under any `on_conflict` — that item's conflict,
/// nothing copied: a copy is not merged into a folder (as a move across volumes is not — ADR-0416 decision 5).
// ADR-0416
pub const COPY_MERGE_REFUSED: &str = "copy-merge-refused";

/// How many base temp names a volume probe or a staging copy draws before giving up on taken names.
const TEMP_DRAWS: usize = 16;

/// The names a volume probe's and a staging's temp names are drawn from (`<base>.tmp<pid>.<n>`) — short and fixed, so an
/// item whose name is near the length limit still gets one; [`Staged::intended`] keeps the name a staging is meant for.
const PROBE_BASE: &str = "engram-probe";
const STAGING_BASE: &str = "engram-staging";

/// How many keep-both names (`name (2)` …) a restore, a move or a copy tries before it answers the conflict, and how many
/// copy names (` - Copy` …) a copy into its item's own folder tries.
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
    /// Something is at this destination path and `on_conflict` said skip.
    Taken(String),
    /// Something came to this destination path during a staging copy (a move across volumes, or a copy) and `on_conflict`
    /// said skip: the copy's staging, named as a failed item names it (`staging_row`, or `staging` · `staging_disk` · `staging_kept`).
    TakenDuringCopy { path: String, staging: Value },
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
    /// With `detach_refs`: the deleted paths whose references were removed (the items that went).
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
    /// What `yes` + `detach_refs` must carry.
    pub plan_digest: String,
}

impl DeletePlan {
    /// The refusal's `args` — its `forms` are the arguments to add to the same call, spelled as the commands declare
    /// them, so a caller can send them back as they are.
    // ADR-0419
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
                "detach": { "yes": true, "detach_refs": true, "plan_digest": self.plan_digest },
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

/// How a move or a copy answers a name taken at its destination — Explorer's three choices; none = that item's
/// conflict.
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
    /// A move across volumes: the trash row its original went to, in its own mount's trash.
    pub original: Option<TrashAddress>,
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
    /// With `detach_refs`: the skill folders whose references were removed.
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

/// What a copied item came to.
#[derive(Clone, Debug, PartialEq)]
pub struct Copied {
    /// The virtual path the copy is at.
    pub to: String,
    /// The entry an overwrite moved into the trash of its mount.
    pub trashed: Option<TrashAddress>,
}

/// One given path of a copy and what came of it.
#[derive(Clone, Debug, PartialEq)]
pub struct CopyItem {
    pub path: String,
    pub outcome: Outcome<Copied>,
}

/// The answer of [`LibraryService::copy_items`].
#[derive(Debug)]
pub struct CopyAnswer {
    /// In the order given.
    pub items: Vec<CopyItem>,
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

#[cfg(test)]
thread_local! {
    /// The next move on this thread runs its hold without the trash waiting count — a test makes its trash step
    /// unexpected.
    pub(crate) static UNRESERVED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
thread_local! {
    /// Run once on the operating thread right after a copy's judgements before its ticket — a test lands a mount change
    /// there.
    pub(crate) static AFTER_JUDGEMENT: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// [`AFTER_JUDGEMENT`]'s seam; nothing outside tests.
fn after_judgement() {
    #[cfg(test)]
    if let Some(hook) = AFTER_JUDGEMENT.with(|hook| hook.borrow_mut().take()) {
        hook();
    }
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

    /// Whether the delete may go on: nothing referenced, plain `yes`, or `yes` + `detach_refs` carrying this digest
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

/// One item of a move, rename or copy: where it was given, where it is and where it goes; a lone `SKILL.md` leaving its skill
/// folder carries that folder (from the catalog — ADR-0339 M110), a folder merging into one of its name the `SKILL.md`
/// of each skill folder the merge would leave without it ([`LibraryService::plan_leaves`]).
#[derive(Clone, Debug)]
struct Request {
    at: usize,
    path: LibPath,
    dest: LibPath,
    skill_folder: Option<LibPath>,
    leaves: Vec<LibPath>,
    /// It is copied into a staging before the ticket and never merged: a move whose destination folder lies on another
    /// volume (the volume probe), or a copy.
    stages: bool,
    /// A copy into its item's own folder ([`LibraryService::into_own_folder`], judged before the ticket): it takes a copy
    /// name.
    own_folder: bool,
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

/// An item copied before its ticket — a move's across volumes, or a copy's: its staging (a base temp name in the destination folder — its mount,
/// path and disk path), the path in that mount it is meant for, the root generation the copy was made under, and what
/// the copy saw — or the failure that stopped it after the staging was made, with the entry it stopped at below the
/// item ([`CopyFailure::Failed`]'s `at`).
struct Staged {
    mount: Mount,
    rest: MountPath,
    disk: PathBuf,
    intended: MountPath,
    generation: u64,
    copy: Result<TreeCopy, (io::Error, PathBuf)>,
}

impl Staged {
    fn lib(&self) -> LibPath {
        virtual_path(&self.mount.name, Some(&self.rest))
    }

    /// The staging as a move's source side.
    fn side(&self) -> Side {
        Side {
            mount: self.mount.clone(),
            rest: self.rest.clone(),
            disk: self.disk.clone(),
        }
    }
}

/// Where a staging copy reads its item ([`LibraryService::copy_source`]).
enum Source {
    /// The tree at `disk` below `root` — the item's mount folder, or the built-in folder for a built-in item.
    Tree { root: PathBuf, disk: PathBuf },
    /// A built-in virtual folder (`engram/settings`): copied as a folder holding the files of its built-ins (below the
    /// built-in folder `root`), each `(disk path, file name, the built-in's last name)`.
    Folder {
        root: PathBuf,
        files: Vec<(PathBuf, String, String)>,
    },
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
    /// Taken, and `on_conflict` said skip: what is at this path.
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
    /// The folders whose direct children changed: both sides of a merge, and the parent a copy or a kept cross-volume
    /// copy landed in.
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
        self.put_root(store, mount, rest, disk, path, None)
    }

    /// [`Trashing::put`] of a staging this operation made (a copy that failed or found its name taken), recorded as a
    /// staging row meant for `intended` — shown, never restorable (ADR-0409 decision 8 · M14).
    fn put_staging(
        &mut self,
        store: &dyn LibraryStore,
        staged: &Staged,
    ) -> Result<(TrashAddress, BundleId), Reason> {
        let path = staged.lib();
        let intended = Some(staged.intended.clone());
        self.put_root(
            store,
            &staged.mount,
            &staged.rest,
            &staged.disk,
            &path,
            intended,
        )
    }

    fn put_root(
        &mut self,
        store: &dyn LibraryStore,
        mount: &Mount,
        rest: &MountPath,
        disk: &Path,
        path: &LibPath,
        staging: Option<MountPath>,
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
            staging,
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
                "library: bundle shell left after a put-back: {e}"
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

/// A move's or a copy's disk step as it goes.
struct Step<'h, 'a> {
    trashing: Trashing<'h, 'a>,
    moves: Moves,
    /// The `SKILL.md` of each skill folder the hold's judgement confirmed a merge may leave behind (`Request.leaves`).
    leaves: Vec<PathKey>,
    /// The current item's merge: the entries left, and those an overwrite displaced.
    left: Vec<(String, Reason)>,
    trashed: Vec<TrashAddress>,
    /// The copies a move across volumes put in place without trashing their originals, and every copy a copy put in
    /// place — walked into the catalog after the hold (ADR-0409 M3).
    walks: Vec<LibPath>,
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
    walks: Vec<LibPath>,
}

/// What a copy's hold did.
struct Copies {
    items: Vec<(usize, Outcome<Copied>)>,
    bundled: Vec<PathBuf>,
    /// The copies put in place — walked into the catalog after the hold (ADR-0409 M3).
    walks: Vec<LibPath>,
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

    /// The reference fix of a delete with `detach_refs` (module header) over the targets that went, kept to what the
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
    /// `root-generation-stale`. Either conflict met inside the hold once items were copied across volumes also names
    /// their stagings (`stagings` — module header).
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
                        stages: false,
                        own_folder: false,
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
    /// item's conflict `name-exists` (no `on_conflict`); a case-only rename goes through the no-replace rename like any
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
                    stages: false,
                    own_folder: false,
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

    /// Copies `paths` into the folder `to` (module header): each keeps its name — a built-in item takes its file's —
    /// or, copied into its own folder, takes the copy name (` - Copy` …); a name taken there follows `on_conflict` (none =
    /// that item's conflict; a same-name folder is never merged). Answers per item.
    ///
    /// Refusals of the whole call, nothing applied: `to` no library path, or a dot or temp name in it `InvalidArgument` ·
    /// `item-path-invalid`; `to` in the built-in `Conflict` · `mount-read-only`; no first scan applied `Unavailable` ·
    /// `library-unavailable`; a stale `root_generation` `Conflict` · `root-generation-stale` — met inside the hold, it
    /// also names the stagings (`stagings` — module header).
    // ADR-0409 decisions 3 and 12
    pub fn copy_items(
        &self,
        paths: &[&str],
        to: &str,
        on_conflict: Option<OnConflict>,
        root_generation: Option<u64>,
    ) -> Result<CopyAnswer, LibraryRefusal> {
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
        let mut outcomes: Vec<Option<Outcome<Copied>>> = vec![None; paths.len()];
        let mut requests = Vec::new();
        for (at, given) in paths.iter().enumerate() {
            match copyable(given) {
                Ok((path, name)) => {
                    let dest = LibPath::parse(&format!("{}/{name}", to.as_str())).expect(
                        "an item's name or a built-in's file name is a valid name in a folder",
                    );
                    requests.push(Request {
                        at,
                        path,
                        dest,
                        skill_folder: None,
                        leaves: Vec::new(),
                        stages: true,
                        own_folder: false,
                    });
                }
                Err(why) => outcomes[at] = Some(Outcome::Failed(why)),
            }
        }
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
        // A copy into its own folder takes a copy name (UD3); one into itself or below it is refused.
        requests.retain(|request| {
            let into = key(&request.dest) != key(&request.path)
                && within(&request.dest, Some(&request.path));
            if into {
                outcomes[request.at] = Some(Outcome::Failed(into_itself(request)));
            }
            outcomes[request.at].is_none()
        });
        if !requests.is_empty() {
            self.first_scan_join()
                .map_err(LibraryRefusal::unavailable)?;
        }
        // ADR-0400 decision 6: nothing is copied out of or into a mount that cannot be read.
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
        // ADR-0377: the generation the hold compares is taken before the mounts the judgements below read, so a mount
        //   change after it — which may make a spelling the item's own folder — stops the item in the hold.
        let generation = self.gate.numbers().root;
        self.refuse_into_itself(&mut requests, &mut outcomes);
        self.refuse_copying(&mut requests, &mut outcomes, on_conflict);
        after_judgement();
        let mut copied = None;
        if !requests.is_empty() {
            // ADR-0377: a stale generation refuses before anything is copied.
            if let Some(given) = root_generation.filter(|&given| given != generation) {
                return Err(stale_root_generation(Some(given), generation));
            }
            let staged = self.stage(&mut requests, &mut outcomes, generation);
            // ADR-0409 decision 14: every copy may put its staging into a trash — raised after the copies, just before
            //   the ticket. ADR-0377: every item that reaches the hold was copied, so its body compares the generation.
            let trash = reserved();
            before_hold();
            let done = match requests.is_empty() {
                true => Ok(Ok(None)),
                false => self.change_held_at(None, trash, |catalog, mounts, hold| {
                    let mut touched = Vec::new();
                    let answer = self.copy_in(
                        &requests,
                        &staged,
                        root_generation,
                        on_conflict,
                        catalog,
                        mounts,
                        hold,
                        &mut touched,
                    );
                    (answer.map(Some), touched)
                }),
            };
            copied = done?.map_err(|(refusal, bundled)| {
                for folder in bundled {
                    self.exclude_trash(folder);
                }
                refusal
            })?;
        }
        let mut walks: Vec<LibPath> = Vec::new();
        if let Some(done) = copied {
            for folder in done.bundled {
                self.exclude_trash(folder);
            }
            walks = done.walks;
            for (at, outcome) in done.items {
                outcomes[at] = Some(outcome);
            }
        }
        self.walk_copies(&walks);
        let items: Vec<CopyItem> = paths
            .iter()
            .zip(outcomes)
            .map(|(path, outcome)| CopyItem {
                path: (*path).to_owned(),
                outcome: outcome.expect("every item answered"),
            })
            .collect();
        let done = items
            .iter()
            .filter(|item| matches!(item.outcome, Outcome::Done(_)))
            .count();
        tracing::info!(items = items.len(), done, "library copy");
        Ok(CopyAnswer { items })
    }

    /// Whether `request` copies its item into the item's own folder — the copy then takes a copy name (UD3), never the
    /// item's own name. The same path key; else (another spelling — a case or short-name alias) whether the destination
    /// folder is the item's folder: by file identity, or where the volume gives none by canonical path (as
    /// `mounts::lies_within` places folders) — both asked through the bounded IO on the admitted spellings (ADR-0365:
    /// these asks follow links). When neither answers it counts as the item's own folder: a copy name never destroys
    /// anything, an overwrite of the item itself would. Asked before the ticket, holding no lock; the hold reads the answer
    /// (`Request.own_folder`).
    // ADR-0409 decision 3
    fn into_own_folder(&self, mounts: &crate::mounts::Mounts, request: &Request) -> bool {
        if key(&request.dest) == key(&request.path) {
            return true;
        }
        if in_builtin(&request.path) {
            return false;
        }
        let (Ok(item), Ok(dest)) = (
            self.admit(mounts, &request.path),
            self.admit(mounts, &request.dest),
        ) else {
            return true;
        };
        let folders: Vec<PathBuf> = [item, dest]
            .iter()
            .map(|path| {
                path.parent()
                    .expect("an admitted path lies in a folder")
                    .to_path_buf()
            })
            .collect();
        let read = self.io.begin_read();
        let probe = DiskProbe {
            io: &self.io,
            read: &read,
        };
        if let [Ok(Some(a)), Ok(Some(b))] = probe.identity_all(&folders).as_slice() {
            return a == b;
        }
        match probe.canonical_all(&folders).as_slice() {
            [Ok(a), Ok(b)] => path_key(a) == path_key(b),
            _ => true,
        }
    }

    /// Before any copy, per item (module header): the destination mount's `.trash` checked without following (a
    /// staging that does not take its place goes there — unusable `trash-unusable`, nothing copied), the destination
    /// folder ([`LibraryService::destination_folder`] — a missing folder, a file or a link fails the item naming the
    /// folder), the item's source ([`LibraryService::copy_source`] — its refusal fails the item), then, unless the copy
    /// goes into its item's own folder ([`LibraryService::into_own_folder`] — it takes a free copy name), its destination
    /// name stat'ed without following: a taken name answers as the
    /// hold would ([`taken_answer`] — a same-name folder [`COPY_MERGE_REFUSED`]). A stat or admission that does not
    /// answer is left to the copy and the hold.
    // ADR-0409 decision 5
    fn refuse_copying(
        &self,
        requests: &mut Vec<Request>,
        outcomes: &mut [Option<Outcome<Copied>>],
        on_conflict: Option<OnConflict>,
    ) {
        let mounts = self.gate.with_read(|state| state.mounts.clone());
        requests.retain_mut(|request| {
            // `Ok(own folder)` = the copy goes on (a stat or admission that does not answer is left to it).
            let judged = || -> Result<bool, Outcome<Copied>> {
                let Some((to_mount, _)) = mounts.locate(&request.dest) else {
                    return Ok(false);
                };
                if let Some(unusable) = self.unusable_trash(to_mount) {
                    return Err(Outcome::Failed(unusable));
                }
                self.destination_folder(&mounts, request)
                    .map_err(Outcome::Failed)?;
                let source = self
                    .copy_source(&mounts, &request.path)
                    .map_err(Outcome::Failed)?;
                if self.into_own_folder(&mounts, request) {
                    return Ok(true);
                }
                let kind = match source {
                    Source::Folder { .. } => EntryKind::Folder,
                    Source::Tree { disk, .. } => match self.store.stat(&disk) {
                        Ok(kind) => kind,
                        Err(_) => return Ok(false),
                    },
                };
                let there = self
                    .admit(&mounts, &request.dest)
                    .ok()
                    .and_then(|disk| self.store.stat(&disk).ok());
                let Some(there) = there else {
                    return Ok(false);
                };
                match taken_answer(request, kind, there, on_conflict, COPY_MERGE_REFUSED) {
                    Some(outcome) => Err(outcome),
                    None => Ok(false),
                }
            };
            match judged() {
                Ok(own_folder) => {
                    request.own_folder = own_folder;
                    true
                }
                Err(outcome) => {
                    outcomes[request.at] = Some(outcome);
                    false
                }
            }
        });
    }

    /// The body of a copy's hold (module header): the root generation, each staged item's checks — all before the
    /// call's first rename —, then each staging placed, the trash records and the catalog.
    #[allow(clippy::too_many_arguments)]
    fn copy_in(
        &self,
        requests: &[Request],
        staged: &HashMap<usize, Staged>,
        root_generation: Option<u64>,
        on_conflict: Option<OnConflict>,
        catalog: &mut Catalog,
        mounts: &MountsNow,
        hold: &Hold,
        touched: &mut Vec<PathKey>,
    ) -> Result<Copies, (LibraryRefusal, Vec<PathBuf>)> {
        // ADR-0377: the gate let the call in unchecked — the generation is judged here, before any disk step.
        let current = self.gate.numbers().root;
        if let Some(given) = root_generation.filter(|&given| given != current) {
            let stale = stale_root_generation(Some(given), current);
            return Err(self.refuse_staged(stale, requests, staged, mounts, hold));
        }
        let mut items: Vec<(usize, Outcome<Copied>)> = Vec::new();
        let mut step = Step {
            trashing: Trashing::new(hold),
            moves: Moves::default(),
            leaves: Vec::new(),
            left: Vec::new(),
            trashed: Vec::new(),
            walks: Vec::new(),
        };
        // ADR-0409 decision 5: the destination mount's `.trash` judged again with each destination.
        let mut checked: Vec<(&Request, &Staged, Side)> = Vec::new();
        for request in requests {
            let staging = staged
                .get(&request.at)
                .expect("every copy that reaches its hold was staged");
            if let Some(why) = self.unready(request, staging, &mut step.trashing) {
                items.push((request.at, Outcome::Failed(why)));
                continue;
            }
            let to = refuse_unusable_now(mounts, &request.dest)
                .map_err(reason_of)
                .and_then(|()| self.destination(request, catalog, mounts))
                .and_then(|to| match self.unusable_trash(&to.mount) {
                    Some(why) => Err(why),
                    None => Ok(to),
                });
            match to {
                Ok(to) => checked.push((request, staging, to)),
                Err(why) => {
                    let why = self.discard_staging(staging, &mut step.trashing, why);
                    items.push((request.at, Outcome::Failed(why)));
                }
            }
        }
        for (request, staging, to) in checked {
            let outcome = self.copy_staged(
                request,
                staging,
                &to,
                on_conflict,
                catalog,
                mounts,
                &mut step,
            );
            items.push((request.at, outcome));
        }
        let Step {
            trashing,
            moves,
            walks,
            ..
        } = step;
        let bundled = trashing.finish(&*self.store);
        self.follow_moves(catalog, mounts, hold, &moves, touched);
        Ok(Copies {
            items,
            bundled,
            walks,
        })
    }

    /// Places one checked copy (module header): its staging renamed onto `to` by the no-replace rename — into its
    /// item's own folder onto the first free copy name ([`LibraryService::place_copy_named`]), else as
    /// [`LibraryService::place_copy`] places a staging, a same-name folder [`COPY_MERGE_REFUSED`] —; a staging that does
    /// not take its place goes into the destination mount's trash ([`LibraryService::settle_staging`]). A placed copy's
    /// folder is re-classified in the hold and its range walked into the catalog after it (ADR-0409 M3).
    #[allow(clippy::too_many_arguments)]
    fn copy_staged(
        &self,
        request: &Request,
        staging: &Staged,
        to: &Side,
        on_conflict: Option<OnConflict>,
        catalog: &Catalog,
        mounts: &MountsNow,
        step: &mut Step<'_, '_>,
    ) -> Outcome<Copied> {
        let given = request.path.as_str();
        let kind = match self.store.stat(&staging.disk) {
            Ok(kind) => kind,
            Err(e) => {
                let why = reason(
                    LIBRARY_READ_FAILED,
                    json!({ "path": given, "why": e.to_string() }),
                );
                return Outcome::Failed(self.discard_staging(staging, &mut step.trashing, why));
            }
        };
        let side = staging.side();
        let placed = match request.own_folder {
            true => self.place_copy_named(&side, to, kind),
            false => {
                let refused = COPY_MERGE_REFUSED;
                self.place_copy(&side, to, kind, on_conflict, refused, catalog, mounts, step)
            }
        };
        match self.settle_staging(placed, staging, given, &mut step.trashing) {
            Ok((to, trashed)) => {
                step.walks.push(to.lib());
                step.moves.folders.extend(to.lib().parent());
                Outcome::Done(Copied {
                    to: to.lib().as_str().to_owned(),
                    trashed,
                })
            }
            Err(outcome) => outcome,
        }
    }

    /// Places `staging` (of `kind`) at the first free copy name of `to` — its item's own path, copied into its own
    /// folder: ` - Copy`, then ` - Copy (2)` … ([`copy_name`]) — by the no-replace rename (ADR-0409 decisions 3 and 10).
    fn place_copy_named(&self, staging: &Side, to: &Side, kind: EntryKind) -> Placement {
        for n in 1..=KEEP_BOTH_TRIES {
            let next = to.at(copy_name(&to.rest, n, kind != EntryKind::Folder));
            match self.store.rename(&staging.disk, &next.disk) {
                Ok(()) => {
                    return Placement::Moved {
                        to: next,
                        trashed: None,
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Placement::Failed(rename_failed(&staging.lib(), &next.lib(), &e)),
            }
        }
        Placement::Conflict(name_taken(&to.lib()))
    }

    /// Walks each range in `walks` — copies put in place by the hold — into the catalog, as a scan classifies it
    /// (ADR-0409 M3), before the answer.
    fn walk_copies(&self, walks: &[LibPath]) {
        for path in walks {
            let report = self.refresh(Some(path));
            if !report.errors.is_empty() {
                tracing::warn!(
                    path = path.as_str(),
                    errors = report.errors.len(),
                    "library: the walk of a copied range reported errors"
                );
            }
        }
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
        // ADR-0409 decisions 12 and 13: which items cross volumes, and what a copy needs, before the plan — the copies
        //   themselves after it, so a refused call copies nothing.
        let generation = self.gate.numbers().root;
        self.probe_volumes(&mut requests, &mut outcomes);
        self.refuse_crossing(&mut requests, &mut outcomes, on_conflict);
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
        let mut walks: Vec<LibPath> = Vec::new();
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
            // ADR-0377: a stale generation refuses before anything is copied.
            if let Some(given) = root_generation.filter(|&given| given != generation) {
                if requests.iter().any(|request| request.stages) {
                    return Err(stale_root_generation(Some(given), generation));
                }
            }
            let staged = self.stage(&mut requests, &mut outcomes, generation);
            // ADR-0409 decision 14: of a same-volume move only an overwrite puts anything in a trash; a move across volumes
            //   may (its original, or its staging) — raised after the copies, just before the ticket.
            let trash =
                (on_conflict == Some(OnConflict::Overwrite) || !staged.is_empty()) && reserved();
            // ADR-0377: once anything was copied the hold's body compares the generation, so its refusal sees to the
            //   stagings first.
            let gate_generation = root_generation.filter(|_| staged.is_empty());
            before_hold();
            let done = match requests.is_empty() {
                true => Ok(Ok(None)),
                false => self.change_held_at(gate_generation, trash, |catalog, mounts, hold| {
                    let mut touched = Vec::new();
                    let answer = self.move_in(
                        &requests,
                        &staged,
                        root_generation,
                        on_conflict,
                        asked,
                        detach,
                        port,
                        catalog,
                        mounts,
                        hold,
                        &mut touched,
                    );
                    (answer.map(Some), touched)
                }),
            };
            let done = done?.map_err(|(refusal, bundled)| {
                for folder in bundled {
                    self.exclude_trash(folder);
                }
                refusal
            });
            match done {
                Err(LibraryRefusal::Refused { reason, .. }) if reason.code == SKILL_UNCONFIRMED => {
                    // The fresh plan classifies as the hold did; the stagings the hold named stay named.
                    self.classify_relocation(&mut requests, on_conflict);
                    let targets: Vec<Target> = requests.iter().flat_map(Request::targets).collect();
                    let mut fresh = skill_unconfirmed(&self.delete_plan(&targets).1);
                    if let (Some(stagings), LibraryRefusal::Refused { reason: asked, .. }) =
                        (reason.args.get("stagings"), &mut fresh)
                    {
                        asked.args["stagings"] = stagings.clone();
                    }
                    return Err(fresh);
                }
                done => relocated = done?,
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
            walks = done.walks;
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
        // ADR-0409 M3: a copy whose original could not be trashed enters the catalog by a walk, as a scan classifies it.
        self.walk_copies(&walks);
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
    /// behind is a `SKILL.md` moved out): with `on_conflict`, a folder item whose destination is a folder of its name
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
            // A move across volumes never merges.
            let Some(choice) = on_conflict.filter(|_| !request.stages) else {
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
    fn refuse_into_itself<T>(
        &self,
        requests: &mut Vec<Request>,
        outcomes: &mut [Option<Outcome<T>>],
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

    /// The volume probe (ADR-0409 decision 13), before the ticket and holding no lock: for each item moving to another
    /// folder, once per (item's folder, destination folder) pair — none for a rename in its own folder —, an empty
    /// folder is made create-new under a base temp name beside the item, renamed without replacing into the
    /// destination folder and removed there empty-only. `CrossesDevices` marks the item `crosses` (the probe is removed
    /// beside the item); another failure fails the item with its code (a probe that cannot be removed stays — a
    /// leftover temp). An item that is not there or does not pass the link admission, or whose destination folder does
    /// not or is no real folder, is not probed: the hold answers it.
    fn probe_volumes(&self, requests: &mut Vec<Request>, outcomes: &mut [Option<Outcome<Moved>>]) {
        let mounts = self.gate.with_read(|state| state.mounts.clone());
        // Per (item's folder, destination folder): crosses, or the probe's failure.
        let mut probed: HashMap<(PathKey, PathKey), Result<bool, (io::ErrorKind, String)>> =
            HashMap::new();
        requests.retain_mut(|request| {
            let (from, into) = (
                request.path.parent().expect("an item lies in a folder"),
                request
                    .dest
                    .parent()
                    .expect("a destination lies in a folder"),
            );
            if key(&from) == key(&into) {
                return true;
            }
            let Ok(item) = self.admit(&mounts, &request.path) else {
                return true;
            };
            let into_disk = match into.parent() {
                None => mounts.disk_path(&into),
                Some(_) => self
                    .admit(&mounts, &into)
                    .ok()
                    .filter(|disk| matches!(self.store.stat(disk), Ok(EntryKind::Folder))),
            };
            let (Some(into_disk), Ok(_)) = (into_disk, self.store.stat(&item)) else {
                return true;
            };
            let beside = item
                .parent()
                .expect("an item's disk path lies in its folder");
            let verdict = probed
                .entry((path_key(beside), path_key(&into_disk)))
                .or_insert_with(|| {
                    self.probe(&item, &into_disk)
                        .map_err(|e| (e.kind(), e.to_string()))
                });
            match verdict {
                Ok(crosses) => {
                    request.stages = *crosses;
                    true
                }
                Err((kind, why)) => {
                    let args =
                        json!({ "path": request.path.as_str(), "to": into.as_str(), "why": why });
                    let code = match kind {
                        io::ErrorKind::Unsupported => NO_REPLACE_UNSUPPORTED,
                        _ => LIBRARY_WRITE_FAILED,
                    };
                    tracing::warn!(
                        path = request.path.as_str(),
                        "library move: volume probe failed: {}",
                        escape_controls(why)
                    );
                    outcomes[request.at] = Some(Outcome::Failed(reason(code, args)));
                    false
                }
            }
        });
    }

    /// One volume probe (module header): whether the folder `into` lies on another volume than the folder holding
    /// `item` — an empty folder made under a fresh base temp name of [`PROBE_BASE`] beside it, renamed into `into`
    /// without replacing, then removed empty-only wherever it is. A taken temp name, beside the item or in `into`, draws
    /// another.
    fn probe(&self, item: &Path, into: &Path) -> io::Result<bool> {
        for _ in 0..TEMP_DRAWS {
            let probe = temp_path(&item.with_file_name(PROBE_BASE))?;
            match self.store.create_dir(&probe) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
            let there = into.join(probe.file_name().expect("a temp path has a name"));
            let (left, crosses) = match self.store.rename(&probe, &there) {
                Ok(()) => (there, Ok(false)),
                Err(e) if e.kind() == io::ErrorKind::CrossesDevices => (probe, Ok(true)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (probe, Err(None)),
                Err(e) => (probe, Err(Some(e))),
            };
            // ADR-0409: only what Engram made — the probe is this call's and empty.
            if let Err(e) = self.store.remove_dir(&left) {
                tracing::warn!(
                    path = %escape_controls(&left.to_string_lossy()),
                    "library move: volume probe left: {e}"
                );
            }
            match crosses {
                Ok(crosses) => return Ok(crosses),
                Err(None) => continue,
                Err(Some(e)) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "every volume probe name drawn was taken",
        ))
    }

    /// Before any copy, each item crossing volumes (ADR-0409 decisions 5 and 12): the destination mount's `.trash` (a
    /// copy that fails goes there) and the item's own mount's (its original goes there) checked without following —
    /// unusable `trash-unusable`, nothing copied —, then its destination name stat'ed without following: a taken name
    /// answers as the hold would, without a copy — no `on_conflict` `name-exists`, skip skipped, a file onto a folder or
    /// the reverse under overwrite [`KIND_MISMATCH`] — and a same-name folder is not merged across volumes
    /// [`CROSSES_VOLUMES`]. A name free, or a stat or admission that does not answer, is left to the copy and the hold.
    fn refuse_crossing(
        &self,
        requests: &mut Vec<Request>,
        outcomes: &mut [Option<Outcome<Moved>>],
        on_conflict: Option<OnConflict>,
    ) {
        let mounts = self.gate.with_read(|state| state.mounts.clone());
        requests.retain(|request| {
            if !request.stages {
                return true;
            }
            let refused = || -> Option<Outcome<Moved>> {
                let (from_mount, _) = mounts.locate(&request.path)?;
                let (to_mount, _) = mounts.locate(&request.dest)?;
                if let Some(unusable) = self
                    .unusable_trash(to_mount)
                    .or_else(|| self.unusable_trash(from_mount))
                {
                    return Some(Outcome::Failed(unusable));
                }
                let there = self
                    .store
                    .stat(&self.admit(&mounts, &request.dest).ok()?)
                    .ok()?;
                let kind = self
                    .store
                    .stat(&self.admit(&mounts, &request.path).ok()?)
                    .ok()?;
                taken_answer(request, kind, there, on_conflict, CROSSES_VOLUMES)
            };
            match refused() {
                Some(outcome) => {
                    outcomes[request.at] = Some(outcome);
                    false
                }
                None => true,
            }
        });
    }

    /// The copies of the items that stage — a move's crossing volumes, every copy's (ADR-0409 decision 12) —, before the
    /// ticket and holding no lock: each into a staging under a fresh base temp name of [`STAGING_BASE`] in the
    /// destination folder ([`LibraryService::copy_from`] — create-new, links refused, tags before bytes, files read only
    /// as listed, hashes, modified times and permissions kept); a taken temp name draws another. Links fail the item
    /// [`LINKS_INSIDE`] and a copy that failed before making anything fails it with its code (`not_copied` = the item) —
    /// nothing to clean; a copy that failed after is carried to the hold, whose trash step takes its staging. Answers
    /// each staged item's [`Staged`], by where it was given.
    fn stage<T>(
        &self,
        requests: &mut Vec<Request>,
        outcomes: &mut [Option<Outcome<T>>],
        generation: u64,
    ) -> HashMap<usize, Staged> {
        let mounts = self.gate.with_read(|state| state.mounts.clone());
        let mut staged = HashMap::new();
        requests.retain(|request| {
            if !request.stages {
                return true;
            }
            let failed = |e: &io::Error| {
                let args =
                    json!({ "path": request.path.as_str(), "to": request.dest.as_str(), "why": e.to_string() });
                match e.kind() {
                    io::ErrorKind::NotFound => not_found_reason(request.path.as_str()),
                    // ADR-0365: a name on the way to the item became a link or no folder during the copy.
                    io::ErrorKind::NotADirectory => {
                        reason(PATH_THROUGH_LINK, json!({ "path": request.path.as_str() }))
                    }
                    _ => reason(LIBRARY_WRITE_FAILED, args),
                }
            };
            let made = || -> Result<Staged, Reason> {
                let source = self.copy_source(&mounts, &request.path)?;
                let to = self.admit(&mounts, &request.dest).map_err(reason_of)?;
                let Some((mount, Some(intended))) = mounts.locate(&request.dest) else {
                    return Err(not_found_reason(request.dest.as_str()));
                };
                for _ in 0..TEMP_DRAWS {
                    let disk = temp_path(&to.with_file_name(STAGING_BASE)).map_err(|e| failed(&e))?;
                    let copy = match self.copy_from(&source, &disk) {
                        Ok(copy) => Ok(copy),
                        Err(CopyFailure::Links(links)) => {
                            let links: Vec<String> = links
                                .iter()
                                .map(|rel| below_item(&request.path, rel))
                                .collect();
                            let args = json!({ "path": request.path.as_str(), "links": links });
                            let why = reason(LINKS_INSIDE, args);
                            // A staging the copy made and could not remove again (a built-in virtual folder's) stays,
                            //   named as every kept staging is.
                            let gone = matches!(
                                self.store.stat(&disk),
                                Err(e) if e.kind() == io::ErrorKind::NotFound
                            );
                            if gone {
                                return Err(why);
                            }
                            let rest = MountPath::from_disk(&mount.folder, &disk)
                                .expect("a temp name in a folder of the mount is a path in it");
                            let lib = virtual_path(&mount.name, Some(&rest));
                            return Err(named_staging(why, &lib, &disk));
                        }
                        Err(CopyFailure::Failed {
                            error, made: false, ..
                        }) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                        Err(CopyFailure::Failed {
                            error,
                            made: false,
                            at,
                        }) => {
                            let mut why = failed(&error);
                            if why.code != NOT_FOUND {
                                why.args["not_copied"] = json!(below_item(&request.path, &at));
                            }
                            return Err(why);
                        }
                        Err(CopyFailure::Failed {
                            error,
                            made: true,
                            at,
                        }) => {
                            tracing::warn!(
                                path = request.path.as_str(),
                                at = below_item(&request.path, &at),
                                "library staging copy failed part-way: {error}"
                            );
                            Err((error, at))
                        }
                    };
                    let rest = MountPath::from_disk(&mount.folder, &disk)
                        .expect("a temp name in a folder of the mount is a path in it");
                    return Ok(Staged {
                        mount: mount.clone(),
                        rest,
                        disk,
                        intended,
                        generation,
                        copy,
                    });
                }
                let taken = io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "every staging name drawn was taken",
                );
                Err(failed(&taken))
            };
            match made() {
                Ok(made) => {
                    staged.insert(request.at, made);
                    true
                }
                Err(why) => {
                    outcomes[request.at] = Some(Outcome::Failed(why));
                    false
                }
            }
        });
        staged
    }

    /// Where a staging copy of `path` reads it (module header): a library item admitted (ADR-0365) below its mount's
    /// folder; a built-in item, its file in the built-in folder (`builtin::file_of`); the built-in virtual folder
    /// (`engram/settings`), the files of the built-ins directly in it — each built-in file reached without following
    /// ([`LibraryService::admit_builtin`]). Another path in the built-in mount `not-found`;
    /// a built-in mount that cannot be read `library-unavailable`.
    // ADR-0400 decision 20
    fn copy_source(
        &self,
        mounts: &crate::mounts::Mounts,
        path: &LibPath,
    ) -> Result<Source, Reason> {
        let Some(inside) = builtin_rest(path) else {
            let disk = self.admit(mounts, path).map_err(reason_of)?;
            let (mount, _) = mounts
                .locate(path)
                .expect("an admitted path lies in a registered mount");
            return Ok(Source::Tree {
                root: mount.folder.clone(),
                disk,
            });
        };
        let root = self.builtin_folder().map_err(reason_of)?.clone();
        if let Ok(AssetRef::BuiltIn(id)) = AssetRef::parse(path.as_str()) {
            let disk = self.admit_builtin(&root, id, path)?;
            return Ok(Source::Tree { root, disk });
        }
        let mut files: Vec<(PathBuf, String, String)> = Vec::new();
        for id in builtins_in(inside) {
            let last = id.path().file_name().to_owned();
            let disk = self.admit_builtin(&root, id, path)?;
            files.push((disk, builtin_file_name(id).to_owned(), last));
        }
        match files.is_empty() {
            true => Err(not_found_reason(path.as_str())),
            false => Ok(Source::Folder { root, files }),
        }
    }

    /// The file of the built-in `id` below the built-in folder `root`, reached without following (ADR-0365, as
    /// [`LibraryService::admit`] reaches a library path — `LibraryStore::first_link` from `root` down, its own name
    /// included): a link on the way or at it `path-through-link` (`path` = the item), a probe that fails
    /// `library-read-failed`; a name missing passes (the copy answers it).
    fn admit_builtin(&self, root: &Path, id: BuiltinId, path: &LibPath) -> Result<PathBuf, Reason> {
        let rel =
            MountPath::parse(file_of(id)).expect("a built-in's file is a path below its folder");
        match self.store.first_link(root, &rel) {
            Ok(None) => Ok(rel.to_path(root)),
            Ok(Some(_)) => Err(reason(PATH_THROUGH_LINK, json!({ "path": path.as_str() }))),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                Ok(rel.to_path(root))
            }
            Err(e) => Err(reason(
                LIBRARY_READ_FAILED,
                json!({ "path": path.as_str(), "why": e.to_string() }),
            )),
        }
    }

    /// Copies `source` to the staging `to` ([`LibraryStore::copy_tree`]). A built-in virtual folder: `to` made
    /// create-new, then each of its files copied into it under its file name — the copy's entries spelled from `to`, a
    /// link or a failure named below the folder by the built-in's last name. A file whose copy made nothing (links, or a
    /// failure before its file was made) removes the folder again, empty-only: removed, nothing was made (a folder that
    /// held earlier files, or cannot be removed, stays — carried to the hold as made, or after links named by
    /// [`LibraryService::stage`]'s refusal).
    fn copy_from(&self, source: &Source, to: &Path) -> Result<TreeCopy, CopyFailure> {
        let (root, files) = match source {
            Source::Tree { root, disk } => return self.store.copy_tree(root, disk, to),
            Source::Folder { root, files } => (root, files),
        };
        let read_at = SystemTime::now();
        self.store
            .create_dir(to)
            .map_err(|error| CopyFailure::Failed {
                error,
                made: false,
                at: PathBuf::new(),
            })?;
        let top = TreeEntry {
            rel: PathBuf::new(),
            kind: EntryKind::Folder,
            tag: None,
        };
        let mut entries = vec![CopiedEntry {
            entry: top,
            hash: None,
        }];
        for (disk, file_name, last) in files {
            let failure = match self.store.copy_tree(root, disk, &to.join(file_name)) {
                Ok(copy) => {
                    entries.extend(copy.entries.into_iter().map(|mut copied| {
                        copied.entry.rel = tree_path(Path::new(file_name), &copied.entry.rel);
                        copied
                    }));
                    continue;
                }
                Err(failure) => failure,
            };
            // ADR-0409: only what Engram made — the staging folder is this call's; emptied, nothing was made.
            let empty = match &failure {
                CopyFailure::Links(_) | CopyFailure::Failed { made: false, .. } => {
                    match self.store.remove_dir(to) {
                        Ok(()) => true,
                        Err(e) => {
                            tracing::warn!(
                                path = %escape_controls(&to.to_string_lossy()),
                                "library copy: staging folder not removed: {e}"
                            );
                            false
                        }
                    }
                }
                CopyFailure::Failed { made: true, .. } => false,
            };
            let below = |rel: &Path| tree_path(Path::new(last), rel);
            return Err(match failure {
                CopyFailure::Links(links) => {
                    CopyFailure::Links(links.iter().map(|rel| below(rel)).collect())
                }
                CopyFailure::Failed { error, at, .. } => CopyFailure::Failed {
                    error,
                    made: !empty,
                    at: below(&at),
                },
            });
        }
        Ok(TreeCopy { read_at, entries })
    }

    /// The body of a move's hold (module header): the `SKILL.md` judgement again, each item's checks, the disk step
    /// item by item, the trash records, the catalog, then the reference fix.
    #[allow(clippy::too_many_arguments)]
    fn move_in(
        &self,
        requests: &[Request],
        staged: &HashMap<usize, Staged>,
        root_generation: Option<u64>,
        on_conflict: Option<OnConflict>,
        asked: (bool, bool, Option<&str>),
        detach: bool,
        port: Option<std::sync::Arc<dyn ProfileEditPort>>,
        catalog: &mut Catalog,
        mounts: &MountsNow,
        hold: &Hold,
        touched: &mut Vec<PathKey>,
    ) -> Result<Relocated, (LibraryRefusal, Vec<PathBuf>)> {
        // ADR-0377: with copies made the gate let the call in unchecked — the generation is judged here, before any
        //   disk step.
        if !staged.is_empty() {
            let current = self.gate.numbers().root;
            if let Some(given) = root_generation.filter(|&given| given != current) {
                let stale = stale_root_generation(Some(given), current);
                return Err(self.refuse_staged(stale, requests, staged, mounts, hold));
            }
        }
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
            let refusal = LibraryRefusal::new(RefusalClass::Conflict, SKILL_UNCONFIRMED, json!({}));
            return Err(self.refuse_staged(refusal, &requests, staged, mounts, hold));
        }
        let mut items: Vec<(usize, Outcome<Moved>)> = Vec::new();
        let mut step = Step {
            trashing: Trashing::new(hold),
            moves: Moves::default(),
            leaves: requests
                .iter()
                .flat_map(|request| request.leaves.iter().map(key))
                .collect(),
            left: Vec::new(),
            trashed: Vec::new(),
            walks: Vec::new(),
        };
        let mut placed: Vec<Placed> = Vec::new();
        for request in &requests {
            let staging = staged.get(&request.at);
            let unready =
                staging.and_then(|staging| self.unready(request, staging, &mut step.trashing));
            if let Some(why) = unready {
                items.push((request.at, Outcome::Failed(why)));
                continue;
            }
            match self.place_request(request, catalog, mounts) {
                Ok(item) => placed.push(item),
                Err(why) => {
                    let gone = why.code == NOT_FOUND && why.args["path"] == request.path.as_str();
                    let why = match staging {
                        // ADR-0409 decision 12: the item gone since its copy is a change — both kept, the copy
                        //   (perhaps all that is left of it) where it was made.
                        Some(staging) if gone => {
                            let args = json!({ "path": request.path.as_str() });
                            kept(reason(SOURCE_CHANGED, args), staging)
                        }
                        Some(staging) => self.discard_staging(staging, &mut step.trashing, why),
                        None => why,
                    };
                    items.push((request.at, Outcome::Failed(why)));
                }
            }
        }
        // ADR-0409 decision 5: judged again before the call's first rename — an overwrite's destination mount, and both
        //   mounts of a move across volumes (its staging's, its original's): a `.trash` broken since the call's check
        //   fails its items with nothing of them moved, a merge's free entries included.
        let mut checked: Vec<Placed> = Vec::with_capacity(placed.len());
        for item in placed {
            let staging = staged.get(&item.request.at);
            let mut unusable = None;
            if on_conflict == Some(OnConflict::Overwrite) || staging.is_some() {
                unusable = self.unusable_trash(&item.to.mount);
            }
            if unusable.is_none() && staging.is_some() {
                unusable = self.unusable_trash(&item.from.mount);
            }
            match (unusable, staging) {
                (None, _) => checked.push(item),
                (Some(why), None) => items.push((item.request.at, Outcome::Failed(why))),
                (Some(why), Some(staging)) => {
                    let why = self.discard_staging(staging, &mut step.trashing, why);
                    items.push((item.request.at, Outcome::Failed(why)));
                }
            }
        }
        let mut gone: Vec<Target> = Vec::new();
        let mut leaving: Vec<Target> = Vec::new();
        for item in &checked {
            let outcome = match staged.get(&item.request.at) {
                Some(staging) => {
                    self.move_staged(item, staging, on_conflict, catalog, mounts, &mut step)
                }
                None => self.move_item(item, on_conflict, catalog, mounts, &mut step),
            };
            if matches!(outcome, Outcome::Done(_)) {
                gone.extend(item.request.lone());
                leaving.extend(item.request.left());
            }
            items.push((item.request.at, outcome));
        }
        let Step {
            trashing,
            moves,
            walks,
            ..
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
            walks,
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

    /// Why a staged item cannot be placed in its hold, before its checks (`None` = it can): the mount registrations
    /// changed since its copy (ADR-0377 — the copy was made where the mounts were: [`TARGET_MOUNT_CHANGED`], it stays
    /// where it is, named), or its copy failed part-way (ADR-0409 M14 · ADR-0400 decision 25 — into the destination
    /// mount's trash, the answer naming the entry it stopped at: it and what follows were not copied).
    fn unready(
        &self,
        request: &Request,
        staging: &Staged,
        trashing: &mut Trashing<'_, '_>,
    ) -> Option<Reason> {
        if self.gate.numbers().root != staging.generation {
            let args = json!({ "path": request.path.as_str(), "to": request.dest.as_str() });
            return Some(kept(reason(TARGET_MOUNT_CHANGED, args), staging));
        }
        let Err((e, at)) = &staging.copy else {
            return None;
        };
        let args = json!({
            "path": request.path.as_str(),
            "to": request.dest.as_str(),
            "why": e.to_string(),
            "not_copied": below_item(&request.path, at),
        });
        Some(self.discard_staging(staging, trashing, reason(LIBRARY_WRITE_FAILED, args)))
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
        let to = self.destination(request, catalog, mounts)?;
        Ok(Placed {
            request: request.clone(),
            from: Side {
                mount: mount.clone(),
                rest,
                disk,
            },
            to,
            kind,
        })
    }

    /// The hold's checks of an item's destination (as [`LibraryService::place_request`] makes them for a move): its
    /// folder checked ([`LibraryService::destination_folder`]) and spelled as the catalog spells it when that is the
    /// same disk entry; the new name stays as given. Answers the destination side.
    fn destination(
        &self,
        request: &Request,
        catalog: &Catalog,
        mounts: &MountsNow,
    ) -> Result<Side, Reason> {
        let folder = request
            .dest
            .parent()
            .expect("a destination lies in a folder");
        let folder = match self.destination_folder(mounts, request)? {
            None => folder,
            Some(folder_disk) => {
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
        Ok(Side {
            disk: to_rest.to_path(&to_mount.folder),
            mount: to_mount.clone(),
            rest: to_rest,
        })
    }

    /// The link admission of `request`'s destination (ADR-0365) and its folder a real folder, checked name by name
    /// without following — absent or a file `not-found` (`{path}` = the folder), a link `path-through-link`, never made
    /// (ADR-0335 decision 7). Answers the folder's disk path below its mount's top (`None` = the mount's top, whose folder
    /// is there: the mount is usable).
    fn destination_folder(
        &self,
        mounts: &crate::mounts::Mounts,
        request: &Request,
    ) -> Result<Option<PathBuf>, Reason> {
        // ADR-0335 decision 7: a destination folder that is not there is not made.
        self.admit(mounts, &request.dest).map_err(reason_of)?;
        let folder = request
            .dest
            .parent()
            .expect("a destination lies in a folder");
        let Some((to_mount, folder_rest)) = mounts.locate(&folder) else {
            return Err(not_found_reason(folder.as_str()));
        };
        let Some(folder_rest) = folder_rest else {
            return Ok(None);
        };
        match self.store.check_names(&to_mount.folder, &folder_rest) {
            Ok(NameWalk::Present(EntryKind::Folder)) => {
                Ok(Some(folder_rest.to_path(&to_mount.folder)))
            }
            Ok(NameWalk::Present(EntryKind::Link) | NameWalk::Link(_)) => Err(reason(
                PATH_THROUGH_LINK,
                json!({ "path": folder.as_str() }),
            )),
            Ok(_) => Err(not_found_reason(folder.as_str())),
            Err(e) => Err(reason(
                LIBRARY_READ_FAILED,
                json!({ "path": folder.as_str(), "why": e.to_string() }),
            )),
        }
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
                original: None,
            }),
            Placement::Merged { .. } => Outcome::Done(Moved {
                to: item.to.lib().as_str().to_owned(),
                trashed: std::mem::take(&mut step.trashed),
                left: std::mem::take(&mut step.left),
                original: None,
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
        self.collide(from, to, kind, on_conflict, None, catalog, mounts, step)
    }

    /// A staging placed at `to` (of `kind`) as [`LibraryService::place`] places an entry — no pair, the staging's
    /// name is no library path —, except that a same-name folder there is not merged: the conflict `merge_refused`
    /// ([`CROSSES_VOLUMES`] for a move, [`COPY_MERGE_REFUSED`] for a copy).
    #[allow(clippy::too_many_arguments)]
    fn place_copy(
        &self,
        staging: &Side,
        to: &Side,
        kind: EntryKind,
        on_conflict: Option<OnConflict>,
        merge_refused: &str,
        catalog: &Catalog,
        mounts: &MountsNow,
        step: &mut Step<'_, '_>,
    ) -> Placement {
        match self.store.rename(&staging.disk, &to.disk) {
            Ok(()) => {
                return Placement::Moved {
                    to: to.clone(),
                    trashed: None,
                }
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Placement::Failed(rename_failed(&staging.lib(), &to.lib(), &e)),
        }
        let refused = Some(merge_refused);
        self.collide(
            staging,
            to,
            kind,
            on_conflict,
            refused,
            catalog,
            mounts,
            step,
        )
    }

    /// What [`LibraryService::place`] does when `to` is taken (its doc); `merge_refused` = `None`: a same-name folder
    /// there is merged into, else it is that conflict.
    #[allow(clippy::too_many_arguments)]
    fn collide(
        &self,
        from: &Side,
        to: &Side,
        kind: EntryKind,
        on_conflict: Option<OnConflict>,
        merge_refused: Option<&str>,
        catalog: &Catalog,
        mounts: &MountsNow,
        step: &mut Step<'_, '_>,
    ) -> Placement {
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
            _ if folder && onto_folder => match merge_refused {
                None => self.merge(from, to, choice, catalog, mounts, step),
                Some(code) => Placement::Conflict(reason(
                    code,
                    json!({ "path": from.lib().as_str(), "to": to.lib().as_str() }),
                )),
            },
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
                    "library: an overwritten entry stays in the trash: {back}"
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

    /// Moves one checked item across volumes (module header): its copy verified ([`LibraryService::verify`] — a change
    /// keeps both), its staging placed at the destination ([`LibraryService::place_copy`] — `on_conflict` as a
    /// same-volume move's, never a merge), then its original moved into its own mount's trash (ADR-0400 decision 25)
    /// and the pair recorded as a same-volume move's. A staging that does not take its place goes to the destination
    /// mount's trash ([`LibraryService::discard_staging`]) — it stays when an overwrite could not put the displaced
    /// entry back (the answer names both); an original that cannot be trashed leaves the copy in place too: no pair, the
    /// copy walked into the catalog after the hold, the answer naming both paths.
    #[allow(clippy::too_many_arguments)]
    fn move_staged(
        &self,
        item: &Placed,
        staged: &Staged,
        on_conflict: Option<OnConflict>,
        catalog: &Catalog,
        mounts: &MountsNow,
        step: &mut Step<'_, '_>,
    ) -> Outcome<Moved> {
        let Ok(copy) = &staged.copy else {
            unreachable!("a failed copy is answered before the item is placed");
        };
        let given = item.request.path.as_str();
        if step.moves.changed(&item.from.lib()) {
            let why = not_found_reason(given);
            return Outcome::Failed(self.discard_staging(staged, &mut step.trashing, why));
        }
        // ADR-0409 decision 12: any change keeps both.
        if let Err(why) = self.verify(&item.from, copy) {
            return Outcome::Failed(kept(why, staged));
        }
        let placed = self.place_copy(
            &staged.side(),
            &item.to,
            item.kind,
            on_conflict,
            CROSSES_VOLUMES,
            catalog,
            mounts,
            step,
        );
        let (to, trashed) = match self.settle_staging(placed, staged, given, &mut step.trashing) {
            Ok(placed) => placed,
            Err(outcome) => return outcome,
        };
        let from = &item.from;
        // ADR-0400 decision 25: the original goes into its own mount's trash.
        match step.trashing.put(
            &*self.store,
            &from.mount,
            &from.rest,
            &from.disk,
            &from.lib(),
        ) {
            Ok((original, _)) => {
                step.moves.moved(from, &to, item.kind, false);
                Outcome::Done(Moved {
                    to: to.lib().as_str().to_owned(),
                    trashed: trashed.into_iter().collect(),
                    left: Vec::new(),
                    original: Some(original),
                })
            }
            Err(mut why) => {
                // ADR-0409 M3: both stay — the copy's range enters the catalog by a walk and its folder is
                //   re-classified; the references stay on the original.
                tracing::warn!(
                    path = given,
                    "library move: original kept beside its copy: {}",
                    why.code
                );
                step.walks.push(to.lib());
                step.moves.folders.extend(to.lib().parent());
                why.args["copy"] = json!(to.lib().as_str());
                if let Some(row) = trashed {
                    why.args["trashed"] = address_json(&row);
                }
                Outcome::Failed(why)
            }
        }
    }

    /// A staging's placement as its item's answer: placed — the side it took and an overwrite's trash row —, or the
    /// item's outcome, the staging going into its destination mount's trash ([`LibraryService::discard_staging`];
    /// skipped: [`Skip::TakenDuringCopy`] names it), except when an overwrite could not put its displaced entry back:
    /// then the staging stays and both are named. A refusal about the staging names the item as `given`.
    fn settle_staging<T>(
        &self,
        placed: Placement,
        staged: &Staged,
        given: &str,
        trashing: &mut Trashing<'_, '_>,
    ) -> Result<(Side, Option<TrashAddress>), Outcome<T>> {
        let named = |mut why: Reason| {
            if why.args["path"] == json!(staged.lib().as_str()) {
                why.args["path"] = json!(given);
            }
            why
        };
        Err(match placed {
            Placement::Moved { to, trashed } => return Ok((to, trashed)),
            Placement::Skipped(at) => {
                let why = self.discard_staging(staged, trashing, name_taken(&at));
                let mut staging = why.args;
                if let Some(args) = staging.as_object_mut() {
                    args.remove("path");
                }
                Outcome::Skipped(Skip::TakenDuringCopy {
                    path: at.as_str().to_owned(),
                    staging,
                })
            }
            Placement::Conflict(why) => {
                Outcome::Conflict(self.discard_staging(staged, trashing, named(why)))
            }
            // `overwrite`: the displaced entry stays in the trash — the staging stays too, both named.
            Placement::Failed(why) if why.args.get("put_back").is_some() => {
                Outcome::Failed(kept(named(why), staged))
            }
            Placement::Failed(why) => {
                Outcome::Failed(self.discard_staging(staged, trashing, named(why)))
            }
            Placement::Merged { .. } => unreachable!("a staging is never merged"),
        })
    }

    /// Whether the item at `from` is still what `copy` copied (ADR-0409 decision 12): listed again without following
    /// and compared entry by entry — an entry added or removed, a kind or a tag changed —, then each file whose tag was
    /// racy when the copy listed it (the catalog's racy rule) hashed again and compared: only those can hide a same-size
    /// edit behind an equal tag; a file no longer the one listed (`LibraryStore::content_hash` — a link on the way, its
    /// name now a link or folder, another tag) is read not at all. `Err` = [`SOURCE_CHANGED`] (the item gone
    /// included), or a listing or hash that failed `library-read-failed`.
    fn verify(&self, from: &Side, copy: &TreeCopy) -> Result<(), Reason> {
        let path = from.lib();
        let changed = || reason(SOURCE_CHANGED, json!({ "path": path.as_str() }));
        // A name on the way that is no longer a real folder (`NotADirectory`) is a change too.
        let unread = |e: io::Error| match e.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => changed(),
            _ => reason(
                LIBRARY_READ_FAILED,
                json!({ "path": path.as_str(), "why": e.to_string() }),
            ),
        };
        let root = &from.mount.folder;
        let now = self.store.list_tree(root, &from.disk).map_err(unread)?;
        let same = now.len() == copy.entries.len()
            && now
                .iter()
                .zip(&copy.entries)
                .all(|(now, copied)| *now == copied.entry);
        if !same {
            return Err(changed());
        }
        for copied in &copy.entries {
            let (Some(tag), Some(hash)) = (copied.entry.tag, copied.hash) else {
                continue;
            };
            if !is_racy(&tag, copy.read_at) {
                continue;
            }
            let now = self
                .store
                .content_hash(root, &from.disk, &copied.entry.rel, &tag);
            if now.map_err(unread)? != Some(hash) {
                return Err(changed());
            }
        }
        Ok(())
    }

    /// Puts a staging (a move's across volumes, or a copy's) that does not take its place into its destination mount's
    /// trash as a staging row (ADR-0409 M14 — [`Trashing::put_staging`]) and names the row in `why` (`staging_row`); one that
    /// cannot go there (`trash-unusable`, `trash-not-reserved` …) stays, and `why` names it ([`kept`]) and the trash
    /// step's refusal (`staging_kept`).
    fn discard_staging(
        &self,
        staged: &Staged,
        trashing: &mut Trashing<'_, '_>,
        mut why: Reason,
    ) -> Reason {
        match trashing.put_staging(&*self.store, staged) {
            Ok((row, _)) => {
                why.args["staging_row"] = address_json(&row);
                why
            }
            Err(refused) => {
                tracing::warn!(
                    path = staged.lib().as_str(),
                    "library staging kept: {}",
                    refused.code
                );
                let mut why = kept(why, staged);
                why.args["staging_kept"] = json!({ "code": refused.code, "args": refused.args });
                why
            }
        }
    }

    /// A whole-call refusal of a hold with items copied before it — a move's across volumes, a copy's (module header):
    /// every staging goes into its destination mount's `.trash` as a staging row ([`LibraryService::discard_staging`])
    /// while that mount is still registered at the folder it was copied into — else it stays where it was made
    /// ([`kept`]) —, except a complete copy whose item is gone (a no-follow stat says `NotFound`), which stays too: it
    /// may be all that is left of it (ADR-0409 decision 12). `stagings` in the refusal's `args` names each, per item
    /// (`path` with `staging_row`, or `staging` · `staging_disk` · `staging_kept`). Answers the refusal and the mount
    /// folders whose `.trash` was made usable.
    // ADR-0409 M14
    fn refuse_staged(
        &self,
        refusal: LibraryRefusal,
        requests: &[Request],
        staged: &HashMap<usize, Staged>,
        mounts: &MountsNow,
        hold: &Hold,
    ) -> (LibraryRefusal, Vec<PathBuf>) {
        let LibraryRefusal::Refused { class, mut reason } = refusal else {
            unreachable!("a move's hold refuses with a reason");
        };
        let mut trashing = Trashing::new(hold);
        let mut named = Vec::new();
        for request in requests {
            let Some(staging) = staged.get(&request.at) else {
                continue;
            };
            let why = Reason {
                code: reason.code.clone(),
                args: json!({ "path": request.path.as_str() }),
                line: None,
            };
            let registered = mounts
                .locate(&staging.lib())
                .is_some_and(|(mount, _)| same_path(&mount.folder, &staging.mount.folder));
            let gone = staging.copy.is_ok()
                && self.admit(mounts, &request.path).is_ok_and(|disk| {
                    matches!(self.store.stat(&disk), Err(e) if e.kind() == io::ErrorKind::NotFound)
                });
            let why = match registered && !gone {
                true => self.discard_staging(staging, &mut trashing, why),
                false => kept(why, staging),
            };
            named.push(why.args);
        }
        reason.args["stagings"] = json!(named);
        let bundled = trashing.finish(&*self.store);
        (LibraryRefusal::Refused { class, reason }, bundled)
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
                    "library: file not re-read; the next scan does it: {e}"
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
                    "library: folder not re-classified; the next scan does it: {e}"
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

/// The path a copy may take and the name its copy takes (module header): a library path below a mount's top with no dot
/// or temp name — its own name —, or in the built-in mount one of its items — its file's name (`builtin::file_of`) — or
/// a virtual folder of it — its name; another path in the built-in mount `not-found` — or why not.
fn copyable(given: &str) -> Result<(LibPath, String), Reason> {
    // A copy reads its item: a built-in's spelling is one.
    let path = read_path(given).map_err(reason_of)?;
    // ADR-0400 decision 5: a mount's top, the built-in's included.
    if path.parent().is_none() {
        return Err(reason_of(virtual_root_read_only(Some(&path))));
    }
    // ADR-0400 decision 20: a copy takes items of the built-in mount, never a path it does not hold. The mount's name
    //   is matched in any case on every OS: re-spelled `engram`, so items given together nest alike everywhere.
    if let Some(inside) = builtin_rest(&path) {
        let path = LibPath::parse(&format!("{BUILTIN_MOUNT}/{inside}"))
            .expect("the built-in mount's name and a path inside it are a library path");
        let name = match AssetRef::parse(path.as_str()) {
            Ok(AssetRef::BuiltIn(id)) => builtin_file_name(id).to_owned(),
            _ if !builtins_in(inside).is_empty() => path.file_name().to_owned(),
            _ => return Err(not_found_reason(path.as_str())),
        };
        return Ok((path, name));
    }
    // ADR-0320: a path under `.trash` is a dot name.
    if !admits_inside(&path) {
        return Err(reason(
            ITEM_PATH_INVALID,
            json!({ "path": path.as_str(), "why": "a dot or temp name" }),
        ));
    }
    let name = path.file_name().to_owned();
    Ok((path, name))
}

/// The built-ins directly in the built-in mount's virtual folder `inside` (its path inside the mount, matched exactly);
/// none = no such folder.
// ADR-0400 decision 20
fn builtins_in(inside: &str) -> Vec<BuiltinId> {
    BuiltinId::ALL
        .into_iter()
        .filter(|id| id.id().rsplit_once('/').map(|(folder, _)| folder) == Some(inside))
        .collect()
}

/// The name of the built-in `id`'s file (`builtin::file_of`'s last name) — what its copy is named.
fn builtin_file_name(id: BuiltinId) -> &'static str {
    let file = file_of(id);
    file.rsplit_once('/').map_or(file, |(_, name)| name)
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
    suffixed(path, &format!(" ({n})"), file)
}

/// `path` with `suffix` added to its last name — before its last extension for a file, at the end for a folder or a
/// name without one.
fn suffixed(path: &MountPath, suffix: &str, file: bool) -> MountPath {
    let name = path.file_name();
    let renamed = match name
        .rsplit_once('.')
        .filter(|(stem, _)| file && !stem.is_empty())
    {
        Some((stem, extension)) => format!("{stem}{suffix}.{extension}"),
        None => format!("{name}{suffix}"),
    };
    let spelled = match path.parent() {
        Some(parent) => format!("{}/{renamed}", parent.as_str()),
        None => renamed,
    };
    MountPath::parse(&spelled).expect("a suffixed name of a valid name is valid")
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

/// The copy name `n` (1, 2 …) of an item copied into its own folder — `review - Copy.md`, then `review - Copy (2).md`
/// … — the one place a copy's naming is decided: English Explorer's, whatever the UI language (UD3).
// ADR-0409 decision 3
fn copy_name(path: &MountPath, n: u32, file: bool) -> MountPath {
    match n {
        1 => suffixed(path, " - Copy", file),
        n => suffixed(path, &format!(" - Copy ({n})"), file),
    }
}

fn into_itself(request: &Request) -> Reason {
    reason(
        INTO_ITSELF,
        json!({ "path": request.path.as_str(), "to": request.dest.as_str() }),
    )
}

/// What an item that is copied before its ticket answers, before any copy, when an entry of kind `there` is at its
/// destination (the item of kind `kind`) — as the hold would answer it: no `on_conflict` `name-exists`, a same-name
/// folder `merge_refused` (nothing is merged into one), skip skipped, a file onto a folder or the reverse under
/// overwrite [`KIND_MISMATCH`]; `None` = keep-both, or an overwrite of one kind: the copy and the hold go on.
fn taken_answer<T>(
    request: &Request,
    kind: EntryKind,
    there: EntryKind,
    on_conflict: Option<OnConflict>,
    merge_refused: &str,
) -> Option<Outcome<T>> {
    let (folder, onto_folder) = (kind == EntryKind::Folder, there == EntryKind::Folder);
    let args = json!({ "path": request.path.as_str(), "to": request.dest.as_str() });
    Some(match on_conflict {
        None => Outcome::Conflict(name_taken(&request.dest)),
        Some(_) if folder && onto_folder => Outcome::Conflict(reason(merge_refused, args)),
        Some(OnConflict::Skip) => Outcome::Skipped(Skip::Taken(request.dest.as_str().to_owned())),
        Some(OnConflict::Overwrite) if folder != onto_folder => {
            Outcome::Conflict(reason(KIND_MISMATCH, args))
        }
        Some(OnConflict::Overwrite | OnConflict::KeepBoth) => return None,
    })
}

/// `name-exists` naming what is at `to`.
fn name_taken(to: &LibPath) -> Reason {
    reason(NAME_EXISTS, json!({ "path": to.as_str() }))
}

/// A failed no-replace rename of `from` to `to` as an item's reason (a taken name is the caller's). Another volume
/// in the hold: the volume probe found one, so the mounts changed under it.
fn rename_failed(from: &LibPath, to: &LibPath, e: &io::Error) -> Reason {
    let args = json!({ "path": from.as_str(), "to": to.as_str(), "why": e.to_string() });
    match e.kind() {
        // ADR-0409 decision 13
        io::ErrorKind::CrossesDevices => reason(TARGET_MOUNT_CHANGED, args),
        io::ErrorKind::Unsupported => reason(NO_REPLACE_UNSUPPORTED, args),
        io::ErrorKind::NotFound => not_found_reason(from.as_str()),
        _ => reason(LIBRARY_WRITE_FAILED, args),
    }
}

/// `why` naming a staging (a move's across volumes, or a copy's) that stays where it was made ([`named_staging`]).
fn kept(why: Reason, staged: &Staged) -> Reason {
    named_staging(why, &staged.lib(), &staged.disk)
}

/// `why` naming a staging that stays where it was made: `staging` — its virtual path `lib` as made, which after a
/// mount change may name another place or none — and `staging_disk`, its disk path, which no mount change moves.
fn named_staging(mut why: Reason, lib: &LibPath, disk: &Path) -> Reason {
    why.args["staging"] = json!(lib.as_str());
    why.args["staging_disk"] = json!(escape_controls(&disk.to_string_lossy()).to_string());
    why
}

/// A trash row as an answer's `args` value.
pub(crate) fn address_json(row: &TrashAddress) -> Value {
    json!({ "mount": row.mount, "bundle": row.bundle, "path": row.path })
}

/// The virtual path of the tree entry `rel` below the item `path` (`path` itself for an empty `rel`), control
/// characters escaped.
fn below_item(path: &LibPath, rel: &Path) -> String {
    let names: Vec<String> = rel
        .components()
        .map(|name| name.as_os_str().to_string_lossy().into_owned())
        .collect();
    let spelled = match names.is_empty() {
        true => path.as_str().to_owned(),
        false => format!("{}/{}", path.as_str(), names.join("/")),
    };
    escape_controls(&spelled).to_string()
}

/// Whether a move's or a copy's hold takes the trash waiting count when it may touch `.trash` — always, outside tests
/// ([`UNRESERVED`]).
fn reserved() -> bool {
    #[cfg(test)]
    if UNRESERVED.with(|unreserved| unreserved.replace(false)) {
        return false;
    }
    true
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

pub(crate) fn reason(code: &str, args: Value) -> Reason {
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
    use engram_dashboard_base::file::is_temp_name;
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
    use crate::store::{DENY_FIRST_REMOVAL, REMOVABLE_ANSWERS};
    use crate::trash::{
        BundleOutcome, Emptied, EmptyAnswer, ListedBundle, RootState, RowKind, RowSource,
        TrashListing, RECORD_NAME, TRASH_UNUSABLE,
    };
    use crate::{
        LibraryValues, LIBRARY_UNAVAILABLE, MOUNT_READ_ONLY, ROOT_GENERATION_STALE,
        VIRTUAL_ROOT_READ_ONLY,
    };

    /// A stat that parks: it tells the sender, then waits for the receiver.
    type Door = (PathBuf, Sender<()>, Receiver<()>);

    /// What runs right after a rename from this path.
    type AfterRename = (PathBuf, Box<dyn FnOnce() + Send>);

    /// [`FsStore`] with seams: the record publication fails (a no-replace rename onto the record name) while
    /// `fail_record` is set, a write of a `fail_write` path fails, a rename from a `fail_renames` path fails after its
    /// passes, `after_rename` runs once right after a rename from its path, a `remove_dir` of a `fail_remove` path
    /// fails, the next stat of the door's path parks, every stat of `fail_stat`'s path fails, and `identity_of` `(at,
    /// as)` answers the identity of `as` for `at`; a rename into or out of the `volume` folder answers
    /// `CrossesDevices`, a rename from a base temp name fails as `fail_temp_rename` says after its passes,
    /// a rename of a base temp name out of the `cross_from` folder (a volume probe) answers `CrossesDevices`,
    /// `after_copy` runs once right after a tree copy, a tree copy answers a part-way failure while `fail_copy` is set and
    /// links while `links_copy` is, a `remove_dir` of a folder it made fails while `fail_remove_made` is set,
    /// hashing `fail_hash`'s path fails, `before_hash` runs once right before hashing its path, the next removal of the
    /// `remove_door`'s path parks; `identities` counts the
    /// identity probes, `lists` logs the folders listed, `made` the folders made, `copies` counts the tree copies,
    /// `hashes` logs the files hashed.
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
        /// Another volume: a rename from inside it to outside, or the reverse, answers `CrossesDevices`.
        volume: Mutex<Option<PathBuf>>,
        /// A rename from a base temp name fails once with this kind after `.0` such renames passed (a rename the
        ///   `volume` refuses first is not counted).
        fail_temp_rename: Mutex<Option<(u32, io::ErrorKind)>>,
        /// A rename of a base temp name out of this folder answers `CrossesDevices`.
        cross_from: Mutex<Option<PathBuf>>,
        made: Mutex<Vec<PathBuf>>,
        after_copy: Mutex<Option<Box<dyn FnOnce() + Send>>>,
        fail_copy: AtomicBool,
        copies: std::sync::atomic::AtomicUsize,
        hashes: Mutex<Vec<PathBuf>>,
        fail_hash: Mutex<Option<PathBuf>>,
        /// Runs once right before the file `.0` is hashed.
        before_hash: Mutex<Option<AfterRename>>,
        /// A tree copy answers links (its top) with nothing made.
        links_copy: AtomicBool,
        /// A `remove_dir` of a folder this store made (`made`) fails.
        fail_remove_made: AtomicBool,
        /// The next removal (`remove_tree`) of this path parks.
        remove_door: Mutex<Option<Door>>,
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
            lock(&self.made).push(path.to_path_buf());
            FsStore.create_dir(path)
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            if let Some(volume) = lock(&self.volume).as_ref() {
                if from.starts_with(volume) != to.starts_with(volume) {
                    return Err(io::Error::from(io::ErrorKind::CrossesDevices));
                }
            }
            let temp = from
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_temp_name);
            if temp && from.parent() == lock(&self.cross_from).as_deref() {
                return Err(io::Error::from(io::ErrorKind::CrossesDevices));
            }
            let mut failing = lock(&self.fail_temp_rename);
            if let Some((passes, kind)) = failing.as_mut().filter(|_| temp) {
                match passes {
                    0 => {
                        let kind = *kind;
                        *failing = None;
                        return Err(io::Error::new(kind, "injected temp rename failure"));
                    }
                    passes => *passes -= 1,
                }
            }
            drop(failing);
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
            if self.fail_remove_made.load(Ordering::SeqCst)
                && lock(&self.made).iter().any(|made| made == path)
            {
                return Err(io::Error::other("injected remove failure"));
            }
            FsStore.remove_dir(path)
        }
        fn remove_file(&self, path: &Path) -> io::Result<()> {
            FsStore.remove_file(path)
        }
        fn remove_tree(&self, root: &Path, path: &Path) -> io::Result<()> {
            let door = {
                let mut door = lock(&self.remove_door);
                match door.as_ref() {
                    Some((at, _, _)) if at == path => door.take(),
                    _ => None,
                }
            };
            if let Some((_, entered, release)) = door {
                entered.send(()).unwrap();
                let _ = release.recv_timeout(Duration::from_secs(20));
            }
            FsStore.remove_tree(root, path)
        }
        fn identity(&self, path: &Path) -> io::Result<engram_dashboard_platform::fs::FileIdentity> {
            self.identities.fetch_add(1, Ordering::SeqCst);
            match lock(&self.identity_of).as_ref() {
                Some((at, other)) if at == path => FsStore.identity(other),
                _ => FsStore.identity(path),
            }
        }
        fn copy_tree(&self, root: &Path, from: &Path, to: &Path) -> Result<TreeCopy, CopyFailure> {
            self.copies.fetch_add(1, Ordering::SeqCst);
            if self.links_copy.load(Ordering::SeqCst) {
                return Err(CopyFailure::Links(vec![PathBuf::new()]));
            }
            let copy = FsStore.copy_tree(root, from, to)?;
            let after = lock(&self.after_copy).take();
            if let Some(hook) = after {
                hook();
            }
            if self.fail_copy.load(Ordering::SeqCst) {
                let error = io::Error::other("injected copy failure");
                let at = copy.entries.last().expect("the top").entry.rel.clone();
                return Err(CopyFailure::Failed {
                    error,
                    made: true,
                    at,
                });
            }
            Ok(copy)
        }
        fn content_hash(
            &self,
            root: &Path,
            top: &Path,
            rel: &Path,
            listed: &ContentTag,
        ) -> io::Result<Option<crate::store::ContentHash>> {
            let path = crate::store::tree_path(top, rel);
            lock(&self.hashes).push(path.clone());
            if lock(&self.fail_hash).as_deref() == Some(path.as_path()) {
                return Err(io::Error::other("injected hash failure"));
            }
            let before = {
                let mut before = lock(&self.before_hash);
                match before.as_ref() {
                    Some((at, _)) if *at == path => before.take(),
                    _ => None,
                }
            };
            if let Some((_, hook)) = before {
                hook();
            }
            FsStore.content_hash(root, top, rel, listed)
        }
    }

    /// [`StdFolderOps`] that logs every path whose canonical path is asked — the overlap probes; `canonical_as` `(at,
    /// as)` answers the canonical path of `as` for `at` (an alias the spelling does not show), `no_identity` answers no
    /// identity and `fail_canonical` fails every canonical path.
    #[derive(Default)]
    struct Probes {
        canonical: Mutex<Vec<PathBuf>>,
        canonical_as: Mutex<Option<(PathBuf, PathBuf)>>,
        /// No file identity (a volume that gives none): every identity answers `Unsupported`.
        no_identity: AtomicBool,
        /// Every canonical path fails.
        fail_canonical: AtomicBool,
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
            if self.fail_canonical.load(Ordering::SeqCst) {
                return Err(io::Error::other("injected canonical failure"));
            }
            match lock(&self.canonical_as).as_ref() {
                Some((at, alias)) if at == p => StdFolderOps.canonical(alias),
                _ => StdFolderOps.canonical(p),
            }
        }
        fn identity(&self, p: &Path) -> io::Result<engram_dashboard_platform::fs::FileIdentity> {
            if self.no_identity.load(Ordering::SeqCst) {
                return Err(io::Error::from(io::ErrorKind::Unsupported));
            }
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
        /// `team`'s folder when it is on another volume ([`World::across`]).
        _elsewhere: Option<TempDir>,
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
            World::make_with(profiles, limit, None, None)
        }

        /// As [`World::new`], with the built-in folder `builtin`.
        fn with_builtin(builtin: &Path) -> World {
            World::make_with(Vec::new(), IO_LIMIT, None, Some(builtin))
        }

        /// As [`World::new`], with `team`'s folder in a fresh temp folder inside `elsewhere` (another volume) and `lib2`
        /// a link to it.
        fn across(elsewhere: &Path) -> World {
            let team = tempfile::tempdir_in(elsewhere).unwrap();
            World::make_with(Vec::new(), IO_LIMIT, Some(team), None)
        }

        fn make_with(
            profiles: Vec<AgentProfile>,
            limit: Duration,
            team: Option<TempDir>,
            builtin: Option<&Path>,
        ) -> World {
            let temp = tempfile::tempdir().unwrap();
            let base = dunce::canonicalize(temp.path()).unwrap();
            for dir in ["lib", "state", "keep"] {
                fs::create_dir_all(base.join(dir)).unwrap();
            }
            // Elsewhere: `lib2` is a link to it, so the test reaches it by the same names.
            let team_folder = match &team {
                Some(team) => {
                    let folder = dunce::canonicalize(team.path()).unwrap();
                    link_dir(&folder, &base.join("lib2")).unwrap();
                    folder
                }
                None => {
                    fs::create_dir_all(base.join("lib2")).unwrap();
                    base.join("lib2")
                }
            };
            let config = LibraryConfig::mounted(
                &base.join("state"),
                vec![
                    MountSetting {
                        name: MountName::parse("library").unwrap(),
                        place: MountPlace::Explicit(base.join("lib")),
                    },
                    MountSetting {
                        name: MountName::parse("team").unwrap(),
                        place: MountPlace::Explicit(team_folder),
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
                builtin_dir: builtin.map(Path::to_path_buf),
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
            let links = team.iter().map(|_| base.join("lib2")).collect();
            World {
                _temp: temp,
                base,
                links: Mutex::new(links),
                _elsewhere: team,
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

        /// [`World::mv`] with the client's `root_generation`.
        fn mv_at(
            &self,
            paths: &[&str],
            to: &str,
            generation: u64,
        ) -> Result<MoveAnswer, LibraryRefusal> {
            self.svc
                .move_items(paths, to, None, false, false, None, Some(generation))
        }

        fn cp(&self, paths: &[&str], to: &str, on_conflict: Option<OnConflict>) -> CopyAnswer {
            self.svc.copy_items(paths, to, on_conflict, None).unwrap()
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

        /// `team` (`lib2`) on another volume: a rename into or out of it answers `CrossesDevices`.
        fn cross(&self) {
            *lock(&self.disk.volume) = Some(self.at("lib2"));
        }

        /// The base temp names in the folder `rel`.
        fn temps(&self, rel: &str) -> Vec<String> {
            fs::read_dir(self.at(rel))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| is_temp_name(name))
                .collect()
        }

        /// The disk path of a staging left where it was made, as the answer names it (`staging_disk`), checked to be
        /// a base temp name below a `team` path as made (`staging`).
        fn staging(&self, args: &Value) -> PathBuf {
            let path = args["staging"].as_str().expect("the staging named");
            assert!(path.starts_with("team/"), "{path}");
            let disk = PathBuf::from(args["staging_disk"].as_str().expect("its disk path"));
            let name = disk.file_name().and_then(|name| name.to_str()).unwrap();
            assert!(
                is_temp_name(name) && path.ends_with(name),
                "{path} at {name}"
            );
            disk
        }

        /// The text of the file a staging named in `args` is (or of `rel` below the staging folder).
        fn staged_text(&self, args: &Value, rel: &str) -> String {
            let staging = self.staging(args);
            let file = match rel {
                "" => staging,
                rel => rel.split('/').fold(staging, |path, name| path.join(name)),
            };
            fs::read_to_string(file).unwrap()
        }

        /// Sets the file `rel`'s modified time to `then`.
        fn touch(&self, rel: &str, then: SystemTime) {
            fs::File::options()
                .write(true)
                .open(self.at(rel))
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(then))
                .unwrap();
        }

        fn modified(&self, rel: &str) -> SystemTime {
            fs::metadata(self.at(rel)).unwrap().modified().unwrap()
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
        assert_eq!(
            code, DELETE_UNCONFIRMED,
            "detach_refs without yes is no yes"
        );

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

    // ADR-0339 M105 · ADR-0409: detach_refs drops the lines of every deleted path (a folder: everything inside), leaves a
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

    // ADR-0409 (P5): a taken name without on_conflict is that item's conflict and nothing is replaced; a child given
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
    //   asks first with the plan; detach_refs without yes is no yes; plain yes keeps the references; yes + detach_refs +
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
        assert_eq!(code, SKILL_UNCONFIRMED, "detach_refs without yes is no yes");
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
                "detach_refs needs the plan's digest: {digest:?}"
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

    // ADR-0339 M108: detach_refs on a move with no lone SKILL.md leaving its skill is refused whole, nothing moved.
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
    //   and keeps the reference; yes + detach_refs + the digest moves and drops the line.
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

    // ── move across volumes ──

    /// The reason of a failed outcome.
    fn failure<T: std::fmt::Debug>(outcome: &Outcome<T>) -> Reason {
        match outcome {
            Outcome::Failed(why) => why.clone(),
            other => panic!("not failed: {other:?}"),
        }
    }

    /// The reason of a conflict.
    fn conflict<T: std::fmt::Debug>(outcome: &Outcome<T>) -> Reason {
        match outcome {
            Outcome::Conflict(why) => why.clone(),
            other => panic!("no conflict: {other:?}"),
        }
    }

    fn long_ago() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000)
    }

    // ADR-0409 decision 12 · ADR-0400 decision 25: an item whose destination is on another volume is copied whole — dot
    //   entries and names no library takes included, modified times kept —, then its original goes into its own mount's
    //   trash with a record, and the catalog and the references follow as for a move on one volume; no probe or staging
    //   is left.
    #[test]
    fn a_move_across_volumes_copies_every_entry_then_trashes_the_original_and_follows_like_one_volume(
    ) {
        let world = World::new();
        world.put("lib/d/a.md", "alpha");
        world.put("lib/d/.dot", "x");
        world.put("lib/d/e/notes.bin", "data");
        world.put("lib2/P.yaml", &preset("instructions:\n- library/d/a.md\n"));
        world.touch("lib/d/a.md", long_ago());
        world.scan();
        world.cross();

        let answer = world.mv(&["library/d"], "team", None);

        let moved = done(&answer.items[0].outcome);
        assert_eq!(moved.to, "team/d");
        let original = moved.original.expect("the original's trash row");
        assert_eq!(
            (original.mount.as_str(), original.path.as_str()),
            ("library", "d")
        );
        assert_eq!(world.text("lib2/d/a.md"), "alpha");
        assert_eq!(world.text("lib2/d/.dot"), "x");
        assert_eq!(world.text("lib2/d/e/notes.bin"), "data");
        assert_eq!(world.modified("lib2/d/a.md"), long_ago());
        assert!(!world.there("lib/d"));
        assert_eq!(
            world.text(&format!("lib/.trash/{}/d/a.md", original.bundle)),
            "alpha"
        );
        match read_record(
            &FsStore,
            &world.at("lib"),
            &BundleId::parse(&original.bundle).unwrap(),
        ) {
            RecordRead::Roots(roots) => {
                assert_eq!(roots.len(), 1);
                assert_eq!(roots[0].root.path.as_str(), "d");
                assert_eq!(roots[0].root.staging, None);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            answer.mapping,
            [("library/d".to_owned(), "team/d".to_owned(), false)]
        );
        assert!(world.text("lib2/P.yaml").contains("- team/d/a.md"));
        assert_eq!(world.kind("library/d/a.md"), None);
        assert!(world.kind("team/d/a.md").is_some());
        assert!(world.temps("lib").is_empty() && world.temps("lib2").is_empty());
    }

    // ADR-0365: an item holding a link is refused across volumes, naming the link — nothing copied or moved.
    #[test]
    fn a_move_across_volumes_refuses_an_item_holding_links_and_copies_nothing() {
        let world = World::new();
        world.put("lib/d/a.md", "a");
        world.put("keep/k.md", "k");
        world.link("keep", "lib/d/linked");
        world.scan();
        world.cross();

        let answer = world.mv(&["library/d"], "team", None);

        let why = failure(&answer.items[0].outcome);
        assert_eq!(why.code, LINKS_INSIDE);
        assert_eq!(why.args["links"], json!(["library/d/linked"]));
        assert_eq!(world.text("lib/d/a.md"), "a");
        assert_eq!(world.text("keep/k.md"), "k");
        assert!(!world.there("lib2/d"));
        assert!(world.temps("lib2").is_empty(), "no staging");
    }

    // ADR-0409 decision 12: an item changed between its copy and its hold keeps both — the item as it is now and the
    //   copy as it was made, named; nothing moves and the catalog stays.
    #[test]
    fn an_item_changed_during_its_copy_keeps_both_and_names_the_staging() {
        let world = World::new();
        world.put("lib/d/a.md", "a");
        world.put("lib/d/b.md", "b");
        world.scan();
        world.cross();
        let a = world.at("lib/d/a.md");
        *lock(&world.disk.after_copy) = Some(Box::new(move || fs::write(a, "changed").unwrap()));

        let answer = world.mv(&["library/d"], "team", None);

        let why = failure(&answer.items[0].outcome);
        assert_eq!(why.code, SOURCE_CHANGED);
        assert_eq!(
            world.staged_text(&why.args, "a.md"),
            "a",
            "the copy as made"
        );
        assert_eq!(world.text("lib/d/a.md"), "changed");
        assert!(!world.there("lib2/d"));
        assert!(world.kind("library/d/a.md").is_some());
        assert!(answer.mapping.is_empty());
    }

    // ADR-0409 decision 12: a same-size edit of a racy file that keeps its tag (its modified time put back) is found by
    //   its hash — the one check that can see it.
    #[test]
    fn a_same_size_edit_of_a_racy_file_keeping_its_tag_is_found_by_its_hash() {
        let world = World::new();
        world.put("lib/a.md", "aaaa");
        world.scan();
        world.cross();
        let a = world.at("lib/a.md");
        *lock(&world.disk.after_copy) = Some(Box::new(move || {
            let then = fs::metadata(&a).unwrap().modified().unwrap();
            fs::write(&a, "bbbb").unwrap();
            fs::File::options()
                .write(true)
                .open(&a)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(then))
                .unwrap();
        }));

        let answer = world.mv(&["library/a.md"], "team", None);

        let why = failure(&answer.items[0].outcome);
        assert_eq!(why.code, SOURCE_CHANGED);
        assert_eq!(*lock(&world.disk.hashes), [world.at("lib/a.md")]);
        assert_eq!(world.staged_text(&why.args, ""), "aaaa");
        assert_eq!(world.text("lib/a.md"), "bbbb");
    }

    // ADR-0409 decision 12 · R2: in the hold only a file racy at copy time is read again — settled files are not.
    #[test]
    fn only_the_files_racy_at_copy_time_are_read_again_in_the_hold() {
        let world = World::new();
        world.put("lib/d/old.md", "o");
        world.put("lib/d/e/old2.md", "o2");
        world.put("lib/d/e/fresh.md", "f");
        world.touch("lib/d/old.md", long_ago());
        world.touch("lib/d/e/old2.md", long_ago());
        world.scan();
        world.cross();

        let answer = world.mv(&["library/d"], "team", None);

        done(&answer.items[0].outcome);
        assert_eq!(*lock(&world.disk.hashes), [world.at("lib/d/e/fresh.md")]);
    }

    // ADR-0409 decision 12: a hash that cannot be read in the hold fails the item with both kept.
    #[test]
    fn a_hash_that_fails_in_the_hold_fails_the_item_and_keeps_both() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        world.cross();
        *lock(&world.disk.fail_hash) = Some(world.at("lib/a.md"));

        let answer = world.mv(&["library/a.md"], "team", None);

        let why = failure(&answer.items[0].outcome);
        assert_eq!(why.code, LIBRARY_READ_FAILED);
        assert_eq!(world.staged_text(&why.args, ""), "a");
        assert_eq!(world.text("lib/a.md"), "a");
        assert!(!world.there("lib2/a.md"));
    }

    // ADR-0409 M3: an original that cannot go into its trash in the hold leaves its copy in place too — walked into the
    //   catalog, no reference re-pointed, both paths answered.
    #[test]
    fn an_original_that_cannot_be_trashed_in_the_hold_keeps_its_copy_walked_in_and_no_reference_follows(
    ) {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("lib2/P.yaml", &preset("instructions:\n- library/a.md\n"));
        world.scan();
        world.cross();
        let p = world.text("lib2/P.yaml");
        lock(&world.disk.fail_renames).push((world.at("lib/a.md"), 0));

        let answer = world.mv(&["library/a.md"], "team", None);

        let why = failure(&answer.items[0].outcome);
        assert_eq!(why.code, LIBRARY_WRITE_FAILED);
        assert_eq!(why.args["copy"], "team/a.md");
        assert_eq!(world.text("lib2/a.md"), "a");
        assert_eq!(world.text("lib/a.md"), "a");
        assert!(world.kind("team/a.md").is_some(), "walked into the catalog");
        assert!(world.kind("library/a.md").is_some());
        assert!(answer.mapping.is_empty());
        assert_eq!(world.text("lib2/P.yaml"), p, "no reference re-pointed");
    }

    // ADR-0377 · ADR-0409 decision 12: a mount relocated during the copy keeps both — the copy stays where it was made.
    #[test]
    fn a_mount_relocated_during_the_copy_keeps_both() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        world.cross();
        let (svc, elsewhere) = (Arc::clone(&world.svc), world.at("lib3"));
        *lock(&world.disk.after_copy) = Some(Box::new(move || {
            svc.relocate_mount("team", &elsewhere, true).unwrap();
        }));

        let answer = world.mv(&["library/a.md"], "team", None);

        let why = failure(&answer.items[0].outcome);
        assert_eq!(why.code, TARGET_MOUNT_CHANGED);
        assert_eq!(world.staged_text(&why.args, ""), "a", "where it was made");
        assert_eq!(world.text("lib/a.md"), "a");
        assert!(!world.there("lib3/a.md") && !world.there("lib2/a.md"));
    }

    // ADR-0409 M14 · decision 8: a copy that fails part-way goes into the destination mount's trash as a staging row
    //   with its record, meant for the destination; the item stays.
    #[test]
    fn a_copy_that_fails_goes_into_the_destination_mounts_trash_as_a_staging_row() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        world.cross();
        world.disk.fail_copy.store(true, Ordering::SeqCst);

        let answer = world.mv(&["library/a.md"], "team", None);

        let why = failure(&answer.items[0].outcome);
        assert_eq!(why.code, LIBRARY_WRITE_FAILED);
        assert_eq!(
            why.args["not_copied"], "library/a.md",
            "where the copy stopped"
        );
        let row = &why.args["staging_row"];
        assert_eq!(row["mount"], "team");
        let (bundle, path) = (
            row["bundle"].as_str().unwrap(),
            row["path"].as_str().unwrap(),
        );
        assert!(is_temp_name(path), "{path}");
        assert_eq!(world.text(&format!("lib2/.trash/{bundle}/{path}")), "a");
        assert!(world.temps("lib2").is_empty());
        match read_record(
            &FsStore,
            &world.at("lib2"),
            &BundleId::parse(bundle).unwrap(),
        ) {
            RecordRead::Roots(roots) => {
                assert_eq!(roots[0].root.path.as_str(), path);
                let intended = roots[0].root.staging.as_ref().map(MountPath::as_str);
                assert_eq!(intended, Some("a.md"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(world.text("lib/a.md"), "a");
        assert!(!world.there("lib2/a.md"));
    }

    // ADR-0409 M14: a name that appears at the destination during the copy is the item's conflict, and its staging goes
    //   into the destination mount's trash.
    #[test]
    fn a_name_taken_during_the_copy_is_a_conflict_and_the_staging_goes_into_the_trash() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        world.cross();
        let there = world.at("lib2/a.md");
        *lock(&world.disk.after_copy) = Some(Box::new(move || fs::write(there, "other").unwrap()));

        let answer = world.mv(&["library/a.md"], "team", None);

        let why = conflict(&answer.items[0].outcome);
        assert_eq!(why.code, NAME_EXISTS);
        assert!(why.args["staging_row"].is_object(), "{}", why.args);
        assert_eq!(world.text("lib2/a.md"), "other");
        assert_eq!(world.text("lib/a.md"), "a");
        assert!(world.temps("lib2").is_empty());
    }

    // ADR-0339 decision 1 · ADR-0400 decision 25: an overwriting move across volumes puts the entry there into its
    //   mount's trash and the original into its own.
    #[test]
    fn an_overwriting_move_across_volumes_trashes_the_entry_there_and_the_original() {
        let world = World::new();
        world.put("lib/a.md", "new");
        world.put("lib2/a.md", "old");
        world.scan();
        world.cross();

        let answer = world.mv(&["library/a.md"], "team", Some(OnConflict::Overwrite));

        let moved = done(&answer.items[0].outcome);
        assert_eq!(moved.to, "team/a.md");
        let (displaced, original) = (&moved.trashed[0], moved.original.as_ref().unwrap());
        assert_eq!(
            (displaced.mount.as_str(), displaced.path.as_str()),
            ("team", "a.md")
        );
        assert_eq!(original.mount, "library");
        assert_eq!(world.text("lib2/a.md"), "new");
        assert_eq!(
            world.text(&format!("lib2/.trash/{}/a.md", displaced.bundle)),
            "old"
        );
        assert_eq!(
            world.text(&format!("lib/.trash/{}/a.md", original.bundle)),
            "new"
        );
        assert!(world.temps("lib2").is_empty());
    }

    // ADR-0339 decision 1: when the staging cannot take the overwritten name, the entry there comes back and the staging
    //   goes into the trash; when that fails too, the answer names the trash row and the staging, both kept.
    #[test]
    fn an_overwriting_move_across_volumes_whose_rename_fails_puts_the_entry_back_or_names_both() {
        let world = World::new();
        world.put("lib/a.md", "new");
        world.put("lib2/a.md", "old");
        world.scan();
        world.cross();
        // The staging's first rename (onto the taken name) passes; the next one, after the displacement, fails.
        *lock(&world.disk.fail_temp_rename) = Some((1, io::ErrorKind::Other));

        let answer = world.mv(&["library/a.md"], "team", Some(OnConflict::Overwrite));

        let why = failure(&answer.items[0].outcome);
        assert_eq!(why.code, LIBRARY_WRITE_FAILED);
        assert!(why.args["staging_row"].is_object(), "{}", why.args);
        assert_eq!(world.text("lib2/a.md"), "old", "put back");
        assert_eq!(world.text("lib/a.md"), "new");
        assert!(world.temps("lib2").is_empty());

        let id = BundleId::parse("20261123T081500Z-a1b2c3").unwrap();
        WANTED_BUNDLE.with(|wanted| *wanted.borrow_mut() = Some(id.clone()));
        let row = world.at(&format!("lib2/.trash/{}/a.md", id.as_str()));
        lock(&world.disk.fail_renames).push((row, 0));
        *lock(&world.disk.fail_temp_rename) = Some((1, io::ErrorKind::Other));

        let answer = world.mv(&["library/a.md"], "team", Some(OnConflict::Overwrite));

        let why = failure(&answer.items[0].outcome);
        assert_eq!(why.args["trashed"]["bundle"], id.as_str());
        assert!(why.args["put_back"].is_string());
        assert_eq!(world.staged_text(&why.args, ""), "new", "the staging kept");
        assert_eq!(
            world.text(&format!("lib2/.trash/{}/a.md", id.as_str())),
            "old"
        );
        assert_eq!(world.text("lib/a.md"), "new");
    }

    // ADR-0409 decision 13: a move the probe found on one volume whose rename in the hold answers another volume — the
    //   mounts changed under it — fails target-mount-changed, nothing moved.
    #[test]
    fn a_rename_in_the_hold_that_crosses_volumes_is_target_mount_changed() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        let (disk, team) = (Arc::clone(&world.disk), world.at("lib2"));
        BEFORE_HOLD.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || *lock(&disk.volume) = Some(team)));
        });

        let answer = world.mv(&["library/a.md"], "team", None);

        assert_eq!(failed_code(&answer.items[0].outcome), TARGET_MOUNT_CHANGED);
        assert_eq!(world.text("lib/a.md"), "a");
        assert!(!world.there("lib2/a.md"));
    }

    // ADR-0409 decision 12: a same-name folder on another volume is not merged into — that item's conflict, nothing
    //   copied; a taken name without on_conflict is answered before any copy too.
    #[test]
    fn a_move_across_volumes_onto_a_taken_name_answers_before_any_copy_and_never_merges() {
        let world = World::new();
        world.put("lib/d/a.md", "a");
        world.put("lib2/d/b.md", "b");
        world.put("lib/x.md", "x");
        world.put("lib2/x.md", "y");
        world.scan();
        world.cross();

        let answer = world.mv(&["library/d"], "team", Some(OnConflict::KeepBoth));
        assert_eq!(conflict(&answer.items[0].outcome).code, CROSSES_VOLUMES);
        let answer = world.mv(&["library/x.md"], "team", None);
        assert_eq!(conflict(&answer.items[0].outcome).code, NAME_EXISTS);

        assert_eq!(
            world.disk.copies.load(Ordering::SeqCst),
            0,
            "nothing copied"
        );
        assert!(!world.there("lib2/.trash"), "no staging made to trash");
        assert!(world.temps("lib2").is_empty());
        assert!(!world.there("lib2/d/a.md") && world.there("lib/d/a.md"));
        assert_eq!(world.text("lib2/x.md"), "y");
    }

    // ADR-0409 decision 13: one volume probe per (item's folder, destination folder) pair and none for a rename in its
    //   own folder; on one volume nothing is left of it.
    #[test]
    fn the_volume_probe_runs_once_per_folder_pair_leaves_nothing_and_is_skipped_for_a_rename() {
        let world = World::new();
        world.put("lib/x/a.md", "a");
        world.put("lib/x/b.md", "b");
        world.put("lib/y/c.md", "c");
        fs::create_dir_all(world.at("lib/z")).unwrap();
        world.scan();
        let probes = |world: &World| {
            lock(&world.disk.made)
                .iter()
                .filter(|made| {
                    made.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(is_temp_name)
                })
                .count()
        };

        let paths = ["library/x/a.md", "library/x/b.md", "library/y/c.md"];
        let answer = world.mv(&paths, "library/z", None);

        for item in &answer.items {
            done(&item.outcome);
        }
        assert_eq!(probes(&world), 2);
        for folder in ["lib/x", "lib/y", "lib/z"] {
            assert!(world.temps(folder).is_empty(), "{folder}");
        }
        done(&world.rename("library/z/a.md", "e.md").items[0].outcome);
        assert_eq!(probes(&world), 2, "none for a rename");
        assert_eq!(world.text("lib/z/e.md"), "a");
    }

    // ADR-0409 decision 13: a probe that fails other than by another volume fails its item with the code; the probe is
    //   removed beside the item.
    #[test]
    fn a_volume_probe_that_fails_fails_its_item_and_leaves_nothing() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        *lock(&world.disk.fail_temp_rename) = Some((0, io::ErrorKind::Other));

        let answer = world.mv(&["library/a.md"], "team", None);

        assert_eq!(failed_code(&answer.items[0].outcome), LIBRARY_WRITE_FAILED);
        assert_eq!(world.text("lib/a.md"), "a");
        assert!(world.temps("lib").is_empty() && world.temps("lib2").is_empty());
        assert_eq!(world.svc.gate.trash().waiting(), 0);
    }

    // ADR-0409 decision 5: across volumes the destination mount's trash (a failed copy goes there) and the item's own
    //   (its original goes there) must be usable before any copy.
    #[test]
    fn an_unusable_trash_refuses_a_move_across_volumes_before_any_copy() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("lib2/.trash", "a file");
        world.scan();
        world.cross();

        let answer = world.mv(&["library/a.md"], "team", None);
        assert_eq!(failed_code(&answer.items[0].outcome), TRASH_UNUSABLE);
        assert!(world.temps("lib2").is_empty());

        fs::remove_file(world.at("lib2/.trash")).unwrap();
        world.put("lib/.trash", "a file");
        let answer = world.mv(&["library/a.md"], "team", None);
        assert_eq!(failed_code(&answer.items[0].outcome), TRASH_UNUSABLE);
        assert!(world.temps("lib2").is_empty());
        assert!(!world.there("lib2/.trash"), "no staging made to trash");
        assert_eq!(
            world.disk.copies.load(Ordering::SeqCst),
            0,
            "nothing copied"
        );
        assert_eq!(world.text("lib/a.md"), "a");
    }

    // ADR-0409 decision 14: a move across volumes raises the waiting count after its probe and copy (the staging is
    //   there before the count), just before its ticket, and lowers it after the hold; one refused before its ticket
    //   leaves it at 0.
    #[test]
    fn a_move_across_volumes_raises_the_waiting_count_after_its_copy_just_before_its_ticket() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        world.cross();
        let trash = || world.svc.gate.trash();
        let (at_hook, reached) = mpsc::channel::<(usize, usize)>();
        let (go, proceed) = mpsc::channel::<()>();
        let (svc, team) = (Arc::clone(&world.svc), world.at("lib2"));
        let worker = on_thread(&world.svc, move |on| {
            BEFORE_HOLD.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    let staged = fs::read_dir(&team)
                        .unwrap()
                        .filter(|entry| {
                            is_temp_name(&entry.as_ref().unwrap().file_name().to_string_lossy())
                        })
                        .count();
                    at_hook.send((svc.gate.trash().waiting(), staged)).unwrap();
                    proceed.recv_timeout(Duration::from_secs(20)).unwrap();
                }));
            });
            on.move_items(&["library/a.md"], "team", None, false, false, None, None)
                .unwrap()
        });

        let (before, staged) = reached.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(before, 0, "not raised before the ticket");
        assert_eq!(staged, 1, "copied before the count");
        let guard = world.svc.gate.lib().write().unwrap();
        go.send(()).unwrap();
        trash().wait_until("the move waits at its write lock", |view| view.waiting == 1);
        assert!(!worker.is_finished());
        drop(guard);
        let answer = worker.join().unwrap();
        done(&answer.items[0].outcome);
        assert_eq!(trash().waiting(), 0, "lowered after the hold");

        world.put("lib/d/b.md", "b");
        world.put("keep/k.md", "k");
        world.link("keep", "lib/d/linked");
        let answer = world.mv(&["library/d"], "team", None);
        assert_eq!(failed_code(&answer.items[0].outcome), LINKS_INSIDE);
        assert_eq!(trash().waiting(), 0, "never raised");
    }

    // ADR-0409 decision 14: a hold that did not raise the waiting count and meets a trash step — a failed copy's staging
    //   — never waits on the trash lock (an emptying holds it here): the staging stays where it is, named.
    #[test]
    fn an_unreserved_hold_leaves_a_failed_copys_staging_in_place_without_waiting() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        world.cross();
        world.disk.fail_copy.store(true, Ordering::SeqCst);
        let emptying = world.svc.gate.trash().take_for_emptying();
        let (sent, answered) = mpsc::channel();
        let _worker = on_thread(&world.svc, move |on| {
            UNRESERVED.with(|unreserved| unreserved.set(true));
            let answer = on.move_items(&["library/a.md"], "team", None, false, false, None, None);
            sent.send(answer.unwrap()).unwrap();
        });

        let answer = answered
            .recv_timeout(Duration::from_secs(10))
            .expect("the move does not wait on the trash lock");
        drop(emptying);

        let why = failure(&answer.items[0].outcome);
        assert_eq!(why.args["staging_kept"]["code"], TRASH_NOT_RESERVED);
        assert_eq!(world.staged_text(&why.args, ""), "a");
        assert_eq!(world.text("lib/a.md"), "a");
    }

    // ADR-0409 decision 12: an entry added to an item whose files are all settled is found by the listing compare
    //   alone — both kept, nothing hashed.
    #[test]
    fn an_entry_added_to_a_settled_item_during_its_copy_is_found_by_the_listing() {
        let world = World::new();
        world.put("lib/d/a.md", "a");
        world.put("lib/d/b.md", "b");
        world.touch("lib/d/a.md", long_ago());
        world.touch("lib/d/b.md", long_ago());
        world.scan();
        world.cross();
        let new = world.at("lib/d/new.md");
        *lock(&world.disk.after_copy) = Some(Box::new(move || fs::write(new, "n").unwrap()));

        let answer = world.mv(&["library/d"], "team", None);

        let why = failure(&answer.items[0].outcome);
        assert_eq!(why.code, SOURCE_CHANGED);
        assert_eq!(world.staged_text(&why.args, "a.md"), "a");
        assert!(lock(&world.disk.hashes).is_empty(), "no file re-read");
        assert!(world.there("lib/d/new.md") && !world.there("lib2/d"));
    }

    // ADR-0409 decision 12: a settled file rewritten to another size with its modified time put back is found by the
    //   listing compare (its tag's size) — it is not racy, so no hash would see it.
    #[test]
    fn a_settled_file_rewritten_to_another_size_with_its_time_set_back_is_found_by_the_listing() {
        let world = World::new();
        world.put("lib/a.md", "aaaa");
        world.touch("lib/a.md", long_ago());
        world.scan();
        world.cross();
        let a = world.at("lib/a.md");
        *lock(&world.disk.after_copy) = Some(Box::new(move || {
            fs::write(&a, "bb").unwrap();
            fs::File::options()
                .write(true)
                .open(&a)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(long_ago()))
                .unwrap();
        }));

        let answer = world.mv(&["library/a.md"], "team", None);

        let why = failure(&answer.items[0].outcome);
        assert_eq!(why.code, SOURCE_CHANGED);
        assert_eq!(world.staged_text(&why.args, ""), "aaaa");
        assert!(lock(&world.disk.hashes).is_empty(), "no file re-read");
        assert_eq!(world.text("lib/a.md"), "bb");
    }

    // ADR-0365 · ADR-0409 decision 12: a racy file's re-hash never reads through a folder of the item swapped for a link
    //   after the hold's listing — even one whose target holds the same bytes: both kept.
    #[test]
    fn a_rehash_reads_nothing_through_a_folder_swapped_for_a_link_and_keeps_both() {
        let world = World::new();
        world.put("lib/d/sub/a.md", "a");
        world.put("keep/sub/a.md", "a");
        world.scan();
        world.cross();
        let (sub, target) = (world.at("lib/d/sub"), world.at("keep/sub"));
        lock(&world.links).push(sub.clone());
        *lock(&world.disk.before_hash) = Some((
            world.at("lib/d/sub/a.md"),
            Box::new(move || {
                fs::rename(&sub, sub.with_file_name("moved")).unwrap();
                link_dir(&target, &sub).unwrap();
            }),
        ));

        let answer = world.mv(&["library/d"], "team", None);

        let why = failure(&answer.items[0].outcome);
        assert_eq!(why.code, SOURCE_CHANGED);
        assert_eq!(world.staged_text(&why.args, "sub/a.md"), "a");
        assert!(!world.there("lib2/d"));
        assert_eq!(world.text("lib/d/moved/a.md"), "a");
    }

    // ADR-0409 decision 12: an item removed whole after its copy is a change, not an absence — its complete copy stays
    //   where it was made, named, and nothing goes into a trash.
    #[test]
    fn an_item_removed_after_its_copy_keeps_the_copy_where_it_was_made() {
        let world = World::new();
        world.put("lib/d/a.md", "a");
        world.put("lib/d/e/b.md", "b");
        world.scan();
        world.cross();
        let d = world.at("lib/d");
        *lock(&world.disk.after_copy) = Some(Box::new(move || fs::remove_dir_all(d).unwrap()));

        let answer = world.mv(&["library/d"], "team", None);

        let why = failure(&answer.items[0].outcome);
        assert_eq!(why.code, SOURCE_CHANGED);
        assert_eq!(world.staged_text(&why.args, "a.md"), "a");
        assert_eq!(world.staged_text(&why.args, "e/b.md"), "b");
        assert!(why.args.get("staging_row").is_none(), "{}", why.args);
        assert!(!world.there("lib2/.trash") && !world.there("lib2/d"));
    }

    // ADR-0377 · ADR-0409 M14: a client's root generation gone stale during the copy is judged in the hold — the call
    //   is refused naming its staging: in its mount's trash as a staging row while that mount is where the copy went,
    //   else kept where it was made.
    #[test]
    fn a_root_generation_gone_stale_during_the_copy_refuses_naming_the_staging() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        world.cross();
        let (svc, other) = (Arc::clone(&world.svc), world.at("lib3"));
        *lock(&world.disk.after_copy) = Some(Box::new(move || {
            svc.add_mount("other", &other, true).unwrap();
        }));

        let (code, args) =
            refused(world.mv_at(&["library/a.md"], "team", world.svc.gate.numbers().root));

        assert_eq!(code, ROOT_GENERATION_STALE);
        let staging = &args["stagings"][0];
        assert_eq!(staging["path"], "library/a.md");
        let row = &staging["staging_row"];
        assert_eq!(row["mount"], "team", "{args}");
        let (bundle, path) = (
            row["bundle"].as_str().unwrap(),
            row["path"].as_str().unwrap(),
        );
        assert_eq!(world.text(&format!("lib2/.trash/{bundle}/{path}")), "a");
        assert!(world.temps("lib2").is_empty());
        assert_eq!(world.text("lib/a.md"), "a");
        assert!(!world.there("lib2/a.md"));

        let (svc, elsewhere) = (Arc::clone(&world.svc), world.at("lib4"));
        *lock(&world.disk.after_copy) = Some(Box::new(move || {
            svc.relocate_mount("team", &elsewhere, true).unwrap();
        }));
        let (code, args) =
            refused(world.mv_at(&["library/a.md"], "team", world.svc.gate.numbers().root));

        assert_eq!(code, ROOT_GENERATION_STALE);
        let staging = &args["stagings"][0];
        assert!(staging.get("staging_row").is_none(), "{args}");
        assert_eq!(
            world.staged_text(staging, ""),
            "a",
            "kept where it was made"
        );
        assert_eq!(world.text("lib/a.md"), "a");
    }

    // ADR-0409 decision 12 · M14: a refusal of the whole call in the hold keeps a complete staging whose item went away
    //   during the copy — it may be all that is left of it — and names it by its disk path; nothing goes to a trash.
    #[test]
    fn a_whole_call_refusal_keeps_a_complete_staging_whose_item_is_gone() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        world.cross();
        let (svc, other, a) = (
            Arc::clone(&world.svc),
            world.at("lib3"),
            world.at("lib/a.md"),
        );
        *lock(&world.disk.after_copy) = Some(Box::new(move || {
            fs::remove_file(a).unwrap();
            svc.add_mount("other", &other, true).unwrap();
        }));

        let generation = world.svc.gate.numbers().root;
        let (code, args) = refused(world.mv_at(&["library/a.md"], "team", generation));

        assert_eq!(code, ROOT_GENERATION_STALE);
        let staging = &args["stagings"][0];
        assert!(staging.get("staging_row").is_none(), "{args}");
        assert_eq!(
            world.staged_text(staging, ""),
            "a",
            "kept where it was made"
        );
        assert!(!world.there("lib2/.trash"));
    }

    // ADR-0409 decision 2 · M14: a SKILL.md judgement that fails in the hold after the copy refuses the call with the
    //   fresh plan, its staging in the destination mount's trash as a staging row and named.
    #[test]
    fn a_skill_md_judged_again_in_the_hold_after_its_copy_names_its_trashed_staging() {
        let world = World::new();
        world.put("lib/s/SKILL.md", "---\nname: s\ndescription: d\n---\n");
        world.put("lib/S.yaml", &preset(""));
        world.scan();
        world.cross();
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
            "team",
            None,
            false,
            false,
            None,
            None,
        ));

        assert_eq!(code, SKILL_UNCONFIRMED);
        assert_eq!(fresh["plan"]["presets"], json!(["library/S.yaml"]));
        let row = &fresh["stagings"][0]["staging_row"];
        let (bundle, path) = (
            row["bundle"].as_str().unwrap(),
            row["path"].as_str().unwrap(),
        );
        assert!(world
            .text(&format!("lib2/.trash/{bundle}/{path}"))
            .contains("name: s"));
        assert!(world.temps("lib2").is_empty());
        assert!(world.there("lib/s/SKILL.md") && !world.there("lib2/SKILL.md"));
    }

    // Names near the 255-unit limit: the probe and the staging are named from short bases, so a 250-unit name moves
    //   across volumes like any other.
    #[test]
    fn an_item_whose_name_is_near_the_length_limit_moves_across_volumes() {
        let world = World::new();
        let name = format!("{}.md", "a".repeat(247));
        world.put(&format!("lib/{name}"), "long");
        world.scan();
        world.cross();

        let answer = world.mv(&[&format!("library/{name}")], "team", None);

        assert_eq!(done(&answer.items[0].outcome).to, format!("team/{name}"));
        assert_eq!(world.text(&format!("lib2/{name}")), "long");
        assert!(world.temps("lib").is_empty() && world.temps("lib2").is_empty());
    }

    // ADR-0409 decision 12: keep both across volumes places the copy under the next free name, then trashes the
    //   original; the entry there stays.
    #[test]
    fn keep_both_across_volumes_places_the_copy_under_the_next_name() {
        let world = World::new();
        world.put("lib/a.md", "new");
        world.put("lib2/a.md", "old");
        world.scan();
        world.cross();

        let answer = world.mv(&["library/a.md"], "team", Some(OnConflict::KeepBoth));

        let moved = done(&answer.items[0].outcome);
        assert_eq!(moved.to, "team/a (2).md");
        assert!(moved.original.is_some());
        assert_eq!(world.text("lib2/a (2).md"), "new");
        assert_eq!(world.text("lib2/a.md"), "old");
        assert!(!world.there("lib/a.md"));
        assert_eq!(
            answer.mapping,
            [("library/a.md".to_owned(), "team/a (2).md".to_owned(), false)]
        );
    }

    // ADR-0409 M14: a name that appears during the copy under skip skips the item and names its staging, which went into
    //   the destination mount's trash.
    #[test]
    fn a_name_taken_during_the_copy_under_skip_skips_naming_the_trashed_staging() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        world.cross();
        let there = world.at("lib2/a.md");
        *lock(&world.disk.after_copy) = Some(Box::new(move || fs::write(there, "other").unwrap()));

        let answer = world.mv(&["library/a.md"], "team", Some(OnConflict::Skip));

        let Outcome::Skipped(Skip::TakenDuringCopy { path, staging }) = &answer.items[0].outcome
        else {
            panic!("{:?}", answer.items[0].outcome);
        };
        assert_eq!(path, "team/a.md");
        let row = &staging["staging_row"];
        let (bundle, rest) = (
            row["bundle"].as_str().unwrap(),
            row["path"].as_str().unwrap(),
        );
        assert_eq!(world.text(&format!("lib2/.trash/{bundle}/{rest}")), "a");
        assert_eq!(world.text("lib2/a.md"), "other");
        assert_eq!(world.text("lib/a.md"), "a");
        assert!(world.temps("lib2").is_empty());
    }

    // ADR-0409 decision 5: the item's own mount's trash broken between the copy and the hold fails the item before any
    //   rename — its staging into the destination mount's trash, nothing placed.
    #[test]
    fn an_original_trash_broken_before_the_hold_fails_the_item_and_trashes_its_staging() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        world.cross();
        let trash = world.at("lib/.trash");
        BEFORE_HOLD.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || fs::write(trash, "a file").unwrap()));
        });

        let answer = world.mv(&["library/a.md"], "team", None);

        let why = failure(&answer.items[0].outcome);
        assert_eq!(why.code, TRASH_UNUSABLE);
        assert!(why.args["staging_row"].is_object(), "{}", why.args);
        assert!(!world.there("lib2/a.md"), "nothing placed");
        assert_eq!(world.text("lib/a.md"), "a");
        assert!(world.temps("lib2").is_empty());
    }

    // ADR-0409: a staged item whose source an earlier item of the call displaced fails not-found, its staging into the
    //   trash as a staging row; the earlier item stands.
    #[test]
    fn a_staged_item_whose_source_an_earlier_item_replaced_fails_not_found() {
        let world = World::new();
        world.put("lib/p/d/e.md", "p");
        world.put("lib/t/d/e.md", "t");
        world.scan();
        // Only the second item's probe (out of lib/t/d) answers another volume.
        *lock(&world.disk.cross_from) = Some(world.at("lib/t/d"));

        let answer = world.mv(
            &["library/p/d", "library/t/d/e.md"],
            "library/t",
            Some(OnConflict::Overwrite),
        );

        assert_eq!(done(&answer.items[0].outcome).trashed.len(), 1);
        let why = failure(&answer.items[1].outcome);
        assert_eq!(why.code, NOT_FOUND);
        assert!(why.args["staging_row"].is_object(), "{}", why.args);
        assert_eq!(
            world.disk.copies.load(Ordering::SeqCst),
            1,
            "the second item was copied"
        );
        assert_eq!(world.text("lib/t/d/e.md"), "p");
        assert!(!world.there("lib/t/e.md"));
        assert!(world.temps("lib/t").is_empty());
    }

    // ADR-0409 decisions 12 and 13 on two real volumes: the probe answers another volume, and the move copies, checks
    //   and trashes the original. ENGRAM_SECOND_VOLUME names a folder on another volume than the temp folder.
    #[test]
    #[ignore = "needs a folder on a second volume in ENGRAM_SECOND_VOLUME"]
    fn a_move_between_two_real_volumes_copies_and_trashes_the_original() {
        let elsewhere = std::env::var("ENGRAM_SECOND_VOLUME").expect("a folder on a second volume");
        let world = World::across(Path::new(&elsewhere));
        world.put("lib/d/a.md", "alpha");
        world.put("lib/d/e/b.md", "bee");
        world.scan();

        let answer = world.mv(&["library/d"], "team", None);

        let moved = done(&answer.items[0].outcome);
        assert!(moved.original.is_some());
        assert_eq!(world.text("lib2/d/e/b.md"), "bee");
        assert!(!world.there("lib/d"));
        assert!(world.kind("team/d/a.md").is_some());
        assert!(world.temps("lib").is_empty() && world.temps("lib2").is_empty());
    }

    // ── copy ──

    // ADR-0409 decision 12 · M3: a copy — across mounts and inside one — stages every entry (dot entries and names no
    //   library takes included, modified times kept), places it, and the catalog has it right after the call; the
    //   original stays and no reference, agent or trash is touched.
    #[test]
    fn a_copy_stages_every_entry_places_it_and_walks_it_in_following_no_reference() {
        let world = World::new();
        world.put("lib/d/a.md", "alpha");
        world.put("lib/d/.dot", "x");
        world.put("lib/d/e/notes.bin", "data");
        let listing = preset("instructions:\n- library/d/a.md\n");
        world.put("lib2/P.yaml", &listing);
        world.touch("lib/d/a.md", long_ago());
        world.scan();
        let change = world.svc.gate.numbers().change;

        let answer = world.cp(&["library/d"], "team", None);

        let copied = done(&answer.items[0].outcome);
        assert!(
            world.svc.gate.numbers().change > change,
            "the hold raised the change number"
        );
        assert_eq!((copied.to.as_str(), copied.trashed), ("team/d", None));
        assert_eq!(world.text("lib2/d/a.md"), "alpha");
        assert_eq!(world.text("lib2/d/.dot"), "x");
        assert_eq!(world.text("lib2/d/e/notes.bin"), "data");
        assert_eq!(world.modified("lib2/d/a.md"), long_ago());
        assert_eq!(world.text("lib/d/a.md"), "alpha", "the original stays");
        assert!(
            world.kind("team/d/a.md").is_some(),
            "walked in before the answer"
        );
        assert!(world.kind("library/d/a.md").is_some());
        assert_eq!(world.text("lib2/P.yaml"), listing, "no reference follows");
        assert!(lock(&world.port.renamed).is_empty(), "no agent step");
        assert!(!world.there("lib2/.trash") && !world.there("lib/.trash"));
        assert!(world.temps("lib").is_empty() && world.temps("lib2").is_empty());

        let answer = world.cp(&["library/d/a.md"], "library", None);

        assert_eq!(done(&answer.items[0].outcome).to, "library/a.md");
        assert_eq!(world.text("lib/a.md"), "alpha");
        assert!(world.kind("library/a.md").is_some());
        assert!(world.temps("lib").is_empty() && world.temps("lib/d").is_empty());
    }

    // ADR-0409 decision 3 (UD3): a copy into its item's own folder is named ` - Copy`, then ` - Copy (2)` — before a
    //   file's extension, after a folder's name — whatever `on_conflict` says; the item itself is never replaced.
    #[test]
    fn a_copy_into_its_own_folder_is_named_copy_then_copy_2_whatever_on_conflict_says() {
        let world = World::new();
        world.put("lib/r.md", "r");
        world.put("lib/d/x.md", "x");
        world.scan();

        let first = world.cp(&["library/r.md"], "library", None);
        let second = world.cp(&["library/r.md"], "library", Some(OnConflict::Overwrite));
        let folder = world.cp(&["library/d"], "library", Some(OnConflict::KeepBoth));

        assert_eq!(done(&first.items[0].outcome).to, "library/r - Copy.md");
        assert_eq!(done(&second.items[0].outcome).to, "library/r - Copy (2).md");
        assert_eq!(done(&folder.items[0].outcome).to, "library/d - Copy");
        assert_eq!(world.text("lib/r.md"), "r");
        assert_eq!(world.text("lib/r - Copy.md"), "r");
        assert_eq!(world.text("lib/r - Copy (2).md"), "r");
        assert_eq!(world.text("lib/d - Copy/x.md"), "x");
        assert!(world.kind("library/r - Copy (2).md").is_some());
        assert!(world.kind("library/d - Copy/x.md").is_some());
        assert!(!world.there("lib/.trash"), "nothing overwritten");
        assert!(world.temps("lib").is_empty());
    }

    // ADR-0409: a copy into its item itself or into a folder below it is refused `into-itself` before anything is
    //   copied.
    #[test]
    fn a_copy_into_its_own_descendant_is_refused_into_itself_and_copies_nothing() {
        let world = World::new();
        world.put("lib/d/e/x.md", "x");
        world.scan();

        let into = world.cp(&["library/d"], "library/d", None);
        let below = world.cp(&["library/d"], "library/d/e", Some(OnConflict::KeepBoth));

        for answer in [&into, &below] {
            let why = failure(&answer.items[0].outcome);
            assert_eq!(why.code, INTO_ITSELF, "{why:?}");
        }
        assert_eq!(world.disk.copies.load(Ordering::SeqCst), 0);
        assert!(world.temps("lib/d").is_empty() && world.temps("lib/d/e").is_empty());
        assert!(!world.there("lib/d/d") && !world.there("lib/d/e/d"));
    }

    // ADR-0409 decision 10: a copy never replaces a taken name unless told to overwrite — a taken name answers before
    //   any copy (conflict, skip, a kind mismatch, a same-name folder never merged), keep-both takes the next name.
    #[test]
    fn a_copy_never_overwrites_and_answers_a_taken_name_before_copying() {
        let world = World::new();
        world.put("lib/a.md", "new");
        world.put("lib2/a.md", "old");
        world.put("lib/d/x.md", "x");
        world.put("lib2/d/y.md", "y");
        world.put("lib/k.md", "k");
        world.put("lib2/k.md/inner.md", "folder");
        world.scan();

        let none = world.cp(&["library/a.md"], "team", None);
        let skip = world.cp(&["library/a.md"], "team", Some(OnConflict::Skip));
        let merge = world.cp(&["library/d"], "team", Some(OnConflict::Overwrite));
        let mixed = world.cp(&["library/k.md"], "team", Some(OnConflict::Overwrite));

        assert_eq!(conflict(&none.items[0].outcome).code, NAME_EXISTS);
        assert_eq!(
            skip.items[0].outcome,
            Outcome::Skipped(Skip::Taken("team/a.md".to_owned()))
        );
        assert_eq!(conflict(&merge.items[0].outcome).code, COPY_MERGE_REFUSED);
        assert_eq!(conflict(&mixed.items[0].outcome).code, KIND_MISMATCH);
        assert_eq!(
            world.disk.copies.load(Ordering::SeqCst),
            0,
            "nothing copied"
        );
        assert_eq!(world.text("lib2/a.md"), "old");
        assert!(!world.there("lib2/d/x.md"), "never merged");

        let kept = world.cp(&["library/a.md"], "team", Some(OnConflict::KeepBoth));

        assert_eq!(done(&kept.items[0].outcome).to, "team/a (2).md");
        assert_eq!(world.text("lib2/a (2).md"), "new");
        assert_eq!(world.text("lib2/a.md"), "old");
        assert!(world.temps("lib2").is_empty() && !world.there("lib2/.trash"));
    }

    // ADR-0339 decision 1: an overwriting copy puts the entry there into its mount's trash, then takes its name; the
    //   original stays.
    #[test]
    fn an_overwriting_copy_puts_the_entry_there_into_its_trash_and_takes_its_name() {
        let world = World::new();
        world.put("lib/a.md", "new");
        world.put("lib2/a.md", "old");
        world.scan();

        let answer = world.cp(&["library/a.md"], "team", Some(OnConflict::Overwrite));

        let copied = done(&answer.items[0].outcome);
        assert_eq!(copied.to, "team/a.md");
        let row = copied.trashed.expect("the displaced entry's row");
        assert_eq!((row.mount.as_str(), row.path.as_str()), ("team", "a.md"));
        assert_eq!(
            world.text(&format!("lib2/.trash/{}/a.md", row.bundle)),
            "old"
        );
        assert_eq!(world.text("lib2/a.md"), "new");
        assert_eq!(world.text("lib/a.md"), "new");
        assert!(world.kind("team/a.md").is_some());
        assert!(world.temps("lib2").is_empty());
    }

    // ADR-0409 decision 10 · M14: a name that appears at the destination during the copy is never replaced — the item's
    //   conflict, its staging into the destination mount's trash; under skip it is skipped naming the staging.
    #[test]
    fn a_name_taken_during_a_copy_is_never_replaced_and_the_staging_goes_into_the_trash() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        let there = world.at("lib2/a.md");
        *lock(&world.disk.after_copy) = Some(Box::new(move || fs::write(there, "other").unwrap()));

        let answer = world.cp(&["library/a.md"], "team", None);

        let why = conflict(&answer.items[0].outcome);
        assert_eq!(why.code, NAME_EXISTS);
        assert_eq!(why.args["path"], "team/a.md");
        assert!(why.args["staging_row"].is_object(), "{}", why.args);
        assert_eq!(world.text("lib2/a.md"), "other");

        world.put("lib/b.md", "b");
        let there = world.at("lib2/b.md");
        *lock(&world.disk.after_copy) = Some(Box::new(move || fs::write(there, "other").unwrap()));
        let answer = world.cp(&["library/b.md"], "team", Some(OnConflict::Skip));

        match &answer.items[0].outcome {
            Outcome::Skipped(Skip::TakenDuringCopy { path, staging }) => {
                assert_eq!(path, "team/b.md");
                assert!(staging["staging_row"].is_object(), "{staging}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(world.text("lib2/b.md"), "other");
        assert_eq!(world.text("lib/a.md"), "a");
        assert!(world.temps("lib2").is_empty());
    }

    // ADR-0409 decision 12: a copy compares nothing with its source — an edit after the copy leaves the copy as made,
    //   and nothing is hashed again in the hold.
    #[test]
    fn a_copy_places_what_it_copied_without_comparing_the_source_again() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        let source = world.at("lib/a.md");
        *lock(&world.disk.after_copy) = Some(Box::new(move || {
            fs::write(source, "changed later").unwrap()
        }));

        let answer = world.cp(&["library/a.md"], "team", None);

        assert_eq!(done(&answer.items[0].outcome).to, "team/a.md");
        assert_eq!(world.text("lib2/a.md"), "a");
        assert_eq!(world.text("lib/a.md"), "changed later");
        assert!(lock(&world.disk.hashes).is_empty(), "no source compare");
    }

    // ADR-0400 decision 20: from the built-in mount a copy takes its items — each its file's name — and `engram/settings`
    //   as a folder holding its item's file (the mount's name in any case); `engram` itself is a mount's top, another
    //   path there is not found, nothing is copied into it, and a built-in mount that cannot be read refuses.
    #[test]
    fn a_copy_from_the_builtin_takes_its_items_files_and_the_settings_folder() {
        let builtin = crate::builtin::tests::full();
        let world = World::with_builtin(builtin.path());
        world.scan();

        let answer = world.cp(
            &[
                "engram/default",
                "engram/priming",
                "engram/settings",
                "Engram/settings/default",
            ],
            "library",
            None,
        );

        let to: Vec<String> = answer.items[..3]
            .iter()
            .map(|item| done(&item.outcome).to)
            .collect();
        assert_eq!(
            to,
            [
                "library/default.yaml",
                "library/priming.md",
                "library/settings"
            ]
        );
        assert_eq!(
            answer.items[3].outcome,
            Outcome::Skipped(Skip::Inside("engram/settings".to_owned()))
        );
        assert_eq!(
            world.text("lib/default.yaml"),
            crate::builtin::tests::DEFAULT_PRESET
        );
        assert_eq!(world.text("lib/priming.md"), crate::builtin::tests::PRIMING);
        assert_eq!(
            world.text("lib/settings/default.yaml"),
            crate::builtin::tests::DEFAULT_SETTINGS
        );
        let answer = world.cp(&["Engram/settings/default"], "team", None);
        assert_eq!(done(&answer.items[0].outcome).to, "team/default.yaml");
        assert_eq!(
            world.text("lib2/default.yaml"),
            crate::builtin::tests::DEFAULT_SETTINGS
        );
        assert_eq!(world.kind("library/default.yaml"), Some(ItemKind::Preset));
        assert_eq!(
            world.kind("library/settings/default.yaml"),
            Some(ItemKind::Settings)
        );
        assert!(world.temps("lib").is_empty() && world.temps("lib/settings").is_empty());

        let answer = world.cp(
            &["engram", "engram/nope", "engram/settings/nope"],
            "team",
            None,
        );

        let codes: Vec<String> = answer
            .items
            .iter()
            .map(|item| failed_code(&item.outcome))
            .collect();
        assert_eq!(codes, [VIRTUAL_ROOT_READ_ONLY, NOT_FOUND, NOT_FOUND]);
        for to in ["engram", "engram/settings"] {
            let (code, _) = refused(
                world
                    .svc
                    .copy_items(&["library/priming.md"], to, None, None),
            );
            assert_eq!(code, MOUNT_READ_ONLY);
        }

        let without = World::new();
        without.scan();
        let answer = without.cp(&["engram/default"], "library", None);
        assert_eq!(failed_code(&answer.items[0].outcome), LIBRARY_UNAVAILABLE);
        assert!(!without.there("lib/default.yaml"));
    }

    // ADR-0409 decision 14: every copy — on one volume and across volumes — raises the trash waiting count after its copy,
    //   just before its ticket, and lowers it after the hold; a copy refused before its ticket never raises it.
    #[test]
    fn every_copy_raises_the_waiting_count_after_its_copy_just_before_its_ticket() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("lib/x/keep.md", "k");
        world.scan();
        world.cross();
        let trash = || world.svc.gate.trash();
        for (to, folder) in [("library/x", "lib/x"), ("team", "lib2")] {
            let (at_hook, reached) = mpsc::channel::<(usize, usize)>();
            let (go, proceed) = mpsc::channel::<()>();
            let (svc, into) = (Arc::clone(&world.svc), world.at(folder));
            let worker = on_thread(&world.svc, move |on| {
                BEFORE_HOLD.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move || {
                        let staged = fs::read_dir(&into)
                            .unwrap()
                            .filter(|entry| {
                                is_temp_name(&entry.as_ref().unwrap().file_name().to_string_lossy())
                            })
                            .count();
                        at_hook.send((svc.gate.trash().waiting(), staged)).unwrap();
                        proceed.recv_timeout(Duration::from_secs(20)).unwrap();
                    }));
                });
                on.copy_items(&["library/a.md"], to, None, None).unwrap()
            });

            let (before, staged) = reached.recv_timeout(Duration::from_secs(10)).unwrap();
            assert_eq!(before, 0, "{to}: not raised before the ticket");
            assert_eq!(staged, 1, "{to}: copied before the count");
            let guard = world.svc.gate.lib().write().unwrap();
            go.send(()).unwrap();
            trash().wait_until("the copy waits at its write lock", |view| view.waiting == 1);
            assert!(!worker.is_finished());
            drop(guard);
            let answer = worker.join().unwrap();
            done(&answer.items[0].outcome);
            assert_eq!(trash().waiting(), 0, "{to}: lowered after the hold");
        }

        world.put("lib/d/b.md", "b");
        world.put("keep/k.md", "k");
        world.link("keep", "lib/d/linked");
        let answer = world.cp(&["library/d", "library/a.md"], "library/x", None);
        assert_eq!(failed_code(&answer.items[0].outcome), LINKS_INSIDE);
        assert_eq!(conflict(&answer.items[1].outcome).code, NAME_EXISTS);
        assert_eq!(trash().waiting(), 0, "never raised");
    }

    // ADR-0409 M14 · ADR-0400 decision 25: a copy that fails part-way goes into the destination mount's trash as a
    //   staging row meant for its name, naming where it stopped; nothing is placed.
    #[test]
    fn a_failed_copy_goes_into_the_destination_mounts_trash_as_a_staging_row_meant_for_its_name() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        world.disk.fail_copy.store(true, Ordering::SeqCst);

        let answer = world.cp(&["library/a.md"], "team", None);

        let why = failure(&answer.items[0].outcome);
        assert_eq!(why.code, LIBRARY_WRITE_FAILED);
        assert_eq!(why.args["not_copied"], "library/a.md");
        let row = &why.args["staging_row"];
        assert_eq!(row["mount"], "team");
        let (bundle, path) = (
            row["bundle"].as_str().unwrap(),
            row["path"].as_str().unwrap(),
        );
        assert_eq!(world.text(&format!("lib2/.trash/{bundle}/{path}")), "a");
        match read_record(
            &FsStore,
            &world.at("lib2"),
            &BundleId::parse(bundle).unwrap(),
        ) {
            RecordRead::Roots(roots) => {
                let intended = roots[0].root.staging.as_ref().map(MountPath::as_str);
                assert_eq!(intended, Some("a.md"));
            }
            other => panic!("{other:?}"),
        }
        assert!(!world.there("lib2/a.md"));
        assert!(world.temps("lib2").is_empty());
        assert_eq!(world.text("lib/a.md"), "a");
    }

    // ADR-0365 · ADR-0409 decision 5: a copy is refused per item by the entrance rules, an item holding links (named) and
    //   a destination trash that cannot be used — before anything is copied; a parent and its child given together copy
    //   the parent only.
    #[test]
    fn a_copy_refuses_per_item_by_the_entrance_links_and_an_unusable_trash_copying_nothing() {
        let world = World::new();
        world.put("lib/d/x.md", "x");
        world.put("lib/l/a.md", "a");
        world.put("keep/k.md", "k");
        world.link("keep", "lib/l/linked");
        world.put("lib2/.trash", "not a folder");
        world.scan();

        let answer = world.cp(
            &[
                "library",
                "library/.trash/x.md",
                "library/none.md",
                "library/l",
            ],
            "library/d",
            None,
        );

        let codes: Vec<String> = answer
            .items
            .iter()
            .map(|item| failed_code(&item.outcome))
            .collect();
        assert_eq!(
            codes,
            [
                VIRTUAL_ROOT_READ_ONLY,
                ITEM_PATH_INVALID,
                NOT_FOUND,
                LINKS_INSIDE
            ]
        );
        assert_eq!(
            failure(&answer.items[3].outcome).args["links"],
            json!(["library/l/linked"])
        );
        let copies = world.disk.copies.load(Ordering::SeqCst);
        let unusable = world.cp(&["library/d/x.md"], "team", None);
        assert_eq!(failed_code(&unusable.items[0].outcome), TRASH_UNUSABLE);
        assert_eq!(
            world.disk.copies.load(Ordering::SeqCst),
            copies,
            "nothing copied"
        );
        assert!(world.temps("lib/d").is_empty() && world.temps("lib2").is_empty());

        let nested = world.cp(&["library/d", "library/d/x.md"], "library/l", None);
        assert_eq!(done(&nested.items[0].outcome).to, "library/l/d");
        assert_eq!(
            nested.items[1].outcome,
            Outcome::Skipped(Skip::Inside("library/d".to_owned()))
        );
    }

    // ADR-0377 · ADR-0409 M14: a stale root generation refuses a copy before anything is copied; one gone stale during
    //   the copy refuses in the hold, naming the staging it put into the destination mount's trash.
    #[test]
    fn a_stale_root_generation_refuses_a_copy_before_copying_or_in_its_hold_naming_the_staging() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        let now = world.svc.gate.numbers().root;

        let (code, _) = refused(world.svc.copy_items(
            &["library/a.md"],
            "team",
            None,
            Some(now + 1),
        ));
        assert_eq!(code, ROOT_GENERATION_STALE);
        assert_eq!(world.disk.copies.load(Ordering::SeqCst), 0);

        let (svc, other) = (Arc::clone(&world.svc), world.at("lib3"));
        *lock(&world.disk.after_copy) = Some(Box::new(move || {
            svc.add_mount("other", &other, true).unwrap();
        }));
        let (code, args) = refused(world.svc.copy_items(
            &["library/a.md"],
            "team",
            None,
            Some(now),
        ));

        assert_eq!(code, ROOT_GENERATION_STALE);
        let row = &args["stagings"][0]["staging_row"];
        assert_eq!(row["mount"], "team", "{args}");
        let (bundle, path) = (
            row["bundle"].as_str().unwrap(),
            row["path"].as_str().unwrap(),
        );
        assert_eq!(world.text(&format!("lib2/.trash/{bundle}/{path}")), "a");
        assert!(!world.there("lib2/a.md"));
        assert!(world.temps("lib2").is_empty());
    }

    // ADR-0365: a built-in's file is reached from the built-in folder down without following — a folder on the way that
    //   is a link refuses the item `path-through-link` before anything is stat'ed or copied through it.
    #[test]
    fn a_builtin_reached_through_a_link_is_refused_and_nothing_is_copied_through_it() {
        let builtin = crate::builtin::tests::full();
        let world = World::with_builtin(builtin.path());
        world.put("keep/settings/default.yaml", "kind: settings\nversion: 1\n");
        let settings = builtin.path().join("settings");
        fs::remove_dir_all(&settings).unwrap();
        link_dir(&world.at("keep/settings"), &settings).unwrap();
        lock(&world.links).push(settings);
        world.scan();

        let item = world.cp(&["engram/settings/default"], "team", None);
        let folder = world.cp(&["engram/settings"], "team", None);
        let other = world.cp(&["engram/default"], "team", None);

        assert_eq!(failed_code(&item.items[0].outcome), PATH_THROUGH_LINK);
        assert_eq!(failed_code(&folder.items[0].outcome), PATH_THROUGH_LINK);
        assert_eq!(
            world.disk.copies.load(Ordering::SeqCst),
            1,
            "only engram/default copied"
        );
        assert_eq!(done(&other.items[0].outcome).to, "team/default.yaml");
        assert!(!world.there("lib2/settings"));
        assert!(world.temps("lib2").is_empty());
    }

    // ADR-0335 decision 7: a destination folder that is missing, a file or a link is answered before anything is copied,
    //   naming the folder — never as the item.
    #[test]
    fn a_copy_into_a_missing_file_or_linked_folder_is_refused_naming_it_before_copying() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("lib/f.md", "f");
        world.put("keep/k.md", "k");
        world.link("keep", "lib/j");
        world.scan();

        let missing = world.cp(&["library/a.md"], "library/none", None);
        let file = world.cp(&["library/a.md"], "library/f.md", None);
        let linked = world.cp(&["library/a.md"], "library/j", None);

        let why = failure(&missing.items[0].outcome);
        assert_eq!(
            (why.code.as_str(), &why.args["path"]),
            (NOT_FOUND, &json!("library/none"))
        );
        let why = failure(&file.items[0].outcome);
        assert_eq!(
            (why.code.as_str(), &why.args["path"]),
            (NOT_FOUND, &json!("library/f.md"))
        );
        assert_eq!(failed_code(&linked.items[0].outcome), PATH_THROUGH_LINK);
        assert_eq!(world.disk.copies.load(Ordering::SeqCst), 0);
        assert!(!world.there("keep/a.md"));
        assert!(world.temps("lib").is_empty());
    }

    // ADR-0409: a staging a built-in folder's copy made and could not remove after meeting a link stays, named by its
    //   virtual and its disk path; one it removed is not named.
    #[test]
    fn a_builtin_folder_staging_left_after_links_is_named() {
        let builtin = crate::builtin::tests::full();
        let world = World::with_builtin(builtin.path());
        world.scan();
        world.disk.links_copy.store(true, Ordering::SeqCst);

        let removed = world.cp(&["engram/settings"], "library", None);

        let why = failure(&removed.items[0].outcome);
        assert_eq!(why.code, LINKS_INSIDE);
        assert_eq!(why.args["links"], json!(["engram/settings/default"]));
        assert!(why.args.get("staging").is_none(), "{}", why.args);
        assert!(world.temps("lib").is_empty());

        world.disk.fail_remove_made.store(true, Ordering::SeqCst);
        let left = world.cp(&["engram/settings"], "library", None);

        let why = failure(&left.items[0].outcome);
        assert_eq!(why.code, LINKS_INSIDE);
        let disk = PathBuf::from(why.args["staging_disk"].as_str().expect("named"));
        assert!(disk.is_dir(), "{}", why.args);
        let name = disk.file_name().unwrap().to_str().unwrap().to_owned();
        assert_eq!(why.args["staging"], json!(format!("library/{name}")));
        assert_eq!(world.temps("lib"), [name]);
    }

    // ADR-0409 decision 3: a copy into its own folder spelled another way — an alias of the folder that is the same
    //   disk entry (a short name, where the volume makes one) — still takes a copy name, never overwriting the item.
    #[test]
    fn a_copy_into_an_alias_of_its_own_folder_takes_a_copy_name() {
        let world = World::new();
        world.put("lib/longfoldername/x.md", "x");
        world.scan();
        let alias = world.at("lib/LONGFO~1");
        let same = matches!(
            (FsStore.identity(&alias), FsStore.identity(&world.at("lib/longfoldername"))),
            (Ok(a), Ok(b)) if a == b
        );
        if !same {
            eprintln!("no short-name alias on this volume; the alias case is not exercised");
            return;
        }

        let answer = world.cp(
            &["library/longfoldername/x.md"],
            "library/LONGFO~1",
            Some(OnConflict::Overwrite),
        );

        assert!(done(&answer.items[0].outcome).to.ends_with("/x - Copy.md"));
        let none = world.cp(&["library/longfoldername/x.md"], "library/LONGFO~1", None);
        assert!(
            done(&none.items[0].outcome)
                .to
                .ends_with("/x - Copy (2).md"),
            "never name-exists"
        );
        world.probes.no_identity.store(true, Ordering::SeqCst);
        let unidentified = world.cp(
            &["library/longfoldername/x.md"],
            "library/LONGFO~1",
            Some(OnConflict::Overwrite),
        );
        assert!(done(&unidentified.items[0].outcome)
            .to
            .ends_with("/x - Copy (3).md"));
        assert_eq!(world.text("lib/longfoldername/x.md"), "x");
        assert_eq!(world.text("lib/longfoldername/x - Copy.md"), "x");
        assert!(!world.there("lib/.trash"), "the item was not overwritten");
    }

    // ADR-0409: the canonical into-itself probe runs for a copy too — a destination folder that is the item's own
    //   descendant under another spelling (an alias neither spelling shows) is refused with nothing copied.
    #[test]
    fn a_copy_into_an_alias_of_its_own_descendant_is_refused_by_the_canonical_probe() {
        let world = World::new();
        world.put("lib/d/e/x.md", "x");
        world.put("lib/f/y.md", "y");
        world.scan();
        *lock(&world.probes.canonical_as) = Some((world.at("lib/f"), world.at("lib/d/e")));

        let answer = world.cp(&["library/d"], "library/f", None);

        assert_eq!(failed_code(&answer.items[0].outcome), INTO_ITSELF);
        assert_eq!(world.disk.copies.load(Ordering::SeqCst), 0);
        assert!(!world.there("lib/f/d"));
    }

    // ADR-0409 decision 3: whether a copy goes into its item's own folder under another spelling is judged by identity,
    //   where the volume gives none by canonical path, and when neither answers it counts as its own folder — the copy
    //   takes a copy name and nothing is overwritten; a folder the canonical path tells apart is overwritten as asked.
    #[test]
    fn without_a_file_identity_the_own_folder_is_judged_by_canonical_path_or_kept_safe() {
        let world = World::new();
        world.put("lib/d/x.md", "item");
        world.put("lib/f/x.md", "there");
        world.scan();
        world.probes.no_identity.store(true, Ordering::SeqCst);

        // `lib/f` an alias of `lib/d` that neither spelling shows: its own folder.
        *lock(&world.probes.canonical_as) = Some((world.at("lib/f"), world.at("lib/d")));
        let alias = world.cp(
            &["library/d/x.md"],
            "library/f",
            Some(OnConflict::Overwrite),
        );
        assert_eq!(done(&alias.items[0].outcome).to, "library/f/x - Copy.md");
        assert_eq!(world.text("lib/f/x.md"), "there");

        // Nothing answers: kept safe as its own folder.
        *lock(&world.probes.canonical_as) = None;
        world.probes.fail_canonical.store(true, Ordering::SeqCst);
        let unknown = world.cp(
            &["library/d/x.md"],
            "library/f",
            Some(OnConflict::Overwrite),
        );
        assert_eq!(
            done(&unknown.items[0].outcome).to,
            "library/f/x - Copy (2).md"
        );
        assert_eq!(world.text("lib/f/x.md"), "there");
        assert!(!world.there("lib/.trash"));

        // The canonical path tells the folders apart: the overwrite goes as asked.
        world.probes.fail_canonical.store(false, Ordering::SeqCst);
        let other = world.cp(
            &["library/d/x.md"],
            "library/f",
            Some(OnConflict::Overwrite),
        );
        let copied = done(&other.items[0].outcome);
        assert_eq!(copied.to, "library/f/x.md");
        assert!(copied.trashed.is_some());
        assert_eq!(world.text("lib/f/x.md"), "item");
        assert_eq!(world.text("lib/d/x.md"), "item");
    }

    // ADR-0377: the generation a copy's hold compares is taken before its judgements — a mount relocated right after
    //   them (where a spelling might alias the item's own folder) stops the item in the hold: nothing is overwritten
    //   or placed, the staging kept and named.
    #[test]
    fn a_mount_changed_after_a_copys_judgements_stops_it_in_the_hold() {
        let world = World::new();
        world.put("lib/a.md", "new");
        world.put("lib2/a.md", "old");
        world.scan();
        let (svc, elsewhere) = (Arc::clone(&world.svc), world.at("lib4"));
        let worker = on_thread(&world.svc, move |on| {
            AFTER_JUDGEMENT.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    svc.relocate_mount("team", &elsewhere, true).unwrap();
                }));
            });
            on.copy_items(&["library/a.md"], "team", Some(OnConflict::Overwrite), None)
                .unwrap()
        });

        let answer = worker.join().unwrap();

        let why = failure(&answer.items[0].outcome);
        assert_eq!(why.code, TARGET_MOUNT_CHANGED, "{why:?}");
        assert!(why.args["staging_disk"].is_string(), "{}", why.args);
        assert_eq!(world.text("lib/a.md"), "new");
        assert_eq!(world.text("lib2/a.md"), "old");
        assert!(!world.there("lib4/a.md"), "nothing placed after the change");
        assert!(!world.there("lib2/.trash") && !world.there("lib4/.trash"));
    }

    // ── emptying and permanent delete on a real disk (ADR-0365 · ADR-0408 · ADR-0409 decisions 5, 6, 9 and 11) ──

    const BUNDLE: &str = "20261123T081500Z-a1b2c3";
    const SECOND: &str = "20261123T081600Z-d4e5f6";
    const THIRD: &str = "20261123T081700Z-0a0b0c";

    impl World {
        /// The record of the bundle `.trash/<bundle>` in the mount folder `mount` (`lib` · `lib2`) naming `roots`.
        fn record(&self, mount: &str, bundle: &str, roots: &[&str]) {
            let roots: Vec<Value> = roots.iter().map(|path| json!({ "path": path })).collect();
            self.put(
                &format!("{mount}/.trash/{bundle}/{RECORD_NAME}"),
                &json!({ "roots": roots }).to_string(),
            );
        }

        fn read_only(&self, rel: &str, on: bool) {
            let path = self.at(rel);
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(on);
            fs::set_permissions(&path, permissions).unwrap();
        }

        /// Every entry below `keep` without following — its path, whether it is a folder, its read-only attribute
        /// and a file's text — by path.
        fn keep(&self) -> Vec<(String, bool, bool, String)> {
            let mut entries = Vec::new();
            let mut folders = vec![self.at("keep")];
            while let Some(folder) = folders.pop() {
                for entry in fs::read_dir(&folder).unwrap() {
                    let path = entry.unwrap().path();
                    let meta = fs::symlink_metadata(&path).unwrap();
                    let rel = path
                        .strip_prefix(self.at("keep"))
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    let text = match meta.is_file() {
                        true => fs::read_to_string(&path).unwrap(),
                        false => String::new(),
                    };
                    if meta.is_dir() {
                        folders.push(path.clone());
                    }
                    entries.push((rel, meta.is_dir(), meta.permissions().readonly(), text));
                }
            }
            entries.sort();
            entries
        }

        fn empty_all(&self, mount: &str, bundle: &str, rows: &[&str]) -> EmptyAnswer {
            let listed = ListedBundle {
                mount: mount.to_owned(),
                bundle: bundle.to_owned(),
                rows: rows.iter().map(|row| (*row).to_owned()).collect(),
            };
            self.svc.empty_trash(&[listed], &[]).unwrap()
        }

        fn delete_for_good(&self, mount: &str, bundle: &str, path: &str) -> EmptyAnswer {
            self.svc
                .empty_trash(&[], &[address(mount, bundle, path)])
                .unwrap()
        }

        fn restore_row(&self, mount: &str, bundle: &str, path: &str) -> Outcome<String> {
            let answer = self
                .svc
                .restore(&[address(mount, bundle, path)], false, None)
                .unwrap();
            answer.items[0].outcome.clone()
        }

        /// The rows the view lists for `bundle` of the mount `mount`, each with its state.
        fn listed(&self, mount: &str, bundle: &str) -> Option<Vec<(String, RootState)>> {
            let view = self.view();
            let mount = view
                .mounts
                .iter()
                .find(|row| row.mount.as_str() == mount)
                .unwrap();
            let bundles = mount.view.as_ref().ok()?;
            bundles
                .iter()
                .find(|row| row.id.as_str() == bundle)
                .map(|bundle| {
                    bundle
                        .rows
                        .iter()
                        .map(|row| (row.path.as_str().to_owned(), row.state.clone()))
                        .collect()
                })
        }
    }

    fn address(mount: &str, bundle: &str, path: &str) -> TrashAddress {
        TrashAddress {
            mount: mount.to_owned(),
            bundle: bundle.to_owned(),
            path: path.to_owned(),
        }
    }

    /// What came of the answer's one row: `gone`, `not-found` or the code it remains with.
    fn said(answer: &EmptyAnswer) -> String {
        assert_eq!(answer.rows.len(), 1, "{answer:?}");
        match &answer.rows[0].outcome {
            Emptied::Gone => "gone".to_owned(),
            Emptied::Remaining(why) => why.code.clone(),
            Emptied::NotFound(why) => why.code.clone(),
        }
    }

    // Link safety (1): a row that is a link is removed (empty-all, permanent delete) or moved (restore) as a link —
    //   `keep`, which it points at, keeps every entry.
    #[test]
    fn a_row_that_is_a_link_is_removed_or_restored_as_a_link_and_nothing_behind_it_goes() {
        let world = World::new();
        world.put("keep/x.md", "keep");
        world.put("keep/link/x.md", "keep");
        for bundle in [BUNDLE, SECOND, THIRD] {
            fs::create_dir_all(world.at(&format!("lib/.trash/{bundle}"))).unwrap();
            world.link("keep", &format!("lib/.trash/{bundle}/link"));
            world.record("lib", bundle, &["link"]);
        }
        let before = world.keep();

        assert_eq!(
            world.listed("library", BUNDLE),
            Some(vec![(
                "link".to_owned(),
                RootState::Present(EntryKind::Link)
            )])
        );
        assert_eq!(said(&world.empty_all("library", BUNDLE, &["link"])), "gone");
        assert_eq!(
            said(&world.delete_for_good("library", SECOND, "link")),
            "gone"
        );
        assert_eq!(
            world.restore_row("library", THIRD, "link"),
            Outcome::Done("library/link".to_owned())
        );
        lock(&world.links).push(world.at("lib/link"));

        assert!(
            !world.there(&format!("lib/.trash/{BUNDLE}")),
            "the bundle went with its only row"
        );
        assert!(!world.there(&format!("lib/.trash/{SECOND}")));
        assert_eq!(
            FsStore.stat(&world.at("lib/link")).unwrap(),
            EntryKind::Link,
            "restored as a link"
        );
        assert_eq!(world.keep(), before, "nothing behind the link changed");
    }

    // Link safety (2) · ADR-0409 decision 11: a folder row with a link two folders down and read-only entries is
    //   removed whole — the first attempt refused (the seam), `make_removable` runs with the link inside the row — and
    //   `keep`, behind the link, keeps every entry and its read-only attribute; permanent delete does the same.
    #[test]
    fn a_folder_row_with_a_link_inside_and_read_only_entries_goes_and_nothing_behind_the_link_changes(
    ) {
        let world = World::new();
        world.put("keep/x.md", "keep");
        world.put("keep/e/b.md", "keep");
        world.read_only("keep/x.md", true);
        for bundle in [BUNDLE, SECOND] {
            world.put(&format!("lib/.trash/{bundle}/d/a.md"), "a");
            world.put(&format!("lib/.trash/{bundle}/d/e/b.md"), "b");
            world.put(&format!("lib/.trash/{bundle}/d/ro.md"), "read-only");
            world.read_only(&format!("lib/.trash/{bundle}/d/ro.md"), true);
            world.link("keep", &format!("lib/.trash/{bundle}/d/e/l"));
            world.record("lib", bundle, &["d"]);
        }
        let before = world.keep();

        DENY_FIRST_REMOVAL.with(|deny| deny.set(true));
        REMOVABLE_ANSWERS.with(|answers| answers.borrow_mut().clear());
        let emptied = world.empty_all("library", BUNDLE, &["d"]);
        DENY_FIRST_REMOVAL.with(|deny| deny.set(false));
        let answers = REMOVABLE_ANSWERS.with(|answers| answers.take());
        let permanent = world.delete_for_good("library", SECOND, "d");
        let after = world.keep();
        world.read_only("keep/x.md", false);

        assert_eq!(said(&emptied), "gone");
        assert_eq!(
            answers,
            [(world.at(&format!("lib/.trash/{BUNDLE}/d")), Ok(Vec::new()))],
            "the refused first attempt ran make_removable on the row, which left nothing"
        );
        assert_eq!(said(&permanent), "gone");
        assert!(!world.there(&format!("lib/.trash/{BUNDLE}")));
        assert!(!world.there(&format!("lib/.trash/{SECOND}")));
        assert_eq!(
            after, before,
            "every entry of keep and its read-only attribute unchanged"
        );
        assert!(before
            .iter()
            .any(|(rel, _, read_only, _)| rel == "x.md" && *read_only));
    }

    // Link safety (3) · ADR-0409 decision 6: a bundle folder that is a link to `keep` is foreign — absent from the
    //   view, `not-found` by address for empty-all, permanent delete and restore — and nothing behind it goes.
    #[test]
    fn a_bundle_folder_that_is_a_link_is_foreign_and_nothing_behind_it_goes() {
        let world = World::new();
        world.put(
            &format!("keep/{RECORD_NAME}"),
            r#"{"roots":[{"path":"x.md"}]}"#,
        );
        world.put("keep/x.md", "keep");
        fs::create_dir_all(world.at("lib/.trash")).unwrap();
        world.link("keep", &format!("lib/.trash/{BUNDLE}"));
        let before = world.keep();

        assert_eq!(world.listed("library", BUNDLE), None, "not in the view");
        let emptied = world.empty_all("library", BUNDLE, &["x.md"]);
        assert_eq!(said(&emptied), NOT_FOUND);
        assert_eq!(emptied.bundles[0].outcome, BundleOutcome::NotFound);
        assert_eq!(
            said(&world.delete_for_good("library", BUNDLE, "x.md")),
            NOT_FOUND
        );
        assert_eq!(
            failed_code(&world.restore_row("library", BUNDLE, "x.md")),
            NOT_FOUND
        );
        assert_eq!(
            FsStore
                .stat(&world.at(&format!("lib/.trash/{BUNDLE}")))
                .unwrap(),
            EntryKind::Link
        );
        assert_eq!(world.keep(), before);
    }

    // Link safety (4) · ADR-0409 decision 5: a `.trash` that is a link to `keep` makes that mount's trash unusable — a
    //   delete refuses, the view names the mount, nothing is restored, emptied or permanently deleted.
    #[test]
    fn a_trash_folder_that_is_a_link_is_unusable_and_nothing_behind_it_goes() {
        let world = World::new();
        world.put(
            &format!("keep/{BUNDLE}/{RECORD_NAME}"),
            r#"{"roots":[{"path":"x.md"}]}"#,
        );
        world.put(&format!("keep/{BUNDLE}/x.md"), "keep");
        world.put("lib2/item.md", "item");
        world.link("keep", "lib2/.trash");
        world.scan();
        let before = world.keep();

        let deleted = world.delete(&["team/item.md"]);
        assert_eq!(failed_code(&deleted.items[0].outcome), TRASH_UNUSABLE);
        assert_eq!(world.text("lib2/item.md"), "item");
        let view = world.view();
        let team = view
            .mounts
            .iter()
            .find(|row| row.mount.as_str() == "team")
            .unwrap();
        assert_eq!(team.view.as_ref().unwrap_err().code, TRASH_UNUSABLE);
        let emptied = world.empty_all("team", BUNDLE, &["x.md"]);
        assert_eq!(said(&emptied), TRASH_UNUSABLE);
        assert_eq!(emptied.bundles[0].outcome, BundleOutcome::Remaining);
        assert_eq!(
            said(&world.delete_for_good("team", BUNDLE, "x.md")),
            TRASH_UNUSABLE
        );
        assert_eq!(
            failed_code(&world.restore_row("team", BUNDLE, "x.md")),
            TRASH_UNUSABLE
        );
        assert_eq!(world.keep(), before);
    }

    // Link safety (5) · ADR-0409 decision 9: a link between the bundle and a row (`bundle/a` → `keep`, `keep/b/x.md`
    //   behind the row `a/b/x.md`) leaves the row untouched, `path-through-link`, for empty-all, permanent delete and
    //   restore; the view says so and `keep` keeps every entry. The link itself is an `other` row: removed, it goes as
    //   a link, and the bundle with it.
    #[test]
    fn a_link_between_the_bundle_and_a_row_leaves_the_row_and_everything_behind_it() {
        let world = World::new();
        world.put("keep/b/x.md", "keep");
        fs::create_dir_all(world.at(&format!("lib/.trash/{BUNDLE}"))).unwrap();
        world.link("keep", &format!("lib/.trash/{BUNDLE}/a"));
        world.record("lib", BUNDLE, &["a/b/x.md"]);
        let before = world.keep();

        assert_eq!(
            world.listed("library", BUNDLE),
            Some(vec![
                ("a".to_owned(), RootState::Present(EntryKind::Link)),
                ("a/b/x.md".to_owned(), RootState::ThroughLink),
            ])
        );
        let emptied = world.empty_all("library", BUNDLE, &["a/b/x.md"]);
        assert_eq!(said(&emptied), PATH_THROUGH_LINK);
        assert_eq!(emptied.bundles[0].outcome, BundleOutcome::Remaining);
        assert_eq!(
            said(&world.delete_for_good("library", BUNDLE, "a/b/x.md")),
            PATH_THROUGH_LINK
        );
        assert_eq!(
            failed_code(&world.restore_row("library", BUNDLE, "a/b/x.md")),
            PATH_THROUGH_LINK
        );
        assert!(
            world.there(&format!("lib/.trash/{BUNDLE}/{RECORD_NAME}")),
            "the record stays"
        );
        assert_eq!(world.keep(), before);

        assert_eq!(said(&world.delete_for_good("library", BUNDLE, "a")), "gone");
        assert!(
            !world.there(&format!("lib/.trash/{BUNDLE}")),
            "no root left: the bundle went"
        );
        assert_eq!(world.keep(), before);
    }

    // ADR-0322 decision 7 · ADR-0409 decision 15: the mount folders are read once — a mount relocated while one of its
    //   bundles is being emptied is emptied on at the folder read first; the bundle of that name in its new folder stays.
    #[test]
    fn a_mount_relocated_while_emptying_is_emptied_on_at_the_folder_read_first() {
        let world = World::new();
        world.put(&format!("lib2/.trash/{BUNDLE}/x.md"), "x");
        world.record("lib2", BUNDLE, &["x.md"]);
        world.put(&format!("lib2/.trash/{SECOND}/y.md"), "y");
        world.record("lib2", SECOND, &["y.md"]);
        world.put(&format!("lib3/.trash/{SECOND}/y.md"), "elsewhere");
        world.record("lib3", SECOND, &["y.md"]);
        let (entered, release) = (mpsc::channel(), mpsc::channel());
        *lock(&world.disk.remove_door) = Some((
            world.at(&format!("lib2/.trash/{BUNDLE}/x.md")),
            entered.0,
            release.1,
        ));
        let worker = on_thread(&world.svc, |on| {
            let listed = |bundle: &str, row: &str| ListedBundle {
                mount: "team".to_owned(),
                bundle: bundle.to_owned(),
                rows: vec![row.to_owned()],
            };
            on.empty_trash(&[listed(BUNDLE, "x.md"), listed(SECOND, "y.md")], &[])
                .unwrap()
        });
        entered
            .1
            .recv_timeout(Duration::from_secs(20))
            .expect("the removal is held");

        world
            .svc
            .relocate_mount("team", &world.at("lib3"), false)
            .unwrap();
        release.0.send(()).unwrap();
        let answer = worker.join().unwrap();

        let rows: Vec<bool> = answer
            .rows
            .iter()
            .map(|row| row.outcome == Emptied::Gone)
            .collect();
        assert_eq!(rows, [true, true], "{answer:?}");
        assert!(
            !world.there(&format!("lib2/.trash/{SECOND}")),
            "emptied at the folder read first"
        );
        assert_eq!(
            world.text(&format!("lib3/.trash/{SECOND}/y.md")),
            "elsewhere"
        );
    }

    // Link safety · ADR-0409 decision 9 (round 1): each row's names are checked again right before its removal — a
    //   folder between the bundle and a row swapped for a link to `keep` while an earlier row was being removed leaves
    //   the row `path-through-link`, and nothing behind the link goes.
    #[test]
    fn a_folder_swapped_for_a_link_before_a_rows_turn_leaves_the_row_and_everything_behind_it() {
        let world = World::new();
        world.put("keep/b/x.md", "keep");
        world.put(&format!("lib/.trash/{BUNDLE}/z.md"), "z");
        world.put(&format!("lib/.trash/{BUNDLE}/a/b/x.md"), "x");
        world.record("lib", BUNDLE, &["z.md", "a/b/x.md"]);
        let before = world.keep();
        let (entered, release) = (mpsc::channel(), mpsc::channel());
        *lock(&world.disk.remove_door) = Some((
            world.at(&format!("lib/.trash/{BUNDLE}/z.md")),
            entered.0,
            release.1,
        ));
        let worker = on_thread(&world.svc, |on| {
            let listed = ListedBundle {
                mount: "library".to_owned(),
                bundle: BUNDLE.to_owned(),
                rows: vec!["z.md".to_owned(), "a/b/x.md".to_owned()],
            };
            on.empty_trash(&[listed], &[]).unwrap()
        });
        entered
            .1
            .recv_timeout(Duration::from_secs(20))
            .expect("z.md's removal is held");
        fs::rename(
            world.at(&format!("lib/.trash/{BUNDLE}/a")),
            world.at("lib/a-moved"),
        )
        .unwrap();
        world.link("keep", &format!("lib/.trash/{BUNDLE}/a"));
        release.0.send(()).unwrap();
        let answer = worker.join().unwrap();

        let said: Vec<String> = answer
            .rows
            .iter()
            .map(|row| match &row.outcome {
                Emptied::Gone => "gone".to_owned(),
                Emptied::Remaining(why) | Emptied::NotFound(why) => why.code.clone(),
            })
            .collect();
        assert_eq!(said, ["gone", PATH_THROUGH_LINK]);
        assert_eq!(world.keep(), before);
        assert_eq!(world.text("lib/a-moved/b/x.md"), "x");
    }

    // Link safety (5) · round 1: named together with the link above it, a row behind the link is judged again after the
    //   link went — gone, the bundle with it — and `keep` keeps every entry.
    #[test]
    fn a_row_behind_a_link_named_with_the_link_goes_with_it() {
        let world = World::new();
        world.put("keep/b/x.md", "keep");
        fs::create_dir_all(world.at(&format!("lib/.trash/{BUNDLE}"))).unwrap();
        world.link("keep", &format!("lib/.trash/{BUNDLE}/a"));
        world.record("lib", BUNDLE, &["a/b/x.md"]);
        let before = world.keep();

        let answer = world.empty_all("library", BUNDLE, &["a/b/x.md", "a"]);

        let gone: Vec<bool> = answer
            .rows
            .iter()
            .map(|row| row.outcome == Emptied::Gone)
            .collect();
        assert_eq!(gone, [true, true], "{answer:?}");
        assert_eq!(answer.bundles[0].outcome, BundleOutcome::Gone);
        assert_eq!(world.keep(), before);
    }

    // ADR-0409 decision 8 (round 1): a deleted folder whose record failed leaves its empty sub-folder as a row of its
    //   own — listed, emptied, and the bundle goes.
    #[test]
    fn an_empty_folder_of_a_recordless_bundle_is_a_row_and_the_bundle_can_be_emptied() {
        let world = World::new();
        world.put("lib/d/y.md", "y");
        fs::create_dir_all(world.at("lib/d/e")).unwrap();
        world.scan();
        world.disk.fail_record.store(true, Ordering::SeqCst);
        let deleted = world.delete(&["library/d"]);
        world.disk.fail_record.store(false, Ordering::SeqCst);
        let bundle = done(&deleted.items[0].outcome).bundle;

        assert_eq!(
            world.listed("library", &bundle),
            Some(vec![
                ("d/e".to_owned(), RootState::Present(EntryKind::Folder)),
                ("d/y.md".to_owned(), RootState::Present(EntryKind::File)),
            ])
        );
        let answer = world.empty_all("library", &bundle, &["d/e", "d/y.md"]);

        let gone: Vec<bool> = answer
            .rows
            .iter()
            .map(|row| row.outcome == Emptied::Gone)
            .collect();
        assert_eq!(gone, [true, true], "{answer:?}");
        assert_eq!(answer.bundles[0].outcome, BundleOutcome::Gone);
    }

    // ADR-0365 · ADR-0409 decision 15 (round 2): a mount folder that is a link is judged by what it points at — its target
    //   gone while emptying (a pulled volume) leaves the row being removed and the later bundle `library-unavailable`,
    //   never gone or not-found.
    #[test]
    fn a_linked_mount_whose_target_goes_while_emptying_is_unavailable() {
        let world = World::new();
        world.put(&format!("target/.trash/{BUNDLE}/x.md"), "x");
        world.record("target", BUNDLE, &["x.md"]);
        world.put(&format!("target/.trash/{SECOND}/y.md"), "y");
        world.record("target", SECOND, &["y.md"]);
        world.link("target", "linked");
        world
            .svc
            .relocate_mount("team", &world.at("linked"), false)
            .unwrap();
        let folder = world
            .svc
            .with_read(|state| state.mounts.get("team").unwrap().folder.clone());
        assert_eq!(
            FsStore.stat(&folder).unwrap(),
            EntryKind::Link,
            "the mount folder is the link"
        );
        let (entered, release) = (mpsc::channel(), mpsc::channel());
        *lock(&world.disk.remove_door) = Some((
            folder.join(".trash").join(BUNDLE).join("x.md"),
            entered.0,
            release.1,
        ));
        let worker = on_thread(&world.svc, |on| {
            let listed = |bundle: &str, row: &str| ListedBundle {
                mount: "team".to_owned(),
                bundle: bundle.to_owned(),
                rows: vec![row.to_owned()],
            };
            on.empty_trash(&[listed(BUNDLE, "x.md"), listed(SECOND, "y.md")], &[])
                .unwrap()
        });
        entered
            .1
            .recv_timeout(Duration::from_secs(20))
            .expect("the removal is held");
        fs::rename(world.at("target"), world.at("target-gone")).unwrap();
        release.0.send(()).unwrap();
        let answer = worker.join().unwrap();

        let said: Vec<String> = answer
            .rows
            .iter()
            .map(|row| match &row.outcome {
                Emptied::Gone => "gone".to_owned(),
                Emptied::Remaining(why) | Emptied::NotFound(why) => why.code.clone(),
            })
            .collect();
        assert_eq!(said, [LIBRARY_UNAVAILABLE, LIBRARY_UNAVAILABLE]);
        assert_eq!(
            world.text(&format!("target-gone/.trash/{BUNDLE}/x.md")),
            "x"
        );
    }
}
