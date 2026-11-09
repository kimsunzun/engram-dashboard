//! `daemon.json` — the daemon discovery file and also the **single-instance lock file** (ADR-0135 decision 1).
//!
//! Here we put only the file name · the daemon-side write/read · the stale decision. The side that **holds** that file
//! is [`crate::instance`], and writes coming from outside the crate must pass through it (the guard).
//!
//! ★That the log of [`read`] does not leak the token is guarded by only this comment right now (known gap)★: on a parse failure
//! it prints the serde error's `Display`, which holds only the position · expected type and not the value —
//! because `token` is a `String`, there is no place for a value echo of the `invalid type` kind. Changing the field type
//! breaks that premise. `ws.rs` blocks the same trap with a regression test, but there is no test here.
//!
//! ★Do not revive atomic replacement (temp file + rename)★: the daemon holds this file without delete sharing, so
//! rename is refused (measured `ERROR_SHARING_VIOLATION`). So [`write_in_place`] writes
//! in place to the held handle.
//!
//! ★So the reading side can see partially written content★: a parse failure must be treated not as corruption but as **not ready yet**
//! (the reason [`read`] gives `None`). The client already polls at 50ms.
//!
//! ★Do not periodically re-emit a vanished file (do not revive it)★: a running app silently rewriting a
//! file the user just deleted is reported as a bug in every precedent we surveyed (ADR-0135 rejected alternative 1).
//! After the merge it cannot be deleted either, so there is no occasion to revive it at all.
//!
//! **Security:** the token is kept in this file only (no logging).

use std::fs::{self, File};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::Path;

pub use engram_dashboard_protocol::DaemonInfo;

/// Connection · lock file name inside the data folder. Generated at runtime, so it is not in the distribution archive.
///
/// ★The daemon must build the path only from this constant★: if the file it holds and the file it writes diverge, singleness silently breaks.
pub const DAEMON_FILE: &str = "daemon.json";

/// Writes the connection record in place with the daemon's **held handle**.
///
/// ★The only entry point from outside the crate is [`crate::instance::InstanceGuard::publish`]★ — that is why it is `pub(crate)`.
/// If this function is opened as `pub` or reverted to a form that takes a path (`&Path`), a side that has not proven ownership
/// could emit, and two daemons' writes would mix.
///
/// ★But what the type blocks stops there★: inside the crate (including this file's tests) it can be called with an arbitrary `File`, and
/// **the real enforcement is the OS share mode held** — before acquisition or after Drop nothing blocks anything.
///
/// Because the length is first cut to 0, a reader in between can see an empty file — the module header's "not ready yet"
/// contract covers that window.
// ADR-0135
pub(crate) fn write_in_place(f: &mut File, info: &DaemonInfo) -> io::Result<()> {
    let json = info
        .to_json_pretty()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    f.set_len(0)?;
    f.seek(SeekFrom::Start(0))?;
    f.write_all(&json)?;
    // Visibility to other processes is already secured at the write_all point (they see the same cache). sync_all is the durability part that keeps the address
    //   even if power is cut, and since it is once per boot the cost is not a concern.
    f.sync_all()
}

/// None if missing or unparseable.
///
/// ★Do not promote a parse failure to corruption★: the daemon may be mid-write (module header). The caller treats it as "not yet
/// ready" and looks again.
pub fn read(path: &Path) -> Option<DaemonInfo> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(_) => return None,
    };
    match DaemonInfo::parse(&bytes) {
        Ok(info) => Some(info),
        Err(e) => {
            tracing::warn!("{DAEMON_FILE} 파싱 실패: {e} — 무시(쓰는 중일 수 있음)");
            None
        }
    }
}

/// Decides whether the recorded daemon is no longer alive (stale). true = dead (can be ignored).
///
/// The liveness decision is delegated to the shared function of the OS-layer crate `platform` (`pid_alive_with_start_time`) — so daemon·tauri
/// both use the same logic (DRY). It is judged alive only when "PID alive AND creation time == recorded value".
/// If start_time==0 (unknown, old daemon.json), it conservatively decides by PID survival alone.
///
/// ★PID reuse (M2) defense★: even if the daemon died and another process got the same PID, the creation time
/// differs so it is decided dead → a wrong process is not mistaken for the living daemon.
pub fn is_stale(info: &DaemonInfo) -> bool {
    !engram_dashboard_platform::process::pid_alive_with_start_time(info.pid, info.start_time)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("engram-daemon-portfile-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample() -> DaemonInfo {
        DaemonInfo {
            pid: 1234,
            host: "127.0.0.1".into(),
            port: 54321,
            token: engram_dashboard_protocol::Secret::new("a".repeat(64)),
            protocol_version: 1,
            start_time: 0,
        }
    }

    fn open_held(path: &Path) -> File {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .unwrap()
    }

    #[test]
    fn write_read_roundtrip() {
        let dir = temp_dir("roundtrip");
        let path = dir.join(DAEMON_FILE);
        let info = sample();
        write_in_place(&mut open_held(&path), &info).unwrap();

        let loaded = read(&path).expect("should read back");
        assert_eq!(loaded, info);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Because it is an in-place write, a tail can remain if the old content was longer — an assertion that set_len cuts it.
    #[test]
    fn rewrite_leaves_no_tail_of_the_previous_record() {
        let dir = temp_dir("no-tail");
        let path = dir.join(DAEMON_FILE);
        let mut long = sample();
        long.token = engram_dashboard_protocol::Secret::new("b".repeat(4096));
        let mut f = open_held(&path);
        write_in_place(&mut f, &long).unwrap();

        let short = sample();
        write_in_place(&mut f, &short).unwrap();
        assert_eq!(
            read(&path).expect("재파싱"),
            short,
            "옛 꼬리가 남으면 안 됨"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_missing_is_none() {
        let dir = temp_dir("missing");
        let path = dir.join(DAEMON_FILE);
        assert!(read(&path).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    /// ★Partial write = not ready★: because it is an in-place write, the client can actually see this state.
    #[test]
    fn read_partial_is_none_not_an_error() {
        let dir = temp_dir("partial");
        let path = dir.join(DAEMON_FILE);
        let full = sample().to_json_pretty().unwrap();
        fs::write(&path, &full[..full.len() / 2]).unwrap();
        assert!(read(&path).is_none());
        // An empty file (the window right after the length is cut to 0) is the same.
        fs::write(&path, b"").unwrap();
        assert!(read(&path).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_corrupt_is_none() {
        let dir = temp_dir("corrupt");
        let path = dir.join(DAEMON_FILE);
        fs::write(&path, b"{ not valid json").unwrap();
        assert!(read(&path).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn serde_shape_is_stable() {
        let info = sample();
        let json = String::from_utf8(info.to_json_pretty().unwrap()).unwrap();
        let back = DaemonInfo::parse(json.as_bytes()).unwrap();
        assert_eq!(back, info);
        assert!(json.contains("\"protocol_version\""));
        assert!(json.contains("\"port\""));
    }

    #[cfg(windows)]
    #[test]
    fn pid_zero_is_stale() {
        let mut info = sample();
        info.pid = 0;
        assert!(is_stale(&info), "PID 0 은 stale");
    }

    #[test]
    fn current_process_with_unknown_start_time_is_not_stale() {
        let mut info = sample();
        info.pid = std::process::id();
        info.start_time = 0;
        assert!(
            !is_stale(&info),
            "미상 start_time + 살아있는 PID → not stale"
        );
    }

    #[cfg(windows)]
    #[test]
    fn current_process_with_matching_start_time_is_not_stale() {
        let mut info = sample();
        info.pid = std::process::id();
        info.start_time = engram_dashboard_platform::process::current_process_start_time().unwrap();
        assert!(!is_stale(&info), "PID+creation time 일치면 not stale");
    }

    #[cfg(windows)]
    #[test]
    fn current_pid_with_mismatched_start_time_is_stale() {
        let mut info = sample();
        info.pid = std::process::id();
        let real = engram_dashboard_platform::process::current_process_start_time().unwrap();
        info.start_time = real.wrapping_add(999);
        assert!(is_stale(&info), "creation time 불일치 = 재사용 PID → stale");
    }

    /// A record of a daemon that exited while someone still holds its process object is stale — the handle still
    /// opens, so the open-process check alone kept it live (TRD S21 trd-3-2-u5 R4).
    ///
    /// Only with an unknown start time: reading the child's start time would take a platform symbol this crate does not
    /// import (CI `net gate 2a`). The known-start-time case is platform's
    /// `an_exited_process_whose_object_is_held_is_gone` — [`is_stale`] is the negation it checks.
    #[cfg(windows)]
    #[test]
    fn an_exited_process_whose_object_is_held_is_stale() {
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/c", "exit"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("cmd.exe spawn");
        let mut info = sample();
        info.pid = child.id();
        info.start_time = 0;
        let _ = child.wait();
        // `child` still holds the handle, so the exited process can still be opened.
        let stale = is_stale(&info);
        drop(child);

        assert!(stale, "the exit code decides — an exited daemon is stale");
    }
}
