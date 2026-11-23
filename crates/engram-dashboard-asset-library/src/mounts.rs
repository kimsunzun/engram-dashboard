//! The mounts of the virtual library root: the mount table, the mount name rules, the overlap check, and the only
//! converter between the two path types.
//!
//! - **Two path types.** [`LibPath`] is a virtual path from the virtual root, `<mount>/…` — what the catalog,
//!   references, the wire and the resolver use; a path of one name names a mount. [`MountPath`] is a path inside one
//!   mount's folder — the only path type the store takes and returns. Only this module turns one into the other
//!   ([`Mounts::locate`] · [`virtual_path`]) and joins a path to a folder on disk ([`MountPath::to_path`] ·
//!   [`MountPath::from_disk`] · [`Mounts::disk_path`]), so the store stays mount-unaware and a missed prefix or strip
//!   does not compile.
//! - **Names** ([`MountName`]): one `LibPath` name, no leading `.`, at most [`NAME_MAX_CHARS`] characters, not the
//!   built-in's [`BUILTIN_MOUNT`] (matched ASCII case-insensitively on every OS); two names are the same mount when their
//!   platform path keys are equal. A name need not equal anything on disk.
//! - **Overlap** ([`overlap`]): judged on canonical final paths (links and junctions resolved) by path key, component
//!   by component, and on the file identities of the folders and their ancestors where the volume gives one, every ask
//!   bounded and batched. Only mounts are compared — a mount may lie anywhere else, the daemon's own folders included.
//!   A folder being added or relocated fails closed: when its own canonical path fails (no answer or an OS error — one
//!   a `create` will make is judged by its nearest ancestor that has one), or an ask about it or about a mount it is
//!   compared with gives no answer, it is refused, never taken as free. A mount whose canonical path the OS refuses (an
//!   offline mount) is judged on its spelling, logged. At load the saved mounts are judged against each other
//!   ([`overlapping`]), where nothing fails closed: a canonical path that cannot be had falls back to the spelling, and
//!   an identity that gives no answer is left unknown while the canonical path still judges — each logged. Known
//!   escapes: SMB servers that give no file id, one share under several host spellings.
//! - **Names inside a mount** ([`admits_inside`]): the store's name rule applies to the names below a mount's top only;
//!   the mount's own name is held by the name rules above.
// ADR-0396
// ADR-0400

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use engram_dashboard_agent::launch::escape_controls;
use engram_dashboard_platform::fs::{path_key, FileIdentity, PathKey};

use crate::folder::{gave_no_answer, BoundedIo, LaunchIo, Need};
use crate::path::{LibPath, PathError};
use crate::store::admits;

/// The reserved name of the built-in mount — never registered, never stored.
// ADR-0400
pub const BUILTIN_MOUNT: &str = "engram";

/// The name of the mount the first start registers at the default place.
// ADR-0400 decision 3
pub const FIRST_MOUNT: &str = "default_library";

/// The longest mount name, in characters.
pub const NAME_MAX_CHARS: usize = 64;

/// Why a mount name was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MountNameError {
    /// Not one `LibPath` name (`/` included) — the path rule's own reason.
    #[error("{0}")]
    NotOneName(PathError),
    #[error("a mount name starts with '.'")]
    LeadingDot,
    #[error("a mount name is longer than {NAME_MAX_CHARS} characters")]
    TooLong,
    #[error("'{BUILTIN_MOUNT}' is reserved for the built-in")]
    Reserved,
}

/// A mount name, valid by construction (module header).
// ADR-0400 decision 2
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountName(String);

impl MountName {
    pub fn parse(text: &str) -> Result<Self, MountNameError> {
        if text.contains('/') {
            return Err(MountNameError::NotOneName(PathError::ForbiddenChar('/')));
        }
        LibPath::parse(text).map_err(MountNameError::NotOneName)?;
        if text.starts_with('.') {
            return Err(MountNameError::LeadingDot);
        }
        if text.chars().count() > NAME_MAX_CHARS {
            return Err(MountNameError::TooLong);
        }
        if is_builtin(text) {
            return Err(MountNameError::Reserved);
        }
        Ok(Self(text.to_owned()))
    }

    /// The name the first start registers ([`FIRST_MOUNT`]).
    pub fn first() -> Self {
        Self(FIRST_MOUNT.to_owned())
    }

    /// The mount name the test fixtures use — `library`, not the first start's name.
    #[cfg(test)]
    pub(crate) fn library() -> Self {
        Self("library".to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The key two names are compared by — the platform path key, as the catalog compares paths.
    pub fn key(&self) -> PathKey {
        name_key(&self.0)
    }
}

/// Whether `name` is the built-in mount's name — ASCII case-insensitive on every OS, so a reference spelled `Engram/…`
/// means the built-in on a case-sensitive OS too.
pub fn is_builtin(name: &str) -> bool {
    name.eq_ignore_ascii_case(BUILTIN_MOUNT)
}

/// Whether `path` lies in the built-in mount — its first name is [`BUILTIN_MOUNT`] (its top included).
// ADR-0400 decision 20
pub fn in_builtin(path: &LibPath) -> bool {
    builtin_rest(path).is_some()
}

/// `path`'s path inside the built-in mount — what follows its first name when that name is the built-in's
/// ([`is_builtin`]: ASCII case-insensitive on every OS), `""` for the mount's top; `None` = not in the built-in mount.
/// The one place the mount segment is matched: what follows it (an id, the folder `settings`) is matched exactly by
/// every reader, `list` and `read` alike.
// ADR-0400 decision 20
pub fn builtin_rest(path: &LibPath) -> Option<&str> {
    let (first, rest) = path.as_str().split_once('/').unwrap_or((path.as_str(), ""));
    is_builtin(first).then_some(rest)
}

fn name_key(name: &str) -> PathKey {
    path_key(Path::new(name))
}

/// A registered mount: its name and the folder it maps to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub name: MountName,
    pub folder: PathBuf,
}

/// Two mounts in one list whose names are the same by path key (`MountName::key`).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the mount name {0:?} is taken")]
pub struct NameTaken(pub String);

/// The registered mounts in the order shown; names unique by [`MountName::key`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mounts(Vec<Mount>);

impl Mounts {
    pub fn new(list: Vec<Mount>) -> Result<Self, NameTaken> {
        let mut seen = HashSet::new();
        for mount in &list {
            if !seen.insert(mount.name.key()) {
                return Err(NameTaken(mount.name.as_str().to_owned()));
            }
        }
        Ok(Self(list))
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Mount> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The mount named `name` by path key; `None` = not registered.
    pub fn get(&self, name: &str) -> Option<&Mount> {
        let want = name_key(name);
        self.0.iter().find(|mount| mount.name.key() == want)
    }

    /// The mount `path` starts in and the rest of `path` inside it — `None` rest = `path` names the mount itself.
    /// `None` = its first name names no registered mount: the path reads as absent, never as a path under some default
    /// mount (ADR-0400 decision 4).
    pub fn locate(&self, path: &LibPath) -> Option<(&Mount, Option<MountPath>)> {
        let (name, rest) = match path.as_str().split_once('/') {
            Some((name, rest)) => (name, Some(rest)),
            None => (path.as_str(), None),
        };
        let mount = self.get(name)?;
        let rest = rest.map(|rest| {
            MountPath(
                LibPath::parse(rest)
                    .expect("the names after the first of a valid path are a valid path"),
            )
        });
        Some((mount, rest))
    }

    /// The disk path of `path`: the rest of it under its mount's folder, or the folder itself for a one-name path.
    /// `None` = its first name names no registered mount.
    pub fn disk_path(&self, path: &LibPath) -> Option<PathBuf> {
        let (mount, rest) = self.locate(path)?;
        Some(match rest {
            Some(rest) => rest.to_path(&mount.folder),
            None => mount.folder.clone(),
        })
    }

    /// Registers `mount` at the end of the list (ADR-0400 decision 9). `Err` = its name is taken, nothing changed.
    pub(crate) fn push(&mut self, mount: Mount) -> Result<(), NameTaken> {
        if self.get(mount.name.as_str()).is_some() {
            return Err(NameTaken(mount.name.as_str().to_owned()));
        }
        self.0.push(mount);
        Ok(())
    }

    /// Drops the mount named `name` (by path key) and answers it; `None` = not registered.
    pub(crate) fn remove(&mut self, name: &MountName) -> Option<Mount> {
        let at = self
            .0
            .iter()
            .position(|mount| mount.name.key() == name.key())?;
        Some(self.0.remove(at))
    }

    /// Re-points the mount named `name` (by path key) at `folder`, keeping its name and place in the list; `false` = not
    /// registered.
    pub(crate) fn relocate(&mut self, name: &MountName, folder: PathBuf) -> bool {
        match self
            .0
            .iter_mut()
            .find(|mount| mount.name.key() == name.key())
        {
            Some(mount) => {
                mount.folder = folder;
                true
            }
            None => false,
        }
    }

    /// Renames the mount named `from` (by path key) to `to`, keeping its folder and its place in the list (ADR-0400
    /// decision 12); `to` may differ from `from` only in case. `false` = `from` is not registered, or another mount's
    /// name equals `to` by path key — nothing changed.
    pub(crate) fn rename(&mut self, from: &MountName, to: MountName) -> bool {
        let taken = self
            .0
            .iter()
            .any(|mount| mount.name.key() == to.key() && mount.name.key() != from.key());
        if taken {
            return false;
        }
        match self
            .0
            .iter_mut()
            .find(|mount| mount.name.key() == from.key())
        {
            Some(mount) => {
                mount.name = to;
                true
            }
            None => false,
        }
    }
}

/// A path inside one mount's folder, relative to it — the store's path type, with the `LibPath` name rules. Made from a
/// virtual path only by [`Mounts::locate`], and from disk by [`MountPath::from_disk`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MountPath(LibPath);

impl MountPath {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The disk path of this path under the mount folder `folder`: `folder` with each name pushed in turn, so a folder
    /// that ends in a separator (`D:\` · `\\nas\lib\` · `/`) gets no second one.
    pub fn to_path(&self, folder: &Path) -> PathBuf {
        let mut path = folder.to_path_buf();
        for name in self.as_str().split('/') {
            path.push(name);
        }
        path
    }

    /// The path of `path`, a disk path under the mount folder `folder` as a scan builds it (`folder` joined with
    /// names). `path` must start with `folder` as spelled — the prefix is compared component by component, not by the
    /// OS rule. `Err(NotUnderRoot)` = not below `folder` (or through `..`); the names must pass [`LibPath::parse`] too,
    /// so a scanned name ending in `.` or a space is refused.
    pub fn from_disk(folder: &Path, path: &Path) -> Result<Self, PathError> {
        let rest = path
            .strip_prefix(folder)
            .map_err(|_| PathError::NotUnderRoot)?;
        let mut names = Vec::new();
        for component in rest.components() {
            match component {
                Component::Normal(name) => names.push(name.to_str().ok_or(PathError::NotUtf8)?),
                _ => return Err(PathError::NotUnderRoot),
            }
        }
        LibPath::parse(&names.join("/")).map(Self)
    }

    /// The folder holding this path inside the mount; `None` = it sits directly in the mount's folder.
    pub fn parent(&self) -> Option<MountPath> {
        self.0.parent().map(Self)
    }

    /// The last name.
    pub fn file_name(&self) -> &str {
        self.0.file_name()
    }

    /// A path inside a mount spelled as a test writes it.
    #[cfg(test)]
    pub(crate) fn parse(text: &str) -> Result<Self, PathError> {
        LibPath::parse(text).map(Self)
    }
}

/// The store's name rule ([`admits`]) over the names of `path` below its mount's top. The mount's own name is held by
/// the name rules instead, so a valid name shaped like a temp file (`work.tmp1.2`) does not hide its mount; a one-name
/// path has no name below a top and passes.
// ADR-0400 decision 2
pub fn admits_inside(path: &LibPath) -> bool {
    path.as_str()
        .split_once('/')
        .map_or(true, |(_, rest)| admits(rest))
}

/// `path` moved from the mount `from` to the mount `to` — the same names after its first, the first spelled as `to` —
/// when its first name is `from` by path key (the mount lookup's rule); `None` = it lies in another mount, or at the
/// virtual root's other names. The one converter a mount rename re-points references and catalog paths with.
// ADR-0400 decision 12
pub fn moved_to(path: &LibPath, from: &MountName, to: &MountName) -> Option<LibPath> {
    let (first, rest) = match path.as_str().split_once('/') {
        Some((first, rest)) => (first, Some(rest)),
        None => (path.as_str(), None),
    };
    if name_key(first) != from.key() {
        return None;
    }
    let rest = rest.map(|rest| {
        MountPath(
            LibPath::parse(rest)
                .expect("the names after the first of a valid path are a valid path"),
        )
    });
    Some(virtual_path(to, rest.as_ref()))
}

/// The virtual path of `rest` inside the mount `name`, or of the mount itself when `rest` is `None`.
pub fn virtual_path(name: &MountName, rest: Option<&MountPath>) -> LibPath {
    let text = match rest {
        Some(rest) => format!("{}/{}", name.as_str(), rest.as_str()),
        None => name.as_str().to_owned(),
    };
    LibPath::parse(&text).expect("a mount name followed by a valid path is a valid path")
}

/// Why a folder's canonical path could not be had.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoCanonical {
    /// The OS answered with an error (the folder is missing, access was refused …): a mount is then judged on its
    /// spelling.
    Os(String),
    /// The ask gave no answer — it timed out, was not checked (the budget was spent) or its worker failed.
    Unanswered(String),
}

/// What [`overlap`] asks of the disk, one batch per call. Production = [`DiskProbe`].
pub trait FolderProbe {
    type Id: PartialEq + Clone;
    /// The canonical final path of each of `folders`, links and junctions resolved — one answer per folder, in order.
    fn canonical_all(&self, folders: &[PathBuf]) -> Vec<Result<PathBuf, NoCanonical>>;
    /// The file identity of each of `folders`, one answer per folder in order: `Ok(None)` = it has none to compare
    /// (`Unsupported`, or the OS refused); `Err(why)` = the ask gave no answer (timed out, not checked, a failed
    /// worker).
    fn identity_all(&self, folders: &[PathBuf]) -> Vec<Result<Option<Self::Id>, String>>;
}

/// The production probe: canonical paths and identities through the bounded IO, as required asks on `read` — so every
/// ask of one judgement shares that read's budget (the caller opens one per judgement).
pub struct DiskProbe<'a> {
    pub io: &'a BoundedIo,
    pub read: &'a LaunchIo,
}

impl FolderProbe for DiskProbe<'_> {
    type Id = FileIdentity;

    fn canonical_all(&self, folders: &[PathBuf]) -> Vec<Result<PathBuf, NoCanonical>> {
        // One answer per path key: a folder asked twice reads the same answer.
        let answers = self.io.canonical_all(self.read, &required(folders));
        folders
            .iter()
            .map(|folder| match answers.get(&path_key(folder)).cloned() {
                Some(Ok(canonical)) => Ok(canonical),
                Some(Err(status)) if gave_no_answer(&status) => {
                    Err(NoCanonical::Unanswered(status.why()))
                }
                Some(Err(status)) => Err(NoCanonical::Os(status.why())),
                None => Err(NoCanonical::Unanswered(
                    "the bounded canonicalize gave no answer".to_owned(),
                )),
            })
            .collect()
    }

    fn identity_all(&self, folders: &[PathBuf]) -> Vec<Result<Option<FileIdentity>, String>> {
        let answers = self.io.identity_all(self.read, &required(folders));
        folders
            .iter()
            .map(|folder| match answers.get(&path_key(folder)).cloned() {
                Some(Ok(identity)) => Ok(identity),
                Some(Err(status)) => Err(status.why()),
                None => Err("the bounded identity gave no answer".to_owned()),
            })
            .collect()
    }
}

fn required(folders: &[PathBuf]) -> Vec<(PathBuf, Need)> {
    folders
        .iter()
        .map(|folder| (folder.clone(), Need::Required))
        .collect()
}

/// The first of `mounts` (in order) that `folder` — a folder being added or relocated — is the same as, holds or lies
/// inside, if any. `mounts` = every mount to judge against — a relocate leaves the mount itself out (module header for
/// how folders compare). Only mounts are judged: a mount may lie anywhere else, the daemon's own folders included
/// (ADR-0400 decision 10 as amended). The canonical paths of `folder` and of every mount are asked in one batch, then
/// the identities of every folder and ancestor they name in one batch, all on the probe's one budget.
///
/// `missing` = `folder` may not exist yet (a command's `create` makes it only after this judgement): every ancestor of
/// it is asked in the same canonical batch, and it is judged by the nearest one whose canonical path the OS gives, with
/// the spelling of the names below that one appended (those names have no identity).
///
/// `Err(why)` = the judgement cannot be made, so the caller refuses `folder` as a folder that cannot be used
/// (`library-root-unusable`), never as free: `folder`'s own canonical path failed — with `missing`, no ancestor's
/// canonical path was had, or one nearer than the first had gave no answer — or an identity of it, or the canonical
/// path or an identity of a mount, gave no answer. A mount whose canonical path the OS refused (an offline mount) is
/// judged on its spelling, with a `warn` log.
// ADR-0400 decision 10
pub fn overlap<'m, P: FolderProbe>(
    folder: &Path,
    missing: bool,
    mounts: impl IntoIterator<Item = &'m Mount>,
    probe: &P,
) -> Result<Option<MountName>, String> {
    let chain: Vec<PathBuf> = match missing {
        true => folder.ancestors().map(Path::to_path_buf).collect(),
        false => vec![folder.to_path_buf()],
    };
    let mounts: Vec<&Mount> = mounts.into_iter().collect();
    let mut asked = chain.clone();
    asked.extend(mounts.iter().map(|mount| mount.folder.clone()));
    let mut placed = places(&asked, probe);
    let sides = placed.split_off(chain.len());
    let it = candidate(folder, &chain, placed)?;
    for (mount, place) in mounts.into_iter().zip(sides) {
        let side = format!("mount {:?}", mount.name.as_str());
        if it.overlaps(&known(place, &mount.folder, &side)?) {
            return Ok(Some(mount.name.clone()));
        }
    }
    Ok(None)
}

/// The place [`overlap`] judges `folder` by, from the places of `chain` — `folder`, then (when it may be missing) each
/// ancestor up to the root: the first one placed, with the names of `folder` below it appended and unknown identities
/// for them. An OS refusal moves on to the next one of `chain`; an ask that gave no answer, or nothing placed, is
/// `Err`.
fn candidate<I: PartialEq>(
    folder: &Path,
    chain: &[PathBuf],
    placed: Vec<Result<Place<I>, Unplaced<I>>>,
) -> Result<Place<I>, String> {
    let mut refused = None;
    for (at, place) in chain.iter().zip(placed) {
        match place {
            Ok(mut place) => {
                let below = folder.strip_prefix(at).map(names_of).unwrap_or_default();
                for _ in &below {
                    place.ids.insert(0, None);
                }
                place.names.extend(below);
                return Ok(place);
            }
            Err(Unplaced::Os(why)) => refused = Some(why),
            Err(Unplaced::Unanswered(why) | Unplaced::Unidentified { why, .. }) => return Err(why),
        }
    }
    Err(refused.unwrap_or_else(|| {
        format!(
            "the canonical path of {} could not be had",
            folder.display()
        )
    }))
}

/// The saved mounts that overlap an earlier one in the list — each such mount with the first earlier mount it
/// overlaps, in list order (ADR-0400 decision 10: once per run). Unlike [`overlap`] nothing fails closed here: a folder
/// whose canonical path cannot be had (an OS error or no answer) is judged on its spelling, and an identity that gives
/// no answer is unknown while its canonical path still judges — each logged `warn`; a mount that does not answer is not
/// walked anyway (its bounded stat).
// ADR-0400 decision 10
pub fn overlapping<P: FolderProbe>(mounts: &Mounts, probe: &P) -> Vec<(MountName, MountName)> {
    let folders: Vec<PathBuf> = mounts.iter().map(|mount| mount.folder.clone()).collect();
    let placed: Vec<Place<P::Id>> = places(&folders, probe)
        .into_iter()
        .zip(&folders)
        .map(|(place, folder)| match place {
            Ok(place) => place,
            Err(Unplaced::Unidentified { place, why }) => {
                tracing::warn!(
                    folder = %escape_controls(&folder.to_string_lossy()),
                    "library overlap check: an identity gave no answer; judged without it: {}",
                    escape_controls(&why)
                );
                place
            }
            Err(Unplaced::Os(why) | Unplaced::Unanswered(why)) => {
                spelling_fallback(folder, &why);
                Place::spelled(folder)
            }
        })
        .collect();
    let list: Vec<&Mount> = mounts.iter().collect();
    let mut found = Vec::new();
    for (at, place) in placed.iter().enumerate() {
        if let Some(earlier) = (0..at).find(|&earlier| place.overlaps(&placed[earlier])) {
            found.push((list[at].name.clone(), list[earlier].name.clone()));
        }
    }
    found
}

/// Why a folder could not be placed whole for a comparison.
enum Unplaced<I> {
    /// Its canonical path failed with an OS error (`why` says so).
    Os(String),
    /// Its canonical path gave no answer (`why` says so).
    Unanswered(String),
    /// Its canonical path was had but an identity of it or of an ancestor gave no answer (`why` says which): `place`
    /// keeps the canonical names, with each such identity unknown.
    Unidentified { place: Place<I>, why: String },
}

/// A known side of a comparison — a mount (`side` names it in a refusal): judged on its spelling
/// when the OS refused its canonical path (logged), refused when an ask gave no answer.
fn known<I: PartialEq>(
    placed: Result<Place<I>, Unplaced<I>>,
    folder: &Path,
    side: &str,
) -> Result<Place<I>, String> {
    match placed {
        Ok(place) => Ok(place),
        Err(Unplaced::Os(why)) => {
            spelling_fallback(folder, &why);
            Ok(Place::spelled(folder))
        }
        Err(Unplaced::Unanswered(why) | Unplaced::Unidentified { why, .. }) => {
            Err(format!("could not tell whether it overlaps {side}: {why}"))
        }
    }
}

fn spelling_fallback(folder: &Path, why: &str) {
    tracing::warn!(
        folder = %escape_controls(&folder.to_string_lossy()),
        "library overlap check: no canonical path; judged on its spelling: {}",
        escape_controls(why)
    );
}

/// Each of `folders` as [`overlap`] compares it: the canonical paths in one batch, then the identities of every folder
/// and ancestor of those paths in one batch.
fn places<P: FolderProbe>(
    folders: &[PathBuf],
    probe: &P,
) -> Vec<Result<Place<P::Id>, Unplaced<P::Id>>> {
    if folders.is_empty() {
        return Vec::new();
    }
    let canonical = probe.canonical_all(folders);
    let mut asked: Vec<PathBuf> = Vec::new();
    for path in canonical.iter().flatten() {
        for at in path.ancestors() {
            if !asked.iter().any(|seen| seen.as_path() == at) {
                asked.push(at.to_path_buf());
            }
        }
    }
    let identities: Vec<(PathBuf, Result<Option<P::Id>, String>)> = match asked.is_empty() {
        true => Vec::new(),
        false => {
            let answers = probe.identity_all(&asked);
            asked.into_iter().zip(answers).collect()
        }
    };
    canonical
        .into_iter()
        .zip(folders)
        .map(|(canonical, folder)| {
            let canonical = match canonical {
                Ok(canonical) => canonical,
                Err(NoCanonical::Os(why)) => {
                    return Err(Unplaced::Os(format!(
                        "the canonical path of {} could not be had: {why}",
                        folder.display()
                    )))
                }
                Err(NoCanonical::Unanswered(why)) => {
                    return Err(Unplaced::Unanswered(format!(
                        "the canonical path of {} gave no answer: {why}",
                        folder.display()
                    )))
                }
            };
            let mut unanswered: Option<String> = None;
            let ids = canonical
                .ancestors()
                .map(
                    |at| match identities.iter().find(|(asked, _)| asked.as_path() == at) {
                        Some((_, Ok(id))) => id.clone(),
                        Some((_, Err(why))) => {
                            unanswered.get_or_insert_with(|| {
                                format!("the identity of {} gave no answer: {why}", at.display())
                            });
                            None
                        }
                        None => None,
                    },
                )
                .collect();
            let place = Place {
                names: names_of(&canonical),
                ids,
            };
            match unanswered {
                None => Ok(place),
                Some(why) => Err(Unplaced::Unidentified { place, why }),
            }
        })
        .collect()
}

/// One folder as [`overlap`] compares it.
struct Place<I> {
    /// The path key of each component of the canonical path (or of the spelling).
    names: Vec<PathKey>,
    /// The identity of the folder and of each ancestor, the folder first (`None` = unknown); empty when judged on its
    /// spelling.
    ids: Vec<Option<I>>,
}

impl<I: PartialEq> Place<I> {
    /// A folder judged on its spelling alone.
    fn spelled(folder: &Path) -> Self {
        Place {
            names: names_of(folder),
            ids: Vec::new(),
        }
    }

    /// `self` is `other` or lies inside it.
    fn within(&self, other: &Place<I>) -> bool {
        if self.names.starts_with(&other.names) {
            return true;
        }
        match other.ids.first() {
            Some(Some(id)) => self.ids.iter().flatten().any(|have| have == id),
            _ => false,
        }
    }

    fn overlaps(&self, other: &Place<I>) -> bool {
        self.within(other) || other.within(self)
    }
}

fn names_of(path: &Path) -> Vec<PathKey> {
    path.components()
        .filter(|component| !matches!(component, Component::CurDir))
        .map(|component| path_key(Path::new(component.as_os_str())))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use engram_dashboard_platform::fs::{link_dir, normalize_folder, same_path, unlink_dir};

    use super::*;
    use crate::folder::{LaunchWaits, StdFolderOps, ThreadSpawn, IO_LIMIT, LIST_CAP};

    fn name(text: &str) -> MountName {
        MountName::parse(text).expect(text)
    }

    fn lib(text: &str) -> LibPath {
        LibPath::parse(text).expect(text)
    }

    fn mount(text: &str, folder: &str) -> Mount {
        Mount {
            name: name(text),
            folder: PathBuf::from(folder),
        }
    }

    #[test]
    fn a_mount_name_is_one_path_name_without_a_leading_dot_at_most_64_characters_and_not_engram() {
        let not_one = |e: PathError| Err(MountNameError::NotOneName(e));
        let cases: &[(&str, Result<(), MountNameError>)] = &[
            ("", not_one(PathError::Empty)),
            ("a/b", not_one(PathError::ForbiddenChar('/'))),
            ("/a", not_one(PathError::ForbiddenChar('/'))),
            ("a\\b", not_one(PathError::ForbiddenChar('\\'))),
            ("a:b", not_one(PathError::ForbiddenChar(':'))),
            ("a\nb", not_one(PathError::ForbiddenChar('\n'))),
            (".", not_one(PathError::DotName)),
            ("..", not_one(PathError::DotName)),
            ("team.", not_one(PathError::NameEndsInDotOrSpace)),
            ("team ", not_one(PathError::NameEndsInDotOrSpace)),
            (
                "CON",
                not_one(PathError::ReservedDeviceName("CON".to_owned())),
            ),
            (
                "nul.x",
                not_one(PathError::ReservedDeviceName("nul.x".to_owned())),
            ),
            (".trash", Err(MountNameError::LeadingDot)),
            (".git", Err(MountNameError::LeadingDot)),
            ("engram", Err(MountNameError::Reserved)),
            ("Engram", Err(MountNameError::Reserved)),
            ("ENGRAM", Err(MountNameError::Reserved)),
            ("library", Ok(())),
            ("company-presets_2", Ok(())),
            ("engram2", Ok(())),
            ("my engram", Ok(())),
            ("업무", Ok(())), // hangul-ok — non-ASCII mount name fixture
            ("Team", Ok(())),
        ];
        for (text, want) in cases {
            assert_eq!(
                MountName::parse(text).map(|parsed| assert_eq!(parsed.as_str(), *text)),
                want.clone(),
                "{text:?}"
            );
        }
        let longest = "가".repeat(NAME_MAX_CHARS); // hangul-ok — characters, not bytes
        assert!(MountName::parse(&longest).is_ok());
        assert_eq!(
            MountName::parse(&format!("{longest}x")),
            Err(MountNameError::TooLong)
        );
    }

    #[test]
    fn the_built_in_name_is_reserved_case_insensitively_on_every_os() {
        for text in ["engram", "Engram", "ENGRAM", "eNgRaM"] {
            assert!(is_builtin(text), "{text}");
        }
        for text in ["engram2", "engra", "engram.x", ""] {
            assert!(!is_builtin(text), "{text}");
        }
    }

    /// Whether the platform path key folds case here — what a name or path comparison by key must follow.
    fn folds_case() -> bool {
        same_path(Path::new("team"), Path::new("Team"))
    }

    #[test]
    fn names_are_unique_by_path_key() {
        let taken = Mounts::new(vec![mount("team", "/a"), mount("Team", "/b")]);
        match folds_case() {
            true => assert_eq!(taken, Err(NameTaken("Team".to_owned()))),
            false => assert!(
                taken.is_ok(),
                "names differ in case where the key keeps case"
            ),
        }
        assert_eq!(
            Mounts::new(vec![mount("team", "/a"), mount("team", "/b")]),
            Err(NameTaken("team".to_owned()))
        );
        let mounts = Mounts::new(vec![mount("team", "/a"), mount("other", "/b")]).unwrap();
        assert_eq!(
            mounts.get("team").map(|m| m.folder.as_path()),
            Some(Path::new("/a"))
        );
        assert_eq!(mounts.get("TEAM").is_some(), folds_case());
        assert!(mounts.get("nope").is_none());
    }

    #[test]
    fn a_virtual_path_locates_its_mount_and_the_rest_and_joins_back() {
        let mounts = Mounts::new(vec![mount("company", "/c"), mount("library", "/l")]).unwrap();
        let (found, rest) = mounts.locate(&lib("company/skills/review")).unwrap();
        assert_eq!(found.name.as_str(), "company");
        let rest = rest.unwrap();
        assert_eq!(rest.as_str(), "skills/review");
        assert_eq!(
            rest.to_path(Path::new("/c")),
            Path::new("/c").join("skills").join("review")
        );
        assert_eq!(
            virtual_path(&found.name, Some(&rest)),
            lib("company/skills/review")
        );

        let (top, none) = mounts.locate(&lib("library")).unwrap();
        assert_eq!((top.name.as_str(), none), ("library", None));
        assert_eq!(virtual_path(&top.name, None), lib("library"));

        assert!(
            mounts.locate(&lib("unknown/a.md")).is_none(),
            "an unknown mount is absent"
        );
        assert!(mounts.locate(&lib("skills/review")).is_none());
        assert_eq!(
            mounts.locate(&lib("Company/a.md")).is_some(),
            folds_case(),
            "the first name is matched by path key"
        );
    }

    #[test]
    fn a_virtual_path_has_a_disk_path_in_its_mounts_folder_and_none_in_no_mount() {
        let mounts = Mounts::new(vec![mount("company", "/c")]).unwrap();
        assert_eq!(
            mounts.disk_path(&lib("company/skills/review")),
            Some(Path::new("/c").join("skills").join("review"))
        );
        assert_eq!(
            mounts.disk_path(&lib("company")),
            Some(PathBuf::from("/c")),
            "a mount's top is its folder"
        );
        assert_eq!(mounts.disk_path(&lib("other/a.md")), None);
    }

    #[test]
    fn a_mount_path_comes_from_disk_under_its_mount_folder_only() {
        let folder = Path::new("mount-folder");
        let disk = folder.join("a").join("b.md");
        let path = MountPath::from_disk(folder, &disk).unwrap();
        assert_eq!(path.as_str(), "a/b.md");
        assert_eq!(path.to_path(folder), disk);
        assert_eq!(
            MountPath::from_disk(folder, Path::new("elsewhere/b.md")),
            Err(PathError::NotUnderRoot)
        );
    }

    #[test]
    fn a_mount_is_pushed_at_the_end_removed_and_relocated_by_name_key_and_a_taken_name_is_refused()
    {
        let mut mounts = Mounts::new(vec![mount("team", "/t")]).unwrap();
        assert_eq!(mounts.push(mount("other", "/o")), Ok(()));
        assert_eq!(
            mounts.push(mount("team", "/x")),
            Err(NameTaken("team".to_owned()))
        );
        assert_eq!(
            mounts,
            Mounts::new(vec![mount("team", "/t"), mount("other", "/o")]).unwrap(),
            "pushed at the end; a taken name changes nothing"
        );
        assert!(mounts.relocate(&name("team"), PathBuf::from("/moved")));
        assert!(!mounts.relocate(&name("nope"), PathBuf::from("/x")));
        assert_eq!(
            mounts,
            Mounts::new(vec![mount("team", "/moved"), mount("other", "/o")]).unwrap(),
            "the name and its place in the list stay"
        );
        assert_eq!(mounts.remove(&name("nope")), None);
        assert_eq!(mounts.remove(&name("other")), Some(mount("other", "/o")));
        assert_eq!(mounts, Mounts::new(vec![mount("team", "/moved")]).unwrap());
        if folds_case() {
            assert!(mounts.relocate(&name("TEAM"), PathBuf::from("/again")));
            assert_eq!(mounts.remove(&name("Team")), Some(mount("team", "/again")));
        }
    }

    // ── overlap ──────────────────────────────────────────────────────────────────────────────────────────

    /// A fake disk: `links` maps a spelled folder to its canonical path (by prefix — `links["/j"] = "/real"` makes
    /// `/j/x` canonical `/real/x`); `ids` gives a folder's identity by its canonical path (none listed = no identity);
    /// `gone` folders (exactly these) give no canonical answer, `refused` ones get an OS error for it; `silent` folders'
    /// identities give no answer; with `no_ids` no folder has an identity. `asked` records each batch, in order.
    #[derive(Default)]
    struct Fake {
        links: Vec<(PathBuf, PathBuf)>,
        ids: HashMap<PathBuf, u32>,
        gone: Vec<PathBuf>,
        refused: Vec<PathBuf>,
        silent: Vec<PathBuf>,
        no_ids: bool,
        asked: std::cell::RefCell<Vec<(&'static str, Vec<PathBuf>)>>,
    }

    impl FolderProbe for Fake {
        type Id = u32;

        fn canonical_all(&self, folders: &[PathBuf]) -> Vec<Result<PathBuf, NoCanonical>> {
            self.asked
                .borrow_mut()
                .push(("canonical", folders.to_vec()));
            folders
                .iter()
                .map(|folder| {
                    if self.gone.contains(folder) {
                        return Err(NoCanonical::Unanswered("timed out".to_owned()));
                    }
                    if self.refused.contains(folder) {
                        return Err(NoCanonical::Os("access denied".to_owned()));
                    }
                    for (link, target) in &self.links {
                        if let Ok(rest) = folder.strip_prefix(link) {
                            return Ok(target.join(rest));
                        }
                    }
                    Ok(folder.clone())
                })
                .collect()
        }

        fn identity_all(&self, folders: &[PathBuf]) -> Vec<Result<Option<u32>, String>> {
            self.asked.borrow_mut().push(("identity", folders.to_vec()));
            folders
                .iter()
                .map(|folder| {
                    if self.silent.contains(folder) {
                        return Err("timed out".to_owned());
                    }
                    if self.no_ids {
                        return Ok(None);
                    }
                    Ok(self.ids.get(folder).copied())
                })
                .collect()
        }
    }

    fn judged(folder: &str, mounts: &[Mount], fake: &Fake) -> Option<MountName> {
        overlap(Path::new(folder), false, mounts, fake).expect("the candidate is judged")
    }

    #[test]
    fn a_folder_overlaps_a_mount_it_equals_holds_or_lies_inside_and_spellings_differing_in_case_are_one_where_the_key_folds(
    ) {
        let mounts = [mount("team", "/m/team"), mount("other", "/m/other")];
        let fake = Fake::default();
        let team = Some(name("team"));
        assert_eq!(judged("/m/team", &mounts, &fake), team, "the same folder");
        assert_eq!(judged("/m/team/sub", &mounts, &fake), team, "inside");
        assert_eq!(
            judged("/m", &mounts, &fake),
            team,
            "holding: the first in order"
        );
        assert_eq!(
            judged("/m/teams", &mounts, &fake),
            None,
            "a name prefix is not a folder prefix"
        );
        assert_eq!(judged("/elsewhere", &mounts, &fake), None);
        assert_eq!(
            judged("/M/Team", &mounts, &fake),
            folds_case().then_some(name("team")),
            "compared by the platform path key"
        );
    }

    #[test]
    fn a_link_to_a_mount_overlaps_it_by_its_canonical_path() {
        let mounts = [mount("team", "/m/team")];
        let fake = Fake {
            links: vec![(PathBuf::from("/j"), PathBuf::from("/m/team"))],
            ..Fake::default()
        };
        let team = Some(name("team"));
        assert_eq!(judged("/j", &mounts, &fake), team);
        assert_eq!(judged("/j/sub", &mounts, &fake), team);
        let up = Fake {
            links: vec![(PathBuf::from("/j"), PathBuf::from("/m"))],
            ..Fake::default()
        };
        assert_eq!(
            judged("/j", &mounts, &up),
            team,
            "a link to a mount's parent holds it"
        );
    }

    #[test]
    fn identities_catch_one_folder_under_two_canonical_spellings_and_unsupported_falls_back_to_paths(
    ) {
        // `/share/x` and `/z/x` are one folder whose two spellings canonicalize apart (a mapped drive and its share).
        let mounts = [mount("team", "/share/x")];
        let ids = HashMap::from([
            (PathBuf::from("/share/x"), 7),
            (PathBuf::from("/z/x"), 7),
            (PathBuf::from("/z/x/sub"), 8),
        ]);
        let fake = Fake {
            ids: ids.clone(),
            ..Fake::default()
        };
        let team = Some(name("team"));
        assert_eq!(judged("/z/x", &mounts, &fake), team, "the same identity");
        assert_eq!(
            judged("/z/x/sub", &mounts, &fake),
            team,
            "an ancestor's identity"
        );
        let parent_ids = Fake {
            ids: HashMap::from([(PathBuf::from("/share"), 3), (PathBuf::from("/z"), 3)]),
            ..Fake::default()
        };
        assert_eq!(
            judged("/z", &mounts, &parent_ids),
            team,
            "the folder's identity is an ancestor of the mount"
        );
        let unsupported = Fake {
            ids,
            no_ids: true,
            ..Fake::default()
        };
        assert_eq!(
            judged("/z/x", &mounts, &unsupported),
            None,
            "no identity: paths alone"
        );
        assert_eq!(
            judged("/share/x/sub", &mounts, &unsupported),
            team,
            "paths still judge"
        );
    }

    // ADR-0400 decision 10: a folder being added or relocated that cannot be judged is refused, never taken as free.
    #[test]
    fn a_candidate_whose_canonical_path_or_any_identity_does_not_answer_is_not_judged_free() {
        let mounts = [mount("team", "/share/x")];
        let gone = Fake {
            gone: vec![PathBuf::from("/z/x")],
            ..Fake::default()
        };
        let why = overlap(Path::new("/z/x"), false, &mounts, &gone).unwrap_err();
        assert!(
            why.contains("canonical") && why.contains("timed out"),
            "{why}"
        );
        let silent_ancestor = Fake {
            silent: vec![PathBuf::from("/z")],
            ..Fake::default()
        };
        let why = overlap(Path::new("/z/x"), false, &mounts, &silent_ancestor).unwrap_err();
        assert!(
            why.contains("identity") && why.contains("timed out"),
            "{why}"
        );
        assert!(
            overlap(Path::new("/z/x"), false, &[], &silent_ancestor).is_err(),
            "refused with no mount to judge against too"
        );
    }

    // ADR-0400 decision 10: a known side that gives no answer cannot be judged free; an OS refusal is judged on spelling.
    #[test]
    fn a_mount_the_os_refuses_is_judged_on_its_spelling_and_one_that_does_not_answer_refuses_the_candidate(
    ) {
        let mounts = [mount("team", "/m/team")];
        let offline = Fake {
            refused: vec![PathBuf::from("/m/team")],
            ids: HashMap::from([(PathBuf::from("/m/team"), 1), (PathBuf::from("/other"), 1)]),
            ..Fake::default()
        };
        assert_eq!(
            judged("/m/team/sub", &mounts, &offline),
            Some(name("team")),
            "the offline mount's spelling"
        );
        assert_eq!(
            judged("/other", &mounts, &offline),
            None,
            "the offline mount's identity is not asked"
        );
        for (fake, asked) in [
            (
                Fake {
                    gone: vec![PathBuf::from("/m/team")],
                    ..Fake::default()
                },
                "canonical",
            ),
            (
                Fake {
                    silent: vec![PathBuf::from("/m/team")],
                    ..Fake::default()
                },
                "identity",
            ),
        ] {
            let why = overlap(Path::new("/other"), false, &mounts, &fake).unwrap_err();
            assert!(
                why.contains("\"team\"") && why.contains(asked) && why.contains("timed out"),
                "{why}"
            );
        }
    }

    // ADR-0400 decision 10 (as amended): only mounts are judged — the daemon's own folders are no side — and one
    //   judgement asks one canonical batch and one identity batch.
    #[test]
    fn only_mounts_are_judged_in_one_canonical_and_one_identity_batch() {
        let mounts = [mount("team", "/x"), mount("other", "/o")];
        let fake = Fake::default();
        assert_eq!(judged("/elsewhere", &mounts, &fake), None);
        let asked = fake.asked.take();
        let kinds: Vec<&str> = asked.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(kinds, ["canonical", "identity"]);
        assert_eq!(
            asked[0].1,
            [
                PathBuf::from("/elsewhere"),
                PathBuf::from("/x"),
                PathBuf::from("/o")
            ]
        );
        assert_eq!(
            asked[1].1.len(),
            4,
            "each ancestor once — `/elsewhere`, `/`, `/x`, `/o`: {:?}",
            asked[1].1
        );
        for folder in [
            "/data",
            "/data/daemon",
            "/data/launch",
            "/dist/builtin",
            "/",
        ] {
            assert_eq!(
                judged(folder, &[], &fake),
                None,
                "{folder}: nothing but a mount is a side"
            );
        }
    }

    // ADR-0400 decision 10: a folder a `create` will make is judged before it exists — by its nearest ancestor whose
    //   canonical path the OS gives, with the names below it spelled; an ancestor nearer than that which gives no
    //   answer, or no ancestor at all, refuses it.
    #[test]
    fn a_folder_that_does_not_exist_yet_is_judged_by_its_nearest_ancestor_with_a_canonical_path() {
        let mounts = [mount("team", "/m/team")];
        let fake = Fake {
            links: vec![(PathBuf::from("/j"), PathBuf::from("/m"))],
            refused: vec![
                PathBuf::from("/j/team/new"),
                PathBuf::from("/j/other/new"),
                PathBuf::from("/j/other"),
            ],
            ..Fake::default()
        };
        let missing = |folder: &str, fake: &Fake| overlap(Path::new(folder), true, &mounts, fake);
        assert_eq!(
            missing("/j/team/new", &fake),
            Ok(Some(name("team"))),
            "`/j/team` canonicalizes to `/m/team`; `new` below it lies inside the mount"
        );
        assert_eq!(
            missing("/j/other/new", &fake),
            Ok(None),
            "`/m` + `other/new`"
        );
        assert!(
            overlap(Path::new("/j/team/new"), false, &mounts, &fake).is_err(),
            "without `create` a folder must canonicalize itself"
        );
        let silent = Fake {
            refused: vec![PathBuf::from("/j/new")],
            gone: vec![PathBuf::from("/j")],
            ..Fake::default()
        };
        let why = missing("/j/new", &silent).unwrap_err();
        assert!(
            why.contains("timed out"),
            "a nearer ancestor that gives no answer: {why}"
        );
    }

    #[test]
    fn saved_mounts_overlapping_an_earlier_one_are_named_with_it_and_nothing_fails_closed() {
        let mounts = Mounts::new(vec![
            mount("a", "/m/a"),
            mount("inner", "/m/a/sub"),
            mount("b", "/m/b"),
            mount("outer", "/m"),
            mount("gone", "/g"),
            mount("under-gone", "/g/x"),
        ])
        .unwrap();
        let fake = Fake {
            gone: vec![PathBuf::from("/g")],
            silent: vec![PathBuf::from("/m/b")],
            ..Fake::default()
        };
        assert_eq!(
            overlapping(&mounts, &fake),
            vec![
                (name("inner"), name("a")),
                (name("outer"), name("a")),
                (name("under-gone"), name("gone")),
            ],
            "the first earlier mount; an unanswered side is judged on its spelling"
        );
        assert!(overlapping(&Mounts::default(), &fake).is_empty());
    }

    // ADR-0400 decision 10: at load an identity that gives no answer is unknown, not a reason to drop the canonical
    //   path — a junction alias of an earlier mount is still found by it.
    #[test]
    fn at_load_an_identity_without_an_answer_leaves_the_canonical_path_to_judge() {
        let mounts = Mounts::new(vec![mount("a", "/m/a"), mount("alias", "/j")]).unwrap();
        let fake = Fake {
            links: vec![(PathBuf::from("/j"), PathBuf::from("/m/a"))],
            silent: vec![PathBuf::from("/m")],
            ..Fake::default()
        };
        assert_eq!(
            overlapping(&mounts, &fake),
            vec![(name("alias"), name("a"))],
            "both canonical paths are `/m/a`; the silent ancestor `/m` only leaves its identity unknown"
        );
    }

    #[test]
    fn a_relocate_into_its_own_subfolder_is_judged_against_every_other_mount_only() {
        let mounts = [mount("library", "/lib"), mount("team", "/t")];
        let fake = Fake::default();
        let others: Vec<&Mount> = mounts
            .iter()
            .filter(|m| m.name.as_str() != "library")
            .collect();
        let relocated =
            |folder: &str| overlap(Path::new(folder), false, others.iter().copied(), &fake);
        assert_eq!(relocated("/lib/sub"), Ok(None));
        assert_eq!(relocated("/t/sub"), Ok(Some(name("team"))));
        assert_eq!(
            overlap(Path::new("/lib/sub"), false, &mounts, &fake),
            Ok(Some(name("library"))),
            "judged with itself it would overlap"
        );
    }

    struct LinkGuard(PathBuf);

    impl Drop for LinkGuard {
        fn drop(&mut self) {
            if let Err(e) = unlink_dir(&self.0) {
                eprintln!("could not remove the test link {}: {e}", self.0.display());
            }
        }
    }

    #[test]
    fn a_real_junction_to_a_mount_overlaps_it_through_the_disk_probe() {
        let temp = tempfile::tempdir().unwrap();
        let base = normalize_folder(temp.path()).unwrap();
        let real = base.join("real");
        std::fs::create_dir_all(real.join("sub")).unwrap();
        std::fs::create_dir_all(base.join("apart")).unwrap();
        let link = base.join("link");
        link_dir(&real, &link).unwrap();
        let _guard = LinkGuard(link.clone());
        let io = BoundedIo::new(
            IO_LIMIT,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::new(StdFolderOps),
            Arc::new(ThreadSpawn),
        );
        let read = io.begin_read();
        let probe = DiskProbe {
            io: &io,
            read: &read,
        };
        let mounts = [Mount {
            name: name("team"),
            folder: real.clone(),
        }];
        let team = Ok(Some(name("team")));
        assert_eq!(overlap(&link, false, &mounts, &probe), team);
        assert_eq!(overlap(&link.join("sub"), false, &mounts, &probe), team);
        assert_eq!(
            overlap(&base.join("apart"), false, &mounts, &probe),
            Ok(None)
        );
        assert!(
            overlap(&base.join("missing"), false, &mounts, &probe).is_err(),
            "a candidate that does not canonicalize is refused"
        );
        assert_eq!(
            overlap(&link.join("new").join("deeper"), true, &mounts, &probe),
            team,
            "a folder to be created, judged through the junction above it"
        );
        assert_eq!(
            overlap(&base.join("new").join("deeper"), true, &mounts, &probe),
            Ok(None)
        );
    }
}
