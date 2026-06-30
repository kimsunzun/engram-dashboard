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

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use engram_dashboard_protocol::{
    decode_frame, AgentCommand, AgentEvent, AgentId, DaemonInfo, PROTOCOL_VERSION,
};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use tauri::Emitter;

use super::lifecycle::{Lifecycle, ReconnectVerdict};
use super::protocol_state::{self, PendingMap, SubState};
use super::{ConnectionState, DaemonDiscovery};
use crate::output_channel::{self, AgentBufferStore, WindowChannelRegistry};
use crate::output_router::OutputRouter;

/// SendCommand 의 reply 채널 타입(T6a). `Ok(event)` = 데몬이 매칭 reply(Ack/Spawned/Created/
/// SubscribeAck/AgentList/…)를 보냄, `Err(msg)` = 데몬 Error 또는 연결 끊김(drain). 호출자
/// (`DaemonClient::send_command`)가 이 oneshot 의 수신단을 await 한다.
pub type CommandReply = oneshot::Sender<Result<AgentEvent, String>>;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// 핸드셰이크(소켓 open ~ Hello 수신) 상한. 서버측 AUTH_TIMEOUT(1s, ws.rs)보다 넉넉히 잡되
/// 정상 핸드셰이크는 loopback 에서 <1s 라 절대 안 닿는다. 이 상한이 없으면 서버가 소켓만 받고
/// 침묵할 때 wait_for_hello 가 영구 대기한다(Fix A). 운영 기본값이며, 테스트는 run_connection 의
/// `handshake_timeout` 파라미터로 짧은 값을 주입한다(const 가 테스트를 10초 기다리게 만들지 않도록).
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// ★재연결 백오프(T4 — wsTransport `scheduleReconnect` 500ms→10s MAX5 이식)★.
/// attach-only 재연결 최대 시도 횟수. 데몬이 죽으면(graceful stop·kill·크래시) 캐시/read_live 주소로의
/// 재연결이 전부 실패한다 — 무한 reconnecting 으로 매달리지 않고 이 횟수만큼 시도한 뒤 Down 으로
/// 정착시킨다(꺼진 채 유지). 일시적 끊김은 이 안에서 회복된다. 복구는 명시 connect 로만.
pub const MAX_RECONNECT_ATTEMPTS: u32 = 5;

/// 백오프 기준 지연(500ms). attempt 마다 2^attempt 배(500ms→1s→2s→4s→8s), 상한 BACKOFF_CAP.
const BACKOFF_BASE: Duration = Duration::from_millis(500);

/// 백오프 상한(10s). 지수 증가가 이 값을 넘지 않게 클램프(wsTransport `Math.min(..., 10000)`).
const BACKOFF_CAP: Duration = Duration::from_secs(10);

/// attempt 번째 재연결 시도의 백오프 지연. attempt=0 → 500ms, 1 → 1s, …, 상한 10s.
/// ★shift 안전★: 2^attempt 가 u64 를 넘기지 않게 checked_shl 로 클램프한 뒤 곱한다(MAX5 라 실제론 최대 8s).
pub(crate) fn backoff_delay(attempt: u32) -> Duration {
    // attempt 가 커도 곱셈 오버플로 없이 BACKOFF_CAP 으로 수렴하게: 먼저 shift 후 cap 으로 min.
    let factor = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
    let millis = (BACKOFF_BASE.as_millis() as u64).saturating_mul(factor);
    Duration::from_millis(millis).min(BACKOFF_CAP)
}

/// 메인 루프 종료 사유 — 재연결 대상(데몬 끊김)인지 명시 종료(close)인지 가른다(T4).
///
/// ★load-bearing★: 이 구분이 재연결 진입 여부를 결정한다. `Disconnected` 만 재연결 루프로 가고,
/// `Closed`(cmd_rx EOF = close()/stale 미저장)는 재연결하지 않는다 — 사용자가 닫았거나(respawn 금지)
/// 더 새 연결이 이미 떴기(stale 미저장 = 좀비 방지) 때문이다. 단, `Disconnected` 라도 진입 직후
/// reconnect_guard(generation + closed_by_user)를 한 번 더 보고 Stop 이면 재연결 안 한다(끊김과 close 가
/// 동시에 들어온 경우).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopExit {
    /// 데몬 stream 이 끊김/오류/Close frame → 재연결 대상. (비의도 끊김)
    Disconnected,
    /// cmd_rx 가 EOF(모든 sender drop) = 명시 close() 또는 stale 미저장 → 재연결 안 함(종료).
    Closed,
}

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
/// ★평면 구분★: `SendCommand` = 요청/응답(reply 기대) — request_id ↔ oneshot 상관을 main_loop 가
/// `PendingMap` 으로 한다. `Subscribe`/`Unsubscribe`/`Fire` = **fire-and-forget**(reply 없음).
/// Subscribe/Unsubscribe 는 wire 인코딩 시 `SubState`(epoch/after_seq) 조회가 필요해 전용 variant 로
/// 두고, Resize 처럼 SubState 무관한 reply 없는 명령은 그냥 `Fire` 로 wire 송신한다.
#[derive(Debug)]
pub enum ConnectionCommand {
    /// 요청/응답 명령(T6a). `cmd` 의 request_id 로 reply 를 매칭한다. main_loop 가:
    ///   1) reply 를 PendingMap[request_id] 에 넣고 → 2) cmd 를 JSON 으로 sink.send.
    /// 데몬 reply(request_id echo) 도착 시 take_pending → oneshot 으로 resolve. send/끊김 실패 시 Err.
    SendCommand {
        cmd: AgentCommand,
        reply: CommandReply,
    },
    /// 출력 구독(T6b). ★epoch/after_seq 필드 없음(G1)★ — layout 은 "이 agent 구독해라"만 안다.
    /// main_loop 가 SubState(연결 task 소유)에서 `resubscribe_params` 로 epoch/after_seq 를 채워
    /// wire `AgentCommand::Subscribe` 를 만든다(신규=FromOldest / 재구독=tail-only, 한 경로로 통일).
    Subscribe {
        agent_id: engram_dashboard_protocol::AgentId,
    },
    /// 출력 구독 해제(T6b). main_loop 가 `AgentCommand::Unsubscribe` 를 wire 로 송신한다.
    /// ★subs 에서 SubState 는 제거하지 않는다★(F-B: 재구독=Resume tail 정합, 유실0 — spike §8).
    Unsubscribe {
        agent_id: engram_dashboard_protocol::AgentId,
    },
    /// reply 없는 fire-and-forget 명령(Resize 등). main_loop 가 그냥 JSON 으로 wire 송신한다.
    /// (Subscribe/Unsubscribe 는 SubState 조회 로직이 달라 전용 variant, Resize 는 일반 fire 라 Fire.)
    Fire { cmd: AgentCommand },
    /// ★mount-즉시-replay(축 B — BLOCK-2 actor 직렬화)★. 새로 생긴 `(window_label, agent)` slot 집합에
    /// 그 시점 공유 버퍼를 즉시 replay 한다. ★actor 경유인 이유(opus F2)★: mount replay 의 buffer
    /// 락+snapshot 수집을 invoke 스레드가 직접 하면 `on_frame`(actor tokio 스레드)과 *다른 스레드* 라
    /// replay→live 순서가 역전될 수 있다(데몬 C4 클라측 위반). 이 variant 로 actor 에 보내 on_frame 과
    /// **같은 스레드(main_loop select! arm)** 에서 직렬 처리한다 — buffer 락 안 snapshot 수집, 락 밖 send.
    ///
    /// ## ★fresh 플래그 = 배정 트리거 ↔ 등록 트리거 분리(FIX-2 — mount 1회 replay 멱등)★
    /// 같은 slot 에 두 트리거가 닿는다 — layout 배정 델타(`slots_to_replay`)와 webview 등록(`subscribe_output`
    /// → `slots_for_window`). 옛 구현은 둘 다 무조건 fresh 리셋(`resubscribe_slot`)이라, 정상 mount(배정→
    /// 등록)에서 같은 버퍼가 **연속 2회 전체 flush** 됐다(무중복이 프론트 `lastDeliveredSeq` dedup 에만 의존 —
    /// ADR-0037 "Rust 단독 진실원"과 어긋남). 그래서 트리거별로 역할을 가른다:
    ///   - `fresh=false`(**배정 트리거**): `store.subscribe` — **cursor 없을 때만** 신설+replay(있으면 불가침).
    ///     정상 mount 의 첫 replay 1회. 이미 보던 slot(1→2 둘째 창은 cursor 없어 신설, 기존 창은 불가침)도
    ///     중복 0.
    ///   - `fresh=true`(**등록 트리거**): `store.resubscribe_slot` — cursor 가 있어도 None 리셋 후 전체 replay.
    ///     webview reload(같은 label 로 Channel *교체* → 새 xterm 빈 화면)를 채운다(stale 이어보기 금지, 근원2).
    /// 결과 불변식: **정상 mount = 전체 replay 1회**(배정 신설 1회, 등록은 cursor 있으면 fresh 지만 같은 mount
    /// 면 배정이 먼저 신설했으니 등록 fresh 는 reload 가 아니면 안 옴 — 등록은 *Channel 등록 이벤트*에서만).
    /// reload = fresh 1회. 재연결 = frontend dedup 2차 안전망(같은 ProtocolClient).
    ReplaySlots {
        slots: Vec<(
            crate::output_router::WindowLabel,
            engram_dashboard_protocol::AgentId,
        )>,
        /// `true` = 등록 트리거(Channel 등록=viewer 재시작 → cursor fresh 리셋+전체 replay, reload 대응).
        /// `false` = 배정 트리거(layout 델타 → cursor 없을 때만 신설+replay, 불가침 — 정상 mount 1회).
        fresh: bool,
    },
    /// ★slot cursor 정리(축 B — FIX-3)★. 사라진 `(window_label, agent)` slot 의 cursor 를 제거한다
    /// (마지막 cursor 면 content drop — 누수 0). frame 도착과 독립(terminal+부분 닫힘도 정상 폐기).
    /// actor 경유로 on_frame 의 sync_viewers 와 같은 스레드에서 처리(cursor 이중관리·race 차단).
    DropSlots {
        slots: Vec<(
            crate::output_router::WindowLabel,
            engram_dashboard_protocol::AgentId,
        )>,
    },
    /// ★connect handoff race resync(FIX-2 — Codex mod.rs:464)★. main_loop 진입 resubscribe 는
    /// `router.current_agents()` 를 *그 시점 스냅샷*으로 뜨는데, 그 직후 cmd_tx 가 lifecycle 에 저장되기
    /// *전*에 들어온 layout invoke 는 `try_enqueue` 가 cmd_tx=None 이라 유실되고, 그 변화가 스냅샷에도
    /// 안 잡혔으면 그 agent 는 데몬에 영영 미구독된다(handoff 창). `store_cmd_if_current` 직후 caller 가
    /// 이 명령을 보내 main_loop 가 **현재 router 집합으로 재구독 + 고아 sweep 을 한 번 더** 돌게 한다 —
    /// 이건 cmd_tx 저장 *후* 에만 enqueue 되므로(저장돼야 current_cmd_tx 가 Some), 그 사이 유실된 변화를
    /// 빠짐없이 따라잡는다(저장 후 변화는 정상 enqueue, 저장 전·스냅샷 후 변화는 이 resync 가 흡수).
    Resync,
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
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_connection(
    info: DaemonInfo,
    my_gen: u64,
    lifecycle: Arc<Lifecycle>,
    discovery: Arc<dyn DaemonDiscovery>,
    rt: Handle,
    handshake_timeout: Duration,
    cmd_rx: mpsc::Receiver<ConnectionCommand>,
    ready_tx: oneshot::Sender<Result<(), HandshakeError>>,
    // ★T6b 출력 평면 주입(G3)★: router=agent_id→[window_label] 라우팅(app-level 공유), registry=
    //   window_label→Channel. 둘 다 Arc 라 재연결 task 수명을 넘어 산다 — main_loop 가 Binary frame 을
    //   디코드해 router.targets 로 라우팅하고 registry 의 각 창 Channel 로 fan-out 한다.
    router: Arc<OutputRouter>,
    registry: WindowChannelRegistry,
    // ★ADR-0040 2단계 공유 버퍼 store★: on_frame(append+cursor read snapshot)·resubscribe(after_seq=
    //   버퍼 최신). registry(Channel)와 분리된 락이라, on_frame 은 store 락에서 snapshot 만 뜨고 락을
    //   푼 뒤 registry 락으로 send(TRD §1 락 규율 — buffer 락 ⊃ registry 락 중첩 0).
    buffer_store: AgentBufferStore,
    // ★T7c: 데몬 broadcast 이벤트를 프론트로 내보내는 AppHandle(emit 경로).
    //   Text arm 의 broadcast(request_id 없는 AgentListUpdated/StatusChanged/…)를 app.emit 로 전 webview 에 push.
    app: tauri::AppHandle,
) {
    // 1) 첫 핸드셰이크 — 결과를 ready_tx 로 caller(connect/ensure)에 1회 보고한다.
    let connected = handshake(&info, my_gen, handshake_timeout).await;
    let (sink, stream) = match connected {
        Ok(conn) => {
            // 핸드셰이크 성공이라도 stale 일 수 있다 — publish_if_current 로 Connected 발행 시도.
            // current 면 ready Ok + main_loop, stale 이면 소켓 닫고 종료(ready 는 drop → caller TaskGone).
            if !lifecycle.publish_if_current(my_gen, ConnectionState::Connected) {
                tracing::debug!(
                    generation = my_gen,
                    "stale 연결 폐기 — Hello 수신했으나 세대가 밀림"
                );
                let _ = conn.sink_close().await; // ★락 밖 await★
                return;
            }
            tracing::info!(
                generation = my_gen,
                "데몬 WS 연결 수립(Hello 수신, 인증 성공)"
            );
            // ★T7c★: 첫 connected 전이를 프론트에 push.
            let _ = app.emit("daemon-connection-state", "connected");
            if ready_tx.send(Ok(())).is_err() {
                // 호출자(connect await)가 사라짐 → 정리 종료.
                tracing::debug!(
                    generation = my_gen,
                    "연결 수립 후 호출자 사라짐(ready 수신자 drop) — 정리 종료"
                );
                let _ = conn.sink_close().await;
                lifecycle.publish_if_current(my_gen, ConnectionState::Down);
                return;
            }
            conn.into_split()
        }
        Err(e) => {
            // 핸드셰이크 실패(접속/Auth/타임아웃/Hello 전 close 등). caller 에 실패 보고 + Down 가드.
            tracing::warn!(generation = my_gen, "데몬 WS 핸드셰이크 실패: {e}");
            let _ = ready_tx.send(Err(e));
            // ★원자 가드★ stale 이면 Down 미송신(current 의 Connected clobber 방지).
            lifecycle.publish_if_current(my_gen, ConnectionState::Down);
            return;
        }
    };

    // 2) connected 이후: main_loop + (비의도 끊김 시) 재연결 루프. 명시 close/stale 종료면 재연결 안 함.
    //    ★재연결 취소 receiver 를 여기서 구독★: 이후 재연결 루프의 모든 await 를 이 receiver 와 select! 로
    //    경쟁시켜, close()/승계 connect 가 in-flight 재연결을 즉시 끊는다(소켓 open 전 탈출).
    let cancel_rx = lifecycle.cancel_subscribe();
    connected_lifetime(
        sink,
        stream,
        cmd_rx,
        info,
        my_gen,
        lifecycle,
        discovery,
        rt,
        handshake_timeout,
        cancel_rx,
        router,
        registry,
        buffer_store,
        app,
    )
    .await;
}

/// 한 소켓의 핸드셰이크 산출물 — split 된 sink/stream 을 들고, 정리(sink_close)·소유 이전(into_split)을
/// 제공한다. 첫 연결과 재연결이 같은 핸드셰이크 경로를 공유하게 묶는다.
struct Handshaked {
    sink: futures_util::stream::SplitSink<Ws, Message>,
    stream: futures_util::stream::SplitStream<Ws>,
}

impl Handshaked {
    async fn sink_close(mut self) -> Result<(), tokio_tungstenite::tungstenite::Error> {
        self.sink.close().await
    }
    fn into_split(
        self,
    ) -> (
        futures_util::stream::SplitSink<Ws, Message>,
        futures_util::stream::SplitStream<Ws>,
    ) {
        (self.sink, self.stream)
    }
}

/// 1회 소켓 열기 + Auth 송신 + Hello 대기(=인증 성공). 성공 시 split 된 소켓을 돌려준다. 공유 상태
/// 전이(Connected/Down)는 **호출자가** 가드(publish_if_current)와 함께 결정한다 — 첫 연결은 ready 보고가
/// 딸리고 재연결은 안 딸려, 그 분기를 호출자에 두는 게 깔끔하다(이 함수는 순수 소켓 핸드셰이크만 —
/// lifecycle 미접촉).
async fn handshake(
    info: &DaemonInfo,
    my_gen: u64,
    handshake_timeout: Duration,
) -> Result<Handshaked, HandshakeError> {
    // 1) ws://host:port 접속(host 는 항상 127.0.0.1 loopback — DaemonInfo).
    let url = format!("ws://{}:{}", info.host, info.port);
    // ★token 미노출★: url·generation 만 필드로(DaemonInfo.token 은 절대 로그에 싣지 않음 — 보안).
    tracing::debug!(%url, generation = my_gen, "데몬 WS 연결 시도");
    // ★connect 타임아웃(load-bearing — T4 재연결 진행성)★: connect_async 를 handshake_timeout 으로
    //   감싼다. 죽은 데몬 port 로의 TCP connect 는 OS 타임아웃(수십초~분)까지 hang 할 수 있어, 감싸지
    //   않으면 재연결 루프가 첫 시도에서 멈춰 백오프 소진(→Down)에 영원히 못 닿는다(테스트로 적출). Hello
    //   대기 timeout 과 같은 상한을 쓴다(운영 10s, 테스트 주입). loopback 정상 연결은 <1s 라 안 닿는다.
    let ws = match tokio::time::timeout(handshake_timeout, connect_async(&url)).await {
        Ok(Ok((ws, _resp))) => ws,
        Ok(Err(e)) => {
            // ★token 미노출★: url 만 싣는다(DaemonInfo.token 은 절대 에러에 넣지 않음).
            tracing::warn!(%url, generation = my_gen, "데몬 WS 접속 실패: {e}");
            return Err(HandshakeError::Connect(format!("{url}: {e}")));
        }
        Err(_elapsed) => {
            tracing::warn!(%url, generation = my_gen, "데몬 WS 접속 타임아웃");
            return Err(HandshakeError::Connect(format!("{url}: connect timeout")));
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
            let _ = sink.close().await; // ★락 밖 await★
            return Err(HandshakeError::AuthSend(format!("직렬화: {e}")));
        }
    };
    if let Err(e) = sink.send(Message::Text(auth_text.into())).await {
        // ★token 미노출★: 송신 실패 에러만(Auth frame 내용=token 은 절대 로그 금지 — 보안).
        tracing::warn!(generation = my_gen, "Auth frame 송신 실패: {e}");
        let _ = sink.close().await;
        return Err(HandshakeError::AuthSend(e.to_string()));
    }

    // 4) Hello 대기(=인증 성공). Hello 는 internal 소비 — control 로 위로 올리지 않는다(wsTransport
    //    와 동일). Hello 전 Error = Auth 실패 → reject. 소켓 닫힘 = ClosedBeforeHello.
    //    ★Fix A★ wait_for_hello 를 timeout 으로 감싼다 — 서버가 침묵하면 영구 hang 이므로 상한을 둔다.
    //    ★Hello 전 도착하는 다른 control(없어야 정상이나)·binary 는 핸드셰이크 단계에선 무시한다.★
    let result = match tokio::time::timeout(handshake_timeout, wait_for_hello(&mut stream)).await {
        Ok(result) => result,
        Err(_elapsed) => Err(HandshakeError::Timeout),
    };
    match result {
        // Hello 수신 = 인증 성공. split 된 소켓을 호출자에 넘긴다(상태 전이·ready 보고는 호출자 몫).
        Ok(()) => Ok(Handshaked { sink, stream }),
        Err(e) => {
            // 핸드셰이크 실패(타임아웃·Auth 거부·Hello 전 close 등). 소켓만 닫고 에러 반환.
            tracing::warn!(%url, generation = my_gen, "데몬 WS 핸드셰이크 실패: {e}");
            let _ = sink.close().await; // ★락 밖 await★
            Err(e)
        }
    }
}

/// ★취소 가능 핸드셰이크 산출(T4 — in-flight 취소)★. 재연결 루프가 쓰는 핸드셰이크 결과 3분기.
/// `Ok`=성공, `Err`=실패(데몬 죽음/거부/타임아웃), `Cancelled`=close()/승계가 cancel 을 켜서 중단됨
/// (소켓을 안 열었거나, connect_async 가 이미 연 소켓을 즉시 닫음 — 어느 쪽이든 데몬에 Auth 안 보냄).
enum HandshakeOutcome {
    Ok(Handshaked),
    Err(HandshakeError),
    /// 취소로 중단 — 호출자가 reconnect_guard 로 재확인 후 탈출/재시도. **소켓은 이미 정리됨**(연 적이
    /// 없거나 self-close 완료)이라 호출자가 추가로 닫을 필요 없다.
    Cancelled,
}

/// 재연결 전용 취소 가능 핸드셰이크. `handshake` 와 같은 단계(connect→Auth→Hello)를 밟되, **모든 await
/// 를 `cancel_rx.changed()` 와 `select!` 로 경쟁**시킨다(작업 지시: connect_async·핸드셰이크 각 await 를
/// 취소와 select). close()/승계 connect 가 cancel 을 켜면:
///   - connect_async 단계 취소 → **소켓을 아예 안 연다**(데몬 무접촉).
///   - Auth send / wait_for_hello 단계 취소 → 이미 연 소켓을 **즉시 self-close** 하고 Cancelled 반환
///     (Auth 가 나갔을 수는 있으나 stale 소켓이 *살아남아* 계속 점유/통신하는 창은 닫는다).
/// 이게 "close 후 stale task 가 소켓 open + Auth 를 보낸다"는 Codex 적출 결함의 1차 방어선이다.
/// generation 가드(publish_if_current)는 상태 발행을 막는 2차 방어선으로 남는다.
///
/// ★cancel-safe★: 모든 arm 의 `cancel_rx.changed()` 와 IO future 는 cancel-safe 라, 한 arm 이 이기면
/// 진 arm 은 부작용 없이 폐기된다(select! 표준). 따라서 부분 진행 상태가 새지 않는다.
async fn handshake_cancellable(
    info: &DaemonInfo,
    my_gen: u64,
    handshake_timeout: Duration,
    cancel_rx: &mut tokio::sync::watch::Receiver<u64>,
) -> HandshakeOutcome {
    let url = format!("ws://{}:{}", info.host, info.port);
    tracing::debug!(%url, generation = my_gen, "데몬 WS 재연결 시도(취소 가능)");

    // 1) connect_async 를 취소·timeout 과 동시에 경쟁. ★취소가 이기면 connect future 를 drop 한다★ —
    //    진행 중이던 WS 업그레이드가 중단돼 *우리가 쥔* 소켓 핸들은 생기지 않고(split→Auth 단계 미도달),
    //    이미 부분적으로 열린 TCP/스트림은 future drop 으로 함께 닫힌다.
    //    ★정직 표기(nit — 과대표기 금지)★: 그 시점에 OS 레벨 TCP SYN/connect 는 *이미 데몬에 닿았을 수 있다*
    //    (커널 backlog 에 들어간 연결은 우리가 future 를 drop 해도 되돌릴 수 없다). 그래서 이 취소의 계약은
    //    "재연결 소켓에서 stale **Auth**(token) 0 · 살아남아 통신을 *유지*하는 stale 소켓 0" 이지 "TCP SYN 0"
    //    이 아니다. connect_async 단계에서 취소되면 split 전이라 Auth 를 절대 안 보내고, 부분 소켓은 drop 으로
    //    닫혀 살아남지 않는다(서버가 곧 EOF 를 본다). 이게 보안상 핵심(token 미송신)을 지키는 지점이다.
    let ws = tokio::select! {
        biased; // 취소를 먼저 본다 — close 후 굳이 소켓을 여는 일이 없게(공정성보다 취소 우선).
        _ = cancel_rx.changed() => return HandshakeOutcome::Cancelled,
        connected = tokio::time::timeout(handshake_timeout, connect_async(&url)) => match connected {
            Ok(Ok((ws, _resp))) => ws,
            Ok(Err(e)) => {
                tracing::warn!(%url, generation = my_gen, "데몬 WS 재연결 접속 실패: {e}");
                return HandshakeOutcome::Err(HandshakeError::Connect(format!("{url}: {e}")));
            }
            Err(_elapsed) => {
                tracing::warn!(%url, generation = my_gen, "데몬 WS 재연결 접속 타임아웃");
                return HandshakeOutcome::Err(HandshakeError::Connect(format!(
                    "{url}: connect timeout"
                )));
            }
        },
    };

    // 2) split — 이 task 가 read/write 양쪽 단독 소유.
    let (mut sink, mut stream) = ws.split();

    // 3) Auth 송신 — 소켓이 이미 열렸으므로 여기부터 취소되면 self-close 로 정리한다(stale 소켓 점유 차단).
    let auth = AgentCommand::Auth {
        token: info.token.clone(),
        protocol_version: PROTOCOL_VERSION,
    };
    let auth_text = match serde_json::to_string(&auth) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(generation = my_gen, "Auth 직렬화 실패: {e}");
            let _ = sink.close().await;
            return HandshakeOutcome::Err(HandshakeError::AuthSend(format!("직렬화: {e}")));
        }
    };
    // ★취소 경쟁★: Auth send await 도중 close/승계가 끼면 즉시 깨어 소켓을 닫는다(stale 소켓이 살아
    //   서버와 계속 통신하는 창을 닫는다). send 가 이기면 Auth 가 나간 것이고, 그 직후 wait_for_hello 에서
    //   다시 취소를 경쟁한다.
    let send_res = tokio::select! {
        biased;
        _ = cancel_rx.changed() => {
            let _ = sink.close().await; // ★연 소켓 즉시 정리★
            return HandshakeOutcome::Cancelled;
        }
        r = sink.send(Message::Text(auth_text.into())) => r,
    };
    if let Err(e) = send_res {
        tracing::warn!(generation = my_gen, "Auth frame 송신 실패: {e}");
        let _ = sink.close().await;
        return HandshakeOutcome::Err(HandshakeError::AuthSend(e.to_string()));
    }

    // 4) Hello 대기 — timeout + 취소를 함께 경쟁. 취소면 소켓 닫고 Cancelled.
    let hello = tokio::select! {
        biased;
        _ = cancel_rx.changed() => {
            let _ = sink.close().await; // ★연 소켓 즉시 정리★
            return HandshakeOutcome::Cancelled;
        }
        result = tokio::time::timeout(handshake_timeout, wait_for_hello(&mut stream)) => match result {
            Ok(result) => result,
            Err(_elapsed) => Err(HandshakeError::Timeout),
        },
    };
    match hello {
        Ok(()) => HandshakeOutcome::Ok(Handshaked { sink, stream }),
        Err(e) => {
            tracing::warn!(%url, generation = my_gen, "데몬 WS 재연결 핸드셰이크 실패: {e}");
            let _ = sink.close().await;
            HandshakeOutcome::Err(e)
        }
    }
}

/// connected 이후 수명주기 — main_loop 를 돌고, **비의도 끊김**이면 재연결 루프로, **명시 close/stale**
/// 이면 종료한다(T4). 단일 task 가 전체를 들고 돌아 generation 가드가 task lifetime 과 한 몸이 된다.
///
/// ## ★재연결 generation/closedByUser 가드(load-bearing — T4 안전 게이트)★
/// 재연결 루프의 매 백오프·재시도 전에 `lifecycle.reconnect_guard(my_gen)` 로 "내가 current + 사용자
/// 안 닫음"을 **원자로** 확인한다. close()(세대 bump + closed_by_user)나 새 connect(세대 bump)가 끼면
/// Stop 을 받아 즉시 종료한다 — 이게 wsTransport `closedByUser` + `openGen` 좀비 가드의 task-lifetime
/// 판이다. ★왜 충분한가★: TS 의 좀비 race 는 "await yield 중 새 소켓 생성 → this.ws hijack" 이었다.
/// Rust 단일 task 모델엔 *공유 가변 소켓 핸들이 없다*(소켓은 이 task 스택에만 산다) → hijack 자체가
/// 불가능하고, 남는 위험은 stale task 가 *공유 상태*(watch/cmd_tx)를 건드리는 것뿐인데 그건 전부
/// publish_if_current/store_cmd_if_current/reconnect_guard 한 락으로 닫혀 있다.
#[allow(clippy::too_many_arguments)]
async fn connected_lifetime(
    mut sink: futures_util::stream::SplitSink<Ws, Message>,
    mut stream: futures_util::stream::SplitStream<Ws>,
    mut cmd_rx: mpsc::Receiver<ConnectionCommand>,
    mut info: DaemonInfo,
    my_gen: u64,
    lifecycle: Arc<Lifecycle>,
    discovery: Arc<dyn DaemonDiscovery>,
    rt: Handle,
    handshake_timeout: Duration,
    mut cancel_rx: tokio::sync::watch::Receiver<u64>,
    router: Arc<OutputRouter>,
    registry: WindowChannelRegistry,
    buffer_store: AgentBufferStore,
    app: tauri::AppHandle,
) {
    // ★pending 소유(T6a — 단일 actor 가 단독 소유, Mutex 없음)★: request_id → reply oneshot 상관 맵을
    //   이 task 가 소유한다. main_loop 에 `&mut` 로 빌려줘 SendCommand(insert)·reply 도착(take)·끊김
    //   (drain→Err) 을 한 actor 안에서 직렬 처리한다. 재연결 루프(이 함수)가 owner 라 소켓이 바뀌어도
    //   맵은 유지되지만, ★끊김마다 drain★ 한다(아래) — 옛 소켓의 in-flight 는 새 소켓에서 reply 가 안
    //   오므로 hang 방지를 위해 Err 로 깨운다(request_id idempotency: 호출자가 재시도, spike §3 불변식).
    let mut pending: PendingMap<CommandReply> = PendingMap::new();
    // ★subs 소유(T6b — 단일 actor 단독 소유, Mutex 없음)★: agent_id → SubState(epoch·high-water dedup).
    //   pending 과 동형으로 이 task 가 단독 소유하고 main_loop 에 `&mut` 로 빌려준다. ★단 pending 과 달리
    //   재연결을 넘어 *유지*한다★(끊김마다 drain 하지 않음) — 재연결 후 resubscribe 가 마지막 epoch/seq 로
    //   tail-only Resume 해야 무손실이기 때문(F-B, spike §8). Subscribe arm 이 entry().or_default() 로 채우고
    //   SubscribeAck 가 epoch 갱신, Binary frame 이 decide_output 으로 dedup/epoch 가드를 적용한다.
    //   ★정리(C3)★: 재구독은 router.current_agents() 기반이라(아래 main_loop) 안 보이는 agent 는 재구독 안
    //   되고, 그 SubState 는 main_loop 진입 resubscribe 직후 router 집합 retain 으로 제거된다(누수 방지).
    let mut subs: HashMap<AgentId, SubState> = HashMap::new();
    let mut attempt: u32 = 0;
    loop {
        // main_loop 가 끝난 사유로 재연결 여부를 가른다.
        let exit = main_loop(
            sink,
            stream,
            &mut cmd_rx,
            &mut pending,
            &mut subs,
            my_gen,
            &router,
            &registry,
            &buffer_store,
            &app,
        )
        .await;
        // ★끊김/종료 시 pending drain(no-leak 불변식, spike §3)★: 이 소켓 수명이 끝났으므로 in-flight
        //   명령은 매칭될 reply 가 더는 안 온다 → 전부 꺼내 Err 로 깨운다(호출자 hang 방지). Closed/
        //   Disconnected 둘 다 동일(재연결되더라도 옛 request_id reply 는 새 소켓에 안 옴 — 호출자 재시도).
        // ★정직한 drain 메시지(FIX-2 — ids.rs no-auto-retry 의도와 정합)★: pending 은 *이미 wire 로
        //   나갔으나* reply 를 못 받은 명령이다 — 데몬이 실행했는지 불명이라, 부작용 명령(WriteStdin 등)을
        //   맹목 재시도하면 입력 중복이 된다(ids.rs RequestId 주석). 그래서 "재시도 필요"가 아니라
        //   "결과 불명·맹목 재시도 금지"로 명시한다(호출자가 reconnect 후 결과 조회로 판단).
        for reply in protocol_state::drain_pending(&mut pending) {
            let _ = reply.send(Err(
                "daemon 연결 끊김 — 명령 전송됨·응답 못 받음(결과 불명; 부작용 명령 맹목 재시도 금지)"
                    .to_string(),
            ));
        }
        // ★cmd_rx 버퍼 drain(FIX-1 — queued-but-not-pending)★: select! 경합에서 진 채 cmd_rx mpsc 버퍼에
        //   들어왔지만 actor 가 아직 안 꺼낸 SendCommand 는 pending 에 *없다* — 위 drain 이 못 깨운다. 그대로
        //   두면 재연결 후 *다음 소켓* 에서 실행돼 부작용이 이중 적용된다(WriteStdin 등). 그래서 지금 버퍼에
        //   있는 것만 try_recv 로 비워(EOF 아님 — Empty 까지) Err 로 깨운다. ★cmd_rx 는 닫지 않는다★:
        //   재연결 너머로 carry 되는 채널이라(미래 명령용) 여기서 close 하면 안 된다. 이 명령들은 wire 로
        //   *나간 적이 없으므로* 메시지가 그렇게 말해야 한다(FIX-2 — "미전송·재전송 안전"). reply 없는
        //   Subscribe/Unsubscribe variant 는 그냥 drop(T6b 가 채울 자리).
        while let Ok(buffered) = cmd_rx.try_recv() {
            if let ConnectionCommand::SendCommand { reply, .. } = buffered {
                let _ = reply.send(Err(
                    "daemon 연결 끊김 — 명령 미전송(재전송 안전)".to_string()
                ));
            }
        }
        match exit {
            LoopExit::Closed => {
                // cmd_rx EOF = 명시 close() 또는 stale 미저장 → 재연결 안 함. 종료 Down 가드.
                // ★원자 가드(Fix B)★: stale task 의 종료가 current 의 Connected 를 Down 으로 clobber
                //    하지 않게 publish_if_current 로 비교+send 를 한 락에 묶는다.
                if lifecycle.publish_if_current(my_gen, ConnectionState::Down) {
                    let _ = app.emit("daemon-connection-state", "down");
                } else {
                    tracing::debug!(
                        generation = my_gen,
                        "stale task 종료(close 경로) — Down 미발행(더 새 연결이 current)"
                    );
                }
                return;
            }
            LoopExit::Disconnected => {
                // 비의도 끊김 — 재연결 대상. 먼저 closedByUser/세대 가드를 본다(끊김과 동시에 close 가
                // 들어왔으면 재연결 금지). Stop 이면 종료 Down 가드.
                if lifecycle.reconnect_guard(my_gen) == ReconnectVerdict::Stop {
                    if lifecycle.publish_if_current(my_gen, ConnectionState::Down) {
                        let _ = app.emit("daemon-connection-state", "down");
                    } else {
                        tracing::debug!(
                            generation = my_gen,
                            "끊김 직후 stale/close — Down 미발행(superseded 또는 close 가 이미 Down)"
                        );
                    }
                    return;
                }
                // 재연결 진입 — reconnecting 전이(가드된). stale 이면 발행 안 되고 아래 루프가 Stop 으로 종료.
                if lifecycle.publish_if_current(my_gen, ConnectionState::Reconnecting) {
                    let _ = app.emit("daemon-connection-state", "reconnecting");
                }
                tracing::info!(generation = my_gen, "데몬 끊김 — 재연결 루프 진입");
            }
        }

        // ── 재연결 백오프 루프(attach-only — read_live no-spawn) ──────────────────────────
        // 성공하면 새 sink/stream 으로 main_loop 를 다시 돌리려고 outer loop 로 continue. 소진/Stop 이면
        // Down 후 return. ★ADR-0038★: sleep 은 tokio::time::sleep — 테스트가 time::pause/advance 로
        // 결정론적으로 진행시킨다(매직 실벽시계 0).
        let reconnected = loop {
            // 매 시도 전 가드 — 백오프 sleep 사이에 close/새 connect 가 끼면 즉시 멈춘다.
            if lifecycle.reconnect_guard(my_gen) == ReconnectVerdict::Stop {
                break None;
            }
            if attempt >= MAX_RECONNECT_ATTEMPTS {
                // 소진 — 데몬이 안 살아남는다. Down 정착(가드된). 복구는 명시 connect 로만.
                tracing::warn!(
                    generation = my_gen,
                    attempt,
                    "재연결 소진 — Down 정착(attach-only, 명시 connect 로만 복구)"
                );
                break None;
            }
            // 지수 백오프. ★취소 경쟁(T4 in-flight 취소)★: sleep 을 cancel_rx 와 select! 한다 — 백오프
            //   대기 중 close()/승계 connect 가 cancel 을 켜면 sleep 을 끝까지 안 기다리고 즉시 깨어
            //   reconnect_guard 로 재확인 → Stop 이면 소켓을 열기 전에 탈출한다(이 창은 read_live·소켓
            //   open 이전이라 데몬 무접촉). ★락 밖 await★(reconnect_guard 는 동기).
            let delay = backoff_delay(attempt);
            attempt += 1;
            tokio::select! {
                _ = cancel_rx.changed() => {
                    // 취소 신호 도착(또는 baseline 변경) — guard 로 진짜 Stop 인지 재확인(거짓 wakeup 무시).
                    if lifecycle.reconnect_guard(my_gen) == ReconnectVerdict::Stop {
                        break None;
                    }
                }
                _ = tokio::time::sleep(delay) => {}
            }
            // sleep 후 다시 가드 — sleep 동안 close 가 들어왔을 수 있다(가장 흔한 race 창).
            if lifecycle.reconnect_guard(my_gen) == ReconnectVerdict::Stop {
                break None;
            }

            // ★ADR-0021 attach-only★: read_live(no-spawn)로 현재 daemon.json 을 재조회해 **옮겨간
            //   데몬(hot-swap·크래시 재spawn)의 새 주소를 따라간다**. read_live 는 read-only(데몬 안
            //   깨움). blocking(파일 IO)이라 spawn_blocking 으로 감싼다(async executor 미차단).
            // ★취소 경쟁(T4)★: spawn_blocking 의 *join await* 를 cancel_rx 와 select! 한다. read_live 자체는
            //   spawn_blocking 이라 시작되면 abort 불가지만, side-effect 없는 파일 읽기라 **결과를 버려도
            //   안전**하다 — await 만 select 로 버리고(blocking task 는 백그라운드에서 알아서 끝남) 취소면
            //   소켓을 열기 전에 탈출한다(아직 connect_async 이전 = 데몬 무접촉).
            let disco = discovery.clone();
            let mut read_live_join = rt.spawn_blocking(move || disco.read_live());
            // ★&mut JoinHandle 로 select★: JoinHandle 은 &mut 로 poll 가능 — 취소 arm 이 이겨도 handle 의
            //   소유권이 안 빠져나가, 거짓 wakeup 시 같은 handle 을 마저 await 할 수 있다.
            let fresh = tokio::select! {
                _ = cancel_rx.changed() => {
                    if lifecycle.reconnect_guard(my_gen) == ReconnectVerdict::Stop {
                        break None;
                    }
                    // 거짓 wakeup(아직 current)이면 join 을 마저 기다린다(read_live 결과가 필요).
                    read_live_join.await
                }
                joined = &mut read_live_join => joined,
            };
            let fresh = match fresh {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!(generation = my_gen, "read_live join 실패: {e}");
                    None
                }
            };
            if let Some(new_info) = fresh {
                // 살아있는 데몬 발견(옮겨갔을 수 있음) → 그 주소로 attach(캐시 갱신).
                info = new_info;
            }
            // fresh=None 이면 옛 info(마지막 알려진 주소)로 마지막 시도 — 여전히 attach-only(spawn 아님).
            //   일시적 daemon.json 부재일 수 있어 옛 주소로 시도하고, 실패하면 다음 백오프로.

            // 소켓을 열기 직전 마지막 가드 — read_live join 사이에 close 가 들어왔으면 여기서 멈춰
            //   connect_async 를 아예 시작하지 않는다(소켓 open + Auth 전송 창을 닫는 핵심 지점).
            if lifecycle.reconnect_guard(my_gen) == ReconnectVerdict::Stop {
                break None;
            }

            // 재핸드셰이크 시도. ★취소 경쟁(T4 — 핵심 결함 수정)★: handshake 내부의 connect_async·Auth
            //   send·wait_for_hello await 를 cancel_rx 와 경쟁시킨다(handshake 가 cancel_rx 를 받아 각 await
            //   를 select). close()/승계가 끼면 **소켓을 열지 않거나(connect_async 취소) 연 소켓을 즉시
            //   닫고** Cancelled 로 빠진다 — close 후 stale 소켓이 살아 Auth 를 보내는 창을 닫는다.
            //   성공하면 새 소켓을 outer loop 로 올린다.
            match handshake_cancellable(&info, my_gen, handshake_timeout, &mut cancel_rx).await {
                HandshakeOutcome::Cancelled => {
                    // 취소(close/승계) — guard 로 재확인. Stop 이면 탈출(소켓 미오픈 또는 self-close 완료).
                    if lifecycle.reconnect_guard(my_gen) == ReconnectVerdict::Stop {
                        break None;
                    }
                    // 거짓 취소(아직 current)면 다음 백오프로 재시도(continue inner loop).
                    tracing::debug!(
                        generation = my_gen,
                        "재연결 취소 wakeup 이나 still current — 재시도"
                    );
                }
                HandshakeOutcome::Ok(conn) => {
                    // 핸드셰이크 성공 — Connected 발행(가드). stale 이면 소켓 닫고 Stop.
                    if !lifecycle.publish_if_current(my_gen, ConnectionState::Connected) {
                        tracing::debug!(
                            generation = my_gen,
                            "재연결 핸드셰이크 성공했으나 stale — 폐기"
                        );
                        let _ = conn.sink_close().await;
                        break None;
                    }
                    let _ = app.emit("daemon-connection-state", "connected");
                    // 회복 — attempt 리셋(wsTransport `reconnectAttempt=0` on Hello). 다음 끊김은 처음부터.
                    attempt = 0;
                    tracing::info!(generation = my_gen, "데몬 재연결 성공(Hello 수신)");
                    break Some(conn.into_split());
                }
                HandshakeOutcome::Err(e) => {
                    // 시도 실패(데몬 죽음/거부) — 다음 백오프로. 소진 시 위 attempt 가드가 None.
                    tracing::debug!(generation = my_gen, attempt, "재연결 시도 실패: {e}");
                    // continue inner loop → 다음 백오프.
                }
            }
        };

        match reconnected {
            Some((new_sink, new_stream)) => {
                // 새 소켓으로 main_loop 재진입(outer loop continue).
                sink = new_sink;
                stream = new_stream;
            }
            None => {
                // 소진 또는 Stop(close/stale) → Down 가드 후 종료.
                if lifecycle.publish_if_current(my_gen, ConnectionState::Down) {
                    let _ = app.emit("daemon-connection-state", "down");
                } else {
                    tracing::debug!(
                        generation = my_gen,
                        "재연결 종료 — Down 미발행(stale/close 가 이미 처리)"
                    );
                }
                return;
            }
        }
    }
}

/// 메인 read/write 루프(connected 이후). 단일 task 가 stream/sink 를 단독 소유한 채
/// `tokio::select!` 로 (a) 데몬 수신 (b) cmd 채널을 동시에 대기한다. 종료 사유(`LoopExit`)를 돌려줘
/// 호출자(`connected_lifetime`)가 재연결(Disconnected) vs 종료(Closed)를 가른다(T4).
///
/// ★상태 전이는 호출자가★: 이 함수는 더 이상 종료 시 Down 을 발행하지 않는다(이전 T2 구현과 다름).
/// 재연결이면 Down 이 아니라 Reconnecting 으로 가야 하므로, 종료 후 상태 결정은 호출자에 모은다 —
/// 이 함수는 sink/stream 을 빌려(`&mut cmd_rx` 포함) 루프만 돌고, 끝나면 사유만 보고한다(lifecycle
/// 미접촉 — 상태 결정은 호출자). sink 는 소유로 받아 루프 종료 시 여기서 닫는다(재연결 시 새 소켓이
/// 오므로 옛 소켓은 확실히 정리).
///
/// ## ★request/reply 상관(T6a — load-bearing)★
/// `pending`(request_id → reply oneshot) 은 이 actor 가 단독 소유(호출자가 `&mut` 로 빌려줌, Mutex
/// 없음). 한 select! 루프 안에서 직렬 처리하므로 두 arm 이 동시에 pending 을 만지지 않는다:
///   - cmd_rx arm `SendCommand`: reply 를 `pending[request_id]` 에 *먼저 넣고* → JSON 인코딩 →
///     `sink.send`. send 실패면 *방금 넣은 reply 를 take 해 되돌려* Err 로 깨운다(맵에 좀비 안 남김).
///   - stream arm `Text`(reply): `take_pending(request_id)` 로 꺼내 `reply_outcome` 으로 resolve.
///     broadcast(request_id 없음)는 매칭을 우회한다(T6b 가 emit 배선 — 지금은 무시).
/// 끊김(루프 종료)시 남은 pending 은 호출자(`connected_lifetime`)가 drain→Err 한다(no-leak).
///
/// ## ★출력 라우팅(T6b — load-bearing)★
/// - **Binary arm**: `decode_frame → decide_output(&mut subs[agent], epoch, seq)` 가 epoch/dedup 가드
///   (ADR-0037 Rust 단독 진실원)를 통과시킨 frame 만 `router.targets(agent)` 의 각 창으로 **원본 frame
///   bytes 그대로**(헤더=agent_id 태그 내장) fan-out. 가드 통과분만 보내므로 창측 2차 가드 없음.
/// - **Text arm `SubscribeAck`**: `apply_subscribe_ack` 로 subs 의 epoch 갱신 + high-water 리셋.
/// - **cmd_rx arm `Subscribe`/`Unsubscribe`/`Fire`**: subs 에서 resubscribe_params 로 epoch/after_seq 를
///   채워 wire 송신(reply 없음 — fire-and-forget).
/// - **connect/재연결 후 resubscribe(C1+C2)**: main_loop 진입 시 **`router.current_agents()`(현재 보이는
///   agent = 구독해야 할 집합 SSOT, ADR-0035)** 를 순회하며 각 agent 에 wire Subscribe 를 재전송한다 —
///   subs(누적 맵)가 아니라 router 스냅샷이 진실원이라 비연결 중 배정분도 빠짐없이 구독(C1)되고 안 보이는
///   agent 는 순회 대상이 아니라 유령 구독 0(C2). epoch/after_seq 는 subs 의 SubState 에서(tail Resume/
///   FromOldest). 직후 router 에 없는 agent 의 SubState 를 정리(C3). router 가 비면 no-op(첫 연결 안전).
///
/// ★ADR-0006★: `registry.lock()`(std Mutex) 보유 중 `.await` 절대 금지 — `Channel::send` 는 동기라 OK.
///
/// ## ★handoff resync(FIX-2 — cmd_rx arm `Resync`)★
/// main_loop 진입 resubscribe 는 그 시점 `router.current_agents()` 스냅샷을 쓰는데, cmd_tx 가 lifecycle 에
/// 저장되기 전 들어온 layout 변화는 enqueue 도 유실되고 스냅샷에도 안 잡힐 수 있다(handoff 창). caller
/// (`store_cmd_if_current` 직후)가 `Resync` 를 enqueue 하면 같은 `resubscribe_and_sweep` 을 한 번 더 돌려
/// 그 사이 변화를 따라잡는다 — cmd_tx 저장 후에만 enqueue 되므로 갭이 닫힌다.
/// ★진입 resubscribe + 고아 sweep 공통 경로(C1+C2+C3 / FIX-1 / handoff resync FIX-2)★. main_loop 진입과
/// `Resync` arm 이 같이 부른다. router.current_agents() 를 순회해 각 agent 에 wire Subscribe 재전송(after_seq=
/// 버퍼 최신, 축 A) + subs 정리(C3) + router 에 없는 store cursor 고아 sweep(FIX-1). ★ADR-0006: buffer 락은
/// 잡았다 즉시 풀고(snapshot/after_seq 만), sink.send().await 는 buffer 락 미보유★.
async fn resubscribe_and_sweep(
    sink: &mut futures_util::stream::SplitSink<Ws, Message>,
    subs: &mut HashMap<AgentId, SubState>,
    my_gen: u64,
    router: &Arc<OutputRouter>,
    registry: &WindowChannelRegistry,
    buffer_store: &AgentBufferStore,
) {
    // ★router 라우팅 스냅샷 기반(C1+C2)★. 순회 대상은 `subs`(누적 맵)가 아니라 router.current_agents()
    //   (현재 화면에 보이는 agent = 구독해야 할 집합 SSOT, ADR-0035) — 비연결 중 배정분도 빠짐없이 구독(C1)
    //   되고 안 보이는 agent 는 순회 대상이 아니라 유령 구독 0(C2). after_seq = buffer 최신(축 A, ADR-0040).
    let current = router.current_agents();
    for agent_id in &current {
        let after_seq = {
            let store = buffer_store.lock().expect("buffer_store poisoned");
            store.resubscribe_after_seq(*agent_id)
        }; // ★락 즉시 drop — 이후 sink.send().await 는 buffer 락 미보유★
        let p = protocol_state::resubscribe_params(subs.entry(*agent_id).or_default(), after_seq);
        let cmd = AgentCommand::Subscribe {
            agent_id: *agent_id,
            epoch: p.epoch,
            after_seq: p.after_seq,
        };
        match serde_json::to_string(&cmd) {
            Ok(text) => {
                if let Err(e) = sink.send(Message::Text(text.into())).await {
                    tracing::debug!(generation = my_gen, "resubscribe 송신 실패: {e}");
                }
            }
            Err(e) => tracing::warn!(generation = my_gen, "resubscribe 직렬화 실패: {e}"),
        }
    }
    // ★subs 메모리 정리(C3 — 보수적)★: router 현재 집합에 없는 agent 의 SubState 제거(누수 방지). 다시
    //   보이면 layout subscribe 델타가 새 SubState(FromOldest)를 만들어 전체 replay → 무손실(효율만 포기).
    //   resubscribe *직후*라 방금 구독한(router 에 있는) agent 는 절대 안 지워진다.
    let visible: std::collections::HashSet<AgentId> = current.iter().copied().collect();
    subs.retain(|agent_id, _| visible.contains(agent_id));
    // ★양방향 slot reconcile(FIX-1 — 유실 replay 복구 + 고아 sweep)★. 옛 단방향 `sweep_orphans`(고아 제거만)
    //   를 양방향으로 일반화한다 — `replay_slots`/`drop_slots` 둘 다 bounded mpsc try_send 라 채널 full 이면
    //   silent drop 되는데, 옛 코드는 drop_slots 유실(고아 cursor)만 흡수하고 ★replay_slots 유실(현재 visible
    //   한데 cursor 없는 slot = 새 창 빈 화면)은 복구 경로가 없었다★. router.current_slots()(현재 실재 slot
    //   SSOT, ADR-0035)를 keep 으로 store 와 양방향 정합:
    //     (a) keep 에 있는데 cursor 없는 slot → fresh 신설 + 전체 replay(유실된 replay_slots 복구).
    //     (b) keep 에 없는 cursor → 제거(drop_slots full 누수 흡수 — 기존 동작).
    //   ★멱등★: (a)는 cursor 없는 slot 만 건드린다 — 정상 전달 중인 cursor 는 불가침이라 중복 replay 0
    //   (이미 잘 전달 중인 slot 을 fresh 리셋해 다시 보내지 않음). 그래서 진입·Resync·재연결마다 무해.
    let keep: Vec<((crate::output_router::WindowLabel, AgentId), AgentId)> = router
        .current_slots()
        .into_iter()
        .map(|(label, agent)| ((label, agent), agent)) // ViewSlotKey = (label, agent), agent 동봉.
        .collect();
    // ★deliverable 게이트(5차 FIX — reconcile 복구도 cursor advance ⟺ delivery)★: 현재 Channel 이 등록된
    //   (label, agent) slot 집합을 buffer 락 *전*에 떠 둔다(registry 락 → 해제 → buffer 락, 중첩 0 ADR-0006 —
    //   on_frame/ReplaySlots arm 동형). reconcile_slots 는 "값 None(신설됐으나 미전달) + deliverable" 인 slot 만
    //   복구(전체 replay)하고, 미deliverable(미mount) slot 은 membership 만 유지한다 — 미등록 slot 을 replay 하면
    //   전달 못 할 구간을 advance 해 다시 영구 유실(그 창은 mount 시 등록 트리거가 채움).
    let registered = output_channel::registered_labels(registry);
    // ★build_deliverable 는 ViewSlotKey 슬라이스를 받는다★: keep 은 `(ViewSlotKey, agent)` wrapper 라
    //   ViewSlotKey(첫 요소)만 떼어 넘긴다(on_frame/ReplaySlots arm 과 동일 순수 필터 — drift 차단).
    let keep_slots: Vec<crate::output_channel::ViewSlotKey> =
        keep.iter().map(|(slot, _)| slot.clone()).collect();
    let deliverable = output_channel::build_deliverable(&keep_slots, &registered);
    let (replay, removed) = {
        let mut store = buffer_store.lock().expect("buffer_store poisoned");
        store.reconcile_slots(&keep, &deliverable)
    }; // ★buffer 락 즉시 drop — 아래 flush_snapshot(registry 락)은 buffer 락 미보유(ADR-0006)★
       // ★락 밖 send★: 복구된 replay snapshot 을 registry 락으로 각 창 Channel 에 flush(buffer 락 ⊃ registry
       //   락 중첩 0 — on_frame/ReplaySlots arm 동형).
    output_channel::flush_snapshot(registry, replay, my_gen);
    if removed > 0 {
        tracing::debug!(
            generation = my_gen,
            removed,
            "고아 cursor sweep(drop_slots full 누수 reconcile)"
        );
    }
}

#[allow(clippy::too_many_arguments)]
async fn main_loop(
    mut sink: futures_util::stream::SplitSink<Ws, Message>,
    mut stream: futures_util::stream::SplitStream<Ws>,
    cmd_rx: &mut mpsc::Receiver<ConnectionCommand>,
    pending: &mut PendingMap<CommandReply>,
    subs: &mut HashMap<AgentId, SubState>,
    my_gen: u64,
    router: &Arc<OutputRouter>,
    registry: &WindowChannelRegistry,
    buffer_store: &AgentBufferStore,
    app: &tauri::AppHandle,
) -> LoopExit {
    // ★connect/재연결 resubscribe — router 라우팅 스냅샷 기반(C1+C2)★. 순회 대상은 `subs`(연결 task 가
    //   한 번이라도 구독한 적 있는 누적 맵)가 아니라 **`router.current_agents()`(현재 화면에 보이는 agent =
    //   구독해야 할 집합의 SSOT, ADR-0035)** 다. 이유:
    //   - (C1) subs 기반이면 connect 로 새 task 가 뜰 때 subs 가 빈 HashMap 으로 시작 → 비연결 중 layout 에
    //     배정된 agent 가 영영 구독 안 된다(connect 후 재동기 트리거 부재). router 는 비연결 중에도 layout
    //     command 가 rebuild 로 항상 최신화하므로(델타 송신만 no-op 이었음), 그 스냅샷을 돌면 비연결 중
    //     배정분도 빠짐없이 구독된다.
    //   - (C2) subs 는 Unsubscribe 해도 SubState 를 제거 안 하므로(F-B), subs 기반 재구독은 지금 안 보이는
    //     agent 까지 유령 구독한다. router 에 없는 agent 는 애초에 순회 대상이 아니라 유령 구독 0.
    //   각 agent 의 epoch 는 subs 의 SubState(있으면 마지막 epoch=tail Resume, 없으면 FromOldest)에서, ★
    //   after_seq 는 buffer_store 최신 seq(축 A — ADR-0040)★ 에서 가져온다 → 데몬은 클라 버퍼에 없는
    //   것만 보내 비중복 append. 첫 연결도 router 에 agent 있으면 구독, 없으면 no-op 이라 안전.
    // ★진입 resubscribe + sweep(C1+C2+C3 / FIX-1)★. handoff resync(FIX-2)도 같은 경로를 재사용한다.
    resubscribe_and_sweep(&mut sink, subs, my_gen, router, registry, buffer_store).await;
    // 루프 종료 사유를 한 곳에서 로깅하려고 break 로 사유를 끌어올린다(핫패스 frame 수신 본문엔
    // 로그 미부착 — Text/Binary 청크는 per-frame 빈도라 trace 미사용 정책 유지).
    let exit = loop {
        tokio::select! {
            // 데몬 → 클라 수신.
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(msg)) => {
                        match msg {
                            Message::Text(text) => {
                                // 데몬 control 이벤트. T6a: reply(request_id echo) 면 pending 매칭→resolve.
                                //   broadcast(request_id 없음)는 매칭 우회 — T6b 가 app.emit 배선(지금은 무시).
                                // 파싱 실패는 무시(데몬은 valid JSON 만 보낸다 — 부분/미래 프레임 방어).
                                if let Ok(ev) = serde_json::from_str::<AgentEvent>(&text) {
                                    if let Some(rid) = protocol_state::event_reply_request_id(&ev) {
                                        // 내 in-flight 요청의 reply — oneshot 으로 resolve(Ok/Err).
                                        //   모르는 request_id(take_pending=None)면 무시(편승/중복 reply 방어).
                                        if let Some(reply) = protocol_state::take_pending(pending, &rid)
                                        {
                                            let _ = reply.send(protocol_state::reply_outcome(ev));
                                        }
                                    } else if let AgentEvent::SubscribeAck {
                                        agent_id,
                                        current_epoch,
                                        ..
                                    } = ev
                                    {
                                        // ★구독 ack(ADR-0040)★: SubState.epoch 갱신만 한다 — 재연결
                                        //   resubscribe 의 wire epoch echo(tail Resume) 용도. ★버퍼 reset 은
                                        //   여기서 안 한다★: epoch 전환에 따른 콘텐츠/커서 reset 은 on_frame 이
                                        //   frame.epoch 태깅으로 단독 처리한다(SubscribeAck 를 기다리지 않음 —
                                        //   §4b, 단일 actor 직렬). 데몬 wire 가 Ack→replay→frame FIFO 를 보장
                                        //   (단일 writer, connection_core R1)하므로 Ack 가 새 epoch frame 보다
                                        //   먼저 오지만, on_frame 태깅이 순서 무관하게 안전해 reset 트리거를
                                        //   frame 쪽에 둔다(두 모델 혼재 차단 — reset_all_windows 폐기).
                                        let _changed = protocol_state::apply_subscribe_ack(
                                            subs.entry(agent_id).or_default(),
                                            current_epoch,
                                        );
                                    }
                                    // ★T7c: request_id 없는 broadcast 를 app.emit 으로 전 webview push.
                                    //   Err 는 무시(webview 없음/채널 오류 등은 치명적이지 않다).
                                    else {
                                        emit_broadcast(app, &ev);
                                    }
                                }
                            }
                            Message::Binary(bytes) => {
                                // ★출력 binary frame → 공유 버퍼 append + per-view cursor read(ADR-0040 2단계)★.
                                //   헤더(tag/agent_id/epoch/seq) 디코드 → buffer_store.on_frame 이
                                //   (a) per-agent epoch 태깅(frame.epoch ≠ 태그면 그 agent 콘텐츠/커서 reset)
                                //   (b) append (c) 그 agent 를 보는 모든 창 cursor 부터 read → 보낼
                                //   (WindowLabel, bytes) snapshot 을 돌려준다. ★dedup·epoch 가드가 여기로
                                //   통합됐다(ADR-0037 Rust 단독 진실원 유지) — decide_epoch/should_deliver/
                                //   per-window WindowSeqTracker(min 모델) 전부 폐기, 공유 버퍼 cursor 가 단일
                                //   진실원★.
                                //
                                //   ★ADR-0006 락 규율(deep 리뷰 급소)★: buffer 락 안에선 snapshot 수집만,
                                //   Channel send 는 락을 푼 뒤 flush_snapshot 이 registry 락으로 한다. buffer
                                //   락 ⊃ registry 락 중첩이 0(데몬 output_core C4 패턴 동형, 역전 데드락 0).
                                //   ★버퍼 저장 단위 = 원본 binary frame bytes 전체(헤더 포함)★(TRD §1·§2):
                                //   프론트 tauriTransport.ts 가 `[tag][agentId:16][epoch:4][seq:8][payload]`
                                //   를 받아 decodeOutputFrame 으로 푼다 — 프론트 무변경 원칙(ADR-0037)상 버퍼는
                                //   payload 가 아니라 원본 frame `bytes` 를 그대로 저장·replay 해야 한다. epoch/
                                //   seq 는 decode_frame 으로 뽑아 on_frame 의 태깅·cursor 인덱스로만 쓴다.
                                match decode_frame(&bytes) {
                                    Ok(frame) => {
                                        // ★cursor 생명주기 동기화(router 스냅샷 파생)★: 이 agent 를 현재
                                        //   보는 창 목록을 router.targets(lock-free load)로 받아 (창, agent)
                                        //   slot 키를 만든다. store.sync_viewers 가 신규 slot 신설(fresh=None)·
                                        //   사라진 slot 제거(생명주기)를 맞춘 뒤 on_frame 이 append+read 한다.
                                        //   router.targets 는 핫패스 락 0(ArcSwap), slot 키 생성은 보이는 창
                                        //   수만큼(소수)이라 핫패스 부담 미미.
                                        let labels = router.targets(frame.agent_id);
                                        // ★viewer 0 가드(FIX-4 — append 진동 차단)★: 어느 창도 이 agent 를
                                        //   안 보면(layout 이 떼어냈는데 데몬 in-flight frame 이 더 옴) store 를
                                        //   아예 안 만진다. 안 그러면 on_frame 이 빈 content 를 신설/append 하고
                                        //   (보낼 slot 0 이라 出力은 0) resubscribe_after_seq 가 그 한 frame 짜리
                                        //   seq 를 반환해 재구독 기준이 오염된다(drop_slots 로 비운 버퍼가 부활).
                                        //   ★축 A 비회귀★: 정상 viewer 있는 agent 는 이 가드를 안 타 콘텐츠 append
                                        //   가 그대로다(unviewed 콘텐츠 버퍼링은 *보는 동안* 만 의미 — 안 보면 폐기가
                                        //   생명주기 정답, TRD §4).
                                        //   ★router 빈(labels 0) = 무손실 무해(3차 재리뷰 확정)★: labels 가 비는 건
                                        //   layout 이 그 agent 를 *떼어낸* 직후다 — 그러면 같은 layout mutation 이
                                        //   축 A `to_unsubscribe`(데몬 Unsubscribe, 구독 소유=layout SSOT)도 곧 보내
                                        //   데몬 스트림 자체가 멈춘다(이 in-flight frame 은 그 직전 발사분의 잔여).
                                        //   그래서 append 를 스킵해도 ① 그 slot 은 곧 drop 대상이라 어차피 폐기될
                                        //   cursor 고(`drop_slots`/reconcile sweep), ② 그 agent 가 *다시* 배정되면
                                        //   데몬이 원본을 보유(장기 원본=데몬, ADR-0040)해 재구독 replay 로 무손실
                                        //   복구된다 — 스킵된 한 frame 은 데몬 원본에 남아 있다. 즉 클라 버퍼에서만
                                        //   누락이고 진실원(데몬)엔 보존이라 재배정 시 복구 = 무손실 무해.
                                        if labels.is_empty() {
                                            continue;
                                        }
                                        let slots: Vec<(String, AgentId)> = labels
                                            .iter()
                                            .map(|label| (label.clone(), frame.agent_id))
                                            .collect();
                                        // ★deliverable 게이트(근원2)★: 현재 Channel 이 등록된 (label, agent)
                                        //   slot 집합을 buffer 락 *전*에 떠 둔다(registry 락 → 해제 → buffer
                                        //   락 — 중첩 0, ADR-0006). on_frame 은 deliverable 에 있는 slot 만
                                        //   cursor advance·snapshot 한다 → 미mount 창의 stale advance(전달 못
                                        //   한 걸 전진)로 인한 영구 유실 차단(그 창은 mount 시 resubscribe_slot
                                        //   이 fresh 리셋+전체 replay 로 무손실 복구).
                                        let registered =
                                            output_channel::registered_labels(registry);
                                        let deliverable =
                                            output_channel::build_deliverable(&slots, &registered);
                                        let snapshot = {
                                            // ★buffer 락 — sync + snapshot 수집까지만(.await 0, send 없음)★
                                            let mut store =
                                                buffer_store.lock().expect("buffer_store poisoned");
                                            store.sync_viewers(frame.agent_id, &slots);
                                            // ★epoch 폭 경계(wire u32 → store u64)★: wire 헤더 epoch 는 u32
                                            //   (codec.rs), store epoch 태그는 u64 다. widening cast 라 손실 0
                                            //   (u32 전체가 u64 에 안전히 들어감). store 가 u64 인 건 seq(u64)와
                                            //   타입 결을 맞춰 한 자료구조에서 다루기 위함 — 값 의미는 wire u32 그대로.
                                            store.on_frame(
                                                frame.agent_id,
                                                frame.epoch as u64,
                                                frame.seq,
                                                bytes.to_vec(), // 원본 frame bytes(헤더 포함) 저장.
                                                &deliverable,
                                            )
                                        }; // ★buffer 락 drop★
                                        // ★락 밖 send★: registry 락 잡고 각 창 Channel 로 원본 frame 전송.
                                        output_channel::flush_snapshot(registry, snapshot, my_gen);
                                    }
                                    // 디코드 실패(부분/미래 프레임) → 무시(방어).
                                    Err(_) => {}
                                }
                            }
                            // Ping/Pong 은 tungstenite 가 자동 응답(내부). Close 면 끊김(재연결 대상).
                            Message::Close(_) => break LoopExit::Disconnected,
                            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                        }
                    }
                    // 데몬이 연결을 닫음/오류 → 끊김(재연결 대상 — 호출자가 백오프 재연결).
                    Some(Err(_)) | None => break LoopExit::Disconnected,
                }
            }
            // invoke → 연결 task 명령. cmd_rx 가 None(모든 sender drop = 명시 close/stale 미저장) 이면
            // 종료(재연결 안 함). DaemonClient.close() 가 cmd_tx 를 drop → 여기로 온다.
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(ConnectionCommand::SendCommand { cmd, reply }) => {
                        // ★request/reply 배선(T6a)★. request_id 추출 → pending 등록 → wire 송신.
                        //   send_command 가 request_id 있는 명령만 넣지만, 방어적으로 None 이면 즉시 Err
                        //   (매칭 키 없는 명령은 reply 가 안 와 영구 pending = hang 이므로).
                        let Some(rid) = protocol_state::command_request_id(&cmd) else {
                            let _ = reply.send(Err(
                                "send_command: request_id 없는 명령은 reply 매칭 불가".to_string(),
                            ));
                            continue;
                        };
                        // ★send 전에 pending 등록★: 인코딩/송신 사이에 reply 가 먼저 도착해도(loopback
                        //   극단) take 할 슬롯이 있어야 한다. 송신 실패 시 아래서 도로 꺼낸다.
                        // ★중복 request_id 가드(FIX-4 — 계약 명시)★: UUIDv4 충돌은 사실상 불가능하나,
                        //   insert 가 prior oneshot 을 *조용히* 떨어뜨리면 그 호출자는 영구 hang 한다. 그래서
                        //   반환값을 잡아 Some(prev)면 그 옛 슬롯을 Err 로 깨우고(no-hang) warn + debug_assert
                        //   로 uniqueness 계약 위반을 시끄럽게 드러낸다(prod 은 계속 진행 = 새 reply 가 승계).
                        if let Some(prev) = pending.insert(rid, reply) {
                            tracing::warn!(
                                generation = my_gen,
                                "중복 request_id 충돌 — 옛 pending 슬롯을 Err 로 깨움(UUIDv4 라 사실상 불가)"
                            );
                            let _ = prev.send(Err("중복 request_id — 옛 요청 취소".to_string()));
                            debug_assert!(false, "request_id 는 UUIDv4 로 유일해야 한다(충돌 발생)");
                        }
                        match serde_json::to_string(&cmd) {
                            Ok(text) => {
                                if let Err(e) = sink.send(Message::Text(text.into())).await {
                                    // 송신 실패(소켓 죽음) → 방금 넣은 reply 를 도로 꺼내 Err 로 깨운다
                                    //   (맵에 좀비 안 남김). 소켓은 곧 끊겨 다음 select 가 Disconnected.
                                    if let Some(reply) = protocol_state::take_pending(pending, &rid) {
                                        let _ = reply.send(Err(format!("명령 송신 실패: {e}")));
                                    }
                                }
                            }
                            Err(e) => {
                                // 직렬화 실패(있어선 안 됨) — pending 되돌려 Err.
                                if let Some(reply) = protocol_state::take_pending(pending, &rid) {
                                    let _ = reply.send(Err(format!("명령 직렬화 실패: {e}")));
                                }
                            }
                        }
                    }
                    // ★출력 구독(ADR-0040 — fire-and-forget, reply 없음)★. SubState 의 epoch + ★버퍼
                    //   최신 seq(축 A)★ 로 after_seq 를 채워 wire Subscribe 송신(신규 agent=버퍼 없음=None=
                    //   FromOldest / 이미 보던 agent=tail-only). min_render_seq(창 합산) 폐기.
                    Some(ConnectionCommand::Subscribe { agent_id }) => {
                        let after_seq = {
                            let store = buffer_store.lock().expect("buffer_store poisoned");
                            store.resubscribe_after_seq(agent_id)
                        }; // ★락 즉시 drop — send_fire().await 는 buffer 락 미보유★
                        let p = protocol_state::resubscribe_params(
                            subs.entry(agent_id).or_default(),
                            after_seq,
                        );
                        let cmd = AgentCommand::Subscribe {
                            agent_id,
                            epoch: p.epoch,
                            after_seq: p.after_seq,
                        };
                        send_fire(&mut sink, &cmd, my_gen, "Subscribe").await;
                    }
                    // 출력 구독 해제(fire-and-forget). ★subs 에서 SubState 제거하지 않는다★(F-B: 재구독=
                    //   Resume tail 정합, 유실0 — spike §8). 정리는 후속 작업.
                    Some(ConnectionCommand::Unsubscribe { agent_id }) => {
                        let cmd = AgentCommand::Unsubscribe { agent_id };
                        send_fire(&mut sink, &cmd, my_gen, "Unsubscribe").await;
                    }
                    // reply 없는 일반 명령(Resize 등) — 그냥 wire 송신.
                    Some(ConnectionCommand::Fire { cmd }) => {
                        send_fire(&mut sink, &cmd, my_gen, "Fire").await;
                    }
                    // ★mount-즉시-replay(축 B — BLOCK-2 + 근원2)★: 새/재등록 (window, agent) slot 에 그
                    //   시점 버퍼를 **cursor fresh 리셋 + 전체 replay**. ★on_frame 과 같은 actor 스레드라
                    //   순서 직렬화★(opus F2 — replay→live 역전 방지).
                    //
                    //   ★fresh 플래그로 배정/등록 트리거 분리(FIX-2 — mount 1회 replay 멱등)★: 옛 구현은
                    //   두 트리거(layout 배정 델타 · subscribe_output Channel 등록) 모두 무조건 fresh 리셋
                    //   (resubscribe_slot)이라 정상 mount 에서 같은 버퍼가 연속 2회 전체 flush 됐다(무중복이
                    //   프론트 dedup 에만 의존 — ADR-0037 어긋남). 이제:
                    //     - fresh=false(배정 트리거): store.subscribe — cursor 없을 때만 신설+replay(불가침).
                    //       정상 mount 의 첫 replay 1회(1→2 둘째 창은 cursor 없어 신설, 기존 창 불가침).
                    //     - fresh=true(등록 트리거): store.resubscribe_slot — cursor 있어도 None 리셋+전체 replay.
                    //       webview reload(Channel 교체 → 새 xterm 빈 화면)를 fresh 로 채운다(stale 이어보기 금지,
                    //       근원2). on_frame 의 deliverable 게이트(미mount 창 advance 금지)와 짝.
                    //
                    //   ★ADR-0006 락 규율★: buffer 락 안에선 snapshot 수집만, Channel send 는 락 밖
                    //   flush_snapshot(registry 락) — buffer 락 ⊃ registry 락 중첩 0(on_frame 동형).
                    Some(ConnectionCommand::ReplaySlots { slots, fresh }) => {
                        // ★deliverable 게이트(5차 FIX — mount replay 도 cursor advance ⟺ delivery)★: 현재
                        //   Channel 이 등록된 (label, agent) slot 집합을 buffer 락 *전*에 떠 둔다(registry 락 →
                        //   해제 → buffer 락 — 중첩 0, ADR-0006). subscribe/resubscribe_slot 은 deliverable 에
                        //   있는 slot 만 advance·snapshot 한다 → 배정만 됐고 webview 미mount(Channel 미등록)인
                        //   slot 의 stale advance(전달 못 할 구간 전진 → 영구 유실)를 차단한다(on_frame 동형).
                        //   ★등록 트리거(fresh=true)는 subscribe_output 이 registry insert *직후* 보내므로 그
                        //   slot 은 항상 deliverable 안 → 즉시 전체 replay(빈 화면 0). 배정 트리거(fresh=false)는
                        //   미등록이면 cursor 만 신설(membership)해 두고, 그 창 mount 시 등록 트리거가 채운다.
                        let registered = output_channel::registered_labels(registry);
                        let deliverable =
                            output_channel::build_deliverable(&slots, &registered);
                        let snapshot = {
                            // ★buffer 락 — subscribe/resubscribe_slot(snapshot 수집)까지만, .await/send 0★
                            let mut store =
                                buffer_store.lock().expect("buffer_store poisoned");
                            let mut out: Vec<((String, AgentId), Vec<u8>)> = Vec::new();
                            for (label, agent_id) in slots {
                                if fresh {
                                    // 등록 트리거: cursor 있어도 fresh 리셋+(deliverable 이면)전체 replay.
                                    out.extend(store.resubscribe_slot(
                                        (label, agent_id),
                                        agent_id,
                                        &deliverable,
                                    ));
                                } else {
                                    // 배정 트리거: cursor 없을 때만 신설+(deliverable 이면)replay(불가침).
                                    out.extend(store.subscribe(
                                        (label, agent_id),
                                        agent_id,
                                        &deliverable,
                                    ));
                                }
                            }
                            out
                        }; // ★buffer 락 drop★
                        output_channel::flush_snapshot(registry, snapshot, my_gen);
                    }
                    // ★slot cursor 정리(축 B — FIX-3)★: 사라진 slot cursor 제거(마지막이면 content drop).
                    //   frame 독립 — terminal+부분 닫힘도 정상 폐기. send 없음(폐기엔 출력 0)이라 락 밖 불필요.
                    Some(ConnectionCommand::DropSlots { slots }) => {
                        let mut store = buffer_store.lock().expect("buffer_store poisoned");
                        for (label, agent_id) in slots {
                            store.unsubscribe(&(label, agent_id));
                        }
                    }
                    // ★handoff race resync(FIX-2)★: cmd_tx 저장 직후 caller 가 보낸 재동기 — 진입 resubscribe
                    //   스냅샷과 cmd_tx 저장 사이에 유실된 layout 변화를 router 현재 집합으로 다시 따라잡는다
                    //   (재구독 + subs 정리 + 고아 sweep). 진입 경로와 동일 함수라 정합 보장.
                    Some(ConnectionCommand::Resync) => {
                        resubscribe_and_sweep(&mut sink, subs, my_gen, router, registry, buffer_store)
                            .await;
                    }
                    None => break LoopExit::Closed,
                }
            }
        }
    };
    // 루프 1회 종료 = 이 소켓의 수명 끝. 옛 소켓은 확실히 닫는다(재연결이면 새 소켓이 온다). ★락 밖★.
    tracing::info!(generation = my_gen, ?exit, "데몬 WS main_loop 종료");
    let _ = sink.close().await;
    // 상태 전이(Down/Reconnecting)는 호출자(connected_lifetime)가 가드와 함께 결정한다 — 여기선 사유만.
    exit
}

/// ★T7c: broadcast 이벤트(request_id 없음) → app.emit 으로 전 webview push.★
///
/// AgentListUpdated / StatusChanged / RestoreResult / ProfileListUpdated 에 대응하는 Tauri 이벤트를
/// 발행한다. 그 외 variant(Ack·SubscribeAck·Output·ReplayComplete·AgentList/ProfileList/Snapshot 등)는
/// request_id 있거나 Binary 평면이거나 내부 소비라 emit 대상이 아니다 → 조용히 no-op.
/// Err 무시(webview 없음·채널 오류는 치명적이지 않음).
fn emit_broadcast(app: &tauri::AppHandle, ev: &AgentEvent) {
    match ev {
        AgentEvent::AgentListUpdated { agents } => {
            let _ = app.emit("agent-list-updated", agents);
        }
        AgentEvent::StatusChanged {
            agent_id,
            status,
            epoch,
        } => {
            let _ = app.emit(
                "status-changed",
                serde_json::json!({
                    "agentId": agent_id,
                    "status": status,
                    "epoch": epoch,
                }),
            );
        }
        AgentEvent::RestoreResult { report } => {
            let _ = app.emit(
                "restore-result",
                serde_json::json!({
                    "result": report,
                }),
            );
        }
        AgentEvent::ProfileListUpdated { profiles } => {
            let _ = app.emit("profile-list-updated", profiles);
        }
        // Ack / SubscribeAck / Output / ReplayComplete / Hello / AgentList / ProfileList /
        // Snapshot / Created / Spawned / InputLeaseChanged — request_id 있거나 내부 소비 or Binary 평면.
        _ => {}
    }
}

/// ★fire-and-forget 송신(T6b)★: reply 없는 명령(Subscribe/Unsubscribe/Resize)을 JSON 으로 wire 송신.
/// 송신 실패(소켓 죽음)는 로깅만 — reply 가 없어 깨울 oneshot 이 없고, 소켓은 곧 끊겨 다음 select 가
/// Disconnected 로 빠진다(재연결 시 layout 이 다시 rebuild/resubscribe).
async fn send_fire(
    sink: &mut futures_util::stream::SplitSink<Ws, Message>,
    cmd: &AgentCommand,
    my_gen: u64,
    kind: &str,
) {
    match serde_json::to_string(cmd) {
        Ok(text) => {
            if let Err(e) = sink.send(Message::Text(text.into())).await {
                tracing::debug!(generation = my_gen, "{kind} fire 송신 실패: {e}");
            }
        }
        Err(e) => tracing::warn!(generation = my_gen, "{kind} 직렬화 실패: {e}"),
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
