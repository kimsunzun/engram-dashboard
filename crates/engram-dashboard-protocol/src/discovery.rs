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
//! **Security:** `daemon.json` keeps its inherited ACL (any account on the PC can read it — that is how another account sees who holds
//!   the folder and takes over after a crash), so it carries no token. The shell token lives in the owner-only per-account file
//!   [`token_file_name`]; a reader takes `daemon.json` first and the token after it, so a new record is never paired with an old token.
// ADR-0395

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonInfo {
    pub pid: u32,
    /// Always "127.0.0.1" (local-only bind).
    pub host: String,
    /// The port the daemon actually bound (random).
    pub port: u16,
    pub protocol_version: u32,
    /// The daemon process's start time (Windows GetProcessTimes creation FILETIME: 100-ns intervals since 1601-01-01 UTC). It tells PID reuse
    /// apart (PID alive AND creation time matches). 0 = unknown.
    ///
    /// ★wire compatibility★: an appended field — old readers ignore it; an old daemon.json without it parses via `#[serde(default)]` (0), and 0 makes
    /// liveness decide by PID survival alone, so a live daemon is not wrongly declared stale.
    #[serde(default)]
    pub start_time: u64,
    /// The OS account the daemon runs under (Windows: the SID text of `platform::account::current_account_id`). ★Only a hint★ — any account
    ///   can write this file, so nobody decides trust on it: the shell checks the listener's account on its own connection before sending
    ///   a token, and a spawned daemon's lock acquire decides who holds the folder. Empty = a record written before the field existed
    ///   (`#[serde(default)]`; such a record also carries an older `protocol_version`, so a reader takes the version-mismatch path).
    // ADR-0395
    #[serde(default)]
    pub owner_account: String,
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

/// The name of the shell-token file of `account` (the text of `platform::account::current_account_id`), next to `daemon.json` in the
///   daemon's run folder. ★The one place the name is built★ — the daemon writes it and the shell reads it; `scripts/engram.mjs` keeps a
///   hand copy. Per account, so two accounts sharing one data folder take turns without ever needing to delete each other's owner-only
///   file.
// ADR-0395
pub fn token_file_name(account: &str) -> String {
    format!("daemon-token.{account}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DaemonInfo {
        DaemonInfo {
            pid: 4242,
            host: "127.0.0.1".into(),
            port: 51234,
            protocol_version: 1,
            start_time: 133_000_000_000_000_000,
            owner_account: "S-1-5-21-1-2-3-1001".into(),
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
        let json = br#"{"pid":7,"host":"127.0.0.1","port":9,"protocol_version":1,"owner_account":"S-1-5-18"}"#;
        let info = DaemonInfo::parse(json).expect("valid json should parse");
        assert_eq!(info.pid, 7);
        assert_eq!(info.port, 9);
        assert_eq!(info.owner_account, "S-1-5-18");
    }

    #[test]
    fn the_record_is_written_without_a_token_and_with_the_owner_account() {
        let json = serde_json::to_string(&sample()).unwrap();
        assert_eq!(
            json,
            r#"{"pid":4242,"host":"127.0.0.1","port":51234,"protocol_version":1,"start_time":133000000000000000,"owner_account":"S-1-5-21-1-2-3-1001"}"#
        );
        assert!(!json.contains("token"), "{json}");
    }

    /// A pre-S37 daemon's record (token inside, no owner) still parses — so a new app meeting an old running daemon sees its older
    ///   `protocol_version` and takes the version-mismatch path instead of failing to parse; the old token is not kept.
    #[test]
    fn an_old_daemons_record_parses_with_its_version_and_drops_the_token() {
        let json = br#"{"pid":7,"host":"127.0.0.1","port":9,"token":"deadbeef","protocol_version":7,"start_time":5}"#;
        let info = DaemonInfo::parse(json).expect("an old record must parse");
        assert_eq!(info.protocol_version, 7);
        assert_ne!(info.protocol_version, crate::PROTOCOL_VERSION);
        assert_eq!(info.owner_account, "", "default when missing");
        assert!(!format!("{info:?}").contains("deadbeef"));
    }

    #[test]
    fn parse_corrupt_json_errors() {
        assert!(DaemonInfo::parse(b"{ not valid json").is_err());
    }

    #[test]
    fn parse_missing_field_errors() {
        let json = br#"{"pid":1,"host":"127.0.0.1","port":2}"#;
        assert!(DaemonInfo::parse(json).is_err());
    }

    #[test]
    fn json_field_names_are_stable() {
        let json = String::from_utf8(sample().to_json_pretty().unwrap()).unwrap();
        for f in [
            "pid",
            "host",
            "port",
            "protocol_version",
            "start_time",
            "owner_account",
        ] {
            assert!(json.contains(&format!("\"{f}\"")), "field {f} missing");
        }
    }

    #[test]
    fn parse_old_json_without_start_time_defaults_to_zero() {
        let json = br#"{"pid":7,"host":"127.0.0.1","port":9,"protocol_version":1}"#;
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

    #[test]
    fn the_token_file_is_named_by_the_account() {
        assert_eq!(
            token_file_name("S-1-5-21-1-2-3-1001"),
            "daemon-token.S-1-5-21-1-2-3-1001"
        );
        assert_ne!(token_file_name("S-1-5-18"), token_file_name("S-1-5-19"));
    }
}
