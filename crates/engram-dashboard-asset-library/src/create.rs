//! Create (`newFolder {parent?, name}` · `newFile {folder?, name, kind}`) — inherent [`LibraryService`] methods answering
//! the new path.
//!
//! ## Rules (invariants)
//!
//! - **One change, never an overwrite.** After the name rules and the first-scan entrance, one
//!   [`LibraryService::change_at`] write lock holds, in this order: the parent folder's link admission (ADR-0365), the
//!   existence check, the create and the new item or folder entered into the catalog. The create itself refuses a name
//!   that is taken (`LibraryStore::create_new` · `create_dir` — the existence check is only a fast path), so a name
//!   that exists is refused, never replaced; a missing parent is refused, never created.
//! - **The catalog follows the create, not a read-back.** A new file's item is made from the bytes written, as a scan
//!   makes it (a `SKILL.md` makes its folder a skill); a new folder replaces whatever the catalog still held at or below
//!   its path (nothing is there on disk once it is made).
//! - **Templates parse.** A markdown file is empty; a settings or preset file holds `kind`, `version: 1` and an empty
//!   `description`, which the readers accept (a file without `version` does not read).
// ADR-0365
// ADR-0366

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::time::SystemTime;

use engram_dashboard_agent::launch::escape_controls;
use engram_dashboard_platform::fs::{same_path, PathKey};
use serde_json::json;

use crate::catalog::{key, within, Catalog, Item, ScanItem, ScanResult};
use crate::edit::{dot_or_temp, lib_path, not_found, LIBRARY_WRITE_FAILED};
use crate::format::read::ITEM_PATH_INVALID;
use crate::gate::NO_ROOT;
use crate::path::LibPath;
use crate::store::{admits, ContentTag, ScanEntry, ScanListing};
use crate::{LibraryRefusal, LibraryService, RefusalClass};

/// The code of a create whose name is already taken (`args` = `{path}`); nothing is replaced.
pub const NAME_EXISTS: &str = "name-exists";

/// What a `newFile` makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewFileKind {
    /// An instruction: an empty `.md`.
    Markdown,
    Settings,
    Preset,
}

impl NewFileKind {
    /// The file's first bytes.
    fn template(self) -> &'static str {
        match self {
            NewFileKind::Markdown => "",
            NewFileKind::Settings => "kind: settings\nversion: 1\ndescription: \n",
            NewFileKind::Preset => "kind: preset\nversion: 1\ndescription: \n",
        }
    }
}

impl LibraryService {
    /// Creates folder `name` in `parent` (`None` = the root) and enters it into the catalog. Answers its path.
    ///
    /// Refusals, nothing created: a parent that is not a library path, a name that is not one path segment or is a dot
    /// or temp name `InvalidArgument` · `item-path-invalid`; plus the refusals every create can answer
    /// ([`LibraryService::new_file`]).
    pub fn new_folder(
        &self,
        parent: Option<&str>,
        name: &str,
        root_generation: Option<u64>,
    ) -> Result<LibPath, LibraryRefusal> {
        let parent = parent.map(lib_path).transpose()?;
        let path = child(parent.as_ref(), name, name)?;
        self.create(parent.as_ref(), &path, None, root_generation)
    }

    /// Creates file `name` of `kind` in `folder` (`None` = the root) from its template and enters its item into the
    /// catalog. A markdown name takes `.md`, a settings or preset name `.yaml` (`.yml` is kept), each appended unless
    /// the name already ends with it. Answers its path.
    ///
    /// Refusals, nothing created: a folder that is not a library path, a name that is not one path segment, a dot or
    /// temp name, or a name ending in the other kinds' extension `InvalidArgument` · `item-path-invalid`; no published
    /// root `Unavailable` · `library-unavailable`; a stale root generation `Conflict` · `root-generation-stale`; the
    /// parent at or below a link `InvalidArgument` · `path-through-link` (a probe error `Unavailable` ·
    /// `library-read-failed`); the name taken `Conflict` · [`NAME_EXISTS`]; the parent missing or not a folder
    /// `NotFound` · `not-found`; another create or write failure `Unavailable` · `library-write-failed` (a failed
    /// write can leave a partial file at the new name — the store never removes it by name).
    pub fn new_file(
        &self,
        folder: Option<&str>,
        name: &str,
        kind: NewFileKind,
        root_generation: Option<u64>,
    ) -> Result<LibPath, LibraryRefusal> {
        let folder = folder.map(lib_path).transpose()?;
        let file_name = with_extension(name, kind)?;
        let path = child(folder.as_ref(), name, &file_name)?;
        self.create(
            folder.as_ref(),
            &path,
            Some(kind.template()),
            root_generation,
        )
    }

    /// One create (module header): a file from `template`, or a folder when `None`.
    fn create(
        &self,
        parent: Option<&LibPath>,
        path: &LibPath,
        template: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<LibPath, LibraryRefusal> {
        if !admits(path) {
            return Err(dot_or_temp(path));
        }
        self.first_scan_join()
            .map_err(LibraryRefusal::unavailable)?;
        self.change_at(root_generation, |catalog, root, seq| {
            let mut touched = Vec::new();
            let answer = match root {
                Some(root) => {
                    self.create_in(parent, path, template, catalog, root, seq, &mut touched)
                }
                None => Err(LibraryRefusal::unavailable(NO_ROOT.to_owned())),
            };
            (answer, touched)
        })?
    }

    /// The steps of [`LibraryService::create`] inside the change; `touched` gathers every key applied.
    #[allow(clippy::too_many_arguments)]
    fn create_in(
        &self,
        parent: Option<&LibPath>,
        path: &LibPath,
        template: Option<&str>,
        catalog: &mut Catalog,
        root: &Path,
        seq: &dyn Fn() -> u64,
        touched: &mut Vec<PathKey>,
    ) -> Result<LibPath, LibraryRefusal> {
        if let Some(parent) = parent {
            self.admit(root, parent)?;
        }
        let disk = path.to_path(root);
        if self.store.exists(&disk) {
            return Err(name_exists(path));
        }
        let made_seq = seq();
        let made = match template {
            None => self.store.create_dir(&disk),
            Some(text) => self.store.create_new(&disk, text.as_bytes()),
        };
        made.map_err(|e| create_refused(parent, path, &e))?;
        match template {
            None => {
                let absent_at = seq();
                let stale: Vec<PathKey> = catalog
                    .items()
                    .filter(|item| within(&item.path, Some(path)))
                    .map(|item| key(&item.path))
                    .collect();
                for at in stale {
                    if catalog.remove(&at, absent_at) {
                        touched.push(at);
                    }
                }
                catalog.insert_folder(path.clone());
                tracing::info!(path = path.as_str(), "library folder created");
            }
            Some(text) => {
                let tag = self.tag_after_write(path, &disk, text.len());
                let items = self.items_of(root, path, text, tag, made_seq);
                touched.extend(catalog.apply_items(items, true));
                tracing::info!(path = path.as_str(), "library file created");
            }
        }
        Ok(path.clone())
    }

    /// The items the file `path` holding `text` makes, decided as a scan decides them (`ScanResult::build`) with no
    /// disk read; their `read_seq` is `made_seq`, drawn before the create.
    fn items_of(
        &self,
        root: &Path,
        path: &LibPath,
        text: &str,
        tag: ContentTag,
        made_seq: u64,
    ) -> Vec<Item> {
        let listing = ScanListing {
            root: root.to_path_buf(),
            files: vec![ScanEntry {
                rel: path.clone(),
                tag,
                seq: made_seq,
            }],
            ..ScanListing::default()
        };
        let read = |_: &Path| Ok(text.as_bytes().to_vec());
        ScanResult::build(
            listing,
            Some(path),
            &HashMap::new(),
            &read,
            &*self.values.backends,
            SystemTime::now(),
        )
        .entries
        .into_iter()
        .filter_map(|entry| match entry {
            ScanItem::Fresh(item) => Some(item),
            ScanItem::Unchanged(..) => None,
        })
        .collect()
    }
}

/// `name` with `kind`'s extension (compared by the OS rule, as the catalog decides kinds), or `item-path-invalid` for
/// a name ending in the other kinds' extension.
fn with_extension(name: &str, kind: NewFileKind) -> Result<String, LibraryRefusal> {
    let extension = name.rsplit_once('.').map(|(_, extension)| extension);
    let is = |want: &str| extension.is_some_and(|ext| same_path(Path::new(ext), Path::new(want)));
    let (markdown, yaml) = (is("md"), is("yaml") || is("yml"));
    let wrong = || {
        Err(LibraryRefusal::new(
            RefusalClass::InvalidArgument,
            ITEM_PATH_INVALID,
            json!({
                "path": escape_controls(name),
                "why": "the name ends in another kind's extension",
            }),
        ))
    };
    match kind {
        NewFileKind::Markdown if markdown => Ok(name.to_owned()),
        NewFileKind::Markdown if yaml => wrong(),
        NewFileKind::Markdown => Ok(format!("{name}.md")),
        NewFileKind::Settings | NewFileKind::Preset if yaml => Ok(name.to_owned()),
        NewFileKind::Settings | NewFileKind::Preset if markdown => wrong(),
        NewFileKind::Settings | NewFileKind::Preset => Ok(format!("{name}.yaml")),
    }
}

/// The library path of `name` in `parent` (`None` = the root), or `item-path-invalid` naming `given` (the name as
/// asked) — `name` must be one path segment.
fn child(parent: Option<&LibPath>, given: &str, name: &str) -> Result<LibPath, LibraryRefusal> {
    let invalid = |why: String| {
        LibraryRefusal::new(
            RefusalClass::InvalidArgument,
            ITEM_PATH_INVALID,
            json!({ "path": escape_controls(given), "why": why }),
        )
    };
    if name.contains('/') {
        return Err(invalid("a name is one path segment".to_owned()));
    }
    let text = match parent {
        Some(parent) => format!("{}/{name}", parent.as_str()),
        None => name.to_owned(),
    };
    LibPath::parse(&text).map_err(|why| invalid(why.to_string()))
}

fn name_exists(path: &LibPath) -> LibraryRefusal {
    LibraryRefusal::new(
        RefusalClass::Conflict,
        NAME_EXISTS,
        json!({ "path": path.as_str() }),
    )
}

/// A create that failed: the name taken meanwhile `name-exists`, the parent missing or not a folder `not-found`
/// (naming the parent, or the new path at the root), anything else `library-write-failed`.
fn create_refused(parent: Option<&LibPath>, path: &LibPath, e: &io::Error) -> LibraryRefusal {
    match e.kind() {
        io::ErrorKind::AlreadyExists => name_exists(path),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => {
            not_found(parent.unwrap_or(path).as_str())
        }
        _ => LibraryRefusal::new(
            RefusalClass::Unavailable,
            LIBRARY_WRITE_FAILED,
            json!({ "path": path.as_str(), "why": e.to_string() }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, OnceLock, Weak};

    use engram_dashboard_agent::launch::Reason;
    use engram_dashboard_agent::profile::{
        AgentProfile, ProfileRegistry, ProfileStore, StoreError,
    };
    use engram_dashboard_base::sync::lock;
    use engram_dashboard_platform::fs::{link_dir, unlink_dir};
    use tempfile::TempDir;

    use super::*;
    use crate::backends::AgentBackends;
    use crate::catalog::{key, ItemKind};
    use crate::config::{LibraryConfig, RootSetting};
    use crate::folder::{BoundedIo, LaunchWaits, StdFolderOps, ThreadSpawn, IO_LIMIT, LIST_CAP};
    use crate::gate::NOT_FOUND;
    use crate::store::{ContentTag, FsStore, LibraryStore, ScanListing, TrashBundle};
    use crate::{LibraryValues, LIBRARY_UNAVAILABLE, PATH_THROUGH_LINK};

    /// [`FsStore`] that fails every write and folder create while a test sets `fail`, answers every existence check
    /// "absent" while it sets `blind`, fails every read while it sets `unreadable`, and logs each create with whether
    /// the library write lock was held.
    #[derive(Default)]
    struct Disk {
        fail: Mutex<bool>,
        blind: Mutex<bool>,
        unreadable: Mutex<bool>,
        svc: OnceLock<Weak<LibraryService>>,
        creates: Mutex<Vec<(PathBuf, bool)>>,
    }

    impl Disk {
        fn failing(&self) -> io::Result<()> {
            match *lock(&self.fail) {
                true => Err(io::Error::other("injected failure")),
                false => Ok(()),
            }
        }

        /// Logs a create of `path`; a read lock lets `try_read` through, so only the write lock counts as held.
        fn note(&self, path: &Path) {
            let held = self.svc.get().and_then(Weak::upgrade).is_some_and(|svc| {
                let lib = svc.gate.lib();
                let held = lib.try_write().is_err() && lib.try_read().is_err();
                held
            });
            lock(&self.creates).push((path.to_path_buf(), held));
        }
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
            if *lock(&self.unreadable) {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            FsStore.read(path, cap)
        }
        fn tag(&self, path: &Path) -> io::Result<ContentTag> {
            FsStore.tag(path)
        }
        fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            self.failing()?;
            FsStore.write_atomic(path, bytes)
        }
        fn create_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            self.note(path);
            self.failing()?;
            FsStore.create_new(path, bytes)
        }
        fn create_dir(&self, path: &Path) -> io::Result<()> {
            self.note(path);
            self.failing()?;
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
            !*lock(&self.blind) && FsStore.exists(path)
        }
    }

    struct NoAgents;

    impl ProfileStore for NoAgents {
        fn save(&self, _: &[AgentProfile]) -> Result<(), StoreError> {
            Ok(())
        }
        fn load(&self) -> Vec<AgentProfile> {
            Vec::new()
        }
    }

    /// A temp folder holding the library root `lib\` (published when `root`).
    struct World {
        _temp: TempDir,
        base: PathBuf,
        disk: Arc<Disk>,
        svc: Arc<LibraryService>,
    }

    impl World {
        fn new() -> World {
            World::make(true)
        }

        fn make(root: bool) -> World {
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
                registry: Arc::new(ProfileRegistry::new(Arc::new(NoAgents))),
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
            let disk = Arc::new(Disk::default());
            let svc = Arc::new(LibraryService::new(
                values,
                Arc::clone(&disk) as Arc<dyn LibraryStore>,
                io,
            ));
            let _ = disk.svc.set(Arc::downgrade(&svc));
            World {
                _temp: temp,
                base,
                disk,
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

        fn file(
            &self,
            folder: Option<&str>,
            name: &str,
            kind: NewFileKind,
        ) -> Result<LibPath, LibraryRefusal> {
            self.svc.new_file(folder, name, kind, None)
        }

        fn kind_of(&self, rel: &str) -> Option<ItemKind> {
            self.svc.with_read(|state| {
                state
                    .catalog
                    .get(&key(&LibPath::parse(rel).unwrap()))
                    .map(|item| item.kind.clone())
            })
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

    #[test]
    fn each_template_parses_and_its_item_enters_the_catalog_in_the_same_change() {
        let w = World::new();
        w.put("rules/keep.md", "k");
        w.svc.first_scan_join().unwrap();
        let swap_ins = w.svc.numbers().swap_ins;
        let cases = [
            (
                "notes",
                NewFileKind::Markdown,
                "rules/notes.md",
                ItemKind::Instruction,
            ),
            (
                "team",
                NewFileKind::Settings,
                "rules/team.yaml",
                ItemKind::Settings,
            ),
            (
                "base",
                NewFileKind::Preset,
                "rules/base.yaml",
                ItemKind::Preset,
            ),
        ];
        for (name, kind, path, item) in cases {
            let made = w.file(Some("rules"), name, kind).unwrap();
            assert_eq!(made.as_str(), path);
            assert_eq!(fs::read_to_string(w.lib(path)).unwrap(), kind.template());
            assert_eq!(
                w.kind_of(path),
                Some(item),
                "{path} reads as its kind, not broken"
            );
        }
        assert_eq!(
            w.svc.numbers().swap_ins,
            swap_ins,
            "entered by the change, not by a scan"
        );
    }

    #[test]
    fn the_kinds_extension_is_appended_unless_present_and_the_other_kinds_is_refused() {
        let w = World::new();
        let made = |name: &str, kind| {
            w.file(None, name, kind)
                .map(|path| path.as_str().to_owned())
        };
        assert_eq!(made("a.md", NewFileKind::Markdown).unwrap(), "a.md");
        assert_eq!(made("b.yml", NewFileKind::Settings).unwrap(), "b.yml");
        assert_eq!(made("c.yaml", NewFileKind::Preset).unwrap(), "c.yaml");
        assert_eq!(made("d", NewFileKind::Preset).unwrap(), "d.yaml");
        for (name, kind) in [
            ("e.yaml", NewFileKind::Markdown),
            ("f.yml", NewFileKind::Markdown),
            ("g.md", NewFileKind::Settings),
            ("h.md", NewFileKind::Preset),
            ("i/j", NewFileKind::Markdown),
            (".hidden", NewFileKind::Markdown),
            ("", NewFileKind::Markdown),
        ] {
            assert_eq!(
                code(w.file(None, name, kind)),
                (RefusalClass::InvalidArgument, ITEM_PATH_INVALID.to_owned()),
                "{name:?}"
            );
        }
        assert!(!w.lib("e.yaml").exists() && !w.lib("g.md").exists());
    }

    #[test]
    fn a_new_folder_enters_the_catalog_and_the_folder_tree() {
        let w = World::new();
        w.put("rules/a.md", "a");

        let made = w.svc.new_folder(Some("rules"), "team", None).unwrap();

        assert_eq!(made.as_str(), "rules/team");
        assert!(w.lib("rules/team").is_dir());
        let folders: Vec<String> = w
            .svc
            .folders()
            .unwrap()
            .folders
            .into_iter()
            .map(|node| node.path)
            .collect();
        assert!(folders.contains(&"rules/team".to_owned()), "{folders:?}");
        assert_eq!(
            code(w.svc.new_folder(None, "a/b", None)),
            (RefusalClass::InvalidArgument, ITEM_PATH_INVALID.to_owned())
        );
    }

    #[test]
    fn an_existing_name_is_refused_and_never_replaced() {
        let w = World::new();
        w.put("rules/a.md", "mine");
        w.put("P.yaml", "kind: preset\nversion: 1\n");

        assert_eq!(
            code(w.file(Some("rules"), "a", NewFileKind::Markdown)),
            (RefusalClass::Conflict, NAME_EXISTS.to_owned())
        );
        assert_eq!(
            code(w.file(None, "P", NewFileKind::Preset)),
            (RefusalClass::Conflict, NAME_EXISTS.to_owned())
        );
        assert_eq!(
            code(w.svc.new_folder(None, "rules", None)),
            (RefusalClass::Conflict, NAME_EXISTS.to_owned())
        );
        assert_eq!(fs::read_to_string(w.lib("rules/a.md")).unwrap(), "mine");
        assert_eq!(
            fs::read_to_string(w.lib("P.yaml")).unwrap(),
            "kind: preset\nversion: 1\n"
        );
    }

    #[test]
    fn a_missing_parent_is_refused_not_found_and_never_created() {
        let w = World::new();
        assert_eq!(
            code(w.file(Some("nope"), "a", NewFileKind::Markdown)),
            (RefusalClass::NotFound, NOT_FOUND.to_owned())
        );
        assert_eq!(
            code(w.svc.new_folder(Some("nope/deeper"), "team", None)),
            (RefusalClass::NotFound, NOT_FOUND.to_owned())
        );
        assert!(!w.lib("nope").exists());
    }

    #[test]
    fn link_admission_on_the_parent_refuses_before_anything_is_created() {
        let w = World::new();
        let outside = w.base.join("outside");
        fs::create_dir_all(&outside).unwrap();
        link_dir(&outside, &w.lib("team")).unwrap();

        assert_eq!(
            code(w.file(Some("team"), "a", NewFileKind::Markdown)),
            (RefusalClass::InvalidArgument, PATH_THROUGH_LINK.to_owned())
        );
        assert_eq!(
            code(w.svc.new_folder(Some("team"), "sub", None)),
            (RefusalClass::InvalidArgument, PATH_THROUGH_LINK.to_owned())
        );
        assert_eq!(
            fs::read_dir(&outside).unwrap().count(),
            0,
            "nothing created through the link"
        );
    }

    #[test]
    fn a_failed_create_answers_library_write_failed_and_enters_nothing() {
        let w = World::new();
        *lock(&w.disk.fail) = true;

        assert_eq!(
            code(w.file(None, "a", NewFileKind::Markdown)),
            (RefusalClass::Unavailable, LIBRARY_WRITE_FAILED.to_owned())
        );
        assert_eq!(
            code(w.svc.new_folder(None, "team", None)),
            (RefusalClass::Unavailable, LIBRARY_WRITE_FAILED.to_owned())
        );
        assert_eq!(w.kind_of("a.md"), None);
    }

    #[test]
    fn with_no_root_a_create_answers_library_unavailable() {
        let w = World::make(false);
        assert_eq!(
            code(w.file(None, "a", NewFileKind::Markdown)),
            (RefusalClass::Unavailable, LIBRARY_UNAVAILABLE.to_owned())
        );
        assert_eq!(
            code(w.svc.new_folder(None, "team", None)),
            (RefusalClass::Unavailable, LIBRARY_UNAVAILABLE.to_owned())
        );
    }

    // ── FIX round 1 ─────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_taken_name_the_existence_check_misses_is_refused_by_the_create_itself() {
        let w = World::new();
        w.put("rules/a.md", "mine");
        w.put("rules/team/keep.md", "k");
        *lock(&w.disk.blind) = true;

        assert_eq!(
            code(w.file(Some("rules"), "a", NewFileKind::Markdown)),
            (RefusalClass::Conflict, NAME_EXISTS.to_owned())
        );
        assert_eq!(
            code(w.svc.new_folder(Some("rules"), "team", None)),
            (RefusalClass::Conflict, NAME_EXISTS.to_owned())
        );
        assert_eq!(fs::read_to_string(w.lib("rules/a.md")).unwrap(), "mine");
        assert!(w.lib("rules/team/keep.md").exists());
    }

    #[test]
    fn a_new_file_enters_the_catalog_from_the_bytes_written_with_no_read_back() {
        let w = World::new();
        w.svc.first_scan_join().unwrap();
        *lock(&w.disk.unreadable) = true;

        w.file(None, "base", NewFileKind::Preset).unwrap();
        w.file(None, "notes", NewFileKind::Markdown).unwrap();

        assert_eq!(w.kind_of("base.yaml"), Some(ItemKind::Preset));
        assert_eq!(w.kind_of("notes.md"), Some(ItemKind::Instruction));
    }

    #[test]
    fn a_new_folder_replaces_what_the_catalog_still_held_at_its_path() {
        let w = World::new();
        w.put("skills/x/SKILL.md", "---\nname: x\ndescription: d\n---\n");
        w.put("skills/x/sub/a.md", "a");
        w.svc.first_scan_join().unwrap();
        assert_eq!(w.kind_of("skills/x"), Some(ItemKind::Skill));
        // Deleted outside Engram; the catalog has not seen it.
        fs::remove_dir_all(w.lib("skills/x")).unwrap();

        w.svc.new_folder(Some("skills"), "x", None).unwrap();

        assert_eq!(w.kind_of("skills/x"), None);
        assert_eq!(w.kind_of("skills/x/SKILL.md"), None);
        assert_eq!(w.kind_of("skills/x/sub/a.md"), None);
        let tree = w.svc.folders().unwrap().folders;
        let node = tree
            .iter()
            .find(|node| node.path == "skills/x")
            .expect("the new folder is in the tree");
        assert!(!node.skill, "a plain folder, not the old skill");
        assert!(
            !tree.iter().any(|node| node.path == "skills/x/sub"),
            "a stale subfolder goes too: {tree:?}"
        );
    }

    #[test]
    fn a_parent_that_is_a_file_answers_not_found() {
        let w = World::new();
        w.put("a.md", "a");

        assert_eq!(
            code(w.file(Some("a.md"), "x", NewFileKind::Markdown)),
            (RefusalClass::NotFound, NOT_FOUND.to_owned())
        );
        assert_eq!(
            code(w.svc.new_folder(Some("a.md"), "sub", None)),
            (RefusalClass::NotFound, NOT_FOUND.to_owned())
        );
        assert_eq!(fs::read_to_string(w.lib("a.md")).unwrap(), "a");
    }

    #[test]
    fn the_create_runs_under_the_library_write_lock() {
        let w = World::new();
        w.svc.first_scan_join().unwrap();

        w.file(None, "notes", NewFileKind::Markdown).unwrap();
        w.svc.new_folder(None, "team", None).unwrap();

        let creates = lock(&w.disk.creates).clone();
        assert_eq!(
            creates,
            [(w.lib("notes.md"), true), (w.lib("team"), true)],
            "each create inside the change"
        );
    }
}
