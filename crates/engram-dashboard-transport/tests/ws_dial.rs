//! Real socket ① — does the WS adapter really connect (TRD §9-1). ★Both ends are this crate's★ (`ws::WsListener` · `ws::WsDialer`), so no other crate or external process is called. ★Requiring even `test-support` is intentional★ —
//! the packet vocabulary (`TestWire`) and the two-step handshake live behind it; `--all-features` turns both on, so gate ④ really runs this file.
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
    // ★Do not hard-code a port★ — request 0 and read back the kernel-picked number.
    assert_ne!(listener.local_addr().port(), 0);
    let addr = listener.dial_address();

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

/// An address of a port nobody listens on, **held**. ★The caller holds the returned socket until the test ends★ — released, the number is freed and the properties vanish. ★Not calling `listen()` is the whole of this scaffolding★
/// (external fact): with no listening socket a connection is refused without being established ([`refusal_of`] re-measures it each time), and the live bind stops anyone else taking the number (measured on Windows 11: an
/// ordinary bind got `AddrInUse` (10048), a forced `SO_REUSEADDR` bind `PermissionDenied` (10013)). ★Do not go back to 「open and close right away」★ — someone else could take the number, accept TCP and cut during the upgrade;
/// that too comes up `Unreachable` with **`os error 10054`** (same number for FIN or RST), so an assertion on `os error` alone would pass that branch — which is why [`refusal_of`] pins the number too.
fn unlistened_port() -> (TcpSocket, SocketAddr) {
    let socket = TcpSocket::new_v4().expect("v4 socket");
    socket
        .bind("127.0.0.1:0".parse().expect("address syntax"))
        .expect("bind");
    let addr = socket.local_addr().expect("local_addr");
    assert_ne!(addr.port(), 0);
    (socket, addr)
}

/// Measures the refusal that address gives **right now** as `os error N` (the measuring confirms 「nobody is listening」). ★The number is measured each time, not hard-coded★ — it differs per platform (Windows 10061 · Linux 111 ·
/// macOS 61). ★It does not look at the message body★ (external fact): `io::Error`'s `Display` appends `(os error N)` after the OS-locale text (the Korean Windows returns Korean), and only the number is untranslated.
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
    // ★This tells 「could not connect」 from 「connected but rejected」★ — undistinguished, 「a refusal does not spend the reconnect budget」 (ADR-0180 decision 5) cannot be confirmed on a real socket; the refusal side is `ws_reject.rs`.
    let (_held, closed_at) = unlistened_port();
    let closed = Address::new(format!("ws://{closed_at}/"));

    let registry = common::registry(Arc::new(common::dialer()), Policy::default());
    let (peer, mut inbox) =
        registry.add(PeerId::new("d1"), closed.clone(), Arc::new(HelloHandshake));

    // ★Extract the expected value directly from that port★ — by branch name 「cut after accept」 · 「bad address syntax」 · 「handshake hit the deadline」 are all `Unreachable`; this test's stimulus is **the connection not being
    //   established at all**, and the OS number is the only evidence. ★Measure it **side by side** with the supervisor's dial★ — one refusal takes 2.0 s on loopback (measured, Windows 11); in sequence it would be paid twice.
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
