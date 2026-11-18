//! Asks the OS "which process has this file open right now". ★It does not know what the answer is used for (ADR-0266 decision 3)★ — one
//!   path in, a list of holders out. The means is the **Windows Restart Manager**; other OSes have none, so
//! the list is always empty and a decision built on it stays at "not received" instead of a false detection (ADR-0218 decision 10). Entry
//!   point = [`holders_of`]; failure is returned as a value (no panic).

// ADR-0266

use std::io;
use std::path::Path;

#[cfg(windows)]
use windows::core::{PCWSTR, PWSTR};
#[cfg(windows)]
use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS, WIN32_ERROR};
#[cfg(windows)]
use windows::Win32::System::RestartManager::{
    RmEndSession, RmGetList, RmRegisterResources, RmStartSession, CCH_RM_SESSION_KEY,
    RM_PROCESS_INFO,
};

/// One process that has a file open. ★The two fields are compared **together**★ — the OS reuses PIDs, so a PID-only match takes someone
///   else's process for that one (ADR-0218 decision 2). [`crate::process::ProcessIdentity`] has the same shape
/// but means "a process under one root" — do not merge them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    /// The process creation FILETIME (100 ns since 1601-01-01 UTC), high/low 32 bits joined — equal to
    ///   [`crate::process::process_creation_time`] for the same PID (measurement in ADR-0218 「Rationale」; this file's test measures the
    ///   equality).
    /// Whether Restart Manager returns 0 for some entry because it cannot fill this is unchecked — such an entry just drops out of the
    ///   comparison (the other side's start time is not 0).
    pub start_time: u64,
}

/// All processes that have `path` open. **Empty list = nobody holds it** — not an error but the normal look of a dead lock file (ADR-0218
///   「Rationale」: asking after killing the owner gives 0). **Error = the query failed** (session create,
/// register or enumerate); kept apart from "no holder" only so the caller can log them differently — for the ownership decision both mean
///   "not a candidate". No administrator rights needed. ★Pass the full path★ — measurement and docs covered
/// only full paths; what Restart Manager resolves a relative path against is unchecked, and the symptom would be "0 holders", not an error.
#[cfg(windows)]
pub fn holders_of(path: &Path) -> io::Result<Vec<Holder>> {
    use std::os::windows::ffi::OsStrExt;

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the path contains NUL, so it cannot be passed as a null-terminated string",
        ));
    }
    wide.push(0);

    let session = RmSession::start()?;
    let files = [PCWSTR(wide.as_ptr())];
    // SAFETY: a live session handle plus one null-terminated wide path owned here; `wide` stays alive until the call ends, and the
    //   registered resource is cleaned up when the session closes.
    let rc = unsafe { RmRegisterResources(session.handle, Some(&files), None, None) };
    if rc != ERROR_SUCCESS {
        return Err(rm_err("RmRegisterResources", rc));
    }

    // The buffer size is asked from the API (`needed`). ★One round may not be enough★ — if holders grow between the two calls the second
    //   also fails with ERROR_MORE_DATA, so it asks a few rounds and errors instead of looping forever.
    let mut buf: Vec<RM_PROCESS_INFO> = Vec::new();
    for _ in 0..GET_LIST_ATTEMPTS {
        let mut needed: u32 = 0;
        let mut have: u32 = buf.len() as u32;
        let mut reasons: u32 = 0;
        let out = if buf.is_empty() {
            None
        } else {
            Some(buf.as_mut_ptr())
        };
        // SAFETY: a live session handle plus three output pointers on the stack plus `buf` holding the `have` field (None/0 if empty).
        let rc = unsafe { RmGetList(session.handle, &mut needed, &mut have, out, &mut reasons) };
        if rc == ERROR_SUCCESS {
            buf.truncate(have as usize);
            return Ok(buf.iter().map(holder_of).collect());
        }
        if rc != ERROR_MORE_DATA {
            return Err(rm_err("RmGetList", rc));
        }
        buf = vec![RM_PROCESS_INFO::default(); needed as usize];
    }
    Err(io::Error::other(format!(
        "RmGetList did not converge on a buffer size within {GET_LIST_ATTEMPTS} attempts"
    )))
}

/// non-windows: no means of asking, so the list is **always empty** — the same shape as "nobody holds it" (ADR-0218 decision 10).
#[cfg(not(windows))]
pub fn holders_of(_path: &Path) -> io::Result<Vec<Holder>> {
    Ok(Vec::new())
}

/// Retry limit for the holder-list query — it retries only when holders grow between asking the size and receiving.
#[cfg(windows)]
const GET_LIST_ATTEMPTS: u32 = 4;

/// One Restart Manager session. ★`Drop` closing it is all this type does★ — held by value so leaving a query midway with `?` does not
///   leave the session behind.
#[cfg(windows)]
struct RmSession {
    handle: u32,
}

#[cfg(windows)]
impl RmSession {
    fn start() -> io::Result<Self> {
        let mut handle: u32 = 0;
        // The key buffer size is set by the API — CCH_RM_SESSION_KEY characters plus one null.
        let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
        // SAFETY: a handle output pointer on the stack plus a key buffer sized per the spec above. Flags are reserved, so 0 only.
        let rc = unsafe { RmStartSession(&mut handle, 0, PWSTR(key.as_mut_ptr())) };
        if rc != ERROR_SUCCESS {
            return Err(rm_err("RmStartSession", rc));
        }
        Ok(Self { handle })
    }
}

#[cfg(windows)]
impl Drop for RmSession {
    /// ★The failure is not swallowed — an unclosed session remains, one more per [`holders_of`] call★; `Drop` cannot return it, so a log
    ///   is all that is left and the only place to see sessions leaking.
    fn drop(&mut self) {
        // SAFETY: closes the handle that start() returned as a success, exactly once.
        let rc = unsafe { RmEndSession(self.handle) };
        if rc != ERROR_SUCCESS {
            tracing::warn!(
                handle = self.handle,
                code = rc.0,
                "RmEndSession failed — the Restart Manager session may remain"
            );
        }
    }
}

#[cfg(windows)]
fn holder_of(info: &RM_PROCESS_INFO) -> Holder {
    let started = info.Process.ProcessStartTime;
    Holder {
        pid: info.Process.dwProcessId,
        start_time: ((started.dwHighDateTime as u64) << 32) | (started.dwLowDateTime as u64),
    }
}

#[cfg(windows)]
fn rm_err(api: &str, code: WIN32_ERROR) -> io::Error {
    io::Error::other(format!("{api} failed — win32 error {}", code.0))
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::*;

    /// Asks about a file it opened itself and measures that the PID and start time **both** come back as ours; the reference start time
    ///   comes from the `process` helper in the same crate — that both read one clock is ADR-0218 decision 2's premise,
    /// and this upholds it.
    #[cfg(windows)]
    #[test]
    fn a_file_this_process_holds_reports_this_process() {
        let path = std::env::temp_dir().join(format!(
            "engram-file-holders-{}-{}.tmp",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let file = std::fs::File::create(&path).expect("create temp file");

        let holders = holders_of(&path).expect("holder query succeeds");

        let me = std::process::id();
        let mine = holders.iter().find(|h| h.pid == me).unwrap_or_else(|| {
            panic!("own PID ({me}) must appear as a holder — received list {holders:?}")
        });
        let expected =
            crate::process::process_creation_time(me).expect("own creation time can be queried");
        assert_eq!(
            mine.start_time, expected,
            "RM's ProcessStartTime and GetProcessTimes' creation time must be the same"
        );

        drop(file);
        let _ = std::fs::remove_file(&path);
    }

    /// A file nobody holds (and a file that does not exist at all) gives an empty list, not an error — the normal look of a dead lock.
    #[cfg(windows)]
    #[test]
    fn an_unheld_file_has_no_holder() {
        let path = std::env::temp_dir().join(format!(
            "engram-file-holders-{}-{}.tmp",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, b"").expect("create temp file");

        let holders = holders_of(&path).expect("holder query succeeds");
        assert!(
            holders.is_empty(),
            "a closed file has 0 holders — received {holders:?}"
        );

        std::fs::remove_file(&path).expect("delete temp file");
        let gone = holders_of(&path).expect("a missing file is not an error either");
        assert!(
            gone.is_empty(),
            "a missing file has 0 holders — received {gone:?}"
        );
    }
}
