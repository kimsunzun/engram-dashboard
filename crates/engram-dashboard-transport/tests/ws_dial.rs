//! 실소켓 ① — WS 어댑터가 실제로 붙나(TRD §9-1).
//!
//! ★두 끝이 다 이 crate 것이다★ — 받는 쪽 `ws::WsListener` · 거는 쪽 `ws::WsDialer`. 그래서 이 파일은
//! 다른 crate 도 외부 프로세스도 부르지 않는다.
//!
//! ★`test-support` 까지 요구하는 것은 의도★ — 패킷 어휘(`TestWire`)와 두 걸음 핸드셰이크가 그 뒤에
//! 산다. `--all-features` 가 둘을 함께 켜므로 게이트 ④ 가 이 파일을 실제로 돈다.
#![cfg(all(feature = "ws", feature = "test-support"))]

mod common;

use std::sync::Arc;

use engram_dashboard_transport::link::LinkRead;
use engram_dashboard_transport::testing::{HelloHandshake, TestIn, TestOut, TestWire};
use engram_dashboard_transport::ws::{WsDialer, WsListener};
use engram_dashboard_transport::{
    ConnectFailure, Frame, Generation, PeerId, Policy, TransportEvent,
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

#[tokio::test]
async fn dialing_a_port_that_nobody_listens_on_is_unreachable_not_rejected() {
    // ★「못 붙었다」와 「붙었는데 거절」이 갈리는 자리다★ — 갈리지 않으면 거절이 재연결 예산을 안
    //   쓴다는 계약(ADR-0180 결정 5)을 실소켓에서 확인할 수 없다. 거절 쪽은 `ws_reject.rs` 가 잰다.
    //
    // ★안 열린 포트 번호를 얻는 방법이 「열고 바로 닫는다」뿐이다★ — 그 사이 다른 테스트나 다른
    //   프로세스가 같은 번호를 가져갈 수 있다(OS 사실이고 이 파일에서 막을 수단이 없다). 그래서 아래는
    //   갈래만 보지 않고 **문구까지** 본다: 남의 리스너에 닿아 핸드셰이크가 시한에 걸리거나 주소 문법이
    //   틀려도 같은 `Unreachable` 이라, 문구를 안 보면 다른 사유가 이 이름을 입고 조용히 통과한다.
    let closed = {
        let listener = WsListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.dial_address().expect("dial_address");
        drop(listener);
        addr
    };

    let (registry, mut inbound) = common::registry(Arc::new(WsDialer), Policy::default());
    let peer = registry.add(PeerId::new("d1"), closed.clone(), Arc::new(HelloHandshake));

    let cause = common::wait_incoming(
        &mut inbound,
        "ConnectFailed 사건(첫 가설 = 그 사이 남이 이 포트를 가져가 핸드셰이크가 매달렸다)",
        |msg| match msg.as_event() {
            Some(TransportEvent::ConnectFailed { cause, .. }) => Some(cause.clone()),
            _ => None,
        },
    )
    .await;
    let ConnectFailure::Unreachable(err) = &cause else {
        panic!("안 열린 포트는 못 붙은 것이어야 한다: {cause:?}");
    };
    assert!(
        err.message.contains(closed.as_str()),
        "우리가 건 주소가 아닌 다른 실패가 올라왔다 ({} 를 기대): {err}",
        closed.as_str()
    );
    // ★`os error N` 접미는 OS 가 거절한 연결에만 붙는다★ — `std::io::Error` 의 Display 가 언제나
    //   그것을 적고(번호는 플랫폼마다 다르다: Windows 10061 · Linux 111), 주소 문법 오류나 핸드셰이크
    //   시한에는 붙지 않는다. 문구 본문은 OS 로케일에 따라 번역되므로 번호 쪽만 본다.
    assert!(
        err.message.contains("os error"),
        "연결 거절이 아니라 다른 실패가 이 이름을 입었다 — 주소 문법 오류나 남의 리스너와의 \
         핸드셰이크 시한이 첫 가설이다: {err}"
    );
    assert!(!peer.peer_state().is_live());
}
