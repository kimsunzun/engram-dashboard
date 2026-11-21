//! The accept-time account check — the shell link (`link::server`) and the control HTTP listener (`control::mcp_server`) admit a client
//!   only when its process runs under the daemon's own OS account, before any handshake or bearer check (ADR-0395 decision 5).
//!
//! - Mechanism (platform `account::tcp_client_account`): the TCP-table row whose local end is the observed client address, its owning
//!   process, that process's token user. IPv4 only — both listeners bind `127.0.0.1`; any other address cannot be verified.
//! - ★Fail closed★: a lookup that cannot read the client's account refuses, with its own message — every `TcpOwnerError` means "cannot
//!   verify". An elevated administrator is not kept out (its token's user is still the account that runs it).
//! - The verdict is per connection: the link checks once at accept; the HTTP listener checks every request, since axum hands requests
//!   rather than connections to the middleware (control traffic is low-frequency, and caching by address would let a later connection
//!   reusing that port inherit a verdict).
//! - The lookup reads OS tables and opens processes (blocking), so [`AccountGate::admit`] runs it on the blocking pool.
//! - Refusals are logged throttled per kind ([`RefusalLog`]): another account can connect in a loop, and a line per refusal would let it
//!   flood the log (the per-branch time-window batched refusal log of ADR-0310 D9 · ADR-0346 decision 5).
// ADR-0395

use std::net::{SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use engram_dashboard_platform::account::TcpOwnerError;

/// The refusal for a client of another account.
pub(crate) const OTHER_ACCOUNT: &str =
    "this daemon belongs to another Windows account — use a separate folder per account";
/// The refusal when the client's account cannot be read (fail closed).
pub(crate) const UNVERIFIED: &str = "could not verify the connecting app's account — run the app and Engram the same way, both elevated or both not";

/// Which account runs the client end of a local connection — the seam the tests fake (a second OS account is not available to them).
pub(crate) trait ClientAccounts: Send + Sync + 'static {
    /// The account text (as platform `account::current_account_id`) running the client end of `client` → `server`.
    fn client_account(
        &self,
        client: SocketAddrV4,
        server: SocketAddrV4,
    ) -> Result<String, TcpOwnerError>;
}

/// The real lookup — platform's TCP-table owner proof.
pub(crate) struct OsClientAccounts;

impl ClientAccounts for OsClientAccounts {
    fn client_account(
        &self,
        client: SocketAddrV4,
        server: SocketAddrV4,
    ) -> Result<String, TcpOwnerError> {
        engram_dashboard_platform::account::tcp_client_account(client, server)
    }
}

/// Why a client was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    OtherAccount,
    Unverified,
}

impl Refusal {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::OtherAccount => OTHER_ACCOUNT,
            Self::Unverified => UNVERIFIED,
        }
    }
}

/// The daemon's own account and the lookup it checks clients with. Cheap to clone (shared lookup and refusal log).
#[derive(Clone)]
pub(crate) struct AccountGate {
    own: Arc<str>,
    lookup: Arc<dyn ClientAccounts>,
    log: Arc<Mutex<RefusalLog>>,
}

impl AccountGate {
    /// `own` = the daemon's account (platform `account::current_account_id`, read once at start).
    pub(crate) fn new(own: &str, lookup: Arc<dyn ClientAccounts>) -> Self {
        Self {
            own: own.into(),
            lookup,
            log: Arc::default(),
        }
    }

    pub(crate) fn os(own: &str) -> Self {
        Self::new(own, Arc::new(OsClientAccounts))
    }

    /// Admits `client` (the observed peer of a connection to this listener's `server` address) only when its process runs under the
    ///   daemon's account. A missing address, or one that is not IPv4, cannot be verified. The refusal is logged throttled.
    pub(crate) async fn admit(
        &self,
        client: Option<SocketAddr>,
        server: Option<SocketAddr>,
    ) -> Result<(), Refusal> {
        let verdict = self.decide(client, server).await;
        if let Err((refusal, detail)) = &verdict {
            let line = self
                .log
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .record(*refusal, Instant::now());
            match line {
                LogLine::First => tracing::warn!(?client, "{}: {detail}", refusal.message()),
                LogLine::Summary { count } => tracing::warn!(
                    count,
                    ?client,
                    "{} — {count} refusals of this kind since the last line; the latest: {detail}",
                    refusal.message()
                ),
                LogLine::Quiet => tracing::debug!(?client, "{}: {detail}", refusal.message()),
            }
        }
        verdict.map_err(|(refusal, _)| refusal)
    }

    /// The verdict with the refusal's detail for the log.
    async fn decide(
        &self,
        client: Option<SocketAddr>,
        server: Option<SocketAddr>,
    ) -> Result<(), (Refusal, String)> {
        let (Some(SocketAddr::V4(client)), Some(SocketAddr::V4(server))) = (client, server) else {
            return Err((
                Refusal::Unverified,
                "the connection has no IPv4 client and server address".into(),
            ));
        };
        let lookup = self.lookup.clone();
        let found = tokio::task::spawn_blocking(move || lookup.client_account(client, server))
            .await
            .map_err(|e| (Refusal::Unverified, format!("the lookup task failed: {e}")))?;
        match found {
            Ok(account) if *account == *self.own => Ok(()),
            Ok(_) => Err((
                Refusal::OtherAccount,
                "the client process runs under another OS account".into(),
            )),
            // Another standard account's process cannot even be opened — the usual cross-account case.
            Err(e @ TcpOwnerError::ProcessAccessDenied) => {
                Err((Refusal::OtherAccount, e.to_string()))
            }
            Err(e) => Err((Refusal::Unverified, e.to_string())),
        }
    }
}

/// Throttles the refusal log per [`Refusal`] kind: the first refusal of a kind gets a line; while it repeats, one summary line at most
///   every [`RefusalLog::SUMMARY_EVERY`] with the count since the previous line; the rest go to debug. Same shape as the shell's failed
///   attempt log (`daemon_client::link::route` `FailureLog`).
#[derive(Default)]
struct RefusalLog {
    other_account: KindLog,
    unverified: KindLog,
}

#[derive(Default)]
struct KindLog {
    line_at: Option<Instant>,
    /// Refusals since the last line.
    unlogged: u32,
}

/// What [`RefusalLog::record`] says to log.
#[derive(Debug, PartialEq, Eq)]
enum LogLine {
    First,
    Summary { count: u32 },
    Quiet,
}

impl RefusalLog {
    const SUMMARY_EVERY: Duration = Duration::from_secs(60);

    fn record(&mut self, refusal: Refusal, now: Instant) -> LogLine {
        let kind = match refusal {
            Refusal::OtherAccount => &mut self.other_account,
            Refusal::Unverified => &mut self.unverified,
        };
        let Some(at) = kind.line_at else {
            kind.line_at = Some(now);
            return LogLine::First;
        };
        kind.unlogged = kind.unlogged.saturating_add(1);
        if now.saturating_duration_since(at) < Self::SUMMARY_EVERY {
            return LogLine::Quiet;
        }
        kind.line_at = Some(now);
        LogLine::Summary {
            count: std::mem::take(&mut kind.unlogged),
        }
    }
}

#[cfg(test)]
pub(crate) mod fakes {
    use super::*;

    /// Answers every lookup with a fixed account, or with "cannot verify".
    pub(crate) struct FixedAccount(pub Option<&'static str>);

    impl ClientAccounts for FixedAccount {
        fn client_account(
            &self,
            _client: SocketAddrV4,
            _server: SocketAddrV4,
        ) -> Result<String, TcpOwnerError> {
            self.0
                .map(str::to_owned)
                .ok_or(TcpOwnerError::NoSuchConnection)
        }
    }

    pub(crate) const OWN: &str = "S-1-5-21-1-2-3-1001";
    pub(crate) const OTHER: &str = "S-1-5-21-1-2-3-1002";

    /// A gate whose lookup answers `client` (`None` = cannot verify) for a daemon running as [`OWN`].
    pub(crate) fn gate(client: Option<&'static str>) -> AccountGate {
        AccountGate::new(OWN, Arc::new(FixedAccount(client)))
    }
}

#[cfg(test)]
mod tests {
    use super::fakes::{gate, OTHER, OWN};
    use super::*;

    fn v4(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    fn some(port: u16) -> Option<SocketAddr> {
        Some(v4(port))
    }

    #[test]
    fn refusals_are_logged_once_then_summarised_per_kind_at_most_once_a_window() {
        let mut log = RefusalLog::default();
        let t0 = Instant::now();
        let after = |secs| t0 + Duration::from_secs(secs);
        assert_eq!(log.record(Refusal::OtherAccount, t0), LogLine::First);
        assert_eq!(log.record(Refusal::OtherAccount, after(1)), LogLine::Quiet);
        assert_eq!(log.record(Refusal::OtherAccount, after(2)), LogLine::Quiet);
        // The other kind keeps its own window.
        assert_eq!(log.record(Refusal::Unverified, after(3)), LogLine::First);
        assert_eq!(
            log.record(Refusal::OtherAccount, after(60)),
            LogLine::Summary { count: 3 }
        );
        assert_eq!(log.record(Refusal::OtherAccount, after(61)), LogLine::Quiet);
        assert_eq!(
            log.record(Refusal::OtherAccount, after(121)),
            LogLine::Summary { count: 2 }
        );
    }

    #[tokio::test]
    async fn the_same_account_is_admitted_and_another_refused() {
        assert_eq!(gate(Some(OWN)).admit(some(5000), some(80)).await, Ok(()));
        assert_eq!(
            gate(Some(OTHER)).admit(some(5000), some(80)).await,
            Err(Refusal::OtherAccount)
        );
    }

    /// A client process of another standard account cannot be opened at all (platform `ProcessAccessDenied`) — refused as another
    ///   account, not as unverifiable. (A real second account is U5.)
    #[tokio::test]
    async fn a_client_process_that_may_not_be_opened_is_another_accounts() {
        struct Denied;
        impl ClientAccounts for Denied {
            fn client_account(
                &self,
                _client: SocketAddrV4,
                _server: SocketAddrV4,
            ) -> Result<String, TcpOwnerError> {
                Err(TcpOwnerError::ProcessAccessDenied)
            }
        }
        let gate = AccountGate::new(OWN, Arc::new(Denied));
        assert_eq!(
            gate.admit(some(5000), some(80)).await,
            Err(Refusal::OtherAccount)
        );
    }

    #[tokio::test]
    async fn an_unreadable_account_or_address_fails_closed() {
        assert_eq!(
            gate(None).admit(some(5000), some(80)).await,
            Err(Refusal::Unverified)
        );
        assert_eq!(
            gate(Some(OWN)).admit(None, some(80)).await,
            Err(Refusal::Unverified)
        );
        assert_eq!(
            gate(Some(OWN)).admit(some(5000), None).await,
            Err(Refusal::Unverified)
        );
        let v6: SocketAddr = "[::1]:5000".parse().unwrap();
        assert_eq!(
            gate(Some(OWN)).admit(Some(v6), some(80)).await,
            Err(Refusal::Unverified)
        );
    }

    #[test]
    fn the_two_refusals_carry_their_own_messages() {
        assert_eq!(Refusal::OtherAccount.message(), OTHER_ACCOUNT);
        assert_eq!(Refusal::Unverified.message(), UNVERIFIED);
        assert_ne!(OTHER_ACCOUNT, UNVERIFIED);
    }

    /// The real lookup on a real loopback connection made by this test process — the same account — is admitted (measures the platform
    ///   lookup over loopback TCP once; another account is not available here).
    #[tokio::test]
    async fn the_real_lookup_admits_a_client_of_this_process() {
        let own = match engram_dashboard_platform::account::current_account_id() {
            Ok(own) => own,
            // Outside Windows the account functions are not built (T-118 / T-119) — nothing to measure.
            Err(e) if e.kind() == std::io::ErrorKind::Unsupported => return,
            Err(e) => panic!("current account: {e}"),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server = listener.local_addr().unwrap();
        let client = tokio::net::TcpStream::connect(server).await.unwrap();
        let (_accepted, observed) = listener.accept().await.unwrap();
        assert_eq!(observed, client.local_addr().unwrap());
        assert_eq!(
            AccountGate::os(&own)
                .admit(Some(observed), Some(server))
                .await,
            Ok(())
        );
        assert_eq!(
            AccountGate::os("S-1-5-21-0-0-0-0")
                .admit(Some(observed), Some(server))
                .await,
            Err(Refusal::OtherAccount),
            "the real account is compared, not just looked up"
        );
    }
}
