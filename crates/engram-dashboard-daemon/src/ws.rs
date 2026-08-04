//! WebSocket 서버 본체 — 소켓 살림만 하는 네트워크 행(ADR-0129).
//!
//! 책임: accept 된 TCP stream 을 WS 업그레이드(Origin allowlist) → 1초 내 첫 frame 토큰 auth →
//! 연결 수명·단일 writer·keepalive·레지스트리. **프레임 내용의 어휘는 모른다** — 들어온 text/binary
//! 는 `ConnectionHandler`(frame_port)로 올리고, 나가는 것은 `FrameSink`(연결당)·`FrameFanout`
//! (전-연결)으로 받는다.
//!
//! ★동시성 모델(위험 지점)★
//! - **연결당 단일 writer**: SplitSink 는 동시 write 불가. 그래서 모든 출력 프레임을 연결당 단일
//!   `mpsc::Sender<Frame>`(conn_tx)에 넣고, write_task 한 곳만 SinkHalf 에 write 한다.
//!   SubscribeAck→replay→live 의 FIFO 순서가 이 단일 큐로 보장된다.
//! - **try_send vs await 경계**: 위층 sink 가 pump 스레드에서 부르는 `FrameSink::try_send` 는 절대
//!   block 금지. async 문맥의 `FrameSink::send` 는 await 허용(.send().await).
//! - **out-of-band 종료 신호(close_signal)**: conn_tx 가 full 이면 큐 안 마커(`Frame::Close`)도
//!   try_send 실패해 좀비 연결이 된다. 그래서 큐 **밖**의 `Arc<Notify>` close_signal 을 둔다.
//!   `ConnFrameSink` 가 try_send 에서 full 을 만나면 `close_signal.notify_one()`(sync 안전)으로
//!   신호하고, write_task 는 `tokio::select!` 로 conn_rx.recv() 와 close_signal.notified() 를 동시에
//!   대기해 큐가 막혀 있어도 깨어 sink_half.close() 후 break → cleanup 한다.
//! - **레지스트리**: 전-연결 팬아웃용. 모든 연결의 conn_tx 를 ConnId→Sender 맵으로 보관하고, 위층에는
//!   `FrameFanout`(불투명 text 하나를 전 연결에 try_send)만 내준다. 등록·해제·id 발급은 이 파일이 쥔
//!   연결 수명이라 그 포트에 없다 — 그 포트로 표현할 수 있는 것은 "전부에게 이 text" 하나뿐이다.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ★ADR-0129 잔여 — auth 핸드셰이크★: 핸드셰이크를 데몬 소유 타입으로 옮기는 것은 후속 슬라이스라,
//   지금은 `AgentCommand::Auth`/`PROTOCOL_VERSION` 만 네트워크 행에 남는다. 이 두 이름이 이 파일에
//   남은 **유일한** 에이전트 어휘다(상태·메시징 sink 는 status_fanout/messaging_host 로 이사했다).
use engram_dashboard_protocol::{AgentCommand, PROTOCOL_VERSION};

use crate::frame_port::{
    ConnFlow, ConnId, ConnectionHandler, ConnectionHandlerFactory, Frame, FrameError, FrameFanout,
    FrameSink,
};

use futures_util::future::BoxFuture;
use futures_util::{SinkExt, Stream, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Notify};
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

/// 전-연결 팬아웃용 연결 레지스트리. connect 시 등록, disconnect 시 제거.
/// 위층은 `FrameFanout`(아래 impl)으로만 이 맵에 닿아 전 연결 conn_tx 에 try_send 한다 — 실린 text 가
/// 무엇인지(상태 통지든 목록 갱신이든)는 이 파일의 관심사가 아니다.
#[derive(Clone)]
pub struct ConnRegistry {
    inner: Arc<Mutex<HashMap<ConnId, mpsc::Sender<Frame>>>>,
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

    fn register(&self, id: ConnId, tx: mpsc::Sender<Frame>) {
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

    /// 이 conn 이 아직 fanout 대상인지 — 테스트가 cleanup 순서(정리 후 등록 해제)를 관측한다.
    #[cfg(test)]
    pub(crate) fn contains(&self, id: ConnId) -> bool {
        self.inner
            .lock()
            .expect("conn registry poisoned")
            .contains_key(&id)
    }
}

/// 전-연결 팬아웃 포트의 WS 구현. 위층은 이 trait 으로만 레지스트리에 닿으므로 등록·해제·id 발급
/// (연결 수명 = 네트워크 살림)에는 손이 닿지 않는다.
// ADR-0129
impl FrameFanout for ConnRegistry {
    /// 전 연결에 Text 브로드캐스트(try_send). full 인 연결은 느린 것으로 보고 로그만.
    ///
    /// ★맵을 잠근 채 보내지 않는다(ADR-0006 락 순서)★: 스냅샷을 뜬 뒤 락을 놓고 그 사본으로 send 한다
    ///   — 락 보유 중 외부(채널)로 나가는 호출을 만들지 않기 위해서다.
    /// ★포화를 연결 종료로 잇지 않는다★: 연결당 `ConnFrameSink::try_send` 는 여기서 close_signal 을
    ///   울리지만 팬아웃은 로그만 남기고 다음 연결로 간다(`FrameFanout` 계약의 비대칭).
    fn broadcast_text(&self, text: String) {
        let conns: Vec<(ConnId, mpsc::Sender<Frame>)> = {
            let guard = self.inner.lock().expect("conn registry poisoned");
            guard.iter().map(|(id, tx)| (*id, tx.clone())).collect()
        };
        for (id, tx) in conns {
            // try_send 만 — 위층 호출자가 pump/manager 스레드(sync)일 수 있어 block 금지(`FrameFanout` 의무 1).
            if let Err(e) = tx.try_send(Frame::Text(text.clone())) {
                tracing::warn!(conn = id, "전-연결 팬아웃 try_send 실패(느린 소비자): {e}");
            }
        }
    }
}

impl Default for ConnRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── ConnFrameSink(연결당 프레임 출구 — 프레임 포트의 WS 구현) ──────────────────────

/// 한 연결의 단일 writer 큐를 `FrameSink` 로 노출한다. 위층(에이전트 시스템)은 이걸 통해서만
/// 내보내므로 프레임에 실린 어휘가 무엇이든 이 파일은 모른다.
///
/// ★R6 close_signal(out-of-band)★: `try_send` 가 큐 포화를 만나면 `FrameError` 를 돌려주면서
/// close_signal 을 notify 해 write_task 가 큐가 막혀도 깨어 닫게 한다(WS-특정 처리라 여기 잔류).
/// `send`(backpressure 허용)는 기다릴 수 있는 호출자이므로 신호하지 않는다 — 이 비대칭이 계약이다.
// ADR-0129
pub(crate) struct ConnFrameSink {
    conn_tx: mpsc::Sender<Frame>,
    /// 큐 밖 종료 신호. full 감지 시 notify_one — write_task 가 큐가 막혀도 깨어 닫는다.
    /// ★pump 스레드(sync)에서 notify_one 호출 OK — Notify 는 sync-safe.
    close_signal: Arc<Notify>,
}

impl ConnFrameSink {
    pub(crate) fn new(conn_tx: mpsc::Sender<Frame>, close_signal: Arc<Notify>) -> Self {
        Self {
            conn_tx,
            close_signal,
        }
    }
}

impl FrameSink for ConnFrameSink {
    fn try_send(&self, frame: Frame) -> Result<(), FrameError> {
        match self.conn_tx.try_send(frame) {
            Ok(()) => Ok(()),
            Err(_) => {
                self.close_signal.notify_one();
                Err(FrameError)
            }
        }
    }

    fn send(&self, frame: Frame) -> BoxFuture<'_, Result<(), FrameError>> {
        Box::pin(async move { self.conn_tx.send(frame).await.map_err(|_| FrameError) })
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
/// WS 업그레이드 → auth → 핸들러 부착 → read/write task → cleanup.
///
/// `expected_token` 은 daemon.json 의 토큰. `handlers` 는 이 연결에 붙일 위층 핸들러 공장 —
/// 프레임의 의미(명령 해석·이벤트 인코딩·연결 정리)는 전부 그쪽이 소유하므로, 이 함수는 에이전트
/// 어휘를 auth 핸드셰이크 말고는 알지 못한다(ADR-0129).
// ADR-0129
pub async fn handle_connection(
    stream: TcpStream,
    peer: std::net::SocketAddr,
    registry: ConnRegistry,
    handlers: Arc<dyn ConnectionHandlerFactory>,
    expected_token: Arc<String>,
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
                        let _ = send_error_and_close(
                            &mut ws,
                            handlers.handshake_error_frame("auth failed"),
                        )
                        .await;
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
                            handlers.handshake_error_frame(&format!(
                                "protocol_version mismatch: client {protocol_version} != server {PROTOCOL_VERSION}"
                            )),
                        )
                        .await;
                        return;
                    }
                    tracing::info!(%peer, "auth 성공");
                }
                Ok(_) => {
                    tracing::warn!(%peer, "첫 frame 이 Auth 가 아님 — close");
                    let _ = send_error_and_close(
                        &mut ws,
                        handlers.handshake_error_frame("expected Auth as first frame"),
                    )
                    .await;
                    return;
                }
                Err(e) => {
                    tracing::warn!(%peer, "첫 frame 파싱 실패: {e} — close");
                    let _ = send_error_and_close(
                        &mut ws,
                        handlers.handshake_error_frame("invalid first frame"),
                    )
                    .await;
                    return;
                }
            }
        }
        Ok(Some(Ok(_))) => {
            tracing::warn!(%peer, "첫 frame 이 Text 가 아님 — close");
            let _ = send_error_and_close(
                &mut ws,
                handlers.handshake_error_frame("expected Auth text frame"),
            )
            .await;
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
            let _ =
                send_error_and_close(&mut ws, handlers.handshake_error_frame("auth timeout")).await;
            return;
        }
    }

    // 3) conn_tx/rx 생성 + close_signal + 레지스트리 등록 + split.
    let (conn_tx, conn_rx) = mpsc::channel::<Frame>(CONN_TX_CAP);
    // ★out-of-band 종료 신호★: 큐 포화로 `Frame::Close` 마저 못 들어갈 때 write_task 를 깨운다.
    let close_signal = Arc::new(Notify::new());
    let conn_id = registry.alloc_id();
    registry.register(conn_id, conn_tx.clone());
    tracing::info!(%peer, conn = conn_id, "연결 인증 완료 — 등록");

    let (sink_half, stream_half) = ws.split();

    // 3b) 프레임 출구 + 위층 핸들러 부착. 이 연결의 모든 출력은 frames(단일 writer 큐)로만 나가고,
    //     들어온 프레임의 의미 해석은 handler 가 소유한다(ADR-0129 — 이 함수는 어휘를 모른다).
    let frames: Arc<dyn FrameSink> =
        Arc::new(ConnFrameSink::new(conn_tx.clone(), close_signal.clone()));
    let handler = handlers.handler_for(conn_id);

    // 4) 연결 직후 인사·초기 상태 push(단일 writer 큐 경유 — 이후 모든 출력과 FIFO 정렬).
    //    ★소비자보다 먼저다★: write_task 는 아직 없으므로 여기서 넣은 프레임은 큐에 쌓이기만 한다
    //    (그 제약이 ConnectionHandler::on_connect 의 계약). 순서상 여기가 두 task 스폰보다 앞이어야
    //    명령 dispatch 가 인사보다 앞설 수 없다. ★단 "연결의 첫 프레임" 보장은 아니다★ — 등록이 이미
    //    끝났으므로 전-연결 팬아웃이 그 사이 큐에 먼저 들어갔을 수 있다.
    handler.on_connect(conn_id, &frames).await;
    // ★관측만 하고 흐름은 바꾸지 않는다★: 패닉을 쓰지 않는 이유는 여기서 죽으면 아래 정리 훅과
    //   레지스트리 해제를 통째로 건너뛴 채 죽은 큐가 fanout 대상으로 남기 때문이다(HEAD 에 없던 종료 경로).
    // ★잡히는 범위 = "정확히 가득 찬 채로 on_connect 이 **반환한**" 경계 하나뿐★: 정작 위험한 쪽
    //   (용량을 넘겨 넣어 `send` 가 영구 대기)은 이 줄에 **도달조차 못 한다** — 그 hang 은 어디에도
    //   로그가 남지 않는다(알려진 미로그 구멍 — `ConnectionHandler::on_connect` 계약에 서술).
    // ★"지금 막혔다"는 뜻이 아니다★: 바로 아래에서 write_task 가 떠 큐를 비우므로 진행은 계속된다.
    if conn_tx.capacity() == 0 {
        tracing::warn!(
            conn = conn_id,
            "on_connect 반환 시점에 연결 큐가 가득 찼다 — 한 프레임만 더 넣었으면 writer 기동 전에 영구 대기했을 것(핸들러 푸시 + 등록 후 전-연결 팬아웃 합계)"
        );
    }

    // ── keepalive 공유 시계(A) ──────────────────────────────────────────────────────
    // base = 연결 시작 시각(tokio Instant). last_recv = base 기준 경과 ms(AtomicU64).
    // read_task 가 클라로부터 무언가(Pong 포함) 받을 때마다 갱신하고, write_task 의 ping arm 이
    // base.elapsed() - last_recv 로 idle 경과를 계산해 idle_timeout 초과 시 close_signal 발동.
    let keepalive_base = tokio::time::Instant::now();
    let last_recv = Arc::new(AtomicU64::new(0));

    // read_task: stream_half 에서 프레임을 읽어 handler 로 올린다. 응답은 handler 가 frames 로
    //   큐잉하므로 read_task 자신은 소켓에 직접 쓰지 않는다.
    let mut read_handle = tokio::spawn(read_task(
        stream_half,
        frames,
        handler.clone(),
        conn_id,
        keepalive_base,
        last_recv.clone(),
    ));

    // write_task: conn_rx 에서 받은 프레임을 sink_half 로 순서대로 write(단일 writer).
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

    // 5) 하나라도 끝나면 cleanup. ★살아남은 쪽을 명시적으로 abort★ — JoinHandle 을 그냥 drop 하면
    //    task 가 detach 되어 계속 돈다(WS half 를 붙든 채 좀비). 그래서 &mut 로 select 해 핸들을
    //    소비하지 않고, 진 쪽을 abort 한다(연결의 read/write 가 함께 끝나게).
    //    회귀 방어 = `the_losing_task_is_aborted_not_detached`(read 를 abort 하는 갈래만).
    //    ★write 를 abort 하는 갈래도 abort 를 제거하지 말 것★: 대개는 송신단이 모두 드롭돼 write_task 가
    //    스스로 끝나지만, 그건 "모든 Sender<Frame> 사본이 함께 죽는다" 는 조건부다 — 구독 기록 누락
    //    (아래 on_disconnect 경쟁)으로 사본이 살아남으면 자기종료가 성립하지 않는다. 전수 열거와 실측
    //    범위는 그 테스트 주석에 있다.
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
    // 위층 정리(구독 해제·viewport 재협상·lease 반납)가 **먼저**, 레지스트리 제거가 **나중**이다.
    // ★이 순서에 배달 정합성이 걸려 있지는 않다★: 브로드캐스트는 맵 스냅샷으로 돌아 다른 연결이
    // 받는 것은 순서와 무관하고, 이 연결 자신의 몫은 배달을 전제할 수 없다(writer 가 먼저 끝난
    // 갈래면 확실히 버려지고, reader 가 먼저 끝난 갈래면 abort 가 먹기 전에 나갈 수도 있다 —
    // ConnectionHandler::on_disconnect 문서). 순서를 지키는 이유는 순수 리팩터로 두려는 것뿐이다.
    // ★위 abort 는 완료를 기다리지 않는다★: 취소된 read_task 가 아직 핸들러 호출 안에 있을 수 있어
    // on_disconnect 와 겹칠 수 있다(HEAD 도 동일한 잔여 경쟁 — ConnectionHandler 문서).
    handler.on_disconnect(conn_id);

    registry.unregister(conn_id);
    tracing::info!(%peer, conn = conn_id, "연결 종료 — cleanup 완료");
}

/// 핸드셰이크 실패 통보 + close 를 소켓에 직접 쓴다(레지스트리 등록 전이라 단일 writer 큐가 없다).
/// `frame` 은 위층이 인코딩한 불투명 text — None 이면 본문 없이 close 만 한다.
async fn send_error_and_close(
    ws: &mut tokio_tungstenite::WebSocketStream<TcpStream>,
    frame: Option<String>,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    if let Some(text) = frame {
        ws.send(Message::Text(text.into())).await?;
    }
    ws.close(None).await
}

// ── write_task(단일 writer) ───────────────────────────────────────────────────

type SinkHalf =
    futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, Message>;

/// conn_rx 에서 받은 출력을 sink_half 로 순서대로 write. 이게 이 연결의 유일한 writer.
/// 종료 트리거 3가지: (1) 큐 안의 `Frame::Close`, (2) sink send 실패, (3) close_signal.
///
/// ★out-of-band 종료(M1 핵심)★: conn_tx 가 full 이면 `Frame::Close` 마저 큐에 못 들어가
/// 좀비 연결이 된다. 그래서 `tokio::select!` 로 conn_rx.recv() 와 close_signal.notified() 를
/// 동시에 대기한다. `ConnFrameSink` 가 try_send 에서 full 을 만나 `close_signal.notify_one()` 하면, 큐가
/// 가득 차 있어도 이 select 가 깨어 sink_half.close() 후 break → cleanup 으로 이어진다.
///
/// ★알려진 구멍 — 진행 중인 소켓 write 는 이 신호로 끊기지 않는다(HEAD 도 동일, 이 슬라이스 범위 밖)★:
/// recv arm 은 프레임을 꺼낸 **뒤** `sink_half.send(msg).await` 를 **select! 밖**(arm 본문)에서 기다린다.
/// 인증된 피어가 읽기를 멈추면 그 send 가 무기한 pending 일 수 있고, 그 동안 이 task 는 select! 를
/// 폴링하지 않으므로 `close_signal` 도 keepalive tick 도 그 연결을 구할 수 없다 — 같은 select! 안에
/// 있어서 둘 다 함께 멈춘다. 즉 코드가 광고하는 "슬로우 소비자 정리" 는 **이 경우를 덮지 못한다**.
/// `Notify` 는 대기자가 없을 때 permit 을 보관하므로 깨우기는 **유실이 아니라 지연**이다(그 send 가
/// 언젠가 풀리면 즉시 발화). 고치려면 write 자체에 타임아웃/취소를 걸어야 하는데 그건 동작 변경이다.
#[allow(clippy::too_many_arguments)]
async fn write_task(
    mut sink_half: SinkHalf,
    mut conn_rx: mpsc::Receiver<Frame>,
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
                    Frame::Text(s) => Message::Text(s.into()),
                    Frame::Binary(b) => Message::Binary(b.into()),
                    Frame::Close(reason) => {
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

/// 수신 프레임을 위층 `ConnectionHandler` 로 올린다. 응답은 handler 가 `FrameSink` 로 큐잉하므로
/// 이 루프는 소켓에 직접 쓰지 않는다. handler 가 `ConnFlow::Close` 를 돌려주면 루프를 탈출한다
/// (StopDaemon·프로토콜 위반).
///
/// ★stream 이 generic 인 이유★: 소켓 없이 합성 프레임열로 이 루프를 돌리는 격리 하네스를 두려고
/// (ADR-0129 — 이 seam 이 뒤 슬라이스의 crate 분리 검증 근거다). 운영 경로는 WS stream half 로만
/// 단형화된다.
async fn read_task<S>(
    mut incoming: S,
    frames: Arc<dyn FrameSink>,
    handler: Arc<dyn ConnectionHandler>,
    conn_id: ConnId,
    keepalive_base: tokio::time::Instant,
    last_recv: Arc<AtomicU64>,
) where
    S: Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin + Send,
{
    while let Some(item) = incoming.next().await {
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
                if handler.on_text(conn_id, &text, &frames).await == ConnFlow::Close {
                    break;
                }
            }
            Message::Binary(payload) => {
                // 페이로드는 **빌려준다** — 거부 경로가 유일한 소비자라 복사하지 않는다(클라가 고른
                //   크기만큼 할당하게 두면 인증 후 최대 프레임 크기까지 낭비 할당이 된다).
                if handler.on_binary(conn_id, &payload, &frames).await == ConnFlow::Close {
                    break;
                }
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

    // ── 3. Frame 매핑(Text/Binary/Close → Message) ───────────────────────────
    // write_task 의 변환 로직과 동일한 매핑을 직접 검증(실제 WS 없이).
    #[test]
    fn frame_maps_to_message() {
        let t = Frame::Text("hi".into());
        let b = Frame::Binary(vec![1, 2, 3]);
        let c = Frame::Close("bye".into());

        let to_msg = |o: Frame| -> Message {
            match o {
                Frame::Text(s) => Message::Text(s.into()),
                Frame::Binary(b) => Message::Binary(b.into()),
                Frame::Close(_) => Message::Close(None),
            }
        };
        assert!(matches!(to_msg(t), Message::Text(_)));
        assert!(matches!(to_msg(b), Message::Binary(_)));
        assert!(matches!(to_msg(c), Message::Close(_)));
    }

    // ── 4. ConnFrameSink: try_send 는 포화 시 out-of-band close_signal 을 울린다 ──
    #[tokio::test]
    async fn conn_frame_sink_notifies_close_signal_when_full() {
        // cap 1 채널을 가득 채운 뒤: try_send 가 Err 를 반환하고, 큐가 막혀 있어도 close_signal 이
        // 발동(write_task 를 깨움)하는지 — 이게 좀비 연결을 막는 유일한 경로(M1).
        let (tx, mut rx) = mpsc::channel::<Frame>(1);
        let close_signal = Arc::new(Notify::new());
        let sink = ConnFrameSink::new(tx, close_signal.clone());

        sink.try_send(Frame::Text("first".into()))
            .expect("빈 큐엔 들어간다");
        assert!(
            sink.try_send(Frame::Text("second".into())).is_err(),
            "full 이면 FrameError"
        );

        tokio::time::timeout(Duration::from_millis(200), close_signal.notified())
            .await
            .expect("close_signal 이 full 에서도 발동해야 함");

        assert!(matches!(rx.recv().await.unwrap(), Frame::Text(_)));
    }

    // ── 4b. ConnFrameSink: send(backpressure)는 close_signal 을 울리지 않는다 ──
    #[tokio::test]
    async fn conn_frame_sink_send_does_not_signal_close() {
        // ★try_send / send 비대칭★: 기다릴 수 있는 호출자는 슬로우 소비자 판정 대상이 아니다.
        //   cap 1 을 채운 뒤 send 가 **실제로 포화에 park 했음을 먼저 확정**하고, 그 상태에서 종료
        //   신호가 없었음을 본 다음 한 칸 비워 통과를 확인한다.
        let (tx, mut rx) = mpsc::channel::<Frame>(1);
        let close_signal = Arc::new(Notify::new());
        let sink = ConnFrameSink::new(tx, close_signal.clone());

        sink.try_send(Frame::Text("first".into())).expect("한 칸");

        // ★포화 경로를 탔다는 **양성 관측**★: future 를 직접 1회 폴링해 Pending 을 확인한다. 이게
        //   없으면 `spawn` + 곧바로 `recv()` 조합에서 send 가 **자리가 빈 뒤에야** 처음 폴링될 수 있어
        //   (spawn 은 폴링을 보장하지 않고, 비어있지 않은 채널의 recv 는 yield 없이 Ready) 정작 검증
        //   대상인 포화 분기를 한 번도 안 타고 통과할 수 있다(실측: 그 형태는 회귀를 놓쳤다).
        let mut pending = Box::pin(sink.send(Frame::Text("second".into())));
        assert!(
            futures_util::poll!(pending.as_mut()).is_pending(),
            "가득 찬 큐에서 send 는 반드시 park 해야(포화 경로 미실행이면 이 테스트가 무의미하다)"
        );
        // park 한 그 상태에서 종료 신호가 없어야 한다(try_send 와 갈리는 지점).
        assert!(
            tokio::time::timeout(Duration::from_millis(50), close_signal.notified())
                .await
                .is_err(),
            "send 는 포화를 기다릴 뿐 종료 신호를 울리지 않는다"
        );

        // 한 칸 비우면 park 이 풀려 통과한다.
        assert!(matches!(rx.recv().await.unwrap(), Frame::Text(_)));
        pending.await.expect("자리가 나면 backpressure 가 풀린다");
    }

    // ── 4c. ConnFrameSink: 세 프레임 종류가 그대로 단일 writer 큐에 FIFO 로 실린다 ──
    #[tokio::test]
    async fn conn_frame_sink_maps_frames_to_queue_in_order() {
        let (tx, mut rx) = mpsc::channel::<Frame>(8);
        let sink = ConnFrameSink::new(tx, Arc::new(Notify::new()));
        sink.send(Frame::Text("hi".into())).await.expect("text ok");
        sink.try_send(Frame::Binary(vec![1, 2, 3]))
            .expect("binary ok");
        sink.send(Frame::Close("bye".into()))
            .await
            .expect("close ok");

        assert!(matches!(rx.recv().await.unwrap(), Frame::Text(_)));
        assert!(matches!(rx.recv().await.unwrap(), Frame::Binary(_)));
        match rx.recv().await.unwrap() {
            Frame::Close(r) => assert_eq!(r, "bye"),
            other => panic!("Close 여야 함: {other:?}"),
        }
    }

    // ── 4d. ConnRegistry: 전-연결 팬아웃 — 포화한 연결 하나가 나머지를 막지 않는다(ADR-0129) ──
    #[test]
    fn broadcast_text_copies_the_text_to_every_connection_that_can_take_it() {
        // ★`FrameFanout` 계약의 두 항을 여기서 못 박는다★: ① 넘겨받은 text 를 연결마다 **같은 바이트**로
        //   복제한다 ② 큐가 포화한 연결은 건너뛰고 **나머지 배달을 계속**한다. 위층(상태 통지·목록
        //   브로드캐스트)은 연결을 볼 수 없으므로 이 둘을 관측할 수 있는 자리는 여기뿐이다. ②가 깨지면
        //   슬로우 클라 하나가 전 클라의 갱신을 세운다.
        // ★포화를 종료 신호로 잇지 않는다★: 연결당 `ConnFrameSink::try_send`(위 4.)와 다른 점 — 여기선
        //   경고 로그만 남기고 다음 연결로 간다. 그 비대칭 자체는 4./4b. 가 지킨다.
        //
        // ★반복하는 이유 = 매 회차 HashMap 순회 순서를 다시 뽑으려는 것(패딩 아님)★: `broadcast_text` 는
        //   레지스트리 맵을 순회해 Vec 으로 뜨므로 방문 순서가 곧 배달 순서다.
        //   - **정상 코드는 순서와 무관하게 통과한다** → 이 루프가 위양성(flake)을 만들 수는 없다. 반복이
        //     바꾸는 것은 오직 **탐지력**이다.
        //   - 잡으려는 회귀 = "첫 try_send 실패에서 fanout 중단". 그 회귀는 포화 연결이 **마지막에** 방문된
        //     회차에서는 멀쩡한 연결들이 이미 다 받은 뒤라 살아남는다(S18.17 이 기록한 "포화 경로를 한 번도
        //     안 밟고 통과" 와 같은 부류). 그래서 회차마다 **새 레지스트리**로 순서를 다시 뽑는다.
        //     ※ 같은 레지스트리를 재사용하면 순서가 고정돼 반복이 무의미하다(그래서 루프 **안**에서 만든다).
        // ★탐지력은 경험적이지 증명이 아니다(정직 명시)★: std 는 `RandomState` 가 인스턴스마다 다른 씨앗을
        //   쓴다고만 하고, **인스턴스 간 순서 독립성도 특정 분포도 보장하지 않는다**. 그러니 "K회면 놓칠 확률
        //   2^-K" 같은 계산을 여기 적을 근거가 없다 — 그건 관측을 보장으로 격상하는 것이다.
        //   ★실측(2026-08-04 · 이 형태 = 포화 1 + 멀쩡 2)★: 위 회귀를 심고 **10회 시도 전부** 잡혔다.
        //   잡히는 회차도, 굶은 연결(ok0/ok1)도 실행마다 달랐다 — 순서가 실제로 매 회차 다시 뽑힌다는
        //   증거. **보장이 아니라 측정치다.**
        // ★결정적 탐지를 원하면 순회 순서를 통제해야 한다★ = `ConnRegistry` 의 맵 타입 교체(정렬 맵 등).
        //   이사 슬라이스(ADR-0129)의 범위 밖이라 하지 않는다 — 이게 **하드 보장**이어야 할 날이 오면 그때
        //   그 교체가 정공법이고, 그 전까지 K 는 탐지력 손잡이일 뿐이다(임계값 튜닝 대상 아님 — ADR-0038).
        // ★멀쩡한 연결을 2개 두는 이유★: 포화가 **가운데**에 오는 배치까지 덮는다. 회귀가 한 회차를
        //   살아남으려면 포화 연결이 **맨 뒤**에 와야 하는데, 멀쩡한 연결이 1개면 "뒤" 가 두 자리 중
        //   하나이고 2개면 세 자리 중 하나다 — 즉 회차당 생존 여지가 좁아진다(분포 보장이 없으므로
        //   이것도 확률 계산이 아니라 **자리 수 논증**이다).
        const K: usize = 20;
        const PAYLOAD: &str = "opaque-fanout-payload";
        for round in 0..K {
            let registry = ConnRegistry::new();
            // 운영 등록 경로(alloc_id + register)를 그대로 쓴다 — 같은 모듈이라 테스트 전용 seam 이 필요 없다.
            let register = |tx: mpsc::Sender<Frame>| {
                let id = registry.alloc_id();
                registry.register(id, tx);
            };
            let (full_tx, mut full_rx) = mpsc::channel::<Frame>(1);
            full_tx
                .try_send(Frame::Text("선점".into()))
                .expect("cap 1 을 미리 채운다");
            register(full_tx);
            let mut oks: Vec<mpsc::Receiver<Frame>> = (0..2)
                .map(|_| {
                    let (ok_tx, ok_rx) = mpsc::channel::<Frame>(8);
                    register(ok_tx);
                    ok_rx
                })
                .collect();

            registry.broadcast_text(PAYLOAD.to_string());

            // 포화 연결엔 새 프레임이 못 들어갔다(선점 프레임만 남아 있다).
            assert!(
                matches!(full_rx.try_recv(), Ok(Frame::Text(s)) if s == "선점"),
                "round {round}: 포화 연결엔 선점 프레임만 있어야"
            );
            assert!(
                full_rx.try_recv().is_err(),
                "round {round}: 포화 연결은 이번 건을 못 받는다"
            );
            // 그래도 멀쩡한 연결은 **전부** 받는다 — 포화 연결의 방문 위치와 무관하게.
            // ★분기마다 회차·연결 번호를 메시지에 담는다★: 잡는 회귀("첫 포화에서 중단")는 "아무것도 못
            //   받음" 으로 나타나므로, 일반 메시지로는 어느 회차가 걸렸는지 안 보인다.
            for (n, ok_rx) in oks.iter_mut().enumerate() {
                match ok_rx.try_recv() {
                    Ok(Frame::Text(s)) => assert_eq!(
                        s, PAYLOAD,
                        "round {round}/ok{n}: 넘겨받은 text 를 그대로 복제해야"
                    ),
                    other => panic!(
                        "round {round}/ok{n}: 멀쩡한 연결이 프레임을 못 받았다(fanout 이 첫 실패에서 멈춘 회귀): {other:?}"
                    ),
                }
                assert!(
                    ok_rx.try_recv().is_err(),
                    "round {round}/ok{n}: 팬아웃 1회는 연결당 정확히 1프레임"
                );
            }
        }
    }

    // (6. Subscribe control 순서 테스트는 connection_core.rs 로, 7·7b·9b 의 flush/도어벨 테스트는
    //  messaging_host.rs 로 이동 — 검증 대상이 거기 있음. ADR-0129)

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

    // ── 11. 프레임 포트 seam — 소켓 없이 도는 격리 하네스(ADR-0129) ──────────────────
    //    가짜 ConnectionHandler + 가짜 FrameSink 로 연결 수명을 재현한다. TcpStream 이 없어야
    //    뒤 슬라이스에서 네트워크 행이 별도 crate 로 떨어져도 이 검증이 그대로 산다.

    #[derive(Debug, PartialEq, Eq)]
    enum SeenFrame {
        Text(String),
        Binary(Vec<u8>),
        Close(String),
    }

    #[derive(Default)]
    struct FakeFrameSink {
        frames: Mutex<Vec<SeenFrame>>,
    }

    impl FakeFrameSink {
        fn frames(&self) -> Vec<String> {
            self.frames
                .lock()
                .unwrap()
                .iter()
                .map(|f| match f {
                    SeenFrame::Text(s) => format!("text:{s}"),
                    SeenFrame::Binary(b) => format!("bin:{}", b.len()),
                    SeenFrame::Close(r) => format!("close:{r}"),
                })
                .collect()
        }
    }

    impl FrameSink for FakeFrameSink {
        fn try_send(&self, frame: Frame) -> Result<(), FrameError> {
            let seen = match frame {
                Frame::Text(s) => SeenFrame::Text(s),
                Frame::Binary(b) => SeenFrame::Binary(b),
                Frame::Close(r) => SeenFrame::Close(r),
            };
            self.frames.lock().unwrap().push(seen);
            Ok(())
        }
        fn send(&self, frame: Frame) -> BoxFuture<'_, Result<(), FrameError>> {
            Box::pin(async move { self.try_send(frame) })
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum HandlerCall {
        Connect(ConnId),
        Text(ConnId, String),
        /// payload 길이만 — 내용은 이 seam 의 관심사가 아니다.
        Binary(ConnId, usize),
        Disconnect(ConnId),
    }

    struct FakeHandler {
        calls: Mutex<Vec<HandlerCall>>,
        /// 이 텍스트를 받으면 `ConnFlow::Close` 를 돌려준다(수신 루프 탈출 검증용).
        close_on: Option<String>,
        /// 이 텍스트를 받으면 `Frame::Close` 를 큐에 넣고 **Continue** 를 돌려준다 — 연결을
        /// read 쪽이 아니라 **write_task 쪽**에서 끝내, select! 의 "read 를 abort" 갈래를 태운다.
        close_queue_on: Option<String>,
        /// `on_connect` 가 인사 프레임을 넣은 뒤 여기서 대기한다. 테스트가 그 창 동안 "아직 아무것도
        /// 소켓으로 안 나갔다"(=writer 미기동) 와 "아직 아무 프레임도 처리 안 됐다"(=reader 미기동)를
        /// 관측한다.
        connect_gate: Option<Arc<Notify>>,
        /// `on_text` 1건 처리 완료 신호 — 테스트가 클라 close 타이밍과 무관하게 진행하기 위한 것.
        text_seen: Arc<Notify>,
        /// `on_connect` 이 받은 프레임 출구의 약참조. 강참조는 read_task 만 들고 있으므로, 연결이
        /// 끝난 뒤에도 upgrade 되면 그 task 가 abort 되지 않고 **detach** 됐다는 뜻이다.
        /// ★이 fake 는 프레임 출구의 **강참조를 절대 보관하면 안 된다**★ — 필드에 `Arc` 를 하나라도
        /// 남기면 upgrade 가 항상 성공하고, `the_losing_task_is_aborted_not_detached` 의 폴링 루프가
        /// 끝까지 `released == false` 로 돌아 **정상 코드에서 그 테스트가 항상 실패한다**(조용한 탐지
        /// 불능이 아니라 시끄러운 위양성 — 그래서 원인을 이 필드로 되짚기 어렵다).
        frames_weak: Mutex<Option<std::sync::Weak<dyn FrameSink>>>,
        /// `on_disconnect` 시점에 이 연결이 아직 fanout 레지스트리에 있었는지(cleanup 순서 관측).
        registry: Option<ConnRegistry>,
        registered_at_disconnect: Mutex<Option<bool>>,
    }

    impl FakeHandler {
        fn new(close_on: Option<&str>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                close_on: close_on.map(|s| s.to_string()),
                close_queue_on: None,
                connect_gate: None,
                text_seen: Arc::new(Notify::new()),
                frames_weak: Mutex::new(None),
                registry: None,
                registered_at_disconnect: Mutex::new(None),
            }
        }

        /// `handle_connection` 의 순서 검증용 — 레지스트리를 들여다보고 on_connect 을 게이트로 잡는다.
        fn probing(registry: ConnRegistry, connect_gate: Arc<Notify>) -> Self {
            Self {
                registry: Some(registry),
                connect_gate: Some(connect_gate),
                ..Self::new(None)
            }
        }

        /// write_task 쪽에서 연결을 끝내는 변종(패자 abort 검증용).
        fn closing_via_writer(close_queue_on: &str) -> Self {
            Self {
                close_queue_on: Some(close_queue_on.to_string()),
                ..Self::new(None)
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|c| match c {
                    HandlerCall::Connect(id) => format!("connect:{id}"),
                    HandlerCall::Text(id, t) => format!("text:{id}:{t}"),
                    HandlerCall::Binary(id, n) => format!("binary:{id}:{n}"),
                    HandlerCall::Disconnect(id) => format!("disconnect:{id}"),
                })
                .collect()
        }

        fn registered_at_disconnect(&self) -> Option<bool> {
            *self.registered_at_disconnect.lock().unwrap()
        }

        /// `on_connect` 이 두 지점에서 쓰는 단언 — 정상 코드에선 `on_connect` 이 끝나기 전에 어떤
        /// 프레임도 처리될 수 없다(read_task 가 아직 없다).
        fn assert_nothing_processed_yet(&self, at: &str) {
            let calls = self.calls.lock().unwrap();
            assert!(
                calls.is_empty(),
                "on_connect({at}) 보다 먼저 처리된 프레임이 있다 — read_task 가 앞서 스폰됐다: {calls:?}"
            );
        }

        fn frames_still_held(&self) -> bool {
            self.frames_weak
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|w| w.upgrade())
                .is_some()
        }
    }

    impl ConnectionHandler for FakeHandler {
        fn on_connect<'a>(
            &'a self,
            conn_id: ConnId,
            frames: &'a Arc<dyn FrameSink>,
        ) -> BoxFuture<'a, ()> {
            Box::pin(async move {
                *self.frames_weak.lock().unwrap() = Some(Arc::downgrade(frames));
                // ★게이트 **앞** 단언(프로그램 순서로 결정)★: 정상 코드에선 read_task 가 아직 스폰조차
                //   안 됐으므로 처리된 프레임이 있을 수 없다. 스케줄러와 무관하게 참이다.
                self.assert_nothing_processed_yet("게이트 진입 전");
                // 인사를 **게이트 앞에서** 넣는다 — writer 가 이미 떠 있다면 이 프레임이 게이트 대기
                //   중에 소켓으로 나가고, 테스트가 그걸 잡는다.
                let _ = frames.send(Frame::Text("greeting".into())).await;
                if let Some(gate) = &self.connect_gate {
                    gate.notified().await;
                }
                // ★게이트 **뒤** 단언★: 게이트가 열릴 때까지의 창(테스트가 그 안에서 명령을 미리
                //   흘려둔다) 동안 잘못 스폰된 read_task 가 그 명령을 처리했는지 잡는다.
                self.assert_nothing_processed_yet("게이트 통과 후");
                self.calls
                    .lock()
                    .unwrap()
                    .push(HandlerCall::Connect(conn_id));
            })
        }

        fn on_text<'a>(
            &'a self,
            conn_id: ConnId,
            text: &'a str,
            frames: &'a Arc<dyn FrameSink>,
        ) -> BoxFuture<'a, ConnFlow> {
            Box::pin(async move {
                let close = self.close_on.as_deref() == Some(text);
                let close_via_writer = self.close_queue_on.as_deref() == Some(text);
                self.calls
                    .lock()
                    .unwrap()
                    .push(HandlerCall::Text(conn_id, text.to_string()));
                self.text_seen.notify_one();
                if close_via_writer {
                    let _ = frames.try_send(Frame::Close("테스트: writer 가 먼저 끝난다".into()));
                    return ConnFlow::Continue;
                }
                if close {
                    ConnFlow::Close
                } else {
                    ConnFlow::Continue
                }
            })
        }

        fn on_binary<'a>(
            &'a self,
            conn_id: ConnId,
            payload: &'a [u8],
            _frames: &'a Arc<dyn FrameSink>,
        ) -> BoxFuture<'a, ConnFlow> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push(HandlerCall::Binary(conn_id, payload.len()));
                ConnFlow::Continue
            })
        }

        fn on_disconnect(&self, conn_id: ConnId) {
            if let Some(registry) = &self.registry {
                *self.registered_at_disconnect.lock().unwrap() = Some(registry.contains(conn_id));
            }
            self.calls
                .lock()
                .unwrap()
                .push(HandlerCall::Disconnect(conn_id));
        }
    }

    struct FakeFactory {
        handler: Arc<FakeHandler>,
    }

    impl ConnectionHandlerFactory for FakeFactory {
        fn handler_for(&self, _conn_id: ConnId) -> Arc<dyn ConnectionHandler> {
            self.handler.clone()
        }
        fn handshake_error_frame(&self, message: &str) -> Option<String> {
            Some(message.to_string())
        }
    }

    fn text_frame(s: &str) -> Result<Message, tokio_tungstenite::tungstenite::Error> {
        Ok(Message::Text(s.to_string().into()))
    }

    #[tokio::test]
    async fn handler_sees_connect_then_frames_then_disconnect() {
        let fake_sink = Arc::new(FakeFrameSink::default());
        let frames: Arc<dyn FrameSink> = fake_sink.clone();
        let fake = Arc::new(FakeHandler::new(None));
        let handler: Arc<dyn ConnectionHandler> = fake.clone();

        handler.on_connect(7, &frames).await;
        read_task(
            futures_util::stream::iter(vec![
                text_frame("cmd"),
                Ok(Message::Binary(vec![1, 2, 3].into())),
                Ok(Message::Close(None)),
                text_frame("after-close"),
            ]),
            frames.clone(),
            handler.clone(),
            7,
            tokio::time::Instant::now(),
            Arc::new(AtomicU64::new(0)),
        )
        .await;
        handler.on_disconnect(7);

        assert_eq!(
            fake.calls(),
            vec![
                "connect:7",
                "text:7:cmd",
                "binary:7:3",
                "disconnect:7", // Close frame 뒤의 프레임은 소비되지 않는다
            ]
        );
        assert_eq!(
            fake_sink.frames(),
            vec!["text:greeting"],
            "on_connect 가 넣은 프레임이 단일 출구로 나간다"
        );
    }

    #[tokio::test]
    async fn close_flow_from_on_text_breaks_the_read_loop() {
        let frames: Arc<dyn FrameSink> = Arc::new(FakeFrameSink::default());
        let fake = Arc::new(FakeHandler::new(Some("stop")));
        let handler: Arc<dyn ConnectionHandler> = fake.clone();

        read_task(
            futures_util::stream::iter(vec![
                text_frame("go"),
                text_frame("stop"),
                text_frame("unreachable"),
            ]),
            frames,
            handler,
            3,
            tokio::time::Instant::now(),
            Arc::new(AtomicU64::new(0)),
        )
        .await;

        assert_eq!(
            fake.calls(),
            vec!["text:3:go", "text:3:stop"],
            "ConnFlow::Close 면 그 자리에서 수신 루프를 나간다"
        );
    }

    /// 메시지 1건을 수신 루프에 태우고 keepalive 시계가 갱신됐는지 돌려준다.
    /// 초기값을 도달 불가능한 sentinel 로 두어 "갱신됐다" 를 타이밍 없이 판정한다.
    async fn clock_updated_by(msg: Message) -> bool {
        let frames: Arc<dyn FrameSink> = Arc::new(FakeFrameSink::default());
        let handler: Arc<dyn ConnectionHandler> = Arc::new(FakeHandler::new(None));
        let last_recv = Arc::new(AtomicU64::new(u64::MAX));

        read_task(
            futures_util::stream::iter(vec![Ok(msg)]),
            frames,
            handler,
            1,
            tokio::time::Instant::now(),
            last_recv.clone(),
        )
        .await;

        last_recv.load(Ordering::Acquire) < u64::MAX
    }

    #[tokio::test]
    async fn every_received_message_updates_the_keepalive_clock() {
        // ★"무엇이든 받았다 = 살아있다"★: 갱신은 메시지 **종류를 가리지 않는다**(생산 코드가 match
        //   **앞에서** 갱신하는 이유). 특히 Pong 이 빠지면 능동 Ping 에만 답하는 정상 연결이 idle 로
        //   오판돼 끊긴다(half-open 위양성). 갱신을 일부 arm 안으로 옮기는 회귀를 잡으려면 6종을 다
        //   태워야 하므로 `Message` 의 variant 전부를 넣는다.
        assert!(clock_updated_by(Message::Text("cmd".to_string().into())).await);
        assert!(clock_updated_by(Message::Binary(vec![1].into())).await);
        assert!(clock_updated_by(Message::Ping(Vec::new().into())).await);
        assert!(clock_updated_by(Message::Pong(Vec::new().into())).await);
        assert!(clock_updated_by(Message::Close(None)).await);
        // Message::Frame 은 실소켓 수신으로는 안 올라오지만(tungstenite 문서) read_task 가 arm 을
        //   갖고 있으므로 합성해 태운다.
        assert!(
            clock_updated_by(Message::Frame(
                tokio_tungstenite::tungstenite::protocol::frame::Frame::pong(Vec::new())
            ))
            .await
        );
    }

    // ── 12. handle_connection 순서 계약(ADR-0129) ─────────────────────────────────

    /// 테스트 서버 1개를 띄우고 인증까지 마친 클라이언트를 돌려준다(공통 뼈대).
    async fn serve_one(
        registry: ConnRegistry,
        fake: Arc<FakeHandler>,
        keepalive: KeepaliveConfig,
    ) -> (
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let factory: Arc<dyn ConnectionHandlerFactory> = Arc::new(FakeFactory { handler: fake });
        let server = tokio::spawn(async move {
            let (stream, peer) = listener.accept().await.unwrap();
            handle_connection(
                stream,
                peer,
                registry,
                factory,
                Arc::new("tok".to_string()),
                keepalive,
            )
            .await;
        });

        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/"))
            .await
            .unwrap();
        let auth = serde_json::to_string(&AgentCommand::Auth {
            token: "tok".to_string(),
            protocol_version: PROTOCOL_VERSION,
        })
        .unwrap();
        client.send(Message::Text(auth.into())).await.unwrap();
        (client, server)
    }

    /// 실제 `handle_connection` 이 지키는 세 순서를 고정한다 — read_task 하네스로는 못 잡는 것들:
    ///   ① `on_connect` 가 **write_task 스폰**보다 앞선다(그래야 `on_connect` 계약의 "소비자가 아직
    ///      없다" 전제가 성립한다)
    ///   ② `on_connect` 가 **read_task 스폰**보다 앞선다(그래야 명령이 인사를 앞지르지 못한다)
    ///   ③ `on_disconnect` 가 레지스트리 제거보다 앞선다
    ///
    /// ★탐지력의 정직한 범위★ — 세 단언 다 **정상 코드를 실패시킬 수는 없다**(전부 부정형: "아직
    ///   아무것도 처리/도달하지 않았다"). 그러나 회귀를 잡는 것은 ③만 결정적이다:
    ///   - ①·② 모두 **확률적**이다. 스폰은 그 자체로 아무것도 실행하지 않으므로, 잘못 앞당겨 스폰된
    ///     task 가 **이 창 안에 폴링되어야** 흔적이 남는다. ②의 경우 회귀 구현이 read_task 를 먼저
    ///     스폰해도 실행기가 그 전에 `on_connect` 을 게이트 앞 단언까지 폴링해 버릴 수 있고, 게이트가
    ///     열릴 때까지도 명령이 처리되지 않았으면 게이트 뒤 단언과 마지막 호출 순서 검사까지 전부
    ///     통과한다. 그래서 단언을 게이트 앞·뒤 **두 곳**에 둬 창을 넓히지만(공짜다) 보증은 아니다.
    ///     ★결정적 판별자는 없다★ — 스케줄링이나 task 생성에 직접 걸 훅이 없으면 만들 수 없다.
    ///   - ③은 `on_disconnect` 안에서 레지스트리를 직접 들여다보므로 사실상 결정적이다.
    ///   요약: 위양성 0 · ③ 결정적 · ①② 확률적(창을 넓힌 표집).
    #[tokio::test]
    async fn handle_connection_orders_connect_before_both_tasks_and_cleanup_before_unregister() {
        let registry = ConnRegistry::new();
        let gate = Arc::new(Notify::new());
        let fake = Arc::new(FakeHandler::probing(registry.clone(), gate.clone()));
        let (mut client, server) =
            serve_one(registry.clone(), fake.clone(), KeepaliveConfig::default()).await;

        // 게이트가 닫힌 동안 서버 소켓에 대기하도록 명령을 미리 흘려둔다.
        client
            .send(Message::Text("cmd".to_string().into()))
            .await
            .unwrap();

        // ★①★ 이 창에서 클라에 도달하는 게 있으면 writer 가 이미 떠 있다는 뜻이다. 정상 코드에선
        //   writer 가 없으므로 **영원히** 아무것도 안 온다 — 즉 위양성(flake)은 불가능하고, 부하가
        //   높으면 위음성(놓침) 쪽으로만 틀린다. **결정적 보증이 아니라 확률적 탐지**다(임계값 튜닝
        //   대상이 아닌 이유이기도 하다 — 값을 키워도 보증이 되지는 않는다).
        assert!(
            tokio::time::timeout(Duration::from_millis(200), client.next())
                .await
                .is_err(),
            "on_connect 진행 중엔 writer 가 없어 소켓으로 나가는 게 없어야 한다"
        );

        gate.notify_one();

        // ★클라 close 타이밍에 의존하지 않는다★: 명령이 실제로 처리된 걸 확인한 **뒤에** 닫는다.
        //   (닫기를 먼저 하면 writer 의 인사 write 실패 → reader abort 경합이 결과를 좌우할 수 있다.)
        tokio::time::timeout(Duration::from_secs(5), fake.text_seen.notified())
            .await
            .expect("명령이 처리돼야");
        client.close(None).await.unwrap();

        tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .expect("handle_connection 이 반환해야")
            .unwrap();

        let calls = fake.calls();
        assert_eq!(
            calls,
            vec!["connect:1", "text:1:cmd", "disconnect:1"],
            "connect → 프레임 → disconnect 순서"
        );
        // ★③★
        assert_eq!(
            fake.registered_at_disconnect(),
            Some(true),
            "on_disconnect 시점엔 아직 fanout 레지스트리에 있다"
        );
        assert!(
            !registry.contains(1),
            "handle_connection 이 반환할 땐 등록 해제돼 있다"
        );
    }

    /// ★패자 task 의 명시적 abort★(`handle_connection` 의 select!) — JoinHandle 을 그냥 drop 하면
    /// detach 되어 WS half 를 쥔 task 가 살아남는다. 그 누수는 e2e 로는 안 보인다(전부 정상 종료라
    /// 남은 half 가 표에 안 드러난다).
    ///
    /// ★관측 방법★: 프레임 출구 `Arc` 의 **강참조는 read_task 만** 들고 있다(`handle_connection` 은
    /// 그것을 read_task 로 move 하고, `on_connect` 은 빌리기만 한다). 그래서 연결 종료 뒤에도 약참조가
    /// upgrade 되면 = read_task 가 살아 있다 = abort 대신 detach 됐다는 뜻이다.
    /// 이 방향(write 가 먼저 끝나 read 를 abort 하는 갈래)을 태우려고 핸들러가 `Frame::Close` 를 큐에
    /// 넣고 Continue 를 돌려준다 — read 는 계속 `next()` 에 파킹된 채로 남는다.
    /// ★대칭 갈래(read 가 먼저 끝나 write 를 abort)는 **미검증으로 남긴다**★ — 아래 조건부 관측 때문에
    /// 부정형 테스트를 못 만들었을 뿐이고, **그쪽 `abort()` 가 불필요하다는 뜻은 아니다.**
    ///
    /// ★관측된 것(2026-08-04, 현 `AgentConnection` + 아래 경쟁 미발생 조건에서만)★: write_task 를
    /// detach 해도 `handle_connection` 반환 시 마지막 `Sender<Frame>` 이 드롭되어 `conn_rx.recv()` 가
    /// None 을 돌려주고, write_task 가 **스스로 종료**하며 sink_half 를 놓았다(짧은 `ping_interval` 로
    /// "종료 후 Ping 이 더 오는가" 를 봤더니 즉시 EOF — 소켓이 닫혔다 = writer 가 이미 끝났다).
    ///
    /// ★그 자기종료는 "모든 `Sender<Frame>` 사본이 `handle_connection` 과 함께 죽는다" 는 **조건부**다★.
    /// 사본 전수: ① 이 함수의 지역 `conn_tx` ② 레지스트리 항목(등록 해제로 소멸) ③ read_task 가 소유한
    /// `ConnFrameSink` ④ 코어 subscribers 에 등록된 `FrameOutputSink` 사본들 — ④는 `session.subs` 기록을
    /// 통해서만 회수된다(`on_disconnect`).
    /// ★그래서 ④가 새면 조건이 깨지고, 살아남은 sender 때문에 detach 된 writer 는 **자기종료하지
    /// 않는다**★. 새는 경로가 실제로 있다: `handle_subscribe` 가 코어에 sink 를 등록한 **뒤**
    /// `subs.insert` 전에 read_task 가 **패닉**하면(그 사이엔 `.await` 가 없어 취소는 못 끼어들지만
    /// 패닉은 낀다) 그 sink 는 어디에도 기록되지 않은 채 코어에 남아 conn_tx 사본을 붙든다.
    /// 릴리스는 `panic=abort` 라 프로세스가 죽지만 debug/테스트 빌드에선 도달 가능하다.
    /// **결론: 그 갈래의 `abort()` 는 제거 금지** — 위 실측은 "그 경쟁이 안 났을 때 그렇더라" 이지
    /// 불변식이 아니다.
    /// 반대 갈래(이 테스트)는 read_task 가 `next()` 에 파킹돼 스스로 끝나지 않으므로 abort 가 유일한
    /// 회수 수단이다 — 그래서 여기만 검증한다.
    #[tokio::test]
    async fn the_losing_task_is_aborted_not_detached() {
        let registry = ConnRegistry::new();
        let fake = Arc::new(FakeHandler::closing_via_writer("die"));
        let (mut client, server) =
            serve_one(registry.clone(), fake.clone(), KeepaliveConfig::default()).await;

        client
            .send(Message::Text("die".to_string().into()))
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .expect("write_task 가 Frame::Close 로 끝나면 handle_connection 도 반환해야")
            .unwrap();

        // abort 는 요청이라 future drop 이 반환과 동기는 아니다 — 넉넉히 폴링한다(정상 코드는 곧
        //   놓고, detach 회귀는 `next()` 에 영원히 파킹돼 절대 놓지 않는다).
        let mut released = false;
        for _ in 0..200 {
            if !fake.frames_still_held() {
                released = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            released,
            "패자 task 가 abort 되지 않았다(detach) — WS half 를 쥔 채 살아남는다"
        );
        drop(client);
    }
}
