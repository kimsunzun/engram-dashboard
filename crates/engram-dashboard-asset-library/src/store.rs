//! The storage seam: every disk access to the library root goes through [`LibraryStore`] — [`FsStore`] in production,
//! `MemStore` (`#[cfg(test)]`) in tests.
//!
//! ## Rules every implementation keeps
//!
//! - **Link policy (U1 (a))**: only the configured library root may be a link. A walk never enters a link below the root
//!   (folder or file) and never lists it; it reports it in [`ScanListing::skipped_links`]. A partial scan whose `under`
//!   is a link or lies below one walks nothing ([`LibraryStore::first_link`] — the one component test for a path below
//!   the root).
//! - **Names**: [`admits`] is the name rule every way into the catalog shares — no name starting with `.` (so the root's
//!   `.trash` is never walked) and no base temp name. Nothing here deletes or rewrites a leftover temp file
//!   (ADR-0335); the only temp files touched are base `write_atomic`'s own, whose base naming could truncate a
//!   leftover only in the rare case of the same pid and counter (base behaviour, not this module's).
//! - **Per-folder listing sequence**: a scan draws `seq()` before every step of the walk; the number drawn
//!   before the step that yields folder D precedes every read of D's entries and is `listed_at[D]` (ADR-0324 decision
//!   5 · TRD S25 step-7 premise 47). A file of a scanned folder that the listing lacks was observed absent at that
//!   number.
//! - **A failed read is not an absence**: a folder whose listing fails (at open or part-way) and a file whose metadata
//!   fails are reported in [`ScanListing::errors`]; the catalog keeps what it knew of them.
//! - **No OS branch here** — link tests use std's `symlink_metadata().file_type().is_symlink()` (on Windows: every
//!   name-surrogate reparse point, junctions included) and identity comes from platform `file_identity`.
// ADR-0324
// ADR-0335
// TRD S25 step-7 §3-5

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use engram_dashboard_base::file::{
    copy_atomic, is_temp_name, read_capped_bytes, write_atomic, OsHooks, BUSY_PAUSE, BUSY_RETRIES,
};
use engram_dashboard_platform::fs::{file_identity, retry_busy, sync_dir, FileIdentity, Retry};
use walkdir::WalkDir;

use crate::path::LibPath;

/// The root's Engram trash folder — a fixed name under the root, never walked ([`admits`] refuses dot names).
// TRD S25 B8 · G31
pub const TRASH_DIR: &str = ".trash";

/// Every disk access to the library. Paths are disk paths; `root` is the configured library root.
pub trait LibraryStore: Send + Sync {
    /// Lists the library under `root` (`under = None`) or the range `under` (a partial scan), drawing `seq()` before
    /// every step of the walk (module header). `Err` = a full scan whose root is missing, not a folder or cannot be
    /// listed at all — nothing of it may be applied (a missing root removes nothing). A partial scan never answers
    /// `Err` for its range: a link or a missing `under` is a sequenced absence ([`ScanListing::absent_ranges`]), any
    /// other failure is an error entry. `under` must pass [`admits`] (else `InvalidInput`).
    fn scan(
        &self,
        root: &Path,
        under: Option<&LibPath>,
        seq: &dyn Fn() -> u64,
    ) -> io::Result<ScanListing>;
    /// The first folder or file of `rel`, from the root down, that is a link (U1). The root itself is not counted.
    /// `Ok(None)` = no link, including when a name of `rel` does not exist (nothing below it can be a link).
    fn first_link(&self, root: &Path, rel: &LibPath) -> io::Result<Option<LibPath>>;
    /// The bytes of the file at `path`, at most `cap` bytes — a longer file is `InvalidData` (base `read_capped`'s
    /// rule), read only up to `cap + 1`.
    fn read(&self, path: &Path, cap: u64) -> io::Result<Vec<u8>>;
    fn tag(&self, path: &Path) -> io::Result<ContentTag>;
    /// Atomic write (base `write_atomic`). The parent folder must exist: a missing parent is `NotFound` and nothing
    /// is created (base would create it).
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    fn create_dir(&self, path: &Path) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Atomic copy (base `copy_atomic`).
    fn copy(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Moves `path` (a file or folder under `root`) into `<root>/.trash/<bundle>/<its path from the root>` by one
    /// same-root rename, creating the folders inside the trash first. Answers the new path. `InvalidInput` = `path`
    /// is not a library path below the root (the root itself, outside it, through `..`), or the bundle is not one
    /// plain name. `AlreadyExists` = something is already at the destination (a rename would replace a file there).
    /// A destination that appears between that check and the rename is not guarded.
    fn trash(&self, root: &Path, path: &Path, bundle: &TrashBundle) -> io::Result<PathBuf>;
    /// Whether anything is at `path` (a link counts — it is not followed).
    fn exists(&self, path: &Path) -> bool;
}

/// What one scan saw.
#[derive(Clone, Debug, Default)]
pub struct ScanListing {
    /// The library root the scan walked from — the base of every [`LibPath`] here.
    pub root: PathBuf,
    /// Admitted files, each with its tag and the number drawn before the step that yielded it.
    pub files: Vec<ScanEntry>,
    /// Admitted folders the walk listed (a range's own folder included).
    pub folders: Vec<LibPath>,
    /// Per listed folder (`None` = the root), the number drawn before the step that yielded it — every read of that
    /// folder's entries comes after it.
    pub listed_at: HashMap<Option<LibPath>, u64>,
    /// A whole range observed absent — a partial scan's `under` that is a link or lies below one, or that was not
    /// found — with the number drawn before that observation.
    pub absent_ranges: Vec<(LibPath, u64)>,
    /// Links below the root that were not entered (and a partial scan's `under` when it lies at or below one).
    pub skipped_links: Vec<LibPath>,
    /// Where the walk failed: a folder whose listing failed at open or part-way (`None` = the root), or a file whose
    /// metadata failed; with the error text.
    pub errors: Vec<(Option<LibPath>, String)>,
}

/// One admitted file of a scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanEntry {
    pub rel: LibPath,
    pub tag: ContentTag,
    /// The number drawn before the step that yielded this file. Its tag and bytes are read after it, so it is the
    /// read sequence of an item made from them.
    pub seq: u64,
}

/// What decides whether a file is re-read: equal tags = unchanged (equality, not "newer").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentTag {
    pub modified: SystemTime,
    pub size: u64,
    /// `None` = the volume gives no per-file identity (platform `file_identity` answered `Unsupported`) — not a
    /// failure.
    pub identity: Option<FileIdentity>,
}

/// The bundle folder name under `.trash` — its naming is step 9b's; this crate only passes it through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashBundle(pub String);

/// The name rule every way into the catalog shares (scan, safety net, late merge): no name starting with `.` and no
/// base temp name (`base::file::is_temp_name` — the shape is base's, not copied here).
pub fn admits(rel: &LibPath) -> bool {
    rel.as_str()
        .split('/')
        .all(|name| !name.starts_with('.') && !is_temp_name(name))
}

/// The production store: walkdir + std + base's atomic write and copy with platform's lock retry and folder sync.
#[derive(Clone, Copy, Debug, Default)]
pub struct FsStore;

// ADR-0291
const OS_HOOKS: OsHooks = OsHooks {
    retry: retry_locked,
    sync_dir,
};

fn retry_locked(attempt: &mut dyn FnMut() -> io::Result<()>) -> io::Result<()> {
    retry_busy(
        Retry {
            retries: BUSY_RETRIES,
            pause: BUSY_PAUSE,
        },
        attempt,
    )
}

impl LibraryStore for FsStore {
    fn scan(
        &self,
        root: &Path,
        under: Option<&LibPath>,
        seq: &dyn Fn() -> u64,
    ) -> io::Result<ScanListing> {
        let mut listing = ScanListing {
            root: root.to_path_buf(),
            ..ScanListing::default()
        };
        let start = match under {
            None => {
                let meta = fs::metadata(root)?;
                if !meta.is_dir() {
                    return Err(io::Error::new(
                        io::ErrorKind::NotADirectory,
                        "the library root is not a folder",
                    ));
                }
                root.to_path_buf()
            }
            Some(under) => {
                refuse_unadmitted(under)?;
                let at = seq();
                match self.first_link(root, under) {
                    Ok(None) => {}
                    Ok(Some(_)) => {
                        listing.skipped_links.push(under.clone());
                        listing.absent_ranges.push((under.clone(), at));
                        return Ok(listing);
                    }
                    Err(e) => {
                        listing.errors.push((Some(under.clone()), e.to_string()));
                        return Ok(listing);
                    }
                }
                under.to_path(root)
            }
        };
        let top = under.cloned();
        // Only the root may be a link: a full scan follows a root link, nothing else is followed. A partial scan's
        //   `under` passed `first_link`; if it became a link since, it is yielded unfollowed and skipped below.
        let mut walk = WalkDir::new(&start)
            .follow_links(false)
            .follow_root_links(under.is_none())
            .into_iter()
            .filter_entry(|entry| entry.depth() == 0 || admitted_on_disk(root, entry.path()));
        // The folder open at each depth — an entry error without a path belongs to the folder one level up.
        let mut open: Vec<Option<LibPath>> = Vec::new();
        loop {
            // Drawn before the step: if it yields folder D, walkdir has opened D inside it and reads D's entries only
            //   in later steps (premise 47) — so this number precedes every read of D.
            let at = seq();
            let Some(step) = walk.next() else { break };
            match step {
                Ok(entry) => {
                    let depth = entry.depth();
                    let rel = if depth == 0 {
                        top.clone()
                    } else {
                        // `filter_entry` admitted it, so the name converts.
                        LibPath::from_disk(root, entry.path()).ok()
                    };
                    // Below the root, or a partial scan's `under` that became a link after `first_link`.
                    if entry.path_is_symlink() && (depth > 0 || under.is_some()) {
                        let rel = rel.expect("a link below the root has a library path");
                        if depth == 0 {
                            listing.absent_ranges.push((rel.clone(), at));
                        }
                        listing.skipped_links.push(rel);
                        continue;
                    }
                    let is_root = depth == 0 && under.is_none();
                    // The root may have changed since the check above. Walk it only while it is a folder (a followed
                    //   link to a folder counts): a root listed with nothing in it would make every item absent.
                    if is_root && !entry.file_type().is_dir() && !followed_folder(&entry) {
                        return Err(io::Error::new(
                            io::ErrorKind::NotADirectory,
                            "the library root is not a folder",
                        ));
                    }
                    if is_root || entry.file_type().is_dir() {
                        open.truncate(depth);
                        open.push(rel.clone());
                        listing.listed_at.insert(rel.clone(), at);
                        if let Some(rel) = rel {
                            listing.folders.push(rel);
                        }
                    } else if entry.file_type().is_file() {
                        let rel = rel.expect("a file below the root has a library path");
                        match self.tag(entry.path()) {
                            Ok(tag) => listing.files.push(ScanEntry { rel, tag, seq: at }),
                            // Deleted after its folder was read: not listed — an absence observed after that folder's
                            //   `listed_at`, so the folder's number stays valid for it.
                            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                            Err(e) => listing.errors.push((Some(rel), e.to_string())),
                        }
                    }
                }
                Err(error) => {
                    let not_found = error
                        .io_error()
                        .is_some_and(|e| e.kind() == io::ErrorKind::NotFound);
                    let placement = place_error(
                        error.path(),
                        error.depth(),
                        not_found,
                        root,
                        &start,
                        &top,
                        &open,
                    );
                    match placement {
                        Placement::Start { not_found } => match under {
                            None => return Err(into_io(error)),
                            Some(under) if not_found => {
                                // The open came inside the step that yielded `under` (or failed before it).
                                let opened_at = listing.listed_at.get(&top).copied().unwrap_or(at);
                                listing.absent_ranges.push((under.clone(), opened_at));
                                return Ok(listing);
                            }
                            Some(under) => {
                                listing
                                    .errors
                                    .push((Some(under.clone()), error.to_string()));
                                return Ok(listing);
                            }
                        },
                        Placement::Vanished(gone) => {
                            // Not a folder of the listing: its former entries count as absent at its parent's
                            //   (earlier) number.
                            if let Some(gone) = gone {
                                listing.listed_at.remove(&Some(gone.clone()));
                                listing.folders.retain(|folder| *folder != gone);
                            }
                        }
                        Placement::Charged(place) => {
                            if error.path().is_none() {
                                // A part-way enumeration failure: leave that folder, so a failing list is not polled
                                //   again and the rest of it is kept.
                                walk.skip_current_dir();
                            }
                            listing.errors.push((place, error.to_string()));
                        }
                    }
                }
            }
        }
        Ok(listing)
    }

    fn first_link(&self, root: &Path, rel: &LibPath) -> io::Result<Option<LibPath>> {
        let mut path = root.to_path_buf();
        let mut end = 0;
        for name in rel.as_str().split('/') {
            path.push(name);
            end += name.len();
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    let prefix = LibPath::parse(&rel.as_str()[..end])
                        .expect("a leading run of names of a library path is one");
                    return Ok(Some(prefix));
                }
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
            }
            end += 1;
        }
        Ok(None)
    }

    fn read(&self, path: &Path, cap: u64) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        (OS_HOOKS.retry)(&mut || {
            bytes = read_capped_bytes(fs::File::open(path)?, cap)?;
            Ok(())
        })?;
        Ok(bytes)
    }

    fn tag(&self, path: &Path) -> io::Result<ContentTag> {
        let meta = fs::metadata(path)?;
        content_tag(meta.modified()?, meta.len(), file_identity(path))
    }

    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent folder"))?;
        if !fs::metadata(parent)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "the parent is not a folder",
            ));
        }
        // The gap between this check and base creating a missing folder remains (TRD S25 step-7 B4).
        write_atomic(path, bytes, OS_HOOKS)
    }

    fn create_dir(&self, path: &Path) -> io::Result<()> {
        fs::create_dir(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        (OS_HOOKS.retry)(&mut || fs::rename(from, to))
    }

    fn copy(&self, from: &Path, to: &Path) -> io::Result<()> {
        copy_atomic(from, to, OS_HOOKS)
    }

    fn trash(&self, root: &Path, path: &Path, bundle: &TrashBundle) -> io::Result<PathBuf> {
        let to = trash_path(root, path, bundle)?;
        destination_free(&to, fs::symlink_metadata(&to).map(drop))?;
        fs::create_dir_all(to.parent().expect("a trash path has a parent"))?;
        self.rename(path, &to)?;
        Ok(to)
    }

    fn exists(&self, path: &Path) -> bool {
        fs::symlink_metadata(path).is_ok()
    }
}

fn refuse_unadmitted(under: &LibPath) -> io::Result<()> {
    if admits(under) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a library path the scan admits", under.as_str()),
        ))
    }
}

// A name that cannot be a library path (not UTF-8, a trailing '.' or space, a reserved device name) can never be an
//   item; it is left out like a dot name and logged.
fn admitted_on_disk(root: &Path, path: &Path) -> bool {
    match LibPath::from_disk(root, path) {
        Ok(rel) => admits(&rel),
        Err(e) => {
            tracing::warn!(
                ?path,
                "라이브러리 훑기: 라이브러리 경로가 될 수 없는 이름이라 뺐다: {e}"
            );
            false
        }
    }
}

fn into_io(error: walkdir::Error) -> io::Error {
    let text = error.to_string();
    error
        .into_io_error()
        .unwrap_or_else(|| io::Error::other(text))
}

/// A depth-0 entry that is a link walkdir followed: a link whose target is a folder.
fn followed_folder(entry: &walkdir::DirEntry) -> bool {
    entry.path_is_symlink() && fs::metadata(entry.path()).is_ok_and(|meta| meta.is_dir())
}

/// Where a walk error lands.
#[derive(Debug, PartialEq)]
enum Placement {
    /// The start of the walk itself could not be read or opened.
    Start { not_found: bool },
    /// An entry (folder or file) that vanished after its parent listed it — as if never listed. The path, when it is a
    /// library path (`None` = it is not one, so nothing of the listing names it).
    Vanished(Option<LibPath>),
    /// A failure charged to this place (`None` = the root): its items are kept.
    Charged(Option<LibPath>),
}

/// The place of one walkdir error. walkdir gives a path for a failure to read or open that path (`depth` = its own
/// depth) and no path for a failure while enumerating a folder (`depth` = that folder's depth + 1), so a path-less
/// error is charged to the folder open one level up (`open[depth - 1]`). A path that is no library path is charged to
/// the whole range (`top`).
fn place_error(
    path: Option<&Path>,
    depth: usize,
    not_found: bool,
    root: &Path,
    start: &Path,
    top: &Option<LibPath>,
    open: &[Option<LibPath>],
) -> Placement {
    match path {
        Some(path) if path == start => Placement::Start { not_found },
        Some(path) if not_found => Placement::Vanished(LibPath::from_disk(root, path).ok()),
        Some(path) => {
            Placement::Charged(LibPath::from_disk(root, path).map_or_else(|_| top.clone(), Some))
        }
        None => Placement::Charged(
            open.get(depth.wrapping_sub(1))
                .cloned()
                .unwrap_or_else(|| top.clone()),
        ),
    }
}

/// Whether the trash destination `to` is free, from the answer of a lookup of it: only `NotFound` is free; something
/// there is `AlreadyExists`; any other lookup failure is returned as is (it does not show the place is empty, and a
/// rename would replace a file there).
fn destination_free(to: &Path, lookup: io::Result<()>) -> io::Result<()> {
    match lookup {
        Ok(()) => Err(already_in_trash(to)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn already_in_trash(to: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("{} is already in the trash", to.display()),
    )
}

/// A tag from the metadata and the identity answer: `Unsupported` identity = `None` (TRD S25 step-7 premise 20).
fn content_tag(
    modified: SystemTime,
    size: u64,
    identity: io::Result<FileIdentity>,
) -> io::Result<ContentTag> {
    let identity = match identity {
        Ok(identity) => Some(identity),
        Err(e) if e.kind() == io::ErrorKind::Unsupported => None,
        Err(e) => return Err(e),
    };
    Ok(ContentTag {
        modified,
        size,
        identity,
    })
}

fn trash_path(root: &Path, path: &Path, bundle: &TrashBundle) -> io::Result<PathBuf> {
    // Through `LibPath`: names only, so `..` cannot leave the root and the root itself is refused.
    let rel = LibPath::from_disk(root, path).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not a library path below the root: {e}"),
        )
    })?;
    let plain = LibPath::parse(&bundle.0).is_ok_and(|name| !name.as_str().contains('/'));
    if !plain {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the trash bundle is not one plain name",
        ));
    }
    Ok(rel.to_path(&root.join(TRASH_DIR).join(&bundle.0)))
}

/// The in-memory store for tests: folders, files and links under one root, with failure injection. Its scan keeps
/// the [`LibraryStore`] rules (links, names, a `seq()` draw before every step, errors as entries) without the store
/// lock held while `seq` runs, so a test's `seq` may change the store between steps.
#[cfg(test)]
pub(crate) mod mem {
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{
        admits, trash_path, ContentTag, LibraryStore, ScanEntry, ScanListing, TrashBundle,
    };
    use crate::path::LibPath;

    #[derive(Clone, Debug)]
    enum Node {
        Dir,
        File {
            bytes: Vec<u8>,
            modified: SystemTime,
        },
        Link,
    }

    /// How listing a folder fails.
    #[derive(Clone, Copy, Debug)]
    pub(crate) enum ListFailure {
        /// The folder cannot be opened (e.g. permission).
        Open,
        /// The first `after` entries are listed, then the enumeration fails.
        PartWay { after: usize },
    }

    #[derive(Default)]
    struct State {
        nodes: BTreeMap<PathBuf, Node>,
        list_failures: HashMap<PathBuf, ListFailure>,
        tag_failures: HashSet<PathBuf>,
        read_failures: HashSet<PathBuf>,
    }

    pub(crate) struct MemStore {
        root: PathBuf,
        state: Mutex<State>,
    }

    /// A modification time far from any test's `now`, so a file is not racy unless a test says so.
    pub(crate) const OLD: SystemTime = UNIX_EPOCH;

    impl MemStore {
        pub(crate) fn new(root: &Path) -> Self {
            let mut state = State::default();
            state.nodes.insert(root.to_path_buf(), Node::Dir);
            Self {
                root: root.to_path_buf(),
                state: Mutex::new(state),
            }
        }

        fn at(&self, rel: &str) -> PathBuf {
            LibPath::parse(rel).unwrap().to_path(&self.root)
        }

        /// A file at `rel` (folders above it are made), modified at [`OLD`] + `version` seconds — a new version
        /// gives a new tag.
        pub(crate) fn file(&self, rel: &str, text: &str) {
            self.file_at(rel, text, OLD);
        }

        pub(crate) fn file_version(&self, rel: &str, text: &str, version: u64) {
            self.file_at(rel, text, OLD + Duration::from_secs(version));
        }

        pub(crate) fn file_at(&self, rel: &str, text: &str, modified: SystemTime) {
            let path = self.at(rel);
            let mut state = self.state.lock().unwrap();
            make_parents(&mut state, &self.root, &path);
            let bytes = text.as_bytes().to_vec();
            state.nodes.insert(path, Node::File { bytes, modified });
        }

        /// A link at `rel` — the store models it as an opaque entry a walk does not enter.
        pub(crate) fn link(&self, rel: &str) {
            let path = self.at(rel);
            let mut state = self.state.lock().unwrap();
            make_parents(&mut state, &self.root, &path);
            remove_tree(&mut state, &path);
            state.nodes.insert(path, Node::Link);
        }

        /// Removes `rel` and everything below it.
        pub(crate) fn remove(&self, rel: &str) {
            let path = self.at(rel);
            remove_tree(&mut self.state.lock().unwrap(), &path);
        }

        /// Listing `rel` (`""` = the root) fails as `failure` from now on.
        pub(crate) fn fail_listing(&self, rel: &str, failure: ListFailure) {
            let path = if rel.is_empty() {
                self.root.clone()
            } else {
                self.at(rel)
            };
            self.state
                .lock()
                .unwrap()
                .list_failures
                .insert(path, failure);
        }

        pub(crate) fn fail_tag(&self, rel: &str) {
            let path = self.at(rel);
            self.state.lock().unwrap().tag_failures.insert(path);
        }

        pub(crate) fn fail_read(&self, rel: &str) {
            let path = self.at(rel);
            self.state.lock().unwrap().read_failures.insert(path);
        }

        /// A read closure for `ScanResult::build` over this store.
        pub(crate) fn reader(&self) -> impl Fn(&Path) -> io::Result<Vec<u8>> + '_ {
            |path| self.read(path, u64::MAX)
        }

        fn children(state: &State, folder: &Path) -> Vec<PathBuf> {
            state
                .nodes
                .keys()
                .filter(|path| path.parent() == Some(folder))
                .cloned()
                .collect()
        }

        fn rel(&self, path: &Path) -> LibPath {
            LibPath::from_disk(&self.root, path).unwrap()
        }
    }

    fn make_parents(state: &mut State, root: &Path, path: &Path) {
        let mut parent = path.parent();
        while let Some(folder) = parent {
            if !folder.starts_with(root) {
                break;
            }
            state.nodes.entry(folder.to_path_buf()).or_insert(Node::Dir);
            parent = folder.parent();
        }
    }

    fn remove_tree(state: &mut State, path: &Path) {
        state.nodes.retain(|p, _| !p.starts_with(path));
    }

    fn not_found() -> io::Error {
        io::Error::from(io::ErrorKind::NotFound)
    }

    // One folder being listed: what is left to yield, and how its enumeration ends.
    struct Open {
        rel: Option<LibPath>,
        left: Vec<PathBuf>,
        failure: Option<io::Error>,
    }

    impl LibraryStore for MemStore {
        fn scan(
            &self,
            root: &Path,
            under: Option<&LibPath>,
            seq: &dyn Fn() -> u64,
        ) -> io::Result<ScanListing> {
            assert_eq!(root, self.root, "MemStore has one root");
            let mut listing = ScanListing {
                root: root.to_path_buf(),
                ..ScanListing::default()
            };
            let start = match under {
                None => root.to_path_buf(),
                Some(under) => {
                    super::refuse_unadmitted(under)?;
                    let at = seq();
                    match self.first_link(root, under)? {
                        Some(_) => {
                            listing.skipped_links.push(under.clone());
                            listing.absent_ranges.push((under.clone(), at));
                            return Ok(listing);
                        }
                        None => under.to_path(root),
                    }
                }
            };
            let top = under.cloned();
            let mut stack: Vec<Open> = Vec::new();
            {
                let at = seq();
                let state = self.state.lock().unwrap();
                let node = state.nodes.get(&start).cloned();
                match (node, under) {
                    (Some(Node::Dir), _) => {}
                    // As FsStore: a range that is a file is listed as that file.
                    (Some(Node::File { .. }), Some(under)) => {
                        drop(state);
                        match self.tag(&start) {
                            Ok(tag) => listing.files.push(ScanEntry {
                                rel: under.clone(),
                                tag,
                                seq: at,
                            }),
                            Err(e) => listing.errors.push((Some(under.clone()), e.to_string())),
                        }
                        return Ok(listing);
                    }
                    // A range that became a link after the link check.
                    (Some(Node::Link), Some(under)) => {
                        listing.skipped_links.push(under.clone());
                        listing.absent_ranges.push((under.clone(), at));
                        return Ok(listing);
                    }
                    (Some(_), None) => {
                        return Err(io::Error::from(io::ErrorKind::NotADirectory));
                    }
                    (None, None) => return Err(not_found()),
                    (None, Some(under)) => {
                        listing.absent_ranges.push((under.clone(), at));
                        return Ok(listing);
                    }
                }
                listing.listed_at.insert(top.clone(), at);
                if let Some(rel) = &top {
                    listing.folders.push(rel.clone());
                }
                stack.push(open_folder(&state, &start, top.clone()));
                if under.is_none()
                    && matches!(state.list_failures.get(&start), Some(ListFailure::Open))
                {
                    return Err(io::Error::from(io::ErrorKind::PermissionDenied));
                }
            }
            while !stack.is_empty() {
                let at = seq();
                let state = self.state.lock().unwrap();
                let top_open = stack.last_mut().unwrap();
                let Some(path) = (!top_open.left.is_empty()).then(|| top_open.left.remove(0))
                else {
                    let done = stack.pop().unwrap();
                    if let Some(error) = done.failure {
                        listing.errors.push((done.rel, error.to_string()));
                    }
                    continue;
                };
                // An entry deleted after its folder was listed is not yielded (a later read finds nothing).
                let Some(node) = state.nodes.get(&path).cloned() else {
                    continue;
                };
                let rel = self.rel(&path);
                if !admits(&rel) {
                    continue;
                }
                match node {
                    Node::Link => listing.skipped_links.push(rel),
                    Node::Dir => {
                        listing.listed_at.insert(Some(rel.clone()), at);
                        listing.folders.push(rel.clone());
                        stack.push(open_folder(&state, &path, Some(rel)));
                    }
                    Node::File { .. } => {
                        drop(state);
                        match self.tag(&path) {
                            Ok(tag) => listing.files.push(ScanEntry { rel, tag, seq: at }),
                            Err(e) => listing.errors.push((Some(rel), e.to_string())),
                        }
                    }
                }
            }
            Ok(listing)
        }

        fn first_link(&self, root: &Path, rel: &LibPath) -> io::Result<Option<LibPath>> {
            let state = self.state.lock().unwrap();
            let mut path = root.to_path_buf();
            let mut names: Vec<&str> = Vec::new();
            for name in rel.as_str().split('/') {
                path.push(name);
                names.push(name);
                match state.nodes.get(&path) {
                    Some(Node::Link) => return Ok(Some(LibPath::parse(&names.join("/")).unwrap())),
                    Some(_) => {}
                    None => return Ok(None),
                }
            }
            Ok(None)
        }

        fn read(&self, path: &Path, cap: u64) -> io::Result<Vec<u8>> {
            let state = self.state.lock().unwrap();
            if state.read_failures.contains(path) {
                return Err(io::Error::other("injected read failure"));
            }
            match state.nodes.get(path) {
                Some(Node::File { bytes, .. }) if bytes.len() as u64 > cap => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "larger than the cap",
                )),
                Some(Node::File { bytes, .. }) => Ok(bytes.clone()),
                Some(_) => Err(io::Error::from(io::ErrorKind::IsADirectory)),
                None => Err(not_found()),
            }
        }

        fn tag(&self, path: &Path) -> io::Result<ContentTag> {
            let state = self.state.lock().unwrap();
            if state.tag_failures.contains(path) {
                return Err(io::Error::other("injected metadata failure"));
            }
            match state.nodes.get(path) {
                Some(Node::File { bytes, modified }) => Ok(ContentTag {
                    modified: *modified,
                    size: bytes.len() as u64,
                    identity: None,
                }),
                Some(_) => Err(io::Error::from(io::ErrorKind::IsADirectory)),
                None => Err(not_found()),
            }
        }

        fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            let mut state = self.state.lock().unwrap();
            match path.parent().and_then(|parent| state.nodes.get(parent)) {
                Some(Node::Dir) => {}
                Some(_) => return Err(io::Error::from(io::ErrorKind::NotADirectory)),
                None => return Err(not_found()),
            }
            let modified = SystemTime::now();
            let bytes = bytes.to_vec();
            state
                .nodes
                .insert(path.to_path_buf(), Node::File { bytes, modified });
            Ok(())
        }

        fn create_dir(&self, path: &Path) -> io::Result<()> {
            let mut state = self.state.lock().unwrap();
            if state.nodes.contains_key(path) {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            match path.parent().and_then(|parent| state.nodes.get(parent)) {
                Some(Node::Dir) => {}
                _ => return Err(not_found()),
            }
            state.nodes.insert(path.to_path_buf(), Node::Dir);
            Ok(())
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            let mut state = self.state.lock().unwrap();
            if !state.nodes.contains_key(from) {
                return Err(not_found());
            }
            match to.parent().and_then(|parent| state.nodes.get(parent)) {
                Some(Node::Dir) => {}
                _ => return Err(not_found()),
            }
            let moved: Vec<(PathBuf, Node)> = state
                .nodes
                .iter()
                .filter(|(path, _)| path.starts_with(from))
                .map(|(path, node)| (path.clone(), node.clone()))
                .collect();
            remove_tree(&mut state, from);
            for (path, node) in moved {
                let rest = path.strip_prefix(from).unwrap();
                let target = if rest.as_os_str().is_empty() {
                    to.to_path_buf()
                } else {
                    to.join(rest)
                };
                state.nodes.insert(target, node);
            }
            Ok(())
        }

        fn copy(&self, from: &Path, to: &Path) -> io::Result<()> {
            let mut state = self.state.lock().unwrap();
            let node = match state.nodes.get(from) {
                Some(node @ Node::File { .. }) => node.clone(),
                Some(_) => return Err(io::Error::from(io::ErrorKind::IsADirectory)),
                None => return Err(not_found()),
            };
            make_parents(&mut state, &self.root, to);
            state.nodes.insert(to.to_path_buf(), node);
            Ok(())
        }

        fn trash(&self, root: &Path, path: &Path, bundle: &TrashBundle) -> io::Result<PathBuf> {
            let to = trash_path(root, path, bundle)?;
            {
                let mut state = self.state.lock().unwrap();
                if state.nodes.contains_key(&to) {
                    return Err(super::already_in_trash(&to));
                }
                make_parents(&mut state, root, &to);
            }
            self.rename(path, &to)?;
            Ok(to)
        }

        fn exists(&self, path: &Path) -> bool {
            self.state.lock().unwrap().nodes.contains_key(path)
        }
    }

    fn open_folder(state: &State, path: &Path, rel: Option<LibPath>) -> Open {
        let mut left = MemStore::children(state, path);
        let failure = match state.list_failures.get(path) {
            None => None,
            Some(ListFailure::Open) => {
                left.clear();
                Some(io::Error::from(io::ErrorKind::PermissionDenied))
            }
            Some(ListFailure::PartWay { after }) => {
                left.truncate(*after);
                Some(io::Error::other("injected enumeration failure"))
            }
        };
        Open { rel, left, failure }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::HashSet;

    use engram_dashboard_platform::fs::link_dir;

    use super::*;

    fn lib(text: &str) -> LibPath {
        LibPath::parse(text).unwrap()
    }

    fn write(root: &Path, rel: &str, text: &str) -> PathBuf {
        let path = lib(rel).to_path(root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }

    fn counter() -> impl Fn() -> u64 {
        let next = Cell::new(0);
        move || {
            next.set(next.get() + 1);
            next.get()
        }
    }

    fn file_names(listing: &ScanListing) -> Vec<&str> {
        let mut names: Vec<&str> = listing.files.iter().map(|f| f.rel.as_str()).collect();
        names.sort_unstable();
        names
    }

    fn folder_names(listing: &ScanListing) -> Vec<&str> {
        let mut names: Vec<&str> = listing.folders.iter().map(LibPath::as_str).collect();
        names.sort_unstable();
        names
    }

    // ── names ─────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn admits_refuses_dot_names_and_base_temp_names_at_any_depth() {
        for good in ["a.md", "rules/a.md", "a.tmp", "a.md.tmp", "x/y.tmp12"] {
            assert!(admits(&lib(good)), "{good}");
        }
        for bad in [
            ".trash",
            ".trash/b/a.md",
            "rules/.x/a.md",
            ".a.md",
            "a.md.tmp4120.0",
            "x/p.yaml.tmp1.2/y.md",
        ] {
            assert!(!admits(&lib(bad)), "{bad}");
        }
    }

    #[test]
    fn a_real_scan_skips_trash_dot_names_and_temp_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "rules/a.md", "a");
        write(root, "top.md", "t");
        write(root, ".trash/b1/rules/old.md", "o");
        write(root, ".git/x.md", "g");
        write(root, ".hidden.md", "h");
        write(root, "rules/.private/p.md", "p");
        write(root, "rules/review.yaml.tmp4120.0", "kind: preset");
        let listing = FsStore.scan(root, None, &counter()).unwrap();
        assert_eq!(file_names(&listing), ["rules/a.md", "top.md"]);
        assert_eq!(folder_names(&listing), ["rules"]);
        assert!(listing.errors.is_empty(), "{:?}", listing.errors);
        assert!(listing.skipped_links.is_empty());
    }

    #[test]
    fn a_leftover_temp_file_is_neither_overwritten_nor_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let leftover = write(root, "p.yaml.tmp4120.0", "left over");
        FsStore
            .write_atomic(&root.join("p.yaml"), b"kind: preset\n")
            .unwrap();
        FsStore.scan(root, None, &counter()).unwrap();
        assert_eq!(fs::read_to_string(&leftover).unwrap(), "left over");
        assert_eq!(
            fs::read_to_string(root.join("p.yaml")).unwrap(),
            "kind: preset\n"
        );
    }

    #[test]
    fn write_atomic_with_the_parent_gone_is_not_found_and_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone");
        let error = FsStore.write_atomic(&gone.join("a.md"), b"x").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound, "{error:?}");
        assert!(!gone.exists(), "the folder is not recreated");
    }

    #[test]
    fn trash_is_a_same_root_rename_into_the_bundle_keeping_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = write(root, "rules/a.md", "body");
        let before = file_identity(&file).ok();
        let moved = FsStore
            .trash(root, &file, &TrashBundle("b1".to_owned()))
            .unwrap();
        assert_eq!(
            moved,
            root.join(".trash").join("b1").join("rules").join("a.md")
        );
        assert!(!file.exists());
        assert_eq!(fs::read_to_string(&moved).unwrap(), "body");
        // A rename keeps the file's identity; a copy would not.
        assert_eq!(file_identity(&moved).ok(), before);
    }

    #[test]
    fn trash_refuses_the_root_a_path_outside_it_and_a_bundle_that_is_not_one_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("lib");
        let file = write(&root, "a.md", "x");
        let bundle = TrashBundle("b1".to_owned());
        for path in [root.clone(), dir.path().join("elsewhere.md")] {
            let error = FsStore.trash(&root, &path, &bundle).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{path:?}");
        }
        for bad in ["", "a/b", "..", "a\\b"] {
            let error = FsStore
                .trash(&root, &file, &TrashBundle(bad.to_owned()))
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{bad:?}");
        }
        assert!(file.exists());
    }

    #[test]
    fn read_answers_invalid_data_past_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "a.md", "12345");
        assert_eq!(FsStore.read(&file, 5).unwrap(), b"12345");
        let error = FsStore.read(&file, 4).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error:?}");
    }

    #[test]
    fn unsupported_identity_is_none_and_other_identity_errors_fail_the_tag() {
        let modified = SystemTime::UNIX_EPOCH;
        let tag = content_tag(modified, 3, Err(io::ErrorKind::Unsupported.into())).unwrap();
        assert_eq!(tag.identity, None);
        assert_eq!((tag.modified, tag.size), (modified, 3));
        let error =
            content_tag(modified, 3, Err(io::ErrorKind::PermissionDenied.into())).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn a_real_tag_changes_with_the_content_size() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "a.md", "one");
        let first = FsStore.tag(&file).unwrap();
        assert_eq!(first.size, 3);
        fs::write(&file, "three").unwrap();
        assert_ne!(FsStore.tag(&file).unwrap(), first);
    }

    #[test]
    fn a_full_scan_of_a_missing_root_or_a_file_root_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let error = FsStore
            .scan(&dir.path().join("gone"), None, &counter())
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        let file = write(dir.path(), "file", "x");
        assert!(FsStore.scan(&file, None, &counter()).is_err());
    }

    // ── links (U1) ────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_root_that_is_a_folder_link_is_followed() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        write(&real, "rules/a.md", "a");
        let root = dir.path().join("linked-root");
        link_dir(&real, &root).unwrap();
        let listing = FsStore.scan(&root, None, &counter()).unwrap();
        assert_eq!(file_names(&listing), ["rules/a.md"]);
        assert!(listing.skipped_links.is_empty());
    }

    #[test]
    fn a_folder_link_inside_is_neither_entered_nor_listed_and_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        write(&outside, "b.md", "b");
        let root = dir.path().join("lib");
        write(&root, "rules/a.md", "a");
        link_dir(&outside, &root.join("rules").join("linked")).unwrap();
        let listing = FsStore.scan(&root, None, &counter()).unwrap();
        assert_eq!(file_names(&listing), ["rules/a.md"]);
        assert_eq!(folder_names(&listing), ["rules"]);
        assert_eq!(listing.skipped_links, [lib("rules/linked")]);
    }

    #[test]
    fn a_partial_scan_at_or_below_an_internal_link_walks_nothing_and_reports_it() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        write(&outside, "sub/b.md", "b");
        let root = dir.path().join("lib");
        write(&root, "a.md", "a");
        link_dir(&outside, &root.join("linked")).unwrap();
        for under in ["linked", "linked/sub"] {
            let draws = Cell::new(0);
            let seq = || {
                draws.set(draws.get() + 1);
                40 + draws.get()
            };
            let listing = FsStore.scan(&root, Some(&lib(under)), &seq).unwrap();
            assert!(
                listing.files.is_empty() && listing.folders.is_empty(),
                "{under}"
            );
            assert_eq!(listing.skipped_links, [lib(under)]);
            // The absence number is the one drawn before the link check — the only draw.
            assert_eq!(listing.absent_ranges, [(lib(under), 41)], "{under}");
            assert_eq!(draws.get(), 1);
        }
    }

    #[test]
    fn first_link_finds_the_first_link_below_the_root_only() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        write(&real, "plain/a.md", "a");
        let outside = dir.path().join("outside");
        write(&outside, "inner/b.md", "b");
        link_dir(&outside, &real.join("mid")).unwrap();
        link_dir(&outside.join("inner"), &real.join("plain").join("leaf")).unwrap();
        let root = dir.path().join("root-link");
        link_dir(&real, &root).unwrap();
        let table = [
            ("plain/a.md", None),
            ("plain/leaf", Some("plain/leaf")),
            ("mid/inner/b.md", Some("mid")),
            ("missing/x.md", None),
        ];
        for (rel, want) in table {
            // Through the root link: the root itself is not counted.
            let found = FsStore.first_link(&root, &lib(rel)).unwrap();
            assert_eq!(found, want.map(lib), "{rel}");
        }
    }

    // ── the per-folder listing sequence on the real walk (premise 47) ─────────────────────────────────────

    #[test]
    fn listed_at_is_the_draw_before_the_step_that_yields_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let x = write(&root, "D/x.md", "x");
        write(&root, "D/y.md", "y");
        let z = lib("D/z.md").to_path(&root);
        let draws = Cell::new(0u64);
        // Step 1 yields the root, step 2 yields D (the root's only entry). Inside draw 2 — after the root was
        //   listed, before the step that opens D — x is deleted and z is created. Had walkdir read D's entries
        //   before that draw, z would be missing. Measured limit (P5 QA, 2026-11-12, Windows): this test still
        //   passes with the draw moved after the step that opens D, so it does not pin that order by itself; the
        //   order is caught by `a_full_scan_whose_root_becomes_a_file_before_the_walk_is_an_error` and
        //   `a_folder_that_vanishes_before_its_open_is_not_in_the_listing`, which fail under that mutation.
        let seq = || {
            draws.set(draws.get() + 1);
            if draws.get() == 2 {
                fs::remove_file(&x).unwrap();
                fs::write(&z, "z").unwrap();
            }
            draws.get()
        };
        let listing = FsStore.scan(&root, None, &seq).unwrap();
        assert_eq!(listing.listed_at[&None], 1);
        assert_eq!(listing.listed_at[&Some(lib("D"))], 2);
        assert_eq!(
            file_names(&listing),
            ["D/y.md", "D/z.md"],
            "D was read after draw 2: x gone, z there"
        );
        assert!(listing.errors.is_empty(), "{:?}", listing.errors);
        for entry in &listing.files {
            assert!(entry.seq > listing.listed_at[&Some(lib("D"))], "{entry:?}");
        }
    }

    #[test]
    fn a_partial_scan_of_a_deleted_range_is_a_sequenced_absence() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "keep.md", "k");
        let listing = FsStore.scan(root, Some(&lib("gone")), &counter()).unwrap();
        // Draw 1 = before the link check, draw 2 = before the step that opens `gone`.
        assert_eq!(listing.absent_ranges, [(lib("gone"), 2)]);
        assert!(listing.errors.is_empty() && listing.files.is_empty());
    }

    #[test]
    fn a_partial_scan_lists_only_its_range_and_records_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a.md", "a");
        write(root, "rules/b.md", "b");
        write(root, "rules/deep/c.md", "c");
        let listing = FsStore.scan(root, Some(&lib("rules")), &counter()).unwrap();
        assert_eq!(file_names(&listing), ["rules/b.md", "rules/deep/c.md"]);
        assert_eq!(folder_names(&listing), ["rules", "rules/deep"]);
        let keys: HashSet<_> = listing.listed_at.keys().cloned().collect();
        assert_eq!(
            keys,
            HashSet::from([Some(lib("rules")), Some(lib("rules/deep"))])
        );
    }

    #[test]
    fn a_partial_scan_refuses_an_unadmitted_range() {
        let dir = tempfile::tempdir().unwrap();
        let error = FsStore
            .scan(dir.path(), Some(&lib(".trash")), &counter())
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    // ── MemStore keeps the same rules ─────────────────────────────────────────────────────────────────────

    #[test]
    fn mem_store_scan_reports_links_failures_and_names() {
        use mem::{ListFailure, MemStore};
        let root = Path::new("/lib");
        let store = MemStore::new(root);
        store.file("a.md", "a");
        store.file(".trash/b/x.md", "x");
        store.file("p.yaml.tmp1.0", "t");
        store.link("linked");
        store.file("locked/c.md", "c");
        store.fail_listing("locked", ListFailure::Open);
        store.file("half/1.md", "1");
        store.file("half/2.md", "2");
        store.fail_listing("half", ListFailure::PartWay { after: 1 });
        store.file("bad.md", "b");
        store.fail_tag("bad.md");
        let listing = store.scan(root, None, &counter()).unwrap();
        assert_eq!(file_names(&listing), ["a.md", "half/1.md"]);
        assert_eq!(listing.skipped_links, [lib("linked")]);
        let mut failed: Vec<_> = listing.errors.iter().map(|(at, _)| at.clone()).collect();
        failed.sort();
        assert_eq!(
            failed,
            [Some(lib("bad.md")), Some(lib("half")), Some(lib("locked"))]
        );
        assert!(listing.listed_at.contains_key(&Some(lib("locked"))));
    }

    #[test]
    fn mem_store_lists_a_range_that_is_a_file_as_that_file() {
        let root = Path::new("/lib");
        let store = mem::MemStore::new(root);
        store.file("rules/a.md", "a");
        let listing = store
            .scan(root, Some(&lib("rules/a.md")), &counter())
            .unwrap();
        assert_eq!(file_names(&listing), ["rules/a.md"]);
        assert!(listing.errors.is_empty() && listing.absent_ranges.is_empty());
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "rules/a.md", "a");
        let real = FsStore
            .scan(dir.path(), Some(&lib("rules/a.md")), &counter())
            .unwrap();
        assert_eq!(
            file_names(&real),
            ["rules/a.md"],
            "the same as the real store"
        );
    }

    // ── trash ─────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn trash_refuses_a_path_that_climbs_out_of_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("lib");
        fs::create_dir_all(&root).unwrap();
        let outside = write(dir.path(), "outside/x.md", "keep");
        let climbing = root.join("..").join("outside").join("x.md");
        let bundle = TrashBundle("b1".to_owned());
        let error = FsStore.trash(&root, &climbing, &bundle).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{error:?}");
        assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
        assert!(!root.join(TRASH_DIR).exists(), "nothing created");

        let store = mem::MemStore::new(Path::new("/lib"));
        let error = store
            .trash(
                Path::new("/lib"),
                &Path::new("/lib").join("..").join("x.md"),
                &bundle,
            )
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_second_trash_of_the_same_path_into_the_same_bundle_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bundle = TrashBundle("b1".to_owned());
        let file = write(root, "rules/a.md", "first");
        let moved = FsStore.trash(root, &file, &bundle).unwrap();
        write(root, "rules/a.md", "second");
        let error = FsStore.trash(root, &file, &bundle).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error:?}");
        assert_eq!(
            fs::read_to_string(&moved).unwrap(),
            "first",
            "the first copy survives"
        );
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "second",
            "nothing moved"
        );

        let mem_root = Path::new("/lib");
        let store = mem::MemStore::new(mem_root);
        store.file("rules/a.md", "first");
        let path = lib("rules/a.md").to_path(mem_root);
        let moved = store.trash(mem_root, &path, &bundle).unwrap();
        store.file("rules/a.md", "second");
        let error = store.trash(mem_root, &path, &bundle).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(store.read(&moved, u64::MAX).unwrap(), b"first");
        assert_eq!(store.read(&path, u64::MAX).unwrap(), b"second");
    }

    #[test]
    fn the_trash_destination_is_free_only_on_not_found() {
        let to = Path::new("/lib/.trash/b1/a.md");
        assert!(destination_free(to, Err(io::ErrorKind::NotFound.into())).is_ok());
        let taken = destination_free(to, Ok(())).unwrap_err();
        assert_eq!(taken.kind(), io::ErrorKind::AlreadyExists);
        let failed = destination_free(to, Err(io::ErrorKind::PermissionDenied.into())).unwrap_err();
        assert_eq!(
            failed.kind(),
            io::ErrorKind::PermissionDenied,
            "a failed lookup is not free"
        );
    }

    // ── the walk's edges ──────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_full_scan_whose_root_becomes_a_file_before_the_walk_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("lib");
        write(&root, "a.md", "a");
        let draws = Cell::new(0u64);
        // The root is checked to be a folder before draw 1; inside draw 1 it is replaced by a file.
        let seq = || {
            draws.set(draws.get() + 1);
            if draws.get() == 1 {
                fs::remove_dir_all(&root).unwrap();
                fs::write(&root, "now a file").unwrap();
            }
            draws.get()
        };
        let error = FsStore.scan(&root, None, &seq).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotADirectory, "{error:?}");
    }

    #[test]
    fn a_file_deleted_before_its_step_is_neither_listed_nor_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        write(&root, "a.md", "a");
        let b = write(&root, "b.md", "b");
        let draws = Cell::new(0u64);
        // Draw 2 precedes the first entry step; whether the walk still yields b (buffered by the OS listing) or not,
        //   a deleted file is not listed and is no error. The tag-NotFound path runs only where the listing is
        //   buffered at open (Windows); elsewhere b is simply not yielded.
        let seq = || {
            draws.set(draws.get() + 1);
            if draws.get() == 2 {
                fs::remove_file(&b).unwrap();
            }
            draws.get()
        };
        let listing = FsStore.scan(&root, None, &seq).unwrap();
        assert_eq!(file_names(&listing), ["a.md"]);
        assert!(listing.errors.is_empty(), "{:?}", listing.errors);
    }

    #[test]
    fn a_folder_that_vanishes_before_its_open_is_not_in_the_listing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        write(&root, "D/x.md", "x");
        write(&root, "keep.md", "k");
        let folder = root.join("D");
        let draws = Cell::new(0u64);
        // The vanished-folder path runs only where the root's listing is buffered at open (Windows); elsewhere D is
        //   simply not yielded and the outcome is the same.
        let seq = || {
            draws.set(draws.get() + 1);
            if draws.get() == 2 {
                fs::remove_dir_all(&folder).unwrap();
            }
            draws.get()
        };
        let listing = FsStore.scan(&root, None, &seq).unwrap();
        assert!(listing.folders.is_empty(), "{:?}", listing.folders);
        assert!(!listing.listed_at.contains_key(&Some(lib("D"))));
        assert!(listing.errors.is_empty(), "{:?}", listing.errors);
        assert_eq!(file_names(&listing), ["keep.md"]);
    }

    #[test]
    fn walk_errors_are_placed_by_path_depth_and_kind() {
        let root = Path::new("/lib");
        let start = Path::new("/lib");
        let top: Option<LibPath> = None;
        let open = [None, Some(lib("a")), Some(lib("a/b"))];
        let disk = |rel: &str| lib(rel).to_path(root);
        let table: [(Option<PathBuf>, usize, bool, Placement); 8] = [
            // The start itself — missing or unreadable.
            (
                Some(start.to_path_buf()),
                0,
                true,
                Placement::Start { not_found: true },
            ),
            (
                Some(start.to_path_buf()),
                0,
                false,
                Placement::Start { not_found: false },
            ),
            // A folder's own open failure (walkdir: its path, its depth).
            (
                Some(disk("a/b")),
                2,
                false,
                Placement::Charged(Some(lib("a/b"))),
            ),
            // A file's metadata failure.
            (
                Some(disk("a/f.md")),
                2,
                false,
                Placement::Charged(Some(lib("a/f.md"))),
            ),
            // Vanished after its parent listed it.
            (
                Some(disk("a/b")),
                2,
                true,
                Placement::Vanished(Some(lib("a/b"))),
            ),
            // Part-way enumeration failure of `a/b` (no path, depth = its depth + 1) and of the root.
            (None, 3, false, Placement::Charged(Some(lib("a/b")))),
            (None, 1, false, Placement::Charged(None)),
            // A path that is no library path is charged to the whole range.
            (Some(root.join("bad.")), 1, false, Placement::Charged(None)),
        ];
        for (path, depth, not_found, want) in table {
            let got = place_error(path.as_deref(), depth, not_found, root, start, &top, &open);
            assert_eq!(got, want, "{path:?} depth {depth} not_found {not_found}");
        }
        // In a partial scan the whole range is `top`.
        let top = Some(lib("a"));
        let got = place_error(
            Some(&root.join("bad.")),
            1,
            false,
            root,
            &disk("a"),
            &top,
            &open[..2],
        );
        assert_eq!(got, Placement::Charged(Some(lib("a"))));
    }

    // ── the link-absence number is drawn before the link check ───────────────────────────────────────────

    #[test]
    fn the_link_absence_number_precedes_the_link_check() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        write(&outside, "b.md", "b");
        let root = dir.path().join("lib");
        write(&root, "team/a.md", "a");
        let team = root.join("team");
        let draws = Cell::new(0u64);
        // Inside draw 1 `team` becomes a link: the check after the draw sees it, so the absence carries draw 1 and
        //   nothing else is drawn. Drawn after the check, the walk would find the link instead, at draw 2.
        let seq = || {
            draws.set(draws.get() + 1);
            if draws.get() == 1 {
                fs::remove_dir_all(&team).unwrap();
                link_dir(&outside, &team).unwrap();
            }
            draws.get()
        };
        let listing = FsStore.scan(&root, Some(&lib("team")), &seq).unwrap();
        assert_eq!(listing.absent_ranges, [(lib("team"), 1)]);
        assert_eq!(draws.get(), 1);

        let mem_root = Path::new("/lib");
        let store = mem::MemStore::new(mem_root);
        store.file("team/a.md", "a");
        let draws = Cell::new(0u64);
        let seq = || {
            draws.set(draws.get() + 1);
            if draws.get() == 1 {
                store.link("team");
            }
            draws.get()
        };
        let listing = store.scan(mem_root, Some(&lib("team")), &seq).unwrap();
        assert_eq!(listing.absent_ranges, [(lib("team"), 1)]);
        assert_eq!(draws.get(), 1);
    }

    #[test]
    fn a_range_that_becomes_a_link_after_the_check_is_skipped_at_the_walk() {
        let mem_root = Path::new("/lib");
        let store = mem::MemStore::new(mem_root);
        store.file("team/a.md", "a");
        let draws = Cell::new(0u64);
        let seq = || {
            draws.set(draws.get() + 1);
            if draws.get() == 2 {
                store.link("team");
            }
            draws.get()
        };
        let listing = store.scan(mem_root, Some(&lib("team")), &seq).unwrap();
        assert_eq!(listing.absent_ranges, [(lib("team"), 2)]);
        assert_eq!(listing.skipped_links, [lib("team")]);
        assert!(listing.files.is_empty());
    }
}
