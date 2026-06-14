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

use engram_dashboard_core::pty::manager::AgentManager;
use engram_dashboard_core::pty::profile::RestoreReport as CoreRestoreReport;
use engram_dashboard_core::pty::profile::SpawnMode;
use engram_dashboard_core::pty::types::{
    AgentId, AgentInfo as CoreAgentInfo, AgentStatus as CoreStatus, OutputFrame, OutputSink,
    ReplayKind, SinkError, SinkId, StatusSink, SubscribeOutcome,
};

use engram_dashboard_protocol::{
    encode_terminal_frame, AgentCommand, AgentEvent, AgentInfo as WireAgentInfo, RestoreReport,
    SubscribeAction, PROTOCOL_VERSION,
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
    fn broadcast_text(&self, text: String) {
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

// ── 타입 변환(core ↔ wire) ─────────────────────────────────────────────────────
//
// core::AgentInfo/RestoreReport 는 Serialize 전용 미러, protocol 타입은 Serialize+Deserialize.
// 둘은 글자 그대로 동일한 JSON 형태(domain.rs 주석 "글자 그대로 일치")라 serde_json roundtrip 으로
// 안전히 변환한다. AgentEvent(wire 타입 임베드)를 직렬화하려면 wire 타입이 필요하다.

fn core_agents_to_wire(agents: Vec<CoreAgentInfo>) -> Vec<WireAgentInfo> {
    agents
        .into_iter()
        .filter_map(|a| {
            // 변환 실패 시 어느 agent 인지 알 수 있게 agent_id 를 로그에 포함(M3).
            let agent_id = a.id;
            match serde_json::to_value(&a).and_then(serde_json::from_value) {
                Ok(w) => Some(w),
                Err(e) => {
                    tracing::error!(%agent_id, "AgentInfo core→wire 변환 실패: {e}");
                    None
                }
            }
        })
        .collect()
}

fn core_report_to_wire(report: CoreRestoreReport) -> Option<RestoreReport> {
    match serde_json::to_value(&report).and_then(serde_json::from_value) {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::error!("RestoreReport core→wire 변환 실패: {e}");
            None
        }
    }
}

/// core::AgentStatus → wire JSON value. StatusChanged 직렬화에 사용.
/// AgentEvent::StatusChanged 는 wire AgentStatus 를 요구하므로 roundtrip 변환.
fn core_status_to_wire(status: CoreStatus) -> Option<engram_dashboard_protocol::AgentStatus> {
    match serde_json::to_value(&status).and_then(serde_json::from_value) {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::error!("AgentStatus core→wire 변환 실패: {e}");
            None
        }
    }
}

/// AgentEvent 를 JSON 문자열로 직렬화(control 전송용). 실패는 거의 불가능하나 로그 후 None.
fn event_json(ev: &AgentEvent) -> Option<String> {
    match serde_json::to_string(ev) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::error!("AgentEvent 직렬화 실패: {e}");
            None
        }
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
        let Some(wire_status) = core_status_to_wire(status) else {
            return;
        };
        let ev = AgentEvent::StatusChanged {
            agent_id: id,
            status: wire_status,
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
        let Some(wire) = core_report_to_wire(report) else {
            return;
        };
        let ev = AgentEvent::RestoreResult { report: wire };
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
    fn new(conn_tx: mpsc::Sender<WsOutbound>, close_signal: Arc<Notify>) -> Self {
        Self {
            conn_tx,
            close_signal,
            replay_dropped: Arc::new(AtomicBool::new(false)),
            sink_id: uuid::Uuid::new_v4(),
        }
    }

    /// replay 구간 동안 frame 이 drop 됐는지 사후 검사용 핸들(handle_subscribe 가 공유 보관).
    fn replay_dropped_flag(&self) -> Arc<AtomicBool> {
        self.replay_dropped.clone()
    }
}

impl OutputSink for WsOutputSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        let buf = encode_terminal_frame(frame.agent_id, frame.epoch, frame.seq, frame.data);
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
pub async fn handle_connection(
    stream: TcpStream,
    peer: std::net::SocketAddr,
    manager: Arc<AgentManager>,
    registry: ConnRegistry,
    expected_token: Arc<String>,
    shutdown_tx: watch::Sender<bool>,
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

    // 4) 연결 직후 Hello + 현재 목록 push(단일 writer 큐 경유 — 이후 모든 출력과 FIFO 정렬).
    if let Some(text) = event_json(&AgentEvent::Hello {
        protocol_version: PROTOCOL_VERSION,
        daemon_version: env!("CARGO_PKG_VERSION").into(),
        capabilities: None,
    }) {
        let _ = conn_tx.send(WsOutbound::Text(text)).await;
    }
    if let Some(text) = event_json(&AgentEvent::AgentListUpdated {
        agents: core_agents_to_wire(manager.list_agents()),
    }) {
        let _ = conn_tx.send(WsOutbound::Text(text)).await;
    }

    // 5) 이 연결이 등록한 (agent_id → sink_id) 추적 — cleanup 에서 누수 없이 unsubscribe 하기 위함.
    //    read_task 와 cleanup 이 공유하므로 Arc<Mutex<..>>.
    let subs: Arc<Mutex<HashMap<AgentId, SinkId>>> = Arc::new(Mutex::new(HashMap::new()));

    // read_task: stream_half 에서 명령을 읽어 dispatch. conn_tx 로 응답을 큐잉.
    //   close_signal 은 handle_subscribe 가 만드는 WsOutputSink 에 주입(full 시 깨우기용).
    let mut read_handle = tokio::spawn(read_task(
        stream_half,
        conn_tx.clone(),
        manager.clone(),
        subs.clone(),
        shutdown_tx,
        conn_id,
        close_signal.clone(),
    ));

    // write_task: conn_rx 에서 받은 WsOutbound 를 sink_half 로 순서대로 write(단일 writer).
    //   close_signal 발동 시 큐가 막혀 있어도 깨어 닫는다(좀비 방지).
    let mut write_handle = tokio::spawn(write_task(sink_half, conn_rx, conn_id, close_signal));

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
        let guard = subs.lock().expect("subs poisoned");
        guard.iter().map(|(a, s)| (*a, *s)).collect()
    };
    for (agent_id, sink_id) in leftovers {
        let _ = manager.unsubscribe(agent_id, sink_id);
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
async fn write_task(
    mut sink_half: SinkHalf,
    mut conn_rx: mpsc::Receiver<WsOutbound>,
    conn_id: ConnId,
    close_signal: Arc<Notify>,
) {
    loop {
        tokio::select! {
            // 큐 밖 종료 신호 — full 로 큐가 막혀 있어도 여기로 깨어 닫는다.
            _ = close_signal.notified() => {
                tracing::info!(conn = conn_id, "write_task: close_signal(슬로우 소비자) — 종료");
                let _ = sink_half.close().await;
                break;
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

/// stream_half 에서 명령 frame 을 읽어 dispatch. 응답은 conn_tx 로 큐잉(직접 write 안 함).
#[allow(clippy::too_many_arguments)]
async fn read_task(
    mut stream_half: StreamHalf,
    conn_tx: mpsc::Sender<WsOutbound>,
    manager: Arc<AgentManager>,
    subs: Arc<Mutex<HashMap<AgentId, SinkId>>>,
    shutdown_tx: watch::Sender<bool>,
    conn_id: ConnId,
    close_signal: Arc<Notify>,
) {
    while let Some(item) = stream_half.next().await {
        let msg = match item {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!(conn = conn_id, "read_task 수신 오류 — 종료: {e}");
                break;
            }
        };
        match msg {
            Message::Text(text) => {
                match serde_json::from_str::<AgentCommand>(&text) {
                    Ok(cmd) => {
                        if dispatch(cmd, &conn_tx, &manager, &subs, &shutdown_tx, &close_signal)
                            .await
                        {
                            // dispatch 가 연결 종료를 요청(StopDaemon 등) — 루프 탈출.
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(conn = conn_id, "명령 파싱 실패: {e}");
                        send_error(&conn_tx, None, format!("invalid command: {e}")).await;
                    }
                }
            }
            Message::Binary(_) => {
                // 클라→데몬 binary 는 프로토콜에 없음 — 오류로 보고 종료.
                tracing::warn!(conn = conn_id, "예상치 못한 binary frame — close");
                send_error(&conn_tx, None, "unexpected binary frame".into()).await;
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

/// 단일 명령 dispatch. 반환 true = 연결 종료 요청(StopDaemon).
/// side-effect 명령은 request_id 있으면 Ack/Error.
async fn dispatch(
    cmd: AgentCommand,
    conn_tx: &mpsc::Sender<WsOutbound>,
    manager: &Arc<AgentManager>,
    subs: &Arc<Mutex<HashMap<AgentId, SinkId>>>,
    shutdown_tx: &watch::Sender<bool>,
    close_signal: &Arc<Notify>,
) -> bool {
    use engram_dashboard_protocol::RequestId;

    /// side-effect 결과를 Ack/Error 로 변환해 큐잉.
    async fn reply(
        conn_tx: &mpsc::Sender<WsOutbound>,
        request_id: RequestId,
        result: Result<(), String>,
    ) {
        let ev = match result {
            Ok(()) => AgentEvent::Ack { request_id },
            Err(message) => AgentEvent::Error {
                request_id: Some(request_id),
                message,
            },
        };
        if let Some(text) = event_json(&ev) {
            let _ = conn_tx.send(WsOutbound::Text(text)).await;
        }
    }

    match cmd {
        // 2번째 Auth 는 무시(Error 만 — 이미 인증된 연결).
        AgentCommand::Auth { .. } => {
            send_error(conn_tx, None, "already authenticated".into()).await;
        }

        AgentCommand::Spawn {
            profile_id,
            request_id,
        } => {
            // 프로필 기반 spawn(profile.rs spawn_profile 미러, resume=false=Fresh).
            let result = match manager.profiles().get(profile_id) {
                Some(profile) => manager
                    .spawn_agent(&profile, SpawnMode::Fresh)
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
                None => Err(format!("profile not found: {profile_id}")),
            };
            reply(conn_tx, request_id, result).await;
        }

        AgentCommand::Kill {
            agent_id,
            request_id,
        } => {
            let result = manager.kill_agent(agent_id).map_err(|e| e.to_string());
            reply(conn_tx, request_id, result).await;
        }

        AgentCommand::Interrupt {
            agent_id,
            request_id,
        } => {
            let result = manager.interrupt(agent_id).map_err(|e| e.to_string());
            reply(conn_tx, request_id, result).await;
        }

        AgentCommand::WriteStdin {
            agent_id,
            data,
            request_id,
        } => {
            // data → InputEvent::Raw(write_stdin 이 내부에서 Raw 로 감쌈).
            let result = manager
                .write_stdin(agent_id, &data)
                .map_err(|e| e.to_string());
            reply(conn_tx, request_id, result).await;
        }

        AgentCommand::Resize {
            agent_id,
            cols,
            rows,
            viewport_id: _,
        } => {
            // Resize 는 request_id 가 없는 명령(messages.rs) — Ack 없이 best-effort, 실패만 Error.
            if let Err(e) = manager.resize(agent_id, cols, rows) {
                send_error(conn_tx, None, format!("resize failed: {e}")).await;
            }
        }

        AgentCommand::Subscribe {
            agent_id,
            epoch,
            after_seq,
        } => {
            // Step 4c: epoch/after_seq 를 코어 subscribe_from 으로 전달 → 무손실 resume(tail 만)
            // 또는 truncated/full replay 분기.
            handle_subscribe(
                agent_id,
                epoch,
                after_seq,
                conn_tx,
                manager,
                subs,
                close_signal,
            )
            .await;
        }

        AgentCommand::Unsubscribe { agent_id } => {
            // 이 연결의 그 agent sink_id 로 unsubscribe + 기록 제거.
            let sink_id = subs.lock().expect("subs poisoned").remove(&agent_id);
            if let Some(sid) = sink_id {
                let _ = manager.unsubscribe(agent_id, sid);
            }
        }

        AgentCommand::ListAgents => {
            if let Some(text) = event_json(&AgentEvent::AgentListUpdated {
                agents: core_agents_to_wire(manager.list_agents()),
            }) {
                let _ = conn_tx.send(WsOutbound::Text(text)).await;
            }
        }

        AgentCommand::StopDaemon {
            force,
            kill_agents,
            request_id,
        } => {
            // ── M4: force 정책 ──────────────────────────────────────────────────
            // force=false 인데 활성 에이전트가 남아 있으면 거부(종료하지 않음). 실수로 데몬을
            // 내려 살아있는 PTY 세션을 모두 죽이는 사고를 막는다. 활성 0이거나 force=true 면 진행.
            let active = manager.list_agents();
            if !force && !active.is_empty() {
                send_error(
                    conn_tx,
                    Some(request_id),
                    format!(
                        "active agents present ({}); use force=true to stop the daemon",
                        active.len()
                    ),
                )
                .await;
                return false; // 거부 — 연결 유지, main 종료 안 함.
            }

            // ★kill_agents 는 v1 에서 무시(always-kill)★: 데몬은 자식 PTY 를 자기
            //   KILL_ON_JOB_CLOSE Job Object 에 담는다. 따라서 데몬이 종료되면 Job 핸들이
            //   닫히며 자식이 **무조건** 함께 죽는다 — detach(데몬만 내리고 자식 유지)는 현
            //   Job 모델에선 불가능하다. kill_agents 플래그는 미래에 detach 를 지원하게 될
            //   여지로 protocol 에 남겨두되, v1 동작은 값과 무관하게 항상 자식을 정리한다.
            let _ = kill_agents; // 의도적 무시(위 주석) — 미래 detach 지원 여지.
            let mgr = manager.clone();
            let _ = tokio::task::spawn_blocking(move || mgr.shutdown_all()).await;

            reply(conn_tx, request_id, Ok(())).await;
            // main 종료 트리거(watch). 수신측은 run() 의 select! 가 감지.
            let _ = shutdown_tx.send(true);
            return true;
        }
    }
    false
}

/// 코어 ReplayKind → protocol SubscribeAction 매핑. SubscribeAck.action 구성에 사용한다.
/// (옛 predict_ack 의 별도 분기 예측을 제거 — 분기는 코어 subscribe_from 단일 스냅샷이 소유한다.)
fn kind_to_action(kind: ReplayKind) -> SubscribeAction {
    match kind {
        ReplayKind::FromOldest => SubscribeAction::Reset,
        ReplayKind::Truncated => SubscribeAction::TruncatedReplay,
        ReplayKind::Resumed => SubscribeAction::Resume,
    }
}

/// Subscribe 처리(Step 4c — afterSeq resume). **M-A(TOCTOU) 근본 해결판.**
///
/// ★TOCTOU 제거★: 옛 구현은 get_snapshot(스냅샷 A)으로 SubscribeAck 를 예측해 보낸 뒤,
/// subscribe_from 이 내부에서 다시 스냅샷 B 를 떠 replay 했다. A≠B(사이에 evict 가 끼면)면
/// Ack.replay_from/latest 가 실제 첫 전송 seq 와 어긋나 클라가 손실을 인지 못 했다. 이제는
/// SubscribeAck 의 모든 필드를 subscribe_from 의 **단일 스냅샷 outcome** 으로 채운다 —
/// get_snapshot/predict_ack 자체를 제거했다.
///
/// ★불변식 2(Ack→replay FIFO) 유지★: subscribe_from 은 subscribers lock 을 보유한 채,
/// replay 를 sink 로 보내기 **직전**에 on_ready(&outcome) 콜백을 1회 호출한다. 콜백 안에서
/// SubscribeAck(control)를 conn_tx 에 try_send 하므로, 그 enqueue 가 replay binary 의
/// try_send 보다 반드시 먼저 일어난다(단일 writer FIFO → Ack→replay→ReplayComplete 순서).
///
/// 흐름:
/// 1. agent_epoch 으로 epoch_matches 계산(없으면 error). (옛 list_agents 전체 순회 대체 — m-1.)
/// 2. WsOutputSink 생성(close_signal/replay_dropped 공유).
/// 3. subscribe_from(.., on_ready) — 콜백 안에서 outcome 기반 SubscribeAck 를 먼저 큐잉,
///    이어서 코어가 replay 를 sink 로 전송.
/// 4. 반환 outcome 으로 subs 맵 교체(옛 sid 다르면 unsubscribe).
/// 5. 사후 truncated 보정: outcome.kind==Truncated 가 아닌데 실측 replay_dropped 이면 Error 통보.
/// 6. ReplayComplete.
#[allow(clippy::too_many_arguments)]
async fn handle_subscribe(
    agent_id: AgentId,
    requested_epoch: Option<u32>,
    after_seq: Option<u64>,
    conn_tx: &mpsc::Sender<WsOutbound>,
    manager: &Arc<AgentManager>,
    subs: &Arc<Mutex<HashMap<AgentId, SinkId>>>,
    close_signal: &Arc<Notify>,
) {
    // 1. current_epoch 경량 조회. agent 없으면 즉시 error(이 경우 subscribe_from 미호출 → Ack 안 나감).
    let current_epoch = match manager.agent_epoch(agent_id) {
        Some(e) => e,
        None => {
            send_error(
                conn_tx,
                None,
                format!("subscribe failed: agent {agent_id} not found"),
            )
            .await;
            return;
        }
    };
    // epoch 일치 = 요청 epoch 이 현재 epoch 과 정확히 같을 때만. None(미지정)은 불일치 취급
    // → 코어가 FromOldest 로 전체 replay(안전 기본값).
    let epoch_matches = requested_epoch == Some(current_epoch);

    // 2. WsOutputSink 생성(close_signal/replay_dropped 공유).
    let sink = Arc::new(WsOutputSink::new(conn_tx.clone(), close_signal.clone()));
    let replay_dropped = sink.replay_dropped_flag();

    // 3. subscribe_from(.., on_ready). on_ready 는 코어가 replay 를 sink 로 보내기 직전
    //    (subscribers lock 보유 중) 1회 호출 → 그 안에서 SubscribeAck 를 conn_tx 에 먼저 try_send.
    //    ★콜백은 sync 클로저(await 불가) → try_send 만★. control 은 작아 보통 성공하나, full 이면
    //    어차피 같은 큐(sink)도 막혀 replay 가 truncated 로 잡히므로 로깅 후 진행한다.
    let conn_tx_cb = conn_tx.clone();
    let on_ready = move |outcome: &SubscribeOutcome| {
        if let Some(text) = event_json(&AgentEvent::SubscribeAck {
            agent_id,
            action: kind_to_action(outcome.kind),
            current_epoch,
            oldest_seq: outcome.oldest_seq,
            latest_seq: outcome.latest_seq,
            replay_from: outcome.replay_from,
            // 단일 스냅샷 기준 truncated. 실측 drop 보정은 호출 후 별도(아래 5).
            truncated: outcome.kind == ReplayKind::Truncated,
        }) {
            if let Err(e) = conn_tx_cb.try_send(WsOutbound::Text(text)) {
                tracing::warn!(%agent_id, "SubscribeAck try_send 실패(느린 소비자): {e}");
            }
        }
    };

    let outcome = match manager.subscribe_from(agent_id, sink, after_seq, epoch_matches, on_ready) {
        Ok(o) => o,
        Err(e) => {
            // agent 없음 등 — 콜백 미호출이라 Ack 안 나감(정상).
            send_error(conn_tx, None, format!("subscribe failed: {e}")).await;
            return;
        }
    };

    // 4. 같은 agent 재구독 시 옛 sink 가 남지 않게 교체(옛 것 unsubscribe).
    let old = subs
        .lock()
        .expect("subs poisoned")
        .insert(agent_id, outcome.sink_id);
    if let Some(old_sid) = old {
        if old_sid != outcome.sink_id {
            let _ = manager.unsubscribe(agent_id, old_sid);
        }
    }

    // 5. ReplayComplete 직전 사후 보정: replay 동기 전송 중 실제 drop 이 있었다면(코어가 sink 로
    //    try_send 하다 full 을 만남) Error 로 통보한다. Ack 엔 이미 정확한 kind 기반 truncated 가
    //    나갔고, 여기선 kind!=Truncated 인데 실측 drop 이 추가로 발생한 경우만 추가 통보한다.
    //    ★사전 capacity 추정 제거★: 단일 스냅샷이라 추정이 무의미 — replay_dropped 실측이 더 정확.
    if outcome.kind != ReplayKind::Truncated && replay_dropped.load(Ordering::Acquire) {
        send_error(
            conn_tx,
            None,
            format!("replay truncated for agent {agent_id}: output dropped during replay; please refresh"),
        )
        .await;
    }

    // 6. ReplayComplete — 이후는 라이브(클라측 C4 전환 신호).
    if let Some(text) = event_json(&AgentEvent::ReplayComplete {
        agent_id,
        epoch: current_epoch,
    }) {
        let _ = conn_tx.send(WsOutbound::Text(text)).await;
    }
}

/// Error 이벤트를 conn_tx 로 큐잉(control).
async fn send_error(
    conn_tx: &mpsc::Sender<WsOutbound>,
    request_id: Option<engram_dashboard_protocol::RequestId>,
    message: String,
) {
    if let Some(text) = event_json(&AgentEvent::Error {
        request_id,
        message,
    }) {
        let _ = conn_tx.send(WsOutbound::Text(text)).await;
    }
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

    // ── 1b. kind_to_action 매핑(Step 4c — M-A fix) ──────────────────────────
    //    옛 predict_ack(분기 예측)을 제거하고, 코어 outcome.kind → SubscribeAction 단순 매핑만
    //    남겼다(분기는 코어 단일 스냅샷이 소유). 3 variant 전수 검증.
    #[test]
    fn kind_to_action_maps_all_variants() {
        assert_eq!(
            kind_to_action(ReplayKind::FromOldest),
            SubscribeAction::Reset
        );
        assert_eq!(
            kind_to_action(ReplayKind::Truncated),
            SubscribeAction::TruncatedReplay
        );
        assert_eq!(kind_to_action(ReplayKind::Resumed), SubscribeAction::Resume);
    }

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
            data,
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
            data: b"x",
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

    // ── 7. core→wire AgentInfo 변환 roundtrip(serde 형태 일치) ────────────────
    #[test]
    fn core_agent_info_converts_to_wire() {
        use engram_dashboard_core::pty::types::{
            AgentInfo as Ci, Capabilities, ControlCaps, InputCaps, ModelCaps, OutputCaps,
            SessionCaps,
        };
        let core = Ci {
            id: uuid::Uuid::new_v4(),
            name: "t".into(),
            cwd: "/tmp".into(),
            status: CoreStatus::Running,
            cols: 80,
            rows: 24,
            epoch: 3,
            capabilities: Capabilities {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: true,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: true,
                    interrupt: true,
                    cancel: false,
                    graceful_shutdown: true,
                },
                session: SessionCaps {
                    resume: true,
                    snapshot: true,
                    cwd_env: true,
                },
                model: ModelCaps {
                    select: false,
                    temperature: false,
                    max_tokens: false,
                },
            },
        };
        let wire = core_agents_to_wire(vec![core.clone()]);
        assert_eq!(wire.len(), 1, "변환 성공(JSON 형태 일치)");
        assert_eq!(wire[0].name, "t");
        assert_eq!(wire[0].epoch, 3);
    }

    // ── 8. (M3) core::AgentStatus 모든 variant 가 wire 로 roundtrip 되는지 ────────
    //    어느 한 variant 라도 serde 태깅/필드가 어긋나면 core_agents_to_wire 가 그 agent 를
    //    silent drop 하므로 wire.len() < 1 이 되어 실패한다. status 값 자체도 wire 와 동일
    //    JSON tag 인지 직접 비교해 "변환은 됐지만 다른 variant 로 둔갑" 도 잡는다.
    #[test]
    fn all_core_status_variants_roundtrip_to_wire() {
        use engram_dashboard_core::pty::types::{
            AgentInfo as Ci, Capabilities, ControlCaps, InputCaps, ModelCaps, OutputCaps,
            SessionCaps,
        };
        use engram_dashboard_protocol::AgentStatus as WireStatus;

        let caps = Capabilities {
            input: InputCaps {
                raw: true,
                message: false,
                attachment: false,
            },
            output: OutputCaps {
                terminal_bytes: true,
                markdown: false,
                tool_events: false,
                usage: false,
            },
            control: ControlCaps {
                resize: true,
                interrupt: true,
                cancel: false,
                graceful_shutdown: true,
            },
            session: SessionCaps {
                resume: true,
                snapshot: true,
                cwd_env: true,
            },
            model: ModelCaps {
                select: false,
                temperature: false,
                max_tokens: false,
            },
        };

        // (core status, 기대 wire status) 쌍 — 6개 variant 전수.
        let cases: Vec<(CoreStatus, WireStatus)> = vec![
            (CoreStatus::Running, WireStatus::Running),
            (CoreStatus::Exiting, WireStatus::Exiting),
            (
                CoreStatus::Exited { code: Some(0) },
                WireStatus::Exited { code: Some(0) },
            ),
            (
                CoreStatus::Exited { code: None },
                WireStatus::Exited { code: None },
            ),
            (
                CoreStatus::Failed {
                    message: "boom".into(),
                },
                WireStatus::Failed {
                    message: "boom".into(),
                },
            ),
            (CoreStatus::Killed, WireStatus::Killed),
        ];

        for (core_status, expected_wire) in cases {
            let core = Ci {
                id: uuid::Uuid::new_v4(),
                name: "v".into(),
                cwd: "/tmp".into(),
                status: core_status.clone(),
                cols: 80,
                rows: 24,
                epoch: 0,
                capabilities: caps.clone(),
            };
            // (a) AgentInfo 전체 변환에서 drop 되지 않아야 한다(silent drop 회귀 방지).
            let wire = core_agents_to_wire(vec![core]);
            assert_eq!(
                wire.len(),
                1,
                "variant {core_status:?} 가 core→wire 에서 drop 됨(태깅/필드 불일치)"
            );
            // (b) status 가 같은 wire variant 로 정확히 매핑됐는지(둔갑 방지).
            assert_eq!(
                wire[0].status, expected_wire,
                "variant {core_status:?} 가 다른 wire status 로 변환됨"
            );
            // (c) core_status_to_wire 단독 경로도 동일 결과.
            let direct = core_status_to_wire(core_status.clone())
                .unwrap_or_else(|| panic!("variant {core_status:?} core_status_to_wire 실패"));
            assert_eq!(direct, expected_wire, "직접 변환 경로도 일치해야 함");
        }
    }
}
