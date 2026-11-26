//! A rename that never replaces what already has the destination name (ADR-0409 decision 10). There is no fallback to an
//! overwriting rename: where the OS or the volume cannot promise no-replace, the answer is `Unsupported`.

// ADR-0409

use std::io;
use std::path::Path;

/// Renames `from` to `to` — a file, a folder, or a link (renamed as itself, never followed) — and fails with
/// `AlreadyExists` when anything already has the name `to`; nothing is replaced. Only the last names of `from` and
/// `to` are taken as themselves: the folders above them resolve as usual, and the caller's no-follow checks cover them
/// (ADR-0409). `NotFound` = `from` missing or `to`'s parent folder missing. One attempt: the caller retries a busy
/// failure with [`super::retry_busy`] (an antivirus or indexer holding the entry).
///
/// - Windows: `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` or `MOVEFILE_COPY_ALLOWED`. A case-only rename
///   (`a.md` → `A.md`) renames the entry. A path std reaches is reached here too (the long form below).
/// - Linux and Android: `renameat2(RENAME_NOREPLACE)`; Apple: `renamex_np(RENAME_EXCL)`. Whether these accept a case-only
///   rename on a case-insensitive volume is unknown — if not, that rename fails (`AlreadyExists` on a casefold Linux
///   folder is possible); the caller that knows the rename is case-only reads it so.
/// - Another volume = `ErrorKind::CrossesDevices` (Windows `ERROR_NOT_SAME_DEVICE`, Unix `EXDEV`); nothing is copied.
/// - The OS or the volume refusing the no-replace flag itself = `ErrorKind::Unsupported` with the OS error inside
///   (`io::Error::get_ref`): `ENOSYS`, `ENOTSUP` / `EOPNOTSUPP`, and `EINVAL` unless `to` lies inside `from` — a
///   folder renamed into its own descendant answers `EINVAL` too, and that is returned as is; "inside" is judged on the
///   spelling, a heuristic (the OS error is carried either way). Other Unix systems: `Unsupported` without an OS error.
#[cfg(windows)]
pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::MoveFileExW;
    let from = long_form(from)?;
    let to = long_form(to)?;
    // SAFETY: both are NUL-terminated UTF-16 strings that outlive the call, which only reads them.
    unsafe { MoveFileExW(PCWSTR(from.as_ptr()), PCWSTR(to.as_ptr()), NO_REPLACE_FLAGS) }
        .map_err(super::win32_io_error)
}

// ADR-0409: no `MOVEFILE_REPLACE_EXISTING` (a taken name fails) and no `MOVEFILE_COPY_ALLOWED` (another volume fails
//   instead of becoming a copy and a delete).
#[cfg(windows)]
const NO_REPLACE_FLAGS: windows::Win32::Storage::FileSystem::MOVE_FILE_FLAGS =
    windows::Win32::Storage::FileSystem::MOVE_FILE_FLAGS(0);
#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let from_c = CString::new(from.as_os_str().as_bytes())?;
    let to_c = CString::new(to.as_os_str().as_bytes())?;
    if rename_exclusive(&from_c, &to_c) == 0 {
        Ok(())
    } else {
        Err(refused_flag(
            io::Error::last_os_error(),
            from,
            to,
            &UNIX_REFUSALS,
        ))
    }
}
#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android", target_vendor = "apple"))
))]
pub fn rename_no_replace(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no no-replace rename on this OS",
    ))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_exclusive(from: &std::ffi::CStr, to: &std::ffi::CStr) -> libc::c_long {
    // Through `syscall`: libc's `renameat2` wrapper exists only for glibc 2.28+, the system call on every Linux 3.15+
    //   (older kernels answer `ENOSYS`).
    // SAFETY: both are NUL-terminated strings that outlive the call, which only reads them; the arguments are
    //   `renameat2`'s (old folder fd, old path, new folder fd, new path, flags).
    unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    }
}
#[cfg(target_vendor = "apple")]
fn rename_exclusive(from: &std::ffi::CStr, to: &std::ffi::CStr) -> libc::c_long {
    // SAFETY: both are NUL-terminated strings that outlive the call, which only reads them.
    libc::c_long::from(unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) })
}

/// The OS codes by which a no-replace rename's flag itself is refused.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    test
))]
struct Refusals {
    /// Always the flag: the call or the flag is unknown to the OS or the volume.
    refused: &'static [i32],
    /// The flag unless `to` lies inside `from` — a folder renamed into its own descendant answers this code too.
    ambiguous: i32,
}

// Linux: `ENOSYS` (kernel before 3.15) · `EOPNOTSUPP` (= `ENOTSUP`) · `EINVAL` (a file system without the flag).
#[cfg(any(target_os = "linux", target_os = "android"))]
const UNIX_REFUSALS: Refusals = Refusals {
    refused: &[libc::ENOSYS, libc::EOPNOTSUPP],
    ambiguous: libc::EINVAL,
};
// Apple: `ENOTSUP` (a volume without the flag) · `EOPNOTSUPP` and `ENOSYS` (distinct codes there) · `EINVAL`.
#[cfg(target_vendor = "apple")]
const UNIX_REFUSALS: Refusals = Refusals {
    refused: &[libc::ENOTSUP, libc::EOPNOTSUPP, libc::ENOSYS],
    ambiguous: libc::EINVAL,
};

/// What a failed no-replace rename answers: an OS code that refuses the flag itself becomes `Unsupported`, the OS error
/// carried inside so it stays readable; any other failure is returned as is. Never a reason to retry without the flag.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    test
))]
fn refused_flag(e: io::Error, from: &Path, to: &Path, codes: &Refusals) -> io::Error {
    let refused = match e.raw_os_error() {
        Some(code) if codes.refused.contains(&code) => true,
        // Lexical — a `to` reaching inside `from` through a link reads as the flag; the code is still carried.
        Some(code) if code == codes.ambiguous => !to.starts_with(from),
        _ => false,
    };
    if refused {
        io::Error::new(io::ErrorKind::Unsupported, e)
    } else {
        e
    }
}

/// `path` as a NUL-terminated UTF-16 string a Win32 call takes at any length — close to std's long-path rule: an
/// absolute path under 248 units, or one already in the `\\?\` form, goes as is; any other is made absolute
/// (`std::path::absolute` — `GetFullPathNameW`) and, unless that answers a device or verbatim path, prefixed `\\?\`
/// (`\\?\UNC\` for `\\server\share`). Without it a path past `MAX_PATH` that std renames would fail here. (std also
/// counts the NUL and turns a `\\.\` answer into `\\?\`; neither matters below `MAX_PATH`.)
#[cfg(windows)]
fn long_form(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    const LEGACY_MAX_PATH: usize = 248;
    let units = |text: &str| text.encode_utf16().collect::<Vec<u16>>();
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains NUL",
        ));
    }
    let verbatim = units(r"\\?\");
    if wide.starts_with(&verbatim)
        || wide.starts_with(&units(r"\??\"))
        || (wide.len() < LEGACY_MAX_PATH && path.is_absolute())
    {
        return Ok(wide.into_iter().chain([0]).collect());
    }
    let mut full: Vec<u16> = std::path::absolute(path)?
        .as_os_str()
        .encode_wide()
        .collect();
    let prefix = if full.starts_with(&verbatim) || full.starts_with(&units(r"\\.\")) {
        Vec::new()
    } else if full.starts_with(&units(r"\\")) {
        full.drain(..2);
        units(r"\\?\UNC\")
    } else {
        verbatim
    };
    Ok(prefix.into_iter().chain(full).chain([0]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::{link_dir, Scratch};
    use std::path::PathBuf;

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("list folder")
            .map(|entry| {
                entry
                    .expect("folder entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    fn folder_with(dir: &Path, file: &str, content: &[u8]) -> PathBuf {
        std::fs::create_dir(dir).expect("folder");
        std::fs::write(dir.join(file), content).expect("file in folder");
        dir.to_path_buf()
    }

    /// Whether this OS has a no-replace rename; on any other the declared answer is `Unsupported`.
    const NO_REPLACE: bool = cfg!(any(
        windows,
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple"
    ));

    /// What a call refused for a taken name answers here.
    fn taken_kind() -> io::ErrorKind {
        if NO_REPLACE {
            io::ErrorKind::AlreadyExists
        } else {
            io::ErrorKind::Unsupported
        }
    }

    /// Renames, true; on an OS without a no-replace rename, checks the `Unsupported` answer with nothing moved — the
    /// source still there, nothing at the destination — false.
    fn renamed(from: &Path, to: &Path) -> bool {
        match rename_no_replace(from, to) {
            Ok(()) if NO_REPLACE => true,
            Err(e) if !NO_REPLACE => {
                assert_eq!(e.kind(), io::ErrorKind::Unsupported, "{e:?}");
                assert!(std::fs::symlink_metadata(from).is_ok(), "the source stays");
                assert!(
                    std::fs::symlink_metadata(to)
                        .is_err_and(|e| e.kind() == io::ErrorKind::NotFound),
                    "nothing arrives"
                );
                false
            }
            outcome => panic!("{} -> {}: {outcome:?}", from.display(), to.display()),
        }
    }

    // Both a file and a folder go through `renamed` on every OS (an OS without a no-replace rename checks both refusals).
    #[test]
    fn a_file_and_a_folder_are_renamed() {
        let scratch = Scratch::new("rename");
        let file = scratch.0.join("a.md");
        std::fs::write(&file, b"a").expect("file");
        let folder = folder_with(&scratch.0.join("dir"), "inner.md", b"inner");
        let file_renamed = renamed(&file, &scratch.0.join("b.md"));
        let folder_renamed = renamed(&folder, &scratch.0.join("moved"));
        assert_eq!((file_renamed, folder_renamed), (NO_REPLACE, NO_REPLACE));
        let (file_now, folder_now) = if NO_REPLACE {
            ("b.md", "moved")
        } else {
            ("a.md", "dir")
        };
        assert_eq!(std::fs::read(scratch.0.join(file_now)).expect("file"), b"a");
        assert_eq!(
            std::fs::read(scratch.0.join(folder_now).join("inner.md")).expect("file in folder"),
            b"inner"
        );
        assert_eq!(names(&scratch.0), [file_now, folder_now]);
    }

    // File over file, folder over folder (empty or not), file over folder, folder over file — each refused, both sides as
    //   they were.
    #[test]
    fn a_taken_name_is_already_exists_and_nothing_is_replaced() {
        let scratch = Scratch::new("rename");
        let file = scratch.0.join("file.md");
        std::fs::write(&file, b"source").expect("source file");
        let taken_file = scratch.0.join("taken.md");
        std::fs::write(&taken_file, b"taken").expect("taken file");
        let folder = folder_with(&scratch.0.join("folder"), "source.md", b"source");
        let taken_folder = folder_with(&scratch.0.join("taken-folder"), "taken.md", b"taken");
        let empty_folder = scratch.0.join("empty");
        std::fs::create_dir(&empty_folder).expect("empty folder");

        for (from, to) in [
            (&file, &taken_file),
            (&folder, &taken_folder),
            (&folder, &empty_folder),
            (&file, &taken_folder),
            (&folder, &taken_file),
        ] {
            let e = rename_no_replace(from, to).expect_err("a taken name is refused");
            assert_eq!(
                e.kind(),
                taken_kind(),
                "{} -> {}: {e:?}",
                from.display(),
                to.display()
            );
        }
        assert_eq!(std::fs::read(&file).expect("source file"), b"source");
        assert_eq!(std::fs::read(&taken_file).expect("taken file"), b"taken");
        assert_eq!(names(&folder), ["source.md"]);
        assert_eq!(names(&taken_folder), ["taken.md"]);
        assert!(
            names(&empty_folder).is_empty(),
            "the empty folder stays empty"
        );
        assert_eq!(
            names(&scratch.0),
            ["empty", "file.md", "folder", "taken-folder", "taken.md"]
        );
    }

    // Windows must rename. Elsewhere the rename may be refused — `AlreadyExists` on a case-insensitive volume (R13 of the
    //   step-9b TRD) or `Unsupported`; an OS without a no-replace rename must answer `Unsupported`. Success lists the new
    //   spelling only; a refusal keeps the old spelling only; the content is never lost.
    #[test]
    fn a_case_only_rename_renames_the_entry_or_refuses_keeping_it() {
        let scratch = Scratch::new("rename");
        let file = scratch.0.join("a.md");
        std::fs::write(&file, b"a").expect("file");
        let folder = folder_with(&scratch.0.join("dir"), "inner.md", b"inner");
        for (from, lower, upper) in [(&file, "a.md", "A.md"), (&folder, "dir", "Dir")] {
            let outcome = rename_no_replace(from, &scratch.0.join(upper));
            let listed = names(&scratch.0);
            let now = match outcome {
                Ok(()) if NO_REPLACE => upper,
                Err(e) if !cfg!(windows) => {
                    let allowed: &[io::ErrorKind] = if NO_REPLACE {
                        &[io::ErrorKind::AlreadyExists, io::ErrorKind::Unsupported]
                    } else {
                        &[io::ErrorKind::Unsupported]
                    };
                    assert!(allowed.contains(&e.kind()), "{upper}: {e:?}");
                    lower
                }
                // Windows refusing, or an OS without the rename succeeding.
                outcome => panic!("{upper}: {outcome:?}"),
            };
            let other = if now == upper { lower } else { upper };
            assert!(listed.iter().any(|name| name == now), "{now} in {listed:?}");
            assert!(
                !listed.iter().any(|name| name == other),
                "{other} not in {listed:?}"
            );
            let path = scratch.0.join(now);
            if path.is_dir() {
                assert_eq!(names(&path), ["inner.md"]);
            } else {
                assert_eq!(std::fs::read(&path).expect("file"), b"a");
            }
        }
    }

    // Past `MAX_PATH` on Windows: the long form must reach what std reaches.
    #[test]
    fn a_long_path_is_renamed_and_its_taken_name_refused() {
        let scratch = Scratch::new("rename");
        let deep = scratch
            .0
            .join("d".repeat(120))
            .join("e".repeat(120))
            .join("f".repeat(40));
        std::fs::create_dir_all(&deep).expect("deep folder");
        assert!(
            deep.as_os_str().len() > 300,
            "premise: a path past MAX_PATH"
        );
        let from = deep.join("a.md");
        std::fs::write(&from, b"a").expect("deep file");
        let to = deep.join("b.md");
        if !renamed(&from, &to) {
            return;
        }
        std::fs::write(&from, b"again").expect("deep file again");
        let e = rename_no_replace(&from, &to).expect_err("taken name");
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists, "{e:?}");
        assert_eq!(std::fs::read(&to).expect("renamed file"), b"a");
    }

    #[test]
    fn a_link_is_renamed_as_itself() {
        let scratch = Scratch::new("rename");
        let target = folder_with(&scratch.0.join("target"), "inner.md", b"kept");
        let link = scratch.0.join("link");
        link_dir(&target, &link).expect("make link");
        let moved = scratch.0.join("moved-link");
        if !renamed(&link, &moved) {
            return;
        }
        assert!(
            std::fs::symlink_metadata(&moved)
                .expect("renamed link")
                .file_type()
                .is_symlink(),
            "the renamed entry is still the link"
        );
        assert_eq!(
            names(&target),
            ["inner.md"],
            "the target stays where it was"
        );
        assert_eq!(
            std::fs::read(moved.join("inner.md")).expect("read through the renamed link"),
            b"kept"
        );
    }

    #[test]
    fn a_missing_source_is_not_found() {
        let scratch = Scratch::new("rename");
        let e = rename_no_replace(&scratch.0.join("missing"), &scratch.0.join("to"))
            .expect_err("missing source");
        let expected = if NO_REPLACE {
            io::ErrorKind::NotFound
        } else {
            io::ErrorKind::Unsupported
        };
        assert_eq!(e.kind(), expected, "{e:?}");
    }

    // A live and a dangling link (its target removed) at the destination: refused, the link and its target as they were.
    #[test]
    fn a_taken_name_that_is_a_link_is_refused_and_the_link_stays() {
        let scratch = Scratch::new("rename");
        let target = folder_with(&scratch.0.join("target"), "inner.md", b"kept");
        let live = scratch.0.join("live");
        link_dir(&target, &live).expect("make link");
        let gone = folder_with(&scratch.0.join("gone"), "x.md", b"x");
        let dangling = scratch.0.join("dangling");
        link_dir(&gone, &dangling).expect("make link");
        std::fs::remove_dir_all(&gone).expect("remove the link's target");
        let file = scratch.0.join("file.md");
        std::fs::write(&file, b"source").expect("source file");
        let folder = folder_with(&scratch.0.join("folder"), "source.md", b"source");

        for to in [&live, &dangling] {
            for from in [&file, &folder] {
                let e = rename_no_replace(from, to).expect_err("a taken name is refused");
                assert_eq!(
                    e.kind(),
                    taken_kind(),
                    "{} -> {}: {e:?}",
                    from.display(),
                    to.display()
                );
            }
            assert!(
                std::fs::symlink_metadata(to)
                    .expect("the link stays")
                    .file_type()
                    .is_symlink(),
                "{} is still a link",
                to.display()
            );
        }
        assert_eq!(
            std::fs::read(live.join("inner.md")).expect("through the link"),
            b"kept"
        );
        assert_eq!(names(&target), ["inner.md"]);
        assert_eq!(std::fs::read(&file).expect("source file"), b"source");
        assert_eq!(names(&folder), ["source.md"]);
        assert_eq!(
            names(&scratch.0),
            ["dangling", "file.md", "folder", "live", "target"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_windows_flags_neither_replace_nor_copy() {
        use windows::Win32::Storage::FileSystem::{
            MOVEFILE_COPY_ALLOWED, MOVEFILE_REPLACE_EXISTING,
        };
        assert_eq!(
            NO_REPLACE_FLAGS.0 & (MOVEFILE_REPLACE_EXISTING.0 | MOVEFILE_COPY_ALLOWED.0),
            0
        );
    }

    // The mapping alone, with Linux's numbers — this machine cannot make the system call refuse its flag.
    #[test]
    fn a_refused_flag_reads_as_unsupported_keeping_the_os_error() {
        const ENOSYS: i32 = 38;
        const EOPNOTSUPP: i32 = 95;
        const EINVAL: i32 = 22;
        const EEXIST: i32 = 17;
        let codes = Refusals {
            refused: &[ENOSYS, EOPNOTSUPP],
            ambiguous: EINVAL,
        };
        let carried = |e: &io::Error| {
            e.get_ref()
                .and_then(|inner| inner.downcast_ref::<io::Error>())
                .and_then(io::Error::raw_os_error)
        };
        let (from, sibling, inside) = (
            Path::new("/lib/a"),
            Path::new("/lib/b"),
            Path::new("/lib/a/sub/a"),
        );
        for code in [ENOSYS, EOPNOTSUPP, EINVAL] {
            let e = refused_flag(io::Error::from_raw_os_error(code), from, sibling, &codes);
            assert_eq!(e.kind(), io::ErrorKind::Unsupported, "{code}");
            assert_eq!(e.raw_os_error(), None, "{code}: not an OS error of its own");
            assert_eq!(carried(&e), Some(code), "the OS code is carried inside");
        }
        for (code, to) in [(EINVAL, inside), (EEXIST, sibling)] {
            let e = refused_flag(io::Error::from_raw_os_error(code), from, to, &codes);
            assert_eq!(e.raw_os_error(), Some(code), "{code}: returned as is");
        }
    }

    // A folder on another volume: `ENGRAM_TEST_OTHER_VOLUME` names one (the temp folder's volume must differ).
    #[test]
    #[ignore = "needs a folder on a second volume, named by ENGRAM_TEST_OTHER_VOLUME"]
    fn another_volume_crosses_devices_and_nothing_moves() {
        let Some(other) = std::env::var_os("ENGRAM_TEST_OTHER_VOLUME") else {
            panic!(
                "set ENGRAM_TEST_OTHER_VOLUME to a folder on a volume other than the temp folder's"
            );
        };
        let here = Scratch::new("rename");
        let there = Scratch::at(Path::new(&other), "rename");
        let file = here.0.join("a.md");
        std::fs::write(&file, b"a").expect("file");
        let folder = folder_with(&here.0.join("dir"), "inner.md", b"inner");
        for from in [&file, &folder] {
            let to = there.0.join(from.file_name().expect("a name"));
            let e = rename_no_replace(from, &to).expect_err("another volume");
            assert_eq!(
                e.kind(),
                io::ErrorKind::CrossesDevices,
                "{}: {e:?}",
                from.display()
            );
            assert!(std::fs::symlink_metadata(&to).is_err(), "nothing arrives");
        }
        assert_eq!(names(&here.0), ["a.md", "dir"], "nothing leaves");
        assert_eq!(names(&folder), ["inner.md"]);
    }
}
