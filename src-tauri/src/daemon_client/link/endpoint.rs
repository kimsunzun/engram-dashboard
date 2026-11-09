//! Where a daemon is and which credential it takes (TRD S21 trd-3 §3-6 — the seam that replaced the old `DaemonDiscovery`).
//!
//! - [`EndpointSource::ensure`] may spawn a daemon; [`EndpointSource::current`] never does. Only the first connection
//!   calls `ensure`, outside `Registry::add`; every dial calls `current` (`link::dialer`) — so a reconnect cannot wake a
//!   daemon by construction (ADR-0021).
//! - `current` returns why there is no usable endpoint instead of `None`, because the dialer treats the reasons
//!   differently (D25 ① · D20 ①): only a sharing violation falls back to the last-known address, a version mismatch is
//!   a terminal refusal, everything else is one failed attempt.
// ADR-0021
// ADR-0310

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use engram_dashboard_protocol::{DaemonInfo, Secret};
use engram_dashboard_transport::Address;

use super::wire::json_class;
use super::DaemonKey;
use crate::discovery::layout::DAEMON_FILE;
use crate::discovery::{
    check_acceptable, AcceptCheck, DataLayout, DiscoveryError, PidLiveness, RealLiveness,
};

/// One daemon's dial target and credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub key: DaemonKey,
    /// The transport address the inner dialer understands (`ws://host:port` for the local daemon).
    pub address: Address,
    pub credential: Secret,
}

/// What [`EndpointSource::current`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Current {
    /// A live daemon that speaks our protocol version.
    Live(Endpoint),
    /// Another process holds the endpoint record without sharing it (a virus scanner · an indexer — Windows sharing
    /// violation). The owner that wrote it is likely still alive, so the last-known endpoint is worth one dial.
    Busy,
    /// No endpoint record — no live daemon.
    Absent,
    /// A record that cannot be used: empty · broken · a dead pid · access denied · any other read error. The text is
    /// a local diagnostic and quotes no file content.
    Unusable(String),
    /// A live daemon on another protocol version. Dialing it can only be refused. `pid` / `start_time` = the record's
    /// identity of that daemon (`start_time` 0 = the record carries none).
    VersionMismatch {
        daemon: u32,
        pid: u32,
        start_time: u64,
    },
}

/// See the module doc. `current` runs blocking file I/O — async callers run it on the blocking pool.
pub trait EndpointSource: Send + Sync {
    /// Find a live daemon, spawning one if there is none. Blocks up to `timeout` (spawn polling).
    fn ensure(&self, timeout: Duration) -> Result<Endpoint, String>;
    /// Read where the daemon is now. ★Never spawns★.
    fn current(&self) -> Current;
}

/// The local daemon of one data folder (`daemon.json`).
///
/// It reads the file itself instead of through `discovery::FileReader`, which turns the read error into text and
/// loses the `io::Error` that tells a sharing violation apart (D25 ①).
pub struct LocalEndpointSource {
    key: DaemonKey,
    data_dir: PathBuf,
    liveness: Box<dyn PidLiveness + Send + Sync>,
}

impl LocalEndpointSource {
    pub fn new(key: DaemonKey, data_dir: PathBuf) -> Self {
        Self {
            key,
            data_dir,
            liveness: Box::new(RealLiveness),
        }
    }

    #[cfg(test)]
    pub(super) fn with_liveness(
        key: DaemonKey,
        data_dir: PathBuf,
        liveness: impl PidLiveness + Send + Sync + 'static,
    ) -> Self {
        Self {
            key,
            data_dir,
            liveness: Box::new(liveness),
        }
    }

    fn endpoint(&self, info: DaemonInfo) -> Endpoint {
        Endpoint {
            key: self.key.clone(),
            address: Address::new(format!("ws://{}:{}", info.host, info.port)),
            credential: info.token,
        }
    }

    fn judge(&self, read: io::Result<Vec<u8>>) -> Current {
        let bytes = match read {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Current::Absent,
            Err(e) if engram_dashboard_platform::fs::is_sharing_violation(&e) => {
                return Current::Busy
            }
            Err(e) => return Current::Unusable(format!("{DAEMON_FILE} read failed: {e}")),
        };
        let info = match DaemonInfo::parse(&bytes) {
            Ok(info) => info,
            Err(e) => {
                return Current::Unusable(format!(
                    "{DAEMON_FILE} does not parse ({} at line {} column {})",
                    json_class(&e),
                    e.line(),
                    e.column()
                ))
            }
        };
        match check_acceptable(&info, self.liveness.as_ref()) {
            AcceptCheck::Accept => Current::Live(self.endpoint(info)),
            AcceptCheck::DeadPid => Current::Unusable(format!(
                "{DAEMON_FILE} names pid {} which is not alive",
                info.pid
            )),
            AcceptCheck::VersionMismatch { daemon } => Current::VersionMismatch {
                daemon,
                pid: info.pid,
                start_time: info.start_time,
            },
        }
    }
}

impl EndpointSource for LocalEndpointSource {
    fn ensure(&self, timeout: Duration) -> Result<Endpoint, String> {
        let exe = crate::discovery::locate_daemon_exe().map_err(|e| e.to_string())?;
        // console = false: a windowless spawn, as the shell has always spawned its daemon.
        match crate::discovery::ensure_daemon(&self.data_dir, &exe, timeout, false) {
            Ok(info) => Ok(self.endpoint(info)),
            Err(e) => {
                if let DiscoveryError::VersionMismatch { daemon, expected } = &e {
                    tracing::warn!(
                        daemon,
                        expected,
                        "the local daemon speaks another protocol version — not connecting"
                    );
                }
                Err(e.to_string())
            }
        }
    }

    fn current(&self) -> Current {
        let path = DataLayout::new(&self.data_dir).daemon_file();
        self.judge(std::fs::read(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::path::Path;

    use engram_dashboard_protocol::PROTOCOL_VERSION;

    const TOKEN: &str = "feedfacefeedfacefeedfacefeedfacefeedfacefeedfacefeedfacefeedface";

    struct FixedLiveness {
        dead: bool,
    }

    impl PidLiveness for FixedLiveness {
        fn is_dead(&self, _pid: u32, _start_time: u64) -> bool {
            self.dead
        }
    }

    fn source(dir: &Path, dead: bool) -> LocalEndpointSource {
        LocalEndpointSource::with_liveness(
            DaemonKey::new("local"),
            dir.to_path_buf(),
            FixedLiveness { dead },
        )
    }

    fn write_record(dir: &Path, bytes: &[u8]) {
        let path = DataLayout::new(dir).daemon_file();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn record(port: u16, version: u32) -> Vec<u8> {
        DaemonInfo {
            pid: 4242,
            host: "127.0.0.1".into(),
            port,
            token: Secret::new(TOKEN),
            protocol_version: version,
            start_time: 0,
        }
        .to_json_pretty()
        .unwrap()
    }

    #[test]
    fn a_live_record_of_our_version_is_a_live_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        write_record(dir.path(), &record(5123, PROTOCOL_VERSION));
        assert_eq!(
            source(dir.path(), false).current(),
            Current::Live(Endpoint {
                key: DaemonKey::new("local"),
                address: Address::new("ws://127.0.0.1:5123"),
                credential: Secret::new(TOKEN),
            })
        );
    }

    #[test]
    fn every_read_follows_the_record_so_a_moved_port_is_picked_up() {
        let dir = tempfile::tempdir().unwrap();
        let src = source(dir.path(), false);
        write_record(dir.path(), &record(5123, PROTOCOL_VERSION));
        let first = src.current();
        write_record(dir.path(), &record(6234, PROTOCOL_VERSION));
        let second = src.current();
        let address = |c: Current| match c {
            Current::Live(ep) => ep.address,
            other => panic!("expected a live endpoint, got {other:?}"),
        };
        assert_eq!(address(first), Address::new("ws://127.0.0.1:5123"));
        assert_eq!(address(second), Address::new("ws://127.0.0.1:6234"));
    }

    #[test]
    fn a_missing_record_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(source(dir.path(), false).current(), Current::Absent);
    }

    #[test]
    fn an_empty_or_broken_record_is_unusable_and_the_reason_quotes_no_content() {
        let dir = tempfile::tempdir().unwrap();
        for bytes in [
            &b""[..],
            br#"{"pid":1,"host":"127.0.0.1","port":"#,
            br#"{"pid":1,"host":"127.0.0.1","port":"peer-text","token":"x","protocol_version":7}"#,
        ] {
            write_record(dir.path(), bytes);
            match source(dir.path(), false).current() {
                Current::Unusable(why) => {
                    assert!(why.contains("does not parse"), "{why}");
                    assert!(!why.contains("peer-text"), "{why}");
                }
                other => panic!("expected unusable, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_record_whose_pid_is_dead_is_unusable() {
        let dir = tempfile::tempdir().unwrap();
        write_record(dir.path(), &record(5123, PROTOCOL_VERSION));
        assert!(matches!(
            source(dir.path(), true).current(),
            Current::Unusable(why) if why.contains("not alive")
        ));
    }

    #[test]
    fn a_live_daemon_on_another_version_is_a_version_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        write_record(dir.path(), &record(5123, PROTOCOL_VERSION + 1));
        assert_eq!(
            source(dir.path(), false).current(),
            Current::VersionMismatch {
                daemon: PROTOCOL_VERSION + 1,
                pid: 4242,
                start_time: 0,
            }
        );
    }

    #[test]
    fn only_a_sharing_violation_reads_as_busy_and_access_denied_is_unusable() {
        let dir = tempfile::tempdir().unwrap();
        let src = source(dir.path(), false);

        // Windows `ERROR_SHARING_VIOLATION` (32). The platform predicate decides — on an OS without share modes the
        //   same raw code is just another read error.
        let sharing = io::Error::from_raw_os_error(32);
        let expected = if engram_dashboard_platform::fs::is_sharing_violation(&sharing) {
            Current::Busy
        } else {
            Current::Unusable(format!("{DAEMON_FILE} read failed: {sharing}"))
        };
        assert_eq!(src.judge(Err(io::Error::from_raw_os_error(32))), expected);

        for denied in [
            io::Error::from(io::ErrorKind::PermissionDenied),
            // Windows `ERROR_ACCESS_DENIED` (5) — never a fallback, it does not clear when someone lets go.
            io::Error::from_raw_os_error(5),
        ] {
            assert!(
                matches!(src.judge(Err(denied)), Current::Unusable(_)),
                "access denied must not fall back"
            );
        }
        assert_eq!(
            src.judge(Err(io::Error::from(io::ErrorKind::NotFound))),
            Current::Absent
        );
    }
}
