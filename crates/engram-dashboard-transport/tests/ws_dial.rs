//! Real socket ① — does the WS adapter really connect (TRD §9-1).
//!
//! ★Both ends are this crate's★ — the receiving side `ws::WsListener` · the dialing side `ws::WsDialer`. So this file
//! calls neither another crate nor an external process.
//!
//! ★Requiring even `test-support` is intentional★ — the packet vocabulary (`TestWire`) and the two-step handshake live behind it.
//! `--all-features` turns both on together, so gate ④ really runs this file.
#![cfg(all(feature = "ws", feature = "test-support"))]

mod common;

use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::{TcpSocket, TcpStream};

use engram_dashboard_transport::link::LinkRead;
use engram_dashboard_transport::testing::{HelloHandshake, TestIn, TestOut, TestWire};
use engram_dashboard_transport::{
    Address, ConnectFailure, Frame, Generation, PeerId, Policy, TransportEvent,
};

#[tokio::test]
async fn a_real_socket_peer_shakes_hands_in_two_steps_and_round_trips_a_request() {
    let mut listener = common::listen();
    // ★Do not hard-code a port★ — request number 0 and read back the number the kernel picked.
    assert_ne!(listener.local_addr().port(), 0);
    let addr = listener.dial_address();

    // The receiving side: receives hello, returns welcome, then answers one request.
    let server = tokio::spawn(async move {
        let (mut tx, mut rx) = common::accept_link(&mut listener).await.expect("accept");
        let hello = rx.recv().await.expect("hello 수신");
        tx.send(Frame::Text(HelloHandshake::WELCOME.into()))
            .await
            .expect("welcome 송신");
        let request = rx.recv().await.expect("요청 수신");
        tx.send(TestWire::reply(7)).await.expect("답장 송신");
        (hello, request)
    });

    let registry = common::registry(Arc::new(common::dialer()), Policy::default());
    let (peer, mut inbox) = registry.add(PeerId::new("d1"), addr, Arc::new(HelloHandshake));

    common::wait_state(&peer, "운영 단계", |state| state.is_live()).await;
    let generation =
        common::wait_incoming(&mut inbox, "Connected 사건", |msg| match msg.as_event() {
            Some(TransportEvent::Connected { generation, .. }) => Some(*generation),
            _ => None,
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

/// Gives the address of a port nobody listens on **while holding it**. ★The caller holds the returned socket until the test
/// ends★ — the moment it is released that number is freed and the properties below vanish with it.
///
/// ★Not calling `listen()` is the whole of this scaffolding★ (external fact) — with no listening socket, a
/// connection arriving at that port is refused without even being established ([`refusal_of`] re-measures that every time), and since the bind is alive nobody else can take that
/// number (measured 2026-09-07, Windows 11: an ordinary bind was refused with `AddrInUse` (10048),
/// and even a forced bind with `SO_REUSEADDR` on was refused with `PermissionDenied` (10013)).
///
/// ★Do not go back to 「열고 바로 닫는다」 (open and close right away)★ — after closing, if someone else takes that number, accepts TCP, and cuts during the upgrade,
/// that failure too comes up as an `Unreachable` carrying the address we dialed and the message gets **`os error 10054`**
/// attached (measured 2026-09-07, Windows 11: the same number whether the peer lets go with FIN or cuts with RST). That is, an assertion that looks only at `os
/// error` passes that branch entirely green — which is why [`refusal_of`] pins down the number too.
fn unlistened_port() -> (TcpSocket, SocketAddr) {
    let socket = TcpSocket::new_v4().expect("v4 소켓");
    socket
        .bind("127.0.0.1:0".parse().expect("주소 문법"))
        .expect("bind");
    let addr = socket.local_addr().expect("local_addr");
    assert_ne!(addr.port(), 0);
    (socket, addr)
}

/// Measures the refusal that address gives **right now** in the form `os error N`. The measuring itself is the confirmation of 「아무도 안 듣는다」 (nobody is listening).
///
/// ★Why the number is measured every time rather than hard-coded in the source★ — the value differs per platform (Windows `WSAECONNREFUSED` 10061 ·
/// Linux `ECONNREFUSED` 111 · macOS 61) and hard-coding would mean managing that list by hand.
///
/// ★It does not look at the message body★ (external fact) — the `Display` of `io::Error` appends `(os error N)` after the OS text, and
/// that body is translated by the OS locale (measured: the Korean edition of Windows returns Korean text). The number is not
/// translated, so the decision hangs only on it.
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
    // ★This is where 「못 붙었다」 (could not connect) and 「붙었는데 거절」 (connected but rejected) are told apart★ — if they are not told apart, the contract that a refusal does not
    //   spend the reconnect budget (ADR-0180 decision 5) cannot be confirmed on a real socket. The refusal side is measured by `ws_reject.rs`.
    let (_held, closed_at) = unlistened_port();
    let closed = Address::new(format!("ws://{closed_at}/"));

    let registry = common::registry(Arc::new(common::dialer()), Policy::default());
    let (peer, mut inbox) =
        registry.add(PeerId::new("d1"), closed.clone(), Arc::new(HelloHandshake));

    // ★Extract the expected value directly from that port★ — by branch name alone 「통로가 수락된 뒤 끊겼다」 (the link was cut after being accepted) · 「주소 문법이 틀렸다」 (the address
    //   syntax is wrong) · 「핸드셰이크가 시한에 걸렸다」 (the handshake hit the deadline) are all the same `Unreachable`. The one stimulus this test means to produce is **the connection
    //   not even being established**, and the OS number is the only evidence of that stimulus.
    // ★Measure it **side by side** with the supervisor's dial★ — one refusal takes 2.0 seconds on loopback (measured 2026-09-07, Windows 11).
    //   Measured in sequence that value would be added to this test twice, and side by side the two waits overlap and cost only one.
    let (refusal, cause) = tokio::join!(
        refusal_of(closed_at),
        common::wait_incoming(
            &mut inbox,
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
