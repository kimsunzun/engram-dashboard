//! A lock that only the current OS account can take, shared by every session of that account on this machine and leaving nothing on disk —
//!   [`acquire`]. Windows = a private object namespace whose boundary holds only this account's SID; the namespace itself is the lock:
//!   the process that creates it holds it, and it goes away when that process closes it or ends (crash included).
//!
//! Why this holds (Win32 "Object Namespaces" and `CreatePrivateNamespace`): "The caller must be within the specified boundary for the
//!   create operation to succeed" — so another account, whose token lacks this SID, cannot create (squat) it; a namespace is "uniquely
//!   identified by its name and boundaries", so another account's namespace of the same name is a different one. The docs' example adds
//!   the session number to a boundary to confine it to one session, so a boundary without it spans sessions. The tests measure the
//!   refusal of a boundary SID the caller lacks and the collision inside one process; the cross-session and other-account behaviour is in
//!   `docs/unverified-tests.md`.
//!
//! Windows only for now: outside Windows [`acquire`] is `Unsupported` without trying; the behaviour defined there (an exclusive `flock`
//!   on a file under the account's runtime folder) is built with T-118 / T-119.

// ADR-0266, ADR-0401 (amends ADR-0389 d9): no name another account can create; held elsewhere never means running without the lock

use std::io;

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WIN32_ERROR};
#[cfg(windows)]
use windows::Win32::Security::Authorization::ConvertStringSidToSidW;
#[cfg(windows)]
use windows::Win32::Security::PSID;
#[cfg(windows)]
use windows::Win32::System::Threading::{
    AddSIDToBoundaryDescriptor, ClosePrivateNamespace, CreateBoundaryDescriptorW,
    CreatePrivateNamespaceW, DeleteBoundaryDescriptor, PRIVATE_NAMESPACE_FLAG_DESTROY,
};

#[cfg(windows)]
use crate::win::{io_error, to_wide, LocalMem};

/// The longest name [`acquire`] takes.
pub const MAX_NAME_LEN: usize = 200;

/// What [`acquire`] found.
#[derive(Debug)]
pub enum AccountLock {
    /// This process now holds the lock; dropping the guard releases it.
    Held(AccountLockGuard),
    /// Another process of this account holds it (any session of this machine).
    HeldElsewhere,
}

/// The held lock — released (the namespace destroyed) when dropped.
#[derive(Debug)]
pub struct AccountLockGuard {
    /// The private-namespace handle, as an integer so the guard is `Send` / `Sync` (it is only closed, once, on drop).
    #[cfg(windows)]
    namespace: isize,
}

#[cfg(windows)]
impl Drop for AccountLockGuard {
    fn drop(&mut self) {
        // SAFETY: the handle CreatePrivateNamespaceW returned, closed exactly once; DESTROY removes the namespace for every later create.
        unsafe {
            let _ = ClosePrivateNamespace(
                HANDLE(self.namespace as *mut _),
                PRIVATE_NAMESPACE_FLAG_DESTROY,
            );
        }
    }
}

/// Takes the lock `name` for the current account — the caller's name (no domain knowledge here): 1 to [`MAX_NAME_LEN`] characters, no
///   backslash, no NUL (`InvalidInput`). `Err` = the current account unreadable or any OS failure other than "already held".
/// Outside Windows = `Unsupported` (module head).
#[cfg(windows)]
pub fn acquire(name: &str) -> io::Result<AccountLock> {
    check_name(name)?;
    acquire_within(name, &crate::account::current_account_id()?)
}
#[cfg(not(windows))]
pub fn acquire(name: &str) -> io::Result<AccountLock> {
    check_name(name)?;
    Err(io::ErrorKind::Unsupported.into())
}

fn check_name(name: &str) -> io::Result<()> {
    if name.is_empty() || name.chars().count() > MAX_NAME_LEN || name.contains(['\\', '\0']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "an account lock name must be 1 to {MAX_NAME_LEN} characters without a backslash or NUL"
            ),
        ));
    }
    Ok(())
}

/// [`acquire`] with the boundary SID given — the tests pass a SID this process lacks.
#[cfg(windows)]
fn acquire_within(name: &str, sid_text: &str) -> io::Result<AccountLock> {
    let wide_sid = to_wide(sid_text.as_ref())?;
    let mut sid = PSID::default();
    // SAFETY: a null-terminated SID string and an output pointer on the stack; the SID is LocalAlloc'd and freed by `LocalMem`.
    unsafe { ConvertStringSidToSidW(PCWSTR(wide_sid.as_ptr()), &mut sid) }.map_err(io_error)?;
    // SAFETY: the SID ConvertStringSidToSidW allocated with LocalAlloc, freed once after the last use below.
    let _free_sid = unsafe { LocalMem::new(sid.0) };

    let wide_name = to_wide(name.as_ref())?;
    // SAFETY: a null-terminated name; the descriptor is freed by `DeleteBoundaryDescriptor` below on every path.
    let mut boundary = unsafe { CreateBoundaryDescriptorW(PCWSTR(wide_name.as_ptr()), 0) };
    if boundary.is_invalid() {
        return Err(io::Error::last_os_error());
    }
    let created = (|| {
        // SAFETY: a live boundary descriptor and a valid SID; the descriptor copies the SID.
        unsafe { AddSIDToBoundaryDescriptor(&mut boundary, sid) }
            // A FACILITY_WIN32 HRESULT carries the Win32 code in its low word.
            .map_err(|e| WIN32_ERROR(e.code().0 as u32 & 0xFFFF))?;
        // SAFETY: a live boundary descriptor and a null-terminated prefix, both alive for the call.
        let namespace = unsafe {
            CreatePrivateNamespaceW(None, boundary.0 as *const _, PCWSTR(wide_name.as_ptr()))
        };
        if namespace.is_invalid() {
            // SAFETY: plain read of this thread's last error, right after the failed call.
            return Err(unsafe { GetLastError() });
        }
        Ok(namespace)
    })();
    // SAFETY: the descriptor made above, deleted once; a created namespace does not need it any more.
    unsafe { DeleteBoundaryDescriptor(boundary) };
    match created {
        Ok(namespace) => Ok(AccountLock::Held(AccountLockGuard {
            namespace: namespace.0 as isize,
        })),
        Err(code) if code == ERROR_ALREADY_EXISTS => Ok(AccountLock::HeldElsewhere),
        Err(code) => Err(io::Error::from_raw_os_error(code.0 as i32)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bad_name_is_refused_before_the_os_is_asked() {
        for bad in [
            String::new(),
            "a\\b".to_string(),
            "a\0b".to_string(),
            "x".repeat(MAX_NAME_LEN + 1),
        ] {
            assert_eq!(
                check_name(&bad).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{bad:?}"
            );
        }
        assert!(check_name(&"x".repeat(MAX_NAME_LEN)).is_ok());
    }

    #[cfg(not(windows))]
    #[test]
    fn the_lock_is_unsupported_off_windows() {
        assert_eq!(acquire("x").unwrap_err().kind(), io::ErrorKind::Unsupported);
    }

    #[cfg(windows)]
    fn unique_name() -> String {
        format!("engram-account-lock-test.{}", uuid::Uuid::new_v4().simple())
    }

    /// Two holders in one process stand in for two sessions of one account (a real second session is not available to the suite).
    #[cfg(windows)]
    #[test]
    fn a_second_acquire_collides_and_a_release_frees_it() {
        let name = unique_name();
        let first = acquire(&name).unwrap();
        assert!(matches!(first, AccountLock::Held(_)), "{first:?}");
        let second = acquire(&name).unwrap();
        assert!(matches!(second, AccountLock::HeldElsewhere), "{second:?}");
        drop(first);
        let again = acquire(&name).unwrap();
        assert!(matches!(again, AccountLock::Held(_)), "{again:?}");
        assert!(
            matches!(acquire(&unique_name()).unwrap(), AccountLock::Held(_)),
            "another name is another lock"
        );
    }

    /// The environment variable that turns [`child_reports_the_lock`] into the child half of the two-process test.
    #[cfg(windows)]
    const CHILD_ENV: &str = "ENGRAM_ACCOUNT_LOCK_CHILD";

    /// Child half of [`a_second_process_collides_and_a_release_frees_it`]: re-launched from this test binary with [`CHILD_ENV`] naming
    ///   the lock, it takes the lock and prints what it found. Ignored in a normal run; without the variable it does nothing.
    #[cfg(windows)]
    #[test]
    #[ignore = "the child half of a_second_process_collides_and_a_release_frees_it, which runs it"]
    fn child_reports_the_lock() {
        let Ok(name) = std::env::var(CHILD_ENV) else {
            return;
        };
        let answer = match acquire(&name) {
            Ok(AccountLock::Held(_)) => "held".to_string(),
            Ok(AccountLock::HeldElsewhere) => "elsewhere".to_string(),
            Err(e) => format!("error {e}"),
        };
        println!("lock-answer={answer}");
    }

    /// Runs [`child_reports_the_lock`] in a new process of this test binary and returns its answer.
    #[cfg(windows)]
    fn ask_another_process(name: &str) -> String {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "account_lock::tests::child_reports_the_lock",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ENV, name)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        stdout
            .lines()
            // libtest may print the test name on the same line first.
            .find_map(|l| {
                l.split_once("lock-answer=")
                    .map(|(_, answer)| answer.trim())
            })
            .map(str::to_string)
            .unwrap_or_else(|| panic!("no answer from the child: {stdout}"))
    }

    /// Two processes of this account (same session — a second session is in `docs/unverified-tests.md`).
    #[cfg(windows)]
    #[test]
    fn a_second_process_collides_and_a_release_frees_it() {
        let name = unique_name();
        let held = acquire(&name).unwrap();
        assert!(matches!(held, AccountLock::Held(_)), "{held:?}");
        assert_eq!(ask_another_process(&name), "elsewhere");
        drop(held);
        assert_eq!(ask_another_process(&name), "held");
    }

    /// A crash closes the creator's handle without the destroy flag — the name must be free again for the next start.
    #[cfg(windows)]
    #[test]
    fn a_holder_that_ends_without_destroying_frees_the_name() {
        let name = unique_name();
        let AccountLock::Held(guard) = acquire(&name).unwrap() else {
            panic!("not held")
        };
        // SAFETY: the guard's handle, closed once here instead of in `Drop` (forgotten below) — as process termination does.
        unsafe {
            let _ = ClosePrivateNamespace(HANDLE(guard.namespace as *mut _), 0);
        }
        std::mem::forget(guard);
        assert!(matches!(acquire(&name).unwrap(), AccountLock::Held(_)));
    }

    /// The rule the squat protection stands on: a boundary SID the caller's token lacks refuses the create — here SYSTEM's, which a
    ///   test process does not run as.
    #[cfg(windows)]
    #[test]
    fn a_boundary_with_a_sid_the_caller_lacks_is_refused() {
        let outcome = acquire_within(&unique_name(), "S-1-5-18");
        assert!(
            matches!(&outcome, Err(e) if e.kind() == io::ErrorKind::PermissionDenied),
            "{outcome:?}"
        );
    }
}
