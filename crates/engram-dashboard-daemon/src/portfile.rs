//! `daemon.json` — the daemon discovery file and also the single-instance lock file (ADR-0135 decision 1).
//!
//! Only the file name · the daemon-side write/read · the stale decision live here. The side that holds the file is [`crate::instance`], and writes from outside the crate must pass through its guard.
//!
//! [`read`]'s parse-failure log prints the serde error's `Display` (position · expected type only). The record holds no secret — the token
//!   is in the owner-only per-account file ([`crate::instance::InstanceGuard::write_token_file`]).
//!
//! ★Do not revive atomic replacement (temp file + rename)★: the daemon holds this file without delete sharing, so rename is refused (`ERROR_SHARING_VIOLATION`). [`write_in_place`] writes in place to the held handle.
//!
//! ★The reading side can see partially written content★: a parse failure means not ready yet, not corruption (why [`read`] gives `None`). The client already polls at 50ms.
//!
//! ★Do not periodically re-emit a vanished file (do not revive it)★: a running app silently rewriting a file the user just deleted is reported as a bug in every surveyed precedent (ADR-0135 rejected alternative 1). It cannot be deleted after the merge anyway.
//!
//! Security: this file keeps its inherited ACL and carries no token; its owner-account field is a hint only (ADR-0395).

use std::fs::{self, File};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::Path;

pub use engram_dashboard_protocol::DaemonInfo;

/// Connection · lock file name inside the data folder. Generated at runtime, so it is not in the distribution archive.
///
/// ★The daemon must build the path only from this constant★: if the file it holds and the file it writes diverge, singleness silently breaks.
pub const DAEMON_FILE: &str = "daemon.json";

/// Writes the connection record in place with the daemon's held handle.
///
/// ★The only entry point is [`crate::instance::InstanceGuard::publish`]★ — hence `pub(crate)`. `pub(crate)` spans the whole daemon, so "only through the guard" is kept by review, not by the type. Opening this as `pub` or taking a path (`&Path`) would let a side that has not proven ownership emit, mixing two daemons' writes.
///
/// ★What the type blocks stops there★: inside the crate (including this file's tests) it can be called with an arbitrary `File`; the real enforcement is the held OS share mode — before acquisition or after Drop nothing blocks anything.
///
/// The length is first cut to 0, so a reader in between can see an empty file — the module header's "not ready yet" contract covers that window.
// ADR-0135
pub(crate) fn write_in_place(f: &mut File, info: &DaemonInfo) -> io::Result<()> {
    let json = info
        .to_json_pretty()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    f.set_len(0)?;
    f.seek(SeekFrom::Start(0))?;
    f.write_all(&json)?;
    // Visibility to other processes is already secured at write_all (they see the same cache). sync_all is the durability part that keeps the address across a power cut; once per boot, so cost is no concern.
    f.sync_all()
}

/// None if missing or unparseable.
///
/// ★Do not promote a parse failure to corruption★: the daemon may be mid-write (module header). The caller treats it as "not yet ready" and looks again.
pub fn read(path: &Path) -> Option<DaemonInfo> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(_) => return None,
    };
    match DaemonInfo::parse(&bytes) {
        Ok(info) => Some(info),
        Err(e) => {
            tracing::warn!("{DAEMON_FILE} parse failed: {e} — ignored (it may be mid-write)");
            None
        }
    }
}

/// Whether the recorded daemon is no longer alive (stale). true = dead (can be ignored).
///
/// Liveness is delegated to the OS-layer crate `platform` (`pid_alive_with_start_time`), shared with tauri. Alive only when "PID alive AND creation time == recorded value"; start_time==0 (unknown, old daemon.json) falls back conservatively to PID survival alone.
///
/// ★PID reuse (M2) defense★: even if the daemon died and another process got the same PID, the creation time differs, so it is decided dead.
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
            protocol_version: 1,
            start_time: 0,
            owner_account: "S-1-5-21-1-2-3-1001".into(),
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

    /// In-place write can leave a tail if the old content was longer — asserts set_len cuts it.
    #[test]
    fn rewrite_leaves_no_tail_of_the_previous_record() {
        let dir = temp_dir("no-tail");
        let path = dir.join(DAEMON_FILE);
        let mut long = sample();
        long.owner_account = "b".repeat(4096);
        let mut f = open_held(&path);
        write_in_place(&mut f, &long).unwrap();

        let short = sample();
        write_in_place(&mut f, &short).unwrap();
        assert_eq!(
            read(&path).expect("reparse"),
            short,
            "the old tail must not remain"
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

    /// ★Partial write = not ready★: the in-place write makes the client able to actually see this state.
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
        assert!(is_stale(&info), "PID 0 is stale");
    }

    #[test]
    fn current_process_with_unknown_start_time_is_not_stale() {
        let mut info = sample();
        info.pid = std::process::id();
        info.start_time = 0;
        assert!(
            !is_stale(&info),
            "unknown start_time + live PID → not stale"
        );
    }

    #[cfg(windows)]
    #[test]
    fn current_process_with_matching_start_time_is_not_stale() {
        let mut info = sample();
        info.pid = std::process::id();
        info.start_time = engram_dashboard_platform::process::current_process_start_time().unwrap();
        assert!(!is_stale(&info), "PID+creation time match means not stale");
    }

    #[cfg(windows)]
    #[test]
    fn current_pid_with_mismatched_start_time_is_stale() {
        let mut info = sample();
        info.pid = std::process::id();
        let real = engram_dashboard_platform::process::current_process_start_time().unwrap();
        info.start_time = real.wrapping_add(999);
        assert!(
            is_stale(&info),
            "creation time mismatch = reused PID → stale"
        );
    }

    /// A record of a daemon that exited while someone still holds its process object is stale — the handle still opens, so the open-process check alone kept it live (TRD S21 trd-3-2-u5 R4).
    ///
    /// Only with an unknown start time; the known-start-time case is platform's `an_exited_process_whose_object_is_held_is_gone` — [`is_stale`] is the negation it checks.
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
