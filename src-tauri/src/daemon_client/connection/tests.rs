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
    encode_terminal_frame, AgentStatus, RequestId, Secret, SubscribeAction, UsageVendorState,
    PROTOCOL_VERSION,
};
use engram_dashboard_transport::testing::{ManualClock, MemoryEndpoint, MemoryNetwork};
use engram_dashboard_transport::{
    Arrival, Close, CloseCode, DisconnectCause, Frame, GaveUpReason, PeerState, QueueFullPolicy,
};
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::task::JoinHandle;

use engram_dashboard_command::{CommandDecl, CommandTable, OwnerLookup, OwnerToken};

use crate::commands::discovery::BootStartWatch;
use crate::commands::layout::RouterSubs;
use crate::commands::popout::{close_requested_popup_window, drop_window_in_model, PopupCounter};
use crate::daemon_client::inbound::{BoxedTask, InboundReceiver, TaskSpawner, ViewCommandPort};
use crate::daemon_client::link::endpoint::{Current, Endpoint};
use crate::daemon_client::link::route::{AUTH_REFUSED, VERSION_REFUSED};
use crate::daemon_client::refusal::RecordingRefusals;
use crate::daemon_client::replay_flight::MARKER_TAG;
use crate::daemon_client::test_support::{
    active_slot, bus_request, bus_table, relayout, state_edges, status_lines, usage_update,
    wire_agent, NoLayoutEvents, OpenWindows, RecordingEvents, POPUP,
};
use crate::daemon_client::usage_interest::{InterestAction, USAGE_INTEREST_SHRINK_DELAY};
use crate::daemon_client::{Band, ConnectionState, ConnectionStatus, DaemonClient};
use crate::layout::commands::CATALOG_VERSION;
use crate::layout::{apply, LayoutState, MAIN_WINDOW_LABEL};
use crate::output_channel::register_window;
use crate::output_router::DropCause;
use crate::tray::actions::{for_each_ui_window, LayoutUsageVisibility, UsageVisibility};

const TOKEN: &str = "cccc2222cccc2222cccc2222cccc2222cccc2222cccc2222cccc2222cccc2222";

/// The daemon's endpoint record as the test sets it. `ensure` (the search that may start a daemon) answers the record
/// unless the test made it fail, and counts its calls.
struct Record {
    now: Mutex<Current>,
    search_fails: Mutex<Option<String>>,
    /// Set: the search answers this endpoint whatever the record says.
    search_finds: Mutex<Option<Endpoint>>,
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
            search_finds: Mutex::new(None),
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

    fn search_finds(&self, endpoint: Endpoint) {
        *self.search_finds.lock().unwrap() = Some(endpoint);
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
        if let Some(endpoint) = self.search_finds.lock().unwrap().clone() {
            return Ok(endpoint);
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
    refusals: Arc<RecordingRefusals>,
    client: Arc<DaemonClient>,
}

impl Harness {
    fn new(now: Current) -> Self {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let record = Record::new(now);
        let events = Arc::new(RecordingEvents::default());
        let refusals = Arc::new(RecordingRefusals::new());
        let client = Arc::new(DaemonClient::new_on_link(
            Handle::current(),
            net.dialer(),
            record.clone(),
            clock.handle(),
            events.clone(),
            events.clone(),
            refusals.clone(),
        ));
        Self {
            net,
            clock,
            record,
            events,
            refusals,
            client,
        }
    }

    fn link(&self) -> &LinkClient {
        &self.client.link
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

    /// The state edges the frontend got, in order (`state:<word>`).
    fn states(&self) -> Vec<String> {
        state_edges(&self.events.seen())
    }

    /// Every status the frontend got, in order, one line per emit (`status:<word>:<band>`).
    fn statuses(&self) -> Vec<String> {
        status_lines(&self.events.seen())
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
fn the_link_policy_retries_network_failures_without_limit_waits_30_s_for_a_reply_and_keeps_the_other_defaults(
) {
    let policy = link_policy();
    assert_eq!(policy.reconnect.max_attempts, u32::MAX);
    assert_eq!(policy.request_timeout, Duration::from_secs(30));
    let mut expected = Policy::default();
    expected.reconnect.max_attempts = u32::MAX;
    expected.request_timeout = Duration::from_secs(30);
    assert_eq!(policy, expected);
    // `Registry::new` debug-asserts the policy's own invariants.
    assert_eq!(policy.check(), Ok(()));
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
            "status:connected:none".to_owned(),
            "agent_list:1".to_owned(),
            format!("names:{agent}=alpha"),
            format!("agent_status:{agent}:Killed:1"),
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
        h.statuses(),
        ["status:down:not_connected"],
        "one status: no Reconnecting, no second Down"
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
    assert_eq!(
        err.to_string(),
        "no live daemon (no endpoint record) — is the daemon running?"
    );
    wait_for("Down", || h.states() == ["state:down"]).await;
    assert_eq!(h.record.searches(), 0);
}

// A record whose daemon is gone says so in the same shape — the endpoint source's cause in place of "no record".
#[tokio::test]
async fn ensure_on_a_stale_record_names_its_cause() {
    let h = Harness::new(Current::Unusable(
        "daemon.json names pid 4242 which is not alive".into(),
    ));
    let err = joined("ensure", h.ensure()).await.expect_err("no daemon");
    assert_eq!(
        err.to_string(),
        "no live daemon (daemon.json names pid 4242 which is not alive) — is the daemon running?"
    );
}

// Any other unreachable first failure keeps the connect text — only "no live daemon" is reworded.
#[tokio::test]
async fn ensure_on_a_busy_record_with_no_known_address_is_a_connect_failure() {
    let h = Harness::new(Current::Busy);
    let err = joined("ensure", h.ensure()).await.expect_err("no address");
    assert!(matches!(err, HandshakeError::Connect(_)), "{err:?}");
}

#[tokio::test]
async fn a_daemon_error_in_the_handshake_stops_for_good_without_a_second_dial() {
    let h = Harness::new(live());
    let task = h.ensure();
    let (endpoint, _) = h.next_link().await;
    // A net-era daemon's refusal: an `Error` frame and a close without a code (the current daemon adds a code; this is the stricter case).
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
    assert_eq!(h.statuses(), ["status:down:refused"]);

    for _ in 0..10 {
        h.clock.advance(Duration::from_secs(60));
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(h.net.dials(), 1, "a refusal is never retried (ADR-0346)");
    assert_eq!(h.record.reads(), 1);
}

#[tokio::test]
async fn command_round_trips_and_their_failures_read_as_the_shell_texts() {
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
        crate::daemon_client::link::errors::SENT_OUTCOME_UNKNOWN
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
    assert_eq!(
        h.statuses(),
        [
            "status:connected:none",
            "status:reconnecting:none",
            "status:down:none",
        ],
        "one emit each — an explicit close shows no band"
    );

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
    assert_eq!(h.statuses(), ["status:down:not_connected"]);
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
    events: Arc<RecordingEvents>,
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
                names: events.clone(),
                router: Arc::new(OutputRouter::new()),
                windows: WindowChannelRegistry::default(),
                inbound: Arc::new(InboundSlot::new()),
                refusals: Arc::new(RefusalAlerts::new(Arc::new(RecordingRefusals::new()))),
                usage: SharedUsageInterest::default(),
            },
        );
        Self {
            driver,
            events,
            _client: client,
            _cmd_rx: cmd_rx,
            _routable_rx: routable_rx,
            _net: net,
        }
    }

    /// Adds a handle and returns its token.
    fn add(&mut self) -> HandleToken {
        let (first, _answer) = oneshot::channel();
        self.driver.add(0, None, first);
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
                Routed::Status(ConnectionStatus {
                    state: ConnectionState::Down,
                    band: Some(Band::NotConnected { refused: false }),
                }),
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

// A stop leaves the link open: the daemon's exit is the link's normal loss, and its first failed reconnect shows ① while
//   the state stays `reconnecting` (D1).
#[tokio::test]
async fn after_the_daemon_exits_the_links_first_failed_reconnect_shows_band_one() {
    let mut b = Bench::new();
    let token = b.add();
    let key = b.key();
    let g = Generation(1);
    let items = [
        bench_connected(&b, g),
        Incoming::Event(TransportEvent::Disconnected {
            peer: key.peer_id().clone(),
            generation: g,
            cause: DisconnectCause::PeerClosed,
        }),
        Incoming::Event(TransportEvent::ConnectFailed {
            peer: key.peer_id().clone(),
            attempt: 1,
            of: u32::MAX,
            cause: ConnectFailure::Unreachable(engram_dashboard_transport::LinkError::new(
                "connection refused",
            )),
            retry_in: Some(Duration::from_millis(500)),
        }),
    ];
    for item in items {
        bench_route(&mut b, token, item);
    }
    assert_eq!(
        status_lines(&b.events.seen()),
        [
            "status:connected:none",
            "status:reconnecting:none",
            "status:reconnecting:not_connected",
        ]
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
    b.driver.add(0, None, first);
    assert_eq!(answer.try_recv(), Ok(Ok(())));
    assert_eq!(b.driver.current, Some(token), "no re-add");
    assert_eq!(b.driver.dialer.registered(), 1);
}

// The order the frontend relies on (`src/api/tauriTransport.ts` `doConnect`): the status watch and
// the frontend's `connected` come before the connect resolves.
#[tokio::test]
async fn a_connect_resolves_only_after_its_connected_is_shown() {
    let mut b = Bench::new();
    let (first, answer) = oneshot::channel();
    b.driver.add(0, None, first);
    let token = b.driver.current.expect("a handle was added");
    let answer = Arc::new(Mutex::new(answer));
    let seen = Arc::new(Mutex::new(None));
    {
        let (answer, seen, status) = (answer.clone(), seen.clone(), b._client.status_rx());
        b.events.then_on_connected(move || {
            let unanswered = matches!(
                answer.lock().unwrap().try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            );
            *seen.lock().unwrap() = Some((unanswered, status.borrow().state));
        });
    }
    let key = b.key();
    let connected = bench_connected(&b, Generation(1));
    b.driver.on_routable(Routable::Item {
        key,
        token,
        item: connected,
    });
    assert_eq!(
        *seen.lock().unwrap(),
        Some((true, ConnectionState::Connected)),
        "(not answered yet, state) when the frontend heard `connected`"
    );
    assert_eq!(answer.lock().unwrap().try_recv(), Ok(Ok(())));
}

// The endpoint `connect`'s search found is the first dial's fallback: a record held busy right after the search (a
// daemon it just spawned) still connects to that daemon, with its credential.
#[tokio::test]
async fn a_connect_dials_the_daemon_its_search_found_while_the_record_is_busy() {
    let h = Harness::new(Current::Busy);
    let Current::Live(found) = live() else {
        unreachable!("live() is a live record")
    };
    h.record.search_finds(found);
    let task = h.connect();
    let (endpoint, auth) = h.next_link().await;
    assert!(auth.contains(TOKEN), "the found endpoint's credential");
    assert!(
        h.record.reads() >= 1,
        "the dial still read the record first"
    );
    endpoint.push(hello());
    assert_eq!(joined("connect", task).await, Ok(()));
}

// The replay deadline sweep runs only while a replay waits for its deadline — an idle actor sets no timer.
#[tokio::test]
async fn an_idle_actor_sets_no_timer() {
    let h = Harness::new(live());
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(h.clock.sleepers(), 0);
    drop(h);
}

#[tokio::test]
async fn a_close_answers_the_waiting_connect_at_once_and_a_later_add_waits_for_its_own_handle() {
    let mut b = Bench::new();
    let (first, mut answer_a) = oneshot::channel();
    b.driver.add(0, None, first);
    let old = b.driver.current.expect("a handle");
    b.driver.close();
    assert_eq!(
        answer_a.try_recv(),
        Ok(Err(HandshakeError::ClosedBeforeHello)),
        "answered by the close itself, before the old Closed is routed"
    );

    let (second, mut answer_b) = oneshot::channel();
    b.driver.add(0, None, second);
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
    let state = b.driver.status_tx.borrow().state;
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

// Copied from the old suite (TRD §6-2 U3 — the A1 safety on the single actor): a
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

// Copied from the old suite (TRD §6-2 U3 — A2): the names sink gets each daemon list
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
            "status:connected:none".to_owned(),
            "agent_list:2".to_owned(),
            format!("names:{a1}=myproj,{a2}=myproj(1)"),
            format!("agent_status:{a1}:Killed:1"),
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
    assert!(matches!(err, HandshakeError::NoLiveDaemon(_)), "{err:?}");
    assert_eq!(
        h.states(),
        ["state:connected", "state:reconnecting", "state:down"],
        "an explicit connect that fails shows Down"
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

// ── daemon commands · registration · usage interest (3-2 U3c) ───────────────────────────────────

/// A command name the shell table does not hold — [`HeldView`] takes it.
const HELD: &str = "view.held";

/// A webview port that takes [`HELD`] and answers it only once the test releases it.
#[derive(Default)]
struct HeldView {
    entered: AtomicUsize,
    answered: Arc<AtomicUsize>,
    release: Arc<tokio::sync::Notify>,
}

impl ViewCommandPort for HeldView {
    fn declarations(&self) -> Vec<CommandDecl> {
        Vec::new()
    }

    fn lookup(&self, name: &str) -> OwnerLookup {
        if name == HELD {
            OwnerLookup::Available(OwnerToken::new("view"))
        } else {
            OwnerLookup::Unknown
        }
    }

    fn dispatch(
        &self,
        env: CommandEnvelope,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = BusReply> + Send>> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let (release, answered) = (self.release.clone(), self.answered.clone());
        Box::pin(async move {
            release.notified().await;
            answered.fetch_add(1, Ordering::SeqCst);
            BusReply::ok(env.request_id, serde_json::json!({}))
        })
    }
}

/// A daemon command request for `name` and its request id.
fn command_request(name: &str) -> (engram_dashboard_command::RequestId, AgentEvent) {
    let envelope = CommandEnvelope {
        name: name.to_owned(),
        request_id: engram_dashboard_command::RequestId::new(),
        owner: OwnerToken::new("shell"),
        proto_ver: CATALOG_VERSION,
        args: serde_json::json!({}),
    };
    let event = AgentEvent::CommandRequest {
        envelope: serde_json::value::to_raw_value(&envelope).unwrap(),
    };
    (envelope.request_id, event)
}

/// A short name for a command the shell wrote.
fn label(cmd: &AgentCommand) -> String {
    match cmd {
        AgentCommand::UsageSubscribe { vendors } => {
            let vendors: Vec<String> = vendors.iter().map(|v| format!("{v:?}")).collect();
            format!("usage[{}]", vendors.join(","))
        }
        AgentCommand::RegisterCommands { .. } => "register".to_owned(),
        AgentCommand::CommandOutcome { reply } => {
            match protocol_state::reply_from_wire(reply).map(|reply| reply.outcome) {
                Ok(Ok(_)) => "outcome:ok".to_owned(),
                Ok(Err(e)) => format!("outcome:err:{e:?}"),
                Err(e) => format!("outcome:broken:{e}"),
            }
        }
        AgentCommand::RefreshUsageLimits { vendor, .. } => format!("refresh:{vendor:?}"),
        AgentCommand::Unsubscribe { agent_id } => format!("unsubscribe:{agent_id}"),
        AgentCommand::ListAgents { .. } => "list".to_owned(),
        other => format!("{other:?}"),
    }
}

/// Queues an unsubscribe on the actor as an order marker and returns its label.
fn marker(client: &DaemonClient) -> String {
    let agent = AgentId::new_v4();
    client.unsubscribe(agent);
    format!("unsubscribe:{agent}")
}

/// The usage state's socket epoch — what the webview pull gets.
fn usage_socket(client: &DaemonClient) -> u64 {
    client
        .usage_interest()
        .lock()
        .snapshot_for_webview(Instant::now())
        .socket_epoch
}

impl Harness {
    /// Drops the daemon link and lets the link redial; returns the daemon side of the new link once connected.
    async fn reconnected(&self, old: &MemoryEndpoint) -> Arc<MemoryEndpoint> {
        old.close();
        wait_for("Reconnecting", || {
            self.client.state() == ConnectionState::Reconnecting
        })
        .await;
        self.advance_until("the redial", || self.net.open_endpoints() > 0)
            .await;
        let (endpoint, _) = self.next_link().await;
        endpoint.push(hello());
        wait_for("connected again", || {
            self.client.state() == ConnectionState::Connected
        })
        .await;
        endpoint
    }

    async fn labels(&self, endpoint: &MemoryEndpoint, n: usize) -> Vec<String> {
        self.commands(endpoint, n).await.iter().map(label).collect()
    }
}

#[tokio::test]
async fn a_daemon_command_is_answered_on_the_connection_it_came_on_and_not_on_a_later_one() {
    let h = Harness::new(live());
    let view = Arc::new(HeldView::default());
    let spawner = Arc::new(CountingSpawner::default());
    h.client.inbound.set(Arc::new(InboundReceiver::with_view(
        CommandTable::new(&[]),
        spawner.clone(),
        CATALOG_VERSION,
        view.clone(),
    )));
    let old = h.connected().await;

    let (asked, request) = command_request("no.such.command");
    old.push(text(&request));
    match h.command(&old).await {
        AgentCommand::CommandOutcome { reply } => {
            let reply = protocol_state::reply_from_wire(&reply).unwrap();
            assert_eq!(reply.request_id, asked);
            assert!(reply.outcome.is_err(), "{:?}", reply.outcome);
        }
        other => panic!("expected the outcome, got {other:?}"),
    }

    let (held, request) = command_request(HELD);
    old.push(text(&request));
    wait_for("the view took the held command", || {
        view.entered.load(Ordering::SeqCst) == 1
    })
    .await;
    let endpoint = h.reconnected(&old).await;
    view.release.notify_one();
    // Its task ends after the outcome was handed to the transport (`ReplySink` → `send_command_outcome`).
    wait_for("the held command's outcome handed over", || {
        spawner.finished.load(Ordering::SeqCst) == 2
    })
    .await;
    assert_eq!(view.answered.load(Ordering::SeqCst), 1);
    let m = marker(&h.client);
    let sent = h.commands(&endpoint, 1).await;
    assert_eq!(
        sent.iter().map(label).collect::<Vec<_>>(),
        [m],
        "the outcome of a request of the old connection is not written on the new one (held {held})"
    );
}

#[tokio::test]
async fn a_daemon_reply_or_request_too_broken_to_read_is_logged_and_dropped() {
    let mut b = Bench::new();
    let g = Generation(1);
    let token = b.add();
    let connected = bench_connected(&b, g);
    bench_route(&mut b, token, connected);
    // Each payload's parse error quotes a marker value — the log must not.
    const MARKER: &str = "MARKER_7Q";
    let (_, request) = command_request("x");
    let AgentEvent::CommandRequest { envelope } = request else {
        unreachable!()
    };
    let mut envelope: serde_json::Value = serde_json::from_str(envelope.get()).unwrap();
    envelope["proto_ver"] = MARKER.into();
    let mut reply = serde_json::to_value(BusReply::ok(
        engram_dashboard_command::RequestId::new(),
        serde_json::json!({}),
    ))
    .unwrap();
    reply["outcome"] = serde_json::json!({ MARKER: 1 });
    let raw = |value: &serde_json::Value| {
        serde_json::value::RawValue::from_string(value.to_string()).unwrap()
    };
    let quoted = [
        protocol_state::envelope_from_wire(&raw(&envelope)).map(|_| ()),
        protocol_state::reply_from_wire(&raw(&reply)).map(|_| ()),
    ];
    for error in quoted {
        let error = error.expect_err("a broken payload").to_string();
        assert!(
            error.contains(MARKER),
            "the parse error must quote the marker: {error}"
        );
    }

    let lines = Arc::new(Mutex::new(Vec::new()));
    crate::daemon_client::link::test_log::capture(lines.clone(), || {
        for event in [
            AgentEvent::CommandReply { reply: raw(&reply) },
            AgentEvent::CommandRequest {
                envelope: raw(&envelope),
            },
        ] {
            let item = bench_message(&b, g, ShellIn::Event(event));
            bench_route(&mut b, token, item);
        }
    });
    let lines = lines.lock().unwrap();
    for event in ["CommandReply", "CommandRequest"] {
        let field = format!("event=\"{event}\" ");
        let warns = lines
            .iter()
            .filter(|line| line.starts_with("level=WARN ") && line.contains(&field))
            .count();
        assert_eq!(warns, 1, "{event}: {lines:?}");
    }
    assert!(
        !lines.iter().any(|line| line.starts_with("level=ERROR ")),
        "a broken envelope is not run (no table lookup): {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains(MARKER)),
        "no daemon text in the log: {lines:?}"
    );
}

const REFUSAL: &str = "INVALID_ARGUMENT: a command name may be at most 128 bytes (got 129)";

#[tokio::test]
async fn every_connect_registers_once_and_a_refusal_shows_its_box_while_the_actor_stays_responsive()
{
    let h = Harness::new(live());
    let state = LayoutState::new();
    h.client.install_command_table(
        bus_table(&state, &h.client),
        CATALOG_VERSION,
        Arc::new(HeldView::default()),
    );
    let old = h.connected().await;
    let AgentCommand::RegisterCommands { request_id, .. } = h.command(&old).await else {
        panic!("the registration comes first");
    };

    // The registration reply is pending — actor round trips still complete (bounded: a stuck actor fails here).
    tokio::time::timeout(Duration::from_secs(10), h.current_handle())
        .await
        .expect("the actor answers while the registration reply is pending");
    let replay = tokio::time::timeout(
        Duration::from_secs(10),
        h.client.request_replay(AgentId::new_v4()),
    )
    .await
    .expect("the actor answers while the registration reply is pending");
    assert_eq!(replay, Err(REPLAY_UNSENT.to_owned()));

    old.push(text(&AgentEvent::Error {
        request_id: Some(request_id),
        message: REFUSAL.into(),
    }));
    wait_for("the refusal box", || !h.refusals.shown().is_empty()).await;
    assert_eq!(
        h.refusals.shown(),
        [(RefusalSite::Register, REFUSAL.to_owned())]
    );

    let endpoint = h.reconnected(&old).await;
    assert_eq!(label(&h.command(&endpoint).await), "register");
    let m = marker(&h.client);
    assert_eq!(
        label(&h.command(&endpoint).await),
        m,
        "one registration per connect"
    );
}

// The order rule of the opened connection (module doc): the actor is held inside its `LinkUp` step — the test holds
// the usage lock that step takes — while the peer is already live; a round trip and a fire-and-forget sent then go out
// after the connection's on-open writes, not before and not failed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)] // Holding the usage lock is the device: it stops the actor in its `LinkUp` step.
async fn a_command_sent_as_soon_as_the_peer_is_live_goes_out_after_the_registration_and_the_usage_subscribe(
) {
    let h = Harness::new(live());
    let state = LayoutState::new();
    h.client.install_command_table(
        bus_table(&state, &h.client),
        CATALOG_VERSION,
        Arc::new(HeldView::default()),
    );
    let _ = relayout(h.client.usage_interest(), true, false);
    let connect = h.connect();
    let (endpoint, _) = h.next_link().await;

    let usage = h.client.usage_interest().lock();
    endpoint.push(hello());
    wait_for("the peer is live", || {
        h.peer_state() == Some(PeerState::Live)
    })
    .await;
    let rid = RequestId::new();
    let request = {
        let client = h.client.clone();
        tokio::spawn(async move {
            client
                .send_command(AgentCommand::ListAgents { request_id: rid })
                .await
        })
    };
    let fired = AgentId::new_v4();
    h.client
        .send_fire_and_forget(AgentCommand::Unsubscribe { agent_id: fired });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        endpoint.frames().is_empty(),
        "nothing goes out before the connection is opened"
    );
    assert!(
        !request.is_finished(),
        "the round trip waits, it does not fail"
    );
    drop(usage);

    let sent = h.labels(&endpoint, 4).await;
    assert_eq!(sent.len(), 4, "{sent:?}");
    assert_eq!(sent[..2], ["usage[Claude]", "register"], "{sent:?}");
    let mut callers = sent[2..].to_vec();
    callers.sort();
    assert_eq!(callers, ["list".to_owned(), format!("unsubscribe:{fired}")]);

    endpoint.push(text(&AgentEvent::Ack { request_id: rid }));
    assert!(matches!(
        joined("the round trip", request).await,
        Ok(AgentEvent::Ack { request_id }) if request_id == rid
    ));
    joined("connect", connect).await.expect("connected");
}

// Copied from the old suite (TRD §6-2).
#[tokio::test]
async fn a_usage_broadcast_notifies_once_with_labels_and_socket_epoch() {
    let h = Harness::new(live());
    let _ = relayout(h.client.usage_interest(), true, false);
    let endpoint = h.connected().await;
    assert_eq!(label(&h.command(&endpoint).await), "usage[Claude]");
    let socket = usage_socket(&h.client);
    assert_ne!(socket, 0);

    endpoint.push(text(&usage_update(
        AgentBackendKind::Claude,
        7,
        &[AgentBackendKind::Claude],
    )));
    wait_for("the usage notice", || h.events.seen().len() >= 2).await;
    let m = marker(&h.client);
    assert_eq!(
        label(&h.command(&endpoint).await),
        m,
        "the same set — no resend"
    );
    assert_eq!(
        h.events.seen(),
        [
            "status:connected:none".to_owned(),
            format!("usage:Claude:7:{MAIN_WINDOW_LABEL}:{socket}"),
        ]
    );
}

// Copied from the old suite (TRD §6-2).
#[tokio::test]
async fn a_mismatched_broadcast_writes_the_interest_back_on_its_socket() {
    let h = Harness::new(live());
    let _ = relayout(h.client.usage_interest(), true, true);
    let endpoint = h.connected().await;
    assert_eq!(label(&h.command(&endpoint).await), "usage[Claude,Codex]");
    let socket = usage_socket(&h.client);

    endpoint.push(text(&usage_update(
        AgentBackendKind::Claude,
        1,
        &[AgentBackendKind::Claude],
    )));
    wait_for("the usage notice", || h.events.seen().len() >= 2).await;
    let m = marker(&h.client);
    assert_eq!(
        h.labels(&endpoint, 2).await,
        ["usage[Claude,Codex]".to_owned(), m],
        "the daemon lacks Codex — the wanted set goes back"
    );
    assert_eq!(
        h.events.seen(),
        [
            "status:connected:none".to_owned(),
            format!("usage:Claude:1:{MAIN_WINDOW_LABEL}:{socket}"),
        ]
    );
}

// Copied from the old suite (TRD §6-2): nudges carry no set, so whatever order they are taken in, the last subscribe
// is the last interest.
#[tokio::test]
async fn nudges_carry_no_set_so_the_last_subscribe_is_the_last_interest() {
    let h = Harness::new(live());
    let _ = relayout(h.client.usage_interest(), true, false);
    let endpoint = h.connected().await;
    assert_eq!(label(&h.command(&endpoint).await), "usage[Claude]");

    assert_eq!(
        relayout(h.client.usage_interest(), true, true),
        InterestAction::Nudge
    );
    assert_eq!(
        relayout(h.client.usage_interest(), false, true),
        InterestAction::Nudge
    );
    h.client.nudge_usage_interest();
    h.client.nudge_usage_interest();
    let m = marker(&h.client);
    assert_eq!(
        h.labels(&endpoint, 2).await,
        ["usage[Codex]".to_owned(), m],
        "the second nudge sends nothing"
    );
}

// Adapted from the old suite's `a_new_socket_sends_the_interest_before_queued_commands` (TRD §6-2) — the order against
// queued commands is pinned by `a_command_sent_as_soon_as_the_peer_is_live_…`.
#[tokio::test]
async fn a_lost_connection_clears_the_usage_cache_and_the_next_one_gets_the_current_interest_first()
{
    let h = Harness::new(live());
    let _ = relayout(h.client.usage_interest(), true, false);
    let old = h.connected().await;
    assert_eq!(label(&h.command(&old).await), "usage[Claude]");
    old.push(text(&usage_update(
        AgentBackendKind::Claude,
        1,
        &[AgentBackendKind::Claude],
    )));
    wait_for("the usage notice", || h.events.seen().len() >= 2).await;
    let cached = || {
        let pulled = h
            .client
            .usage_interest()
            .lock()
            .snapshot_for_webview(Instant::now());
        (pulled.socket_epoch, pulled.snapshots.len())
    };
    assert_eq!(cached().1, 1, "cached before the loss");

    old.close();
    wait_for("Reconnecting", || {
        h.client.state() == ConnectionState::Reconnecting
    })
    .await;
    assert_eq!(
        cached(),
        (0, 0),
        "the loss empties the cache and the socket"
    );
    assert_eq!(
        relayout(h.client.usage_interest(), true, true),
        InterestAction::Nudge
    );
    h.client.nudge_usage_interest(); // no connection open — nothing to write

    h.advance_until("the redial", || h.net.open_endpoints() > 0)
        .await;
    let (endpoint, _) = h.next_link().await;
    endpoint.push(hello());
    assert_eq!(
        label(&h.command(&endpoint).await),
        "usage[Claude,Codex]",
        "the new connection's first frame is the interest of now"
    );
    h.client.nudge_usage_interest();
    let m = marker(&h.client);
    assert_eq!(
        label(&h.command(&endpoint).await),
        m,
        "what the new connection got is what it was sent — the nudge has nothing to add"
    );
}

// Copied from the old suite (TRD §6-2).
#[tokio::test]
async fn a_refresh_writes_the_interest_before_the_refresh_frame() {
    let h = Harness::new(live());
    let _ = relayout(h.client.usage_interest(), true, false);
    let endpoint = h.connected().await;
    assert_eq!(label(&h.command(&endpoint).await), "usage[Claude]");

    let request_id = RequestId::new();
    let refresh = {
        let client = h.client.clone();
        tokio::spawn(async move {
            client
                .send_command(AgentCommand::RefreshUsageLimits {
                    vendor: AgentBackendKind::Claude,
                    request_id,
                })
                .await
        })
    };
    assert_eq!(
        h.labels(&endpoint, 2).await,
        ["usage[Claude]", "refresh:Claude"],
        "the interest ahead of the refresh — even when it is the same set"
    );
    endpoint.push(text(&AgentEvent::Ack { request_id }));
    assert!(matches!(
        joined("the refresh", refresh).await,
        Ok(AgentEvent::Ack { request_id: got }) if got == request_id
    ));
}

// Copied from the old suite (TRD §6-2): the usage state sees the new socket when `Connected` is announced — first
// connect and reconnect.
#[tokio::test]
async fn connected_is_announced_after_the_usage_state_sees_the_new_socket() {
    let h = Harness::new(live());
    let _ = relayout(h.client.usage_interest(), true, false);
    let at_connected = Arc::new(Mutex::new(Vec::new()));
    for _ in 0..2 {
        let usage = h.client.usage_interest().clone();
        let at_connected = at_connected.clone();
        h.events.then_on_connected(move || {
            let epoch = usage
                .lock()
                .snapshot_for_webview(Instant::now())
                .socket_epoch;
            at_connected.lock().unwrap().push(epoch);
        });
    }
    let old = h.connected().await;
    assert_eq!(label(&h.command(&old).await), "usage[Claude]");
    let first = usage_socket(&h.client);

    let endpoint = h.reconnected(&old).await;
    assert_eq!(label(&h.command(&endpoint).await), "usage[Claude]");
    let second = usage_socket(&h.client);
    assert_ne!(first, second);
    assert_eq!(*at_connected.lock().unwrap(), [first, second]);
}

// Copied from the old suite (TRD §6-2): a command the daemon delivers runs through the table, its usage nudge writes
// the subscribe, and a shrink goes out only after the shrink delay — the deferred nudge reaches this driver.
#[tokio::test]
async fn a_usage_slot_placed_over_the_bus_subscribes_and_a_bus_toggle_off_shrinks_later() {
    let h = Harness::new(live());
    let state = LayoutState::new();
    h.client.install_command_table(
        bus_table(&state, &h.client),
        CATALOG_VERSION,
        Arc::new(HeldView::default()),
    );
    let endpoint = h.connected().await;
    assert_eq!(
        label(&h.command(&endpoint).await),
        "register",
        "no usage slot — no subscribe"
    );

    let (view, slot) = active_slot(&state, MAIN_WINDOW_LABEL);
    let slot_args = |extra: serde_json::Value| {
        let mut args = serde_json::json!({
            "view_id": view.to_string(),
            "slot_id": slot.to_string(),
            "content": "Usage",
        });
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        args
    };
    endpoint.push(text(&bus_request(
        "layout.setSlotContent",
        slot_args(serde_json::json!({})),
    )));
    // The outcome and the nudge reach the wire on different paths — their order is no contract.
    let mut placed = h.labels(&endpoint, 2).await;
    placed.sort();
    assert_eq!(placed, ["outcome:ok", "usage[Claude,Codex]"]);

    let asked = Instant::now();
    endpoint.push(text(&bus_request(
        "layout.setSlotContent",
        slot_args(serde_json::json!({ "show_codex": false })),
    )));
    assert_eq!(
        label(&h.command(&endpoint).await),
        "outcome:ok",
        "a shrink does not go out at once"
    );
    assert_eq!(label(&h.command(&endpoint).await), "usage[Claude]");
    // The timer does not wake before its deadline — the slack below is millisecond rounding only.
    assert!(
        asked.elapsed() >= USAGE_INTEREST_SHRINK_DELAY - Duration::from_millis(20),
        "the shrink went out before the delay: {:?}",
        asked.elapsed()
    );
}

// The skip rule of a connection being dropped (`apply`'s `ToDriver` arm): a daemon command request and a usage snapshot
// of it are not handled; the same items on the next connection are.
#[tokio::test]
async fn a_daemon_command_request_or_usage_snapshot_on_a_connection_being_dropped_is_not_handled() {
    let mut b = Bench::new();
    let agent = AgentId::new_v4();
    let _ = relayout(&b.driver.usage, true, false);
    let g = Generation(1);
    let token = b.add();
    let connected = bench_connected(&b, g);
    bench_route(&mut b, token, connected);
    // A drop the actor decides — the replay Subscribe cannot be written (the bench peer is not live).
    b.driver.router.set_visible_agents_for_test(&[agent]);
    assert_eq!(
        b.driver.request_replay(agent),
        Err(REPLAY_UNSENT.to_owned())
    );
    assert_eq!(b.driver.dropping, Some(g));

    // With no command table installed, handling a request logs an error and answers it (`accept_inbound`).
    let route_both = |b: &mut Bench, generation: Generation| {
        let lines = Arc::new(Mutex::new(Vec::new()));
        crate::daemon_client::link::test_log::capture(lines.clone(), || {
            let (_, request) = command_request("no.such.command");
            let snapshot = usage_update(AgentBackendKind::Claude, 1, &[AgentBackendKind::Claude]);
            for event in [request, snapshot] {
                let item = bench_message(b, generation, ShellIn::Event(event));
                bench_route(b, token, item);
            }
        });
        let lines = lines.lock().unwrap().clone();
        lines
    };
    let lines = route_both(&mut b, g);
    assert!(
        !lines.iter().any(|line| line.starts_with("level=ERROR ")),
        "the request was not run: {lines:?}"
    );
    let skipped = lines
        .iter()
        .filter(|line| line.starts_with("level=DEBUG ") && line.contains("what=\"event\" "))
        .count();
    assert_eq!(skipped, 2, "{lines:?}");
    assert!(
        !b.events
            .seen()
            .iter()
            .any(|line| line.starts_with("usage:")),
        "{:?}",
        b.events.seen()
    );

    let lost = Incoming::Event(TransportEvent::Disconnected {
        peer: b.key().peer_id().clone(),
        generation: g,
        cause: DisconnectCause::WriteFailed,
    });
    bench_route(&mut b, token, lost);
    let next = Generation(2);
    let connected = bench_connected(&b, next);
    bench_route(&mut b, token, connected);
    let lines = route_both(&mut b, next);
    assert!(
        lines.iter().any(|line| line.starts_with("level=ERROR ")),
        "the next connection's request is run: {lines:?}"
    );
    assert!(
        b.events
            .seen()
            .iter()
            .any(|line| line.starts_with("usage:Claude:1:")),
        "{:?}",
        b.events.seen()
    );
}

// ── review round 1 (3-2 U3c) ────────────────────────────────────────────────────────────────────

/// Runs inbound command tasks and counts the ones that ran to their end — the outcome was handed over by then.
#[derive(Default)]
struct CountingSpawner {
    finished: Arc<AtomicUsize>,
}

impl TaskSpawner for CountingSpawner {
    fn spawn(&self, task: BoxedTask) {
        let finished = self.finished.clone();
        tokio::spawn(async move {
            task.await;
            finished.fetch_add(1, Ordering::SeqCst);
        });
    }
}

fn published(link: &LinkClient) -> Option<Opened> {
    lock_gate(&link.gate).opened
}

/// The generations of the fire-and-forgets pending in the gate, in order.
fn pending(gate: &Mutex<Gate>) -> Vec<Generation> {
    lock_gate(gate)
        .pending
        .iter()
        .map(|(generation, _)| *generation)
        .collect()
}

fn unsubscribe(agent_id: AgentId) -> AgentCommand {
    AgentCommand::Unsubscribe { agent_id }
}

fn list() -> AgentCommand {
    AgentCommand::ListAgents {
        request_id: RequestId::new(),
    }
}

impl Harness {
    /// Commands written on `endpoint` up to and including the one labelled `last`.
    async fn labels_until(&self, endpoint: &MemoryEndpoint, last: &str) -> Vec<String> {
        let mut got = Vec::new();
        while !got.iter().any(|label| label == last) {
            got.extend(self.labels(endpoint, 1).await);
        }
        got
    }
}

// A round trip waiting for its connection to open fails as unsent when that connection is lost meanwhile, and nothing
// of it goes out on the next connection.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)] // Holding the usage lock is the device: it stops the actor in its `LinkUp` step.
async fn a_waiting_command_fails_as_unsent_when_its_connection_is_lost() {
    let h = Harness::new(live());
    let connect = h.connect();
    let (old, _) = h.next_link().await;
    let usage = h.client.usage_interest().lock();
    old.push(hello());
    wait_for("the peer is live", || {
        h.peer_state() == Some(PeerState::Live)
    })
    .await;
    let request = {
        let client = h.client.clone();
        tokio::spawn(async move { client.send_command(list()).await })
    };
    // Lets the call reach its wait — it is not opened while the actor is held.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!request.is_finished());
    old.close();
    let ended = joined("the waiting command", request).await;
    assert!(
        matches!(&ended, Err(text) if text == UNSENT_ON_DISCONNECT),
        "{ended:?}"
    );
    drop(usage);
    joined("connect", connect).await.expect("connected");

    h.advance_until("the redial", || h.net.open_endpoints() > 0)
        .await;
    let (endpoint, _) = h.next_link().await;
    endpoint.push(hello());
    wait_for("the next connection opened", || {
        published(h.link()).is_some_and(|opened| opened.generation > Generation(1))
    })
    .await;
    let m = marker(&h.client);
    assert_eq!(h.labels(&endpoint, 1).await, [m]);
}

// F1 (ADR-0195): a round trip waiting for its connection to open is bound to it. Polled again only once the next
// connection is open and published, it fails as unsent — it does not take the next connection — and writes nothing
// there.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)] // Holding the usage lock is the device: it stops the actor in its `LinkUp` step.
async fn a_waiting_command_woken_only_after_the_next_connection_opened_fails_and_writes_nothing_there(
) {
    let h = Harness::new(live());
    let connect = h.connect();
    let (old, _) = h.next_link().await;
    let usage = h.client.usage_interest().lock();
    old.push(hello());
    wait_for("the peer is live", || {
        h.peer_state() == Some(PeerState::Live)
    })
    .await;
    let mut call = Box::pin(h.link().send_command(list()));
    let mut cx = Context::from_waker(Waker::noop());
    assert!(
        call.as_mut().poll(&mut cx).is_pending(),
        "waits for its connection to open"
    );

    old.close();
    drop(usage);
    joined("connect", connect).await.expect("connected");
    h.advance_until("the redial", || h.net.open_endpoints() > 0)
        .await;
    let (endpoint, _) = h.next_link().await;
    endpoint.push(hello());
    wait_for("the next connection opened", || {
        published(h.link()).is_some_and(|opened| opened.generation > Generation(1))
    })
    .await;

    match call.as_mut().poll(&mut cx) {
        Poll::Ready(Err(CommandFailure::Local(text))) => assert_eq!(text, UNSENT_ON_DISCONNECT),
        other => panic!("expected the unsent failure, got {other:?}"),
    }
    let m = marker(&h.client);
    assert_eq!(h.labels(&endpoint, 1).await, [m]);
}

// The gate's pending list is bounded at the caller, as the old command channel was; on `LinkUp` the pending ones go
// out in the order sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)] // Holding the usage lock is the device: it stops the actor in its `LinkUp` step.
async fn fire_and_forgets_pending_for_a_connection_not_opened_yet_are_bounded_at_the_caller() {
    let h = Harness::new(live());
    let connect = h.connect();
    let (endpoint, _) = h.next_link().await;
    let usage = h.client.usage_interest().lock();
    endpoint.push(hello());
    wait_for("the peer is live", || {
        h.peer_state() == Some(PeerState::Live)
    })
    .await;
    let fired: Vec<AgentId> = (0..=QUEUED_FIRES_MAX).map(|_| AgentId::new_v4()).collect();
    for agent_id in &fired {
        h.client.send_fire_and_forget(unsubscribe(*agent_id));
    }
    assert_eq!(
        pending(&h.link().gate).len(),
        QUEUED_FIRES_MAX,
        "the one past the bound is dropped at the caller"
    );
    drop(usage);
    joined("connect", connect).await.expect("connected");

    let m = marker(&h.client);
    let sent = h.labels_until(&endpoint, &m).await;
    let expected: Vec<String> = fired[..QUEUED_FIRES_MAX]
        .iter()
        .map(|agent| format!("unsubscribe:{agent}"))
        .chain([m])
        .collect();
    assert_eq!(sent, expected, "in the order sent, the dropped one missing");
    assert!(pending(&h.link().gate).is_empty());
}

// Finding 2: one caller's fire-and-forget sent while its connection is not opened yet, then a round trip once it is —
// written in that order (the actor writes the pending list before it sets the connection opened, under the gate).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)] // Holding the usage lock is the device: it stops the actor in its `LinkUp` step.
async fn a_fire_and_forget_sent_before_the_open_goes_out_ahead_of_the_same_callers_round_trip_after_it(
) {
    let h = Harness::new(live());
    let connect = h.connect();
    let (endpoint, _) = h.next_link().await;
    let usage = h.client.usage_interest().lock();
    endpoint.push(hello());
    wait_for("the peer is live", || {
        h.peer_state() == Some(PeerState::Live)
    })
    .await;
    let fired = AgentId::new_v4();
    h.client.send_fire_and_forget(unsubscribe(fired));
    assert_eq!(pending(&h.link().gate).len(), 1);
    drop(usage);
    joined("connect", connect).await.expect("connected");
    wait_for("opened", || published(h.link()).is_some()).await;

    let rid = RequestId::new();
    let request = {
        let client = h.client.clone();
        tokio::spawn(async move {
            client
                .send_command(AgentCommand::ListAgents { request_id: rid })
                .await
        })
    };
    assert_eq!(
        h.labels(&endpoint, 2).await,
        [format!("unsubscribe:{fired}"), "list".to_owned()]
    );
    endpoint.push(text(&AgentEvent::Ack { request_id: rid }));
    assert!(joined("the round trip", request).await.is_ok());
}

// Finding 1: a fire-and-forget left pending for a connection that was lost (a caller saw it live just before) does not
// hold anything back — the next connection opens and a round trip on it completes.
#[tokio::test]
async fn a_pending_fire_and_forget_of_a_lost_connection_does_not_hold_back_the_next_one() {
    let h = Harness::new(live());
    let old = h.connected().await;
    wait_for("opened", || published(h.link()).is_some()).await;
    let first = published(h.link()).unwrap().generation;
    old.close();
    wait_for("Reconnecting", || {
        h.client.state() == ConnectionState::Reconnecting
    })
    .await;
    assert_eq!(published(h.link()), None);
    let stale = AgentId::new_v4();
    lock_gate(&h.link().gate)
        .pending
        .push_back((first, unsubscribe(stale)));

    h.advance_until("the redial", || h.net.open_endpoints() > 0)
        .await;
    let (endpoint, _) = h.next_link().await;
    endpoint.push(hello());
    let rid = RequestId::new();
    let request = {
        let client = h.client.clone();
        tokio::spawn(async move {
            client
                .send_command(AgentCommand::ListAgents { request_id: rid })
                .await
        })
    };
    assert_eq!(
        h.labels(&endpoint, 1).await,
        ["list"],
        "the stale one is not written"
    );
    endpoint.push(text(&AgentEvent::Ack { request_id: rid }));
    assert!(joined("the round trip", request).await.is_ok());
    assert!(pending(&h.link().gate).is_empty());
}

// F3: on an opened connection one caller's fire-and-forget and round trip go out in the order made.
#[tokio::test]
async fn a_fire_and_forget_then_a_round_trip_go_out_in_that_order_on_an_opened_connection() {
    let h = Harness::new(live());
    let endpoint = h.connected().await;
    wait_for("opened", || published(h.link()).is_some()).await;
    let rid = RequestId::new();
    let fired = AgentId::new_v4();
    let client = h.client.clone();
    let caller = tokio::spawn(async move {
        client.send_fire_and_forget(AgentCommand::Unsubscribe { agent_id: fired });
        client
            .send_command(AgentCommand::ListAgents { request_id: rid })
            .await
    });
    assert_eq!(
        h.labels(&endpoint, 2).await,
        [format!("unsubscribe:{fired}"), "list".to_owned()]
    );
    endpoint.push(text(&AgentEvent::Ack { request_id: rid }));
    assert!(joined("the round trip", caller).await.is_ok());
}

// F4: `close()` followed at once by a write fails it.
#[tokio::test]
async fn a_command_right_after_a_close_fails_at_once_and_nothing_goes_out() {
    let h = Harness::new(live());
    let endpoint = h.connected().await;
    wait_for("opened", || published(h.link()).is_some()).await;

    h.client.close();
    // No await before both calls decide: on this single-thread runtime the actor has not taken the close yet, so the
    //   peer is still live and published — only the close epoch tells them apart.
    let lines = Arc::new(Mutex::new(Vec::new()));
    crate::daemon_client::link::test_log::capture(lines.clone(), || {
        h.client
            .send_fire_and_forget(unsubscribe(AgentId::new_v4()))
    });
    assert!(
        lines
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.starts_with("level=DEBUG ") && line.contains("why=\"closed\" ")),
        "dropped at the caller as closed: {:?}",
        lines.lock().unwrap()
    );
    assert!(matches!(
        h.client.send_command(list()).await,
        Err(text) if text == NOT_CONNECTED
    ));
    wait_for("Down", || h.client.state() == ConnectionState::Down).await;
    assert_eq!(endpoint.frames(), Vec::<Frame>::new());
}

// G1: a fire-and-forget sent after `close()` while the connection is live but not opened yet is dropped at the caller —
// it does not wait in the pending list for the `LinkUp` step to write it on the closing link.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)] // Holding the usage lock is the device: it stops the actor in its `LinkUp` step.
async fn a_fire_and_forget_after_a_close_while_the_connection_is_not_opened_yet_is_dropped() {
    let h = Harness::new(live());
    let connect = h.connect();
    let (endpoint, _) = h.next_link().await;
    let usage = h.client.usage_interest().lock();
    endpoint.push(hello());
    wait_for("the peer is live", || {
        h.peer_state() == Some(PeerState::Live)
    })
    .await;
    assert_eq!(published(h.link()), None, "not opened — the actor is held");

    h.client.close();
    let fired = AgentId::new_v4();
    let lines = Arc::new(Mutex::new(Vec::new()));
    crate::daemon_client::link::test_log::capture(lines.clone(), || {
        h.client.send_fire_and_forget(unsubscribe(fired))
    });
    assert!(
        lines
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.starts_with("level=DEBUG ") && line.contains("why=\"closed\" ")),
        "dropped at the caller as closed: {:?}",
        lines.lock().unwrap()
    );
    assert!(pending(&h.link().gate).is_empty());
    drop(usage);
    // The connect may resolve either way — its handle was closed right after it connected.
    let _ = joined("connect", connect).await;
    wait_for("Down", || h.client.state() == ConnectionState::Down).await;
    assert_eq!(
        endpoint.frames(),
        Vec::<Frame>::new(),
        "nothing reached the closing link"
    );
}

// F5: a daemon command request in the tail of a link the user closed is not run.
#[tokio::test]
async fn a_daemon_command_request_of_a_closed_links_tail_is_not_run() {
    let mut b = Bench::new();
    let g = Generation(1);
    let token = b.add();
    let connected = bench_connected(&b, g);
    bench_route(&mut b, token, connected);
    // With no command table installed, running a request logs an error and answers it (`accept_inbound`).
    let route_request = |b: &mut Bench| {
        let lines = Arc::new(Mutex::new(Vec::new()));
        crate::daemon_client::link::test_log::capture(lines.clone(), || {
            let (_, request) = command_request("no.such.command");
            let item = bench_message(b, g, ShellIn::Event(request));
            bench_route(b, token, item);
        });
        let errors = lines
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.starts_with("level=ERROR "))
            .count();
        errors
    };
    assert_eq!(route_request(&mut b), 1, "run while the link is the user's");
    b.driver.close();
    assert_eq!(route_request(&mut b), 0, "not run after the close");
}

// T1: a round trip whose connection does not open within the request timeout gives up with its text and a warn.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)] // Holding the usage lock is the device: it stops the actor in its `LinkUp` step.
async fn a_command_whose_connection_does_not_open_in_time_gives_up_with_a_warn() {
    let h = Harness::new(live());
    let connect = h.connect();
    let (endpoint, _) = h.next_link().await;
    let usage = h.client.usage_interest().lock();
    endpoint.push(hello());
    wait_for("the peer is live", || {
        h.peer_state() == Some(PeerState::Live)
    })
    .await;
    let after = link_policy().request_timeout;
    // The link stays live over the advance: the keepalive ping it passes only writes, and the idle limit is beyond it.
    assert!(after + Duration::from_secs(1) < link_policy().idle_timeout);

    let mut call = Box::pin(h.link().send_command(list()));
    let mut cx = Context::from_waker(Waker::noop());
    assert!(
        call.as_mut().poll(&mut cx).is_pending(),
        "waits for the open"
    );
    h.clock.advance(after + Duration::from_secs(1));
    let lines = Arc::new(Mutex::new(Vec::new()));
    let polled = crate::daemon_client::link::test_log::capture(lines.clone(), || {
        call.as_mut().poll(&mut cx)
    });
    match polled {
        Poll::Ready(Err(CommandFailure::Local(text))) => assert_eq!(
            text,
            format!(
                "{SEND_FAILED_PREFIX}: the daemon connection did not finish opening within {}s",
                after.as_secs()
            )
        ),
        other => panic!("expected the timeout, got {other:?}"),
    }
    let lines = lines.lock().unwrap().clone();
    let field = format!("after_secs={} ", after.as_secs());
    assert!(
        lines.iter().any(|line| line.starts_with("level=WARN ")
            && line.contains(&field)
            && line.contains("generation=")),
        "{lines:?}"
    );
    drop(usage);
    joined("connect", connect).await.expect("connected");
}

// The frontend command path has no timeout of its own — the policy's request timeout ends a reply that never comes,
// with the text that path always gave (golden = its former 30 s wrapper's text).
#[tokio::test]
async fn a_frontend_command_whose_reply_never_comes_fails_after_30_s_with_the_reply_timeout_text() {
    let h = Harness::new(live());
    let endpoint = h.connected().await;
    let client = h.client.clone();
    let cmd = serde_json::to_value(list()).unwrap();
    let forward =
        tokio::spawn(async move { crate::commands::agent::forward_to_daemon(&client, cmd).await });
    assert!(matches!(
        h.command(&endpoint).await,
        AgentCommand::ListAgents { .. }
    ));

    h.clock.advance(Duration::from_secs(29));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!forward.is_finished(), "not before 30 s");
    assert_eq!(h.peer_state(), Some(PeerState::Live), "the link stays up");
    h.clock.advance(Duration::from_secs(2));
    assert_eq!(
        joined("the forward", forward).await,
        Err(
            "daemon reply timeout — 연결은 살아있으나 데몬이 응답을 보내지 않음(30s 초과)"
                .to_owned()
        )
    );
}

// The actor's rules for the gate's pending list on `LinkUp`: that connection's are written (before it is set opened —
// same lock), older ones dropped, newer ones kept; a close clears the list.
#[tokio::test]
async fn pending_fire_and_forgets_follow_their_connection() {
    let mut b = Bench::new();
    let token = b.add();
    let (g1, g2, g3) = (Generation(1), Generation(2), Generation(3));
    for generation in [g1, g2, g3] {
        lock_gate(&b.driver.gate)
            .pending
            .push_back((generation, unsubscribe(AgentId::new_v4())));
    }
    let lines = Arc::new(Mutex::new(Vec::new()));
    let connected = bench_connected(&b, g2);
    crate::daemon_client::link::test_log::capture(lines.clone(), || {
        bench_route(&mut b, token, connected)
    });
    let lines = lines.lock().unwrap().clone();
    // The bench peer is not live, so the write is refused — and logged with its generation.
    let refused = format!("error={UNSENT_ON_DISCONNECT} ");
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("level=DEBUG ")
                && line.contains("generation=2 ")
                && line.contains(&refused))
            .count(),
        1,
        "g2's one was written: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("level=DEBUG ") && line.contains("dropped=1 ")),
        "g1's one was dropped: {lines:?}"
    );
    assert_eq!(pending(&b.driver.gate), [g3], "a newer one is kept");
    assert_eq!(
        lock_gate(&b.driver.gate).opened,
        Some(Opened {
            generation: g2,
            closes: 0
        })
    );

    b.driver.close();
    assert!(pending(&b.driver.gate).is_empty());
    assert_eq!(lock_gate(&b.driver.gate).opened, None);
}

// T3 — adapted from the old suite's `a_drop_before_the_reply_shows_no_box` (TRD §6-2): a registration whose link drops
// before the reply is a local failure — a warn, no box (ADR-0281); the next connection's refusal shows one.
#[tokio::test]
async fn a_drop_before_the_registration_reply_shows_no_box() {
    let h = Harness::new(live());
    let state = LayoutState::new();
    h.client.install_command_table(
        bus_table(&state, &h.client),
        CATALOG_VERSION,
        Arc::new(HeldView::default()),
    );
    let old = h.connected().await;
    assert_eq!(label(&h.command(&old).await), "register");

    let endpoint = h.reconnected(&old).await;
    let AgentCommand::RegisterCommands { request_id, .. } = h.command(&endpoint).await else {
        panic!("the next connection registers");
    };
    endpoint.push(text(&AgentEvent::Error {
        request_id: Some(request_id),
        message: REFUSAL.into(),
    }));
    wait_for("the refusal box", || !h.refusals.shown().is_empty()).await;
    assert_eq!(
        h.refusals.shown(),
        [(RefusalSite::Register, REFUSAL.to_owned())],
        "the dropped registration showed a box"
    );
}

// T4 — copied from the old suite (TRD §6-2): with no interest ⟳ still writes an empty subscribe ahead of the refresh.
#[tokio::test]
async fn a_refresh_with_no_interest_writes_an_empty_subscribe_first() {
    let h = Harness::new(live());
    let endpoint = h.connected().await;
    let request_id = RequestId::new();
    let refresh = {
        let client = h.client.clone();
        tokio::spawn(async move {
            client
                .send_command(AgentCommand::RefreshUsageLimits {
                    vendor: AgentBackendKind::Claude,
                    request_id,
                })
                .await
        })
    };
    assert_eq!(h.labels(&endpoint, 2).await, ["usage[]", "refresh:Claude"]);
    endpoint.push(text(&AgentEvent::Ack { request_id }));
    assert!(joined("the refresh", refresh).await.is_ok());
}

// ── The production assembly on a real socket ─────────────────────────────────────────────────────────────────────

/// Bounds a real-socket wait, so a regression fails instead of hanging.
async fn within<F: std::future::Future>(what: &str, wait: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(10), wait)
        .await
        .unwrap_or_else(|_| panic!("not within 10 s: {what}"))
}

/// Writes the daemon's endpoint record into `dir` — this test process's pid stands for a live daemon.
fn write_daemon_record(dir: &std::path::Path, port: u16, token: &str) {
    let path = crate::discovery::DataLayout::new(dir).daemon_file();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let record = engram_dashboard_protocol::DaemonInfo {
        pid: std::process::id(),
        host: "127.0.0.1".into(),
        port,
        token: Secret::new(token),
        protocol_version: PROTOCOL_VERSION,
        start_time: 0,
    };
    std::fs::write(path, record.to_json_pretty().unwrap()).unwrap();
}

type DaemonSocket = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

/// Plays the daemon's side of one handshake: takes the next connection, reads the Auth text, answers `Hello`.
async fn accept_daemon(listener: &tokio::net::TcpListener) -> (String, DaemonSocket) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let (stream, _) = listener.accept().await.unwrap();
    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
    let auth = match ws.next().await {
        Some(Ok(Message::Text(auth))) => auth.to_string(),
        other => panic!("the first frame must be the Auth text, got {other:?}"),
    };
    let hello = serde_json::to_string(&AgentEvent::Hello {
        protocol_version: PROTOCOL_VERSION,
        daemon_version: "0.0.0".into(),
        capabilities: None,
    })
    .unwrap();
    ws.send(Message::Text(hello.into())).await.unwrap();
    (auth, ws)
}

/// Answers the next `ListAgents` with an empty list — the commands the shell writes on open come first and are
/// skipped.
async fn answer_list(ws: &mut DaemonSocket) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    loop {
        match ws.next().await {
            Some(Ok(Message::Text(sent))) => {
                if let Ok(AgentCommand::ListAgents { request_id }) = serde_json::from_str(&sent) {
                    let reply = serde_json::to_string(&AgentEvent::AgentList {
                        request_id,
                        agents: Vec::new(),
                    })
                    .unwrap();
                    ws.send(Message::Text(reply.into())).await.unwrap();
                    return;
                }
            }
            Some(Ok(_)) => {}
            other => panic!("the link ended before the command: {other:?}"),
        }
    }
}

async fn round_trip(what: &str, client: &Arc<DaemonClient>, ws: &mut DaemonSocket) {
    let call = {
        let client = client.clone();
        tokio::spawn(async move { client.send_command(list()).await })
    };
    within(what, answer_list(ws)).await;
    let reply = within(what, call).await.expect("the call ran");
    assert!(
        matches!(reply, Ok(AgentEvent::AgentList { .. })),
        "{reply:?}"
    );
}

// Production's transport (`DaemonClient::on_local_daemon`: ws dialer · the folder's `daemon.json` · system clock) on a
// real socket: attach, one round trip, the daemon's link drops, and the retry alone reaches a daemon on a new port and
// token written to `daemon.json`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_production_transport_attaches_answers_and_follows_the_daemon_to_a_new_port() {
    const TOKEN_B: &str = "eeee4444eeee4444eeee4444eeee4444eeee4444eeee4444eeee4444eeee4444";
    let dir = tempfile::tempdir().unwrap();
    let first = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    write_daemon_record(dir.path(), first.local_addr().unwrap().port(), TOKEN);
    let events = Arc::new(RecordingEvents::default());
    let client = Arc::new(DaemonClient::on_local_daemon(
        Handle::current(),
        None,
        dir.path().to_path_buf(),
        events.clone(),
        events.clone(),
        Arc::new(RefusalAlerts::new(Arc::new(RecordingRefusals::new()))),
        Arc::new(OutputRouter::new()),
        WindowChannelRegistry::default(),
    ));

    let ensure = {
        let client = client.clone();
        tokio::spawn(async move { client.ensure().await })
    };
    let (auth, mut ws) = within("the first handshake", accept_daemon(&first)).await;
    assert!(auth.contains(TOKEN), "the record's credential");
    assert_eq!(within("ensure", ensure).await.expect("ran"), Ok(()));
    round_trip("the first round trip", &client, &mut ws).await;

    let second = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    write_daemon_record(dir.path(), second.local_addr().unwrap().port(), TOKEN_B);
    drop(ws);
    drop(first);
    let (auth, mut ws) = within("the handshake after the loss", accept_daemon(&second)).await;
    assert!(auth.contains(TOKEN_B), "the new record's credential");
    // A command needs the link live on the shell's side — the daemon's `Hello` may not be read yet.
    wait_for("the loss is shown", || {
        events
            .seen()
            .contains(&"status:reconnecting:none".to_owned())
    })
    .await;
    wait_for("connected again", || {
        client.state() == ConnectionState::Connected
    })
    .await;
    round_trip("the round trip after the loss", &client, &mut ws).await;
}

// ══════════════════════════════════════════════════════════════════════════════════
// Ported from the old driver's suite (TRD §6-2 「셸 시험 운명」 — the 「옮김」 rows this suite had not yet copied).
// ══════════════════════════════════════════════════════════════════════════════════

// A second connect on a connected client answers at once — no second search, no second dial.
#[tokio::test]
async fn connect_idempotent_when_already_connected() {
    let h = Harness::new(live());
    let _endpoint = h.connected().await;
    joined("the second connect", h.connect())
        .await
        .expect("already connected");
    assert_eq!(h.record.searches(), 1, "no second search");
    assert_eq!(h.net.dials(), 1, "no second dial");
}

/// A daemon search that panics — `connect`'s search runs on the blocking pool, so the panic ends that task only.
struct PanickingSearch;

impl EndpointSource for PanickingSearch {
    fn ensure(&self, _timeout: Duration) -> Result<Endpoint, String> {
        panic!("the daemon search panicked (test)");
    }

    fn current(&self) -> Current {
        Current::Absent
    }
}

// A search that dies is a search failure: the caller gets `Discovery`, and every window learns `down` once.
#[tokio::test]
async fn connect_discovery_join_failure_tells_every_window_down() {
    let net = MemoryNetwork::new();
    let clock = ManualClock::new();
    let events = Arc::new(RecordingEvents::default());
    let client = DaemonClient::new_on_link(
        Handle::current(),
        net.dialer(),
        Arc::new(PanickingSearch),
        clock.handle(),
        events.clone(),
        events.clone(),
        Arc::new(RecordingRefusals::new()),
    );
    let err = client.connect().await.expect_err("the search died");
    assert!(
        matches!(&err, HandshakeError::Discovery(m) if m.contains("the daemon search task failed")),
        "{err:?}"
    );
    wait_for("Down", || client.state() == ConnectionState::Down).await;
    wait_for("the down event", || !events.seen().is_empty()).await;
    // One status — `down` with band ① (TRD S21 trd-3-2-u5 R7: a never-connected handle's first failure).
    assert_eq!(events.seen(), ["status:down:not_connected"]);
}

// A webview `UsageSubscribe` never reaches the daemon (usage subscriptions are the shell's alone) — having no request
// id, it would go out as a fire-and-forget if not stopped. The marker being the next frame means nothing went out.
#[tokio::test]
async fn forward_drops_a_webview_usage_subscribe() {
    let h = Harness::new(live());
    let endpoint = h.connected().await;
    let cmd = serde_json::to_value(AgentCommand::UsageSubscribe {
        vendors: vec![AgentBackendKind::Claude],
    })
    .unwrap();
    assert_eq!(
        crate::commands::agent::forward_to_daemon(&h.client, cmd).await,
        Ok(None)
    );
    let m = marker(&h.client);
    assert_eq!(
        label(&h.command(&endpoint).await),
        m,
        "웹뷰의 사용량 구독은 소켓에 안 나간다"
    );
}

// A broken command slot from the webview (envelope · reply) fails the invoke and never reaches the link. The slots are
// raw, so the packet parse passes — unstopped, an envelope would go out as a fire-and-forget without an id (the
// frontend's pending never rejects) and a reply would be refused by the daemon without an id (the frontend never learns
// of the failure).
async fn assert_forward_refuses_and_writes_nothing(cmd: serde_json::Value) {
    let h = Harness::new(live());
    let endpoint = h.connected().await;
    let refused = crate::commands::agent::forward_to_daemon(&h.client, cmd).await;
    assert!(
        matches!(&refused, Err(m) if m.starts_with("AgentCommand 명령 칸 파싱 실패")),
        "{refused:?}"
    );
    let m = marker(&h.client);
    assert_eq!(
        label(&h.command(&endpoint).await),
        m,
        "a broken slot does not go out"
    );
}

#[tokio::test]
async fn forward_rejects_a_webview_command_with_a_broken_envelope() {
    assert_forward_refuses_and_writes_nothing(
        serde_json::json!({ "Command": { "envelope": { "name": "tab.create" } } }),
    )
    .await;
}

#[tokio::test]
async fn forward_rejects_a_webview_outcome_with_a_broken_reply() {
    assert_forward_refuses_and_writes_nothing(serde_json::json!({
        "CommandOutcome": { "reply": { "request_id": "00000000-0000-0000-0000-000000000000" } }
    }))
    .await;
}

// `get_usage_snapshot` = the whole cache (aged by the seconds since it arrived) + the current connection's epoch; no
// connection → an empty list and 0. The aging is measured as a difference of two reads — the arrival time is unknown,
// but two reads a whole number of seconds apart differ by exactly that many seconds.
#[tokio::test]
async fn get_usage_snapshot_hands_off_the_cache_with_the_socket_epoch() {
    use crate::commands::usage::{usage_snapshot_reply, UsageSnapshotReply};

    let h = Harness::new(live());
    assert_eq!(
        usage_snapshot_reply(&h.client, Instant::now()),
        UsageSnapshotReply {
            socket_epoch: 0,
            snapshots: Vec::new(),
        },
        "no connection"
    );

    let _ = relayout(h.client.usage_interest(), true, false);
    let endpoint = h.connected().await;
    assert_eq!(label(&h.command(&endpoint).await), "usage[Claude]");
    let socket = usage_socket(&h.client);
    assert_ne!(socket, 0);
    let mut update = usage_update(AgentBackendKind::Claude, 4, &[AgentBackendKind::Claude]);
    if let AgentEvent::UsageLimitsUpdated { snapshot, .. } = &mut update {
        snapshot.five_hour = Some(engram_dashboard_protocol::UsageWindow {
            used_pct: Some(12.0),
            resets_at: None,
            age_secs: 10,
            expired: false,
        });
    }
    endpoint.push(text(&update));
    wait_for("the usage notice", || h.events.seen().len() >= 2).await;

    let now = Instant::now();
    let age = |reply: &UsageSnapshotReply| {
        assert_eq!(reply.socket_epoch, socket);
        assert_eq!(reply.snapshots.len(), 1);
        assert_eq!(reply.snapshots[0].revision, 4);
        reply.snapshots[0].five_hour.as_ref().unwrap().age_secs
    };
    let early = usage_snapshot_reply(&h.client, now);
    let later = usage_snapshot_reply(&h.client, now + Duration::from_secs(100));
    assert!(age(&early) >= 10);
    assert_eq!(age(&later), age(&early) + 100, "받은 뒤 흐른 초만큼 늙는다");

    let wire = serde_json::to_value(&later).unwrap();
    let mut keys: Vec<&str> = wire
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["snapshots", "socket_epoch"],
        "웹뷰 계약 = snake_case 두 칸"
    );
}

// A daemon command that comes before a command table is installed **is answered with an error** — dropped silently,
// the sender would hold until its deadline. An assembly miss does not heal on retry (Internal).
#[tokio::test]
async fn an_envelope_arriving_before_the_table_still_gets_an_answer() {
    let h = Harness::new(live());
    let endpoint = h.connected().await;
    let (request_id, request) = command_request("tab.create");
    endpoint.push(text(&request));
    let AgentCommand::CommandOutcome { reply } = h.command(&endpoint).await else {
        panic!("the answer is a command outcome");
    };
    let reply = protocol_state::reply_from_wire(&reply).expect("a readable outcome");
    assert_eq!(reply.request_id, request_id);
    assert_eq!(
        reply.outcome.expect_err("no table is a failure").code(),
        engram_dashboard_command::ErrorCode::Internal
    );
}

const CONFLICT_REFUSAL: &str = "CONFLICT: answered by this daemon itself, so they cannot be registered: agent.rename — pick different names; this packet was not applied";
const LAST_REFUSAL: &str = "INVALID_ARGUMENT: 표지 — 마지막 연결의 거절(시험)";

impl Harness {
    /// Answers the registration the shell wrote first on `endpoint` with a daemon refusal.
    async fn refuse_registration(&self, endpoint: &MemoryEndpoint, message: &str) {
        let AgentCommand::RegisterCommands { request_id, .. } = self.command(endpoint).await else {
            panic!("the registration comes first");
        };
        endpoint.push(text(&AgentEvent::Error {
            request_id: Some(request_id),
            message: message.into(),
        }));
    }

    /// Waits until `message` reached the refusal box record — the positive signal that what came before is handled.
    async fn box_shows(&self, message: &str) {
        wait_for("the refusal reaches the box", || {
            self.refusals
                .shown()
                .iter()
                .any(|(_, shown)| shown == message)
        })
        .await;
    }
}

// One box per (site, text), across reconnects too — the client holds one filter, never one per connection.
#[tokio::test]
async fn a_conflict_refusal_shows_one_box_and_the_same_refusal_after_a_reconnect_shows_none() {
    let h = Harness::new(live());
    let state = LayoutState::new();
    h.client.install_command_table(
        bus_table(&state, &h.client),
        CATALOG_VERSION,
        Arc::new(HeldView::default()),
    );
    // The filter's holders at rest — the client and the actor's ports. A registration's reply task holds one more
    //   until it has handled the reply, so dropping back to this count is the completion barrier.
    let at_rest = Arc::strong_count(&h.client.refusals);
    let first = h.connected().await;
    h.refuse_registration(&first, CONFLICT_REFUSAL).await;
    h.box_shows(CONFLICT_REFUSAL).await;

    let second = h.reconnected(&first).await;
    h.refuse_registration(&second, CONFLICT_REFUSAL).await;
    wait_for("the repeated refusal is handled", || {
        Arc::strong_count(&h.client.refusals) == at_rest
    })
    .await;
    assert_eq!(
        h.refusals.shown(),
        [(RefusalSite::Register, CONFLICT_REFUSAL.to_owned())],
        "the repeated refusal shows no box"
    );
    let third = h.reconnected(&second).await;
    h.refuse_registration(&third, LAST_REFUSAL).await;
    h.box_shows(LAST_REFUSAL).await;
    assert_eq!(
        h.refusals.shown(),
        [
            (RefusalSite::Register, CONFLICT_REFUSAL.to_owned()),
            (RefusalSite::Register, LAST_REFUSAL.to_owned()),
        ],
        "the same (site, text) shows once across reconnects"
    );
}

// A code this shell does not know still shows once the daemon answered with it — new codes are an allowed extension
// (`engram_dashboard_command::ErrorCode` doc).
#[tokio::test]
async fn an_unknown_code_refusal_shows_one_box() {
    const UNKNOWN: &str = "BRAND_NEW_CODE: a refusal this shell has never heard of";
    let h = Harness::new(live());
    let state = LayoutState::new();
    h.client.install_command_table(
        bus_table(&state, &h.client),
        CATALOG_VERSION,
        Arc::new(HeldView::default()),
    );
    let endpoint = h.connected().await;
    h.refuse_registration(&endpoint, UNKNOWN).await;
    h.box_shows(UNKNOWN).await;
    assert_eq!(
        h.refusals.shown(),
        [(RefusalSite::Register, UNKNOWN.to_owned())]
    );
}

// ══════════════════════════════════════════════════════════════════════════════════
// The usage interest's recompute sites (TRD S21 usage-limit-slot §1-7 「재계산 자리」 · §4 셸 관심 행) — a layout write,
// a popup's destruction and the tray's hide · show recompute the interest and handle the result (a nudge at once · the
// 1.5 s shrink timer). Ported from the old suite onto this driver: the actor is built and never started, so every nudge
// and unsubscribe waits in its command channel, and the test plays the actor's usage step (`sync`) itself. The shrink
// delay runs on paused tokio time — the timer is the client's runtime sleep, not the transport clock.
// ══════════════════════════════════════════════════════════════════════════════════

/// A client whose actor is built and never started — what its callers send waits in `cmds`.
struct Idle {
    client: DaemonClient,
    cmds: tokio::sync::mpsc::UnboundedReceiver<DriverCmd>,
    _driver: Driver,
    _routable_rx: tokio::sync::mpsc::Receiver<Routable>,
}

/// The connection generation the idle client's usage state is opened on — what the actor's `LinkUp` would hand it.
const IDLE_SOCKET: u64 = 1;

/// The usage state opened on [`IDLE_SOCKET`] with an empty interest (`sent` = ∅).
fn idle_client() -> Idle {
    let mut parts = None;
    let client = DaemonClient::around_link(
        Handle::current(),
        None,
        Arc::new(RecordingEvents::default()),
        Arc::new(crate::daemon_client::NoAgentNames),
        Arc::new(RefusalAlerts::new(Arc::new(RecordingRefusals::new()))),
        Arc::new(OutputRouter::new()),
        WindowChannelRegistry::default(),
        crate::daemon_client::shutdown::inert_ports(MemoryNetwork::new().dialer()),
        |rt, ports| {
            let (link, driver, cmds, routable_rx) = LinkClient::build(
                rt,
                MemoryNetwork::new().dialer(),
                Record::new(Current::Absent),
                ManualClock::new().handle(),
                ports,
            );
            parts = Some((driver, cmds, routable_rx));
            link
        },
    );
    let (driver, cmds, routable_rx) = parts.expect("the link was built");
    assert_eq!(
        client.usage_interest().lock().on_socket_open(IDLE_SOCKET),
        None
    );
    Idle {
        client,
        cmds,
        _driver: driver,
        _routable_rx: routable_rx,
    }
}

fn kinds(cmds: &[DriverCmd]) -> Vec<&'static str> {
    cmds.iter()
        .map(|cmd| match cmd {
            DriverCmd::IsLive { .. } => "IsLive",
            DriverCmd::Add { .. } => "Add",
            DriverCmd::Close { .. } => "Close",
            DriverCmd::Unsubscribe { .. } => "Unsubscribe",
            DriverCmd::RequestReplay { .. } => "RequestReplay",
            DriverCmd::UsageNudge => "UsageNudge",
            DriverCmd::HoldForBootStart => "HoldForBootStart",
            DriverCmd::BoundBootStart { .. } => "BoundBootStart",
            DriverCmd::EndBootStartHold { .. } => "EndBootStartHold",
            DriverCmd::CurrentHandle { .. } => "CurrentHandle",
            DriverCmd::BootStartHeld { .. } => "BootStartHeld",
        })
        .collect()
}

/// The nudges waiting for the actor. Any other command breaks this section's premise.
fn nudges(rx: &mut tokio::sync::mpsc::UnboundedReceiver<DriverCmd>) -> usize {
    let mut count = 0;
    while let Ok(cmd) = rx.try_recv() {
        assert!(
            matches!(cmd, DriverCmd::UsageNudge),
            "not a nudge: {:?}",
            kinds(&[cmd])
        );
        count += 1;
    }
    count
}

/// What the actor does when it takes a nudge — the set it would write on the connection.
fn sync_as_the_actor(client: &DaemonClient, socket: u64) -> Option<BTreeSet<AgentBackendKind>> {
    client.usage_interest().lock().sync(socket, false)
}

fn set_usage_slot(
    state: &LayoutState,
    client: &DaemonClient,
    window: &str,
    show_claude: bool,
    show_codex: bool,
) {
    let (view, slot) = active_slot(state, window);
    apply::set_usage_slot(
        state,
        &RouterSubs {
            router: &client.router,
            client,
        },
        &NoLayoutEvents,
        view,
        slot,
        Some(show_claude),
        Some(show_codex),
    )
    .expect("the usage slot is placed");
}

/// main's active tab = a usage slot with Claude only, and that interest already sent (`sent` = {Claude}).
fn subscribed_to_claude(
    state: &LayoutState,
    client: &DaemonClient,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<DriverCmd>,
    socket: u64,
) {
    set_usage_slot(state, client, MAIN_WINDOW_LABEL, true, false);
    assert_eq!(nudges(rx), 1, "a grown interest nudges at once");
    assert_eq!(
        sync_as_the_actor(client, socket),
        Some(BTreeSet::from([AgentBackendKind::Claude]))
    );
}

/// Room for a timer task woken on the paused clock to run — `yield_now` turns the driver once before waking.
async fn settle() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

// ── One layout write → one recompute (`RouterSubs::resync`) ─────────────────────────────
// Interest that grew while `sent` stays empty (the connection task does not take it out) issues one nudge per recompute — the nudge count is
// the recompute count.
#[tokio::test]
async fn one_layout_write_recomputes_the_usage_interest_once() {
    let Idle {
        client,
        cmds: mut rx,
        _driver,
        _routable_rx,
    } = idle_client();
    let state = LayoutState::new();
    set_usage_slot(&state, &client, MAIN_WINDOW_LABEL, true, true);
    assert_eq!(nudges(&mut rx), 1);
}

// ── Tray hide → interest shrunk after the deadline · show → interest grown at once ─────────────────────────────
// Drives what the two tray functions do per window (`for_each_ui_window` + the production implementation `LayoutUsageVisibility`) as is —
// only fetching the window list from the OS and the OS window calls are missing.
#[tokio::test(start_paused = true)]
async fn tray_hide_shrinks_after_the_delay_and_show_grows_at_once() {
    let Idle {
        client,
        cmds: mut rx,
        _driver,
        _routable_rx,
    } = idle_client();
    let socket = IDLE_SOCKET;
    let state = LayoutState::new();
    subscribed_to_claude(&state, &client, &mut rx, socket);
    let tray = LayoutUsageVisibility {
        state: &state,
        client: &client,
    };

    for_each_ui_window(
        [MAIN_WINDOW_LABEL],
        false,
        &tray,
        |_| false,
        |_| {},
        |_| true,
    );
    settle().await;
    assert_eq!(nudges(&mut rx), 0, "줄임은 곧바로 안 나간다");
    tokio::time::advance(USAGE_INTEREST_SHRINK_DELAY - Duration::from_millis(100)).await;
    settle().await;
    assert_eq!(nudges(&mut rx), 0, "기한 전");
    tokio::time::advance(Duration::from_millis(200)).await;
    settle().await;
    assert_eq!(nudges(&mut rx), 1, "기한에 넛지 한 장");
    assert_eq!(
        sync_as_the_actor(&client, socket),
        Some(BTreeSet::new()),
        "그 넛지가 보내는 것 = 줄인 관심"
    );

    for_each_ui_window(
        [MAIN_WINDOW_LABEL],
        true,
        &tray,
        |_| false,
        |_| {},
        |_| true,
    );
    assert_eq!(nudges(&mut rx), 1, "늘어남은 기다리지 않는다");
    assert_eq!(
        sync_as_the_actor(&client, socket),
        Some(BTreeSet::from([AgentBackendKind::Claude]))
    );
}

// ── Shown again within the deadline, the shrink backs off — a timer that wakes late emits nothing ─────────────────
#[tokio::test(start_paused = true)]
async fn showing_again_within_the_delay_cancels_the_shrink() {
    let Idle {
        client,
        cmds: mut rx,
        _driver,
        _routable_rx,
    } = idle_client();
    let socket = IDLE_SOCKET;
    let state = LayoutState::new();
    subscribed_to_claude(&state, &client, &mut rx, socket);
    let tray = LayoutUsageVisibility {
        state: &state,
        client: &client,
    };

    tray.set_visible(MAIN_WINDOW_LABEL, false);
    tokio::time::advance(USAGE_INTEREST_SHRINK_DELAY / 2).await;
    settle().await;
    tray.set_visible(MAIN_WINDOW_LABEL, true);
    assert_eq!(nudges(&mut rx), 0, "`sent` 로 돌아왔다 — 보낼 것이 없다");

    tokio::time::advance(USAGE_INTEREST_SHRINK_DELAY * 2).await;
    settle().await;
    assert_eq!(nudges(&mut rx), 0, "물러진 줄임의 타이머가 넛지를 냈다");
    assert_eq!(sync_as_the_actor(&client, socket), None);
}

// ── Popup destroyed → that window's interest drops out after the deadline ─────────────────────────────────────────
#[tokio::test(start_paused = true)]
async fn a_destroyed_popup_leaves_the_interest_after_the_delay() {
    let Idle {
        client,
        cmds: mut rx,
        _driver,
        _routable_rx,
    } = idle_client();
    let socket = IDLE_SOCKET;
    let state = LayoutState::new();
    state.0.lock().unwrap().create_window(POPUP).unwrap();
    set_usage_slot(&state, &client, POPUP, true, false);
    assert_eq!(nudges(&mut rx), 1);
    assert_eq!(
        sync_as_the_actor(&client, socket),
        Some(BTreeSet::from([AgentBackendKind::Claude]))
    );

    assert_eq!(
        drop_window_in_model(POPUP, DropCause::Destroyed, &state, &client.router, &client),
        Some(1)
    );
    settle().await;
    assert_eq!(nudges(&mut rx), 0, "줄임은 곧바로 안 나간다");
    tokio::time::advance(USAGE_INTEREST_SHRINK_DELAY + Duration::from_millis(100)).await;
    settle().await;
    assert_eq!(nudges(&mut rx), 1);
    assert_eq!(sync_as_the_actor(&client, socket), Some(BTreeSet::new()));
}

// ── A popup destroyed forgets the hidden mark even if the model did not hold that window ─────────────────────────────────
// Moving a slot to a new window opens the OS window first and adds the model window later — if the tray hides and the window closes in that gap, the model
// does not have that window. If a window with the same label is stood up in a new model with no tombstone (tests only — production labels are not reused, and a label with a tombstone is
// not accepted by the model), the forgotten mark counts as shown.
#[tokio::test]
async fn a_destroyed_popup_forgets_its_hidden_mark_even_when_the_model_never_held_it() {
    let Idle {
        client,
        cmds: mut rx,
        _driver,
        _routable_rx,
    } = idle_client();
    let state = LayoutState::new();
    LayoutUsageVisibility {
        state: &state,
        client: &client,
    }
    .set_visible(POPUP, false);
    assert_eq!(
        drop_window_in_model(POPUP, DropCause::Destroyed, &state, &client.router, &client),
        Some(0)
    );
    assert_eq!(nudges(&mut rx), 0);

    let state = LayoutState::new();
    state.0.lock().unwrap().create_window(POPUP).unwrap();
    set_usage_slot(&state, &client, POPUP, true, false);
    assert_eq!(
        nudges(&mut rx),
        1,
        "숨김 표시가 남아 그 창을 숨은 창으로 셌다"
    );
}

// ── A popout close request releases once the subscription of an agent that was visible only in that window, and the destroy cleanup that follows sends nothing ─────────
// The close request removes the model first (TRD S21-slot-docking 5.8th edition §1-2 「닫히는 창」), so the release goes out then, and the destroy cleanup has nothing to emit even if it
// does the same share again.
#[tokio::test]
async fn a_popout_close_request_unsubscribes_once_and_the_later_destroyed_sends_nothing() {
    let Idle {
        client,
        cmds: mut rx,
        _driver,
        _routable_rx,
    } = idle_client();
    let state = LayoutState::new();
    let agent = uuid::Uuid::new_v4();
    state.0.lock().unwrap().create_window(POPUP).unwrap();
    let (view, slot) = active_slot(&state, POPUP);
    apply::assign_agent(
        &state,
        &RouterSubs {
            router: &client.router,
            client: &client,
        },
        &NoLayoutEvents,
        view,
        slot,
        agent.to_string(),
    )
    .expect("팝아웃 슬롯에 배정");
    assert!(rx.try_recv().is_err(), "배정은 해제를 내지 않는다");

    close_requested_popup_window(POPUP, &state, &client.router, &client);
    let mut sent = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        sent.push(cmd);
    }
    assert!(
        matches!(sent.as_slice(), [DriverCmd::Unsubscribe { agent_id }] if *agent_id == agent),
        "{:?}",
        kinds(&sent)
    );

    assert_eq!(
        drop_window_in_model(POPUP, DropCause::Destroyed, &state, &client.router, &client),
        Some(0)
    );
    assert!(rx.try_recv().is_err(), "소멸 정리가 명령을 냈다");
}

// ── A destroy that came without a close request (a popout left in the model) — the one model share the `Destroyed` cleanup of `lib.rs` calls does all of: subscription release ·
//   tombstone · usage interest shrink · window number release (TRD S26-window-chrome §4-1) ─────────────────────────
#[tokio::test(start_paused = true)]
async fn a_destroyed_popout_still_in_the_model_is_cleaned_up_in_one_pass_and_frees_its_number() {
    let Idle {
        client,
        cmds: mut rx,
        _driver,
        _routable_rx,
    } = idle_client();
    let socket = IDLE_SOCKET;
    let state = LayoutState::new();
    let subs = RouterSubs {
        router: &client.router,
        client: &client,
    };
    let agent = uuid::Uuid::new_v4();
    state.0.lock().unwrap().create_window(POPUP).unwrap();
    let (view, slot) = active_slot(&state, POPUP);
    apply::assign_agent(
        &state,
        &subs,
        &NoLayoutEvents,
        view,
        slot,
        agent.to_string(),
    )
    .expect("팝아웃 슬롯에 배정");
    apply::create_tab(&state, &subs, &NoLayoutEvents, POPUP, None).expect("팝아웃의 둘째 탭");
    set_usage_slot(&state, &client, POPUP, true, false);
    assert_eq!(nudges(&mut rx), 1);
    assert_eq!(
        sync_as_the_actor(&client, socket),
        Some(BTreeSet::from([AgentBackendKind::Claude]))
    );

    assert_eq!(
        drop_window_in_model(POPUP, DropCause::Destroyed, &state, &client.router, &client),
        Some(2)
    );
    let mut sent = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        sent.push(cmd);
    }
    assert!(
        matches!(sent.as_slice(), [DriverCmd::Unsubscribe { agent_id }] if *agent_id == agent),
        "{:?}",
        kinds(&sent)
    );
    tokio::time::advance(USAGE_INTEREST_SHRINK_DELAY + Duration::from_millis(100)).await;
    settle().await;
    assert_eq!(nudges(&mut rx), 1);
    assert_eq!(sync_as_the_actor(&client, socket), Some(BTreeSet::new()));

    let mut mgr = state.0.lock().unwrap();
    assert!(!mgr.windows.contains_key(POPUP));
    assert!(mgr.closing_numbers().is_empty());
    assert_eq!(
        mgr.create_window(POPUP),
        Err(crate::layout::LayoutError::WindowClosed(POPUP.to_string())),
        "묘비가 섰다"
    );
    mgr.create_window("slot-popup-8").unwrap();
    assert_eq!(
        mgr.windows["slot-popup-8"].number, 2,
        "풀린 번호를 다음 새 창이 받는다"
    );
    mgr.assert_number_invariants();
}

// ── A window whose tray hide failed stays a visible window — the interest does not shrink ─────────────────────────
#[tokio::test(start_paused = true)]
async fn a_failed_tray_hide_keeps_the_window_in_the_interest() {
    let Idle {
        client,
        cmds: mut rx,
        _driver,
        _routable_rx,
    } = idle_client();
    let socket = IDLE_SOCKET;
    let state = LayoutState::new();
    subscribed_to_claude(&state, &client, &mut rx, socket);
    let tray = LayoutUsageVisibility {
        state: &state,
        client: &client,
    };

    for_each_ui_window(
        [MAIN_WINDOW_LABEL],
        false,
        &tray,
        |_| false,
        |_| {},
        |_| false,
    );
    tokio::time::advance(USAGE_INTEREST_SHRINK_DELAY * 2).await;
    settle().await;
    assert_eq!(nudges(&mut rx), 0, "숨기지 못한 창을 관심에서 뺐다");
    assert_eq!(sync_as_the_actor(&client, socket), None);
}

// ── Moving a usage slot to a new window sends nothing to the daemon (TRD §1-9 #9) ─────────────────────────────
// Every step of the move recomputes — phase A (detaching to a temporary view) leaves the original slot as is, and phase C closes the original **inside the same
// lock** that attached it to the new window. So at no recompute does the interest leave `sent`, so not even a shrink stands. The broadcast after
// the move goes to the new window.
#[tokio::test(start_paused = true)]
async fn moving_a_usage_slot_to_a_new_window_sends_nothing() {
    let Idle {
        client,
        cmds: mut rx,
        _driver,
        _routable_rx,
    } = idle_client();
    let socket = IDLE_SOCKET;
    let state = LayoutState::new();
    subscribed_to_claude(&state, &client, &mut rx, socket);
    let (view, slot) = active_slot(&state, MAIN_WINDOW_LABEL);

    let moved = apply::move_slot_to_window(
        &state,
        &RouterSubs {
            router: &client.router,
            client: &client,
        },
        &NoLayoutEvents,
        &OpenWindows,
        &PopupCounter::default(),
        view,
        slot,
        None,
    )
    .expect("새 창으로 옮기기");
    assert_ne!(moved.window, MAIN_WINDOW_LABEL);
    assert_eq!(nudges(&mut rx), 0, "옮기는 동안 넛지가 났다");
    tokio::time::advance(USAGE_INTEREST_SHRINK_DELAY * 2).await;
    settle().await;
    assert_eq!(nudges(&mut rx), 0, "옮긴 뒤 줄임 타이머가 넛지를 냈다");
    assert_eq!(sync_as_the_actor(&client, socket), None);

    let outcome = client.usage_interest().lock().on_snapshot(
        socket,
        &UsageLimitSnapshot {
            vendor: AgentBackendKind::Claude,
            account_key: "default".into(),
            five_hour: None,
            weekly: None,
            model_scoped: Vec::new(),
            plan: None,
            in_flight: false,
            state: UsageVendorState::Ready,
            revision: 1,
        },
        &[AgentBackendKind::Claude],
        Instant::now(),
    );
    assert_eq!(outcome.labels, [moved.window], "방송은 새 창으로");
    assert_eq!(outcome.resend, None);
}

// ══════════════════════════════════════════════════════════════════════════════════
// Backoff rows of the old suite rewritten on the manual clock and the memory network (TRD §6-2 「다시 씀」).
// ══════════════════════════════════════════════════════════════════════════════════

// From `reconnect_close_during_backoff_no_zombie`: a close while the link waits in backoff after a loss ends it — Down,
// and no dial however long the clock runs.
#[tokio::test]
async fn a_close_during_the_reconnect_backoff_shows_down_and_never_dials_again() {
    let h = Harness::new(live());
    let old = h.connected().await;
    old.close();
    wait_for("backoff", || h.peer_state() == Some(PeerState::Backoff)).await;

    h.client.close();
    wait_for("Down", || h.client.state() == ConnectionState::Down).await;
    for _ in 0..10 {
        h.clock.advance(Duration::from_secs(60));
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(h.net.dials(), 1, "a closed link does not redial");
    assert_eq!(
        h.states(),
        ["state:connected", "state:reconnecting", "state:down"]
    );
}

// From `reconnect_connect_during_backoff_no_hijack`: a connect while the link waits in backoff connects on one new link,
// and the handle it replaced dials nothing more.
#[tokio::test]
async fn a_connect_during_the_reconnect_backoff_connects_on_one_new_link() {
    let h = Harness::new(live());
    let old = h.connected().await;
    old.close();
    wait_for("backoff", || h.peer_state() == Some(PeerState::Backoff)).await;

    let task = h.connect();
    let (endpoint, _) = h.next_link().await;
    endpoint.push(hello());
    joined("connect", task).await.expect("connected");
    assert_eq!(h.client.state(), ConnectionState::Connected);
    // Past the longest backoff step — a replaced handle that still ran would dial now.
    h.clock.advance(Duration::from_secs(11));
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(
        h.net.dials(),
        2,
        "one dial for the connect, none from the old handle"
    );
    assert_eq!(h.client.state(), ConnectionState::Connected);
}

// ADR-0195 — the shell adapter row of the fate table: a command sent while the link waits in backoff after a loss fails
// at once as not connected, and nothing of it reaches the next connection.
#[tokio::test]
async fn a_command_during_the_reconnect_backoff_fails_at_once_and_never_reaches_the_next_link() {
    let h = Harness::new(live());
    let old = h.connected().await;
    old.close();
    wait_for("backoff", || h.peer_state() == Some(PeerState::Backoff)).await;

    let rid = RequestId::new();
    let sent = tokio::time::timeout(
        Duration::from_secs(10),
        h.client
            .send_command(AgentCommand::ListAgents { request_id: rid }),
    )
    .await
    .expect("fails at once, never waits");
    assert_eq!(sent.expect_err("not live"), NOT_CONNECTED);

    h.advance_until("the redial", || h.net.open_endpoints() > 0)
        .await;
    let (endpoint, _) = h.next_link().await;
    endpoint.push(hello());
    wait_for("connected again", || {
        h.client.state() == ConnectionState::Connected
    })
    .await;
    let m = marker(&h.client);
    assert_eq!(
        label(&h.command(&endpoint).await),
        m,
        "the failed command never reaches the next connection"
    );
}

// ── bands on the client (TRD S21 trd-3-2-u5 §4) ───────────────────────────────────────────────

const BAND_ONE: Option<Band> = Some(Band::NotConnected { refused: false });

impl Harness {
    /// A client that holds its bands for the boot start, as the production constructor does.
    fn held(now: Current) -> Self {
        let h = Self::new(now);
        h.client.link.hold_bands_for_boot_start();
        h
    }

    /// The boot hold is on, as the actor sees it — every command sent before is applied by then.
    async fn boot_start_held(&self) -> bool {
        self.link().boot_start_held().await
    }
}

/// A few real milliseconds for the actor — for "this did not happen" checks.
async fn let_the_actor_run() {
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

// Finding A: the shell showed `reconnecting` and nothing more once the reconnect failed.
#[tokio::test]
async fn a_daemon_that_dies_while_connected_shows_band_one_after_the_first_failed_reconnect() {
    let h = Harness::new(live());
    let endpoint = h.connected().await;
    h.record.set(Current::Absent);
    endpoint.close();
    wait_for("reconnecting", || {
        h.client.state() == ConnectionState::Reconnecting
    })
    .await;
    assert_eq!(
        h.client.status().band,
        None,
        "no band before a reconnect failed"
    );
    h.advance_until("the first failed reconnect", || {
        h.client.status().band.is_some()
    })
    .await;
    assert_eq!(
        h.client.status(),
        ConnectionStatus {
            state: ConnectionState::Reconnecting,
            band: BAND_ONE,
        }
    );
    assert_eq!(
        h.statuses(),
        [
            "status:connected:none",
            "status:reconnecting:none",
            "status:reconnecting:not_connected",
        ],
        "the band came without a new state word"
    );
}

#[tokio::test]
async fn a_refusal_tells_the_token_from_the_version_in_the_band_and_in_the_answer() {
    // The daemon refuses the handshake — the token text, ① refused.
    let h = Harness::new(live());
    let task = h.connect();
    let (endpoint, _) = h.next_link().await;
    endpoint.push(text(&AgentEvent::Error {
        request_id: None,
        message: "auth failed".into(),
    }));
    assert_eq!(
        joined("connect", task).await,
        Err(HandshakeError::AuthRejected(AUTH_REFUSED.to_owned()))
    );
    wait_for("the band", || h.client.status().band.is_some()).await;
    assert_eq!(
        h.client.status(),
        ConnectionStatus {
            state: ConnectionState::Down,
            band: Some(Band::NotConnected { refused: true }),
        }
    );

    // The dial refused the daemon's version — the version text, ②. Finding B's shell half: the attach-only ensure a
    //   failed boot start still leaves (the boot pulls' `ensureReady`) surfaces it.
    let h = Harness::new(Current::VersionMismatch {
        daemon: PROTOCOL_VERSION - 1,
        pid: 9,
        start_time: 0,
    });
    assert_eq!(
        joined("ensure", h.ensure()).await,
        Err(HandshakeError::AuthRejected(VERSION_REFUSED.to_owned()))
    );
    wait_for("the band", || h.client.status().band.is_some()).await;
    assert!(
        matches!(
            h.client.status(),
            ConnectionStatus {
                state: ConnectionState::Down,
                band: Some(Band::Version { .. }),
            }
        ),
        "{:?}",
        h.client.status()
    );
}

#[tokio::test]
async fn an_explicit_close_takes_the_band_away() {
    let h = Harness::new(Current::Absent);
    assert!(joined("attach", h.ensure()).await.is_err());
    wait_for("band ①", || h.client.status().band == BAND_ONE).await;
    h.client.close();
    wait_for("no band", || h.client.status().band.is_none()).await;
    assert_eq!(h.client.state(), ConnectionState::Down);
}

// R20 — a cold boot with a spare: the spare's attach fails before main's start answers.
#[tokio::test]
async fn a_cold_boot_shows_no_band_until_the_main_windows_start_answers() {
    let h = Harness::held(Current::Absent);
    assert!(joined("the spare's attach", h.ensure()).await.is_err());
    wait_for("down", || h.states() == ["state:down"]).await;
    assert!(h.boot_start_held().await);
    assert_eq!(h.statuses(), ["status:down:none"]);

    main_window(&h).start_answered::<()>(&Err("no daemon exe".into()));
    wait_for("band ①", || h.client.status().band == BAND_ONE).await;
    assert!(!h.boot_start_held().await);
}

#[tokio::test]
async fn a_refusal_while_the_boot_hold_is_on_shows_when_it_ends() {
    let h = Harness::held(Current::VersionMismatch {
        daemon: PROTOCOL_VERSION - 1,
        pid: 9,
        start_time: 0,
    });
    assert!(joined("attach", h.ensure()).await.is_err());
    wait_for("down", || h.states() == ["state:down"]).await;
    assert!(h.boot_start_held().await);
    assert_eq!(h.client.status().band, None);
    main_window(&h).start_answered::<()>(&Err("version mismatch".into()));
    wait_for("band ②", || {
        matches!(h.client.status().band, Some(Band::Version { .. }))
    })
    .await;
}

fn main_window(h: &Harness) -> BootStartWatch<'_> {
    BootStartWatch::of(MAIN_WINDOW_LABEL, Some(&h.client))
}

fn popout(h: &Harness) -> BootStartWatch<'_> {
    BootStartWatch::of(POPUP, Some(&h.client))
}

// M1 — only the main window's boot start answers for the cold boot: a popout's (or the spare's) failed connect keeps
//   the hold; main's answer ends it.
#[tokio::test]
async fn a_non_main_connect_failure_keeps_the_boot_hold_and_mains_connect_answer_ends_it() {
    let h = Harness::held(Current::Absent);
    assert!(joined("attach", h.ensure()).await.is_err());
    h.record.fail_search("no daemon exe");
    assert!(matches!(
        joined("the popout's connect", h.connect()).await,
        Err(HandshakeError::Discovery(_))
    ));
    popout(&h).connect_answered();
    assert!(
        h.boot_start_held().await,
        "a popout's connect is not the boot start"
    );
    let_the_actor_run().await;
    assert_eq!(h.client.status().band, None);

    assert!(joined("main's connect", h.connect()).await.is_err());
    main_window(&h).connect_answered();
    wait_for("band ①", || h.client.status().band == BAND_ONE).await;
    assert!(!h.boot_start_held().await);
}

// R-d — the window gate of each boot-start step.
#[tokio::test]
async fn only_the_main_windows_boot_start_steps_reach_the_hold() {
    let h = Harness::held(Current::Absent);
    // A popout's search start does not bound the hold — only the 60 s cap does.
    popout(&h).search_began(Duration::from_secs(5));
    assert!(h.boot_start_held().await);
    h.clock.advance(Duration::from_secs(30));
    let_the_actor_run().await;
    assert!(h.boot_start_held().await, "no bound armed by a popout");
    // A popout's failure, and main's success, end nothing.
    popout(&h).start_answered::<()>(&Err("x".into()));
    main_window(&h).start_answered(&Ok(()));
    assert!(h.boot_start_held().await);
    // No client — nothing to tell, nothing breaks.
    BootStartWatch::of(MAIN_WINDOW_LABEL, None).connect_answered();
    // Main's search start bounds it.
    main_window(&h).search_began(Duration::from_secs(5));
    assert!(h.boot_start_held().await, "armed");
    h.clock.advance(Duration::from_secs(5) + BOOT_START_MARGIN);
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.boot_start_held().await {
        assert!(Instant::now() < deadline, "the bound did not end the hold");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

// R-h — the first `Connected` of any handle ends the cold boot: a later handle that never connected is not held.
#[tokio::test]
async fn a_link_that_connected_ends_the_boot_hold() {
    let h = Harness::held(live());
    let task = h.ensure();
    let (endpoint, _) = h.next_link().await;
    endpoint.push(hello());
    joined("attach", task).await.expect("attached");
    assert!(!h.boot_start_held().await, "the cold boot is over");

    h.client.close();
    wait_for("down", || h.client.state() == ConnectionState::Down).await;
    drop(endpoint);
    h.record.set(Current::Absent);
    assert!(joined("a re-add", h.ensure()).await.is_err());
    wait_for("band ①", || h.client.status().band == BAND_ONE).await;
}

#[tokio::test]
async fn the_boot_hold_ends_by_itself_60_s_after_construction() {
    let h = Harness::held(Current::Absent);
    assert!(joined("attach", h.ensure()).await.is_err());
    wait_for("down", || h.states() == ["state:down"]).await;
    h.clock.advance(Duration::from_secs(59));
    let_the_actor_run().await;
    assert_eq!(h.client.status().band, None, "not yet");
    h.clock.advance(Duration::from_secs(1));
    wait_for("band ①", || h.client.status().band == BAND_ONE).await;
}

// R-g (TRD R24) — main's search bound replaces the cap: a search that begins at +55 s keeps its whole bound past +60 s.
#[tokio::test]
async fn a_main_search_that_begins_late_bounds_the_hold_from_its_own_start_past_the_cap() {
    let h = Harness::held(Current::Absent);
    assert!(joined("attach", h.ensure()).await.is_err());
    wait_for("down", || h.states() == ["state:down"]).await;
    h.clock.advance(Duration::from_secs(55));
    let_the_actor_run().await;
    assert_eq!(h.client.status().band, None, "no band before main's search");

    main_window(&h).search_began(Duration::from_secs(5));
    assert!(h.boot_start_held().await, "the bound is armed");
    // +61 s: past the cap, within the search and its margin (+55 + 5 + 5 = +65).
    h.clock.advance(Duration::from_secs(6));
    let_the_actor_run().await;
    assert!(h.boot_start_held().await, "the cap no longer applies");
    assert_eq!(h.client.status().band, None);
    h.clock.advance(Duration::from_secs(4));
    wait_for("band ①", || h.client.status().band == BAND_ONE).await;
}
