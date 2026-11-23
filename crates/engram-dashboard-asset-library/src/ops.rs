//! Step-9b file operations — inherent [`LibraryService`] methods answering crate types: delete (into the item's own mount's
//! `.trash`) and restore (back out of it). Move, rename and copy join them later. Each runs plan → execute → fix
//! references (ADR-0335) under one `Change` hold (ADR-0409 M1).
//!
//! ## Rules (invariants)
//!
//! - **Per item** (ADR-0409): every given path answers on its own ([`Outcome`]) — done, skipped (a path given with a folder
//!   above it: only the folder counts), a conflict, or failed with a code — and what finished stays finished. Refused
//!   per item before the hold: a spelling that is no library path or holds a dot or temp name (a path under `.trash`
//!   included) `item-path-invalid`, a mount's top `virtual-root-read-only`, the built-in mount `mount-read-only`, a mount
//!   that cannot be read `library-unavailable` (walked again first — ADR-0400 decision 6). Inside the hold, before any
//!   disk step: the mount judged again from the recorded state, the link admission (ADR-0365 — `path-through-link`), and
//!   a no-follow stat (absent `not-found`).
//! - **The wait before the ticket** (ADR-0409 decision 14): delete and restore always may touch `.trash`, so they run
//!   through [`LibraryService::change_held_at`] with the waiting count — raised just before the ticket, lowered after the
//!   hold or on any earlier exit — and take the trash lock once, before their per-mount loop, holding it to the end of
//!   their trash moves only (it is released before any preset file or agent is touched).
//! - **Delete** (ADR-0409 decisions 5–8): the plan names who refers to what goes — the preset files listing a deleted path
//!   (a folder: anything inside it; a lone `SKILL.md` — a file the catalog holds as a skill's, never a folder of that
//!   name: its skill folder or itself — ADR-0339) that are not deleted themselves, and the agents attaching a deleted path
//!   (never for a `SKILL.md`); its digest pins those and the items. Referenced and no `yes` → one
//!   [`DELETE_UNCONFIRMED`] for the call with the references and the two forms; `yes` keeps them (they read as not
//!   found until restored); `yes` + `detachRefs` must carry the plan's digest; `detachRefs` without `yes` is no `yes`
//!   (ADR-0339 M108 — every delete item is of a detachable shape, so a delete never refuses `detachRefs`). The same
//!   judgement runs again inside the hold before any disk step: a plan that changed answers the conflict again with the
//!   fresh plan, nothing written (ADR-0405 decision 1 · ADR-0409 decision 2). In the hold, per mount: `.trash` checked and
//!   made without following (unusable → every item there `trash-unusable`), one bundle id per operation (a taken id in a
//!   later mount draws a new one there), each item moved by the store's no-replace trash step into
//!   `.trash/<bundle>/<its path in the mount, as the catalog spells it when that is the same entry>`, the record published
//!   after that mount's moves
//!   (a failed publication leaves the moves standing — the bundle reads by its files), a bundle that received no root
//!   pruned. Then the
//!   catalog range leaves and the parent folder is re-classified from disk ([`LibraryService::reclassify`] — a folder
//!   that lost its `SKILL.md` is no skill at once). With `detachRefs`, kept to the confirmed plan (the hold's
//!   judgement): its preset files that still list a path that went drop those lines (a preset inside an item that did
//!   not go, or trashed with the operation, is not rewritten), then the change number is raised, then its agents that
//!   still attach a deleted preset path are written without it (`ProfileEditPort::set_attachments`, one by one under the
//!   write lock; an agent inserted after the judgement keeps its attachment — an `agent.new` judged before the raise and
//!   inserted after it judges again, ADR-0400 decision 18). After the hold, the `.git/info/exclude` line is checked for
//!   every mount whose `.trash` was made usable (even when its bundle then failed).
//! - **Stop rules** (ADR-0335, relink's): a per-file refusal skips the file; a read or write failure stops before the
//!   agents (the answer carries it); a file in a mount that cannot be read is skipped `library-unavailable`, a mount never
//!   walked this run is one skipped row at its top. Nothing is journaled: an interrupted detach leaves kept references.
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
// ADR-0365
// ADR-0400 decisions 6, 18 and 23
// ADR-0405
// ADR-0409

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use engram_dashboard_agent::launch::{escape_controls, Reason};
use engram_dashboard_agent::profile::{StoreError, StoreStatus};
use engram_dashboard_agent::profile_edit::{Attachments, ProfileEditError, ProfileEditPort};
use engram_dashboard_agent::types::AgentId;
use engram_dashboard_platform::fs::{path_key, same_path, PathKey};
use serde_json::{json, Value};

use crate::agent_edit::port_refused;
use crate::catalog::{key, makes_skill, preset_refs, within, Catalog, ItemContent, ItemKind};
use crate::create::NAME_EXISTS;
use crate::edit::{as_read, lib_path, list_mut, ITEM_LISTS, LIBRARY_WRITE_FAILED};
use crate::format::read::ITEM_PATH_INVALID;
use crate::gate::NOT_FOUND;
use crate::launch::LIBRARY_READ_FAILED;
use crate::mounts::{
    admits_inside, in_builtin, is_builtin, virtual_path, Mount, MountName, MountPath,
};
use crate::path::{AssetRef, LibPath};
use crate::relink::{RelinkSkip, RelinkStop};
use crate::rename::lines_digest;
use crate::store::{EntryKind, NameWalk, NO_REPLACE_UNSUPPORTED};
use crate::trash::{
    below, check_trash_folder, ensure_trash_folder, fallback_rows, make_bundle, prune_shell,
    publish_record, read_record, row_path, trash_unusable, BundleId, FallbackRow, RecordRead,
    RecordRoot, RootState, TrashUnavailable, TRASH_NOT_RESERVED,
};
use crate::{
    mount_read_only, mount_unavailable, refuse_unusable_now, virtual_root_read_only, Hold,
    LibraryRefusal, LibraryService, MountsNow, RefusalClass, PATH_THROUGH_LINK,
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

/// How many keep-both names (`name (2)` …) a restore tries before it answers the row's conflict.
const KEEP_BOTH_TRIES: u32 = 1_000;

/// What one item or row of an operation came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome<T> {
    Done(T),
    /// Not acted on: it lies inside `inside`, a folder given in the same call (only the folder counts).
    Skipped {
        inside: String,
    },
    /// A name collision — nothing replaced.
    Conflict(Reason),
    Failed(Reason),
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

/// What a delete would take references from, as its `CONFLICT` names it.
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
        for (at, inside) in nested(&targets) {
            outcomes[at] = Some(Outcome::Skipped { inside });
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
        let gone = self.trash_items(checked, hold, &mut items, &mut deleted.bundled);
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
            match self.reclassify(catalog, mount, folder, hold.seq()) {
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

    /// Moves `checked` into their mounts' trash under the trash lock, mount by mount (module header); answers the items
    /// that went and pushes every outcome into `items` and every mount folder whose `.trash` was made usable into `bundled`.
    fn trash_items(
        &self,
        checked: Vec<Checked>,
        hold: &Hold,
        items: &mut Vec<(usize, Outcome<TrashAddress>)>,
        bundled: &mut Vec<PathBuf>,
    ) -> Vec<Checked> {
        let mut gone = Vec::new();
        if checked.is_empty() {
            return gone;
        }
        // ADR-0409 decision 14 · ADR-0412 decision 5: taken once, before the per-mount loop, kept to the last move.
        let held = match hold.take_trash() {
            Ok(held) => held,
            Err(TrashUnavailable::NotReserved) => {
                for item in checked {
                    let args = json!({ "path": item.target.path.as_str() });
                    items.push((
                        item.target.at,
                        Outcome::Failed(reason(TRASH_NOT_RESERVED, args)),
                    ));
                }
                return gone;
            }
            Err(TrashUnavailable::AlreadyHeld) => {
                unreachable!("a delete takes the trash lock once")
            }
        };
        let wanted = wanted_bundle();
        let mut by_mount: Vec<(Mount, Vec<Checked>)> = Vec::new();
        for item in checked {
            match by_mount
                .iter_mut()
                .find(|(mount, _)| mount.name.key() == item.mount.name.key())
            {
                Some((_, group)) => group.push(item),
                None => by_mount.push((item.mount.clone(), vec![item])),
            }
        }
        for (mount, group) in by_mount {
            let store = &*self.store;
            if let Err(unusable) = ensure_trash_folder(store, &mount.folder, &held) {
                tracing::warn!(
                    mount = mount.name.as_str(),
                    "library delete: trash unusable: {}",
                    escape_controls(&unusable.why)
                );
                for item in group {
                    items.push((
                        item.target.at,
                        Outcome::Failed(trash_unusable(&mount.name, &unusable)),
                    ));
                }
                continue;
            }
            // The exclude line is checked after the hold for every mount whose `.trash` this step may have made.
            bundled.push(mount.folder.clone());
            let id = match make_bundle(store, &mount.folder, &wanted, &held) {
                Ok(id) => id,
                Err(e) => {
                    for item in group {
                        let args =
                            json!({ "path": item.target.path.as_str(), "why": e.to_string() });
                        items.push((
                            item.target.at,
                            Outcome::Failed(reason(LIBRARY_WRITE_FAILED, args)),
                        ));
                    }
                    continue;
                }
            };
            let mut roots = Vec::new();
            for item in group {
                match store.trash(&mount.folder, &item.disk, &id.bundle()) {
                    Ok(_) => {
                        roots.push(RecordRoot {
                            path: item.rest.clone(),
                            staging: None,
                        });
                        let address = TrashAddress {
                            mount: mount.name.as_str().to_owned(),
                            bundle: id.as_str().to_owned(),
                            path: item.rest.as_str().to_owned(),
                        };
                        items.push((item.target.at, Outcome::Done(address)));
                        gone.push(item);
                    }
                    Err(e) => {
                        let failure = move_failed(&item.target.path, &e);
                        items.push((item.target.at, Outcome::Failed(failure)));
                    }
                }
            }
            if roots.is_empty() {
                // ADR-0409 decision 7: a bundle that received no root goes again, empty-only.
                if let Err(e) = prune_shell(store, &mount.folder, &id, None, &held) {
                    tracing::warn!(
                        bundle = id.as_str(),
                        "library delete: empty bundle left: {e}"
                    );
                }
            } else if let Err(e) = publish_record(store, &mount.folder, &id, &roots, &held) {
                // The moves stand; the bundle reads by its files.
                tracing::warn!(
                    bundle = id.as_str(),
                    "library delete: bundle record not published: {e}"
                );
            }
        }
        gone
    }

    /// Re-classifies `folder` (`rest` in `mount`) after an entry left it (ADR-0409 M3): a skill whose folder no longer
    /// holds a file the scan's classifier takes for its `SKILL.md` — one no-follow listing of the folder — is no skill:
    /// its item goes at `at`. Answers the keys removed; `Err` = the listing failed and the catalog keeps what it knew.
    fn reclassify(
        &self,
        catalog: &mut Catalog,
        mount: &Mount,
        rest: &MountPath,
        at: u64,
    ) -> io::Result<Vec<PathKey>> {
        let folder = virtual_path(&mount.name, Some(rest));
        let skill = catalog
            .get(&key(&folder))
            .is_some_and(|item| item.kind == ItemKind::Skill);
        if !skill {
            return Ok(Vec::new());
        }
        let still = self
            .store
            .list(&rest.to_path(&mount.folder))?
            .iter()
            .any(|(name, kind)| {
                *kind == EntryKind::File
                    && name.to_str().is_some_and(|name| makes_skill(name, &folder))
            });
        if still || !catalog.remove(&key(&folder), at) {
            return Ok(Vec::new());
        }
        tracing::info!(
            folder = folder.as_str(),
            "library folder is no skill any more"
        );
        Ok(vec![key(&folder)])
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
        // ADR-0400 decision 6: a mount never walked this run is one skipped row; a file in a mount that cannot be read is
        //   skipped, its disk untouched.
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
        deleted.skipped.extend(unknown);
        let seq = || hold.seq();
        for file in &files {
            if let Err(refusal) = refuse_unusable_now(mounts, file) {
                deleted.skipped.push(RelinkSkip {
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
                |edit| Ok((dropped(edit.doc, gone), ())),
                touched,
            );
            match rewritten {
                Ok(done) => {
                    if done.written {
                        deleted.written.push(file.as_str().to_owned());
                    }
                }
                Err(LibraryRefusal::Refused { class, reason })
                    if class != RefusalClass::Unavailable =>
                {
                    deleted.skipped.push(RelinkSkip {
                        path: file.as_str().to_owned(),
                        reason,
                    });
                }
                Err(refusal) => {
                    tracing::warn!(path = file.as_str(), refusal = ?refusal, "library delete: reference fix stopped");
                    deleted.stopped = Some(RelinkStop {
                        path: Some(file.as_str().to_owned()),
                        refusal,
                    });
                    return;
                }
            }
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

/// The library path a delete may take: a library path below a mount's top, outside the built-in mount, with no dot or
/// temp name — or why not.
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

/// The targets given inside another target (or given twice), each with the path of the one it lies in — only that one
/// counts (ADR-0409 · P5).
fn nested(targets: &[Target]) -> Vec<(usize, String)> {
    targets
        .iter()
        .enumerate()
        .filter_map(|(i, target)| {
            targets
                .iter()
                .enumerate()
                .find(|(j, other)| {
                    *j != i
                        && within(&target.path, Some(&other.path))
                        && (key(&target.path) != key(&other.path) || *j < i)
                })
                .map(|(_, other)| (target.at, other.path.as_str().to_owned()))
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
    use tempfile::TempDir;

    use super::*;
    use crate::backends::AgentBackends;
    use crate::config::{LibraryConfig, MountPlace, MountSetting};
    use crate::edit::{AddEntries, NewItem};
    use crate::folder::{BoundedIo, LaunchWaits, StdFolderOps, ThreadSpawn, IO_LIMIT, LIST_CAP};
    use crate::store::{ContentTag, FsStore, LibraryStore, ScanListing};
    use crate::trash::{RowKind, RowSource, TrashListing, RECORD_NAME, TRASH_UNUSABLE};
    use crate::{
        LibraryValues, LIBRARY_UNAVAILABLE, MOUNT_READ_ONLY, ROOT_GENERATION_STALE,
        VIRTUAL_ROOT_READ_ONLY,
    };

    /// A stat that parks: it tells the sender, then waits for the receiver.
    type Door = (PathBuf, Sender<()>, Receiver<()>);

    /// [`FsStore`] with four seams: the record publication fails (a no-replace rename onto the record name) while
    /// `fail_record` is set, the next stat of the door's path parks, every stat of `fail_stat`'s path fails, and
    /// `identity_of` `(at, as)` answers the identity of `as` for `at`; `identities` counts the identity probes.
    #[derive(Default)]
    struct Disk {
        fail_record: AtomicBool,
        stat_door: Mutex<Option<Door>>,
        fail_stat: Mutex<Option<PathBuf>>,
        identity_of: Mutex<Option<(PathBuf, PathBuf)>>,
        identities: std::sync::atomic::AtomicUsize,
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
            FsStore.write_atomic(path, bytes)
        }
        fn create_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            FsStore.create_new(path, bytes)
        }
        fn create_dir(&self, path: &Path) -> io::Result<()> {
            FsStore.create_dir(path)
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            if self.fail_record.load(Ordering::SeqCst)
                && to.file_name().is_some_and(|name| name == RECORD_NAME)
            {
                return Err(io::Error::other("injected record publication failure"));
            }
            FsStore.rename(from, to)
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
            FsStore.list(path)
        }
        fn remove_dir(&self, path: &Path) -> io::Result<()> {
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

    /// A profile-edit port over the service's registry: an attachment write goes through the registry's public update
    /// and the paths it drops are logged; `entered` is told when a write starts, which then waits for `release` (when
    /// set).
    struct Port {
        registry: Arc<ProfileRegistry>,
        removed: Mutex<Vec<Vec<String>>>,
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
            _: &[PresetPathRename],
        ) -> Result<Vec<AgentId>, ProfileEditError> {
            unreachable!("not a delete's call")
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

    /// Two mounts in a temp folder — `library` at `lib` and `team` at `lib2` — on [`Disk`], with the port plugged.
    struct World {
        _temp: TempDir,
        base: PathBuf,
        links: Mutex<Vec<PathBuf>>,
        disk: Arc<Disk>,
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
            let io = BoundedIo::new(
                limit,
                LaunchWaits::LAUNCH,
                LIST_CAP,
                Arc::new(StdFolderOps),
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
            Outcome::Skipped {
                inside: "library/d".to_owned()
            }
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
            world.removed().is_empty(),
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
}
