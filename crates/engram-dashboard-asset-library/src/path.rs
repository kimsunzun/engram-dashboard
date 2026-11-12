//! Reference spellings. A library asset is named by [`LibPath`] (a `/` path from the library root), a built-in asset by
//! `engram:/<id>` ([`BuiltinId`]); [`AssetRef`] is either. `LibPath` refuses `:`, so the two never mix. A preset's folder
//! entry is not an asset: [`FolderRef`] holds an absolute folder path as the file spells it and its normalized form.
//!
//! Equality on these types is spelling equality. Whether two library paths name the same file is the OS rule:
//! compare [`LibPath::key`]s.
// ADR-0315
// TRD S25 step-7 §3-3

use std::path::{Component, Path, PathBuf};

use engram_dashboard_platform::fs::{
    is_reserved_device_name, normalize_folder, path_key, NotAbsolute, PathKey,
};
use relative_path::RelativePathBuf;

const BUILTIN_SCHEME: &str = "engram:/";

/// Why a reference spelling was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("empty path")]
    Empty,
    #[error("path starts with '/'")]
    LeadingSlash,
    #[error("path ends with '/'")]
    TrailingSlash,
    #[error("empty name between two '/'")]
    EmptyName,
    #[error("'.' or '..' as a name")]
    DotName,
    #[error("a name ends in '.' or a space")]
    NameEndsInDotOrSpace,
    #[error("name {0:?} is a Windows reserved device name")]
    ReservedDeviceName(String),
    #[error("character {0:?} is not allowed")]
    ForbiddenChar(char),
    #[error("unknown built-in asset id {0:?} (after 'engram:/')")]
    UnknownBuiltin(String),
    #[error("path is not under the library root")]
    NotUnderRoot,
    #[error("path is not valid UTF-8")]
    NotUtf8,
}

/// A library reference: the path of an asset from the library root, `/`-separated. Valid by construction — UTF-8, at
/// least one name, no leading or trailing `/`, no empty name, no `.` or `..` name, no name ending in `.` or a space,
/// no Win32 reserved device name, no `\`, `:` or control character.
///
/// The trailing `.` / space rule and the reserved-name rule hold on every OS. Win32 trims a trailing `.` or space, so
/// `rules/a.md.` would open `rules\a.md` and `.. ` would name a different entry than the one spelled; a reserved device
/// name (platform `fs::is_reserved_device_name` — `rules/CON`, `nul.md`, `COM1.txt`, `COM¹.md`) would make
/// [`LibPath::to_path`] open a device. Both amend ADR-0315 decision 2 (main 2026-11-11, recorded in the step-7 doc
/// pass).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LibPath(RelativePathBuf);

impl LibPath {
    pub fn parse(text: &str) -> Result<Self, PathError> {
        if text.is_empty() {
            return Err(PathError::Empty);
        }
        if text.starts_with('/') {
            return Err(PathError::LeadingSlash);
        }
        if text.ends_with('/') {
            return Err(PathError::TrailingSlash);
        }
        if let Some(c) = text
            .chars()
            .find(|&c| c == '\\' || c == ':' || c.is_control())
        {
            return Err(PathError::ForbiddenChar(c));
        }
        for name in text.split('/') {
            match name {
                "" => return Err(PathError::EmptyName),
                "." | ".." => return Err(PathError::DotName),
                _ if name.ends_with(['.', ' ']) => return Err(PathError::NameEndsInDotOrSpace),
                _ if is_reserved_device_name(name) => {
                    return Err(PathError::ReservedDeviceName(name.to_owned()))
                }
                _ => {}
            }
        }
        Ok(Self(RelativePathBuf::from(text)))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The disk path of this asset under `root`: `root` with each name pushed in turn, so a root that ends in a
    /// separator (`D:\` · `\\nas\lib\` · `/`) gets no second one.
    pub fn to_path(&self, root: &Path) -> PathBuf {
        let mut path = root.to_path_buf();
        for name in self.names() {
            path.push(name);
        }
        path
    }

    /// The reference spelling of `path`, a disk path under `root` as the scan builds it (`root` joined with names).
    /// `path` must start with `root` as spelled — the prefix is compared component by component, not by the OS rule.
    /// `Err(NotUnderRoot)` = not below `root` (or through `..`); the names must pass [`LibPath::parse`] too, so a
    /// scanned name ending in `.` or a space is refused.
    pub fn from_disk(root: &Path, path: &Path) -> Result<Self, PathError> {
        let rest = path
            .strip_prefix(root)
            .map_err(|_| PathError::NotUnderRoot)?;
        let mut names = Vec::new();
        for component in rest.components() {
            match component {
                Component::Normal(name) => names.push(name.to_str().ok_or(PathError::NotUtf8)?),
                _ => return Err(PathError::NotUnderRoot),
            }
        }
        Self::parse(&names.join("/"))
    }

    /// The folder holding this asset; `None` = it sits directly under the root.
    pub fn parent(&self) -> Option<LibPath> {
        self.as_str()
            .rsplit_once('/')
            .map(|(parent, _)| Self(RelativePathBuf::from(parent)))
    }

    /// The last name.
    pub fn file_name(&self) -> &str {
        self.as_str()
            .rsplit_once('/')
            .map_or(self.as_str(), |(_, name)| name)
    }

    /// The platform path key of [`LibPath::to_path`] — two references name the same file when their keys are equal.
    /// Catalog lookups take `catalog::key` (the spelling from the root), not this key.
    pub fn key(&self, root: &Path) -> PathKey {
        path_key(&self.to_path(root))
    }

    /// This path with the leading names `from` replaced by `to` — a move of `from` to `to` (a file or a folder) carries
    /// this reference along. `None` = `self` is not `from` or below it. The leading names are matched name by name by
    /// the platform path key (on Windows `Team/rules/a.md` is below `team/rules`); the answer is `to` as spelled
    /// followed by the rest of `self` as spelled.
    pub fn replace_prefix(&self, from: &LibPath, to: &LibPath) -> Option<LibPath> {
        let mut names = self.names();
        for want in from.names() {
            let have = names.next()?;
            if path_key(Path::new(have)) != path_key(Path::new(want)) {
                return None;
            }
        }
        let mut moved = to.as_str().to_owned();
        for name in names {
            moved.push('/');
            moved.push_str(name);
        }
        Some(Self(RelativePathBuf::from(moved)))
    }

    fn names(&self) -> std::str::Split<'_, char> {
        self.as_str().split('/')
    }
}

/// A built-in asset — the closed set of `engram:/…` ids, backed by files of the distribution's `builtin\` folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BuiltinId {
    /// `engram:/default` — the built-in preset 「기본」.
    DefaultPreset,
    /// `engram:/priming` — the priming instruction.
    Priming,
    /// `engram:/settings/default` — the built-in settings.
    DefaultSettings,
}

impl BuiltinId {
    pub const ALL: [BuiltinId; 3] = [
        BuiltinId::DefaultPreset,
        BuiltinId::Priming,
        BuiltinId::DefaultSettings,
    ];

    /// The id after `engram:/`.
    pub fn id(self) -> &'static str {
        match self {
            BuiltinId::DefaultPreset => "default",
            BuiltinId::Priming => "priming",
            BuiltinId::DefaultSettings => "settings/default",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|builtin| builtin.id() == id)
    }
}

/// What a preset item or an attached preset points at.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AssetRef {
    Library(LibPath),
    BuiltIn(BuiltinId),
}

impl AssetRef {
    /// `engram:/<id>` = a built-in (an id outside [`BuiltinId`] = `UnknownBuiltin`); anything else = a [`LibPath`].
    pub fn parse(text: &str) -> Result<Self, PathError> {
        match text.strip_prefix(BUILTIN_SCHEME) {
            Some(id) => BuiltinId::from_id(id)
                .map(AssetRef::BuiltIn)
                .ok_or_else(|| PathError::UnknownBuiltin(id.to_owned())),
            None => LibPath::parse(text).map(AssetRef::Library),
        }
    }

    /// The text [`AssetRef::parse`] reads back to `self`.
    pub fn spelling(&self) -> String {
        match self {
            AssetRef::Library(path) => path.as_str().to_owned(),
            AssetRef::BuiltIn(builtin) => format!("{BUILTIN_SCHEME}{}", builtin.id()),
        }
    }
}

/// One entry of a preset's `instructions` · `settings` · `skills` · `presets` list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetItem {
    pub target: AssetRef,
    /// The backend tag, a word the [`crate::backends::Backends`] registry knows; `None` = common to every backend.
    pub backend: Option<String>,
    /// 1-based line of the entry in the file.
    pub line: u32,
    /// Where the item was read from; `None` = a new item (not read from the file).
    pub origin: Option<ItemOrigin>,
}

/// The position of an entry in its list in the file, counting every entry of that list (also those not modeled).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemOrigin {
    pub index: u32,
}

/// Why a folder entry's path cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FolderPathError {
    /// A relative path — a library reference spelling included. Folder entries are absolute folder paths.
    #[error("not an absolute folder path")]
    NotAbsolute,
}

impl From<NotAbsolute> for FolderPathError {
    fn from(_: NotAbsolute) -> Self {
        FolderPathError::NotAbsolute
    }
}

/// One entry of a preset's `folders` list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderRef {
    /// The path as the file spells it — a read never rewrites the file text.
    pub raw: String,
    /// `raw` through the platform folder normalizer — the spelling the folder is keyed by.
    pub normalized: Result<PathBuf, FolderPathError>,
    /// 1-based line of the entry in the file.
    pub line: u32,
    /// Where the entry was read from; `None` = a new entry.
    pub origin: Option<ItemOrigin>,
}

impl FolderRef {
    /// The entry for `raw` with `normalized` computed from it.
    pub fn new(raw: String, line: u32, origin: Option<ItemOrigin>) -> Self {
        let normalized = normalize_folder(Path::new(&raw)).map_err(FolderPathError::from);
        Self {
            raw,
            normalized,
            line,
            origin,
        }
    }
}

#[cfg(test)]
mod tests {
    use engram_dashboard_platform::fs::same_path;

    use super::*;

    fn lib(text: &str) -> LibPath {
        LibPath::parse(text).expect(text)
    }

    fn reserved(name: &str) -> PathError {
        PathError::ReservedDeviceName(name.to_owned())
    }

    // TRD S25 step-7 §8-1 P1 path row — the refusal table.
    #[test]
    fn lib_path_refuses_each_rule() {
        let cases: &[(&str, PathError)] = &[
            ("", PathError::Empty),
            ("/a", PathError::LeadingSlash),
            ("/", PathError::LeadingSlash),
            ("a/", PathError::TrailingSlash),
            ("a//b", PathError::EmptyName),
            (".", PathError::DotName),
            ("..", PathError::DotName),
            ("a/./b", PathError::DotName),
            ("a/../b", PathError::DotName),
            ("a/..", PathError::DotName),
            ("a.md.", PathError::NameEndsInDotOrSpace),
            ("a.md ", PathError::NameEndsInDotOrSpace),
            ("x./a.md", PathError::NameEndsInDotOrSpace),
            ("x /a.md", PathError::NameEndsInDotOrSpace),
            (".. ", PathError::NameEndsInDotOrSpace),
            ("...", PathError::NameEndsInDotOrSpace),
            ("a/...", PathError::NameEndsInDotOrSpace),
            ("rules/CON", reserved("CON")),
            ("nul.md", reserved("nul.md")),
            ("COM1.txt", reserved("COM1.txt")),
            ("x/Com1 .tar.gz", reserved("Com1 .tar.gz")),
            ("COM¹.md", reserved("COM¹.md")),
            ("lpt³/a.md", reserved("lpt³")),
            ("aux.d/x.md", reserved("aux.d")),
            ("a\\b", PathError::ForbiddenChar('\\')),
            ("D:/a", PathError::ForbiddenChar(':')),
            ("engram:/default", PathError::ForbiddenChar(':')),
            ("a\nb", PathError::ForbiddenChar('\n')),
            ("a\tb", PathError::ForbiddenChar('\t')),
            ("a\u{7f}b", PathError::ForbiddenChar('\u{7f}')),
            ("a\u{85}b", PathError::ForbiddenChar('\u{85}')),
        ];
        for (text, want) in cases {
            assert_eq!(LibPath::parse(text), Err(want.clone()), "{text:?}");
        }
    }

    #[test]
    fn lib_path_accepts_names_that_only_look_odd() {
        for text in [
            "a",
            "업무/규칙.md",
            "a/b/c/SKILL.md",
            ".hidden/x.md",
            "a/.x",
            "a/..x",
            " lead/x.md",
            "with space/x y.md",
            "CONSOLE/x.md",
            "rules/nulx.md",
            "com10.txt",
            "COM⁴.md",
            "x.con",
        ] {
            assert_eq!(lib(text).as_str(), text);
        }
    }

    #[test]
    fn parent_and_file_name_split_at_the_last_slash() {
        let path = lib("a/b/c.md");
        assert_eq!(path.file_name(), "c.md");
        assert_eq!(path.parent(), Some(lib("a/b")));
        assert_eq!(lib("a/b").parent(), Some(lib("a")));
        assert_eq!(lib("c.md").parent(), None);
        assert_eq!(lib("c.md").file_name(), "c.md");
    }

    // Review round 1 must 1: a root that ends in a separator gets no second one.
    #[test]
    fn to_path_under_a_root_that_ends_in_a_separator_matches_join() {
        let root = std::env::temp_dir();
        let root = root.ancestors().last().expect("a root");
        let joined = root.join("a").join("b.md");
        let path = lib("a/b.md");
        assert_eq!(path.to_path(root).as_os_str(), joined.as_os_str());
        assert_eq!(path.key(root), path_key(&joined));
        let back = LibPath::from_disk(root, &joined).expect("below the root");
        assert_eq!(back, path);
        assert_eq!(back.to_path(root).as_os_str(), joined.as_os_str());
    }

    #[test]
    fn to_path_and_from_disk_round_trip_under_a_root() {
        let root = Path::new("lib-root");
        for text in ["a.md", "업무/규칙.md", "a/b/c/SKILL.md"] {
            let disk = lib(text).to_path(root);
            assert!(disk.starts_with(root), "{disk:?}");
            let joined = text
                .split('/')
                .fold(root.to_path_buf(), |path, name| path.join(name));
            assert_eq!(disk.as_os_str(), joined.as_os_str(), "{text}");
            assert_eq!(LibPath::from_disk(root, &disk), Ok(lib(text)), "{text}");
        }
    }

    #[test]
    fn from_disk_refuses_what_is_not_a_reference_below_the_root() {
        let root = Path::new("lib-root");
        assert_eq!(
            LibPath::from_disk(root, Path::new("other/a.md")),
            Err(PathError::NotUnderRoot)
        );
        assert_eq!(
            LibPath::from_disk(root, &root.join("..").join("a.md")),
            Err(PathError::NotUnderRoot)
        );
        assert_eq!(LibPath::from_disk(root, root), Err(PathError::Empty));
        // On Windows `join` replaces the buffer (`a:` is a drive prefix), so the path is not under the root; elsewhere
        //   `:` is a name character the parse refuses.
        assert!(matches!(
            LibPath::from_disk(root, &root.join("a:b.md")),
            Err(PathError::NotUnderRoot | PathError::ForbiddenChar(':'))
        ));
        assert_eq!(
            LibPath::from_disk(root, &root.join("rules").join("a.md.")),
            Err(PathError::NameEndsInDotOrSpace)
        );
        assert_eq!(
            LibPath::from_disk(root, &root.join("nul.md")),
            Err(reserved("nul.md"))
        );
    }

    #[test]
    fn keys_follow_the_os_rule_and_equality_follows_the_spelling() {
        let root = Path::new("lib-root");
        let (lower, upper) = (lib("rules/a.md"), lib("Rules/A.md"));
        assert_ne!(lower, upper);
        assert_eq!(lower.key(root), path_key(&lower.to_path(root)));
        assert_eq!(
            lower.key(root) == upper.key(root),
            same_path(&lower.to_path(root), &upper.to_path(root))
        );
        assert_ne!(lower.key(root), lib("rules/b.md").key(root));
    }

    // TRD S25 step-7 §8-1 P1 path row — prefix swap.
    #[test]
    fn replace_prefix_moves_the_path_and_whatever_is_below_it() {
        let (from, to) = (lib("team/rules"), lib("archive/old rules"));
        assert_eq!(
            lib("team/rules/a.md").replace_prefix(&from, &to),
            Some(lib("archive/old rules/a.md"))
        );
        assert_eq!(
            lib("team/rules/x/SKILL.md").replace_prefix(&from, &to),
            Some(lib("archive/old rules/x/SKILL.md"))
        );
        assert_eq!(
            lib("team/rules").replace_prefix(&from, &to),
            Some(to.clone())
        );
        assert_eq!(lib("team/rules2/a.md").replace_prefix(&from, &to), None);
        assert_eq!(lib("team/a.md").replace_prefix(&from, &to), None);
        assert_eq!(lib("team").replace_prefix(&from, &to), None);
        assert_eq!(
            lib("team/rules/A.md").replace_prefix(&from, &to),
            Some(lib("archive/old rules/A.md"))
        );
        assert_eq!(
            lib("a.md").replace_prefix(&lib("a.md"), &lib("b/c.md")),
            Some(lib("b/c.md"))
        );
    }

    // Review round 1 main decision 4: the leading names match by the platform path key — on Windows a different case
    //   still matches, elsewhere it does not. The expected answer follows `path_key`, so the crate needs no OS cfg.
    #[test]
    fn replace_prefix_matches_leading_names_by_path_key_and_keeps_both_spellings() {
        let (from, to) = (lib("team/rules"), lib("Archive/Old"));
        let folds = path_key(Path::new("Team")) == path_key(Path::new("team"))
            && path_key(Path::new("RULES")) == path_key(Path::new("rules"));
        let moved = folds.then(|| lib("Archive/Old/X/a.MD"));
        assert_eq!(lib("Team/RULES/X/a.MD").replace_prefix(&from, &to), moved);
        assert_eq!(
            lib("Team/RULES").replace_prefix(&from, &to),
            folds.then(|| to.clone())
        );
        assert_eq!(lib("Team/RULES2/a.md").replace_prefix(&from, &to), None);
    }

    // TRD S25 step-7 §8-1 P1 path row — `engram:/…` vs library; unknown built-in.
    #[test]
    fn asset_ref_parse_decides_by_the_builtin_scheme() {
        assert_eq!(
            AssetRef::parse("engram:/default"),
            Ok(AssetRef::BuiltIn(BuiltinId::DefaultPreset))
        );
        assert_eq!(
            AssetRef::parse("engram:/priming"),
            Ok(AssetRef::BuiltIn(BuiltinId::Priming))
        );
        assert_eq!(
            AssetRef::parse("engram:/settings/default"),
            Ok(AssetRef::BuiltIn(BuiltinId::DefaultSettings))
        );
        assert_eq!(
            AssetRef::parse("rules/a.md"),
            Ok(AssetRef::Library(lib("rules/a.md")))
        );
        for unknown in [
            "engram:/",
            "engram:/nope",
            "engram:/settings",
            "engram:/Default",
        ] {
            let id = unknown.strip_prefix(BUILTIN_SCHEME).unwrap();
            assert_eq!(
                AssetRef::parse(unknown),
                Err(PathError::UnknownBuiltin(id.to_owned())),
                "{unknown}"
            );
        }
        assert_eq!(
            AssetRef::parse("engram:default"),
            Err(PathError::ForbiddenChar(':'))
        );
        assert_eq!(AssetRef::parse("/rules/a.md"), Err(PathError::LeadingSlash));
    }

    #[test]
    fn asset_ref_spelling_reads_back_to_the_same_ref() {
        let refs = BuiltinId::ALL
            .into_iter()
            .map(AssetRef::BuiltIn)
            .chain([AssetRef::Library(lib("업무/a.md"))]);
        for asset in refs {
            assert_eq!(
                AssetRef::parse(&asset.spelling()),
                Ok(asset.clone()),
                "{asset:?}"
            );
        }
        assert_eq!(
            AssetRef::BuiltIn(BuiltinId::DefaultSettings).spelling(),
            "engram:/settings/default"
        );
    }

    #[test]
    fn folder_ref_normalizes_its_raw_spelling() {
        let relative = FolderRef::new("rules/company".to_owned(), 4, None);
        assert_eq!(relative.normalized, Err(FolderPathError::NotAbsolute));
        assert_eq!(relative.raw, "rules/company");

        let absolute = std::env::temp_dir()
            .join("Rules")
            .join("x")
            .join("..")
            .join("Company");
        let absolute = absolute.to_str().expect("a UTF-8 temp folder");
        let entry = FolderRef::new(absolute.to_owned(), 7, Some(ItemOrigin { index: 2 }));
        assert_eq!(entry.raw, absolute);
        assert_eq!(
            entry.normalized,
            normalize_folder(Path::new(absolute)).map_err(FolderPathError::from)
        );
        assert!(entry.normalized.is_ok());
        assert_eq!(
            (entry.line, entry.origin),
            (7, Some(ItemOrigin { index: 2 }))
        );
    }
}
