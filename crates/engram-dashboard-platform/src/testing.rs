//! Real-process test helpers — shared by this crate's tests and the calling crates' tests that turn on the `test-support` feature as a dev dependency
//! (TRD 1-3 §3-8 — `docs/process/S21-crate-boundaries/trd-1-3-platform-crate.md`). They all exchange
//! only OS-level values (`Child` · PID · start time · probe).
//!
//! There is no helper that builds a group handle — the handle the calling crate's tests use is that crate's adapter type, so it cannot be
//! built here. The group itself is built with [`crate::group::GroupOwner`].
// ADR-0275

use std::io::Write;
use std::process::Child;
use std::time::{Duration, Instant};

/// `cmd.exe /d /c <line>` — but it does not start `line` until it receives one line on stdin. If it is put in the group in between,
/// everything `line` launches is born inside the group. [`open_gate`] gives that one line. `flags` = the creation flags as is.
/// It panics if launching fails.
#[cfg(windows)]
pub fn spawn_gated_cmd(line: &str, flags: u32) -> Child {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    Command::new("cmd.exe")
        .raw_arg(format!("/d /c set /p _= & {line}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(flags)
        .spawn()
        .expect("cmd.exe 기동")
}

/// Opens the gate of [`spawn_gated_cmd`] — writes one line to stdin and closes that end. It panics if stdin has already been taken.
pub fn open_gate(child: &mut Child) {
    let mut stdin = child.stdin.take().expect("stdin 파이프");
    let _ = stdin.write_all(b"go\r\n");
}

/// Asks every 20 ms until `probe` gives `Some` — if it does not see one within 10 seconds it panics with `what` in the message.
///
/// It is a different function from base's `testing::wait_until` (deadline · condition → `bool`) — this crate does not depend on base, so they are not
/// merged (ADR-0266 · ADR-0268).
pub fn wait_until<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(found) = probe() {
            return found;
        }
        assert!(Instant::now() < deadline, "10 초 안에 {what} 를 못 봤다");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Is the executable of that number `ping.exe`. `false` if it cannot be opened.
#[cfg(windows)]
pub fn is_ping(pid: u32) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    // SAFETY: opens only the query right.
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return false;
    };
    let image = crate::group::image_path(handle);
    // SAFETY: closes the handle just opened, once.
    unsafe {
        let _ = CloseHandle(handle);
    }
    image.is_some_and(|p| p.to_ascii_lowercase().ends_with("\\ping.exe"))
}

/// Ends that PID if it is still the very process born at `start_time` (the process creation FILETIME — the same scale as [`crate::process::process_creation_time`])
/// and is still running. The start-time check · liveness check · ending are done through the **same handle** — while the handle is
/// held that PID does not pass to someone else, so there is no way to kill a wrong process after checking. It does not touch descendants.
///
/// `Ok(true)` = the ending was handed off (it does not wait) · `Ok(false)` = it is not that process (absent · start time differs · already
/// ended) · `Err` = it could not open, read or end (including access denied · ending by itself between the check and the ending) — it was not killed.
#[cfg(windows)]
pub fn terminate_if_same(pid: u32, start_time: u64) -> std::io::Result<bool> {
    use windows::core::HRESULT;
    use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, FILETIME, HANDLE};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, TerminateProcess,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };
    const STILL_ACTIVE: u32 = 259;

    fn decide(handle: HANDLE, start_time: u64) -> windows::core::Result<bool> {
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        // SAFETY: a valid handle opened by the caller + four outputs on the stack.
        unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }?;
        let created = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        if created != start_time {
            return Ok(false);
        }
        let mut code = 0u32;
        // SAFETY: the same handle + stack output.
        unsafe { GetExitCodeProcess(handle, &mut code) }?;
        if code != STILL_ACTIVE {
            return Ok(false);
        }
        // SAFETY: the same handle — it was opened with the terminate right.
        unsafe { TerminateProcess(handle, 1) }?;
        Ok(true)
    }

    let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE;
    // SAFETY: opens only the query and terminate rights.
    let handle = match unsafe { OpenProcess(access, false, pid) } {
        Ok(h) => h,
        Err(e) if e.code() == HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) => return Ok(false),
        Err(e) => return Err(std::io::Error::other(format!("PID {pid} 열기: {e}"))),
    };
    let outcome = decide(handle, start_time)
        .map_err(|e| std::io::Error::other(format!("PID {pid} 확인 · 끝내기: {e}")));
    // SAFETY: closes the handle opened above, once.
    unsafe {
        let _ = CloseHandle(handle);
    }
    outcome
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::process::{pid_alive, process_creation_time};

    /// It does not kill with an identity whose start time differs and kills only with the same identity — after the ending even the same identity is 「그 프로세스가 아니다」 (not that process).
    #[test]
    fn terminate_if_same_ends_only_the_matching_identity() {
        let mut child = spawn_gated_cmd("rem", 0);
        let pid = child.id();
        let start = process_creation_time(pid).expect("시작시각");

        let other = terminate_if_same(pid, start + 1);
        let alive_after_other = pid_alive(pid);
        let same = terminate_if_same(pid, start);
        let _ = child.kill();
        let _ = child.wait();
        let after_exit = terminate_if_same(pid, start);

        assert!(matches!(other, Ok(false)), "{other:?}");
        assert!(alive_after_other, "시작시각이 다른 신원으로 껐다");
        assert!(matches!(same, Ok(true)), "{same:?}");
        assert!(matches!(after_exit, Ok(false)), "{after_exit:?}");
    }
}
