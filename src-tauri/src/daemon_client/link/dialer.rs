//! The dialer the shell's registry uses — resolves a logical daemon address to where that daemon is now
//! (TRD S21 trd-3 §3-6 · D25 ① · D20 ①).
//!
//! The registry dials one fixed address per peer for its whole life; a daemon moves to a new port and token on every
//! restart. So the registry gets a logical address (`engram-local:<key>#<n>`) and every dial here asks that
//! registration's [`EndpointSource::current`] — never `ensure`, so a reconnect cannot spawn a daemon (ADR-0021):
//!
//! - `Live` → remember it as the last-known endpoint, put its credential in the registration's [`CredentialSlot`],
//!   dial it.
//! - `Busy` (sharing violation only) → dial the last-known endpoint with its credential; none known → one failed
//!   attempt.
//! - `Absent` · `Unusable` → forget the last-known endpoint, one failed attempt (budget), no dial.
//! - `VersionMismatch` → forget the last-known endpoint, a terminal refusal (`HANDSHAKE_REJECTED`) without a dial: the
//!   transport gives up as `Rejected` and spends no budget (ADR-0180 decision 5). Logged here at warn. The refusal is
//!   recorded in the registration's [`RefusalSlot`] first — the one way routing tells a version refusal from the
//!   daemon's own refusal, which carries the same close code (TRD S21 trd-3-2-u5 §4-3).
//!
//! The last-known endpoint is forgotten on every answer but `Live` and `Busy` because those answers say its daemon is
//! gone or unusable (TRD §3-6) — a later `Busy` must not send that daemon's token to whoever holds its old port now.
//!
//! ★One registration per `Registry::add`★ — the credential slot is written by a dial and read by the handshake of
//! the same attempt, which the transport runs in that order on one supervisor task. A re-add of the same key starts a
//! new supervisor while the old one may still finish a dial it began (closing races that dial), so a slot shared by
//! two supervisors could pair one daemon's token with the other daemon's address. Each registration therefore has its
//! own address, slot, source and last-known endpoint; never pass one registration's address to two adds.
// ADR-0021
// ADR-0310

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use engram_dashboard_protocol::PROTOCOL_VERSION;
use engram_dashboard_transport::{
    Address, CloseCode, DialFailure, Dialer, Handshake, LinkError, LinkRx, LinkTx,
};
use futures_util::future::BoxFuture;

use super::endpoint::{Current, Endpoint, EndpointSource};
use super::handshake::{CredentialSlot, ShellHandshake};
use super::DaemonKey;

/// The texts of the failed attempts this dialer makes — routing keys its throttled failure log on them
/// (`route::UnreachableClass`), and a handle's first answer reads the two that mean "no live daemon"
/// ([`no_live_daemon_cause`]). Nothing else decides on them.
pub(super) const NOT_REGISTERED: &str = "no daemon registered at";
pub(super) const READ_FAILED: &str = "reading the daemon endpoint failed";
pub(super) const BUSY_UNKNOWN: &str =
    "the daemon endpoint record is busy and no earlier address is known";
pub(super) const NO_RECORD: &str = "no daemon endpoint record — no live daemon";
pub(super) const UNUSABLE: &str = "the daemon endpoint record is unusable";

/// The cause when a failed attempt's text says there is no live daemon — no endpoint record ([`NO_RECORD`]), or one
/// that cannot be used ([`UNUSABLE`] — broken · dead pid · unreadable); `None` for any other failure. Only this
/// dialer writes texts with these openings (the ws dialer's start with `ws connect`, the transport's are fixed
/// words), and the cause is the endpoint source's local diagnostic — it quotes no file content.
pub(crate) fn no_live_daemon_cause(failure: &str) -> Option<String> {
    if failure == NO_RECORD {
        return Some("no endpoint record".to_owned());
    }
    failure
        .strip_prefix(UNUSABLE)
        .and_then(|rest| rest.strip_prefix(": "))
        .map(str::to_owned)
}

/// The daemon a dial refused for its protocol version, as its endpoint record names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionRefusal {
    /// The daemon's protocol version.
    pub daemon: u32,
    pub pid: u32,
    /// The daemon's process creation time from its record — 0 = the record carries none.
    pub start_time: u64,
}

/// What the last dial of one registration refused for the protocol version — written before that dial's
/// `DialFailure::Refused` returns, so the `Rejected` it becomes is routed after the write, and cleared by every other
/// answer of the endpoint record. Empty = the refusal (if any) did not come from the version check.
#[derive(Debug, Default)]
pub struct RefusalSlot(Mutex<Option<VersionRefusal>>);

impl RefusalSlot {
    pub fn get(&self) -> Option<VersionRefusal> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn set(&self, refusal: Option<VersionRefusal>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = refusal;
    }
}

struct Registration {
    source: Arc<dyn EndpointSource>,
    credential: Arc<CredentialSlot>,
    refusal: Arc<RefusalSlot>,
    last_known: Mutex<Option<Endpoint>>,
}

/// See the module doc. `inner` dials the resolved address — the transport's ws dialer in production.
pub struct LocalDaemonDialer {
    inner: Arc<dyn Dialer>,
    next: AtomicU64,
    registrations: Mutex<HashMap<Address, Arc<Registration>>>,
}

impl LocalDaemonDialer {
    pub fn new(inner: Arc<dyn Dialer>) -> Self {
        Self {
            inner,
            next: AtomicU64::new(0),
            registrations: Mutex::new(HashMap::new()),
        }
    }

    /// Registers one `Registry::add` of `key` and returns the logical address and the handshake to pass to it, and the
    /// registration's [`RefusalSlot`]. Every call makes a new registration, also for a key registered before. `found` =
    /// the endpoint the daemon search of this connect just returned (`EndpointSource::ensure`) — the registration
    /// starts with it as its last-known endpoint, so a record held busy right after the search (a daemon it just
    /// spawned) still dials that daemon. Every dial still reads the record first.
    pub fn register(
        &self,
        key: &DaemonKey,
        source: Arc<dyn EndpointSource>,
        found: Option<Endpoint>,
    ) -> (Address, Arc<dyn Handshake>, Arc<RefusalSlot>) {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        let address = Address::new(format!("engram-local:{}#{n}", key.peer_id().as_str()));
        let credential = Arc::new(CredentialSlot::default());
        let refusal = Arc::new(RefusalSlot::default());
        let registration = Arc::new(Registration {
            source,
            credential: credential.clone(),
            refusal: refusal.clone(),
            last_known: Mutex::new(found),
        });
        self.registrations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(address.clone(), registration);
        (address, Arc::new(ShellHandshake::new(credential)), refusal)
    }

    /// Drops a registration once its peer has ended. A dial of it that is still running finishes against what it
    /// already holds; later dials of the address fail as unreachable.
    pub fn unregister(&self, address: &Address) {
        self.registrations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(address);
    }

    /// How many registrations are held — for tests of the callers' unregistering.
    #[cfg(test)]
    pub(crate) fn registered(&self) -> usize {
        self.registrations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    async fn resolve(&self, addr: &Address) -> Result<Address, DialFailure> {
        let registration = self
            .registrations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(addr)
            .cloned()
            .ok_or_else(|| LinkError::new(format!("{NOT_REGISTERED} {addr}")))?;
        let source = registration.source.clone();
        // Blocking file I/O. A dial dropped mid-read leaves the read to finish on its own — it has no side effect.
        let current = tokio::task::spawn_blocking(move || source.current())
            .await
            .map_err(|e| LinkError::new(format!("{READ_FAILED}: {e}")))?;
        let version_refusal = match current {
            Current::VersionMismatch {
                daemon,
                pid,
                start_time,
            } => Some(VersionRefusal {
                daemon,
                pid,
                start_time,
            }),
            _ => None,
        };
        registration.refusal.set(version_refusal);
        let mut last_known = registration
            .last_known
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let endpoint = match current {
            Current::Live(endpoint) => {
                *last_known = Some(endpoint.clone());
                endpoint
            }
            Current::Busy => match last_known.as_ref() {
                Some(endpoint) => {
                    tracing::warn!(
                        "the daemon endpoint record is held by another process — dialing the last-known address"
                    );
                    endpoint.clone()
                }
                None => return Err(LinkError::new(BUSY_UNKNOWN).into()),
            },
            Current::Absent => {
                *last_known = None;
                return Err(LinkError::new(NO_RECORD).into());
            }
            Current::Unusable(why) => {
                *last_known = None;
                return Err(LinkError::new(format!("{UNUSABLE}: {why}")).into());
            }
            Current::VersionMismatch { daemon, .. } => {
                *last_known = None;
                tracing::warn!(
                    daemon,
                    expected = PROTOCOL_VERSION,
                    "the daemon speaks another protocol version — giving up without dialing"
                );
                return Err(DialFailure::Refused {
                    code: CloseCode::HANDSHAKE_REJECTED,
                    reason: format!(
                        "daemon protocol version {daemon} differs from the shell's {PROTOCOL_VERSION}"
                    ),
                });
            }
        };
        registration.credential.set(endpoint.credential);
        Ok(endpoint.address)
    }
}

impl Dialer for LocalDaemonDialer {
    fn dial(
        &self,
        addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>> {
        let addr = addr.clone();
        Box::pin(async move {
            let real = self.resolve(&addr).await?;
            self.inner.dial(&real).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::VecDeque;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use engram_dashboard_protocol::{AgentEvent, DaemonInfo, Secret};
    use engram_dashboard_transport::testing::{settle, ManualClock, MemoryEndpoint, MemoryNetwork};
    use engram_dashboard_transport::{
        Close, ConnectFailure, Frame, GaveUpReason, HandshakeStep, Inbox, Incoming, Peer,
        PeerState, Policy, Registry, TransportEvent,
    };

    use super::super::endpoint::LocalEndpointSource;
    use super::super::wire::ShellWire;
    use crate::discovery::{DataLayout, PidLiveness};

    const TOKEN_A: &str = "aaaa0000aaaa0000aaaa0000aaaa0000aaaa0000aaaa0000aaaa0000aaaa0000";
    const TOKEN_B: &str = "bbbb1111bbbb1111bbbb1111bbbb1111bbbb1111bbbb1111bbbb1111bbbb1111";

    /// Answers `current()` from a script; the last answer repeats. Counts `ensure` calls — dials must make none
    /// (ADR-0021).
    struct ScriptedSource {
        answers: Mutex<VecDeque<Current>>,
        reads: AtomicUsize,
        ensures: AtomicUsize,
    }

    impl ScriptedSource {
        fn new(answers: impl IntoIterator<Item = Current>) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers.into_iter().collect()),
                reads: AtomicUsize::new(0),
                ensures: AtomicUsize::new(0),
            })
        }

        fn reads(&self) -> usize {
            self.reads.load(Ordering::SeqCst)
        }

        fn ensures(&self) -> usize {
            self.ensures.load(Ordering::SeqCst)
        }
    }

    impl EndpointSource for ScriptedSource {
        fn ensure(&self, _timeout: Duration) -> Result<Endpoint, String> {
            self.ensures.fetch_add(1, Ordering::SeqCst);
            Err("dials never call ensure".into())
        }

        fn current(&self) -> Current {
            self.reads.fetch_add(1, Ordering::SeqCst);
            let mut answers = self.answers.lock().unwrap();
            if answers.len() > 1 {
                answers.pop_front().unwrap()
            } else {
                answers.front().cloned().expect("a scripted answer")
            }
        }
    }

    /// Answers one endpoint, but only after the test releases it — holds a dial inside `resolve`.
    struct GatedSource {
        answer: Current,
        entered: Mutex<mpsc::Sender<()>>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl EndpointSource for GatedSource {
        fn ensure(&self, _timeout: Duration) -> Result<Endpoint, String> {
            Err("dials never call ensure".into())
        }

        fn current(&self) -> Current {
            let _ = self.entered.lock().unwrap().send(());
            // Bounded, so a failing test does not leave a blocked pool thread behind.
            let _ = self
                .release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10));
            self.answer.clone()
        }
    }

    /// The memory network behind a recorder of the addresses it was asked to dial.
    struct Recording {
        net: MemoryNetwork,
        dialed: Mutex<Vec<Address>>,
    }

    impl Dialer for Recording {
        fn dial(
            &self,
            addr: &Address,
        ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>> {
            self.dialed.lock().unwrap().push(addr.clone());
            self.net.dial(addr)
        }
    }

    struct Harness {
        net: MemoryNetwork,
        recording: Arc<Recording>,
        clock: ManualClock,
        peer: Peer<ShellWire>,
        inbox: Inbox<ShellWire>,
        events: Vec<TransportEvent<ShellWire>>,
        messages: usize,
        // Keeps the supervisors' registry alive for the test.
        _registry: Registry<ShellWire>,
    }

    impl Harness {
        fn start(source: Arc<dyn EndpointSource>) -> Self {
            let net = MemoryNetwork::new();
            let recording = Arc::new(Recording {
                net: net.clone(),
                dialed: Mutex::new(Vec::new()),
            });
            let dialer = Arc::new(LocalDaemonDialer::new(recording.clone()));
            let key = DaemonKey::new("local");
            let (address, handshake, _) = dialer.register(&key, source, None);
            let clock = ManualClock::new();
            let registry = Registry::new(
                Arc::new(ShellWire),
                dialer,
                clock.handle(),
                Policy::default(),
            );
            let (peer, inbox) = registry.add(key.peer_id().clone(), address, handshake);
            Self {
                net,
                recording,
                clock,
                peer,
                inbox,
                events: Vec::new(),
                messages: 0,
                _registry: registry,
            }
        }

        fn dialed(&self) -> Vec<Address> {
            self.recording.dialed.lock().unwrap().clone()
        }

        fn state(&self) -> PeerState {
            self.peer.peer_state()
        }

        fn collect(&mut self) -> &[TransportEvent<ShellWire>] {
            while let Some(item) = self.inbox.try_recv() {
                match item {
                    Incoming::Event(ev) => self.events.push(ev),
                    Incoming::Message(_) => self.messages += 1,
                }
            }
            &self.events
        }

        fn connect_failures(&mut self) -> Vec<ConnectFailure> {
            self.collect()
                .iter()
                .filter_map(|e| match e {
                    TransportEvent::ConnectFailed { cause, .. } => Some(cause.clone()),
                    _ => None,
                })
                .collect()
        }

        fn saw(&mut self, kind: &str) -> bool {
            self.collect().iter().any(|e| e.kind() == kind)
        }

        /// The next link the supervisor opened, after it wrote its first frame (the Auth).
        async fn next_link(&self) -> (Arc<MemoryEndpoint>, String) {
            wait_for("a link is opened", || self.net.open_endpoints() > 0).await;
            let endpoint = self.net.accept().unwrap();
            let mut frames = Vec::new();
            wait_for("the Auth frame is written", || {
                frames.extend(endpoint.frames());
                !frames.is_empty()
            })
            .await;
            match frames.remove(0) {
                Frame::Text(auth) => (endpoint, auth),
                other => panic!("the first frame must be the Auth text, got {other:?}"),
            }
        }

        async fn go_live(&self, endpoint: &MemoryEndpoint) {
            endpoint.push(hello());
            wait_for("live", || self.state() == PeerState::Live).await;
        }

        /// The daemon closes a live link; the clock then passes the first backoff.
        async fn drop_live_link(&self, endpoint: &MemoryEndpoint) {
            endpoint.close();
            wait_for("backoff", || self.state() == PeerState::Backoff).await;
            settle().await;
            self.clock.advance(Duration::from_secs(1));
        }
    }

    /// Polls with real sleeps up to a bound, then panics — a regression fails instead of hanging. `settle_until` does
    /// not fit here: `current()` runs on the blocking pool, a real thread its yield budget does not wait for.
    async fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "not reached within 10 s: {what}");
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    fn live(port: u16, token: &str) -> Current {
        Current::Live(Endpoint {
            key: DaemonKey::new("local"),
            address: ws(port),
            credential: Secret::new(token),
        })
    }

    fn version_mismatch(daemon: u32) -> Current {
        Current::VersionMismatch {
            daemon,
            pid: 4242,
            start_time: 77,
        }
    }

    fn ws(port: u16) -> Address {
        Address::new(format!("ws://127.0.0.1:{port}"))
    }

    fn auth(token: &str) -> String {
        format!(r#"{{"Auth":{{"token":"{token}","protocol_version":{PROTOCOL_VERSION}}}}}"#)
    }

    fn text(ev: &AgentEvent) -> Frame {
        Frame::Text(serde_json::to_string(ev).unwrap())
    }

    fn hello() -> Frame {
        text(&AgentEvent::Hello {
            protocol_version: PROTOCOL_VERSION,
            daemon_version: "0.0.0".into(),
            capabilities: None,
        })
    }

    fn daemon_error() -> Frame {
        text(&AgentEvent::Error {
            request_id: None,
            message: "auth failed".into(),
        })
    }

    #[tokio::test]
    async fn auth_then_hello_brings_the_daemon_live_and_other_frames_before_hello_are_dropped() {
        let source = ScriptedSource::new([live(5123, TOKEN_A)]);
        let mut h = Harness::start(source.clone());
        let (endpoint, auth_text) = h.next_link().await;
        assert_eq!(auth_text, auth(TOKEN_A));
        assert_eq!(h.dialed(), vec![ws(5123)]);

        endpoint.push(text(&AgentEvent::AgentListUpdated { agents: vec![] }));
        endpoint.push(Frame::Binary(vec![1, 2, 3]));
        endpoint.push(Frame::Text("not json".into()));
        settle().await;
        assert_eq!(h.state(), PeerState::Handshaking);

        h.go_live(&endpoint).await;
        settle().await;
        assert!(h.saw("Connected"));
        assert_eq!(
            h.messages, 0,
            "the handshake consumed the frames before Hello"
        );
        assert_eq!(source.ensures(), 0);
    }

    /// The dial ends in a terminal refusal after one dial — an `Error` frame (the real daemon then closes without a
    /// code — ADR-0346) or a bare 4001 close.
    async fn assert_gave_up_rejected_after_one_dial(h: &mut Harness, source: &ScriptedSource) {
        wait_for("gave up", || h.state().is_terminal()).await;
        assert_eq!(h.state(), PeerState::GaveUp(GaveUpReason::Rejected));
        assert!(h.saw("Rejected"));
        assert!(
            h.connect_failures().is_empty(),
            "a terminal refusal spends no budget"
        );
        h.clock.advance(Duration::from_secs(60));
        settle().await;
        assert_eq!(h.net.dials(), 1, "a refusal is never re-dialed");
        assert_eq!(source.reads(), 1);
        assert_eq!(source.ensures(), 0);
    }

    #[tokio::test]
    async fn an_error_frame_gives_up_rejected_without_spending_budget() {
        let source = ScriptedSource::new([live(5123, TOKEN_A)]);
        let mut h = Harness::start(source.clone());
        let (endpoint, _) = h.next_link().await;

        endpoint.push(daemon_error());
        assert_gave_up_rejected_after_one_dial(&mut h, &source).await;
    }

    #[tokio::test]
    async fn a_bare_close_4001_gives_up_rejected_without_spending_budget() {
        let source = ScriptedSource::new([live(5123, TOKEN_A)]);
        let mut h = Harness::start(source.clone());
        let (endpoint, _) = h.next_link().await;

        endpoint.reject(Close::new(CloseCode::HANDSHAKE_REJECTED, "version"));
        assert_gave_up_rejected_after_one_dial(&mut h, &source).await;
    }

    #[tokio::test]
    async fn an_error_frame_before_a_4006_close_gives_up_instead_of_retrying() {
        let source = ScriptedSource::new([live(5123, TOKEN_A), live(5123, TOKEN_B)]);
        let mut h = Harness::start(source.clone());
        let (endpoint, _) = h.next_link().await;

        endpoint.push(daemon_error());
        endpoint.reject(Close::new(CloseCode::HANDSHAKE_RETRYABLE, "stale token"));
        assert_gave_up_rejected_after_one_dial(&mut h, &source).await;
    }

    #[tokio::test]
    async fn a_version_mismatch_gives_up_rejected_without_a_dial_or_budget() {
        let source = ScriptedSource::new([version_mismatch(PROTOCOL_VERSION + 1)]);
        let mut h = Harness::start(source.clone());
        wait_for("gave up", || h.state().is_terminal()).await;

        assert_eq!(h.state(), PeerState::GaveUp(GaveUpReason::Rejected));
        let rejected = h.collect().iter().find_map(|e| match e {
            TransportEvent::Rejected { code, .. } => Some(*code),
            _ => None,
        });
        assert_eq!(rejected, Some(CloseCode::HANDSHAKE_REJECTED));
        assert!(h.connect_failures().is_empty(), "no budget spent");
        h.clock.advance(Duration::from_secs(60));
        settle().await;
        assert_eq!(h.net.dials(), 0);
        assert_eq!(source.reads(), 1);
        assert_eq!(source.ensures(), 0);
    }

    #[tokio::test]
    async fn an_absent_or_unusable_record_fails_one_attempt_without_a_dial() {
        for answer in [
            Current::Absent,
            Current::Unusable("daemon.json names pid 7 which is not alive".into()),
        ] {
            let source = ScriptedSource::new([answer.clone()]);
            let mut h = Harness::start(source.clone());
            wait_for("backoff", || h.state() == PeerState::Backoff).await;
            assert!(
                matches!(
                    h.connect_failures().as_slice(),
                    [ConnectFailure::Unreachable(_)]
                ),
                "{answer:?}"
            );
            assert_eq!(h.net.dials(), 0, "{answer:?}");
            assert_eq!(source.reads(), 1, "{answer:?}");
            assert_eq!(source.ensures(), 0, "{answer:?}");
        }
    }

    #[tokio::test]
    async fn a_local_record_that_is_missing_empty_broken_or_dead_is_never_dialed() {
        struct Dead;
        impl PidLiveness for Dead {
            fn is_dead(&self, _pid: u32, _start_time: u64) -> bool {
                true
            }
        }
        let write = |dir: &Path, bytes: &[u8]| {
            let path = DataLayout::new(dir).daemon_file();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        let valid = DaemonInfo {
            pid: 4242,
            host: "127.0.0.1".into(),
            port: 5123,
            token: Secret::new(TOKEN_A),
            protocol_version: PROTOCOL_VERSION,
            start_time: 0,
        }
        .to_json_pretty()
        .unwrap();
        for case in ["missing", "empty", "broken", "dead pid"] {
            let dir = tempfile::tempdir().unwrap();
            match case {
                "missing" => {}
                "empty" => write(dir.path(), b""),
                "broken" => write(dir.path(), br#"{"pid":4242,"port":"#),
                _ => write(dir.path(), &valid),
            }
            let source = Arc::new(LocalEndpointSource::with_liveness(
                DaemonKey::new("local"),
                dir.path().to_path_buf(),
                Dead,
            ));
            let mut h = Harness::start(source);
            wait_for("backoff", || h.state() == PeerState::Backoff).await;
            assert!(
                matches!(
                    h.connect_failures().as_slice(),
                    [ConnectFailure::Unreachable(_)]
                ),
                "{case}"
            );
            assert_eq!(h.net.dials(), 0, "{case}");
        }
    }

    #[tokio::test]
    async fn a_busy_record_dials_the_last_known_endpoint_and_no_other_failure_does() {
        let source = ScriptedSource::new([
            live(5123, TOKEN_A),
            Current::Busy,
            Current::Unusable("daemon.json read failed: access denied".into()),
        ]);
        let mut h = Harness::start(source.clone());
        let (first, _) = h.next_link().await;
        h.go_live(&first).await;

        h.drop_live_link(&first).await;
        let (second, second_auth) = h.next_link().await;
        assert_eq!(h.dialed(), vec![ws(5123), ws(5123)]);
        assert_eq!(second_auth, auth(TOKEN_A), "the last-known credential");
        h.go_live(&second).await;

        h.drop_live_link(&second).await;
        wait_for("the third read failed the attempt", || {
            source.reads() == 3 && h.state() == PeerState::Backoff
        })
        .await;
        assert_eq!(h.net.dials(), 2, "only a sharing violation falls back");
        assert!(matches!(
            h.connect_failures().as_slice(),
            [ConnectFailure::Unreachable(_)]
        ));
        assert_eq!(source.ensures(), 0);
    }

    #[tokio::test]
    async fn a_busy_record_with_no_last_known_endpoint_is_not_dialed() {
        let source = ScriptedSource::new([Current::Busy]);
        let mut h = Harness::start(source.clone());
        wait_for("backoff", || h.state() == PeerState::Backoff).await;
        assert_eq!(h.net.dials(), 0);
        assert_eq!(h.connect_failures().len(), 1);
        assert_eq!(source.ensures(), 0);
    }

    #[tokio::test]
    async fn a_reconnect_follows_the_port_and_credential_the_daemon_moved_to() {
        let source = ScriptedSource::new([live(5123, TOKEN_A), live(6234, TOKEN_B)]);
        let h = Harness::start(source.clone());
        let (first, _) = h.next_link().await;
        h.go_live(&first).await;

        h.drop_live_link(&first).await;
        let (second, second_auth) = h.next_link().await;
        assert_eq!(h.dialed(), vec![ws(5123), ws(6234)]);
        assert_eq!(second_auth, auth(TOKEN_B));
        h.go_live(&second).await;
        assert_eq!(source.ensures(), 0);
    }

    fn sent_auth(handshake: &Arc<dyn Handshake>) -> String {
        match handshake.start() {
            HandshakeStep::Send(Frame::Text(sent)) => sent,
            other => panic!("expected the Auth text frame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_busy_record_after_the_daemon_was_found_gone_is_not_dialed() {
        for gone in [
            Current::Absent,
            Current::Unusable("daemon.json names pid 7 which is not alive".into()),
            version_mismatch(PROTOCOL_VERSION + 1),
        ] {
            let net = MemoryNetwork::new();
            let dialer = LocalDaemonDialer::new(net.dialer());
            let source = ScriptedSource::new([live(5123, TOKEN_A), gone.clone(), Current::Busy]);
            let (address, _, _) = dialer.register(&DaemonKey::new("local"), source.clone(), None);
            assert!(dialer.dial(&address).await.is_ok(), "{gone:?}");
            assert!(dialer.dial(&address).await.is_err(), "{gone:?}");
            let busy = dialer.dial(&address).await.err();
            assert!(
                matches!(busy, Some(DialFailure::Unreachable(_))),
                "{gone:?}: the last-known endpoint was forgotten"
            );
            assert_eq!(net.dials(), 1, "{gone:?}");
            assert_eq!(source.reads(), 3, "{gone:?}");
            assert_eq!(source.ensures(), 0, "{gone:?}");
        }
    }

    #[tokio::test]
    async fn an_old_registrations_late_dial_cannot_change_what_the_new_handshake_sends() {
        let net = MemoryNetwork::new();
        let recording = Arc::new(Recording {
            net: net.clone(),
            dialed: Mutex::new(Vec::new()),
        });
        let dialer = Arc::new(LocalDaemonDialer::new(recording.clone()));
        let key = DaemonKey::new("local");
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let old_source = Arc::new(GatedSource {
            answer: live(5123, TOKEN_A),
            entered: Mutex::new(entered_tx),
            release: Mutex::new(release_rx),
        });
        let (old_address, old_handshake, _) = dialer.register(&key, old_source, None);
        let (new_address, new_handshake, _) =
            dialer.register(&key, ScriptedSource::new([live(6234, TOKEN_B)]), None);
        assert_ne!(old_address, new_address);

        // The old supervisor's dial is reading its source when the re-add's supervisor dials.
        let old_dial = {
            let dialer = dialer.clone();
            let address = old_address.clone();
            tokio::spawn(async move { dialer.dial(&address).await.is_ok() })
        };
        wait_for("the old dial reads its source", || {
            entered_rx.try_recv().is_ok()
        })
        .await;
        assert!(dialer.dial(&new_address).await.is_ok());
        assert_eq!(sent_auth(&new_handshake), auth(TOKEN_B));

        // It finishes late — a closed supervisor can still complete a connecting step (`peer.rs` unbiased select).
        release_tx.send(()).unwrap();
        wait_for("the old dial finishes", || old_dial.is_finished()).await;
        assert!(old_dial.await.unwrap());
        assert_eq!(
            sent_auth(&new_handshake),
            auth(TOKEN_B),
            "the new handshake sends only the token its own dial resolved"
        );
        assert_eq!(sent_auth(&old_handshake), auth(TOKEN_A));
        assert_eq!(*recording.dialed.lock().unwrap(), vec![ws(6234), ws(5123)]);
    }

    #[tokio::test]
    async fn an_unknown_or_unregistered_address_is_unreachable_without_a_dial() {
        let net = MemoryNetwork::new();
        let dialer = LocalDaemonDialer::new(net.dialer());
        let source = ScriptedSource::new([live(5123, TOKEN_A)]);
        let (address, _, _) = dialer.register(&DaemonKey::new("local"), source.clone(), None);
        dialer.unregister(&address);
        for target in [address, Address::new("engram-local:nobody#0")] {
            let failure = dialer.dial(&target).await.err();
            assert!(
                matches!(failure, Some(DialFailure::Unreachable(_))),
                "{target}"
            );
        }
        assert_eq!(net.dials(), 0);
        assert_eq!(source.reads(), 0);
    }

    // TRD S21 trd-3-2-u5 §4-3 (D4): the slot is how routing tells a version refusal from the daemon's own refusal.
    #[tokio::test]
    async fn a_version_refusal_is_recorded_in_the_slot_before_the_dial_fails_and_any_other_answer_clears_it(
    ) {
        let net = MemoryNetwork::new();
        let dialer = LocalDaemonDialer::new(net.dialer());
        let source = ScriptedSource::new([
            version_mismatch(PROTOCOL_VERSION - 1),
            live(5123, TOKEN_A),
            version_mismatch(PROTOCOL_VERSION - 1),
            Current::Absent,
        ]);
        let (address, _, slot) = dialer.register(&DaemonKey::new("local"), source.clone(), None);
        assert_eq!(slot.get(), None, "nothing dialed yet");

        let refused = dialer.dial(&address).await.err();
        assert!(matches!(
            refused,
            Some(DialFailure::Refused {
                code: CloseCode::HANDSHAKE_REJECTED,
                ..
            })
        ));
        let recorded = Some(VersionRefusal {
            daemon: PROTOCOL_VERSION - 1,
            pid: 4242,
            start_time: 77,
        });
        assert_eq!(slot.get(), recorded);

        assert!(dialer.dial(&address).await.is_ok());
        assert_eq!(slot.get(), None, "a live answer clears it");

        assert!(dialer.dial(&address).await.is_err());
        assert_eq!(slot.get(), recorded);
        assert!(matches!(
            dialer.dial(&address).await.err(),
            Some(DialFailure::Unreachable(_))
        ));
        assert_eq!(slot.get(), None, "an absent record clears it");

        // Each registration has its own slot.
        let (_, _, other) = dialer.register(&DaemonKey::new("local"), source, None);
        assert_eq!(other.get(), None);
    }

    #[test]
    fn only_a_missing_or_unusable_record_reads_as_no_live_daemon() {
        assert_eq!(
            no_live_daemon_cause(NO_RECORD).as_deref(),
            Some("no endpoint record")
        );
        assert_eq!(
            no_live_daemon_cause(&format!(
                "{UNUSABLE}: daemon.json names pid 7 which is not alive"
            ))
            .as_deref(),
            Some("daemon.json names pid 7 which is not alive")
        );
        for other in [
            BUSY_UNKNOWN.to_owned(),
            format!("{READ_FAILED}: task gone"),
            format!("{NOT_REGISTERED} engram-local:local#1"),
            "ws connect ws://127.0.0.1:5123: refused".to_owned(),
            "connect timed out".to_owned(),
        ] {
            assert_eq!(no_live_daemon_cause(&other), None, "{other}");
        }
    }
}
