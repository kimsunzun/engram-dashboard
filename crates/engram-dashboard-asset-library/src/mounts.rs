//! The mounts of the virtual library root: the mount table, the mount name rules, the overlap check, and the only
//! converter between the two path types.
//!
//! - **Two path types.** [`LibPath`] is a virtual path from the virtual root, `<mount>/…` — what the catalog,
//!   references, the wire and the resolver use; a path of one name names a mount. [`MountPath`] is a path inside one
//!   mount's folder — what the store is to take and return (the store still takes the root until the service reads
//!   mounts). Only this module turns one into the other
//!   ([`Mounts::locate`] · [`virtual_path`]), so the store stays mount-unaware and a missed prefix or strip does not
//!   compile.
//! - **Names** ([`MountName`]): one `LibPath` name, no leading `.`, at most [`NAME_MAX_CHARS`] characters, not the
//!   built-in's [`BUILTIN_MOUNT`] (matched ASCII case-insensitively on every OS); two names are the same mount when their
//!   platform path keys are equal. A name need not equal anything on disk.
//! - **Overlap** ([`overlap`]): judged on canonical final paths (links and junctions resolved) by path key, component
//!   by component, and on the file identities of the folders and their ancestors where the volume gives one, every ask
//!   bounded. The folder being added or relocated fails closed: when its canonical path fails (no answer or an OS error)
//!   or one of its identities gives no answer it is refused, never taken as free. A mount or reserved folder whose canonical path cannot be had is judged on its
//!   spelling, and an identity that gives no answer is unknown. Known escapes: SMB servers that give no file id, one
//!   share under several host spellings.
//! - **The service still reads one root.** Until virtual paths are threaded through the service, it reads the first
//!   mount's folder as its root ([`Mounts::root`]) and a root change relocates the first mount
//!   ([`Mounts::set_root`]).
// ADR-0396
// ADR-0400

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use engram_dashboard_platform::fs::{path_key, FileIdentity, PathKey};

use crate::folder::{BoundedIo, LaunchIo, Need};
use crate::path::{LibPath, PathError};

/// The reserved name of the built-in mount — never registered, never stored.
// ADR-0400
pub const BUILTIN_MOUNT: &str = "engram";

/// The name of the mount the first start registers at the default place.
// ADR-0400 decision 3
pub const FIRST_MOUNT: &str = "library";

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

    /// The folder the service reads as its one root: the first mount's; `None` = no mount is registered.
    pub(crate) fn root(&self) -> Option<&Path> {
        self.0.first().map(|mount| mount.folder.as_path())
    }

    /// The service's one-root change: the first mount now maps to `folder`; with no mount registered, `folder` is
    /// registered as [`FIRST_MOUNT`].
    pub(crate) fn set_root(&mut self, folder: PathBuf) {
        match self.0.first_mut() {
            Some(first) => first.folder = folder,
            None => self.0.push(Mount {
                name: MountName::first(),
                folder,
            }),
        }
    }
}

/// A path inside one mount's folder, relative to it — the store's path type. Made from a virtual path only by
/// [`Mounts::locate`], and from disk by [`MountPath::from_disk`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MountPath(LibPath);

impl MountPath {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The disk path of this path under the mount folder `folder` ([`LibPath::to_path`]).
    pub fn to_path(&self, folder: &Path) -> PathBuf {
        self.0.to_path(folder)
    }

    /// The path of `path`, a disk path under the mount folder `folder` as a scan builds it ([`LibPath::from_disk`]).
    pub fn from_disk(folder: &Path, path: &Path) -> Result<Self, PathError> {
        LibPath::from_disk(folder, path).map(Self)
    }
}

/// The virtual path of `rest` inside the mount `name`, or of the mount itself when `rest` is `None`.
pub fn virtual_path(name: &MountName, rest: Option<&MountPath>) -> LibPath {
    let text = match rest {
        Some(rest) => format!("{}/{}", name.as_str(), rest.as_str()),
        None => name.as_str().to_owned(),
    };
    LibPath::parse(&text).expect("a mount name followed by a valid path is a valid path")
}

/// The folders a mount may not overlap besides the other mounts (ADR-0400 decision 10).
pub struct Reserved<'a> {
    /// The data folder — every part of it (`daemon\`, `shell\`, `logs\` …).
    pub data_root: &'a Path,
    /// The default place: only this folder itself is exempt from the data-root rule.
    pub default_place: &'a Path,
    pub launch_root: &'a Path,
    /// `None` = there is no built-in folder.
    pub builtin_dir: Option<&'a Path>,
}

/// What a folder overlaps — the `with` of a `mount-overlaps` refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overlap {
    Mount(MountName),
    DataRoot,
    LaunchFolder,
    BuiltinFolder,
}

/// What [`overlap`] asks of the disk. Production = [`DiskProbe`].
pub trait FolderProbe {
    type Id: PartialEq;
    /// The canonical final path of `folder`, links and junctions resolved; `Err(why)` = none could be had.
    fn canonical(&self, folder: &Path) -> Result<PathBuf, String>;
    /// The file identity of `folder`: `Ok(None)` = it has none to compare (`Unsupported`, or the OS refused);
    /// `Err(why)` = the ask gave no answer (timed out, not checked, a failed worker).
    fn identity(&self, folder: &Path) -> Result<Option<Self::Id>, String>;
}

/// The production probe: the canonical path and the identity through the bounded IO, as required asks on `read` —
/// so every ask of one judgement shares that read's budget.
pub struct DiskProbe<'a> {
    pub io: &'a BoundedIo,
    pub read: &'a LaunchIo,
}

impl FolderProbe for DiskProbe<'_> {
    type Id = FileIdentity;

    fn canonical(&self, folder: &Path) -> Result<PathBuf, String> {
        match self
            .io
            .canonical_all(self.read, &[(folder.to_path_buf(), Need::Required)])
            .remove(&path_key(folder))
        {
            Some(Ok(canonical)) => Ok(canonical),
            Some(Err(status)) => Err(status.why()),
            None => Err("the bounded canonicalize gave no answer".to_owned()),
        }
    }

    fn identity(&self, folder: &Path) -> Result<Option<FileIdentity>, String> {
        match self
            .io
            .identity_all(self.read, &[(folder.to_path_buf(), Need::Required)])
            .remove(&path_key(folder))
        {
            Some(Ok(identity)) => Ok(identity),
            Some(Err(status)) => Err(status.why()),
            None => Err("the bounded identity gave no answer".to_owned()),
        }
    }
}

/// What `folder` — a folder being added or relocated — overlaps, if anything: the first of `mounts` (in order) that is
/// the same folder, holds it or lies inside it; then the data folder (unless `folder` is the default place itself),
/// the launch folder and the built-in folder. `mounts` = every mount to judge against — a relocate leaves the mount
/// itself out (module header for how folders compare).
///
/// `Err(why)` = `folder` itself could not be judged — its canonical path, or the identity of it or of an ancestor, gave
/// no answer: the caller refuses it as a folder that cannot be used (`library-root-unusable`), never as free. A mount or
/// reserved folder that cannot be had is judged on its spelling (an offline mount) and an identity that does not answer
/// counts as unknown.
// ADR-0400 decision 10
pub fn overlap<'m, P: FolderProbe>(
    folder: &Path,
    mounts: impl IntoIterator<Item = &'m Mount>,
    reserved: &Reserved,
    probe: &P,
) -> Result<Option<Overlap>, String> {
    let it = Place::candidate(folder, probe)?;
    for mount in mounts {
        if it.overlaps(&Place::known(&mount.folder, probe)) {
            return Ok(Some(Overlap::Mount(mount.name.clone())));
        }
    }
    if it.overlaps(&Place::known(reserved.data_root, probe))
        && !it.is(&Place::known(reserved.default_place, probe))
    {
        return Ok(Some(Overlap::DataRoot));
    }
    if it.overlaps(&Place::known(reserved.launch_root, probe)) {
        return Ok(Some(Overlap::LaunchFolder));
    }
    if let Some(builtin) = reserved.builtin_dir {
        if it.overlaps(&Place::known(builtin, probe)) {
            return Ok(Some(Overlap::BuiltinFolder));
        }
    }
    Ok(None)
}

/// One folder as [`overlap`] compares it.
struct Place<I> {
    /// The path key of each component of the canonical path (or of the spelling).
    names: Vec<PathKey>,
    /// The identity of the folder and of each ancestor, the folder first (`None` = unknown); empty when the canonical
    /// path did not answer.
    ids: Vec<Option<I>>,
}

impl<I: PartialEq> Place<I> {
    /// The folder being judged: fails closed when its canonical path fails (no answer or an OS error) or any identity
    /// gives no answer.
    fn candidate<P: FolderProbe<Id = I>>(folder: &Path, probe: &P) -> Result<Self, String> {
        let canonical = probe
            .canonical(folder)
            .map_err(|why| format!("its canonical path could not be had: {why}"))?;
        let ids = canonical
            .ancestors()
            .map(|at| {
                probe.identity(at).map_err(|why| {
                    format!("the identity of {} gave no answer: {why}", at.display())
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Place {
            names: names_of(&canonical),
            ids,
        })
    }

    /// A folder judged against: its spelling when the canonical path cannot be had, unknown identities skipped.
    fn known<P: FolderProbe<Id = I>>(folder: &Path, probe: &P) -> Self {
        match probe.canonical(folder) {
            Ok(canonical) => Place {
                names: names_of(&canonical),
                ids: canonical
                    .ancestors()
                    .map(|at| probe.identity(at).ok().flatten())
                    .collect(),
            },
            Err(_) => Place {
                names: names_of(folder),
                ids: Vec::new(),
            },
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

    /// `self` is the same folder as `other`.
    fn is(&self, other: &Place<I>) -> bool {
        if self.names == other.names {
            return true;
        }
        matches!((self.ids.first(), other.ids.first()), (Some(Some(a)), Some(Some(b))) if a == b)
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
    fn the_one_root_view_is_the_first_mount_and_a_root_change_relocates_it_or_registers_library() {
        let mut mounts = Mounts::default();
        assert_eq!(mounts.root(), None);
        mounts.set_root(PathBuf::from("/new"));
        assert_eq!(mounts, Mounts::new(vec![mount("library", "/new")]).unwrap());
        let mut two = Mounts::new(vec![mount("team", "/t"), mount("other", "/o")]).unwrap();
        two.set_root(PathBuf::from("/moved"));
        assert_eq!(two.root(), Some(Path::new("/moved")));
        assert_eq!(
            two,
            Mounts::new(vec![mount("team", "/moved"), mount("other", "/o")]).unwrap(),
            "the name and the other mounts stay"
        );
    }

    // ── overlap ──────────────────────────────────────────────────────────────────────────────────────────

    /// A fake disk: `links` maps a spelled folder to its canonical path (by prefix — `links["/j"] = "/real"` makes
    /// `/j/x` canonical `/real/x`); `ids` gives a folder's identity by its canonical path (none listed = no identity);
    /// `gone` folders (exactly these) do not canonicalize; `silent` folders' identities give no answer; with `no_ids` no folder has an
    /// identity.
    #[derive(Default)]
    struct Fake {
        links: Vec<(PathBuf, PathBuf)>,
        ids: HashMap<PathBuf, u32>,
        gone: Vec<PathBuf>,
        silent: Vec<PathBuf>,
        no_ids: bool,
    }

    impl FolderProbe for Fake {
        type Id = u32;

        fn canonical(&self, folder: &Path) -> Result<PathBuf, String> {
            if self.gone.iter().any(|gone| gone == folder) {
                return Err("timed out".to_owned());
            }
            for (link, target) in &self.links {
                if let Ok(rest) = folder.strip_prefix(link) {
                    return Ok(target.join(rest));
                }
            }
            Ok(folder.to_path_buf())
        }

        fn identity(&self, folder: &Path) -> Result<Option<u32>, String> {
            if self.silent.iter().any(|silent| silent == folder) {
                return Err("timed out".to_owned());
            }
            if self.no_ids {
                return Ok(None);
            }
            Ok(self.ids.get(folder).copied())
        }
    }

    fn reserved() -> Reserved<'static> {
        Reserved {
            data_root: Path::new("/data"),
            default_place: Path::new("/data/library"),
            launch_root: Path::new("/data/launch"),
            builtin_dir: Some(Path::new("/dist/builtin")),
        }
    }

    fn judged(folder: &str, mounts: &[Mount], fake: &Fake) -> Option<Overlap> {
        overlap(Path::new(folder), mounts, &reserved(), fake).expect("the candidate is judged")
    }

    #[test]
    fn a_folder_overlaps_a_mount_it_equals_holds_or_lies_inside_and_spellings_differing_in_case_are_one_where_the_key_folds(
    ) {
        let mounts = [mount("team", "/m/team"), mount("other", "/m/other")];
        let fake = Fake::default();
        let team = Some(Overlap::Mount(name("team")));
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
            folds_case().then_some(Overlap::Mount(name("team"))),
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
        let team = Some(Overlap::Mount(name("team")));
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
        let team = Some(Overlap::Mount(name("team")));
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
        let why = overlap(Path::new("/z/x"), &mounts, &reserved(), &gone).unwrap_err();
        assert!(
            why.contains("canonical") && why.contains("timed out"),
            "{why}"
        );
        let silent_ancestor = Fake {
            silent: vec![PathBuf::from("/z")],
            ..Fake::default()
        };
        let why = overlap(Path::new("/z/x"), &mounts, &reserved(), &silent_ancestor).unwrap_err();
        assert!(
            why.contains("identity") && why.contains("timed out"),
            "{why}"
        );
        assert!(
            overlap(Path::new("/z/x"), &[], &reserved(), &silent_ancestor).is_err(),
            "refused with no mount to judge against too"
        );
    }

    #[test]
    fn an_offline_mount_is_judged_on_its_spelling_and_an_identity_that_does_not_answer_is_unknown()
    {
        let mounts = [mount("team", "/m/team")];
        let offline = Fake {
            gone: vec![PathBuf::from("/m/team")],
            ids: HashMap::from([(PathBuf::from("/m/team"), 1), (PathBuf::from("/other"), 1)]),
            ..Fake::default()
        };
        assert_eq!(
            judged("/m/team/sub", &mounts, &offline),
            Some(Overlap::Mount(name("team"))),
            "the offline mount's spelling"
        );
        assert_eq!(
            judged("/other", &mounts, &offline),
            None,
            "the offline mount's identity is not asked"
        );
        let silent_mount = Fake {
            ids: HashMap::from([(PathBuf::from("/m/team"), 1), (PathBuf::from("/other"), 1)]),
            silent: vec![PathBuf::from("/m/team")],
            ..Fake::default()
        };
        assert_eq!(
            judged("/other", &mounts, &silent_mount),
            None,
            "the mount's silent identity is unknown, not a match"
        );
        assert_eq!(
            judged("/m", &mounts, &silent_mount),
            Some(Overlap::Mount(name("team"))),
            "its path still judges a folder holding it"
        );
    }

    // ADR-0400 decision 10: only the default place itself is exempt from the data-root rule.
    #[test]
    fn the_data_launch_and_built_in_folders_are_refused_but_the_default_place_itself_is_not() {
        let fake = Fake::default();
        for (folder, want) in [
            ("/data", Some(Overlap::DataRoot)),
            ("/data/daemon", Some(Overlap::DataRoot)),
            ("/data/logs/x", Some(Overlap::DataRoot)),
            ("/", Some(Overlap::DataRoot)),
            ("/data/library", None),
            ("/data/library/sub", Some(Overlap::DataRoot)),
            ("/data/launch", Some(Overlap::DataRoot)),
            ("/dist/builtin", Some(Overlap::BuiltinFolder)),
            ("/dist/builtin/presets", Some(Overlap::BuiltinFolder)),
            ("/dist", Some(Overlap::BuiltinFolder)),
            ("/dist/library", None),
        ] {
            assert_eq!(judged(folder, &[], &fake), want, "{folder}");
        }
        // `/z` is the data folder and `/z/lib` the default place under other spellings, told by their identities.
        let same_by_id = Fake {
            ids: HashMap::from([
                (PathBuf::from("/data"), 9),
                (PathBuf::from("/z"), 9),
                (PathBuf::from("/data/library"), 5),
                (PathBuf::from("/z/lib"), 5),
            ]),
            ..Fake::default()
        };
        assert_eq!(
            judged("/z/lib", &[], &same_by_id),
            None,
            "the default place itself"
        );
        assert_eq!(
            judged("/z/other", &[], &same_by_id),
            Some(Overlap::DataRoot),
            "inside the data folder by identity"
        );
        let release = Reserved {
            data_root: Path::new("/dist/data"),
            default_place: Path::new("/dist/library"),
            launch_root: Path::new("/tmp/launch"),
            builtin_dir: None,
        };
        let at = |folder: &str| overlap(Path::new(folder), &[], &release, &fake).unwrap();
        assert_eq!(at("/tmp/launch/a"), Some(Overlap::LaunchFolder));
        assert_eq!(at("/tmp"), Some(Overlap::LaunchFolder));
        assert_eq!(
            at("/dist/library"),
            None,
            "a default place beside the data folder"
        );
        assert_eq!(at("/dist/library/sub"), None, "outside the data folder");
        assert_eq!(at("/dist"), Some(Overlap::DataRoot));
    }

    #[test]
    fn a_relocate_into_its_own_subfolder_is_judged_against_every_other_mount_only() {
        let mounts = [mount("library", "/lib"), mount("team", "/t")];
        let fake = Fake::default();
        let others: Vec<&Mount> = mounts
            .iter()
            .filter(|m| m.name.as_str() != "library")
            .collect();
        let relocated = |folder: &str| {
            overlap(
                Path::new(folder),
                others.iter().copied(),
                &reserved(),
                &fake,
            )
        };
        assert_eq!(relocated("/lib/sub"), Ok(None));
        assert_eq!(relocated("/t/sub"), Ok(Some(Overlap::Mount(name("team")))));
        assert_eq!(
            relocated("/data/library/sub"),
            Ok(Some(Overlap::DataRoot)),
            "below the default place is inside the data folder"
        );
        assert_eq!(
            overlap(Path::new("/lib/sub"), &mounts, &reserved(), &fake),
            Ok(Some(Overlap::Mount(name("library")))),
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
        let elsewhere = base.join("elsewhere");
        let reserved = Reserved {
            data_root: &elsewhere,
            default_place: &elsewhere,
            launch_root: &elsewhere,
            builtin_dir: None,
        };
        let team = Ok(Some(Overlap::Mount(name("team"))));
        assert_eq!(overlap(&link, &mounts, &reserved, &probe), team);
        assert_eq!(overlap(&link.join("sub"), &mounts, &reserved, &probe), team);
        assert_eq!(
            overlap(&base.join("apart"), &mounts, &reserved, &probe),
            Ok(None)
        );
        assert!(
            overlap(&base.join("missing"), &mounts, &reserved, &probe).is_err(),
            "a candidate that does not canonicalize is refused"
        );
    }
}
