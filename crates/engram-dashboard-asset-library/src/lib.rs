//! # engram-dashboard-asset-library — the preset library
//!
//! The library holds instruction, skill, settings and preset files under one root folder. This crate reads, checks and
//! resolves them and makes the launch materials; the daemon creates the service, plugs its ports in and passes it values,
//! nothing more (ADR-0311).
//!
//! ## Dependency rule (invariant)
//!
//! - Among workspace crates exactly agent · platform · command · base (ADR-0311 decision 2). agent defines the ports this
//!   crate implements or calls; agent does not know this crate. This crate calls no daemon internals.
//! - No test feature (ADR-0311 decision 4): fakes are `#[cfg(test)]` inside the crate.
//! - No network client, no process runner — settings are not schema-checked, so nothing is fetched or probed
//!   (ADR-0359).
//! - No OS `cfg`: OS behaviour comes from platform (the platform gates 4 · 6 file lists do not name this crate).
//!
//! ## Backend extensibility (invariant — TRD S25 step-7 §1-1)
//!
//! Adding a backend to agent (its `AgentCommand` variant, backend word and settings check in the registry) must make
//! tags, raw settings fields, the selection of what an agent loads, the hand-over checks and the notices work with no
//! change here. So:
//!
//! - A backend tag and a raw-field name are strings, accepted only when the registry knows the word (ASCII
//!   case-insensitive). There is no backend enum here.
//! - The registry is reached only through [`backends::Backends`]; [`backends::AgentBackends`] is the only code here that
//!   calls agent's settings-check registry.
//! - Every per-backend difference comes from a port answer; the code never branches on a backend word, and no backend
//!   name appears in the source outside tests, doc comments describing today's port answers and built-in content.
//!
//! ## Gates
//!
//! `.github/workflows/ci.yml` holds the commands and expected values (canonical — not copied here), as steps
//! `asset-library gate 1`–`4` (ADR-0311 decision 6 numbering; gate 3 is the shared messaging step): ① the test line
//! `cargo test -p engram-dashboard-asset-library` · ② the direct workspace dependency name set {agent · asset-library ·
//! base · command · platform} and no direct `tauri*` package · ③ `asset_library` in the messaging isolation regex ·
//! ④ zero `use tauri` lines under `src/`.
//!
//! Nothing calls this crate yet — the daemon wiring is step 8.
//!
//! ## The service
//!
//! [`LibraryService`] owns the catalog behind the ADR-0324 gate (`gate.rs` — the lock order and the ticket rules are
//! there). Library files under the root are read with the store directly (local by assumption); every folder operation
//! outside it goes through [`folder::BoundedIo`], never under the library lock.
// ADR-0311
// ADR-0324

pub mod backends;
pub mod catalog;
pub mod excerpt;
pub mod folder;
pub mod format;
mod gate;
pub mod path;
pub mod refs;
pub mod store;
pub mod validate;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use engram_dashboard_agent::launch::Reason;
use engram_dashboard_agent::profile::ProfileRegistry;
use engram_dashboard_base::sync::lock;
use engram_dashboard_platform::fs::{path_key, PathKey};
use serde_json::json;

use crate::backends::Backends;
use crate::catalog::{key, Catalog, Item, ItemContent, ItemKind, ScanItem, ScanResult};
use crate::folder::{BoundedIo, FolderStatus, Need};
use crate::gate::{Gate, Tags};
pub use crate::gate::{Numbers, State, NOT_FOUND};
use crate::path::LibPath;
use crate::refs::reference_count;
use crate::store::LibraryStore;

/// The largest library file a scan or the list safety net reads. A longer file is a failed read: its item is kept as
/// it was and the scan reports the error.
pub const LIBRARY_READ_CAP: u64 = 16 * 1024 * 1024;

/// The code of a path that lies at or below a link inside the library (`args` = `{path}`) — such a path is never read
/// or merged (U1).
// TRD S25 step-7 B8
pub const PATH_THROUGH_LINK: &str = "path-through-link";

/// The code of a catalog read that could not be answered because no full scan of the library could be applied — no
/// root is set, or the root could not be walked (`args` = `{message}`, the reason).
// TRD S25 step-7 B8
pub const LIBRARY_UNAVAILABLE: &str = "library-unavailable";

const MERGE_JOB: &str = "library-merge";

/// What the daemon hands the service.
pub struct LibraryValues {
    pub registry: Arc<ProfileRegistry>,
    /// `AgentBackends` in production.
    pub backends: Arc<dyn Backends>,
    /// The library root at construction; `None` = none is set (nothing is scanned). Read only by `new` — after a
    /// `change_root` the live root is the gate's, not this field. Step 9a replaces it with the library config.
    pub root: Option<PathBuf>,
    /// The built-in assets folder — read by the built-ins (step-7 P8).
    pub builtin_dir: Option<PathBuf>,
    /// The volatile launch folder — written by the launch materials (step-7 P9).
    pub launch_root: PathBuf,
    /// The home folder handed to the backend's native skill roots (step-7 P9).
    pub home: Option<PathBuf>,
    /// The daemon's environment; a profile's env applies on top per agent (step-7 P9).
    pub env: Arc<dyn Fn(&str) -> Option<String> + Send + Sync>,
}

/// The library service: the catalog, its scans and its reads (module header · `gate.rs`).
pub struct LibraryService {
    values: LibraryValues,
    store: Arc<dyn LibraryStore>,
    io: BoundedIo,
    gate: Arc<Gate>,
}

/// The answer of [`LibraryService::list`].
#[derive(Clone, Debug, PartialEq)]
pub struct ListAnswer {
    pub folder: Option<LibPath>,
    /// The items directly in the folder, by path — a skill folder is an item (kind = skill).
    pub rows: Vec<ListRow>,
    /// The plain folders directly in the folder (not skill folders), by path.
    pub folders: Vec<FolderRow>,
    /// The built-in rows — filled by the built-ins (step-7 P8); empty until then.
    pub builtins: Vec<ListRow>,
}

/// One plain subfolder line of a [`ListAnswer`].
#[derive(Clone, Debug, PartialEq)]
pub struct FolderRow {
    pub path: String,
    /// Its last name.
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListRow {
    pub path: String,
    pub kind: ItemKind,
    /// A skill's name; every other item's file name.
    pub name: String,
    /// A broken file's reason, or [`PATH_THROUGH_LINK`]; `None` = it reads.
    pub state: Option<Reason>,
    /// Presets that reach it plus agents that attach it (`refs::reference_count`).
    pub references: usize,
    /// An instruction's excerpt, a skill's description, a preset's or settings asset's `description`; else empty.
    pub description: String,
}

/// The answer of [`LibraryService::refresh`] — over the folder asked for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RefreshReport {
    /// Items inserted, replaced or removed (a skill counts its folder and its `SKILL.md`).
    pub changed: usize,
    /// The problems of the files re-read: a broken file's reason, a preset's line problems and the references it lists
    /// that the catalog does not hold ([`NOT_FOUND`]), a settings asset's refusals.
    pub problems: Vec<(LibPath, Reason)>,
    /// Re-read `.yaml` / `.yml` files that declare no `kind` — not listed.
    pub no_kind: Vec<LibPath>,
    /// Links below the root that were not entered.
    pub skipped_links: Vec<LibPath>,
    /// Where the walk or a read failed (`None` = the root), with the error; those items are kept as they were. When
    /// nothing could be scanned (no root, the root unreadable, or a refresh's own scan of a folder with a dot or temp
    /// name), the one entry is `(None, why)`.
    pub errors: Vec<(Option<LibPath>, String)>,
}

/// What one path on disk is, read the way the scan admits paths.
enum Observed {
    /// The items read at or below it (none when it is gone).
    Items(Vec<Item>),
    /// It is at or below a link inside the library — not read.
    ThroughLink,
    /// Its name is not admitted (a dot name, a temp name) — not read.
    Refused,
}

impl LibraryService {
    /// `io` bounds every folder operation outside the library and spawns the service's short-lived jobs.
    pub fn new(values: LibraryValues, store: Arc<dyn LibraryStore>, io: BoundedIo) -> Self {
        let gate = Arc::new(Gate::new(values.root.clone()));
        LibraryService {
            values,
            store,
            io,
            gate,
        }
    }

    /// The items and plain subfolders directly in `folder` (`None` = the root), after the first-scan entrance, under
    /// one read lock — no scan once the first scan is raised. `Err` = the first scan could not be applied
    /// ([`LIBRARY_UNAVAILABLE`] — ADR-0324 decision 6: that failure is the answer). A `folder` the catalog does not
    /// know is checked on disk alone and merged (the safety net — a late merge, ADR-0324 decision 5); one at or below
    /// a link answers a single [`PATH_THROUGH_LINK`] row; one with a dot or temp name answers no rows. A folder the
    /// catalog does not know answers its items only — its subfolder lines appear after a refresh.
    pub fn list(&self, folder: Option<&LibPath>) -> Result<ListAnswer, Reason> {
        self.first_scan_join().map_err(|why| Reason {
            code: LIBRARY_UNAVAILABLE.to_owned(),
            args: json!({ "message": why }),
            line: None,
        })?;
        // ADR-0324 decision 5: the numbers are noted in the read lock that reads the root the disk read uses, so a root
        //   change published after it moves the root generation and discards the merge.
        let known = self.gate.with_read(|state| match folder {
            Some(path) if !is_known(&state.catalog, path) => {
                Err((path, state.root.clone(), self.gate.numbers()))
            }
            _ => Ok(self.answer(state, folder)),
        });
        let (path, root, noted) = match known {
            Ok(answer) => return Ok(answer),
            Err(unknown) => unknown,
        };
        if let Some(root) = root {
            match self.observe(&root, path) {
                Observed::ThroughLink => {
                    return Ok(ListAnswer {
                        folder: folder.cloned(),
                        rows: vec![through_link_row(path)],
                        folders: Vec::new(),
                        builtins: Vec::new(),
                    })
                }
                Observed::Items(items) => {
                    self.gate.late_merge(noted, |catalog, may_insert| {
                        catalog.apply_items(items, may_insert)
                    });
                }
                Observed::Refused => {}
            }
        }
        Ok(self.gate.with_read(|state| self.answer(state, folder)))
    }

    /// Re-scans `folder` (`None` = the whole library) on its own ticket, or joins a covering scan queued and not yet
    /// walking (ADR-0324 decision 7). Before the first scan is raised a folder refresh passes through a full scan
    /// (decision 6): it joins a queued one or runs its own and takes its share; if a full scan (or a first-start root
    /// change) is already walking, it waits for it and then scans its folder like any refresh — or, when what it
    /// waited for failed, goes through a full scan once more.
    pub fn refresh(&self, folder: Option<&LibPath>) -> RefreshReport {
        match self.gate.refresh(folder, &|root, range, tags| {
            self.build_scan(root, range, tags)
        }) {
            Ok(report) => report.share(folder),
            Err(why) => failed(why),
        }
    }

    /// The first-scan entrance every method that reads the catalog passes first, before any ticket or lock (ADR-0324
    /// decision 6). `Err` = no full scan could be applied, with the reason; the catalog then holds only what late
    /// merges put in. The launch check does not pass it.
    pub fn first_scan_join(&self) -> Result<(), String> {
        self.gate
            .first_scan_join(&|root, range, tags| self.build_scan(root, range, tags))
    }

    /// `f` over the catalog and the root under one read lock. `f` may read the profile registry and nothing else
    /// outward; it must not call back into the service.
    pub fn with_read<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        self.gate.with_read(f)
    }

    /// The numbers to note before a disk read whose result will be merged late ([`LibraryService::late_merge`]) — and
    /// before reading the root that disk read uses, or inside the same read lock ([`LibraryService::with_read`]):
    /// noted after the root was read, they may already include a root change the read does not see.
    pub fn numbers(&self) -> Numbers {
        self.gate.numbers()
    }

    /// The conditional late merge (ADR-0324 decision 5) — `f(catalog, may_insert)` under the write lock unless a
    /// generation moved since `noted`; `may_insert` is false when a scan was applied in between. `f` applies per item
    /// (`Catalog::apply_items`) and returns the touched keys. Positive observations only — a late merge never removes.
    pub fn late_merge(
        &self,
        noted: Numbers,
        f: impl FnOnce(&mut Catalog, bool) -> Vec<PathKey>,
    ) -> Vec<PathKey> {
        self.gate.late_merge(noted, f)
    }

    /// Hands a late merge of `items` (read after `noted`) to a short-lived job that takes the write lock normally, so
    /// the caller never waits on it and a contended lock delays it without dropping it (TRD S25 step-7 B26). The work
    /// sits in a take-once slot outside the job: if the job cannot be started (it is dropped unrun), the merge runs
    /// here instead, exactly once — so call it holding no library guard (the in-place merge takes the write lock).
    pub fn hand_off_merge(&self, noted: Numbers, items: Vec<Item>) {
        let slot = Arc::new(Mutex::new(Some(items)));
        let job = {
            let (slot, gate) = (Arc::clone(&slot), Arc::clone(&self.gate));
            Box::new(move || merge_from(&gate, noted, &slot))
        };
        if let Err(e) = self.io.spawner().spawn(MERGE_JOB, job) {
            tracing::warn!("라이브러리 늦은 합치기: 작업을 띄우지 못해 이 자리에서 합친다: {e}");
            merge_from(&self.gate, noted, &slot);
        }
    }

    /// A change inside Engram (TRD S25 step-7 §6 writer 5 — step 9a and 9b edits): its own ticket, then `f` under the
    /// write lock with the catalog, the root and the sequence; `f` writes per item and returns the touched keys; the
    /// change generation is raised before the lock is released. A caller that reads the catalog passes the first-scan
    /// entrance before this call. The change's disk effect must be finished inside `f` (or before the call): a reader
    /// that notes the numbers after the raise assumes it sees the new disk.
    pub fn change<R>(
        &self,
        f: impl FnOnce(&mut Catalog, Option<&Path>, &dyn Fn() -> u64) -> (R, Vec<PathKey>),
    ) -> R {
        self.gate.change(f)
    }

    /// Moves the library to `new_root` (TRD S25 step-7 §6 writer 6 — step 9a): the new root is stat'ed first on one
    /// read-computation budget, outside the gate; then, exclusive behind every earlier ticket, it is walked with empty
    /// tags and published with a fresh catalog in one write lock. `Err` = refused, nothing published: the stat did not
    /// answer `Ok`, or the walk failed at the root.
    pub fn change_root(&self, new_root: PathBuf) -> Result<RefreshReport, String> {
        let io = self.io.begin_read();
        let status = self
            .io
            .stat_all(&io, &[(new_root.clone(), Need::Required)])
            .remove(&path_key(&new_root));
        match status {
            Some(FolderStatus::Ok) => {}
            other => return Err(format!("the new library root cannot be used: {other:?}")),
        }
        let walk = |root: &Path| self.build_scan(root, None, &Tags::new());
        self.gate
            .change_root(new_root, walk)
            .map(|report| report.share(None))
    }

    /// Walks and reads `range` under `root` outside every lock.
    fn build_scan(
        &self,
        root: &Path,
        range: Option<&LibPath>,
        tags: &Tags,
    ) -> io::Result<ScanResult> {
        // The racy rule's reference — taken before the walk (`ScanResult::build`).
        let now = SystemTime::now();
        let listing = self.store.scan(root, range, &|| self.gate.next_seq())?;
        let read = |path: &Path| self.store.read(path, LIBRARY_READ_CAP);
        Ok(ScanResult::build(
            listing,
            range,
            tags,
            &read,
            &*self.values.backends,
            now,
        ))
    }

    /// One path read through the same partial scan a refresh of it would run — so it admits exactly what the scan
    /// admits (names, links, kinds), and every read draws its number first.
    fn observe(&self, root: &Path, path: &LibPath) -> Observed {
        match self.build_scan(root, Some(path), &Tags::new()) {
            Err(e) => {
                tracing::debug!(
                    path = path.as_str(),
                    "라이브러리 안전망: 훑기가 받지 않는 이름이라 읽지 않았다: {e}"
                );
                Observed::Refused
            }
            Ok(result)
                if result
                    .skipped_links
                    .iter()
                    .any(|link| key(link) == key(path)) =>
            {
                Observed::ThroughLink
            }
            Ok(result) => Observed::Items(
                result
                    .entries
                    .into_iter()
                    .filter_map(|entry| match entry {
                        ScanItem::Fresh(item) => Some(item),
                        ScanItem::Unchanged(..) => None,
                    })
                    .collect(),
            ),
        }
    }

    fn answer(&self, state: &State, folder: Option<&LibPath>) -> ListAnswer {
        let rows = state
            .catalog
            .children(folder)
            .into_iter()
            .map(|item| {
                let references = reference_count(&state.catalog, &self.values.registry, &item.path);
                row(item, references)
            })
            .collect();
        let want = folder.map(key);
        let mut subfolders: Vec<&LibPath> = state
            .catalog
            .folders()
            .filter(|sub| sub.parent().as_ref().map(key) == want)
            .filter(|sub| state.catalog.get(&key(sub)).is_none())
            .collect();
        subfolders.sort();
        let folders = subfolders
            .into_iter()
            .map(|sub| FolderRow {
                path: sub.as_str().to_owned(),
                name: sub.file_name().to_owned(),
            })
            .collect();
        ListAnswer {
            folder: folder.cloned(),
            rows,
            folders,
            builtins: Vec::new(),
        }
    }
}

fn merge_from(gate: &Gate, noted: Numbers, slot: &Mutex<Option<Vec<Item>>>) {
    let work = lock(slot).take();
    if let Some(items) = work {
        gate.late_merge(noted, |catalog, may_insert| {
            catalog.apply_items(items, may_insert)
        });
    }
}

fn is_known(catalog: &Catalog, path: &LibPath) -> bool {
    let want = key(path);
    catalog.get(&want).is_some() || catalog.folders().any(|folder| key(folder) == want)
}

fn failed(why: String) -> RefreshReport {
    RefreshReport {
        errors: vec![(None, why)],
        ..RefreshReport::default()
    }
}

fn row(item: &Item, references: usize) -> ListRow {
    let file_name = || item.path.file_name().to_owned();
    let (name, description, state) = match &item.content {
        ItemContent::Instruction { excerpt } => (file_name(), excerpt.clone(), None),
        ItemContent::Skill {
            name, description, ..
        } => (name.clone(), description.clone(), None),
        ItemContent::SkillFile => (file_name(), String::new(), None),
        ItemContent::Preset { doc } => (
            file_name(),
            doc.description.clone().unwrap_or_default(),
            None,
        ),
        ItemContent::Settings { doc, .. } => (
            file_name(),
            doc.description.clone().unwrap_or_default(),
            None,
        ),
        ItemContent::Broken { reason } => (file_name(), String::new(), Some(reason.clone())),
    };
    ListRow {
        path: item.path.as_str().to_owned(),
        kind: item.kind.clone(),
        name,
        state,
        references,
        description,
    }
}

fn through_link_row(path: &LibPath) -> ListRow {
    ListRow {
        path: path.as_str().to_owned(),
        kind: ItemKind::Broken { claimed: None },
        name: path.file_name().to_owned(),
        state: Some(Reason {
            code: PATH_THROUGH_LINK.to_owned(),
            args: json!({ "path": path.as_str() }),
            line: None,
        }),
        references: 0,
        description: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::sync::Condvar;
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    use engram_dashboard_agent::profile::{
        AgentCommand, AgentProfile, AttachedPreset, ProfileStore, StoreError,
    };
    use engram_dashboard_base::sync::wait_timeout;

    use super::*;
    use crate::backends::AgentBackends;
    use crate::excerpt::excerpt;
    use crate::folder::{
        FolderListing, FolderOps, LaunchWaits, Spawn, ThreadSpawn, IO_LIMIT, LIST_CAP,
    };
    use crate::store::mem::{ListFailure, MemStore, OLD};
    use crate::store::{ContentTag, ScanListing, TrashBundle};

    fn lp(text: &str) -> LibPath {
        LibPath::parse(text).unwrap()
    }

    // ── the store: MemStores under fixed roots, a log of scans, one hold point a test arms ──────────────────

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum At {
        /// At the walk's first sequence draw — the walk has started and listed nothing yet.
        Start,
        /// After the listing, before it is returned — before any file is read and before the apply.
        End,
    }

    #[derive(Default)]
    struct Hold {
        armed: Option<(At, PathBuf)>,
        parked: bool,
        released: bool,
    }

    struct TestStore {
        roots: Vec<(PathBuf, MemStore)>,
        scans: Mutex<Vec<(PathBuf, Option<LibPath>)>>,
        read_caps: Mutex<Vec<u64>>,
        /// The next scan parks at `Start` (if armed) and then fails as an unreadable root.
        fail_next_scan: Mutex<bool>,
        hold: Mutex<Hold>,
        moved: Condvar,
    }

    impl TestStore {
        fn new(roots: &[&str]) -> Arc<Self> {
            Arc::new(TestStore {
                roots: roots
                    .iter()
                    .map(|root| (PathBuf::from(root), MemStore::new(Path::new(root))))
                    .collect(),
                scans: Mutex::default(),
                read_caps: Mutex::default(),
                fail_next_scan: Mutex::default(),
                hold: Mutex::default(),
                moved: Condvar::new(),
            })
        }

        fn mem(&self, root: &str) -> &MemStore {
            self.at_root(Path::new(root))
        }

        fn at_root(&self, root: &Path) -> &MemStore {
            &self
                .roots
                .iter()
                .find(|(at, _)| at == root)
                .expect("a root of the test store")
                .1
        }

        /// The store of the deepest root holding `path` — nested roots model one disk only while their files agree.
        fn holding(&self, path: &Path) -> &MemStore {
            &self
                .roots
                .iter()
                .filter(|(root, _)| path.starts_with(root))
                .max_by_key(|(root, _)| root.components().count())
                .expect("a path under a root of the test store")
                .1
        }

        fn scans(&self) -> Vec<(String, Option<String>)> {
            lock(&self.scans)
                .iter()
                .map(|(root, under)| {
                    let under = under.as_ref().map(|under| under.as_str().to_owned());
                    (root.display().to_string(), under)
                })
                .collect()
        }

        fn hold_at(&self, at: At, root: &str) {
            lock(&self.hold).armed = Some((at, PathBuf::from(root)));
        }

        fn wait_held(&self) {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut hold = lock(&self.hold);
            while !hold.parked {
                let now = Instant::now();
                assert!(now < deadline, "no scan reached the hold");
                hold = wait_timeout(&self.moved, hold, deadline - now).0;
            }
        }

        fn release(&self) {
            lock(&self.hold).released = true;
            self.moved.notify_all();
        }

        fn park(&self, at: At, root: &Path) {
            let mut hold = lock(&self.hold);
            if hold.armed.as_ref() != Some(&(at, root.to_path_buf())) {
                return;
            }
            hold.armed = None;
            hold.parked = true;
            self.moved.notify_all();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !hold.released {
                let now = Instant::now();
                assert!(now < deadline, "a held scan was never released");
                hold = wait_timeout(&self.moved, hold, deadline - now).0;
            }
            hold.released = false;
            hold.parked = false;
        }
    }

    fn scan(root: &str, under: Option<&str>) -> (String, Option<String>) {
        (root.to_owned(), under.map(str::to_owned))
    }

    impl LibraryStore for TestStore {
        fn scan(
            &self,
            root: &Path,
            under: Option<&LibPath>,
            seq: &dyn Fn() -> u64,
        ) -> io::Result<ScanListing> {
            lock(&self.scans).push((root.to_path_buf(), under.cloned()));
            if std::mem::take(&mut *lock(&self.fail_next_scan)) {
                self.park(At::Start, root);
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            let started = Cell::new(false);
            let listing = self.at_root(root).scan(root, under, &|| {
                if !started.replace(true) {
                    self.park(At::Start, root);
                }
                seq()
            });
            self.park(At::End, root);
            listing
        }

        fn first_link(&self, root: &Path, rel: &LibPath) -> io::Result<Option<LibPath>> {
            self.at_root(root).first_link(root, rel)
        }

        fn read(&self, path: &Path, cap: u64) -> io::Result<Vec<u8>> {
            lock(&self.read_caps).push(cap);
            self.holding(path).read(path, cap)
        }

        fn tag(&self, path: &Path) -> io::Result<ContentTag> {
            self.holding(path).tag(path)
        }

        fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            self.holding(path).write_atomic(path, bytes)
        }

        fn create_dir(&self, path: &Path) -> io::Result<()> {
            self.holding(path).create_dir(path)
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.holding(from).rename(from, to)
        }

        fn copy(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.holding(from).copy(from, to)
        }

        fn trash(&self, root: &Path, path: &Path, bundle: &TrashBundle) -> io::Result<PathBuf> {
            self.at_root(root).trash(root, path, bundle)
        }

        fn exists(&self, path: &Path) -> bool {
            self.holding(path).exists(path)
        }
    }

    // ── folder ops, spawners, the registry ───────────────────────────────────────────────────────────────

    /// Folder ops whose stat answers from a table (default `Ok`); nothing else is modeled.
    #[derive(Default)]
    struct StatOps(Mutex<HashMap<PathBuf, FolderStatus>>);

    impl FolderOps for StatOps {
        fn stat(&self, p: &Path) -> FolderStatus {
            lock(&self.0).get(p).cloned().unwrap_or(FolderStatus::Ok)
        }

        fn list(&self, _: &Path, _: usize) -> Result<FolderListing, FolderStatus> {
            Err(FolderStatus::Unreadable("not modeled".to_owned()))
        }

        fn canonical(&self, _: &Path) -> io::Result<PathBuf> {
            Err(io::Error::other("not modeled"))
        }

        fn read(&self, _: &Path, _: u64) -> io::Result<String> {
            Err(io::Error::other("not modeled"))
        }
    }

    /// Spawns threads and keeps their handles, so a test can wait for every job.
    #[derive(Default)]
    struct JoinSpawn(Mutex<Vec<JoinHandle<()>>>);

    impl Spawn for JoinSpawn {
        fn spawn(&self, name: &str, job: Box<dyn FnOnce() + Send>) -> io::Result<()> {
            let handle = thread::Builder::new().name(name.to_owned()).spawn(job)?;
            lock(&self.0).push(handle);
            Ok(())
        }
    }

    impl JoinSpawn {
        fn join_all(&self) {
            for handle in std::mem::take(&mut *lock(&self.0)) {
                join(handle);
            }
        }
    }

    struct FailingSpawn;

    impl Spawn for FailingSpawn {
        fn spawn(&self, _: &str, job: Box<dyn FnOnce() + Send>) -> io::Result<()> {
            drop(job);
            Err(io::Error::other("no threads left"))
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

    fn service(
        root: Option<&str>,
        store: &Arc<TestStore>,
        profiles: Vec<AgentProfile>,
        ops: Arc<dyn FolderOps>,
        spawn: Arc<dyn Spawn>,
    ) -> Arc<LibraryService> {
        let values = LibraryValues {
            registry: Arc::new(ProfileRegistry::new(Arc::new(Fixed(profiles)))),
            backends: Arc::new(AgentBackends),
            root: root.map(PathBuf::from),
            builtin_dir: None,
            launch_root: PathBuf::from("/launch"),
            home: None,
            env: Arc::new(|_| None),
        };
        let io = BoundedIo::new(IO_LIMIT, LaunchWaits::LAUNCH, LIST_CAP, ops, spawn);
        let store: Arc<dyn LibraryStore> = Arc::clone(store) as Arc<dyn LibraryStore>;
        Arc::new(LibraryService::new(values, store, io))
    }

    fn service_on(roots: &[&str], root: Option<&str>) -> (Arc<LibraryService>, Arc<TestStore>) {
        let store = TestStore::new(roots);
        let svc = service(
            root,
            &store,
            Vec::new(),
            Arc::new(StatOps::default()),
            Arc::new(ThreadSpawn),
        );
        (svc, store)
    }

    /// A service on `/lib` with no agents, every stat `Ok` and threads for jobs.
    fn library() -> (Arc<LibraryService>, Arc<TestStore>) {
        service_on(&["/lib"], Some("/lib"))
    }

    /// Joins `handle`, failing after 20 s instead of hanging.
    fn join<T>(handle: JoinHandle<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !handle.is_finished() {
            assert!(Instant::now() < deadline, "a test thread hung");
            thread::sleep(Duration::from_millis(2));
        }
        handle.join().expect("the test thread panicked")
    }

    fn on_thread<T: Send + 'static>(
        svc: &Arc<LibraryService>,
        f: impl FnOnce(&LibraryService) -> T + Send + 'static,
    ) -> JoinHandle<T> {
        let svc = Arc::clone(svc);
        thread::spawn(move || f(&svc))
    }

    /// What a launch check's overlay read hands its merge: the numbers noted first, then the items read.
    fn check_read(svc: &LibraryService, rel: &str) -> (Numbers, Vec<Item>) {
        let noted = svc.numbers();
        match svc.observe(Path::new("/lib"), &lp(rel)) {
            Observed::Items(items) => (noted, items),
            Observed::ThroughLink | Observed::Refused => panic!("{rel} is not readable"),
        }
    }

    fn merge(svc: &LibraryService, (noted, items): (Numbers, Vec<Item>)) {
        svc.late_merge(noted, |catalog, may_insert| {
            catalog.apply_items(items, may_insert)
        });
    }

    fn has(svc: &LibraryService, rel: &str) -> bool {
        svc.with_read(|state| state.catalog.get(&key(&lp(rel))).is_some())
    }

    /// The version of the file the catalog's item was read from (`MemStore::file_version`).
    fn version(svc: &LibraryService, rel: &str) -> u64 {
        svc.with_read(|state| {
            let item = state.catalog.get(&key(&lp(rel))).expect(rel);
            item.tag.modified.duration_since(OLD).unwrap().as_secs()
        })
    }

    fn root_of(svc: &LibraryService) -> Option<PathBuf> {
        svc.with_read(|state| state.root.clone())
    }

    fn paths(answer: &ListAnswer) -> Vec<&str> {
        answer.rows.iter().map(|row| row.path.as_str()).collect()
    }

    fn folder_paths(answer: &ListAnswer) -> Vec<&str> {
        answer
            .folders
            .iter()
            .map(|folder| folder.path.as_str())
            .collect()
    }

    // ── list ─────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn list_rows_carry_kind_name_state_description_and_reference_counts_and_never_rescan() {
        let store = TestStore::new(&["/lib"]);
        let svc = service(
            Some("/lib"),
            &store,
            vec![agent(&["top.yaml"])],
            Arc::new(StatOps::default()),
            Arc::new(ThreadSpawn),
        );
        let mem = store.mem("/lib");
        let rules = "# Rules\n\nKeep it short.\n";
        mem.file("rules/a.md", rules);
        mem.file(
            "top.yaml",
            "kind: preset\nversion: 1\ndescription: the top\ninstructions:\n  - rules/a.md\nskills:\n  - skills/s\n",
        );
        mem.file(
            "skills/s/SKILL.md",
            "---\nname: s\ndescription: does s\n---\nbody\n",
        );
        mem.file("broken.yaml", "kind: preset\nversion: [\n");
        mem.file("plain.yaml", "a: 1\n");

        let root = svc.list(None).unwrap();
        assert_eq!(paths(&root), ["broken.yaml", "top.yaml"]);
        assert_eq!(folder_paths(&root), ["rules", "skills"]);
        let broken = &root.rows[0];
        assert!(matches!(broken.kind, ItemKind::Broken { .. }));
        assert_eq!(broken.state.as_ref().unwrap().code, "yaml-syntax");
        let top = &root.rows[1];
        assert_eq!(
            (
                top.kind.clone(),
                top.name.as_str(),
                top.description.as_str()
            ),
            (ItemKind::Preset, "top.yaml", "the top")
        );
        assert_eq!((top.state.clone(), top.references), (None, 1), "the agent");

        let rules_rows = svc.list(Some(&lp("rules"))).unwrap();
        assert_eq!(paths(&rules_rows), ["rules/a.md"]);
        let a = &rules_rows.rows[0];
        assert_eq!(a.kind, ItemKind::Instruction);
        assert_eq!(a.description, excerpt(rules));
        assert_eq!(a.references, 1, "the preset");

        let skills = svc.list(Some(&lp("skills"))).unwrap();
        assert_eq!(paths(&skills), ["skills/s"]);
        assert!(
            skills.folders.is_empty(),
            "a skill folder is an item, not a folder line"
        );
        let s = &skills.rows[0];
        assert_eq!(
            (
                s.kind.clone(),
                s.name.as_str(),
                s.description.as_str(),
                s.references
            ),
            (ItemKind::Skill, "s", "does s", 1)
        );
        assert!(root.builtins.is_empty());
        assert_eq!(store.scans(), [scan("/lib", None)], "only the first scan");
    }

    #[test]
    fn list_answers_the_plain_subfolders_directly_in_the_folder() {
        let (svc, store) = library();
        store.mem("/lib").file("a/b/c.md", "c");
        store.mem("/lib").file("a/z/d.md", "d");
        let root = svc.list(None).unwrap();
        assert!(root.rows.is_empty());
        assert_eq!(
            root.folders,
            [FolderRow {
                path: "a".to_owned(),
                name: "a".to_owned()
            }]
        );
        let a = svc.list(Some(&lp("a"))).unwrap();
        assert!(a.rows.is_empty());
        assert_eq!(
            folder_paths(&a),
            ["a/b", "a/z"],
            "direct children only, by path"
        );
        assert_eq!(a.folders[1].name, "z");
    }

    #[test]
    fn every_catalog_entrance_passes_the_whole_first_scan_first() {
        for entrance in 0..4 {
            let (svc, store) = library();
            store.mem("/lib").file("a/x.md", "x");
            store.mem("/lib").file("b/y.md", "y");
            match entrance {
                0 => drop(svc.list(None).unwrap()),
                1 => drop(svc.list(Some(&lp("a"))).unwrap()),
                2 => drop(svc.refresh(Some(&lp("a")))),
                _ => svc.first_scan_join().unwrap(),
            }
            assert_eq!(store.scans(), [scan("/lib", None)], "entrance {entrance}");
            assert!(has(&svc, "b/y.md"), "entrance {entrance}");
        }
    }

    #[test]
    fn readers_attach_to_a_walking_first_scan() {
        let (svc, store) = library();
        store.mem("/lib").file("a.md", "a");
        store.hold_at(At::Start, "/lib");
        let first = on_thread(&svc, |svc| svc.list(None).unwrap());
        store.wait_held();
        let second = on_thread(&svc, |svc| svc.list(None).unwrap());
        svc.gate
            .wait_for("the second reader waits", |_, waiting| waiting == 1);
        store.release();
        let (first, second) = (join(first), join(second));
        assert_eq!(paths(&first), ["a.md"]);
        assert_eq!(first, second);
        assert_eq!(store.scans(), [scan("/lib", None)]);
    }

    #[test]
    fn the_safety_net_merges_one_path_the_catalog_lacks_and_refuses_links_dot_and_temp_names() {
        let (svc, store) = library();
        let mem = store.mem("/lib");
        mem.file("a.md", "a");
        mem.link("lnk");
        mem.file(".hidden.md", "h");
        mem.file("x.md.tmp12.0", "t");
        svc.list(None).unwrap();
        mem.file("later.md", "l");
        assert!(
            svc.list(Some(&lp("later.md"))).unwrap().rows.is_empty(),
            "a file has no children"
        );
        assert!(has(&svc, "later.md"), "merged by the safety net");

        let through = svc.list(Some(&lp("lnk/x.md"))).unwrap();
        assert_eq!(paths(&through), ["lnk/x.md"]);
        assert_eq!(
            through.rows[0].state.as_ref().unwrap().code,
            PATH_THROUGH_LINK
        );
        assert!(!has(&svc, "lnk/x.md"));
        for name in [".hidden.md", "x.md.tmp12.0"] {
            assert!(svc.list(Some(&lp(name))).unwrap().rows.is_empty(), "{name}");
            assert!(!has(&svc, name), "{name}");
        }
        // The same admission gates every late merge's read.
        let root = Path::new("/lib");
        assert!(matches!(
            svc.observe(root, &lp("lnk/x.md")),
            Observed::ThroughLink
        ));
        assert!(matches!(
            svc.observe(root, &lp(".hidden.md")),
            Observed::Refused
        ));
        assert!(matches!(
            svc.observe(root, &lp("x.md.tmp12.0")),
            Observed::Refused
        ));
    }

    // ── refresh ──────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_folder_refresh_before_the_first_scan_answers_its_share_of_a_whole_scan() {
        let (svc, store) = library();
        let mem = store.mem("/lib");
        mem.file(
            "a/p.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - b/x.md\n  - a/missing.md\n",
        );
        mem.file("b/x.md", "x");
        mem.file("b/broken.yaml", "kind: preset\nversion: [\n");
        let report = svc.refresh(Some(&lp("a")));
        assert_eq!(store.scans(), [scan("/lib", None)]);
        let missing = Reason {
            code: NOT_FOUND.to_owned(),
            args: json!({ "path": "a/missing.md" }),
            line: None,
        };
        // `b/x.md` is found (the whole library was scanned) and `b/broken.yaml` is outside `a`.
        assert_eq!(report.problems, [(lp("a/p.yaml"), missing)]);
        assert_eq!(report.changed, 1);
    }

    #[test]
    fn a_folder_refresh_arriving_at_a_walking_first_scan_scans_its_folder_after_it() {
        let (svc, store) = library();
        store.mem("/lib").file("a/x.md", "x");
        store.hold_at(At::Start, "/lib");
        let first = on_thread(&svc, |svc| svc.list(None).unwrap());
        store.wait_held();
        let refresh = on_thread(&svc, |svc| svc.refresh(Some(&lp("a"))));
        svc.gate
            .wait_for("the refresh waits for the first scan", |_, waiting| {
                waiting == 1
            });
        store.release();
        join(first);
        join(refresh);
        assert_eq!(store.scans(), [scan("/lib", None), scan("/lib", Some("a"))]);
    }

    #[test]
    fn a_folder_refresh_whose_waited_first_scan_failed_goes_through_a_whole_one() {
        let (svc, store) = library();
        store.mem("/lib").file(
            "a/p.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - b/x.md\n",
        );
        store.mem("/lib").file("b/x.md", "x");
        store.mem("/lib").file("c.md", "c");
        *lock(&store.fail_next_scan) = true;
        store.hold_at(At::Start, "/lib");
        let first = on_thread(&svc, |svc| svc.list(None));
        store.wait_held();
        let refresh = on_thread(&svc, |svc| svc.refresh(Some(&lp("a"))));
        svc.gate
            .wait_for("the refresh waits for the first scan", |_, waiting| {
                waiting == 1
            });
        store.release();
        let failed = join(first).expect_err("the failed first scan is the list's answer");
        assert_eq!(failed.code, LIBRARY_UNAVAILABLE);
        let report = join(refresh);
        assert_eq!(store.scans(), [scan("/lib", None), scan("/lib", None)]);
        assert!(report.problems.is_empty(), "no false not-found: {report:?}");
        assert!(report.errors.is_empty());
        assert_eq!(report.changed, 1);
        assert_eq!(
            paths(&svc.list(None).unwrap()),
            ["c.md"],
            "the second scan was applied"
        );
    }

    #[test]
    fn a_refresh_arriving_at_a_walking_covering_scan_scans_again_and_answers_the_new_bytes() {
        let (svc, store) = library();
        store.mem("/lib").file_version("a/x.md", "old", 1);
        svc.list(None).unwrap();
        store.hold_at(At::End, "/lib");
        let walking = on_thread(&svc, |svc| svc.refresh(None));
        store.wait_held();
        store.mem("/lib").file_version("a/x.md", "new", 2);
        let late = on_thread(&svc, |svc| svc.refresh(Some(&lp("a"))));
        svc.gate
            .wait_for("the late refresh has its own ticket", |line, _| {
                line == [("scan", true), ("scan", false)]
            });
        store.release();
        assert_eq!(join(walking).changed, 0);
        assert_eq!(join(late).changed, 1);
        assert_eq!(version(&svc, "a/x.md"), 2);
        assert_eq!(
            store.scans(),
            [
                scan("/lib", None),
                scan("/lib", None),
                scan("/lib", Some("a"))
            ]
        );
    }

    #[test]
    fn refresh_reports_skipped_links_walk_errors_and_no_kind_and_reads_with_the_library_cap() {
        let (svc, store) = library();
        let mem = store.mem("/lib");
        mem.file("c.md", "c");
        mem.file("sub/x.md", "x");
        mem.file("plain.yaml", "a: 1\n");
        mem.link("lnk");
        mem.fail_listing("sub", ListFailure::Open);
        let report = svc.refresh(None);
        assert_eq!(report.skipped_links, [lp("lnk")]);
        assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
        assert_eq!(report.errors[0].0, Some(lp("sub")));
        assert_eq!(report.no_kind, [lp("plain.yaml")]);
        let caps = lock(&store.read_caps).clone();
        assert!(!caps.is_empty());
        assert!(caps.iter().all(|&cap| cap == LIBRARY_READ_CAP), "{caps:?}");
        assert_eq!(LIBRARY_READ_CAP, 16 * 1024 * 1024);
    }

    #[test]
    fn without_a_root_a_refresh_and_a_list_answer_why() {
        let (svc, store) = service_on(&["/lib"], None);
        assert_eq!(svc.refresh(None).errors, [(None, gate::NO_ROOT.to_owned())]);
        let unavailable = Reason {
            code: LIBRARY_UNAVAILABLE.to_owned(),
            args: json!({ "message": gate::NO_ROOT }),
            line: None,
        };
        assert_eq!(svc.list(None), Err(unavailable));
        assert!(store.scans().is_empty());
    }

    // ── late merges and scans landing in either order ────────────────────────────────────────────────────

    #[test]
    fn a_file_read_by_a_check_and_then_deleted_is_gone_after_the_scan_that_listed_its_folder_in_either_order(
    ) {
        for merge_first in [false, true] {
            let (svc, store) = library();
            store.mem("/lib").file("d/x.md", "x");
            store.mem("/lib").file("d/y.md", "y");
            svc.list(None).unwrap();
            let read = check_read(&svc, "d/x.md");
            store.mem("/lib").remove("d/x.md");
            store.hold_at(At::End, "/lib");
            let scan = on_thread(&svc, |svc| svc.refresh(Some(&lp("d"))));
            store.wait_held();
            if merge_first {
                merge(&svc, read);
                store.release();
                join(scan);
            } else {
                store.release();
                join(scan);
                merge(&svc, read);
            }
            assert!(!has(&svc, "d/x.md"), "merge first: {merge_first}");
            assert!(has(&svc, "d/y.md"));
        }
    }

    #[test]
    fn a_file_created_after_the_scan_listed_its_folder_lands_per_order_and_the_safety_net_inserts_it_later(
    ) {
        for merge_first in [false, true] {
            let (svc, store) = library();
            store.mem("/lib").file("d/y.md", "y");
            svc.list(None).unwrap();
            store.hold_at(At::End, "/lib");
            let scan = on_thread(&svc, |svc| svc.refresh(Some(&lp("d"))));
            store.wait_held();
            store.mem("/lib").file("d/x.md", "x");
            let read = check_read(&svc, "d/x.md");
            if merge_first {
                merge(&svc, read);
                store.release();
                join(scan);
                assert!(has(&svc, "d/x.md"), "read after the listing: kept");
            } else {
                store.release();
                join(scan);
                merge(&svc, read);
                assert!(
                    !has(&svc, "d/x.md"),
                    "a swap-in landed between: the insert is dropped"
                );
                svc.list(Some(&lp("d/x.md"))).unwrap();
                assert!(
                    has(&svc, "d/x.md"),
                    "the safety net with no swap-in between"
                );
            }
        }
    }

    #[test]
    fn a_launch_merge_of_folder_b_survives_the_apply_of_a_folder_a_walk() {
        let (svc, store) = library();
        store.mem("/lib").file_version("a/x.md", "x", 1);
        store.mem("/lib").file_version("b/y.md", "y", 1);
        svc.list(None).unwrap();
        store.mem("/lib").file_version("b/y.md", "y2", 2);
        let read = check_read(&svc, "b/y.md");
        store.hold_at(At::Start, "/lib");
        let scan = on_thread(&svc, |svc| svc.refresh(Some(&lp("a"))));
        store.wait_held();
        merge(&svc, read);
        store.release();
        join(scan);
        assert_eq!(version(&svc, "b/y.md"), 2);
    }

    #[test]
    fn a_merge_inside_the_range_read_later_survives_the_whole_scan_apply() {
        let (svc, store) = library();
        store.mem("/lib").file_version("x.md", "v1", 1);
        svc.list(None).unwrap();
        // The scan lists x at version 2 and reads it only after the hold — by then it holds version 3.
        store.mem("/lib").file_version("x.md", "v2", 2);
        store.hold_at(At::End, "/lib");
        let scan = on_thread(&svc, |svc| svc.refresh(None));
        store.wait_held();
        store.mem("/lib").file_version("x.md", "v3", 3);
        merge(&svc, check_read(&svc, "x.md"));
        store.release();
        join(scan);
        assert_eq!(version(&svc, "x.md"), 3, "the later read stays");
    }

    #[test]
    fn a_safety_net_insert_landing_during_a_walk_survives_its_apply() {
        let (svc, store) = library();
        store.mem("/lib").file("a.md", "a");
        svc.list(None).unwrap();
        store.hold_at(At::End, "/lib");
        let scan = on_thread(&svc, |svc| svc.refresh(None));
        store.wait_held();
        store.mem("/lib").file("n.md", "n");
        svc.list(Some(&lp("n.md"))).unwrap();
        assert!(has(&svc, "n.md"));
        store.release();
        join(scan);
        assert!(has(&svc, "n.md"), "read after the listing");
    }

    #[test]
    fn with_the_numbers_unchanged_the_later_read_wins_in_either_landing_order() {
        for later_first in [false, true] {
            let (svc, store) = library();
            store.mem("/lib").file_version("x.md", "v1", 1);
            svc.list(None).unwrap();
            store.mem("/lib").file_version("x.md", "v2", 2);
            let older = check_read(&svc, "x.md");
            store.mem("/lib").file_version("x.md", "v3", 3);
            let newer = check_read(&svc, "x.md");
            if later_first {
                merge(&svc, newer);
                merge(&svc, older);
            } else {
                merge(&svc, older);
                merge(&svc, newer);
            }
            assert_eq!(version(&svc, "x.md"), 3, "later first: {later_first}");
        }
    }

    #[test]
    fn a_merge_read_before_a_change_does_not_revive_what_the_change_removed() {
        let (svc, store) = library();
        store.mem("/lib").file("p.md", "p");
        svc.list(None).unwrap();
        let read = check_read(&svc, "p.md");
        svc.change(|catalog, _, seq| {
            store.mem("/lib").remove("p.md");
            let k = key(&lp("p.md"));
            let removed = catalog.remove(&k, seq());
            ((), if removed { vec![k] } else { Vec::new() })
        });
        merge(&svc, read);
        assert!(!has(&svc, "p.md"));
    }

    #[test]
    fn a_handed_off_merge_waits_behind_a_held_write_lock_and_lands_after_it() {
        let store = TestStore::new(&["/lib"]);
        let spawn = Arc::new(JoinSpawn::default());
        let svc = service(
            Some("/lib"),
            &store,
            Vec::new(),
            Arc::new(StatOps::default()),
            Arc::clone(&spawn) as Arc<dyn Spawn>,
        );
        store.mem("/lib").file_version("x.md", "v1", 1);
        svc.list(None).unwrap();
        store.mem("/lib").file_version("x.md", "v2", 2);
        let (noted, items) = check_read(&svc, "x.md");
        {
            let held = svc.gate.lib().write().unwrap();
            // The hand-off returns while the lock is held (merging in place, it would hang here).
            join(on_thread(&svc, move |svc| svc.hand_off_merge(noted, items)));
            let still = held.catalog.get(&key(&lp("x.md"))).unwrap().tag;
            assert_eq!(
                still.modified,
                OLD + Duration::from_secs(1),
                "not merged yet"
            );
        }
        spawn.join_all();
        assert_eq!(version(&svc, "x.md"), 2, "delayed, not dropped");
    }

    #[test]
    fn a_hand_off_whose_job_cannot_start_merges_in_place() {
        let store = TestStore::new(&["/lib"]);
        let svc = service(
            Some("/lib"),
            &store,
            Vec::new(),
            Arc::new(StatOps::default()),
            Arc::new(FailingSpawn),
        );
        store.mem("/lib").file_version("x.md", "v1", 1);
        svc.list(None).unwrap();
        store.mem("/lib").file_version("x.md", "v2", 2);
        let (noted, items) = check_read(&svc, "x.md");
        svc.hand_off_merge(noted, items);
        assert_eq!(version(&svc, "x.md"), 2);
    }

    // ── changes ──────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_change_drawn_during_a_walk_waits_for_its_apply_and_is_not_undone() {
        let (svc, store) = library();
        store.mem("/lib").file("x.md", "x");
        svc.list(None).unwrap();
        store.hold_at(At::Start, "/lib");
        let scan = on_thread(&svc, |svc| svc.refresh(None));
        store.wait_held();
        let disk = Arc::clone(&store);
        let change = on_thread(&svc, move |svc| {
            svc.change(|catalog, _, seq| {
                disk.mem("/lib").remove("x.md");
                let k = key(&lp("x.md"));
                let removed = catalog.remove(&k, seq());
                (removed, vec![k])
            })
        });
        svc.gate
            .wait_for("the change waits behind the walk", |line, _| {
                line == [("scan", true), ("change", false)]
            });
        store.release();
        join(scan);
        assert!(join(change), "the change removed what the scan had applied");
        assert!(!has(&svc, "x.md"));
    }

    #[test]
    fn a_change_generation_moved_during_a_walk_makes_the_scan_walk_again() {
        let (svc, store) = library();
        store.mem("/lib").file("x.md", "x");
        svc.list(None).unwrap();
        store.hold_at(At::Start, "/lib");
        let walking = on_thread(&svc, |svc| svc.refresh(None));
        store.wait_held();
        svc.gate.change_unordered(|catalog| {
            store.mem("/lib").remove("x.md");
            catalog.remove(&key(&lp("x.md")), svc.gate.next_seq());
        });
        store.release();
        join(walking);
        assert!(!has(&svc, "x.md"));
        assert_eq!(
            store.scans(),
            [scan("/lib", None), scan("/lib", None), scan("/lib", None)]
        );
    }

    // ── root change ──────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn readers_see_the_old_root_during_the_new_roots_walk() {
        let (svc, store) = service_on(&["/lib", "/lib2"], Some("/lib"));
        store.mem("/lib").file("a.md", "a");
        store.mem("/lib2").file("b.md", "b");
        assert_eq!(paths(&svc.list(None).unwrap()), ["a.md"]);
        store.hold_at(At::Start, "/lib2");
        let change = on_thread(&svc, |svc| svc.change_root(PathBuf::from("/lib2")));
        store.wait_held();
        assert_eq!(
            paths(&svc.list(None).unwrap()),
            ["a.md"],
            "never an empty catalog"
        );
        store.release();
        join(change).unwrap();
        assert_eq!(paths(&svc.list(None).unwrap()), ["b.md"]);
        assert_eq!(root_of(&svc), Some(PathBuf::from("/lib2")));
    }

    #[test]
    fn a_nested_root_change_keeps_a_file_whose_tag_equals_an_old_item_of_the_same_key() {
        let (svc, store) = service_on(&["/lib", "/lib/team"], Some("/lib"));
        // `X.md` under the old root and `team/X.md` (the new root's `X.md`) have equal tags: same size, same time.
        store.mem("/lib").file_version("X.md", "top!", 1);
        store.mem("/lib").file_version("team/X.md", "team", 1);
        store.mem("/lib/team").file_version("X.md", "team", 1);
        assert_eq!(svc.list(None).unwrap().rows[0].description, excerpt("top!"));
        svc.change_root(PathBuf::from("/lib/team")).unwrap();
        let rows = svc.list(None).unwrap();
        assert_eq!(paths(&rows), ["X.md"]);
        assert_eq!(rows.rows[0].description, excerpt("team"));
    }

    #[test]
    fn a_first_start_root_change_raises_the_first_scan_for_the_readers_waiting_on_it() {
        let (svc, store) = service_on(&["/lib"], None);
        store.mem("/lib").file("a.md", "a");
        store.hold_at(At::Start, "/lib");
        let change = on_thread(&svc, |svc| svc.change_root(PathBuf::from("/lib")));
        store.wait_held();
        let reader = on_thread(&svc, |svc| svc.list(None).unwrap());
        svc.gate
            .wait_for("the reader waits for the root walk", |_, waiting| {
                waiting == 1
            });
        store.release();
        join(change).unwrap();
        assert_eq!(paths(&join(reader)), ["a.md"]);
        assert_eq!(store.scans(), [scan("/lib", None)], "no second scan");
    }

    #[test]
    fn a_root_change_whose_walk_fails_at_the_root_publishes_nothing() {
        let (svc, store) = service_on(&["/lib", "/lib2"], Some("/lib"));
        store.mem("/lib").file("a.md", "a");
        svc.list(None).unwrap();
        store.mem("/lib2").fail_listing("", ListFailure::Open);
        assert!(svc.change_root(PathBuf::from("/lib2")).is_err());
        let answer = join(on_thread(&svc, |svc| svc.list(None).unwrap()));
        assert_eq!(paths(&answer), ["a.md"]);
        assert_eq!(root_of(&svc), Some(PathBuf::from("/lib")));
        let report = join(on_thread(&svc, |svc| svc.refresh(None)));
        assert!(report.errors.is_empty(), "the ticket was released");
    }

    #[test]
    fn a_new_root_that_does_not_stat_as_a_folder_is_refused_before_any_walk() {
        let store = TestStore::new(&["/lib", "/lib2"]);
        let ops = Arc::new(StatOps::default());
        lock(&ops.0).insert(PathBuf::from("/lib2"), FolderStatus::Missing);
        let svc = service(Some("/lib"), &store, Vec::new(), ops, Arc::new(ThreadSpawn));
        assert!(svc.change_root(PathBuf::from("/lib2")).is_err());
        assert!(store.scans().is_empty());
        assert_eq!(root_of(&svc), Some(PathBuf::from("/lib")));
    }
}
