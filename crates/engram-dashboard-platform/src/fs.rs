//! OS rules for handling files — opening while blocking others' writes and classifying its failures · deciding failures caused by someone briefly holding a file and
//! retrying · folder sync. Uses only std — `OpenOptionsExt::share_mode` goes straight down to the share mode of `CreateFileW`
//! and gets the same kernel behavior without unsafe, so no `windows` bindings are needed.

// ADR-0266
// ADR-0291

use std::fs::File;
use std::io;
use std::path::Path;
use std::time::Duration;

const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_SHARING_VIOLATION: i32 = 32;
const ERROR_LOCK_VIOLATION: i32 = 33;

/// Opens for read+write, creating the file if missing. Existing content is not truncated.
///
/// While this handle is open, other opens are shared **read only** — an open requesting write · delete (including rename)
/// fails with a sharing violation ([`is_sharing_violation`]). Conversely, if someone already has this file open with write access
/// or open while blocking write sharing, this open fails with a sharing violation. A read open where the other side shares writing
/// (the default of Rust `std::fs::read`) opens alongside this handle. A read open that shares only reading collides with this
/// handle's write access and is a sharing violation (measured 2026-08-14 — ADR-0135).
///
/// A file with the read-only **attribute** fails not with a sharing violation but with access denied ([`is_access_denied`]).
///
/// Outside Windows = fails with `ErrorKind::Unsupported` without trying to open — there is no means to put this sharing restriction in place.
/// That error has no OS error code (`raw_os_error() == None`) — that is how it is told apart from an `Unsupported`
/// the OS emitted while trying to open.
#[cfg(windows)]
pub fn open_deny_write(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    // `FILE_SHARE_READ` alone. Adding `WRITE` or `DELETE` would let others' write opens · deletes succeed as is, and what this function
    //   blocks would vanish.
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

/// Was it blocked by the sharing restriction of a handle someone else holds (Windows `ERROR_SHARING_VIOLATION`). Once that handle closes, the same open
/// can succeed. Outside Windows = always false.
///
/// std classifies this code as `ErrorKind::Uncategorized`, so it cannot be told apart by `kind()` (measured 2026-08-14).
pub fn is_sharing_violation(e: &io::Error) -> bool {
    cfg!(windows) && e.raw_os_error() == Some(ERROR_SHARING_VIOLATION)
}

/// Was access itself denied (Windows `ERROR_ACCESS_DENIED` — ACL denial, and when opening a file with the read-only attribute
/// for writing). Unlike a sharing violation, it does not clear when someone else's handle closes. Outside Windows = always false — other OSes'
/// `PermissionDenied` does not count as this.
pub fn is_access_denied(e: &io::Error) -> bool {
    cfg!(windows) && e.raw_os_error() == Some(ERROR_ACCESS_DENIED)
}

/// Is it a failure caused by someone briefly holding the file — when antivirus · indexers hold it, open · read · rename fail like this, and once they let go
/// the same attempt can succeed. [`retry_busy`] uses this to decide whether to retry.
///
/// What counts = `ErrorKind::PermissionDenied` (all OSes — Windows `ERROR_ACCESS_DENIED` is this kind too) · Windows's
/// `ERROR_SHARING_VIOLATION`(32) · `ERROR_LOCK_VIOLATION`(33).
///
/// ★Unlike [`is_access_denied`], access denied counts as locked★ — denials that never clear, like the read-only attribute · ACLs, are
/// mixed in too, so such failures come back late, by [`retry_busy`]'s budget.
///
/// ★32 · 33 cannot be caught by kind★ — Rust std maps only 5 to `PermissionDenied` and leaves those two as an unnamed kind.
/// So it looks at the OS code. On other OSes those numbers are different errors (Linux 32 = `EPIPE`).
// ADR-0291: the lock decision · retry are a single platform policy.
pub fn is_busy(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied
        || (cfg!(windows)
            && matches!(
                e.raw_os_error(),
                Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
            ))
}

/// The budget of [`retry_busy`] — retries up to `retries` times after the first attempt, waiting `pause` before each retry.
/// The extra waiting time in the worst case = `retries × pause`.
#[derive(Clone, Copy, Debug)]
pub struct Retry {
    pub retries: u32,
    pub pause: Duration,
}

/// Does `attempt` once, and if [`is_busy`], sleeps `retry.pause` and does it again — at most
/// `retry.retries + 1` times including the first attempt.
///
/// Success and non-lock errors are returned right away. It does not sleep after the last attempt. When the budget runs out, it is the last lock error
/// as is. It emits no logs (header rule 4).
pub fn retry_busy<T>(retry: Retry, attempt: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    retry_busy_with(retry, attempt, std::thread::sleep)
}

/// Same as [`retry_busy`] but calls `pause` instead of sleeping — passing `retry.pause` before each retry. The seam that lets a test
/// count retries without actually sleeping.
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

/// Persists one rename inside a folder to disk — called after the rename of an atomic write.
///
/// Outside Windows = opens the folder and `sync_all` (a missing folder = `NotFound`). Windows = does nothing and `Ok` — `Ok` even if the folder
/// is missing.
// ADR-0291: the OS split of folder sync is only here, with `#[cfg]`.
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
