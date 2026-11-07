//! 실소켓 WS 어댑터 — feature `ws` 뒤에 산다. ★이 crate 의 유일한 실전송 구현체다★.
//!
//! ## 이 파일이 소유하는 것
//!
//! 거는 쪽([`WsDialer`] = [`Dialer`] 구현) · 받는 쪽([`WsListener`] = [`Listener`] 구현과 그 수락 정책
//! [`AcceptPolicy`] · 거절 보고 [`AcceptRefusals`]) · 열린 WS 스트림 하나를 [`LinkTx`]/[`LinkRx`] 두 끝으로 나누는
//! 일. ★두 방향이 함께 있는 것이 요점★ — 그래서 이 crate 는 다른 crate 없이 **자기에게 붙어** 인메모리로 못 재는
//! 성질을 잰다(ADR-0177 결정 8).
//!
//! ★이 어댑터의 스트림 감싸기는 crate 안 전용이다★ — 스트림을 두 끝으로 나누는 함수가 crate 밖에 없어, 실 WS 스트림을
//! 받은 통로([`Accepted`])로 내보내는 길은 [`WsListener`] 하나다(TRD 3 §3-5 · §5 17). ★소비자가 검사를 못 건너뛴다는
//! 보증은 아니다★ — [`Accepted::new`] 는 다른 전송 구현이 쓰라고 공개로 둔 것이라, 손으로 쓴 [`LinkTx`]/[`LinkRx`]
//! 로 지은 통로도 명부에 얹힌다. 그 우회는 컴파일러가 아니라 코드 리뷰가 잡는다.
//!
//! ## 받는 쪽이 지키는 것 (TRD 3 §5 1 · 2 · 12 · 14 · 15 · 16 · 18)
//!
//! 정책 값은 전부 소비자가 [`AcceptPolicy`] 로 준다. 이 파일이 고른 값은 루프를 굴리는 둘뿐이다 — 커널 수락 큐
//! 길이(`LISTEN_BACKLOG`)와 수락 오류 뒤 쉬는 시간([`ACCEPT_ERROR_PAUSE`]).
//! - loopback 에만 연다 — 바인드한 주소가 loopback 이 아니면 listen 전에 거부한다.
//! - TCP 수락은 한 태스크가 돌고 WS 업그레이드는 연결마다 따로 띄운 태스크가 한다 — 업그레이드를 안 끝내는 상대
//!   하나가 다른 수락을 막지 않는다. 업그레이드 시한의 기준은 TCP 수락 시각이다 — 띄운 태스크가 늦게 돈 시간도
//!   시한에 들고, 시한을 넘겨 끝난 업그레이드도 받지 않는다.
//! - 업그레이드 칸([`crate::PermitPool`])을 TCP 수락에 쥐고 [`Accepted`] 에 실어 보낸다 — 업그레이드 중 · 준비 큐 ·
//!   받아 갔지만 얹기 전의 통로를 한 셈으로 센다. 다 찼으면 새 TCP 를 수락 직후 닫는다.
//! - 업그레이드 요청의 Origin · Host 머리를 정책에 비춰 밖이면 403 으로 끊는다.
//! - 한 메시지 · 한 프레임의 크기 상한을 받는 쪽 · 나가는 쪽에 같은 값으로 건다.
//! - 위에서 끊은 것은 갈래와 가린 원격 주소로 [`AcceptRefusals`] 에 알린다 — [`Listener::accept`] 의 `Err` 는
//!   청취기의 종결만 뜻한다.
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
//! keepalive 주기(전부 [`crate::Policy`] 와 감독 몫 — ★거는 쪽 어댑터는 자기 시한을 또 걸지 않는다★,
//! [`Dialer`] rustdoc. ★받는 쪽의 업그레이드 시한만 예외다★ — 감독이 생기기 전 구간이라 [`AcceptPolicy`] 에
//! 산다) · 인증·버전 협상(핸드셰이크 프레임은 [`crate::Handshake`] 구현체가 소유한다) · 받는 쪽의 수락 루프와
//! 상대 명부(소비자가 [`Listener::accept`] 를 부르고 [`crate::Registry::attach`] 로 얹는다).
//!
//! ## 알려진 한계 (보증으로 읽지 말 것)
//!
//! - ★업그레이드 칸이 다 찼을 때의 거절은 HTTP 응답이 없다★ — TCP 를 수락 직후 그냥 닫으므로 거는 쪽에는
//!   업그레이드 중 끊김(못 붙음)으로 보인다. 칸을 채우는 것은 같은 PC 의 누구든 할 수 있다 — loopback 은 OS 사용자
//!   모두가 함께 쓴다(TRD 3 §5 14 · R11 — 알려진 한계).
//! - ★거절 보고는 잃을 수 있다★ — [`AcceptRefusals`] 의 줄([`AcceptPolicy::refusal_queue`])이 차면 새 보고를 버리고
//!   [`AcceptRefusals::dropped`] 만 올린다. 거절이 쏟아지는 동안 수락 루프가 소비자를 기다리면 그게 곧 서비스
//!   거부라서다.
//! - ★TCP 수락 오류는 종결이 아니다★ — 보고하고 다시 받는다. 연결 하나에 묶인 오류(`ConnectionRefused` ·
//!   `ConnectionAborted` · `ConnectionReset` — 받기 전에 상대가 끊었다)는 쉬지 않고 곧바로 다음을 받고, 그 밖의
//!   오류(파일 핸들이 바닥난 `EMFILE` 등)에만 [`ACCEPT_ERROR_PAUSE`] 를 쉰다 — 그 오류는 연결을 커널 수락 큐에 남겨
//!   곧바로 다시 나기 때문이다. 갈래는 axum 0.8.9 의 수락 루프와 같다(소스로 읽은 것이고 실측은 아니다 —
//!   `serve/listener.rs` 의 `handle_accept_error` 가 같은 셋을 `is_connection_error` 로 건너뛰고 나머지에만 1 초를
//!   쉬며, 그 주석이 출처로 hyper 0.14 를 든다). 그래서 [`Listener::accept`] 가 `Err` 를 내는 것은 수락 태스크가
//!   끝났을 때(패닉)뿐이다.
//! - ★TLS 를 켜지 않았다★ — `tokio-tungstenite` 의 default feature 만 쓰므로 `wss://` 는 dial 단계에서
//!   실패한다.
//! - ★나가는 닫기 **문구**는 넘치면 조용히 잘린다★ — 상한은 123 바이트(코드 2 바이트를 뺀 제어 프레임
//!   payload 125 바이트, RFC 6455 §5.5)와 그 통로의 크기 상한 − 2 바이트 중 작은 쪽이고 자르는 자리가
//!   [`LinkTx::close`] 구현이다. ★말줄임표도 사건도 없다★ — 잘렸다는 것을 소비자가 알 길이 없고, 겪어 보는
//!   것이 유일한 발견 경로다. 코드는 그대로 나가므로 분기는 안 무너진다(문구에 판정 재료를 담지 말 것) — 단
//!   크기 상한이 2 바이트 밑이면 코드도 못 실어 빈 닫기가 나간다. 상한 값의 정본은 아래 `close_reason_room`
//!   이고 그 사유는 그 함수와 `MAX_CLOSE_REASON_BYTES` rustdoc 에 있다.
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
//! - ★잘린 쓰기의 프레임이 뒤늦게 나갈 수 있다★ — 소스를 읽어 확인한 것이다(실행으로 잰 것이 아니다).
//!   `SinkExt::send` 는 항목을 넘긴 **뒤** `flush` 에서 기다리고(futures-util 0.3.32 `sink/feed.rs:38-40`
//!   → `sink/send.rs:37`), 넘겨진 항목은 `SplitSink` 의 한 칸(`stream/stream/split.rs:114`)이나
//!   tungstenite 쓰기 버퍼(tokio-tungstenite 0.26.2 `lib.rs:340-347`)에 남는다. 그 자리에서 취소돼도 같은
//!   끝으로 다음에 쓰는 것이 그것을 먼저 밀어낸다(`split.rs:104-109`). 감독은 제어나 시한으로 쓰기를
//!   자른 뒤 통로를 버리며 [`LinkTx::close`] 로 닫기를 보내므로 **잘린 프레임이 그 닫기 앞에 실려 나갈
//!   수 있고**, 그래서 감독이 그것을 유실로 세는 것은 부풀 수 있다([`crate::Direction::Outbound`]).
//!   ★인메모리 하네스는 이 모양을 흉내내지 않는다★ — 막힌 쓰기가 넘기기 **전에** 멈추므로 잘리면
//!   아무것도 안 남는다(`testing::MemTx`).
//! - ★tungstenite 는 들어오는 것의 크기만 재고 나가는 것은 안 잰다★ — 소스를 읽어 확인한 것(0.26.2: 읽기 쪽만
//!   `max_frame_size` · `max_message_size` 를 보고 `WebSocketContext::write` 에는 크기 검사가 없다). 넘으면 **받는 쪽
//!   읽기 오류**라 통로가 끊긴다(실측 — 기본값 아래에서 32 MiB 한 프레임을 보내면 받는 쪽이 `Space limit exceeded:
//!   Message too long: 33554432 > 16777216` 으로 끊고, **전송 계층은 그것을 막지 않는다**). 이 어댑터는 두 방향에
//!   소비자가 준 한 값을 건다 — 거는 쪽은 [`WsDialer::new`] · 받는 쪽은 [`AcceptPolicy::max_message_bytes`]. 받는
//!   쪽은 `max_message_size` = `max_frame_size` = 그 값이다.
//!   ★그래서 [`LinkTx::send`] 구현이 그 값을 넘는 프레임을 **오류로 돌려준다**★ — 상한을 집행하는 것이 받는
//!   쪽뿐이면 **아무 잘못 없는 상대가 자기 재연결 예산을 태운다**. ★이 거절은 「상대도 같은 값을 쓴다」 위에
//!   선다★ — 더 작은 상한을 쓰는 상대에게는 여전히 받는 쪽이 끊는다. ★패닉이 아니라 값인 것이
//!   요점이다★ — 감독 태스크 안에서 터진 패닉은 패닉 훅을 건 운영 프로세스면 프로세스째 끝내고
//!   (ADR-0292), 훅 없는 프로세스면 그 `JoinHandle` 을 아무도 안 쥐므로 관측되지 않은 채 발행 상태가
//!   `Live` 에 얼어붙는다(그 자리 주석이 증상을 든다). 오류는 감독이 `LinkLost` 로 옮겨
//!   끊김 사건 + 재연결로 만든다. 릴리스와 디버그가 같게 동작한다.
//! - ★**단 이 갈래에 진단은 안 남는다**★ — 위 오류 문구는 감독이 `Ok(Err(_))` 를 `LinkLost(WriteFailed)`
//!   로 옮기면서 버리고(`peer.rs` 의 `Supervisor::write`), [`crate::TransportEvent::Disconnected`] 에는
//!   문구 칸이 아예 없다(`lib.rs` 「운영 중 끊김은 사유 문구를 안 나른다」). 상한을 넘긴 그 프레임은
//!   알림·팬아웃이면 나가는 쪽 유실 한 건으로 잡히고 요청이면 `FailPending` 이 깨우지만, 어느 쪽도
//!   「너무 커서」를 말하지 않는다. 소비자가 보는 것은 **사유 없는 `WriteFailed` 끊김**이다. 이 갈래를
//!   진단 가능하게 하려면 끊김 사건에 문구 칸이 먼저 필요하다.
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

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpSocket};
use tokio::sync::mpsc;
use tokio::task::{JoinHandle, JoinSet};
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};
use tokio_tungstenite::tungstenite::http::{header, HeaderMap, StatusCode};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode as WsCloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Bytes, Message};
use tokio_tungstenite::WebSocketStream;

use crate::clock::{with_deadline, Clock, Elapsed};
use crate::frame::{Close, CloseCode, Frame};
use crate::link::{
    Accepted, Address, ConnPermit, DialFailure, Dialer, LinkError, LinkInfo, LinkRead, LinkRx,
    LinkTx, Listener, PermitPool,
};
use crate::mask::mask_reason;

/// 닫기 문구가 실릴 수 있는 최대 바이트 — **문자 수가 아니다**.
///
/// ★외부 사실★ — RFC 6455 §5.5 가 제어 프레임 payload 를 125 바이트로 못박고 닫기 프레임은 그 앞
/// 2 바이트를 코드에 쓴다. tungstenite 0.26.2 는 **나가는 쪽을 검사하지 않지만**(`Frame::close` 가
/// 코드와 문구를 그대로 이어 붙인다) **받는 쪽에서 125 초과를 `ControlFrameTooBig` 로 끊는다**
/// (`protocol/mod.rs:640`). 그래서 문구가 길면 「붙었는데 거절」이 상대에게 **읽기 오류**로 도착해
/// 거절 판정이 무너지고 재연결 예산까지 태운다 — [`WsTx::close`] 가 그 앞에서 자른다.
const MAX_CLOSE_REASON_BYTES: usize = 123;

/// 커널 수락 큐 길이 — tokio `TcpListener::bind` 가 쓰는 값과 같다. ★업그레이드 칸과 무관하다★ — 이 큐에 앉은
/// 연결은 아직 우리가 수락하지 않았다.
const LISTEN_BACKLOG: u32 = 1024;

/// TCP 수락 오류 뒤 다시 받기 전에 쉬는 시간 — 사유와 쉬는 갈래는 파일 헤더 「알려진 한계」. ★보안 값이 아니라 루프가
/// 도는 것을 막는 값이라 [`AcceptPolicy`] 칸으로 올리지 않았다★ — 오류 하나에 연결 하나를 늦출 뿐이다.
///
/// ★청취기에 준 [`Clock`] 이 아니라 tokio 실시계로 쉰다★ — 실소켓 수락 오류에만 걸리는 쉼이라 손으로 미는 시계로 잴
/// 것이 없고, 그 시계를 따르면 시험이 시계를 밀 때까지 수락 루프가 멈춘다.
pub const ACCEPT_ERROR_PAUSE: Duration = Duration::from_millis(100);

/// TCP 수락 오류 뒤 [`ACCEPT_ERROR_PAUSE`] 를 쉬나. ★연결 하나에 묶인 오류는 쉬지 않는다★ — 받기 전에 상대가 끊은
/// 것이라 다음 수락은 다른 연결이고, 거기서 쉬면 연결을 열자마자 끊는 상대가 받기를 늦출 수 있다(파일 헤더 「알려진
/// 한계」).
fn pauses_after_accept_error(kind: io::ErrorKind) -> bool {
    !matches!(
        kind,
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    )
}

/// 크기 상한이 `max_payload` 인 통로에서 닫기 문구에 쓸 수 있는 바이트 — [`MAX_CLOSE_REASON_BYTES`] 와
/// `max_payload` − 2(코드 몫) 중 작은 쪽. `None` = `max_payload` 가 2 밑이라 코드도 못 싣는다.
///
/// ★`max_payload` 로도 자르는 까닭★(외부 사실) — tungstenite 0.26.2 는 받는 쪽 `max_frame_size` 를 프레임 머리를 읽자마자
/// 갈래 없이 건다(`protocol/frame/mod.rs:181` — 제어 프레임도 든다). 125 바이트로만 자르면 상한을 그 밑으로 준 통로에서
/// 닫기 프레임 하나가 상대 읽기를 `MessageTooLong` 으로 끊는다.
fn close_reason_room(max_payload: usize) -> Option<usize> {
    max_payload
        .checked_sub(2)
        .map(|room| room.min(MAX_CLOSE_REASON_BYTES))
}

/// 닫기 문구를 `room` 바이트 안으로 자른다([`close_reason_room`]).
///
/// ★문자 경계에서 자른다★ — 바이트로 자르면 깨진 UTF-8 이 되고, 그건 길이 초과와 **다른** 방식으로
/// 실패한다(tungstenite 의 `Utf8Bytes` 가 문구를 UTF-8 로 보증한다).
fn clamp_close_reason(reason: &str, room: usize) -> &str {
    if reason.len() <= room {
        return reason;
    }
    &reason[..reason.floor_char_boundary(room)]
}

/// 받는 쪽 크기 상한 — 한 메시지와 한 프레임에 같은 값(TRD 3 §5 15). 나머지 칸은 tungstenite 기본값이다.
fn ws_config(max_message_bytes: usize) -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(max_message_bytes))
        .max_frame_size(Some(max_message_bytes))
}

/// 통로 한 끝에 필요한 바탕 성질. `MaybeTlsStream<TcpStream>`(거는 쪽)과 `TcpStream`(받는 쪽)이 둘 다
/// 들어서, 두 방향이 같은 [`LinkTx`]/[`LinkRx`] 구현을 쓴다.
pub(crate) trait WsStream: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

impl<S> WsStream for S where S: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

// ── 거는 쪽 ──────────────────────────────────────────────────────────────────

/// 실 WS 통로를 여는 [`Dialer`].
///
/// [`Address`] 는 `tokio-tungstenite` 에 그대로 넘어간다 — 즉 `ws://host:port/path` 문법이고, 그 문법
/// 위반도 [`LinkError`] 로 온다(주소를 crate 가 해석하지 않는다는 계약 그대로다).
///
/// ★실패는 전부 [`DialFailure::Unreachable`] 이다 — [`DialFailure::Refused`] 를 내지 않는다★. 업그레이드 단계의 HTTP
/// 거절(101 이 아닌 상태 — 예: Origin 검사의 403)도 그렇다: 그 응답에는 닫기 코드가 없고, 상태 번호를 종결 · 다시
/// 걸기로 옮기는 것은 이 어댑터가 모르는 정책이다(지어내면 일시 장애가 예산 없는 포기가 된다). 걸기 전에 종결을 아는
/// 소비자는 이것을 감싼 자기 [`Dialer`] 에서 `Refused` 를 돌려준다(TRD 3 §3-0 ⑤).
pub struct WsDialer {
    max_message_bytes: usize,
}

impl WsDialer {
    /// `max_message_bytes` = 한 메시지 · 한 프레임의 바이트 상한 — 받는 쪽 크기 상한이자 나가는 프레임을 오류로
    /// 돌려주는 경계다(파일 헤더 「통로 성질」). 상대 청취기의 [`AcceptPolicy::max_message_bytes`] 와 같은 값을 준다.
    pub fn new(max_message_bytes: usize) -> Self {
        Self { max_message_bytes }
    }
}

impl Dialer for WsDialer {
    fn dial(
        &self,
        addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>> {
        // ★통로를 future 안에서 만든다★ — [`Dialer`] 계약이 그것을 요구한다(취소되면 아무것도 남지
        //   않아야 한다).
        // ★주소를 소유한 String 으로 옮겨 담는 것은 **시그니처가 강제한다**★ — 반환 타입의 `'_` 는
        //   `&self` 에 묶이고 `addr` 에는 안 묶이므로(수명 생략 규칙) future 가 `addr` 를 빌릴 수 없다.
        let url = addr.as_str().to_string();
        let max = self.max_message_bytes;
        Box::pin(async move {
            let (ws, _response) = tokio_tungstenite::connect_async_with_config(
                url.as_str(),
                Some(ws_config(max)),
                false,
            )
            .await
            .map_err(|e| LinkError::new(format!("ws connect {url}: {e}")))?;
            Ok(split_link(ws, max))
        })
    }
}

// ── 받는 쪽 ──────────────────────────────────────────────────────────────────

/// [`WsListener`] 의 수락 정책(TRD 3 §5 17 · C1).
///
/// ★`Default` 가 없다★ — crate 가 값을 고르면 그 값이 곧 소비자 모르게 정해진 보안 정책이 된다. ★허용 전부를 뜻하는
/// 값도 없다★ — 검사를 끄는 길이 없어서 시험도 운영과 같은 칸에 실제 값을 넣어 돈다. ★`#[non_exhaustive]` 도 아니다★
/// — 칸이 늘면 소비자의 생성 자리가 컴파일 오류로 그 값을 고르게 한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptPolicy {
    /// Origin 머리를 단 업그레이드 중 받는 값 — 바이트 그대로 견준다(TRD 3 §5 2).
    /// ★비면 Origin 을 단 업그레이드를 전부 403 으로 끊는다★(D21 ① 의 릴리스 값). ★Origin 이 없는 업그레이드는 늘
    /// 받는다★ — 브라우저 밖 클라는 그 머리를 안 보내고, 이 검사가 막는 것은 브라우저 페이지가 띄운 요청이다(인증은
    /// 핸드셰이크 몫). 머리가 둘 이상이거나 글자가 아니면 끊는다.
    pub allowed_origins: Vec<String>,
    /// Host 머리의 이름 쪽(포트 앞 — 대소문자 무시)으로 받는 값. 예 `127.0.0.1` · `localhost`(TRD 3 §5 16 — DNS
    /// 리바인딩). 포트는 이 청취기가 바인드한 것이어야 한다(포트 없음 = 80). ★Host 가 없는 업그레이드는 받는다★(D9) ·
    /// 머리가 둘 이상이거나 글자가 아니면 끊는다. 비면 Host 를 단 업그레이드를 전부 끊는다.
    /// ★IPv6 이름은 대괄호째 적는다★(`[::1]`) — Host 머리가 IPv6 를 그렇게 싣고 이름을 그대로 견준다. 대괄호 밖에 콜론이
    /// 있는 이름(`::1` · `localhost:8080`)은 어느 머리와도 안 맞으므로 [`WsListener::bind`] 가 `Err` 로 돌려준다.
    pub allowed_host_names: Vec<String>,
    /// TCP 수락부터 WS 업그레이드 끝까지의 시한(TRD 3 §5 12) — 넘기면 그 TCP 를 닫는다. ★받은 상대의 인증 시한은 이
    /// 시한이 끝난 자리([`Accepted::upgraded_at`])부터 잰다★ — 둘은 겹치지 않고 더해진다.
    pub upgrade_timeout: Duration,
    /// 업그레이드 칸 수(TRD 3 §5 14) — TCP 수락에 쥐고 얹은 상대가 운영 단계에 들 때(또는 [`Accepted`] 를 떨어뜨릴 때)
    /// 놓는다. ★준비 큐도 이 칸으로 센다★ — 그래서 준비 큐의 상한이 곧 이 값이고 따로 세지 않는다. 다 찼으면 새 TCP 를
    /// 수락 직후 닫는다. 0 = 아무것도 안 받는다.
    pub max_pending_upgrades: usize,
    /// 한 메시지 · 한 프레임의 바이트 상한(TRD 3 §5 15) — 받은 통로의 받는 쪽 상한이자 나가는 프레임을 오류로
    /// 돌려주는 경계다(파일 헤더 「통로 성질」). 거는 쪽 [`WsDialer::new`] 에 같은 값을 준다.
    pub max_message_bytes: usize,
    /// 거절 보고 줄의 칸 수(TRD 3 §5 18) — 차면 새 보고를 버리고 [`AcceptRefusals::dropped`] 를 올린다. 0 은 1 로 친다.
    pub refusal_queue: usize,
}

/// 청취기가 상대 하나를 끊은 까닭 — [`AcceptRefusal::kind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RefusalKind {
    /// 업그레이드 칸이 다 찼다 — TCP 를 수락 직후 닫았다(HTTP 응답 없음).
    SlotsFull,
    /// [`AcceptPolicy::upgrade_timeout`] 안에 업그레이드가 안 끝났다.
    UpgradeTimeout,
    /// Origin 머리가 [`AcceptPolicy::allowed_origins`] 밖이다 — 403.
    OriginRejected,
    /// Host 머리가 [`AcceptPolicy::allowed_host_names`] · 바인드 포트 밖이다 — 403.
    HostRejected,
    /// 업그레이드가 위 갈래 밖의 까닭으로 실패했다 — WS 요청이 아니다 · 상대가 업그레이드 중 끊었다 등.
    UpgradeFailed,
    /// TCP 수락 자체가 실패했다 — 상대가 없을 수 있어 원격 주소가 비어 있다.
    AcceptError,
}

/// 끊은 상대 하나의 보고(TRD 3 §5 18).
///
/// ★글 칸은 이미 [`crate::mask_reason`] 으로 가린 값이다★ — 그대로 찍어도 된다. ★인증 프레임을 읽기 전에 끊으므로
/// 토큰이 실릴 자리가 없다★.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AcceptRefusal {
    pub kind: RefusalKind,
    /// 상대 끝 주소(`ip:port`). [`RefusalKind::AcceptError`] 면 `None`.
    pub remote: Option<String>,
    /// 업그레이드 요청의 Origin 머리 — 머리까지 읽은 Origin · Host 거절에서만 채운다.
    pub origin: Option<String>,
    /// 업그레이드 요청의 Host 머리 — 위와 같다.
    pub host: Option<String>,
}

/// [`WsListener::bind`] 가 청취기와 함께 돌려주는 거절 보고 수신단 — [`Listener`] trait 밖에서 알리는 자리다(그 trait
/// 의 `Err` 는 종결만 뜻한다).
///
/// ★떨어뜨려도 된다★ — 그 뒤의 보고는 버려지고 청취기는 그대로 돈다. ★읽지 않으면 줄이 차 새 보고부터 잃는다★ —
/// 잃은 수는 [`AcceptRefusals::dropped`]. 청취기를 떨어뜨리면 남은 보고를 내준 뒤 `None` 으로 끝난다.
pub struct AcceptRefusals {
    rx: mpsc::Receiver<AcceptRefusal>,
    dropped: Arc<AtomicU64>,
}

impl AcceptRefusals {
    /// 다음 보고를 기다린다. ★취소 안전★.
    pub async fn recv(&mut self) -> Option<AcceptRefusal> {
        self.rx.recv().await
    }

    /// 지금 쌓인 보고 하나 — 없으면 `None`(기다리지 않는다).
    pub fn try_recv(&mut self) -> Option<AcceptRefusal> {
        self.rx.try_recv().ok()
    }

    /// 줄이 차서 버린 보고의 누적 수 — 줄지 않는다.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Acquire)
    }
}

/// 거절 보고를 줄에 넣는다 — 차면 버리고 센다.
struct Reporter {
    tx: mpsc::Sender<AcceptRefusal>,
    dropped: Arc<AtomicU64>,
}

impl Reporter {
    fn report(
        &self,
        kind: RefusalKind,
        remote: Option<SocketAddr>,
        origin: Option<&str>,
        host: Option<&str>,
    ) {
        let refusal = AcceptRefusal {
            kind,
            remote: remote.map(|addr| mask_reason(&addr.to_string())),
            origin: origin.map(mask_reason),
            host: host.map(mask_reason),
        };
        // ★기다리지 않는다★ — 수락 루프 · 업그레이드 태스크가 소비자를 기다리면 거절이 쏟아질 때 받기가 선다.
        if self.tx.try_send(refusal).is_err() {
            self.dropped.fetch_add(1, Ordering::AcqRel);
        }
    }
}

/// 실 WS 통로를 받는 [`Listener`].
///
/// TCP 수락은 청취기 안의 한 태스크가 돌고, 업그레이드는 연결마다 따로 띄운 태스크가 한다. 업그레이드를 마친 통로는
/// 준비 큐에 앉아 [`Listener::accept`] 를 기다린다(그 구간도 업그레이드 칸을 쥔다). ★떨어뜨리면 그 태스크들을 거두고
/// 준비 큐의 통로를 닫는다★ — 거는 쪽은 그것을 통로 손실로 본다.
pub struct WsListener {
    local: SocketAddr,
    ready: mpsc::Receiver<Accepted>,
    permits: PermitPool,
    driver: JoinHandle<()>,
}

impl WsListener {
    /// `addr` 에 열고 수락을 시작한다 — 청취기와 그 거절 보고 수신단을 돌려준다. 포트 0을 주면 커널이 고른 포트로
    /// 열린다(실제 번호는 [`WsListener::local_addr`]).
    ///
    /// ★loopback 에만 연다★(TRD 3 §5 1 · C8) — 바인드한 주소가 loopback 이 아니면 listen 하기 전에 `Err` 다. 평문
    /// `ws://` 를 밖으로 열지 않는다(원격은 TLS 와 함께 — TRD 3 §4). [`AcceptPolicy::allowed_host_names`] 에 대괄호 밖
    /// 콜론이 있는 이름이 있어도 `Err` 다 — 그 이름은 아무것도 안 받는 정책을 조용히 세운다.
    ///
    /// ★`clock` 은 받은 통로를 얹을 명부와 같은 시계여야 한다★ — [`Accepted::upgraded_at`] 을 이것으로 찍고 명부가 그
    /// 시각부터 인증 시한을 잰다([`crate::Registry::attach_with_policy`] 는 debug 빌드에서 그 일치를 단언한다).
    /// 업그레이드 시한도 이것으로 잰다.
    ///
    /// tokio 런타임 안에서 부른다 — 밖이면 패닉한다(수락 태스크를 띄운다).
    pub fn bind(
        addr: SocketAddr,
        policy: AcceptPolicy,
        clock: Arc<dyn Clock>,
    ) -> Result<(Self, AcceptRefusals), LinkError> {
        ensure_host_names(&policy.allowed_host_names)?;
        let socket = if addr.is_ipv4() {
            TcpSocket::new_v4()
        } else {
            TcpSocket::new_v6()
        }
        .map_err(|e| LinkError::new(format!("ws bind {addr}: {e}")))?;
        socket
            .bind(addr)
            .map_err(|e| LinkError::new(format!("ws bind {addr}: {e}")))?;
        let local = socket
            .local_addr()
            .map_err(|e| LinkError::new(format!("ws local_addr: {e}")))?;
        // ★listen 앞에서 본다★ — 들은 뒤 거부하면 그 사이 밖에서 온 연결이 커널 수락 큐에 이미 들어온다.
        ensure_loopback(local)?;
        let tcp = socket
            .listen(LISTEN_BACKLOG)
            .map_err(|e| LinkError::new(format!("ws listen {local}: {e}")))?;

        let permits = PermitPool::new(policy.max_pending_upgrades);
        // ★준비 큐가 차는 일은 없다★ — 앉은 통로마다 칸을 쥐므로 큐 길이 ≤ 쥔 칸 ≤ 이 용량이다.
        let (ready_tx, ready) = mpsc::channel(policy.max_pending_upgrades.max(1));
        let (refusal_tx, refusal_rx) = mpsc::channel(policy.refusal_queue.max(1));
        let dropped = Arc::new(AtomicU64::new(0));
        let shared = Arc::new(Shared {
            local,
            permits: permits.clone(),
            clock,
            reporter: Reporter {
                tx: refusal_tx,
                dropped: dropped.clone(),
            },
            policy,
        });
        let driver = tokio::spawn(drive(tcp, shared, ready_tx));
        Ok((
            Self {
                local,
                ready,
                permits,
                driver,
            },
            AcceptRefusals {
                rx: refusal_rx,
                dropped,
            },
        ))
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// 이 청취기에 붙는 [`Address`] — [`WsDialer`] 에 그대로 넣는다.
    pub fn dial_address(&self) -> Address {
        Address::new(format!("ws://{}/", self.local))
    }

    /// 지금 쥔 업그레이드 칸 수 — 업그레이드 중 · 준비 큐 · 받아 갔지만 아직 놓지 않은 통로를 함께 센다.
    ///
    /// ★칸 묶음 자체는 내주지 않는다★ — 내주면 그것으로 칸을 쥐어 이 청취기의 수락을 막을 수 있다.
    pub fn permits_in_use(&self) -> usize {
        self.permits.in_use()
    }

    /// 업그레이드 칸 수 — [`AcceptPolicy::max_pending_upgrades`].
    pub fn permit_limit(&self) -> usize {
        self.permits.limit()
    }

    /// 준비 큐에 앉아 아직 안 받아 간 수.
    pub fn queued(&self) -> usize {
        self.ready.len()
    }
}

impl Listener for WsListener {
    fn accept(&mut self) -> BoxFuture<'_, Result<Accepted, LinkError>> {
        Box::pin(async move {
            self.ready
                .recv()
                .await
                .ok_or_else(|| LinkError::new("ws listener: accept task ended"))
        })
    }
}

impl Drop for WsListener {
    fn drop(&mut self) {
        // 수락 태스크가 업그레이드 태스크들의 묶음을 쥐고 있어 함께 거둬진다(`drive`).
        self.driver.abort();
    }
}

/// 바인드한 주소가 loopback 인지 — 아니면 거부한다(TRD 3 §5 1).
fn ensure_loopback(local: SocketAddr) -> Result<(), LinkError> {
    if local.ip().is_loopback() {
        Ok(())
    } else {
        Err(LinkError::new(format!(
            "ws bind {local}: loopback 이 아니다 — 평문 ws:// 청취기는 loopback 에만 연다"
        )))
    }
}

/// 받는 Host 이름에 대괄호 밖 콜론이 없나 — 있으면 거부한다([`AcceptPolicy::allowed_host_names`]).
fn ensure_host_names(names: &[String]) -> Result<(), LinkError> {
    let bracketed = |name: &str| name.starts_with('[') && name.ends_with(']');
    match names
        .iter()
        .find(|name| name.contains(':') && !bracketed(name))
    {
        Some(name) => Err(LinkError::new(format!(
            "ws accept policy: Host 이름 {name:?} 에 대괄호 밖 콜론이 있다 — IPv6 는 `[::1]` 꼴로 · 포트는 빼고 적는다"
        ))),
        None => Ok(()),
    }
}

/// 수락 태스크와 업그레이드 태스크들이 함께 읽는 것.
struct Shared {
    local: SocketAddr,
    policy: AcceptPolicy,
    permits: PermitPool,
    clock: Arc<dyn Clock>,
    reporter: Reporter,
}

/// TCP 수락 루프. ★업그레이드를 여기서 기다리지 않는다★ — 연결마다 태스크를 띄워 느린 상대 하나가 다음 수락을 막지
/// 않게 한다(TRD 3 §5 12). 띄운 태스크는 `upgrades` 가 쥐어, 이 태스크가 거둬지면(청취기를 떨어뜨림) 함께 거둬진다.
async fn drive(tcp: TcpListener, shared: Arc<Shared>, ready: mpsc::Sender<Accepted>) {
    let mut upgrades = JoinSet::new();
    loop {
        tokio::select! {
            accepted = tcp.accept() => match accepted {
                Ok((stream, remote)) => match shared.permits.try_take() {
                    Some(permit) => {
                        // ★시한의 기준을 띄우기 전 여기서 찍는다★ — 업그레이드 태스크 안에서 찍으면 띄운 뒤 처음 돌기까지의
                        //   지연이 시한 밖에 남는다(TRD 3 §5 12 — TCP 수락부터).
                        let accepted_at = shared.clock.now();
                        upgrades.spawn(upgrade(
                            stream,
                            remote,
                            accepted_at,
                            permit,
                            shared.clone(),
                            ready.clone(),
                        ));
                    }
                    None => {
                        // 넘친 것은 **새것을** 닫는다(TRD 3 §5 14) — 이미 칸을 쥔 상대는 건드리지 않는다.
                        drop(stream);
                        shared
                            .reporter
                            .report(RefusalKind::SlotsFull, Some(remote), None, None);
                    }
                },
                Err(error) => {
                    shared.reporter.report(RefusalKind::AcceptError, None, None, None);
                    if pauses_after_accept_error(error.kind()) {
                        tokio::time::sleep(ACCEPT_ERROR_PAUSE).await;
                    }
                }
            },
            // 끝난 업그레이드 태스크를 걷는다 — 안 걷으면 결과가 묶음에 쌓인다.
            Some(_) = upgrades.join_next(), if !upgrades.is_empty() => {}
        }
    }
}

/// 업그레이드 요청 머리에 대한 판정. 끊으면 그 갈래와 본 머리 값(가리기 전)을 싣는다.
#[derive(Debug, PartialEq, Eq)]
struct Screened {
    kind: RefusalKind,
    origin: Option<String>,
    host: Option<String>,
}

/// 통로 하나의 업그레이드 — 시한 시각(`accepted_at` + [`AcceptPolicy::upgrade_timeout`]) 전에 끝나면 준비 큐에
/// 앉히고, 아니면 닫고 보고한다. `accepted_at` = TCP 수락 시각(`shared.clock`). `permit` 은 받은 통로에 실려 나가거나
/// 여기서 떨어진다.
///
/// ★시한을 세 자리에서 본다★ — 시작 전(띄운 태스크가 늦게 돌아 이미 넘겼다) · 기다리는 동안 · 끝난 뒤. 끝난 뒤를 따로
/// 보는 것은 끝남과 시한이 한 폴링에 함께 서면 [`with_deadline`] 이 업그레이드 쪽을 먼저 폴링해 그쪽이 이기기
/// 때문이다. 시한 시각 **그 순간**부터 넘긴 것으로 친다.
async fn upgrade<S: WsStream>(
    stream: S,
    remote: SocketAddr,
    accepted_at: Instant,
    permit: ConnPermit,
    shared: Arc<Shared>,
    ready: mpsc::Sender<Accepted>,
) {
    let policy = &shared.policy;
    let report = |kind, origin: Option<&str>, host: Option<&str>| {
        shared.reporter.report(kind, Some(remote), origin, host)
    };
    // 시한까지 남은 시간 — `None` = 시한 시각에 닿았거나 지났다. ★시각에 시한을 더하지 않고 지난 시간을 뺀다★ —
    //   `Instant + Duration` 은 넘치면 패닉하는데 시한 칸에는 상한이 없다.
    let remaining_at = |now: Instant| {
        policy
            .upgrade_timeout
            .checked_sub(now.saturating_duration_since(accepted_at))
            .filter(|left| !left.is_zero())
    };
    let Some(remaining) = remaining_at(shared.clock.now()) else {
        report(RefusalKind::UpgradeTimeout, None, None);
        return;
    };
    let mut verdict: Option<Result<Option<String>, Screened>> = None;
    let handshake = tokio_tungstenite::accept_hdr_async_with_config(
        stream,
        Gate {
            policy,
            local_port: shared.local.port(),
            verdict: &mut verdict,
        },
        Some(ws_config(policy.max_message_bytes)),
    );
    let upgraded = with_deadline(&*shared.clock, remaining, handshake).await;
    let ws = match upgraded {
        Ok(Ok(ws)) => ws,
        Ok(Err(_)) => {
            match verdict {
                Some(Err(screened)) => report(
                    screened.kind,
                    screened.origin.as_deref(),
                    screened.host.as_deref(),
                ),
                _ => report(RefusalKind::UpgradeFailed, None, None),
            }
            return;
        }
        Err(Elapsed) => {
            // ★머리 판정으로 끊고 403 을 쓰던 중이면 그 갈래로 알린다★ — 시한으로 알리면 끊은 까닭과 본 머리가
            //   사라진다(상대가 403 을 안 읽어 쓰기가 멎은 경우).
            match verdict {
                Some(Err(screened)) => report(
                    screened.kind,
                    screened.origin.as_deref(),
                    screened.host.as_deref(),
                ),
                _ => report(RefusalKind::UpgradeTimeout, None, None),
            }
            return;
        }
    };
    // ★업그레이드가 끝난 그 순간을 명부와 같은 시계로 찍는다★ — 인증 시한이 여기서 시작한다(TRD 3 §5 3).
    let upgraded_at = shared.clock.now();
    if remaining_at(upgraded_at).is_none() {
        // 시한을 넘겨 끝났다 — 끝난 업그레이드라도 받지 않는다(이 함수 rustdoc). 통로와 칸은 여기서 떨어진다.
        report(RefusalKind::UpgradeTimeout, None, None);
        return;
    }
    let origin = match verdict {
        Some(Ok(origin)) => origin,
        _ => None,
    };
    let (tx, rx) = split_link(ws, policy.max_message_bytes);
    let info = LinkInfo {
        remote: Some(remote.to_string()),
        origin,
        peer_key: None,
    };
    let accepted = Accepted::new(tx, rx, info, upgraded_at, permit);
    match ready.try_send(accepted) {
        Ok(()) => {}
        // 닿지 않는 갈래다(`WsListener::bind` 의 용량 주석) — 닿으면 칸이 찬 것과 같게 다룬다.
        Err(mpsc::error::TrySendError::Full(_)) => report(RefusalKind::SlotsFull, None, None),
        // 청취기를 떨어뜨렸다 — 통로는 여기서 닫힌다.
        Err(mpsc::error::TrySendError::Closed(_)) => {}
    }
}

/// tungstenite 의 머리 콜백 — 업그레이드 요청 머리를 정책에 비추고 그 판정을 `verdict` 에 남긴다(통과면 받을
/// Origin · 아니면 끊은 갈래). 끊으면 403 을 돌려준다.
struct Gate<'a> {
    policy: &'a AcceptPolicy,
    local_port: u16,
    verdict: &'a mut Option<Result<Option<String>, Screened>>,
}

impl Callback for Gate<'_> {
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        let screened = screen(self.policy, request.headers(), self.local_port);
        let passed = screened.is_ok();
        *self.verdict = Some(screened);
        if passed {
            Ok(response)
        } else {
            Err(forbidden())
        }
    }
}

/// 403 응답. ★어느 검사에 걸렸는지 본문에 싣지 않는다★ — 상대에게 정책을 알려 줄 까닭이 없고, 갈래는 거절 보고가
/// 나른다.
fn forbidden() -> ErrorResponse {
    let mut response = ErrorResponse::new(None);
    *response.status_mut() = StatusCode::FORBIDDEN;
    response
}

/// 업그레이드 요청 머리를 수락 정책에 비춘다 — 통과면 받을 Origin(없으면 `None`)을 돌려준다. Origin 을 먼저 본다.
fn screen(
    policy: &AcceptPolicy,
    headers: &HeaderMap,
    local_port: u16,
) -> Result<Option<String>, Screened> {
    let origin = single(headers, header::ORIGIN);
    let host = single(headers, header::HOST);
    let refused = |kind| Screened {
        kind,
        origin: origin.raw(),
        host: host.raw(),
    };
    let origin = match &origin {
        OneHeader::Absent => None,
        OneHeader::One(value) if policy.allowed_origins.iter().any(|a| a == value) => {
            Some(value.clone())
        }
        _ => return Err(refused(RefusalKind::OriginRejected)),
    };
    match &host {
        OneHeader::Absent => {}
        OneHeader::One(value) if host_matches(value, &policy.allowed_host_names, local_port) => {}
        _ => return Err(refused(RefusalKind::HostRejected)),
    }
    Ok(origin)
}

/// 한 이름의 머리를 읽은 모양.
enum OneHeader {
    Absent,
    One(String),
    /// 둘 이상이거나 글자가 아닌 바이트 — 판정하지 않고 끊는다. 실은 값은 보고용(손실 변환)이다.
    Malformed(String),
}

impl OneHeader {
    fn raw(&self) -> Option<String> {
        match self {
            OneHeader::Absent => None,
            OneHeader::One(value) | OneHeader::Malformed(value) => Some(value.clone()),
        }
    }
}

fn single(headers: &HeaderMap, name: header::HeaderName) -> OneHeader {
    let mut values = headers.get_all(name).iter();
    match (values.next(), values.next()) {
        (None, _) => OneHeader::Absent,
        (Some(value), None) => match value.to_str() {
            Ok(text) => OneHeader::One(text.to_owned()),
            Err(_) => OneHeader::Malformed(String::from_utf8_lossy(value.as_bytes()).into_owned()),
        },
        (Some(first), Some(_)) => {
            OneHeader::Malformed(String::from_utf8_lossy(first.as_bytes()).into_owned())
        }
    }
}

/// Host 머리 값 `이름[:포트]` 이 받는 이름 · 이 청취기 포트와 맞나. `[::1]:포트` 꼴은 대괄호 뒤 콜론만 포트다.
/// 포트는 숫자만 받는다(부호 · 빈 칸 거절) · 없으면 80.
fn host_matches(value: &str, names: &[String], local_port: u16) -> bool {
    let (name, port) = match value.rsplit_once(':') {
        Some((name, port)) if !port.contains(']') => (name, Some(port)),
        _ => (value, None),
    };
    let port_ok = match port {
        Some(text) => {
            !text.is_empty()
                && text.bytes().all(|b| b.is_ascii_digit())
                && text.parse::<u16>() == Ok(local_port)
        }
        None => local_port == 80,
    };
    port_ok
        && names
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(name))
}

/// 열린 스트림을 통로 양 끝으로 나눈다. `max_payload` = 나가는 프레임 상한(파일 헤더 「통로 성질」).
///
/// ★crate 안 전용이다★(TRD 3 §3-5 · §5 17) — 밖에 열면 누구나 자기 수락으로 통로를 지어 [`WsListener`] 의 검사를
/// 전부 건너뛴다.
///
/// ★두 끝은 `BiLock` 으로 같은 스트림을 공유한다★ — 그래서 서로 다른 태스크가 쥐어도 되고(감독은 읽는
/// 쪽을 전용 태스크에 넘긴다), 한쪽이 `Pending` 을 낼 때 잠금을 놓아 다른 쪽을 막지 않는다.
/// ★양 끝이 **둘 다** 떨어져야 소켓이 닫힌다★ — 여기서는 그 `BiLock` 이 마지막 한 끝이 떨어질 때만
/// 안쪽 스트림을 놓기 때문이다. 그것을 지킬 의무는 부르는 쪽에 있고 seam 이 그 의무를 진다
/// ([`LinkTx`] rustdoc).
pub(crate) fn split_link<S: WsStream>(
    ws: WebSocketStream<S>,
    max_payload: usize,
) -> (Box<dyn LinkTx>, Box<dyn LinkRx>) {
    let (sink, stream) = ws.split();
    (
        Box::new(WsTx { sink, max_payload }),
        Box::new(WsRx {
            stream,
            failed: false,
        }),
    )
}

// ── 통로 두 끝 ───────────────────────────────────────────────────────────────

struct WsTx<S: WsStream> {
    sink: SplitSink<WebSocketStream<S>, Message>,
    /// 나가는 프레임 payload 의 상한 — 이 통로를 연 쪽이 받은 크기 상한과 같은 값이다([`split_link`]).
    max_payload: usize,
}

impl<S: WsStream> LinkTx for WsTx<S> {
    fn send(&mut self, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(async move {
            // ★상한을 넘는 프레임은 **값으로** 실패한다★ — 안 그러면 상한을 집행하는 것이 받는 쪽뿐이라
            //   (파일 헤더 「tungstenite 는 들어오는 것의 크기만 재고 나가는 것은 안 잰다」) 통로가 끊기고
            //   **아무 잘못 없는 상대가 자기 재연결 예산을 태운다**. ★패닉으로 돌아가지 말 것★ —
            //   `debug_assert` 였던 판은 감독 태스크 안에서 터지는데 그 `JoinHandle` 을 아무도 안 쥐고
            //   있어(`Unstarted::start`) 패닉을 **관측하는 자리가 없었다**: 상태 watch 의 보내는 쪽이 떨어져
            //   발행된 상태가 `Live` 에 영구히 얼고, 끊김 사건도 포기 사건도 안 나고, 읽는 태스크는
            //   통로 절반을 쥔 채 떨어져 나갔다 — 패닉 훅을 건 운영 프로세스라면 그 대신 프로세스째 끝난다
            //   (ADR-0292). 값으로 실패하면 감독이 그것을 `LinkLost` 로 옮겨
            //   (`Supervisor::write`) 끊김 사건과 재연결이 남는다. ★단 **문구는 안 남는다**★ — 그 옮김이
            //   `LinkError` 문구를 버리고 `Disconnected` 에는 문구 칸이 없다(이 파일 헤더의 그 항목).
            let len = payload_len(&frame);
            if len > self.max_payload {
                return Err(LinkError::new(format!(
                    "ws write: 나가는 프레임이 {len} 바이트다 — 상한 {} 을 넘으면 받는 쪽이 통로를 끊는다",
                    self.max_payload
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
            // 문구를 [`close_reason_room`] 으로 자른다 — 그 함수가 사유를 든다.
            let message = match close_reason_room(self.max_payload) {
                Some(room) => Message::Close(Some(CloseFrame {
                    code: WsCloseCode::from(close.code.0),
                    reason: clamp_close_reason(&close.reason, room).into(),
                })),
                // ★코드도 못 싣는 상한이면 빈 닫기를 보낸다★ — 상대는 코드 없는 닫기로 읽는다. 코드를 실으면 상한을
                //   넘겨 상대 읽기가 끊긴다.
                None => Message::Close(None),
            };
            // 실패해도 알릴 데가 없다([`LinkTx::close`] 계약). 닫기 프레임이 안 나가면 상대는 코드를
            //   못 보고 말 없이 끊긴 것으로 읽는다 — `frame.rs` 의 그 한계 그대로다.
            let _ = self.sink.send(message).await;
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
        assert_eq!(clamp_close_reason("nope", MAX_CLOSE_REASON_BYTES), "nope");
        let exact = "x".repeat(MAX_CLOSE_REASON_BYTES);
        assert_eq!(clamp_close_reason(&exact, MAX_CLOSE_REASON_BYTES), exact);
    }

    #[test]
    fn a_long_reason_is_cut_on_a_character_boundary() {
        // 한 글자가 3 바이트라 41 글자 = 123 바이트고, 42 글자째가 상한을 넘긴다.
        let long = "가".repeat(60);
        let cut = clamp_close_reason(&long, MAX_CLOSE_REASON_BYTES);
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
        let cut = clamp_close_reason(&emoji, MAX_CLOSE_REASON_BYTES);
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
        let (mut tx, _rx) = split_link(ws, usize::MAX);
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
                reason: clamp_close_reason(reason, close_reason_room(usize::MAX).unwrap()).into(),
            }));
            assert!(frame.payload().len() <= 125, "{}", frame.payload().len());
        }
    }

    // ★125 밑의 크기 상한도 닫기 프레임에 걸린다★ — 받는 쪽 `max_frame_size` 가 제어 프레임에도 걸리므로
    //   ([`close_reason_room`]) 123 으로만 자르면 이 상한들에서 상대 읽기가 끊긴다.
    #[tokio::test]
    async fn the_close_frame_stays_within_a_small_configured_limit() {
        // (크기 상한, 나가야 할 payload) — 2 밑이면 코드도 못 실어 빈 닫기다.
        for (limit, payload) in [(10usize, 10usize), (2, 2), (1, 0), (0, 0)] {
            let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let ws = WebSocketStream::from_raw_socket(
                Recorder(written.clone()),
                tokio_tungstenite::tungstenite::protocol::Role::Server,
                None,
            )
            .await;
            let (mut tx, _rx) = split_link(ws, limit);
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                tx.close(Close::new(CloseCode(4001), "a".repeat(200))),
            )
            .await
            .expect("닫기가 안 끝났다");

            let bytes = written.lock().unwrap().clone();
            assert_eq!(bytes[0], 0x88, "상한 {limit}: 닫기 프레임이 아니다");
            assert_eq!(usize::from(bytes[1]), payload, "상한 {limit}: payload 길이");
            if payload >= 2 {
                assert_eq!(
                    u16::from_be_bytes([bytes[2], bytes[3]]),
                    4001,
                    "상한 {limit}: 코드"
                );
            }
        }
    }

    // ── 나가는 크기 상한 ──
    //
    // ★상한이 [`split_link`] 에 준 값이라는 것을 잰다★ — 고정 상수로 되돌리면 소비자가 준 값이 받는 쪽에만 걸리고
    //   나가는 쪽은 옛 값으로 남아, 상대를 끊게 만드는 프레임을 그대로 내보낸다.
    #[tokio::test]
    async fn a_frame_over_the_given_limit_fails_as_a_value_and_one_at_it_goes_out() {
        let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let ws = WebSocketStream::from_raw_socket(
            Recorder(written.clone()),
            tokio_tungstenite::tungstenite::protocol::Role::Server,
            None,
        )
        .await;
        let (mut tx, _rx) = split_link(ws, 8);
        let over = tx.send(Frame::Binary(vec![0; 9])).await;
        assert!(over.is_err(), "상한 + 1 이 나갔다");
        assert!(
            written.lock().unwrap().is_empty(),
            "거절한 프레임이 소켓에 닿았다"
        );
        tx.send(Frame::Binary(vec![0; 8]))
            .await
            .expect("상한 그대로는 나가야 한다");
        assert!(!written.lock().unwrap().is_empty());
    }

    #[test]
    fn the_receive_limit_covers_a_message_and_a_frame_alike() {
        // ★둘 다 덮어야 한다★ — 프레임만 덮으면 조각난 메시지가 tungstenite 기본 64 MiB 까지 자란다.
        let config = ws_config(1234);
        assert_eq!(config.max_message_size, Some(1234));
        assert_eq!(config.max_frame_size, Some(1234));
    }

    // ── loopback 단언 ──
    #[test]
    fn only_a_loopback_address_may_be_listened_on() {
        for ok in ["127.0.0.1:0", "127.9.9.9:4000", "[::1]:0"] {
            assert!(ensure_loopback(ok.parse().unwrap()).is_ok(), "{ok}");
        }
        // ★`::ffff:127.0.0.1` 도 거절한다★ — 표준 판정(`Ipv6Addr::is_loopback`)이 `::1` 만 loopback 으로 본다.
        for refused in [
            "0.0.0.0:0",
            "192.168.0.10:0",
            "[::]:0",
            "[::ffff:127.0.0.1]:0",
        ] {
            let err = ensure_loopback(refused.parse().unwrap()).expect_err(refused);
            assert!(err.message.contains("loopback"), "{err}");
        }
    }

    // ── 받는 Host 이름 ──
    #[test]
    fn a_host_name_with_a_colon_outside_brackets_is_refused_at_bind() {
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(ensure_host_names(&names(&["127.0.0.1", "localhost", "[::1]"])).is_ok());
        assert!(ensure_host_names(&[]).is_ok());
        for bad in ["::1", "fe80::1", "localhost:8080", "[::1]:8080"] {
            let err = ensure_host_names(&names(&["localhost", bad])).expect_err(bad);
            assert!(err.message.contains(bad), "{err}");
        }
        // ★대괄호째 적은 이름이 실제 머리와 맞는다★ — 위 판정이 받는 꼴이 머리 판정이 받는 꼴과 같아야 한다.
        assert!(host_matches("[::1]:4000", &names(&["[::1]"]), 4000));
        assert!(!host_matches("[::1]:4000", &names(&["::1"]), 4000));
    }

    // ── 수락 오류 뒤 쉼 ──
    #[test]
    fn only_accept_errors_not_tied_to_one_connection_pause_the_loop() {
        for kind in [
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::ConnectionReset,
        ] {
            assert!(!pauses_after_accept_error(kind), "{kind:?} 에 쉬었다");
        }
        // 파일 핸들 바닥(`EMFILE`)은 이름 붙은 갈래가 없어 플랫폼마다 다른 갈래로 온다 — 이름 붙은 나머지로 잰다.
        for kind in [
            io::ErrorKind::Other,
            io::ErrorKind::OutOfMemory,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::InvalidInput,
        ] {
            assert!(pauses_after_accept_error(kind), "{kind:?} 에 안 쉬었다");
        }
    }

    // ── 업그레이드 시한 (§5 12) ──
    //
    // ★시한의 기준은 TCP 수락 시각이다★ — 띄운 태스크가 늦게 돌아도 · 끝남과 시한이 한 폴링에 함께 서도 시한 시각부터는
    //   받지 않는다. 손으로 미는 시계와 메모리 스트림으로 소켓 · 실시간 없이, 폴링 시점을 시험이 골라 잰다.
    const UPGRADE_TIMEOUT: Duration = Duration::from_secs(10);

    /// Host · Origin 없는 업그레이드 요청 — 머리 판정을 통과한다.
    const UPGRADE_REQUEST: &[u8] =
        b"GET / HTTP/1.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
        Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n";

    struct Rig {
        clock: crate::testing::ManualClock,
        permits: PermitPool,
        shared: Arc<Shared>,
        ready_tx: mpsc::Sender<Accepted>,
        ready: mpsc::Receiver<Accepted>,
        refusals: AcceptRefusals,
    }

    fn rig() -> Rig {
        let clock = crate::testing::ManualClock::new();
        let permits = PermitPool::new(1);
        let (ready_tx, ready) = mpsc::channel(1);
        let (tx, rx) = mpsc::channel(4);
        let dropped = Arc::new(AtomicU64::new(0));
        let shared = Arc::new(Shared {
            local: "127.0.0.1:4000".parse().unwrap(),
            policy: AcceptPolicy {
                upgrade_timeout: UPGRADE_TIMEOUT,
                max_message_bytes: 1024,
                ..policy(&[], &[])
            },
            permits: permits.clone(),
            clock: clock.handle(),
            reporter: Reporter {
                tx,
                dropped: dropped.clone(),
            },
        });
        Rig {
            clock,
            permits,
            shared,
            ready_tx,
            ready,
            refusals: AcceptRefusals { rx, dropped },
        }
    }

    impl Rig {
        /// 칸 하나를 쥐고 지금을 TCP 수락 시각으로 한 업그레이드 — 띄우지 않는다(시험이 폴링 시점을 고른다).
        fn upgrade<S: WsStream>(&self, stream: S) -> impl std::future::Future<Output = ()> {
            upgrade(
                stream,
                "127.0.0.1:5555".parse().unwrap(),
                self.clock.now(),
                self.permits.try_take().expect("칸"),
                self.shared.clone(),
                self.ready_tx.clone(),
            )
        }

        fn assert_timed_out(&mut self, what: &str) {
            assert!(
                self.ready.try_recv().is_err(),
                "{what}: 시한을 넘긴 업그레이드가 준비 큐에 앉았다"
            );
            let report = self
                .refusals
                .try_recv()
                .unwrap_or_else(|| panic!("{what}: 보고가 없다"));
            assert_eq!(report.kind, RefusalKind::UpgradeTimeout, "{what}");
            assert_eq!(self.permits.in_use(), 0, "{what}: 칸이 안 돌아갔다");
        }
    }

    /// 한 번만 폴링한다 — 끝났으면 `true`.
    async fn poll_once<F: std::future::Future<Output = ()>>(
        task: &mut std::pin::Pin<Box<F>>,
    ) -> bool {
        std::future::poll_fn(|cx| {
            std::task::Poll::Ready(std::future::Future::poll(task.as_mut(), cx).is_ready())
        })
        .await
    }

    #[tokio::test]
    async fn an_upgrade_first_polled_at_or_after_the_cutoff_is_refused_before_its_handshake() {
        for late in [UPGRADE_TIMEOUT, UPGRADE_TIMEOUT + Duration::from_millis(1)] {
            let mut rig = rig();
            let (mut client, server) = tokio::io::duplex(64 * 1024);
            // 요청은 이미 닿아 있다 — 시한만 아니면 첫 폴링에 업그레이드가 끝난다.
            tokio::io::AsyncWriteExt::write_all(&mut client, UPGRADE_REQUEST)
                .await
                .unwrap();
            let task = rig.upgrade(server);
            // 띄운 태스크가 시한까지 한 번도 못 돌았다.
            rig.clock.advance(late);
            task.await;
            rig.assert_timed_out(&format!("수락 뒤 {late:?} 에 처음 돎"));
            let mut answer = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut client, &mut answer)
                .await
                .unwrap();
            assert!(answer.is_empty(), "시한을 넘긴 뒤 응답을 썼다");
        }
    }

    #[tokio::test]
    async fn an_upgrade_that_finishes_at_or_after_the_cutoff_is_refused() {
        for late in [UPGRADE_TIMEOUT, UPGRADE_TIMEOUT + Duration::from_millis(1)] {
            let mut rig = rig();
            let (mut client, server) = tokio::io::duplex(64 * 1024);
            let mut task = Box::pin(rig.upgrade(server));
            assert!(!poll_once(&mut task).await, "요청 없이 끝났다");
            // 시한과 요청이 함께 선다 — 다음 폴링에서 업그레이드가 시한보다 먼저 폴링돼 끝난다.
            rig.clock.advance(late);
            tokio::io::AsyncWriteExt::write_all(&mut client, UPGRADE_REQUEST)
                .await
                .unwrap();
            task.await;
            rig.assert_timed_out(&format!("수락 뒤 {late:?} 에 끝남"));
        }
    }

    // 위 시험의 짝 — 같은 순서에서 시한 직전에 끝나면 받고, 인증 시한의 기준은 수락 시각이 아니라 끝난 시각이다.
    #[tokio::test]
    async fn an_upgrade_that_finishes_just_before_the_cutoff_is_accepted_stamped_at_completion() {
        let mut rig = rig();
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        let accepted_at = rig.clock.now();
        let mut task = Box::pin(rig.upgrade(server));
        assert!(!poll_once(&mut task).await, "요청 없이 끝났다");
        let early = UPGRADE_TIMEOUT - Duration::from_millis(1);
        rig.clock.advance(early);
        tokio::io::AsyncWriteExt::write_all(&mut client, UPGRADE_REQUEST)
            .await
            .unwrap();
        task.await;
        let accepted = rig.ready.try_recv().expect("시한 전에 끝난 업그레이드");
        assert_eq!(accepted.upgraded_at(), accepted_at + early);
        assert!(rig.refusals.try_recv().is_none());
        assert_eq!(rig.permits.in_use(), 1, "받은 통로가 칸을 쥔다");
    }

    /// 읽기는 준 바이트를 내고 쓰기는 영영 안 받는 스트림 — 응답을 쓰다 멎은 업그레이드를 만든다.
    struct WriteStalled(std::io::Cursor<Vec<u8>>);

    impl tokio::io::AsyncRead for WriteStalled {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.0).poll_read(cx, buf)
        }
    }

    impl tokio::io::AsyncWrite for WriteStalled {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::task::Poll::Pending
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Pending
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Pending
        }
    }

    #[tokio::test]
    async fn a_403_cut_by_the_deadline_is_reported_as_its_screening_with_the_headers() {
        let mut rig = rig();
        let request = b"GET / HTTP/1.1\r\nOrigin: http://evil\r\nConnection: Upgrade\r\n\
            Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
            Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n";
        let mut task = Box::pin(rig.upgrade(WriteStalled(std::io::Cursor::new(request.to_vec()))));
        assert!(!poll_once(&mut task).await, "403 을 다 썼다");
        rig.clock.advance(UPGRADE_TIMEOUT);
        task.await;
        let report = rig.refusals.try_recv().expect("보고");
        assert_eq!(report.kind, RefusalKind::OriginRejected);
        assert_eq!(report.origin.as_deref(), Some("http://evil"));
        assert!(rig.ready.try_recv().is_err());
        assert_eq!(rig.permits.in_use(), 0);
    }

    // ── 업그레이드 머리 판정 ──
    fn policy(origins: &[&str], hosts: &[&str]) -> AcceptPolicy {
        AcceptPolicy {
            allowed_origins: origins.iter().map(|s| s.to_string()).collect(),
            allowed_host_names: hosts.iter().map(|s| s.to_string()).collect(),
            upgrade_timeout: Duration::from_secs(1),
            max_pending_upgrades: 1,
            max_message_bytes: 1,
            refusal_queue: 1,
        }
    }

    fn headers(pairs: &[(&str, &[u8])]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                header::HeaderValue::from_bytes(value).unwrap(),
            );
        }
        map
    }

    fn kind(result: Result<Option<String>, Screened>) -> Option<RefusalKind> {
        result.err().map(|s| s.kind)
    }

    #[test]
    fn origin_absent_passes_listed_passes_and_anything_else_is_refused() {
        let p = policy(&["http://localhost:1420"], &["127.0.0.1"]);
        assert_eq!(screen(&p, &headers(&[]), 9000), Ok(None));
        assert_eq!(
            screen(&p, &headers(&[("origin", b"http://localhost:1420")]), 9000),
            Ok(Some("http://localhost:1420".into()))
        );
        for bad in [
            &b"http://evil.example"[..],
            b"http://LOCALHOST:1420",
            b"null",
            b"",
            b"http://localhost:1420/",
        ] {
            assert_eq!(
                kind(screen(&p, &headers(&[("origin", bad)]), 9000)),
                Some(RefusalKind::OriginRejected),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        // 둘 이상 · 글자 아님 — 목록에 든 값이어도 끊는다.
        let twice = headers(&[
            ("origin", b"http://localhost:1420"),
            ("origin", b"http://localhost:1420"),
        ]);
        assert_eq!(
            kind(screen(&p, &twice, 9000)),
            Some(RefusalKind::OriginRejected)
        );
        let bytes = headers(&[("origin", b"http://localhost:1420\xff")]);
        assert_eq!(
            kind(screen(&p, &bytes, 9000)),
            Some(RefusalKind::OriginRejected)
        );
    }

    #[test]
    fn an_empty_origin_list_refuses_every_upgrade_that_carries_one() {
        // D21 ① 의 릴리스 값 — Origin 이 없으면 그대로 받는다.
        let p = policy(&[], &["127.0.0.1"]);
        assert_eq!(screen(&p, &headers(&[]), 9000), Ok(None));
        assert_eq!(
            kind(screen(
                &p,
                &headers(&[("origin", b"tauri://localhost")]),
                9000
            )),
            Some(RefusalKind::OriginRejected)
        );
    }

    #[test]
    fn host_must_name_an_allowed_host_on_the_bound_port_or_be_absent() {
        let p = policy(&[], &["127.0.0.1", "localhost", "[::1]"]);
        for ok in [
            &b"127.0.0.1:9000"[..],
            b"localhost:9000",
            b"LocalHost:9000",
            b"[::1]:9000",
        ] {
            assert_eq!(
                screen(&p, &headers(&[("host", ok)]), 9000),
                Ok(None),
                "{}",
                String::from_utf8_lossy(ok)
            );
        }
        assert_eq!(
            screen(&p, &headers(&[]), 9000),
            Ok(None),
            "Host 없음은 받는다(D9)"
        );
        for bad in [
            &b"evil.example:9000"[..],
            b"127.0.0.1:9001",
            b"127.0.0.1",
            b"127.0.0.1:",
            b"127.0.0.1:+9000",
            b"[::1]",
            b"127.0.0.1.evil.example:9000",
        ] {
            assert_eq!(
                kind(screen(&p, &headers(&[("host", bad)]), 9000)),
                Some(RefusalKind::HostRejected),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        let twice = headers(&[("host", b"127.0.0.1:9000"), ("host", b"127.0.0.1:9000")]);
        assert_eq!(
            kind(screen(&p, &twice, 9000)),
            Some(RefusalKind::HostRejected)
        );
        // 포트가 빠지면 80 이다 — 80 에 연 청취기면 받는다.
        assert_eq!(
            screen(&p, &headers(&[("host", b"localhost")]), 80),
            Ok(None)
        );
    }

    #[test]
    fn a_refusal_carries_both_headers_it_saw() {
        let p = policy(&["http://ok"], &["127.0.0.1"]);
        let seen = screen(
            &p,
            &headers(&[("origin", b"http://ok"), ("host", b"evil:9000")]),
            9000,
        )
        .expect_err("Host 가 밖이다");
        assert_eq!(
            seen,
            Screened {
                kind: RefusalKind::HostRejected,
                origin: Some("http://ok".into()),
                host: Some("evil:9000".into()),
            }
        );
    }

    // ── 거절 보고 ──
    #[tokio::test]
    async fn a_full_refusal_queue_drops_new_reports_and_counts_them() {
        let (tx, rx) = mpsc::channel(1);
        let dropped = Arc::new(AtomicU64::new(0));
        let reporter = Reporter {
            tx,
            dropped: dropped.clone(),
        };
        let mut refusals = AcceptRefusals { rx, dropped };
        let remote: SocketAddr = "127.0.0.1:5555".parse().unwrap();
        // 상대가 지은 글은 가린 채 싣는다 — 줄바꿈 · 보이지 않는 글자가 빠진다.
        reporter.report(
            RefusalKind::OriginRejected,
            Some(remote),
            Some("http://evil\n\u{202E}x"),
            None,
        );
        reporter.report(RefusalKind::SlotsFull, Some(remote), None, None);
        reporter.report(RefusalKind::SlotsFull, Some(remote), None, None);
        assert_eq!(refusals.dropped(), 2, "찬 줄에서 버린 수");
        let first = refusals.try_recv().expect("첫 보고");
        assert_eq!(
            first.kind,
            RefusalKind::OriginRejected,
            "먼저 온 것이 남는다"
        );
        assert_eq!(first.remote.as_deref(), Some("127.0.0.1:5555"));
        assert_eq!(first.origin.as_deref(), Some("http://evilx"));
        assert!(refusals.try_recv().is_none());
    }
}
