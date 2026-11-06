//! The driver on a memory network and a manual clock, through `DaemonClient::new_on_link` — the test side plays the
//! daemon: it answers the Auth frame with `Hello`, pushes broadcasts and replies, and drops the link.
//!
//! Waits poll with real sleeps up to a bound: the dialer reads the endpoint record on the blocking pool, a thread the
//! transport's `settle` does not wait for.

use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_protocol::{
    encode_terminal_frame, AgentStatus, RequestId, Secret, SubscribeAction, PROTOCOL_VERSION,
};
use engram_dashboard_transport::testing::{ManualClock, MemoryEndpoint, MemoryNetwork};
use engram_dashboard_transport::{
    Arrival, Close, CloseCode, DisconnectCause, Frame, GaveUpReason, PeerState, QueueFullPolicy,
};
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::task::JoinHandle;

use crate::commands::layout::RouterSubs;
use crate::commands::popout::close_requested_popup_window;
use crate::daemon_client::link::endpoint::{Current, Endpoint};
use crate::daemon_client::replay_flight::MARKER_TAG;
use crate::daemon_client::tests::{
    active_slot, wire_agent, NoLayoutEvents, RecordingEvents, POPUP,
};
use crate::daemon_client::DaemonClient;
use crate::layout::{apply, LayoutState, MAIN_WINDOW_LABEL};
use crate::output_channel::register_window;

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

    /// The key's current handle, as the actor sees it.
    async fn current_handle(&self) -> HandleToken {
        let (reply, answer) = oneshot::channel();
        self.link()
            .cmd_tx
            .send(DriverCmd::CurrentHandle { reply })
            .expect("the actor runs");
        answer.await.expect("the actor answers").expect("a handle")
    }

    /// The next `n` commands the shell wrote on `endpoint`, and any that came with them.
    async fn commands(&self, endpoint: &MemoryEndpoint, n: usize) -> Vec<AgentCommand> {
        let mut frames = Vec::new();
        wait_for("commands are written", || {
            frames.extend(endpoint.frames());
            frames.len() >= n
        })
        .await;
        frames
            .into_iter()
            .map(|frame| match frame {
                Frame::Text(sent) => serde_json::from_str(&sent)
                    .unwrap_or_else(|e| panic!("not a command ({e}): {sent}")),
                other => panic!("expected a text frame, got {other:?}"),
            })
            .collect()
    }

    /// The next command the shell wrote on `endpoint` — the only one since the last read.
    async fn command(&self, endpoint: &MemoryEndpoint) -> AgentCommand {
        let mut sent = self.commands(endpoint, 1).await;
        assert_eq!(sent.len(), 1, "{sent:?}");
        sent.remove(0)
    }

    /// The next text frame the shell wrote on `endpoint` — the only frame since the last read (frames read together
    /// would otherwise be lost).
    async fn written(&self, endpoint: &MemoryEndpoint) -> String {
        let mut frames = Vec::new();
        wait_for("a frame is written", || {
            frames.extend(endpoint.frames());
            !frames.is_empty()
        })
        .await;
        assert_eq!(frames.len(), 1, "{frames:?}");
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
    // `drop_connection` relies on it: a drop that meets a full queue still drops the link.
    assert_eq!(policy.full_queue, QueueFullPolicy::Disconnect);
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
            Ports {
                events: events.clone(),
                names: events,
                router: Arc::new(OutputRouter::new()),
                windows: WindowChannelRegistry::default(),
            },
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
async fn a_handle_routed_as_connected_counts_as_live_until_its_loss_is_routed() {
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
        "the routed branch alone — the bench's peer is not live: {peer_state:?}"
    );
    assert!(b.driver.is_live());

    let (first, mut answer) = oneshot::channel();
    b.driver.add(0, first);
    assert_eq!(answer.try_recv(), Ok(Ok(())));
    assert_eq!(b.driver.current, Some(token), "no re-add");
    assert_eq!(b.driver.dialer.registered(), 1);
}

#[tokio::test]
async fn a_close_answers_the_waiting_connect_at_once_and_a_later_add_waits_for_its_own_handle() {
    let mut b = Bench::new();
    let (first, mut answer_a) = oneshot::channel();
    b.driver.add(0, first);
    let old = b.driver.current.expect("a handle");
    b.driver.close();
    assert_eq!(
        answer_a.try_recv(),
        Ok(Err(HandshakeError::ClosedBeforeHello)),
        "answered by the close itself, before the old Closed is routed"
    );

    let (second, mut answer_b) = oneshot::channel();
    b.driver.add(0, second);
    assert_ne!(b.driver.current, Some(old));
    assert_eq!(
        answer_b.try_recv(),
        Err(oneshot::error::TryRecvError::Empty),
        "still waiting for the new handle"
    );
}

/// A user close, then a connect whose new handle cannot reach the daemon — the old handle's `Disconnected` · `Closed`
/// routed before the re-add or after it (as the actor's `select!` happens to order them).
async fn close_then_failed_reconnect(old_tail_first: bool) -> ConnectionState {
    let mut b = Bench::new();
    let old = b.add();
    let key = b.key();
    let peer = key.peer_id().clone();
    let g = engram_dashboard_transport::Generation(1);
    let connected = Incoming::Event(TransportEvent::Connected {
        peer: peer.clone(),
        generation: g,
    });
    let out = b.driver.route_item(&key, old, connected);
    b.driver.apply(out);
    b.driver.close();
    let tail = || {
        [
            Incoming::Event(TransportEvent::Disconnected {
                peer: peer.clone(),
                generation: g,
                cause: DisconnectCause::Explicit,
            }),
            Incoming::Event(TransportEvent::Closed {
                peer: peer.clone(),
                generation: g,
                cause: DisconnectCause::Explicit,
            }),
        ]
    };
    if old_tail_first {
        for item in tail() {
            let out = b.driver.route_item(&key, old, item);
            b.driver.apply(out);
        }
    }
    let new = b.add();
    if !old_tail_first {
        for item in tail() {
            let out = b.driver.route_item(&key, old, item);
            b.driver.apply(out);
        }
    }
    let failed = Incoming::Event(TransportEvent::ConnectFailed {
        peer: peer.clone(),
        attempt: 1,
        of: u32::MAX,
        cause: ConnectFailure::Unreachable(engram_dashboard_transport::LinkError::new(
            "connection refused",
        )),
        retry_in: Some(Duration::from_millis(500)),
    });
    let out = b.driver.route_item(&key, new, failed);
    b.driver.apply(out);
    let state = *b.driver.state_tx.borrow();
    state
}

#[tokio::test]
async fn a_close_then_a_connect_that_cannot_reach_the_daemon_shows_down_in_either_order() {
    assert_eq!(
        close_then_failed_reconnect(true).await,
        ConnectionState::Down
    );
    assert_eq!(
        close_then_failed_reconnect(false).await,
        ConnectionState::Down
    );
}

#[tokio::test]
async fn a_retryable_refusal_of_a_replaced_handle_leaves_the_current_peer_alone() {
    let mut b = Bench::new();
    let old = b.add();
    let new = b.add();
    assert_ne!(old, new, "a re-add — the first handle never went live");
    let key = b.key();
    let out = b.driver.route_item(&key, old, retryable_refusal(&key));
    assert!(
        !out.iter().any(|r| matches!(r, Routed::Notice(_))),
        "{out:?}"
    );
    assert_eq!(b.driver.current, Some(new));
    assert!(
        b.driver.registry.get(key.peer_id()).is_some(),
        "the new peer was not stopped"
    );
}

// ── subscribe · unsubscribe · replay · output (3-2 U3b) ──────────────────────────────────────────

/// A window channel registered as the main window — it records what the actor relays to it.
fn recording_window(client: &DaemonClient) -> Arc<Mutex<Vec<Vec<u8>>>> {
    let got = Arc::new(Mutex::new(Vec::new()));
    let sink = got.clone();
    let channel = Channel::new(move |body| {
        if let InvokeResponseBody::Raw(bytes) = body {
            sink.lock().unwrap().push(bytes);
        }
        Ok(())
    });
    register_window(&client.registry, MAIN_WINDOW_LABEL.to_owned(), channel);
    got
}

/// `(generation, failed)` of every replay marker the window got, in order.
fn markers(window: &Mutex<Vec<Vec<u8>>>) -> Vec<(u64, bool)> {
    window
        .lock()
        .unwrap()
        .iter()
        .filter(|frame| frame.first() == Some(&MARKER_TAG))
        .map(|frame| {
            let generation = u64::from_be_bytes(frame[21..29].try_into().unwrap());
            (generation, frame[29] & 0b10 != 0)
        })
        .collect()
}

fn ack(agent_id: AgentId, epoch: u32) -> AgentEvent {
    AgentEvent::SubscribeAck {
        agent_id,
        action: SubscribeAction::Reset,
        current_epoch: epoch,
        oldest_seq: 0,
        latest_seq: 0,
        replay_from: 0,
        truncated: false,
        continues_conversation: false,
    }
}

fn is_subscribe(cmd: &AgentCommand, agent: AgentId, known: Option<u32>) -> bool {
    matches!(cmd, AgentCommand::Subscribe { agent_id, epoch, after_seq: None } if *agent_id == agent && *epoch == known)
}

fn is_unsubscribe(cmd: &AgentCommand, agent: AgentId) -> bool {
    matches!(cmd, AgentCommand::Unsubscribe { agent_id } if *agent_id == agent)
}

#[tokio::test]
async fn a_replay_goes_out_once_its_completion_marks_the_window_and_the_merged_request_follows() {
    let h = Harness::new(live());
    let agent = AgentId::new_v4();
    assert_eq!(
        h.client.request_replay(agent).await,
        Err(NOT_CONNECTED.to_owned())
    );
    let endpoint = h.connected().await;
    let window = recording_window(&h.client);
    h.client.router.set_visible_agents_for_test(&[agent]);

    let first = h.client.request_replay(agent).await.expect("written");
    assert!(is_subscribe(&h.command(&endpoint).await, agent, None));
    let merged = h.client.request_replay(agent).await.expect("merged");
    assert!(merged > first, "{merged} after {first}");
    // A later write on the same queue — nothing went out for the merged request before it.
    let other = AgentId::new_v4();
    h.client.unsubscribe(other);
    assert!(is_unsubscribe(&h.command(&endpoint).await, other));

    endpoint.push(text(&ack(agent, 7)));
    endpoint.push(text(&AgentEvent::ReplayComplete {
        agent_id: agent,
        epoch: 7,
    }));
    assert!(
        is_subscribe(&h.command(&endpoint).await, agent, Some(7)),
        "the merged request follows with the epoch the Ack taught"
    );
    wait_for("the success marker", || !markers(&window).is_empty()).await;
    assert_eq!(markers(&window), [(first, false)]);

    endpoint.push(text(&AgentEvent::SubscribeFailed {
        agent_id: agent,
        reason: "no such agent".into(),
    }));
    wait_for("the failure marker", || markers(&window).len() == 2).await;
    assert_eq!(markers(&window), [(first, false), (merged, true)]);
}

#[tokio::test]
async fn output_frames_reach_only_the_windows_that_show_their_agent() {
    let h = Harness::new(live());
    let (shown, hidden) = (AgentId::new_v4(), AgentId::new_v4());
    let endpoint = h.connected().await;
    let window = recording_window(&h.client);
    h.client.router.set_visible_agents_for_test(&[shown]);

    let frame = encode_terminal_frame(shown, 1, 0, b"hi");
    endpoint.push(Frame::Binary(encode_terminal_frame(hidden, 1, 0, b"no")));
    endpoint.push(Frame::Binary(frame.clone()));
    wait_for("the shown agent's frame", || {
        !window.lock().unwrap().is_empty()
    })
    .await;
    assert_eq!(
        *window.lock().unwrap(),
        [frame],
        "in order — the hidden one went nowhere"
    );
}

#[tokio::test]
async fn a_frame_too_short_to_read_drops_the_connection_and_the_next_one_replays_afresh() {
    let h = Harness::new(live());
    let agent = AgentId::new_v4();
    let old = h.connected().await;
    h.client.router.set_visible_agents_for_test(&[agent]);
    h.client.request_replay(agent).await.expect("written");
    assert!(is_subscribe(&h.command(&old).await, agent, None));

    old.push(Frame::Binary(vec![0; 3]));
    wait_for("Reconnecting", || {
        h.client.state() == ConnectionState::Reconnecting
    })
    .await;
    wait_for("the backoff", || h.peer_state() == Some(PeerState::Backoff)).await;
    // The redial waits the backoff (base less its jitter is above this) — a daemon that keeps sending a short frame
    // cannot drive a reconnect loop.
    tokio::time::sleep(Duration::from_millis(20)).await;
    h.clock.advance(Duration::from_millis(399));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(h.net.dials(), 1, "no redial before the backoff ran out");
    h.advance_until("the redial", || h.net.open_endpoints() > 0)
        .await;
    let (endpoint, _) = h.next_link().await;
    endpoint.push(hello());
    wait_for("connected again", || {
        h.client.state() == ConnectionState::Connected
    })
    .await;
    assert_eq!(h.net.dials(), 2);

    h.client
        .request_replay(agent)
        .await
        .expect("a fresh replay");
    assert!(
        is_subscribe(&h.command(&endpoint).await, agent, None),
        "the lost connection's flight was cleared — this one goes out instead of merging"
    );
}

#[tokio::test]
async fn a_replay_without_progress_gets_one_failure_marker_after_the_deadline_on_the_injected_clock(
) {
    let h = Harness::new(live());
    let agent = AgentId::new_v4();
    let endpoint = h.connected().await;
    let window = recording_window(&h.client);
    h.client.router.set_visible_agents_for_test(&[agent]);
    let generation = h.client.request_replay(agent).await.expect("written");
    h.command(&endpoint).await;
    // The link stays live while the clock moves: none of its own timers on this clock fires within the advance.
    let policy = link_policy();
    let advanced = REPLAY_DEADLINE + Duration::from_secs(1);
    assert!(
        advanced < policy.ping_interval && advanced < policy.idle_timeout,
        "{policy:?}"
    );

    h.clock.advance(REPLAY_DEADLINE - Duration::from_secs(1));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(markers(&window).is_empty(), "not before the deadline");
    h.clock.advance(Duration::from_secs(2));
    // Far less real time than one real 1 s tick — the sweep runs on the injected clock.
    let real = Instant::now();
    while markers(&window).is_empty() {
        assert!(
            real.elapsed() < Duration::from_millis(500),
            "no failure marker within 500 ms of real time"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(markers(&window), [(generation, true)]);
    assert_eq!(h.client.state(), ConnectionState::Connected);

    // The slot stays taken (no answer came): another request merges and writes nothing.
    h.client.request_replay(agent).await.expect("merged");
    let other = AgentId::new_v4();
    h.client.unsubscribe(other);
    assert!(is_unsubscribe(&h.command(&endpoint).await, other));
}

// Copied from the old suite, which keeps its own until U4 (TRD §6-2 U3 — the A1 safety on the single actor): a
// replay request a dying popout's webview sends after its close request unsubscribed sends no Subscribe; a request
// for an agent another window shows goes out.
#[tokio::test]
async fn a_replay_request_after_the_close_request_sends_no_subscribe() {
    let h = Harness::new(live());
    let endpoint = h.connected().await;
    let client: &DaemonClient = &h.client;
    let state = LayoutState::new();
    let subs = RouterSubs {
        router: &client.router,
        client,
    };
    let (shown, gone) = (AgentId::new_v4(), AgentId::new_v4());
    let (view, slot) = active_slot(&state, MAIN_WINDOW_LABEL);
    apply::assign_agent(
        &state,
        &subs,
        &NoLayoutEvents,
        view,
        slot,
        shown.to_string(),
    )
    .expect("assigned in main");
    state.0.lock().unwrap().create_window(POPUP).unwrap();
    let (view, slot) = active_slot(&state, POPUP);
    apply::assign_agent(&state, &subs, &NoLayoutEvents, view, slot, gone.to_string())
        .expect("assigned in the popout");

    close_requested_popup_window(POPUP, &state, &client.router, client);
    assert_eq!(
        client.request_replay(gone).await,
        Err(REPLAY_UNSENT.to_owned()),
        "no window shows it — no generation"
    );
    client
        .request_replay(shown)
        .await
        .expect("a shown agent still subscribes");
    // A later write on the same queue closes the list.
    let other = AgentId::new_v4();
    client.unsubscribe(other);

    let sent = h.commands(&endpoint, 3).await;
    assert_eq!(sent.len(), 3, "{sent:?}");
    assert!(
        is_unsubscribe(&sent[0], gone),
        "the close request's unsubscribe first: {sent:?}"
    );
    assert!(is_subscribe(&sent[1], shown, None), "{sent:?}");
    assert!(
        is_unsubscribe(&sent[2], other),
        "no Subscribe for the agent no window shows: {sent:?}"
    );
}

// Copied from the old suite, which keeps its own until U4 (TRD §6-2 U3 — A2): the names sink gets each daemon list
// after the frontend event, as it came; a broadcast that is not a list does not reach it.
#[tokio::test]
async fn agent_names_sink_receives_each_daemon_list_after_the_frontend_event() {
    let h = Harness::new(live());
    let (a1, a2) = (AgentId::new_v4(), AgentId::new_v4());
    let endpoint = h.connected().await;
    for event in [
        AgentEvent::AgentListUpdated {
            agents: vec![wire_agent(a1, "myproj"), wire_agent(a2, "myproj(1)")],
        },
        AgentEvent::StatusChanged {
            agent_id: a1,
            status: AgentStatus::Killed,
            epoch: 1,
        },
        AgentEvent::AgentListUpdated {
            agents: vec![wire_agent(a1, "renamed")],
        },
    ] {
        endpoint.push(text(&event));
    }
    wait_for("six lines", || h.events.seen().len() >= 6).await;
    assert_eq!(
        h.events.seen(),
        [
            "state:connected".to_owned(),
            "agent_list:2".to_owned(),
            format!("names:{a1}=myproj,{a2}=myproj(1)"),
            format!("status:{a1}:Killed:1"),
            "agent_list:1".to_owned(),
            format!("names:{a1}=renamed"),
        ]
    );
}

// ── same-key re-add (3-2 U3) ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_connect_overtaken_by_a_re_add_gets_the_new_handles_answer() {
    let h = Harness::new(live());
    let first = h.ensure();
    // Held without Hello — the first handle waits in its handshake.
    let (_held, _) = h.next_link().await;
    let second = h.ensure();
    let (endpoint, _) = h.next_link().await;
    endpoint.push(hello());
    assert_eq!(joined("the second ensure", second).await, Ok(()));
    assert_eq!(
        joined("the first ensure", first).await,
        Ok(()),
        "the re-add carried on its connect"
    );
}

#[tokio::test]
async fn a_re_add_of_a_once_connected_key_shows_its_first_failure_as_down() {
    let h = Harness::new(live());
    let old = h.connected().await;
    old.close();
    wait_for("Reconnecting", || {
        h.client.state() == ConnectionState::Reconnecting
    })
    .await;

    h.record.set(Current::Absent);
    let err = joined("ensure", h.ensure()).await.expect_err("no daemon");
    assert!(matches!(err, HandshakeError::Connect(_)), "{err:?}");
    assert_eq!(
        h.states(),
        ["state:connected", "state:reconnecting", "state:down"],
        "an explicit connect that fails shows Down, as the old driver does"
    );
}

#[tokio::test]
async fn a_re_add_keeps_the_new_flight_list_and_names_when_the_old_handles_tail_comes_after_the_new_snapshot(
) {
    let h = Harness::new(live());
    let agent = AgentId::new_v4();
    let old_endpoint = h.connected().await;
    let old = h.current_handle().await;
    // The old handle loses its link and waits in its backoff — the clock is not moved.
    old_endpoint.close();
    wait_for("Reconnecting", || {
        h.client.state() == ConnectionState::Reconnecting
    })
    .await;

    let task = h.ensure();
    let (endpoint, _) = h.next_link().await;
    endpoint.push(hello());
    joined("ensure", task)
        .await
        .expect("the new handle connected");
    assert_ne!(h.current_handle().await, old, "a re-add");

    let window = recording_window(&h.client);
    h.client.router.set_visible_agents_for_test(&[agent]);
    let generation = h.client.request_replay(agent).await.expect("written");
    h.command(&endpoint).await;
    endpoint.push(text(&AgentEvent::AgentListUpdated {
        agents: vec![wire_agent(agent, "new")],
    }));
    let names = format!("names:{agent}=new");
    wait_for("the new snapshot", || h.events.seen().contains(&names)).await;
    let before = h.events.seen();

    // The old supervisor's tail, routed only now — with a newer generation than the new connection's (a displaced
    // supervisor can still finish a handshake), so only the handle check drops it.
    let key = h.link().key.clone();
    let peer = key.peer_id().clone();
    let newer = Generation(
        h.link()
            .registry
            .get(&peer)
            .expect("the new peer")
            .generation()
            .0
            + 1,
    );
    let tail = [
        Incoming::Event(TransportEvent::Connected {
            peer: peer.clone(),
            generation: newer,
        }),
        Incoming::Message(Arrival {
            peer: peer.clone(),
            generation: newer,
            msg: ShellIn::Event(AgentEvent::AgentListUpdated {
                agents: vec![wire_agent(agent, "stale")],
            }),
        }),
        Incoming::Event(TransportEvent::Disconnected {
            peer: peer.clone(),
            generation: newer,
            cause: DisconnectCause::PeerClosed,
        }),
        retryable_refusal(&key),
        Incoming::Event(TransportEvent::Closed {
            peer: peer.clone(),
            generation: newer,
            cause: DisconnectCause::Explicit,
        }),
    ];
    for item in tail {
        h.link()
            .routable_tx
            .send(Routable::Item {
                key: key.clone(),
                token: old,
                item,
            })
            .await
            .expect("the actor runs");
    }

    endpoint.push(text(&ack(agent, 3)));
    endpoint.push(text(&AgentEvent::ReplayComplete {
        agent_id: agent,
        epoch: 3,
    }));
    wait_for("the success marker", || !markers(&window).is_empty()).await;
    assert_eq!(
        markers(&window),
        [(generation, false)],
        "the new flight was not cleared"
    );
    assert_eq!(
        h.events.seen(),
        before,
        "no stale list or names, no state edge"
    );
    assert_eq!(h.client.state(), ConnectionState::Connected);
    assert!(
        h.peer_state().is_some_and(|state| state.is_live()),
        "the old handle's refusal did not stop the new peer: {:?}",
        h.peer_state()
    );
}

#[tokio::test]
async fn a_merged_replay_request_whose_last_window_closed_meanwhile_writes_nothing_and_frees_the_slot(
) {
    let h = Harness::new(live());
    let agent = AgentId::new_v4();
    let endpoint = h.connected().await;
    h.client.router.set_visible_agents_for_test(&[agent]);
    h.client.request_replay(agent).await.expect("written");
    assert!(is_subscribe(&h.command(&endpoint).await, agent, None));
    h.client.request_replay(agent).await.expect("merged");

    // Its last window closes — the layout rebuilds the routing and unsubscribes.
    h.client.router.set_visible_agents_for_test(&[]);
    h.client.unsubscribe(agent);
    assert!(is_unsubscribe(&h.command(&endpoint).await, agent));
    endpoint.push(text(&ack(agent, 4)));
    endpoint.push(text(&AgentEvent::ReplayComplete {
        agent_id: agent,
        epoch: 4,
    }));
    // A broadcast behind them on the same connection: once it reached the frontend, the actor has taken both events
    // and written whatever they made it write.
    endpoint.push(text(&AgentEvent::AgentListUpdated {
        agents: vec![wire_agent(agent, "barrier")],
    }));
    let names = format!("names:{agent}=barrier");
    wait_for("the barrier list", || h.events.seen().contains(&names)).await;
    let other = AgentId::new_v4();
    h.client.unsubscribe(other);
    assert!(
        is_unsubscribe(&h.command(&endpoint).await, other),
        "no Subscribe after the agent's Unsubscribe"
    );

    // A window shows it again: the slot is free, so the request goes out instead of merging.
    h.client.router.set_visible_agents_for_test(&[agent]);
    h.client.request_replay(agent).await.expect("written");
    assert!(is_subscribe(&h.command(&endpoint).await, agent, Some(4)));
}

#[test]
fn a_subscribe_refusal_logs_the_daemons_reason_masked() {
    let agent = AgentId::new_v4();
    let mut flight = ReplayFlightSet::new(REPLAY_DEADLINE);
    let mut subs = HashMap::new();
    let router = OutputRouter::new();
    let lines = Arc::new(Mutex::new(Vec::new()));
    crate::daemon_client::link::test_log::capture(lines.clone(), || {
        apply_replay_event(
            &AgentEvent::SubscribeFailed {
                agent_id: agent,
                reason: "evil\nline\u{1b}[31m".into(),
            },
            &mut flight,
            &mut subs,
            &router,
            std::time::Instant::now(),
            &mut |_: &[WindowLabel], _: &[u8]| {},
        )
    });
    let lines = lines.lock().unwrap();
    let warn = lines
        .iter()
        .find(|line| line.starts_with("level=WARN ") && line.contains(&agent.to_string()))
        .unwrap_or_else(|| panic!("{lines:?}"));
    assert!(warn.contains("reason=evilline[31m "), "{warn}");
    assert!(!warn.contains('\n') && !warn.contains('\u{1b}'), "{warn:?}");
}

#[tokio::test]
async fn after_a_frame_too_short_to_read_nothing_more_of_that_connection_is_relayed() {
    let h = Harness::new(live());
    let agent = AgentId::new_v4();
    let old = h.connected().await;
    let window = recording_window(&h.client);
    h.client.router.set_visible_agents_for_test(&[agent]);
    h.client.request_replay(agent).await.expect("written");
    assert!(is_subscribe(&h.command(&old).await, agent, None));
    old.push(text(&ack(agent, 2)));

    // The unreadable frame, then what the daemon sent behind it on the same connection.
    old.push(Frame::Binary(vec![0; 3]));
    old.push(Frame::Binary(encode_terminal_frame(agent, 2, 0, b"after")));
    old.push(text(&AgentEvent::ReplayComplete {
        agent_id: agent,
        epoch: 2,
    }));
    wait_for("the backoff", || h.peer_state() == Some(PeerState::Backoff)).await;
    // Everything the old connection delivered is routed by the time its `Disconnected` made the state Reconnecting.
    wait_for("Reconnecting", || {
        h.client.state() == ConnectionState::Reconnecting
    })
    .await;
    assert!(
        window.lock().unwrap().is_empty(),
        "neither the frame behind the unreadable one nor a success marker: {:?}",
        markers(&window)
    );

    h.advance_until("the redial", || h.net.open_endpoints() > 0)
        .await;
    let (endpoint, _) = h.next_link().await;
    endpoint.push(hello());
    wait_for("connected again", || {
        h.client.state() == ConnectionState::Connected
    })
    .await;
    h.client
        .request_replay(agent)
        .await
        .expect("a fresh replay");
    assert!(is_subscribe(&h.command(&endpoint).await, agent, Some(2)));
}

// ── a connection being dropped (bench) ──────────────────────────────────────────────────────────

/// Routes `item` for the bench's handle and applies what routing returns.
fn bench_route(b: &mut Bench, token: HandleToken, item: Incoming<ShellWire>) {
    let key = b.key();
    let out = b.driver.route_item(&key, token, item);
    b.driver.apply(out);
}

fn bench_message(b: &Bench, generation: Generation, msg: ShellIn) -> Incoming<ShellWire> {
    Incoming::Message(Arrival {
        peer: b.key().peer_id().clone(),
        generation,
        msg,
    })
}

fn bench_connected(b: &Bench, generation: Generation) -> Incoming<ShellWire> {
    Incoming::Event(TransportEvent::Connected {
        peer: b.key().peer_id().clone(),
        generation,
    })
}

#[tokio::test]
async fn a_short_frame_in_a_closed_links_tail_still_stops_relaying_that_connection() {
    let mut b = Bench::new();
    let agent = AgentId::new_v4();
    let g = Generation(1);
    let token = b.add();
    let connected = bench_connected(&b, g);
    bench_route(&mut b, token, connected);
    b.driver.router.set_visible_agents_for_test(&[agent]);
    let got = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let sink = got.clone();
    register_window(
        &b.driver.windows,
        MAIN_WINDOW_LABEL.to_owned(),
        Channel::new(move |body| {
            if let InvokeResponseBody::Raw(bytes) = body {
                sink.lock().unwrap().push(bytes);
            }
            Ok(())
        }),
    );
    // A replay in flight on g, acknowledged.
    let now = b.driver.clock.now();
    assert!(b.driver.flight.request_replay(agent, now).send_now);
    let acked = bench_message(&b, g, ShellIn::Event(ack(agent, 2)));
    bench_route(&mut b, token, acked);

    // The user closes; g's tail is still routed — the registry holds no peer any more.
    b.driver.close();
    assert!(b.driver.registry.get(b.key().peer_id()).is_none());
    for msg in [
        ShellIn::Output(vec![0; 3]),
        ShellIn::Output(encode_terminal_frame(agent, 2, 0, b"after")),
        ShellIn::Event(AgentEvent::ReplayComplete {
            agent_id: agent,
            epoch: 2,
        }),
    ] {
        let item = bench_message(&b, g, msg);
        bench_route(&mut b, token, item);
    }
    assert!(got.lock().unwrap().is_empty(), "nothing relayed, no marker");
    assert_eq!(b.driver.dropping, Some(g));
    assert_eq!(b.driver.open, None);
}

#[tokio::test]
async fn a_subscribe_that_cannot_be_written_ends_the_connection_for_the_actor_until_its_link_down()
{
    let mut b = Bench::new();
    let agent = AgentId::new_v4();
    let g = Generation(1);
    let token = b.add();
    // Routed as connected while the bench's peer is not live — the write fails.
    let connected = bench_connected(&b, g);
    bench_route(&mut b, token, connected);
    b.driver.router.set_visible_agents_for_test(&[agent]);
    assert_eq!(
        b.driver.request_replay(agent),
        Err(REPLAY_UNSENT.to_owned())
    );
    assert_eq!(b.driver.open, None);
    assert_eq!(b.driver.dropping, Some(g));
    assert_eq!(
        b.driver.request_replay(agent),
        Err(NOT_CONNECTED.to_owned())
    );

    let lost = Incoming::Event(TransportEvent::Disconnected {
        peer: b.key().peer_id().clone(),
        generation: g,
        cause: DisconnectCause::WriteFailed,
    });
    bench_route(&mut b, token, lost);
    assert_eq!(b.driver.dropping, None, "cleared by g's LinkDown");
}
