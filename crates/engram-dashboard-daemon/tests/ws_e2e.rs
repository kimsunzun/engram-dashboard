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
//! (데몬 .exe kill→PTY child Job 동반 정리, single-instance mutex, stale daemon.json discovery)은
//! 실제 OS 프로세스/Job 이 필요하다. 이들은 이 파일 하단 `#[ignore]` 테스트로 두고 수동 실행법을
//! 주석에 적었다. 기본 `cargo test` 는 in-process 케이스만 빠르게 돈다.

use std::sync::Arc;
use std::time::Duration;

use engram_dashboard_core::agent::profile::{AgentCommand, AgentProfile, SpawnMode};
use engram_dashboard_daemon::ws::KeepaliveConfig;
use engram_dashboard_daemon::{
    start_test_server, start_test_server_with_keepalive, TestServerHandle,
};
use engram_dashboard_protocol::{
    decode_frame, AgentCommand as WireCommand, AgentEvent, RequestId, SubscribeAction,
    PROTOCOL_VERSION,
};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// 모든 네트워크 await 에 거는 기본 timeout. hang 방지(테스트가 영구 멈추지 않게).
const NET_TIMEOUT: Duration = Duration::from_secs(10);

// ── 클라이언트 헬퍼 ────────────────────────────────────────────────────────────────

/// 한 WS 연결을 감싼 테스트 클라이언트. connect+auth, command 송신, 이벤트/binary frame 수신을
/// 작은 메서드로 제공한다. control 은 JSON text, 출력은 codec binary frame 으로 온다.
struct Client {
    ws: Ws,
}

/// 수신 단위 — control 이벤트(JSON) 또는 출력 frame(binary 디코드 결과).
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
    /// ws://127.0.0.1:{port} 로 붙고 auth 까지 마친다(Auth 첫 frame). auth 검증은 하지 않고
    /// 연결만 — Hello/Error 수신은 호출자가 next_event 로 확인한다.
    async fn connect_and_auth(port: u16, token: &str) -> Self {
        let url = format!("ws://127.0.0.1:{port}");
        let (mut ws, _resp) = tokio::time::timeout(NET_TIMEOUT, connect_async(url))
            .await
            .expect("connect timeout")
            .expect("connect failed");
        let auth = WireCommand::Auth {
            token: token.to_string(),
            protocol_version: PROTOCOL_VERSION,
        };
        let text = serde_json::to_string(&auth).unwrap();
        tokio::time::timeout(NET_TIMEOUT, ws.send(Message::Text(text.into())))
            .await
            .expect("auth send timeout")
            .expect("auth send failed");
        Self { ws }
    }

    /// auth frame 만 보내지 않고 raw 연결만(타임아웃/잘못된 첫 frame 테스트용).
    async fn connect_raw(port: u16) -> Self {
        let url = format!("ws://127.0.0.1:{port}");
        let (ws, _resp) = tokio::time::timeout(NET_TIMEOUT, connect_async(url))
            .await
            .expect("connect timeout")
            .expect("connect failed");
        Self { ws }
    }

    /// 임의 AgentCommand 를 JSON text 로 송신.
    async fn send(&mut self, cmd: &WireCommand) {
        let text = serde_json::to_string(cmd).unwrap();
        tokio::time::timeout(NET_TIMEOUT, self.ws.send(Message::Text(text.into())))
            .await
            .expect("send timeout")
            .expect("send failed");
    }

    /// 다음 메시지 1건 수신 → Incoming. control=JSON event, binary=frame 디코드.
    /// Ping/Pong 은 건너뛰고 다음 실제 메시지를 반환한다. Close/None 은 None.
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

    /// 다음 control event 만 기대(중간 binary frame 은 모아 반환). 순서 검증용.
    async fn next_event(&mut self) -> AgentEvent {
        loop {
            match self.next().await.expect("연결이 끊김(이벤트 기대)") {
                Incoming::Event(ev) => return ev,
                Incoming::Frame(..) => continue,
            }
        }
    }

    /// binary frame 도 보내고 싶을 때(프로토콜 위반 케이스 검증용) raw binary 송신.
    async fn send_binary(&mut self, data: Vec<u8>) {
        tokio::time::timeout(NET_TIMEOUT, self.ws.send(Message::Binary(data.into())))
            .await
            .expect("send binary timeout")
            .expect("send binary failed");
    }

    /// 깨진/모르는 raw text 송신(파싱 실패 케이스용).
    async fn send_raw_text(&mut self, text: &str) {
        tokio::time::timeout(
            NET_TIMEOUT,
            self.ws.send(Message::Text(text.to_string().into())),
        )
        .await
        .expect("send raw timeout")
        .expect("send raw failed");
    }

    /// 주어진 request_id 의 Ack 가 올 때까지 대기(중간 다른 event/frame 흡수). request_id echo 검증.
    /// Error(같은 request_id)가 오면 panic(Ack 기대인데 실패).
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

    /// 주어진 request_id 의 Error 가 올 때까지 대기(request_id echo 검증). 메시지 반환.
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

    /// request_id 없는 Error(파싱 실패·resize 실패 등)를 대기. 메시지 반환.
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

    /// ListProfiles 조회 응답(전용 reply ProfileList, req echo) 의 profiles 를 반환(중간 event/frame 흡수).
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

    /// CRUD 응답을 한 번에 대기: Ack(req echo) **와** ProfileListUpdated 를 둘 다 본다(순서 무관).
    /// reply(Ack) 와 broadcast_profile_list 의 큐잉 순서에 의존하지 않게 한 루프에서 함께 모은다.
    /// 반환: 마지막으로 본 ProfileListUpdated 의 profiles.
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

    /// CreateProfile 응답 대기: Created(req echo, 프로필 동봉) 를 본다(phase4-2 #6).
    /// 기존 CRUD 와 달리 Ack 가 아니라 Created 로 응답한다(requester 가 "내 것" 매칭).
    /// broadcast 되는 ProfileListUpdated 는 흡수. 반환: Created 에 동봉된 프로필.
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

    /// SpawnByCwd/SpawnProfile 응답 대기: Spawned(req echo, AgentInfo 동봉) 를 본다(phase4-2 #6).
    /// 기존 Spawn 과 달리 Ack 가 아니라 Spawned 로 응답한다. broadcast 되는 AgentListUpdated 는 흡수.
    /// 반환: Spawned 에 동봉된 AgentInfo.
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

    /// GetSnapshot 조회 응답(전용 reply Snapshot, req echo) 의 (agent_id, chunks) 를 반환(중간 event/frame 흡수).
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

    /// ListAgents 조회 응답(전용 reply AgentList, req echo) 의 agents 를 반환(중간 event/frame·
    /// broadcast AgentListUpdated 흡수). 편승 매칭이 아니라 request_id 로만 매칭함을 검증한다.
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

    /// Spawn 응답을 한 번에 대기: Ack(req echo) **와** wanted agent_id 포함 AgentListUpdated 를
    /// **둘 다** 볼 때까지 수신한다(순서 무관). ★중요★: spawn_agent 은 agent_list_updated 브로드캐스트를
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

    /// Kill 응답을 한 번에 대기: Ack(req echo) **와** wanted 가 빠진 AgentListUpdated 를 둘 다 본다.
    /// kill_agent 도 list 갱신을 reply(Ack) 전에 큐잉하므로(순서 [list, Ack]) 함께 모은다.
    /// 중간 StatusChanged(Exiting/Killed) 등 control 은 흡수한다.
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

    /// 연결이 서버에 의해 닫히는지 확인 — next 가 None(Close/Err) 을 반환할 때까지.
    /// 중간에 Error event 가 오면 그것도 수용(닫기 직전 통보). true=닫힘 관측.
    async fn expect_closed(&mut self) -> bool {
        loop {
            match self.next().await {
                None => return true,
                Some(Incoming::Event(AgentEvent::Error { .. })) => continue,
                Some(_) => continue,
            }
        }
    }

    /// expect_closed 의 deadline 형. ★느린 소비자★ 테스트용: 들어오는 메시지를 **읽지 않고**
    /// (소켓 버퍼/서버 큐가 차야 하므로) raw stream 을 deadline 까지 대기해 Close/Err 만 본다.
    /// 큐를 비우면 안 되므로 next() 처럼 frame 을 파싱·소비하지 않고, 닫힘 신호만 감지한다.
    /// deadline 내 닫히면 true, 아니면 false.
    async fn expect_closed_within(&mut self, deadline: Duration) -> bool {
        let end = std::time::Instant::now() + deadline;
        loop {
            let remaining = end.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return false;
            }
            match tokio::time::timeout(remaining, self.ws.next()).await {
                // 타임아웃 — 아직 안 닫힘.
                Err(_) => return false,
                // 스트림 종료/오류 = 닫힘.
                Ok(None) | Ok(Some(Err(_))) => return true,
                Ok(Some(Ok(Message::Close(_)))) => return true,
                // 그 외 메시지는 무시(읽긴 하지만 — tungstenite 는 한 메시지씩 디코드).
                Ok(Some(Ok(_))) => continue,
            }
        }
    }

    /// keepalive 검증용: deadline 내에 서버가 보낸 **raw Ping** 프레임을 1회 이상 보면 true.
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
                // 다른 메시지(Pong/Text/Binary/Close)는 흡수하고 계속(자동 Pong 은 tungstenite 처리).
                Ok(Some(Ok(_))) => continue,
            }
        }
    }

    /// keepalive 회귀 검증용: deadline 동안 **계속 읽으며**(자동 Pong 유발) 연결이 닫히지 않으면
    /// true(=정상 활성 클라는 keepalive 로 끊기지 않음). 닫히면 false.
    async fn stays_alive_while_reading(&mut self, deadline: Duration) -> bool {
        let end = std::time::Instant::now() + deadline;
        loop {
            let remaining = end.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return true; // deadline 까지 안 닫힘 = 살아있음.
            }
            match tokio::time::timeout(remaining, self.ws.next()).await {
                Err(_) => return true, // 타임아웃 = 그 사이 닫힘 없음.
                Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(Message::Close(_)))) => return false,
                Ok(Some(Ok(_))) => continue, // 정상 읽기(자동 Pong) 지속.
            }
        }
    }
}

/// Client 메서드를 모듈 자유 함수로 노출(slow consumer 케이스 가독성).
async fn expect_closed_within(c: &mut Client, deadline: Duration) -> bool {
    c.expect_closed_within(deadline).await
}

/// 협상된 PTY 크기 검증용: manager 의 AgentInfo(cols/rows)가 (cols,rows)가 될 때까지 폴링.
/// ★resize 는 비동기(WS → read_task → dispatch → manager)라 즉시 반영이 아니다★ → 폴링한다.
/// AgentInfo 가 cols/rows 를 노출하므로(agent_info_to_wire) manager.list_agents 로 직접 확인 가능.
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
/// ShellBackend 가 program/args 를 그대로 PTY 에 싣는다(claude 같은 shim 아님 → cmd /c 래핑 없음).
/// 반환 agent_id 로 subscribe/write_stdin 한다. 테스트가 stdin 으로 출력 타이밍을 통제한다.
fn spawn_shell_agent(handle: &TestServerHandle) -> Uuid {
    // Windows 는 interactive cmd.exe(살아있는 셸), 비Windows 는 sh -i.
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
        .expect("shell agent spawn 실패");
    id
}

/// 결정적 출력 shell 프로필을 **ProfileRegistry 에 등록만** 하고(spawn 하지 않음) profile_id 를 반환.
/// WS `Spawn{profile_id}` dispatch 경로를 타려면 manager 의 레지스트리에 알려진 프로필이 있어야 한다.
/// ★운영 회귀 0★: 등록은 manager 의 공개 API(`profiles().upsert`)만 사용 — start_test_server/run()
///   배선을 건드리지 않는다(프로필 주입 인자 추가 불필요). 운영 경로도 같은 upsert 를 쓴다.
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
    handle.manager.profiles().upsert(profile);
    id
}

/// 출력이 누적될 때까지 짧게 대기(폴링). 결정적 출력은 PTY 가 즉시 내지만, OS 스케줄 지연을
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

    // 첫 control 은 Hello(버전 동봉).
    match c.next_event().await {
        AgentEvent::Hello {
            protocol_version, ..
        } => assert_eq!(protocol_version, PROTOCOL_VERSION),
        ev => panic!("Hello 기대, got {ev:?}"),
    }
    // 이어서 초기 AgentListUpdated(빈 목록).
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
    // 서버는 Error 후 close 해야 한다. ★짧은 deadline★: 옛 expect_closed 는 10s recv timeout 에
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
    // auth frame 을 보내지 않고 대기 → 서버 AUTH_TIMEOUT(1s) 후 close.
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

    // subscribe(처음부터) — Hello/list 이후 SubscribeAck → replay binary → ReplayComplete.
    c.send(&WireCommand::Subscribe {
        agent_id: id,
        epoch: None,
        after_seq: None,
    })
    .await;

    // stdin 으로 결정적 출력 유도(에코됨).
    server
        .manager
        .write_stdin(id, b"echo CASE4_MARKER\r\n")
        .unwrap();

    // SubscribeAck → (replay) → ReplayComplete → 이후 live frame 들. seq 를 수집해 0..n 연속 검증.
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

    // 구독 전에 출력 일부 쌓기.
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

    // 순서: SubscribeAck(text) → [replay binary…] → ReplayComplete(text) → live binary.
    // 1) 첫 control 은 SubscribeAck.
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
    // 2) ReplayComplete 가 올 때까지 사이의 frame 은 모두 replay(데이터 있음).
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

    // 3) ReplayComplete 이후 live — 새 stdin 출력이 frame 으로 도착.
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
    // 첫 구독에서 일부 받기.
    server.manager.write_stdin(id, b"echo R6A\r\n").unwrap();
    let first = collect_frame_seqs_until_marker(&mut c, id, "R6A").await;
    let last_seq = *first.iter().max().unwrap();

    // 끊고(연결 drop) 재연결 + after_seq=last_seq 로 resume.
    drop(c);
    // 끊긴 사이 추가 출력(이게 tail 로 와야 함).
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
    // replay 로 온 frame 들은 모두 seq > last_seq (tail 만).
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

    // 2MB ring 을 넘기는 대량 출력 — for 루프로 긴 줄 다수 출력.
    // (한 줄 ~80B × 40000 ≈ 3MB → oldest 가 0 위로 밀린다.)
    server
        .manager
        .write_stdin(
            id,
            b"for /L %i in (1,1,40000) do @echo TRUNCATE_LINE_PADDING_XXXXXXXXXXXXXXXXXXXXXXXXXXXX %i\r\n",
        )
        .unwrap();

    // oldest 가 0 위로 밀릴 때까지 대기(snapshot 의 첫 seq > 0).
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
    // after_seq=0 < oldest → Truncated.
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
    // 틀린 epoch(+1) + after_seq 지정 → after_seq 무시하고 Reset.
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
//   WsOutputSink.try_send 가 full 을 만나 close_signal 을 발동 → write_task 가 그 연결만 닫는다.
//   good 소비자는 **백그라운드 task 로 계속 drain** 해 살아남아야 한다(타 연결 무영향).
#[tokio::test]
async fn case09_slow_consumer_closed_others_unaffected() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);

    // 정상 소비자(B) — 별도 task 가 계속 읽어 살아있게 한다.
    let mut good = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut good).await;
    good.send(&WireCommand::Subscribe {
        agent_id: id,
        epoch: None,
        after_seq: None,
    })
    .await;
    wait_replay_complete(&mut good, id).await;

    // 느린 소비자(A) — 구독만 하고 이후 수신을 멈춘다(읽지 않음 → 서버 큐+소켓 버퍼가 찬다).
    let mut slow = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut slow).await;
    slow.send(&WireCommand::Subscribe {
        agent_id: id,
        epoch: None,
        after_seq: None,
    })
    .await;
    wait_replay_complete(&mut slow, id).await;

    // good 을 백그라운드에서 계속 drain — slow 가 막힌 동안에도 good 은 살아남아야 한다.
    // good_frames 로 "good 이 새 출력을 계속 받았는지" 를 확인한다.
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
            // None = good 연결이 닫힘(있어선 안 됨).
            good_alive.store(false, std::sync::atomic::Ordering::Relaxed);
        })
    };

    // 대량 출력 — slow 의 송신 큐(CONN_TX_CAP=4608)+OS 소켓 버퍼를 넘겨 close_signal 경로를 발동.
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

    // good 은 그 사이에도 살아있고 frame 을 계속 받았어야 한다(타 연결 무영향).
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

    // 1차 연결 — 일부 받기.
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

    // 끊긴 사이 출력.
    server.manager.write_stdin(id, b"echo RC10B\r\n").unwrap();
    wait_for_output(&server, id, (max1 as usize) + 2).await;

    // 재연결 + resume(after_seq=max1).
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

    // 합쳐서 seq 가 max1 까지 연속이고 max1 이후도 연속(gap 0) — dedup(set) 후 검증.
    let mut all: Vec<u64> = got1.clone();
    all.extend(got2.iter().copied());
    all.sort_unstable();
    all.dedup();
    // 0..=max(all) 모든 seq 가 존재해야 무손실.
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

    // 대량(긴 루프) 출력 + 끝 마커. 클라가 계속 읽어 데드락/유실 없이 마커까지 수신.
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

    // 두 agent 에 서로 다른 마커 출력.
    server.manager.write_stdin(id_a, b"echo AAA12\r\n").unwrap();
    server.manager.write_stdin(id_b, b"echo BBB12\r\n").unwrap();

    // frame 의 agent_id 로 역다중화 — 각 agent 의 payload 가 자기 agent_id 로만 와야 한다.
    let mut saw_a = false;
    let mut saw_b = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while (!saw_a || !saw_b) && std::time::Instant::now() < deadline {
        match c.next().await {
            Some(Incoming::Frame(aid, _, _, payload)) => {
                let text = String::from_utf8_lossy(&payload);
                // 마커가 섞여 엉뚱한 agent_id 로 오면 역다중화 실패.
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

    // dispatch Spawn → manager.spawn_agent → Ack(req echo) + agent_list_updated(새 agent_id).
    // (spawn_agent 이 list 를 Ack 보다 먼저 큐잉하므로 둘을 한 루프에서 함께 받는다.)
    c.await_spawn(profile_id, req).await;
    // manager 에도 실제로 떠 있어야(dispatch 가 실제 spawn 했다는 사실 확인).
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

    // 1) WS Spawn.
    let req_spawn = RequestId::new();
    c.send(&WireCommand::Spawn {
        profile_id,
        request_id: req_spawn,
    })
    .await;
    c.await_spawn(profile_id, req_spawn).await;

    // 2) WS Subscribe(출력 평면 받기) — replay 끝까지 소진.
    c.send(&WireCommand::Subscribe {
        agent_id: profile_id,
        epoch: None,
        after_seq: None,
    })
    .await;
    wait_replay_complete(&mut c, profile_id).await;

    // 3) WS WriteStdin — dispatch 가 data → InputEvent::Raw 로 변환해 PTY 에 싣는다.
    let req_write = RequestId::new();
    c.send(&WireCommand::WriteStdin {
        agent_id: profile_id,
        data: b"echo VIA_WS\r\n".to_vec(),
        request_id: req_write,
    })
    .await;
    c.await_ack(req_write).await;

    // 출력이 binary frame 으로 도착(VIA_WS 마커 — 에코됨).
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

    // WS Kill → Ack + (kill_agent 이 list 갱신 브로드캐스트). CLAUDE.md 불변식: terminal 판정은
    // status_changed 가 아니라 agent-list-updated 로 — 목록에서 빠지는 것으로 확인한다.
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

    // WS Interrupt → Ack. interrupt 는 Ctrl+C 만 — 프로세스는 살아있어야 한다.
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

    // subscribe → replay 끝 → 출력 한 번 받아 살아있는 구독 확인.
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

    // WS Unsubscribe — 이 연결의 그 agent sink 제거.
    c.send(&WireCommand::Unsubscribe {
        agent_id: profile_id,
    })
    .await;

    // unsubscribe 가 dispatch·코어에 반영될 시간을 준 뒤 새 출력 유발.
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

    // 이후 일정 시간 동안 frame 이 더 오면 안 된다(control 만 허용). deadline 동안 frame 0 검증.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match tokio::time::timeout(remaining, c.next()).await {
            Ok(Some(Incoming::Frame(..))) => {
                panic!("Unsubscribe 후 live frame 이 도착하면 안 됨");
            }
            Ok(Some(Incoming::Event(_))) => continue, // status/list 등 control 은 허용.
            Ok(None) => break,
            Err(_) => break, // timeout = frame 안 옴(정상).
        }
    }

    server.shutdown().await;
}

// ── 케이스 19: WS ListAgents → AgentListUpdated ────────────────────────────────────
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

    // 명시 WS ListAgents → 전용 reply AgentList(req echo, 그 agent 포함).
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

    // 1) force=false + 활성 agent → 거부 Error(req echo). 서버는 살아있어야 한다.
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
    // 서버 생존 확인 — 같은 연결로 ListAgents 가 정상 응답(연결·서버 살아있음).
    let req_alive = RequestId::new();
    c.send(&WireCommand::ListAgents {
        request_id: req_alive,
    })
    .await;
    let _ = c.await_agent_list(req_alive).await;

    // 2) force=true + kill_agents=true → Ack 후 종료(연결 close + 서버 watch 종료).
    let req_stop = RequestId::new();
    c.send(&WireCommand::StopDaemon {
        force: true,
        kill_agents: true,
        request_id: req_stop,
    })
    .await;
    c.await_ack(req_stop).await;
    // 종료 신호 → main(accept loop) 종료 → 이 연결도 닫힌다.
    assert!(
        c.expect_closed().await,
        "StopDaemon(force) 후 연결이 닫혀야"
    );

    // accept loop 가 watch 로 종료됐는지 — 새 연결이 더는 안 붙어야(서버 종료 실증).
    // accept_handle join 으로 확정(shutdown 은 idempotent — 이미 종료된 watch 에 재send).
    server.shutdown().await;
}

// ── 케이스 21: 2차 Auth → already authenticated Error(현 dispatch 동작) ─────────────
#[tokio::test]
async fn case21_ws_second_auth_rejected() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    // 이미 auth 된 연결에서 또 Auth — dispatch 가 request_id 없는 Error("already authenticated").
    c.send(&WireCommand::Auth {
        token: server.token.clone(),
        protocol_version: PROTOCOL_VERSION,
    })
    .await;
    let msg = c.await_error_no_id().await;
    assert!(
        msg.contains("already authenticated"),
        "2차 Auth 는 already authenticated Error 여야: {msg}"
    );
    // 연결은 닫히지 않고 유지(Error 만) — 후속 ListAgents 가 응답하는지로 확인.
    let req_alive = RequestId::new();
    c.send(&WireCommand::ListAgents {
        request_id: req_alive,
    })
    .await;
    let _ = c.await_agent_list(req_alive).await;

    server.shutdown().await;
}

// ── 케이스 22: control 자리에 binary frame → Error + 연결 close(ws.rs:610) ──────────
#[tokio::test]
async fn case22_ws_binary_frame_rejected() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    // 클라→데몬 binary 는 프로토콜에 없음 → read_task 가 Error 후 close.
    c.send_binary(vec![0xde, 0xad, 0xbe, 0xef]).await;
    // Error("unexpected binary frame") 후 연결 close. expect_closed 가 Error 를 흡수하며 닫힘 관측.
    assert!(
        c.expect_closed().await,
        "control 자리 binary 는 Error 후 연결이 닫혀야"
    );

    server.shutdown().await;
}

// ── 케이스 23: 깨진 JSON text → Error(req 없음), 연결은 유지(ws.rs:604) ─────────────
#[tokio::test]
async fn case23_ws_parse_failure_error() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    // 모르는/깨진 명령 JSON → serde 파싱 실패 → Error(request_id None). 연결은 유지된다.
    c.send_raw_text("{\"NotACommand\":true}").await;
    let msg = c.await_error_no_id().await;
    assert!(
        msg.contains("invalid command"),
        "파싱 실패 Error 메시지: {msg}"
    );
    // 연결 유지 확인 — 후속 ListAgents 정상 응답.
    let req_alive = RequestId::new();
    c.send(&WireCommand::ListAgents {
        request_id: req_alive,
    })
    .await;
    let _ = c.await_agent_list(req_alive).await;

    server.shutdown().await;
}

// ── 케이스 24: dispatch 실패 arm — 없는 agent_id Kill → Error(req echo) ─────────────
//   정상계(case13~)만 타던 갭을 메운다. manager 가 NotFound 를 반환하면 dispatch 가 그 에러를
//   Error{request_id: Some(보낸 req)} 로 매핑하는지.
#[tokio::test]
async fn case24_ws_kill_unknown_agent_error() {
    let server = start_test_server().await.unwrap();
    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    let req = RequestId::new();
    c.send(&WireCommand::Kill {
        agent_id: Uuid::new_v4(), // 존재하지 않는 agent
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
        profile_id: Uuid::new_v4(), // 레지스트리에 없는 프로필
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
//   데몬이 능동 Ping 을 보내고, 마지막 클라 수신 후 idle_timeout 초과 시 연결을 닫는다.
//   ★짧은 주입값★(ping 200ms / idle 600ms)으로 테스트가 수 초 내 끝나게 한다.
// ══════════════════════════════════════════════════════════════════════════════════

/// 테스트용 짧은 keepalive 설정(상수 하드코딩 회피 — 운영 20s/50s 와 분리).
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
//   tungstenite 는 stream 을 poll 할 때만 자동 Pong 한다. 죽은 클라를 흉내내려고 auth 후
//   idle 구간 동안 **전혀 읽지 않는다**(자동 Pong 미발생) → 서버 last_recv 가 갱신되지 않아
//   idle_timeout(600ms) 초과 → close_signal → 서버가 이 연결을 닫는다.
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

    // 이제 비로소 poll → 이미 닫혔거나(backlog 소진 후 Close) 즉시 Close 를 관측해야 한다.
    // (이 시점에 버퍼된 Ping 들에 뒤늦게 Pong 을 쓰려 해도 서버는 이미 닫는 중 → 무해.)
    assert!(
        c.expect_closed_within(Duration::from_secs(3)).await,
        "Pong 미응답(죽은 클라)이면 idle_timeout 후 서버가 연결을 닫아야"
    );

    server.shutdown().await;
}

// ── 케이스 29: 정상 활성 클라는 keepalive 로 끊기지 않음(회귀 방지) ──────────────────
//   클라가 계속 읽으면 tungstenite 가 서버 Ping 에 자동 Pong → 서버 last_recv 가 갱신돼
//   idle_timeout 을 넘지 않는다. idle_timeout(600ms)의 수 배 동안 살아있어야 한다.
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
//   두 연결로 같은 agent 를 동시 attach 한 상황을 시뮬한다.
// ══════════════════════════════════════════════════════════════════════════════════

// ── 케이스 30: resize 협상 — 두 viewport 의 smallest 로 PTY, detach 후 재협상 ──────────
#[tokio::test]
async fn case30_multiviewer_resize_smallest_and_renegotiate() {
    let server = start_test_server().await.unwrap();
    let id = spawn_shell_agent(&server);

    // 연결1: viewport "a" → (80,40).
    let mut c1 = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c1).await;
    c1.send(&WireCommand::Resize {
        agent_id: id,
        cols: 80,
        rows: 40,
        viewport_id: Some("a".into()),
    })
    .await;
    // viewport 하나뿐이면 그 크기가 곧 협상값.
    assert!(
        wait_for_size(&server, id, 80, 40).await,
        "viewport a 단독이면 (80,40)"
    );

    // 연결2: viewport "b" → (40,20). 두 뷰어 중 smallest = (40,20) 로 PTY 가 맞춰져야 한다.
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

    // 연결2 끊김 → 그 viewport 가 빠지고 남은 a 기준 (80,40) 으로 재협상(복귀).
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
    // viewport_id=None(v1 프론트 기본) → 협상 없이 그 크기로 직접.
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

    // 연결1 이 lease 획득 → Ack.
    let req_acq = RequestId::new();
    c1.send(&WireCommand::AcquireInput {
        agent_id: id,
        request_id: req_acq,
    })
    .await;
    c1.await_ack(req_acq).await;

    // 연결2 WriteStdin → lease 가 c1 에 잠겨 있어 Error.
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

    // 보유자(c1) WriteStdin 은 통과(Ack).
    let req_w1 = RequestId::new();
    c1.send(&WireCommand::WriteStdin {
        agent_id: id,
        data: b"echo HOLDER_OK\r\n".to_vec(),
        request_id: req_w1,
    })
    .await;
    c1.await_ack(req_w1).await;

    // c1 이 ReleaseInput → 이후 c2 WriteStdin 통과.
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

    // c1 이 lease 획득.
    let req_acq = RequestId::new();
    c1.send(&WireCommand::AcquireInput {
        agent_id: id,
        request_id: req_acq,
    })
    .await;
    c1.await_ack(req_acq).await;

    // c1 끊김 → cleanup 이 lease 자동 해제해야 한다(보유자 사망 시 다른 뷰어가 영영 막히면 안 됨).
    drop(c1);

    // c2 가 acquire 시도 → 끊긴 보유자 lease 가 풀렸으므로 성공해야 한다.
    //   끊김 cleanup 이 비동기라 즉시 반영 아님 → 재시도 폴링.
    let mut acquired = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while std::time::Instant::now() < deadline {
        let req = RequestId::new();
        c2.send(&WireCommand::AcquireInput {
            agent_id: id,
            request_id: req,
        })
        .await;
        // Ack 또는 Error 중 무엇이 오는지 본다.
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

    // lease 를 잡지 않은 상태에서 WriteStdin → 자유 통과(Ack). 단일 뷰어 흔한 경우.
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
//   각 case 는 EmbeddedClient(invoke)와 동일 의미인지(인자/부작용)를 dispatch 경로로 검증한다.
// ══════════════════════════════════════════════════════════════════════════════════

// ── 케이스 35: WS CreateProfile → Created(req echo, 생성 프로필 동봉) ────────────────
// phase4-2 #6: Ack 대신 Created 로 응답. request_id 에 생성된 프로필을 동봉(DaemonClient 매칭용).
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
        request_id: req,
    })
    .await;

    // Created event 가 생성된 프로필을 직접 동봉 — request_id race 없이 "내 것" 식별.
    let created = c.await_created(req).await;
    assert_eq!(created.name, "p35", "Created 에 동봉된 프로필 이름 일치");
    assert_eq!(created.cwd, sent_cwd, "Created 에 동봉된 cwd 일치");
    assert!(
        matches!(&created.command, engram_dashboard_protocol::AgentSpawnCommand::Claude { extra_args, .. } if extra_args == &vec!["--foo".to_string()]),
        "claude 프로필이 extra_args 보존"
    );
    assert!(created.auto_restore, "auto_restore 반영");
    // manager(공유 레지스트리)에도 실제 등록됐는지 — dispatch 가 upsert 했다는 사실 확인.
    assert!(
        server.manager.profiles().get(created.id).is_some(),
        "create 후 manager 레지스트리에 존재해야"
    );

    server.shutdown().await;
}

// ── 케이스 36: WS ListProfiles → ProfileList(req echo, 전용 reply) ──────────────────
#[tokio::test]
async fn case36_ws_list_profiles() {
    let server = start_test_server().await.unwrap();
    // 미리 1개 등록(공개 API — start_test_server 배선 무수정).
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
// phase4-2 #6: Ack 대신 Spawned 로 응답. agent_id == profile_id(프로필 id 가 곧 agent id).
#[tokio::test]
async fn case37_ws_spawn_profile() {
    let server = start_test_server().await.unwrap();
    let profile_id = register_shell_profile(&server);

    let mut c = Client::connect_and_auth(server.port, &server.token).await;
    drain_handshake(&mut c).await;

    // resume=false → Fresh spawn. agent_id == profile_id(프로필 id 가 곧 agent id).
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
        server.manager.profiles().get(profile_id).is_none(),
        "manager 레지스트리에서도 제거"
    );

    server.shutdown().await;
}

// ── 케이스 39: WS SetProfileAutoRestore → Ack + ProfileListUpdated(토글 반영) ────────
#[tokio::test]
async fn case39_ws_set_auto_restore() {
    let server = start_test_server().await.unwrap();
    // register_shell_profile 은 auto_restore=false 로 등록 → true 로 토글되는지 본다.
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
            .profiles()
            .get(profile_id)
            .map(|p| p.auto_restore)
            .unwrap_or(false),
        "manager 레지스트리에도 토글 반영"
    );

    server.shutdown().await;
}

// ── 케이스 40: WS SpawnByCwd → Spawned(req echo, AgentInfo 동봉) ─────────────────────
// phase4-2 #6: Ack 대신 Spawned 로 응답 — 새 uuid 를 미리 모르던 문제를 동봉 AgentInfo 로 해소.
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
    // Spawned 가 새 agent 의 AgentInfo 를 직접 동봉 → id 를 미리 몰라도 식별 가능.
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

    // 결정적 출력을 쌓아 snapshot 에 chunk 가 있게 한다.
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
    // Snapshot 에 request_id 동봉(전용 reply)이라 별도 Ack 는 오지 않는다(await_snapshot 이 req echo 검증).

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

/// connect 직후의 Hello + 초기 AgentListUpdated 2건을 소진한다(이후 검증 노이즈 제거).
/// AgentListUpdated 는 spawn 으로 추가 발생할 수 있어 Hello 만 보장 소진하고, 첫 list 1건도 소진.
async fn drain_handshake(c: &mut Client) {
    // Hello.
    match c.next_event().await {
        AgentEvent::Hello { .. } => {}
        ev => panic!("Hello 기대(handshake), got {ev:?}"),
    }
    // 초기 AgentListUpdated 1건.
    loop {
        match c.next_event().await {
            AgentEvent::AgentListUpdated { .. } => break,
            // spawn 타이밍에 따라 StatusChanged 등이 먼저 올 수 있어 흡수.
            _ => continue,
        }
    }
}

/// SubscribeAck 후 ReplayComplete 까지(중간 replay frame·이벤트 소진) 대기.
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

/// id 의 frame(payload)을 모으며 marker 도달 시 멈춘다.
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

/// seq 들이 0 부터 연속(0,1,2,…)인지 검증. PTY 가 첫 구독부터 모든 출력을 흘리므로
/// FromOldest replay+live 는 0 부터 빈틈없이 와야 한다.
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
// 아래는 실제 데몬 .exe / OS Job Object / named mutex / 파일시스템 discovery 가 필요해
// in-process 서버로는 재현할 수 없다(데몬을 진짜 별도 프로세스로 띄워야만 인과가 성립).
// 기본 `cargo test` 에서는 제외(#[ignore] — 실 OS·느림)하고, 다음으로 돌린다:
//   cargo test -p engram-dashboard-daemon --test ws_e2e -- --ignored --nocapture
//
// ★Step7 구현 완료★: 세 케이스 모두 실제 데몬 .exe 를 spawn 해 검증한다(이전 unimplemented! RED →
// 이제 GREEN). 각 테스트는 ENGRAM_DATA_DIR 로 임시 data_dir 을 격리한다 — ★이 격리가 먹는 이유★:
// 아래 spawn 은 `std::process::Command`(직접 spawn)라 자식이 **부모 env 를 상속**하므로
// ENGRAM_DATA_DIR override 가 데몬까지 전달돼 daemon.json/agents.json 을 임시 디렉토리에 쓴다.
// 따라서 운영 data_dir(디버그=repo 루트 `.engram-data`, 더는 %APPDATA% 가 아님)을 오염시키지 않는다.
// ★차이 명시★: 만약 이 경로가 WMI(Win32_Process.Create) spawn 이었다면 자식이 부모 env 를 상속하지
//   않아 이 격리가 안 먹었을 것이다(그 경우 데몬은 운영 `.engram-data` 를 본다 — discovery 의
//   real_wmi_spawn_* smoke 가 그래서 env 격리 대신 백업/복원으로 운영 파일을 보호한다).
// 끝에서 데몬·자식 프로세스 kill + 임시 디렉토리 삭제로 자원 누수 0.
//
// ★Windows 전용★: 데몬은 Windows 1차. named mutex/Job Object/child_pids 가 Windows 구현이라
//   #[cfg(windows)] 로 한정한다(다른 OS 에선 컴파일 자체에서 제외 — 위장 PASS 없음).
// ══════════════════════════════════════════════════════════════════════════════════

#[cfg(windows)]
mod real_process {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};

    use engram_dashboard_protocol::DaemonInfo;

    /// 데몬 바이너리 절대경로(cargo 가 통합테스트에 주입). 실제 빌드된 .exe.
    const DAEMON_EXE: &str = env!("CARGO_BIN_EXE_engram-dashboard-daemon");

    /// 테스트별 고유 격리 컨텍스트. data_dir(ENGRAM_DATA_DIR)·instance_key(ENGRAM_INSTANCE_KEY)를
    /// 함께 묶어 데몬에 주입한다. 둘 다 유니크하면 데몬이 ★data_dir·mutex 모두 독립★이라 cargo 의
    /// 병렬 실행에서도 다른 테스트 데몬과 충돌하지 않는다(USERNAME Global mutex 공유가 flaky 원인이었음).
    struct IsoCtx {
        data_dir: PathBuf,
        instance_key: String,
    }

    /// 테스트마다 고유한 임시 data_dir + instance_key 생성(이전 잔여 정리). nanos + 테스트명 + 카운터로
    /// 유니크 — 병렬 실행 충돌 없음. ENGRAM_DATA_DIR/ENGRAM_INSTANCE_KEY 로 데몬에 주입해 운영
    /// data_dir(디버그=repo 루트 `.engram-data`)과 운영 USERNAME mutex 를 둘 다 건드리지 않는다.
    /// (직접-spawn 이라 env 가 상속돼 ENGRAM_DATA_DIR 격리가 먹는다 — 모듈 상단 주석 참조.)
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
        IsoCtx {
            data_dir: dir,
            instance_key: format!("step7-{uniq}"),
        }
    }

    /// 주어진 격리 컨텍스트로 데몬 .exe 를 별도 OS 프로세스로 spawn.
    /// ENGRAM_DATA_DIR override 로 daemon.json/agents.json 을 임시 디렉토리에 쓰게 하고,
    /// ENGRAM_INSTANCE_KEY override 로 단일 인스턴스 mutex 를 테스트별로 격리한다.
    /// ★stderr 캡처(진단)★: 데몬이 왜 daemon.json 을 못 쓰는지(mutex 거부? data_dir? panic?)를
    ///   실패 시 인용할 수 있도록 stderr 를 piped 로 받고 RUST_LOG=info 로 진단 로그를 켠다.
    ///   (토큰 등 민감값은 데몬이 애초에 로그에 안 찍는다 — port/pid 만.)
    fn spawn_daemon_iso(ctx: &IsoCtx) -> Child {
        spawn_daemon_with_key(&ctx.data_dir, &ctx.instance_key)
    }

    /// data_dir + instance_key 를 명시 주입하는 spawn(단일인스턴스 테스트가 같은 key 2개를 띄울 때 사용).
    fn spawn_daemon_with_key(data_dir: &Path, instance_key: &str) -> Child {
        Command::new(DAEMON_EXE)
            .env("ENGRAM_DATA_DIR", data_dir)
            .env("ENGRAM_INSTANCE_KEY", instance_key)
            // 진단용: info 레벨로 "데몬 시작/이미 실행 중/stale 덮어씀" 등 진단 로그를 받는다.
            .env("RUST_LOG", "info")
            .stdin(std::process::Stdio::null())
            // ★stdout 캡처(진단)★: core 의 tracing fmt::layer() 는 기본 stdout 으로 쓴다. 데몬이 왜
            //   daemon.json 을 못 쓰는지(mutex 거부? data_dir? panic?)를 실패 시 인용하려고 stdout 을
            //   piped 로 받는다. (토큰 등 민감값은 데몬이 애초에 로그에 안 찍는다 — port/pid 만.)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("데몬 .exe spawn")
    }

    /// 실행 중/종료된 데몬의 진단 로그(stdout+stderr)를 비차단으로 회수(진단 인용용). 핸들을 take 해
    /// 끝까지 읽는다 — 호출 전 데몬이 이미 종료했거나(EOF) kill 됐어야 블록되지 않는다.
    /// tracing fmt 는 stdout 으로 쓰므로 stdout 이 주된 출처다(stderr 는 패닉 백트레이스 등 보조).
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

    /// daemon.json 이 써질 때까지 폴링해 DaemonInfo 회수. deadline 초과면 None.
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

    /// 특정 조건(predicate)이 참이 될 때까지 폴링(true 반환). deadline 초과면 false.
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

    /// 데몬 프로세스를 확실히 종료(kill + wait). 정리 경로에서 호출(누수 0).
    fn kill_daemon(child: &mut Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    /// auto_restore=true 인 cmd.exe shell 프로필 1개를 담은 agents.json 을 data_dir 에 써둔다.
    /// 데몬이 부팅 시 restore_all 로 이 프로필을 복원 → 살아있는 PTY child(cmd.exe)를 만든다.
    /// ShellBackend 는 program/args 를 그대로 PTY 에 싣는다(shim 래핑 없음) → cmd.exe 가 데몬의
    /// 직계 자식이 되어 child_pids(daemon_pid) 로 식별 가능하다.
    ///
    /// ★왜 agents.json 직접 작성★: WS 프로토콜에는 프로필 생성(CRUD)이 아직 없어(messages.rs)
    ///   WS Spawn 만으로는 실프로세스 데몬에 새 프로필을 만들 수 없다. 데몬의 부팅 복원 경로
    ///   (restore_all)를 통해 살아있는 PTY child 를 띄우는 것이 현 프로토콜에서 가능한 길이다.
    ///   FileProfileStore 의 디스크 포맷({schema_version, profiles})에 맞춰 직접 직렬화한다.
    fn write_restorable_shell_agents_json(data_dir: &Path) {
        // FileProfileStore::SCHEMA_VERSION == 1 (persistence/mod.rs). 형태 고정(회귀 시 감지).
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
    //
    // 검증: 데몬 .exe spawn(임시 data_dir, restorable cmd.exe 프로필 동봉) → 데몬이 부팅 복원으로
    //   cmd.exe child 를 띄움 → child_pids(daemon_pid) 로 그 PID 식별 → 데몬 프로세스 kill →
    //   그 child PID 가 죽는지(pid_alive=false) 폴링. KILL_ON_JOB_CLOSE 면 동반 사망 → PASS.
    //
    // child PID 식별: AgentInfo/WS 프로토콜은 child pid 를 노출하지 않으므로, OS 프로세스 트리
    //   열거(core platform::child_pids, Toolhelp32Snapshot)로 데몬의 직계 자식 cmd.exe 를 찾는다.
    #[tokio::test]
    #[ignore = "실프로세스/Job 필요 — `-- --ignored` 로 실행(Windows 전용)"]
    async fn ignored_daemon_kill_cleans_pty_child() {
        use engram_dashboard_core::agent::platform::{
            child_pids, pid_alive_with_start_time, process_creation_time,
        };

        let ctx = fresh_iso("kill");
        let data_dir = ctx.data_dir.clone();
        write_restorable_shell_agents_json(&data_dir);
        let mut daemon = spawn_daemon_iso(&ctx);

        // 1) 데몬 기동 확인(daemon.json) + 그 PID 회수. 미발행 시 stderr 를 인용해 원인 가시화.
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

        // 2) 부팅 복원으로 cmd.exe child 가 뜰 때까지 대기 → 그 PID 들 기록.
        //    복원은 3s 조기종료 윈도가 있어 넉넉히 대기한다.
        let mut child_set: Vec<u32> = Vec::new();
        let appeared = poll_until(std::time::Duration::from_secs(20), || {
            child_set = child_pids(daemon_pid);
            !child_set.is_empty()
        });
        if !appeared {
            // 식별 실패 — 은폐 금지: 정리 후 명확히 실패 보고(자식을 못 찾으면 검증 불가).
            kill_daemon(&mut daemon);
            let _ = std::fs::remove_dir_all(&data_dir);
            panic!(
                "데몬(pid={daemon_pid})의 PTY child(cmd.exe)를 OS 트리 열거로 식별하지 못함 — \
                 복원이 자식을 안 띄웠거나 ppid 미반영. 이 케이스는 살아있는 child 식별이 전제다."
            );
        }
        // 식별된 자식들의 (pid, creation_time) 을 기록한다.
        // ★왜 creation_time 인가★: `pid_alive` 는 OpenProcess 실패 시 *보수적으로 true* 를 반환해
        //   "죽음" 검증에 부적합하다(실측: 죽은 PID 도 pid_alive=true → 오판). 죽음을 정확히 보려면
        //   creation_time 을 사전 기록하고 kill 후 `pid_alive_with_start_time(pid, 기록값)` 로 판정한다.
        //   죽으면 process_creation_time=None + expected!=0 → false(dead). PID 재사용되면 creation_time
        //   이 달라 false. 살아있고 같으면 true. = 정확한 동일-프로세스 생존 판정.
        let live_children: Vec<(u32, u64)> = child_set
            .iter()
            .copied()
            .filter_map(|p| process_creation_time(p).map(|ct| (p, ct)))
            .collect();
        assert!(
            !live_children.is_empty(),
            "kill 전 데몬의 살아있는 PTY child 가 있어야(creation_time 조회됨): {child_set:?}"
        );

        // 3) 데몬 프로세스를 강제 종료(TerminateProcess). Job 핸들이 닫히며 KILL_ON_JOB_CLOSE 발동.
        let _ = daemon.kill();
        let _ = daemon.wait();

        // 4) 자식 PID 들이 동반 사망하는지 폴링(Job 정리는 즉시는 아닐 수 있어 여유).
        //    pid_alive_with_start_time(p, 기록 creation_time): 같은 프로세스가 살아있을 때만 true.
        let all_dead = poll_until(std::time::Duration::from_secs(15), || {
            live_children
                .iter()
                .all(|&(p, ct)| !pid_alive_with_start_time(p, ct))
        });

        // 정리 — 만약 안 죽었으면 잔존 자식을 직접 kill(누수 방지) 후 단언.
        if !all_dead {
            for &(p, _) in &live_children {
                // best-effort 잔존 정리(taskkill).
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

    // ── case2: single-instance — 두 번째 데몬이 named mutex 로 거부(빠른 정상 종료 + json 불변) ──
    //
    // 검증: 데몬 A spawn → daemon.json 발행 확인 → 데몬 B spawn(같은 data_dir/env) → B 가
    //   named mutex 로 거부돼 빠르게(3s 내) 정상 종료(exit 0)하고, A 의 daemon.json 이 보존
    //   (B 가 안 덮어씀 = pid/token 불변)되는지 확인.
    //
    // exit code: instance.rs 가 중복 시 run() 이 Ok(()) → main 이 정상 종료(exit 0). 따라서
    //   "exit 0 + 빠른 종료 + json 불변" 으로 단언한다(중복 전용 특수 코드는 없음 — 그 사실 명시).
    #[tokio::test]
    #[ignore = "실프로세스 2개 필요 — `-- --ignored` 로 실행(Windows 전용)"]
    async fn ignored_single_instance_second_rejected() {
        // ★single-instance 충돌을 의도적으로 유발★: A·B 가 **같은 instance_key + data_dir** 를 쓴다.
        //   다른 테스트와는 유니크한 key 라 격리되지만, 이 테스트 내부 두 데몬은 동일 key 라 같은 mutex 를
        //   다퉈 B 가 거부된다(검증 목적). 다른 ignored 테스트가 병렬로 돌아도 key 가 달라 영향 없음.
        let ctx = fresh_iso("single");
        let data_dir = ctx.data_dir.clone();
        let key = ctx.instance_key.clone();

        // 1) 데몬 A — daemon.json 발행 확인 + 원본 정보 보관.
        let mut daemon_a = spawn_daemon_with_key(&data_dir, &key);
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

        // 2) 데몬 B — 같은 instance_key + data_dir 로 spawn. named mutex 거부 → 빠르게 정상 종료해야 한다.
        let mut daemon_b = spawn_daemon_with_key(&data_dir, &key);
        let exited_fast = poll_until(std::time::Duration::from_secs(3), || {
            matches!(daemon_b.try_wait(), Ok(Some(_)))
        });

        // B 종료 상태 회수(아직이면 정리에서 kill).
        let b_status = daemon_b.try_wait().ok().flatten();

        // 3) A 의 daemon.json 이 보존됐는지(B 가 안 덮어씀) — pid/token 동일.
        let info_after = poll_daemon_json(&data_dir, std::time::Duration::from_secs(2));

        // 정리 — A kill, B 가 혹시 살아있으면 kill, B stderr 회수(진단), 디렉토리 삭제.
        kill_daemon(&mut daemon_a);
        if b_status.is_none() {
            let _ = daemon_b.kill();
            let _ = daemon_b.wait();
        }
        let b_logs = drain_logs(&mut daemon_b);
        let _ = std::fs::remove_dir_all(&data_dir);

        // 단언: B 가 3s 내 종료.
        assert!(
            exited_fast,
            "두 번째 데몬은 mutex 거부로 빠르게(3s 내) 종료해야 — B 로그:\n{b_logs}"
        );
        // 단언: B 가 정상 종료(exit 0). 중복은 run() 이 Ok → exit 0(중복 전용 코드 없음).
        if let Some(status) = b_status {
            assert!(
                status.success(),
                "두 번째 데몬은 정상 종료(exit 0)해야 — got {status:?}, B 로그:\n{b_logs}"
            );
        } else {
            panic!("두 번째 데몬이 3s 내 종료하지 않음(mutex 거부 실패 가능) — B 로그:\n{b_logs}");
        }
        // 단언: A 의 daemon.json 이 보존(B 가 안 덮어씀) — pid/token 동일.
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
    // 검증(선택지 A — 데몬 자가 발행): 죽은 PID 를 가진 stale daemon.json 을 임시 data_dir 에 써둠 →
    //   데몬 .exe spawn(같은 data_dir) → 데몬이 is_stale 판정 후 자기 pid/port/token/start_time 으로
    //   덮어쓰는지(run() 2.5단계) 확인. 새 pid != stale pid, start_time != 0, port/token 발행.
    //
    // src-tauri 의 ensure_daemon(WMI spawn) 경로는 별도 테스트(discovery::real_wmi_spawn_smoke)로
    //   분리해 채운다(daemon crate 에서 src-tauri 함수 호출 불가). 그건 같은 step7 에서 구현.
    #[tokio::test]
    #[ignore = "실프로세스 + 파일 discovery 필요 — `-- --ignored` 로 실행(Windows 전용)"]
    async fn ignored_stale_daemon_json_discovery() {
        let ctx = fresh_iso("stale");
        let data_dir = ctx.data_dir.clone();

        // 1) 죽은 PID 를 가진 stale daemon.json 을 써둔다.
        //    죽은 PID 만들기: 짧은 자식 spawn 후 kill → 그 PID 는 곧 dead(creation time 없음 → is_stale).
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
            token: "d".repeat(64), // stale 토큰(데몬이 새 것으로 바꿔야 함)
            protocol_version: PROTOCOL_VERSION,
            // ★start_time 은 0 이 아닌 임의값★. is_stale 로직: start_time==0 이면 pid_alive() 로
            //   fallback 하는데, pid_alive() 는 OpenProcess 실패(죽은 PID) 시 *보수적으로 true*(살아있음)
            //   를 반환해 stale 판정이 안 된다 → 데몬이 "살아있는 데몬 보호"로 종료해버린다(run() 2.5).
            //   start_time!=0 이면 process_creation_time(dead_pid)=None(죽음) + expected!=0 → false(dead)
            //   로 확실히 stale 판정된다(이 분기가 stale 판정을 보장).
            start_time: 0xDEAD_BEEF,
        };
        let stale_json = serde_json::to_vec_pretty(&stale).expect("stale 직렬화");
        std::fs::write(data_dir.join("daemon.json"), &stale_json).expect("stale daemon.json 작성");

        // 2) 데몬 .exe spawn — stale 을 감지하고 덮어써야 한다.
        let mut daemon = spawn_daemon_iso(&ctx);

        // 3) daemon.json 이 새 데몬 정보(살아있는 pid != dead_pid)로 바뀔 때까지 폴링.
        let mut latest: Option<DaemonInfo> = None;
        let overwritten = poll_until(std::time::Duration::from_secs(15), || {
            latest = poll_daemon_json(&data_dir, std::time::Duration::from_millis(100));
            matches!(&latest, Some(i) if i.pid != dead_pid)
        });

        // 정리 — 데몬 kill, stderr 회수(진단), 임시 디렉토리 삭제.
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
