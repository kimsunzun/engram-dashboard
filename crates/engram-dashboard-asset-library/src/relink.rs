//! Relink (`relink {from, to}`) — an inherent [`LibraryService`] method answering a crate-local type: the file picked
//! for one reference takes every reference to the old path, and finishes the recovery of the other missing lines of
//! the same old folder by name.
//!
//! ## Rules (invariants)
//!
//! - **One change, presets then agents.** After the first-scan entrance, everything runs inside one
//!   [`LibraryService::change_at`] write lock: `to` in the catalog and its link admission, the registry read, the kind
//!   the referencing lists and agents need, the port cell when agents attach a pair (before any write), each referencing preset file rewritten through the edit pipeline
//!   (`LibraryService::rewrite_in` — its own admission, read, rewrite with origins kept, write and per-item apply), and
//!   then the agents once through `ProfileEditPort::rewrite_preset_paths` (ADR-0335).
//! - **Part (b) only for a missing `from`.** When the catalog holds neither an item nor a folder at `from`, every other
//!   missing target referenced in `from`'s old folder whose name is in the catalog in `to`'s folder (spelled as the
//!   catalog spells it, admitted as `to` is), with the kind every reference to it needs, is re-pointed too; a present
//!   `from` re-points only its own references (ADR-0330).
//! - **A reference is re-pointed only in the list its new target goes into.** An entry of the old path in another list
//!   of the file as read stays as it is. A touched list keeps the first place of each target (ADR-0339). Folder
//!   entries are never touched.
//! - **A per-file refusal skips the file; an IO failure stops.** A refusal tied to the file's content or name (a reader or
//!   writer refusal, a link or a dot / temp name, the file gone, a rewrite that would make the preset reach itself —
//!   `preset-cycle`) is recorded and the walk and the agent rewrite go on. A read or write failure stops the walk, and
//!   the agents are not rewritten. A stop before any file was written answers that failure's refusal; after one, the
//!   answer carries it. Once the failure is cleared, running the same relink again recovers the rest — there is no
//!   journal (ADR-0335).
// ADR-0330
// ADR-0335
// ADR-0339
// ADR-0365
// ADR-0366

use std::collections::{HashMap, HashSet};
use std::path::Path;

use engram_dashboard_agent::launch::Reason;
use engram_dashboard_agent::profile_edit::{Attachments, PresetPathRename};
use engram_dashboard_agent::types::AgentId;
use engram_dashboard_platform::fs::{same_path, PathKey};
use serde_json::json;

use crate::agent_edit::port_refused;
use crate::catalog::{key, preset_refs, Catalog, ItemContent};
use crate::edit::{
    as_read, closes_cycle, doc_list, lib_path, list_mut, list_of, not_found, ITEM_LISTS,
};
use crate::format::{ListName, PresetDoc, PresetLists};
use crate::gate::NO_ROOT;
use crate::path::{AssetRef, LibPath};
use crate::resolve::{ref_key, PRESET_CYCLE};
use crate::{is_known, LibraryRefusal, LibraryService, RefusalClass};

/// The code of a relink whose `to` is not of the kind a list referencing `from` needs (`args` = `{from, to, expected,
/// found}` — `expected` the name of that list, `found` the list `to` goes into, `null` when it goes into none).
// ADR-0330
pub const RELINK_KIND_MISMATCH: &str = "relink-kind-mismatch";

/// The answer of [`LibraryService::relink`].
#[derive(Debug)]
pub struct RelinkAnswer {
    /// The pairs fixed, `(old, new)` as spelled from the root, the picked pair first: at least one reference was
    /// re-pointed for it — in a preset file, in the list its new target goes into, or by the agent rewrite — every
    /// preset file the catalog had listing the old path was rewritten or needed no change, and when agents attached
    /// it, their rewrite succeeded. An agent deleted between the registry read and the rewrite still counts: no
    /// reference to the old path is left either way.
    pub fixed: Vec<(String, String)>,
    /// How many pairs of `fixed` part (b) found by name.
    pub more: usize,
    /// The preset files written, in walk (path) order.
    pub written: Vec<String>,
    /// The preset files skipped, each with its refusal.
    pub skipped: Vec<RelinkSkip>,
    /// Why the walk stopped part-way; `None` = it ran to the end.
    pub stopped: Option<RelinkStop>,
}

/// A preset file the relink skipped and why (the refusal's reason; its class is not kept — a skip applies nothing to
/// that file).
#[derive(Clone, Debug, PartialEq)]
pub struct RelinkSkip {
    pub path: String,
    pub reason: Reason,
}

/// Why the walk stopped after something was applied (a stop before any write is answered as the refusal instead).
#[derive(Debug)]
pub struct RelinkStop {
    /// The preset file whose read or write failed; `None` = the agent rewrite failed.
    pub path: Option<String>,
    /// The failure as a refusal would answer it — `library-read-failed`, `library-write-failed`, or the port's store
    /// error passed through.
    pub refusal: LibraryRefusal,
}

/// One old → new pair and who refers to the old path.
struct Pair {
    from: LibPath,
    to: LibPath,
    /// The list `to` goes into — the only list the pair re-points in; `None` = none (nothing refers to `from`).
    list: Option<ListName>,
    /// Found by part (b).
    more: bool,
    uses: Uses,
    /// The agent rewrite changes an attachment for it: an agent's stored path names `from` and is not spelled as `to`
    /// (the registry rewrites only a spelling that differs) — read from the registry inside the change.
    agent_repoint: bool,
}

/// Who refers to a target, read inside the change.
struct Uses {
    /// The presets listing it directly, by path.
    files: Vec<LibPath>,
    /// The lists it is in, in [`ITEM_LISTS`] order — `presets` too when an agent attaches it.
    needs: Vec<ListName>,
    /// An agent attaches it.
    agents: bool,
}

impl LibraryService {
    /// Re-points every reference to `from` at `to` and — when the catalog lacks `from` — every other missing target of
    /// `from`'s old folder whose name is in `to`'s folder with the kind its references need (module header). Preset
    /// files first, one at a time, then the agents once. Answers what was fixed, written, skipped and why it stopped.
    ///
    /// Refusals, nothing applied: a spelling that is not a library path `InvalidArgument` · `item-path-invalid`; no
    /// published root `Unavailable` · `library-unavailable`; a stale root generation `Conflict` ·
    /// `root-generation-stale`; `to` not in the catalog (incl. a dot or temp name, which the catalog never holds)
    /// `NotFound` · `not-found`; `to` at or below a link `InvalidArgument` · `path-through-link` (a probe error `Unavailable` ·
    /// `library-read-failed`); `to` of another kind than a list referencing `from` needs `InvalidArgument` ·
    /// [`RELINK_KIND_MISMATCH`]; agents to rewrite while the port cell is empty or dead `Internal` ·
    /// `profile-edit-unavailable`; a stop before any file was written — its own refusal.
    pub fn relink(
        &self,
        from: &str,
        to: &str,
        root_generation: Option<u64>,
    ) -> Result<RelinkAnswer, LibraryRefusal> {
        let from = lib_path(from)?;
        let to = lib_path(to)?;
        self.first_scan_join()
            .map_err(LibraryRefusal::unavailable)?;
        self.change_at(root_generation, |catalog, root, seq| {
            let mut touched = Vec::new();
            let answer = match root {
                Some(root) => self.relink_in(&from, &to, catalog, root, seq, &mut touched),
                None => Err(LibraryRefusal::unavailable(NO_ROOT.to_owned())),
            };
            (answer, touched)
        })?
    }

    /// The steps of [`LibraryService::relink`] inside the change; `touched` gathers every key applied.
    fn relink_in(
        &self,
        from: &LibPath,
        to: &LibPath,
        catalog: &mut Catalog,
        root: &Path,
        seq: &dyn Fn() -> u64,
        touched: &mut Vec<PathKey>,
    ) -> Result<RelinkAnswer, LibraryRefusal> {
        let found = match catalog.get(&key(to)) {
            Some(item) => list_of(&item.kind),
            None => return Err(not_found(to.as_str())),
        };
        self.admit(root, to)?;
        let attachments = self.values.registry.attachments();
        let uses = uses_of(catalog, &attachments, from);
        if let Some(expected) = uses.needs.iter().find(|need| Some(**need) != found) {
            return Err(LibraryRefusal::new(
                RefusalClass::InvalidArgument,
                RELINK_KIND_MISMATCH,
                json!({
                    "from": from.as_str(),
                    "to": to.as_str(),
                    "expected": expected.key(),
                    "found": found.map(ListName::key),
                }),
            ));
        }
        let mut pairs = vec![Pair {
            from: from.clone(),
            to: to.clone(),
            list: found,
            more: false,
            uses,
            agent_repoint: agent_repoints(&attachments, from, to),
        }];
        if !is_known(catalog, from) {
            for missing in missing_beside(catalog, &attachments, from) {
                let Some(candidate) = beside(to, missing.file_name()) else {
                    continue;
                };
                let Some(item) = catalog.get(&key(&candidate)) else {
                    continue;
                };
                let (candidate, fits) = (item.path.clone(), list_of(&item.kind));
                // ADR-0365
                if let Err(e) = self.admit(root, &candidate) {
                    tracing::debug!(
                        path = candidate.as_str(),
                        "relink candidate not admitted: {e:?}"
                    );
                    continue;
                }
                let uses = uses_of(catalog, &attachments, &missing);
                if !uses.needs.is_empty() && uses.needs.iter().all(|need| Some(*need) == fits) {
                    let agent_repoint = agent_repoints(&attachments, &missing, &candidate);
                    pairs.push(Pair {
                        from: missing,
                        to: candidate,
                        list: fits,
                        more: true,
                        uses,
                        agent_repoint,
                    });
                }
            }
        }
        let port = match pairs.iter().any(|pair| pair.uses.agents) {
            true => Some(self.profile_edit()?),
            false => None,
        };
        let renames: HashMap<PathKey, (LibPath, ListName)> = pairs
            .iter()
            .filter_map(|pair| Some((key(&pair.from), (pair.to.clone(), pair.list?))))
            .collect();
        let mut files: Vec<LibPath> = pairs
            .iter()
            .flat_map(|pair| pair.uses.files.iter().cloned())
            .collect();
        files.sort();
        files.dedup();

        let (mut written, mut skipped) = (Vec::new(), Vec::new());
        // The old paths (by key) a preset file re-pointed an entry of.
        let mut repointed: HashSet<PathKey> = HashSet::new();
        // The files not fixed: skipped, or not reached because the walk stopped.
        let mut unfixed: HashSet<PathKey> = HashSet::new();
        let mut stopped = None;
        for (at, file) in files.iter().enumerate() {
            let rewritten = self.rewrite_in(
                file,
                None,
                catalog,
                root,
                seq,
                |edit| renamed(edit.doc, &renames, edit.catalog, file),
                touched,
            );
            match rewritten {
                Ok(done) => {
                    if done.written {
                        written.push(file.as_str().to_owned());
                        repointed.extend(done.answer);
                    }
                }
                Err(LibraryRefusal::Refused { class, reason })
                    if class != RefusalClass::Unavailable =>
                {
                    unfixed.insert(key(file));
                    skipped.push(RelinkSkip {
                        path: file.as_str().to_owned(),
                        reason,
                    });
                }
                Err(refusal) => {
                    tracing::warn!(path = %file.as_str(), refusal = ?refusal, "relink stopped");
                    unfixed.extend(files[at..].iter().map(key));
                    stopped = Some(RelinkStop {
                        path: Some(file.as_str().to_owned()),
                        refusal,
                    });
                    break;
                }
            }
        }
        let mut agents_rewritten = false;
        if let (None, Some(port)) = (&stopped, &port) {
            let renames: Vec<PresetPathRename> = pairs
                .iter()
                .filter(|pair| pair.uses.agents)
                .map(|pair| PresetPathRename {
                    from: pair.from.as_str().to_owned(),
                    to: pair.to.as_str().to_owned(),
                })
                .collect();
            match port.rewrite_preset_paths(&renames) {
                Ok(changed) => {
                    agents_rewritten = true;
                    tracing::info!(agents = changed.len(), "agent presets re-pointed by relink");
                }
                Err(e) => {
                    stopped = Some(RelinkStop {
                        path: None,
                        refusal: port_refused(e),
                    })
                }
            }
        }
        if written.is_empty() {
            if let Some(stop) = stopped {
                return Err(stop.refusal);
            }
        }
        let fixed: Vec<&Pair> = pairs
            .iter()
            .filter(|pair| {
                repointed.contains(&key(&pair.from)) || (pair.agent_repoint && agents_rewritten)
            })
            .filter(|pair| {
                pair.uses
                    .files
                    .iter()
                    .all(|file| !unfixed.contains(&key(file)))
            })
            .filter(|pair| !pair.uses.agents || agents_rewritten)
            .collect();
        tracing::info!(
            from = from.as_str(),
            to = to.as_str(),
            fixed = fixed.len(),
            written = written.len(),
            skipped = skipped.len(),
            stopped = stopped.is_some(),
            "relink applied"
        );
        Ok(RelinkAnswer {
            fixed: fixed
                .iter()
                .map(|pair| (pair.from.as_str().to_owned(), pair.to.as_str().to_owned()))
                .collect(),
            more: fixed.iter().filter(|pair| pair.more).count(),
            written,
            skipped,
            stopped,
        })
    }
}

/// Who refers to `target` (by catalog key; agents by platform `same_path`).
fn uses_of(catalog: &Catalog, attachments: &[(AgentId, Attachments)], target: &LibPath) -> Uses {
    let at = key(target);
    let names_it = |item: &crate::path::PresetItem| matches!(&item.target, AssetRef::Library(path) if key(path) == at);
    let mut files = Vec::new();
    let mut lists = HashSet::new();
    for preset in catalog.presets_listing(&at) {
        if let ItemContent::Preset { doc, .. } = &preset.content {
            lists.extend(
                ITEM_LISTS
                    .into_iter()
                    .filter(|list| doc_list(doc, *list).iter().any(|item| names_it(item))),
            );
            files.push(preset.path.clone());
        }
    }
    let agents = attachments.iter().any(|(_, attached)| {
        attached
            .presets
            .iter()
            .any(|preset| same_path(Path::new(&preset.path), Path::new(target.as_str())))
    });
    if agents {
        lists.insert(ListName::Presets);
    }
    Uses {
        files,
        needs: ITEM_LISTS
            .into_iter()
            .filter(|list| lists.contains(list))
            .collect(),
        agents,
    }
}

/// Whether an agent's stored attachment names `from` (platform `same_path`, as `uses_of` matches) and is spelled
/// otherwise than `to` — what the agent rewrite changes.
fn agent_repoints(attachments: &[(AgentId, Attachments)], from: &LibPath, to: &LibPath) -> bool {
    attachments.iter().any(|(_, attached)| {
        attached.presets.iter().any(|preset| {
            same_path(Path::new(&preset.path), Path::new(from.as_str()))
                && preset.path != to.as_str()
        })
    })
}

/// Every target a preset lists or an agent attaches that the catalog lacks and that lies directly in `from`'s folder,
/// `from` excluded — each once (by catalog key), by path.
fn missing_beside(
    catalog: &Catalog,
    attachments: &[(AgentId, Attachments)],
    from: &LibPath,
) -> Vec<LibPath> {
    let folder = from.parent().as_ref().map(key);
    let mut seen = HashSet::from([key(from)]);
    let mut missing = Vec::new();
    let mut consider = |path: &LibPath| {
        if path.parent().as_ref().map(key) == folder
            && catalog.get(&key(path)).is_none()
            && seen.insert(key(path))
        {
            missing.push(path.clone());
        }
    };
    for item in catalog.items() {
        if let ItemContent::Preset { doc, .. } = &item.content {
            preset_refs(doc).for_each(&mut consider);
        }
    }
    for (_, attached) in attachments {
        for preset in &attached.presets {
            if let Ok(path) = LibPath::parse(&preset.path) {
                consider(&path);
            }
        }
    }
    missing.sort();
    missing
}

/// `name` in `to`'s folder; `None` = not a library path.
fn beside(to: &LibPath, name: &str) -> Option<LibPath> {
    match to.parent() {
        Some(folder) => LibPath::parse(&format!("{}/{name}", folder.as_str())).ok(),
        None => LibPath::parse(name).ok(),
    }
}

/// The lists of `doc` (preset `own` as read) with every entry whose target `renames` maps re-pointed in the list the
/// new target goes into (its origin and tag kept); in a list that changed, only the first place of each target stays.
/// A re-pointed `presets` entry that would make `own` reach itself refuses `InvalidArgument` · `preset-cycle` (`args` =
/// `{path, target}`). Answers the lists and the old paths (by key) it re-pointed an entry of.
fn renamed(
    doc: &PresetDoc,
    renames: &HashMap<PathKey, (LibPath, ListName)>,
    catalog: &Catalog,
    own: &LibPath,
) -> Result<(PresetLists, HashSet<PathKey>), LibraryRefusal> {
    let mut lists = as_read(doc);
    let mut repointed = HashSet::new();
    for list in ITEM_LISTS {
        let entries = list_mut(&mut lists, list);
        let mut changed = false;
        for entry in entries.iter_mut() {
            let AssetRef::Library(path) = &entry.target else {
                continue;
            };
            let Some((to, _)) = renames
                .get(&key(path))
                .filter(|(to, goes)| *goes == list && to.as_str() != path.as_str())
            else {
                continue;
            };
            let target = AssetRef::Library(to.clone());
            if list == ListName::Presets && closes_cycle(catalog, own, &target) {
                return Err(LibraryRefusal::new(
                    RefusalClass::InvalidArgument,
                    PRESET_CYCLE,
                    json!({ "path": own.as_str(), "target": to.as_str() }),
                ));
            }
            repointed.insert(key(path));
            entry.target = target;
            changed = true;
        }
        if changed {
            // ADR-0339
            let mut seen = HashSet::new();
            entries.retain(|entry| seen.insert(ref_key(&entry.target)));
        }
    }
    Ok((lists, repointed))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, OnceLock, Weak};

    use engram_dashboard_agent::profile::{
        AgentCommand, AgentProfile, AttachedPreset, ProfileRegistry, ProfileStore, StoreError,
    };
    use engram_dashboard_agent::profile_edit::{ProfileEditError, ProfileEditPort, WorkspaceLock};
    use engram_dashboard_base::sync::lock;
    use engram_dashboard_platform::fs::{link_dir, normalize_folder, unlink_dir};
    use tempfile::TempDir;

    use super::*;
    use crate::agent_edit::PROFILE_EDIT_UNAVAILABLE;
    use crate::backends::AgentBackends;
    use crate::config::{LibraryConfig, RootSetting};
    use crate::edit::LIBRARY_WRITE_FAILED;
    use crate::folder::{BoundedIo, LaunchWaits, StdFolderOps, ThreadSpawn, IO_LIMIT, LIST_CAP};
    use crate::format::read::ITEM_PATH_INVALID;
    use crate::format::YAML_SYNTAX;
    use crate::gate::NOT_FOUND;
    use crate::launch::LIBRARY_READ_FAILED;
    use crate::store::{ContentTag, FsStore, LibraryStore, ScanListing, TrashBundle};
    use crate::{LibraryValues, LIBRARY_UNAVAILABLE, PATH_THROUGH_LINK, ROOT_GENERATION_STALE};

    /// The ordered log of preset writes and agent rewrites, each with whether the library write lock was held.
    #[derive(Default)]
    struct Probe {
        svc: OnceLock<Weak<LibraryService>>,
        events: Mutex<Vec<(String, bool)>>,
    }

    impl Probe {
        /// Logs `event`; a read lock lets `try_read` through, so only the write lock counts as held.
        fn note(&self, event: String) {
            let held = self.svc.get().and_then(Weak::upgrade).is_some_and(|svc| {
                let lib = svc.gate.lib();
                let held = lib.try_write().is_err() && lib.try_read().is_err();
                held
            });
            lock(&self.events).push((event, held));
        }

        fn events(&self) -> Vec<(String, bool)> {
            lock(&self.events).clone()
        }
    }

    /// [`FsStore`] that logs each write by library path and fails the read or the write of one file when a test sets it.
    struct Disk {
        probe: Arc<Probe>,
        root: PathBuf,
        fail_read: Mutex<Option<PathBuf>>,
        fail_write: Mutex<Option<PathBuf>>,
    }

    impl LibraryStore for Disk {
        fn scan(
            &self,
            root: &Path,
            under: Option<&LibPath>,
            seq: &dyn Fn() -> u64,
        ) -> io::Result<ScanListing> {
            FsStore.scan(root, under, seq)
        }
        fn first_link(&self, root: &Path, rel: &LibPath) -> io::Result<Option<LibPath>> {
            FsStore.first_link(root, rel)
        }
        fn read(&self, path: &Path, cap: u64) -> io::Result<Vec<u8>> {
            if lock(&self.fail_read).as_deref() == Some(path) {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            FsStore.read(path, cap)
        }
        fn tag(&self, path: &Path) -> io::Result<ContentTag> {
            FsStore.tag(path)
        }
        fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            let rel = LibPath::from_disk(&self.root, path).expect("a library file");
            self.probe.note(format!("write {}", rel.as_str()));
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

    /// A profile-edit port that logs each agent rewrite (and its renames) and fails the next one when a test sets it.
    struct Port {
        probe: Arc<Probe>,
        renames: Mutex<Vec<Vec<PresetPathRename>>>,
        fail: Mutex<Option<ProfileEditError>>,
    }

    impl ProfileEditPort for Port {
        fn set_attachments(&self, _: AgentId, _: Attachments) -> Result<(), ProfileEditError> {
            unreachable!("not a relink call")
        }

        fn rewrite_preset_paths(
            &self,
            renames: &[PresetPathRename],
        ) -> Result<Vec<AgentId>, ProfileEditError> {
            self.probe.note("agents".to_owned());
            lock(&self.renames).push(renames.to_vec());
            match lock(&self.fail).take() {
                Some(failure) => Err(failure),
                None => Ok(Vec::new()),
            }
        }

        fn remove_preset_paths(&self, _: &[String]) -> Result<Vec<AgentId>, ProfileEditError> {
            unreachable!("not a relink call")
        }

        fn workspace_lock(&self, _: AgentId) -> Result<Option<WorkspaceLock>, ProfileEditError> {
            unreachable!("not a relink call")
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

    /// A temp folder holding the library root `lib\` (published when `root`), a registry over the profiles, and the
    /// port (plugged when `plugged`).
    struct World {
        _temp: TempDir,
        base: PathBuf,
        /// The links a test made inside the library — removed on drop.
        links: Mutex<Vec<PathBuf>>,
        probe: Arc<Probe>,
        disk: Arc<Disk>,
        port: Arc<Port>,
        svc: Arc<LibraryService>,
    }

    impl World {
        fn new() -> World {
            World::make(Vec::new(), true, true)
        }

        fn with(profiles: Vec<AgentProfile>) -> World {
            World::make(profiles, true, true)
        }

        fn make(profiles: Vec<AgentProfile>, root: bool, plugged: bool) -> World {
            let temp = tempfile::tempdir().unwrap();
            let base = dunce::canonicalize(temp.path()).unwrap();
            fs::create_dir_all(base.join("lib")).unwrap();
            let state = base.join("state");
            fs::create_dir_all(&state).unwrap();
            let config = match root {
                true => LibraryConfig::usable(&state, RootSetting::Explicit(base.join("lib"))),
                false => LibraryConfig::missing(&state),
            };
            let values = LibraryValues {
                registry: Arc::new(ProfileRegistry::new(Arc::new(Fixed(profiles)))),
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
                IO_LIMIT,
                LaunchWaits::LAUNCH,
                LIST_CAP,
                Arc::new(StdFolderOps),
                Arc::new(ThreadSpawn),
            );
            let probe = Arc::new(Probe::default());
            let disk = Arc::new(Disk {
                probe: Arc::clone(&probe),
                root: base.join("lib"),
                fail_read: Mutex::new(None),
                fail_write: Mutex::new(None),
            });
            let svc = Arc::new(LibraryService::new(
                values,
                Arc::clone(&disk) as Arc<dyn LibraryStore>,
                io,
            ));
            let _ = probe.svc.set(Arc::downgrade(&svc));
            let port = Arc::new(Port {
                probe: Arc::clone(&probe),
                renames: Mutex::new(Vec::new()),
                fail: Mutex::new(None),
            });
            if plugged {
                let weak: Weak<Port> = Arc::downgrade(&port);
                svc.plug_profile_edit(weak);
            }
            World {
                _temp: temp,
                base,
                links: Mutex::new(Vec::new()),
                probe,
                disk,
                port,
                svc,
            }
        }

        fn lib(&self, rel: &str) -> PathBuf {
            LibPath::parse(rel).unwrap().to_path(&self.base.join("lib"))
        }

        fn put(&self, rel: &str, text: &str) {
            let path = self.lib(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        fn text(&self, rel: &str) -> String {
            fs::read_to_string(self.lib(rel)).unwrap()
        }

        fn scan(&self) {
            self.svc.first_scan_join().unwrap();
        }

        fn relink(&self, from: &str, to: &str) -> Result<RelinkAnswer, LibraryRefusal> {
            self.svc.relink(from, to, None)
        }

        fn writes(&self) -> usize {
            self.probe
                .events()
                .iter()
                .filter(|(event, _)| event.starts_with("write "))
                .count()
        }

        fn renames(&self) -> Vec<Vec<PresetPathRename>> {
            lock(&self.port.renames).clone()
        }

        /// Replaces library folder `rel` (already scanned) with a link to an outside folder holding `files`; answers
        /// that folder.
        fn swap_for_a_link(&self, rel: &str, files: &[(&str, &str)]) -> PathBuf {
            fs::remove_dir_all(self.lib(rel)).unwrap();
            let outside = self.base.join(format!("outside-{}", rel.replace('/', "-")));
            fs::create_dir_all(&outside).unwrap();
            for (name, text) in files {
                fs::write(outside.join(name), text).unwrap();
            }
            link_dir(&outside, &self.lib(rel)).unwrap();
            lock(&self.links).push(self.lib(rel));
            outside
        }

        /// The doc of preset `preset` as the catalog holds it.
        fn doc(&self, preset: &str) -> PresetDoc {
            self.svc.with_read(|state| {
                match state
                    .catalog
                    .get(&key(&LibPath::parse(preset).unwrap()))
                    .map(|item| &item.content)
                {
                    Some(ItemContent::Preset { doc, .. }) => doc.clone(),
                    other => panic!("{preset} is no preset: {other:?}"),
                }
            })
        }

        /// The entries of `preset`'s `list` as the catalog holds them: spelling and tag.
        fn listed(&self, preset: &str, list: ListName) -> Vec<(String, Option<String>)> {
            doc_list(&self.doc(preset), list)
                .iter()
                .map(|item| (item.target.spelling(), item.backend.clone()))
                .collect()
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            for link in lock(&self.links).iter() {
                if let Err(e) = unlink_dir(link) {
                    eprintln!("could not remove the test link {}: {e}", link.display());
                }
            }
        }
    }

    fn preset(body: &str) -> String {
        format!("kind: preset\nversion: 1\n{body}")
    }

    fn rename(from: &str, to: &str) -> PresetPathRename {
        PresetPathRename {
            from: from.to_owned(),
            to: to.to_owned(),
        }
    }

    fn entry(path: &str) -> (String, Option<String>) {
        (path.to_owned(), None)
    }

    fn fixed(answer: &RelinkAnswer) -> Vec<(&str, &str)> {
        answer
            .fixed
            .iter()
            .map(|(from, to)| (from.as_str(), to.as_str()))
            .collect()
    }

    fn skipped(answer: &RelinkAnswer) -> Vec<(&str, &str)> {
        answer
            .skipped
            .iter()
            .map(|skip| (skip.path.as_str(), skip.reason.code.as_str()))
            .collect()
    }

    fn refused<T: std::fmt::Debug>(answer: Result<T, LibraryRefusal>) -> (RefusalClass, Reason) {
        match answer {
            Err(LibraryRefusal::Refused { class, reason }) => (class, reason),
            other => panic!("expected a refusal: {other:?}"),
        }
    }

    fn code<T: std::fmt::Debug>(answer: Result<T, LibraryRefusal>) -> (RefusalClass, String) {
        let (class, reason) = refused(answer);
        (class, reason.code)
    }

    fn stop_code(answer: &RelinkAnswer) -> (Option<&str>, RefusalClass, &str) {
        let stop = answer.stopped.as_ref().expect("a stop");
        match &stop.refusal {
            LibraryRefusal::Refused { class, reason } => {
                (stop.path.as_deref(), *class, reason.code.as_str())
            }
            other => panic!("not a refusal: {other:?}"),
        }
    }

    fn store_failure() -> ProfileEditError {
        ProfileEditError::Store(StoreError::Io(io::Error::other("disk full")))
    }

    // ── re-pointing: (a), (b), order ────────────────────────────────────────────────────────────────────────────

    #[test]
    fn every_reference_is_re_pointed_preset_files_first_then_the_agents_once_inside_the_change() {
        let w = World::with(vec![agent(&["old/X.yaml"])]);
        w.put("new/X.yaml", &preset(""));
        w.put("P.yaml", &preset("presets:\n- old/X.yaml\n"));
        w.put("Q.yaml", &preset("presets:\n- old/X.yaml\n"));

        let answer = w.relink("old/X.yaml", "new/X.yaml").unwrap();

        assert_eq!(fixed(&answer), [("old/X.yaml", "new/X.yaml")]);
        assert_eq!(
            (answer.more, answer.written.as_slice()),
            (0, ["P.yaml".to_owned(), "Q.yaml".to_owned()].as_slice())
        );
        assert!(answer.skipped.is_empty() && answer.stopped.is_none());
        assert_eq!(
            w.probe.events(),
            [
                ("write P.yaml".to_owned(), true),
                ("write Q.yaml".to_owned(), true),
                ("agents".to_owned(), true),
            ],
            "presets first, then the agents once, all inside the change"
        );
        assert_eq!(w.renames(), [vec![rename("old/X.yaml", "new/X.yaml")]]);
        for file in ["P.yaml", "Q.yaml"] {
            assert!(!w.text(file).contains("old/X.yaml"), "{file}");
            assert_eq!(
                w.listed(file, ListName::Presets),
                [entry("new/X.yaml")],
                "the catalog holds the written bytes"
            );
        }
    }

    #[test]
    fn a_missing_from_also_links_the_other_missing_lines_of_its_old_folder_by_name_and_kind() {
        let w = World::new();
        w.put("new/a.md", "a");
        w.put("new/b.md", "b");
        w.put("new/d.md", "d");
        // Named like the missing settings line, but a preset.
        w.put("new/s.yaml", &preset(""));
        w.put(
            "P.yaml",
            &preset(
                "instructions:\n- old/a.md\n- old/b.md\n- old/c.md\n- other/d.md\nsettings:\n- old/s.yaml\n",
            ),
        );

        let answer = w.relink("old/a.md", "new/a.md").unwrap();

        assert_eq!(
            fixed(&answer),
            [("old/a.md", "new/a.md"), ("old/b.md", "new/b.md")]
        );
        assert_eq!(answer.more, 1);
        assert_eq!(
            w.listed("P.yaml", ListName::Instructions),
            [
                entry("new/a.md"),
                entry("new/b.md"),
                entry("old/c.md"),
                entry("other/d.md")
            ],
            "no file of that name in the new folder, or another old folder: left as it is"
        );
        assert_eq!(
            w.listed("P.yaml", ListName::Settings),
            [entry("old/s.yaml")],
            "same name, other kind: left as it is"
        );
    }

    #[test]
    fn a_present_from_re_points_only_its_own_references() {
        let w = World::new();
        w.put("old/a.md", "a");
        w.put("new/a.md", "a2");
        w.put("new/b.md", "b");
        w.put("P.yaml", &preset("instructions:\n- old/a.md\n- old/b.md\n"));

        let answer = w.relink("old/a.md", "new/a.md").unwrap();

        assert_eq!(fixed(&answer), [("old/a.md", "new/a.md")]);
        assert_eq!(answer.more, 0);
        assert_eq!(
            w.listed("P.yaml", ListName::Instructions),
            [entry("new/a.md"), entry("old/b.md")]
        );
    }

    #[test]
    fn a_touched_list_keeps_the_first_place_of_each_target_and_folder_entries_stay_as_written() {
        let w = World::new();
        w.put("new/a.md", "a");
        let outside = w.base.join("outside");
        fs::create_dir_all(&outside).unwrap();
        // Not normalized: `/` separators and a trailing one.
        let folder = format!(
            "{}/",
            normalize_folder(&outside)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        );
        w.put(
            "P.yaml",
            &preset(&format!(
                "instructions:\n- path: old/a.md\n  backend: codex\n  note: kept\n- new/a.md\nfolders:\n- '{folder}'\n"
            )),
        );
        w.put(
            "Q.yaml",
            &preset("instructions:\n- new/a.md\n- path: old/a.md\n  backend: codex\n"),
        );

        let answer = w.relink("old/a.md", "new/a.md").unwrap();

        assert_eq!(answer.written, ["P.yaml", "Q.yaml"]);
        assert_eq!(
            w.listed("P.yaml", ListName::Instructions),
            [("new/a.md".to_owned(), Some("codex".to_owned()))],
            "the re-pointed entry was first: it stays, with its tag"
        );
        assert!(
            w.text("P.yaml").contains("note: kept"),
            "its other keys survive"
        );
        assert_eq!(
            w.listed("Q.yaml", ListName::Instructions),
            [entry("new/a.md")],
            "the entry already there was first: it stays"
        );
        let folders: Vec<String> = w
            .doc("P.yaml")
            .folders
            .iter()
            .map(|entry| entry.raw.clone())
            .collect();
        assert_eq!(folders, [folder], "the folder entry is untouched");
    }

    // ── refusals: nothing applied ───────────────────────────────────────────────────────────────────────────────

    #[test]
    fn to_must_be_in_the_catalog_and_of_the_kind_every_list_referencing_from_needs() {
        let w = World::with(vec![agent(&["old/Y.yaml"])]);
        w.put("new/a.md", "a");
        w.put("new/X.yaml", &preset(""));
        w.put("P.yaml", &preset("instructions:\n- old/a.md\n"));

        assert_eq!(
            code(w.relink("old/a.md", "new/gone.md")),
            (RefusalClass::NotFound, NOT_FOUND.to_owned())
        );
        let (class, reason) = refused(w.relink("old/a.md", "new/X.yaml"));
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::InvalidArgument, RELINK_KIND_MISMATCH)
        );
        assert_eq!(
            reason.args,
            json!({ "from": "old/a.md", "to": "new/X.yaml", "expected": "instructions", "found": "presets" })
        );
        let (_, reason) = refused(w.relink("old/Y.yaml", "new/a.md"));
        assert_eq!(
            (
                reason.code.as_str(),
                &reason.args["expected"],
                &reason.args["found"]
            ),
            (
                RELINK_KIND_MISMATCH,
                &json!("presets"),
                &json!("instructions")
            ),
            "an agent attaching from needs a preset"
        );
        assert_eq!(w.writes(), 0);
        assert!(w.renames().is_empty());
        assert!(w.text("P.yaml").contains("old/a.md"));
    }

    #[test]
    fn from_and_to_must_be_library_paths() {
        let w = World::new();
        w.put("new/a.md", "a");
        for (from, to) in [
            ("engram:/default", "new/a.md"),
            ("old/a.md", "engram:/priming"),
            ("old//a.md", "new/a.md"),
            ("old/a.md", "/new/a.md"),
        ] {
            assert_eq!(
                code(w.relink(from, to)),
                (RefusalClass::InvalidArgument, ITEM_PATH_INVALID.to_owned()),
                "{from} -> {to}"
            );
        }
    }

    #[test]
    fn link_admission_on_to_refuses_before_anything_is_written() {
        let w = World::new();
        w.put("team/a.md", "a");
        w.put("P.yaml", &preset("instructions:\n- old/a.md\n"));
        w.scan();
        // The catalog still holds `team/a.md`; the folder is a link now.
        w.swap_for_a_link("team", &[("a.md", "theirs")]);
        let before = w.text("P.yaml");

        assert_eq!(
            code(w.relink("old/a.md", "team/a.md")),
            (RefusalClass::InvalidArgument, PATH_THROUGH_LINK.to_owned())
        );
        assert_eq!(w.writes(), 0);
        assert_eq!(w.text("P.yaml"), before);
    }

    #[test]
    fn with_agents_to_rewrite_an_empty_port_cell_refuses_before_any_file_is_written() {
        let w = World::make(vec![agent(&["old/X.yaml"])], true, false);
        w.put("new/X.yaml", &preset(""));
        w.put("P.yaml", &preset("presets:\n- old/X.yaml\n"));

        assert_eq!(
            code(w.relink("old/X.yaml", "new/X.yaml")),
            (RefusalClass::Internal, PROFILE_EDIT_UNAVAILABLE.to_owned())
        );
        assert_eq!(w.writes(), 0);
    }

    #[test]
    fn with_no_root_a_relink_answers_library_unavailable() {
        let w = World::make(Vec::new(), false, true);
        assert_eq!(
            code(w.relink("old/a.md", "new/a.md")),
            (RefusalClass::Unavailable, LIBRARY_UNAVAILABLE.to_owned())
        );
    }

    #[test]
    fn a_relink_never_crosses_roots_one_carrying_the_old_root_generation_writes_nothing() {
        let w = World::new();
        w.put("new/a.md", "a");
        w.put("P.yaml", &preset("instructions:\n- old/a.md\n"));
        w.scan();
        let generation = w.svc.status().root_generation;
        let other = w.base.join("other");
        fs::create_dir_all(other.join("new")).unwrap();
        fs::write(other.join("new").join("a.md"), "a").unwrap();
        fs::write(other.join("P.yaml"), preset("instructions:\n- old/a.md\n")).unwrap();
        w.svc.set_root(&other, false).unwrap();

        assert_eq!(
            code(w.svc.relink("old/a.md", "new/a.md", Some(generation))),
            (RefusalClass::Conflict, ROOT_GENERATION_STALE.to_owned())
        );
        assert_eq!(w.writes(), 0);
        assert!(w.text("P.yaml").contains("old/a.md"));
        assert!(fs::read_to_string(other.join("P.yaml"))
            .unwrap()
            .contains("old/a.md"));
    }

    // ── part-way: skips and stops ───────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_per_file_refusal_is_skipped_and_the_walk_and_the_agent_rewrite_go_on() {
        let w = World::with(vec![agent(&["old/X.yaml"])]);
        w.put("new/X.yaml", &preset(""));
        w.put("new/Y.yaml", &preset(""));
        let lists_x = preset("presets:\n- old/X.yaml\n");
        for file in ["A.yaml", "B.yaml", "team/T.yaml"] {
            w.put(file, &lists_x);
        }
        // Reads, but the writer cannot write it back exactly.
        w.put("D.yaml", &preset("big: 1e20\npresets:\n- old/X.yaml\n"));
        w.put("C.yaml", &preset("presets:\n- old/X.yaml\n- old/Y.yaml\n"));
        w.scan();
        // After the scan: A is gone, B no longer reads, `team` is a link.
        fs::remove_file(w.lib("A.yaml")).unwrap();
        let broken = "kind: preset\nversion: 1\npresets: [old/X.yaml\n";
        fs::write(w.lib("B.yaml"), broken).unwrap();
        let outside = w.swap_for_a_link("team", &[("T.yaml", &lists_x)]);

        let answer = w.relink("old/X.yaml", "new/X.yaml").unwrap();

        assert_eq!(
            skipped(&answer),
            [
                ("A.yaml", NOT_FOUND),
                ("B.yaml", YAML_SYNTAX),
                ("D.yaml", "preset-write-number-range"),
                ("team/T.yaml", PATH_THROUGH_LINK),
            ]
        );
        assert_eq!(answer.written, ["C.yaml"], "the walk went on");
        assert_eq!(
            w.renames(),
            [vec![rename("old/X.yaml", "new/X.yaml")]],
            "the agents were rewritten"
        );
        assert!(answer.stopped.is_none());
        assert_eq!(
            (fixed(&answer), answer.more),
            (vec![("old/Y.yaml", "new/Y.yaml")], 1),
            "a pair counts as fixed only when every file listing it was"
        );
        assert_eq!(w.text("B.yaml"), broken);
        assert_eq!(
            fs::read_to_string(outside.join("T.yaml")).unwrap(),
            lists_x,
            "nothing written through the link"
        );
    }

    #[test]
    fn an_io_failure_stops_the_walk_skips_the_agent_rewrite_and_answers_what_was_applied() {
        for failure in ["read", "write"] {
            let w = World::with(vec![agent(&["old/X.yaml"])]);
            w.put("new/X.yaml", &preset(""));
            w.put("new/Y.yaml", &preset(""));
            w.put("new/Z.yaml", &preset(""));
            w.put("A.yaml", &preset("presets:\n- old/X.yaml\n- old/Y.yaml\n"));
            w.put("B.yaml", &preset("presets:\n- old/X.yaml\n"));
            // Z is listed only in a file the walk does not reach.
            w.put("C.yaml", &preset("presets:\n- old/X.yaml\n- old/Z.yaml\n"));
            w.scan();
            let (slot, expected) = match failure {
                "read" => (&w.disk.fail_read, LIBRARY_READ_FAILED),
                _ => (&w.disk.fail_write, LIBRARY_WRITE_FAILED),
            };
            *lock(slot) = Some(w.lib("B.yaml"));

            let answer = w.relink("old/X.yaml", "new/X.yaml").unwrap();

            assert_eq!(
                stop_code(&answer),
                (Some("B.yaml"), RefusalClass::Unavailable, expected),
                "{failure}"
            );
            assert_eq!(answer.written, ["A.yaml"], "{failure}");
            assert!(answer.skipped.is_empty(), "{failure}");
            assert!(
                w.renames().is_empty(),
                "{failure}: the agents are not rewritten after a stop"
            );
            assert_eq!(
                (fixed(&answer), answer.more),
                (vec![("old/Y.yaml", "new/Y.yaml")], 1),
                "{failure}: only the pair whose every file was written"
            );
            assert!(
                w.text("C.yaml").contains("old/X.yaml"),
                "{failure}: not reached"
            );
            assert!(w.text("B.yaml").contains("old/X.yaml"), "{failure}");
        }
    }

    #[test]
    fn a_stop_before_any_file_was_written_answers_that_failures_refusal() {
        let w = World::with(vec![agent(&["old/X.yaml"])]);
        w.put("new/X.yaml", &preset(""));
        w.put("P.yaml", &preset("presets:\n- old/X.yaml\n"));
        w.scan();
        *lock(&w.disk.fail_write) = Some(w.lib("P.yaml"));

        assert_eq!(
            code(w.relink("old/X.yaml", "new/X.yaml")),
            (RefusalClass::Unavailable, LIBRARY_WRITE_FAILED.to_owned())
        );
        assert!(w.renames().is_empty());
        assert!(w.text("P.yaml").contains("old/X.yaml"));
    }

    #[test]
    fn a_failed_agent_rewrite_is_a_stop_with_no_path_or_the_refusal_when_no_file_was_written() {
        let w = World::with(vec![agent(&["old/X.yaml"])]);
        w.put("new/X.yaml", &preset(""));
        w.put("P.yaml", &preset("presets:\n- old/X.yaml\n"));
        *lock(&w.port.fail) = Some(store_failure());

        let answer = w.relink("old/X.yaml", "new/X.yaml").unwrap();

        assert_eq!(answer.written, ["P.yaml"]);
        let stop = answer.stopped.as_ref().expect("a stop");
        assert!(stop.path.is_none());
        assert!(
            matches!(stop.refusal, LibraryRefusal::Store(_)),
            "passed through: {stop:?}"
        );
        assert!(answer.fixed.is_empty(), "its agents were not rewritten");

        let agents_only = World::with(vec![agent(&["old/X.yaml"])]);
        agents_only.put("new/X.yaml", &preset(""));
        *lock(&agents_only.port.fail) = Some(store_failure());
        let answer = agents_only.relink("old/X.yaml", "new/X.yaml");
        assert!(
            matches!(answer, Err(LibraryRefusal::Store(_))),
            "{answer:?}"
        );
    }

    // ── FIX round 1 ─────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_reference_is_re_pointed_only_in_the_list_its_new_target_goes_into() {
        let w = World::new();
        w.put("new/a.md", "a");
        w.put("new/b.md", "b");
        w.put("P.yaml", &preset("instructions:\n- old/a.md\n- old/b.md\n"));
        w.scan();
        // Edited outside Engram after the scan: both old paths in a list an instruction does not fit too.
        w.put(
            "P.yaml",
            &preset("instructions:\n- old/a.md\n- old/b.md\nskills:\n- old/a.md\n- old/b.md\n"),
        );

        let answer = w.relink("old/a.md", "new/a.md").unwrap();

        assert_eq!(answer.written, ["P.yaml"]);
        assert_eq!(answer.more, 1);
        assert_eq!(
            w.listed("P.yaml", ListName::Instructions),
            [entry("new/a.md"), entry("new/b.md")]
        );
        assert_eq!(
            w.listed("P.yaml", ListName::Skills),
            [entry("old/a.md"), entry("old/b.md")],
            "left as it is, for the picked pair and the part (b) one alike"
        );
    }

    #[test]
    fn a_pair_nothing_was_re_pointed_for_is_not_fixed() {
        let w = World::new();
        w.put("new/a.md", "a");
        w.put("P.yaml", &preset("instructions:\n- old/a.md\n"));
        w.scan();
        // Moved outside Engram after the scan into a list an instruction does not fit.
        let moved = preset("skills:\n- old/a.md\n");
        w.put("P.yaml", &moved);

        let answer = w.relink("old/a.md", "new/a.md").unwrap();

        assert!(answer.fixed.is_empty(), "{:?}", answer.fixed);
        assert!(answer.written.is_empty() && answer.skipped.is_empty());
        assert_eq!(w.text("P.yaml"), moved);
    }

    #[test]
    fn a_rewrite_that_would_make_a_preset_reach_itself_is_skipped_preset_cycle() {
        let lists_x = preset("presets:\n- old/X.yaml\n");
        // `to` is the file itself.
        let w = World::new();
        w.put("P.yaml", &lists_x);
        w.put("Q.yaml", &lists_x);

        let answer = w.relink("old/X.yaml", "P.yaml").unwrap();

        assert_eq!(skipped(&answer), [("P.yaml", PRESET_CYCLE)]);
        assert_eq!(answer.written, ["Q.yaml"]);
        assert!(answer.fixed.is_empty());
        assert_eq!(w.text("P.yaml"), lists_x);

        // `to` reaches the file.
        let w = World::new();
        w.put("P.yaml", &lists_x);
        w.put("Q.yaml", &preset("presets:\n- P.yaml\n"));
        w.put("S.yaml", &lists_x);

        let answer = w.relink("old/X.yaml", "Q.yaml").unwrap();

        assert_eq!(skipped(&answer), [("P.yaml", PRESET_CYCLE)]);
        assert_eq!(answer.written, ["S.yaml"]);
        assert_eq!(w.text("P.yaml"), lists_x);
    }

    #[test]
    fn part_b_skips_a_candidate_at_or_below_a_link() {
        let skill = "---\nname: s\ndescription: d\n---\n";
        let w = World::new();
        w.put("team/s/SKILL.md", skill);
        w.put("team/t/SKILL.md", skill);
        w.put("P.yaml", &preset("skills:\n- old/s\n- old/t\n"));
        w.scan();
        // The catalog still holds the skill `team/t`; it is a link now.
        w.swap_for_a_link("team/t", &[("SKILL.md", skill)]);

        let answer = w.relink("old/s", "team/s").unwrap();

        assert_eq!(
            (fixed(&answer), answer.more),
            (vec![("old/s", "team/s")], 0)
        );
        assert_eq!(
            w.listed("P.yaml", ListName::Skills),
            [entry("team/s"), entry("old/t")]
        );
    }

    #[test]
    fn part_b_does_not_run_for_a_from_the_catalog_holds_as_a_folder() {
        let w = World::new();
        // `old/sub` is a plain folder (a skill that lost its `SKILL.md`).
        w.put("old/sub/notes.md", "n");
        w.put("new/sub/SKILL.md", "---\nname: sub\ndescription: d\n---\n");
        w.put("new/b.md", "b");
        w.put(
            "P.yaml",
            &preset("instructions:\n- old/b.md\nskills:\n- old/sub\n"),
        );

        let answer = w.relink("old/sub", "new/sub").unwrap();

        assert_eq!(
            (fixed(&answer), answer.more),
            (vec![("old/sub", "new/sub")], 0)
        );
        assert_eq!(
            w.listed("P.yaml", ListName::Instructions),
            [entry("old/b.md")]
        );
    }

    #[test]
    fn a_part_b_pair_is_spelled_as_the_catalog_spells_its_new_target() {
        let w = World::new();
        w.put("new/a.md", "a");
        w.put("new/b.md", "b");
        w.put("P.yaml", &preset("instructions:\n- old/a.md\n- old/B.md\n"));

        let answer = w.relink("old/a.md", "new/a.md").unwrap();

        // Whether `new/B.md` names `new/b.md` is the platform's key rule.
        let names_it =
            key(&LibPath::parse("new/B.md").unwrap()) == key(&LibPath::parse("new/b.md").unwrap());
        match names_it {
            true => {
                assert_eq!(
                    fixed(&answer),
                    [("old/a.md", "new/a.md"), ("old/B.md", "new/b.md")]
                );
                assert_eq!(
                    w.listed("P.yaml", ListName::Instructions),
                    [entry("new/a.md"), entry("new/b.md")]
                );
            }
            false => {
                assert_eq!(fixed(&answer), [("old/a.md", "new/a.md")]);
                assert_eq!(
                    w.listed("P.yaml", ListName::Instructions),
                    [entry("new/a.md"), entry("old/B.md")]
                );
            }
        }
    }

    #[test]
    fn part_b_also_links_a_missing_preset_an_agent_attaches() {
        let w = World::with(vec![agent(&["old/Y.yaml"])]);
        w.put("new/X.yaml", &preset(""));
        w.put("new/Y.yaml", &preset(""));
        w.put("P.yaml", &preset("presets:\n- old/X.yaml\n"));

        let answer = w.relink("old/X.yaml", "new/X.yaml").unwrap();

        assert_eq!(
            (fixed(&answer), answer.more),
            (
                vec![("old/X.yaml", "new/X.yaml"), ("old/Y.yaml", "new/Y.yaml")],
                1
            )
        );
        assert_eq!(w.renames(), [vec![rename("old/Y.yaml", "new/Y.yaml")]]);
    }

    #[test]
    fn the_agent_half_of_fixed_follows_the_stored_spelling() {
        // Whether the two spellings name one path is the platform's key rule.
        let folds =
            key(&LibPath::parse("P.yaml").unwrap()) == key(&LibPath::parse("p.yaml").unwrap());

        // The agent already holds the target spelling: nothing changes, so nothing is fixed.
        let w = World::with(vec![agent(&["p.yaml"])]);
        w.put("p.yaml", &preset(""));
        let answer = w.relink("P.yaml", "p.yaml").unwrap();
        assert!(answer.fixed.is_empty(), "{:?}", answer.fixed);
        let expected_calls = match folds {
            true => vec![vec![rename("P.yaml", "p.yaml")]],
            false => Vec::new(),
        };
        assert_eq!(w.renames(), expected_calls);

        // The agent holds another spelling of the same path: the rewrite normalizes it, so the pair is fixed.
        let w = World::with(vec![agent(&["P.yaml"])]);
        w.put("P.yaml", &preset(""));
        let answer = w.relink("p.yaml", "p.yaml");
        match folds {
            true => {
                let answer = answer.unwrap();
                assert_eq!(fixed(&answer), [("p.yaml", "p.yaml")]);
                assert_eq!(w.renames(), [vec![rename("p.yaml", "p.yaml")]]);
            }
            false => assert_eq!(
                code(answer),
                (RefusalClass::NotFound, NOT_FOUND.to_owned()),
                "another file on a case-sensitive system"
            ),
        }
    }
}
