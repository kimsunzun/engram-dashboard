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
        let hello = rx.recv().await.expect("hello received");
        tx.send(Frame::Text(HelloHandshake::WELCOME.into()))
            .await
            .expect("welcome sent");
        let request = rx.recv().await.expect("request received");
        tx.send(TestWire::reply(7)).await.expect("reply sent");
        (hello, request)
    });

    let registry = common::registry(Arc::new(common::dialer()), Policy::default());
    let (peer, mut inbox) = registry.add(PeerId::new("d1"), addr, Arc::new(HelloHandshake));

    common::wait_state(&peer, "operating phase", |state| state.is_live()).await;
    let generation =
        common::wait_incoming(&mut inbox, "Connected event", |msg| match msg.as_event() {
            Some(TransportEvent::Connected { generation, .. }) => Some(*generation),
            _ => None,
        })
        .await;
    assert_eq!(
        generation,
        Generation(1),
        "generation of the first connection"
    );

    let reply = tokio::time::timeout(
        common::PATIENCE,
        peer.request(TestOut::Request {
            tag: 7,
            body: "ping".into(),
        }),
    )
    .await
    .expect("reply cap")
    .expect("reply");
    assert_eq!(
        reply,
        TestIn::Reply {
            tag: 7,
            body: "ok".into()
        }
    );

    let (hello, request) = tokio::time::timeout(common::PATIENCE, server)
        .await
        .expect("receiving-side cap")
        .expect("receiving-side task");
    match hello {
        LinkRead::Frame(Frame::Text(text)) => assert_eq!(text, HelloHandshake::HELLO),
        other => panic!("the first step of the handshake did not cross the socket: {other:?}"),
    }
    match request {
        LinkRead::Frame(Frame::Text(text)) => assert_eq!(text, "req|7|ping"),
        other => panic!("the request did not cross the socket: {other:?}"),
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
/// ★Do not go back to 「open and close right away」★ — after closing, if someone else takes that number, accepts TCP, and cuts during the upgrade,
/// that failure too comes up as an `Unreachable` carrying the address we dialed and the message gets **`os error 10054`**
/// attached (measured 2026-09-07, Windows 11: the same number whether the peer lets go with FIN or cuts with RST). That is, an assertion that looks only at `os
/// error` passes that branch entirely green — which is why [`refusal_of`] pins down the number too.
fn unlistened_port() -> (TcpSocket, SocketAddr) {
    let socket = TcpSocket::new_v4().expect("v4 socket");
    socket
        .bind("127.0.0.1:0".parse().expect("address syntax"))
        .expect("bind");
    let addr = socket.local_addr().expect("local_addr");
    assert_ne!(addr.port(), 0);
    (socket, addr)
}

/// Measures the refusal that address gives **right now** in the form `os error N`. The measuring itself is the confirmation of 「nobody is listening」.
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
        .expect("TCP attached to a port nobody listens on — it means someone else forcibly took this number");
    assert_eq!(
        refused.kind(),
        ErrorKind::ConnectionRefused,
        "this port fails in a way other than a refusal — the first hypothesis is a firewall eating loopback: {refused}"
    );
    format!(
        "os error {}",
        refused.raw_os_error().expect("OS error number")
    )
}

#[tokio::test]
async fn dialing_a_port_that_nobody_listens_on_is_unreachable_not_rejected() {
    // ★This is where 「could not connect」 and 「connected but rejected」 are told apart★ — if they are not told apart, the contract that a refusal does not
    //   spend the reconnect budget (ADR-0180 decision 5) cannot be confirmed on a real socket. The refusal side is measured by `ws_reject.rs`.
    let (_held, closed_at) = unlistened_port();
    let closed = Address::new(format!("ws://{closed_at}/"));

    let registry = common::registry(Arc::new(common::dialer()), Policy::default());
    let (peer, mut inbox) =
        registry.add(PeerId::new("d1"), closed.clone(), Arc::new(HelloHandshake));

    // ★Extract the expected value directly from that port★ — by branch name alone 「the link was cut after being accepted」 · 「the address
    //   syntax is wrong」 · 「the handshake hit the deadline」 are all the same `Unreachable`. The one stimulus this test means to produce is **the connection
    //   not even being established**, and the OS number is the only evidence of that stimulus.
    // ★Measure it **side by side** with the supervisor's dial★ — one refusal takes 2.0 seconds on loopback (measured 2026-09-07, Windows 11).
    //   Measured in sequence that value would be added to this test twice, and side by side the two waits overlap and cost only one.
    let (refusal, cause) = tokio::join!(
        refusal_of(closed_at),
        common::wait_incoming(
            &mut inbox,
            "ConnectFailed event (first hypothesis = the supervisor does not raise an incoming reject as an event)",
            |msg| match msg.as_event() {
                Some(TransportEvent::ConnectFailed { cause, .. }) => Some(cause.clone()),
                _ => None,
            },
        )
    );
    let ConnectFailure::Unreachable(err) = &cause else {
        panic!("a port that is not open must be a failure to attach: {cause:?}");
    };
    assert!(
        err.message.contains(closed.as_str()),
        "a failure other than for the address we dialed came up ({} expected): {err}",
        closed.as_str()
    );
    assert!(
        err.message.contains(&refusal),
        "another failure, not the refusal this port just gave ({refusal}), took on this name — the first hypothesis is that the link was cut \
          after being accepted (that one has a different number): {err}"
    );
    assert!(!peer.peer_state().is_live());
}
