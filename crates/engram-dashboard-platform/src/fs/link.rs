//! Directory links — making · removing. Windows = directory junction (no admin rights or developer mode needed) · elsewhere = symbolic link.

// ADR-0336

use std::io;
use std::path::Path;

/// Makes a directory link at `link` pointing to the folder `target` — Windows = directory junction · elsewhere = symbolic link.
///
/// - `target` must be absolute and contain no NUL — otherwise `InvalidInput` and nothing is made. It need not exist (the link is
///   then broken).
/// - Anything already at `link` = `AlreadyExists`, and it is left as is.
/// - A Windows junction stands only on volumes that take reparse points (NTFS etc.).
/// - Any other failure on Windows may leave an empty folder at `link` — it is not deleted (it could be someone else's folder). The
///   users work inside a volatile folder, which is cleared as a whole.
/// - On Windows, no permission to open the folder at the link = access denied (raw 5 — [`super::is_access_denied`]).
#[cfg(windows)]
pub fn link_dir(target: &Path, link: &Path) -> io::Result<()> {
    check_target(target)?;
    check_link(link)?;
    junction::create(target, link).map_err(junction_error)
}
#[cfg(unix)]
pub fn link_dir(target: &Path, link: &Path) -> io::Result<()> {
    check_target(target)?;
    std::os::unix::fs::symlink(target, link)
}

/// Removes a link made by [`link_dir`] — only the link goes; the target folder and its contents stay. A broken link (target gone) is
/// removed too.
///
/// - `link` not a link = `InvalidInput` and nothing is deleted. Windows counts only junctions as links — a real folder · file ·
///   directory symbolic link · other reparse points such as cloud placeholders are refused. So is a real folder someone is using or
///   that cannot be deleted (if even its attributes cannot be read, that open error). Elsewhere only symbolic links count.
/// - Missing = `NotFound`.
/// - Windows deletes, through the same handle, exactly the object it opened and confirmed to be a junction — another object taking
///   that name after the check is not deleted (object identity). While held, sharing is read-only, so others cannot rename it or
///   open it for write · delete; but a handle asking only `FILE_WRITE_ATTRIBUTES` skips the sharing check, and per MS-FSA that right
///   can strip the reparse point (`FSCTL_DELETE_REPARSE_POINT`). If someone strips it between check and delete, the deleted object
///   is the same one — now an empty folder (a non-empty folder fails the delete mark). Unverified — that race was not measured.
/// - On Windows, someone holding that junction open without delete sharing or with write · delete access = sharing violation (raw
///   32 — [`super::is_sharing_violation`]); no permission to delete = access denied (raw 5).
/// - Outside Windows, check and delete are not one action (a file changed in between can be deleted).
/// - A Windows volume mount point has the same reparse tag as a junction and is not told apart (unverified — only admins make mount
///   points).
#[cfg(windows)]
pub fn unlink_dir(link: &Path) -> io::Result<()> {
    unlink_junction(link).map(|_| ())
}
#[cfg(unix)]
pub fn unlink_dir(link: &Path) -> io::Result<()> {
    if !std::fs::symlink_metadata(link)?.file_type().is_symlink() {
        return Err(not_a_link());
    }
    // A symbolic link is removed as a file even when it points to a folder — `remove_dir` gives `ENOTDIR`.
    std::fs::remove_file(link)
}

// Finding the name again by path, as `std::fs::remove_dir` does, would delete someone else's empty folder that took the name after
//   the check — so delete through the checked handle. It is a handle on the reparse point itself, so only the link goes and the
//   target is untouched. `junction::delete` strips only the reparse data and leaves an empty folder, and `junction::exists` follows
//   the link and says `false` for a broken junction, so neither can check. The returned disposition lets a test tell when the name
//   of a junction held open elsewhere disappears.
#[cfg(windows)]
fn unlink_junction(link: &Path) -> io::Result<Disposition> {
    let junction = open_junction(link)?;
    delete_on_close(&junction)
}

fn check_target(target: &Path) -> io::Result<()> {
    if !target.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "link target must be an absolute path",
        ));
    }
    // `junction::create` passes a NUL through unchecked, and the OS links the path cut before it (2.1.0 `get_full_path`).
    if target.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "link target contains NUL",
        ));
    }
    Ok(())
}

// `symlink_metadata` below also refuses NUL with `InvalidInput`, but its message does not say which argument — check first to name
//   the link path.
#[cfg(windows)]
fn check_link(link: &Path) -> io::Result<()> {
    if link.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "link path contains NUL",
        ));
    }
    match std::fs::symlink_metadata(link) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "something already exists at the link path",
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn not_a_link() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "not a directory link")
}

// junction 2.1.0, when opening the link folder is access denied, tries turning on the symbolic-link privilege, and if the token
//   lacks it returns `ERROR_NOT_ALL_ASSIGNED` (1300) instead of the original denial (`helpers.rs` source) — 1300 arises only there.
#[cfg(windows)]
fn junction_error(e: io::Error) -> io::Error {
    const ERROR_NOT_ALL_ASSIGNED: i32 = 1300;
    if e.raw_os_error() == Some(ERROR_NOT_ALL_ASSIGNED) {
        io::Error::from_raw_os_error(super::ERROR_ACCESS_DENIED)
    } else {
        e
    }
}

// Opens the spot itself, without following the link, with the right to delete it, and reads the reparse tag — std does not expose
//   the tag. Why sharing is read-only and what that does not block = the `unlink_dir` doc.
#[cfg(windows)]
fn open_junction(path: &Path) -> io::Result<std::fs::File> {
    use windows::Win32::Storage::FileSystem::{
        DELETE, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let file = match open_reparse_point(path, (DELETE | FILE_READ_ATTRIBUTES).0, FILE_SHARE_READ.0)
    {
        Ok(file) => file,
        // Failing to open with delete access may mean a real folder that is in use or cannot be deleted — read the tag through an
        //   attributes-only handle: not a link = `InvalidInput`; a link, or even that fails = the original error. Never delete through that handle.
        Err(e) if super::is_access_denied(&e) || super::is_sharing_violation(&e) => {
            let probed = open_reparse_point(
                path,
                FILE_READ_ATTRIBUTES.0,
                (FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0,
            )
            .and_then(|probe| is_junction(&probe));
            return Err(match probed {
                Ok(false) => not_a_link(),
                _ => e,
            });
        }
        Err(e) => return Err(e),
    };
    if is_junction(&file)? {
        Ok(file)
    } else {
        Err(not_a_link())
    }
}

#[cfg(windows)]
fn open_reparse_point(path: &Path, access: u32, share: u32) -> io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    };
    std::fs::OpenOptions::new()
        .access_mode(access)
        .share_mode(share)
        .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
        .open(path)
}

#[cfg(windows)]
fn is_junction(file: &std::fs::File) -> io::Result<bool> {
    use windows::Win32::Storage::FileSystem::{
        FileAttributeTagInfo, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
    };
    // Not turning on `Win32_System_SystemServices` for this one constant.
    const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;

    let info: FILE_ATTRIBUTE_TAG_INFO = super::file_info(file, FileAttributeTagInfo)?;
    Ok(info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        && info.ReparseTag == IO_REPARSE_TAG_MOUNT_POINT)
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Disposition {
    Posix,
    Classic,
}

// Only sets the delete mark; the deletion happens when `file` closes. The POSIX form removes the name the moment this handle closes —
//   even if someone else has the junction open. The classic form keeps the name until the other handles close, so deleting the
//   parent folder next fails with "directory not empty" (145) (measured 2026-11-01 · Windows 11 26200 NTFS). Errors from a Windows
//   or file system that does not know the POSIX form (1 · 50 · 87) fall back to the classic form (unverified — this PC's NTFS takes it).
#[cfg(windows)]
fn delete_on_close(file: &std::fs::File) -> io::Result<Disposition> {
    const ERROR_INVALID_FUNCTION: i32 = 1;
    const ERROR_NOT_SUPPORTED: i32 = 50;
    const ERROR_INVALID_PARAMETER: i32 = 87;
    match posix_delete_on_close(file) {
        Ok(()) => Ok(Disposition::Posix),
        Err(e)
            if matches!(
                e.raw_os_error(),
                Some(ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED | ERROR_INVALID_PARAMETER)
            ) =>
        {
            classic_delete_on_close(file).map(|()| Disposition::Classic)
        }
        Err(e) => Err(e),
    }
}

// Also deletes a junction with the read-only attribute — the same flag std `remove_dir_all` uses. The classic form lacks this flag, so
//   a read-only junction is access denied there (unverified). A Windows that does not know the flag is assumed to give 87 and fall back (unverified).
#[cfg(windows)]
fn posix_delete_on_close(file: &std::fs::File) -> io::Result<()> {
    use windows::Win32::Storage::FileSystem::{
        FileDispositionInfoEx, FILE_DISPOSITION_FLAG_DELETE,
        FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
        FILE_DISPOSITION_INFO_EX, FILE_DISPOSITION_INFO_EX_FLAGS,
    };
    let info = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_INFO_EX_FLAGS(
            FILE_DISPOSITION_FLAG_DELETE.0
                | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS.0
                | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE.0,
        ),
    };
    super::set_file_info(file, FileDispositionInfoEx, &info)
}

#[cfg(windows)]
fn classic_delete_on_close(file: &std::fs::File) -> io::Result<()> {
    use windows::Win32::Foundation::BOOLEAN;
    use windows::Win32::Storage::FileSystem::{FileDispositionInfo, FILE_DISPOSITION_INFO};
    let info = FILE_DISPOSITION_INFO {
        DeleteFile: BOOLEAN(1),
    };
    super::set_file_info(file, FileDispositionInfo, &info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::{file_identity, Scratch};
    use std::path::PathBuf;

    impl Scratch {
        fn target_with_file(&self) -> PathBuf {
            let target = self.0.join("target");
            std::fs::create_dir(&target).expect("target folder");
            std::fs::write(target.join("inner.txt"), b"kept").expect("file inside target");
            target
        }
    }

    fn gone(path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
    }

    #[test]
    fn a_link_reads_through_to_the_target_and_unlinking_leaves_the_target_intact() {
        let scratch = Scratch::new("link");
        let target = scratch.target_with_file();
        let link = scratch.0.join("link");

        link_dir(&target, &link).expect("make link");
        assert_eq!(
            std::fs::read(link.join("inner.txt")).expect("read through link"),
            b"kept"
        );
        assert_eq!(
            file_identity(&link).expect("link identity"),
            file_identity(&target).expect("target identity"),
            "the identity follows the link"
        );

        unlink_dir(&link).expect("unlink");
        assert!(gone(&link), "the link path must be empty");
        assert_eq!(
            std::fs::read(target.join("inner.txt")).expect("file inside target"),
            b"kept",
            "the target stays"
        );
    }

    #[test]
    fn unlink_refuses_a_real_directory_and_a_file() {
        let scratch = Scratch::new("link");
        let real = scratch.target_with_file();
        let empty = scratch.0.join("empty");
        std::fs::create_dir(&empty).expect("empty folder");
        let file = scratch.0.join("file.txt");
        std::fs::write(&file, b"f").expect("file");

        for path in [&real, &empty, &file] {
            let e = unlink_dir(path).expect_err("refused when not a link");
            assert_eq!(
                e.kind(),
                io::ErrorKind::InvalidInput,
                "{}: {e}",
                path.display()
            );
            assert!(path.exists(), "{} stays", path.display());
        }
        assert!(real.join("inner.txt").exists());
    }

    #[test]
    fn a_broken_link_is_still_unlinked() {
        let scratch = Scratch::new("link");
        let target = scratch.target_with_file();
        let link = scratch.0.join("link");
        link_dir(&target, &link).expect("make link");
        std::fs::remove_dir_all(&target).expect("delete target");

        unlink_dir(&link).expect("a broken link is unlinked too");
        assert!(gone(&link));
    }

    #[test]
    fn unlinking_a_missing_path_is_not_found() {
        let scratch = Scratch::new("link");
        let e = unlink_dir(&scratch.0.join("missing")).expect_err("missing path");
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_relative_target_is_refused_and_nothing_is_made() {
        let scratch = Scratch::new("link");
        let link = scratch.0.join("link");
        let e = link_dir(Path::new("relative/target"), &link).expect_err("relative path refused");
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
        assert!(gone(&link));
    }

    #[test]
    fn an_occupied_link_path_is_left_alone() {
        let scratch = Scratch::new("link");
        let target = scratch.target_with_file();
        let occupied = scratch.0.join("occupied");
        std::fs::create_dir(&occupied).expect("existing folder");
        std::fs::write(occupied.join("mine.txt"), b"m").expect("file inside it");
        let empty = scratch.0.join("empty");
        std::fs::create_dir(&empty).expect("existing empty folder");

        let e = link_dir(&target, &occupied).expect_err("occupied path");
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert!(occupied.join("mine.txt").exists(), "what was there stays");

        let e = link_dir(&target, &empty).expect_err("occupied empty path");
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert!(
            std::fs::symlink_metadata(&empty).is_ok_and(|m| m.is_dir()),
            "the existing empty folder stays a folder, not a link"
        );
        assert!(
            std::fs::read_dir(&empty)
                .expect("read empty folder")
                .next()
                .is_none(),
            "still empty"
        );
    }

    #[test]
    fn a_target_with_nul_is_refused_and_nothing_is_made() {
        let scratch = Scratch::new("link");
        let link = scratch.0.join("link");
        let e = link_dir(&scratch.0.join("tar\0get"), &link).expect_err("NUL refused");
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{e}");
        assert!(gone(&link));
    }

    #[test]
    fn unlinking_keeps_everything_under_the_target() {
        let scratch = Scratch::new("link");
        let target = scratch.target_with_file();
        let nested = target.join("nested");
        std::fs::create_dir(&nested).expect("inner folder");
        std::fs::write(nested.join("deep.txt"), b"deep").expect("file in inner folder");
        let identity = file_identity(&target).expect("target identity");
        let link = scratch.0.join("link");
        link_dir(&target, &link).expect("make link");

        unlink_dir(&link).expect("unlink");
        assert!(gone(&link));
        assert_eq!(
            file_identity(&target).expect("identity after unlink"),
            identity
        );
        assert_eq!(
            std::fs::read(target.join("inner.txt")).expect("file inside target"),
            b"kept"
        );
        assert_eq!(
            std::fs::read(nested.join("deep.txt")).expect("file in inner folder"),
            b"deep"
        );
    }

    // Checks only the mapping by raw code — the test does not create the spot where 1300 arises (link folder ACL denial + a token without the privilege).
    #[cfg(windows)]
    #[test]
    fn a_privilege_failure_from_junction_reads_as_access_denied() {
        let mapped = junction_error(io::Error::from_raw_os_error(1300));
        assert_eq!(
            mapped.raw_os_error(),
            Some(super::super::ERROR_ACCESS_DENIED)
        );
        assert!(super::super::is_access_denied(&mapped));
        assert_eq!(mapped.kind(), io::ErrorKind::PermissionDenied);

        let other = junction_error(io::Error::from_raw_os_error(3));
        assert_eq!(other.raw_os_error(), Some(3), "other errors stay as is");
    }

    // A 5000-character target does not fit the junction buffer (16 KiB) and fails junction 2.1.0's length check (`InvalidInput`
    //   without an OS code). That check runs after the link folder is made, so an empty folder stays — measuring it catches a revived cleanup that deletes.
    #[cfg(windows)]
    #[test]
    fn a_too_long_target_fails_and_deletes_nothing() {
        let scratch = Scratch::new("link");
        let sibling = scratch.target_with_file();
        let too_long = scratch.0.join("x".repeat(5000));
        let link = scratch.0.join("link");

        let e = link_dir(&too_long, &link).expect_err("target over the buffer");
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{e:?}");
        assert_eq!(e.raw_os_error(), None);
        assert!(
            std::fs::symlink_metadata(&link).is_ok_and(|m| m.is_dir()),
            "a folder that is not a link stays at the link path"
        );
        assert!(
            std::fs::read_dir(&link)
                .expect("read the remaining folder")
                .next()
                .is_none(),
            "still empty"
        );
        assert_eq!(
            std::fs::read(sibling.join("inner.txt")).expect("file in the sibling folder"),
            b"kept",
            "the sibling stays"
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_classic_disposition_also_removes_only_the_link() {
        let scratch = Scratch::new("link");
        let target = scratch.target_with_file();
        let link = scratch.0.join("link");
        link_dir(&target, &link).expect("make link");

        let junction = open_junction(&link).expect("open junction");
        classic_delete_on_close(&junction).expect("classic delete mark");
        drop(junction);
        assert!(gone(&link));
        assert_eq!(
            std::fs::read(target.join("inner.txt")).expect("file inside target"),
            b"kept"
        );
    }

    // Someone else opens the spot itself for read (or read+write) — an attributes-only handle takes no part in the sharing check
    //   (measured), so with such a handle the tests below measure nothing.
    #[cfg(windows)]
    fn hold(path: &Path, write: bool, share: u32) -> std::fs::File {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        };
        std::fs::OpenOptions::new()
            .read(true)
            .write(write)
            .share_mode(share)
            .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
            .open(path)
            .expect("someone else opens the spot")
    }

    // Measures why the POSIX form was chosen (`delete_on_close` comment) — even with the junction itself held open elsewhere, the name
    //   disappears at once on unlink and the parent folder can be deleted. On a volume that fell back to the classic form the name goes when that handle closes.
    #[cfg(windows)]
    #[test]
    fn a_junction_held_open_elsewhere_is_gone_once_unlinked() {
        use windows::Win32::Storage::FileSystem::{
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        let scratch = Scratch::new("link");
        let target = scratch.target_with_file();
        let parent = scratch.0.join("parent");
        std::fs::create_dir(&parent).expect("parent folder");
        let link = parent.join("link");
        link_dir(&target, &link).expect("make link");
        let held = hold(
            &link,
            false,
            (FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0,
        );

        match unlink_junction(&link).expect("unlink") {
            Disposition::Posix => {
                assert!(
                    gone(&link),
                    "the name must go even with the other handle open"
                );
                std::fs::remove_dir(&parent).expect("the parent folder must be deletable");
                drop(held);
            }
            Disposition::Classic => {
                drop(held);
                assert!(gone(&link), "the name must go once the other handle closes");
                std::fs::remove_dir(&parent).expect("the parent folder must be deletable");
            }
        }
        assert!(target.join("inner.txt").exists());
    }

    #[cfg(windows)]
    #[test]
    fn unlink_is_a_sharing_violation_while_someone_holds_the_junction_without_delete_sharing() {
        use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
        let scratch = Scratch::new("link");
        let target = scratch.target_with_file();
        let link = scratch.0.join("link");
        link_dir(&target, &link).expect("make link");
        let held = hold(&link, false, FILE_SHARE_READ.0);

        let e = unlink_dir(&link).expect_err("sharing violation");
        assert!(super::super::is_sharing_violation(&e), "{e:?}");
        drop(held);
        unlink_dir(&link).expect("unlinks once released");
        assert!(gone(&link));
    }

    // The unlink handle shares read only — a junction someone opened for writing with every share is a sharing violation too.
    #[cfg(windows)]
    #[test]
    fn unlink_is_a_sharing_violation_while_someone_holds_the_junction_for_writing() {
        use windows::Win32::Storage::FileSystem::{
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        let scratch = Scratch::new("link");
        let target = scratch.target_with_file();
        let link = scratch.0.join("link");
        link_dir(&target, &link).expect("make link");
        let held = hold(
            &link,
            true,
            (FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0,
        );

        let e = unlink_dir(&link).expect_err("sharing violation");
        assert!(super::super::is_sharing_violation(&e), "{e:?}");
        drop(held);
        unlink_dir(&link).expect("unlinks once released");
        assert!(gone(&link));
    }

    // A real folder someone opened without delete sharing (another process's working folder is one) cannot be opened with delete
    //   access, but it is not a link, so `InvalidInput` — the first assertion measures that the open is really blocked.
    #[cfg(windows)]
    #[test]
    fn a_busy_real_directory_is_refused_as_not_a_link() {
        use windows::Win32::Storage::FileSystem::{
            DELETE, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        let scratch = Scratch::new("link");
        let real = scratch.target_with_file();
        let held = hold(&real, false, (FILE_SHARE_READ | FILE_SHARE_WRITE).0);
        let blocked =
            open_reparse_point(&real, (DELETE | FILE_READ_ATTRIBUTES).0, FILE_SHARE_READ.0)
                .expect_err("premise: the delete-access open is blocked");
        assert!(
            super::super::is_sharing_violation(&blocked),
            "premise: {blocked:?}"
        );

        let e = unlink_dir(&real).expect_err("refused when not a link");
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{e:?}");
        drop(held);
        assert!(real.join("inner.txt").exists(), "the real folder stays");
    }

    // A directory symbolic link needs developer mode or the privilege — on a PC that cannot make one there is nothing to check.
    #[cfg(windows)]
    #[test]
    fn a_directory_symlink_is_not_taken_for_a_junction() {
        let scratch = Scratch::new("link");
        let target = scratch.target_with_file();
        let symlink = scratch.0.join("symlink");
        if let Err(e) = std::os::windows::fs::symlink_dir(&target, &symlink) {
            eprintln!("cannot create a directory symbolic link, skipping: {e}");
            return;
        }
        let e = unlink_dir(&symlink).expect_err("refused when not a junction");
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
        assert!(
            std::fs::symlink_metadata(&symlink).is_ok(),
            "the symbolic link stays"
        );
    }
}
