//! The storage seam: every disk access to the library goes through [`LibraryStore`] — [`FsStore`] in production,
//! `MemStore` (`#[cfg(test)]`) in tests. The store works on one mount's folder at a time (`root` below) and knows no
//! mount names: it takes and returns [`MountPath`]s, paths inside that folder (ADR-0400 — `mounts.rs` is the only
//! converter to the virtual paths of the catalog).
//!
//! ## Rules every implementation keeps
//!
//! - **Link policy (U1 (a))**: only a mount's folder itself may be a link. A walk never enters a link below it
//!   (folder or file) and never lists it; it reports it in [`ScanListing::skipped_links`]. A partial scan whose `under`
//!   is a link or lies below one walks nothing ([`LibraryStore::first_link`] — the one component test for a path below
//!   the folder).
//! - **Names**: [`admits`] is the name rule every way into the catalog shares — no name starting with `.` (so a mount's
//!   `.trash` is never walked) and no base temp name. Nothing here deletes or rewrites a leftover temp file (ADR-0335);
//!   the only temp files touched are base `write_atomic`'s own, whose base naming could truncate a leftover only in the
//!   rare case of the same pid and counter (base behaviour, not this module's — T-124), the bundle record's temps in
//!   `.trash`, which `trash.rs` makes, publishes and prunes through this store, and a move's own volume probe and
//!   staging folders (`ops.rs` — made create-new, removed or trashed only by the call that made them).
//! - **Per-folder listing sequence**: a scan draws `seq()` before every step of the walk; the number drawn
//!   before the step that yields folder D precedes every read of D's entries and is `listed_at[D]` (ADR-0324 decision
//!   5 · TRD S25 step-7 premise 47). A file of a scanned folder that the listing lacks was observed absent at that
//!   number.
//! - **A failed read is not an absence**: a folder whose listing fails (at open or part-way) and a file whose metadata
//!   fails are reported in [`ScanListing::errors`]; the catalog keeps what it knew of them.
//! - **No OS branch here** — link tests use std's `symlink_metadata().file_type().is_symlink()` (on Windows: every
//!   name-surrogate reparse point, junctions included) and identity comes from platform `file_identity`.
//! - **No rename replaces** (ADR-0409 decision 10): [`LibraryStore::rename`] and [`LibraryStore::trash`] go through
//!   platform `rename_no_replace` — a taken name is `AlreadyExists` with nothing moved, a volume that refuses the flag is
//!   `Unsupported`, and nothing falls back to an overwriting rename.
//! - **Absence is a `NotFound`** ([`LibraryStore::stat`]): the trash steps judge a path absent only from a no-follow
//!   stat that says `NotFound`; [`LibraryStore::check_names`] checks every name from the mount folder down without
//!   following, and the trash's own folder steps (`list`, `remove_dir`, `remove_file`) never act through a link.
//! - **Tree copies never replace and never follow** (ADR-0409 decision 12 · M13): [`LibraryStore::copy_tree`] lists the
//!   source without following ([`LibraryStore::list_tree`] — a file's tag before its bytes; every name from the mount
//!   folder down to each folder listed and each file tagged stat'ed again first) and refuses one holding a link before
//!   making anything (ADR-0365); then each file is read only through a handle checked to be the file listed — every
//!   name between the mount folder and it stat'ed again without following, the file opened without following its last
//!   name (a reparse point that is no link opened again, followed, only as the same entry — platform
//!   `open_read_no_follow`), the handle's kind, size, modified time and identity equal to the listed tag — and a file
//!   that is not stops the copy there; [`LibraryStore::content_hash`] reads the same way. A folder swapped for a link
//!   between that stat and the open is caught by the identity, where the volume gives one (otherwise only size and time
//!   — the check-then-call race ADR-0409 accepts). Every entry is made create-new, each file hashed while copied,
//!   modified times and permissions kept; a copy that fails part-way leaves what it made to its caller.
// ADR-0324
// ADR-0335
// ADR-0365
// ADR-0400
// ADR-0409
// TRD S25 step-7 §3-5

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use engram_dashboard_base::file::{
    copy_atomic, is_temp_name, read_capped_bytes, write_atomic, OsHooks, BUSY_PAUSE, BUSY_RETRIES,
};
use engram_dashboard_platform::fs::{
    file_identity, open_read_no_follow, opened_identity, rename_no_replace, retry_busy,
    set_folder_modified, sync_dir, FileIdentity, Retry,
};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::mounts::MountPath;
use crate::path::LibPath;

/// A mount's Engram trash folder — a fixed name in the mount's folder, never walked ([`admits`] refuses dot names).
// TRD S25 B8 · G31
pub const TRASH_DIR: &str = ".trash";

/// The code of an item whose no-replace rename the volume refused ([`LibraryStore::rename`] answered `Unsupported`) —
/// it fails, and nothing retries it with an overwriting rename.
// ADR-0409 decision 10
pub const NO_REPLACE_UNSUPPORTED: &str = "no-replace-unsupported";

/// The bytes read at a time by a tree copy and a content hash.
const COPY_PIECE: usize = 64 * 1024;

/// Every disk access to the library. Paths are disk paths; `root` is one mount's folder.
pub trait LibraryStore: Send + Sync {
    /// Lists the library under `root` (`under = None`) or the range `under` (a partial scan), drawing `seq()` before
    /// every step of the walk (module header). `Err` = a full scan whose root is missing, not a folder or cannot be
    /// listed at all — nothing of it may be applied (a missing root removes nothing). A partial scan never answers
    /// `Err` for its range: a link or a missing `under` (or one below a file) is a sequenced absence
    /// ([`ScanListing::absent_ranges`]), any other failure is an error entry. `under` must pass [`admits`] (else
    /// `InvalidInput`).
    fn scan(
        &self,
        root: &Path,
        under: Option<&MountPath>,
        seq: &dyn Fn() -> u64,
    ) -> io::Result<ScanListing>;
    /// The first folder or file of `rel`, from the root down, that is a link (U1). The root itself is not counted.
    /// `Ok(None)` = no link, including when a name of `rel` does not exist (nothing below it can be a link).
    // ADR-0365
    fn first_link(&self, root: &Path, rel: &MountPath) -> io::Result<Option<MountPath>>;
    /// The bytes of the file at `path`, at most `cap` bytes — a longer file is `InvalidData` (base `read_capped`'s
    /// rule), read only up to `cap + 1`.
    fn read(&self, path: &Path, cap: u64) -> io::Result<Vec<u8>>;
    fn tag(&self, path: &Path) -> io::Result<ContentTag>;
    /// Atomic write (base `write_atomic`). The parent folder must exist: a missing parent is `NotFound` and nothing
    /// is created (base would create it).
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    /// Creates the file at `path` holding `bytes` only if nothing is there — it never replaces. `AlreadyExists` =
    /// something is at `path`; a missing parent is `NotFound` and nothing is created. A write that fails after the file
    /// was made leaves it in place and answers an error naming it: it is never removed by name, since another program
    /// may have put its own file there since. A crash during the write can likewise leave a partial file at `path`.
    fn create_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    fn create_dir(&self, path: &Path) -> io::Result<()>;
    /// Renames `from` to `to` (a file or a folder, a link as a link) without ever replacing: platform
    /// `rename_no_replace`, retried while the OS says busy. `AlreadyExists` = something is at `to`, nothing moved;
    /// `Unsupported` = the volume refuses the no-replace flag ([`NO_REPLACE_UNSUPPORTED`] — never retried with an
    /// overwriting rename); `CrossesDevices` = `to` is on another volume, nothing moved. A case-only rename goes through
    /// it too.
    // ADR-0409 decision 10
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Atomic copy (base `copy_atomic`) — it replaces an existing `to` and drops permissions, so step 9b does not use it
    /// (ADR-0400 decision 25).
    fn copy(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Moves `path` (a file or folder under `root`, a link as a link) into `<root>/.trash/<bundle>/<its path from the
    /// root>` by one [`LibraryStore::rename`] — never replacing. Answers the new path. The bundle folder must already be
    /// there (the trash step makes it under the trash lock); every name from the root down to the destination's folder,
    /// and to the item's own folder, is checked without following ([`LibraryStore::check_names`]) and a missing folder
    /// below the bundle is made with `create_dir`, one name at a time; when the move then fails, the folders this call
    /// made are removed again, deepest first, empty-only. `InvalidInput` = `path` is not a library path below the root
    /// (the root itself, outside it, through `..`), or the bundle is not one plain name; `NotFound` = `.trash`, the
    /// bundle folder or a folder of the item's path is missing; `NotADirectory` = a link or a file among those names
    /// (nothing is made or moved through it); `AlreadyExists` = something is at the destination, nothing moved. A name swapped for a link between its check and
    /// the rename is not caught (local write access is trusted — ADR-0409).
    // ADR-0365
    // ADR-0409 decisions 9 and 10
    fn trash(&self, root: &Path, path: &Path, bundle: &TrashBundle) -> io::Result<PathBuf> {
        let to = trash_path(root, path, bundle)?;
        let not_a_folder = |at: &MountPath, way: &str| {
            io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("{} is not a folder on the way {way}", at.as_str()),
            )
        };
        // A rename does not stop at a link among the source's folders: they are checked from the root down too.
        let from = MountPath::from_disk(root, path)
            .expect("trash_path took it as a path inside the mount");
        if let Some(parent) = from.parent() {
            match self.check_names(root, &parent)? {
                NameWalk::Present(EntryKind::Folder) => {}
                NameWalk::Missing(at) => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("{} is missing", at.as_str()),
                    ))
                }
                NameWalk::Present(_) | NameWalk::Link(_) | NameWalk::NotAFolder(_) => {
                    return Err(not_a_folder(&parent, "to the item"))
                }
            }
        }
        let folder = MountPath::from_disk(root, to.parent().expect("a trash path has a parent"))
            .expect("a trash path is a path inside the mount");
        // `.trash` and the bundle are the first two names; only a folder below them is made here.
        let below_bundle = |at: &MountPath| at.as_str().split('/').count() > 2;
        let mut made: Vec<PathBuf> = Vec::new();
        let mut walk = || {
            // Each pass makes one missing folder (or finds it made meanwhile), so one pass per name ends the walk.
            for _ in 0..=folder.as_str().split('/').count() {
                match self.check_names(root, &folder)? {
                    NameWalk::Present(EntryKind::Folder) => return self.rename(path, &to),
                    NameWalk::Missing(at) if below_bundle(&at) => {
                        match self.create_dir(&at.to_path(root)) {
                            Ok(()) => made.push(at.to_path(root)),
                            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                            Err(e) => return Err(e),
                        }
                    }
                    NameWalk::Missing(at) => {
                        return Err(io::Error::new(
                            io::ErrorKind::NotFound,
                            format!("{} is missing", at.as_str()),
                        ))
                    }
                    NameWalk::Present(_) | NameWalk::Link(_) | NameWalk::NotAFolder(_) => {
                        return Err(not_a_folder(&folder, "into the trash"))
                    }
                }
            }
            Err(io::Error::other(format!(
                "{} kept reading as missing after it was made",
                folder.as_str()
            )))
        };
        match walk() {
            Ok(()) => Ok(to),
            Err(e) => {
                // The folders this call made go again, deepest first and empty-only, so a failed item leaves no shell
                //   inside a bundle whose other items go on.
                for dir in made.iter().rev() {
                    if self.remove_dir(dir).is_err() {
                        break;
                    }
                }
                Err(e)
            }
        }
    }
    /// Whether anything is at `path` (a link counts — it is not followed): only a stat that answers says so, and a stat
    /// error of any kind answers `false`. A fast path only — `create_new` and `create_dir` decide a collision.
    fn exists(&self, path: &Path) -> bool;
    /// What is at `path`, without following a link. `NotFound` = nothing is there; any other error says nothing about
    /// whether something is (ADR-0409: absence is a stat that says `NotFound`, never an error read as absence).
    // ADR-0409
    fn stat(&self, path: &Path) -> io::Result<EntryKind>;
    /// The entries of the folder `path` with their kinds, nothing followed (a link to a folder is a link), in no set
    /// order. `NotADirectory` = `path` is a file or a link.
    fn list(&self, path: &Path) -> io::Result<Vec<(OsString, EntryKind)>>;
    /// Removes the folder `path` only when it is empty, never through a link: a link or a file at `path` is
    /// `NotADirectory` and stays; a folder with anything in it stays (an error, `DirectoryNotEmpty` or the OS's own).
    fn remove_dir(&self, path: &Path) -> io::Result<()>;
    /// Removes the file at `path`; a folder or a link there is `InvalidInput` and stays.
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    /// The identity of the entry at `path` (platform `file_identity` — a link is followed): whether two spellings name
    /// one entry. A store without disk identities answers an error.
    fn identity(&self, path: &Path) -> io::Result<FileIdentity> {
        file_identity(path)
    }
    /// Every name of `rel` from `root` down, each [`LibraryStore::stat`]ed in turn without following: the first name
    /// above the last that is not a folder, or the last name's kind. The root itself is not checked (only a mount's
    /// folder may be a link — ADR-0365). A stat error other than `NotFound` is answered as is.
    // ADR-0365
    // ADR-0409 decision 9
    fn check_names(&self, root: &Path, rel: &MountPath) -> io::Result<NameWalk> {
        let names: Vec<&str> = rel.as_str().split('/').collect();
        let mut path = root.to_path_buf();
        for (at, name) in names.iter().enumerate() {
            path.push(name);
            let last = at + 1 == names.len();
            let here = || {
                MountPath::from_disk(root, &path)
                    .expect("a leading run of names of a path inside a mount is one")
            };
            match self.stat(&path) {
                Ok(kind) if last => return Ok(NameWalk::Present(kind)),
                Ok(EntryKind::Folder) => {}
                Ok(EntryKind::Link) => return Ok(NameWalk::Link(here())),
                Ok(EntryKind::File) => return Ok(NameWalk::NotAFolder(here())),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    return Ok(NameWalk::Missing(here()))
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!("a library path has at least one name")
    }
    /// Every entry of the tree at `top` (below the mount folder `root`) without following a link, in walk order —
    /// `top` itself first (`rel` empty), each folder before what it holds and a folder's entries by name — with its
    /// kind and a file's tag. A link is listed as a link and not entered; a file at `top` is the tree alone. Before
    /// `top` is stat'ed, before each folder is listed and before each file's tag is taken, every name between `root`
    /// and it is stat'ed again without following ([`way_holds`]): one that is no longer a real folder answers
    /// `NotADirectory`, so no listing reaches past a link put above the tree or inside it meanwhile. A stat, listing
    /// or tag failure is the answer.
    // ADR-0365
    fn list_tree(&self, root: &Path, top: &Path) -> io::Result<Vec<TreeEntry>> {
        let held = |path: &Path| match way_holds(self, root, path)? {
            true => Ok(()),
            false => Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "a name on the way to the tree is no longer a real folder",
            )),
        };
        let entry = |rel: PathBuf, kind: EntryKind| -> io::Result<TreeEntry> {
            let tag = match kind {
                EntryKind::File => {
                    let path = tree_path(top, &rel);
                    held(&path)?;
                    Some(self.tag(&path)?)
                }
                EntryKind::Folder | EntryKind::Link => None,
            };
            Ok(TreeEntry { rel, kind, tag })
        };
        held(top)?;
        let mut entries = vec![entry(PathBuf::new(), self.stat(top)?)?];
        let mut next = 0;
        while next < entries.len() {
            let (kind, folder) = (entries[next].kind, entries[next].rel.clone());
            next += 1;
            if kind != EntryKind::Folder {
                continue;
            }
            let path = tree_path(top, &folder);
            held(&path)?;
            let mut names = self.list(&path)?;
            names.sort_by(|a, b| a.0.cmp(&b.0));
            for (name, kind) in names {
                entries.push(entry(folder.join(name), kind)?);
            }
        }
        Ok(entries)
    }
    /// Copies the tree at `from` (below the mount folder `root`) to `to` (ADR-0409 decision 12): every entry — dot names and names no library takes
    /// included — made create-new, never replacing (`to` itself taken = `AlreadyExists` with nothing made). The tree
    /// is listed first ([`LibraryStore::list_tree`], so each file's tag is taken before its bytes are read); a link
    /// anywhere in it is [`CopyFailure::Links`] with nothing made (ADR-0365). Each file is read only while it is the
    /// file listed (the module header's check — a file that is not fails the copy there), its bytes hashed while they
    /// are copied, then synced, then its listed modified time and its permissions (Unix mode bits, the Windows
    /// read-only attribute) are set in that order; each folder gets its modified time and permissions after what it
    /// holds (ADR-0409 M13), read from a no-follow stat that must still say folder. A failure after `to` was made
    /// leaves what was copied ([`CopyFailure::Failed`] `made`) — the caller owns it. Default: the disk, through std
    /// and platform `open_read_no_follow` and `set_folder_modified`.
    // ADR-0365
    // ADR-0409 decision 12 and M13
    fn copy_tree(&self, root: &Path, from: &Path, to: &Path) -> Result<TreeCopy, CopyFailure> {
        copy_on_disk(self, root, from, to)
    }
    /// The SHA-256 of the bytes of the file `rel` of the tree at `top` (below the mount folder `root`), read in pieces with no cap — the hash
    /// [`LibraryStore::copy_tree`] takes while copying —, read only while it is the file `listed` saw (the module
    /// header's check). `Ok(None)` = it is not (a folder on the way or the name a link, a folder now, another tag):
    /// nothing was read. Default: the disk.
    // ADR-0365
    fn content_hash(
        &self,
        root: &Path,
        top: &Path,
        rel: &Path,
        listed: &ContentTag,
    ) -> io::Result<Option<ContentHash>> {
        let Some(mut file) = open_as_listed(self, root, &tree_path(top, rel), listed)? else {
            return Ok(None);
        };
        let mut hasher = Sha256::new();
        let mut piece = vec![0u8; COPY_PIECE];
        loop {
            match file.read(&mut piece) {
                Ok(0) => return Ok(Some(digest(hasher))),
                Ok(read) => hasher.update(&piece[..read]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }
}

/// What one entry is, seen without following a link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// A real folder (not a link to one).
    Folder,
    /// Anything else that is not a link: a file (or another non-folder entry).
    File,
    /// A link of any kind (on Windows every name-surrogate reparse point, junctions included), never followed.
    Link,
}

/// What [`LibraryStore::check_names`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameWalk {
    /// Every name is there and every name above the last is a folder; the last name's kind.
    Present(EntryKind),
    /// This name is absent; the names above it are folders.
    Missing(MountPath),
    /// This name, above the last, is a link.
    Link(MountPath),
    /// This name, above the last, is a file.
    NotAFolder(MountPath),
}

/// What one scan saw.
#[derive(Clone, Debug, Default)]
pub struct ScanListing {
    /// The mount folder the scan walked from — the base of every [`MountPath`] here.
    pub root: PathBuf,
    /// Admitted files, each with its tag and the number drawn before the step that yielded it.
    pub files: Vec<ScanEntry>,
    /// Admitted folders the walk listed (a range's own folder included).
    pub folders: Vec<MountPath>,
    /// Per listed folder (`None` = the root), the number drawn before the step that yielded it — every read of that
    /// folder's entries comes after it.
    pub listed_at: HashMap<Option<MountPath>, u64>,
    /// A whole range observed absent — a partial scan's `under` that is a link or lies below one, or that was not
    /// found — with the number drawn before that observation.
    pub absent_ranges: Vec<(MountPath, u64)>,
    /// Links below the root that were not entered (and a partial scan's `under` when it lies at or below one).
    pub skipped_links: Vec<MountPath>,
    /// Where the walk failed: a folder whose listing failed at open or part-way (`None` = the root), or a file whose
    /// metadata failed; with the error text.
    pub errors: Vec<(Option<MountPath>, String)>,
}

/// One admitted file of a scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanEntry {
    pub rel: MountPath,
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

/// One entry of a tree as [`LibraryStore::list_tree`] lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    /// Its path below the tree's top; empty = the top itself.
    pub rel: PathBuf,
    pub kind: EntryKind,
    /// A file's tag; `None` for a folder or a link.
    pub tag: Option<ContentTag>,
}

/// A SHA-256 of a file's bytes.
pub type ContentHash = [u8; 32];

/// One entry a tree copy made: the source entry as listed (a file's tag taken before its bytes) and a file's hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopiedEntry {
    pub entry: TreeEntry,
    /// The hash of the bytes copied; `None` for a folder.
    pub hash: Option<ContentHash>,
}

/// What [`LibraryStore::copy_tree`] copied.
#[derive(Clone, Debug)]
pub struct TreeCopy {
    /// Taken before the source was listed — the racy rule's reference for every tag below.
    pub read_at: SystemTime,
    /// In [`LibraryStore::list_tree`]'s order.
    pub entries: Vec<CopiedEntry>,
}

/// Why a tree copy did not finish.
#[derive(Debug)]
pub enum CopyFailure {
    /// Links in the source, by their paths below its top (empty = the top itself) — nothing was made (ADR-0365).
    Links(Vec<PathBuf>),
    /// `made` = the copy's top had been made: it holds what was copied before the failure. `at` = the entry the copy
    /// stopped at, below the source's top (empty = the top itself): it and the entries after it in listing order were
    /// not copied — or, for a folder, everything in it was and its own time and permissions were not kept.
    Failed {
        error: io::Error,
        made: bool,
        at: PathBuf,
    },
}

/// The disk path of the tree entry `rel` below `top` (`top` itself for an empty `rel` — a path joined with an empty
/// name gains a separator, which names no file).
pub(crate) fn tree_path(top: &Path, rel: &Path) -> PathBuf {
    match rel.as_os_str().is_empty() {
        true => top.to_path_buf(),
        false => top.join(rel),
    }
}

/// The bundle folder name under `.trash` — one plain name to the store; its form is `trash::BundleId`'s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashBundle(pub String);

/// The name rule every way into the catalog shares (scan, safety net, late merge), over every name of the `/` path
/// `names` inside one mount: no name starting with `.` and no base temp name (`base::file::is_temp_name` — the shape
/// is base's, not copied here). A virtual path is judged through `mounts::admits_inside`, which leaves the mount's own
/// name out.
pub fn admits(names: &str) -> bool {
    names
        .split('/')
        .all(|name| !name.starts_with('.') && !is_temp_name(name))
}

/// The production store: walkdir + std + base's atomic write and copy with platform's lock retry and folder sync.
#[derive(Clone, Copy, Debug, Default)]
pub struct FsStore;

// ADR-0291
pub(crate) const OS_HOOKS: OsHooks = OsHooks {
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
        under: Option<&MountPath>,
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
                if !range_probe(self.first_link(root, under), under, at, &mut listing) {
                    return Ok(listing);
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
        let mut open: Vec<Option<MountPath>> = Vec::new();
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
                        MountPath::from_disk(root, entry.path()).ok()
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

    fn first_link(&self, root: &Path, rel: &MountPath) -> io::Result<Option<MountPath>> {
        let mut path = root.to_path_buf();
        for name in rel.as_str().split('/') {
            path.push(name);
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    let prefix = MountPath::from_disk(root, &path)
                        .expect("a leading run of names of a path inside a mount is one");
                    return Ok(Some(prefix));
                }
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
            }
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

    fn create_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        create_new_with(path, |file| {
            file.write_all(bytes).and_then(|()| file.sync_all())
        })
    }

    fn create_dir(&self, path: &Path) -> io::Result<()> {
        fs::create_dir(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        (OS_HOOKS.retry)(&mut || rename_no_replace(from, to))
    }

    fn copy(&self, from: &Path, to: &Path) -> io::Result<()> {
        copy_atomic(from, to, OS_HOOKS)
    }

    fn exists(&self, path: &Path) -> bool {
        present(&fs::symlink_metadata(path))
    }

    fn stat(&self, path: &Path) -> io::Result<EntryKind> {
        Ok(entry_kind(fs::symlink_metadata(path)?.file_type()))
    }

    fn list(&self, path: &Path) -> io::Result<Vec<(OsString, EntryKind)>> {
        if self.stat(path)? != EntryKind::Folder {
            return Err(not_a_folder(path));
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            // `DirEntry::file_type` does not follow a link.
            entries.push((entry.file_name(), entry_kind(entry.file_type()?)));
        }
        Ok(entries)
    }

    fn remove_dir(&self, path: &Path) -> io::Result<()> {
        // A junction is a folder to `fs::remove_dir` on Windows: only a real folder is handed to it.
        if self.stat(path)? != EntryKind::Folder {
            return Err(not_a_folder(path));
        }
        (OS_HOOKS.retry)(&mut || fs::remove_dir(path))
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        if self.stat(path)? != EntryKind::File {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a file", path.display()),
            ));
        }
        (OS_HOOKS.retry)(&mut || fs::remove_file(path))
    }
}

fn entry_kind(file_type: fs::FileType) -> EntryKind {
    if file_type.is_symlink() {
        EntryKind::Link
    } else if file_type.is_dir() {
        EntryKind::Folder
    } else {
        EntryKind::File
    }
}

fn not_a_folder(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotADirectory,
        format!("{} is not a folder", path.display()),
    )
}

/// [`FsStore`]'s `create_new`, with `write` filling the file just made.
fn create_new_with(
    path: &Path,
    write: impl FnOnce(&mut fs::File) -> io::Result<()>,
) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    if let Err(e) = write(&mut file) {
        // Not removed by name: another program may have put its own file there since.
        tracing::warn!(path = %path.display(), "create_new: write failed; partial file left: {e}");
        return Err(io::Error::other(format!(
            "{e}; a partial file may be left at {}",
            path.display()
        )));
    }
    drop(file);
    if let Some(parent) = path.parent() {
        if let Err(e) = (OS_HOOKS.sync_dir)(parent) {
            tracing::debug!(path = %parent.display(), "create_new: folder sync failed: {e}");
        }
    }
    Ok(())
}

/// Whether a stat says something is there — only one that answers does.
fn present<T>(stat: &io::Result<T>) -> bool {
    stat.is_ok()
}

fn refuse_unadmitted(under: &MountPath) -> io::Result<()> {
    if admits(under.as_str()) {
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
    match MountPath::from_disk(root, path) {
        Ok(rel) => admits(rel.as_str()),
        Err(e) => {
            tracing::warn!(
                ?path,
                "library scan: left out a name that cannot be a library path: {e}"
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

/// What a partial scan makes of the link probe of its range `under` (`at` = the number drawn before the probe):
/// `true` = walk the range. A link at or above it is a skipped link and a sequenced absence; a range below a file
/// (`NotADirectory`) is a sequenced absence too — absent, as `link_verdict` judges it, so a launch's overlay read
/// and the problems agree (Windows answers `NotFound` there, which `first_link` already takes as no link); any other
/// failure is an error entry for the range.
// ADR-0387
fn range_probe(
    probe: io::Result<Option<MountPath>>,
    under: &MountPath,
    at: u64,
    listing: &mut ScanListing,
) -> bool {
    match probe {
        Ok(None) => return true,
        Ok(Some(_)) => {
            listing.skipped_links.push(under.clone());
            listing.absent_ranges.push((under.clone(), at));
        }
        Err(e) if e.kind() == io::ErrorKind::NotADirectory => {
            listing.absent_ranges.push((under.clone(), at));
        }
        Err(e) => listing.errors.push((Some(under.clone()), e.to_string())),
    }
    false
}

/// Where a walk error lands.
#[derive(Debug, PartialEq)]
enum Placement {
    /// The start of the walk itself could not be read or opened.
    Start { not_found: bool },
    /// An entry (folder or file) that vanished after its parent listed it — as if never listed. The path, when it is a
    /// library path (`None` = it is not one, so nothing of the listing names it).
    Vanished(Option<MountPath>),
    /// A failure charged to this place (`None` = the root): its items are kept.
    Charged(Option<MountPath>),
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
    top: &Option<MountPath>,
    open: &[Option<MountPath>],
) -> Placement {
    match path {
        Some(path) if path == start => Placement::Start { not_found },
        Some(path) if not_found => Placement::Vanished(MountPath::from_disk(root, path).ok()),
        Some(path) => {
            Placement::Charged(MountPath::from_disk(root, path).map_or_else(|_| top.clone(), Some))
        }
        None => Placement::Charged(
            open.get(depth.wrapping_sub(1))
                .cloned()
                .unwrap_or_else(|| top.clone()),
        ),
    }
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
    // Through `MountPath`: names only, so `..` cannot leave the folder and the folder itself is refused.
    let rel = MountPath::from_disk(root, path).map_err(|e| {
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

/// [`LibraryStore::copy_tree`] on the disk (its doc): `store` lists the source, std and platform make the copy.
// ADR-0409 decision 12 and M13
fn copy_on_disk<S: LibraryStore + ?Sized>(
    store: &S,
    root: &Path,
    from: &Path,
    to: &Path,
) -> Result<TreeCopy, CopyFailure> {
    let read_at = SystemTime::now();
    let failed = |error, made, at: &Path| CopyFailure::Failed {
        error,
        made,
        at: at.to_path_buf(),
    };
    let listing = store
        .list_tree(root, from)
        .map_err(|error| failed(error, false, Path::new("")))?;
    refuse_links(&listing)?;
    let mut made = false;
    let mut entries = Vec::with_capacity(listing.len());
    let mut folders = Vec::new();
    for entry in listing {
        let copy = tree_path(to, &entry.rel);
        let hash = match &entry.tag {
            None => {
                fs::create_dir(&copy).map_err(|error| failed(error, made, &entry.rel))?;
                made = true;
                folders.push(entry.rel.clone());
                None
            }
            Some(tag) => {
                let source = tree_path(from, &entry.rel);
                let copied = copy_file(store, root, &source, &copy, tag, &mut made);
                Some(copied.map_err(|error| failed(error, made, &entry.rel))?)
            }
        };
        entries.push(CopiedEntry { entry, hash });
    }
    // ADR-0409 M13: a folder's own time and permissions after what it holds — making an entry inside changes them.
    for rel in folders.iter().rev() {
        let copy = tree_path(to, rel);
        let source = tree_path(from, rel);
        let kept = way_holds(store, root, &source).and_then(|held| {
            // ADR-0365: a name on the way that is no longer a real folder stops here — nothing is read through it.
            if !held {
                return Err(not_as_listed());
            }
            let meta = fs::symlink_metadata(&source)?;
            if !meta.is_dir() {
                return Err(not_as_listed());
            }
            set_folder_modified(&copy, meta.modified()?)?;
            fs::set_permissions(&copy, meta.permissions())
        });
        kept.map_err(|error| failed(error, made, rel))?;
    }
    Ok(TreeCopy { read_at, entries })
}

/// The file at `from` (below the mount folder `root`), opened as `listed` saw it ([`open_as_listed`] — one that is
/// not fails here, nothing read), made create-new at `to` (`made` set once it is), its bytes copied and hashed,
/// synced, then the listed modified time and the opened file's permissions set — read-only last (ADR-0409 M13).
fn copy_file<S: LibraryStore + ?Sized>(
    store: &S,
    root: &Path,
    from: &Path,
    to: &Path,
    listed: &ContentTag,
    made: &mut bool,
) -> io::Result<ContentHash> {
    let Some(mut source) = open_as_listed(store, root, from, listed)? else {
        return Err(not_as_listed());
    };
    let permissions = source.metadata()?.permissions();
    let mut copy = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)?;
    *made = true;
    let mut hasher = Sha256::new();
    let mut piece = vec![0u8; COPY_PIECE];
    loop {
        let read = match source.read(&mut piece) {
            Ok(0) => break,
            Ok(read) => read,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        hasher.update(&piece[..read]);
        copy.write_all(&piece[..read])?;
    }
    copy.sync_all()?;
    copy.set_times(fs::FileTimes::new().set_modified(listed.modified))?;
    copy.set_permissions(permissions)?;
    Ok(digest(hasher))
}

/// The file at `path` (below the mount folder `root`) opened for reading only while it is the file `listed` saw
/// (ADR-0365 · ADR-0409 decision 12): every name between `root` and it stat'ed again without following
/// ([`way_holds`]), the file opened without following its last name (platform `open_read_no_follow`), and the
/// handle's kind, size, modified time and identity compared with `listed` — all before a byte is read. `Ok(None)` = it
/// is not. Retried while the OS says busy (an antivirus or indexer holding it), each try checking again.
// ADR-0365
fn open_as_listed<S: LibraryStore + ?Sized>(
    store: &S,
    root: &Path,
    path: &Path,
    listed: &ContentTag,
) -> io::Result<Option<fs::File>> {
    let mut opened = None;
    (OS_HOOKS.retry)(&mut || {
        opened = None;
        if !way_holds(store, root, path)? {
            return Ok(());
        }
        let file = match open_read_no_follow(path) {
            Ok(file) => file,
            // Outside Windows a link at the name is refused (`O_NOFOLLOW`) rather than opened as itself.
            Err(e) => {
                return match store.stat(path) {
                    Ok(EntryKind::Link) => Ok(()),
                    _ => Err(e),
                }
            }
        };
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Ok(());
        }
        let tag = content_tag(meta.modified()?, meta.len(), opened_identity(&file))?;
        if tag == *listed {
            opened = Some(file);
        }
        Ok(())
    })?;
    Ok(opened)
}

/// Whether every name between the mount folder `root` and `path` — neither of them included — is still a real
/// folder, each stat'ed without following, from `root` down (one gone = no). `root` itself is not checked: only a
/// mount's folder may be a link (ADR-0365). `InvalidInput` = `path` does not lie below `root`.
// ADR-0365
fn way_holds<S: LibraryStore + ?Sized>(store: &S, root: &Path, path: &Path) -> io::Result<bool> {
    let below = path.strip_prefix(root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "a tree path that is not below its mount folder",
        )
    })?;
    let mut names = below.components().peekable();
    let mut folder = root.to_path_buf();
    while let Some(name) = names.next() {
        if names.peek().is_none() {
            break;
        }
        folder.push(name);
        match store.stat(&folder) {
            Ok(EntryKind::Folder) => {}
            Ok(EntryKind::File | EntryKind::Link) => return Ok(false),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

/// A tree copy's source entry that is no longer what the copy listed.
fn not_as_listed() -> io::Error {
    io::Error::other("no longer the entry the copy listed")
}

/// A tree listing's links as [`CopyFailure::Links`]; `Ok` = none (ADR-0365).
fn refuse_links(listing: &[TreeEntry]) -> Result<(), CopyFailure> {
    let links: Vec<PathBuf> = listing
        .iter()
        .filter(|entry| entry.kind == EntryKind::Link)
        .map(|entry| entry.rel.clone())
        .collect();
    match links.is_empty() {
        true => Ok(()),
        false => Err(CopyFailure::Links(links)),
    }
}

fn digest(hasher: Sha256) -> ContentHash {
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&hasher.finalize());
    hash
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

    use std::ffi::OsString;

    use super::{
        admits, digest, refuse_links, tree_path, ContentHash, ContentTag, CopiedEntry, CopyFailure,
        EntryKind, LibraryStore, ScanEntry, ScanListing, TreeCopy,
    };
    use crate::mounts::MountPath;
    use sha2::{Digest, Sha256};

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
        stat_failures: HashSet<PathBuf>,
        /// Renames whose destination is one of these fail.
        rename_failures: HashSet<PathBuf>,
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
            MountPath::parse(rel).unwrap().to_path(&self.root)
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

        /// An empty folder at `rel` (folders above it are made).
        pub(crate) fn dir(&self, rel: &str) {
            let path = self.at(rel);
            let mut state = self.state.lock().unwrap();
            make_parents(&mut state, &self.root, &path);
            state.nodes.insert(path, Node::Dir);
        }

        /// A stat of `rel` fails (other than `NotFound`) from now on.
        pub(crate) fn fail_stat(&self, rel: &str) {
            let path = self.at(rel);
            self.state.lock().unwrap().stat_failures.insert(path);
        }

        /// A rename onto `rel` fails from now on.
        pub(crate) fn fail_rename_to(&self, rel: &str) {
            let path = self.at(rel);
            self.state.lock().unwrap().rename_failures.insert(path);
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

        /// Ends a [`MemStore::fail_read`] of `rel`.
        pub(crate) fn heal_read(&self, rel: &str) {
            let path = self.at(rel);
            self.state.lock().unwrap().read_failures.remove(&path);
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

        fn rel(&self, path: &Path) -> MountPath {
            MountPath::from_disk(&self.root, path).unwrap()
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
        rel: Option<MountPath>,
        left: Vec<PathBuf>,
        failure: Option<io::Error>,
    }

    impl LibraryStore for MemStore {
        fn identity(&self, _: &Path) -> io::Result<super::FileIdentity> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "the memory store has no identities",
            ))
        }

        fn scan(
            &self,
            root: &Path,
            under: Option<&MountPath>,
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
                if !admits(rel.as_str()) {
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

        fn first_link(&self, root: &Path, rel: &MountPath) -> io::Result<Option<MountPath>> {
            let state = self.state.lock().unwrap();
            let mut path = root.to_path_buf();
            for name in rel.as_str().split('/') {
                path.push(name);
                match state.nodes.get(&path) {
                    Some(Node::Link) => {
                        return Ok(Some(MountPath::from_disk(root, &path).unwrap()))
                    }
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

        fn create_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            let mut state = self.state.lock().unwrap();
            if state.nodes.contains_key(path) {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
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
            if state.rename_failures.contains(to) {
                return Err(io::Error::other("injected rename failure"));
            }
            if !state.nodes.contains_key(from) {
                return Err(not_found());
            }
            // As FsStore's no-replace rename.
            if state.nodes.contains_key(to) {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            if to.starts_with(from) {
                return Err(io::Error::from(io::ErrorKind::InvalidInput));
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

        fn exists(&self, path: &Path) -> bool {
            self.state.lock().unwrap().nodes.contains_key(path)
        }

        fn stat(&self, path: &Path) -> io::Result<EntryKind> {
            let state = self.state.lock().unwrap();
            if state.stat_failures.contains(path) {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            match state.nodes.get(path) {
                Some(Node::Dir) => Ok(EntryKind::Folder),
                Some(Node::File { .. }) => Ok(EntryKind::File),
                Some(Node::Link) => Ok(EntryKind::Link),
                None => Err(not_found()),
            }
        }

        fn list(&self, path: &Path) -> io::Result<Vec<(OsString, EntryKind)>> {
            let state = self.state.lock().unwrap();
            match state.nodes.get(path) {
                Some(Node::Dir) => {}
                Some(_) => return Err(io::Error::from(io::ErrorKind::NotADirectory)),
                None => return Err(not_found()),
            }
            match state.list_failures.get(path) {
                Some(ListFailure::Open) => {
                    return Err(io::Error::from(io::ErrorKind::PermissionDenied))
                }
                Some(ListFailure::PartWay { .. }) => {
                    return Err(io::Error::other("injected enumeration failure"))
                }
                None => {}
            }
            Ok(MemStore::children(&state, path)
                .into_iter()
                .map(|child| {
                    let kind = match state.nodes[&child] {
                        Node::Dir => EntryKind::Folder,
                        Node::File { .. } => EntryKind::File,
                        Node::Link => EntryKind::Link,
                    };
                    (child.file_name().unwrap().to_owned(), kind)
                })
                .collect())
        }

        fn remove_dir(&self, path: &Path) -> io::Result<()> {
            let mut state = self.state.lock().unwrap();
            match state.nodes.get(path) {
                Some(Node::Dir) => {}
                Some(_) => return Err(io::Error::from(io::ErrorKind::NotADirectory)),
                None => return Err(not_found()),
            }
            if !MemStore::children(&state, path).is_empty() {
                return Err(io::Error::from(io::ErrorKind::DirectoryNotEmpty));
            }
            state.nodes.remove(path);
            Ok(())
        }

        fn remove_file(&self, path: &Path) -> io::Result<()> {
            let mut state = self.state.lock().unwrap();
            match state.nodes.get(path) {
                Some(Node::File { .. }) => {
                    state.nodes.remove(path);
                    Ok(())
                }
                Some(_) => Err(io::Error::from(io::ErrorKind::InvalidInput)),
                None => Err(not_found()),
            }
        }

        /// As the disk's (the trait doc): a file keeps its bytes and modified time; a failing read of a file fails
        /// the copy there.
        fn copy_tree(&self, root: &Path, from: &Path, to: &Path) -> Result<TreeCopy, CopyFailure> {
            let read_at = SystemTime::now();
            let listing = self
                .list_tree(root, from)
                .map_err(|error| CopyFailure::Failed {
                    error,
                    made: false,
                    at: PathBuf::new(),
                })?;
            refuse_links(&listing)?;
            let mut state = self.state.lock().unwrap();
            let mut made = false;
            let mut entries = Vec::with_capacity(listing.len());
            for entry in listing {
                let (source, copy) = (tree_path(from, &entry.rel), tree_path(to, &entry.rel));
                let refused = if state.nodes.contains_key(&copy) {
                    Some(io::Error::from(io::ErrorKind::AlreadyExists))
                } else if !matches!(
                    copy.parent().and_then(|p| state.nodes.get(p)),
                    Some(Node::Dir)
                ) {
                    Some(not_found())
                } else if state.read_failures.contains(&source) {
                    Some(io::Error::other("injected read failure"))
                } else {
                    None
                };
                let node = match (refused, state.nodes.get(&source).cloned()) {
                    (None, Some(node)) => node,
                    (refused, _) => {
                        return Err(CopyFailure::Failed {
                            error: refused.unwrap_or_else(not_found),
                            made,
                            at: entry.rel,
                        })
                    }
                };
                let hash = match &node {
                    Node::File { bytes, .. } => Some(digest(Sha256::new_with_prefix(bytes))),
                    Node::Dir | Node::Link => None,
                };
                state.nodes.insert(copy, node);
                made = true;
                entries.push(CopiedEntry { entry, hash });
            }
            Ok(TreeCopy { read_at, entries })
        }

        /// As the disk's (the trait doc): a file whose tag is not `listed`, a link or a folder at its name, or a link
        /// or file among the folders between `root` and it is no longer the file listed.
        fn content_hash(
            &self,
            root: &Path,
            top: &Path,
            rel: &Path,
            listed: &ContentTag,
        ) -> io::Result<Option<ContentHash>> {
            let path = tree_path(top, rel);
            let state = self.state.lock().unwrap();
            if state.read_failures.contains(&path) {
                return Err(io::Error::other("injected read failure"));
            }
            let folders_hold = path
                .ancestors()
                .skip(1)
                .take_while(|folder| folder.starts_with(root) && *folder != root)
                .all(|folder| matches!(state.nodes.get(folder), Some(Node::Dir)));
            match state.nodes.get(&path) {
                Some(Node::File { bytes, modified })
                    if folders_hold
                        && *modified == listed.modified
                        && bytes.len() as u64 == listed.size =>
                {
                    Ok(Some(digest(Sha256::new_with_prefix(bytes))))
                }
                Some(_) => Ok(None),
                None => Err(not_found()),
            }
        }
    }

    fn open_folder(state: &State, path: &Path, rel: Option<MountPath>) -> Open {
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

    use std::time::Duration;

    use engram_dashboard_platform::fs::link_dir;

    use super::*;

    fn lib(text: &str) -> MountPath {
        MountPath::parse(text).unwrap()
    }

    fn modified(path: &Path) -> SystemTime {
        fs::symlink_metadata(path).unwrap().modified().unwrap()
    }

    fn set_read_only(path: &Path, read_only: bool) {
        let mut permissions = fs::metadata(path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(read_only);
        fs::set_permissions(path, permissions).unwrap();
    }

    fn rels(copy: &TreeCopy) -> Vec<String> {
        copy.entries
            .iter()
            .map(|copied| copied.entry.rel.to_string_lossy().replace('\\', "/"))
            .collect()
    }

    // ADR-0409 decision 12 · M13: every entry — dot names and names no library takes included — is made, a folder
    //   before what it holds; each file's bytes are hashed while copied; a file's modified time and read-only attribute
    //   and a folder's modified time are kept.
    #[test]
    fn a_tree_copy_makes_every_entry_keeping_modified_times_and_read_only_and_hashes_the_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        let a = write(&from, "a.md", "alpha");
        write(&from, "sub/.hidden", "h");
        let b = write(&from, "sub/deep/b.txt", "bee");
        let then = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let later = then + Duration::from_secs(3600);
        fs::File::options()
            .write(true)
            .open(&a)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(then))
            .unwrap();
        set_read_only(&b, true);
        set_folder_modified(&from.join("sub").join("deep"), later).unwrap();
        let to = dir.path().join("to");

        let copy = FsStore.copy_tree(dir.path(), &from, &to).unwrap();

        assert_eq!(
            rels(&copy),
            [
                "",
                "a.md",
                "sub",
                "sub/.hidden",
                "sub/deep",
                "sub/deep/b.txt"
            ]
        );
        let copied_b = to.join("sub").join("deep").join("b.txt");
        assert_eq!(fs::read_to_string(&copied_b).unwrap(), "bee");
        assert_eq!(
            fs::read_to_string(to.join("sub").join(".hidden")).unwrap(),
            "h"
        );
        assert_eq!(modified(&to.join("a.md")), then);
        assert!(fs::metadata(&copied_b).unwrap().permissions().readonly());
        assert_eq!(modified(&to.join("sub").join("deep")), later);
        let hash = copy.entries[1].hash.expect("a file's hash");
        assert_eq!(hash, digest(Sha256::new_with_prefix(b"alpha")));
        let tag = FsStore.tag(&to.join("a.md")).unwrap();
        let rehash = FsStore
            .content_hash(dir.path(), &to, Path::new("a.md"), &tag)
            .unwrap();
        assert_eq!(rehash, Some(hash));
        assert_eq!(copy.entries[1].entry.tag, Some(FsStore.tag(&a).unwrap()));
        assert_eq!(copy.entries[2].hash, None, "a folder has no hash");
        set_read_only(&b, false);
        set_read_only(&copied_b, false);
    }

    // ADR-0409 decision 12: a copy is create-new — a taken destination, folder or file, is `AlreadyExists` with
    //   nothing made and the entry there untouched.
    #[test]
    fn a_tree_copy_never_replaces_what_is_at_its_destination() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        let a = write(&from, "a.md", "new");
        let taken = write(dir.path(), "taken/a.md", "old");

        for (source, onto) in [(&from, taken.parent().unwrap()), (&a, taken.as_path())] {
            match FsStore.copy_tree(dir.path(), source, onto) {
                Err(CopyFailure::Failed {
                    error, made: false, ..
                }) if error.kind() == io::ErrorKind::AlreadyExists => {}
                other => panic!("not refused as taken: {other:?}"),
            }
        }
        assert_eq!(fs::read_to_string(&taken).unwrap(), "old");
        assert_eq!(fs::read_dir(taken.parent().unwrap()).unwrap().count(), 1);
    }

    // ADR-0365: a link anywhere in the source refuses the copy, naming it, before anything is made.
    #[test]
    fn a_tree_copy_refuses_a_source_holding_a_link_and_makes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        write(&outside, "keep.md", "k");
        let from = dir.path().join("from");
        write(&from, "sub/a.md", "a");
        link_dir(&outside, &from.join("sub").join("linked")).unwrap();
        let to = dir.path().join("to");

        match FsStore.copy_tree(dir.path(), &from, &to) {
            Err(CopyFailure::Links(links)) => {
                assert_eq!(links, [Path::new("sub").join("linked")])
            }
            other => panic!("not refused for its link: {other:?}"),
        }
        assert!(fs::symlink_metadata(&to).is_err(), "nothing made");
    }

    // The memory store's copy keeps bytes and modified times, and one that fails part-way says it made its top.
    #[test]
    fn mem_store_copies_a_tree_and_a_failing_read_stops_it_part_way() {
        let store = mem::MemStore::new(Path::new("/m"));
        store.file_version("d/a.md", "a", 5);
        store.file("d/e/b.md", "b");
        let copy = store
            .copy_tree(Path::new("/m"), Path::new("/m/d"), Path::new("/m/c"))
            .unwrap();
        assert_eq!(rels(&copy), ["", "a.md", "e", "e/b.md"]);
        assert_eq!(
            store.tag(Path::new("/m/c/a.md")).unwrap(),
            store.tag(Path::new("/m/d/a.md")).unwrap()
        );
        store.fail_read("d/e/b.md");
        match store.copy_tree(Path::new("/m"), Path::new("/m/d"), Path::new("/m/f")) {
            Err(CopyFailure::Failed { made: true, at, .. }) => {
                assert_eq!(at, Path::new("e").join("b.md"), "where it stopped")
            }
            other => panic!("not a part-way failure: {other:?}"),
        }
        assert!(store.exists(Path::new("/m/f/a.md")));
    }

    /// [`FsStore`] whose stat or listing of `at` runs `hook` once first, after `passes` such calls went by — a change
    /// landing in the middle of a tree listing or copy (the copy stats each file's folders again just before opening
    /// the file).
    struct Swapping {
        at: PathBuf,
        passes: std::sync::Mutex<usize>,
        hook: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }

    impl Swapping {
        fn touch(&self, path: &Path) {
            if path != self.at {
                return;
            }
            let mut passes = self.passes.lock().unwrap();
            if *passes > 0 {
                *passes -= 1;
                return;
            }
            drop(passes);
            if let Some(hook) = self.hook.lock().unwrap().take() {
                hook();
            }
        }
    }

    impl LibraryStore for Swapping {
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
            FsStore.rename(from, to)
        }
        fn copy(&self, from: &Path, to: &Path) -> io::Result<()> {
            FsStore.copy(from, to)
        }
        fn exists(&self, path: &Path) -> bool {
            FsStore.exists(path)
        }
        fn stat(&self, path: &Path) -> io::Result<EntryKind> {
            self.touch(path);
            FsStore.stat(path)
        }
        fn list(&self, path: &Path) -> io::Result<Vec<(OsString, EntryKind)>> {
            self.touch(path);
            FsStore.list(path)
        }
        fn remove_dir(&self, path: &Path) -> io::Result<()> {
            FsStore.remove_dir(path)
        }
        fn remove_file(&self, path: &Path) -> io::Result<()> {
            FsStore.remove_file(path)
        }
    }

    fn swapping(at: PathBuf, hook: impl FnOnce() + Send + 'static) -> Swapping {
        swapping_after(at, 0, hook)
    }

    fn swapping_after(
        at: PathBuf,
        passes: usize,
        hook: impl FnOnce() + Send + 'static,
    ) -> Swapping {
        Swapping {
            at,
            passes: std::sync::Mutex::new(passes),
            hook: std::sync::Mutex::new(Some(Box::new(hook))),
        }
    }

    /// The part-way failure of a tree copy: where it stopped.
    fn stopped_at(copied: Result<TreeCopy, CopyFailure>) -> PathBuf {
        match copied {
            Err(CopyFailure::Failed { made: true, at, .. }) => at,
            other => panic!("not a part-way failure: {other:?}"),
        }
    }

    // ADR-0365: a folder of the source swapped for a link after the listing — here, after the first file was copied —
    //   is not read through: the copy stops there and nothing of the link's target is copied.
    #[test]
    fn a_tree_copy_reads_nothing_through_a_folder_swapped_for_a_link_during_it() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        write(&from, "a.md", "a");
        write(&from, "sub/b.md", "b");
        write(dir.path(), "outside/b.md", "secret");
        let (sub, outside) = (from.join("sub"), dir.path().join("outside"));
        let link = sub.clone();
        // The listing lists `sub` and stats it on the way to `sub/b.md`'s tag; the third touch is the copy's.
        let store = swapping_after(sub.clone(), 2, move || {
            fs::rename(&link, link.with_file_name("moved")).unwrap();
            link_dir(&outside, &link).unwrap();
        });
        let to = dir.path().join("to");

        let at = stopped_at(store.copy_tree(dir.path(), &from, &to));

        assert_eq!(at, Path::new("sub").join("b.md"));
        assert_eq!(fs::read_to_string(to.join("a.md")).unwrap(), "a");
        assert!(
            !to.join("sub").join("b.md").exists(),
            "nothing read through the link"
        );
        engram_dashboard_platform::fs::unlink_dir(&sub).unwrap();
    }

    // ADR-0409 decision 12: a file replaced after the listing by another one of the same size and modified time is
    //   told apart on the opened handle (its identity) — nothing of it is copied; a link at a file's name is not read.
    #[test]
    fn a_tree_copy_stops_at_a_file_that_is_no_longer_the_one_listed() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        write(&from, "a.md", "a");
        let b = write(&from, "sub/b.md", "bbbb");
        let other = write(dir.path(), "other.md", "cccc");
        let then = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        for file in [&b, &other] {
            let opened = fs::File::options().write(true).open(file).unwrap();
            opened
                .set_times(fs::FileTimes::new().set_modified(then))
                .unwrap();
        }
        let replaced = b.clone();
        // The listing lists `sub` and stats it on the way to `sub/b.md`'s tag; the third touch is the copy's.
        let store = swapping_after(from.join("sub"), 2, move || {
            fs::rename(&other, &replaced).unwrap()
        });
        let to = dir.path().join("to");

        assert_eq!(
            stopped_at(store.copy_tree(dir.path(), &from, &to)),
            Path::new("sub").join("b.md")
        );
        assert!(
            !to.join("sub").join("b.md").exists(),
            "nothing of it copied"
        );

        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        let store = swapping_after(from.join("sub"), 2, move || {
            fs::remove_file(&b).unwrap();
            link_dir(&target, &b).unwrap();
        });
        let at = stopped_at(store.copy_tree(dir.path(), &from, &dir.path().join("to2")));
        assert_eq!(at, Path::new("sub").join("b.md"));
        engram_dashboard_platform::fs::unlink_dir(&from.join("sub").join("b.md")).unwrap();
    }

    // ADR-0365 · ADR-0409 decision 12: a re-hash reads only the file listed — another tag, a link at its name or a
    //   folder on the way swapped for a link answers `None` with nothing read.
    #[test]
    fn a_content_hash_reads_only_the_file_listed() {
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path().join("top");
        let a = write(&top, "sub/a.md", "a");
        write(dir.path(), "outside/a.md", "a");
        let rel = Path::new("sub").join("a.md");
        let listed = FsStore.tag(&a).unwrap();
        let hash = FsStore
            .content_hash(dir.path(), &top, &rel, &listed)
            .unwrap();
        assert_eq!(hash, Some(digest(Sha256::new_with_prefix(b"a"))));

        fs::write(&a, "a longer body").unwrap();
        assert_eq!(
            FsStore
                .content_hash(dir.path(), &top, &rel, &listed)
                .unwrap(),
            None,
            "another tag"
        );

        let (sub, outside) = (top.join("sub"), dir.path().join("outside"));
        fs::rename(&sub, top.join("moved")).unwrap();
        link_dir(&outside, &sub).unwrap();
        let through = FsStore.content_hash(
            dir.path(),
            &top,
            &rel,
            &FsStore.tag(&outside.join("a.md")).unwrap(),
        );
        engram_dashboard_platform::fs::unlink_dir(&sub).unwrap();
        assert_eq!(through.unwrap(), None, "a folder on the way is a link");
    }

    /// A mount folder `dir/mount` holding the item `parent/tree/sub/b.md`, and outside it the same names with other
    /// bytes, the same size and modified time — what a link put in place of `parent` would show.
    fn above_the_item(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let root = dir.join("mount");
        let b = write(&root, "parent/tree/sub/b.md", "real");
        let secret = write(dir, "outside/tree/sub/b.md", "XXXX");
        let then = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        for file in [&b, &secret] {
            let opened = fs::File::options().write(true).open(file).unwrap();
            opened
                .set_times(fs::FileTimes::new().set_modified(then))
                .unwrap();
        }
        (root.clone(), root.join("parent"), dir.join("outside"))
    }

    // ADR-0365: a folder ABOVE the copied item swapped for a link while the tree is listed — the listing stops there
    //   (it checks every name from the mount folder down), nothing is made and nothing of the link's target read.
    #[test]
    fn a_tree_listing_stops_at_a_folder_above_the_item_swapped_for_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let (root, parent, outside) = above_the_item(dir.path());
        let from = parent.join("tree");
        let link = parent.clone();
        let store = swapping(from.join("sub"), move || {
            fs::rename(&link, link.with_file_name("moved")).unwrap();
            link_dir(&outside, &link).unwrap();
        });
        let to = dir.path().join("to");

        let copied = store.copy_tree(&root, &from, &to);

        engram_dashboard_platform::fs::unlink_dir(&parent).unwrap();
        match copied {
            Err(CopyFailure::Failed {
                error, made: false, ..
            }) => assert_eq!(error.kind(), io::ErrorKind::NotADirectory, "{error}"),
            other => panic!("not refused at the listing: {other:?}"),
        }
        assert!(fs::symlink_metadata(&to).is_err(), "nothing made");
    }

    // ADR-0365: a folder ABOVE the copied item swapped for a link during the copy, after the listing — the file is not
    //   read through it: the copy stops there.
    #[test]
    fn a_tree_copy_stops_at_a_folder_above_the_item_swapped_for_a_link_during_it() {
        let dir = tempfile::tempdir().unwrap();
        let (root, parent, outside) = above_the_item(dir.path());
        let from = parent.join("tree");
        let link = parent.clone();
        // The listing lists `sub` and stats it on the way to `sub/b.md`'s tag; the third touch is the copy's.
        let store = swapping_after(from.join("sub"), 2, move || {
            fs::rename(&link, link.with_file_name("moved")).unwrap();
            link_dir(&outside, &link).unwrap();
        });
        let to = dir.path().join("to");

        let at = stopped_at(store.copy_tree(&root, &from, &to));

        engram_dashboard_platform::fs::unlink_dir(&parent).unwrap();
        assert_eq!(at, Path::new("sub").join("b.md"));
        assert!(
            !to.join("sub").join("b.md").exists(),
            "nothing read through the link"
        );
    }

    // ADR-0365: a folder of the source swapped for a link after the files were copied, before the folders' times are
    //   kept — the folder pass stops at the check, before any metadata read through the link. The link's target is gone,
    //   so a read through it would answer `NotFound`; the copy fails as `not as listed` instead.
    #[test]
    fn the_folder_pass_reads_no_metadata_through_a_folder_swapped_for_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        write(&from, "sub/deep/b.md", "b");
        let (sub, target) = (from.join("sub"), dir.path().join("target"));
        let link = sub.clone();
        // `sub` is touched three times by the listing and once by the copy of `sub/deep/b.md`; the fifth touch is the
        //   folder pass's check on the way to `sub/deep`.
        let store = swapping_after(sub.clone(), 4, move || {
            fs::rename(&link, link.with_file_name("moved")).unwrap();
            fs::create_dir(&target).unwrap();
            link_dir(&target, &link).unwrap();
            fs::remove_dir(&target).unwrap();
        });
        let to = dir.path().join("to");

        let copied = store.copy_tree(dir.path(), &from, &to);

        engram_dashboard_platform::fs::unlink_dir(&sub).unwrap();
        match copied {
            Err(CopyFailure::Failed {
                error,
                made: true,
                at,
            }) => {
                assert_eq!(at, Path::new("sub").join("deep"));
                assert_eq!(error.to_string(), not_as_listed().to_string(), "{error:?}");
            }
            other => panic!("not stopped at the folder pass: {other:?}"),
        }
        assert_eq!(
            fs::read_to_string(to.join("sub").join("deep").join("b.md")).unwrap(),
            "b"
        );
    }

    // A file a file-system filter answers for (`compact /exe` — WOF) is copied with its own bytes. Skipped where
    //   `compact` is not there or does not compress.
    #[test]
    fn a_tree_copy_of_a_compressed_file_copies_its_own_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        let bytes: String = (0..200_000u32)
            .map(|n| char::from(b'a' + (n % 26) as u8))
            .collect();
        let file = write(&from, "w.txt", &bytes);
        let compressed = std::process::Command::new("compact")
            .args(["/c", "/exe:xpress4k"])
            .arg(&file)
            .output()
            .is_ok_and(|done| done.status.success());
        if !compressed {
            eprintln!("compact unavailable: skipped");
            return;
        }
        let to = dir.path().join("to");

        FsStore.copy_tree(dir.path(), &from, &to).unwrap();

        assert_eq!(fs::read_to_string(to.join("w.txt")).unwrap(), bytes);
    }

    fn write(root: &Path, rel: &str, text: &str) -> PathBuf {
        let path = lib(rel).to_path(root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn a_create_whose_write_fails_leaves_the_file_it_made_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");

        let answer = create_new_with(&path, |_| Err(io::Error::other("injected write failure")));

        let error = answer.expect_err("the write failed");
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert!(error.to_string().contains("a.md"), "{error}");
        assert!(path.exists(), "never removed by name");
        assert_eq!(
            FsStore.create_new(&path, b"x").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
    }

    #[test]
    fn exists_answers_present_only_for_a_stat_that_answers() {
        assert!(present(&Ok(())));
        for kind in [
            io::ErrorKind::NotFound,
            io::ErrorKind::NotADirectory,
            io::ErrorKind::PermissionDenied,
        ] {
            assert!(!present(&Err::<(), _>(io::Error::from(kind))), "{kind:?}");
        }
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "a.md", "a");
        assert!(FsStore.exists(&file));
        assert!(!FsStore.exists(&dir.path().join("gone.md")));
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
        let mut names: Vec<&str> = listing.folders.iter().map(MountPath::as_str).collect();
        names.sort_unstable();
        names
    }

    // ── names ─────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn admits_refuses_dot_names_and_base_temp_names_at_any_depth() {
        for good in ["a.md", "rules/a.md", "a.tmp", "a.md.tmp", "x/y.tmp12"] {
            assert!(admits(good), "{good}");
        }
        for bad in [
            ".trash",
            ".trash/b/a.md",
            "rules/.x/a.md",
            ".a.md",
            "a.md.tmp4120.0",
            "x/p.yaml.tmp1.2/y.md",
        ] {
            assert!(!admits(bad), "{bad}");
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
        fs::create_dir_all(root.join(TRASH_DIR).join("b1")).unwrap();
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

    /// A range below a file is absent on every system — Windows answers `NotFound` there, others `NotADirectory`,
    /// which the link probe turns into the same sequenced absence instead of an error entry (ADR-0387).
    #[test]
    fn a_partial_scan_below_a_file_is_a_sequenced_absence() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "afile.md", "a");
        let listing = FsStore
            .scan(root, Some(&lib("afile.md/child")), &counter())
            .unwrap();
        assert_eq!(
            listing.absent_ranges.len(),
            1,
            "{:?}",
            listing.absent_ranges
        );
        assert_eq!(listing.absent_ranges[0].0, lib("afile.md/child"));
        assert!(listing.errors.is_empty(), "{:?}", listing.errors);
        assert!(listing.files.is_empty() && listing.folders.is_empty());
    }

    /// The link probe of a partial scan's range: no link walks it; a link skips it; `NotADirectory` (below a file) is
    /// a plain absence; any other failure is an error entry (ADR-0387).
    #[test]
    fn the_range_probe_takes_a_path_below_a_file_as_absent_and_other_failures_as_errors() {
        let under = lib("a/b");
        let probe = |answer: io::Result<Option<MountPath>>| {
            let mut listing = ScanListing::default();
            let walk = range_probe(answer, &under, 7, &mut listing);
            (walk, listing)
        };
        let (walk, listing) = probe(Ok(None));
        assert!(walk);
        assert!(listing.absent_ranges.is_empty() && listing.errors.is_empty());

        let (walk, listing) = probe(Ok(Some(lib("a"))));
        assert!(!walk);
        assert_eq!(listing.skipped_links, [under.clone()]);
        assert_eq!(listing.absent_ranges, [(under.clone(), 7)]);

        let (walk, listing) = probe(Err(io::Error::from(io::ErrorKind::NotADirectory)));
        assert!(!walk);
        assert_eq!(listing.absent_ranges, [(under.clone(), 7)]);
        assert!(listing.skipped_links.is_empty() && listing.errors.is_empty());

        let (walk, listing) = probe(Err(io::Error::from(io::ErrorKind::PermissionDenied)));
        assert!(!walk);
        assert!(listing.absent_ranges.is_empty());
        assert_eq!(listing.errors.len(), 1);
        assert_eq!(listing.errors[0].0, Some(under.clone()));
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
        fs::create_dir_all(root.join(TRASH_DIR).join("b1")).unwrap();
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
        store.dir(".trash/b1");
        let path = lib("rules/a.md").to_path(mem_root);
        let moved = store.trash(mem_root, &path, &bundle).unwrap();
        store.file("rules/a.md", "second");
        let error = store.trash(mem_root, &path, &bundle).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(store.read(&moved, u64::MAX).unwrap(), b"first");
        assert_eq!(store.read(&path, u64::MAX).unwrap(), b"second");
    }

    // ADR-0409 decision 10 — there is no check before the rename, so only the
    //   rename itself can refuse the taken name here.
    #[test]
    fn trash_never_replaces_a_file_at_the_destination() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = write(root, "rules/a.md", "moving");
        let there = write(root, ".trash/b1/rules/a.md", "already there");
        let bundle = TrashBundle("b1".to_owned());

        let error = FsStore.trash(root, &file, &bundle).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error:?}");
        assert_eq!(fs::read_to_string(&there).unwrap(), "already there");
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "moving",
            "nothing moved"
        );

        let other = write(root, "rules/b.md", "other");
        let error = FsStore.rename(&other, &file).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error:?}");
        assert_eq!(fs::read_to_string(&file).unwrap(), "moving");

        let mem_root = Path::new("/lib");
        let store = mem::MemStore::new(mem_root);
        store.file("rules/a.md", "moving");
        store.file(".trash/b1/rules/a.md", "already there");
        let path = lib("rules/a.md").to_path(mem_root);
        let error = store.trash(mem_root, &path, &bundle).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        let kept = lib(".trash/b1/rules/a.md").to_path(mem_root);
        assert_eq!(store.read(&kept, u64::MAX).unwrap(), b"already there");
        assert_eq!(store.read(&path, u64::MAX).unwrap(), b"moving");
    }

    #[test]
    fn trash_needs_its_bundle_and_makes_the_folders_below_it_never_through_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("lib");
        let file = write(&root, "rules/deep/a.md", "x");
        let bundle = TrashBundle("b1".to_owned());

        let error = FsStore.trash(&root, &file, &bundle).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound, "{error:?}");
        assert!(
            !root.join(TRASH_DIR).exists(),
            "the trash step makes .trash and the bundle"
        );

        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(root.join(TRASH_DIR).join("b1")).unwrap();
        link_dir(&outside, &root.join(TRASH_DIR).join("b1").join("rules")).unwrap();
        let error = FsStore.trash(&root, &file, &bundle).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotADirectory, "{error:?}");
        assert!(file.exists(), "nothing moved");
        assert_eq!(
            fs::read_dir(&outside).unwrap().count(),
            0,
            "nothing made through the link"
        );

        let other = TrashBundle("b2".to_owned());
        fs::create_dir_all(root.join(TRASH_DIR).join("b2")).unwrap();
        let moved = FsStore.trash(&root, &file, &other).unwrap();
        assert_eq!(moved, lib(".trash/b2/rules/deep/a.md").to_path(&root));
        assert_eq!(fs::read_to_string(&moved).unwrap(), "x");
    }

    #[test]
    fn trash_never_moves_an_item_through_a_link_among_its_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("lib");
        let outside = dir.path().join("outside");
        let kept = write(&outside, "x.md", "keep");
        fs::create_dir_all(root.join(TRASH_DIR).join("b1")).unwrap();
        link_dir(&outside, &root.join("linked")).unwrap();
        let through = root.join("linked").join("x.md");

        let error = FsStore
            .trash(&root, &through, &TrashBundle("b1".to_owned()))
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotADirectory, "{error:?}");
        assert_eq!(fs::read_to_string(&kept).unwrap(), "keep");
        assert_eq!(
            fs::read_dir(root.join(TRASH_DIR).join("b1"))
                .unwrap()
                .count(),
            0,
            "nothing made or moved"
        );
    }

    #[test]
    fn trash_of_an_item_whose_folder_is_gone_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("lib");
        fs::create_dir_all(root.join(TRASH_DIR).join("b1")).unwrap();
        let gone = root.join("gone").join("x.md");

        let error = FsStore
            .trash(&root, &gone, &TrashBundle("b1".to_owned()))
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound, "{error:?}");
        assert_eq!(
            fs::read_dir(root.join(TRASH_DIR).join("b1"))
                .unwrap()
                .count(),
            0,
            "nothing made"
        );
    }

    #[test]
    fn check_names_answers_the_first_name_that_is_a_link_a_file_or_missing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("lib");
        write(&root, "a/b/c.md", "x");
        let outside = dir.path().join("outside");
        write(&outside, "d/e.md", "behind the link");
        link_dir(&outside, &root.join("a").join("link")).unwrap();
        let twin = mem_twin();
        let stores: [(&dyn LibraryStore, &Path); 2] =
            [(&FsStore, &root), (&twin, Path::new("/lib"))];
        for (store, root) in stores {
            let walk = |rel: &str| store.check_names(root, &lib(rel)).unwrap();
            assert_eq!(walk("a/b/c.md"), NameWalk::Present(EntryKind::File));
            assert_eq!(walk("a/b"), NameWalk::Present(EntryKind::Folder));
            assert_eq!(walk("a/link"), NameWalk::Present(EntryKind::Link));
            assert_eq!(walk("a/link/d/e.md"), NameWalk::Link(lib("a/link")));
            assert_eq!(walk("a/b/c.md/x"), NameWalk::NotAFolder(lib("a/b/c.md")));
            assert_eq!(walk("a/gone/x.md"), NameWalk::Missing(lib("a/gone")));
        }
    }

    /// A [`mem::MemStore`] at `/lib` holding what [`check_names_answers_the_first_name_that_is_a_link_a_file_or_missing`]
    /// puts on disk.
    fn mem_twin() -> mem::MemStore {
        let store = mem::MemStore::new(Path::new("/lib"));
        store.file("a/b/c.md", "x");
        store.link("a/link");
        store
    }

    #[test]
    fn stat_tells_not_found_from_other_errors_and_never_follows() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = write(root, "a.md", "x");
        link_dir(root, &root.join("link")).unwrap();
        assert_eq!(FsStore.stat(&file).unwrap(), EntryKind::File);
        assert_eq!(FsStore.stat(&root.join("link")).unwrap(), EntryKind::Link);
        assert_eq!(FsStore.stat(root).unwrap(), EntryKind::Folder);
        let gone = FsStore.stat(&root.join("gone.md")).unwrap_err();
        assert_eq!(gone.kind(), io::ErrorKind::NotFound);

        let store = mem::MemStore::new(Path::new("/lib"));
        store.file("a.md", "x");
        store.fail_stat("a.md");
        let failed = store
            .stat(&lib("a.md").to_path(Path::new("/lib")))
            .unwrap_err();
        assert_ne!(
            failed.kind(),
            io::ErrorKind::NotFound,
            "a failed stat is not an absence"
        );
        assert!(store.check_names(Path::new("/lib"), &lib("a.md")).is_err());
    }

    #[test]
    fn list_and_the_removals_never_act_through_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("lib");
        let outside = dir.path().join("outside");
        let kept = write(&outside, "x.md", "keep");
        let empty_target = dir.path().join("empty");
        fs::create_dir_all(&empty_target).unwrap();
        write(&root, "full/a.md", "a");
        fs::create_dir_all(root.join("empty")).unwrap();
        link_dir(&outside, &root.join("link")).unwrap();
        link_dir(&empty_target, &root.join("empty-link")).unwrap();

        let mut names: Vec<(String, EntryKind)> = FsStore
            .list(&root)
            .unwrap()
            .into_iter()
            .map(|(name, kind)| (name.into_string().unwrap(), kind))
            .collect();
        names.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            names,
            [
                ("empty".to_owned(), EntryKind::Folder),
                ("empty-link".to_owned(), EntryKind::Link),
                ("full".to_owned(), EntryKind::Folder),
                ("link".to_owned(), EntryKind::Link),
            ]
        );
        let error = FsStore.list(&root.join("link")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotADirectory);

        for link in ["link", "empty-link"] {
            let error = FsStore.remove_dir(&root.join(link)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::NotADirectory, "{link}");
            assert!(
                fs::symlink_metadata(root.join(link)).is_ok(),
                "{link} stays"
            );
        }
        assert!(FsStore.remove_dir(&root.join("full")).is_err());
        assert!(
            root.join("full").join("a.md").exists(),
            "content is never removed"
        );
        FsStore.remove_dir(&root.join("empty")).unwrap();
        assert!(!root.join("empty").exists());

        let error = FsStore.remove_file(&root.join("link")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        let error = FsStore.remove_file(&root.join("full")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        FsStore
            .remove_file(&root.join("full").join("a.md"))
            .unwrap();
        assert_eq!(fs::read_to_string(&kept).unwrap(), "keep");
        assert!(empty_target.exists());
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
        let top: Option<MountPath> = None;
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
