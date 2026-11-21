//! Isolation harness — daemon-only WS E2E (phase 2 step 6).
//!
//! Without the frontend, it starts the daemon's WS server really in-process (`start_test_server`) and this test acts as the WS client, verifying auth → subscribe → binary frame decode → command send (ADR-0012 — a module runs alone in an isolation harness).
//!
//! Isolation: bind is 127.0.0.1:0 (OS-assigned port — no parallel collisions), the store is in-memory, each test starts an independent server and shuts it down at the end (kills all agents — no zombie PTYs). Every await has a timeout guard.
//!
//! Real-process cases stay separate (no concealment): what in-process tests cannot cover (daemon .exe kill → PTY child Job cleanup, single-instance folder lock, stale daemon.json discovery) needs a real OS process/Job — `#[ignore]` tests at the bottom with manual run instructions. Default `cargo test` runs only the fast in-process cases.

use std::sync::Arc;
use std::time::Duration;

use engram_dashboard_agent::profile::{AgentCommand, AgentOutputFormat, AgentProfile, SpawnMode};
use engram_dashboard_agent::types::InputOrigin;
use engram_dashboard_daemon::{
    start_test_server, start_test_server_with_keepalive, KeepaliveConfig, TestServerHandle,
};
use engram_dashboard_protocol::handshake::AuthFrame;
use engram_dashboard_protocol::{
    decode_frame, AgentBackendKind as WireBackendKind, AgentCommand as WireCommand, AgentEvent,
    AgentOutputFormat as WireOutputFormat, RequestId, Secret, SubscribeAction, PROTOCOL_VERSION,
};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const NET_TIMEOUT: Duration = Duration::from_secs(10);

// ── client helpers ────────────────────────────────────────────────────────────────

struct Client {
    ws: Ws,
}

// AgentEvent holds an AgentProfile (past clippy's 200B threshold after adding failed_reason), so the variant size gap is large; as a test helper only the lint is allowed instead of Box indirection (a behavior change).
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
enum Incoming {
    Event(AgentEvent),
    /// Decoded (agent_id, epoch, seq, payload). epoch is for debugging visibility (unused by the current assertions).
    #[allow(dead_code)]
    Frame(Uuid, u32, u64, Vec<u8>),
}

impl Client {
    async fn connect_and_auth(port: u16, token: &str) -> Self {
        let url = format!("ws://127.0.0.1:{port}");
        let (ws, _resp) = tokio::time::timeout(NET_TIMEOUT, connect_async(url))
            .await
            .expect("connect timeout")
            .expect("connect failed");
        let mut client = Self { ws };
        client.send_auth(token).await;
        client
    }

    /// A handshake is not a command (ADR-0129 0-4) — a separate frame type (protocol `handshake`) that cannot ride `send` (commands only). The first frame (normal auth) and the second (case21's protocol violation) must be the same bytes, so both share this helper.
    async fn send_auth(&mut self, token: &str) {
        let auth = AuthFrame::Auth {
            token: Secret::new(token),
            protocol_version: PROTOCOL_VERSION,
        };
        let text = serde_json::to_string(&auth).unwrap();
        tokio::time::timeout(NET_TIMEOUT, self.ws.send(Message::Text(text.into())))
            .await
            .expect("auth send timeout")
            .expect("auth send failed");
    }

    async fn connect_raw(port: u16) -> Self {
        let url = format!("ws://127.0.0.1:{port}");
        let (ws, _resp) = tokio::time::timeout(NET_TIMEOUT, connect_async(url))
            .await
            .expect("connect timeout")
            .expect("connect failed");
        Self { ws }
    }

    async fn send(&mut self, cmd: &WireCommand) {
        let text = serde_json::to_string(cmd).unwrap();
        tokio::time::timeout(NET_TIMEOUT, self.ws.send(Message::Text(text.into())))
            .await
            .expect("send timeout")
            .expect("send failed");
    }

    async fn next(&mut self) -> Option<Incoming> {
        loop {
            let item = tokio::time::timeout(NET_TIMEOUT, self.ws.next())
                .await
                .expect("recv timeout")?;
            match item {
                Ok(Message::Text(t)) => {
                    let ev: AgentEvent =
                        serde_json::from_str(&t).expect("control JSON parse failed");
                    return Some(Incoming::Event(ev));
                }
                Ok(Message::Binary(b)) => {
                    let f = decode_frame(&b).expect("binary frame decode failed");
                    return Some(Incoming::Frame(
                        f.agent_id,
                        f.epoch,
                        f.seq,
                        f.payload.to_vec(),
                    ));
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(Message::Frame(_)) => continue,
            }
        }
    }

    async fn next_event(&mut self) -> AgentEvent {
        loop {
            match self
                .next()
                .await
                .expect("connection dropped (an event was expected)")
            {
                Incoming::Event(ev) => return ev,
                Incoming::Frame(..) => continue,
            }
        }
    }

    async fn send_binary(&mut self, data: Vec<u8>) {
        tokio::time::timeout(NET_TIMEOUT, self.ws.send(Message::Binary(data.into())))
            .await
            .expect("send binary timeout")
            .expect("send binary failed");
    }

    async fn send_raw_text(&mut self, text: &str) {
        tokio::time::timeout(
            NET_TIMEOUT,
            self.ws.send(Message::Text(text.to_string().into())),
        )
        .await
        .expect("send raw timeout")
        .expect("send raw failed");
    }

    async fn await_ack(&mut self, expect_id: engram_dashboard_protocol::RequestId) {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match self.next().await {
                Some(Incoming::Event(AgentEvent::Ack { request_id })) => {
                    assert_eq!(
                        request_id, expect_id,
                        "the request_id of the Ack must be echoed"
                    );
                    return;
                }
                Some(Incoming::Event(AgentEvent::Error {
                    request_id: Some(rid),
                    message,
                })) if rid == expect_id => {
                    panic!("expected Ack but got Error(req={rid:?}): {message}");
                }
                Some(_) => continue,
                None => break,
            }
        }
        panic!("timeout/close before the Ack of request_id={expect_id:?} arrived");
    }

    async fn await_error(&mut self, expect_id: engram_dashboard_protocol::RequestId) -> String {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match self.next().await {
                Some(Incoming::Event(AgentEvent::Error {
                    request_id: Some(rid),
                    message,
                })) if rid == expect_id => {
                    return message;
                }
                Some(Incoming::Event(AgentEvent::Ack { request_id }))
                    if request_id == expect_id =>
                {
                    panic!("expected Error but got Ack(req={request_id:?})");
                }
                Some(_) => continue,
                None => break,
            }
        }
        panic!("timeout/close before the Error of request_id={expect_id:?} arrived");
    }

    async fn await_error_no_id(&mut self) -> String {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match self.next().await {
                Some(Incoming::Event(AgentEvent::Error { message, .. })) => return message,
                Some(_) => continue,
                None => break,
            }
        }
        panic!("timeout/close before the Error arrived");
    }

    async fn await_profile_list(
        &mut self,
        req: RequestId,
    ) -> Vec<engram_dashboard_protocol::AgentProfile> {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match self.next().await {
                Some(Incoming::Event(AgentEvent::ProfileList {
                    request_id,
                    profiles,
                })) => {
                    assert_eq!(request_id, req, "request_id echo of ProfileList");
                    return profiles;
                }
                Some(_) => continue,
                None => break,
            }
        }
        panic!("timeout/close before ProfileList arrived");
    }

    /// Collects the reply (Ack) and broadcast_profile_list together in one loop so as not to depend on their queueing order.
    async fn await_crud(&mut self, req: RequestId) -> Vec<engram_dashboard_protocol::AgentProfile> {
        let mut saw_ack = false;
        let mut profiles: Option<Vec<engram_dashboard_protocol::AgentProfile>> = None;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while (!saw_ack || profiles.is_none()) && std::time::Instant::now() < deadline {
            match self.next().await {
                Some(Incoming::Event(AgentEvent::Ack { request_id })) => {
                    assert_eq!(request_id, req, "request_id echo of the CRUD Ack");
                    saw_ack = true;
                }
                Some(Incoming::Event(AgentEvent::ProfileListUpdated { profiles: p })) => {
                    profiles = Some(p);
                }
                Some(Incoming::Event(AgentEvent::Error {
                    request_id: Some(rid),
                    message,
                })) if rid == req => panic!("CRUD failure Error(req={rid:?}): {message}"),
                Some(_) => continue,
                None => break,
            }
        }
        assert!(
            saw_ack && profiles.is_some(),
            "after CRUD both Ack({saw_ack}) and ProfileListUpdated({}) must arrive",
            profiles.is_some()
        );
        profiles.unwrap()
    }

    async fn await_created(&mut self, req: RequestId) -> engram_dashboard_protocol::AgentProfile {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match self.next().await {
                Some(Incoming::Event(AgentEvent::Created {
                    request_id,
                    profile,
                })) => {
                    assert_eq!(request_id, req, "request_id echo of Created");
                    return profile;
                }
                Some(Incoming::Event(AgentEvent::Ack { request_id })) if request_id == req => {
                    panic!("expected Created but got Ack(req={request_id:?}) — no duplicate Ack");
                }
                Some(Incoming::Event(AgentEvent::Error {
                    request_id: Some(rid),
                    message,
                })) if rid == req => {
                    panic!("expected Created but got Error(req={rid:?}): {message}")
                }
                Some(_) => continue,
                None => break,
            }
        }
        panic!("timeout/close before the Created of request_id={req:?} arrived");
    }

    async fn await_spawned(&mut self, req: RequestId) -> engram_dashboard_protocol::AgentInfo {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match self.next().await {
                Some(Incoming::Event(AgentEvent::Spawned { request_id, agent })) => {
                    assert_eq!(request_id, req, "request_id echo of Spawned");
                    return agent;
                }
                Some(Incoming::Event(AgentEvent::Ack { request_id })) if request_id == req => {
                    panic!("expected Spawned but got Ack(req={request_id:?}) — no duplicate Ack");
                }
                Some(Incoming::Event(AgentEvent::Error {
                    request_id: Some(rid),
                    message,
                })) if rid == req => {
                    panic!("expected Spawned but got Error(req={rid:?}): {message}")
                }
                Some(_) => continue,
                None => break,
            }
        }
        panic!("timeout/close before the Spawned of request_id={req:?} arrived");
    }

    async fn await_snapshot(
        &mut self,
        req: RequestId,
    ) -> (Uuid, Vec<engram_dashboard_protocol::SnapshotChunk>) {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match self.next().await {
                Some(Incoming::Event(AgentEvent::Snapshot {
                    request_id,
                    agent_id,
                    chunks,
                })) => {
                    assert_eq!(request_id, req, "request_id echo of Snapshot");
                    return (agent_id, chunks);
                }
                Some(_) => continue,
                None => break,
            }
        }
        panic!("timeout/close before Snapshot arrived");
    }

    async fn await_agent_list(
        &mut self,
        req: RequestId,
    ) -> Vec<engram_dashboard_protocol::AgentInfo> {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match self.next().await {
                Some(Incoming::Event(AgentEvent::AgentList { request_id, agents })) => {
                    assert_eq!(request_id, req, "request_id echo of AgentList");
                    return agents;
                }
                Some(_) => continue,
                None => break,
            }
        }
        panic!("timeout/close before AgentList arrived");
    }

    /// Important: spawn_agent queues the agent_list_updated broadcast before the reply (Ack), so the conn_tx order is [list, Ack]; calling await_ack first would let the list slip by, so both are collected in one loop.
    async fn await_spawn(&mut self, wanted: Uuid, req: RequestId) {
        let mut saw_ack = false;
        let mut saw_list = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while (!saw_ack || !saw_list) && std::time::Instant::now() < deadline {
            match self.next().await {
                Some(Incoming::Event(AgentEvent::Ack { request_id })) => {
                    assert_eq!(request_id, req, "request_id echo of the Spawn Ack");
                    saw_ack = true;
                }
                Some(Incoming::Event(AgentEvent::AgentListUpdated { agents })) => {
                    if agents.iter().any(|a| a.id == wanted) {
                        saw_list = true;
                    }
                }
                Some(Incoming::Event(AgentEvent::Error {
                    request_id: Some(rid),
                    message,
                })) if rid == req => panic!("Spawn failure Error(req={rid:?}): {message}"),
                Some(_) => continue,
                None => break,
            }
        }
        assert!(
            saw_ack && saw_list,
            "after Spawn both Ack({saw_ack}) and AgentListUpdated({saw_list}) must arrive"
        );
    }

    /// kill_agent also queues the list update before the reply (Ack) (order [list, Ack]) so they are collected together.
    async fn await_kill(&mut self, wanted: Uuid, req: RequestId) {
        let mut saw_ack = false;
        let mut saw_excluded = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while (!saw_ack || !saw_excluded) && std::time::Instant::now() < deadline {
            match self.next().await {
                Some(Incoming::Event(AgentEvent::Ack { request_id })) => {
                    assert_eq!(request_id, req, "request_id echo of the Kill Ack");
                    saw_ack = true;
                }
                Some(Incoming::Event(AgentEvent::AgentListUpdated { agents })) => {
                    if !agents.iter().any(|a| a.id == wanted) {
                        saw_excluded = true;
                    }
                }
                Some(Incoming::Event(AgentEvent::Error {
                    request_id: Some(rid),
                    message,
                })) if rid == req => panic!("Kill failure Error(req={rid:?}): {message}"),
                Some(_) => continue,
                None => break,
            }
        }
        assert!(
            saw_ack && saw_excluded,
            "after Kill both Ack({saw_ack}) and list exclusion({saw_excluded}) must arrive"
        );
    }

    async fn expect_closed(&mut self) -> bool {
        loop {
            match self.next().await {
                None => return true,
                Some(Incoming::Event(AgentEvent::Error { .. })) => continue,
                Some(_) => continue,
            }
        }
    }

    async fn expect_closed_within(&mut self, deadline: Duration) -> bool {
        let end = std::time::Instant::now() + deadline;
        loop {
            let remaining = end.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return false;
            }
            match tokio::time::timeout(remaining, self.ws.next()).await {
                Err(_) => return false,
                Ok(None) | Ok(Some(Err(_))) => return true,
                Ok(Some(Ok(Message::Close(_)))) => return true,
                // Other messages are ignored (read but — tungstenite decodes one message at a time).
                Ok(Some(Ok(_))) => continue,
            }
        }
    }

    /// Important: tungstenite auto-Pongs an incoming Ping when it polls the stream; this method keeps reading like a normal client (triggering the auto Pong) and observes the Ping's arrival. Other control/binary frames are absorbed.
    async fn saw_ping_within(&mut self, deadline: Duration) -> bool {
        let end = std::time::Instant::now() + deadline;
        loop {
            let remaining = end.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return false;
            }
            match tokio::time::timeout(remaining, self.ws.next()).await {
                Err(_) => return false,
                Ok(None) | Ok(Some(Err(_))) => return false,
                Ok(Some(Ok(Message::Ping(_)))) => return true,
                Ok(Some(Ok(_))) => continue,
            }
        }
    }

    async fn stays_alive_while_reading(&mut self, deadline: Duration) -> bool {
        let end = std::time::Instant::now() + deadline;
        loop {
            let remaining = end.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return true;
            }
            match tokio::time::timeout(remaining, self.ws.next()).await {
                Err(_) => return true,
                Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(Message::Close(_)))) => return false,
                Ok(Some(Ok(_))) => continue,
            }
        }
    }
}

/// Exposes Client methods as module free functions (readability of the slow consumer case).
async fn expect_closed_within(c: &mut Client, deadline: Duration) -> bool {
    c.expect_closed_within(deadline).await
}

/// resize is asynchronous (WS → read_task → dispatch → manager), so it is not reflected immediately → poll.
async fn wait_for_size(handle: &TestServerHandle, id: Uuid, cols: u16, rows: u16) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    loop {
        let got = handle
            .manager
            .list_agents()
            .into_iter()
            .find(|a| a.id == id)
            .map(|a| (a.cols, a.rows));
        if got == Some((cols, rows)) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// ── server helpers ──────────────────────────────────────────────────────────────────────

/// Directly launches an interactive `cmd.exe` for deterministic output (without /c — a live shell).
fn spawn_shell_agent(handle: &TestServerHandle) -> Uuid {
    #[cfg(windows)]
    let command = AgentCommand::Shell {
        program: "cmd.exe".into(),
        args: vec![],
    };
    #[cfg(not(windows))]
    let command = AgentCommand::Shell {
        program: "sh".into(),
        args: vec!["-i".into()],
    };
    let profile = AgentProfile::new(
        "e2e-shell".into(),
        command,
        std::env::temp_dir(),
        vec![],
        false, // auto_restore=false (not a restore target)
    );
    let id = profile.id;
    handle
        .manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("shell agent spawn failed")
        .into_started()
        .expect("this call really launches (not a duplicate request)");
    id
}

/// To ride the WS `Spawn{profile_id}` dispatch path, the manager's registry needs a known profile.
/// Zero production regression: registration uses only the manager's public API (`try_create_agent` — the single roster entrance, ADR-0119), not the start_test_server/run() wiring. Production `CreateProfile` uses the same verb, so it rides the name-uniqueness enforcement (ADR-0120) too.
fn register_shell_profile(handle: &TestServerHandle) -> Uuid {
    #[cfg(windows)]
    let command = AgentCommand::Shell {
        program: "cmd.exe".into(),
        args: vec![],
    };
    #[cfg(not(windows))]
    let command = AgentCommand::Shell {
        program: "sh".into(),
        args: vec!["-i".into()],
    };
    let profile = AgentProfile::new(
        "e2e-ws-shell".into(),
        command,
        std::env::temp_dir(),
        vec![],
        false, // auto_restore=false (not a restore target)
    );
    let id = profile.id;
    handle
        .manager
        .try_create_agent(profile)
        .expect("registration succeeded");
    id
}

/// The PTY emits deterministic output immediately, but to absorb OS scheduling delay it waits up to a deadline until the snapshot seq count reaches min_events.
async fn wait_for_output(handle: &TestServerHandle, id: Uuid, min_events: usize) {
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    loop {
        let n = handle
            .manager
            .get_snapshot(id)
            .map(|s| s.len())
            .unwrap_or(0);
        if n >= min_events {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!("timeout waiting for {min_events} outputs (currently {n})");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// ── case 1: auth success → Hello + AgentListUpdated ────────────────────────────────
#[tokio::test]
async fn case01_auth_success_hello_and_list() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;

    match c.next_event().await {
        AgentEvent::Hello {
            protocol_version, ..
        } => assert_eq!(protocol_version, PROTOCOL_VERSION),
        ev => panic!("expected Hello, got {ev:?}"),
    }
    match c.next_event().await {
        AgentEvent::AgentListUpdated { agents } => {
            assert!(agents.is_empty(), "the initial list must be empty")
        }
        ev => panic!("expected AgentListUpdated, got {ev:?}"),
    }

    server.shutdown().await;
}

// ── case 2: auth failure (wrong token) → the server closes ──────────────────────────────────
#[tokio::test]
async fn case02_auth_wrong_token_closes() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &"f".repeat(64)).await;
    // Short deadline: the old expect_closed caught the expected close within a 10 s recv timeout — slow and unclear (mutation D); the close is immediate, so assert within 3 s.
    assert!(
        c.expect_closed_within(Duration::from_secs(3)).await,
        "with a wrong token the connection must close immediately (within 3s)"
    );
    server.shutdown().await;
}

// ── Case 3: auth timeout (no first frame) → the server closes after AUTH_TIMEOUT ──────────────────────
#[tokio::test]
async fn case03_auth_timeout_closes() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_raw(server.port).await;
    // Still open well past the old 1 s window, then closed once the 15 s window passes.
    assert!(
        !c.expect_closed_within(Duration::from_secs(3)).await,
        "a connection without an auth frame must stay open for AUTH_TIMEOUT, not close at once"
    );
    assert!(
        c.expect_closed_within(Duration::from_secs(20)).await,
        "a connection without an auth frame must close once AUTH_TIMEOUT passes"
    );
    server.shutdown().await;
}

// ── case 4: output order (seq 0,1,2… intact) ───────────────────────────────────────────
#[tokio::test]
async fn case04_output_order_exact() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    c.send(&WireCommand::Subscribe {
        agent_id: id,
        epoch: None,
        after_seq: None,
    })
    .await;

    server
        .manager
        .write_stdin(id, b"echo CASE4_MARKER\r\n", InputOrigin::User)
        .unwrap();

    let seqs = collect_frame_seqs_until_marker(&mut c, id, "CASE4_MARKER").await;
    assert!(!seqs.is_empty(), "frames must be received");
    assert_seq_contiguous_from_zero(&seqs);

    server.shutdown().await;
}

// ── case 5: replay→live FIFO order (SubscribeAck→replay→ReplayComplete→live) ────────
#[tokio::test]
async fn case05_replay_then_live_order() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);

    server
        .manager
        .write_stdin(id, b"echo PREFILL\r\n", InputOrigin::User)
        .unwrap();
    wait_for_output(&server, id, 1).await;

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;
    c.send(&WireCommand::Subscribe {
        agent_id: id,
        epoch: None,
        after_seq: None,
    })
    .await;

    let ack = c.next_event().await;
    match ack {
        AgentEvent::SubscribeAck {
            action, agent_id, ..
        } => {
            assert_eq!(agent_id, id);
            assert_eq!(
                action,
                SubscribeAction::Reset,
                "after_seq=None → Reset(oldest)"
            );
        }
        ev => panic!("expected SubscribeAck, got {ev:?}"),
    }
    let mut replay_frames = 0usize;
    loop {
        match c.next().await.expect("dropped before ReplayComplete") {
            Incoming::Frame(aid, _, _, _) => {
                assert_eq!(aid, id);
                replay_frames += 1;
            }
            Incoming::Event(AgentEvent::ReplayComplete { agent_id, .. }) => {
                assert_eq!(agent_id, id);
                break;
            }
            Incoming::Event(ev) => panic!("unexpected event in the replay phase: {ev:?}"),
        }
    }
    assert!(replay_frames >= 1, "at least 1 PREFILL replay frame");

    server
        .manager
        .write_stdin(id, b"echo LIVE5\r\n", InputOrigin::User)
        .unwrap();
    let live = collect_frames_until_marker(&mut c, id, "LIVE5").await;
    assert!(
        !live.is_empty(),
        "live frames must arrive after ReplayComplete"
    );

    server.shutdown().await;
}

// ── case 6: afterSeq resume — tail only ────────────────────────────────────────────
#[tokio::test]
async fn case06_after_seq_resume_tail_only() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);
    let epoch = server.manager.agent_epoch(id).unwrap();

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;
    c.send(&WireCommand::Subscribe {
        agent_id: id,
        epoch: Some(epoch),
        after_seq: None,
    })
    .await;
    server
        .manager
        .write_stdin(id, b"echo R6A\r\n", InputOrigin::User)
        .unwrap();
    let first = collect_frame_seqs_until_marker(&mut c, id, "R6A").await;
    let last_seq = *first.iter().max().unwrap();

    drop(c);
    server
        .manager
        .write_stdin(id, b"echo R6B\r\n", InputOrigin::User)
        .unwrap();
    wait_for_output(&server, id, (last_seq as usize) + 2).await;

    let mut c2 = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c2).await;
    c2.send(&WireCommand::Subscribe {
        agent_id: id,
        epoch: Some(epoch),
        after_seq: Some(last_seq),
    })
    .await;
    match c2.next_event().await {
        AgentEvent::SubscribeAck {
            action,
            replay_from,
            ..
        } => {
            assert_eq!(
                action,
                SubscribeAction::Resume,
                "after_seq>=oldest → Resume"
            );
            assert!(
                replay_from > last_seq,
                "resume starts from beyond last_seq({last_seq}) (replay_from={replay_from})"
            );
        }
        ev => panic!("expected SubscribeAck(Resume), got {ev:?}"),
    }
    let tail = collect_frame_seqs_until_marker(&mut c2, id, "R6B").await;
    for s in &tail {
        assert!(
            *s > last_seq,
            "tail frame seq({s}) must exceed last_seq({last_seq})"
        );
    }

    server.shutdown().await;
}

// ── case 7: truncated — after_seq<oldest after output exceeding the ring (2MB) ───────────────────
#[tokio::test]
async fn case07_truncated_replay() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);
    let epoch = server.manager.agent_epoch(id).unwrap();

    // (~80B per line × 40000 ≈ 3MB → the 2MB ring's oldest gets pushed above 0.)
    server
        .manager
        .write_stdin(
            id,
            b"for /L %i in (1,1,40000) do @echo TRUNCATE_LINE_PADDING_XXXXXXXXXXXXXXXXXXXXXXXXXXXX %i\r\n", InputOrigin::User,
        )
        .unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let snap = server.manager.get_snapshot(id).unwrap();
        if snap.first().map(|c| c.seq).unwrap_or(0) > 0 && snap.len() >= 10 {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!("timeout waiting for ring eviction (oldest>0)");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;
    c.send(&WireCommand::Subscribe {
        agent_id: id,
        epoch: Some(epoch),
        after_seq: Some(0),
    })
    .await;
    match c.next_event().await {
        AgentEvent::SubscribeAck {
            action,
            truncated,
            oldest_seq,
            ..
        } => {
            assert_eq!(action, SubscribeAction::TruncatedReplay);
            assert!(truncated, "the truncated flag is set");
            assert!(
                oldest_seq > 0,
                "oldest must be pushed above 0 (eviction occurred)"
            );
        }
        ev => panic!("expected SubscribeAck(Truncated), got {ev:?}"),
    }

    server.shutdown().await;
}

// ── case 8: epoch mismatch → Reset (from oldest) ──────────────────────────────────
#[tokio::test]
async fn case08_epoch_mismatch_reset() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);
    let epoch = server.manager.agent_epoch(id).unwrap();

    server
        .manager
        .write_stdin(id, b"echo E8\r\n", InputOrigin::User)
        .unwrap();
    wait_for_output(&server, id, 1).await;

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;
    // A different incarnation marker + after_seq given → after_seq is ignored and it Resets.
    c.send(&WireCommand::Subscribe {
        agent_id: id,
        epoch: Some(epoch.wrapping_add(1)),
        after_seq: Some(5),
    })
    .await;
    match c.next_event().await {
        AgentEvent::SubscribeAck {
            action,
            current_epoch,
            ..
        } => {
            assert_eq!(action, SubscribeAction::Reset, "epoch mismatch → Reset");
            assert_eq!(current_epoch, epoch, "the current epoch is reported");
        }
        ev => panic!("expected SubscribeAck(Reset), got {ev:?}"),
    }

    server.shutdown().await;
}

// ── case 9: slow consumer → only that connection closes, other connections unaffected ──────────────────────
//
// Mechanism: after ReplayComplete the slow consumer never reads its socket. With bulk output on the same agent, its server-side outbound queue (`CONN_TX_CAP` = 4608 in `link::server`) and the OS socket buffers fill, and the frame outlet's `try_send` meets a full queue and closes that connection only (`link::server::ConnFrameSink`).
// The good consumer keeps draining in a background task and must survive (other connections unaffected).
#[tokio::test]
async fn case09_slow_consumer_closed_others_unaffected() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);

    let mut good = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut good).await;
    good.send(&WireCommand::Subscribe {
        agent_id: id,
        epoch: None,
        after_seq: None,
    })
    .await;
    wait_replay_complete(&mut good, id).await;

    let mut slow = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut slow).await;
    slow.send(&WireCommand::Subscribe {
        agent_id: id,
        epoch: None,
        after_seq: None,
    })
    .await;
    wait_replay_complete(&mut slow, id).await;

    let good_alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let good_frames = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let drain_task = {
        let good_alive = good_alive.clone();
        let good_frames = good_frames.clone();
        tokio::spawn(async move {
            while let Some(item) = good.next().await {
                if let Incoming::Frame(..) = item {
                    good_frames.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
            good_alive.store(false, std::sync::atomic::Ordering::Relaxed);
        })
    };

    // m1 (hardening): the output volume is raised to avoid a false positive (no blocking where socket buffers are large). One line ~140 B (padded) × 200000 ≈ 28 MB; the 4608-slot queue (≈0.6 MB) plus any realistic OS socket send buffer (tens of KB to a few MB) cannot absorb it, so while slow does not read, `try_send` surely meets a full queue.
    //   (Limit: OS buffers above 28 MB could make this flaky; not with realistic defaults.)
    server
        .manager
        .write_stdin(
            id,
            b"for /L %i in (1,1,200000) do @echo SLOW9_PADDING_XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX %i\r\n", InputOrigin::User,
        )
        .unwrap();

    // Important: if slow read at once the socket would drain and the server queue would not fill. It first waits without reading so the server outbound queue + OS buffers fill and the connection closes, then reads down what reached its socket and meets the close.
    tokio::time::sleep(Duration::from_secs(3)).await;

    // slow is a connection the server closed, so after draining the backlog it reaches Close/None — a generous deadline.
    assert!(
        expect_closed_within(&mut slow, Duration::from_secs(40)).await,
        "the slow consumer's connection must be closed by the server"
    );

    assert!(
        good_alive.load(std::sync::atomic::Ordering::Relaxed),
        "the normal consumer must not be closed"
    );
    assert!(
        good_frames.load(std::sync::atomic::Ordering::Relaxed) > 0,
        "the normal consumer keeps receiving frames unaffected"
    );

    drain_task.abort();
    server.shutdown().await;
}

// ── case 10: reconnect recovery — lossless after resume (gap 0 after seq dedup) ────────────────
#[tokio::test]
async fn case10_reconnect_lossless() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);
    let epoch = server.manager.agent_epoch(id).unwrap();

    let mut c1 = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c1).await;
    c1.send(&WireCommand::Subscribe {
        agent_id: id,
        epoch: Some(epoch),
        after_seq: None,
    })
    .await;
    server
        .manager
        .write_stdin(id, b"echo RC10A\r\n", InputOrigin::User)
        .unwrap();
    let got1 = collect_frame_seqs_until_marker(&mut c1, id, "RC10A").await;
    let max1 = *got1.iter().max().unwrap();
    drop(c1);

    server
        .manager
        .write_stdin(id, b"echo RC10B\r\n", InputOrigin::User)
        .unwrap();
    wait_for_output(&server, id, (max1 as usize) + 2).await;

    let mut c2 = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c2).await;
    c2.send(&WireCommand::Subscribe {
        agent_id: id,
        epoch: Some(epoch),
        after_seq: Some(max1),
    })
    .await;
    // Drain the SubscribeAck.
    let _ = c2.next_event().await;
    let got2 = collect_frame_seqs_until_marker(&mut c2, id, "RC10B").await;

    let mut all: Vec<u64> = got1.clone();
    all.extend(got2.iter().copied());
    all.sort_unstable();
    all.dedup();
    let max_all = *all.last().unwrap();
    let expected: Vec<u64> = (0..=max_all).collect();
    assert_eq!(all, expected, "seq gap 0 after reconnect+resume (lossless)");

    server.shutdown().await;
}

// ── case 11: high throughput — order intact, no deadlock ────────────────────────────────
#[tokio::test]
async fn case11_high_throughput_no_deadlock() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;
    c.send(&WireCommand::Subscribe {
        agent_id: id,
        epoch: None,
        after_seq: None,
    })
    .await;
    wait_replay_complete(&mut c, id).await;

    server
        .manager
        .write_stdin(
            id,
            b"for /L %i in (1,1,3000) do @echo HT11 %i\r\necho HT11_DONE\r\n",
            InputOrigin::User,
        )
        .unwrap();

    let seqs = collect_frame_seqs_until_marker(&mut c, id, "HT11_DONE").await;
    assert!(!seqs.is_empty(), "frames of bulk output received");
    // Received seqs are strictly increasing (order intact). Without a truncated notice they should be contiguous, but with possible ring eviction only "increasing" is asserted — no deadlock/order reversal is the point.
    for w in seqs.windows(2) {
        assert!(
            w[1] > w[0],
            "frame seqs must increase monotonically (order intact): {:?}",
            w
        );
    }

    server.shutdown().await;
}

// ── case 12: multi-subscribe (demultiplexing) — one connection subscribes to 2 agents ──────────────────────
#[tokio::test]
async fn case12_multi_subscribe_demux() {
    let server = start_test_server().await.unwrap();
    let id_a = spawn_shell_agent(&server);
    let id_b = spawn_shell_agent(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;
    c.send(&WireCommand::Subscribe {
        agent_id: id_a,
        epoch: None,
        after_seq: None,
    })
    .await;
    wait_replay_complete(&mut c, id_a).await;
    c.send(&WireCommand::Subscribe {
        agent_id: id_b,
        epoch: None,
        after_seq: None,
    })
    .await;
    wait_replay_complete(&mut c, id_b).await;

    server
        .manager
        .write_stdin(id_a, b"echo AAA12\r\n", InputOrigin::User)
        .unwrap();
    server
        .manager
        .write_stdin(id_b, b"echo BBB12\r\n", InputOrigin::User)
        .unwrap();

    let mut saw_a = false;
    let mut saw_b = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while (!saw_a || !saw_b) && std::time::Instant::now() < deadline {
        match c.next().await {
            Some(Incoming::Frame(aid, _, _, payload)) => {
                let text = String::from_utf8_lossy(&payload);
                if text.contains("AAA12") {
                    assert_eq!(aid, id_a, "AAA12 must come only to agent A");
                    saw_a = true;
                }
                if text.contains("BBB12") {
                    assert_eq!(aid, id_b, "BBB12 must come only to agent B");
                    saw_b = true;
                }
            }
            Some(Incoming::Event(_)) => continue,
            None => break,
        }
    }
    assert!(
        saw_a && saw_b,
        "must demultiplex the output of the two agents by their own agent_id (a={saw_a}, b={saw_b})"
    );

    server.shutdown().await;
}

// ══════════════════════════════════════════════════════════════════════════════════
// M1: E2E that really rides WS dispatch().
//
// Cases case01~12 above create the agent directly with `server.manager.spawn_agent` to see output-plane (replay/seq/slow-consumer) determinism, bypassing dispatch. Cases case13~ below verify the path that sends commands as WS frames (JSON text control) and really rides read_task→dispatch() —
// Spawn/WriteStdin/Kill/Interrupt/Resize/Unsubscribe/ListAgents/StopDaemon/second Auth/binary rejection/parse failure — with the request_id echo (Ack/Error mapping) asserted too.
// ══════════════════════════════════════════════════════════════════════════════════

// ── case 13: WS Spawn → Ack(req echo) + AgentListUpdated(new agent_id) ─────────────
#[tokio::test]
async fn case13_ws_spawn_ack_and_list() {
    let server = start_test_server().await.unwrap();
    let profile_id = register_shell_profile(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req = RequestId::new();
    c.send(&WireCommand::Spawn {
        profile_id,
        request_id: req,
    })
    .await;

    c.await_spawn(profile_id, req).await;
    assert!(
        server.manager.agent_epoch(profile_id).is_some(),
        "after WS Spawn the agent must be alive in the manager"
    );

    server.shutdown().await;
}

// ── case 14: WS Spawn → WriteStdin (the VIA_WS marker as a binary frame) ───────────────
#[tokio::test]
async fn case14_ws_write_stdin_roundtrip() {
    let server = start_test_server().await.unwrap();
    let profile_id = register_shell_profile(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req_spawn = RequestId::new();
    c.send(&WireCommand::Spawn {
        profile_id,
        request_id: req_spawn,
    })
    .await;
    c.await_spawn(profile_id, req_spawn).await;

    c.send(&WireCommand::Subscribe {
        agent_id: profile_id,
        epoch: None,
        after_seq: None,
    })
    .await;
    wait_replay_complete(&mut c, profile_id).await;

    let req_write = RequestId::new();
    c.send(&WireCommand::WriteStdin {
        agent_id: profile_id,
        data: b"echo VIA_WS\r\n".to_vec(),
        request_id: req_write,
    })
    .await;
    c.await_ack(req_write).await;

    let frames = collect_frames_until_marker(&mut c, profile_id, "VIA_WS").await;
    assert!(
        !frames.is_empty(),
        "the VIA_WS output must arrive as a binary frame"
    );

    server.shutdown().await;
}

// ── case 15: WS Kill → Ack + AgentListUpdated reflecting termination (invariant: terminal=list) ──
#[tokio::test]
async fn case15_ws_kill_ack_and_list_excludes() {
    let server = start_test_server().await.unwrap();
    let profile_id = register_shell_profile(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req_spawn = RequestId::new();
    c.send(&WireCommand::Spawn {
        profile_id,
        request_id: req_spawn,
    })
    .await;
    c.await_spawn(profile_id, req_spawn).await;

    let req_kill = RequestId::new();
    c.send(&WireCommand::Kill {
        agent_id: profile_id,
        request_id: req_kill,
    })
    .await;
    c.await_kill(profile_id, req_kill).await;
    assert!(
        server.manager.agent_epoch(profile_id).is_none(),
        "after kill the agent is removed from the manager"
    );

    server.shutdown().await;
}

// ── case 16: WS Interrupt → terminal mode refuses (Error) · the process survives ──────────────────
// Terminal mode does not accept an interrupt command — 0x03 could kill an idle TUI on a second Ctrl-C.
// ADR-0245
#[tokio::test]
async fn case16_ws_interrupt_refused_on_terminal_process_alive() {
    let server = start_test_server().await.unwrap();
    let profile_id = register_shell_profile(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req_spawn = RequestId::new();
    c.send(&WireCommand::Spawn {
        profile_id,
        request_id: req_spawn,
    })
    .await;
    c.await_spawn(profile_id, req_spawn).await;

    let req_int = RequestId::new();
    c.send(&WireCommand::Interrupt {
        agent_id: profile_id,
        request_id: req_int,
    })
    .await;
    let msg = c.await_error(req_int).await;
    assert!(
        msg.contains("terminal mode does not accept the interrupt command"),
        "a PTY interrupt must be answered with the refusal wording: {msg}"
    );
    assert!(
        server.manager.agent_epoch(profile_id).is_some(),
        "the process survives even after a refused Interrupt (stays in the manager)"
    );

    server.shutdown().await;
}

// ── case 17: WS Resize → accepted without error (Resize has no request_id → no Ack) ──────
#[tokio::test]
async fn case17_ws_resize_no_error() {
    let server = start_test_server().await.unwrap();
    let profile_id = register_shell_profile(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req_spawn = RequestId::new();
    c.send(&WireCommand::Spawn {
        profile_id,
        request_id: req_spawn,
    })
    .await;
    c.await_spawn(profile_id, req_spawn).await;

    // WS Resize has no request_id (messages.rs): dispatch is silent on success and emits only Error on failure, so both "no Ack" (as designed) and "no Error" are asserted.
    c.send(&WireCommand::Resize {
        agent_id: profile_id,
        cols: 100,
        rows: 40,
        viewport_id: None,
    })
    .await;

    // Send a follow-up command (ListAgents) and verify "silence" by whether any Resize Error/Ack gets in before its response.
    let req_list = RequestId::new();
    c.send(&WireCommand::ListAgents {
        request_id: req_list,
    })
    .await;
    let agents = c.await_agent_list(req_list).await;
    assert!(
        agents.iter().any(|a| a.id == profile_id),
        "the ListAgents response must arrive (Resize must be silent)"
    );

    server.shutdown().await;
}

// ── case 18: WS Unsubscribe → no more live frames afterwards ────────────────────────────
#[tokio::test]
async fn case18_ws_unsubscribe_stops_live() {
    let server = start_test_server().await.unwrap();
    let profile_id = register_shell_profile(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req_spawn = RequestId::new();
    c.send(&WireCommand::Spawn {
        profile_id,
        request_id: req_spawn,
    })
    .await;
    c.await_spawn(profile_id, req_spawn).await;

    c.send(&WireCommand::Subscribe {
        agent_id: profile_id,
        epoch: None,
        after_seq: None,
    })
    .await;
    wait_replay_complete(&mut c, profile_id).await;
    server
        .manager
        .write_stdin(profile_id, b"echo PRE_UNSUB\r\n", InputOrigin::User)
        .unwrap();
    let _ = collect_frames_until_marker(&mut c, profile_id, "PRE_UNSUB").await;

    c.send(&WireCommand::Unsubscribe {
        agent_id: profile_id,
    })
    .await;

    // Timing-independent guarantee: round-trip ListAgents to confirm Unsubscribe was already processed in the read_task (the same read_task is FIFO), then produce new output.
    let req_list = RequestId::new();
    c.send(&WireCommand::ListAgents {
        request_id: req_list,
    })
    .await;
    loop {
        match c
            .next()
            .await
            .expect("dropped before the ListAgents response")
        {
            Incoming::Event(AgentEvent::AgentList { request_id, .. }) => {
                assert_eq!(
                    request_id, req_list,
                    "request_id echo of the ListAgents response"
                );
                break;
            }
            Incoming::Frame(..) => {
                panic!("a leftover frame arrived after Unsubscribe (the subscription was not cut)")
            }
            _ => continue,
        }
    }
    server
        .manager
        .write_stdin(profile_id, b"echo POST_UNSUB\r\n", InputOrigin::User)
        .unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match tokio::time::timeout(remaining, c.next()).await {
            Ok(Some(Incoming::Frame(..))) => {
                panic!("a live frame must not arrive after Unsubscribe");
            }
            Ok(Some(Incoming::Event(_))) => continue,
            Ok(None) => break,
            Err(_) => break, // timeout = no frame arrives (normal).
        }
    }

    server.shutdown().await;
}

// ── case 19: WS ListAgents → AgentList ───────────────────────────────────────────
#[tokio::test]
async fn case19_ws_list_agents() {
    let server = start_test_server().await.unwrap();
    let profile_id = register_shell_profile(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req_spawn = RequestId::new();
    c.send(&WireCommand::Spawn {
        profile_id,
        request_id: req_spawn,
    })
    .await;
    c.await_spawn(profile_id, req_spawn).await;

    let req_list = RequestId::new();
    c.send(&WireCommand::ListAgents {
        request_id: req_list,
    })
    .await;
    let agents = c.await_agent_list(req_list).await;
    assert!(
        agents.iter().any(|a| a.id == profile_id),
        "the ListAgents response includes the spawned agent"
    );

    server.shutdown().await;
}

// ── case 20: WS StopDaemon force policy (M4) ────────────────────────────────────────
//   With an active agent, force=false → refusal Error (server stays alive). Then force=true → shutdown.
#[tokio::test]
async fn case20_ws_stop_daemon_force_policy() {
    let server = start_test_server().await.unwrap();
    let profile_id = register_shell_profile(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req_spawn = RequestId::new();
    c.send(&WireCommand::Spawn {
        profile_id,
        request_id: req_spawn,
    })
    .await;
    c.await_spawn(profile_id, req_spawn).await;

    let req_reject = RequestId::new();
    c.send(&WireCommand::StopDaemon {
        force: false,
        kill_agents: false,
        request_id: req_reject,
    })
    .await;
    let msg = c.await_error(req_reject).await;
    assert!(
        msg.contains("active agents"),
        "the force=false refusal message must state the active agents: {msg}"
    );
    let req_alive = RequestId::new();
    c.send(&WireCommand::ListAgents {
        request_id: req_alive,
    })
    .await;
    let _ = c.await_agent_list(req_alive).await;

    let req_stop = RequestId::new();
    c.send(&WireCommand::StopDaemon {
        force: true,
        kill_agents: true,
        request_id: req_stop,
    })
    .await;
    // The Ack is best effort (the daemon's `Close` aborts its writer), so the test asserts the close and the shutdown.
    assert!(
        c.expect_closed().await,
        "after StopDaemon(force) the connection must close"
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !server.shutdown_signalled() {
        assert!(
            std::time::Instant::now() < deadline,
            "StopDaemon(force) must fire the daemon's shutdown within 10 s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Confirm the accept loop ended by joining accept_handle (shutdown is idempotent — re-send on an already ended watch).
    server.shutdown().await;
}

// ── case 21: second Auth → already authenticated Error ───────────────────────────────
#[tokio::test]
async fn case21_ws_second_auth_rejected() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    c.send_auth(&server.token).await;
    let msg = c.await_error_no_id().await;
    assert!(
        msg.contains("already authenticated"),
        "a second Auth must be an already authenticated Error: {msg}"
    );
    let req_alive = RequestId::new();
    c.send(&WireCommand::ListAgents {
        request_id: req_alive,
    })
    .await;
    let _ = c.await_agent_list(req_alive).await;

    server.shutdown().await;
}

// ── case 22: binary frame in the control position → Error + connection close (AgentConnection::on_binary, agent_conn.rs) ──────────
#[tokio::test]
async fn case22_ws_binary_frame_rejected() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    c.send_binary(vec![0xde, 0xad, 0xbe, 0xef]).await;
    assert!(
        c.expect_closed().await,
        "a binary in the control position must close the connection after an Error"
    );

    server.shutdown().await;
}

// ── case 23: broken JSON text → Error (no req), connection kept (AgentConnection::on_text, agent_conn.rs) ─────────────
#[tokio::test]
async fn case23_ws_parse_failure_error() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    c.send_raw_text("{\"NotACommand\":true}").await;
    let msg = c.await_error_no_id().await;
    assert!(
        msg.contains("invalid command"),
        "parse failure Error message: {msg}"
    );
    let req_alive = RequestId::new();
    c.send(&WireCommand::ListAgents {
        request_id: req_alive,
    })
    .await;
    let _ = c.await_agent_list(req_alive).await;

    server.shutdown().await;
}

// ── case 24: dispatch failure arm — Kill of a nonexistent agent_id → Error (req echo) ─────────────
#[tokio::test]
async fn case24_ws_kill_unknown_agent_error() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req = RequestId::new();
    c.send(&WireCommand::Kill {
        agent_id: Uuid::new_v4(),
        request_id: req,
    })
    .await;
    let msg = c.await_error(req).await;
    assert!(
        msg.contains("not found"),
        "Kill of a nonexistent agent must be a not found Error: {msg}"
    );

    server.shutdown().await;
}

// ── case 25: dispatch failure arm — WriteStdin to a nonexistent agent_id → Error (req echo) ─────────
#[tokio::test]
async fn case25_ws_write_unknown_agent_error() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req = RequestId::new();
    c.send(&WireCommand::WriteStdin {
        agent_id: Uuid::new_v4(),
        data: b"x".to_vec(),
        request_id: req,
    })
    .await;
    let msg = c.await_error(req).await;
    assert!(
        msg.contains("not found"),
        "WriteStdin to a nonexistent agent must be a not found Error: {msg}"
    );

    server.shutdown().await;
}

// ── case 26: dispatch failure arm — Spawn of a nonexistent profile_id → Error (req echo) ────────────
#[tokio::test]
async fn case26_ws_spawn_unknown_profile_error() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req = RequestId::new();
    c.send(&WireCommand::Spawn {
        profile_id: Uuid::new_v4(),
        request_id: req,
    })
    .await;
    let msg = c.await_error(req).await;
    assert!(
        msg.contains("profile not found"),
        "Spawn of a nonexistent profile must be a profile not found Error: {msg}"
    );

    server.shutdown().await;
}

// ══════════════════════════════════════════════════════════════════════════════════
// A: WS application-level keepalive (half-open connection detection).
// ══════════════════════════════════════════════════════════════════════════════════

fn fast_keepalive() -> KeepaliveConfig {
    KeepaliveConfig {
        ping_interval: Duration::from_millis(200),
        idle_timeout: Duration::from_millis(600),
    }
}

// ── case 27: the daemon actively sends a Ping (the premise of half-open detection) ─────────────────────
#[tokio::test]
async fn case27_keepalive_server_sends_ping() {
    let server = start_test_server_with_keepalive(fast_keepalive())
        .await
        .unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    // The first Ping must arrive within ping_interval (200ms). Slack deadline (2s).
    assert!(
        c.saw_ping_within(Duration::from_secs(2)).await,
        "the daemon must actively send a WS Ping within ping_interval (premise of half-open detection)"
    );

    server.shutdown().await;
}

// ── case 28: no Pong reply (dead client) → the server closes after idle_timeout ─────────────────
#[tokio::test]
async fn case28_keepalive_dead_client_closed() {
    let server = start_test_server_with_keepalive(fast_keepalive())
        .await
        .unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    // Caution: whether drain_handshake or expect_closed_within, polling the stream makes tungstenite auto-Pong the server Ping, updating last_recv (it stops being a dead client). So first sleep without reading at all for several times idle_timeout to cut off the auto Pong at the source;
    //   during that sleep the server ping arm detects idle (last_recv fixed at the auth moment) and closes the connection.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    assert!(
        c.expect_closed_within(Duration::from_secs(3)).await,
        "if there is no Pong reply (dead client) the server must close the connection after idle_timeout"
    );

    server.shutdown().await;
}

// ── case 29: a normal active client is not cut by keepalive (regression guard) ──────────────────
#[tokio::test]
async fn case29_keepalive_active_client_survives() {
    let server = start_test_server_with_keepalive(fast_keepalive())
        .await
        .unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    // Keeps reading for ~4× idle_timeout (600ms) (2.5s) (auto Pong) to see it is not cut.
    assert!(
        c.stays_alive_while_reading(Duration::from_millis(2500))
            .await,
        "a normal active client must not be cut — keepalive (auto Pong) keeps it from exceeding idle_timeout"
    );

    server.shutdown().await;
}

// ══════════════════════════════════════════════════════════════════════════════════
// Multi-viewer: resize negotiation (tmux smallest) + input lease (Zellij explicit lease).
// ══════════════════════════════════════════════════════════════════════════════════

// ── case 30: resize negotiation — PTY at the smallest of two viewports, renegotiated after detach ──────────
#[tokio::test]
async fn case30_multiviewer_resize_smallest_and_renegotiate() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);

    let mut c1 = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c1).await;
    c1.send(&WireCommand::Resize {
        agent_id: id,
        cols: 80,
        rows: 40,
        viewport_id: Some("a".into()),
    })
    .await;
    assert!(
        wait_for_size(&server, id, 80, 40).await,
        "viewport a alone gives (80,40)"
    );

    let mut c2 = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c2).await;
    c2.send(&WireCommand::Resize {
        agent_id: id,
        cols: 40,
        rows: 20,
        viewport_id: Some("b".into()),
    })
    .await;
    assert!(
        wait_for_size(&server, id, 40, 20).await,
        "the smallest of the two viewports (a=80x40, b=40x20) = (40,20)"
    );

    drop(c2);
    assert!(
        wait_for_size(&server, id, 80, 40).await,
        "when viewport b's connection drops, it returns to renegotiating on the remaining a as (80,40)"
    );

    server.shutdown().await;
}

// ── case 31: resize backward compat — no viewport_id bypasses negotiation (that size directly) ─────────────
#[tokio::test]
async fn case31_resize_no_viewport_id_bypasses_negotiation() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;
    c.send(&WireCommand::Resize {
        agent_id: id,
        cols: 120,
        rows: 50,
        viewport_id: None,
    })
    .await;
    assert!(
        wait_for_size(&server, id, 120, 50).await,
        "without viewport_id it resizes directly to that size (backward compat)"
    );

    server.shutdown().await;
}

// ── case 32: input lease — while held another connection's WriteStdin is refused, passes after release ───────────────
#[tokio::test]
async fn case32_input_lease_locks_other_viewer() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);

    let mut c1 = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c1).await;
    let mut c2 = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c2).await;

    let req_acq = RequestId::new();
    c1.send(&WireCommand::AcquireInput {
        agent_id: id,
        request_id: req_acq,
    })
    .await;
    c1.await_ack(req_acq).await;

    let req_w2 = RequestId::new();
    c2.send(&WireCommand::WriteStdin {
        agent_id: id,
        data: b"echo BLOCKED\r\n".to_vec(),
        request_id: req_w2,
    })
    .await;
    let msg = c2.await_error(req_w2).await;
    assert!(
        msg.contains("locked by another viewer"),
        "while a lease is held another connection's WriteStdin must be a locked Error: {msg}"
    );

    let req_w1 = RequestId::new();
    c1.send(&WireCommand::WriteStdin {
        agent_id: id,
        data: b"echo HOLDER_OK\r\n".to_vec(),
        request_id: req_w1,
    })
    .await;
    c1.await_ack(req_w1).await;

    let req_rel = RequestId::new();
    c1.send(&WireCommand::ReleaseInput {
        agent_id: id,
        request_id: req_rel,
    })
    .await;
    c1.await_ack(req_rel).await;

    let req_w2b = RequestId::new();
    c2.send(&WireCommand::WriteStdin {
        agent_id: id,
        data: b"echo NOW_OK\r\n".to_vec(),
        request_id: req_w2b,
    })
    .await;
    c2.await_ack(req_w2b).await;

    server.shutdown().await;
}

// ── case 33: the lease is released automatically when the holder's connection drops (prevents a zombie lock) ───────────────────────
#[tokio::test]
async fn case33_input_lease_auto_released_on_disconnect() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);

    let mut c1 = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c1).await;
    let mut c2 = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c2).await;

    let req_acq = RequestId::new();
    c1.send(&WireCommand::AcquireInput {
        agent_id: id,
        request_id: req_acq,
    })
    .await;
    c1.await_ack(req_acq).await;

    drop(c1);

    // The disconnect cleanup is asynchronous so it is not reflected immediately → retry polling.
    let mut acquired = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while std::time::Instant::now() < deadline {
        let req = RequestId::new();
        c2.send(&WireCommand::AcquireInput {
            agent_id: id,
            request_id: req,
        })
        .await;
        let mut got = None;
        let inner = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < inner {
            match c2.next().await {
                Some(Incoming::Event(AgentEvent::Ack { request_id })) if request_id == req => {
                    got = Some(true);
                    break;
                }
                Some(Incoming::Event(AgentEvent::Error {
                    request_id: Some(rid),
                    ..
                })) if rid == req => {
                    got = Some(false);
                    break;
                }
                Some(_) => continue,
                None => break,
            }
        }
        if got == Some(true) {
            acquired = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        acquired,
        "after the holder (c1) drops, c2 must be able to acquire the lease (the zombie lock is auto-released)"
    );

    server.shutdown().await;
}

// ── case 34: WriteStdin passes freely with no lease (case14 regression — zero friction for a single viewer) ───────
#[tokio::test]
async fn case34_no_lease_write_stdin_passes_freely() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req = RequestId::new();
    c.send(&WireCommand::WriteStdin {
        agent_id: id,
        data: b"echo FREE\r\n".to_vec(),
        request_id: req,
    })
    .await;
    c.await_ack(req).await;

    server.shutdown().await;
}

// ══════════════════════════════════════════════════════════════════════════════════
// phase4 stage 1: the WS wire path of profile CRUD + ad-hoc spawn.
// ══════════════════════════════════════════════════════════════════════════════════

// ── case 35: WS CreateProfile → Created (req echo, with the created profile attached) ────────────────
#[tokio::test]
async fn case35_ws_create_profile() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req = RequestId::new();
    let sent_cwd = std::env::temp_dir().to_string_lossy().into_owned();
    c.send(&WireCommand::CreateProfile {
        name: "p35".into(),
        cwd: sent_cwd.clone(),
        extra_args: vec!["--foo".into()],
        env: vec![],
        auto_restore: true,
        output_format: WireOutputFormat::Terminal,
        backend: Some(WireBackendKind::Claude),
        request_id: req,
    })
    .await;

    let created = c.await_created(req).await;
    assert_eq!(
        created.name, "p35",
        "the profile name attached to Created matches"
    );
    assert_eq!(created.cwd, sent_cwd, "the cwd attached to Created matches");
    assert!(
        matches!(&created.command, engram_dashboard_protocol::AgentSpawnCommand::Claude { extra_args, output_format, .. }
            if extra_args == &vec!["--foo".to_string()] && *output_format == WireOutputFormat::Terminal),
        "a claude profile keeps extra_args + default output_format=Terminal"
    );
    assert!(created.auto_restore, "auto_restore is reflected");
    assert!(
        server.manager.agent_snapshot(created.id).is_some(),
        "must exist in the manager registry after create"
    );

    // ── ADR-0044 M2: create an output_format=StreamJson profile through the same WS path → confirm it is stored in json mode ──
    let req_json = RequestId::new();
    c.send(&WireCommand::CreateProfile {
        name: "p35-json".into(),
        cwd: sent_cwd.clone(),
        extra_args: vec![],
        env: vec![],
        auto_restore: false,
        output_format: WireOutputFormat::StreamJson,
        backend: Some(WireBackendKind::Claude),
        request_id: req_json,
    })
    .await;
    let created_json = c.await_created(req_json).await;
    assert!(
        matches!(&created_json.command, engram_dashboard_protocol::AgentSpawnCommand::Claude { output_format, .. }
            if *output_format == WireOutputFormat::StreamJson),
        "a profile made as StreamJson must come back over the wire in json mode"
    );
    assert!(
        matches!(
            &server
                .manager
                .agent_snapshot(created_json.id)
                .expect("the json profile is registered")
                .command,
            AgentCommand::Claude {
                output_format: AgentOutputFormat::StreamJson,
                ..
            }
        ),
        "the core registry profile must be in json mode (wire output_format→core mapping)"
    );

    server.shutdown().await;
}

// ── case 36: WS ListProfiles → ProfileList (req echo, dedicated reply) ──────────────────
#[tokio::test]
async fn case36_ws_list_profiles() {
    let server = start_test_server().await.unwrap();
    let pre_id = register_shell_profile(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req_list = RequestId::new();
    c.send(&WireCommand::ListProfiles {
        request_id: req_list,
    })
    .await;
    let profiles = c.await_profile_list(req_list).await;
    assert!(
        profiles.iter().any(|p| p.id == pre_id),
        "the ListProfiles response includes the pre-registered profile"
    );

    server.shutdown().await;
}

// ── case 37: WS SpawnProfile → Spawned (req echo, with AgentInfo attached) ───────────────────
#[tokio::test]
async fn case37_ws_spawn_profile() {
    let server = start_test_server().await.unwrap();
    let profile_id = register_shell_profile(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req = RequestId::new();
    c.send(&WireCommand::SpawnProfile {
        profile_id,
        resume: false,
        request_id: req,
    })
    .await;
    let agent = c.await_spawned(req).await;
    assert_eq!(agent.id, profile_id, "agent.id of Spawned == profile_id");
    assert!(
        server.manager.agent_epoch(profile_id).is_some(),
        "after SpawnProfile the agent must be alive in the manager"
    );

    server.shutdown().await;
}

// ── case 38: WS DeleteProfile → Ack + ProfileListUpdated (removed) ──────────────────
#[tokio::test]
async fn case38_ws_delete_profile() {
    let server = start_test_server().await.unwrap();
    let profile_id = register_shell_profile(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req = RequestId::new();
    c.send(&WireCommand::DeleteProfile {
        profile_id,
        request_id: req,
    })
    .await;
    let profiles = c.await_crud(req).await;
    assert!(
        !profiles.iter().any(|p| p.id == profile_id),
        "must be removed from the list after DeleteProfile"
    );
    assert!(
        server.manager.agent_snapshot(profile_id).is_none(),
        "removed from the manager registry too"
    );

    server.shutdown().await;
}

// ── case 39: WS SetProfileAutoRestore → Ack + ProfileListUpdated (toggle reflected) ────────
#[tokio::test]
async fn case39_ws_set_auto_restore() {
    let server = start_test_server().await.unwrap();
    let profile_id = register_shell_profile(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req = RequestId::new();
    c.send(&WireCommand::SetProfileAutoRestore {
        profile_id,
        auto_restore: true,
        request_id: req,
    })
    .await;
    let profiles = c.await_crud(req).await;
    let p = profiles
        .iter()
        .find(|p| p.id == profile_id)
        .expect("the toggled profile must be in the list");
    assert!(p.auto_restore, "auto_restore must be toggled to true");
    assert!(
        server
            .manager
            .agent_snapshot(profile_id)
            .map(|p| p.auto_restore)
            .unwrap_or(false),
        "the toggle is reflected in the manager registry too"
    );

    server.shutdown().await;
}

// ── case 40: WS SpawnByCwd → Spawned (req echo, with AgentInfo attached) ─────────────────────
#[tokio::test]
async fn case40_ws_spawn_by_cwd() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let before = server.manager.list_agents().len();
    let req = RequestId::new();
    c.send(&WireCommand::SpawnByCwd {
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        backend: Some(WireBackendKind::Claude),
        request_id: req,
    })
    .await;
    let agent = c.await_spawned(req).await;
    assert!(
        server.manager.agent_epoch(agent.id).is_some(),
        "the agent.id attached to Spawned must be alive in the manager"
    );
    assert!(
        server.manager.list_agents().len() > before,
        "the ad-hoc agent must be added to the manager"
    );

    server.shutdown().await;
}

// ── case 41: WS GetSnapshot → Snapshot (req echo, chunks) — dedicated reply, no Ack ────
#[tokio::test]
async fn case41_ws_get_snapshot() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);

    server
        .manager
        .write_stdin(id, b"echo SNAP41\r\n", InputOrigin::User)
        .unwrap();
    wait_for_output(&server, id, 1).await;

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req = RequestId::new();
    c.send(&WireCommand::GetSnapshot {
        agent_id: id,
        request_id: req,
    })
    .await;
    let (aid, chunks) = c.await_snapshot(req).await;
    assert_eq!(aid, id, "agent_id echo of Snapshot");
    assert!(
        !chunks.is_empty(),
        "the accumulated output must arrive as snapshot chunks"
    );

    server.shutdown().await;
}

// ── case 42: WS SpawnProfile with a nonexistent profile_id → Error (req echo) ────────────────────
#[tokio::test]
async fn case42_ws_spawn_profile_unknown_error() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req = RequestId::new();
    c.send(&WireCommand::SpawnProfile {
        profile_id: Uuid::new_v4(),
        resume: false,
        request_id: req,
    })
    .await;
    let msg = c.await_error(req).await;
    assert!(
        msg.contains("profile not found"),
        "SpawnProfile of a nonexistent profile must be a not found Error: {msg}"
    );

    server.shutdown().await;
}

// ── case 43: WS SetProfileAutoRestore with a nonexistent profile_id → Error (req echo) ────────────
#[tokio::test]
async fn case43_ws_set_auto_restore_unknown_error() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req = RequestId::new();
    c.send(&WireCommand::SetProfileAutoRestore {
        profile_id: Uuid::new_v4(),
        auto_restore: true,
        request_id: req,
    })
    .await;
    let msg = c.await_error(req).await;
    assert!(
        msg.contains("profile not found"),
        "SetProfileAutoRestore of a nonexistent profile must be a not found Error: {msg}"
    );

    server.shutdown().await;
}

// ── case 44: WS GetSnapshot with a nonexistent agent_id → Error (req echo) ───────────────────────
#[tokio::test]
async fn case44_ws_get_snapshot_unknown_error() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req = RequestId::new();
    c.send(&WireCommand::GetSnapshot {
        agent_id: Uuid::new_v4(),
        request_id: req,
    })
    .await;
    let msg = c.await_error(req).await;
    assert!(
        msg.contains("not found") || !msg.is_empty(),
        "GetSnapshot of a nonexistent agent must be an Error: {msg}"
    );

    server.shutdown().await;
}

// ── case 45: while an activation runs the later command answers first (socket head-of-line blocking regression net) ────────
//
// The old shape: the read loop awaited `on_text` to completion, so until one activation (real process creation + link wait — typically 2 s, backstop 15 s) finished, the same client's other commands were not even taken off the socket and the two answers came in sent order.
// What flips this: activation now runs outside the connection's arrival-order lane (`connection_core::dispatch_order`) and `ListAgents` is handled straight in that lane; the jobs differ by 3 orders of magnitude (process creation vs an in-memory map lookup), so the assertion does not lean on timing slack.
// An e2e because the flip occurs only when socket → read row → inbound queue → handling row → dispatch all connect; the two unit tests (the pair in `agent_conn`) measure only each piece.
#[tokio::test]
async fn case45_ws_activation_does_not_block_the_next_command() {
    let server = start_test_server().await.unwrap();
    let profile_id = register_shell_profile(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let spawn_req = RequestId::new();
    let list_req = RequestId::new();
    c.send(&WireCommand::Spawn {
        profile_id,
        request_id: spawn_req,
    })
    .await;
    c.send(&WireCommand::ListAgents {
        request_id: list_req,
    })
    .await;

    let mut order: Vec<&str> = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while order.len() < 2 && std::time::Instant::now() < deadline {
        match c.next().await {
            Some(Incoming::Event(AgentEvent::AgentList { request_id, .. }))
                if request_id == list_req =>
            {
                order.push("list")
            }
            Some(Incoming::Event(AgentEvent::Ack { request_id })) if request_id == spawn_req => {
                order.push("spawn")
            }
            Some(Incoming::Event(AgentEvent::Error {
                request_id: Some(rid),
                message,
            })) if rid == spawn_req => {
                panic!("Spawn failed — that is not what this case measures: {message}")
            }
            Some(_) => continue,
            None => break,
        }
    }
    assert_eq!(
        order,
        vec!["list", "spawn"],
        "the later ListAgents waited for the activation — socket head-of-line blocking regression"
    );

    server.shutdown().await;
}

// ── helper functions ──────────────────────────────────────────────────────────────────────

/// AgentListUpdated can additionally occur from spawn, so only Hello is guaranteed drained, plus the first 1 list.
async fn drain_handshake(c: &mut Client) {
    match c.next_event().await {
        AgentEvent::Hello { .. } => {}
        ev => panic!("expected Hello (handshake), got {ev:?}"),
    }
    loop {
        match c.next_event().await {
            AgentEvent::AgentListUpdated { .. } => break,
            _ => continue,
        }
    }
}

async fn wait_replay_complete(c: &mut Client, id: Uuid) {
    loop {
        match c.next().await.expect("dropped before ReplayComplete") {
            Incoming::Event(AgentEvent::ReplayComplete { agent_id, .. }) if agent_id == id => {
                return
            }
            _ => continue,
        }
    }
}

/// Collects the frame seqs of an id, stopping when the marker appears in the payload (including that frame's seq).
async fn collect_frame_seqs_until_marker(c: &mut Client, id: Uuid, marker: &str) -> Vec<u64> {
    let mut seqs = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        match c.next().await {
            Some(Incoming::Frame(aid, _, seq, payload)) if aid == id => {
                seqs.push(seq);
                if String::from_utf8_lossy(&payload).contains(marker) {
                    return seqs;
                }
            }
            Some(_) => continue,
            None => break,
        }
    }
    panic!(
        "timeout/close before marker '{marker}' arrived (collected {})",
        seqs.len()
    );
}

async fn collect_frames_until_marker(c: &mut Client, id: Uuid, marker: &str) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        match c.next().await {
            Some(Incoming::Frame(aid, _, _, payload)) if aid == id => {
                let hit = String::from_utf8_lossy(&payload).contains(marker);
                frames.push(payload);
                if hit {
                    return frames;
                }
            }
            Some(_) => continue,
            None => break,
        }
    }
    panic!("timeout/close before marker '{marker}' arrived");
}

/// The PTY streams all output from the first subscribe, so FromOldest replay+live must come from 0 with no gaps.
fn assert_seq_contiguous_from_zero(seqs: &[u64]) {
    let mut sorted = seqs.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    for (i, s) in sorted.iter().enumerate() {
        assert_eq!(*s, i as u64, "seq must be contiguous from 0: {sorted:?}");
    }
}

// ══════════════════════════════════════════════════════════════════════════════════
// Real-process-only cases (#[cfg(windows)] + #[ignore]) — cannot be verified in-process.
//
// Excluded from the default `cargo test` (#[ignore] — real OS, slow); run with:
//   cargo test -p engram-dashboard-daemon --test ws_e2e -- --ignored --nocapture
//
// Windows only: the single-instance lock/Job Object/child_pids are Windows implementations, so limited with #[cfg(windows)] (excluded from compilation elsewhere — no fake PASS).
// ══════════════════════════════════════════════════════════════════════════════════

#[cfg(windows)]
mod real_process {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};

    use engram_dashboard_base::testing::wait_until;
    use engram_dashboard_daemon::data_dir::DataLayout;
    use engram_dashboard_protocol::DaemonInfo;

    /// This account's token file in `data_dir` (ADR-0395) — `None` if it cannot be read.
    fn read_token_file(data_dir: &Path) -> Option<String> {
        let account = engram_dashboard_platform::account::current_account_id().ok()?;
        std::fs::read_to_string(DataLayout::new(data_dir).token_file(&account)).ok()
    }

    const DAEMON_EXE: &str = env!("CARGO_BIN_EXE_engram-dashboard-daemon");

    /// A unique isolation context per test. A unique data_dir (ENGRAM_DATA_DIR) suffices: since ADR-0134 the single-instance lock scope is the data folder, so different folders diverge both roster and lock.
    struct IsoCtx {
        data_dir: PathBuf,
    }

    fn fresh_iso(tag: &str) -> IsoCtx {
        use std::sync::atomic::{AtomicU64, Ordering};
        // Mix in an in-process monotonic counter so two calls in the same nanosecond do not collide.
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let uniq = format!("{tag}-{nanos}-{n}");
        let dir = std::env::temp_dir().join(format!("engram-step7-{uniq}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp data_dir creation");
        IsoCtx { data_dir: dir }
    }

    fn spawn_daemon_iso(ctx: &IsoCtx) -> Child {
        spawn_daemon_in(&ctx.data_dir)
    }

    /// A spawn that explicitly injects data_dir (used when the single-instance test launches two on the same folder).
    fn spawn_daemon_in(data_dir: &Path) -> Child {
        Command::new(DAEMON_EXE)
            .env("ENGRAM_DATA_DIR", data_dir)
            .env("RUST_LOG", "info")
            .stdin(std::process::Stdio::null())
            // stdout capture (diagnostics): the agent's tracing fmt::layer() writes to stdout by default; stdout is piped to quote it when the daemon fails to write daemon.json (lock refused? data_dir? panic?). The daemon never logs sensitive values like the token — only port/pid.
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("daemon .exe spawn")
    }

    /// Takes the handle and reads to the end — the daemon must already have exited (EOF) or been killed before the call, to avoid blocking.
    fn drain_logs(child: &mut Child) -> String {
        use std::io::Read;
        let mut buf = String::new();
        if let Some(mut out) = child.stdout.take() {
            let _ = out.read_to_string(&mut buf);
        }
        if let Some(mut err) = child.stderr.take() {
            let mut e = String::new();
            let _ = err.read_to_string(&mut e);
            if !e.is_empty() {
                buf.push_str("\n--- stderr ---\n");
                buf.push_str(&e);
            }
        }
        buf
    }

    fn poll_daemon_json(data_dir: &Path, deadline: std::time::Duration) -> Option<DaemonInfo> {
        let path = DataLayout::new(data_dir).daemon_file();
        let end = std::time::Instant::now() + deadline;
        while std::time::Instant::now() < end {
            if let Ok(bytes) = std::fs::read(&path) {
                if let Ok(info) = DaemonInfo::parse(&bytes) {
                    return Some(info);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        None
    }

    fn kill_daemon(child: &mut Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    /// At boot the daemon restores this profile via restore_all → a live PTY child (cmd.exe). ShellBackend loads program/args onto the PTY as is (no shim wrapping), so cmd.exe is the daemon's direct child and identifiable by child_pids(daemon_pid).
    fn write_restorable_shell_agents_json(data_dir: &Path) {
        // schema_version == 1 (`PROFILES` in persistence/mod.rs); shape pinned (regression detection).
        let profile = AgentProfile::new(
            "step7-restore-shell".into(),
            AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
            std::env::temp_dir(),
            vec![],
            true, // auto_restore=true → a boot-restore target
        );
        // ProfilesFile is a private struct so equivalent JSON is built directly (schema_version=1 + profiles array).
        let profiles_json = serde_json::to_string(&[profile]).expect("profile serialization");
        let file = format!("{{\"schema_version\":1,\"profiles\":{profiles_json}}}");
        let state = DataLayout::new(data_dir).daemon_state_dir();
        std::fs::create_dir_all(&state).expect("state folder");
        std::fs::write(state.join("agents.json"), file).expect("agents.json write");
    }

    // ── case1: daemon .exe kill → PTY child (cmd.exe) Job (KILL_ON_JOB_CLOSE) cleanup together ──────
    #[tokio::test]
    #[ignore = "needs a real process/Job — run with `-- --ignored` (Windows only)"]
    async fn ignored_daemon_kill_cleans_pty_child() {
        use engram_dashboard_platform::process::{
            child_pids, pid_alive_with_start_time, process_creation_time,
        };

        let ctx = fresh_iso("kill");
        let data_dir = ctx.data_dir.clone();
        write_restorable_shell_agents_json(&data_dir);
        let mut daemon = spawn_daemon_iso(&ctx);

        let info = match poll_daemon_json(&data_dir, std::time::Duration::from_secs(15)) {
            Some(i) => i,
            None => {
                let _ = daemon.kill();
                let _ = daemon.wait();
                let err = drain_logs(&mut daemon);
                let _ = std::fs::remove_dir_all(&data_dir);
                panic!("the daemon must publish daemon.json — daemon log:\n{err}");
            }
        };
        let daemon_pid = info.pid;
        assert!(daemon_pid != 0, "the daemon PID is valid");

        // Restore has a 3s early-exit window so wait generously.
        let mut child_set: Vec<u32> = Vec::new();
        let appeared = wait_until(std::time::Duration::from_secs(20), || {
            child_set = child_pids(daemon_pid);
            !child_set.is_empty()
        });
        if !appeared {
            kill_daemon(&mut daemon);
            let _ = std::fs::remove_dir_all(&data_dir);
            panic!(
                "the PTY child (cmd.exe) of the daemon (pid={daemon_pid}) could not be identified by OS tree enumeration — \
                 restore did not launch the child or ppid is not reflected. This case presupposes identifying a live child."
            );
        }
        let live_children: Vec<(u32, u64)> = child_set
            .iter()
            .copied()
            .filter_map(|p| process_creation_time(p).map(|ct| (p, ct)))
            .collect();
        assert!(
            !live_children.is_empty(),
            "there must be a live PTY child of the daemon before the kill (creation_time was looked up): {child_set:?}"
        );

        let _ = daemon.kill();
        let _ = daemon.wait();

        // Poll whether the child PIDs die together (Job cleanup may not be immediate, so allow slack).
        let all_dead = wait_until(std::time::Duration::from_secs(15), || {
            live_children
                .iter()
                .all(|&(p, ct)| !pid_alive_with_start_time(p, ct))
        });

        if !all_dead {
            for &(p, _) in &live_children {
                let _ = Command::new("taskkill")
                    .args(["/PID", &p.to_string(), "/F"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
        }
        let _ = std::fs::remove_dir_all(&data_dir);

        assert!(
            all_dead,
            "after the daemon kill the PTY child ({live_children:?}) must die together via the Job (KILL_ON_JOB_CLOSE)"
        );
    }

    // ── case2: single-instance — the second daemon is refused by the folder lock (quick normal exit + json unchanged) ──
    //
    // exit code: on duplicate instance.rs run() returns Ok(()) → main exits normally (exit 0), so assert "exit 0 + quick exit + json unchanged" (no special code dedicated to duplicates).
    #[tokio::test]
    #[ignore = "needs 2 real processes — run with `-- --ignored` (Windows only)"]
    async fn ignored_single_instance_second_rejected() {
        // Deliberately trigger a single-instance collision: A and B use the same data_dir (ADR-0134 — the lock scope is the folder). Other ignored tests use unique folders.
        let ctx = fresh_iso("single");
        let data_dir = ctx.data_dir.clone();

        let mut daemon_a = spawn_daemon_in(&data_dir);
        let info_a = match poll_daemon_json(&data_dir, std::time::Duration::from_secs(15)) {
            Some(i) => i,
            None => {
                let _ = daemon_a.kill();
                let _ = daemon_a.wait();
                let err = drain_logs(&mut daemon_a);
                let _ = std::fs::remove_dir_all(&data_dir);
                panic!("daemon A must publish daemon.json — daemon A log:\n{err}");
            }
        };

        let token_a = read_token_file(&data_dir);
        let mut daemon_b = spawn_daemon_in(&data_dir);
        let exited_fast = wait_until(std::time::Duration::from_secs(3), || {
            matches!(daemon_b.try_wait(), Ok(Some(_)))
        });

        let b_status = daemon_b.try_wait().ok().flatten();

        let info_after = poll_daemon_json(&data_dir, std::time::Duration::from_secs(2));
        let token_after = read_token_file(&data_dir);

        kill_daemon(&mut daemon_a);
        if b_status.is_none() {
            let _ = daemon_b.kill();
            let _ = daemon_b.wait();
        }
        let b_logs = drain_logs(&mut daemon_b);
        let _ = std::fs::remove_dir_all(&data_dir);

        assert!(
            exited_fast,
            "the second daemon must exit quickly (within 3s) by lock refusal — B log:\n{b_logs}"
        );
        if let Some(status) = b_status {
            assert!(
                status.success(),
                "the second daemon must exit normally (exit 0) — got {status:?}, B log:\n{b_logs}"
            );
        } else {
            panic!("the second daemon did not exit within 3s (lock refusal may have failed) — B log:\n{b_logs}");
        }
        let info_after = info_after.expect("A's daemon.json must be kept");
        assert_eq!(
            info_after.pid, info_a.pid,
            "the second daemon must not overwrite daemon.json (pid unchanged)"
        );
        assert!(token_a.is_some(), "A must write its token file");
        assert_eq!(
            token_after, token_a,
            "the token file too is unchanged (B must not issue a new token)"
        );
    }

    // ── case3: stale daemon.json → the daemon detects stale and overwrites with its own info ────────────────
    //
    // The src-tauri ensure_daemon (WMI spawn) path is filled by a separate test (shell discovery::tests::real_wmi_spawn_smoke) — a src-tauri function cannot be called from the daemon crate.
    #[tokio::test]
    #[ignore = "needs a real process + file discovery — run with `-- --ignored` (Windows only)"]
    async fn ignored_stale_daemon_json_discovery() {
        let ctx = fresh_iso("stale");
        let data_dir = ctx.data_dir.clone();

        let mut tmp_child = Command::new("cmd.exe")
            .args(["/c", "exit"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("temp child spawn");
        let dead_pid = tmp_child.id();
        let _ = tmp_child.wait(); // termination guaranteed → dead_pid is now dead.

        let stale = DaemonInfo {
            pid: dead_pid,
            host: "127.0.0.1".into(),
            port: 59999,
            protocol_version: PROTOCOL_VERSION,
            // After ADR-0134 the stale judgment is for diagnostic logs, so this value does not decide whether to start — the right to overwrite comes from the folder lock. The value stays to keep the realistic "dead pid + mismatched creation time" combination.
            start_time: 0xDEAD_BEEF,
            owner_account: String::new(),
        };
        let stale_json = serde_json::to_vec_pretty(&stale).expect("stale serialization");
        let record_path = DataLayout::new(&data_dir).daemon_file();
        std::fs::create_dir_all(record_path.parent().expect("run folder"))
            .expect("run folder creation");
        std::fs::write(&record_path, &stale_json).expect("stale daemon.json write");
        // The stale run's token file, with the folder's inherited ACL (as a pre-S37 folder or a planted file would have).
        let account = engram_dashboard_platform::account::current_account_id().expect("account");
        let token_path = DataLayout::new(&data_dir).token_file(&account);
        std::fs::write(&token_path, "d".repeat(64)).expect("stale token file write");

        let mut daemon = spawn_daemon_iso(&ctx);

        let mut latest: Option<DaemonInfo> = None;
        // ADR-0395: the token file is replaced before the record is published — read the token the moment the new record is first seen.
        //   (Probabilistic pin: with the order reversed, this read could land in the window where the stale token is still in place.)
        let mut fresh_token = None;
        let overwritten = wait_until(std::time::Duration::from_secs(15), || {
            latest = poll_daemon_json(&data_dir, std::time::Duration::from_millis(100));
            let seen = matches!(&latest, Some(i) if i.pid != dead_pid);
            if seen {
                fresh_token = read_token_file(&data_dir);
            }
            seen
        });

        let token_owner_only =
            engram_dashboard_platform::owner_only::is_owner_only(&token_path).ok();
        kill_daemon(&mut daemon);
        let err = drain_logs(&mut daemon);
        let _ = std::fs::remove_dir_all(&data_dir);

        assert!(
            overwritten,
            "the daemon must overwrite the stale daemon.json with its own info (pid must differ from stale dead_pid={dead_pid}) — daemon log:\n{err}"
        );
        let fresh = latest.expect("the overwritten daemon.json");
        assert_ne!(
            fresh.pid, dead_pid,
            "the new pid must differ from stale dead_pid"
        );
        assert!(
            fresh.start_time != 0,
            "the new daemon must record a valid start_time"
        );
        let fresh_token =
            fresh_token.expect("the new daemon's token file, there when its record first appears");
        assert_ne!(
            fresh_token,
            "d".repeat(64),
            "the new daemon must issue a new token before it publishes its record (keeping the stale token is forbidden)"
        );
        assert_eq!(fresh_token.len(), 64);
        assert_eq!(
            token_owner_only,
            Some(true),
            "the wider stale file is replaced by an owner-only one"
        );
        assert!(fresh.port != 0, "the new daemon must record a valid port");
    }
}
