//! OS rules for handling files — opening while blocking others' writes and classifying its failures · deciding failures caused by someone
//!   briefly holding a file and retrying · folder sync · a rename that never replaces. All but [`rename_no_replace`] use only std —
//! `OpenOptionsExt::share_mode` goes straight to `CreateFileW`'s share mode; the no-replace rename is `MoveFileExW` (std's `rename`
//!   always replaces on Windows).

// ADR-0266, ADR-0291

use std::fs::File;
use std::io;
use std::path::Path;
use std::time::Duration;

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

/// Renames `from` to `to` only if nothing is at `to` — otherwise `AlreadyExists` and both stay as they were. Windows = `MoveFileExW`
///   without `MOVEFILE_REPLACE_EXISTING` (with `MOVEFILE_WRITE_THROUGH`). A briefly held file fails as [`is_busy`] — retrying is the
///   caller's. Outside Windows = `Unsupported` without trying (defined: `renameat2` with `RENAME_NOREPLACE` — T-118 / T-119).
// ADR-0266, ADR-0389
#[cfg(windows)]
pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    let from = crate::win::to_wide(from.as_os_str())?;
    let to = crate::win::to_wide(to.as_os_str())?;
    // SAFETY: two null-terminated wide paths alive for the call.
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(crate::win::io_error)
}
#[cfg(not(windows))]
pub fn rename_no_replace(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn a_rename_never_replaces_an_existing_file() {
        let dir = std::env::temp_dir().join(format!("engram-fs-rename-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b, c) = (dir.join("a"), dir.join("b"), dir.join("c"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        let e = rename_no_replace(&a, &b).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&a).unwrap(), b"a");
        assert_eq!(std::fs::read(&b).unwrap(), b"b");
        rename_no_replace(&a, &c).unwrap();
        assert!(!a.exists());
        assert_eq!(std::fs::read(&c).unwrap(), b"a");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(windows))]
    #[test]
    fn the_no_replace_rename_is_unsupported_off_windows() {
        let e = rename_no_replace(Path::new("a"), Path::new("b")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Unsupported);
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
}
