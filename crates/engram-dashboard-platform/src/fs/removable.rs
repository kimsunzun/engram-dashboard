//! Clearing what keeps a removal from going through (ADR-0409 decision 11), without following links (ADR-0365): the
//! read-only attribute below a folder on Windows; on Unix, where removability is the parent folder's write bit, each
//! folder's owner read, write and search bits (T-131 — a folder copied with mode 0555 keeps its children otherwise).
//! The whole tree ([`make_removable`]), the entry alone ([`make_entry_removable`]), or what a folder needs for its
//! entries to go ([`make_entries_removable`]). Other OSes: nothing is done.

// ADR-0409, ADR-0365
// T-131

use std::io;
use std::path::{Path, PathBuf};

/// Makes `path` and everything below it removable, so a removal std refused can go through: on Windows the read-only
/// attribute cleared, on Unix each folder's owner read, write and search bits added (a file's own mode never matters to
/// its removal and stays). Answers the entries it left: read-only links, read-only files with more than one hard link
/// (the attribute is shared with the other names, which may lie outside `path`), entries below `path` whose attributes
/// or mode could not be read or written, and names that were no longer a folder at the second check (whatever their
/// attribute).
///
/// - Windows: each entry is opened without following its last name (the folders above `path` resolve as usual — the
///   caller's no-follow checks cover them, ADR-0409), for reading its attributes only, and checked. A file to clear
///   is reopened for writing them from that handle (`ReOpenFile` — the same object, the path is not resolved again); a
///   folder to clear is opened again by its path, without following, and checked again on that handle before it is
///   written (a folder has no other names). So the entry changed is the entry checked. An entry below `path` whose
///   attributes cannot be read or written (access denied — an ACL, a name pending deletion), or a folder that is no
///   longer one at the second check, is left and answered; `path` itself unreadable is an error. A link (a
///   name-surrogate reparse point — junction, symbolic link; the line std's `remove_dir_all` draws) is neither entered
///   nor changed, `path` itself included. A folder is listed by its path
///   after its check, so a folder swapped for a link between that check and its listing is listed through the link (the
///   check-then-call race ADR-0409 accepts); each entry so listed is still opened without following and checked — what
///   is reached that way is a real entry on the other side.
/// - Unix: each entry is stat'ed without following; a folder is opened without following its last name
///   (`O_NOFOLLOW | O_DIRECTORY`) and its mode read and written through that handle (`fstat` · `fchmod`), so the
///   folder changed is the folder checked; a name that is no longer a folder at the open is left and answered, never
///   listed. A folder the process may not open for reading (its owner read bit missing — 0o000, 0o100, 0o300) gets
///   its mode, on Linux, through a path-only handle checked to hold a folder (`O_PATH | O_NOFOLLOW | O_DIRECTORY`,
///   the mode written through `/proc/self/fd/<n>`), so nothing but the folder checked is changed, and is opened
///   without following again before it is listed; other Unix systems leave it. A link is neither entered nor changed;
///   a file needs nothing. A folder whose mode this process may not write (not its owner), or one below `path` it may
///   not open, is left and answered; a left folder is still walked. As on Windows, a folder is listed by its path after
///   its check.
/// - A folder that cannot be listed (access denied) is left and answered, and the walk goes on. An entry that
///   disappears during the walk needs nothing; `path` missing = `NotFound`. Any other failure ends the walk with that
///   error, and what was cleared before stays cleared.
/// - Other OSes: does nothing and answers nothing.
#[cfg(any(windows, unix))]
pub fn make_removable(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut left = Vec::new();
    let mut pending = vec![(path.to_path_buf(), false)];
    while let Some((entry, below)) = pending.pop() {
        let vanished = |e: &io::Error| below && e.kind() == io::ErrorKind::NotFound;
        let visited = match visit(&entry) {
            Ok(visited) => visited,
            Err(e) if vanished(&e) => continue,
            Err(e) if below && denied(&e) => {
                left.push(entry);
                continue;
            }
            Err(e) => return Err(e),
        };
        let mut left_here = visited.left;
        if visited.folder {
            match std::fs::read_dir(&entry) {
                Ok(children) => {
                    for child in children {
                        match child {
                            Ok(child) => pending.push((child.path(), true)),
                            // The folder went while listed: nothing more to list.
                            Err(e) if e.kind() == io::ErrorKind::NotFound => break,
                            Err(e) => return Err(e),
                        }
                    }
                }
                Err(e) if vanished(&e) => {}
                // One folder that cannot be listed is left; the rest of the walk goes on.
                Err(e) if denied(&e) => left_here = true,
                Err(e) => return Err(e),
            }
        }
        if left_here {
            left.push(entry);
        }
    }
    Ok(left)
}
#[cfg(not(any(windows, unix)))]
pub fn make_removable(_path: &Path) -> io::Result<Vec<PathBuf>> {
    Ok(Vec::new())
}

/// [`make_removable`] for `path` alone: the entry itself is made removable as that walk makes each entry, and nothing
/// below it is visited — what a removal that must not change entries it does not remove needs (an empty folder that
/// may have gained entries meanwhile, ADR-0409 decision 11). Answers `[path]` when it was left, else nothing; `path`
/// missing = `NotFound`. Unix and other OSes: does nothing — on Unix an entry's own mode never matters to its removal
/// (its parent folder's write bit does: [`make_entries_removable`]).
// T-131
#[cfg(windows)]
pub fn make_entry_removable(path: &Path) -> io::Result<Vec<PathBuf>> {
    Ok(match visit(path)?.left {
        true => vec![path.to_path_buf()],
        false => Vec::new(),
    })
}
#[cfg(not(windows))]
pub fn make_entry_removable(_path: &Path) -> io::Result<Vec<PathBuf>> {
    Ok(Vec::new())
}

/// Makes the folder at `path` itself let its entries be removed, without following its last name: on Unix its owner
/// read, write and search bits added, as [`make_removable`] adds them; on Windows and other OSes nothing — a folder's
/// read-only attribute never keeps its entries from going. Answers whether the folder changed; a name that is no real
/// folder, or one this process may not change (not its owner), is left as it is (`false`).
// T-131
#[cfg(unix)]
pub fn make_entries_removable(path: &Path) -> io::Result<bool> {
    let checked = std::fs::symlink_metadata(path)?;
    if !checked.file_type().is_dir() {
        return Ok(false);
    }
    Ok(matches!(add_owner_rwx(path)?, Owner::Changed))
}
#[cfg(not(unix))]
pub fn make_entries_removable(_path: &Path) -> io::Result<bool> {
    Ok(false)
}

/// An entry below the walk's path this process may not reach: left and answered, the walk goes on.
#[cfg(windows)]
fn denied(e: &io::Error) -> bool {
    super::is_access_denied(e)
}
#[cfg(unix)]
fn denied(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied
}

/// The owner's read, write and search bits — what removing a folder's entries needs of it.
#[cfg(unix)]
const OWNER_RWX: u32 = 0o700;

#[cfg(unix)]
fn visit(path: &Path) -> io::Result<Visited> {
    let checked = std::fs::symlink_metadata(path)?;
    if !checked.file_type().is_dir() {
        return Ok(Visited {
            folder: false,
            left: false,
        });
    }
    Ok(match add_owner_rwx(path)? {
        Owner::Changed | Owner::Unchanged => Visited {
            folder: true,
            left: false,
        },
        // Not its owner (or no race-free way to change it): still a real folder, still walked.
        Owner::Left => Visited {
            folder: true,
            left: true,
        },
        // The name is a link or a file now: never listed — listing it would follow the link.
        Owner::NotAFolder => Visited {
            folder: false,
            left: true,
        },
    })
}

/// What [`add_owner_rwx`] did to a folder.
#[cfg(unix)]
enum Owner {
    Changed,
    /// It had the bits already.
    Unchanged,
    /// Still a real folder; this process may not change its mode (not its owner, or no race-free way on this OS).
    Left,
    /// The name is a link or a file now (or the mode could not be set without following); nothing written.
    NotAFolder,
}

/// Adds the owner's read, write and search bits to the folder at `path`, through a handle opened without following its
/// last name (`O_NOFOLLOW | O_DIRECTORY`) — or, when the folder cannot be opened for reading (its own read bit
/// missing), through a path-only handle ([`chmod_unreadable`]).
#[cfg(unix)]
fn add_owner_rwx(path: &Path) -> io::Result<Owner> {
    let folder = match open_folder_no_follow(path) {
        Ok(folder) => folder,
        Err(e) if not_a_folder(&e) => return Ok(Owner::NotAFolder),
        // T-131: an owned folder without its read bit (0o000, 0o100, 0o300) still takes a mode change.
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return chmod_unreadable(path),
        Err(e) => return Err(e),
    };
    add_through(&folder)
}

/// Adds the owner's bits to the folder `folder` holds, through that handle (`fstat` · `fchmod`): the folder changed is
/// the folder checked.
#[cfg(unix)]
fn add_through(folder: &std::fs::File) -> io::Result<Owner> {
    use std::os::unix::fs::PermissionsExt;
    let mode = folder.metadata()?.permissions().mode();
    if mode & OWNER_RWX == OWNER_RWX {
        return Ok(Owner::Unchanged);
    }
    let added = std::fs::Permissions::from_mode((mode | OWNER_RWX) & 0o7777);
    match folder.set_permissions(added) {
        Ok(()) => Ok(Owner::Changed),
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => Ok(Owner::Left),
        Err(e) => Err(e),
    }
}

#[cfg(unix)]
fn open_folder_no_follow(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(path)
}

/// The name is a link (`ELOOP`) or a file (`ENOTDIR`) at a no-follow folder open.
#[cfg(unix)]
fn not_a_folder(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(libc::ELOOP) | Some(libc::ENOTDIR))
}

/// Linux: adds the owner's bits to the folder at `path` that this process may not open for reading, race-free — a
/// path-only handle (`O_PATH | O_NOFOLLOW | O_DIRECTORY`, which needs no permission on the folder itself) is checked
/// to hold a folder (`fstat`), and its mode is changed through `/proc/self/fd/<n>` (`chmod` there reaches the object
/// the handle holds, never a name looked up again — what glibc's own no-follow `fchmodat` does); then the folder is
/// opened again without following for its listing. A name swapped for a file, a hard link or a link meanwhile is
/// never changed. Floor: Linux 3.6 (`fstat` on an `O_PATH` handle) with `/proc` mounted; without `/proc` the folder
/// is left.
#[cfg(target_os = "linux")]
fn chmod_unreadable(path: &Path) -> io::Result<Owner> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::os::unix::io::AsRawFd;
    let held = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(path)
    {
        Ok(held) => held,
        Err(e) if not_a_folder(&e) => return Ok(Owner::NotAFolder),
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Ok(Owner::Left),
        Err(e) => return Err(e),
    };
    let checked = held.metadata()?;
    if !checked.file_type().is_dir() {
        return Ok(Owner::NotAFolder);
    }
    let mode = checked.permissions().mode();
    if mode & OWNER_RWX != OWNER_RWX {
        let proc_path = std::ffi::CString::new(format!("/proc/self/fd/{}", held.as_raw_fd()))
            .expect("a number holds no NUL");
        let mode = ((mode | OWNER_RWX) & 0o7777) as libc::mode_t;
        // SAFETY: `proc_path` is NUL-terminated and outlives the call, which only reads it; `held` stays open across
        //   it.
        if unsafe { libc::chmod(proc_path.as_ptr(), mode) } != 0 {
            let e = io::Error::last_os_error();
            return match e.raw_os_error() {
                // Not its owner, or no `/proc`: left.
                Some(libc::EPERM) | Some(libc::EACCES) | Some(libc::ENOENT) => Ok(Owner::Left),
                _ => Err(e),
            };
        }
    }
    drop(held);
    match open_folder_no_follow(path) {
        Ok(_) => Ok(Owner::Changed),
        Err(e) if not_a_folder(&e) => Ok(Owner::NotAFolder),
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => Ok(Owner::Left),
        Err(e) => Err(e),
    }
}

/// Other Unix (macOS, the BSDs): no race-free way to change a folder this process may not open — it is left.
#[cfg(all(unix, not(target_os = "linux")))]
fn chmod_unreadable(_path: &Path) -> io::Result<Owner> {
    Ok(Owner::Left)
}

#[cfg(any(windows, unix))]
struct Visited {
    /// A real folder (not a link) — walk its children, whether or not its own attribute was cleared.
    folder: bool,
    /// Left as it is: a read-only link, a read-only file with other names, an entry whose attributes this process may not
    /// read or write, a name that is no longer a folder at the second check.
    left: bool,
}

#[cfg(windows)]
fn visit(path: &Path) -> io::Result<Visited> {
    let checked = open_no_follow(path, false)?;
    let (attributes, link) = classify(&checked)?;
    act(path, &checked, attributes, link)
}

/// Acts on what `checked` (open on `path`) was classified as. Split from [`visit`] so a test can change the name between
/// the two.
#[cfg(windows)]
fn act(path: &Path, checked: &std::fs::File, attributes: u32, link: bool) -> io::Result<Visited> {
    use windows::Win32::Storage::FileSystem::{
        FileStandardInfo, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_READONLY, FILE_STANDARD_INFO,
    };
    let read_only = attributes & FILE_ATTRIBUTE_READONLY.0 != 0;
    if link {
        return Ok(Visited {
            folder: false,
            left: read_only,
        });
    }
    if attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
        if !read_only {
            return Ok(Visited {
                folder: true,
                left: false,
            });
        }
        return Ok(match clear_folder(path)? {
            FolderClear::Cleared => Visited {
                folder: true,
                left: false,
            },
            FolderClear::Left => Visited {
                folder: true,
                left: true,
            },
            // The name is a link or a file now: never listed — listing it would follow the link.
            FolderClear::NotAFolder => Visited {
                folder: false,
                left: true,
            },
        });
    }
    let left = read_only && {
        let standard: FILE_STANDARD_INFO = super::file_info(checked, FileStandardInfo)?;
        standard.NumberOfLinks > 1 || !clear_file(checked)?
    };
    Ok(Visited {
        folder: false,
        left,
    })
}

/// The attributes of the entry `file` holds, and whether it is a link: a name-surrogate reparse point (junction,
/// symbolic link — `IsReparseTagNameSurrogate`).
#[cfg(windows)]
fn classify(file: &std::fs::File) -> io::Result<(u32, bool)> {
    use windows::Win32::Storage::FileSystem::{
        FileAttributeTagInfo, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
    };
    const NAME_SURROGATE: u32 = 0x2000_0000;
    let tag: FILE_ATTRIBUTE_TAG_INFO = super::file_info(file, FileAttributeTagInfo)?;
    let link = tag.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        && tag.ReparseTag & NAME_SURROGATE != 0;
    Ok((tag.FileAttributes, link))
}

// The spot itself, never what a link points to. Attribute access alone takes no part in sharing, so an entry someone
//   holds open is still reached; for classifying, read access alone, so an entry that needs no change is never refused
//   for write.
#[cfg(windows)]
fn open_no_follow(path: &Path, write: bool) -> io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{FILE_READ_ATTRIBUTES, FILE_WRITE_ATTRIBUTES};
    let access = if write {
        FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES
    } else {
        FILE_READ_ATTRIBUTES
    };
    std::fs::OpenOptions::new()
        .access_mode(access.0)
        .share_mode(SHARE_ALL)
        .custom_flags(NO_FOLLOW)
        .open(path)
}

#[cfg(windows)]
const SHARE_ALL: u32 = windows::Win32::Storage::FileSystem::FILE_SHARE_READ.0
    | windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE.0
    | windows::Win32::Storage::FileSystem::FILE_SHARE_DELETE.0;
// Backup semantics so a folder opens too.
#[cfg(windows)]
const NO_FOLLOW: u32 = windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT.0
    | windows::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS.0;

/// Clears the read-only attribute of the file `checked` holds, through a handle `ReOpenFile` opens on that same object —
/// no name is looked up again — writing back the attributes read on that handle. False = left: this process may not
/// write its attributes.
#[cfg(windows)]
fn clear_file(checked: &std::fs::File) -> io::Result<bool> {
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_READONLY;
    denied_as_left(reopen_to_write_attributes(checked).and_then(|writable| {
        let (attributes, _) = classify(&writable)?;
        if attributes & FILE_ATTRIBUTE_READONLY.0 == 0 {
            return Ok(());
        }
        write_cleared(&writable, attributes)
    }))
}

#[cfg(windows)]
fn reopen_to_write_attributes(checked: &std::fs::File) -> io::Result<std::fs::File> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        ReOpenFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_READ_ATTRIBUTES, FILE_SHARE_MODE,
        FILE_WRITE_ATTRIBUTES,
    };
    // SAFETY: `checked` is a handle open for this call; on success the returned handle is new and owned by nobody else.
    let handle = unsafe {
        ReOpenFile(
            HANDLE(checked.as_raw_handle()),
            (FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES).0,
            FILE_SHARE_MODE(SHARE_ALL),
            FILE_FLAGS_AND_ATTRIBUTES(NO_FOLLOW),
        )
    }
    .map_err(super::win32_io_error)?;
    // SAFETY: `handle` is a valid handle this function owns; the `File` takes it over and closes it.
    Ok(unsafe { std::fs::File::from_raw_handle(handle.0) })
}

/// Clears the read-only attribute of the folder at `path`. `ReOpenFile` refuses a folder (access denied — measured) and
/// `OpenFileById` needs file ids FAT and SMB do not give, so the folder is opened again by its path, without following,
/// and checked again on that write handle: written only if it is still a folder and not a link — a folder has no other
/// names, so the handle checked is the handle written.
#[cfg(windows)]
fn clear_folder(path: &Path) -> io::Result<FolderClear> {
    use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_READONLY};
    let writable = match open_no_follow(path, true) {
        Ok(writable) => writable,
        Err(e) if super::is_access_denied(&e) => return Ok(FolderClear::Left),
        Err(e) => return Err(e),
    };
    let (attributes, link) = classify(&writable)?;
    if link || attributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0 {
        return Ok(FolderClear::NotAFolder);
    }
    if attributes & FILE_ATTRIBUTE_READONLY.0 == 0 {
        return Ok(FolderClear::Cleared);
    }
    Ok(if denied_as_left(write_cleared(&writable, attributes))? {
        FolderClear::Cleared
    } else {
        FolderClear::Left
    })
}

#[cfg(windows)]
#[derive(Debug, PartialEq)]
enum FolderClear {
    Cleared,
    /// Still a real folder; its attributes may not be written (access denied).
    Left,
    /// The name is a link or a file now; nothing written.
    NotAFolder,
}

#[cfg(windows)]
fn denied_as_left(written: io::Result<()>) -> io::Result<bool> {
    match written {
        Ok(()) => Ok(true),
        Err(e) if super::is_access_denied(&e) => Ok(false),
        Err(e) => Err(e),
    }
}

// Every other attribute is passed back as it was (the file system ignores the ones it does not let a caller set); zero
//   would mean "leave unchanged", so an entry with no attribute left gets `FILE_ATTRIBUTE_NORMAL`. Zero times = unchanged.
#[cfg(windows)]
fn write_cleared(writable: &std::fs::File, attributes: u32) -> io::Result<()> {
    use windows::Win32::Storage::FileSystem::{
        FileBasicInfo, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_READONLY, FILE_BASIC_INFO,
    };
    let rest = attributes & !FILE_ATTRIBUTE_READONLY.0;
    let info = FILE_BASIC_INFO {
        FileAttributes: if rest == 0 {
            FILE_ATTRIBUTE_NORMAL.0
        } else {
            rest
        },
        ..FILE_BASIC_INFO::default()
    };
    super::set_file_info(writable, FileBasicInfo, &info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::{link_dir, Scratch};

    fn set_read_only(path: &Path, read_only: bool) {
        let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
        permissions.set_readonly(read_only);
        std::fs::set_permissions(path, permissions).expect("set read-only");
    }

    fn read_only(path: &Path) -> bool {
        std::fs::metadata(path)
            .expect("metadata")
            .permissions()
            .readonly()
    }

    /// A folder `name` under `parent` holding a read-only file `x.md`, a read-only subfolder `sub` with a read-only
    /// `y.md`, and itself read-only. Read-only last — on Unix a read-only folder takes no new entries.
    fn read_only_tree(parent: &Path, name: &str) -> PathBuf {
        let root = parent.join(name);
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).expect("folders");
        std::fs::write(root.join("x.md"), b"x").expect("file");
        std::fs::write(sub.join("y.md"), b"y").expect("file in subfolder");
        for path in [
            root.join("x.md"),
            sub.join("y.md"),
            sub.clone(),
            root.clone(),
        ] {
            set_read_only(&path, true);
        }
        root
    }

    // A Unix folder left read-only keeps its entries, which would keep the scratch folder from being removed.
    fn writable_again(root: &Path) {
        set_read_only(root, false);
        set_read_only(&root.join("sub"), false);
    }

    // T-131: on Unix a folder's own write bit is what its entries' removal needs, so the folders get it and the files
    //   keep their modes; on Windows every read-only attribute is cleared.
    #[test]
    fn read_only_entries_below_a_folder_are_cleared_on_windows_and_its_folders_made_writable_on_unix(
    ) {
        let scratch = Scratch::new("removable");
        let root = read_only_tree(&scratch.0, "row");
        let left = make_removable(&root).expect("make removable");
        assert!(left.is_empty(), "{left:?}");
        for path in [root.join("x.md"), root.join("sub").join("y.md")] {
            assert_eq!(read_only(&path), !cfg!(windows), "{}", path.display());
        }
        for path in [root.join("sub"), root.clone()] {
            assert!(!read_only(&path), "{}", path.display());
        }
        writable_again(&root);
    }

    // T-131: an owned folder without its read bit cannot be opened for reading; its mode is set by its name without
    //   following, and the walk lists it then — the row itself so too.
    #[cfg(target_os = "linux")]
    #[test]
    fn an_owned_folder_without_its_read_bit_still_gets_owner_rwx() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: `geteuid` reads the process's effective user id and has no preconditions.
        if unsafe { libc::geteuid() } == 0 {
            // Root (`CAP_DAC_OVERRIDE`) opens a 0o000 folder for reading, so the path-only fallback is never reached.
            eprintln!("skipped: running as root, the folders' read bits never stop an open");
            return;
        }
        let scratch = Scratch::new("removable");
        let row = scratch.0.join("row");
        let mut folders = Vec::new();
        for (name, mode) in [("none", 0o000), ("search", 0o100), ("write", 0o300)] {
            let folder = row.join(name);
            std::fs::create_dir_all(folder.join("inner")).expect("folders");
            std::fs::write(folder.join("x.md"), b"x").expect("file");
            folders.push((folder, mode));
        }
        for (folder, mode) in &folders {
            std::fs::set_permissions(folder, std::fs::Permissions::from_mode(*mode)).expect("mode");
        }
        std::fs::set_permissions(&row, std::fs::Permissions::from_mode(0o000)).expect("row mode");
        let mode = |path: &Path| {
            std::fs::symlink_metadata(path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777
        };

        let left = make_removable(&row).expect("make removable");

        assert!(left.is_empty(), "{left:?}");
        assert_eq!(mode(&row), 0o700);
        for (folder, before) in &folders {
            assert_eq!(mode(folder), before | 0o700, "{}", folder.display());
            assert_eq!(
                mode(&folder.join("inner")) & 0o700,
                0o700,
                "walked below it"
            );
        }
    }

    // A folder the walk may not list (an ACL denying everyone `FILE_LIST_DIRECTORY`) is left and answered; the walk
    //   goes on to the rest. The premise is checked through the listing the walk uses.
    #[cfg(windows)]
    #[test]
    fn a_folder_that_cannot_be_listed_is_left_and_the_walk_goes_on() {
        let scratch = Scratch::new("removable");
        let row = scratch.0.join("row");
        let (shut, other) = (row.join("shut"), row.join("other.md"));
        std::fs::create_dir_all(&shut).expect("folders");
        std::fs::write(&other, b"o").expect("file");
        set_read_only(&other, true);
        icacls(&shut, &["/deny", "*S-1-1-0:(RD)"]);
        let premise = std::fs::read_dir(&shut).map(|_| ());

        let left = make_removable(&row);
        icacls(&shut, &["/remove:d", "*S-1-1-0"]);

        assert!(
            premise.as_ref().is_err_and(super::super::is_access_denied),
            "premise: the listing is denied: {premise:?}"
        );
        assert_eq!(left.expect("the walk is not ended"), vec![shut.clone()]);
        assert!(!read_only(&other), "the walk went on past it");
    }

    // ADR-0409 decision 11: the entry alone is made removable — what lies below it keeps its attributes and modes. On
    //   Unix an entry's own mode never matters to its removal, so nothing changes.
    #[test]
    fn the_entry_alone_is_made_removable_and_nothing_below_it() {
        let scratch = Scratch::new("removable");
        let root = read_only_tree(&scratch.0, "row");

        let left = make_entry_removable(&root).expect("make the entry removable");

        assert!(left.is_empty(), "{left:?}");
        assert_eq!(
            read_only(&root),
            !cfg!(windows),
            "the entry itself, on Windows only"
        );
        for path in [
            root.join("x.md"),
            root.join("sub"),
            root.join("sub").join("y.md"),
        ] {
            assert!(read_only(&path), "{} is untouched", path.display());
        }
        writable_again(&root);
    }

    // T-131: on Unix a folder whose entries must go gets its owner bits (a 0555 shell); elsewhere a read-only folder
    //   never keeps its entries, so nothing changes.
    #[test]
    fn a_folder_lets_its_entries_go_on_unix_and_is_untouched_elsewhere() {
        let scratch = Scratch::new("removable");
        let shell = scratch.0.join("shell");
        std::fs::create_dir(&shell).expect("folder");
        std::fs::write(shell.join("x.md"), b"x").expect("file");
        set_read_only(&shell.join("x.md"), true);
        set_read_only(&shell, true);

        let changed = make_entries_removable(&shell).expect("let its entries go");

        assert_eq!(changed, cfg!(unix));
        assert_eq!(read_only(&shell), !cfg!(unix));
        assert!(read_only(&shell.join("x.md")), "its entries are untouched");
        assert!(
            !make_entries_removable(&shell.join("x.md")).expect("a file"),
            "a file is no folder"
        );
        set_read_only(&shell, false);
    }

    #[test]
    fn a_single_file_is_cleared_too() {
        let scratch = Scratch::new("removable");
        let file = scratch.0.join("x.md");
        std::fs::write(&file, b"x").expect("file");
        set_read_only(&file, true);
        assert!(make_removable(&file).expect("make removable").is_empty());
        assert_eq!(read_only(&file), !cfg!(windows));
    }

    // `keep` stands outside the folder with read-only entries; a link inside the folder points to it. Following the
    //   link would clear them.
    #[test]
    fn a_link_inside_is_neither_entered_nor_changed() {
        let scratch = Scratch::new("removable");
        let keep = read_only_tree(&scratch.0, "keep");
        let row = scratch.0.join("row");
        std::fs::create_dir(&row).expect("row folder");
        std::fs::write(row.join("a.md"), b"a").expect("file in row");
        set_read_only(&row.join("a.md"), true);
        let link = row.join("link");
        link_dir(&keep, &link).expect("make link");
        let link_read_only = || {
            std::fs::symlink_metadata(&link)
                .expect("the link itself")
                .permissions()
                .readonly()
        };
        let link_set = make_link_itself_read_only(&link);
        assert_eq!(
            link_read_only(),
            link_set,
            "premise: the link's own attribute"
        );

        let left = make_removable(&row).expect("make removable");
        assert_eq!(
            read_only(&row.join("a.md")),
            !cfg!(windows),
            "the row's own file"
        );
        for path in [
            keep.join("x.md"),
            keep.join("sub").join("y.md"),
            keep.join("sub"),
            keep.clone(),
        ] {
            assert!(read_only(&path), "{} is untouched", path.display());
        }
        assert_eq!(link_read_only(), link_set, "the link itself is unchanged");
        let link_left = if cfg!(windows) {
            vec![link.clone()]
        } else {
            vec![]
        };
        assert_eq!(left, link_left);

        // The link given as the path itself.
        let left = make_removable(&link).expect("make removable on the link");
        assert_eq!(left, link_left);
        assert_eq!(link_read_only(), link_set, "the link itself is unchanged");
        assert!(
            read_only(&keep.join("x.md")),
            "not entered through the path either"
        );
        assert!(read_only(&keep), "the target folder is untouched");
        writable_again(&keep);
    }

    /// Sets the read-only attribute of the link itself, not of its target — Windows only: a Unix symbolic link has no
    /// mode of its own to set. Answers whether it did.
    #[cfg(windows)]
    fn make_link_itself_read_only(link: &Path) -> bool {
        use windows::Win32::Storage::FileSystem::{
            FileBasicInfo, FILE_ATTRIBUTE_READONLY, FILE_BASIC_INFO,
        };
        let writable = open_no_follow(link, true).expect("open the link itself");
        let (attributes, link) = classify(&writable).expect("its attributes");
        assert!(link, "premise: the link itself is open");
        let info = FILE_BASIC_INFO {
            FileAttributes: attributes | FILE_ATTRIBUTE_READONLY.0,
            ..FILE_BASIC_INFO::default()
        };
        crate::fs::set_file_info(&writable, FileBasicInfo, &info).expect("set read-only");
        true
    }
    #[cfg(not(windows))]
    fn make_link_itself_read_only(_link: &Path) -> bool {
        false
    }

    #[test]
    fn a_missing_path_is_not_found_on_windows_and_unix_and_nothing_elsewhere() {
        let scratch = Scratch::new("removable");
        let outcome = make_removable(&scratch.0.join("missing"));
        if cfg!(any(windows, unix)) {
            assert_eq!(
                outcome.expect_err("missing").kind(),
                io::ErrorKind::NotFound
            );
        } else {
            assert!(outcome.expect("does nothing").is_empty());
        }
    }

    // An ACL denying everyone `FILE_WRITE_ATTRIBUTES` (`icacls`): a read-only entry so denied is left and answered; one
    //   that needs no change is not refused; the walk goes on to the rest. A name pending deletion (held open by a handle
    //   with the classic delete mark) cannot even be opened for its attributes: left and answered too. The premise is
    //   checked through the opens the walk uses — a process holding the backup privilege passes the deny.
    #[cfg(windows)]
    #[test]
    fn an_entry_whose_attributes_cannot_be_written_is_left_and_the_walk_goes_on() {
        let scratch = Scratch::new("removable");
        let row = scratch.0.join("row");
        std::fs::create_dir(&row).expect("row folder");
        let (plain, denied, other) = (
            row.join("plain.md"),
            row.join("denied.md"),
            row.join("other.md"),
        );
        // A denied read-only folder is left too, and still walked: the file inside is cleared.
        let sub = row.join("sub");
        std::fs::create_dir(&sub).expect("subfolder");
        let inner = sub.join("inner.md");
        for path in [&plain, &denied, &other, &inner] {
            std::fs::write(path, b"x").expect("file");
        }
        for path in [&denied, &other, &inner, &sub] {
            set_read_only(path, true);
        }
        for path in [&plain, &denied, &sub] {
            icacls(path, &["/deny", "*S-1-1-0:(WA)"]);
        }
        let pending = row.join("pending.md");
        std::fs::write(&pending, b"x").expect("file");
        let marked = mark_for_deletion(&pending);

        let denied_premise = [
            open_no_follow(&plain, false).and_then(|checked| reopen_to_write_attributes(&checked)),
            open_no_follow(&sub, true),
            open_no_follow(&pending, false),
        ];
        for premise in &denied_premise {
            assert!(
                premise.as_ref().is_err_and(super::super::is_access_denied),
                "premise: the open is denied: {premise:?}"
            );
        }

        let left = make_removable(&row);
        drop(marked);
        for path in [&plain, &denied, &sub] {
            icacls(path, &["/remove:d", "*S-1-1-0"]);
        }
        let mut left = left.expect("the walk is not ended");
        left.sort();
        assert_eq!(left, vec![denied.clone(), pending.clone(), sub.clone()]);
        assert!(read_only(&denied) && read_only(&sub), "left read-only");
        assert!(!read_only(&other), "the walk went on past it");
        assert!(!read_only(&inner), "the left folder was still walked");
        assert!(
            !read_only(&plain),
            "the entry that needed nothing is as it was"
        );
    }

    /// Holds `path` open with the classic delete mark: its name stays, pending deletion, until the handle closes.
    #[cfg(windows)]
    fn mark_for_deletion(path: &Path) -> std::fs::File {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Foundation::BOOLEAN;
        use windows::Win32::Storage::FileSystem::{
            FileDispositionInfo, DELETE, FILE_DISPOSITION_INFO,
        };
        let file = std::fs::OpenOptions::new()
            .access_mode(DELETE.0)
            .share_mode(SHARE_ALL)
            .open(path)
            .expect("open for deletion");
        let info = FILE_DISPOSITION_INFO {
            DeleteFile: BOOLEAN(1),
        };
        crate::fs::set_file_info(&file, FileDispositionInfo, &info).expect("delete mark");
        file
    }

    // The original name is taken by another read-only file between the check and the clear: the clear reaches the file
    //   checked (now under another name), never the newcomer. A folder is opened again by its path and checked again
    //   on that handle instead, so it has no such test.
    #[cfg(windows)]
    #[test]
    fn a_file_is_cleared_through_the_handle_checked_not_by_its_name() {
        let scratch = Scratch::new("removable");
        let original = scratch.0.join("x.md");
        std::fs::write(&original, b"checked").expect("file");
        set_read_only(&original, true);
        let checked = open_no_follow(&original, false).expect("check");
        let moved = scratch.0.join("moved.md");
        std::fs::rename(&original, &moved).expect("rename the checked file away");
        std::fs::write(&original, b"newcomer").expect("newcomer");
        set_read_only(&original, true);

        assert!(clear_file(&checked).expect("clear"));
        assert!(!read_only(&moved), "the file checked is cleared");
        assert!(read_only(&original), "the newcomer is untouched");
        assert_eq!(std::fs::read(&original).expect("newcomer"), b"newcomer");
    }

    // Another attribute set between the check and the clear (here: hidden) is kept — the write starts from the
    //   attributes read on the write handle.
    #[cfg(windows)]
    #[test]
    fn a_file_clear_keeps_an_attribute_set_after_the_check() {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        let scratch = Scratch::new("removable");
        let file = scratch.0.join("x.md");
        std::fs::write(&file, b"x").expect("file");
        set_read_only(&file, true);
        let checked = open_no_follow(&file, false).expect("check");
        let attributes = |path: &Path| std::fs::metadata(path).expect("metadata").file_attributes();
        set_hidden(&file, attributes(&file) | FILE_ATTRIBUTE_HIDDEN);

        assert!(clear_file(&checked).expect("clear"));
        assert!(!read_only(&file));
        assert_ne!(
            attributes(&file) & FILE_ATTRIBUTE_HIDDEN,
            0,
            "hidden is kept"
        );
    }

    // The folder checked is renamed away and a link to read-only `keep` takes its name before the act: the second
    //   check sees the link, so the name is reported left and not walked (listing it would follow the link), and
    //   nothing behind the link changes.
    #[cfg(windows)]
    #[test]
    fn a_folder_swapped_for_a_link_after_its_check_is_left_and_not_walked() {
        let scratch = Scratch::new("removable");
        let keep = read_only_tree(&scratch.0, "keep");
        let row = scratch.0.join("row");
        std::fs::create_dir(&row).expect("row folder");
        set_read_only(&row, true);
        let checked = open_no_follow(&row, false).expect("check");
        let (attributes, link) = classify(&checked).expect("classify");
        assert!(!link, "premise: a real folder");
        std::fs::rename(&row, scratch.0.join("moved")).expect("rename the folder away");
        link_dir(&keep, &row).expect("a link takes the name");

        let visited = act(&row, &checked, attributes, link).expect("act");
        assert!(!visited.folder, "not walked");
        assert!(visited.left, "reported left");
        for path in [
            keep.join("x.md"),
            keep.join("sub").join("y.md"),
            keep.join("sub"),
            keep.clone(),
        ] {
            assert!(read_only(&path), "{} is untouched", path.display());
        }
    }

    // Each guard of the second check alone: a read-only junction (a folder by its attributes, but a link) and a read-only
    //   file with two names (not a folder) at the name — neither is written.
    #[cfg(windows)]
    #[test]
    fn a_folder_clear_writes_neither_a_link_nor_a_file() {
        let scratch = Scratch::new("removable");
        let keep = read_only_tree(&scratch.0, "keep");
        let link = scratch.0.join("link");
        link_dir(&keep, &link).expect("make link");
        assert!(make_link_itself_read_only(&link));
        let link_read_only = || {
            std::fs::symlink_metadata(&link)
                .expect("the link itself")
                .permissions()
                .readonly()
        };
        assert!(link_read_only(), "premise: the link itself is read-only");
        assert_eq!(clear_folder(&link).expect("clear"), FolderClear::NotAFolder);
        assert!(link_read_only(), "the link stays read-only");
        assert!(
            read_only(&keep) && read_only(&keep.join("x.md")),
            "its target untouched"
        );

        let one = scratch.0.join("one.md");
        std::fs::write(&one, b"x").expect("file");
        let two = scratch.0.join("two.md");
        std::fs::hard_link(&one, &two).expect("second name");
        set_read_only(&one, true);
        assert_eq!(clear_folder(&one).expect("clear"), FolderClear::NotAFolder);
        assert!(
            read_only(&one) && read_only(&two),
            "both names stay read-only"
        );
        writable_again(&keep);
    }

    #[cfg(windows)]
    fn icacls(path: &Path, args: &[&str]) {
        let status = std::process::Command::new("icacls")
            .arg(path)
            .args(args)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("run icacls");
        assert!(
            status.success(),
            "icacls {} {args:?}: {status}",
            path.display()
        );
    }

    // `two.md` is another name of `one.md` and lies outside the folder: clearing `one.md` would clear `two.md` too.
    #[test]
    fn a_file_with_two_hard_links_is_skipped_and_reported() {
        let scratch = Scratch::new("removable");
        let row = scratch.0.join("row");
        std::fs::create_dir(&row).expect("row folder");
        let one = row.join("one.md");
        std::fs::write(&one, b"shared").expect("file");
        let two = scratch.0.join("two.md");
        std::fs::hard_link(&one, &two).expect("second name");
        let single = row.join("single.md");
        std::fs::write(&single, b"single").expect("single-name file");
        for path in [&one, &single] {
            set_read_only(path, true);
        }

        let left = make_removable(&row).expect("make removable");
        assert!(
            read_only(&one) && read_only(&two),
            "the shared attribute stays"
        );
        assert_eq!(
            read_only(&single),
            !cfg!(windows),
            "the walk went on past it"
        );
        assert_eq!(
            left,
            if cfg!(windows) {
                vec![one.clone()]
            } else {
                vec![]
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn clearing_read_only_keeps_every_other_attribute() {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        let scratch = Scratch::new("removable");
        let root = read_only_tree(&scratch.0, "row");
        let file = root.join("x.md");
        let attributes = |path: &Path| std::fs::metadata(path).expect("metadata").file_attributes();
        set_hidden(&file, attributes(&file) | FILE_ATTRIBUTE_HIDDEN);
        let (file_before, folder_before) = (attributes(&file), attributes(&root));
        assert_ne!(file_before & FILE_ATTRIBUTE_HIDDEN, 0, "premise: hidden");

        make_removable(&root).expect("make removable");
        assert_eq!(attributes(&file), file_before & !FILE_ATTRIBUTE_READONLY);
        assert_eq!(attributes(&root), folder_before & !FILE_ATTRIBUTE_READONLY);
    }

    #[cfg(windows)]
    fn set_hidden(path: &Path, attributes: u32) {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{SetFileAttributesW, FILE_FLAGS_AND_ATTRIBUTES};
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call, which only reads it.
        unsafe { SetFileAttributesW(PCWSTR(wide.as_ptr()), FILE_FLAGS_AND_ATTRIBUTES(attributes)) }
            .expect("set attributes");
    }
}
