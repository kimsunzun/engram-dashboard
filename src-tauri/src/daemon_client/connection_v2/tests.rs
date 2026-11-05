//! The driver on a memory network and a manual clock, through `DaemonClient::new_on_link` — the test side plays the
//! daemon: it answers the Auth frame with `Hello`, pushes broadcasts and replies, and drops the link.
//!
//! Waits poll with real sleeps up to a bound: the dialer reads the endpoint record on the blocking pool, a thread the
//! transport's `settle` does not wait for.

use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_protocol::{AgentId, AgentStatus, RequestId, Secret, PROTOCOL_VERSION};
use engram_dashboard_transport::testing::{ManualClock, MemoryEndpoint, MemoryNetwork};
use engram_dashboard_transport::{Close, CloseCode, Frame, GaveUpReason, PeerState};
use tokio::task::JoinHandle;

use crate::daemon_client::link::endpoint::{Current, Endpoint};
use crate::daemon_client::tests::{wire_agent, RecordingEvents};
use crate::daemon_client::DaemonClient;

const TOKEN: &str = "cccc2222cccc2222cccc2222cccc2222cccc2222cccc2222cccc2222cccc2222";

/// The daemon's endpoint record as the test sets it. `ensure` (the search that may start a daemon) answers the record
/// unless the test made it fail, and counts its calls.
struct Record {
    now: Mutex<Current>,
    search_fails: Mutex<Option<String>>,
    /// Set: the search reports that it started and waits for the release.
    search_gate: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
    reads: AtomicUsize,
    searches: AtomicUsize,
}

impl Record {
    fn new(now: Current) -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(now),
            search_fails: Mutex::new(None),
            search_gate: Mutex::new(None),
            reads: AtomicUsize::new(0),
            searches: AtomicUsize::new(0),
        })
    }

    fn set(&self, now: Current) {
        *self.now.lock().unwrap() = now;
    }

    fn fail_search(&self, why: &str) {
        *self.search_fails.lock().unwrap() = Some(why.to_owned());
    }

    /// Holds the next search: returns "it started" and the release.
    fn hold_search(&self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        *self.search_gate.lock().unwrap() = Some((entered_tx, release_rx));
        (entered_rx, release_tx)
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }

    fn searches(&self) -> usize {
        self.searches.load(Ordering::SeqCst)
    }
}

impl EndpointSource for Record {
    fn ensure(&self, _timeout: Duration) -> Result<Endpoint, String> {
        self.searches.fetch_add(1, Ordering::SeqCst);
        if let Some((entered, release)) = self.search_gate.lock().unwrap().take() {
            let _ = entered.send(());
            // Bounded, so a failing test does not leave a blocked pool thread behind.
            let _ = release.recv_timeout(Duration::from_secs(10));
        }
        if let Some(why) = self.search_fails.lock().unwrap().clone() {
            return Err(why);
        }
        match self.now.lock().unwrap().clone() {
            Current::Live(endpoint) => Ok(endpoint),
            other => Err(format!("no live daemon: {other:?}")),
        }
    }

    fn current(&self) -> Current {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.now.lock().unwrap().clone()
    }
}

fn live() -> Current {
    Current::Live(Endpoint {
        key: DaemonKey::new(DAEMON),
        address: Address::new("ws://127.0.0.1:5123"),
        credential: Secret::new(TOKEN),
    })
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

/// Polls with real sleeps up to a bound, then panics — a regression fails instead of hanging.
async fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "not reached within 10 s: {what}");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

/// Awaits a task only once it finished — never a bare await on a task that may wait on the manual clock.
async fn joined<T>(what: &str, task: JoinHandle<T>) -> T {
    wait_for(what, || task.is_finished()).await;
    task.await.expect("the task ran to its end")
}

struct Harness {
    net: MemoryNetwork,
    clock: ManualClock,
    record: Arc<Record>,
    events: Arc<RecordingEvents>,
    client: Arc<DaemonClient>,
}

impl Harness {
    fn new(now: Current) -> Self {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let record = Record::new(now);
        let events = Arc::new(RecordingEvents::default());
        let client = Arc::new(DaemonClient::new_on_link(
            Handle::current(),
            net.dialer(),
            record.clone(),
            clock.handle(),
            events.clone(),
            events.clone(),
        ));
        Self {
            net,
            clock,
            record,
            events,
            client,
        }
    }

    fn link(&self) -> &LinkClient {
        self.client.link.as_ref().expect("built on the link driver")
    }

    fn peer_state(&self) -> Option<PeerState> {
        self.link()
            .registry
            .get(self.link().key.peer_id())
            .map(|peer| peer.peer_state())
    }

    fn connect(&self) -> JoinHandle<Result<(), HandshakeError>> {
        let client = self.client.clone();
        tokio::spawn(async move { client.connect().await })
    }

    fn ensure(&self) -> JoinHandle<Result<(), HandshakeError>> {
        let client = self.client.clone();
        tokio::spawn(async move { client.ensure().await })
    }

    /// The connection states the frontend got, in order.
    fn states(&self) -> Vec<String> {
        self.events
            .seen()
            .into_iter()
            .filter(|line| line.starts_with("state:"))
            .collect()
    }

    /// The daemon side of the next link and the Auth text it got.
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

    /// Connects and lets the daemon accept.
    async fn connected(&self) -> Arc<MemoryEndpoint> {
        let task = self.connect();
        let (endpoint, _) = self.next_link().await;
        endpoint.push(hello());
        joined("connect", task).await.expect("connected");
        endpoint
    }

    /// Moves the clock past one backoff at a time until `done` — only while the peer waits in backoff, so no dial or
    /// handshake deadline fires.
    async fn advance_until(&self, what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "not reached within 10 s: {what}");
            if self.peer_state() == Some(PeerState::Backoff) {
                self.clock.advance(Duration::from_secs(11));
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    /// The next frame the shell wrote on `endpoint`.
    async fn written(&self, endpoint: &MemoryEndpoint) -> String {
        let mut frames = Vec::new();
        wait_for("a frame is written", || {
            frames.extend(endpoint.frames());
            !frames.is_empty()
        })
        .await;
        match frames.remove(0) {
            Frame::Text(sent) => sent,
            other => panic!("expected a text frame, got {other:?}"),
        }
    }
}

#[test]
fn the_link_policy_retries_network_failures_without_limit_and_keeps_the_other_defaults() {
    let policy = link_policy();
    assert_eq!(policy.reconnect.max_attempts, u32::MAX);
    let mut expected = Policy::default();
    expected.reconnect.max_attempts = u32::MAX;
    assert_eq!(policy, expected);
}

#[tokio::test]
async fn a_first_connect_resolves_on_hello_and_broadcasts_reach_the_frontend_before_the_names() {
    let h = Harness::new(live());
    let task = h.connect();
    let (endpoint, auth) = h.next_link().await;
    assert!(auth.contains(TOKEN), "the record's credential");
    endpoint.push(hello());
    joined("connect", task).await.expect("connected");
    assert_eq!(h.client.state(), ConnectionState::Connected);
    assert_eq!(h.record.searches(), 1, "connect may start a daemon");

    let agent = AgentId::new_v4();
    endpoint.push(text(&AgentEvent::AgentListUpdated {
        agents: vec![wire_agent(agent, "alpha")],
    }));
    endpoint.push(text(&AgentEvent::StatusChanged {
        agent_id: agent,
        status: AgentStatus::Killed,
        epoch: 1,
    }));
    wait_for("the broadcasts", || h.events.seen().len() >= 4).await;
    assert_eq!(
        h.events.seen(),
        [
            "state:connected".to_owned(),
            "agent_list:1".to_owned(),
            format!("names:{agent}=alpha"),
            format!("status:{agent}:Killed:1"),
        ]
    );
}

#[tokio::test]
async fn a_cold_start_failure_shows_down_at_once_and_retries_without_limit_until_a_daemon_is_there()
{
    let h = Harness::new(Current::Absent);
    h.record.fail_search("the daemon exe is missing");
    let err = joined("connect", h.connect())
        .await
        .expect_err("the search failed");
    assert!(matches!(err, HandshakeError::Discovery(_)), "{err:?}");
    wait_for("Down", || h.states() == ["state:down"]).await;

    // Far past the transport's default budget (3) — the handle keeps reading the record.
    h.advance_until("ten failed attempts", || h.record.reads() >= 10)
        .await;
    assert!(
        !matches!(h.peer_state(), Some(PeerState::GaveUp(_)) | None),
        "the link never gives up on a missing daemon: {:?}",
        h.peer_state()
    );
    assert_eq!(
        h.states(),
        ["state:down"],
        "no Reconnecting, no second Down"
    );

    h.record.set(live());
    h.advance_until("a dial to the new daemon", || h.net.open_endpoints() > 0)
        .await;
    let (endpoint, _) = h.next_link().await;
    endpoint.push(hello());
    wait_for("connected", || {
        h.client.state() == ConnectionState::Connected
    })
    .await;
    assert_eq!(h.states(), ["state:down", "state:connected"]);
    assert_eq!(h.record.searches(), 1, "retries never search (ADR-0021)");
}

#[tokio::test]
async fn ensure_never_searches_and_a_missing_daemon_shows_down() {
    let h = Harness::new(Current::Absent);
    let err = joined("ensure", h.ensure()).await.expect_err("no daemon");
    assert!(matches!(err, HandshakeError::Connect(_)), "{err:?}");
    wait_for("Down", || h.states() == ["state:down"]).await;
    assert_eq!(h.record.searches(), 0);
}

#[tokio::test]
async fn a_daemon_error_in_the_handshake_stops_for_good_without_a_second_dial() {
    let h = Harness::new(live());
    let task = h.ensure();
    let (endpoint, _) = h.next_link().await;
    // The real daemon's refusal: an `Error` frame and a close without a code (net `ws.rs`).
    endpoint.push(text(&AgentEvent::Error {
        request_id: None,
        message: "auth failed".into(),
    }));
    endpoint.close();
    let err = joined("ensure", task).await.expect_err("refused");
    assert!(
        matches!(&err, HandshakeError::AuthRejected(text) if !text.is_empty() && !text.contains("auth failed")),
        "a fixed local text, nothing the daemon sent: {err:?}"
    );
    wait_for("gave up", || {
        h.peer_state() == Some(PeerState::GaveUp(GaveUpReason::Rejected))
    })
    .await;
    assert_eq!(h.states(), ["state:down"]);

    for _ in 0..10 {
        h.clock.advance(Duration::from_secs(60));
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(h.net.dials(), 1, "a refusal is never retried (ADR-0346)");
    assert_eq!(h.record.reads(), 1);
}

#[tokio::test]
async fn command_round_trips_and_their_failures_read_as_the_old_driver_texts() {
    let h = Harness::new(live());
    let list = || AgentCommand::ListAgents {
        request_id: RequestId::new(),
    };
    assert_eq!(
        h.client
            .send_command(list())
            .await
            .expect_err("not connected"),
        NOT_CONNECTED
    );
    assert_eq!(
        h.client
            .send_command(AgentCommand::Unsubscribe {
                agent_id: AgentId::new_v4()
            })
            .await
            .expect_err("no request id"),
        NO_REQUEST_ID
    );

    let endpoint = h.connected().await;

    let rid = RequestId::new();
    let task = {
        let client = h.client.clone();
        tokio::spawn(async move {
            client
                .send_command(AgentCommand::ListAgents { request_id: rid })
                .await
        })
    };
    assert!(h.written(&endpoint).await.contains(&rid.0.to_string()));
    endpoint.push(text(&AgentEvent::Ack { request_id: rid }));
    assert!(matches!(
        joined("the reply", task).await,
        Ok(AgentEvent::Ack { request_id }) if request_id == rid
    ));

    // A daemon `Error` reply is the daemon's failure, not a local one (ADR-0281).
    let rid = RequestId::new();
    let task = {
        let client = h.client.clone();
        tokio::spawn(async move {
            client
                .send_command_with_origin(AgentCommand::ListAgents { request_id: rid })
                .await
        })
    };
    h.written(&endpoint).await;
    endpoint.push(text(&AgentEvent::Error {
        request_id: Some(rid),
        message: "no such agent".into(),
    }));
    assert!(matches!(
        joined("the error reply", task).await,
        Err(CommandFailure::Daemon { message }) if message == "no such agent"
    ));

    // Written, then the link drops before the reply.
    let task = {
        let client = h.client.clone();
        tokio::spawn(async move { client.send_command(list()).await })
    };
    h.written(&endpoint).await;
    endpoint.close();
    assert_eq!(
        joined("the dropped request", task)
            .await
            .expect_err("the link dropped"),
        crate::daemon_client::connection::SENT_OUTCOME_UNKNOWN
    );
}

#[tokio::test]
async fn close_shows_down_only_through_routing_and_the_ended_handle_gives_its_dial_registration_back(
) {
    let h = Harness::new(live());
    let _endpoint = h.connected().await;
    assert_eq!(h.link().dialer.registered(), 1);

    h.client.close();
    wait_for("Down", || h.client.state() == ConnectionState::Down).await;
    wait_for("the registration is given back", || {
        h.link().dialer.registered() == 0
    })
    .await;
    let states = h.states();
    assert_eq!(
        states.iter().filter(|s| *s == "state:down").count(),
        1,
        "{states:?}"
    );
    assert_eq!(states.last().map(String::as_str), Some("state:down"));

    for _ in 0..5 {
        h.clock.advance(Duration::from_secs(60));
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(h.net.dials(), 1, "a closed link does not reconnect");
}

#[tokio::test]
async fn a_refusal_the_transport_would_retry_stops_the_link_after_one_dial() {
    let h = Harness::new(live());
    let task = h.ensure();
    let (endpoint, _) = h.next_link().await;
    // A bare retryable close code — `RejectKind::of` reads every code but 4001 as retryable.
    endpoint.reject(Close::new(CloseCode::HANDSHAKE_RETRYABLE, "stale token"));
    let err = joined("ensure", task).await.expect_err("refused");
    assert!(
        matches!(&err, HandshakeError::AuthRejected(text) if !text.contains("stale token")),
        "{err:?}"
    );
    wait_for("the peer is removed", || {
        h.peer_state().is_none() && h.link().dialer.registered() == 0
    })
    .await;
    for _ in 0..10 {
        h.clock.advance(Duration::from_secs(60));
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(h.net.dials(), 1, "no refusal is retried (ADR-0346)");
    assert_eq!(h.states(), ["state:down"]);
}

#[tokio::test]
async fn a_connect_right_after_a_close_connects_again() {
    let h = Harness::new(live());
    let _endpoint = h.connected().await;

    h.client.close();
    let task = h.connect();
    let (endpoint, _) = h.next_link().await;
    endpoint.push(hello());
    joined("connect", task).await.expect("connected again");
    wait_for("connected", || {
        h.client.state() == ConnectionState::Connected
    })
    .await;
    assert_eq!(h.net.dials(), 2);
}

#[tokio::test]
async fn a_close_during_the_daemon_search_wins_over_the_connect() {
    let h = Harness::new(live());
    let (entered, release) = h.record.hold_search();
    let task = h.connect();
    wait_for("the search started", || entered.try_recv().is_ok()).await;

    h.client.close();
    release.send(()).unwrap();
    let err = joined("connect", task)
        .await
        .expect_err("overtaken by the close");
    assert_eq!(err, HandshakeError::TaskGone);
    for _ in 0..10 {
        h.clock.advance(Duration::from_secs(60));
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(h.record.reads(), 0, "no handle was built");
    assert_eq!(h.link().dialer.registered(), 0);
    assert_eq!(h.peer_state(), None);
}

#[tokio::test]
async fn a_close_before_hello_answers_the_waiting_ensure() {
    let h = Harness::new(live());
    let task = h.ensure();
    // Held: a dropped endpoint is a failed link, not a close.
    let (_endpoint, _) = h.next_link().await;
    h.client.close();
    assert_eq!(
        joined("ensure", task).await,
        Err(HandshakeError::ClosedBeforeHello)
    );
}

#[tokio::test]
async fn a_fire_and_forget_goes_out_only_on_a_live_link() {
    let h = Harness::new(live());
    let (early, late) = (AgentId::new_v4(), AgentId::new_v4());
    let task = h.connect();
    let (endpoint, _) = h.next_link().await;
    assert!(
        matches!(h.peer_state(), Some(state) if !state.is_live()),
        "a peer that is not live yet: {:?}",
        h.peer_state()
    );
    h.client
        .send_fire_and_forget(AgentCommand::Unsubscribe { agent_id: early });
    endpoint.push(hello());
    joined("connect", task).await.expect("connected");
    h.client
        .send_fire_and_forget(AgentCommand::Unsubscribe { agent_id: late });
    let sent = h.written(&endpoint).await;
    assert!(sent.contains(&late.to_string()), "{sent}");
    assert!(!sent.contains(&early.to_string()), "{sent}");
}

#[tokio::test]
async fn dropping_the_client_closes_the_link_and_gives_its_one_registration_back() {
    let h = Harness::new(live());
    let _endpoint = h.connected().await;
    let dialer = h.link().dialer.clone();
    let registry = h.link().registry.clone();
    let key = h.link().key.clone();
    assert_eq!(dialer.registered(), 1);

    let Harness { client, .. } = h;
    assert_eq!(Arc::strong_count(&client), 1);
    drop(client);
    wait_for("the actor is gone", || {
        dialer.registered() == 0 && registry.get(key.peer_id()).is_none()
    })
    .await;
}

/// An actor the test drives by hand — never spawned; its pumps' items wait unread in `_routable_rx`.
struct Bench {
    driver: Driver,
    _client: LinkClient,
    _cmd_rx: tokio::sync::mpsc::UnboundedReceiver<DriverCmd>,
    _routable_rx: tokio::sync::mpsc::Receiver<Routable>,
    _net: MemoryNetwork,
}

impl Bench {
    fn new() -> Self {
        let net = MemoryNetwork::new();
        let events = Arc::new(RecordingEvents::default());
        let (client, driver, cmd_rx, routable_rx) = LinkClient::build(
            &Handle::current(),
            net.dialer(),
            Record::new(live()),
            ManualClock::new().handle(),
            events.clone(),
            events,
        );
        Self {
            driver,
            _client: client,
            _cmd_rx: cmd_rx,
            _routable_rx: routable_rx,
            _net: net,
        }
    }

    /// Adds a handle and returns its token.
    fn add(&mut self) -> HandleToken {
        let (first, _answer) = oneshot::channel();
        self.driver.add(0, first);
        self.driver.current.expect("a handle was added")
    }

    fn key(&self) -> DaemonKey {
        self.driver.key.clone()
    }
}

fn retryable_refusal(key: &DaemonKey) -> Incoming<ShellWire> {
    Incoming::Event(TransportEvent::ConnectFailed {
        peer: key.peer_id().clone(),
        attempt: 1,
        of: u32::MAX,
        cause: ConnectFailure::Rejected {
            kind: RejectKind::Retryable,
            code: CloseCode::HANDSHAKE_RETRYABLE,
            reason: "stale token".into(),
        },
        retry_in: Some(Duration::from_millis(500)),
    })
}

#[tokio::test]
async fn a_retryable_refusal_on_a_first_connect_gives_its_notice_before_down() {
    let mut b = Bench::new();
    let token = b.add();
    let key = b.key();
    let out = b.driver.route_item(&key, token, retryable_refusal(&key));
    assert!(
        matches!(
            out.as_slice(),
            [
                Routed::Notice(LinkNotice::Refused {
                    code: CloseCode::HANDSHAKE_RETRYABLE
                }),
                Routed::State(ConnectionState::Down),
            ]
        ),
        "the order of a terminal refusal: {out:?}"
    );
    assert!(
        b.driver.registry.get(key.peer_id()).is_none(),
        "the peer stopped"
    );
}

#[tokio::test]
async fn a_late_refusal_of_a_handle_the_user_closed_gives_no_notice() {
    let mut b = Bench::new();
    let token = b.add();
    let key = b.key();
    b.driver.close();
    let out = b.driver.route_item(&key, token, retryable_refusal(&key));
    assert!(
        !out.iter().any(|r| matches!(r, Routed::Notice(_))),
        "{out:?}"
    );
}

#[tokio::test]
async fn a_handle_routed_as_connected_counts_as_live_before_its_peer_says_so() {
    let mut b = Bench::new();
    let token = b.add();
    let key = b.key();
    let connected = Incoming::Event(TransportEvent::Connected {
        peer: key.peer_id().clone(),
        generation: engram_dashboard_transport::Generation(1),
    });
    let _ = b.driver.route_item(&key, token, connected);
    let peer_state = b.driver.registry.get(key.peer_id()).map(|p| p.peer_state());
    assert!(
        matches!(peer_state, Some(state) if !state.is_live()),
        "the supervisor has not published Live: {peer_state:?}"
    );
    assert!(b.driver.is_live());

    let (first, mut answer) = oneshot::channel();
    b.driver.add(0, first);
    assert_eq!(answer.try_recv(), Ok(Ok(())));
    assert_eq!(b.driver.current, Some(token), "no re-add");
    assert_eq!(b.driver.dialer.registered(), 1);
}
