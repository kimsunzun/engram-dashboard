//! 실소켓 WS 어댑터 — feature `ws` 뒤에 산다. ★이 crate 의 유일한 실전송 구현체다★.
//!
//! ## 이 파일이 소유하는 것
//!
//! 거는 쪽([`WsDialer`] = [`Dialer`] 구현) · 받는 쪽([`WsListener`]) · 열린 WS 스트림 하나를
//! [`LinkTx`]/[`LinkRx`] 두 끝으로 나누는 일. ★두 방향이 함께 있는 것이 요점★ — 그래서 이 crate 는
//! 다른 crate 없이 **자기에게 붙어** 인메모리로 못 재는 성질을 잰다(ADR-0177 결정 8).
//!
//! ## 보증하는 것 / 소비자에게 남기는 것
//!
//! 보증 = [`Frame`]↔`Message` 왕복 · 닫기 코드와 문구를 **양방향으로** 나르는 것(그래서 「붙었는데
//! 거절」이 [`LinkRead::Closed`] 로 건너오고 ADR-0180 결정 5 가 코드로 성립한다) · 전송 계층 Ping/Pong 을
//! [`Frame::Keepalive`] 로 올리는 것.
//!
//! ★이 셋 중 **옮기는 표**는 이 파일 아래 단위 테스트가 단언한다★ — [`to_message`]/[`from_message`] 가
//! 순수 함수라 소켓 없이 재진다. **표가 아닌 것**(실소켓 왕복 · 닫기 프레임이 실제로 나가는지)은
//! `tests/` 의 실소켓 스위트 몫이고, 그중 keepalive 왕복은 아직 아무도 안 잰다(아래 「알려진 한계」).
//!
//! 남긴다 = 주소 문법(이 어댑터는 [`Address`] 를 `tokio-tungstenite` 에 그대로 넘긴다) · 재연결·시한·
//! keepalive 주기(전부 [`crate::Policy`] 와 감독 몫 — ★어댑터가 자기 시한을 또 걸지 않는다★,
//! [`Dialer`] rustdoc) · 인증·버전 협상(핸드셰이크 프레임은 [`crate::Handshake`] 구현체가 소유한다) ·
//! 받는 쪽의 상대 명부(아래).
//!
//! ## 알려진 한계 (보증으로 읽지 말 것)
//!
//! - ★[`WsListener`] 는 seam 이 아니다★ — `link.rs` 에 `Listener` trait 이 없고 [`crate::Registry`] 는
//!   들어오는 연결을 여전히 모른다. 여기 있는 것은 「통로 양 끝을 얻는 구체 타입」 하나뿐이고, 받은
//!   통로를 감독 태스크에 얹는 일은 아직 아무도 안 한다.
//! - ★[`WsListener::accept`] 가 TCP 수락과 WS 핸드셰이크를 **한 자리에서** 한다★ — 핸드셰이크를 안
//!   끝내는 상대 하나가 그 동안 다른 수락을 막는다. 서버 행이 실제로 들어오면 갈라야 하는 지점이다.
//! - ★Origin 검사가 없다★ — `net` 의 서버 행은 `accept_hdr_async` 로 그것을 하지만(ADR-0129) 여기는
//!   `accept_async` 다. 이 타입으로 외부에 닿는 포트를 열면 그 검사 없이 열린다.
//! - ★TLS 를 켜지 않았다★ — `tokio-tungstenite` 의 default feature 만 쓰므로 `wss://` 는 dial 단계에서
//!   실패한다.
//!
//! ## 통로 성질 (외부 세계의 사실)
//!
//! - ★쓰기는 커널 send buffer 가 차면 실제로 멈춘다★ — `SinkExt::send` 가 tungstenite 의 `flush` 를
//!   부르고 그것이 `WouldBlock` 을 `Pending` 으로 옮긴다(tokio-tungstenite 0.26.2 의
//!   `Sink::poll_flush` → `cvt(s.flush())`). 그래서 상대가 안 읽으면 감독의 `write_deadline` 이 그
//!   자리를 잘라낸다 — `tests/ws_write_deadline.rs` 가 재는 것이 그것이다.
//! - ★단 **한 번의 큰 쓰기**는 안 멈춘다★ — **실측**(2026-09-07, Windows 11 loopback): 안 읽는
//!   상대에게 16 MiB 한 프레임이 28 ms 만에 그대로 들어갔고(4·8 MiB 도 같다), ★`SO_SNDBUF`/`SO_RCVBUF`
//!   를 4 KiB 로 명시해도 결과가 같았다★. 같은 실측의 다른 축: 1 MiB 프레임을 반복하면 2 MiB 를 삼킨
//!   뒤 세 번째에서 멈춘다. **해석**(관측이 아니다) — 두 수치가 같은 판에서 갈리는 것은 Windows 의
//!   동적 송신 버퍼링이 한 번의 큰 send 를 통째로 삼키기 때문으로 읽었다. 그 해석이 틀려도 두 수치는
//!   그대로다. ★배압을 「한 프레임이 크면 걸린다」로 읽지 말 것★ — 걸리는 축은 크기가 아니라 **누적**이다.
//! - ★들어오는 것에는 크기 상한이 있고 나가는 것에는 없다★ — tungstenite 기본값
//!   `max_frame_size` 16 MiB · `max_message_size` 64 MiB 를 이 어댑터가 덮지 않는다. 넘으면 **받는 쪽
//!   읽기 오류**라 통로가 끊긴다(실측: 32 MiB 한 프레임을 보내면 받는 쪽이 `Space limit exceeded:
//!   Message too long: 33554432 > 16777216` 으로 끊고, **전송 계층은 그것을 막지 않는다**).
//!   ★그래서 [`LinkTx::send`] 구현이 그 16 MiB 로 `debug_assert` 를 건다★ — 상한을 집행하는
//!   것이 받는 쪽뿐이면 **아무 잘못 없는 상대가 자기 재연결 예산을 태우고**, 끊김 사건에는 문구조차
//!   없어 진단이 남지 않는다. 릴리스에서는 여전히 통과한다(그 자리 주석이 사유를 든다).
//! - ★들어온 Ping 에 대한 자동 Pong 이 **언제** 나가는지★ — tungstenite 0.26.2 소스를 읽어 확인한 것
//!   (실행으로 잰 것이 아니다): 수신부가 Ping 을 만나면 Pong 을 `additional_send` 에 넣고
//!   (`protocol/mod.rs:651`), **읽기 루프가 매 바퀴 앞에서 그 대기 쓰기를 flush 한다**
//!   (`protocol/mod.rs:452`). 즉 나가는 자리는 쓰는 쪽 폴링 **하나가 아니라 읽기·쓰기 둘**이다.
//!   ★송신 버퍼가 차 있어도 읽기가 멈추지는 않는다★ — 그 flush 의 `WouldBlock` 은 삼키고
//!   `unflushed_additional` 만 세운 뒤 계속 읽으며(그 자리 주석: "If blocked continue reading, but try
//!   again later"), tokio-tungstenite 의 waker 대리자가 읽는 쪽 waker 를 쓰기 축에도 걸어 두어
//!   (`compat.rs` 의 `WakerProxy`) 소켓이 쓸 수 있게 되면 읽던 태스크가 깨어 다시 flush 한다. 그래서
//!   혼잡의 증상은 「우리 Pong 이 늦는다」이고 「우리가 못 읽는다」가 아니다 — 감독이 재는 마지막
//!   수신 시각이 이것 때문에 멎지는 않는다. 늦은 Pong 이 상대의 인내를 넘기면 증상은 「상대가
//!   우리를 침묵으로 끊는다」인데, `Policy::check` 가 `write_deadline < ping_interval < idle_timeout`
//!   을 강제하므로 걸린 쓰기가 먼저 자기 시한에 잘려 통로가 버려진다 — 유계다.
//! - ★keepalive **왕복**은 실소켓으로 미검이다★ — 옮기는 표는 이 파일 단위 테스트가 양방향으로
//!   단언하지만(위 「보증」), 실 Ping 이 나가고 실 Pong 이 [`Frame::Keepalive`] 로 돌아오는 것을 재는
//!   테스트는 없다. crate 의 `ping_interval` 이 어느 실소켓 테스트의 수명보다 훨씬 길어 그 왕복이
//!   테스트 안에서 일어나지 않는다. 새면 증상은 바로 위 항목의 그것이다.
//! - ★`Message::Frame` 은 실소켓 수신으로 올라오지 않는다★(tungstenite 문서) — 그래서 그 갈래는
//!   [`Frame`] 으로 옮기지 않고 건너뛴다.
//! - ★[`LinkRx::recv`] 의 취소 안전성은 이 crate 에서 **아무것도 지탱하지 않는다**★ — 취소하는 자리가
//!   모두 그 직후 읽는 쪽을 통째로 버리기 때문이다: 핸드셰이크 시한·상대 종료 갈래는 `Err` 로 돌아가며
//!   통로를 버리고(`peer.rs` 의 `handshake_once`), 제어 신호가 이긴 갈래는 핸드셰이크 future 를 양 끝과
//!   함께 떨어뜨리고(`shake_hands`), 운영 단계에서 읽는 쪽을 쥔 태스크는 `select!` 팔이 아니라 전용
//!   태스크라 취소되지 않는다. ★그래서 여기 「미검」 딱지를 두지 않는다★ — 갚을 빚이 아니라 애초에
//!   지지 않은 빚이고, 딱지가 남으면 다음 세션이 없는 위험을 재려 든다. **취소 안전성에 기대는 호출을
//!   새로 만들면 그때 이 문장이 무효가 된다.**

use std::net::SocketAddr;

use futures_util::future::BoxFuture;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, ToSocketAddrs};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode as WsCloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::{Bytes, Message};
use tokio_tungstenite::WebSocketStream;

use crate::frame::{Close, CloseCode, Frame};
use crate::link::{Address, Dialer, LinkError, LinkRead, LinkRx, LinkTx};

/// 닫기 문구가 실릴 수 있는 최대 바이트 — **문자 수가 아니다**.
///
/// ★외부 사실★ — RFC 6455 §5.5 가 제어 프레임 payload 를 125 바이트로 못박고 닫기 프레임은 그 앞
/// 2 바이트를 코드에 쓴다. tungstenite 0.26.2 는 **나가는 쪽을 검사하지 않지만**(`Frame::close` 가
/// 코드와 문구를 그대로 이어 붙인다) **받는 쪽에서 125 초과를 `ControlFrameTooBig` 로 끊는다**
/// (`protocol/mod.rs:640`). 그래서 문구가 길면 「붙었는데 거절」이 상대에게 **읽기 오류**로 도착해
/// 거절 판정이 무너지고 재연결 예산까지 태운다 — [`WsTx::close`] 가 그 앞에서 자른다.
const MAX_CLOSE_REASON_BYTES: usize = 123;

/// 나가는 프레임 payload 의 상한 — 받는 쪽 tungstenite 의 `max_frame_size` 기본값이다.
///
/// ★외부 사실★ — tungstenite 0.26.2 의 `WebSocketConfig::default()` 가 `max_frame_size` 를 16 MiB 로
/// 둔다(`protocol/mod.rs:102`). 이 어댑터는 그것을 덮지 않으므로 **받는 쪽**이 이 값을 집행한다.
const MAX_OUTGOING_PAYLOAD: usize = 16 << 20;

/// 닫기 문구를 [`MAX_CLOSE_REASON_BYTES`] 안으로 자른다.
///
/// ★문자 경계에서 자른다★ — 바이트로 자르면 깨진 UTF-8 이 되고, 그건 길이 초과와 **다른** 방식으로
/// 실패한다(tungstenite 의 `Utf8Bytes` 가 문구를 UTF-8 로 보증한다).
fn clamp_close_reason(reason: &str) -> &str {
    if reason.len() <= MAX_CLOSE_REASON_BYTES {
        return reason;
    }
    let mut end = MAX_CLOSE_REASON_BYTES;
    while end > 0 && !reason.is_char_boundary(end) {
        end -= 1;
    }
    &reason[..end]
}

/// 통로 한 끝에 필요한 바탕 성질. `MaybeTlsStream<TcpStream>`(거는 쪽)과 `TcpStream`(받는 쪽)이 둘 다
/// 들어서, 두 방향이 같은 [`LinkTx`]/[`LinkRx`] 구현을 쓴다.
pub trait WsStream: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

impl<S> WsStream for S where S: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

// ── 방향 ─────────────────────────────────────────────────────────────────────

/// 실 WS 통로를 여는 [`Dialer`].
///
/// [`Address`] 는 `tokio-tungstenite` 에 그대로 넘어간다 — 즉 `ws://host:port/path` 문법이고, 그 문법
/// 위반도 [`LinkError`] 로 온다(주소를 crate 가 해석하지 않는다는 계약 그대로다).
pub struct WsDialer;

impl Dialer for WsDialer {
    fn dial(
        &self,
        addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), LinkError>> {
        // ★통로를 future 안에서 만든다★ — [`Dialer`] 계약이 그것을 요구한다(취소되면 아무것도 남지
        //   않아야 한다).
        // ★주소를 소유한 String 으로 옮겨 담는 것은 **시그니처가 강제한다**★ — 반환 타입의 `'_` 는
        //   `&self` 에 묶이고 `addr` 에는 안 묶이므로(수명 생략 규칙) future 가 `addr` 를 빌릴 수 없다.
        let url = addr.as_str().to_string();
        Box::pin(async move {
            let (ws, _response) = tokio_tungstenite::connect_async(url.as_str())
                .await
                .map_err(|e| LinkError::new(format!("ws connect {url}: {e}")))?;
            Ok(split_link(ws))
        })
    }
}

/// 실 WS 통로를 받는 쪽.
///
/// ★[`Dialer`] 처럼 trait 뒤에 있지 않다★ — 받는 쪽 seam 이 아직 없어서(파일 헤더 「알려진 한계」)
/// 구체 타입 그대로다. 소비자가 이것에 직접 의존하면 seam 이 들어올 때 함께 바뀐다.
pub struct WsListener {
    tcp: TcpListener,
}

impl WsListener {
    /// 포트 0을 주면 커널이 고른 포트로 열린다 — 실제 번호는 [`WsListener::local_addr`] 로 읽는다.
    pub async fn bind<A: ToSocketAddrs>(addr: A) -> Result<Self, LinkError> {
        let tcp = TcpListener::bind(addr)
            .await
            .map_err(|e| LinkError::new(format!("ws bind: {e}")))?;
        Ok(Self { tcp })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, LinkError> {
        self.tcp
            .local_addr()
            .map_err(|e| LinkError::new(format!("ws local_addr: {e}")))
    }

    /// 이 청취기에 붙는 [`Address`] — [`WsDialer`] 에 그대로 넣는다.
    pub fn dial_address(&self) -> Result<Address, LinkError> {
        Ok(Address::new(format!("ws://{}/", self.local_addr()?)))
    }

    /// 한 통로를 받는다. ★TCP 수락과 WS 핸드셰이크를 함께 한다★ — 그 대가는 파일 헤더에 있다.
    pub async fn accept(&self) -> Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), LinkError> {
        let (stream, from) = self
            .tcp
            .accept()
            .await
            .map_err(|e| LinkError::new(format!("ws accept: {e}")))?;
        let ws = tokio_tungstenite::accept_async(stream)
            .await
            .map_err(|e| LinkError::new(format!("ws handshake from {from}: {e}")))?;
        Ok(split_link(ws))
    }
}

/// 열린 스트림을 통로 양 끝으로 나눈다.
///
/// ★두 끝은 `BiLock` 으로 같은 스트림을 공유한다★ — 그래서 서로 다른 태스크가 쥐어도 되고(감독은 읽는
/// 쪽을 전용 태스크에 넘긴다), 한쪽이 `Pending` 을 낼 때 잠금을 놓아 다른 쪽을 막지 않는다.
/// ★양 끝이 **둘 다** 떨어져야 소켓이 닫힌다★ — 여기서는 그 `BiLock` 이 마지막 한 끝이 떨어질 때만
/// 안쪽 스트림을 놓기 때문이다. 그것을 지킬 의무는 부르는 쪽에 있고 seam 이 그 의무를 진다
/// ([`LinkTx`] rustdoc).
pub fn split_link<S: WsStream>(ws: WebSocketStream<S>) -> (Box<dyn LinkTx>, Box<dyn LinkRx>) {
    let (sink, stream) = ws.split();
    (Box::new(WsTx { sink }), Box::new(WsRx { stream }))
}

// ── 통로 두 끝 ───────────────────────────────────────────────────────────────

struct WsTx<S: WsStream> {
    sink: SplitSink<WebSocketStream<S>, Message>,
}

impl<S: WsStream> LinkTx for WsTx<S> {
    fn send(&mut self, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>> {
        // ★상한을 넘는 프레임은 **보내는 쪽이** 시끄럽게 죽는다★ — 안 그러면 상한을 집행하는 것이
        //   받는 쪽뿐이라(파일 헤더 「들어오는 것에는 크기 상한이 있고 나가는 것에는 없다」) 통로가
        //   끊기고 **아무 잘못 없는 상대가 자기 재연결 예산을 태운다**. 릴리스에서는 패닉하지 않는다 —
        //   소비자 입력으로 감독 태스크를 죽이면 그 상대의 모든 통로가 함께 사라지고, 이 crate 가
        //   사건 스트림 말고는 진단을 낼 자리도 없다(`lib.rs` 「`tracing` 이 없다」).
        debug_assert!(
            payload_len(&frame) <= MAX_OUTGOING_PAYLOAD,
            "나가는 프레임이 {} 바이트다 — 받는 쪽 tungstenite 가 max_frame_size {MAX_OUTGOING_PAYLOAD} 에서 통로를 끊는다",
            payload_len(&frame)
        );
        Box::pin(async move {
            self.sink
                .send(to_message(frame))
                .await
                .map_err(|e| LinkError::new(format!("ws write: {e}")))
        })
    }

    fn ping(&mut self) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(async move {
            self.sink
                .send(Message::Ping(Bytes::new()))
                .await
                .map_err(|e| LinkError::new(format!("ws ping: {e}")))
        })
    }

    fn close(&mut self, close: Close) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let frame = CloseFrame {
                code: WsCloseCode::from(close.code.0),
                // 문구를 [`MAX_CLOSE_REASON_BYTES`] 로 자른다 — 그 상수가 사유를 든다.
                reason: clamp_close_reason(&close.reason).into(),
            };
            // 실패해도 알릴 데가 없다([`LinkTx::close`] 계약). 닫기 프레임이 안 나가면 상대는 코드를
            //   못 보고 말 없이 끊긴 것으로 읽는다 — `frame.rs` 의 그 한계 그대로다.
            let _ = self.sink.send(Message::Close(Some(frame))).await;
            let _ = self.sink.close().await;
        })
    }
}

struct WsRx<S: WsStream> {
    stream: SplitStream<WebSocketStream<S>>,
}

impl<S: WsStream> LinkRx for WsRx<S> {
    fn recv(&mut self) -> BoxFuture<'_, Result<LinkRead, LinkError>> {
        Box::pin(async move {
            loop {
                let msg = match self.stream.next().await {
                    Some(Ok(msg)) => msg,
                    Some(Err(e)) => return Err(LinkError::new(format!("ws read: {e}"))),
                    // ★스트림이 마른 것은 「말 없이 끊겼다」가 **아니다**★ — 닫기 프레임을 이미 읽은
                    //   뒤에만 여기로 온다(외부 사실: tokio-tungstenite 0.26.2 는 `ConnectionClosed`
                    //   /`AlreadyClosed` 만 `Ready(None)` 로 옮기고 `lib.rs:303`, 그 오류는 상태가
                    //   `ClosedByPeer`/`CloseAcknowledged` 일 때만 난다 `protocol/mod.rs:706`).
                    //   말 없이 사라진 상대는 `Protocol(ResetWithoutClosingHandshake)` 라 위의 `Err`
                    //   갈래로 온다 — [`LinkRead::Closed`] rustdoc 이 그 계약을 든다.
                    None => return Ok(LinkRead::Closed(None)),
                };
                if let Some(read) = from_message(msg) {
                    return Ok(read);
                }
            }
        })
    }
}

/// 들어온 `Message` 를 통로 어휘로 옮긴다. `None` = 이 갈래는 올리지 않고 다음 것을 읽는다.
fn from_message(msg: Message) -> Option<LinkRead> {
    match msg {
        Message::Text(text) => Some(LinkRead::Frame(Frame::Text(text.as_str().to_owned()))),
        Message::Binary(bytes) => Some(LinkRead::Frame(Frame::Binary(bytes.to_vec()))),
        Message::Ping(_) | Message::Pong(_) => Some(LinkRead::Frame(Frame::Keepalive)),
        Message::Close(Some(frame)) => Some(LinkRead::Closed(Some(Close::new(
            CloseCode(u16::from(frame.code)),
            frame.reason.as_str().to_owned(),
        )))),
        Message::Close(None) => Some(LinkRead::Closed(None)),
        // 실소켓 수신으로는 안 올라온다(파일 헤더).
        Message::Frame(_) => None,
    }
}

fn to_message(frame: Frame) -> Message {
    match frame {
        Frame::Text(text) => Message::Text(text.into()),
        Frame::Binary(bytes) => Message::Binary(bytes.into()),
        // ★[`LinkTx::ping`] 과 같은 것으로 나간다★ — 살아있음 확인은 전송 계층 어휘라 payload 를 새로
        //   짜지 않는다([`LinkTx::ping`] rustdoc 이 든 그 사유).
        Frame::Keepalive => Message::Ping(Bytes::new()),
    }
}

fn payload_len(frame: &Frame) -> usize {
    match frame {
        Frame::Text(text) => text.len(),
        Frame::Binary(bytes) => bytes.len(),
        Frame::Keepalive => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::protocol::frame::Frame as WsFrame;
    use tokio_tungstenite::tungstenite::Utf8Bytes;

    // ── 프레임 ↔ Message 표 ──
    #[test]
    fn outgoing_frames_map_to_their_message() {
        assert_eq!(
            to_message(Frame::Text("hi".into())),
            Message::Text(Utf8Bytes::from("hi"))
        );
        assert_eq!(
            to_message(Frame::Binary(vec![1, 2, 3])),
            Message::Binary(Bytes::from_static(&[1, 2, 3]))
        );
        // ★keepalive 는 Ping 으로 나가고 payload 가 비어 있다★ — 그 사유는 [`to_message`] 주석.
        assert_eq!(to_message(Frame::Keepalive), Message::Ping(Bytes::new()));
    }

    #[test]
    fn incoming_messages_map_back() {
        let text = from_message(Message::Text(Utf8Bytes::from("hi")));
        assert!(matches!(text, Some(LinkRead::Frame(Frame::Text(t))) if t == "hi"));
        let binary = from_message(Message::Binary(Bytes::from_static(&[7])));
        assert!(matches!(binary, Some(LinkRead::Frame(Frame::Binary(b))) if b == vec![7]));
    }

    #[test]
    fn both_keepalive_directions_arrive_as_one_frame() {
        // ★Pong 도 Ping 도 같은 것으로 올라온다★ — 도착 사실만이 신호다(`frame.rs` 의 그 계약).
        for msg in [
            Message::Ping(Bytes::from_static(&[9])),
            Message::Pong(Bytes::new()),
        ] {
            assert!(matches!(
                from_message(msg),
                Some(LinkRead::Frame(Frame::Keepalive))
            ));
        }
    }

    #[test]
    fn a_spoken_close_carries_its_code_and_reason() {
        let msg = Message::Close(Some(CloseFrame {
            code: WsCloseCode::from(4001),
            reason: Utf8Bytes::from("version 3 != 4"),
        }));
        match from_message(msg) {
            Some(LinkRead::Closed(Some(close))) => {
                assert_eq!(close.code, CloseCode(4001));
                assert_eq!(close.reason, "version 3 != 4");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            from_message(Message::Close(None)),
            Some(LinkRead::Closed(None))
        ));
    }

    #[test]
    fn a_raw_frame_is_skipped_rather_than_lifted() {
        assert!(from_message(Message::Frame(WsFrame::ping(Bytes::new()))).is_none());
    }

    // ── 닫기 문구 상한 ──
    #[test]
    fn a_short_reason_is_left_alone() {
        assert_eq!(clamp_close_reason("nope"), "nope");
        let exact = "x".repeat(MAX_CLOSE_REASON_BYTES);
        assert_eq!(clamp_close_reason(&exact), exact);
    }

    #[test]
    fn a_long_reason_is_cut_on_a_character_boundary() {
        // 한 글자가 3 바이트라 41 글자 = 123 바이트고, 42 글자째가 상한을 넘긴다.
        let long = "가".repeat(60);
        let cut = clamp_close_reason(&long);
        assert!(cut.len() <= MAX_CLOSE_REASON_BYTES);
        assert_eq!(cut.chars().count(), MAX_CLOSE_REASON_BYTES / 3);
        // 자른 결과가 여전히 원문의 접두이고 UTF-8 로 성립한다 — 깨진 바이트면 여기가 깨진다.
        assert!(long.starts_with(cut));
    }

    #[test]
    fn a_cut_reason_fits_a_control_frame() {
        // 코드 2 바이트 + 문구가 125 를 넘지 않는지 실제 프레임으로 잰다 — 넘으면 받는 쪽이
        //   `ControlFrameTooBig` 로 끊는다([`MAX_CLOSE_REASON_BYTES`]).
        for reason in ["", "짧다", &"긴".repeat(200), &"a".repeat(200)] {
            let frame = WsFrame::close(Some(CloseFrame {
                code: WsCloseCode::from(4001),
                reason: clamp_close_reason(reason).into(),
            }));
            assert!(frame.payload().len() <= 125, "{}", frame.payload().len());
        }
    }
}
