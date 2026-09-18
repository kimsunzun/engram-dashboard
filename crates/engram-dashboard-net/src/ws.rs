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
//! - **연결당 수신 큐(읽기 ≠ 처리)**: 읽기 루프는 프레임을 디코드해 `Inbound` 큐에 넣고 곧바로 다음
//!   프레임을 읽는다. 핸들러 호출은 그 큐의 **단일** 소비자(`dispatch_task`)가 도착 순서대로 한다.
//!   둘을 한 줄로 묶어 두면 명령 하나의 처리 시간이 곧 그 연결 전체의 정지 시간이 된다(소켓
//!   head-of-line blocking). 순서가 뜻을 갖는 명령들은 소비자가 하나라는 성질로 그대로 지켜진다 —
//!   오래 걸리는 명령을 그 줄에서 떼는 판단은 위층 몫이다(`frame_port::ConnectionHandler` 계약).
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

use crate::auth::AuthFrame;

use crate::frame_port::{
    ConnFlow, ConnId, ConnectionHandler, ConnectionHandlerFactory, Frame, FrameError, FrameFanout,
    FrameSink, Saturated,
};

use futures_util::future::BoxFuture;
use futures_util::{SinkExt, Stream, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Notify};
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::{Bytes, Message, Utf8Bytes};

/// 연결당 송신 큐 용량. ReplayBuffer.max_events(4096) + control_slack(512) = 4608.
/// replay 전체가 들어가도 control 여유가 남게 한다(output_core.rs 불변식과 정합).
const CONN_TX_CAP: usize = 4608;

/// 연결당 **수신** 도착순 줄의 **칸 수** — 읽기 루프가 핸들러의 일을 기다리지 않고 다음 프레임을
/// 읽게 하는 여유. 메모리 축은 이 값이 아니라 [`CONN_RX_MAX_BYTES`] 가 진다(그 doc 이 회계의 정본).
///
/// ★이 큐는 backlog 저장소가 아니라 **버스트 흡수기**다★: 소비자(`dispatch_task`)가 한 번에 붙드는
///   것은 도착 순서가 뜻을 갖는 프레임 **하나**뿐이고, 긴 일은 위층이 이 줄 밖으로 떼어 낸다
///   (`frame_port::ConnectionHandler` 의 「도착 순서」 계약). 그래서 replay 전량을 담아야 하는
///   [`CONN_TX_CAP`] 과 달리 큰 값이 필요하지 않다 — 두 값을 같은 축으로 견주지 말 것.
/// ★★가득 차도 **읽기 루프는 서지 않는다** — 그 한 프레임의 처분을 위층에 묻고 계속 읽는다★★
///   ([`ConnectionHandler::on_inbound_saturated`]). ★한때 여기 적혀 있던 「가득 차면 읽기 루프가
///   기다린다」는 **틀린 설계였다**★: TCP 는 스트림이라 기다리는 동안 **그 뒤 프레임은 소켓에서
///   꺼내지지도 않는다**. 그러면 위층이 「굶으면 안 된다」고 고른 프레임([`Saturated::Bypass`])이
///   막힌 줄 뒤에서 굶고, 이 배치가 없애려던 바로 그 증상이 다른 경로로 되돌아온다.
///   ㆍ**그래도 조용히 버리지는 않는다**: 처분을 위층에 되물어 [`Saturated::Refused`] 를 받는다는 것은
///     **위층이 이미 답장을 냈다**는 뜻이다(포트 계약의 의무 2). 그 답장조차 못 냈을 때의 처분은
///     [`Saturated::Unanswered`] 가 따로 갖는다 — 그 자리에서만 연결을 끝낸다.
///   ㆍ**그 밖에는 끊지도 않는다**: 송신 큐([`ConnFrameSink::try_send`])가 포화에서 연결을 끊는 것은
///     그쪽 호출자가 **답장을 만들 수 없는** 동기 경로이기 때문이다(비대칭의 정본 =
///     `frame_port::FrameSink`). 여기서는 위층이 거절을 말로 할 수 있으므로 연결을 죽일 이유가 없다.
/// ★그래서 이 값이 재는 것★ = 「소비자가 막힌 동안 **손실 없이** 받아 둘 프레임 **개수**」. 둘 중
///   먼저 걸리는 쪽이 포화다 — 칸이 다 차거나, 바이트 예산이 다 차거나.
/// ★소비자가 멀쩡하면 이 값은 보이지도 않는다★: 줄에 남는 것은 await 지점이 없는 짧은 프레임들뿐이라
///   (긴 것은 위층이 떼어 낸다) 정상 운용에서 64 칸이 차는 일이 없다. 차 있다는 것은 **소비자가
///   막혔다**는 신호이고, 그 상태에서 옳은 답은 기다리는 것이 아니라 건너뛸 것을 통과시키는 것이다.
// ADR-0206
const CONN_RX_CAP: usize = 64;

/// 도착순 줄이 붙들 수 있는 **페이로드 바이트** 예산 — [`CONN_RX_CAP`] 과 **함께** 포화를 정한다.
///
/// ★왜 칸 수만으로는 모자란가★: 한 칸에 드는 것은 tungstenite 가 준 페이로드 핸들이라 **칸마다 크기가
///   다르다**. 칸 수만 세면 이 줄의 실제 천장이 `CONN_RX_CAP × (한 프레임의 상한)` 이 되는데, 그
///   상한은 이 crate 가 정한 적이 없는 **tungstenite 기본값 `max_message_size` = 64 MiB** 다
///   (업그레이드에 `WebSocketConfig` 를 넘기지 않는다 — `handle_connection` 의 `accept_hdr_async`).
///   즉 64 × 64 MiB = **4 GiB** 였다. ★한때 여기 적혀 있던 「프레임 하나의 상한은 이 상수의 관심사가
///   아니다」는 그 곱을 아무도 세지 않게 만든 문장이다★ — 회계는 둘 중 하나가 반드시 져야 한다.
/// ★판정은 **더하기 전 물높이**로 한다★: 이미 든 바이트가 이 값 이상이면 포화로 본다. 그래서 빈 줄에
///   오는 프레임은 **크기와 무관하게 언제나 한 장 받는다** — 크다는 이유만으로 정당한 명령이 거절되는
///   일이 없다. 대가로 천장이 「예산 + 프레임 하나」가 된다(= 16 MiB + max_message_size).
/// ★남는 항 — 정직하게 적는다★: 위 곱의 둘째 항(프레임 하나의 상한)을 좁히려면 업그레이드에
///   `WebSocketConfig` 를 넘겨야 하는데, 그것은 **받아 주는 프레임의 크기를 바꾸는 동작 변경**이라
///   별건이다. 여기서는 개수 축만 닫는다.
/// ★값의 근거★: 이 줄에 서는 것은 짧은 제어 프레임들이고 큰 것은 어쩌다 한 장이다 — 16 MiB 면 그
///   「어쩌다 한 장」이 여러 번 겹쳐도 남고, 정상 버스트는 이 값을 볼 일이 없다.
const CONN_RX_MAX_BYTES: usize = 16 * 1024 * 1024;

/// 도착순 줄을 건너뛴 프레임([`Saturated::Bypass`])이 서는 줄의 용량.
///
/// ★작은 이유★: 이 줄에 드는 것은 「막힌 줄을 풀려는 것」뿐이라 정상 운용에서 **한 칸도 안 쓴다**.
///   8 칸이 차 있다는 것은 그것을 여덟 번 보냈는데 하나도 안 끝났다는 뜻 — 그 경로 자체가 막힌
///   상태이고, 그때는 더 받아 봐야 같은 일을 여덟 번 더 줄 세울 뿐이다.
/// ★여기서는 기다린다(위 줄과 다르다)★: 이 줄이 찼을 때 읽기를 멈추면 굶는 것은 **도착순 줄로 갈
///   프레임**이고, 그것들은 어차피 거절되던 참이다. 즉 이 자리의 대기는 지켜야 할 것을 굶기지 않는다.
/// ★바이트 예산이 여기엔 **걸리지 않는다**★: 위층이 「굶으면 안 된다」고 고른 프레임을 예산으로
///   거절하면 이 줄의 존재 이유가 사라진다. 그래서 이 줄의 천장은 칸 수 그대로이고, 그 값은
///   8 × max_message_size 다([`CONN_RX_MAX_BYTES`] 가 적은 둘째 항과 같은 항). 정상 운용에서 이 줄이
///   비어 있다는 성질이 그 천장을 실효 0 으로 만든다 — 보장이 아니라 관측이므로 그대로 적어 둔다.
const CONN_BYPASS_CAP: usize = 8;

/// 연결 정리에서 우선 줄이 **받아 둔 것을 비울** 때까지 기다리는 유예([`finish_bypass_lane`]).
///
/// ★이 값이 정확성을 지지는 **않는다**★ — 정확성은 「기다린다」 자체가 지고, 이 값은 그 기다림이
///   무한이 되지 않게 하는 상한일 뿐이다. 유예를 넘기면 옛 동작(abort)으로 떨어지므로, 이 상수가
///   틀려도 **이전보다 나쁠 수는 없다**.
/// ★크기의 근거★: 이 줄의 한 건은 위층에서 「막힌 것을 끝내는」 동작이고, 그런 동작은 대상이 죽기를
///   동기로 기다리는 구간을 갖는다(오늘 그 구간의 상한은 위층의 5초 join 이다). 10초면 진행 중 한 건
///   + 다음 한 건이 끝나고, 그보다 더 밀려 있다면 그 경로 자체가 막힌 상태라 더 기다려도 같은 일을
///   줄 세울 뿐이다([`CONN_BYPASS_CAP`] 의 「8칸이 차 있다는 것은」과 같은 판단).
/// ★이 유예 동안 무엇이 붙들리나★ = 그 **연결 하나의 정리**뿐이다. 소켓은 이미 끝났고, accept 루프도
///   다른 연결도 이 대기에 걸리지 않는다.
const BYPASS_DRAIN_GRACE: Duration = Duration::from_secs(10);

const AUTH_TIMEOUT: Duration = Duration::from_secs(1);

const DEFAULT_PING_INTERVAL: Duration = Duration::from_secs(20);
/// ping_interval 의 2.5배(여러 Ping 을 놓쳐야 끊김 — 일시 지연 위양성 방지).
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(50);

/// WS application-level keepalive 설정(A).
///
/// ★half-open 감지★: tungstenite 는 들어온 Ping 에 자동 Pong 만 하고 능동 Ping 은 안 보낸다.
/// FIN 없이 끊기는 연결(sleep/wake·NAT 타임아웃·모바일 터널)에서 TCP keepalive(기본 2시간)는
/// 무의미하므로, write_task 가 ping_interval 마다 Ping 을 보내고 read_task 가 마지막 수신 시각을
/// 기록한다. idle_timeout 초과면 그 연결을 끊는다(좀비 구독/broadcast 누수 방지).
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

/// 허용 Origin allowlist. Origin 없음(네이티브/하네스)은 허용 — 토큰이 주 방어.
/// ★미실측 — 알려진 미확인★: 아래 네 문자열은 **설계값**이다. 실제 Tauri WebView2·모바일이 보내는
///   Origin 을 실측해 확정한 적이 없다. 불일치 = 403. 클라이언트 표면(패키징 변형·모바일 터널 등)을
///   늘릴 때 이 목록을 검증된 것으로 다루지 말 것 — 403 은 클라 버그가 아니라 이 목록 쪽일 수 있다.
const ALLOWED_ORIGINS: &[&str] = &[
    "http://localhost:1420",
    "http://127.0.0.1:1420",
    "tauri://localhost",
    "https://tauri.localhost",
];

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

impl FrameFanout for ConnRegistry {
    /// ★맵을 잠근 채 보내지 않는다(ADR-0006 락 순서)★: 스냅샷을 뜬 뒤 락을 놓고 그 사본으로 send 한다
    ///   — 락 보유 중 외부(채널)로 나가는 호출을 만들지 않기 위해서다.
    fn broadcast_text(&self, text: String) {
        let conns: Vec<(ConnId, mpsc::Sender<Frame>)> = {
            let guard = self.inner.lock().expect("conn registry poisoned");
            guard.iter().map(|(id, tx)| (*id, tx.clone())).collect()
        };
        for (id, tx) in conns {
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

pub(crate) struct ConnFrameSink {
    conn_tx: mpsc::Sender<Frame>,
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
                tracing::debug!("WS upgrade: Origin 없음 — 허용(토큰 검증으로 방어)");
                Ok(response)
            }
            Some(value) => {
                let origin = value.to_str().unwrap_or("");
                if ALLOWED_ORIGINS.contains(&origin) {
                    tracing::debug!(origin, "WS upgrade: Origin 허용");
                    Ok(response)
                } else {
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

/// 길이 노출은 토큰 길이가 고정(hex 64자)이라 무해.
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

/// `expected_token` 은 daemon.json 의 토큰. `handlers` 는 이 연결에 붙일 위층 핸들러 공장 —
/// 프레임의 의미(명령 해석·이벤트 인코딩·연결 정리)는 전부 그쪽이 소유하므로, 이 함수는 에이전트
/// 어휘를 알지 못한다(ADR-0129).
///
/// ★`expected_protocol_version` 도 **주입**이다(0-4)★: 토큰과 같은 결로 조립부가 값을 넣어 준다.
/// 그래서 이 crate 는 "지금 버전이 몇" 을 모르고 **"클라가 말한 숫자가 내가 받은 숫자와 같은가"** 만
/// 판정한다 — 버전 상수를 여기서 읽으면 그 순간 프로토콜 어휘가 네트워크 행으로 돌아온다.
pub async fn handle_connection(
    stream: TcpStream,
    peer: std::net::SocketAddr,
    registry: ConnRegistry,
    handlers: Arc<dyn ConnectionHandlerFactory>,
    expected_token: Arc<String>,
    expected_protocol_version: u32,
    keepalive: KeepaliveConfig,
) {
    let mut ws = match tokio_tungstenite::accept_hdr_async(stream, OriginCheck).await {
        Ok(ws) => ws,
        Err(e) => {
            tracing::warn!(%peer, "WS 업그레이드 실패(또는 Origin 거부): {e}");
            return;
        }
    };

    match tokio::time::timeout(AUTH_TIMEOUT, ws.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => {
            match serde_json::from_str::<AuthFrame>(&text) {
                Ok(AuthFrame::Auth {
                    token,
                    protocol_version,
                }) => {
                    // 보안: 토큰 값은 로그 금지.
                    if !constant_time_eq(&token, expected_token.as_str()) {
                        tracing::warn!(%peer, "auth 실패: 토큰 불일치 — close");
                        let _ = send_error_and_close(
                            &mut ws,
                            handlers.handshake_error_frame("auth failed"),
                        )
                        .await;
                        return;
                    }
                    if protocol_version != expected_protocol_version {
                        tracing::warn!(
                            %peer,
                            client = protocol_version,
                            server = expected_protocol_version,
                            "auth 실패: protocol_version 불일치 — close"
                        );
                        let _ = send_error_and_close(
                            &mut ws,
                            handlers.handshake_error_frame(&format!(
                                "protocol_version mismatch: client {protocol_version} != server {expected_protocol_version}"
                            )),
                        )
                        .await;
                        return;
                    }
                    tracing::info!(%peer, "auth 성공");
                }
                // ★두 실패 문구의 경계★: 이 crate 는 위층 명령을 모르므로 "위층 명령으로는 파싱되지만
                //   auth 가 아님" 을 판정하지 못한다 —
                //   대신 serde 가 이미 계산해 둔 에러 분류를 쓴다(재파싱 0):
                //     · data 에러  = JSON 은 맞는데 이 모양이 아님 → "auth 를 기대했다"
                //     · 그 밖(syntax/eof/io) = 애초에 JSON 이 아님 → "프레임 자체가 잘못됐다"
                //   이 문구를 단언하는 테스트는 없고(핸드셰이크 실패는 close 로 관측된다) 발신자 중
                //   어느 것도 문구로 분기하지 않는다 — 사람이 읽는 진단 텍스트다.
                // ★에러를 `{e}` 로 싣지 않는다★: `{"Auth":"<토큰>"}` 의 Display 는 그 문자열을 그대로
                //   내놓아 토큰이 로그로 샌다(`docs/reference/logging-conventions.md`). 분류와 위치만
                //   남긴다 — 회귀 방어는 아래 `auth_parse_failure_log_fields_do_not_carry_the_payload`.
                Err(e) if e.is_data() => {
                    tracing::warn!(
                        %peer,
                        kind = ?e.classify(),
                        line = e.line(),
                        column = e.column(),
                        "첫 frame 이 Auth 가 아님 — close"
                    );
                    let _ = send_error_and_close(
                        &mut ws,
                        handlers.handshake_error_frame("expected Auth as first frame"),
                    )
                    .await;
                    return;
                }
                // 위 갈래와 같은 이유로 Display 를 싣지 않는다(토큰이 이 갈래로도 올 수 있다).
                Err(e) => {
                    tracing::warn!(
                        %peer,
                        kind = ?e.classify(),
                        line = e.line(),
                        column = e.column(),
                        "첫 frame 파싱 실패 — close"
                    );
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

    let (conn_tx, conn_rx) = mpsc::channel::<Frame>(CONN_TX_CAP);
    let close_signal = Arc::new(Notify::new());
    let conn_id = registry.alloc_id();
    registry.register(conn_id, conn_tx.clone());
    tracing::info!(%peer, conn = conn_id, "연결 인증 완료 — 등록");

    let (sink_half, stream_half) = ws.split();

    let frames: Arc<dyn FrameSink> =
        Arc::new(ConnFrameSink::new(conn_tx.clone(), close_signal.clone()));
    let handler = handlers.handler_for(conn_id);

    // 순서상 여기가 두 task 스폰보다 앞이어야 명령 dispatch 가 인사보다 앞설 수 없다.
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

    // ★수신 큐 = 읽기 행과 처리 행의 경계★: 읽기는 소켓에서 꺼내 여기 넣기만 하고, 핸들러 호출은
    //   [`dispatch_task`] 가 도착 순서대로 진다. 용량과 포화 처분의 근거는 [`CONN_RX_CAP`].
    let (inbound_tx, inbound_rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
    // ★칸 수와 **함께** 포화를 정하는 바이트 예산의 물높이★ — 근거 정본 = [`CONN_RX_MAX_BYTES`].
    let rx_bytes: RxBytes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    // ★막힌 줄 뒤에서 굶으면 안 되는 프레임을 위한 두 번째 줄★ — 왜 별도 용량·별도 소비자여야
    //   하는지는 [`CONN_BYPASS_CAP`] 과 [`bypass_task`] 가 각각 갖는다.
    let (bypass_tx, bypass_rx) = mpsc::channel::<Inbound>(CONN_BYPASS_CAP);

    let read_handle = tokio::spawn(read_task(
        stream_half,
        inbound_tx,
        bypass_tx,
        rx_bytes.clone(),
        frames.clone(),
        handler.clone(),
        conn_id,
        keepalive_base,
        last_recv.clone(),
    ));

    let bypass_handle = tokio::spawn(bypass_task(
        bypass_rx,
        frames.clone(),
        handler.clone(),
        conn_id,
    ));

    // ★프레임 출구의 강참조가 read_task 에서 이 task 로 옮겨 갔다★ — `the_losing_task_is_aborted_not_detached`
    //   의 관측이 그 사실 위에 서 있다(그 테스트 주석).
    let mut dispatch_handle = tokio::spawn(dispatch_task(
        inbound_rx,
        rx_bytes,
        frames,
        handler.clone(),
        conn_id,
    ));

    let mut write_handle = tokio::spawn(write_task(
        sink_half,
        conn_rx,
        conn_id,
        close_signal,
        keepalive,
        keepalive_base,
        last_recv,
    ));

    // ★살아남은 쪽을 명시적으로 abort★ — JoinHandle 을 그냥 drop 하면
    //    task 가 detach 되어 계속 돈다(WS half 를 붙든 채 좀비). 그래서 &mut 로 select 해 핸들을
    //    소비하지 않고, 진 쪽을 abort 한다(연결의 read/write 가 함께 끝나게).
    //    회귀 방어 = `the_losing_task_is_aborted_not_detached`(read 를 abort 하는 갈래만).
    //    ★write 를 abort 하는 갈래도 abort 를 제거하지 말 것★: 대개는 송신단이 모두 드롭돼 write_task 가
    //    스스로 끝나지만, 그건 "모든 Sender<Frame> 사본이 함께 죽는다" 는 조건부다 — 구독 기록 누락
    //    (아래 on_disconnect 경쟁)으로 사본이 살아남으면 자기종료가 성립하지 않는다. 전수 열거와 실측
    //    범위는 그 테스트 주석에 있다.
    // ★★task 는 셋인데 select 대상이 **둘**인 것은 의도다 — read_task 는 여기 안 든다★★:
    //    읽기 루프가 끝나는 것(클라 close·EOF·수신 오류)은 「입력이 끝났다」는 뜻이지 「이 연결을 그
    //    자리에서 끊으라」가 아니다. 끝나면 수신 큐의 **유일한 송신단**이 드롭되므로 dispatch_task 가
    //    남은 것을 처리한 **뒤** 스스로 끝나고, 그 종료를 아래 첫 갈래가 잡는다. read 를 여기 넣어 즉시
    //    abort 하면 「명령 한 장 보내고 곧바로 소켓을 닫는」 클라의 마지막 명령이 처리 전에 잘린다 —
    //    읽기와 처리가 한 줄이던 HEAD 에서는 그 명령이 **항상** 끝난 뒤에야 close frame 을 읽었으므로
    //    그건 회귀다.
    //    ★대가 = 핸들러가 영영 반환하지 않으면 이 연결은 정리되지 않는다★. HEAD 도 같았다(그때는 읽기
    //    루프가 `on_text` 안에 파킹된 채 남았다) — 이 배치가 새로 만든 위험이 아니다.
    //    ★우선 줄(bypass_task)도 select 대상이 아니다★ — 그것이 끝나는 것은 read 가 끝났다는 뜻일
    //    뿐이고(자기 송신단이 read 에 있다), 연결을 끊을 사유가 아니다. 그쪽이 낸 종료 의사는
    //    `Frame::Close` 로 write 줄을 타고 아래 둘째 갈래로 돌아온다(그 task 주석).
    //    ★★우선 줄은 **abort 하지 않는다 — 비운다**★★: 그 줄에 든 프레임은 위층이 「굶으면 안 된다」고
    //    골라 **받아 준** 것이다. 여기서 abort 하면 받아 놓고 안 하는 것이 되어, 이 줄을 둔 이유가
    //    통째로 무너진다(도착순 줄에는 그런 구멍이 없다 — 송신단이 드롭되면 남은 것을 비우고 끝난다).
    //    그래서 read 를 먼저 끊어 **그 줄의 유일한 송신단**을 놓고, [`finish_bypass_lane`] 이 그
    //    비움을 기다린다.
    tokio::select! {
        _ = &mut dispatch_handle => {
            tracing::debug!(conn = conn_id, "dispatch_task 종료 → read abort · bypass 비움 · write abort + cleanup");
            read_handle.abort();
            finish_bypass_lane(bypass_handle, conn_id).await;
            write_handle.abort();
        }
        _ = &mut write_handle => {
            tracing::debug!(conn = conn_id, "write_task 종료 → read abort · bypass 비움 · dispatch abort + cleanup");
            read_handle.abort();
            finish_bypass_lane(bypass_handle, conn_id).await;
            dispatch_handle.abort();
        }
    }

    // ── cleanup(누수 방지 — 리뷰 필수) ──────────────────────────────────────────
    handler.on_disconnect(conn_id);

    registry.unregister(conn_id);
    tracing::info!(%peer, conn = conn_id, "연결 종료 — cleanup 완료");
}

/// 핸드셰이크 실패 통보 + close 를 소켓에 직접 쓴다(레지스트리 등록 전이라 단일 writer 큐가 없다).
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
    let mut ping_tick = tokio::time::interval(keepalive.ping_interval);
    // 첫 tick 즉발 방지(연결 직후 바로 Ping 쏘지 않게) — 정상 첫 주기부터.
    ping_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = close_signal.notified() => {
                tracing::info!(conn = conn_id, "write_task: close_signal(슬로우 소비자) — 종료");
                let _ = sink_half.close().await;
                break;
            }
            _ = ping_tick.tick() => {
                let now_ms = keepalive_base.elapsed().as_millis() as u64;
                let last_ms = last_recv.load(Ordering::Acquire);
                let idle = Duration::from_millis(now_ms.saturating_sub(last_ms));
                if idle >= keepalive.idle_timeout {
                    tracing::info!(
                        conn = conn_id,
                        idle_ms = idle.as_millis() as u64,
                        "write_task: keepalive idle_timeout 초과(half-open 추정) — 종료"
                    );
                    let _ = sink_half.close().await;
                    break;
                }
                if let Err(e) = sink_half.send(Message::Ping(Vec::new().into())).await {
                    tracing::debug!(conn = conn_id, "write_task keepalive Ping 송신 실패 — 종료: {e}");
                    break;
                }
            }
            recv = conn_rx.recv() => {
                let Some(out) = recv else {
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

// ── read_task + dispatch_task(수신 큐로 갈라진 두 행) ─────────────────────────────

/// 읽기 루프가 디코드해 수신 큐로 넘기는 단위.
///
/// ★나가는 쪽 [`Frame`] 과 달리 Close 가 없다★: 나가는 Close 는 위층이 큐에 넣는 마커지만, 들어오는
///   쪽의 close 판정은 프레임 **내용**이 아니라 핸들러의 답(`ConnFlow::Close`)이 낸다 — 그래서 이
///   어휘에는 그 모양이 없다.
/// ★페이로드를 **복사하지 않는다**★: tungstenite 가 준 payload 타입을 그대로 옮긴다(둘 다 refcount
///   핸들이라 이동이 값싸다). `String`/`Vec<u8>` 으로 바꾸면 **클라가 고른 크기만큼** 프레임마다
///   복사가 생긴다 — 옛 `on_binary` 가 페이로드를 빌려주던 것과 같은 축의 이유다.
enum Inbound {
    Text(Utf8Bytes),
    Binary(Bytes),
}

impl Inbound {
    /// 이 프레임이 줄에서 붙들고 있는 페이로드 바이트 — [`CONN_RX_MAX_BYTES`] 회계의 단위.
    ///
    /// ★핸들 자체의 크기가 아니라 **페이로드** 길이다★: 둘 다 refcount 핸들이라 `size_of` 는 몇십
    ///   바이트로 똑같고, 실제로 메모리를 무는 것은 그 뒤의 버퍼다.
    fn payload_len(&self) -> usize {
        match self {
            Inbound::Text(t) => t.len(),
            Inbound::Binary(b) => b.len(),
        }
    }
}

/// 도착순 줄이 지금 붙들고 있는 페이로드 바이트 — 생산자(`read_task`)가 더하고 소비자
/// (`dispatch_task`)가 뺀다. 한 연결에 하나.
///
/// ★빼는 자리가 **핸들러 호출 뒤**인 것은 의도다★: 꺼내자마자 빼면 「꺼냈지만 아직 처리 중인」 프레임이
///   0 으로 세어져, 예산이 실제로는 두 배가 된다(같은 축의 실수 = `input_queue` 의 in-flight 회계).
type RxBytes = Arc<std::sync::atomic::AtomicUsize>;

/// ★stream 이 generic 인 이유★: 소켓 없이 합성 프레임열로 이 루프를 돌리는 격리 하네스를 두려고
/// (ADR-0129). 운영 경로는 WS stream half 로만 단형화된다.
///
/// ★핸들러의 일을 **기다리지 않는다**★: 디코드한 프레임을 수신 큐에 넘기고 곧바로 다음 프레임을
/// 읽는다. 옛 모양(여기서 `on_text` 를 완료까지 await)에서는 한 명령의 처리 시간이 곧 그 연결
/// **전체**의 정지 시간이었다 — 에이전트 활성화 하나가 다른 에이전트로 가는 입력·kill 까지 소켓에서
/// 꺼내지지도 않게 했다. 처리는 [`dispatch_task`] 가 도착 순서대로 진다.
// ADR-0206
#[allow(clippy::too_many_arguments)]
async fn read_task<S>(
    mut incoming: S,
    inbound_tx: mpsc::Sender<Inbound>,
    bypass_tx: mpsc::Sender<Inbound>,
    rx_bytes: RxBytes,
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
        // ★keepalive(A) 갱신이 **디스패치보다 앞**인 것은 그대로다★: 이 줄은 옛 모양에서도 핸들러
        //   호출 앞이었으므로, 처리를 다른 task 로 옮겨도 keepalive 판정은 달라지지 않는다.
        //   tungstenite 는 Pong 을 Message::Pong 으로 올려주므로 능동 Ping 의 응답도 여기서 잡힌다.
        last_recv.store(
            keepalive_base.elapsed().as_millis() as u64,
            Ordering::Release,
        );
        let queued = match msg {
            Message::Text(text) => Inbound::Text(text),
            Message::Binary(payload) => Inbound::Binary(payload),
            // Ping/Pong 은 tungstenite 가 자동 응답(write_task 가 아닌 내부). 여기선 무시.
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => {
                tracing::debug!(conn = conn_id, "Close frame 수신 — 종료");
                break;
            }
            Message::Frame(_) => continue,
        };
        // ★칸보다 **바이트 예산이 먼저 걸릴 수 있다**★ — 판정은 더하기 전 물높이로 한다(빈 줄에는
        //   크기와 무관하게 한 장은 들어간다). 근거 정본 = [`CONN_RX_MAX_BYTES`].
        let over_budget = rx_bytes.load(Ordering::Acquire) >= CONN_RX_MAX_BYTES;
        // ★`try_send` 다 — 여기서 기다리면 그 뒤 프레임을 **소켓에서 꺼내지도 못한다**★(근거 정본 =
        //   [`CONN_RX_CAP`]). 자리가 있으면 이게 전부이고(정상 경로 비용 0), 없을 때만 아래로 간다.
        let full = if over_budget {
            queued
        } else {
            let len = queued.payload_len();
            match inbound_tx.try_send(queued) {
                Ok(()) => {
                    rx_bytes.fetch_add(len, Ordering::AcqRel);
                    continue;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    tracing::debug!(conn = conn_id, "수신 큐 소비자 없음 — read_task 종료");
                    break;
                }
                // 넣지 못한 프레임을 **되돌려 받는다** — 이게 있어야 조용한 유실 없이 처분을 고를 수 있다.
                Err(mpsc::error::TrySendError::Full(item)) => item,
            }
        };

        // ★줄이 찼다 — 이 한 프레임의 처분은 위층이 정한다★(어휘를 아는 쪽이 위층뿐이다).
        //   binary 는 묻지 않는다: 이 프로토콜에서 클라→데몬 binary 는 그 자체가 위반이라 위층이 어차피
        //   연결을 닫으며, 그 판정에 줄 순서가 걸려 있지 않다.
        let disposition = match &full {
            Inbound::Text(text) => handler.on_inbound_saturated(conn_id, text.as_str(), &frames),
            Inbound::Binary(_) => Saturated::Bypass,
        };
        match disposition {
            Saturated::Refused => {
                // 위층이 답장을 냈다 — 버리고 계속 읽는다. 「계속 읽는다」가 이 갈래의 존재 이유다.
                tracing::debug!(
                    conn = conn_id,
                    "수신 줄 포화 — 위층이 거절했다(계속 읽는다)"
                );
            }
            // ★거절조차 못 냈다 — 「버리고 계속 읽는다」의 근거가 사라졌으므로 끝낸다★(근거 정본 =
            //   [`Saturated::Unanswered`]). 읽기만 끝내면 도착순 줄은 이미 받아 둔 것을 마저 처리한 뒤
            //   스스로 끝나고, 그 종료를 `handle_connection` 이 잡아 정리한다.
            Saturated::Unanswered => {
                tracing::warn!(
                    conn = conn_id,
                    "수신 줄 포화 — 거절 답장조차 큐에 못 넣었다(송신 큐도 포화) — 읽기를 끝낸다"
                );
                break;
            }
            // ★여기서는 기다려도 된다★ — 근거는 [`CONN_BYPASS_CAP`].
            Saturated::Bypass => {
                if bypass_tx.send(full).await.is_err() {
                    tracing::debug!(conn = conn_id, "우선 줄 소비자 없음 — read_task 종료");
                    break;
                }
            }
        }
    }
    tracing::debug!(conn = conn_id, "read_task 루프 종료");
}

/// 수신 큐의 **단일** 소비자 — 한 연결의 프레임을 도착 순서대로, 서로 겹치지 않게 핸들러에 올린다.
///
/// ★이 직렬성이 위층 순서 불변식의 **실물**이다★: 「같은 에이전트로 가는 입력끼리」·「입력 lease
/// 획득과 그 뒤 입력」·「같은 연결의 구독/해지」·「명령 명부 등록과 그 차분」이 전부 도착 순서에
/// 걸려 있고, 그것을 지키는 것은 이 task 가 하나이고 한 번에 하나만 await 한다는 성질뿐이다
/// (계약의 정본 = `frame_port::ConnectionHandler`). **두 번째 소비자를 띄우거나 여기서 spawn 으로
/// 흩으면 그 넷이 한꺼번에 깨진다** — 오래 걸리는 명령을 줄에서 떼는 판단은 위층 몫이다.
async fn dispatch_task(
    mut inbound_rx: mpsc::Receiver<Inbound>,
    rx_bytes: RxBytes,
    frames: Arc<dyn FrameSink>,
    handler: Arc<dyn ConnectionHandler>,
    conn_id: ConnId,
) {
    while let Some(item) = inbound_rx.recv().await {
        let len = item.payload_len();
        let flow = match item {
            Inbound::Text(text) => handler.on_text(conn_id, text.as_str(), &frames).await,
            // 페이로드는 **빌려준다** — 거부 경로가 유일한 소비자라 복사하지 않는다(클라가 고른
            //   크기만큼 할당하게 두면 인증 후 최대 프레임 크기까지 낭비 할당이 된다).
            Inbound::Binary(payload) => handler.on_binary(conn_id, payload.as_ref(), &frames).await,
        };
        // ★핸들러가 반환한 **뒤**에 뺀다★ — 처리 중인 한 장을 0 으로 세면 예산이 두 배가 된다
        //   ([`RxBytes`]). 핸들러가 panic 하면 이 줄에 못 오지만, 그때는 이 task 자체가 죽어 연결이
        //   통째로 정리되므로 새는 예산이 남을 곳이 없다.
        rx_bytes.fetch_sub(len, Ordering::AcqRel);
        if flow == ConnFlow::Close {
            tracing::debug!(conn = conn_id, "핸들러가 Close — dispatch_task 종료");
            break;
        }
    }
    tracing::debug!(conn = conn_id, "dispatch_task 루프 종료");
}

/// 도착순 줄을 건너뛴 프레임([`Saturated::Bypass`])의 소비자.
///
/// ★★[`dispatch_task`] 와 **별도 task 여야 한다** — 같은 task 의 `select!` 로 합칠 수 없다★★: 이 줄이
/// 존재하는 상황은 곧 **그쪽 소비자가 막혀 있는** 상황이다. 막힌 task 는 아무것도 폴링하지 못하므로,
/// 한 task 안에 우선순위를 두는 형태로는 이 프레임이 영영 안 돈다.
///
/// ★그래서 두 줄은 **겹쳐서 돈다**★ — 포트 계약의 「겹치지 않는다」에 대한 유일한 예외이고, 그 예외를
/// 고른 것은 위층 자신이다(`on_inbound_saturated`). 여기서 도는 것과 저기서 막혀 있는 것이 같은 대상을
/// 건드릴 수 있다는 뜻이므로, 그래도 되는 프레임만 고르는 책임도 위층에 있다.
///
/// ★`Close` 를 큐 안 마커로 돌린다★: 이 task 는 `handle_connection` 의 select 대상이 **아니다**(그랬다면
/// 읽기가 끝나 이 줄이 닫히는 순간 teardown 이 시작돼, 도착순 줄이 남은 것을 비울 기회를 잃는다).
/// 그래서 종료 의사는 `Frame::Close` 로 write 줄에 실어 보낸다 — 앞서 넣은 프레임이 먼저 나간 뒤 닫힌다.
async fn bypass_task(
    mut bypass_rx: mpsc::Receiver<Inbound>,
    frames: Arc<dyn FrameSink>,
    handler: Arc<dyn ConnectionHandler>,
    conn_id: ConnId,
) {
    while let Some(item) = bypass_rx.recv().await {
        let flow = match item {
            Inbound::Text(text) => handler.on_text(conn_id, text.as_str(), &frames).await,
            Inbound::Binary(payload) => handler.on_binary(conn_id, payload.as_ref(), &frames).await,
        };
        if flow == ConnFlow::Close {
            tracing::debug!(
                conn = conn_id,
                "우선 줄에서 Close — write 줄로 종료를 넘긴다"
            );
            let _ = frames.try_send(Frame::Close("bypass lane requested close".into()));
            break;
        }
    }
    tracing::debug!(conn = conn_id, "bypass_task 루프 종료");
}

/// 우선 줄을 **거둔다** — `abort()` 가 아니라, 남은 것을 비우고 스스로 끝나기를 기다린다.
///
/// ★왜 abort 가 결함이었나★: 우선 줄에 들어간 프레임은 위층이 「이건 굶으면 안 된다」고 판정해
///   **받아 준** 것이다(`on_inbound_saturated` → [`Saturated::Bypass`]). 그런데 도착순 줄이 먼저
///   끝나면(피어가 명령 한 장 보내고 곧바로 close 하는 흔한 모양) 그 자리에서 이 task 를 abort 했고,
///   **받아 놓고 한 번도 폴링되지 않은 프레임이 그대로 사라졌다.** 이 줄을 둔 이유가 통째로 무너지는
///   갈래다 — 도착순 줄에는 같은 구멍이 없다(송신단이 드롭되면 남은 것을 비운 뒤 끝난다).
///
/// ★부르는 쪽의 의무 = **먼저 `read_task` 를 끊을 것**★: 이 줄의 유일한 송신단이 거기 있어서, 그것이
///   드롭되지 않으면 `recv()` 가 영영 `None` 을 못 받아 이 대기가 유예를 다 쓴다.
/// ★유예를 넘기면 옛 처분으로 떨어진다★ — 그래서 이 함수는 **손해가 없다**: 잘 되면 받아 둔 것을
///   마저 하고, 안 되면 예전과 똑같이 abort 한다. ★`timeout` 에 핸들을 **넘기지 않고 빌려주는 것은
///   의도다**★: 넘기면 시한 초과 시 핸들이 drop 되어 task 가 abort 가 아니라 **detach** 되고, 그건
///   `handle_connection` 이 명시적 abort 로 막고 있는 바로 그 누수다.
async fn finish_bypass_lane(mut handle: tokio::task::JoinHandle<()>, conn_id: ConnId) {
    match tokio::time::timeout(BYPASS_DRAIN_GRACE, &mut handle).await {
        Ok(_) => {}
        Err(_) => {
            tracing::warn!(
                conn = conn_id,
                grace_ms = BYPASS_DRAIN_GRACE.as_millis() as u64,
                "우선 줄이 유예 안에 안 비워졌다 — abort(받아 둔 것이 남아 있으면 유실된다)"
            );
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    // ── 2. 토큰 상수시간 비교 정확성 ──────────────────────────────────────────
    #[test]
    fn constant_time_eq_correctness() {
        let a = "a".repeat(64);
        assert!(constant_time_eq(&a, &"a".repeat(64)), "동일 토큰은 true");
        assert!(!constant_time_eq(&a, &"b".repeat(64)), "다른 토큰은 false");
        assert!(!constant_time_eq(&a, &"a".repeat(63)));
        assert!(!constant_time_eq(&a, &"a".repeat(65)));
        let mut almost = "a".repeat(64);
        almost.replace_range(63..64, "b");
        assert!(!constant_time_eq(&a, &almost));
        assert!(constant_time_eq("", ""));
    }

    // ── 2b. auth 파싱 실패 로그에 페이로드가 실리지 않는다 ─────────────────────
    #[test]
    fn auth_parse_failure_log_fields_do_not_carry_the_payload() {
        let token = "deadbeef".repeat(8); // 운영과 같은 64자
        let e = serde_json::from_str::<AuthFrame>(&format!(r#"{{"Auth":"{token}"}}"#)).unwrap_err();
        assert!(
            e.to_string().contains(&token),
            "Display 가 토큰을 싣지 않으면 이 테스트가 지키는 게 없다: {e}"
        );
        let logged = format!("{:?} {} {}", e.classify(), e.line(), e.column());
        assert!(
            !logged.contains(&token),
            "로깅 필드에 토큰이 실렸다: {logged}"
        );
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
        assert!(
            tokio::time::timeout(Duration::from_millis(50), close_signal.notified())
                .await
                .is_err(),
            "send 는 포화를 기다릴 뿐 종료 신호를 울리지 않는다"
        );

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

            assert!(
                matches!(full_rx.try_recv(), Ok(Frame::Text(s)) if s == "선점"),
                "round {round}: 포화 연결엔 선점 프레임만 있어야"
            );
            assert!(
                full_rx.try_recv().is_err(),
                "round {round}: 포화 연결은 이번 건을 못 받는다"
            );
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

    // ── 10. (적용4-1) OriginCheck::on_request 분기 — 무방비 였던 거부/허용 분기 검증 ──────
    //    순수 헤더 검사라 in-process 서버 불필요. Request 를 직접 만들어 콜백을 호출한다.
    fn run_origin_check(origin: Option<&str>) -> Result<(), ()> {
        use tokio_tungstenite::tungstenite::http::Request as HttpRequest;
        let mut builder = HttpRequest::builder().uri("/");
        if let Some(o) = origin {
            builder = builder.header("origin", o);
        }
        let request = builder.body(()).unwrap();
        // Response 는 콜백이 통과시키는 더미.
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
        assert!(run_origin_check(Some("tauri://localhost")).is_ok());
        assert!(run_origin_check(Some("http://localhost:1420")).is_ok());
    }

    #[test]
    fn origin_check_rejects_unlisted_origin() {
        assert!(run_origin_check(Some("http://evil.example.com")).is_err());
    }

    #[test]
    fn origin_check_allows_missing_origin() {
        assert!(run_origin_check(None).is_ok());
    }

    // ── 11. 프레임 포트 seam — 소켓 없이 도는 격리 하네스(ADR-0129) ──────────────────
    //    가짜 ConnectionHandler + 가짜 FrameSink 로 연결 수명을 재현한다. TcpStream 이 없어야
    //    네트워크 행이 별도 crate 로 떨어져도 이 검증이 그대로 산다.

    #[derive(Debug, PartialEq, Eq)]
    enum SeenFrame {
        Text(String),
        Binary(Vec<u8>),
        Close(String),
    }

    #[derive(Default)]
    struct FakeFrameSink {
        frames: Mutex<Vec<SeenFrame>>,
        /// 켜면 모든 `try_send`/`send` 가 실패한다 — 「송신 큐도 포화」를 시한 없이 만드는 손잡이.
        refuse: AtomicBool,
    }

    impl FakeFrameSink {
        /// 한 프레임도 받아 주지 않는 출구(= 그 연결의 송신 큐가 포화이거나 닫힌 상태의 대역).
        fn refusing() -> Self {
            Self {
                refuse: AtomicBool::new(true),
                ..Self::default()
            }
        }

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
            if self.refuse.load(Ordering::Acquire) {
                return Err(FrameError);
            }
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
        /// 이 텍스트를 받으면 [`Self::slow_gate`] 가 열릴 때까지 `on_text` **안에서** 대기한다 —
        /// 「핸들러가 한 명령에 붙들려 있는 동안 읽기 루프가 앞서 나가는가」를 재는 창을 만든다.
        /// ★타이밍 가정을 쓰지 않으려는 장치다★: sleep 으로 흉내 내면 느린 러너에서 창이 닫혀 위양성이
        ///   난다. 게이트는 테스트가 열기 전까지 **영원히** 닫혀 있다.
        slow_on: Option<String>,
        slow_gate: Arc<Notify>,
        /// [`Self::slow_on`] 의 **우선 줄 판**: 이 텍스트를 받으면 [`Self::bypass_gate`] 가 열릴 때까지
        /// `on_text` 안에서 대기한다. 도착순 줄과 **따로** 잠가야 「도착순은 끝났는데 우선 줄은 아직
        /// 한 건에 붙들려 있다」는 상태를 만들 수 있다(그 상태가 곧 abort 결함의 무대다).
        bypass_gate_on: Option<String>,
        bypass_gate: Arc<Notify>,
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
                slow_on: None,
                slow_gate: Arc::new(Notify::new()),
                bypass_gate_on: None,
                bypass_gate: Arc::new(Notify::new()),
                frames_weak: Mutex::new(None),
                registry: None,
                registered_at_disconnect: Mutex::new(None),
            }
        }

        /// 한 명령에 붙들리는 변종 — `slow_gate` 를 열 때까지 그 `on_text` 이 반환하지 않는다.
        fn blocking_on(slow_on: &str) -> Self {
            Self {
                slow_on: Some(slow_on.to_string()),
                ..Self::new(None)
            }
        }

        /// 두 줄을 **각각** 잠그는 변종 — 도착순 줄은 `slow_on` 에, 우선 줄은 `bypass_gate_on` 에.
        fn blocking_on_both(slow_on: &str, bypass_gate_on: &str) -> Self {
            Self {
                bypass_gate_on: Some(bypass_gate_on.to_string()),
                ..Self::blocking_on(slow_on)
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
                if self.slow_on.as_deref() == Some(text) {
                    self.slow_gate.notified().await;
                }
                if self.bypass_gate_on.as_deref() == Some(text) {
                    self.bypass_gate.notified().await;
                }
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

        /// 이 fake 는 "bypass:" 로 시작하는 text 만 줄을 건너뛰게 한다. 나머지는 거절하고, 거절도
        /// **기록해** 조용한 유실과 구별한다(포트 계약의 의무 2 에 해당하는 이 fake 의 답장).
        ///
        /// ★답장 결과를 보고 [`Saturated::Unanswered`] 로 갈리는 것까지 운영 핸들러와 같은 모양이다★ —
        ///   결과를 버리고 늘 `Refused` 를 돌려주면 이 fake 는 그 갈래를 **표현조차 못 한다.**
        fn on_inbound_saturated(
            &self,
            _conn_id: ConnId,
            text: &str,
            frames: &Arc<dyn FrameSink>,
        ) -> Saturated {
            if text.starts_with("bypass:") {
                return Saturated::Bypass;
            }
            match frames.try_send(Frame::Text(format!("refused:{text}"))) {
                Ok(()) => Saturated::Refused,
                Err(_) => Saturated::Unanswered,
            }
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

    /// 읽기 행과 처리 행을 **실제 배치대로**(수신 큐로 이어) 함께 돌린다 — `handle_connection` 이
    /// 하는 배선의 최소 재현이고, 둘 다 끝난 뒤 반환한다.
    ///
    /// ★소켓 없이 도는 격리 하네스라는 성질은 그대로다★(ADR-0129) — 바뀐 것은 이 seam 을 태우는 데
    ///   task 가 하나가 아니라 둘이라는 점뿐이다.
    async fn run_read_and_dispatch<S>(
        incoming: S,
        frames: Arc<dyn FrameSink>,
        handler: Arc<dyn ConnectionHandler>,
        conn_id: ConnId,
    ) where
        S: Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
            + Unpin
            + Send
            + 'static,
    {
        let (tx, rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
        let rx_bytes = new_rx_bytes();
        let (bypass_tx, bypass_rx) = mpsc::channel::<Inbound>(CONN_BYPASS_CAP);
        let read = tokio::spawn(read_task(
            incoming,
            tx,
            bypass_tx,
            rx_bytes.clone(),
            frames.clone(),
            handler.clone(),
            conn_id,
            tokio::time::Instant::now(),
            Arc::new(AtomicU64::new(0)),
        ));
        let bypass = tokio::spawn(bypass_task(
            bypass_rx,
            frames.clone(),
            handler.clone(),
            conn_id,
        ));
        dispatch_task(rx, rx_bytes, frames, handler, conn_id).await;
        // `handle_connection` 의 "dispatch 가 먼저 끝난" 갈래와 같은 처분.
        read.abort();
        let _ = read.await;
        finish_bypass_lane(bypass, conn_id).await;
    }

    fn new_rx_bytes() -> RxBytes {
        Arc::new(std::sync::atomic::AtomicUsize::new(0))
    }

    #[tokio::test]
    async fn handler_sees_connect_then_frames_then_disconnect() {
        let fake_sink = Arc::new(FakeFrameSink::default());
        let frames: Arc<dyn FrameSink> = fake_sink.clone();
        let fake = Arc::new(FakeHandler::new(None));
        let handler: Arc<dyn ConnectionHandler> = fake.clone();

        handler.on_connect(7, &frames).await;
        run_read_and_dispatch(
            futures_util::stream::iter(vec![
                text_frame("cmd"),
                Ok(Message::Binary(vec![1, 2, 3].into())),
                Ok(Message::Close(None)),
                text_frame("after-close"),
            ]),
            frames.clone(),
            handler.clone(),
            7,
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

    /// ★close 판정이 읽기 루프 밖으로 나간 뒤에도 그 뜻은 같다★: 처리 행이 `ConnFlow::Close` 를 받으면
    /// 그 자리에서 멈추고, 뒤에 이미 **큐에 들어와 있던** 프레임도 처리되지 않는다. 옛 모양에서
    /// "소켓에서 더 읽지 않는다" 였던 것이 지금은 "더 처리하지 않는다" 이고, 연결 종료로 잇는 것은
    /// `handle_connection` 의 select 갈래다.
    #[tokio::test]
    async fn close_flow_from_on_text_stops_the_dispatch_row() {
        let frames: Arc<dyn FrameSink> = Arc::new(FakeFrameSink::default());
        let fake = Arc::new(FakeHandler::new(Some("stop")));
        let handler: Arc<dyn ConnectionHandler> = fake.clone();

        run_read_and_dispatch(
            futures_util::stream::iter(vec![
                text_frame("go"),
                text_frame("stop"),
                text_frame("unreachable"),
            ]),
            frames,
            handler,
            3,
        )
        .await;

        assert_eq!(
            fake.calls(),
            vec!["text:3:go", "text:3:stop"],
            "ConnFlow::Close 면 그 자리에서 처리를 멈춘다(뒤엣것은 큐에 있어도 안 돈다)"
        );
    }

    /// ★★이 변경이 고친 결함의 회귀망★★ — 옛 읽기 루프는 핸들러 호출을 **완료까지 await** 한 뒤에야
    /// 다음 프레임을 읽었다. 그래서 에이전트 활성화 한 건(전형 2s·백스톱 15s)이 도는 동안 같은
    /// 클라이언트가 보낸 **다른 에이전트로 가는 입력·kill·또 다른 활성화**가 소켓에서 꺼내지지도
    /// 않았다(소켓 head-of-line blocking).
    ///
    /// ★두 가지를 한 번에 잰다★: ① 핸들러가 한 명령에 붙들려 있는 동안 나머지 프레임이 **전부** 읽힌다
    /// ② 그럼에도 처리는 여전히 **도착 순서대로 하나씩**이다(순서가 뜻을 갖는 명령들이 이 성질 하나에
    /// 걸려 있다 — `frame_port::ConnectionHandler` 계약). ①만 재면 "순서를 버려서 빨라진" 회귀를 못
    /// 잡고, ②만 재면 원래 결함이 그대로 있어도 초록이다.
    ///
    /// ★게이트로 재고 sleep 으로 재지 않는다★: 느린 러너에서 창이 닫혀 나는 위양성을 없앤다.
    #[tokio::test]
    async fn the_read_loop_runs_ahead_while_a_handler_call_is_still_running() {
        let frames: Arc<dyn FrameSink> = Arc::new(FakeFrameSink::default());
        let fake = Arc::new(FakeHandler::blocking_on("slow"));
        let handler: Arc<dyn ConnectionHandler> = fake.clone();

        let yielded = Arc::new(AtomicU64::new(0));
        let counter = yielded.clone();
        let incoming = futures_util::stream::iter(vec![
            text_frame("slow"),
            text_frame("other-agent-input"),
            text_frame("kill"),
            Ok(Message::Binary(vec![9, 9].into())),
            text_frame("tail"),
        ])
        .inspect(move |_| {
            counter.fetch_add(1, Ordering::Release);
        });

        let (tx, rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
        let rx_bytes = new_rx_bytes();
        let (bypass_tx, _bypass_rx) = mpsc::channel::<Inbound>(CONN_BYPASS_CAP);
        let read = tokio::spawn(read_task(
            incoming,
            tx,
            bypass_tx,
            rx_bytes.clone(),
            frames.clone(),
            handler.clone(),
            5,
            tokio::time::Instant::now(),
            Arc::new(AtomicU64::new(0)),
        ));
        let dispatch = tokio::spawn(dispatch_task(rx, rx_bytes, frames, handler, 5));

        // 첫 명령이 핸들러 **안에서** 붙들린 것을 확인하고 나서 관측한다.
        tokio::time::timeout(Duration::from_secs(5), fake.text_seen.notified())
            .await
            .expect("첫 명령이 핸들러에 닿아야");

        let mut all_read = false;
        for _ in 0..500 {
            if yielded.load(Ordering::Acquire) == 5 {
                all_read = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            all_read,
            "핸들러가 첫 명령에 붙들린 동안 나머지 프레임이 소켓에서 꺼내지지 않았다 — head-of-line blocking 회귀"
        );
        assert_eq!(
            fake.calls(),
            vec!["text:5:slow"],
            "읽기가 앞서 나가도 처리는 한 번에 하나다 — 앞 호출이 반환하기 전에 다음이 시작되면 안 된다"
        );

        fake.slow_gate.notify_one();
        tokio::time::timeout(Duration::from_secs(10), dispatch)
            .await
            .expect("게이트가 열리면 처리 행이 끝나야")
            .unwrap();
        let _ = read.await;

        assert_eq!(
            fake.calls(),
            vec![
                "text:5:slow",
                "text:5:other-agent-input",
                "text:5:kill",
                "binary:5:2",
                "text:5:tail",
            ],
            "큐를 거쳐도 처리 순서는 도착 순서 그대로다"
        );
    }

    /// ★★적대 리뷰가 찾아낸 두 번째 구멍의 회귀망★★ — 수신 줄에 **취소를 위한 자리가 없으면**, 줄이
    /// 가득 찬 순간 `Kill` 이 그 줄 뒤에서 굶어 이 변경이 없애려던 증상(「취소가 안 먹는다」)이 다른
    /// 경로로 되돌아온다.
    ///
    /// ★핵심은 「우선 줄이 있다」가 아니라 「**읽기가 안 선다**」다★: 읽기 루프가 포화한 줄 위에서
    /// 기다리면, 그 뒤에 오는 `Kill` 은 **소켓에서 꺼내지지도 않는다**(TCP 는 스트림이다). 그래서 이
    /// 시험은 `Kill` 을 **가득 찬 뒤에 오는 프레임**으로 놓고, 그 앞에 일반 명령을 하나 더 끼워
    /// 「그 한 장을 처분하고 계속 읽었는가」까지 함께 본다.
    ///
    /// ★소비자가 막힌 상태를 만든다★: 첫 프레임이 게이트에 붙들리므로 도착순 줄은 절대 비지 않는다 —
    /// 실제 사고(막힌 PTY write 뒤로 키 입력이 쌓인다)와 같은 모양이다.
    #[tokio::test]
    async fn a_cancel_still_gets_through_a_saturated_inbound_queue() {
        let sink = Arc::new(FakeFrameSink::default());
        let frames: Arc<dyn FrameSink> = sink.clone();
        let fake = Arc::new(FakeHandler::blocking_on("slow"));
        let handler: Arc<dyn ConnectionHandler> = fake.clone();

        // 줄을 꽉 채운다: 게이트에 붙들릴 1장 + 줄을 메울 CONN_RX_CAP 장.
        let mut items = vec![text_frame("slow")];
        for i in 0..CONN_RX_CAP {
            items.push(text_frame(&format!("filler-{i}")));
        }
        // ★여기부터가 이 시험의 본론★ — 줄에 자리가 없는 상태에서 도착하는 두 장.
        items.push(text_frame("overflow-ordered"));
        items.push(text_frame("bypass:kill"));

        let (tx, rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
        let rx_bytes = new_rx_bytes();
        let (bypass_tx, bypass_rx) = mpsc::channel::<Inbound>(CONN_BYPASS_CAP);
        let read = tokio::spawn(read_task(
            futures_util::stream::iter(items),
            tx,
            bypass_tx,
            rx_bytes.clone(),
            frames.clone(),
            handler.clone(),
            11,
            tokio::time::Instant::now(),
            Arc::new(AtomicU64::new(0)),
        ));
        let bypass = tokio::spawn(bypass_task(bypass_rx, frames.clone(), handler.clone(), 11));
        let dispatch = tokio::spawn(dispatch_task(rx, rx_bytes, frames, handler, 11));

        // 취소가 **도착순 소비자가 아직 첫 명령에 붙들려 있는 동안** 실제로 처리되는지 본다.
        let mut cancelled = false;
        for _ in 0..500 {
            if fake.calls().iter().any(|c| c == "text:11:bypass:kill") {
                cancelled = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            cancelled,
            "줄이 가득 찬 동안 취소가 통과하지 못했다 — 취소 자리 없음 회귀: {:?}",
            fake.calls()
        );
        // ★여기서 재는 것은 **집합이지 순서가 아니다**★: 두 줄이 서로 다른 task 라 "slow" 와 취소 중
        //   어느 쪽이 먼저 기록되는지는 스케줄러 몫이고, 그것을 단언하면 위양성이 난다. 지켜야 할 것은
        //   「줄에 든 것들은 아직 하나도 안 돌았다」 — 즉 건너뛴 것이 취소뿐이라는 사실이다.
        let ran: Vec<String> = fake.calls();
        assert!(
            ran.iter()
                .all(|c| c == "text:11:slow" || c == "text:11:bypass:kill"),
            "취소 말고 다른 것이 줄을 건너뛰었다(또는 막힌 줄이 돌았다): {ran:?}"
        );
        // ★넘친 일반 명령은 **조용히 사라지지 않는다**★ — 이 fake 의 거절 답장이 그 증거다.
        assert!(
            sink.frames()
                .iter()
                .any(|f| f == "text:refused:overflow-ordered"),
            "넘친 명령이 답장 없이 버려졌다: {:?}",
            sink.frames()
        );

        fake.slow_gate.notify_one();
        let _ = tokio::time::timeout(Duration::from_secs(10), dispatch).await;
        read.abort();
        bypass.abort();
    }

    /// ★★칸은 남았는데 **바이트가 먼저 찬다**★★ — [`CONN_RX_MAX_BYTES`] 회귀망.
    ///
    /// ★잡는 회귀★: 포화를 칸 수로만 재던 모양. 그러면 이 줄의 실제 천장이
    ///   `CONN_RX_CAP × max_message_size` (= 64 × 64 MiB) 가 되는데, 그 곱을 아무도 세지 않았다.
    /// ★결정적이다★: sleep 도 스케줄러 가정도 없다 — 소비자를 게이트로 붙들어 두면 줄은 절대 비지
    ///   않으므로, 예산 판정은 프레임을 넣는 그 순서만으로 결정된다.
    /// ★큰 프레임 **두 장**인 이유★: 판정이 「더하기 전 물높이」라 빈 줄에 오는 한 장은 크기와 무관하게
    ///   언제나 들어간다(그 성질도 함께 고정한다). 예산을 넘기는 것은 그 다음 장부터다.
    #[tokio::test]
    async fn the_inbound_row_is_bounded_by_bytes_not_only_by_slots() {
        let sink = Arc::new(FakeFrameSink::default());
        let frames: Arc<dyn FrameSink> = sink.clone();
        let fake = Arc::new(FakeHandler::blocking_on("slow"));
        let handler: Arc<dyn ConnectionHandler> = fake.clone();

        // 예산의 절반보다 한 바이트 큰 장 둘 — 둘이면 예산을 넘고, 칸은 64 중 셋밖에 안 쓴다.
        let big = "x".repeat(CONN_RX_MAX_BYTES / 2 + 1);
        let items = vec![
            text_frame("slow"),
            text_frame(&big),
            text_frame(&big),
            text_frame("after-budget"),
        ];

        let (tx, rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
        let rx_bytes = new_rx_bytes();
        let (bypass_tx, _bypass_rx) = mpsc::channel::<Inbound>(CONN_BYPASS_CAP);
        let read = tokio::spawn(read_task(
            futures_util::stream::iter(items),
            tx,
            bypass_tx,
            rx_bytes.clone(),
            frames.clone(),
            handler.clone(),
            21,
            tokio::time::Instant::now(),
            Arc::new(AtomicU64::new(0)),
        ));
        let dispatch = tokio::spawn(dispatch_task(rx, rx_bytes, frames, handler, 21));

        // 읽기 루프는 프레임을 다 훑고 끝난다(예산 초과에서도 서지 않는다 — 그게 이 배치의 전제).
        tokio::time::timeout(Duration::from_secs(10), read)
            .await
            .expect("읽기 루프가 예산 초과에서 서면 안 된다")
            .unwrap();

        assert!(
            sink.frames()
                .iter()
                .any(|f| f == "text:refused:after-budget"),
            "칸이 남았다고 통과시켰다 — 바이트 예산이 안 걸렸다: {:?}",
            sink.frames()
        );
        // 소비자가 첫 장을 실제로 집었음을 **신호로** 확인하고 나서 아래를 단언한다(스케줄러 가정 금지).
        tokio::time::timeout(Duration::from_secs(5), fake.text_seen.notified())
            .await
            .expect("소비자가 첫 장을 집어야");
        assert_eq!(
            fake.calls(),
            vec!["text:21:slow"],
            "소비자는 여전히 첫 장에 붙들려 있어야(줄이 비면 이 시험의 전제가 무너진다)"
        );

        fake.slow_gate.notify_one();
        let _ = tokio::time::timeout(Duration::from_secs(10), dispatch).await;
    }

    /// ★★거절을 **답장하지 못하면** 그 연결은 끝난다★★ — [`Saturated::Unanswered`] 회귀망.
    ///
    /// ★잡는 회귀★: 위층이 `Error` 답장 enqueue 결과를 버리고 늘 `Refused` 를 돌려주던 모양. 그러면
    ///   네트워크 행은 없는 답장을 믿고 원래 명령을 버리고, 보낸 쪽은 **명령도 거절도** 못 받은 채
    ///   자기 마감시각까지 기다린다.
    /// ★관측 = 「그 뒤 프레임을 소켓에서 꺼냈는가」★: 종료 여부를 join 으로 재면 두 갈래가 구별되지
    ///   않는다(둘 다 곧 끝난다). 스트림에서 **몇 장을 뽑았는지**를 세면 갈린다 — 끝냈으면 뒤엣것은
    ///   뽑히지 않는다.
    /// ★소비자를 아예 안 띄운다★: 띄우면 「소비자가 첫 장을 꺼내 갔는가」가 스케줄러에 달려 포화 시점이
    ///   한 장씩 흔들린다(실측으로 그 흔들림을 봤다). 수신단만 살려 두면 줄은 정확히 `CONN_RX_CAP` 장에서
    ///   차므로 판정이 결정적이다 — 이 시험이 재는 것은 소비자 동작이 아니라 **읽기 행의 처분**이다.
    #[tokio::test]
    async fn a_refusal_that_could_not_be_sent_ends_the_read_row() {
        // 한 프레임도 못 받는 출구 = 송신 큐도 포화인 상태.
        let frames: Arc<dyn FrameSink> = Arc::new(FakeFrameSink::refusing());
        let handler: Arc<dyn ConnectionHandler> = Arc::new(FakeHandler::new(None));

        let mut items = Vec::new();
        for i in 0..CONN_RX_CAP {
            items.push(text_frame(&format!("filler-{i}")));
        }
        items.push(text_frame("unanswerable")); // 줄도 차고 답장도 못 내는 그 한 장
        items.push(text_frame("tail-1"));
        items.push(text_frame("tail-2"));
        let total = items.len();
        let trigger_index = CONN_RX_CAP + 1; // filler 들 + 그 한 장

        let pulled = Arc::new(AtomicU64::new(0));
        let counter = pulled.clone();
        let incoming = futures_util::stream::iter(items).inspect(move |_| {
            counter.fetch_add(1, Ordering::Release);
        });

        // ★수신단을 살려 둔다★ — 드롭하면 첫 `try_send` 가 Closed 로 끊겨 이 시험이 포화가 아니라
        //   "소비자 없음" 을 재게 된다.
        let (tx, _rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
        let (bypass_tx, _bypass_rx) = mpsc::channel::<Inbound>(CONN_BYPASS_CAP);
        read_task(
            incoming,
            tx,
            bypass_tx,
            new_rx_bytes(),
            frames,
            handler,
            23,
            tokio::time::Instant::now(),
            Arc::new(AtomicU64::new(0)),
        )
        .await;

        assert_eq!(
            pulled.load(Ordering::Acquire) as usize,
            trigger_index,
            "답장 못 한 거절을 '거절했다'로 읽고 계속 읽었다 — 전체 {total} 장 중 {trigger_index} 장에서 멈췄어야",
        );
    }

    /// 초기값을 도달 불가능한 sentinel 로 두어 "갱신됐다" 를 타이밍 없이 판정한다.
    async fn clock_updated_by(msg: Message) -> bool {
        let last_recv = Arc::new(AtomicU64::new(u64::MAX));

        // ★수신단을 살려 둔다★: 드롭하면 `send` 가 실패해 루프가 첫 프레임에서 끊겨, 이 판정이
        //   "갱신됐나" 가 아니라 "큐가 살아 있나" 를 재게 된다.
        let (tx, _rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
        let (bypass_tx, _bypass_rx) = mpsc::channel::<Inbound>(CONN_BYPASS_CAP);
        read_task(
            futures_util::stream::iter(vec![Ok(msg)]),
            tx,
            bypass_tx,
            new_rx_bytes(),
            Arc::new(FakeFrameSink::default()),
            Arc::new(FakeHandler::new(None)),
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

    /// 이 crate 의 테스트가 쓰는 임의 프로토콜 버전. ★실제 운영 버전과 일부러 다른 값★ — 서버가
    /// 상수를 도로 읽는 회귀가 생기면 이 값과 어긋나 테스트가 깨진다(주입 경로의 존재 증명).
    const TEST_WIRE_VERSION: u32 = 4242;

    /// 테스트 서버 1개를 띄우고 인증까지 마친 클라이언트를 돌려준다(공통 뼈대).
    async fn serve_one(
        registry: ConnRegistry,
        fake: Arc<FakeHandler>,
        keepalive: KeepaliveConfig,
    ) -> (
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
        tokio::task::JoinHandle<()>,
    ) {
        serve_one_with_versions(
            registry,
            fake,
            keepalive,
            TEST_WIRE_VERSION,
            TEST_WIRE_VERSION,
        )
        .await
    }

    async fn serve_one_with_versions(
        registry: ConnRegistry,
        fake: Arc<FakeHandler>,
        keepalive: KeepaliveConfig,
        server_expects: u32,
        client_sends: u32,
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
                server_expects,
                keepalive,
            )
            .await;
        });

        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/"))
            .await
            .unwrap();
        let auth = serde_json::to_string(&AuthFrame::Auth {
            token: "tok".to_string(),
            protocol_version: client_sends,
        })
        .unwrap();
        client.send(Message::Text(auth.into())).await.unwrap();
        (client, server)
    }

    #[tokio::test]
    async fn auth_rejects_version_mismatch_using_the_injected_expectation() {
        let registry = ConnRegistry::new();
        let fake = Arc::new(FakeHandler::new(None));
        let (mut client, server) = serve_one_with_versions(
            registry.clone(),
            fake.clone(),
            KeepaliveConfig::default(),
            TEST_WIRE_VERSION,
            TEST_WIRE_VERSION + 1,
        )
        .await;

        let msg = tokio::time::timeout(Duration::from_secs(5), client.next())
            .await
            .expect("거부 통보가 와야")
            .expect("스트림이 살아 있어야")
            .expect("프레임 수신");
        match msg {
            Message::Text(t) => assert_eq!(
                t.as_str(),
                format!(
                    "protocol_version mismatch: client {} != server {}",
                    TEST_WIRE_VERSION + 1,
                    TEST_WIRE_VERSION
                ),
                "두 숫자 모두 주입값 기준이어야(상수 하드코딩이면 어긋난다)"
            ),
            other => panic!("Text 거부 통보여야: {other:?}"),
        }

        tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .expect("버전 불일치면 handle_connection 이 반환해야")
            .unwrap();
        assert!(!registry.contains(1), "인증 실패 연결은 등록되지 않는다");
        assert_eq!(
            fake.calls(),
            Vec::<String>::new(),
            "인증 실패면 위층 핸들러가 아예 붙지 않는다"
        );
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

    /// ★★받아 준 우선 프레임을 teardown 이 **버리지 않는다**★★ — [`finish_bypass_lane`] 회귀망.
    ///
    /// ★잡는 결함★: 도착순 줄이 먼저 끝나는 갈래(피어가 명령을 보내고 곧바로 close 하는 흔한 모양)에서
    ///   `handle_connection` 이 우선 줄 task 를 그냥 `abort()` 했다. 위층이 「이건 굶으면 안 된다」고
    ///   판정해 **받아 준** 프레임이 한 번도 돌지 않고 사라진다 — 그 줄을 둔 이유가 통째로 무너진다.
    ///
    /// ★무대 만들기★: 두 줄을 **각각** 잠근다. 도착순 줄은 게이트에 붙들어 수신 큐를 채우고(그래야
    ///   뒤엣것이 우선 줄로 간다), 우선 줄은 **첫 건에서** 따로 잠가 둘째 건이 큐에 남게 한다. 그 상태로
    ///   도착순 줄을 풀고 소켓을 닫으면 teardown 이 시작되는데, 그때 우선 줄에는 **받아 놓고 아직 안 돈
    ///   한 장**이 있다 — 결함이 있으면 그 장이 abort 와 함께 사라진다.
    /// ★탐지의 정직한 범위★: 300ms 창은 **버그판이 그 안에 정리를 끝내는가**만 본다(정상 코드는 우선
    ///   줄을 기다리느라 그 창에 절대 안 끝난다 — 위양성 불가). 창이 짧아 버그판이 아직 안 끝났으면
    ///   위음성 쪽으로만 틀린다. 그 뒤 게이트를 열고 결과를 보는 단언은 두 갈래 모두에서 결정적이다.
    #[tokio::test]
    async fn an_accepted_bypass_frame_survives_the_ordered_row_finishing_first() {
        let registry = ConnRegistry::new();
        let fake = Arc::new(FakeHandler::blocking_on_both("slow", "bypass:first"));
        let (mut client, mut server) =
            serve_one(registry.clone(), fake.clone(), KeepaliveConfig::default()).await;

        // ① 도착순 소비자를 붙든다 — 이 뒤로 그 줄은 절대 비지 않는다.
        client
            .send(Message::Text("slow".to_string().into()))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), fake.text_seen.notified())
            .await
            .expect("첫 명령이 핸들러에 닿아야");

        // ② 도착순 줄을 꽉 채운다(소비자가 하나를 꺼내 갔으므로 정확히 CONN_RX_CAP 장).
        for i in 0..CONN_RX_CAP {
            client
                .send(Message::Text(format!("filler-{i}").into()))
                .await
                .unwrap();
        }

        // ③ 포화 상태에서 우선 줄로 두 장 — 첫 장은 그 줄의 게이트에 붙들리고, 둘째 장은 큐에 남는다.
        client
            .send(Message::Text("bypass:first".to_string().into()))
            .await
            .unwrap();
        client
            .send(Message::Text("bypass:second".to_string().into()))
            .await
            .unwrap();

        let mut first_running = false;
        for _ in 0..500 {
            if fake.calls().iter().any(|c| c == "text:1:bypass:first") {
                first_running = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            first_running,
            "우선 줄이 첫 장을 집지 못했다 — 이 시험의 무대가 안 섰다: {:?}",
            fake.calls()
        );

        // ④ 도착순 줄을 풀고 소켓을 닫는다 → read 종료 → dispatch 가 남은 것을 비우고 종료 → teardown.
        fake.slow_gate.notify_one();
        client.close(None).await.unwrap();

        // ⑤ 정상 코드는 우선 줄을 기다리므로 이 창 안에 안 끝난다. 끝났다면 버려 버린 것이다.
        let finished_before_the_gate =
            tokio::time::timeout(Duration::from_millis(300), &mut server)
                .await
                .is_ok();

        // ⑥ 이제 우선 줄을 풀어 준다 — 받아 둔 둘째 장이 여기서 돌아야 한다.
        fake.bypass_gate.notify_one();
        if !finished_before_the_gate {
            tokio::time::timeout(Duration::from_secs(15), &mut server)
                .await
                .expect("우선 줄이 비면 정리가 끝나야")
                .unwrap();
        }

        assert!(
            fake.calls().iter().any(|c| c == "text:1:bypass:second"),
            "받아 둔 우선 프레임이 teardown 의 abort 로 사라졌다(창 안 종료 = {finished_before_the_gate}): {:?}",
            fake.calls()
        );
        drop(client);
    }

    /// ★패자 task 의 명시적 abort★(`handle_connection` 의 select!) — JoinHandle 을 그냥 drop 하면
    /// detach 되어 WS half 를 쥔 task 가 살아남는다. 그 누수는 e2e 로는 안 보인다(전부 정상 종료라
    /// 남은 half 가 표에 안 드러난다).
    ///
    /// ★관측 방법★: 프레임 출구 `Arc` 의 **강참조는 dispatch_task 만** 들고 있다(`handle_connection` 은
    /// 그것을 dispatch_task 로 move 하고, `on_connect` 은 빌리기만 한다). 그래서 연결 종료 뒤에도
    /// 약참조가 upgrade 되면 = 그 task 가 살아 있다 = abort 대신 detach 됐다는 뜻이다.
    /// ★그 관측이 **read_task 의 abort 도 함께** 무는 것은 배선 때문이다★: dispatch_task 가 스스로
    /// 끝나려면 수신 큐의 유일한 송신단(read_task 가 쥔다)이 드롭돼야 하는데, 이 갈래의 read_task 는
    /// `next()` 에 파킹돼 스스로 끝나지 않는다 — 즉 read 의 abort 가 빠져도 이 폴링이 끝까지
    /// `released == false` 로 돈다. 옛 주석은 이 강참조가 read_task 에 있다고 적혀 있었다.
    /// ★이 관측은 **이 테스트의 fake 핸들러에서만** 성립한다★ — 그 fake 는 강참조를 하나도 보관하지
    /// 않는다(아래 `FakeHandler` 주석의 금지 조항). 운영 핸들러는 `on_connect` 에서 사본을 하나 떠
    /// **명령 명부**에 넣고 연결 수명 내내 들고 있으므로 이 테스트를 그쪽으로 옮기면 관측이 성립하지
    /// 않는다. 즉 아래 「사본 전수」의 ⑤는 이 테스트가 **지켜 주지 않는다**.
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
    /// 통해서만 회수된다(`on_disconnect`) ⑤ **명령 버스의 주인 명부**가 `on_connect` 부터 `on_disconnect`
    /// 까지 드는 사본 하나 — 위층이 어느 연결로 명령 봉투를 보낼지 지목하려고 든다. ⑤ 본체는 넣는 곳도
    /// 빼는 곳도 각각 한 곳이고 둘 다 그 명부의 한 잠금 안이라 ④ 같은 누락 경쟁이 없다.
    /// ★다만 ⑤가 **내주는** 사본은 이 전수에 안 든다★ — 명부는 지목할 때 사본을 떠서 배달하는 쪽에
    /// 건네고, 내준 뒤로는 회수할 수단이 없다. 그것을 대기표에 넣고 답장을 기다리는 배달이 있으면 그
    /// 사본은 `on_disconnect` 를 넘겨 산다. 그래서 그쪽은 **호출자 계약**(받아서 즉시 쓰고 버린다)으로만
    /// 막혀 있고, 컴파일러도 이 crate 도 그것을 강제하지 않는다.
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
