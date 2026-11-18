//! Daemon discovery shared contract — the contents of daemon.json: the daemon **writes** it (portfile::write_in_place); the src-tauri
//!   shell or an
//! external client **reads** it (discovery::ensure_daemon).
//!
//! ★Replacement is not atomic (ADR-0135)★: daemon.json doubles as the single-instance lock file held by the daemon, so it is written in
//!   place and a
//! reader can see **partially written content** — treat a parse failure as "not ready yet", not corruption.
//!
//! ★No ts-rs export★: a Rust-only file between the two backend processes; serde only.
//!
//! **Security:** `token` is kept in this file only.

use serde::{Deserialize, Serialize};

use crate::Secret;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonInfo {
    pub pid: u32,
    /// Always "127.0.0.1" (local-only bind).
    pub host: String,
    /// The port the daemon actually bound (random).
    pub port: u16,
    /// Connection token (256-bit, 64 hex chars). No logging — `Debug` masks it, but a value taken out with [`Secret::expose`] cannot be masked.
    pub token: Secret,
    pub protocol_version: u32,
    /// The daemon process's start time (Windows GetProcessTimes creation FILETIME: 100-ns intervals since 1601-01-01 UTC). It tells PID reuse
    /// apart (PID alive AND creation time matches). 0 = unknown.
    ///
    /// ★wire compatibility★: an appended field — old readers ignore it; an old daemon.json without it parses via `#[serde(default)]` (0), and 0 makes
    /// liveness decide by PID survival alone, so a live daemon is not wrongly declared stale.
    #[serde(default)]
    pub start_time: u64,
}

impl DaemonInfo {
    /// A **pure function** separated from file IO: the caller passes the bytes it read (testable without a filesystem).
    pub fn parse(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    pub fn to_json_pretty(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DaemonInfo {
        DaemonInfo {
            pid: 4242,
            host: "127.0.0.1".into(),
            port: 51234,
            token: Secret::new("b".repeat(64)),
            protocol_version: 1,
            start_time: 133_000_000_000_000_000,
        }
    }

    #[test]
    fn serde_roundtrip_is_identity() {
        let info = sample();
        let bytes = info.to_json_pretty().unwrap();
        let back = DaemonInfo::parse(&bytes).unwrap();
        assert_eq!(back, info);
    }

    #[test]
    fn parse_valid_json_succeeds() {
        let json =
            br#"{"pid":7,"host":"127.0.0.1","port":9,"token":"deadbeef","protocol_version":1}"#;
        let info = DaemonInfo::parse(json).expect("valid json should parse");
        assert_eq!(info.pid, 7);
        assert_eq!(info.port, 9);
        assert_eq!(info.token.expose(), "deadbeef");
    }

    #[test]
    fn token_is_written_as_a_bare_string() {
        let json = serde_json::to_string(&sample()).unwrap();
        let want = format!(
            r#"{{"pid":4242,"host":"127.0.0.1","port":51234,"token":"{}","protocol_version":1,"start_time":133000000000000000}}"#,
            "b".repeat(64)
        );
        assert_eq!(json, want);
    }

    #[test]
    fn debug_does_not_carry_the_token() {
        let info = sample();
        for shown in [format!("{info:?}"), format!("{info:#?}")] {
            assert!(
                !shown.contains(info.token.expose()),
                "Debug printed the token: {shown}"
            );
            assert!(shown.contains("51234"), "only the token is masked: {shown}");
        }
    }

    #[test]
    fn parse_corrupt_json_errors() {
        assert!(DaemonInfo::parse(b"{ not valid json").is_err());
    }

    #[test]
    fn parse_missing_field_errors() {
        let json = br#"{"pid":1,"host":"127.0.0.1","port":2,"protocol_version":1}"#;
        assert!(DaemonInfo::parse(json).is_err());
    }

    #[test]
    fn json_field_names_are_stable() {
        let json = String::from_utf8(sample().to_json_pretty().unwrap()).unwrap();
        for f in [
            "pid",
            "host",
            "port",
            "token",
            "protocol_version",
            "start_time",
        ] {
            assert!(json.contains(&format!("\"{f}\"")), "field {f} missing");
        }
    }

    #[test]
    fn parse_old_json_without_start_time_defaults_to_zero() {
        let json =
            br#"{"pid":7,"host":"127.0.0.1","port":9,"token":"deadbeef","protocol_version":1}"#;
        let info = DaemonInfo::parse(json).expect("an old file (no start_time) must parse too");
        assert_eq!(info.start_time, 0, "default=0 (unknown) when missing");
        assert_eq!(info.pid, 7);
    }

    #[test]
    fn start_time_roundtrips() {
        let info = sample();
        let back = DaemonInfo::parse(&info.to_json_pretty().unwrap()).unwrap();
        assert_eq!(back.start_time, info.start_time);
    }
}
