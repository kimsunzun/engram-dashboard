//! The shell's side of the OS-account boundary (ADR-0395 decision 6): before its token goes out, on the connection it just opened, the
//!   shell checks that the process owning the server end runs under the shell's own OS account — and refuses otherwise, sending nothing.
//!
//! - Why on the shell's own connection: another account can write a live-looking `daemon.json` (its own pid and port, this account's SID as
//!   owner), and the daemon link authenticates only the client — without this check a forged record would collect the token. Checking the
//!   connection after it opened means the port cannot change hands between the check and the token.
//! - Mechanism (platform `account::tcp_server_account`): the TCP-table row of the server end, its owning process, that process's token
//!   user. ★Fail closed★: a lookup that cannot read the account refuses ([`AccountRefusal::Unverified`]); so does a link without typed TCP
//!   ends, or one whose far end is not an IPv4 loopback address (the daemon binds `127.0.0.1`).
//! - A connection or owner that vanished during the lookup (platform `TcpOwnerError::is_gone` — e.g. the daemon exited between the dial
//!   and the check) is [`CheckFailure::Vanished`]: nothing is sent either, but it is one failed attempt, not a refusal — the next dial may
//!   meet a daemon of this account.
//! - Where it runs: the link's dialer asks the endpoint source after each dial (`link::dialer` — the refusal reaches the band through the
//!   registration's `RefusalSlot`); the stop client dials through [`AccountCheckedDialer`], which records a refusal in an
//!   [`AccountRefusalRecord`] so the stop flight can tell it apart and kill nothing.
// ADR-0395

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use engram_dashboard_platform::account::TcpOwnerError;
use engram_dashboard_transport::{
    Address, CloseCode, DialFailure, Dialer, LinkError, LinkRx, LinkTx, SocketEnds,
};
use futures_util::future::BoxFuture;

/// The English refusal texts — for logs and the `AuthRejected` answer; the band shows its own translated words (`src/i18n`).
pub const OTHER_ACCOUNT: &str =
    "this daemon belongs to another Windows account — use a separate folder per account";
pub const UNVERIFIED: &str = "could not verify the daemon's account — run the app and Engram the same way, both elevated or both not";
/// The failed attempt when the listener went away during its check.
pub const LISTENER_GONE: &str = "the daemon listener went away during its account check";

/// Why the shell refused to send its token on a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountRefusal {
    /// The listener runs under another OS account.
    OtherAccount,
    /// Its account could not be read (fail closed).
    Unverified,
}

impl AccountRefusal {
    pub fn message(self) -> &'static str {
        match self {
            Self::OtherAccount => OTHER_ACCOUNT,
            Self::Unverified => UNVERIFIED,
        }
    }
}

/// Why [`verify_server`] did not pass. Either way nothing is sent on that connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckFailure {
    /// A terminal refusal — redialing meets the same listener.
    Refused(AccountRefusal),
    /// The connection or its owner went away during the lookup — one failed attempt.
    Vanished,
}

/// The OS facts the check needs — the seam the tests fake (a second OS account is not available to them).
pub trait Accounts: Send + Sync {
    /// This process's account (platform `account::current_account_id`).
    fn current(&self) -> std::io::Result<String>;
    /// The account running the server end of the connection `server` ↔ `client` (platform `account::tcp_server_account`).
    fn server_account(
        &self,
        server: std::net::SocketAddrV4,
        client: std::net::SocketAddrV4,
    ) -> Result<String, TcpOwnerError>;
}

/// The real OS.
pub struct OsAccounts;

impl Accounts for OsAccounts {
    fn current(&self) -> std::io::Result<String> {
        engram_dashboard_platform::account::current_account_id()
    }

    fn server_account(
        &self,
        server: std::net::SocketAddrV4,
        client: std::net::SocketAddrV4,
    ) -> Result<String, TcpOwnerError> {
        engram_dashboard_platform::account::tcp_server_account(server, client)
    }
}

/// Is the server end of the connection with `ends` run by this process's account? Blocking (OS tables) — callers run it off the async
///   threads.
pub fn verify_server(
    accounts: &dyn Accounts,
    ends: Option<SocketEnds>,
) -> Result<(), CheckFailure> {
    let refused = |refusal| Err(CheckFailure::Refused(refusal));
    let Some(SocketEnds {
        local: SocketAddr::V4(client),
        peer: SocketAddr::V4(server),
    }) = ends
    else {
        tracing::warn!(?ends, "the daemon connection has no IPv4 TCP ends — its account cannot be verified, nothing is sent");
        return refused(AccountRefusal::Unverified);
    };
    if !server.ip().is_loopback() {
        // The account of a socket on another machine is not in this machine's table.
        tracing::warn!(%server, "the daemon record points off this machine — its account cannot be verified, nothing is sent");
        return refused(AccountRefusal::Unverified);
    }
    let own = match accounts.current() {
        Ok(own) => own,
        Err(e) => {
            tracing::warn!(
                "this app's OS account could not be read — nothing is sent to the daemon: {e}"
            );
            return refused(AccountRefusal::Unverified);
        }
    };
    match accounts.server_account(server, client) {
        Ok(account) if account == own => Ok(()),
        Ok(_) => {
            tracing::warn!(%server, "the daemon listener runs under another OS account — the token is not sent");
            refused(AccountRefusal::OtherAccount)
        }
        // A process of another account cannot be opened at all (platform `ProcessAccessDenied`) — the usual cross-account case.
        Err(TcpOwnerError::ProcessAccessDenied) => {
            tracing::warn!(%server, "the daemon listener's process belongs to another OS account — the token is not sent");
            refused(AccountRefusal::OtherAccount)
        }
        Err(e) if e.is_gone() => {
            tracing::info!(%server, "the daemon listener went away during its account check — nothing sent, the attempt fails: {e}");
            Err(CheckFailure::Vanished)
        }
        Err(e) => {
            tracing::warn!(%server, "the daemon listener's OS account could not be verified — the token is not sent: {e}");
            refused(AccountRefusal::Unverified)
        }
    }
}

/// The dial failure for a failed check: a refusal is terminal (no redial), a vanished listener is one failed attempt.
pub fn dial_failure(failure: CheckFailure) -> DialFailure {
    match failure {
        CheckFailure::Refused(refusal) => DialFailure::Refused {
            code: CloseCode::HANDSHAKE_REJECTED,
            reason: refusal.message().to_owned(),
        },
        CheckFailure::Vanished => LinkError::new(LISTENER_GONE).into(),
    }
}

/// The last account refusal of the dials through one [`AccountCheckedDialer`] — the stop flight clears it before its dial and reads it
///   after a dial that sent nothing, so a refusal of the listener's account is not mistaken for an unreachable daemon (which it would
///   kill).
#[derive(Debug, Default)]
pub struct AccountRefusalRecord(Mutex<Option<AccountRefusal>>);

impl AccountRefusalRecord {
    pub fn take(&self) -> Option<AccountRefusal> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    fn set(&self, refusal: AccountRefusal) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(refusal);
    }
}

/// A [`Dialer`] that hands out a link only after [`verify_server`] passed on it — for a client with no band to tell (the stop client).
///   A refusal is written to `refusals` before the dial fails.
pub struct AccountCheckedDialer {
    inner: Arc<dyn Dialer>,
    accounts: Arc<dyn Accounts>,
    refusals: Arc<AccountRefusalRecord>,
}

impl AccountCheckedDialer {
    pub fn new(
        inner: Arc<dyn Dialer>,
        accounts: Arc<dyn Accounts>,
        refusals: Arc<AccountRefusalRecord>,
    ) -> Self {
        Self {
            inner,
            accounts,
            refusals,
        }
    }
}

impl Dialer for AccountCheckedDialer {
    fn dial(
        &self,
        addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>> {
        let addr = addr.clone();
        Box::pin(async move {
            let (tx, rx) = self.inner.dial(&addr).await?;
            let accounts = self.accounts.clone();
            let ends = tx.socket_ends();
            let verdict =
                tokio::task::spawn_blocking(move || verify_server(accounts.as_ref(), ends))
                    .await
                    .unwrap_or(Err(CheckFailure::Refused(AccountRefusal::Unverified)));
            match verdict {
                Ok(()) => Ok((tx, rx)),
                // Dropping both ends closes the link before anything was sent.
                Err(failure) => {
                    if let CheckFailure::Refused(refusal) = failure {
                        self.refusals.set(refusal);
                    }
                    Err(dial_failure(failure))
                }
            }
        })
    }
}

#[cfg(test)]
pub(crate) mod fakes {
    use super::*;

    pub(crate) const OWN: &str = "S-1-5-21-1-2-3-1001";
    pub(crate) const OTHER: &str = "S-1-5-21-1-2-3-1002";

    /// This process runs as [`OWN`]; every listener runs as `server` (`None` = its account cannot be read — access denied after opening,
    ///   e.g. an elevated listener's token).
    pub(crate) struct FixedAccounts(pub Option<&'static str>);

    impl Accounts for FixedAccounts {
        fn current(&self) -> std::io::Result<String> {
            Ok(OWN.to_owned())
        }

        fn server_account(
            &self,
            _server: std::net::SocketAddrV4,
            _client: std::net::SocketAddrV4,
        ) -> Result<String, TcpOwnerError> {
            self.0
                .map(str::to_owned)
                .ok_or_else(|| TcpOwnerError::Os(std::io::Error::from_raw_os_error(5)))
        }
    }

    /// Every listener's connection is gone by the time it is looked up.
    pub(crate) struct GoneListener;

    impl Accounts for GoneListener {
        fn current(&self) -> std::io::Result<String> {
            Ok(OWN.to_owned())
        }

        fn server_account(
            &self,
            _server: std::net::SocketAddrV4,
            _client: std::net::SocketAddrV4,
        ) -> Result<String, TcpOwnerError> {
            Err(TcpOwnerError::NoSuchConnection)
        }
    }

    pub(crate) fn ends() -> Option<SocketEnds> {
        Some(SocketEnds {
            local: "127.0.0.1:50000".parse().unwrap(),
            peer: "127.0.0.1:6000".parse().unwrap(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::fakes::{ends, FixedAccounts, GoneListener, OTHER, OWN};
    use super::*;

    use engram_dashboard_transport::testing::MemoryNetwork;

    const REFUSED_OTHER: Result<(), CheckFailure> =
        Err(CheckFailure::Refused(AccountRefusal::OtherAccount));
    const REFUSED_UNVERIFIED: Result<(), CheckFailure> =
        Err(CheckFailure::Refused(AccountRefusal::Unverified));

    #[test]
    fn only_a_listener_of_this_account_passes() {
        assert_eq!(verify_server(&FixedAccounts(Some(OWN)), ends()), Ok(()));
        assert_eq!(
            verify_server(&FixedAccounts(Some(OTHER)), ends()),
            REFUSED_OTHER
        );
    }

    #[test]
    fn anything_unverifiable_fails_closed() {
        assert_eq!(
            verify_server(&FixedAccounts(None), ends()),
            REFUSED_UNVERIFIED,
            "an unreadable account (access denied)"
        );
        assert_eq!(
            verify_server(&FixedAccounts(Some(OWN)), None),
            REFUSED_UNVERIFIED,
            "a link without TCP ends"
        );
        let v6 = Some(SocketEnds {
            local: "[::1]:50000".parse().unwrap(),
            peer: "[::1]:6000".parse().unwrap(),
        });
        assert_eq!(
            verify_server(&FixedAccounts(Some(OWN)), v6),
            REFUSED_UNVERIFIED,
            "an end that is not IPv4"
        );
        let remote = Some(SocketEnds {
            local: "192.168.0.2:50000".parse().unwrap(),
            peer: "192.168.0.9:6000".parse().unwrap(),
        });
        assert_eq!(
            verify_server(&FixedAccounts(Some(OWN)), remote),
            REFUSED_UNVERIFIED,
            "a listener off this machine"
        );
    }

    /// AC17's usual case: another standard account's listener process cannot even be opened — that reads as another account, not as an
    ///   elevation mismatch. (A real second account is U5.)
    #[test]
    fn a_listener_process_that_may_not_be_opened_is_another_accounts() {
        struct Denied;
        impl Accounts for Denied {
            fn current(&self) -> std::io::Result<String> {
                Ok(OWN.to_owned())
            }
            fn server_account(
                &self,
                _server: std::net::SocketAddrV4,
                _client: std::net::SocketAddrV4,
            ) -> Result<String, TcpOwnerError> {
                Err(TcpOwnerError::ProcessAccessDenied)
            }
        }
        assert_eq!(verify_server(&Denied, ends()), REFUSED_OTHER);
    }

    #[test]
    fn a_listener_that_vanished_during_the_lookup_is_not_a_refusal() {
        assert_eq!(
            verify_server(&GoneListener, ends()),
            Err(CheckFailure::Vanished)
        );
    }

    #[test]
    fn a_refusal_is_terminal_and_a_vanished_listener_one_failed_attempt() {
        for (refusal, text) in [
            (AccountRefusal::OtherAccount, OTHER_ACCOUNT),
            (AccountRefusal::Unverified, UNVERIFIED),
        ] {
            assert_eq!(
                dial_failure(CheckFailure::Refused(refusal)),
                DialFailure::Refused {
                    code: CloseCode::HANDSHAKE_REJECTED,
                    reason: text.to_owned(),
                }
            );
        }
        assert!(matches!(
            dial_failure(CheckFailure::Vanished),
            DialFailure::Unreachable(e) if e.message == LISTENER_GONE
        ));
    }

    /// The checked dialer: a listener of another account → a terminal refusal, recorded, and the link dropped with nothing written; a
    ///   listener of this account → the link is handed out. (A memory link has no TCP ends, so the "own account" case goes through a fake
    ///   that passes any ends — the refusal case is what this pins.)
    #[tokio::test]
    async fn the_checked_dialer_refuses_records_and_sends_nothing() {
        let net = MemoryNetwork::new();
        let record = Arc::new(AccountRefusalRecord::default());
        let dialer = AccountCheckedDialer::new(
            net.dialer(),
            Arc::new(FixedAccounts(Some(OTHER))),
            record.clone(),
        );
        let failure = dialer
            .dial(&Address::new("ws://127.0.0.1:6000"))
            .await
            .err();
        assert!(
            matches!(
                &failure,
                Some(DialFailure::Refused { code, .. }) if *code == CloseCode::HANDSHAKE_REJECTED
            ),
            "{failure:?}"
        );
        // A memory link has no TCP ends, so it is refused as unverifiable — still an account refusal, recorded.
        assert_eq!(record.take(), Some(AccountRefusal::Unverified));
        assert_eq!(record.take(), None, "taken once");
        assert_eq!(net.dials(), 1);
        let far = net.accept().expect("the dialed link's far end");
        assert!(far.frames().is_empty(), "nothing was sent");
    }

    /// On real TCP ends the checked dialer compares the real listener's account: a ws listener of this test process, seen through a fake
    ///   that names another account for it, is refused with `OtherAccount`; seen as this account, it is handed out.
    #[tokio::test]
    async fn the_checked_dialer_compares_the_listener_account_on_real_tcp_ends() {
        use engram_dashboard_transport::ws::WsDialer;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepting = tokio::spawn(async move {
            let mut kept = Vec::new();
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                kept.push(tokio_tungstenite::accept_async(stream).await.unwrap());
            }
        });
        for (listener_account, want) in [(OTHER, Some(AccountRefusal::OtherAccount)), (OWN, None)] {
            let record = Arc::new(AccountRefusalRecord::default());
            let dialer = AccountCheckedDialer::new(
                Arc::new(WsDialer::new(1 << 20)),
                Arc::new(FixedAccounts(Some(listener_account))),
                record.clone(),
            );
            let dialed = dialer
                .dial(&Address::new(format!("ws://127.0.0.1:{port}/")))
                .await;
            assert_eq!(dialed.is_ok(), want.is_none(), "{listener_account}");
            assert_eq!(record.take(), want, "{listener_account}");
        }
        accepting.abort();
    }

    /// The real lookup on a real loopback connection of this test process (same account) passes — the platform lookup measured once
    ///   over loopback TCP from the client side.
    #[tokio::test]
    async fn the_real_lookup_passes_a_listener_of_this_process() {
        if let Err(e) = OsAccounts.current() {
            // Outside Windows the account functions are not built (T-118 / T-119).
            assert_eq!(e.kind(), std::io::ErrorKind::Unsupported, "{e}");
            return;
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let _accepted = listener.accept().await.unwrap();
        let ends = Some(SocketEnds {
            local: client.local_addr().unwrap(),
            peer: client.peer_addr().unwrap(),
        });
        let verdict = tokio::task::spawn_blocking(move || verify_server(&OsAccounts, ends))
            .await
            .unwrap();
        assert_eq!(verdict, Ok(()));
    }
}
