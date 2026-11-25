//! The OS rules for handling files — opening while denying others' writes and classifying that failure · the verdict
//! and retry for a failure caused by someone holding the file briefly · folder sync · creating and walking directory
//! links (`link`) · path keys and path comparison (`path_key`) · folder-path normalization and the Win32
//! reserved-device-name test (`normalize`) · file identity (`identity`) · the network share a path is on
//! (`share_root`) · a rename that never replaces (`rename` — `rename_no_replace`) · making what is below a folder
//! removable without following links (`removable` — `make_removable` · `make_entry_removable` ·
//! `make_entries_removable`) · a folder's modified time (`times` —
//! `set_folder_modified`) · opening a file for reading without following its last name (`open_read_no_follow`) · a
//! path as forward-slash text (`forward_slash_text`). The opens in this file use std only —
//! `OpenOptionsExt::share_mode` goes down as is to `CreateFileW`'s share mode, so the same kernel behaviour comes
//! without unsafe.

// ADR-0266, ADR-0291

use std::fs::File;
use std::io;
use std::path::Path;
use std::time::Duration;

mod identity;
mod link;
mod normalize;
mod path_key;
mod removable;
mod rename;
mod times;

pub use identity::{file_identity, opened_identity, FileIdentity};
pub use link::{link_dir, unlink_dir};
pub use normalize::{is_reserved_device_name, normalize_folder, NotAbsolute};
pub use path_key::{path_key, same_path, PathKey};
pub use removable::{make_entries_removable, make_entry_removable, make_removable};
pub use rename::rename_no_replace;
pub use times::set_folder_modified;

const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_SHARING_VIOLATION: i32 = 32;
const ERROR_LOCK_VIOLATION: i32 = 33;

/// Opens for read+write, creating the file if missing; existing content is not truncated. While this handle is open, other opens are
///   shared **read only** — an open requesting write or delete (rename included) fails with a sharing violation
/// ([`is_sharing_violation`]); conversely, if someone already has the file open with write access or blocking write sharing, this open
///   fails with a sharing violation. A read open where the other side shares writing (the default of `std::fs::read`)
/// opens alongside; a read open sharing only reading collides with this handle's write access (measured 2026-08-14 — ADR-0135). A
///   read-only-**attribute** file fails with access denied ([`is_access_denied`]), not a sharing violation.
/// Outside Windows = fails with `ErrorKind::Unsupported` without trying (no means to apply this sharing restriction); that error has no OS
///   error code (`raw_os_error() == None`), which tells it from an OS-emitted `Unsupported`.
#[cfg(windows)]
pub fn open_deny_write(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    // `FILE_SHARE_READ` alone — adding `WRITE` or `DELETE` would let others' write opens and deletes succeed and what this function blocks
    //   would vanish.
    const FILE_SHARE_READ: u32 = 1;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
}
#[cfg(not(windows))]
pub fn open_deny_write(_path: &Path) -> io::Result<File> {
    Err(io::ErrorKind::Unsupported.into())
}

/// Opens the file at `path` for reading without following its last name — the folders above it resolve as usual. The
/// caller checks what it opened on the handle (`File::metadata`, [`opened_identity`]) before it reads.
///
/// - Windows: opened first as itself (`FILE_FLAG_OPEN_REPARSE_POINT` with `FILE_FLAG_BACKUP_SEMANTICS`, so a folder
///   opens too). A link there (a name-surrogate reparse point, a junction included) is answered as so opened — the
///   handle's metadata says link. Any other reparse point (a cloud placeholder, a deduplicated file — data a file-system
///   filter supplies, which a handle opened as itself would read raw) is opened again by its path, followed (it is no
///   link), and that handle is answered only when it has the identity of the first; otherwise an error, nothing read.
///   (A `compact /exe` WOF file reads the same either way here: the filter hides its reparse point — measured
///   2026-11-24.)
/// - Other OSes: `O_NOFOLLOW` — a link there is refused (`ELOOP`); a folder opens.
// ADR-0365
#[cfg(windows)]
pub fn open_read_no_follow(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{FileAttributeTagInfo, FILE_ATTRIBUTE_TAG_INFO};
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    let open = |flags: u32| {
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(flags)
            .open(path)
    };
    let itself = open(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)?;
    let tag: FILE_ATTRIBUTE_TAG_INFO = file_info(&itself, FileAttributeTagInfo)?;
    if !filtered_reparse_point(tag.FileAttributes, tag.ReparseTag) {
        return Ok(itself);
    }
    let followed = open(FILE_FLAG_BACKUP_SEMANTICS)?;
    match (opened_identity(&itself), opened_identity(&followed)) {
        (Ok(first), Ok(second)) if first == second => Ok(followed),
        _ => Err(io::Error::other(
            "a reparse point opened again is not provably the entry opened first",
        )),
    }
}

/// Whether attributes and a reparse tag (`FILE_ATTRIBUTE_TAG_INFO`) are of a reparse point that is no link — one a
/// file-system filter answers for, whose data a handle opened as itself may not show.
#[cfg(windows)]
fn filtered_reparse_point(attributes: u32, reparse_tag: u32) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const NAME_SURROGATE: u32 = 0x2000_0000;
    attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 && reparse_tag & NAME_SURROGATE == 0
}
#[cfg(unix)]
pub fn open_read_no_follow(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

/// Was it blocked by the sharing restriction of a handle someone else holds (Windows `ERROR_SHARING_VIOLATION`); once that handle closes
///   the same open can succeed. Outside Windows = always false.
/// std classifies this code as `ErrorKind::Uncategorized`, so `kind()` cannot tell it (measured 2026-08-14).
pub fn is_sharing_violation(e: &io::Error) -> bool {
    cfg!(windows) && e.raw_os_error() == Some(ERROR_SHARING_VIOLATION)
}

/// Was access itself denied (Windows `ERROR_ACCESS_DENIED` — ACL denial, and opening a read-only-attribute file for writing). Unlike a
///   sharing violation it does not clear when another handle closes. Outside Windows = always false —
/// other OSes' `PermissionDenied` does not count.
pub fn is_access_denied(e: &io::Error) -> bool {
    cfg!(windows) && e.raw_os_error() == Some(ERROR_ACCESS_DENIED)
}

/// Is it a failure caused by someone briefly holding the file — antivirus/indexers make open, read and rename fail like this and the same
///   attempt can succeed after they let go; [`retry_busy`] uses this. Counts: `ErrorKind::PermissionDenied`
/// (all OSes; Windows `ERROR_ACCESS_DENIED` is this kind too) · Windows `ERROR_SHARING_VIOLATION`(32) · `ERROR_LOCK_VIOLATION`(33).
/// ★Unlike [`is_access_denied`], access denied counts as locked★ — never-clearing denials (read-only attribute, ACLs) are mixed in, so
///   those failures come back late, by [`retry_busy`]'s budget.
/// ★32 and 33 cannot be caught by kind★ — std maps only 5 to `PermissionDenied`, leaving those two unnamed, so the OS code is read. On
///   other OSes those numbers are different errors (Linux 32 = `EPIPE`).
// ADR-0291: the lock decision · retry are a single platform policy.
pub fn is_busy(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied
        || (cfg!(windows)
            && matches!(
                e.raw_os_error(),
                Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
            ))
}

/// The budget of [`retry_busy`] — up to `retries` retries after the first attempt, waiting `pause` before each; worst-case extra wait =
///   `retries × pause`.
#[derive(Clone, Copy, Debug)]
pub struct Retry {
    pub retries: u32,
    pub pause: Duration,
}

/// Does `attempt` once and, if [`is_busy`], sleeps `retry.pause` and retries — at most `retry.retries + 1` attempts. Success and non-lock
///   errors return at once; no sleep after the last attempt; on budget exhaustion the last lock error
/// is returned as is. No logs (header rule 4).
pub fn retry_busy<T>(retry: Retry, attempt: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    retry_busy_with(retry, attempt, std::thread::sleep)
}

/// Same as [`retry_busy`] but calls `pause` instead of sleeping (passing `retry.pause` before each retry) — the seam letting a test count
///   retries without sleeping.
pub fn retry_busy_with<T>(
    retry: Retry,
    mut attempt: impl FnMut() -> io::Result<T>,
    mut pause: impl FnMut(Duration),
) -> io::Result<T> {
    let mut retries = 0;
    loop {
        match attempt() {
            Err(e) if is_busy(&e) && retries < retry.retries => {
                retries += 1;
                pause(retry.pause);
            }
            outcome => return outcome,
        }
    }
}

/// Persists one rename inside a folder to disk — called after an atomic write's rename. Outside Windows = opens the folder and `sync_all`
///   (missing folder = `NotFound`); Windows = nothing, `Ok` even if the folder is missing.
// ADR-0291
#[cfg(windows)]
pub fn sync_dir(_dir: &Path) -> io::Result<()> {
    // std `File::open` cannot open folders on Windows (`PermissionDenied` · raw 5 — measured 2026-10-10 · rustc 1.95).
    Ok(())
}
#[cfg(not(windows))]
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// The root of the network share `path` is on, or `None`. Windows: `\\server\share\` for a UNC path (`\\server\share\…`
/// or `\\?\UNC\server\share\…`); `X:\` for a drive letter whose drive type is remote (a mapped network drive), or that
/// has no volume behind it but a network connection Windows remembers for the letter (a mapped drive that is not
/// connected — it may answer another drive type than remote); `None` for a relative path (`Z:child` included), any
/// other drive letter (local, removable, absent) or another prefix. Other systems: `None`.
/// No stat of `path` — for a drive letter, only the drive-type query and, for a letter with no volume, the remembered
/// connection query (`WNetGetConnectionW`). Whether these two calls contact the server for a disconnected drive is not
/// confirmed (TRD R9); this function puts no time bound on them.
/// A caller stats the answer to tell an absent folder from a share that is not reachable (Windows answers
/// `NotFound` for both).
// ADR-0387
#[cfg(windows)]
pub fn share_root(path: &Path) -> Option<std::path::PathBuf> {
    share_root_with(path, |letter, root| {
        network_drive(drive_type(root), || remembered_connection(letter))
    })
}
#[cfg(not(windows))]
pub fn share_root(_path: &Path) -> Option<std::path::PathBuf> {
    None
}

/// `path` as text with `/` as its separator: on Windows each `\` becomes `/` (`D:\Library\hooks` →
/// `D:/Library/hooks`, `\\srv\team$\x` → `//srv/team$/x`); other systems keep it as spelled. A path that is not
/// UTF-8 comes back lossy (`U+FFFD` for what cannot be read). No I/O.
// ADR-0420
pub fn forward_slash_text(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text.into_owned()
    }
}

/// [`share_root`] with the drive-letter question (`network(letter, "X:\")` — whether that drive is a network drive)
/// passed in, so the path rules are tested without a mapped drive.
#[cfg(windows)]
fn share_root_with(
    path: &Path,
    network: impl FnOnce(u8, &str) -> bool,
) -> Option<std::path::PathBuf> {
    use std::path::{Component, Prefix};
    // A drive-relative path (`Z:child`) has a `Disk` prefix too — it names no folder of its own.
    if !path.is_absolute() {
        return None;
    }
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return None;
    };
    match prefix.kind() {
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
            let mut root = std::ffi::OsString::from(r"\\");
            root.push(server);
            root.push(r"\");
            root.push(share);
            root.push(r"\");
            Some(root.into())
        }
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
            let root = format!(r"{}:\", char::from(letter));
            network(letter, &root).then(|| root.into())
        }
        _ => None,
    }
}

/// `GetDriveTypeW`'s answers this file reads — the constants sit in the `Win32_System_WindowsProgramming` binding
/// feature, which this crate does not turn on for three numbers.
#[cfg(windows)]
const DRIVE_UNKNOWN: u32 = 0;
#[cfg(windows)]
const DRIVE_NO_ROOT_DIR: u32 = 1;
#[cfg(windows)]
const DRIVE_REMOTE: u32 = 4;

/// Whether a drive letter of drive type `kind` is a network drive: remote, or — with no volume behind the letter
/// (unknown / no root) — a letter with a remembered network connection (`remembered`, asked only then). A letter with a
/// local volume is never a mapped one, so it is not asked.
#[cfg(windows)]
fn network_drive(kind: u32, remembered: impl FnOnce() -> bool) -> bool {
    match kind {
        DRIVE_REMOTE => true,
        DRIVE_UNKNOWN | DRIVE_NO_ROOT_DIR => remembered(),
        _ => false,
    }
}

/// The drive type of drive root `root` (`X:\`).
#[cfg(windows)]
fn drive_type(root: &str) -> u32 {
    use windows::core::PCWSTR;
    let wide = wide_nul(root);
    // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the call; the call only reads it.
    unsafe { windows::Win32::Storage::FileSystem::GetDriveTypeW(PCWSTR(wide.as_ptr())) }
}

/// Whether Windows holds a network connection for drive letter `letter` (`X:`), connected or remembered but not
/// connected (`ERROR_CONNECTION_UNAVAIL`). Any other answer — not redirected, no network provider, a failure — is no.
#[cfg(windows)]
fn remembered_connection(letter: u8) -> bool {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{ERROR_CONNECTION_UNAVAIL, ERROR_MORE_DATA, NO_ERROR};
    let local = wide_nul(&format!("{}:", char::from(letter)));
    let mut remote = [0u16; 512];
    let mut len = remote.len() as u32;
    // SAFETY: `local` is NUL-terminated and `remote` holds `len` UTF-16 units; both outlive the call, which writes at
    //   most `len` units into `remote` and the needed size into `len`.
    let answer = unsafe {
        windows::Win32::NetworkManagement::WNet::WNetGetConnectionW(
            PCWSTR(local.as_ptr()),
            PWSTR(remote.as_mut_ptr()),
            &mut len,
        )
    };
    answer == NO_ERROR || answer == ERROR_CONNECTION_UNAVAIL || answer == ERROR_MORE_DATA
}

/// `text` as a NUL-terminated UTF-16 string.
#[cfg(windows)]
fn wide_nul(text: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// One `GetFileInformationByHandleEx` query of `file` — `class` is the info class whose struct is `T`.
#[cfg(windows)]
fn file_info<T: Default>(
    file: &File,
    class: windows::Win32::Storage::FileSystem::FILE_INFO_BY_HANDLE_CLASS,
) -> io::Result<T> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::GetFileInformationByHandleEx;
    let mut info = T::default();
    // SAFETY: `file` is a handle open for this call, and the buffer is a `T` of exactly the size passed — every caller
    //   passes the info class and its struct as a pair.
    unsafe {
        GetFileInformationByHandleEx(
            HANDLE(file.as_raw_handle()),
            class,
            (&mut info as *mut T).cast(),
            std::mem::size_of::<T>() as u32,
        )
    }
    .map_err(win32_io_error)?;
    Ok(info)
}

/// One `SetFileInformationByHandle` call on `file` — `class` is the info class whose struct is `T`.
#[cfg(windows)]
fn set_file_info<T>(
    file: &File,
    class: windows::Win32::Storage::FileSystem::FILE_INFO_BY_HANDLE_CLASS,
    info: &T,
) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::SetFileInformationByHandle;
    // SAFETY: `file` is a handle open for this call, and `info` is a `T` of exactly the size passed — every caller
    //   passes the info class and its struct as a pair.
    unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            class,
            (info as *const T).cast(),
            std::mem::size_of::<T>() as u32,
        )
    }
    .map_err(win32_io_error)
}

// A FACILITY_WIN32 HRESULT (`0x8007xxxx`) turns back into the original Win32 code in its low 16 bits — carrying the HRESULT as the
//   OS code would make `kind()` `Uncategorized`, unreadable to the classifiers above.
#[cfg(windows)]
fn win32_io_error(e: windows::core::Error) -> io::Error {
    let hresult = e.code().0 as u32;
    if hresult & 0xFFFF_0000 == 0x8007_0000 {
        io::Error::from_raw_os_error((hresult & 0xFFFF) as i32)
    } else {
        io::Error::other(e)
    }
}

/// A fresh folder for one test (`engram-platform-<tag>-<pid>-<uuid>`), removed with all it holds when dropped — also
/// when an assertion fails midway; best effort (a removal error is ignored). std `remove_dir_all` does not follow links.
#[cfg(test)]
struct Scratch(std::path::PathBuf);

#[cfg(test)]
impl Scratch {
    fn new(tag: &str) -> Self {
        Self::at(&std::env::temp_dir(), tag)
    }

    fn at(parent: &Path, tag: &str) -> Self {
        let dir = parent.join(format!(
            "engram-platform-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&dir).expect("test folder");
        Self(dir)
    }
}

#[cfg(test)]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ADR-0420
    #[test]
    fn forward_slash_text_turns_windows_separators_and_keeps_other_spellings() {
        let spelled = r"D:\Library\hooks\notify";
        let expected = if cfg!(windows) {
            "D:/Library/hooks/notify"
        } else {
            spelled
        };
        assert_eq!(forward_slash_text(Path::new(spelled)), expected);
        let share = r"\\srv\team$\x";
        let expected = if cfg!(windows) {
            "//srv/team$/x"
        } else {
            share
        };
        assert_eq!(forward_slash_text(Path::new(share)), expected);
        assert_eq!(forward_slash_text(Path::new("/srv/a b/x")), "/srv/a b/x");
    }

    // ADR-0365: a file opens and reads; a link at the name is never opened through — opened as itself (its metadata says
    //   link) or refused.
    #[test]
    fn a_no_follow_open_reads_a_file_and_never_opens_through_a_link() {
        let scratch = Scratch::new("no-follow");
        let file = scratch.0.join("a.md");
        std::fs::write(&file, b"alpha").unwrap();
        let mut read = String::new();
        std::io::Read::read_to_string(&mut open_read_no_follow(&file).unwrap(), &mut read).unwrap();
        assert_eq!(read, "alpha");

        let target = scratch.0.join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("inside.md"), b"i").unwrap();
        let link = scratch.0.join("link");
        link_dir(&target, &link).unwrap();
        let opened = open_read_no_follow(&link).map(|file| file.metadata().unwrap().file_type());
        unlink_dir(&link).unwrap();
        if let Ok(kind) = opened {
            assert!(
                kind.is_symlink(),
                "opened as itself, not as its target: {kind:?}"
            );
        }
        let folder = open_read_no_follow(&target).unwrap();
        assert!(folder.metadata().unwrap().is_dir(), "a folder opens as one");
    }

    // A reparse point that is no link (WOF `0x80000017`, a cloud placeholder `0x9000101A`, dedup `0x80000013`) is opened
    //   again followed; a name surrogate (a junction `0xA0000003`, a symbolic link `0xA000000C`) and a plain file never.
    #[cfg(windows)]
    #[test]
    fn only_a_reparse_point_that_is_no_link_is_opened_again() {
        const REPARSE: u32 = 0x400;
        for tag in [0x8000_0017, 0x9000_101A, 0x8000_0013] {
            assert!(filtered_reparse_point(REPARSE | 0x20, tag), "{tag:#x}");
        }
        for tag in [0xA000_0003, 0xA000_000C] {
            assert!(!filtered_reparse_point(REPARSE, tag), "{tag:#x}");
        }
        assert!(!filtered_reparse_point(0x20, 0), "a plain file");
    }

    // A file compressed by `compact /exe` (a WOF reparse point under its filter) reads the same bytes through the
    //   no-follow open as through a plain open. Skipped where `compact` is not there or does not compress.
    #[cfg(windows)]
    #[test]
    fn a_wof_compressed_file_reads_its_own_bytes_through_the_no_follow_open() {
        let scratch = Scratch::new("wof");
        let file = scratch.0.join("w.txt");
        let bytes: Vec<u8> = (0..200_000u32).map(|n| (n % 251) as u8).collect();
        std::fs::write(&file, &bytes).unwrap();
        let compressed = std::process::Command::new("compact")
            .args(["/c", "/exe:xpress4k"])
            .arg(&file)
            .output()
            .is_ok_and(|done| done.status.success());
        if !compressed {
            eprintln!("compact unavailable: skipped");
            return;
        }
        let mut read = Vec::new();
        std::io::Read::read_to_end(&mut open_read_no_follow(&file).unwrap(), &mut read).unwrap();
        assert_eq!(read, bytes);
    }

    #[test]
    fn raw_codes_are_classified_only_on_windows() {
        let sharing = io::Error::from_raw_os_error(ERROR_SHARING_VIOLATION);
        let denied = io::Error::from_raw_os_error(ERROR_ACCESS_DENIED);
        assert_eq!(is_sharing_violation(&sharing), cfg!(windows));
        assert_eq!(is_access_denied(&denied), cfg!(windows));
        assert!(!is_sharing_violation(&denied));
        assert!(!is_access_denied(&sharing));
        // An error of the same kind without a code is neither — it is told apart only by the raw code.
        assert!(!is_access_denied(&io::ErrorKind::PermissionDenied.into()));
        assert!(!is_sharing_violation(&io::ErrorKind::Other.into()));
    }

    #[cfg(not(windows))]
    #[test]
    fn open_deny_write_refuses_without_an_os_code() {
        let e = open_deny_write(Path::new(".")).expect_err("an OS without the means does not open");
        assert_eq!(e.kind(), io::ErrorKind::Unsupported);
        assert_eq!(e.raw_os_error(), None);
    }

    #[cfg(windows)]
    fn fresh_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "engram-platform-fs-{}-{}.tmp",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    #[cfg(windows)]
    #[test]
    fn it_creates_a_missing_file_and_keeps_existing_content() {
        let path = fresh_path();
        drop(open_deny_write(&path).expect("a missing file is created and opened"));
        assert!(path.exists());

        std::fs::write(&path, b"kept").expect("write the content first");
        drop(open_deny_write(&path).expect("opens an existing file"));
        assert_eq!(
            std::fs::read(&path).expect("read"),
            b"kept",
            "must not truncate"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[cfg(windows)]
    #[test]
    fn while_held_a_second_writer_is_refused_with_a_sharing_violation() {
        let path = fresh_path();
        let held = open_deny_write(&path).expect("first open");

        let e = open_deny_write(&path).expect_err("a second write open must be blocked");
        assert!(
            is_sharing_violation(&e),
            "must be a sharing violation: {e:?}"
        );
        assert!(
            !is_access_denied(&e),
            "must not read as access denied: {e:?}"
        );

        drop(held);
        drop(open_deny_write(&path).expect("opens again once released"));
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(windows)]
    #[test]
    fn while_held_a_permissive_reader_opens_but_delete_and_rename_are_refused() {
        let path = fresh_path();
        let held = open_deny_write(&path).expect("open");

        assert!(
            std::fs::read(&path).is_ok(),
            "the std default read must open"
        );
        let e = std::fs::remove_file(&path).expect_err("delete must be blocked");
        assert!(
            is_sharing_violation(&e),
            "delete refusal = sharing violation: {e:?}"
        );
        let moved = path.with_extension("moved");
        let e = std::fs::rename(&path, &moved).expect_err("rename must be blocked too");
        assert!(
            is_sharing_violation(&e),
            "rename refusal = sharing violation: {e:?}"
        );

        drop(held);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&moved);
    }

    #[cfg(windows)]
    #[test]
    fn a_read_only_attribute_is_access_denied_not_a_sharing_violation() {
        let path = fresh_path();
        std::fs::write(&path, b"").expect("create file");
        let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms.clone()).expect("read-only attribute");

        let e = open_deny_write(&path).expect_err("the read-only attribute blocks a write open");
        assert!(is_access_denied(&e), "must be access denied: {e:?}");
        assert!(
            !is_sharing_violation(&e),
            "must not read as a sharing violation: {e:?}"
        );

        perms.set_readonly(false);
        let _ = std::fs::set_permissions(&path, perms);
        let _ = std::fs::remove_file(&path);
    }

    // ── Lock decision · retry ──

    const RETRY: Retry = Retry {
        retries: 5,
        pause: Duration::from_millis(20),
    };

    fn denied() -> io::Error {
        io::Error::from(io::ErrorKind::PermissionDenied)
    }

    #[test]
    fn a_brief_denial_is_retried_until_it_clears() {
        let mut failures = 2;
        let mut pauses = Vec::new();
        let outcome = retry_busy_with(
            RETRY,
            || {
                if failures > 0 {
                    failures -= 1;
                    Err(denied())
                } else {
                    Ok(())
                }
            },
            |pause| pauses.push(pause),
        );
        outcome.unwrap();
        assert_eq!(
            pauses,
            vec![RETRY.pause; 2],
            "the budget's interval before each retry"
        );
    }

    #[test]
    fn a_lasting_denial_gives_up_after_the_bound() {
        let mut attempts = 0;
        let mut pauses = 0;
        let outcome = retry_busy_with(
            RETRY,
            || -> io::Result<()> {
                attempts += 1;
                Err(denied())
            },
            |_| pauses += 1,
        );
        assert_eq!(outcome.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!((attempts, pauses), (RETRY.retries + 1, RETRY.retries));
    }

    #[test]
    fn windows_sharing_and_lock_violations_are_retried_too() {
        for code in [ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION] {
            let mut failures = 2;
            let mut pauses = 0;
            let outcome = retry_busy_with(
                RETRY,
                || {
                    if failures > 0 {
                        failures -= 1;
                        Err(io::Error::from_raw_os_error(code))
                    } else {
                        Ok(())
                    }
                },
                |_| pauses += 1,
            );
            if cfg!(windows) {
                outcome.unwrap();
                assert_eq!(pauses, 2, "{code}");
            } else {
                assert_eq!(outcome.unwrap_err().raw_os_error(), Some(code));
                assert_eq!(pauses, 0, "{code}: not a lock on other OSes");
            }
        }
    }

    #[test]
    fn a_lasting_sharing_violation_gives_up_after_the_same_bound() {
        let mut attempts = 0;
        let outcome = retry_busy_with(
            RETRY,
            || -> io::Result<()> {
                attempts += 1;
                Err(io::Error::from_raw_os_error(ERROR_SHARING_VIOLATION))
            },
            |_| {},
        );
        assert_eq!(
            outcome.unwrap_err().raw_os_error(),
            Some(ERROR_SHARING_VIOLATION)
        );
        let expected = if cfg!(windows) { RETRY.retries + 1 } else { 1 };
        assert_eq!(attempts, expected);
    }

    #[test]
    fn other_errors_are_not_retried() {
        let mut attempts = 0;
        let outcome = retry_busy_with(
            RETRY,
            || -> io::Result<()> {
                attempts += 1;
                Err(io::Error::from(io::ErrorKind::NotFound))
            },
            |_| panic!("does not wait"),
        );
        assert_eq!(outcome.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(attempts, 1);
    }

    // ── Folder sync ──

    #[test]
    fn sync_dir_syncs_a_folder_and_a_missing_one_fails_only_off_windows() {
        let dir = std::env::temp_dir().join(format!(
            "engram-platform-fs-dir-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&dir).expect("create folder");
        sync_dir(&dir).expect("existing folder");
        std::fs::remove_dir(&dir).expect("clean up folder");

        let missing = sync_dir(&dir);
        if cfg!(windows) {
            missing.expect("Windows is a no-op, so even a missing folder is Ok");
        } else {
            assert_eq!(missing.unwrap_err().kind(), io::ErrorKind::NotFound);
        }
    }

    // ── share root ──

    /// A UNC path (plain or verbatim) answers its `\\server\share\`; a local fixed drive, a drive letter with nothing
    /// behind it, a relative path (`X:child` included) and a device path answer `None` (ADR-0387). The drive letters
    /// are picked from this machine: a case with no such letter is skipped with a message.
    #[cfg(windows)]
    #[test]
    fn the_share_root_is_the_unc_volume_and_none_for_a_local_or_absent_drive() {
        use std::path::PathBuf;
        /// `GetDriveTypeW`'s answer for a local fixed disk.
        const DRIVE_FIXED: u32 = 3;
        for (path, root) in [
            (r"\\nohost\share\x\y", r"\\nohost\share\"),
            (r"\\nohost\share", r"\\nohost\share\"),
            (r"\\?\UNC\nohost\share\x", r"\\nohost\share\"),
        ] {
            assert_eq!(
                share_root(Path::new(path)),
                Some(PathBuf::from(root)),
                "{path}"
            );
        }
        assert_eq!(share_root(Path::new(r"rel\x")), None);
        assert_eq!(share_root(Path::new(r"\\.\D:\x")), None);
        let letters = || (b'A'..=b'Z').map(|letter| (letter, format!(r"{}:\", char::from(letter))));
        match letters().find(|(_, root)| drive_type(root) == DRIVE_FIXED) {
            Some((letter, root)) => {
                assert_eq!(share_root(&Path::new(&root).join("x")), None, "{root}");
                let relative = format!("{}:child", char::from(letter));
                assert_eq!(share_root(Path::new(&relative)), None, "{relative}");
            }
            None => eprintln!("skipped: no local fixed drive letter on this machine"),
        }
        match letters().find(|(letter, root)| {
            drive_type(root) == DRIVE_NO_ROOT_DIR && !remembered_connection(*letter)
        }) {
            Some((letter, root)) => {
                assert_eq!(share_root(&Path::new(&root).join("x")), None, "{root}");
                let relative = format!("{}:child", char::from(letter));
                assert_eq!(share_root(Path::new(&relative)), None, "{relative}");
            }
            None => eprintln!("skipped: every drive letter is in use on this machine"),
        }
    }

    /// On a network drive letter, an absolute path answers the drive root and a drive-relative one (`Z:child`, `Z:`)
    /// answers `None` before the drive is asked (ADR-0387).
    #[cfg(windows)]
    #[test]
    fn a_drive_relative_path_has_no_share_root_even_on_a_network_drive() {
        use std::path::PathBuf;
        let network = |letter: u8, root: &str| {
            assert_eq!((letter, root), (b'Z', r"Z:\"));
            true
        };
        assert_eq!(
            share_root_with(Path::new(r"Z:\lib\x"), network),
            Some(PathBuf::from(r"Z:\"))
        );
        assert_eq!(
            share_root_with(Path::new(r"\\?\Z:\lib"), network),
            Some(PathBuf::from(r"Z:\"))
        );
        let never = |_: u8, _: &str| -> bool { panic!("a relative path does not ask the drive") };
        for relative in ["Z:child", "Z:", r"Z:child\x"] {
            assert_eq!(
                share_root_with(Path::new(relative), never),
                None,
                "{relative}"
            );
        }
    }

    /// A remote drive type is a network drive; a letter with no volume is one only with a remembered connection; a
    /// letter with a local volume is not, and its connection is not asked (ADR-0387).
    #[cfg(windows)]
    #[test]
    fn a_letter_with_no_volume_is_a_network_drive_only_with_a_remembered_connection() {
        let never = || -> bool { panic!("a letter with a local volume is not asked") };
        assert!(network_drive(DRIVE_REMOTE, never));
        assert!(network_drive(DRIVE_NO_ROOT_DIR, || true));
        assert!(network_drive(DRIVE_UNKNOWN, || true));
        assert!(!network_drive(DRIVE_NO_ROOT_DIR, || false));
        assert!(!network_drive(DRIVE_UNKNOWN, || false));
        // Removable · fixed · CD-ROM · RAM disk.
        for kind in [2, 3, 5, 6] {
            assert!(!network_drive(kind, never), "{kind}");
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn there_is_no_share_root_outside_windows() {
        assert_eq!(share_root(Path::new("//server/share/x")), None);
        assert_eq!(share_root(Path::new("/mnt/x")), None);
    }

    // ── carrying windows failures over ──

    #[cfg(windows)]
    #[test]
    fn a_wrapped_win32_failure_keeps_its_code_and_kind() {
        use windows::core::{Error, HRESULT};
        let not_found = win32_io_error(Error::from(HRESULT(0x8007_0002_u32 as i32)));
        assert_eq!(not_found.raw_os_error(), Some(2));
        assert_eq!(not_found.kind(), io::ErrorKind::NotFound);
        let sharing = win32_io_error(Error::from(HRESULT(0x8007_0020_u32 as i32)));
        assert!(is_sharing_violation(&sharing));

        let foreign = win32_io_error(Error::from(HRESULT(0x8000_4005_u32 as i32))); // E_FAIL
        assert_eq!(
            foreign.raw_os_error(),
            None,
            "an HRESULT that is not Win32 is not carried as an OS code"
        );
    }
}
