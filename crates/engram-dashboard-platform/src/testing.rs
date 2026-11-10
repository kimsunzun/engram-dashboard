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
