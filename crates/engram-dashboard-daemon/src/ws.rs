//! WebSocket 서버 본체 (phase 2 step 4b).
//!
//! 책임: accept 된 TCP stream 을 WS 업그레이드(Origin allowlist) → 1초 내 첫 frame 토큰 auth →
//! AgentCommand/AgentEvent 프레임 핸들링(manager 위임). 출력 hot path 는 binary frame(codec),
//! control 은 JSON.
//!
//! ★동시성 모델(위험 지점)★
//! - **연결당 단일 writer**: SplitSink 는 동시 write 불가. 그래서 모든 출력 frame·control JSON 을
//!   연결당 단일 `mpsc::Sender<WsOutbound>`(conn_tx)에 넣고, write_task 한 곳만 SinkHalf 에 write
//!   한다. SubscribeAck→replay→live 의 FIFO 순서가 이 단일 큐로 보장된다.
//! - **try_send vs await 경계**: pump 스레드에서 호출되는 `WsOutputSink::send` 는 절대 block 금지
//!   (try_send 만). async read_task 의 control 전송은 await 허용(.send().await).
//! - **out-of-band 종료 신호(close_signal)**: conn_tx 가 full 이면 큐 안 마커(WsOutbound::Close)도
//!   try_send 실패해 좀비 연결이 된다. 그래서 큐 **밖**의 `Arc<Notify>` close_signal 을 둔다.
//!   WsOutputSink 가 full 을 만나면 `close_signal.notify_one()`(sync 안전)으로 신호하고,
//!   write_task 는 `tokio::select!` 로 conn_rx.recv() 와 close_signal.notified() 를 동시에 대기해
//!   큐가 막혀 있어도 깨어 sink_half.close() 후 break → cleanup 한다.
//! - **레지스트리**: status 브로드캐스트용. 모든 연결의 conn_tx 를 ConnId→Sender 맵으로 보관해
//!   DaemonStatusSink 가 try_send(Text) 로 전 연결에 fanout.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::profile::RestoreReport as CoreRestoreReport;
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo as CoreAgentInfo, AgentStatus as CoreStatus, OutputFrame, OutputPayload,
    OutputSink, SinkError, SinkId, StatusSink,
};

use engram_dashboard_protocol::{
    encode_structured_frame, encode_terminal_frame, AgentCommand, AgentEvent, PROTOCOL_VERSION,
};

use crate::connection_core::{
    agent_list_event, core_agents_to_wire, core_report_to_wire, core_status_to_wire, event_json,
    hello_event, output_event_to_wire, ConnectionCore, ConnectionSession, MultiViewState, Outbound,
    OutboundSink as CoreOutboundSink, SinkError as CoreSinkError,
};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch, Notify};
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::Message;

/// 연결당 송신 큐 용량. ReplayBuffer.max_events(4096) + control_slack(512) = 4608.
/// replay 전체가 들어가도 control 여유가 남게 한다(output_core.rs 불변식과 정합).
const CONN_TX_CAP: usize = 4608;

/// auth 첫 frame 대기 한도. 이 안에 Auth Text 가 안 오면 close.
const AUTH_TIMEOUT: Duration = Duration::from_secs(1);

/// 운영 기본 keepalive 주기 — 데몬이 능동 WS Ping 을 보내는 간격.
const DEFAULT_PING_INTERVAL: Duration = Duration::from_secs(20);
/// 운영 기본 idle 한도 — 마지막 클라 수신 후 이 시간 넘게 무응답이면 half-open 으로 보고 close.
/// ping_interval 의 2.5배(여러 Ping 을 놓쳐야 끊김 — 일시 지연 위양성 방지).
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(50);

/// WS application-level keepalive 설정(A). 능동 Ping 주기 + idle 한도.
///
/// ★half-open 감지★: tungstenite 는 들어온 Ping 에 자동 Pong 만 하고 능동 Ping 은 안 보낸다.
/// FIN 없이 끊기는 연결(sleep/wake·NAT 타임아웃·모바일 터널)에서 TCP keepalive(기본 2시간)는
/// 무의미하므로, write_task 가 ping_interval 마다 Ping 을 보내고 read_task 가 마지막 수신 시각을
/// 기록한다. idle_timeout 초과면 close_signal 로 그 연결을 끊는다(좀비 구독/broadcast 누수 방지).
///
/// ★테스트 주입★: 상수 하드코딩이면 테스트가 수십 초 걸리므로, 짧은 값(예 200ms/600ms)을
/// 주입할 수 있게 설정 가능하게 둔다. 운영 경로는 `default()`(20s/50s) 그대로.
#[derive(Clone, Copy, Debug)]
pub struct KeepaliveConfig {
    pub ping_interval: Duration,
    pub idle_timeout: Duration,
}

impl Default for KeepaliveConfig {
    fn default() -> Self {
        Self {
            ping_interval: DEFAULT_PING_INTERVAL,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
        }
    }
}

/// 허용 Origin allowlist(기본). Origin 없음(네이티브/하네스)은 허용 — 토큰이 주 방어.
const ALLOWED_ORIGINS: &[&str] = &[
    "http://localhost:1420",
    "http://127.0.0.1:1420",
    "tauri://localhost",
    "https://tauri.localhost",
];

/// 연결 식별자(단조 증가). 레지스트리 키.
pub type ConnId = u64;

/// 단일 writer 큐로 흐르는 출력 단위. 모든 frame·control·close 가 이걸 통해 write_task 로 간다.
#[derive(Debug)]
pub enum WsOutbound {
    /// control JSON(AgentEvent 직렬화).
    Text(String),
    /// 출력 binary frame(codec).
    Binary(Vec<u8>),
    /// 연결 종료 — write_task 가 이걸 받으면 close 후 break. reason 은 로그/디버깅용.
    Close(String),
}

/// status 브로드캐스트용 연결 레지스트리. connect 시 등록, disconnect 시 제거.
/// DaemonStatusSink 가 전 연결 conn_tx 에 try_send 하기 위해 공유된다.
#[derive(Clone)]
pub struct ConnRegistry {
    inner: Arc<Mutex<HashMap<ConnId, mpsc::Sender<WsOutbound>>>>,
    next_id: Arc<AtomicU64>,
}

impl ConnRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    fn alloc_id(&self) -> ConnId {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn register(&self, id: ConnId, tx: mpsc::Sender<WsOutbound>) {
        self.inner
            .lock()
            .expect("conn registry poisoned")
            .insert(id, tx);
    }

    fn unregister(&self, id: ConnId) {
        self.inner
            .lock()
            .expect("conn registry poisoned")
            .remove(&id);
    }

    /// 전 연결에 Text 브로드캐스트(try_send). full 인 연결은 느린 것으로 보고 로그만.
    pub(crate) fn broadcast_text(&self, text: String) {
        let conns: Vec<(ConnId, mpsc::Sender<WsOutbound>)> = {
            let guard = self.inner.lock().expect("conn registry poisoned");
            guard.iter().map(|(id, tx)| (*id, tx.clone())).collect()
        };
        for (id, tx) in conns {
            // try_send 만 — StatusSink 는 pump/manager 스레드(sync)에서 불릴 수 있어 block 금지.
            if let Err(e) = tx.try_send(WsOutbound::Text(text.clone())) {
                tracing::warn!(
                    conn = id,
                    "status 브로드캐스트 try_send 실패(느린 소비자): {e}"
                );
            }
        }
    }
}

impl Default for ConnRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── DaemonStatusSink(global) ─────────────────────────────────────────────────────

/// AgentManager 에 주입되는 전역 StatusSink. status_changed/agent_list_updated/restore_result
/// 를 AgentEvent JSON 으로 직렬화해 레지스트리의 모든 conn_tx 에 try_send(Text) 한다.
/// (LogStatusSink 대체 — build_manager 가 이걸 주입.)
///
/// ★호출 컨텍스트: pump/manager 의 동기 스레드★ → 절대 block 금지. broadcast_text 가 try_send 만 쓴다.
pub struct DaemonStatusSink {
    registry: ConnRegistry,
}

impl DaemonStatusSink {
    pub fn new(registry: ConnRegistry) -> Self {
        Self { registry }
    }
}

impl StatusSink for DaemonStatusSink {
    fn status_changed(&self, id: AgentId, status: CoreStatus, epoch: u32) {
        let ev = AgentEvent::StatusChanged {
            agent_id: id,
            status: core_status_to_wire(status),
            epoch,
        };
        if let Some(text) = event_json(&ev) {
            self.registry.broadcast_text(text);
        }
    }

    fn agent_list_updated(&self, agents: Vec<CoreAgentInfo>) {
        let ev = AgentEvent::AgentListUpdated {
            agents: core_agents_to_wire(agents),
        };
        if let Some(text) = event_json(&ev) {
            self.registry.broadcast_text(text);
        }
    }

    fn restore_result(&self, report: CoreRestoreReport) {
        let ev = AgentEvent::RestoreResult {
            report: core_report_to_wire(report),
        };
        if let Some(text) = event_json(&ev) {
            self.registry.broadcast_text(text);
        }
    }
}

// ── WsOutputSink(연결당 출력 sink, pump 스레드에서 호출) ───────────────────────────

/// 한 연결의 한 에이전트 구독에 대응하는 OutputSink. pump 스레드가 `send` 를 호출한다.
/// frame 을 codec binary 로 인코딩해 conn_tx 에 **try_send 만**(block 금지) 한다.
/// 큐가 full/closed 면 SinkError 반환(코어가 dead-sink 로 제거) + out-of-band close 신호.
pub struct WsOutputSink {
    conn_tx: mpsc::Sender<WsOutbound>,
    /// 큐 밖 종료 신호. full 감지 시 notify_one — write_task 가 큐가 막혀도 깨어 닫는다.
    /// ★pump 스레드(sync)에서 notify_one 호출 OK — Notify 는 sync-safe.
    close_signal: Arc<Notify>,
    /// replay 구간 중 try_send 실패(frame drop)가 한 번이라도 있었는지.
    /// handle_subscribe 가 ReplayComplete 직전 검사해 SubscribeAck.truncated 를 사후 보정한다.
    /// 평소(라이브)엔 코어가 dead-sink 로 제거하므로 의미가 없고, replay 구간 정확성에만 쓴다.
    replay_dropped: Arc<AtomicBool>,
    sink_id: SinkId,
}

impl WsOutputSink {
    pub(crate) fn new(conn_tx: mpsc::Sender<WsOutbound>, close_signal: Arc<Notify>) -> Self {
        Self {
            conn_tx,
            close_signal,
            replay_dropped: Arc::new(AtomicBool::new(false)),
            sink_id: uuid::Uuid::new_v4(),
        }
    }

    /// replay 구간 동안 frame 이 drop 됐는지 사후 검사용 핸들(handle_subscribe 가 공유 보관).
    pub(crate) fn replay_dropped_flag(&self) -> Arc<AtomicBool> {
        self.replay_dropped.clone()
    }
}

// ── WsOutboundSink(연결당 control sink, ConnectionCore.dispatch 의 응답 경로) ──────────
//
// ConnectionCore 의 `OutboundSink` 를 WS 로 구현한다. dispatch 가 enqueue 하는 Outbound 를
// WsOutbound 로 변환해 conn_tx(단일 writer 큐)에 넣는다. 인코딩(AgentEvent→JSON text)은 이
// 어댑터가 소유한다(코어는 모름 — ADR-0003 정합).
//
// ★FIFO(R1)★: dispatch 의 control(Ack/SubscribeAck/ReplayComplete/Error 등)과 코어 output
// 평면(WsOutputSink 의 binary frame)이 같은 conn_tx 단일 writer 로 합류하므로, dispatch 가
// SubscribeAck 를 replay binary 보다 먼저 enqueue 하면 순서가 보존된다.
//
// ★R6 close_signal(out-of-band)★: 큐 포화 시 SinkError 를 반환하고, 동시에 close_signal 을
// notify 해 write_task 가 큐가 막혀도 깨어 닫게 한다(WS-특정 처리 — 어댑터에 잔류). enqueue 의
// `.await` 가 불가능한 sync trait 이므로 try_send 만 쓴다(control 도 큐 여유분으로 보통 성공).
pub struct WsOutboundSink {
    conn_tx: mpsc::Sender<WsOutbound>,
    close_signal: Arc<Notify>,
}

impl WsOutboundSink {
    pub(crate) fn new(conn_tx: mpsc::Sender<WsOutbound>, close_signal: Arc<Notify>) -> Self {
        Self {
            conn_tx,
            close_signal,
        }
    }
}

impl CoreOutboundSink for WsOutboundSink {
    fn enqueue(&self, out: Outbound) -> Result<(), CoreSinkError> {
        let msg = match out {
            // control 이벤트 — JSON text 로 인코딩(어댑터 소유). 직렬화 실패는 drop(기존 event_json 동작).
            Outbound::Event(ev) => match event_json(&ev) {
                Some(text) => WsOutbound::Text(text),
                None => return Ok(()), // 직렬화 실패는 무시(기존 `let _ = ...` 동작과 동일)
            },
            Outbound::Binary(b) => WsOutbound::Binary(b),
            Outbound::Close(reason) => WsOutbound::Close(reason),
        };
        match self.conn_tx.try_send(msg) {
            Ok(()) => Ok(()),
            Err(_) => {
                // 큐 포화/닫힘 — out-of-band close 신호로 write_task 를 깨운다(R6, WS-특정 잔류).
                self.close_signal.notify_one();
                Err(CoreSinkError)
            }
        }
    }

    fn make_output_sink(&self) -> (Arc<dyn OutputSink>, Arc<AtomicBool>) {
        // handle_subscribe 가 코어 subscribe_from 에 넘길 output 평면 sink. 같은 conn_tx/close_signal
        // 을 공유해 control(이 sink)과 output(WsOutputSink)이 한 단일 writer 큐로 합류한다(FIFO).
        // ★Stage 2 generic★: 반환을 Arc<dyn OutputSink> trait object 로(carrier-중립). replay_dropped
        //   플래그를 함께 돌려 handle_subscribe 가 truncated 사후 보정에 쓰게 한다.
        let sink = Arc::new(WsOutputSink::new(
            self.conn_tx.clone(),
            self.close_signal.clone(),
        ));
        let flag = sink.replay_dropped_flag();
        (sink, flag)
    }
}

impl OutputSink for WsOutputSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        // ★S15 B5/B7 payload 분기(ADR-0045)★: 콘솔 바이트는 tag0 terminal frame, 구조화 이벤트는 tag1
        //   structured frame 으로 인코딩한다. sink 가 wire 인코딩을 소유(코어는 wire 모름, ADR-0003) —
        //   Bytes 는 raw payload 를, Event 는 core `OutputEvent` → wire `StructuredEvent`(daemon adapter)
        //   → JSON payload 를 헤더에 실어 보낸다.
        //   ★현 배선 상태★: 구조화 이벤트 생산자(B3 decoder→pump 배선)는 아직 미배선이라 런타임엔 Bytes 만
        //   흐른다 — Event arm 은 B7 단위테스트(합성 OutputEvent)로만 도달·검증된다(정상).
        let buf = match frame.payload {
            OutputPayload::Bytes(b) => {
                encode_terminal_frame(frame.agent_id, frame.epoch, frame.seq, b)
            }
            // ★tag1 인코딩(B7)★: core OutputEvent → wire StructuredEvent(adapter) → JSON payload →
            //   tag1 structured frame. codec 은 payload 스키마 무지(opaque) — 직렬화 형식(JSON)·이벤트
            //   타입은 여기(daemon)가 소유한다(ADR-0045 self-describing).
            OutputPayload::Event(ev) => {
                // (1) core→wire 변환. TerminalBytes 가 여기 오면(정상 경로상 tag0 로 갈려 안 옴 — 상류
                //     배선 버그) 매핑 불가(None) → debug 는 조기 발견, release 는 warn 후 drop(연결 유지).
                let wire = match output_event_to_wire(ev) {
                    Some(w) => w,
                    None => {
                        debug_assert!(
                            false,
                            "TerminalBytes(tag0 전용)가 Event(tag1) arm 에 도달 — 상류 payload 분기 버그"
                        );
                        tracing::warn!(
                            agent = %frame.agent_id,
                            "tag1 인코딩 불가(TerminalBytes 가 Event arm 도달) — drop"
                        );
                        return Ok(());
                    }
                };
                // (2) JSON 직렬화. 실패는 거의 불가능(문자열/숫자 필드뿐)하나, 나면 이 frame 만 warn 후
                //     drop 한다(SinkError 로 연결을 죽이지 않음 — 직렬화 실패는 슬로우 소비자와 무관한
                //     데이터 문제고, control event_json 실패 처리와 동일 관례).
                let payload = match serde_json::to_vec(&wire) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!(
                            agent = %frame.agent_id,
                            "StructuredEvent 직렬화 실패 — drop: {e}"
                        );
                        return Ok(());
                    }
                };
                // (3) tag1 frame(헤더+payload). 헤더 레이아웃은 tag0 과 동일, tag=1(codec, ADR-0045).
                encode_structured_frame(frame.agent_id, frame.epoch, frame.seq, &payload)
            }
        };
        // ★pump 스레드 — try_send 만(절대 block 금지). full/closed = 느린 소비자 → 코어가 이 sink 제거.
        match self.conn_tx.try_send(WsOutbound::Binary(buf)) {
            Ok(()) => Ok(()),
            Err(_) => {
                // frame 이 drop 됐음을 기록(replay 구간 truncated 사후 보정용).
                self.replay_dropped.store(true, Ordering::Release);
                // ★out-of-band 종료 신호★: 큐가 full 이라 WsOutbound::Close try_send 는 실패할 수
                //   있으나, Notify 는 큐와 무관하게 write_task 를 깨운다(좀비 연결 방지).
                self.close_signal.notify_one();
                Err(SinkError)
            }
        }
    }

    fn sink_id(&self) -> SinkId {
        self.sink_id
    }
}

// ── Origin allowlist 콜백 ─────────────────────────────────────────────────────────

/// upgrade 콜백 — Origin 헤더 검사. 없으면 허용(네이티브/하네스), 있고 allowlist 밖이면 거부.
struct OriginCheck;

impl Callback for OriginCheck {
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        match request.headers().get("origin") {
            None => {
                // Origin 없음 = 네이티브/하네스 클라이언트. 토큰이 주 방어이므로 허용.
                tracing::debug!("WS upgrade: Origin 없음 — 허용(토큰 검증으로 방어)");
                Ok(response)
            }
            Some(value) => {
                let origin = value.to_str().unwrap_or("");
                if ALLOWED_ORIGINS.contains(&origin) {
                    tracing::debug!(origin, "WS upgrade: Origin 허용");
                    Ok(response)
                } else {
                    // ★TODO(실측)★: 실제 Tauri WebView2/모바일이 보내는 Origin 문자열을 실측해
                    // allowlist 를 확정할 것(설계값 기준). 불일치 = 거부.
                    tracing::warn!(origin, "WS upgrade: Origin 불일치 — 거부");
                    let mut resp = ErrorResponse::new(Some("origin not allowed".into()));
                    *resp.status_mut() = StatusCode::FORBIDDEN;
                    Err(resp)
                }
            }
        }
    }
}

// ── 상수시간 토큰 비교 ──────────────────────────────────────────────────────────

/// 토큰 상수시간 비교 — 길이 먼저(다르면 즉시 false), 같으면 바이트 XOR 누적으로
/// timing 부채널을 줄인다. 길이 노출은 토큰 길이가 고정(hex 64자)이라 무해.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ── 연결 핸들러 ────────────────────────────────────────────────────────────────

/// 연결 1개의 전 수명을 처리한다. accept 된 raw TCP stream 을 받아:
/// WS 업그레이드 → auth → Hello/list push → read/write task → cleanup.
///
/// `expected_token` 은 daemon.json 의 토큰. `shutdown_tx` 는 StopDaemon 수신 시 main 종료를 트리거.
#[allow(clippy::too_many_arguments)]
pub async fn handle_connection(
    stream: TcpStream,
    peer: std::net::SocketAddr,
    manager: Arc<AgentManager>,
    registry: ConnRegistry,
    multiview: MultiViewState,
    expected_token: Arc<String>,
    shutdown_tx: watch::Sender<bool>,
    keepalive: KeepaliveConfig,
) {
    // 1) WS 업그레이드 + Origin 검사.
    let mut ws = match tokio_tungstenite::accept_hdr_async(stream, OriginCheck).await {
        Ok(ws) => ws,
        Err(e) => {
            tracing::warn!(%peer, "WS 업그레이드 실패(또는 Origin 거부): {e}");
            return;
        }
    };

    // 2) 첫 frame(1초 내) → Auth 파싱 + 토큰 상수시간 비교 + 버전 검사.
    match tokio::time::timeout(AUTH_TIMEOUT, ws.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => {
            match serde_json::from_str::<AgentCommand>(&text) {
                Ok(AgentCommand::Auth {
                    token,
                    protocol_version,
                }) => {
                    // 토큰 비교(상수시간). 보안: 토큰 값은 로그 금지.
                    if !constant_time_eq(&token, expected_token.as_str()) {
                        tracing::warn!(%peer, "auth 실패: 토큰 불일치 — close");
                        let _ = send_error_and_close(&mut ws, "auth failed").await;
                        return;
                    }
                    if protocol_version != PROTOCOL_VERSION {
                        tracing::warn!(
                            %peer,
                            client = protocol_version,
                            server = PROTOCOL_VERSION,
                            "auth 실패: protocol_version 불일치 — close"
                        );
                        let _ = send_error_and_close(
                            &mut ws,
                            &format!(
                                "protocol_version mismatch: client {protocol_version} != server {PROTOCOL_VERSION}"
                            ),
                        )
                        .await;
                        return;
                    }
                    tracing::info!(%peer, "auth 성공");
                }
                Ok(_) => {
                    tracing::warn!(%peer, "첫 frame 이 Auth 가 아님 — close");
                    let _ = send_error_and_close(&mut ws, "expected Auth as first frame").await;
                    return;
                }
                Err(e) => {
                    tracing::warn!(%peer, "첫 frame 파싱 실패: {e} — close");
                    let _ = send_error_and_close(&mut ws, "invalid first frame").await;
                    return;
                }
            }
        }
        Ok(Some(Ok(_))) => {
            tracing::warn!(%peer, "첫 frame 이 Text 가 아님 — close");
            let _ = send_error_and_close(&mut ws, "expected Auth text frame").await;
            return;
        }
        Ok(Some(Err(e))) => {
            tracing::warn!(%peer, "첫 frame 수신 오류: {e} — close");
            return;
        }
        Ok(None) => {
            tracing::warn!(%peer, "auth 전에 연결 종료");
            return;
        }
        Err(_) => {
            tracing::warn!(%peer, "auth 타임아웃(1s) — close");
            let _ = send_error_and_close(&mut ws, "auth timeout").await;
            return;
        }
    }

    // 3) conn_tx/rx 생성 + close_signal + 레지스트리 등록 + split.
    let (conn_tx, conn_rx) = mpsc::channel::<WsOutbound>(CONN_TX_CAP);
    // ★out-of-band 종료 신호★: 큐 포화로 WsOutbound::Close 마저 못 들어갈 때 write_task 를 깨운다.
    let close_signal = Arc::new(Notify::new());
    let conn_id = registry.alloc_id();
    registry.register(conn_id, conn_tx.clone());
    tracing::info!(%peer, conn = conn_id, "연결 인증 완료 — 등록");

    let (sink_half, stream_half) = ws.split();

    // 3b) ConnectionCore(transport-중립 dispatch) 배선. 연결당 1개 — manager/multiview/registry/
    //     shutdown_tx 는 전 연결이 공유하나, dispatch 호출 경로를 캡슐화하려고 연결마다 묶는다.
    //     read_task 가 이걸 통해 명령을 처리하고, cleanup 도 core 의 manager/multiview/registry 를 쓴다.
    let core = Arc::new(ConnectionCore::new(
        manager.clone(),
        multiview.clone(),
        registry.clone(),
        shutdown_tx,
    ));

    // 4) 연결 직후 Hello + 현재 목록 push(단일 writer 큐 경유 — 이후 모든 출력과 FIFO 정렬).
    if let Some(text) = event_json(&hello_event(env!("CARGO_PKG_VERSION").into())) {
        let _ = conn_tx.send(WsOutbound::Text(text)).await;
    }
    if let Some(text) = event_json(&agent_list_event(&manager)) {
        let _ = conn_tx.send(WsOutbound::Text(text)).await;
    }

    // 5) 이 연결의 per-conn 수명 상태(subs/owned_viewports + conn_id). read_task 와 cleanup 이 공유.
    let session = Arc::new(ConnectionSession::new(conn_id));

    // ── keepalive 공유 시계(A) ──────────────────────────────────────────────────────
    // base = 연결 시작 시각(tokio Instant). last_recv = base 기준 경과 ms(AtomicU64).
    // read_task 가 클라로부터 무언가(Pong 포함) 받을 때마다 갱신하고, write_task 의 ping arm 이
    // base.elapsed() - last_recv 로 idle 경과를 계산해 idle_timeout 초과 시 close_signal 발동.
    let keepalive_base = tokio::time::Instant::now();
    let last_recv = Arc::new(AtomicU64::new(0));

    // read_task: stream_half 에서 명령을 읽어 ConnectionCore.dispatch 로 처리. 응답은 WsOutboundSink
    //   (control)와 WsOutputSink(output, handle_subscribe 가 생성)가 conn_tx 로 큐잉한다.
    //   close_signal 은 두 sink 에 공유(full 시 write_task 깨우기 — R6).
    let mut read_handle = tokio::spawn(read_task(
        stream_half,
        conn_tx.clone(),
        core.clone(),
        session.clone(),
        conn_id,
        close_signal.clone(),
        keepalive_base,
        last_recv.clone(),
    ));

    // write_task: conn_rx 에서 받은 WsOutbound 를 sink_half 로 순서대로 write(단일 writer).
    //   close_signal 발동 시 큐가 막혀 있어도 깨어 닫는다(좀비 방지). keepalive Ping 도 여기서 송신.
    let mut write_handle = tokio::spawn(write_task(
        sink_half,
        conn_rx,
        conn_id,
        close_signal,
        keepalive,
        keepalive_base,
        last_recv,
    ));

    // 6) 하나라도 끝나면 cleanup. ★살아남은 쪽을 명시적으로 abort★ — JoinHandle 을 그냥 drop 하면
    //    task 가 detach 되어 계속 돈다(WS half 를 붙든 채 좀비). 그래서 &mut 로 select 해 핸들을
    //    소비하지 않고, 진 쪽을 abort 한다(연결의 read/write 가 함께 끝나게).
    tokio::select! {
        _ = &mut read_handle => {
            tracing::debug!(conn = conn_id, "read_task 종료 → write_task abort + cleanup");
            write_handle.abort();
        }
        _ = &mut write_handle => {
            tracing::debug!(conn = conn_id, "write_task 종료 → read_task abort + cleanup");
            read_handle.abort();
        }
    }

    // ── cleanup(누수 방지 — 리뷰 필수) ──────────────────────────────────────────
    // 이 연결이 등록한 모든 (agent_id, sink_id) 를 manager 에서 unsubscribe + 레지스트리 제거.
    // 안 하면 죽은 conn_tx 로 영원히 try_send 하는 좀비 sink 가 코어 subscribers 에 남는다
    // (코어가 try_send 실패로 결국 제거하긴 하나, 다음 emit 까지 잔존 — 명시적으로 끊는다).
    let leftovers: Vec<(AgentId, SinkId)> = {
        let guard = session.subs.lock().expect("subs poisoned");
        guard.iter().map(|(a, s)| (*a, *s)).collect()
    };
    for (agent_id, sink_id) in leftovers {
        let _ = manager.unsubscribe(agent_id, sink_id);
    }

    // ── 멀티뷰어 cleanup ───────────────────────────────────────────────────────
    // (a) viewport 재협상: 끊긴 연결의 viewport 들을 맵에서 빼고, 영향받은 agent 를 남은 뷰어 기준
    //     smallest 로 다시 resize 한다(tmux detach 후 잔여 클라 기준으로 다시 키우는 것과 동일).
    //     ★lock 순서★: remove_conn_viewports 가 multiview lock 안에서 협상값만 계산해 반환한 뒤
    //     lock 을 푼 상태에서 manager.resize 를 부른다(lock 보유 중 코어 호출 금지).
    let owned: Vec<(AgentId, String)> = {
        let g = session
            .owned_viewports
            .lock()
            .expect("owned_viewports poisoned");
        g.clone()
    };
    if !owned.is_empty() {
        for (agent_id, negotiated) in core.multiview().remove_conn_viewports(&owned) {
            if let Some((cols, rows)) = negotiated {
                // 남은 뷰어가 있으면 그 smallest 로 복귀. 없으면(None) 그대로 둔다(마지막 크기 유지).
                let _ = manager.resize(agent_id, cols, rows);
            }
        }
    }
    // (b) 입력 lease 자동 해제: 보유자가 끊기면 다른 뷰어가 영영 막히면 안 된다(좀비 lock 방지).
    //     해제된 agent 는 이제 lease 가 비었으니 InputLeaseChanged{held:false} 를 전 연결에 통보.
    for agent_id in core.multiview().release_all_for_conn(conn_id) {
        crate::connection_core::broadcast_lease_changed(&registry, agent_id, false);
    }

    registry.unregister(conn_id);
    tracing::info!(%peer, conn = conn_id, "연결 종료 — cleanup 완료");
}

/// auth 실패 시 Error + close 를 직접(레지스트리 등록 전이라 conn_tx 없음) 보낸다.
async fn send_error_and_close(
    ws: &mut tokio_tungstenite::WebSocketStream<TcpStream>,
    message: &str,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    if let Some(text) = event_json(&AgentEvent::Error {
        request_id: None,
        message: message.to_string(),
    }) {
        ws.send(Message::Text(text.into())).await?;
    }
    ws.close(None).await
}

// ── write_task(단일 writer) ───────────────────────────────────────────────────

type SinkHalf =
    futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, Message>;

/// conn_rx 에서 받은 출력을 sink_half 로 순서대로 write. 이게 이 연결의 유일한 writer.
/// 종료 트리거 3가지: (1) conn_rx 큐의 WsOutbound::Close, (2) sink send 실패, (3) close_signal.
///
/// ★out-of-band 종료(M1 핵심)★: conn_tx 가 full 이면 WsOutbound::Close 마저 큐에 못 들어가
/// 좀비 연결이 된다. 그래서 `tokio::select!` 로 conn_rx.recv() 와 close_signal.notified() 를
/// 동시에 대기한다. WsOutputSink 가 full 을 만나 `close_signal.notify_one()` 하면, 큐가
/// 가득 차 있어도 이 select 가 깨어 sink_half.close() 후 break → cleanup 으로 이어진다.
#[allow(clippy::too_many_arguments)]
async fn write_task(
    mut sink_half: SinkHalf,
    mut conn_rx: mpsc::Receiver<WsOutbound>,
    conn_id: ConnId,
    close_signal: Arc<Notify>,
    keepalive: KeepaliveConfig,
    keepalive_base: tokio::time::Instant,
    last_recv: Arc<AtomicU64>,
) {
    // ★keepalive Ping 주기(A)★: ping_interval 마다 능동 Ping 을 보낸다(half-open 감지).
    //   tick 마다 마지막 수신 후 경과가 idle_timeout 초과면 close_signal 로 이 연결을 끊는다.
    let mut ping_tick = tokio::time::interval(keepalive.ping_interval);
    // 첫 tick 즉발 방지(연결 직후 바로 Ping 쏘지 않게) — 정상 첫 주기부터.
    ping_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            // 큐 밖 종료 신호 — full 로 큐가 막혀 있어도 여기로 깨어 닫는다.
            _ = close_signal.notified() => {
                tracing::info!(conn = conn_id, "write_task: close_signal(슬로우 소비자) — 종료");
                let _ = sink_half.close().await;
                break;
            }
            // keepalive: 주기적 능동 Ping + idle 판정.
            _ = ping_tick.tick() => {
                // idle 판정: 마지막 클라 수신(Pong 또는 임의 메시지) 이후 경과.
                let now_ms = keepalive_base.elapsed().as_millis() as u64;
                let last_ms = last_recv.load(Ordering::Acquire);
                let idle = Duration::from_millis(now_ms.saturating_sub(last_ms));
                if idle >= keepalive.idle_timeout {
                    // half-open 추정 — Pong 미응답이 idle_timeout 넘김. 이 연결을 닫는다.
                    tracing::info!(
                        conn = conn_id,
                        idle_ms = idle.as_millis() as u64,
                        "write_task: keepalive idle_timeout 초과(half-open 추정) — 종료"
                    );
                    let _ = sink_half.close().await;
                    break;
                }
                // 능동 Ping 송신. 실패(소켓 닫힘)면 종료.
                if let Err(e) = sink_half.send(Message::Ping(Vec::new().into())).await {
                    tracing::debug!(conn = conn_id, "write_task keepalive Ping 송신 실패 — 종료: {e}");
                    break;
                }
            }
            recv = conn_rx.recv() => {
                let Some(out) = recv else {
                    // 모든 conn_tx drop — 정상 종료.
                    break;
                };
                let msg = match out {
                    WsOutbound::Text(s) => Message::Text(s.into()),
                    WsOutbound::Binary(b) => Message::Binary(b.into()),
                    WsOutbound::Close(reason) => {
                        tracing::info!(conn = conn_id, %reason, "write_task: close 신호 — 종료");
                        let _ = sink_half.close().await;
                        break;
                    }
                };
                if let Err(e) = sink_half.send(msg).await {
                    tracing::debug!(conn = conn_id, "write_task send 실패 — 종료: {e}");
                    break;
                }
            }
        }
    }
    tracing::debug!(conn = conn_id, "write_task 루프 종료");
}

// ── read_task ────────────────────────────────────────────────────────────────

type StreamHalf = futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<TcpStream>>;

/// stream_half 에서 명령 frame 을 읽어 ConnectionCore.dispatch 로 처리. 응답은 WsOutboundSink
/// (control)를 통해 conn_tx 로 큐잉된다(직접 write 안 함).
///
/// ★Stage 1 배선★: 옛 read_task 는 dispatch 자유함수를 직접 불렀다. 이제 transport-중립
/// ConnectionCore 가 dispatch 를 소유하고, read_task 는 WS 프레임→AgentCommand 파싱과
/// WsOutboundSink(control 인코딩)만 담당한다(carrier 경계). DispatchFlow::Close 면 루프 탈출
/// (옛 dispatch 의 bool true 와 동일 동작 — StopDaemon).
#[allow(clippy::too_many_arguments)]
async fn read_task(
    mut stream_half: StreamHalf,
    conn_tx: mpsc::Sender<WsOutbound>,
    core: Arc<ConnectionCore>,
    session: Arc<ConnectionSession>,
    conn_id: ConnId,
    close_signal: Arc<Notify>,
    keepalive_base: tokio::time::Instant,
    last_recv: Arc<AtomicU64>,
) {
    use crate::connection_core::DispatchFlow;

    // 이 연결의 control 응답 sink — dispatch 의 Ack/Error/SubscribeAck/ReplayComplete 등이 여기로.
    // output 평면(replay/live binary)은 handle_subscribe 가 make_output_sink 로 별도 생성하나,
    // 같은 conn_tx/close_signal 을 공유해 한 단일 writer 큐로 합류한다(FIFO 보존).
    let ws_sink = WsOutboundSink::new(conn_tx.clone(), close_signal.clone());

    while let Some(item) = stream_half.next().await {
        let msg = match item {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!(conn = conn_id, "read_task 수신 오류 — 종료: {e}");
                break;
            }
        };
        // ★keepalive(A)★: 클라로부터 무언가 받았다 = 연결이 살아있다는 증거. Pong 포함 모든
        //   메시지에서 마지막 수신 시각을 갱신한다(write_task 의 idle 판정 분모). tungstenite 는
        //   Pong 을 Message::Pong 으로 올려주므로 능동 Ping 의 응답도 여기서 잡힌다.
        last_recv.store(
            keepalive_base.elapsed().as_millis() as u64,
            Ordering::Release,
        );
        match msg {
            Message::Text(text) => {
                match serde_json::from_str::<AgentCommand>(&text) {
                    Ok(cmd) => {
                        if core.dispatch(cmd, &session, &ws_sink).await == DispatchFlow::Close {
                            // dispatch 가 연결 종료를 요청(StopDaemon 등) — 루프 탈출.
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(conn = conn_id, "명령 파싱 실패: {e}");
                        // 옛 send_error(conn_tx, ..) 와 동일: Error 이벤트를 conn_tx 로 큐잉.
                        let _ = ws_sink.enqueue(Outbound::event(AgentEvent::Error {
                            request_id: None,
                            message: format!("invalid command: {e}"),
                        }));
                    }
                }
            }
            Message::Binary(_) => {
                // 클라→데몬 binary 는 프로토콜에 없음 — 오류로 보고 종료.
                tracing::warn!(conn = conn_id, "예상치 못한 binary frame — close");
                let _ = ws_sink.enqueue(Outbound::event(AgentEvent::Error {
                    request_id: None,
                    message: "unexpected binary frame".into(),
                }));
                let _ = conn_tx
                    .send(WsOutbound::Close("protocol error".into()))
                    .await;
                break;
            }
            // Ping/Pong 은 tungstenite 가 자동 응답(write_task 가 아닌 내부). 여기선 무시.
            Message::Ping(_) | Message::Pong(_) => {}
            Message::Close(_) => {
                tracing::debug!(conn = conn_id, "Close frame 수신 — 종료");
                break;
            }
            Message::Frame(_) => {}
        }
    }
    tracing::debug!(conn = conn_id, "read_task 루프 종료");
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 1. Auth 직렬화 roundtrip ────────────────────────────────────────────
    #[test]
    fn auth_command_roundtrip() {
        let cmd = AgentCommand::Auth {
            token: "deadbeef".repeat(8), // 64자
            protocol_version: PROTOCOL_VERSION,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        match back {
            AgentCommand::Auth {
                token,
                protocol_version,
            } => {
                assert_eq!(token, "deadbeef".repeat(8));
                assert_eq!(protocol_version, PROTOCOL_VERSION);
            }
            _ => panic!("Auth 가 아님"),
        }
    }

    // (kind_to_action 매핑 테스트는 connection_core.rs 로 이동 — 함수가 거기 있음.)

    // ── 2. 토큰 상수시간 비교 정확성 ──────────────────────────────────────────
    #[test]
    fn constant_time_eq_correctness() {
        let a = "a".repeat(64);
        assert!(constant_time_eq(&a, &"a".repeat(64)), "동일 토큰은 true");
        assert!(!constant_time_eq(&a, &"b".repeat(64)), "다른 토큰은 false");
        // 길이 다르면 즉시 false.
        assert!(!constant_time_eq(&a, &"a".repeat(63)));
        assert!(!constant_time_eq(&a, &"a".repeat(65)));
        // 한 바이트만 달라도 false.
        let mut almost = "a".repeat(64);
        almost.replace_range(63..64, "b");
        assert!(!constant_time_eq(&a, &almost));
        // 빈 문자열 동일.
        assert!(constant_time_eq("", ""));
    }

    // ── 3. WsOutbound 매핑(Text/Binary/Close → Message) ──────────────────────
    // write_task 의 변환 로직과 동일한 매핑을 직접 검증(실제 WS 없이).
    #[test]
    fn ws_outbound_maps_to_message() {
        let t = WsOutbound::Text("hi".into());
        let b = WsOutbound::Binary(vec![1, 2, 3]);
        let c = WsOutbound::Close("bye".into());

        let to_msg = |o: WsOutbound| -> Message {
            match o {
                WsOutbound::Text(s) => Message::Text(s.into()),
                WsOutbound::Binary(b) => Message::Binary(b.into()),
                WsOutbound::Close(_) => Message::Close(None),
            }
        };
        assert!(matches!(to_msg(t), Message::Text(_)));
        assert!(matches!(to_msg(b), Message::Binary(_)));
        assert!(matches!(to_msg(c), Message::Close(_)));
    }

    // ── 4. WsOutputSink 가 conn_tx 에 binary frame 을 try_send 하는지 ─────────
    #[tokio::test]
    async fn ws_output_sink_encodes_and_sends_binary() {
        let (tx, mut rx) = mpsc::channel::<WsOutbound>(8);
        let sink = WsOutputSink::new(tx, Arc::new(Notify::new()));
        let agent_id = uuid::Uuid::new_v4();
        let data = b"abc";
        let frame = OutputFrame {
            agent_id,
            epoch: 7,
            seq: 42,
            payload: OutputPayload::Bytes(data),
        };
        sink.send(frame).expect("send ok");

        match rx.recv().await.expect("one item") {
            WsOutbound::Binary(buf) => {
                // codec 으로 디코드해 헤더가 맞는지 확인.
                let decoded = engram_dashboard_protocol::decode_frame(&buf).expect("decode");
                assert_eq!(decoded.agent_id, agent_id);
                assert_eq!(decoded.epoch, 7);
                assert_eq!(decoded.seq, 42);
                assert_eq!(decoded.payload, b"abc");
            }
            _ => panic!("Binary 가 아님"),
        }
    }

    // ── 4b. (S15 B7) WsOutputSink 가 Event(구조화) payload 를 tag1 frame 으로 인코딩하는지 ──────
    //    합성 OutputEvent → send → conn_tx 의 Binary 를 decode_frame 으로 풀어 tag1·헤더 확인 후,
    //    payload 를 다시 wire StructuredEvent 로 serde 파싱해 필드가 보존됐는지 단언(ADR-0045 self-describing).
    #[tokio::test]
    async fn ws_output_sink_encodes_event_as_tag1_structured_frame() {
        use engram_dashboard_core::agent::types::OutputEvent as CoreOutputEvent;
        use engram_dashboard_protocol::{
            decode_frame, StructuredEvent as WireStructuredEvent, FRAME_TAG_STRUCTURED_EVENT,
        };

        let (tx, mut rx) = mpsc::channel::<WsOutbound>(8);
        let sink = WsOutputSink::new(tx, Arc::new(Notify::new()));
        let agent_id = uuid::Uuid::new_v4();
        // 합성 구조화 이벤트(B3 미배선이라 런타임 생산자 없음 — 여기선 직접 만들어 tag1 경로를 태운다).
        let ev = CoreOutputEvent::ToolCall {
            name: "read".into(),
            args_json: r#"{"path":"/x"}"#.into(),
            id: Some("call_1".into()),
            turn_id: Some("t9".into()),
            message_id: None,
        };
        let frame = OutputFrame {
            agent_id,
            epoch: 3,
            seq: 100,
            payload: OutputPayload::Event(&ev),
        };
        sink.send(frame).expect("Event send ok");

        match rx.recv().await.expect("one item") {
            WsOutbound::Binary(buf) => {
                let decoded = decode_frame(&buf).expect("decode");
                // tag=1(structured) + 헤더 필드 그대로.
                assert_eq!(decoded.tag, FRAME_TAG_STRUCTURED_EVENT, "tag1 이어야 함");
                assert_eq!(decoded.agent_id, agent_id);
                assert_eq!(decoded.epoch, 3);
                assert_eq!(decoded.seq, 100);
                // payload = JSON self-describing StructuredEvent. 파싱해 필드 보존 단언.
                let parsed: WireStructuredEvent =
                    serde_json::from_slice(decoded.payload).expect("payload JSON 파싱");
                assert_eq!(
                    parsed,
                    WireStructuredEvent::ToolCall {
                        name: "read".into(),
                        args_json: r#"{"path":"/x"}"#.into(),
                        id: Some("call_1".into()),
                        turn_id: Some("t9".into()),
                        message_id: None,
                    },
                    "tag1 payload 가 wire StructuredEvent 로 무손실 복원"
                );
            }
            other => panic!("Binary(tag1) 여야 함: {other:?}"),
        }
    }

    // ── 5. WsOutputSink full → SinkError + close_signal notify + replay_dropped ──
    #[tokio::test]
    async fn ws_output_sink_full_returns_error_and_notifies_close_signal() {
        // cap 1 채널을 가득 채운 뒤: send 가 Err 를 반환하고, 큐가 막혀 있어도 out-of-band
        // close_signal 이 발동(write_task 를 깨움)하며, replay_dropped 가 set 되는지.
        let (tx, mut rx) = mpsc::channel::<WsOutbound>(1);
        let close_signal = Arc::new(Notify::new());
        let sink = WsOutputSink::new(tx, close_signal.clone());
        let replay_dropped = sink.replay_dropped_flag();
        let agent_id = uuid::Uuid::new_v4();
        let frame = |seq: u64| OutputFrame {
            agent_id,
            epoch: 0,
            seq,
            payload: OutputPayload::Bytes(b"x"),
        };
        // 첫 send 성공(큐 1칸 채움).
        sink.send(frame(0)).expect("first ok");
        // 두 번째는 full → Err.
        assert!(sink.send(frame(1)).is_err(), "full 이면 SinkError");

        // ★out-of-band 종료 신호★: 큐가 full 이어도 close_signal 은 발동해야 한다.
        //   notified() 가 즉시 깨면 write_task 가 깨어 닫을 수 있다는 의미(M1 핵심 근거).
        tokio::time::timeout(Duration::from_millis(200), close_signal.notified())
            .await
            .expect("close_signal 이 full 에서도 발동해야 함");

        // replay 구간 사후 보정용 플래그도 set.
        assert!(
            replay_dropped.load(Ordering::Acquire),
            "drop 시 replay_dropped set"
        );

        // 큐 첫 항목은 Binary(첫 frame).
        assert!(matches!(rx.recv().await.unwrap(), WsOutbound::Binary(_)));
    }

    // ── 6. Subscribe 시 conn_tx 에 SubscribeAck → ReplayComplete 순서로 들어가는지 ──
    //    (mock manager 가 없어 실 AgentManager 의 비어있는 snapshot 경로로는 NotFound 가 나므로,
    //     여기선 control 메시지 순서 로직을 직접 재현해 검증한다. 실 manager subscribe 의 replay
    //     동기 전송은 output_core.rs 단위테스트가 이미 커버.)
    #[tokio::test]
    async fn subscribe_control_order_ack_then_complete() {
        use engram_dashboard_protocol::SubscribeAction;
        let (tx, mut rx) = mpsc::channel::<WsOutbound>(16);
        let agent_id = uuid::Uuid::new_v4();

        // handle_subscribe 가 보내는 control 순서를 직접 재현(SubscribeAck → [replay binary] → ReplayComplete).
        let ack = event_json(&AgentEvent::SubscribeAck {
            agent_id,
            action: SubscribeAction::Reset,
            current_epoch: 0,
            oldest_seq: 0,
            latest_seq: 0,
            replay_from: 0,
            truncated: false,
        })
        .unwrap();
        tx.send(WsOutbound::Text(ack)).await.unwrap();
        // 가상의 replay binary 1건.
        tx.send(WsOutbound::Binary(encode_terminal_frame(
            agent_id, 0, 0, b"r",
        )))
        .await
        .unwrap();
        let complete = event_json(&AgentEvent::ReplayComplete { agent_id, epoch: 0 }).unwrap();
        tx.send(WsOutbound::Text(complete)).await.unwrap();

        // 순서 검증: Text(SubscribeAck) → Binary(replay) → Text(ReplayComplete).
        let first = rx.recv().await.unwrap();
        let second = rx.recv().await.unwrap();
        let third = rx.recv().await.unwrap();

        match first {
            WsOutbound::Text(s) => assert!(s.contains("SubscribeAck")),
            _ => panic!("1번째는 SubscribeAck Text 여야 함"),
        }
        assert!(
            matches!(second, WsOutbound::Binary(_)),
            "2번째는 replay binary"
        );
        match third {
            WsOutbound::Text(s) => assert!(s.contains("ReplayComplete")),
            _ => panic!("3번째는 ReplayComplete Text 여야 함"),
        }
    }

    // ── 10. (적용4-1) OriginCheck::on_request 분기 — 무방비 였던 거부/허용 분기 검증 ──────
    //    순수 헤더 검사라 in-process 서버 불필요. Request 를 직접 만들어 콜백을 호출한다.
    fn run_origin_check(origin: Option<&str>) -> Result<(), ()> {
        use tokio_tungstenite::tungstenite::http::Request as HttpRequest;
        let mut builder = HttpRequest::builder().uri("/");
        if let Some(o) = origin {
            builder = builder.header("origin", o);
        }
        let request = builder.body(()).unwrap();
        // Response 는 콜백이 통과시키는 더미. on_request 는 self 를 소비한다.
        let response = tokio_tungstenite::tungstenite::http::Response::builder()
            .body(())
            .unwrap();
        OriginCheck
            .on_request(&request, response)
            .map(|_| ())
            .map_err(|_| ())
    }

    #[test]
    fn origin_check_allows_listed_origin() {
        // allowlist 에 있는 Origin → 허용.
        assert!(run_origin_check(Some("tauri://localhost")).is_ok());
        assert!(run_origin_check(Some("http://localhost:1420")).is_ok());
    }

    #[test]
    fn origin_check_rejects_unlisted_origin() {
        // allowlist 밖 Origin → 거부(mutation 으로 무방비 였던 분기).
        assert!(run_origin_check(Some("http://evil.example.com")).is_err());
    }

    #[test]
    fn origin_check_allows_missing_origin() {
        // Origin 헤더 없음 → 현 정책상 허용(네이티브/하네스, 토큰이 주 방어).
        assert!(run_origin_check(None).is_ok());
    }
}
