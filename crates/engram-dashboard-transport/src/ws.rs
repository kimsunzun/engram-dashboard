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
//! - ★나가는 닫기 **문구**는 넘치면 조용히 잘린다★ — 상한은 123 바이트(코드 2 바이트를 뺀 제어 프레임
//!   payload 125 바이트, RFC 6455 §5.5)이고 자르는 자리가 [`LinkTx::close`] 구현이다. ★말줄임표도
//!   사건도 없다★ — 잘렸다는 것을 소비자가 알 길이 없고, 겪어 보는 것이 유일한 발견 경로다. 코드는
//!   그대로 나가므로 분기는 안 무너진다(문구에 판정 재료를 담지 말 것). 상한 값의 정본은 아래
//!   `MAX_CLOSE_REASON_BYTES` 이고 그 사유는 그 상수 rustdoc 에 있다.
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
//!   ★그래서 [`LinkTx::send`] 구현이 그 16 MiB 를 넘는 프레임을 **오류로 돌려준다**★ — 상한을 집행하는
//!   것이 받는 쪽뿐이면 **아무 잘못 없는 상대가 자기 재연결 예산을 태운다**. ★패닉이 아니라 값인 것이
//!   요점이다★ — 감독 태스크의 `JoinHandle` 을 아무도 안 쥐므로 그 안에서 터진 패닉은 관측되지 않고
//!   발행 상태가 `Live` 에 얼어붙는다(그 자리 주석이 증상을 든다). 오류는 감독이 `LinkLost` 로 옮겨
//!   끊김 사건 + 재연결로 만든다. 릴리스와 디버그가 같게 동작한다.
//! - ★**단 이 갈래에 진단은 안 남는다**★ — 위 오류 문구는 감독이 `Ok(Err(_))` 를 `LinkLost(LinkError)`
//!   로 옮기면서 버리고(`peer.rs` 의 `Supervisor::write`), [`crate::TransportEvent::Disconnected`] 에는
//!   문구 칸이 아예 없다(`lib.rs` 「운영 중 끊김은 사유 문구를 안 나른다」). 상한을 넘긴 그 프레임도
//!   유실로 세지 않는다 — 대기 요청은 `FailPending` 이 깨우지만 답장 없는 알림은 그대로 사라진다.
//!   소비자가 보는 것은 **사유 없는 `LinkError` 끊김 하나**다. 이 갈래를 진단 가능하게 하려면 끊김
//!   사건에 문구 칸이 먼저 필요하다.
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
//!   우리를 침묵으로 끊는다」다. ★유계인 근거는 `write_deadline` 하나다★ — 감독은 **모든 쓰기**에
//!   그 시한을 걸므로(`peer.rs` 의 그 불변식) 걸린 쓰기가 그 시한에 잘리고 통로가 버려진다.
//!   ★`Policy::check` 를 근거로 들지 말 것★ — 그것이 요구하는
//!   `write_deadline < ping_interval < idle_timeout` 은 **디버그 빌드에서만** 확인된다(운영 호출자는
//!   `registry.rs` 의 `debug_assert_eq!` 둘뿐이고 나머지 호출자는 테스트다). 릴리스에서 소비자가 그
//!   순서를 뒤집어도 아무도 막지 않는다. ★그리고 그 부등식은 애초에 **상대의** 인내를 정렬하지
//!   못한다★ — 상대의 `idle_timeout` 은 우리 정책과 무관하게 그쪽에서 설정되므로, 우리 값끼리의
//!   대소로 「우리의 늦음 < 상대의 인내」를 세울 수 없다. 세워지는 것은 「우리 쓰기가 무한히 걸려
//!   있지는 않다」까지다.
//! - ★keepalive **왕복**은 실소켓으로 미검이다★ — 옮기는 표는 이 파일 단위 테스트가 양방향으로
//!   단언하지만(위 「보증」), 실 Ping 이 나가고 실 Pong 이 [`Frame::Keepalive`] 로 돌아오는 것을 재는
//!   테스트는 없다. crate 의 `ping_interval` 이 어느 실소켓 테스트의 수명보다 훨씬 길어 그 왕복이
//!   테스트 안에서 일어나지 않는다. 새면 증상은 바로 위 항목의 그것이다.
//! - ★`Message::Frame` 은 실소켓 수신으로 올라오지 않는다★(tungstenite 문서) — 그래서 그 갈래는
//!   [`Frame`] 으로 옮기지 않고 건너뛴다.
//! - ★[`LinkRx::recv`] 의 취소 안전성은 이 crate 에서 **아무것도 지탱하지 않는다**★ — ★단 사유는
//!   「취소되지 않아서」가 **아니다**★: 운영 단계의 읽기는 링크를 버릴 때마다 실제로 폴링 도중에
//!   떨어진다(`peer.rs` 의 `drop_link` 가 읽는 태스크를 `abort()` 하고 그 끝을 기다린다 — 그 순간
//!   진행 중이던 `recv()` future 가 그대로 버려진다). 지탱하지 않는 진짜 이유는 **취소하는 자리가 모두
//!   그 직후 양 끝을 통째로 버리기 때문**이다: 그 `abort()` 뒤에 `frames_rx`·`rx_raw` 가 함께 놓이고
//!   같은 통로로 다시 읽는 일이 없다. 핸드셰이크 쪽도 같다 — 시한·상대 종료 갈래는 `Err` 로 돌아가며
//!   통로를 버리고(`handshake_once`), 제어가 이긴 갈래는 핸드셰이크 future 를 양 끝과 함께 떨어뜨린다
//!   (`shake_hands`). 즉 취소된 폴링에서 잃을 것이 남아 있는 자리가 없다. ★그래서 여기 「미검」 딱지를
//!   두지 않는다★ — 갚을 빚이 아니라 애초에 지지 않은 빚이고, 딱지가 남으면 다음 세션이 없는 위험을
//!   재려 든다. **취소한 뒤 같은 통로로 다시 읽는 호출을 새로 만들면 그때 이 문장이 무효가 된다.**

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
    (
        Box::new(WsTx { sink }),
        Box::new(WsRx {
            stream,
            failed: false,
        }),
    )
}

// ── 통로 두 끝 ───────────────────────────────────────────────────────────────

struct WsTx<S: WsStream> {
    sink: SplitSink<WebSocketStream<S>, Message>,
}

impl<S: WsStream> LinkTx for WsTx<S> {
    fn send(&mut self, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(async move {
            // ★상한을 넘는 프레임은 **값으로** 실패한다★ — 안 그러면 상한을 집행하는 것이 받는 쪽뿐이라
            //   (파일 헤더 「들어오는 것에는 크기 상한이 있고 나가는 것에는 없다」) 통로가 끊기고
            //   **아무 잘못 없는 상대가 자기 재연결 예산을 태운다**. ★패닉으로 돌아가지 말 것★ —
            //   `debug_assert` 였던 판은 감독 태스크 안에서 터지는데 그 `JoinHandle` 을 아무도 안 쥐고
            //   있어(`spawn_peer`) 패닉을 **관측하는 자리가 없었다**: 상태 watch 의 보내는 쪽이 떨어져
            //   발행된 상태가 `Live` 에 영구히 얼고, 끊김 사건도 포기 사건도 안 나고, 읽는 태스크는
            //   통로 절반을 쥔 채 떨어져 나갔다. 값으로 실패하면 감독이 그것을 `LinkLost` 로 옮겨
            //   (`Supervisor::write`) 끊김 사건과 재연결이 남는다. ★단 **문구는 안 남는다**★ — 그 옮김이
            //   `LinkError` 문구를 버리고 `Disconnected` 에는 문구 칸이 없다(이 파일 헤더의 그 항목).
            let len = payload_len(&frame);
            if len > MAX_OUTGOING_PAYLOAD {
                return Err(LinkError::new(format!(
                    "ws write: 나가는 프레임이 {len} 바이트다 — 받는 쪽 tungstenite 가 max_frame_size {MAX_OUTGOING_PAYLOAD} 에서 통로를 끊는다"
                )));
            }
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
    /// 이 통로에서 이미 읽기가 실패했다.
    ///
    /// ★[`LinkRead::Closed`] 를 「상대가 닫았다고 말했다」로 유지하려고 있다★ — 사유는 [`WsRx::recv`].
    failed: bool,
}

impl<S: WsStream> LinkRx for WsRx<S> {
    fn recv(&mut self) -> BoxFuture<'_, Result<LinkRead, LinkError>> {
        Box::pin(async move {
            // ★한 번 실패한 읽기는 계속 실패한다★ — 그래야 [`LinkRead::Closed`] 계약(「상대가 닫았다고
            //   **말했다**」)이 조건 없이 성립한다. 없으면 이렇게 새는 자리가 있다: 말 없이 버려진
            //   소켓의 첫 읽기는 `Protocol(ResetWithoutClosingHandshake)` 오류지만, 그 뒤 tungstenite
            //   가 자기 `ended` 표식을 세워 **다음 읽기는 `Ready(None)`** 을 낸다(0.26.2 의
            //   `lib.rs:303`). 그 `None` 을 그대로 옮기면 아무도 말하지 않은 닫기가 보고되고, 그 값으로
            //   분기하는 소비자는 「거절당했다/곱게 갔다」를 「끊겼다」와 맞바꾼다.
            if self.failed {
                return Err(LinkError::new("ws read: link already failed"));
            }
            loop {
                let msg = match self.stream.next().await {
                    Some(Ok(msg)) => msg,
                    Some(Err(e)) => {
                        self.failed = true;
                        return Err(LinkError::new(format!("ws read: {e}")));
                    }
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
    fn a_four_byte_character_is_cut_below_the_limit_not_at_it() {
        // ★한 글자가 4 바이트인 입력이 이 테스트의 존재 이유다★ — 3 바이트 글자만 쓰면 123 이 3 의
        //   배수라서 「경계 맞추기를 지우고 123 에서 그냥 자른다」는 변이가 통과한다(실측 2026-09-07:
        //   그 변이 아래에서 옛 입력 넷이 전부 초록이었다). 4 바이트 글자에서는 123 이 경계가 아니라
        //   그 변이가 슬라이스 패닉으로 죽는다.
        let emoji = "\u{1F600}".repeat(40);
        assert_eq!(emoji.len(), 160);
        let cut = clamp_close_reason(&emoji);
        assert_eq!(cut.len(), 120, "4 바이트 글자는 120 에서 잘려야 한다");
        assert_eq!(cut.chars().count(), 30);
        assert!(emoji.starts_with(cut));
    }

    // ── 닫기 경로가 실제로 내보내는 바이트 ──
    //
    // ★[`clamp_close_reason`] 을 직접 부르는 위 테스트들과 짝이다★ — 그것들만으로는 **[`WsTx::close`]
    //   가 그 함수를 부르는지**를 아무도 재지 않아, 자르는 줄을 지우는 변이가 통째로 초록으로 통과한다
    //   (실측 2026-09-07).
    /// 쓴 바이트를 모아 두는 최소 스트림. 실 소켓 없이 나가는 프레임을 그대로 본다.
    struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl tokio::io::AsyncWrite for Recorder {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            self.0.lock().unwrap().extend_from_slice(buf);
            std::task::Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    impl tokio::io::AsyncRead for Recorder {
        /// ★EOF 를 낸다★ — `Pending` 을 내면 닫기 뒷정리가 그 자리에 매달리고, libtest 에는 테스트별
        /// 시한이 없어 그 회귀가 스위트를 통째로 멈춘다.
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn the_close_path_puts_a_legal_control_frame_on_the_wire() {
        let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        // 서버 역할이라 나가는 프레임이 **마스킹되지 않는다** — 그래서 바이트를 그대로 읽는다.
        let ws = WebSocketStream::from_raw_socket(
            Recorder(written.clone()),
            tokio_tungstenite::tungstenite::protocol::Role::Server,
            None,
        )
        .await;
        let (mut tx, _rx) = split_link(ws);
        // ★유계 대기★ — 매달리면 깨끗이 실패한다.
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tx.close(Close::new(CloseCode(4001), "긴".repeat(200))),
        )
        .await
        .expect("닫기가 안 끝났다");

        let bytes = written.lock().unwrap().clone();
        assert_eq!(bytes[0], 0x88, "닫기 프레임이 아니다: {:02x?}", &bytes[..2]);
        // ★두 번째 바이트가 payload 길이다★ — 126 이면 확장 길이로 넘어간 것이고, 그것이 곧 125 초과다
        //   (외부 사실: RFC 6455 §5.5 는 제어 프레임에 확장 길이를 금지하고 tungstenite 는 받는 쪽에서
        //   그것을 `ControlFrameTooBig` 로 끊는다).
        assert!(
            bytes[1] <= 125,
            "제어 프레임 payload 가 {} 바이트다 — 받는 쪽이 통로를 끊는다",
            bytes[1]
        );
        // 코드는 잘려도 그대로 나간다 — 분기 재료가 문구가 아니라 코드인 이유다.
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 4001);
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
