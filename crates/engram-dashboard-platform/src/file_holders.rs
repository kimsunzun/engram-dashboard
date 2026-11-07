//! Asks the OS 「이 파일을 지금 어느 프로세스가 열고 있나」 (which process has this file open right now).
//!
//! ★This file does not know what the answer is used to look for (ADR-0266 decision 3)★ — it takes one path and returns
//! only a list of holders.
//!
//! The means of asking is the **Windows Restart Manager**; no means of asking this question was provided on other OSes —
//! there the list is always empty, so a decision built on it stays at 「못 받음」 (not received) instead of a false detection (ADR-0218 decision 10).
//!
//! Entry point = [`holders_of`]. Failure is returned as a value (no panic).

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

/// One process that has a file open.
///
/// ★The two fields are there to be compared **together**★ — the OS reuses PIDs, so matching on PID alone takes
/// someone else's process for that process (ADR-0218 decision 2).
/// [`crate::process::ProcessIdentity`] has the same shape but means 「한 뿌리 아래의 프로세스」 (a process under one root), so the two differ — do not merge them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    /// The high/low 32 bits of the process creation FILETIME (count of 100-nanosecond intervals since 1601-01-01 UTC) joined into one value —
    /// the same as what [`crate::process::process_creation_time`] returns for the same PID (the
    /// measurement in ADR-0218 「근거」 (Rationale); this file's test measures that equality).
    ///
    /// Whether Restart Manager returns 0 for some entry because it cannot fill this field is unchecked — such an entry
    /// simply drops out of the comparison (the start time of the other side is not 0).
    pub start_time: u64,
}

/// All processes that have `path` open.
///
/// - **Empty list = nobody holds it**. It is not an error but the normal look of a dead lock file (ADR-0218 「근거」 (Rationale) —
///   we measured that asking about the same file after killing the owning process gives 0).
/// - **Error = the query failed** (session create, register or enumerate failed). The only reason to keep this apart from 「홀더 없음」 (no holder) is so the caller can
///   log the two differently — for the ownership decision both mean 「후보 아님」 (not a candidate).
/// - Administrator rights are not needed (same measurement).
/// - ★Pass the full path★ — the measurement and the docs covered only full paths, and what Restart Manager resolves a
///   relative path against is unchecked. If it goes wrong the symptom is not an error but 「홀더 0」 (0 holders).
#[cfg(windows)]
pub fn holders_of(path: &Path) -> io::Result<Vec<Holder>> {
    use std::os::windows::ffi::OsStrExt;

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "경로 안에 NUL 이 있어 널종단 문자열로 넘길 수 없다",
        ));
    }
    wide.push(0);

    let session = RmSession::start()?;
    let files = [PCWSTR(wide.as_ptr())];
    // SAFETY: a live session handle plus one null-terminated wide path owned by this function. `wide` stays alive
    // until the call ends, and the registered resource is cleaned up when the session closes.
    let rc = unsafe { RmRegisterResources(session.handle, Some(&files), None, None) };
    if rc != ERROR_SUCCESS {
        return Err(rm_err("RmRegisterResources", rc));
    }

    // The buffer size is obtained by asking the API (`needed`). ★One round may not be enough★ — if holders grow
    // between the two calls, the second also fails with ERROR_MORE_DATA, so it asks again a few rounds, and if it does not converge it is an error instead of an infinite loop.
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
        "RmGetList 가 {GET_LIST_ATTEMPTS} 번 안에 버퍼 크기에 수렴하지 않았다"
    )))
}

/// non-windows: there is no means of asking, so the list is **always empty** — it is passed down in the same shape as 「아무도 안 쥐고 있다」
/// (nobody holds it) (ADR-0218 decision 10).
#[cfg(not(windows))]
pub fn holders_of(_path: &Path) -> io::Result<Vec<Holder>> {
    Ok(Vec::new())
}

/// Retry limit for the holder-list query — it retries only when holders grow between asking the size and receiving.
#[cfg(windows)]
const GET_LIST_ATTEMPTS: u32 = 4;

/// One Restart Manager session. ★`Drop` closing it is all this type does★ — it is held by value so that
/// leaving midway through a query with `?` does not leave the session behind.
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
    /// ★The failure is not swallowed — a session left unclosed remains, one more with each call to [`holders_of`]★ (the more a
    /// caller calls it repeatedly, the more pile up). Being in `Drop` there is nowhere to return it, so all that is left is a log, and that
    /// log is the only place where you can see that sessions are leaking.
    fn drop(&mut self) {
        // SAFETY: closes the handle that start() returned as a success, exactly once.
        let rc = unsafe { RmEndSession(self.handle) };
        if rc != ERROR_SUCCESS {
            tracing::warn!(
                handle = self.handle,
                code = rc.0,
                "RmEndSession 실패 — Restart Manager 세션이 남았을 수 있다"
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
    io::Error::other(format!("{api} 실패 — win32 error {}", code.0))
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::*;

    /// Asks about a file it opened itself and measures whether the PID and start time **both** come back as ours. The
    /// reference start time comes from the `process` helper in the same crate — that the two paths read the same clock is the premise of ADR-0218 decision 2,
    /// and this is the place that upholds that premise.
    #[cfg(windows)]
    #[test]
    fn a_file_this_process_holds_reports_this_process() {
        let path = std::env::temp_dir().join(format!(
            "engram-file-holders-{}-{}.tmp",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let file = std::fs::File::create(&path).expect("임시 파일 생성");

        let holders = holders_of(&path).expect("홀더 조회 성공");

        let me = std::process::id();
        let mine = holders
            .iter()
            .find(|h| h.pid == me)
            .unwrap_or_else(|| panic!("자기 PID({me}) 가 홀더로 나와야 — 받은 목록 {holders:?}"));
        let expected =
            crate::process::process_creation_time(me).expect("자기 creation time 조회 가능");
        assert_eq!(
            mine.start_time, expected,
            "RM 의 ProcessStartTime 과 GetProcessTimes 의 creation time 이 같아야"
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
        std::fs::write(&path, b"").expect("임시 파일 생성");

        let holders = holders_of(&path).expect("홀더 조회 성공");
        assert!(
            holders.is_empty(),
            "닫아 둔 파일은 홀더 0 — 받은 것 {holders:?}"
        );

        std::fs::remove_file(&path).expect("임시 파일 삭제");
        let gone = holders_of(&path).expect("없는 파일도 오류가 아니다");
        assert!(gone.is_empty(), "없는 파일은 홀더 0 — 받은 것 {gone:?}");
    }
}
