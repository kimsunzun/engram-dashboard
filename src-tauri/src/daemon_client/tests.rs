//! DaemonClient 핸드셰이크/연결 단위·integration 테스트 (S14 모듈① T2).
//!
//! `src/api/wsTransport.test.ts` 의 connect/handshake 케이스를 Rust 로 이식한다(명세서 보존).
//! 매핑(TS 케이스 ↔ 이 파일):
//!   - TS `'... Auth 전송 + Hello 로 connected'`          → `connect_sends_auth_first_frame`
//!   - TS `'Hello/Auth 는 onMessage 로 안 올라온다'`       → `hello_consumed_internally_reaches_connected`
//!     ★부분 이식★: control 라우팅 표면이 아직 없어(T5) "Hello 가 control 로 안 샌다"를 직접 단언
//!     못 한다 — connected 도달(= Hello 가 핸드셰이크에서 내부 소비됨)로만 우회 검증한다. Hello 가
//!     실제로 위로 누수되지 않는지의 강한 단언은 라우팅 표면이 생기는 T5 몫.
//!   - TS `'ensureReady = 캐시 없으면 reject + spawn 0회'`  → `ensure_no_spawn_when_no_daemon`
//!   - TS `'start 만 discover_daemon 호출(spawn 유발)'`     → `connect_may_spawn`
//!
//! 적대 리뷰 회귀 가드(이식이 아닌 결함 재현 방지):
//!   - Fix C(protocol_version echo)  → `auth_sends_compiled_protocol_version_not_echo`
//!   - Fix B(연결 lifecycle race)    → `concurrent_connect_settles_connected_no_flap`(핸드셰이크 단계
//!                                     self-close) · `close_in_flight_stays_down_no_revival`(지연 Hello
//!                                     부활 차단) · `lifecycle_guard_blocks_stale_publish`(가드 판정점 단위)
//!                                     · `lifecycle_close_clears_cmd_and_blocks_stale_revival`(close 원자성)
//!                                     · `connected_then_close_reconnect_no_down_clobber`(main_loop 종료
//!                                     Down 가드 — connected 이후 stale 종료가 새 연결을 clobber 안 함)
//!   - Fix B(TOCTOU 회귀 — 체크+변경 원자화, 결정론적 단위) → `guard_stale_down_cannot_clobber_current_connected`
//!                                     (stale Down 차단) · `guard_concurrent_connect_settles_to_newest_generation`
//!                                     (최신 세대 수렴) · `guard_store_cmd_rejects_stale_generation`
//!                                     (cmd_tx 가드) · `guard_close_blocks_stale_revival`(close 후 stale 부활
//!                                     차단). 옛 확률적 `toctou_stress_*` 2개를 supersede — 가드 메서드를
//!                                     소켓·서버·sleep 없이 직접 순서 호출해 가드의 *논리 계약*(stale→거부,
//!                                     current→허용)을 결정론적으로 증명한다. 비교+변경의 *원자성*(동시
//!                                     스레드에서 진짜 안 깨짐)은 std Mutex 가 보장하며 이 단위 테스트가
//!                                     증명하는 게 아니다(그건 loom 영역). 실 소켓 race 통합 wiring 은 위쪽
//!                                     single-shot 결정론 회귀 테스트가 커버한다.
//!   - Fix A(핸드셰이크 timeout)      → `handshake_times_out_when_server_silent`
//!
//! ## 격리(실 데몬 없이)
//! - **실 핸드셰이크**: daemon crate `start_test_server()`(in-process WS 서버) 재사용 — connect→Auth
//!   →Hello→connected 를 실제 서버 응답으로 단언(`hello_consumed_internally_reaches_connected`).
//! - **첫 프레임 캡처**: 작은 mock WS 서버를 테스트 task 로 띄워 클라가 보낸 **첫 frame 이 Auth** 인지
//!   직접 본 뒤 Hello 로 응답한다(`connect_sends_auth_first_frame`). FakeWebSocket(TS)의 parsedSent()[0] 대응.
//! - **spawn 가드**: `DaemonDiscovery` mock 의 호출 카운터로 ensure(no-spawn)/connect(spawn 가능)를 단언.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use engram_dashboard_protocol::{AgentCommand, AgentEvent, DaemonInfo, PROTOCOL_VERSION};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::runtime::Handle;
use tokio_tungstenite::tungstenite::Message;

use super::connection::HandshakeError;
use super::lifecycle::Lifecycle;
use super::{ConnectionState, DaemonClient, DaemonDiscovery};

// ── mock DaemonDiscovery ────────────────────────────────────────────────────────────
//
// ensure_spawn(connect 경로)/read_live(ensure 경로) 호출 횟수를 카운트해 ADR-0021 분리를 단언한다.
// 둘 다 같은 host/port/token 을 돌려주되, "어느 메서드가 불렸는가" 로 spawn 가능/no-spawn 을 가린다.

struct MockDiscovery {
    /// read_live 가 돌려줄 값(None=살아있는 데몬 없음). ensure(no-spawn)가 본다.
    live: Option<DaemonInfo>,
    /// ensure_spawn 이 돌려줄 값(connect 경로 = spawn 가능).
    spawn_result: Result<DaemonInfo, String>,
    ensure_spawn_calls: Arc<AtomicUsize>,
    read_live_calls: Arc<AtomicUsize>,
}

impl MockDiscovery {
    fn new(live: Option<DaemonInfo>, spawn_result: Result<DaemonInfo, String>) -> Self {
        Self {
            live,
            spawn_result,
            ensure_spawn_calls: Arc::new(AtomicUsize::new(0)),
            read_live_calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl DaemonDiscovery for MockDiscovery {
    fn ensure_spawn(&self, _timeout: Duration) -> Result<DaemonInfo, String> {
        self.ensure_spawn_calls.fetch_add(1, Ordering::SeqCst);
        self.spawn_result.clone()
    }

    fn read_live(&self) -> Option<DaemonInfo> {
        self.read_live_calls.fetch_add(1, Ordering::SeqCst);
        self.live.clone()
    }
}

fn info_for(port: u16, token: &str) -> DaemonInfo {
    info_for_version(port, token, PROTOCOL_VERSION)
}

/// protocol_version 을 임의로 지정한 DaemonInfo(Fix C 회귀 테스트가 틀린 값을 주입하는 용도).
fn info_for_version(port: u16, token: &str, protocol_version: u32) -> DaemonInfo {
    DaemonInfo {
        pid: 4321,
        host: "127.0.0.1".into(),
        port,
        token: token.to_string(),
        protocol_version,
        start_time: 0,
    }
}

// ── mock WS 서버 ──────────────────────────────────────────────────────────────────────
//
// 127.0.0.1:0 에 bind 해 한 연결을 받아: 첫 frame 을 캡처해 oneshot 으로 돌려주고, Hello 를 응답한다.
// (데몬 ws.rs 의 Auth→Hello 흐름을 핸드셰이크 검증에 필요한 만큼만 흉내낸다.)

/// 첫 수신 frame 을 oneshot 으로 보고하는 mock 서버를 띄운다. 반환: (port, 첫프레임 수신 future).
async fn spawn_mock_server_capturing_first_frame() -> (u16, tokio::sync::oneshot::Receiver<String>)
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (first_tx, first_rx) = tokio::sync::oneshot::channel::<String>();

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        // 첫 frame 캡처(클라가 가장 먼저 보내는 것 = Auth 여야 한다).
        if let Some(Ok(Message::Text(text))) = ws.next().await {
            let _ = first_tx.send(text.to_string());
        }
        // Hello 응답(인증 성공). 데몬 ws.rs hello_event 형태와 동일 enum.
        let hello = serde_json::to_string(&AgentEvent::Hello {
            protocol_version: PROTOCOL_VERSION,
            daemon_version: "test".into(),
            capabilities: None,
        })
        .unwrap();
        let _ = ws.send(Message::Text(hello.into())).await;
        // 연결을 잠시 유지(클라 connected 전이까지) — drop 으로 닫히면 클라 메인 루프가 Down 으로 갈 뿐.
        tokio::time::sleep(Duration::from_millis(500)).await;
    });

    (port, first_rx)
}

// ── 케이스 ① connect 첫 송신 frame = Auth (TS: '... Auth 전송 + Hello 로 connected') ──────
// ★multi-thread★: 연결 task(run_connection) + mock/실 서버 task + connect await 가 동시에 진행돼야
// 핸드셰이크가 데드락 없이 돈다(spike §2 tokio multi-thread). current-thread 면 spawn 된 task 가 await
// 양보 시에만 돌아 핸드셰이크가 막힐 수 있다.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_sends_auth_first_frame() {
    let (port, first_rx) = spawn_mock_server_capturing_first_frame().await;
    let token = "test-token-aaaa".to_string();
    let disco = Arc::new(MockDiscovery::new(
        None,                       // ensure 는 안 쓰는 케이스
        Ok(info_for(port, &token)), // connect = ensure_spawn → mock 서버로 attach
    ));
    let client = DaemonClient::new(Handle::current(), disco);

    client
        .connect()
        .await
        .expect("connect 가 Hello 로 connected 돼야");
    assert_eq!(client.state(), ConnectionState::Connected);

    // ★첫 frame 이 Auth(token 은 DaemonInfo, protocol_version 은 자기 컴파일 버전)★
    //   — wsTransport.test.ts parsedSent()[0] 대응. protocol_version 단언은 echo 가 아니라
    //   "컴파일된 PROTOCOL_VERSION 송신"(Fix C). echo 회귀는 별도 테스트가 틀린 값으로 잡는다.
    let first = first_rx.await.expect("첫 frame 수신");
    let cmd: AgentCommand = serde_json::from_str(&first).expect("첫 frame 은 valid AgentCommand");
    match cmd {
        AgentCommand::Auth {
            token: t,
            protocol_version,
        } => {
            assert_eq!(t, token, "Auth.token 이 DaemonInfo.token 을 그대로 싣는다");
            assert_eq!(
                protocol_version, PROTOCOL_VERSION,
                "protocol_version 은 자기 컴파일 버전(Fix C)"
            );
        }
        other => panic!("첫 frame 은 Auth 여야 하는데 {other:?}"),
    }

    client.close();
    assert_eq!(client.state(), ConnectionState::Down);
}

// ── 케이스 ② Hello 내부 소비 → connected (TS: 'Hello/Auth 는 onMessage 로 안 올라온다') ─────
// ★부분 이식(T5 에서 강화 필요)★: 실 데몬(start_test_server)로 핸드셰이크 전체를 돌려 Hello→connected
// 를 단언한다. 그러나 T2 는 control 이벤트를 위로 올리는 라우팅 표면(T5)이 아직 없어, "Hello 가
// control 로 안 샌다"를 직접 단언하지 못한다 — connected 도달(= Hello 가 핸드셰이크에서 내부 소비됨)
// 으로만 우회 검증한다. Hello 누수 자체를 막는 강한 단언은 라우팅 표면이 생기는 T5 몫이다(현재
// 매핑표의 "이식됨"은 과대표기였어서 정정).
// ★multi-thread★: 연결 task(run_connection) + mock/실 서버 task + connect await 가 동시에 진행돼야
// 핸드셰이크가 데드락 없이 돈다(spike §2 tokio multi-thread). current-thread 면 spawn 된 task 가 await
// 양보 시에만 돌아 핸드셰이크가 막힐 수 있다.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hello_consumed_internally_reaches_connected() {
    let server = engram_dashboard_daemon::start_test_server()
        .await
        .expect("test server");
    let disco = Arc::new(MockDiscovery::new(
        Some(info_for(server.port, &server.token)), // ensure 도 가능하게(같은 데몬)
        Ok(info_for(server.port, &server.token)),
    ));
    let client = DaemonClient::new(Handle::current(), disco);

    // connect: discover(mock ensure_spawn) → 실 데몬 WS → Auth → 데몬 Hello → connected.
    client
        .connect()
        .await
        .expect("실 데몬 핸드셰이크가 connected 로");
    assert_eq!(
        client.state(),
        ConnectionState::Connected,
        "Hello 내부 소비 후 connected"
    );

    client.close();
    server.shutdown().await;
}

// ── 케이스 ③ ensure(attach-only)는 데몬 없으면 spawn 0회 (TS: 'ensureReady = ... spawn 0회') ──
// ★multi-thread★: 연결 task(run_connection) + mock/실 서버 task + connect await 가 동시에 진행돼야
// 핸드셰이크가 데드락 없이 돈다(spike §2 tokio multi-thread). current-thread 면 spawn 된 task 가 await
// 양보 시에만 돌아 핸드셰이크가 막힐 수 있다.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ensure_no_spawn_when_no_daemon() {
    let disco = Arc::new(MockDiscovery::new(
        None,                                     // 살아있는 데몬 없음(read_live=None)
        Ok(info_for(9999, "should-not-be-used")), // ensure_spawn 이 불리면 안 됨
    ));
    let ensure_spawn_calls = disco.ensure_spawn_calls.clone();
    let read_live_calls = disco.read_live_calls.clone();
    let client = DaemonClient::new(Handle::current(), disco);

    // ★ADR-0021★: ensure 는 read_live(no-spawn)만 본다 → None 이면 NoLiveDaemon 으로 실패.
    let err = client.ensure().await.expect_err("데몬 없으면 ensure 실패");
    assert_eq!(err, HandshakeError::NoLiveDaemon);

    // ★불변식 단언★: ensure_spawn(=spawn 유발) 0회, read_live 만 1회. 명령/ensure 가 데몬 못 깨움.
    assert_eq!(
        ensure_spawn_calls.load(Ordering::SeqCst),
        0,
        "ensure 는 spawn(ensure_spawn) 절대 호출 안 함"
    );
    assert_eq!(
        read_live_calls.load(Ordering::SeqCst),
        1,
        "ensure 는 read_live(no-spawn)만 호출"
    );
    assert_eq!(client.state(), ConnectionState::Down);
}

// ── 케이스 ④ connect 는 spawn 가능 (TS: 'start 만 discover_daemon 호출(spawn 유발)') ─────────
// ★multi-thread★: 연결 task(run_connection) + mock/실 서버 task + connect await 가 동시에 진행돼야
// 핸드셰이크가 데드락 없이 돈다(spike §2 tokio multi-thread). current-thread 면 spawn 된 task 가 await
// 양보 시에만 돌아 핸드셰이크가 막힐 수 있다.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_may_spawn() {
    let (port, _first_rx) = spawn_mock_server_capturing_first_frame().await;
    let token = "spawn-token".to_string();
    let disco = Arc::new(MockDiscovery::new(
        None,                       // read_live=None: connect 가 read_live 를 안 쓴다는 것도 확인
        Ok(info_for(port, &token)), // ensure_spawn 이 접속 정보를 준다(spawn 성공 흉내)
    ));
    let ensure_spawn_calls = disco.ensure_spawn_calls.clone();
    let read_live_calls = disco.read_live_calls.clone();
    let client = DaemonClient::new(Handle::current(), disco);

    client.connect().await.expect("connect 성공");
    assert_eq!(client.state(), ConnectionState::Connected);

    // ★connect = spawn 경로★: ensure_spawn 1회(데몬 없어도 띄울 수 있음), read_live 0회.
    assert_eq!(
        ensure_spawn_calls.load(Ordering::SeqCst),
        1,
        "connect 는 ensure_spawn(spawn 가능) 경로"
    );
    assert_eq!(
        read_live_calls.load(Ordering::SeqCst),
        0,
        "connect 는 read_live(no-spawn) 경로를 타지 않음"
    );

    client.close();
}

// ── 보강: connect 가 ensure_spawn 실패를 핸드셰이크 에러로 전파 ────────────────────────────
// ★multi-thread★: 연결 task(run_connection) + mock/실 서버 task + connect await 가 동시에 진행돼야
// 핸드셰이크가 데드락 없이 돈다(spike §2 tokio multi-thread). current-thread 면 spawn 된 task 가 await
// 양보 시에만 돌아 핸드셰이크가 막힐 수 있다.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_propagates_spawn_failure() {
    let disco = Arc::new(MockDiscovery::new(
        None,
        Err("spawn timeout".into()), // ensure_spawn 실패
    ));
    let client = DaemonClient::new(Handle::current(), disco);

    let err = client
        .connect()
        .await
        .expect_err("spawn 실패면 connect 실패");
    assert!(
        matches!(err, HandshakeError::Discovery(_)),
        "spawn 실패는 Discovery 에러로: {err:?}"
    );
    assert_eq!(client.state(), ConnectionState::Down);
}

// ── 보강: 이미 connected 면 connect 는 즉시 Ok + 재spawn 안 함 ─────────────────────────────
// ★multi-thread★: 연결 task(run_connection) + mock/실 서버 task + connect await 가 동시에 진행돼야
// 핸드셰이크가 데드락 없이 돈다(spike §2 tokio multi-thread). current-thread 면 spawn 된 task 가 await
// 양보 시에만 돌아 핸드셰이크가 막힐 수 있다.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_idempotent_when_already_connected() {
    let (port, _first_rx) = spawn_mock_server_capturing_first_frame().await;
    let disco = Arc::new(MockDiscovery::new(None, Ok(info_for(port, "tok"))));
    let ensure_spawn_calls = disco.ensure_spawn_calls.clone();
    let client = DaemonClient::new(Handle::current(), disco);

    client.connect().await.expect("first connect");
    assert_eq!(client.state(), ConnectionState::Connected);

    // 두 번째 connect — 이미 connected 라 즉시 Ok, ensure_spawn 추가 호출 없음(중복 연결 방지).
    client.connect().await.expect("second connect noop");
    assert_eq!(
        ensure_spawn_calls.load(Ordering::SeqCst),
        1,
        "이미 connected 면 재spawn 안 함"
    );

    client.close();
}

// ── 추가 mock 서버(적대 리뷰 회귀 가드용) ───────────────────────────────────────────────────

/// 여러 연결을 받아 각각 Hello 로 응답하는 mock 서버(동시 connect 테스트용 — 짧은 순간 소켓 2개가
/// 동시에 열릴 수 있으므로 1개만 받으면 둘째 task 의 connect_async 가 막힌다). 연결을 잠시 유지한다.
async fn spawn_mock_server_multi_accept() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                // 첫 frame(Auth) 소비.
                let _ = ws.next().await;
                let hello = serde_json::to_string(&AgentEvent::Hello {
                    protocol_version: PROTOCOL_VERSION,
                    daemon_version: "test".into(),
                    capabilities: None,
                })
                .unwrap();
                let _ = ws.send(Message::Text(hello.into())).await;
                // connected 전이까지 유지(drop 으로 닫히면 클라 메인 루프가 Down 으로 갈 뿐).
                tokio::time::sleep(Duration::from_millis(500)).await;
            });
        }
    });
    port
}

/// Auth 수신을 신호한 뒤 Hello 를 **지연** 전송하는 mock 서버(close-in-flight 테스트용).
/// 반환: (port, auth 수신 신호 수신 future). 테스트는 auth 신호를 받고 close() 를 부른 뒤,
/// 서버가 지연 후 Hello 를 보내도 클라가 stale 이라 Connected 로 부활하지 않음을 단언한다.
async fn spawn_mock_server_delayed_hello() -> (u16, tokio::sync::oneshot::Receiver<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (auth_tx, auth_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        // 첫 frame(Auth) 수신 → 테스트에 신호(이 시점에 클라는 핸드셰이크 중 = in-flight).
        let _ = ws.next().await;
        let _ = auth_tx.send(());
        // Hello 를 일부러 늦게 보낸다 — 그 사이 테스트가 close() 로 세대를 올린다.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let hello = serde_json::to_string(&AgentEvent::Hello {
            protocol_version: PROTOCOL_VERSION,
            daemon_version: "test".into(),
            capabilities: None,
        })
        .unwrap();
        let _ = ws.send(Message::Text(hello.into())).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    });
    (port, auth_rx)
}

/// accept 만 하고 Hello/Error/Close 중 무엇도 안 보내는 **침묵** mock 서버(timeout 테스트용).
/// Auth 를 읽기만 하고 영원히 잠잔다 — 클라의 wait_for_hello 가 무한 대기에 빠지는 상황 재현.
async fn spawn_mock_server_silent() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _ = ws.next().await; // Auth 소비
                                 // 의도적으로 아무 응답도 안 함 → 소켓을 살려둔 채 침묵.
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(ws);
    });
    port
}

// ── Fix C 회귀: Auth 는 echo 가 아니라 컴파일된 PROTOCOL_VERSION 을 보낸다 ────────────────────
// DaemonInfo.protocol_version 을 틀린 값(999)으로 줘도, 송신된 Auth.protocol_version 은 컴파일된
// PROTOCOL_VERSION 이어야 한다. echo(info 값 되쏘기)면 999 가 나가 이 단언이 깨진다 → 버전 게이트
// 무력화 회귀를 잡는다.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_sends_compiled_protocol_version_not_echo() {
    let (port, first_rx) = spawn_mock_server_capturing_first_frame().await;
    let token = "ver-token".to_string();
    // ★틀린 버전 주입★: 999. echo 결함이면 이게 그대로 wire 로 나간다.
    let wrong = 999u32;
    assert_ne!(
        wrong, PROTOCOL_VERSION,
        "테스트 전제: 주입값이 실제 버전과 달라야"
    );
    let disco = Arc::new(MockDiscovery::new(
        None,
        Ok(info_for_version(port, &token, wrong)),
    ));
    let client = DaemonClient::new(Handle::current(), disco);

    client.connect().await.expect("connect");

    let first = first_rx.await.expect("첫 frame");
    let cmd: AgentCommand = serde_json::from_str(&first).expect("valid AgentCommand");
    match cmd {
        AgentCommand::Auth {
            protocol_version, ..
        } => {
            assert_eq!(
                protocol_version, PROTOCOL_VERSION,
                "Auth 는 컴파일된 PROTOCOL_VERSION 을 보내야(echo 아님). 받은 값 {protocol_version}"
            );
            assert_ne!(
                protocol_version, wrong,
                "DaemonInfo 가 준 틀린 버전(999)을 echo 하면 안 됨"
            );
        }
        other => panic!("Auth 여야 하는데 {other:?}"),
    }
    client.close();
}

// ── Fix B 회귀: 동시 connect 가 Connected 로 수렴(Down 으로 flap 안 함, 고아 채널/좀비 없음) ────
// 두 connect() 를 동시에 던지면 둘 다 generation 을 bump 하고 둘 다 소켓을 연다(짧은 순간 2개 허용).
// 최신 세대 task 만 Connected 를 송신하고, 밀려난 task 는 self-close(공유 상태 미접촉)한다. 결과적으로
// 고아 Down clobber 없이 최종 상태가 Connected 여야 한다. (Down 가드 없으면 stale task 의 종료가
// Connected 를 Down 으로 덮어쓰는 flap 이 발생.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_connect_settles_connected_no_flap() {
    let port = spawn_mock_server_multi_accept().await;
    let disco = Arc::new(MockDiscovery::new(
        None,
        Ok(info_for(port, "concurrent-tok")),
    ));
    let client = Arc::new(DaemonClient::new(Handle::current(), disco));

    // 동시 connect 2회(join 으로 진짜 병렬 진행 — panic 없이 둘 다 완주해야).
    // ★계약(generation 씨앗)★: 동시 호출이면 둘 다 세대를 bump 하므로 밀려난(stale) 쪽은 의도적으로
    //   실패(TaskGone — self-close 로 ready_tx drop)할 수 있다. 이건 결함이 아니라 stale caller 가
    //   자기가 밀린 걸 인지하는 Fix B 의 메커니즘 그대로다(mod.rs Err(_) arm 주석 참조). 따라서
    //   "둘 다 Ok"를 강요하지 않는다 — 패닉 없이 둘 다 완주하고 ① 최소 하나는 Ok(최신 gen task 가
    //   connected) ② 최종 상태가 Connected(고아 Down clobber 로 flap 안 함) 면 통과다.
    let c1 = client.clone();
    let c2 = client.clone();
    let (r1, r2) = tokio::join!(c1.connect(), c2.connect());
    assert!(
        r1.is_ok() || r2.is_ok(),
        "동시 connect 중 최신 세대 task 는 connected 로 성공해야: r1={r1:?} r2={r2:?}"
    );

    // 밀려난 task 의 self-close 가 비동기로 끝날 시간을 잠깐 준 뒤 상태가 Connected 로 안정됐는지 본다.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        client.state(),
        ConnectionState::Connected,
        "동시 connect 후 최종 상태는 Connected(고아 Down clobber 로 flap 하면 안 됨)"
    );

    client.close();
    assert_eq!(client.state(), ConnectionState::Down);
}

// ── Fix B 회귀: 핸드셰이크 중 close() → Down 유지, close 후 Connected 부활 없음 ────────────────
// Hello 를 지연하는 서버로 클라를 핸드셰이크 in-flight 상태로 만든 뒤 close() 를 호출한다. close 가
// generation 을 bump 했으므로, 뒤늦게 Hello 를 받은 연결 task 는 stale 이라 Connected 를 송신하지
// 않고 self-close 한다 → 최종 상태 Down 유지(부활 없음).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn close_in_flight_stays_down_no_revival() {
    let (port, auth_rx) = spawn_mock_server_delayed_hello().await;
    let disco = Arc::new(MockDiscovery::new(None, Ok(info_for(port, "inflight-tok"))));
    let client = Arc::new(DaemonClient::new(Handle::current(), disco));

    // connect 를 백그라운드로 시작(핸드셰이크가 Hello 대기에서 멈춰 있음).
    let c = client.clone();
    let connect_task = tokio::spawn(async move { c.connect().await });

    // 서버가 Auth 를 받은 시점 = 클라가 핸드셰이크 in-flight. 이때 close() 로 세대를 올린다.
    auth_rx.await.expect("서버가 Auth 수신 신호");
    client.close();
    assert_eq!(client.state(), ConnectionState::Down, "close 직후 Down");

    // connect 는 stale self-close 로 인해 ready 가 drop → TaskGone 으로 귀결(부활 Ok 아님).
    let connect_result = connect_task.await.expect("connect task panic 없이");
    assert!(
        connect_result.is_err(),
        "close 로 stale 된 connect 는 실패해야(Connected Ok 부활 금지): {connect_result:?}"
    );

    // 지연 Hello 가 도착해 stale task 가 처리할 시간을 준 뒤에도 Down 유지(Connected 부활 없음).
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        client.state(),
        ConnectionState::Down,
        "close 이후 stale task 의 지연 Hello 가 Connected 로 부활시키면 안 됨"
    );
}

// ── Fix A 회귀: 서버가 Hello 없이 침묵하면 핸드셰이크가 bound 내 Timeout 으로 빠진다 ──────────
// 침묵 서버(accept 만, Hello/Error/Close 안 보냄)에 connect 하면, timeout 이 없을 땐 영구 hang.
// 짧은 handshake_timeout 을 주입해 connect 가 bound 내 HandshakeError::Timeout 으로 실패함을 단언한다.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handshake_times_out_when_server_silent() {
    let port = spawn_mock_server_silent().await;
    let disco = Arc::new(MockDiscovery::new(None, Ok(info_for(port, "silent-tok"))));
    // 짧은 상한(200ms) 주입 — const 10s 를 기다리지 않게.
    let client = DaemonClient::new_with_handshake_timeout(
        Handle::current(),
        disco,
        Duration::from_millis(200),
    );

    // 전체가 bound(여유 2s) 내에 Timeout 으로 빠져야 — 무한 hang 이면 이 timeout 이 panic 으로 잡는다.
    let result = tokio::time::timeout(Duration::from_secs(2), client.connect())
        .await
        .expect("connect 가 bound 내 반환(영구 hang 아님)");
    assert_eq!(
        result,
        Err(HandshakeError::Timeout),
        "침묵 서버면 핸드셰이크는 Timeout 으로 실패해야: {result:?}"
    );
    assert_eq!(client.state(), ConnectionState::Down);
}

// ── Fix B 회귀(단위): lifecycle 가드 판정점 — stale 이면 공유 상태(watch) 미접촉 ─────────────
// main_loop/run_connection 의 "공유 상태 전이 = publish_if_current" 한 곳을 실 소켓 없이 결정적으로
// 단언한다 — 동시 connect 테스트가 핸드셰이크 단계 self-close 만 커버하는 사각(main_loop 종료 Down
// 가드)을 이 단위 테스트가 메운다. publish_if_current 가 stale 일 때 true 를 돌려주거나 watch 를
// 발행하게 깨지면(가드 무력화) stale task 의 Down 이 current 의 Connected 를 clobber 한다.
//
// ★TOCTOU 핵심★: 이전 구현은 generation(AtomicU64) load 와 watch send 가 분리돼, 그 사이 다른
// 스레드의 bump 가 끼어 stale task 가 current 의 상태를 덮었다. 지금은 비교+send 가 한 락 안이라
// 끼어들 수 없다 — 아래는 그 가드 메서드의 계약(stale→미발행, current→발행)을 단언한다.
#[test]
fn lifecycle_guard_blocks_stale_publish() {
    let (lifecycle, mut state_rx) = Lifecycle::new();
    // 세대 1 캡처(=내 my_gen). 초기 상태 Down.
    let my_gen = lifecycle.bump_and_capture(None);
    assert_eq!(my_gen, 1, "첫 bump 는 세대 1");
    assert_eq!(*state_rx.borrow(), ConnectionState::Down, "초기 Down");

    // 내 세대 == 공유 세대 → current → Connected 발행 허용.
    assert!(
        lifecycle.publish_if_current(my_gen, ConnectionState::Connected),
        "내 세대가 current 면 발행됨"
    );
    assert!(state_rx.has_changed().unwrap());
    assert_eq!(*state_rx.borrow_and_update(), ConnectionState::Connected);

    // 다른 connect/close 가 세대를 올림(bump) → 나는 stale.
    let newer_gen = lifecycle.bump_and_capture(None);
    assert_eq!(newer_gen, 2, "두 번째 bump 는 세대 2");
    // stale(옛 my_gen)로 Down 을 발행하려 해도 차단되어야(false) + watch 미변경.
    assert!(
        !lifecycle.publish_if_current(my_gen, ConnectionState::Down),
        "세대가 bump 되면 옛 task 는 stale → 공유 상태 미접촉이어야"
    );
    assert!(
        !state_rx.has_changed().unwrap(),
        "stale 발행은 watch 를 바꾸지 않는다(Connected 유지)"
    );
    assert_eq!(*state_rx.borrow(), ConnectionState::Connected);

    // 더 새 task(newer_gen) 자신은 여전히 current → 발행 가능.
    assert!(
        lifecycle.publish_if_current(newer_gen, ConnectionState::Down),
        "최신 세대 task 만 current"
    );
    assert_eq!(*state_rx.borrow_and_update(), ConnectionState::Down);
    assert_eq!(lifecycle.current_generation(), 2);
}

// ── Fix B 회귀(단위): close 는 bump + cmd_tx=None + Down 을 원자로 한다 ────────────────────────
// store_cmd_if_current 로 sender 를 저장한 뒤 close() 가 그것을 비우고(가드된 cmd_tx clear) Down 을
// 발행함을 단언한다. close 이후 옛 세대로 store_cmd_if_current 를 다시 부르면(stale 부활 시도) 저장이
// 거부되어야(false) — 좀비 sender 부활 차단.
#[test]
fn lifecycle_close_clears_cmd_and_blocks_stale_revival() {
    let (lifecycle, state_rx) = Lifecycle::new();
    let my_gen = lifecycle.bump_and_capture(Some(ConnectionState::Connecting));
    assert_eq!(*state_rx.borrow(), ConnectionState::Connecting);

    // current sender 저장 성공.
    let (tx, _rx) = tokio::sync::mpsc::channel(4);
    assert!(
        lifecycle.store_cmd_if_current(my_gen, tx),
        "current 면 cmd_tx 저장됨"
    );

    // close → bump(세대 3 으로) + cmd_tx clear + Down. 원자.
    lifecycle.close();
    assert_eq!(*state_rx.borrow(), ConnectionState::Down, "close 후 Down");
    assert_eq!(lifecycle.current_generation(), 2, "close 가 세대 bump");

    // stale(옛 my_gen)로 sender 재저장 시도 → 거부(좀비 부활 차단).
    let (tx2, _rx2) = tokio::sync::mpsc::channel(4);
    assert!(
        !lifecycle.store_cmd_if_current(my_gen, tx2),
        "close 로 stale 된 옛 세대는 cmd_tx 를 부활시킬 수 없다"
    );
}

// ── Fix B 회귀(통합): connected 이후 close → 옛 task 의 main_loop 종료가 새 연결을 clobber 안 함 ──
// 동시 connect/close-in-flight 테스트는 stale task 가 *핸드셰이크 단계*에서 self-close 하는 경로만
// 탄다(main_loop 미진입). main_loop 의 종료 Down 가드(connection.rs)는 connected 까지 간 task 가
// 나중에 stale 화되어 루프를 빠져나갈 때 발동한다 — 그 경로를 탄다: connect 로 connected → close(gen
// bump, cmd_tx drop) 로 옛 task 의 main_loop 를 stale 상태로 종료시킴 → 곧바로 새 connect 로 새 연결을
// Connected 로 만든다. 옛 task 의 종료 Down 이 가드를 통과해 broadcast 되면 새 Connected 를 Down 으로
// 덮어쓰는 flap 이 난다. 가드가 옛 task(stale)의 Down 을 삼켜야 최종 Connected 가 유지된다.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connected_then_close_reconnect_no_down_clobber() {
    let port = spawn_mock_server_multi_accept().await;
    let disco = Arc::new(MockDiscovery::new(
        None,
        Ok(info_for(port, "reconnect-tok")),
    ));
    let client = Arc::new(DaemonClient::new(Handle::current(), disco));

    // 1) 첫 connect → connected (옛 task 가 main_loop 진입).
    client.connect().await.expect("first connect");
    assert_eq!(client.state(), ConnectionState::Connected);

    // 2) close() → gen bump(옛 task stale) + cmd_tx drop(옛 task main_loop 가 cmd_rx EOF 로 종료).
    client.close();
    assert_eq!(client.state(), ConnectionState::Down, "close 직후 Down");

    // 3) 곧바로 새 connect → 새 task 가 새 소켓으로 connected. 옛 task 의 main_loop 종료 Down 이
    //    가드 없이 broadcast 되면 이 Connected 를 Down 으로 clobber 한다.
    client.connect().await.expect("reconnect");
    assert_eq!(
        client.state(),
        ConnectionState::Connected,
        "재연결 후 Connected"
    );

    // 4) 옛 task 의 main_loop 종료 Down 이 (있었다면) 도착할 충분한 시간을 준 뒤에도 Connected 유지.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        client.state(),
        ConnectionState::Connected,
        "옛(stale) task 의 main_loop 종료 Down 이 새 Connected 를 clobber 하면 안 됨"
    );

    client.close();
    assert_eq!(client.state(), ConnectionState::Down);
}

// ── Fix B 회귀(결정론적 단위): TOCTOU 가드 불변식 — 소켓·서버·sleep 0, 가드 메서드 직접 순서 호출 ──
//
// ★왜 확률적 stress 를 이 결정론적 단위로 교체했나 (다음 세션이 매직타이밍 stress 를 재도입하지 말 것)★
// 이전엔 `toctou_stress_*` 두 테스트가 mock 서버 hold(5/30ms) vs assert sleep(3ms) 의 매직 타이밍에
// 의존해 race 창을 *확률적으로* 때렸다. 그 타이밍 가정이 깨지면 current task 의 *정상* Down 을 stale
// clobber 로 오판하는 false positive 가 났다(baseline 에서도 ~1/20 간헐 실패). cross-family 리서치
// (docs/research/toctou-concurrency-test-verification-research-2026-06-28.md) 결론 = loom 도입은 지금
// 저ROI(tokio::sync 사용자 검증엔 tokio 재컴파일 필요), 대신 **단위 수준으로 내려 가드 메서드를 직접
// 순서대로 호출**해 가드의 *논리 계약*(stale→미발행/거부, current→발행/허용)을 네트워크 타이밍 없이
// 결정론적으로 증명한다. ★범위 한계(정직성)★: 비교+변경의 *원자성*(동시 스레드에서 진짜 안 깨짐)은
// std Mutex 가 보장하며 이 단위 테스트가 증명하는 게 아니다 — 그건 loom 영역이다(아래 §loom). 실 소켓
// race 를 통한 통합 wiring 커버는 위쪽 single-shot 결정론 회귀 테스트
// (`concurrent_connect_settles_connected_no_flap` · `connected_then_close_reconnect_no_down_clobber`
// 등)가 계속 맡는다 — 이 단위 테스트는 그 가드 *판정점*(논리 계약) 자체를 race 없이 박제한다.

// ── (a) stale Down 은 current Connected 를 clobber 못 함 (옛 stress① 대체) ──────────────────────
// 더 새 세대(gen_b)가 Connected 를 발행한 뒤, 밀려난 옛 세대(gen_a)가 Down 을 발행하려 해도 거부되어
// watch 가 Connected 로 유지됨을 단언. publish_if_current 의 "세대 비교 + send" 가 분리되면(옛 구현)
// 이 stale Down 이 current Connected 를 덮어쓴다 → 그 회귀를 결정적으로 잡는다.
#[test]
fn guard_stale_down_cannot_clobber_current_connected() {
    let (lc, rx) = Lifecycle::new();
    // 옛 세대 진입(gen_a) → 이어서 더 새 세대 등장 모델(gen_b). gen_b > gen_a 라 gen_a 는 stale.
    let gen_a = lc.bump_and_capture(Some(ConnectionState::Connecting));
    let gen_b = lc.bump_and_capture(Some(ConnectionState::Connecting));
    assert!(gen_b > gen_a, "두 번째 bump 가 더 새 세대여야");
    assert_eq!(lc.current_generation(), gen_b, "current = 최신 세대");

    // current(gen_b) 는 Connected 발행 허용.
    assert!(
        lc.publish_if_current(gen_b, ConnectionState::Connected),
        "current 세대는 Connected 발행됨"
    );
    // stale(gen_a) 의 Down 발행은 거부 → clobber 없음.
    assert!(
        !lc.publish_if_current(gen_a, ConnectionState::Down),
        "stale 세대의 Down 은 거부되어야(clobber 차단)"
    );
    assert_eq!(
        *rx.borrow(),
        ConnectionState::Connected,
        "stale Down 이 거부됐으니 watch 는 Connected 유지"
    );
}

// ── (b) 동시 connect 는 최신 세대로 수렴 (옛 stress② 대체) ──────────────────────────────────────
// 두 connect 가 각자 세대를 bump 한 모델(g1 < g2). 의도: 동시 connect 면 밀려난(stale) 시도는 거부되고
// 최신 세대만 발행된다.
//
// ★단언 순서가 load-bearing(vacuity 회피)★: 먼저 current(g2)가 Connected 를 발행한 뒤, stale(g1)이
// **구별 가능한 값(Down)** 을 발행 시도해 ① 거부(false) ② watch 가 여전히 Connected(clobber 안 됨)임을
// 단언한다. stale 도 Connected 를 쏘게 두면(옛 구현) 발행값이 current 와 같아 watch 가 안 변하므로 "가드가
// false 만 반환하고 send 는 그대로 함" 같은 mutation 이 헛 단언을 통과한다 — Down 으로 갈라 그 누수를 watch
// 에 드러낸다(가드가 stale 을 send 하면 watch=Down 으로 떨어져 마지막 단언이 실패).
#[test]
fn guard_concurrent_connect_settles_to_newest_generation() {
    let (lc, rx) = Lifecycle::new();
    let g1 = lc.bump_and_capture(Some(ConnectionState::Connecting));
    let g2 = lc.bump_and_capture(Some(ConnectionState::Connecting));
    assert!(g2 > g1, "동시 connect 모델: 둘 다 bump 라 g2 > g1");

    // 최신 세대(g2) 만 current → Connected 발행 성공.
    assert!(
        lc.publish_if_current(g2, ConnectionState::Connected),
        "최신 세대(g2)는 Connected 발행 성공"
    );
    assert_eq!(
        *rx.borrow(),
        ConnectionState::Connected,
        "current 발행 반영"
    );

    // 밀려난 세대(g1) 가 구별 가능한 Down 을 발행 시도 → 거부(false). Connected 와 다른 값이라, 가드가
    // 새는 mutation 이면 watch 가 Down 으로 떨어져 아래 단언이 실패한다.
    assert!(
        !lc.publish_if_current(g1, ConnectionState::Down),
        "밀려난 세대(g1)의 발행은 거부"
    );
    assert_eq!(
        *rx.borrow(),
        ConnectionState::Connected,
        "stale 이 거부됐으니 watch 는 current 의 Connected 유지(stale Down 으로 clobber 안 됨)"
    );
}

// ── (c) store_cmd_if_current 가드 — stale 거부, current 허용 + 기존 sender 미덮음 ────────────────────
// 가드의 *목적* = 좀비 sender 차단(stale 저장이 current cmd_tx 를 덮으면 안 됨). 반환 bool 만 보면 그
// 목적을 증명 못 하므로, `lifecycle_close_clears_cmd_and_blocks_stale_revival` 과 같은 기법으로 저장된
// cmd_tx 상태를 관찰한다 — 여기선 cmd_tx_snapshot(#[cfg(test)] 접근자)으로 *어떤* sender 가 저장됐는지를
// `same_channel` 로 본다. 순서: current 먼저 저장(true) → stale 저장 시도(false) → 저장된 cmd_tx 가 여전히
// current 의 것(stale 로 안 덮임). (mpsc::channel(1) 더미 — 실제 송수신은 검증 대상 아님.)
#[test]
fn guard_store_cmd_rejects_stale_generation() {
    let (lc, _rx) = Lifecycle::new();
    let gen_a = lc.bump_and_capture(None);
    let gen_b = lc.bump_and_capture(None); // gen_a 를 stale 로 만듦
    assert!(gen_b > gen_a);

    // current(gen_b) 저장 성공 — 이게 살아남아야 할 sender.
    let (tx_cur, _rx_cur) = tokio::sync::mpsc::channel(1);
    assert!(
        lc.store_cmd_if_current(gen_b, tx_cur.clone()),
        "current 세대는 cmd_tx 저장 허용(true)"
    );
    let stored = lc.cmd_tx_snapshot().expect("current 저장 후 cmd_tx 존재");
    assert!(
        stored.same_channel(&tx_cur),
        "저장된 cmd_tx 는 current(gen_b)의 것"
    );

    // stale(gen_a) 저장 시도 → 거부(false). 그리고 저장된 sender 는 여전히 current 의 것(stale 미덮음).
    let (tx_stale, _rx_stale) = tokio::sync::mpsc::channel(1);
    assert!(
        !lc.store_cmd_if_current(gen_a, tx_stale.clone()),
        "stale 세대는 cmd_tx 저장 거부(false)"
    );
    let after = lc
        .cmd_tx_snapshot()
        .expect("거부 후에도 current cmd_tx 유지");
    assert!(
        after.same_channel(&tx_cur),
        "stale 저장이 거부됐으니 저장된 cmd_tx 는 여전히 current(gen_b)의 것"
    );
    assert!(
        !after.same_channel(&tx_stale),
        "stale sender 로 덮이지 않았다(좀비 sender 차단)"
    );
}

// ── (d) close 후 stale 부활 방지 ────────────────────────────────────────────────────────────────
// close() 가 세대를 bump 하고 Down 을 발행한 뒤, 옛 세대(gen_a)로 Connected 를 발행하려 해도 거부되어
// close 의 Down 이 stale 에 의해 Connected 로 되살아나지 않음을 단언. close 의 bump+Down 원자성 계약.
#[test]
fn guard_close_blocks_stale_revival() {
    let (lc, rx) = Lifecycle::new();
    let gen_a = lc.bump_and_capture(Some(ConnectionState::Connecting));
    let gen_before_close = lc.current_generation();

    lc.close(); // 세대 bump + cmd_tx=None + Down 발행(원자)
    assert!(
        lc.current_generation() > gen_before_close,
        "close 가 세대를 bump 해야(stale 무력화)"
    );
    assert_eq!(*rx.borrow(), ConnectionState::Down, "close 후 Down");

    // 옛 세대(gen_a)의 Connected 발행 = 부활 시도 → 거부.
    assert!(
        !lc.publish_if_current(gen_a, ConnectionState::Connected),
        "close 로 stale 된 옛 세대는 close 의 Down 을 Connected 로 되살릴 수 없다"
    );
    assert_eq!(
        *rx.borrow(),
        ConnectionState::Down,
        "stale 부활이 거부됐으니 watch 는 Down 유지"
    );
}
