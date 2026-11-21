//! Where a daemon is and which credential it takes (TRD S21 trd-3 §3-6 — the seam that replaced the old `DaemonDiscovery`).
//! - [`EndpointSource::ensure`] may spawn a daemon; [`EndpointSource::current`] never does. Only the first connection calls `ensure`, outside `Registry::add`; every dial calls `current` (`link::dialer`) — so a reconnect cannot wake a daemon by construction (ADR-0021).
//! - `current` returns why there is no usable endpoint instead of `None`, because the dialer treats the reasons differently (D25 ① · D20 ①): only a sharing violation falls back to the last-known address, a version mismatch is a terminal refusal, everything else is one failed attempt.
//! - The credential is this account's token file, read after `daemon.json` (ADR-0395). A live record whose token file cannot be read is still
//!   dialed — the account check on that connection ([`EndpointSource::verify_server`]) is what tells "another account's daemon" apart; the
//!   dialer sends nothing and fails the attempt when the check passes but there is no credential.
// ADR-0021 · ADR-0310 · ADR-0395

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use std::sync::Arc;

use engram_dashboard_protocol::{DaemonInfo, Secret};
use engram_dashboard_transport::{Address, SocketEnds};

use super::account::{verify_server, Accounts, CheckFailure, OsAccounts};
use super::wire::json_class;
use super::DaemonKey;
use crate::discovery::layout::DAEMON_FILE;
use crate::discovery::{
    check_acceptable, AcceptCheck, DataLayout, DiscoveryError, PidLiveness, RealLiveness,
    StartFailure,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub key: DaemonKey,
    pub address: Address,
    /// This account's token for that daemon; `None` = its token file could not be read (the dialer still checks the listener's account,
    ///   then fails the attempt without sending anything).
    pub credential: Option<Secret>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Current {
    Live(Endpoint),
    /// Another process holds the endpoint record without sharing it (virus scanner · indexer — Windows sharing violation). The owner that wrote it is likely still alive, so the last-known endpoint is worth one dial.
    Busy,
    Absent,
    /// A record that cannot be used: empty · broken · a dead pid · access denied · any other read error. The text is a local diagnostic and quotes no file content.
    Unusable(String),
    /// A live daemon on another protocol version — dialing it can only be refused. `pid` / `start_time` = that daemon's identity from the record (`start_time` 0 = none).
    VersionMismatch {
        daemon: u32,
        pid: u32,
        start_time: u64,
    },
}

pub trait EndpointSource: Send + Sync {
    /// `Err` carries the typed start refusal the band shows when the search was refused before a spawn (ADR-0399).
    fn ensure(&self, timeout: Duration) -> Result<Endpoint, StartFailure>;
    fn current(&self) -> Current;
    /// Whether the server end of a connection just dialed to this source's endpoint runs under this app's OS account — asked after every
    ///   dial, before the token goes out (ADR-0395). Blocking (OS tables). `ends` = the dialed link's TCP ends.
    fn verify_server(&self, ends: Option<SocketEnds>) -> Result<(), CheckFailure>;
}

/// The local daemon of one data folder (`daemon.json`). It reads the file itself instead of through `discovery::FileReader`, which turns the read error into text and loses the `io::Error` that tells a sharing violation apart (D25 ①).
pub struct LocalEndpointSource {
    key: DaemonKey,
    data_dir: PathBuf,
    liveness: Box<dyn PidLiveness + Send + Sync>,
    accounts: Arc<dyn Accounts>,
}

impl LocalEndpointSource {
    pub fn new(key: DaemonKey, data_dir: PathBuf) -> Self {
        Self {
            key,
            data_dir,
            liveness: Box::new(RealLiveness),
            accounts: Arc::new(OsAccounts),
        }
    }

    #[cfg(test)]
    pub(super) fn with_liveness(
        key: DaemonKey,
        data_dir: PathBuf,
        liveness: impl PidLiveness + Send + Sync + 'static,
        accounts: Arc<dyn Accounts>,
    ) -> Self {
        Self {
            key,
            data_dir,
            liveness: Box::new(liveness),
            accounts,
        }
    }

    /// The endpoint of the record `info` — its token read now, after the record (ADR-0395).
    fn endpoint(&self, info: DaemonInfo) -> Endpoint {
        Endpoint {
            key: self.key.clone(),
            address: Address::new(format!("ws://{}:{}", info.host, info.port)),
            credential: self.token(),
        }
    }

    /// This account's token, or `None` (logged) — another account's live daemon leaves no file of ours, so this is not an error yet.
    fn token(&self) -> Option<Secret> {
        let read = self
            .accounts
            .current()
            .and_then(|account| crate::discovery::read_token(&self.data_dir, &account));
        match read {
            Ok(token) => Some(token),
            Err(e) => {
                tracing::debug!("this account's daemon token file could not be read: {e}");
                None
            }
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
    fn ensure(&self, timeout: Duration) -> Result<Endpoint, StartFailure> {
        let exe = crate::discovery::locate_daemon_exe()?;
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
                Err(e.into())
            }
        }
    }

    fn current(&self) -> Current {
        let path = DataLayout::new(&self.data_dir).daemon_file();
        self.judge(std::fs::read(path))
    }

    fn verify_server(&self, ends: Option<SocketEnds>) -> Result<(), CheckFailure> {
        verify_server(self.accounts.as_ref(), ends)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::path::Path;

    use engram_dashboard_protocol::PROTOCOL_VERSION;

    use super::super::account::fakes::{ends, FixedAccounts, OTHER, OWN};

    const TOKEN: &str = "feedfacefeedfacefeedfacefeedfacefeedfacefeedfacefeedfacefeedface";

    struct FixedLiveness {
        dead: bool,
    }

    impl PidLiveness for FixedLiveness {
        fn is_dead(&self, _pid: u32, _start_time: u64) -> bool {
            self.dead
        }
    }

    /// The source as this account ([`OWN`]); every listener runs as `server`.
    fn source_as(dir: &Path, dead: bool, server: Option<&'static str>) -> LocalEndpointSource {
        LocalEndpointSource::with_liveness(
            DaemonKey::new("local"),
            dir.to_path_buf(),
            FixedLiveness { dead },
            Arc::new(FixedAccounts(server)),
        )
    }

    fn source(dir: &Path, dead: bool) -> LocalEndpointSource {
        source_as(dir, dead, Some(OWN))
    }

    fn write_record(dir: &Path, bytes: &[u8]) {
        let path = DataLayout::new(dir).daemon_file();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn write_token(dir: &Path, account: &str, token: &str) {
        let path = DataLayout::new(dir).token_file(account);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, token).unwrap();
    }

    fn record(port: u16, version: u32) -> Vec<u8> {
        DaemonInfo {
            pid: 4242,
            host: "127.0.0.1".into(),
            port,
            protocol_version: version,
            start_time: 0,
            owner_account: OWN.into(),
        }
        .to_json_pretty()
        .unwrap()
    }

    #[test]
    fn a_live_record_of_our_version_is_a_live_endpoint_with_this_accounts_token() {
        let dir = tempfile::tempdir().unwrap();
        write_record(dir.path(), &record(5123, PROTOCOL_VERSION));
        write_token(dir.path(), OWN, TOKEN);
        write_token(dir.path(), OTHER, "another account's token");
        assert_eq!(
            source(dir.path(), false).current(),
            Current::Live(Endpoint {
                key: DaemonKey::new("local"),
                address: Address::new("ws://127.0.0.1:5123"),
                credential: Some(Secret::new(TOKEN)),
            })
        );
    }

    /// Another account's live daemon leaves no token file of ours: the record is still a live endpoint (the dial's account check names
    ///   the cause), with no credential.
    #[test]
    fn a_live_record_without_our_token_file_is_live_without_a_credential() {
        let dir = tempfile::tempdir().unwrap();
        write_record(dir.path(), &record(5123, PROTOCOL_VERSION));
        match source(dir.path(), false).current() {
            Current::Live(endpoint) => assert_eq!(endpoint.credential, None),
            other => panic!("expected a live endpoint, got {other:?}"),
        }
    }

    /// The restart race: the token is read after the record, so a record published after a token rewrite pairs with the new token —
    ///   each read of `current` takes the token file as it is after that record.
    #[test]
    fn every_read_takes_the_token_after_the_record() {
        let dir = tempfile::tempdir().unwrap();
        let src = source(dir.path(), false);
        write_token(dir.path(), OWN, "old-token");
        write_record(dir.path(), &record(5123, PROTOCOL_VERSION));
        let credential = |c: Current| match c {
            Current::Live(ep) => ep.credential,
            other => panic!("expected a live endpoint, got {other:?}"),
        };
        assert_eq!(credential(src.current()), Some(Secret::new("old-token")));
        // A restarted daemon writes its token first, then its record (instance `write_token_file` · `publish`).
        write_token(dir.path(), OWN, "new-token");
        write_record(dir.path(), &record(6234, PROTOCOL_VERSION));
        match src.current() {
            Current::Live(ep) => {
                assert_eq!(ep.address, Address::new("ws://127.0.0.1:6234"));
                assert_eq!(ep.credential, Some(Secret::new("new-token")));
            }
            other => panic!("expected a live endpoint, got {other:?}"),
        }
    }

    /// A daemon that restarts between the two reads: the record read first is the old one and the token read after it is the new one —
    ///   never a new record with an old token. (Read in the other order, this read would pair the new record with whatever token came
    ///   before it.) The restart is staged inside the token read's account lookup, which runs only after the record was read.
    #[test]
    fn a_restart_between_the_two_reads_never_pairs_a_new_record_with_an_old_token() {
        struct RestartsOnTokenRead(std::path::PathBuf);
        impl Accounts for RestartsOnTokenRead {
            fn current(&self) -> std::io::Result<String> {
                // The restarted daemon: its token first, then its record.
                write_token(&self.0, OWN, "new-token");
                write_record(&self.0, &record(6234, PROTOCOL_VERSION));
                Ok(OWN.to_owned())
            }
            fn server_account(
                &self,
                _server: std::net::SocketAddrV4,
                _client: std::net::SocketAddrV4,
            ) -> Result<String, engram_dashboard_platform::account::TcpOwnerError> {
                Ok(OWN.to_owned())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        write_token(dir.path(), OWN, "old-token");
        write_record(dir.path(), &record(5123, PROTOCOL_VERSION));
        let src = LocalEndpointSource::with_liveness(
            DaemonKey::new("local"),
            dir.path().to_path_buf(),
            FixedLiveness { dead: false },
            Arc::new(RestartsOnTokenRead(dir.path().to_path_buf())),
        );
        match src.current() {
            Current::Live(ep) => {
                assert_eq!(
                    ep.address,
                    Address::new("ws://127.0.0.1:5123"),
                    "the record read first"
                );
                assert_eq!(
                    ep.credential,
                    Some(Secret::new("new-token")),
                    "the token read after it"
                );
            }
            other => panic!("expected a live endpoint, got {other:?}"),
        }
    }

    #[test]
    fn the_listener_check_passes_only_this_accounts_listener() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(source(dir.path(), false).verify_server(ends()), Ok(()));
        assert_eq!(
            source_as(dir.path(), false, Some(OTHER)).verify_server(ends()),
            Err(CheckFailure::Refused(
                super::super::account::AccountRefusal::OtherAccount
            ))
        );
        assert_eq!(
            source_as(dir.path(), false, None).verify_server(ends()),
            Err(CheckFailure::Refused(
                super::super::account::AccountRefusal::Unverified
            ))
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
            br#"{"pid":1,"host":"127.0.0.1","port":"peer-text","protocol_version":8}"#,
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

        // Windows `ERROR_SHARING_VIOLATION` (32). The platform predicate decides — on an OS without share modes the same raw code is just another read error.
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
