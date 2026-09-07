//! 실소켓 ① — WS 어댑터가 실제로 붙나(TRD §9-1).
//!
//! ★두 끝이 다 이 crate 것이다★ — 받는 쪽 `ws::WsListener` · 거는 쪽 `ws::WsDialer`. 그래서 이 파일은
//! 다른 crate 도 외부 프로세스도 부르지 않는다.
//!
//! ★`test-support` 까지 요구하는 것은 의도★ — 패킷 어휘(`TestWire`)와 두 걸음 핸드셰이크가 그 뒤에
//! 산다. `--all-features` 가 둘을 함께 켜므로 게이트 ④ 가 이 파일을 실제로 돈다.
#![cfg(all(feature = "ws", feature = "test-support"))]

mod common;

use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::{TcpSocket, TcpStream};

use engram_dashboard_transport::link::LinkRead;
use engram_dashboard_transport::testing::{HelloHandshake, TestIn, TestOut, TestWire};
use engram_dashboard_transport::ws::{WsDialer, WsListener};
use engram_dashboard_transport::{
    Address, ConnectFailure, Frame, Generation, PeerId, Policy, TransportEvent,
};

#[tokio::test]
async fn a_real_socket_peer_shakes_hands_in_two_steps_and_round_trips_a_request() {
    let listener = WsListener::bind("127.0.0.1:0").await.expect("bind");
    // ★포트를 박지 않는다★ — 0번을 요청하고 커널이 고른 번호를 되읽는다.
    assert_ne!(listener.local_addr().expect("local_addr").port(), 0);
    let addr = listener.dial_address().expect("dial_address");

    // 받는 쪽: hello 를 받고 welcome 을 돌려준 뒤, 요청 하나에 답장한다.
    let server = tokio::spawn(async move {
        let (mut tx, mut rx) = listener.accept().await.expect("accept");
        let hello = rx.recv().await.expect("hello 수신");
        tx.send(Frame::Text(HelloHandshake::WELCOME.into()))
            .await
            .expect("welcome 송신");
        let request = rx.recv().await.expect("요청 수신");
        tx.send(TestWire::reply(7)).await.expect("답장 송신");
        (hello, request)
    });

    let (registry, mut inbound) = common::registry(Arc::new(WsDialer), Policy::default());
    let peer = registry.add(PeerId::new("d1"), addr, Arc::new(HelloHandshake));

    common::wait_state(&peer, "운영 단계", |state| state.is_live()).await;
    let generation = common::wait_incoming(&mut inbound, "Connected 사건", |msg| {
        match msg.as_event() {
            Some(TransportEvent::Connected { generation, .. }) => Some(*generation),
            _ => None,
        }
    })
    .await;
    assert_eq!(generation, Generation(1), "첫 연결의 세대");

    let reply = tokio::time::timeout(
        common::PATIENCE,
        peer.request(TestOut::Request {
            tag: 7,
            body: "ping".into(),
        }),
    )
    .await
    .expect("답장 상한")
    .expect("답장");
    assert_eq!(
        reply,
        TestIn::Reply {
            tag: 7,
            body: "ok".into()
        }
    );

    let (hello, request) = tokio::time::timeout(common::PATIENCE, server)
        .await
        .expect("받는 쪽 상한")
        .expect("받는 쪽 태스크");
    match hello {
        LinkRead::Frame(Frame::Text(text)) => assert_eq!(text, HelloHandshake::HELLO),
        other => panic!("핸드셰이크 첫 걸음이 소켓을 건너오지 않았다: {other:?}"),
    }
    match request {
        LinkRead::Frame(Frame::Text(text)) => assert_eq!(text, "req|7|ping"),
        other => panic!("요청이 소켓을 건너오지 않았다: {other:?}"),
    }
}

/// 아무도 안 듣는 포트를 **잡아 둔 채로** 그 주소를 준다. ★돌려준 소켓을 부르는 쪽이 테스트가 끝날
/// 때까지 쥔다★ — 놓는 순간 그 번호가 풀려 아래 성질이 함께 사라진다.
///
/// ★`listen()` 을 부르지 않는 것이 이 발판의 전부다★(외부 사실) — 듣는 소켓이 없으니 그 포트에 오는
/// 연결은 성립조차 하지 않고 거절되고([`refusal_of`] 가 그것을 매번 되잰다), bind 는 살아 있으니 그
/// 번호를 남이 가져가지 못한다(실측 2026-09-07, Windows 11: 평범한 bind 는 `AddrInUse`(10048) 로,
/// `SO_REUSEADDR` 를 켠 강제 bind 마저 `PermissionDenied`(10013) 로 거부됐다).
///
/// ★「열고 바로 닫는다」로 되돌리지 말 것★ — 닫은 뒤 남이 그 번호를 가져가 TCP 를 수락하고 업그레이드
/// 중에 끊으면, 그 실패도 우리가 건 주소를 담은 `Unreachable` 로 올라오고 문구에는 **`os error 10054`**
/// 가 붙는다(실측 2026-09-07, Windows 11: 상대가 FIN 으로 놓아도 RST 로 끊어도 같은 번호다). 즉 `os
/// error` 만 보는 단언은 그 갈래를 전부 초록으로 통과시킨다 — [`refusal_of`] 가 번호까지 못박는 이유다.
fn unlistened_port() -> (TcpSocket, SocketAddr) {
    let socket = TcpSocket::new_v4().expect("v4 소켓");
    socket
        .bind("127.0.0.1:0".parse().expect("주소 문법"))
        .expect("bind");
    let addr = socket.local_addr().expect("local_addr");
    assert_ne!(addr.port(), 0);
    (socket, addr)
}

/// 그 주소가 **지금** 내는 거절을 `os error N` 꼴로 잰다. 재는 일 자체가 「아무도 안 듣는다」의 확인이다.
///
/// ★번호를 소스에 박지 않고 매번 재는 이유★ — 값이 플랫폼마다 갈리고(Windows `WSAECONNREFUSED` 10061 ·
/// Linux `ECONNREFUSED` 111 · macOS 61) 박아 두면 그 목록을 손으로 관리해야 한다.
///
/// ★문구 본문은 안 본다★(외부 사실) — `io::Error` 의 `Display` 는 OS 문구 뒤에 `(os error N)` 을 붙이는데
/// 그 본문은 OS 로케일에 따라 번역된다(실측: Windows 한국어 판이 한글 문구로 돌려준다). 번호는 번역되지
/// 않으므로 판정을 그쪽에만 건다.
async fn refusal_of(addr: SocketAddr) -> String {
    let refused = TcpStream::connect(addr)
        .await
        .err()
        .expect("아무도 안 듣는 포트에 TCP 가 붙었다 — 남이 이 번호를 강제로 가져갔다는 뜻이다");
    assert_eq!(
        refused.kind(),
        ErrorKind::ConnectionRefused,
        "이 포트가 거절이 아닌 방식으로 실패한다 — 방화벽이 loopback 을 먹는 것이 첫 가설이다: {refused}"
    );
    format!("os error {}", refused.raw_os_error().expect("OS 오류 번호"))
}

#[tokio::test]
async fn dialing_a_port_that_nobody_listens_on_is_unreachable_not_rejected() {
    // ★「못 붙었다」와 「붙었는데 거절」이 갈리는 자리다★ — 갈리지 않으면 거절이 재연결 예산을 안
    //   쓴다는 계약(ADR-0180 결정 5)을 실소켓에서 확인할 수 없다. 거절 쪽은 `ws_reject.rs` 가 잰다.
    let (_held, closed_at) = unlistened_port();
    let closed = Address::new(format!("ws://{closed_at}/"));

    let (registry, mut inbound) = common::registry(Arc::new(WsDialer), Policy::default());
    let peer = registry.add(PeerId::new("d1"), closed.clone(), Arc::new(HelloHandshake));

    // ★기대값을 그 포트에서 직접 뽑는다★ — 갈래 이름만으로는 「통로가 수락된 뒤 끊겼다」·「주소 문법이
    //   틀렸다」·「핸드셰이크가 시한에 걸렸다」가 전부 같은 `Unreachable` 이다. 이 테스트가 만들려는
    //   자극은 **연결이 성립조차 안 한 것** 하나이고, OS 번호가 그 자극의 유일한 물증이다.
    // ★감독의 dial 과 **나란히** 잰다★ — 거절 한 번이 loopback 에서 2.0초다(실측 2026-09-07, Windows 11).
    //   순서대로 재면 그 값이 이 테스트에 두 번 더해지고, 나란히 재면 두 기다림이 겹쳐 한 번에 든다.
    let (refusal, cause) = tokio::join!(
        refusal_of(closed_at),
        common::wait_incoming(
            &mut inbound,
            "ConnectFailed 사건(첫 가설 = 온 거절을 감독이 사건으로 올리지 않는다)",
            |msg| match msg.as_event() {
                Some(TransportEvent::ConnectFailed { cause, .. }) => Some(cause.clone()),
                _ => None,
            },
        )
    );
    let ConnectFailure::Unreachable(err) = &cause else {
        panic!("안 열린 포트는 못 붙은 것이어야 한다: {cause:?}");
    };
    assert!(
        err.message.contains(closed.as_str()),
        "우리가 건 주소가 아닌 다른 실패가 올라왔다 ({} 를 기대): {err}",
        closed.as_str()
    );
    assert!(
        err.message.contains(&refusal),
        "이 포트가 방금 낸 거절({refusal})이 아니라 다른 실패가 이 이름을 입었다 — 통로가 수락된 뒤 \
         끊긴 것(그쪽은 다른 번호다)이 첫 가설이다: {err}"
    );
    assert!(!peer.peer_state().is_live());
}
