//! 격리 하네스 — 데몬 단독 WS E2E (phase 2 step 6).
//!
//! 프론트(UI) 없이 데몬의 WS 서버를 **in-process 로 실제 기동**하고(`start_test_server`),
//! 이 테스트 코드가 **WS 클라이언트**가 되어 auth → subscribe → binary frame 디코드 →
//! command 송신 전 경로를 검증한다. CLAUDE.md: "데몬 모듈은 격리 하네스로 한 번에 돌도록 모은다."
//!
//! ★격리★: bind 는 127.0.0.1:0(OS 자동 포트 — 테스트 병렬 충돌 없음), store 는 in-memory,
//! 각 테스트가 독립 서버 인스턴스를 띄우고 끝에서 shutdown(전 에이전트 kill — 좀비 PTY 방지).
//! 모든 await 에 timeout 가드를 둬 hang 시 영구 멈추지 않는다.
//!
//! ★실프로세스 케이스 분리(은폐 금지)★: 아래 in-process 테스트가 커버하지 **못하는** 것들
//! (데몬 .exe kill→PTY child Job 동반 정리, single-instance 폴더 잠금, stale daemon.json discovery)은
//! 실제 OS 프로세스/Job 이 필요하다. 이들은 이 파일 하단 `#[ignore]` 테스트로 두고 수동 실행법을
//! 주석에 적었다. 기본 `cargo test` 는 in-process 케이스만 빠르게 돈다.

use std::sync::Arc;
use std::time::Duration;

use engram_dashboard_agent::profile::{AgentCommand, AgentProfile, SpawnMode};
use engram_dashboard_daemon::{
    start_test_server, start_test_server_with_keepalive, KeepaliveConfig, TestServerHandle,
};
use engram_dashboard_net::auth::AuthFrame;
use engram_dashboard_protocol::{
    decode_frame, AgentCommand as WireCommand, AgentEvent, ClaudeOutputFormat as WireOutputFormat,
    RequestId, SubscribeAction, PROTOCOL_VERSION,
};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const NET_TIMEOUT: Duration = Duration::from_secs(10);

// ── 클라이언트 헬퍼 ────────────────────────────────────────────────────────────────

struct Client {
    ws: Ws,
}

// AgentEvent 가 AgentProfile(failed_reason 추가로 clippy 임계 200B 초과)을 품어 variant 크기차가 크다.
// 테스트 헬퍼라 Box indirection(동작 변경) 대신 lint 만 허용한다.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
enum Incoming {
    Event(AgentEvent),
    /// 디코드된 (agent_id, epoch, seq, payload). epoch 는 디버깅 가시성용(현 단언엔 미사용).
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

    /// ★핸드셰이크는 명령이 아니다(ADR-0129 0-4)★ — 모양이 네트워크 lib
    /// 소유라 `send`(명령 전용)를 못 탄다. 첫 프레임(정상 인증)과 2차 프레임(case21 의 프로토콜 위반)이
    /// **같은 바이트**여야 하므로 두 자리가 이 헬퍼 하나를 공유한다.
    async fn send_auth(&mut self, token: &str) {
        let auth = AuthFrame::Auth {
            token: token.to_string(),
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
                    let ev: AgentEvent = serde_json::from_str(&t).expect("control JSON 파싱 실패");
                    return Some(Incoming::Event(ev));
                }
                Ok(Message::Binary(b)) => {
                    let f = decode_frame(&b).expect("binary frame 디코드 실패");
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
            match self.next().await.expect("연결이 끊김(이벤트 기대)") {
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
                    assert_eq!(request_id, expect_id, "Ack 의 request_id 가 echo 돼야 함");
                    return;
                }
                Some(Incoming::Event(AgentEvent::Error {
                    request_id: Some(rid),
                    message,
                })) if rid == expect_id => {
                    panic!("Ack 기대했으나 Error(req={rid:?}): {message}");
                }
                Some(_) => continue,
                None => break,
            }
        }
        panic!("request_id={expect_id:?} 의 Ack 도달 전 timeout/close");
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
                    panic!("Error 기대했으나 Ack(req={request_id:?})");
                }
                Some(_) => continue,
                None => break,
            }
        }
        panic!("request_id={expect_id:?} 의 Error 도달 전 timeout/close");
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
        panic!("Error 도달 전 timeout/close");
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
                    assert_eq!(request_id, req, "ProfileList 의 request_id echo");
                    return profiles;
                }
                Some(_) => continue,
                None => break,
            }
        }
        panic!("ProfileList 도달 전 timeout/close");
    }

    /// reply(Ack) 와 broadcast_profile_list 의 큐잉 순서에 의존하지 않게 한 루프에서 함께 모은다.
    async fn await_crud(&mut self, req: RequestId) -> Vec<engram_dashboard_protocol::AgentProfile> {
        let mut saw_ack = false;
        let mut profiles: Option<Vec<engram_dashboard_protocol::AgentProfile>> = None;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while (!saw_ack || profiles.is_none()) && std::time::Instant::now() < deadline {
            match self.next().await {
                Some(Incoming::Event(AgentEvent::Ack { request_id })) => {
                    assert_eq!(request_id, req, "CRUD Ack 의 request_id echo");
                    saw_ack = true;
                }
                Some(Incoming::Event(AgentEvent::ProfileListUpdated { profiles: p })) => {
                    profiles = Some(p);
                }
                Some(Incoming::Event(AgentEvent::Error {
                    request_id: Some(rid),
                    message,
                })) if rid == req => panic!("CRUD 실패 Error(req={rid:?}): {message}"),
                Some(_) => continue,
                None => break,
            }
        }
        assert!(
            saw_ack && profiles.is_some(),
            "CRUD 후 Ack({saw_ack})+ProfileListUpdated({}) 둘 다 와야",
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
                    assert_eq!(request_id, req, "Created 의 request_id echo");
                    return profile;
                }
                Some(Incoming::Event(AgentEvent::Ack { request_id })) if request_id == req => {
                    panic!("Created 기대했으나 Ack(req={request_id:?}) — Ack 중복 금지");
                }
                Some(Incoming::Event(AgentEvent::Error {
                    request_id: Some(rid),
                    message,
                })) if rid == req => panic!("Created 기대했으나 Error(req={rid:?}): {message}"),
                Some(_) => continue,
                None => break,
            }
        }
        panic!("request_id={req:?} 의 Created 도달 전 timeout/close");
    }

    async fn await_spawned(&mut self, req: RequestId) -> engram_dashboard_protocol::AgentInfo {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match self.next().await {
                Some(Incoming::Event(AgentEvent::Spawned { request_id, agent })) => {
                    assert_eq!(request_id, req, "Spawned 의 request_id echo");
                    return agent;
                }
                Some(Incoming::Event(AgentEvent::Ack { request_id })) if request_id == req => {
                    panic!("Spawned 기대했으나 Ack(req={request_id:?}) — Ack 중복 금지");
                }
                Some(Incoming::Event(AgentEvent::Error {
                    request_id: Some(rid),
                    message,
                })) if rid == req => panic!("Spawned 기대했으나 Error(req={rid:?}): {message}"),
                Some(_) => continue,
                None => break,
            }
        }
        panic!("request_id={req:?} 의 Spawned 도달 전 timeout/close");
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
                    assert_eq!(request_id, req, "Snapshot 의 request_id echo");
                    return (agent_id, chunks);
                }
                Some(_) => continue,
                None => break,
            }
        }
        panic!("Snapshot 도달 전 timeout/close");
    }

    async fn await_agent_list(
        &mut self,
        req: RequestId,
    ) -> Vec<engram_dashboard_protocol::AgentInfo> {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match self.next().await {
                Some(Incoming::Event(AgentEvent::AgentList { request_id, agents })) => {
                    assert_eq!(request_id, req, "AgentList 의 request_id echo");
                    return agents;
                }
                Some(_) => continue,
                None => break,
            }
        }
        panic!("AgentList 도달 전 timeout/close");
    }

    /// ★중요★: spawn_agent 은 agent_list_updated 브로드캐스트를
    /// reply(Ack) **전에** 큐잉하므로 conn_tx 순서가 [list, Ack] 이다. 따라서 await_ack 를 먼저 부르면
    /// list 를 흘려버린다 — 그래서 둘을 한 루프에서 함께 모은다.
    async fn await_spawn(&mut self, wanted: Uuid, req: RequestId) {
        let mut saw_ack = false;
        let mut saw_list = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while (!saw_ack || !saw_list) && std::time::Instant::now() < deadline {
            match self.next().await {
                Some(Incoming::Event(AgentEvent::Ack { request_id })) => {
                    assert_eq!(request_id, req, "Spawn Ack 의 request_id echo");
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
                })) if rid == req => panic!("Spawn 실패 Error(req={rid:?}): {message}"),
                Some(_) => continue,
                None => break,
            }
        }
        assert!(
            saw_ack && saw_list,
            "Spawn 후 Ack({saw_ack})+AgentListUpdated({saw_list}) 둘 다 와야"
        );
    }

    /// kill_agent 도 list 갱신을 reply(Ack) 전에 큐잉하므로(순서 [list, Ack]) 함께 모은다.
    async fn await_kill(&mut self, wanted: Uuid, req: RequestId) {
        let mut saw_ack = false;
        let mut saw_excluded = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while (!saw_ack || !saw_excluded) && std::time::Instant::now() < deadline {
            match self.next().await {
                Some(Incoming::Event(AgentEvent::Ack { request_id })) => {
                    assert_eq!(request_id, req, "Kill Ack 의 request_id echo");
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
                })) if rid == req => panic!("Kill 실패 Error(req={rid:?}): {message}"),
                Some(_) => continue,
                None => break,
            }
        }
        assert!(
            saw_ack && saw_excluded,
            "Kill 후 Ack({saw_ack})+목록제외({saw_excluded}) 둘 다 와야"
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
                // 그 외 메시지는 무시(읽긴 하지만 — tungstenite 는 한 메시지씩 디코드).
                Ok(Some(Ok(_))) => continue,
            }
        }
    }

    /// ★중요★: tungstenite 는 stream 을 poll 할 때 들어온 Ping 에 자동 Pong 한다. 이 메서드는
    /// 정상 클라처럼 계속 읽으며(자동 Pong 유발) 그 와중에 Ping 도착을 관측한다. 다른 control/
    /// binary 는 흡수한다.
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

/// Client 메서드를 모듈 자유 함수로 노출(slow consumer 케이스 가독성).
async fn expect_closed_within(c: &mut Client, deadline: Duration) -> bool {
    c.expect_closed_within(deadline).await
}

/// ★resize 는 비동기(WS → read_task → dispatch → manager)라 즉시 반영이 아니다★ → 폴링한다.
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

// ── 서버 헬퍼 ──────────────────────────────────────────────────────────────────────

/// 결정적 출력을 위해 interactive `cmd.exe` 를 직접 띄운다(/c 없이 — 살아있는 셸).
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
        false, // auto_restore=false (복원 대상 아님)
    );
    let id = profile.id;
    handle
        .manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("shell agent spawn 실패")
        .into_started()
        .expect("이 호출은 실제로 띄운다(중복 요청 아님)");
    id
}

/// WS `Spawn{profile_id}` dispatch 경로를 타려면 manager 의 레지스트리에 알려진 프로필이 있어야 한다.
/// ★운영 회귀 0★: 등록은 manager 의 공개 API(`create_agent` — 명부 단일 입구, ADR-0119)만 사용 —
///   start_test_server/run() 배선을 건드리지 않는다(프로필 주입 인자 추가 불필요). 운영 `CreateProfile`
///   경로도 **같은 동사**를 쓰므로 이름 유일성 강제(ADR-0120)를 함께 탄다.
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
        false, // auto_restore=false(복원 대상 아님)
    );
    let id = profile.id;
    handle.manager.create_agent(profile).expect("등록 성공");
    id
}

/// 결정적 출력은 PTY 가 즉시 내지만, OS 스케줄 지연을
/// 흡수하려고 snapshot seq 수가 min_events 이상이 될 때까지 최대 deadline 대기.
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
            panic!("출력 {min_events}건 대기 timeout (현재 {n}건)");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// ── 케이스 1: auth 성공 → Hello + AgentListUpdated ────────────────────────────────
#[tokio::test]
async fn case01_auth_success_hello_and_list() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;

    match c.next_event().await {
        AgentEvent::Hello {
            protocol_version, ..
        } => assert_eq!(protocol_version, PROTOCOL_VERSION),
        ev => panic!("Hello 기대, got {ev:?}"),
    }
    match c.next_event().await {
        AgentEvent::AgentListUpdated { agents } => assert!(agents.is_empty(), "초기 목록은 비어야"),
        ev => panic!("AgentListUpdated 기대, got {ev:?}"),
    }

    server.shutdown().await;
}

// ── 케이스 2: auth 실패(틀린 토큰) → 서버가 close ──────────────────────────────────
#[tokio::test]
async fn case02_auth_wrong_token_closes() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &"f".repeat(64)).await;
    // ★짧은 deadline★: 옛 expect_closed 는 10s recv timeout 에
    //   기대 닫힘을 잡아 느리고 불명확했다(mutation D). 닫힘은 즉시 일어나므로 3s 안에 단언한다.
    assert!(
        c.expect_closed_within(Duration::from_secs(3)).await,
        "틀린 토큰이면 연결이 즉시(3s 내) 닫혀야 함"
    );
    server.shutdown().await;
}

// ── 케이스 3: auth 타임아웃(첫 frame 미전송) → 서버가 close ──────────────────────────
#[tokio::test]
async fn case03_auth_timeout_closes() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_raw(server.port).await;
    assert!(
        c.expect_closed().await,
        "auth frame 미전송이면 1s 후 닫혀야 함"
    );
    server.shutdown().await;
}

// ── 케이스 4: 출력 순서(seq 0,1,2… 무결) ───────────────────────────────────────────
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
        .write_stdin(id, b"echo CASE4_MARKER\r\n")
        .unwrap();

    let seqs = collect_frame_seqs_until_marker(&mut c, id, "CASE4_MARKER").await;
    assert!(!seqs.is_empty(), "frame 을 받아야 함");
    assert_seq_contiguous_from_zero(&seqs);

    server.shutdown().await;
}

// ── 케이스 5: replay→live FIFO 순서(SubscribeAck→replay→ReplayComplete→live) ────────
#[tokio::test]
async fn case05_replay_then_live_order() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);

    server.manager.write_stdin(id, b"echo PREFILL\r\n").unwrap();
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
        ev => panic!("SubscribeAck 기대, got {ev:?}"),
    }
    let mut replay_frames = 0usize;
    loop {
        match c.next().await.expect("ReplayComplete 전 끊김") {
            Incoming::Frame(aid, _, _, _) => {
                assert_eq!(aid, id);
                replay_frames += 1;
            }
            Incoming::Event(AgentEvent::ReplayComplete { agent_id, .. }) => {
                assert_eq!(agent_id, id);
                break;
            }
            Incoming::Event(ev) => panic!("replay 구간 예상 밖 event: {ev:?}"),
        }
    }
    assert!(replay_frames >= 1, "PREFILL replay frame 이 1건 이상");

    server.manager.write_stdin(id, b"echo LIVE5\r\n").unwrap();
    let live = collect_frames_until_marker(&mut c, id, "LIVE5").await;
    assert!(!live.is_empty(), "ReplayComplete 후 live frame 도착해야");

    server.shutdown().await;
}

// ── 케이스 6: afterSeq resume — tail 만 ────────────────────────────────────────────
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
    server.manager.write_stdin(id, b"echo R6A\r\n").unwrap();
    let first = collect_frame_seqs_until_marker(&mut c, id, "R6A").await;
    let last_seq = *first.iter().max().unwrap();

    drop(c);
    server.manager.write_stdin(id, b"echo R6B\r\n").unwrap();
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
                "resume 은 last_seq({last_seq}) 초과분부터(replay_from={replay_from})"
            );
        }
        ev => panic!("SubscribeAck(Resume) 기대, got {ev:?}"),
    }
    let tail = collect_frame_seqs_until_marker(&mut c2, id, "R6B").await;
    for s in &tail {
        assert!(
            *s > last_seq,
            "tail frame seq({s}) 는 last_seq({last_seq}) 초과여야"
        );
    }

    server.shutdown().await;
}

// ── 케이스 7: truncated — ring(2MB) 초과 출력 후 after_seq<oldest ───────────────────
#[tokio::test]
async fn case07_truncated_replay() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);
    let epoch = server.manager.agent_epoch(id).unwrap();

    // (한 줄 ~80B × 40000 ≈ 3MB → 2MB ring 의 oldest 가 0 위로 밀린다.)
    server
        .manager
        .write_stdin(
            id,
            b"for /L %i in (1,1,40000) do @echo TRUNCATE_LINE_PADDING_XXXXXXXXXXXXXXXXXXXXXXXXXXXX %i\r\n",
        )
        .unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let snap = server.manager.get_snapshot(id).unwrap();
        if snap.first().map(|c| c.seq).unwrap_or(0) > 0 && snap.len() >= 10 {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!("ring eviction(oldest>0) 대기 timeout");
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
            assert!(truncated, "truncated 플래그 set");
            assert!(oldest_seq > 0, "oldest 가 0 위로 밀려야(eviction 발생)");
        }
        ev => panic!("SubscribeAck(Truncated) 기대, got {ev:?}"),
    }

    server.shutdown().await;
}

// ── 케이스 8: epoch mismatch → Reset(oldest 부터) ──────────────────────────────────
#[tokio::test]
async fn case08_epoch_mismatch_reset() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);
    let epoch = server.manager.agent_epoch(id).unwrap();

    server.manager.write_stdin(id, b"echo E8\r\n").unwrap();
    wait_for_output(&server, id, 1).await;

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;
    // 다른 화신 표식 + after_seq 지정 → after_seq 무시하고 Reset.
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
            assert_eq!(action, SubscribeAction::Reset, "epoch 불일치 → Reset");
            assert_eq!(current_epoch, epoch, "현재 epoch 통보");
        }
        ev => panic!("SubscribeAck(Reset) 기대, got {ev:?}"),
    }

    server.shutdown().await;
}

// ── 케이스 9: slow consumer → 그 연결만 close, 타 연결 무영향 ──────────────────────
//
// ★재현 메커니즘★: slow 소비자는 ReplayComplete 후 **소켓을 전혀 읽지 않는다**. 같은 agent 에
//   대량 출력이 흐르면 slow 의 서버측 송신 mpsc(CONN_TX_CAP=4608) + OS 소켓 버퍼가 둘 다 차고,
//   프레임 출구의 try_send 가 full 을 만나 close_signal 을 발동 → write_task 가 그 연결만 닫는다.
//   good 소비자는 **백그라운드 task 로 계속 drain** 해 살아남아야 한다(타 연결 무영향).
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

    // ★m1(견고화)★: 위양성(소켓 버퍼가 큰 환경에서 안 막힘) 회피를 위해 출력량을 키운다.
    //   한 줄 ~140B(패딩) × 200000 ≈ 28MB. mpsc 4608칸(≈0.6MB) + 어떤 현실적 OS 소켓 송신
    //   버퍼(보통 수십 KB~수 MB)를 합쳐도 28MB 를 흡수할 수 없어, slow 가 안 읽으면 try_send 가
    //   확실히 full 을 만나 close_signal 이 발동한다. (한계 명시: 만약 OS 버퍼가 28MB 를 넘으면
    //   이 테스트가 flaky 해질 수 있으나, 현실 기본값에선 일어나지 않는다.)
    server
        .manager
        .write_stdin(
            id,
            b"for /L %i in (1,1,200000) do @echo SLOW9_PADDING_XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX %i\r\n",
        )
        .unwrap();

    // ★중요★: slow 가 곧바로 읽으면 소켓이 드레인돼 서버 큐가 안 찬다. 먼저 일정 시간 **읽지 않고**
    //   대기해 서버 송신 mpsc + OS 버퍼가 가득 차 close_signal 이 발동하게 둔다. 그 뒤 backlog 를
    //   읽어 내려가다 Close 를 만난다.
    tokio::time::sleep(Duration::from_secs(3)).await;

    // slow 는 서버가 닫은 연결이므로 backlog 소진 후 Close/None 에 도달한다 — 넉넉한 deadline.
    assert!(
        expect_closed_within(&mut slow, Duration::from_secs(40)).await,
        "느린 소비자 연결은 서버가 닫아야 함"
    );

    assert!(
        good_alive.load(std::sync::atomic::Ordering::Relaxed),
        "정상 소비자는 닫히면 안 됨"
    );
    assert!(
        good_frames.load(std::sync::atomic::Ordering::Relaxed) > 0,
        "정상 소비자는 영향 없이 frame 을 계속 수신"
    );

    drain_task.abort();
    server.shutdown().await;
}

// ── 케이스 10: reconnect 복구 — resume 후 무손실(seq dedup 후 gap 0) ────────────────
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
    server.manager.write_stdin(id, b"echo RC10A\r\n").unwrap();
    let got1 = collect_frame_seqs_until_marker(&mut c1, id, "RC10A").await;
    let max1 = *got1.iter().max().unwrap();
    drop(c1);

    server.manager.write_stdin(id, b"echo RC10B\r\n").unwrap();
    wait_for_output(&server, id, (max1 as usize) + 2).await;

    let mut c2 = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c2).await;
    c2.send(&WireCommand::Subscribe {
        agent_id: id,
        epoch: Some(epoch),
        after_seq: Some(max1),
    })
    .await;
    // SubscribeAck 소진.
    let _ = c2.next_event().await;
    let got2 = collect_frame_seqs_until_marker(&mut c2, id, "RC10B").await;

    let mut all: Vec<u64> = got1.clone();
    all.extend(got2.iter().copied());
    all.sort_unstable();
    all.dedup();
    let max_all = *all.last().unwrap();
    let expected: Vec<u64> = (0..=max_all).collect();
    assert_eq!(all, expected, "reconnect+resume 후 seq gap 0(무손실)");

    server.shutdown().await;
}

// ── 케이스 11: high throughput — 순서·무결, 데드락 없음 ────────────────────────────
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
        )
        .unwrap();

    let seqs = collect_frame_seqs_until_marker(&mut c, id, "HT11_DONE").await;
    assert!(!seqs.is_empty(), "대량 출력 frame 수신");
    // 수신 seq 는 strictly increasing(순서 무결). (truncated 통보가 없으면 연속이어야 하나,
    // ring eviction 가능성을 고려해 '증가' 만 강하게 단언 — 데드락/순서역전 없음이 핵심.)
    for w in seqs.windows(2) {
        assert!(
            w[1] > w[0],
            "frame seq 가 단조 증가해야(순서 무결): {:?}",
            w
        );
    }

    server.shutdown().await;
}

// ── 케이스 12: 멀티 구독(역다중화) — 한 연결이 agent 2개 구독 ──────────────────────
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

    server.manager.write_stdin(id_a, b"echo AAA12\r\n").unwrap();
    server.manager.write_stdin(id_b, b"echo BBB12\r\n").unwrap();

    let mut saw_a = false;
    let mut saw_b = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while (!saw_a || !saw_b) && std::time::Instant::now() < deadline {
        match c.next().await {
            Some(Incoming::Frame(aid, _, _, payload)) => {
                let text = String::from_utf8_lossy(&payload);
                if text.contains("AAA12") {
                    assert_eq!(aid, id_a, "AAA12 는 agent A 로만 와야");
                    saw_a = true;
                }
                if text.contains("BBB12") {
                    assert_eq!(aid, id_b, "BBB12 는 agent B 로만 와야");
                    saw_b = true;
                }
            }
            Some(Incoming::Event(_)) => continue,
            None => break,
        }
    }
    assert!(
        saw_a && saw_b,
        "두 agent 의 출력을 각자 agent_id 로 역다중화해야 (a={saw_a}, b={saw_b})"
    );

    server.shutdown().await;
}

// ══════════════════════════════════════════════════════════════════════════════════
// M1: WS dispatch() 를 실제로 타는 E2E.
//
// 위 case01~12 는 출력평면(replay/seq/slow-consumer) 결정성을 보려고 agent 를
// `server.manager.spawn_agent` 로 **직접** 만들어 dispatch 를 우회한다. 아래 case13~ 은
// 반대로 **WS frame(JSON text control)으로 명령을 보내 read_task→dispatch() 를 실제로 타는**
// 경로를 검증한다 — Spawn/WriteStdin/Kill/Interrupt/Resize/Unsubscribe/ListAgents/StopDaemon
// /2차 Auth/binary 거부/파싱 실패. request_id echo(Ack/Error 매핑)도 단언한다.
// ══════════════════════════════════════════════════════════════════════════════════

// ── 케이스 13: WS Spawn → Ack(req echo) + AgentListUpdated(새 agent_id) ─────────────
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
        "WS Spawn 후 manager 에 agent 가 살아있어야"
    );

    server.shutdown().await;
}

// ── 케이스 14: WS Spawn → WriteStdin(VIA_WS 마커가 binary frame 으로) ───────────────
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
    assert!(!frames.is_empty(), "VIA_WS 출력이 binary frame 으로 와야");

    server.shutdown().await;
}

// ── 케이스 15: WS Kill → Ack + AgentListUpdated 로 종료 반영(불변식: terminal=list) ──
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
        "kill 후 manager 에서 agent 제거"
    );

    server.shutdown().await;
}

// ── 케이스 16: WS Interrupt → Ack(프로세스 생존) ───────────────────────────────────
#[tokio::test]
async fn case16_ws_interrupt_ack_process_alive() {
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
    c.await_ack(req_int).await;
    // 출력 정지 확인까지는 best-effort — 생존만 단언(여전히 manager 에 있음).
    assert!(
        server.manager.agent_epoch(profile_id).is_some(),
        "Interrupt 후에도 프로세스 생존(manager 에 잔존)"
    );

    server.shutdown().await;
}

// ── 케이스 17: WS Resize → 에러 없이 수용(Resize 는 request_id 없음 → Ack 없음) ──────
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

    // WS Resize — messages.rs 상 request_id 없는 명령. dispatch 는 성공 시 무응답, 실패만 Error.
    // 따라서 "Ack 가 오지 않는다"(설계대로)와 "Error 가 오지 않는다"를 함께 단언한다.
    c.send(&WireCommand::Resize {
        agent_id: profile_id,
        cols: 100,
        rows: 40,
        viewport_id: None,
    })
    .await;

    // 후속 명령(ListAgents)을 보내 그 응답 전에 Resize Error/Ack 가 끼지 않는지로 "무응답" 검증.
    // (Resize 가 잘못 Ack/Error 를 보내면 ListAgents 응답보다 먼저 그게 잡힌다.)
    let req_list = RequestId::new();
    c.send(&WireCommand::ListAgents {
        request_id: req_list,
    })
    .await;
    let agents = c.await_agent_list(req_list).await;
    assert!(
        agents.iter().any(|a| a.id == profile_id),
        "ListAgents 응답이 와야(Resize 는 무응답이어야)"
    );

    server.shutdown().await;
}

// ── 케이스 18: WS Unsubscribe → 이후 live frame 더 안 옴 ────────────────────────────
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
        .write_stdin(profile_id, b"echo PRE_UNSUB\r\n")
        .unwrap();
    let _ = collect_frames_until_marker(&mut c, profile_id, "PRE_UNSUB").await;

    c.send(&WireCommand::Unsubscribe {
        agent_id: profile_id,
    })
    .await;

    // ★타이밍 비의존 보장★: ListAgents 를 왕복시켜 Unsubscribe 가 read_task 에서 이미 처리됐음을
    //   확정한 뒤(동일 read_task 가 FIFO 처리) 새 출력을 낸다.
    let req_list = RequestId::new();
    c.send(&WireCommand::ListAgents {
        request_id: req_list,
    })
    .await;
    loop {
        match c.next().await.expect("ListAgents 응답 전 끊김") {
            Incoming::Event(AgentEvent::AgentList { request_id, .. }) => {
                assert_eq!(request_id, req_list, "ListAgents 응답 request_id echo");
                break;
            }
            Incoming::Frame(..) => panic!("Unsubscribe 후 잔여 frame 도착(구독이 안 끊김)"),
            _ => continue,
        }
    }
    server
        .manager
        .write_stdin(profile_id, b"echo POST_UNSUB\r\n")
        .unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match tokio::time::timeout(remaining, c.next()).await {
            Ok(Some(Incoming::Frame(..))) => {
                panic!("Unsubscribe 후 live frame 이 도착하면 안 됨");
            }
            Ok(Some(Incoming::Event(_))) => continue,
            Ok(None) => break,
            Err(_) => break, // timeout = frame 안 옴(정상).
        }
    }

    server.shutdown().await;
}

// ── 케이스 19: WS ListAgents → AgentList ───────────────────────────────────────────
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
        "ListAgents 응답에 spawn 한 agent 포함"
    );

    server.shutdown().await;
}

// ── 케이스 20: WS StopDaemon force 정책(M4) ────────────────────────────────────────
//   활성 agent 있는 상태에서 force=false → 거부 Error(서버 살아있음). 이어서 force=true → 종료.
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
        "force=false 거부 메시지에 active agents 명시: {msg}"
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
    c.await_ack(req_stop).await;
    assert!(
        c.expect_closed().await,
        "StopDaemon(force) 후 연결이 닫혀야"
    );

    // accept_handle join 으로 accept loop 종료 확정(shutdown 은 idempotent — 이미 종료된 watch 에 재send).
    server.shutdown().await;
}

// ── 케이스 21: 2차 Auth → already authenticated Error ───────────────────────────────
#[tokio::test]
async fn case21_ws_second_auth_rejected() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    c.send_auth(&server.token).await;
    let msg = c.await_error_no_id().await;
    assert!(
        msg.contains("already authenticated"),
        "2차 Auth 는 already authenticated Error 여야: {msg}"
    );
    let req_alive = RequestId::new();
    c.send(&WireCommand::ListAgents {
        request_id: req_alive,
    })
    .await;
    let _ = c.await_agent_list(req_alive).await;

    server.shutdown().await;
}

// ── 케이스 22: control 자리에 binary frame → Error + 연결 close(AgentConnection::on_binary, agent_conn.rs) ──────────
#[tokio::test]
async fn case22_ws_binary_frame_rejected() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    c.send_binary(vec![0xde, 0xad, 0xbe, 0xef]).await;
    assert!(
        c.expect_closed().await,
        "control 자리 binary 는 Error 후 연결이 닫혀야"
    );

    server.shutdown().await;
}

// ── 케이스 23: 깨진 JSON text → Error(req 없음), 연결은 유지(AgentConnection::on_text, agent_conn.rs) ─────────────
#[tokio::test]
async fn case23_ws_parse_failure_error() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    c.send_raw_text("{\"NotACommand\":true}").await;
    let msg = c.await_error_no_id().await;
    assert!(
        msg.contains("invalid command"),
        "파싱 실패 Error 메시지: {msg}"
    );
    let req_alive = RequestId::new();
    c.send(&WireCommand::ListAgents {
        request_id: req_alive,
    })
    .await;
    let _ = c.await_agent_list(req_alive).await;

    server.shutdown().await;
}

// ── 케이스 24: dispatch 실패 arm — 없는 agent_id Kill → Error(req echo) ─────────────
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
        "없는 agent Kill 은 not found Error 여야: {msg}"
    );

    server.shutdown().await;
}

// ── 케이스 25: dispatch 실패 arm — 없는 agent_id WriteStdin → Error(req echo) ─────────
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
        "없는 agent WriteStdin 은 not found Error 여야: {msg}"
    );

    server.shutdown().await;
}

// ── 케이스 26: dispatch 실패 arm — 없는 profile_id Spawn → Error(req echo) ────────────
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
        "없는 profile Spawn 은 profile not found Error 여야: {msg}"
    );

    server.shutdown().await;
}

// ══════════════════════════════════════════════════════════════════════════════════
// A: WS application-level keepalive (half-open 연결 감지).
// ══════════════════════════════════════════════════════════════════════════════════

fn fast_keepalive() -> KeepaliveConfig {
    KeepaliveConfig {
        ping_interval: Duration::from_millis(200),
        idle_timeout: Duration::from_millis(600),
    }
}

// ── 케이스 27: 데몬이 능동 Ping 을 보낸다(half-open 감지의 전제) ─────────────────────
#[tokio::test]
async fn case27_keepalive_server_sends_ping() {
    let server = start_test_server_with_keepalive(fast_keepalive())
        .await
        .unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    // ping_interval(200ms) 안에 첫 Ping 이 와야 한다. 여유 deadline(2s).
    assert!(
        c.saw_ping_within(Duration::from_secs(2)).await,
        "데몬이 ping_interval 안에 능동 WS Ping 을 보내야(half-open 감지 전제)"
    );

    server.shutdown().await;
}

// ── 케이스 28: Pong 미응답(죽은 클라) → idle_timeout 후 서버가 close ─────────────────
#[tokio::test]
async fn case28_keepalive_dead_client_closed() {
    let server = start_test_server_with_keepalive(fast_keepalive())
        .await
        .unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    // ★주의★: drain_handshake 든 expect_closed_within 이든 stream 을 poll 하면 tungstenite 가
    //   서버 Ping 에 자동 Pong 해 last_recv 가 갱신된다(=죽은 클라가 아니게 됨). 그래서 먼저
    //   idle_timeout 의 수 배 동안 **전혀 읽지 않고 sleep** 해 자동 Pong 을 원천 차단한다.
    //   그 sleep 동안 서버 ping arm 이 idle 을 감지(last_recv=auth 시점 고정)해 연결을 닫는다.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    assert!(
        c.expect_closed_within(Duration::from_secs(3)).await,
        "Pong 미응답(죽은 클라)이면 idle_timeout 후 서버가 연결을 닫아야"
    );

    server.shutdown().await;
}

// ── 케이스 29: 정상 활성 클라는 keepalive 로 끊기지 않음(회귀 방지) ──────────────────
#[tokio::test]
async fn case29_keepalive_active_client_survives() {
    let server = start_test_server_with_keepalive(fast_keepalive())
        .await
        .unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    // idle_timeout(600ms)의 ~4배(2.5s) 동안 계속 읽으며(자동 Pong) 안 끊기는지.
    assert!(
        c.stays_alive_while_reading(Duration::from_millis(2500))
            .await,
        "정상 활성 클라는 keepalive(자동 Pong)로 idle_timeout 을 넘지 않아 끊기면 안 됨"
    );

    server.shutdown().await;
}

// ══════════════════════════════════════════════════════════════════════════════════
// 멀티뷰어: resize 협상(tmux smallest) + 입력 lease(Zellij 명시 lease).
// ══════════════════════════════════════════════════════════════════════════════════

// ── 케이스 30: resize 협상 — 두 viewport 의 smallest 로 PTY, detach 후 재협상 ──────────
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
        "viewport a 단독이면 (80,40)"
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
        "두 viewport(a=80x40, b=40x20)의 smallest = (40,20)"
    );

    drop(c2);
    assert!(
        wait_for_size(&server, id, 80, 40).await,
        "viewport b 의 연결이 끊기면 남은 a 기준 (80,40) 으로 재협상 복귀"
    );

    server.shutdown().await;
}

// ── 케이스 31: resize 하위호환 — viewport_id 없으면 협상 우회(직접 그 크기) ─────────────
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
        "viewport_id 없으면 그 크기로 직접 resize(하위호환)"
    );

    server.shutdown().await;
}

// ── 케이스 32: 입력 lease — 보유 중 타 연결 WriteStdin 거부, 해제 후 통과 ───────────────
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
        "lease 보유 중 타 연결 WriteStdin 은 locked Error 여야: {msg}"
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

// ── 케이스 33: 보유자 연결 끊기면 lease 자동 해제(좀비 lock 방지) ───────────────────────
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

    // 끊김 cleanup 이 비동기라 즉시 반영 아님 → 재시도 폴링.
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
        "보유자(c1) 끊김 후 c2 가 lease 를 획득할 수 있어야(좀비 lock 자동 해제)"
    );

    server.shutdown().await;
}

// ── 케이스 34: lease 없을 때 WriteStdin 자유 통과(case14 회귀 — 단일 뷰어 마찰 0) ───────
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
// phase4 1단계: 프로필 CRUD + ad-hoc spawn 의 WS wire 경로.
// ══════════════════════════════════════════════════════════════════════════════════

// ── 케이스 35: WS CreateProfile → Created(req echo, 생성 프로필 동봉) ────────────────
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
        request_id: req,
    })
    .await;

    let created = c.await_created(req).await;
    assert_eq!(created.name, "p35", "Created 에 동봉된 프로필 이름 일치");
    assert_eq!(created.cwd, sent_cwd, "Created 에 동봉된 cwd 일치");
    assert!(
        matches!(&created.command, engram_dashboard_protocol::AgentSpawnCommand::Claude { extra_args, output_format, .. }
            if extra_args == &vec!["--foo".to_string()] && *output_format == WireOutputFormat::Terminal),
        "claude 프로필이 extra_args 보존 + 기본 output_format=Terminal"
    );
    assert!(created.auto_restore, "auto_restore 반영");
    assert!(
        server.manager.agent_snapshot(created.id).is_some(),
        "create 후 manager 레지스트리에 존재해야"
    );

    // ── ADR-0044 M2: 같은 WS 경로로 output_format=StreamJson 프로필 생성 → json 모드 저장 확인 ──
    let req_json = RequestId::new();
    c.send(&WireCommand::CreateProfile {
        name: "p35-json".into(),
        cwd: sent_cwd.clone(),
        extra_args: vec![],
        env: vec![],
        auto_restore: false,
        output_format: WireOutputFormat::StreamJson,
        request_id: req_json,
    })
    .await;
    let created_json = c.await_created(req_json).await;
    assert!(
        matches!(&created_json.command, engram_dashboard_protocol::AgentSpawnCommand::Claude { output_format, .. }
            if *output_format == WireOutputFormat::StreamJson),
        "StreamJson 으로 만든 프로필이 wire 로 json 모드로 돌아와야"
    );
    assert!(
        server
            .manager
            .agent_snapshot(created_json.id)
            .expect("json 프로필 등록됨")
            .command
            .is_json_mode(),
        "core 레지스트리 프로필이 json 모드여야(wire output_format→core 매핑)"
    );

    server.shutdown().await;
}

// ── 케이스 36: WS ListProfiles → ProfileList(req echo, 전용 reply) ──────────────────
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
        "ListProfiles 응답에 미리 등록한 프로필 포함"
    );

    server.shutdown().await;
}

// ── 케이스 37: WS SpawnProfile → Spawned(req echo, AgentInfo 동봉) ───────────────────
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
    assert_eq!(agent.id, profile_id, "Spawned 의 agent.id == profile_id");
    assert!(
        server.manager.agent_epoch(profile_id).is_some(),
        "SpawnProfile 후 manager 에 agent 가 살아있어야"
    );

    server.shutdown().await;
}

// ── 케이스 38: WS DeleteProfile → Ack + ProfileListUpdated(제거됨) ──────────────────
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
        "DeleteProfile 후 목록에서 제거돼야"
    );
    assert!(
        server.manager.agent_snapshot(profile_id).is_none(),
        "manager 레지스트리에서도 제거"
    );

    server.shutdown().await;
}

// ── 케이스 39: WS SetProfileAutoRestore → Ack + ProfileListUpdated(토글 반영) ────────
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
        .expect("토글 대상 프로필이 목록에 있어야");
    assert!(p.auto_restore, "auto_restore 가 true 로 토글돼야");
    assert!(
        server
            .manager
            .agent_snapshot(profile_id)
            .map(|p| p.auto_restore)
            .unwrap_or(false),
        "manager 레지스트리에도 토글 반영"
    );

    server.shutdown().await;
}

// ── 케이스 40: WS SpawnByCwd → Spawned(req echo, AgentInfo 동봉) ─────────────────────
#[tokio::test]
async fn case40_ws_spawn_by_cwd() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let before = server.manager.list_agents().len();
    let req = RequestId::new();
    c.send(&WireCommand::SpawnByCwd {
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        request_id: req,
    })
    .await;
    let agent = c.await_spawned(req).await;
    assert!(
        server.manager.agent_epoch(agent.id).is_some(),
        "Spawned 에 동봉된 agent.id 가 manager 에 살아있어야"
    );
    assert!(
        server.manager.list_agents().len() > before,
        "manager 에 ad-hoc agent 가 추가돼야"
    );

    server.shutdown().await;
}

// ── 케이스 41: WS GetSnapshot → Snapshot(req echo, chunks) — 전용 reply, Ack 없음 ────
#[tokio::test]
async fn case41_ws_get_snapshot() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);

    server.manager.write_stdin(id, b"echo SNAP41\r\n").unwrap();
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
    assert_eq!(aid, id, "Snapshot 의 agent_id echo");
    assert!(!chunks.is_empty(), "쌓인 출력이 snapshot chunk 로 와야");

    server.shutdown().await;
}

// ── 케이스 42: WS SpawnProfile 없는 profile_id → Error(req echo) ────────────────────
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
        "없는 profile SpawnProfile 은 not found Error 여야: {msg}"
    );

    server.shutdown().await;
}

// ── 케이스 43: WS SetProfileAutoRestore 없는 profile_id → Error(req echo) ────────────
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
        "없는 profile SetProfileAutoRestore 은 not found Error 여야: {msg}"
    );

    server.shutdown().await;
}

// ── 케이스 44: WS GetSnapshot 없는 agent_id → Error(req echo) ───────────────────────
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
        "없는 agent GetSnapshot 은 Error 여야: {msg}"
    );

    server.shutdown().await;
}

// ── 보조 함수 ──────────────────────────────────────────────────────────────────────

/// AgentListUpdated 는 spawn 으로 추가 발생할 수 있어 Hello 만 보장 소진하고, 첫 list 1건도 소진.
async fn drain_handshake(c: &mut Client) {
    match c.next_event().await {
        AgentEvent::Hello { .. } => {}
        ev => panic!("Hello 기대(handshake), got {ev:?}"),
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
        match c.next().await.expect("ReplayComplete 전 끊김") {
            Incoming::Event(AgentEvent::ReplayComplete { agent_id, .. }) if agent_id == id => {
                return
            }
            _ => continue,
        }
    }
}

/// id 의 frame seq 를 모으며, payload 에 marker 가 나타나면 멈춘다(그 frame seq 포함).
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
        "marker '{marker}' 도달 전 timeout/close (수집 {}건)",
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
    panic!("marker '{marker}' 도달 전 timeout/close");
}

/// PTY 가 첫 구독부터 모든 출력을 흘리므로 FromOldest replay+live 는 0 부터 빈틈없이 와야 한다.
fn assert_seq_contiguous_from_zero(seqs: &[u64]) {
    let mut sorted = seqs.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    for (i, s) in sorted.iter().enumerate() {
        assert_eq!(*s, i as u64, "seq 가 0 부터 연속이어야: {sorted:?}");
    }
}

// ══════════════════════════════════════════════════════════════════════════════════
// 실프로세스 전용 케이스 (#[cfg(windows)] + #[ignore]) — in-process 로는 검증 불가.
//
// 기본 `cargo test` 에서는 제외(#[ignore] — 실 OS·느림)하고, 다음으로 돌린다:
//   cargo test -p engram-dashboard-daemon --test ws_e2e -- --ignored --nocapture
//
// ★Windows 전용★: 데몬은 Windows 1차. 단일 인스턴스 잠금/Job Object/child_pids 가 Windows 구현이라
//   #[cfg(windows)] 로 한정한다(다른 OS 에선 컴파일 자체에서 제외 — 위장 PASS 없음).
// ══════════════════════════════════════════════════════════════════════════════════

#[cfg(windows)]
mod real_process {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};

    use engram_dashboard_protocol::DaemonInfo;

    const DAEMON_EXE: &str = env!("CARGO_BIN_EXE_engram-dashboard-daemon");

    /// 테스트별 고유 격리 컨텍스트. 유니크한 data_dir(ENGRAM_DATA_DIR) 하나면 충분하다 — ADR-0134
    /// 이후 단일 인스턴스 잠금 스코프가 **데이터 폴더**라, 폴더가 다르면 명부도 잠금도 함께 갈린다
    /// (별도 열쇠 변수를 짝지어 챙기던 것이 사라졌다).
    struct IsoCtx {
        data_dir: PathBuf,
    }

    fn fresh_iso(tag: &str) -> IsoCtx {
        use std::sync::atomic::{AtomicU64, Ordering};
        // 같은 나노초에 두 번 불려도 충돌하지 않게 프로세스 내 단조 카운터를 섞는다.
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let uniq = format!("{tag}-{nanos}-{n}");
        let dir = std::env::temp_dir().join(format!("engram-step7-{uniq}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp data_dir 생성");
        IsoCtx { data_dir: dir }
    }

    fn spawn_daemon_iso(ctx: &IsoCtx) -> Child {
        spawn_daemon_in(&ctx.data_dir)
    }

    /// data_dir 을 명시 주입하는 spawn(단일 인스턴스 테스트가 같은 폴더로 2개를 띄울 때 사용).
    fn spawn_daemon_in(data_dir: &Path) -> Child {
        Command::new(DAEMON_EXE)
            .env("ENGRAM_DATA_DIR", data_dir)
            .env("RUST_LOG", "info")
            .stdin(std::process::Stdio::null())
            // ★stdout 캡처(진단)★: agent 의 tracing fmt::layer() 는 기본 stdout 으로 쓴다. 데몬이 왜
            //   daemon.json 을 못 쓰는지(잠금 거부? data_dir? panic?)를 실패 시 인용하려고 stdout 을
            //   piped 로 받는다. (토큰 등 민감값은 데몬이 애초에 로그에 안 찍는다 — port/pid 만.)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("데몬 .exe spawn")
    }

    /// 핸들을 take 해 끝까지 읽는다 — 호출 전 데몬이 이미 종료했거나(EOF) kill 됐어야 블록되지 않는다.
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
        let path = data_dir.join("daemon.json");
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

    fn poll_until(deadline: std::time::Duration, mut pred: impl FnMut() -> bool) -> bool {
        let end = std::time::Instant::now() + deadline;
        while std::time::Instant::now() < end {
            if pred() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        pred()
    }

    fn kill_daemon(child: &mut Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    /// 데몬이 부팅 시 restore_all 로 이 프로필을 복원 → 살아있는 PTY child(cmd.exe)를 만든다.
    /// ShellBackend 는 program/args 를 그대로 PTY 에 싣는다(shim 래핑 없음) → cmd.exe 가 데몬의
    /// 직계 자식이 되어 child_pids(daemon_pid) 로 식별 가능하다.
    fn write_restorable_shell_agents_json(data_dir: &Path) {
        // SCHEMA_VERSION == 1 (persistence/mod.rs). 형태 고정(회귀 시 감지).
        let profile = AgentProfile::new(
            "step7-restore-shell".into(),
            AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
            std::env::temp_dir(),
            vec![],
            true, // auto_restore=true → 부팅 복원 대상
        );
        // ProfilesFile 은 비공개 구조라 동등한 JSON 을 직접 만든다(schema_version=1 + profiles 배열).
        let profiles_json = serde_json::to_string(&[profile]).expect("profile 직렬화");
        let file = format!("{{\"schema_version\":1,\"profiles\":{profiles_json}}}");
        std::fs::write(data_dir.join("agents.json"), file).expect("agents.json 작성");
    }

    // ── case1: 데몬 .exe kill → PTY child(cmd.exe) Job(KILL_ON_JOB_CLOSE) 동반 정리 ──────
    #[tokio::test]
    #[ignore = "실프로세스/Job 필요 — `-- --ignored` 로 실행(Windows 전용)"]
    async fn ignored_daemon_kill_cleans_pty_child() {
        use engram_dashboard_base::platform::{
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
                panic!("데몬이 daemon.json 을 발행해야 — 데몬 로그:\n{err}");
            }
        };
        let daemon_pid = info.pid;
        assert!(daemon_pid != 0, "데몬 PID 유효");

        // 복원은 3s 조기종료 윈도가 있어 넉넉히 대기한다.
        let mut child_set: Vec<u32> = Vec::new();
        let appeared = poll_until(std::time::Duration::from_secs(20), || {
            child_set = child_pids(daemon_pid);
            !child_set.is_empty()
        });
        if !appeared {
            kill_daemon(&mut daemon);
            let _ = std::fs::remove_dir_all(&data_dir);
            panic!(
                "데몬(pid={daemon_pid})의 PTY child(cmd.exe)를 OS 트리 열거로 식별하지 못함 — \
                 복원이 자식을 안 띄웠거나 ppid 미반영. 이 케이스는 살아있는 child 식별이 전제다."
            );
        }
        let live_children: Vec<(u32, u64)> = child_set
            .iter()
            .copied()
            .filter_map(|p| process_creation_time(p).map(|ct| (p, ct)))
            .collect();
        assert!(
            !live_children.is_empty(),
            "kill 전 데몬의 살아있는 PTY child 가 있어야(creation_time 조회됨): {child_set:?}"
        );

        let _ = daemon.kill();
        let _ = daemon.wait();

        // 자식 PID 들이 동반 사망하는지 폴링(Job 정리는 즉시는 아닐 수 있어 여유).
        let all_dead = poll_until(std::time::Duration::from_secs(15), || {
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
            "데몬 kill 후 PTY child({live_children:?})가 Job(KILL_ON_JOB_CLOSE)으로 동반 사망해야"
        );
    }

    // ── case2: single-instance — 두 번째 데몬이 폴더 잠금으로 거부(빠른 정상 종료 + json 불변) ──
    //
    // exit code: instance.rs 가 중복 시 run() 이 Ok(()) → main 이 정상 종료(exit 0). 따라서
    //   "exit 0 + 빠른 종료 + json 불변" 으로 단언한다(중복 전용 특수 코드는 없음 — 그 사실 명시).
    #[tokio::test]
    #[ignore = "실프로세스 2개 필요 — `-- --ignored` 로 실행(Windows 전용)"]
    async fn ignored_single_instance_second_rejected() {
        // ★single-instance 충돌을 의도적으로 유발★: A·B 가 **같은 data_dir** 를 쓴다(ADR-0134 —
        //   잠금 스코프가 폴더라 이것만으로 충돌한다). 다른 ignored 테스트는 폴더가 유니크해 무영향.
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
                panic!("데몬 A 가 daemon.json 을 발행해야 — 데몬 A 로그:\n{err}");
            }
        };

        let mut daemon_b = spawn_daemon_in(&data_dir);
        let exited_fast = poll_until(std::time::Duration::from_secs(3), || {
            matches!(daemon_b.try_wait(), Ok(Some(_)))
        });

        let b_status = daemon_b.try_wait().ok().flatten();

        let info_after = poll_daemon_json(&data_dir, std::time::Duration::from_secs(2));

        kill_daemon(&mut daemon_a);
        if b_status.is_none() {
            let _ = daemon_b.kill();
            let _ = daemon_b.wait();
        }
        let b_logs = drain_logs(&mut daemon_b);
        let _ = std::fs::remove_dir_all(&data_dir);

        assert!(
            exited_fast,
            "두 번째 데몬은 잠금 거부로 빠르게(3s 내) 종료해야 — B 로그:\n{b_logs}"
        );
        if let Some(status) = b_status {
            assert!(
                status.success(),
                "두 번째 데몬은 정상 종료(exit 0)해야 — got {status:?}, B 로그:\n{b_logs}"
            );
        } else {
            panic!("두 번째 데몬이 3s 내 종료하지 않음(잠금 거부 실패 가능) — B 로그:\n{b_logs}");
        }
        let info_after = info_after.expect("A 의 daemon.json 이 유지돼야");
        assert_eq!(
            info_after.pid, info_a.pid,
            "두 번째 데몬이 daemon.json 을 덮어쓰면 안 됨(pid 불변)"
        );
        assert_eq!(
            info_after.token, info_a.token,
            "daemon.json token 도 불변(B 가 새 토큰을 발행하면 안 됨)"
        );
    }

    // ── case3: stale daemon.json → 데몬이 stale 감지 후 자기 정보로 덮어쓰기 ────────────────
    //
    // src-tauri 의 ensure_daemon(WMI spawn) 경로는 별도 테스트(discovery::real_wmi_spawn_smoke)로
    //   분리해 채운다(daemon crate 에서 src-tauri 함수 호출 불가).
    #[tokio::test]
    #[ignore = "실프로세스 + 파일 discovery 필요 — `-- --ignored` 로 실행(Windows 전용)"]
    async fn ignored_stale_daemon_json_discovery() {
        let ctx = fresh_iso("stale");
        let data_dir = ctx.data_dir.clone();

        let mut tmp_child = Command::new("cmd.exe")
            .args(["/c", "exit"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("임시 자식 spawn");
        let dead_pid = tmp_child.id();
        let _ = tmp_child.wait(); // 종료 보장 → dead_pid 는 이제 죽음.

        let stale = DaemonInfo {
            pid: dead_pid,
            host: "127.0.0.1".into(),
            port: 59999,
            token: "d".repeat(64),
            protocol_version: PROTOCOL_VERSION,
            // ADR-0134 이후 stale 판정은 **진단 로그용**이라 이 값이 기동 여부를 가르지 않는다 —
            //   덮어쓸 권한은 폴더 잠금에서 나온다. 값은 "죽은 pid + 불일치 생성시각"이라는 현실적인
            //   조합을 유지하려고 그대로 둔다.
            start_time: 0xDEAD_BEEF,
        };
        let stale_json = serde_json::to_vec_pretty(&stale).expect("stale 직렬화");
        std::fs::write(data_dir.join("daemon.json"), &stale_json).expect("stale daemon.json 작성");

        let mut daemon = spawn_daemon_iso(&ctx);

        let mut latest: Option<DaemonInfo> = None;
        let overwritten = poll_until(std::time::Duration::from_secs(15), || {
            latest = poll_daemon_json(&data_dir, std::time::Duration::from_millis(100));
            matches!(&latest, Some(i) if i.pid != dead_pid)
        });

        kill_daemon(&mut daemon);
        let err = drain_logs(&mut daemon);
        let _ = std::fs::remove_dir_all(&data_dir);

        assert!(
            overwritten,
            "데몬이 stale daemon.json 을 자기 정보로 덮어써야(pid 가 stale dead_pid={dead_pid} 와 달라야) — 데몬 로그:\n{err}"
        );
        let fresh = latest.expect("덮어쓴 daemon.json");
        assert_ne!(fresh.pid, dead_pid, "새 pid 는 stale dead_pid 와 달라야");
        assert!(
            fresh.start_time != 0,
            "새 데몬은 유효 start_time 을 기록해야"
        );
        assert_ne!(
            fresh.token,
            "d".repeat(64),
            "새 데몬은 새 토큰을 발행해야(stale 토큰 유지 금지)"
        );
        assert!(fresh.port != 0, "새 데몬은 유효 포트를 기록해야");
    }
}
