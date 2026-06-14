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

use engram_dashboard_core::pty::profile::{AgentCommand, AgentProfile, SpawnMode};
use engram_dashboard_daemon::{start_test_server, TestServerHandle};
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
}

/// Client 메서드를 모듈 자유 함수로 노출(slow consumer 케이스 가독성).
async fn expect_closed_within(c: &mut Client, deadline: Duration) -> bool {
    c.expect_closed_within(deadline).await
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
    c.send(&WireCommand::ListAgents).await;
    match c.next_event().await {
        AgentEvent::AgentListUpdated { agents } => {
            assert!(
                agents.iter().any(|a| a.id == profile_id),
                "ListAgents 응답이 와야(Resize 는 무응답이어야)"
            );
        }
        ev => panic!("Resize 후 첫 control 은 ListAgents 응답이어야(Resize 무응답), got {ev:?}"),
    }

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
    c.send(&WireCommand::ListAgents).await;
    loop {
        match c.next().await.expect("ListAgents 응답 전 끊김") {
            Incoming::Event(AgentEvent::AgentListUpdated { .. }) => break,
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

    // 명시 WS ListAgents → AgentListUpdated(그 agent 포함).
    c.send(&WireCommand::ListAgents).await;
    match c.next_event().await {
        AgentEvent::AgentListUpdated { agents } => {
            assert!(
                agents.iter().any(|a| a.id == profile_id),
                "ListAgents 응답에 spawn 한 agent 포함"
            );
        }
        ev => panic!("AgentListUpdated 기대, got {ev:?}"),
    }

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
    c.send(&WireCommand::ListAgents).await;
    match c.next_event().await {
        AgentEvent::AgentListUpdated { .. } => {}
        ev => panic!("거부 후 서버가 살아있어 ListAgents 응답해야, got {ev:?}"),
    }

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
    c.send(&WireCommand::ListAgents).await;
    match c.next_event().await {
        AgentEvent::AgentListUpdated { .. } => {}
        ev => panic!("2차 Auth Error 후에도 연결 유지·동작해야, got {ev:?}"),
    }

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
    c.send(&WireCommand::ListAgents).await;
    match c.next_event().await {
        AgentEvent::AgentListUpdated { .. } => {}
        ev => panic!("파싱 실패 Error 후에도 연결 유지·동작해야, got {ev:?}"),
    }

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
// 실프로세스 전용 케이스 (#[ignore]) — in-process 로는 검증 불가, 수동 실행.
//
// 아래는 실제 데몬 .exe / OS Job Object / named mutex / 파일시스템 discovery 가 필요해
// in-process 서버로는 재현할 수 없다. 기본 `cargo test` 에서 제외(#[ignore])하고, 수동으로
//   cargo test -p engram-dashboard-daemon --test ws_e2e -- --ignored --nocapture
// 로 돌린다. ★현재는 스캐폴드(미구현)★ — 실제 .exe 기동/검증 로직은 후속 단위에서 채운다.
// 은폐 금지: 이 경로들은 지금 자동 검증되지 않음을 명시한다.
// ══════════════════════════════════════════════════════════════════════════════════

/// 데몬 .exe kill → PTY child(자식 프로세스)가 Job(KILL_ON_JOB_CLOSE)으로 동반 정리되는지.
/// 검증법(수동): 데몬 .exe 를 spawn → WS 로 shell agent spawn → 자식 PID 기록 →
///   데몬 프로세스를 TerminateProcess → 자식 PID 가 사라지는지 폴링.
#[tokio::test]
#[ignore = "실프로세스/Job 필요 — 수동 실행(미구현)"]
async fn ignored_daemon_kill_cleans_pty_child() {
    // ★위장 금지(M2)★: 빈 body 면 `--ignored` 실행 시 PASS 로 집계돼 "검증됨"으로 오인된다.
    // unimplemented! 로 두어 실행 시 panic(RED) 이 나게 한다 — 미구현이 통과로 둔갑하지 않게.
    //
    // 무엇을: 데몬 .exe 를 별도 OS 프로세스로 spawn → WS 로 shell agent spawn → 그 PTY 자식 PID
    //   기록 → 데몬 프로세스를 TerminateProcess → 자식 PID 가 함께 사라지는지(KILL_ON_JOB_CLOSE) 폴링.
    // 왜 실프로세스: in-process 서버는 데몬과 같은 프로세스라 "데몬 프로세스 종료 → Job 핸들 close →
    //   자식 동반 사망" 의 인과를 재현할 수 없다(같은 프로세스를 죽이면 테스트 자신이 죽음).
    // 언제: phase 2 step 7(실 .exe 기동 하네스 bin) 에서 데몬 바이너리 spawn 유틸과 함께 구현.
    unimplemented!("Step6 후속: 데몬 .exe kill→PTY 자식 Job 동반 정리 — 실프로세스/Job 필요");
}

/// single-instance: 두 번째 데몬 .exe 기동이 named mutex 로 거부(정상 종료 + daemon.json 미덮어쓰기)되는지.
/// 검증법(수동): 데몬 A 기동 → 데몬 B 기동 → B 가 즉시 종료(exit 0)하고 A 의 daemon.json 이 보존되는지.
#[tokio::test]
#[ignore = "실프로세스 2개 필요 — 수동 실행(미구현)"]
async fn ignored_single_instance_second_rejected() {
    // ★위장 금지(M2)★: unimplemented! 로 두어 `--ignored` 실행 시 panic(RED). 빈 body 면 PASS 둔갑.
    //
    // 무엇을: 데몬 A 를 .exe 로 기동 → 데몬 B 를 .exe 로 기동 → B 가 named mutex 로 거부돼 즉시
    //   정상 종료(exit 0)하고 A 의 daemon.json 이 보존(미덮어쓰기)되는지 확인.
    // 왜 실프로세스: single-instance 가드는 named mutex 로 구현되는데, 같은 프로세스 안에서는
    //   mutex 재획득이 막히는 의미가 없다(다른 OS 프로세스라야 단일성 검증이 성립).
    // 언제: phase 2 step 7 에서 데몬 .exe 2회 기동 하네스로 구현.
    unimplemented!("Step6 후속: 두 번째 데몬 .exe 가 named mutex 로 거부 — 실프로세스 2개 필요");
}

/// stale daemon.json + discovery spawn(Step5 real_wmi_spawn_smoke 연계).
/// 검증법(수동): stale(죽은 PID) daemon.json 을 둔 뒤 데몬 .exe 기동 → 덮어쓰고 새 토큰/포트 발행되는지.
#[tokio::test]
#[ignore = "실프로세스 + 파일시스템 discovery 필요 — 수동 실행(미구현)"]
async fn ignored_stale_daemon_json_discovery() {
    // ★위장 금지(M2)★: unimplemented! 로 두어 `--ignored` 실행 시 panic(RED). 빈 body 면 PASS 둔갑.
    //
    // 무엇을: stale(죽은 PID) daemon.json 을 data_dir 에 둔 뒤 데몬 .exe 기동 → 그 파일을 덮어쓰고
    //   새 토큰/포트/PID 가 발행되는지(stale 검사 → 덮어쓰기 경로, lib.rs run() 의 2.5단계) 확인.
    // 왜 실프로세스: run() 은 실제 data_dir(파일시스템)과 자기 PID 로 stale 을 판정·기록한다.
    //   in-process start_test_server 는 이 daemon.json/파일 IO 경로를 의도적으로 생략(격리)하므로
    //   discovery 를 재현할 수 없다.
    // 언제: phase 2 step 7 에서 임시 data_dir + .exe 기동 하네스로 구현.
    unimplemented!(
        "Step6 후속: stale daemon.json 덮어쓰기 + 새 발행 — 실프로세스/파일 discovery 필요"
    );
}
