//! Folder-path normalization — one spelling per absolute folder path, so that [`super::path_key`] (which folds case
//! only) gives one key per folder. Lexical only: no disk access, no link resolution. The answer is a fixed point: the
//! normalizer gives it back unchanged.
//!
//! Known gaps — two spellings of one folder that still key apart (each is a rare hand-typed form; none makes two
//! different folders key alike):
//! - Win32 trims a final name's trailing `.` and spaces (`D:\Rules ` opens `D:\Rules`) and one trailing `.` of any
//!   name, even before a separator (`D:\Rules.`, `D:\Rules.\` and `D:\Rules` all open `D:\Rules`); this normalizer
//!   does not trim names, so `D:\Rules.` keys apart from `D:\Rules`.
//! - A device path (`\\.\D:\x`) is not turned into its drive form (`D:\x`).
//! - A verbatim path that is not safe to strip is returned as spelled, its trailing separator included
//!   (`\\?\D:\a \` and `\\?\D:\a \\` would differ only there).

// TRD S25 step-7 §3-10

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{is_separator, Component, Path, PathBuf, Prefix, MAIN_SEPARATOR_STR};

/// Win32's `MAX_PATH`; a longer verbatim path keeps its prefix (without it Win32 would refuse the path).
const MAX_PATH: usize = 260;
/// The longest name Win32 accepts, in UTF-16 units.
const MAX_NAME: usize = 255;
/// Microsoft "Naming Files, Paths, and Namespaces" (2024-08-28): the superscript digits `¹ ² ³` are digits to Win32,
/// so `COM¹` … `LPT³` are reserved too.
const RESERVED_NAMES: [&str; 28] = [
    "CON",
    "PRN",
    "AUX",
    "NUL",
    "COM1",
    "COM2",
    "COM3",
    "COM4",
    "COM5",
    "COM6",
    "COM7",
    "COM8",
    "COM9",
    "COM\u{b9}",
    "COM\u{b2}",
    "COM\u{b3}",
    "LPT1",
    "LPT2",
    "LPT3",
    "LPT4",
    "LPT5",
    "LPT6",
    "LPT7",
    "LPT8",
    "LPT9",
    "LPT\u{b9}",
    "LPT\u{b2}",
    "LPT\u{b3}",
];

/// Whether Win32 reads the name `name` (one path component) as a reserved device — `CON` · `PRN` · `AUX` · `NUL` ·
/// `COM1`–`COM9` · `LPT1`–`LPT9` and the superscript `COM¹ COM² COM³ LPT¹ LPT² LPT³`, ASCII case-insensitive, with
/// any extension and with spaces before it (`nul.txt` · `Com1 .tar.gz` · `COM¹.md`). The part before the first `.`,
/// trailing spaces trimmed, is compared — stricter than Win32 for a few spellings, never looser. `CONSOLE` ·
/// `nulx.md` · `com10.txt` are not reserved. The rule is the same on every OS: a caller that keeps names usable on
/// Windows asks it whatever OS it runs on.
pub fn is_reserved_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    RESERVED_NAMES
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
}

/// [`normalize_folder`] refused a path that is not absolute — a relative path (`rules\x`, `.\x`), a drive-relative
/// path (`D:x`) or a root-relative path without a drive (`\x` on Windows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotAbsolute;

impl fmt::Display for NotAbsolute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not an absolute path")
    }
}

impl std::error::Error for NotAbsolute {}

/// The spelling of the absolute folder `path` that the caller hands to [`super::path_key`] — the spelling choice that
/// function leaves to its caller. Steps, in order:
///
/// 1. A verbatim disk or UNC prefix is stripped when that keeps the meaning: `\\?\D:\x` → `D:\x` ·
///    `\\?\UNC\srv\share\x` → `\\srv\share\x`. It is kept — and the path returned as spelled — when a name ends in `.`
///    or a space, holds a character Win32 refuses in a name (`/` included), is a reserved device name
///    ([`is_reserved_device_name`]) or longer than 255 units, a name is `.` or `..` (literal names in a verbatim path),
///    or the path is longer than 260 units. Other verbatim forms (`\\?\Volume{…}`) are returned as spelled. A verbatim
///    prefix with nothing to name (`\\?\` · `\\?\UNC\` · `\\?\UNC\srv` without a share) is refused ([`NotAbsolute`]).
/// 2. A path that is not absolute is refused ([`NotAbsolute`]) — it is not resolved against any folder.
/// 3. Separators become the OS separator (`\` on Windows, `/` elsewhere). A UNC server named `?` (`//?/D:/x`,
///    `\\?/D:\x`) is a device path to Win32 and is spelled as one (`\\.\D:\x`), like `\\.\…` itself.
/// 4. `.` is dropped and `..` removes the name before it; `..` at the root stays at the root.
/// 5. No trailing separator, except on a root (`D:\` · `\\srv\share\` · `\\.\D:\` · `/` — a UNC root always ends with
///    `\`) and, on Windows, after a final name whose meaning the separator protects. Win32 removes one trailing `.` of
///    a name even before a separator, and without a separator it removes every trailing `.` and space. So the
///    separator is kept when the name, less one trailing `.`, still ends in a space or `.` (`D:\Rules \` names
///    `Rules `, `D:\Rules ` names `Rules`; `D:\Rules .\` names `Rules `), and dropped otherwise (`D:\Rules.\` →
///    `D:\Rules.` — both name `Rules`).
///
/// Case is kept as spelled — folding is [`super::path_key`]'s job — except that std reads a verbatim drive letter in
/// upper case (`\\?\d:\x` → `D:\x`). Known gaps: the module doc.
pub fn normalize_folder(path: &Path) -> Result<PathBuf, NotAbsolute> {
    let plain = match plain_spelling(path) {
        Spelling::Plain(plain) => plain,
        Spelling::KeepVerbatim => return Ok(path.to_path_buf()),
        Spelling::Refused => return Err(NotAbsolute),
    };
    let ends_in_separator = plain
        .as_encoded_bytes()
        .last()
        .is_some_and(|&byte| is_separator(char::from(byte)));
    let plain = Path::new(&plain);
    if !plain.is_absolute() {
        return Err(NotAbsolute);
    }
    let mut root = OsString::new();
    let mut names: Vec<&OsStr> = Vec::new();
    for component in plain.components() {
        match component {
            Component::Prefix(prefix) => root = prefix_spelling(prefix.kind(), prefix.as_os_str()),
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => {
                names.pop();
            }
            Component::Normal(name) => names.push(name),
        }
    }
    let keep_final_separator = cfg!(windows)
        && ends_in_separator
        && names.last().is_some_and(|name| separator_protects(name));
    root.push(MAIN_SEPARATOR_STR);
    for (i, name) in names.iter().enumerate() {
        if i > 0 {
            root.push(MAIN_SEPARATOR_STR);
        }
        root.push(name);
    }
    if keep_final_separator {
        root.push(MAIN_SEPARATOR_STR);
    }
    Ok(PathBuf::from(root))
}

enum Spelling {
    /// The path to normalize: `path` itself, or a verbatim disk / UNC path rewritten to its plain form.
    Plain(OsString),
    /// A verbatim path whose plain form would mean something else (step 1).
    KeepVerbatim,
    /// A verbatim prefix that names no drive, share or volume (step 1).
    Refused,
}

fn plain_spelling(path: &Path) -> Spelling {
    let unchanged = || Spelling::Plain(path.as_os_str().to_owned());
    let mut components = path.components();
    let mut plain = match components.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(letter) => OsString::from(format!("{}:", char::from(letter))),
            Prefix::VerbatimUNC(server, share) if server.is_empty() || share.is_empty() => {
                return Spelling::Refused
            }
            Prefix::Verbatim(rest) if rest.is_empty() => return Spelling::Refused,
            Prefix::VerbatimUNC(server, share) => {
                if !safe_name(server) || !safe_name(share) {
                    return Spelling::KeepVerbatim;
                }
                unc_root(server, share)
            }
            Prefix::Verbatim(_) => return Spelling::KeepVerbatim,
            _ => return unchanged(),
        },
        _ => return unchanged(),
    };
    let too_long = path
        .to_str()
        .map_or(true, |text| text.encode_utf16().count() > MAX_PATH);
    if too_long {
        return Spelling::KeepVerbatim;
    }
    plain.push("\\");
    let mut first = true;
    for component in components {
        match component {
            Component::RootDir => continue,
            Component::Normal(name) if safe_name(name) => {}
            _ => return Spelling::KeepVerbatim,
        }
        if !first {
            plain.push("\\");
        }
        plain.push(component.as_os_str());
        first = false;
    }
    Spelling::Plain(plain)
}

/// Whether `name` means the same with and without a verbatim prefix: Win32 neither trims it, refuses it nor reads it as
/// a device.
fn safe_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let refused = |c: char| {
        c.is_ascii_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
    };
    !name.is_empty()
        && name.encode_utf16().count() <= MAX_NAME
        && !name.chars().any(refused)
        && !name.ends_with(['.', ' '])
        && !is_reserved_device_name(name)
}

/// Whether a separator after the final name `name` changes which entry Win32 opens (step 5): with it Win32 removes one
/// trailing `.`, without it every trailing `.` and space (Microsoft "File path formats on Windows systems", "Trim
/// characters").
fn separator_protects(name: &OsStr) -> bool {
    let bytes = name.as_encoded_bytes();
    let rest = bytes.strip_suffix(b".").unwrap_or(bytes);
    matches!(rest.last(), Some(b' ' | b'.'))
}

/// The prefix written with `\` separators (`//srv/share` → `\\srv\share`). A UNC server `?` is a device path (step 3);
/// a server `.` never arrives as UNC — std parses `//./x` as a device path already. Verbatim prefixes never reach
/// here — step 1 strips them, returns the path as spelled or refuses it.
fn prefix_spelling(kind: Prefix<'_>, spelled: &OsStr) -> OsString {
    match kind {
        Prefix::UNC(server, share) if server == "?" => device_root(share),
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => unc_root(server, share),
        Prefix::DeviceNS(device) => device_root(device),
        Prefix::Disk(_) | Prefix::VerbatimDisk(_) | Prefix::Verbatim(_) => spelled.to_owned(),
    }
}

fn device_root(device: &OsStr) -> OsString {
    let mut root = OsString::from(r"\\.\");
    root.push(device);
    root
}

fn unc_root(server: &OsStr, share: &OsStr) -> OsString {
    let mut root = OsString::from(r"\\");
    root.push(server);
    root.push(r"\");
    root.push(share);
    root
}

#[cfg(test)]
mod tests {
    use std::path::MAIN_SEPARATOR;

    use super::*;
    use crate::fs::path_key;

    /// The expected answer on this OS: `windows` on Windows, `other` elsewhere. `None` = refused as not absolute.
    fn on_this_os<'a>(windows: Option<&'a str>, other: Option<&'a str>) -> Option<&'a str> {
        if cfg!(windows) {
            windows
        } else {
            other
        }
    }

    fn normalized(input: &str) -> Option<OsString> {
        normalize_folder(Path::new(input))
            .ok()
            .map(PathBuf::into_os_string)
    }

    // TRD S25 step-7 §8-1 P1 platform row. (input, Windows answer, answer elsewhere) — compared as spelled, trailing
    //   separator included.
    const TABLE: &[(&str, Option<&str>, Option<&str>)] = &[
        (r"\\?\D:\x", Some(r"D:\x"), None),
        (r"\\?\d:\x\", Some(r"D:\x"), None),
        (r"\\?\UNC\s\x", Some(r"\\s\x\"), None),
        (r"\\?\UNC\srv\share\x", Some(r"\\srv\share\x"), None),
        (r"\\?\D:\", Some(r"D:\"), None),
        // Verbatim paths whose plain form would mean something else keep their spelling.
        (r"\\?\D:\x\..\Rules\", Some(r"\\?\D:\x\..\Rules\"), None),
        (r"\\?\D:\x\.\y", Some(r"\\?\D:\x\.\y"), None),
        (r"\\?\D:\a/b", Some(r"\\?\D:\a/b"), None),
        (r"\\?\D:\Rules \x", Some(r"\\?\D:\Rules \x"), None),
        (r"\\?\D:\Rules.", Some(r"\\?\D:\Rules."), None),
        (r"\\?\D:\CON", Some(r"\\?\D:\CON"), None),
        (r"\\?\D:\x\nul.txt", Some(r"\\?\D:\x\nul.txt"), None),
        (
            r"\\?\D:\x\Com1 .tar.gz",
            Some(r"\\?\D:\x\Com1 .tar.gz"),
            None,
        ),
        (r"\\?\UNC\?\x", Some(r"\\?\UNC\?\x"), None),
        (r"\\?\D:\COM¹", Some(r"\\?\D:\COM¹"), None),
        (r"\\?\D:\com².txt", Some(r"\\?\D:\com².txt"), None),
        (r"\\?\D:\x\COM³.tar.gz", Some(r"\\?\D:\x\COM³.tar.gz"), None),
        (r"\\?\D:\LPT¹", Some(r"\\?\D:\LPT¹"), None),
        (r"\\?\D:\lpt² .md\x", Some(r"\\?\D:\lpt² .md\x"), None),
        (r"\\?\D:\LPT³.log", Some(r"\\?\D:\LPT³.log"), None),
        (r"\\?\D:\COM⁴\x", Some(r"D:\COM⁴\x"), None),
        (r"\\?\D:\com10.txt", Some(r"D:\com10.txt"), None),
        (r"\\?\UNC\.\x", Some(r"\\?\UNC\.\x"), None),
        (r"\\?\Volume{0}\a\..\b", Some(r"\\?\Volume{0}\a\..\b"), None),
        // A verbatim prefix that names nothing.
        (r"\\?\", None, None),
        (r"\\?\UNC\", None, None),
        (r"\\?\UNC\srv", None, None),
        (r"\\?\UNC\srv\", None, None),
        (r"\\?\D:\CONSOLE\x", Some(r"D:\CONSOLE\x"), None),
        // Plain paths.
        (r"D:\x\..\Rules\", Some(r"D:\Rules"), None),
        (r"D:/x/./y//z/", Some(r"D:\x\y\z"), None),
        (r"d:/rules", Some(r"d:\rules"), None),
        (r"D:\", Some(r"D:\"), None),
        (r"D:\..\..", Some(r"D:\"), None),
        (r"D:\Rules\..", Some(r"D:\"), None),
        (r"D:\Rules \", Some(r"D:\Rules \"), None),
        (r"D:\Rules \\", Some(r"D:\Rules \"), None),
        (r"D:\Rules.\", Some(r"D:\Rules."), None),
        (r"D:\Rules. \", Some(r"D:\Rules. \"), None),
        (r"D:\Rules .\", Some(r"D:\Rules .\"), None),
        (r"D:\Rules..\", Some(r"D:\Rules..\"), None),
        (r"D:\Rules.", Some(r"D:\Rules."), None),
        (r"D:\Rules ", Some(r"D:\Rules "), None),
        (r"D:\a \b\", Some(r"D:\a \b"), None),
        (r"\\srv\share", Some(r"\\srv\share\"), None),
        (r"\\srv\share\", Some(r"\\srv\share\"), None),
        (r"\\srv\share\a\..", Some(r"\\srv\share\"), None),
        (
            r"//srv/share/a/./b/",
            Some(r"\\srv\share\a\b"),
            Some("/srv/share/a/b"),
        ),
        // Device paths, including a UNC server named `?` or `.`.
        (r"//?/D:/x", Some(r"\\.\D:\x"), Some("/?/D:/x")),
        (r"\\?/D:\x", Some(r"\\.\D:\x"), None),
        (r"/\?\D:\x\..\y", Some(r"\\.\D:\y"), Some(r"/\?\D:\x\..\y")),
        (r"//./D:/x/", Some(r"\\.\D:\x"), Some("/D:/x")),
        (r"\\.\COM1\x\..\y", Some(r"\\.\COM1\y"), None),
        (r"\\.\D:\", Some(r"\\.\D:\"), None),
        // Refused.
        (r"D:x", None, None),
        (r"D:", None, None),
        (r"\x", None, None),
        (r"rules\x", None, None),
        (r".\x", None, None),
        (r"..\x", None, None),
        ("rules/x.md", None, None),
        ("", None, None),
        // Other OSes.
        ("/a/b/../c/", None, Some("/a/c")),
        ("/Rules /", None, Some("/Rules ")),
        ("/", None, Some("/")),
        ("/..", None, Some("/")),
    ];

    /// Inputs a later step could misread — only the fixed-point test uses them, beside the table.
    const MORE_INPUTS: &[&str] = &[
        r"//?/UNC/s/x",
        r"\\?/UNC\s\x\..",
        r"//./x",
        r"\\./D:\x",
        r"//?/D:",
        r"\\?\Volume{0}\a\..\b",
        r"\\?\UNC\srv\share\a.\b",
        r"//srv/share/a ./",
    ];

    fn long_verbatim() -> String {
        format!(r"\\?\D:{}", r"\abcdefghij".repeat(30))
    }

    #[test]
    fn the_table_answers_per_os() {
        for &(input, windows, other) in TABLE {
            let want = on_this_os(windows, other).map(OsString::from);
            assert_eq!(normalized(input), want, "{input:?}");
        }
    }

    #[test]
    fn a_long_verbatim_path_keeps_its_prefix() {
        let long = long_verbatim();
        let want = on_this_os(Some(&long), None).map(OsString::from);
        assert_eq!(normalized(&long), want);
        let short = format!(r"\\?\D:{}", r"\abcdefghij".repeat(20));
        let want = on_this_os(Some(&short[4..]), None).map(OsString::from);
        assert_eq!(normalized(&short), want);
    }

    #[test]
    fn a_normalized_path_is_its_own_normal_form() {
        let long = long_verbatim();
        let inputs = TABLE
            .iter()
            .map(|&(input, _, _)| input)
            .chain(MORE_INPUTS.iter().copied())
            .chain([long.as_str()]);
        for input in inputs {
            if let Some(once) = normalized(input) {
                let twice = normalize_folder(Path::new(&once)).map(PathBuf::into_os_string);
                assert_eq!(twice, Ok(once.clone()), "{input:?}");
            }
        }
    }

    // Review round 1 must 2: a trailing separator only on a root — or, on Windows, after a final name whose meaning it
    //   protects (step 5), and in a path kept verbatim.
    #[test]
    fn only_a_root_keeps_its_trailing_separator() {
        for &(input, _, _) in TABLE {
            let Some(out) = normalized(input) else {
                continue;
            };
            let out_path = Path::new(&out);
            let last_name = out_path.components().rev().find_map(|c| match c {
                Component::Normal(name) => Some(name),
                _ => None,
            });
            let ends_in_separator = out.as_encoded_bytes().last() == Some(&(MAIN_SEPARATOR as u8));
            let verbatim = out.as_encoded_bytes().starts_with(br"\\?\");
            let separator_keeps_name = cfg!(windows) && last_name.is_some_and(separator_protects);
            match last_name {
                _ if verbatim => {}
                None => assert!(
                    ends_in_separator,
                    "a root ends in a separator: {input:?} → {out:?}"
                ),
                Some(_) if separator_keeps_name => {}
                Some(_) => assert!(!ends_in_separator, "{input:?} → {out:?}"),
            }
        }
    }

    // Review round 2: the reserved list holds the superscript forms; the stem is the part before the first `.`, trailing
    //   spaces trimmed, ASCII case-insensitive.
    #[test]
    fn reserved_device_names_with_any_extension() {
        for name in [
            "CON",
            "con",
            "Prn.txt",
            "aux.tar.gz",
            "NUL .md",
            "nul",
            "COM1",
            "com9.log",
            "LPT1",
            "Lpt9 .x",
            "COM¹",
            "com²",
            "COM³.txt",
            "LPT¹.md",
            "lpt².tar.gz",
            "LPT³ .log",
            "Com1 .tar.gz",
        ] {
            assert!(is_reserved_device_name(name), "{name:?}");
        }
        for name in [
            "CONSOLE",
            "nulx.md",
            "com10.txt",
            "COM0",
            "LPT0",
            "COM⁴",
            "COM",
            "LPT",
            " CON",
            "xNUL",
            "",
            ".con",
            "COM1x",
        ] {
            assert!(!is_reserved_device_name(name), "{name:?}");
        }
    }

    #[test]
    fn spellings_of_one_folder_key_alike_after_normalizing() {
        let key = |text: &str| path_key(&normalize_folder(Path::new(text)).expect(text));
        if cfg!(windows) {
            assert_eq!(key(r"D:\Rules"), key("d:/rules"));
            assert_eq!(key(r"D:\Rules"), key(r"\\?\D:\rules\"));
            assert_eq!(key(r"D:\Rules"), key(r"d:\x\..\RULES\."));
            assert_eq!(key(r"\\srv\share\x"), key(r"\\?\UNC\SRV\share\x\"));
            assert_eq!(key(r"\\srv\share"), key(r"\\srv\share\"));
            assert_eq!(key(r"\\.\D:\x"), key("//?/d:/x"));
            assert_ne!(key(r"D:\Rules"), key(r"D:\Rules2"));
            assert_ne!(key(r"D:\Rules \"), key(r"D:\Rules"));
        } else {
            assert_eq!(key("/rules"), key("/x/../rules/."));
            assert_ne!(key("/Rules"), key("/rules"));
        }
    }
}
