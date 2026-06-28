//! 단일 연결 task(actor) 본체 + Auth/Hello 핸드셰이크 (S14 모듈① T2, ADR-0036).
//!
//! 데몬 WS 서버(`crates/engram-dashboard-daemon/src/ws.rs`)의 **대칭 클라이언트**다. 서버측 기대:
//!   1) WS 업그레이드 후 **1초 내 첫 frame 이 `AgentCommand::Auth`(Text JSON)** 여야 한다 — 아니면
//!      서버가 Error 후 close(ws.rs `handle_connection` step 2 / AUTH_TIMEOUT).
//!   2) 토큰·protocol_version 일치 시 서버가 `AgentEvent::Hello`(Text JSON) → `AgentListUpdated`
//!      를 push 한다(ws.rs step 4 `hello_event`). 불일치면 Error 후 close.
//! 그래서 이 task 는 소켓 open 직후 **가장 먼저 Auth 를 보내고**, 첫 Hello 를 internal 소비해
//! connected 로 전이한다(Hello 는 control 로 위로 안 올린다 — wsTransport 와 동일).
//!
//! ## ★동시성(load-bearing)★
//! - 이 task 가 `WebSocketStream` 을 **split 해 read/write 양쪽을 단독 소유**한다(Mutex 없음).
//!   외부(invoke)는 `cmd_rx` 로만 의도를 보내고, 실제 write 는 이 task 의 send arm 한 곳뿐 —
//!   데몬 ws.rs "연결당 단일 writer" 와 대칭. 이게 SplitSink 동시 write 불가를 구조로 회피한다.
//! - 핸드셰이크 결과는 `ready_tx`(oneshot) 1회로 호출자에게 보고하고, 이후 상태 전이는
//!   `state_tx`(watch) 로 broadcast 한다.

use std::sync::Arc;
use std::time::Duration;

use engram_dashboard_protocol::{AgentCommand, AgentEvent, DaemonInfo, PROTOCOL_VERSION};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use super::lifecycle::Lifecycle;
use super::ConnectionState;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// 핸드셰이크(소켓 open ~ Hello 수신) 상한. 서버측 AUTH_TIMEOUT(1s, ws.rs)보다 넉넉히 잡되
/// 정상 핸드셰이크는 loopback 에서 <1s 라 절대 안 닿는다. 이 상한이 없으면 서버가 소켓만 받고
/// 침묵할 때 wait_for_hello 가 영구 대기한다(Fix A). 운영 기본값이며, 테스트는 run_connection 의
/// `handshake_timeout` 파라미터로 짧은 값을 주입한다(const 가 테스트를 10초 기다리게 만들지 않도록).
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// 핸드셰이크 실패 사유. wsTransport 의 reject 문자열에 대응.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeError {
    /// 발견/spawn 단계 실패(connect 경로). discovery 에러 메시지를 그대로 싣는다(token 미포함).
    Discovery(String),
    /// ensure(attach-only)인데 살아있는 데몬이 없음(no-spawn 이라 못 띄움 — ADR-0021).
    /// wsTransport 의 "daemon down — daemon_start 로 명시 시작 필요" 대응.
    NoLiveDaemon,
    /// ws://host:port 접속 실패(데몬 죽음/거부).
    Connect(String),
    /// Auth frame 송신 실패.
    AuthSend(String),
    /// 데몬이 Hello 전에 Error 를 보냄(토큰/버전 불일치 등).
    AuthRejected(String),
    /// Hello 전에 소켓이 닫힘.
    ClosedBeforeHello,
    /// 핸드셰이크(소켓 open ~ Hello 수신)가 HANDSHAKE_TIMEOUT 을 넘김. 서버가 소켓만 받고 Hello/
    /// Error/Close 중 무엇도 안 보내면 wait_for_hello 가 무한 대기하므로(깨울 외부 경로 없음),
    /// 상한을 둬 확정적으로 실패로 빠진다. ★Fix A★ — 영구 hang 방지.
    Timeout,
    /// 연결 task 가 ready 신호 전에 사라짐(panic 등).
    TaskGone,
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandshakeError::Discovery(m) => write!(f, "daemon discovery 실패: {m}"),
            HandshakeError::NoLiveDaemon => write!(
                f,
                "daemon down — connect(명시 시작) 필요 (ADR-0021: ensure 는 respawn 안 함)"
            ),
            HandshakeError::Connect(m) => write!(f, "daemon websocket 접속 실패: {m}"),
            HandshakeError::AuthSend(m) => write!(f, "Auth 전송 실패: {m}"),
            HandshakeError::AuthRejected(m) => write!(f, "daemon auth failed: {m}"),
            HandshakeError::ClosedBeforeHello => {
                write!(f, "daemon websocket closed before handshake")
            }
            HandshakeError::Timeout => {
                write!(f, "daemon handshake timeout — Hello 가 시간 내 안 옴")
            }
            HandshakeError::TaskGone => write!(f, "연결 task 가 핸드셰이크 전 종료됨"),
        }
    }
}

impl std::error::Error for HandshakeError {}

/// 연결 task 로 보내는 명령(단일 task 소유 — invoke 는 이걸 보내고 task 가 처리).
///
/// ★T2 범위★: 채널 스켈레톤만 깐다. 실제 명령 variant(Spawn/Kill/WriteStdin/Resize/Subscribe)와
/// 그 reply(oneshot) 처리는 T6 가 채운다. 지금은 빈 enum 이 아니라 forward-compat 자리로 둔다.
#[derive(Debug)]
pub enum ConnectionCommand {
    // TODO(T6): SendCommand { cmd: AgentCommand, reply: oneshot::Sender<...> } 등 invoke 명령.
    //   현재는 variant 없음 — cmd_tx.send 호출처(T6)가 생기면 채운다. 채널/소유 구조만 T2 에서 검증.
    #[doc(hidden)]
    __Placeholder,
}

/// 연결 task 본체. 소켓을 열어 Auth/Hello 핸드셰이크를 마치고, 그 결과를 `ready_tx` 로 1회 보고한
/// 뒤 메인 루프(read/write/cmd)로 들어간다. 이 함수가 stream 을 split 해 단독 소유한다.
///
/// ## ★generation 가드(load-bearing, Fix B — 락으로 원자화)★
/// `my_gen` = 이 task 가 spawn 될 때 캡처한 세대값, `lifecycle` = 공유 lifecycle 락(DaemonClient 소유).
/// 동시 connect/ensure·close-in-flight 로 더 새 task 가 떠 세대가 올라가면, **밀려난(stale) task 는
/// 공유 상태(watch 전이 · cmd_tx 슬롯)를 건드리지 않고 자기 소켓만 닫고 조용히 종료**한다. 모든
/// 가드된 전이는 `lifecycle.publish_if_current(my_gen, state)` 한 곳을 통과한다 — 이 메서드가 "세대
/// 비교 + watch send" 를 같은 락 critical section 으로 묶어 원자화하므로, 비교 통과 후 send 전에 다른
/// 스레드가 세대를 바꿔 끼어들 수 없다(이전 `AtomicU64::load` → `send` 분리가 만든 TOCTOU 를 닫음).
/// 이게 wsTransport openGen 가드의 씨앗 — 현재(current) 연결 task 는 최대 1개라는 불변식을 코드로
/// 강제한다. ⚠️ 완전한 "동시 시도 abort/백오프"는 T4 — 여기선 짧은 순간 소켓 2개가 동시에 열릴 수
/// 있음(둘 다 connect_async 진행)을 허용하되, stale task 가 *공유 상태를 안 건드리고* 즉시 self-close
/// 하므로 관찰 가능한 오염(고아 Down clobber·좀비 채널·Connected 부활)은 없앤다.
///
/// ## ★ADR-0006 — 락 .await across 보유 금지★
/// `publish_if_current`/`store_cmd_if_current` 는 전부 동기(내부에서 await 안 함)다. 따라서 아래
/// `connect_async`·`sink.send`·`wait_for_hello`·`sink.close` 등 모든 await 는 lifecycle 락을 보유하지
/// 않은 채 일어난다(락은 publish_if_current 호출 안에서만 잡혔다 즉시 풀린다).
pub(crate) async fn run_connection(
    info: DaemonInfo,
    my_gen: u64,
    lifecycle: Arc<Lifecycle>,
    handshake_timeout: Duration,
    cmd_rx: mpsc::Receiver<ConnectionCommand>,
    ready_tx: oneshot::Sender<Result<(), HandshakeError>>,
) {
    // 1) ws://host:port 접속(host 는 항상 127.0.0.1 loopback — DaemonInfo).
    let url = format!("ws://{}:{}", info.host, info.port);
    // ★token 미노출★: url·generation 만 필드로(DaemonInfo.token 은 절대 로그에 싣지 않음 — 보안).
    tracing::debug!(%url, generation = my_gen, "데몬 WS 연결 시도");
    let ws = match connect_async(&url).await {
        Ok((ws, _resp)) => ws,
        Err(e) => {
            // ★token 미노출★: url 만 싣는다(DaemonInfo.token 은 절대 에러에 넣지 않음).
            tracing::warn!(%url, generation = my_gen, "데몬 WS 접속 실패: {e}");
            let _ = ready_tx.send(Err(HandshakeError::Connect(format!("{url}: {e}"))));
            // ★원자 가드★ stale 이면 공유 상태 미접촉(고아 Down clobber 방지) — 비교+send 가 한 락 안.
            lifecycle.publish_if_current(my_gen, ConnectionState::Down);
            return;
        }
    };

    // 2) split — 이 task 가 read(stream)/write(sink) 양쪽을 단독 소유한다(Mutex 없음).
    let (mut sink, mut stream) = ws.split();

    // 3) ★첫 frame = Auth(Text JSON)★: 데몬 ws.rs 가 1초 내 첫 frame 으로 이걸 기대한다(AUTH_TIMEOUT).
    //    ★Fix C★ protocol_version 은 **우리가 컴파일된 PROTOCOL_VERSION**(protocol crate)을 보낸다
    //    — DaemonInfo 가 준 값을 되쏘면(echo) 서버 버전 비교(ws.rs)가 항상 통과해 버전 게이트가
    //    무력화된다. 버전 불일치 시 서버가 거부하는 게 의도된 게이트(discovery/lib.rs build_auth_command
    //    과 동형). token 은 wire 로만(로그/에러 미노출).
    let auth = AgentCommand::Auth {
        token: info.token.clone(),
        protocol_version: PROTOCOL_VERSION,
    };
    let auth_text = match serde_json::to_string(&auth) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(generation = my_gen, "Auth 직렬화 실패: {e}");
            let _ = ready_tx.send(Err(HandshakeError::AuthSend(format!("직렬화: {e}"))));
            lifecycle.publish_if_current(my_gen, ConnectionState::Down);
            return;
        }
    };
    if let Err(e) = sink.send(Message::Text(auth_text.into())).await {
        // ★token 미노출★: 송신 실패 에러만(Auth frame 내용=token 은 절대 로그 금지 — 보안).
        tracing::warn!(generation = my_gen, "Auth frame 송신 실패: {e}");
        let _ = ready_tx.send(Err(HandshakeError::AuthSend(e.to_string())));
        lifecycle.publish_if_current(my_gen, ConnectionState::Down);
        return;
    }

    // 4) Hello 대기(=인증 성공). Hello 는 internal 소비 — control 로 위로 올리지 않는다(wsTransport
    //    와 동일). Hello 전 Error = Auth 실패 → reject. 소켓 닫힘 = ClosedBeforeHello.
    //    ★Fix A★ wait_for_hello 를 timeout 으로 감싼다 — 서버가 침묵하면 영구 hang 이므로 상한을 둔다.
    //    ★Hello 전 도착하는 다른 control(없어야 정상이나)·binary 는 핸드셰이크 단계에선 무시한다.★
    let handshake = match tokio::time::timeout(handshake_timeout, wait_for_hello(&mut stream)).await
    {
        Ok(result) => result,
        Err(_elapsed) => Err(HandshakeError::Timeout),
    };
    match handshake {
        Ok(()) => {
            // ★원자 가드(Fix B)★: Connected 전이를 publish_if_current 로 — "세대 비교 + watch send" 가
            //    한 락 안이라, 비교 통과 후 send 전에 close()/새 connect 가 끼어들 수 없다(TOCTOU 차단).
            //    stale(밀려남)이면 발행 false → 공유 상태 미접촉 후 self-close. ready_tx 는 send 없이
            //    drop → caller 의 ready_rx.await 가 RecvError → caller 가 stale 을 인지(start_connection
            //    의 Err(_) arm). current task 만 Connected 를 broadcast + ready Ok 보고.
            if !lifecycle.publish_if_current(my_gen, ConnectionState::Connected) {
                // ★generation 가드 발동★: Hello 까지 왔으나 더 새 connect/close 가 세대를 올려 이
                //    연결은 stale — 공유 상태를 건드리지 않고 소켓만 닫고 조용히 종료(고아 Connected 방지).
                tracing::debug!(
                    generation = my_gen,
                    "stale 연결 폐기 — Hello 수신했으나 세대가 밀림"
                );
                let _ = sink.close().await; // ★락 밖 await★
                return;
            }
            // Hello 수신 = 인증 성공 = 연결 수립. 정상 수명주기(info).
            tracing::info!(%url, generation = my_gen, "데몬 WS 연결 수립(Hello 수신, 인증 성공)");
            // ready_tx 수신자가 사라졌으면(호출자 drop) 그대로 정리 종료.
            if ready_tx.send(Ok(())).is_err() {
                tracing::debug!(
                    generation = my_gen,
                    "연결 수립 후 호출자 사라짐(ready 수신자 drop) — 정리 종료"
                );
                let _ = sink.close().await; // ★락 밖 await★
                lifecycle.publish_if_current(my_gen, ConnectionState::Down);
                return;
            }
        }
        Err(e) => {
            // 핸드셰이크 실패(타임아웃·Auth 거부·Hello 전 close 등). 안전 폴백(warn).
            tracing::warn!(%url, generation = my_gen, "데몬 WS 핸드셰이크 실패: {e}");
            let _ = ready_tx.send(Err(e));
            // ★원자 가드★ stale 이면 Down 미송신(current 의 Connected clobber 방지).
            lifecycle.publish_if_current(my_gen, ConnectionState::Down);
            let _ = sink.close().await; // ★락 밖 await★
            return;
        }
    }

    // 5) 메인 루프 — connected 이후. read(데몬 이벤트/frame) · write(cmd) 를 한 task 가 전담한다.
    //    ★T2 범위★: control/binary 의 실제 라우팅(T5)·명령 처리(T6)·재연결(T4)은 미구현. 여기선
    //    "단일 task 가 stream 을 단독 소유하며 살아있다 + 명시 close(cmd 채널 drop) 시 정리" 만.
    //    final Down 송신도 원자 가드(publish_if_current)를 통과한다(stale 종료가 current 를 clobber 못함).
    main_loop(sink, stream, cmd_rx, &lifecycle, my_gen).await;
}

/// 메인 read/write 루프(connected 이후). 단일 task 가 stream/sink 를 단독 소유한 채
/// `tokio::select!` 로 (a) 데몬 수신 (b) cmd 채널을 동시에 대기한다.
async fn main_loop(
    mut sink: futures_util::stream::SplitSink<Ws, Message>,
    mut stream: futures_util::stream::SplitStream<Ws>,
    mut cmd_rx: mpsc::Receiver<ConnectionCommand>,
    lifecycle: &Arc<Lifecycle>,
    my_gen: u64,
) {
    // 루프 종료 사유를 한 곳에서 로깅하려고 break 로 사유를 끌어올린다(핫패스 frame 수신 본문엔
    // 로그 미부착 — Text/Binary 청크는 per-frame 빈도라 trace 미사용 정책 유지). 종료=연결 1회뿐.
    let reason = loop {
        tokio::select! {
            // 데몬 → 클라 수신.
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(msg)) => {
                        // ★T2★: control JSON 디코드 형태만 확인하고 소비한다(라우팅/dedup 은 T3/T5).
                        match msg {
                            Message::Text(text) => {
                                // 데몬 control 이벤트. T3 가 순수 결정 함수를 깔았다(protocol_state) — T5/T6 이
                                // 여기서 그 함수를 호출해 배선한다.
                                // TODO(T5/T6): AgentEvent 파싱 → variant 분기:
                                //   Ack/Spawned/Created/Error/AgentList/ProfileList/Snapshot →
                                //     protocol_state::take_pending(&mut pending, request_id) → oneshot resolve/reject.
                                //   SubscribeAck → protocol_state::apply_subscribe_ack(&mut sub, current_epoch).
                                //   StatusChanged/RestoreResult/AgentListUpdated/ProfileListUpdated → app.emit broadcast.
                                let _ = serde_json::from_str::<AgentEvent>(&text);
                            }
                            Message::Binary(_bytes) => {
                                // 출력 binary frame(codec). T3 가 seq dedup·epoch 가드(decide_output)를 깔았다.
                                // TODO(T5): decode_frame → protocol_state::decide_output(&mut sub, epoch, seq)
                                //   → Deliver 면 OutputRouter(arc-swap) 로 라우팅, Drop* 이면 무시.
                            }
                            // Ping/Pong 은 tungstenite 가 자동 응답(내부). Close 면 종료.
                            Message::Close(_) => break "데몬이 Close frame 송신",
                            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                        }
                    }
                    // 데몬이 연결을 닫음/오류 → 메인 루프 종료. 재연결(T4) 은 미구현이라 Down 으로.
                    Some(Err(_)) | None => break "데몬 스트림 종료/오류",
                }
            }
            // invoke → 연결 task 명령. cmd_rx 가 None(모든 sender drop = 명시 close) 이면 종료.
            cmd = cmd_rx.recv() => {
                match cmd {
                    // TODO(T6): ConnectionCommand variant 처리(cmd 를 wire 로 인코딩해 sink.send).
                    Some(_cmd) => { /* T6 에서 처리 */ }
                    None => break "명시 close(cmd_tx drop)", // DaemonClient.close() 가 cmd_tx 를 drop.
                }
            }
        }
    };
    // 연결 task 종료 = 연결 수명 끝. 정상 수명주기(info) — reason 으로 데몬 close/명시 close 등 구분.
    tracing::info!(generation = my_gen, reason, "데몬 WS 연결 task 종료");
    // 정리: 소켓 닫고 Down 전이(재연결 전이는 T4 가 여기서 분기). ★락 밖 await★.
    let _ = sink.close().await;
    // ★원자 가드(Fix B)★: stale task(close()/새 connect 가 세대를 올림)의 종료가 current 연결의
    //    Connected 를 Down 으로 clobber 하지 않도록, publish_if_current 로 "세대 비교 + Down send" 를
    //    한 락 안에 묶는다 — 비교 후 send 전에 새 연결이 끼어 Connected 를 올려도 stale 인 이 task 는
    //    이미 false 로 빠진다(이전 load→send 분리의 TOCTOU 를 닫음). current 일 때만 Down broadcast.
    if !lifecycle.publish_if_current(my_gen, ConnectionState::Down) {
        // stale 종료 — 더 새 연결이 이미 떠 current 를 잡았으므로 Down 을 삼켰다(clobber 방지 발동).
        tracing::debug!(
            generation = my_gen,
            "stale task 종료 — Down 미발행(더 새 연결이 current)"
        );
    }
}

/// Hello 가 올 때까지 stream 을 읽는다(internal 소비). Hello=Ok, Error=AuthRejected, 닫힘=ClosedBeforeHello.
///
/// ★Hello 내부 소비(wsTransport 와 동형)★: Hello 는 핸드셰이크 신호라 control 로 위로 올리지
/// 않는다 — 여기서 먹고 connected 로만 전이한다. Hello 전에 오는 다른 control/binary 는 정상
/// 흐름엔 없지만(데몬은 Hello 를 가장 먼저 push), 방어적으로 무시하고 Hello 만 기다린다.
async fn wait_for_hello(
    stream: &mut futures_util::stream::SplitStream<Ws>,
) -> Result<(), HandshakeError> {
    while let Some(item) = stream.next().await {
        let msg = match item {
            Ok(m) => m,
            Err(e) => return Err(HandshakeError::Connect(e.to_string())),
        };
        match msg {
            Message::Text(text) => {
                // 데몬 control event 파싱. Hello=성공, Error=인증 실패.
                match serde_json::from_str::<AgentEvent>(&text) {
                    Ok(AgentEvent::Hello { .. }) => return Ok(()),
                    Ok(AgentEvent::Error { message, .. }) => {
                        return Err(HandshakeError::AuthRejected(message))
                    }
                    // 그 외 control(정상 흐름엔 Hello 가 먼저라 없음) — 무시하고 Hello 계속 대기.
                    Ok(_) => {}
                    // 파싱 실패도 무시(부분 프레임 등) — 데몬은 valid JSON 만 보낸다.
                    Err(_) => {}
                }
            }
            // Hello 전 binary 는 정상엔 없음 — 무시.
            Message::Binary(_) => {}
            Message::Close(_) => return Err(HandshakeError::ClosedBeforeHello),
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }
    // 스트림 종료(None) = 닫힘.
    Err(HandshakeError::ClosedBeforeHello)
}
