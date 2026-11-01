//! 실소켓 ④ — 받는 쪽 수락 정책이 실제 업그레이드에서 서나(TRD 3 §5 1 · 2 · 12 · 14 · 15 · 16 · 18 · 3-0 U7).
//!
//! ★인메모리로 못 재는 것만 여기 둔다★ — 머리 판정 표 · loopback 판정 · 보고 줄의 손실은 `ws.rs` 단위 테스트가 소켓
//! 없이 잰다. 여기서 재는 것은 그 판정이 **실제 HTTP 업그레이드**에 걸리나(403 이 상대에게 닿나 · 거절이 보고로
//! 나오나), 시한이 실제 TCP 를 닫나, 크기 상한이 실제 tungstenite 설정에 들어갔나(양방향), 칸 셈이 실제 수락
//! 순서에서 맞나, 그리고 받은 통로가 명부에 얹혀 양쪽으로 요청이 흐르나다.
//!
//! ★거는 쪽 일부는 이 crate 것이 아니다★ — 우리 [`WsDialer`] 는 Origin 을 안 달고 Host 를 바꾸지 않으며 나가는
//! 크기를 스스로 막으므로, 그 머리 · 크기를 만드는 자리는 `tokio-tungstenite` 를 그대로 부르거나 TCP 에 손으로 쓴다.
#![cfg(all(feature = "ws", feature = "test-support"))]

// 이 스위트는 `TestWire` 발판(`registry` · `wait_*` · `listen` · `accept_link`)을 안 쓴다 — 나머지 셋은 쓴다.
#[allow(dead_code)]
mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{header, HeaderValue, StatusCode};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;

use futures_util::{SinkExt, StreamExt};

use engram_dashboard_transport::testing::{accepted_ends, HelloHandshake};
use engram_dashboard_transport::ws::{
    AcceptPolicy, AcceptRefusal, AcceptRefusals, RefusalKind, WsDialer, WsListener,
};
use engram_dashboard_transport::{
    Accepted, Address, Clock, DecodeClass, DialFailure, Dialer, Frame, Handshake, HandshakeStep,
    Inbox, LinkRead, LinkRx, LinkTx, Listener, Peer, PeerId, PeerState, Policy, Registry,
    RejectKind, StreamMark, SystemClock, Wire,
};

// ── 발판 ──

fn bind(policy: AcceptPolicy) -> (WsListener, AcceptRefusals) {
    WsListener::bind(common::loopback(), policy, Arc::new(SystemClock)).expect("bind")
}

/// 다음 거절 보고. 패닉 조건: [`common::PATIENCE`] 초과 · 청취기가 사라짐.
async fn next_refusal(refusals: &mut AcceptRefusals, what: &str) -> AcceptRefusal {
    tokio::time::timeout(common::PATIENCE, refusals.recv())
        .await
        .unwrap_or_else(|_| panic!("{:?} 안에 {what} 보고가 안 왔다", common::PATIENCE))
        .unwrap_or_else(|| panic!("{what} 보고 전에 수신단이 끝났다"))
}

/// 조건이 설 때까지 짧게 다시 본다 — ★알림이 없는 셈(칸 · 준비 큐)을 기다리는 자리에만 쓴다★. 시각을 재는 데 쓰지
/// 않는다(`common` 헤더). 패닉 조건: [`common::PATIENCE`] 초과.
async fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    let waited = tokio::time::timeout(common::PATIENCE, async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(
        waited.is_ok(),
        "{:?} 안에 {what} 이 되지 않았다",
        common::PATIENCE
    );
}

/// 머리를 손으로 단 업그레이드 — 같은 이름의 머리는 갈아 끼운다(Host 도).
async fn raw_upgrade(
    listener: &WsListener,
    headers: &[(header::HeaderName, &str)],
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, WsError> {
    let mut request = format!("ws://{}/", listener.local_addr())
        .into_client_request()
        .expect("요청");
    for (name, value) in headers {
        request
            .headers_mut()
            .insert(name.clone(), HeaderValue::from_str(value).expect("머리 값"));
    }
    tokio::time::timeout(common::PATIENCE, tokio_tungstenite::connect_async(request))
        .await
        .expect("업그레이드 상한")
        .map(|(ws, _response)| ws)
}

fn status(result: Result<WebSocketStream<MaybeTlsStream<TcpStream>>, WsError>) -> StatusCode {
    match result {
        Err(WsError::Http(response)) => response.status(),
        Err(other) => panic!("HTTP 거절이 아닌 실패: {other}"),
        Ok(_) => panic!("업그레이드가 받아들여졌다"),
    }
}

async fn accept(listener: &mut WsListener) -> Accepted {
    tokio::time::timeout(common::PATIENCE, listener.accept())
        .await
        .expect("수락 상한")
        .expect("수락")
}

/// 상대가 닫았나 — 0 바이트(FIN) 또는 오류(RST)면 닫힌 것이다. 패닉 조건: [`common::PATIENCE`] 안에 안 닫힘.
async fn closed_by_peer(stream: &mut TcpStream, what: &str) {
    let mut buf = [0u8; 64];
    match tokio::time::timeout(common::PATIENCE, stream.read(&mut buf)).await {
        Ok(Ok(0)) | Ok(Err(_)) => {}
        Ok(Ok(n)) => panic!("{what}: 닫는 대신 {n} 바이트가 왔다"),
        Err(_) => panic!("{:?} 안에 {what} 이 닫히지 않았다", common::PATIENCE),
    }
}

// ── loopback (§5 1) ──

#[tokio::test]
async fn a_listener_opens_only_on_loopback() {
    let (listener, _refusals) = bind(common::accept_policy());
    let local = listener.local_addr();
    assert!(local.ip().is_loopback(), "{local}");
    assert_ne!(local.port(), 0, "커널이 고른 번호를 되읽어야 한다");

    let anywhere: SocketAddr = "0.0.0.0:0".parse().unwrap();
    let refused = WsListener::bind(anywhere, common::accept_policy(), Arc::new(SystemClock))
        .err()
        .expect("모든 주소에 평문 ws:// 를 열었다");
    assert!(refused.message.contains("loopback"), "{refused}");
}

// ── Origin (§5 2) ──

#[tokio::test]
async fn an_origin_outside_the_list_gets_403_and_a_report_while_listed_or_absent_ones_pass() {
    let (mut listener, mut refusals) = bind(AcceptPolicy {
        allowed_origins: vec!["http://localhost:1420".into()],
        ..common::accept_policy()
    });

    raw_upgrade(&listener, &[]).await.expect("Origin 없음");
    assert_eq!(accept(&mut listener).await.info().origin, None);

    raw_upgrade(&listener, &[(header::ORIGIN, "http://localhost:1420")])
        .await
        .expect("목록에 든 Origin");
    assert_eq!(
        accept(&mut listener).await.info().origin.as_deref(),
        Some("http://localhost:1420"),
        "받은 Origin 이 소비자에게 안 건너왔다"
    );

    let refused = raw_upgrade(&listener, &[(header::ORIGIN, "http://evil.example")]).await;
    assert_eq!(status(refused), StatusCode::FORBIDDEN);
    let report = next_refusal(&mut refusals, "Origin 거절").await;
    assert_eq!(report.kind, RefusalKind::OriginRejected);
    assert_eq!(report.origin.as_deref(), Some("http://evil.example"));
    assert!(report.remote.is_some());
    assert_eq!(listener.queued(), 0, "거절한 통로가 준비 큐에 앉았다");
    assert_eq!(listener.permits_in_use(), 0, "거절한 통로가 칸을 쥐고 있다");
}

#[tokio::test]
async fn the_release_value_refuses_any_origin() {
    // D21 ① — 목록이 비면 Origin 을 단 업그레이드는 전부 끊긴다.
    let (listener, mut refusals) = bind(common::accept_policy());
    let refused = raw_upgrade(&listener, &[(header::ORIGIN, "http://tauri.localhost")]).await;
    assert_eq!(status(refused), StatusCode::FORBIDDEN);
    assert_eq!(
        next_refusal(&mut refusals, "Origin 거절").await.kind,
        RefusalKind::OriginRejected
    );
}

// ── Host (§5 16) ──

#[tokio::test]
async fn a_host_other_than_a_loopback_name_on_this_port_gets_403() {
    let (mut listener, mut refusals) = bind(common::accept_policy());
    let port = listener.local_addr().port();

    raw_upgrade(&listener, &[(header::HOST, &format!("localhost:{port}"))])
        .await
        .expect("받는 이름");
    accept(&mut listener).await;

    for bad in [
        format!("evil.example:{port}"),
        format!("127.0.0.1:{}", port.wrapping_add(1)),
    ] {
        let refused = raw_upgrade(&listener, &[(header::HOST, &bad)]).await;
        assert_eq!(status(refused), StatusCode::FORBIDDEN, "{bad}");
        let report = next_refusal(&mut refusals, "Host 거절").await;
        assert_eq!(report.kind, RefusalKind::HostRejected, "{bad}");
        assert_eq!(report.host.as_deref(), Some(bad.as_str()));
    }
}

#[tokio::test]
async fn an_upgrade_without_a_host_header_is_accepted() {
    // D9 — Host 가 없으면 받는다. tungstenite 의 거는 쪽은 Host 를 빼고는 요청을 안 지으므로 TCP 에 손으로 쓴다.
    let (mut listener, _refusals) = bind(common::accept_policy());
    let mut tcp = TcpStream::connect(listener.local_addr())
        .await
        .expect("TCP");
    tcp.write_all(
        b"GET / HTTP/1.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
          Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
    )
    .await
    .expect("요청 쓰기");
    let mut head = Vec::new();
    let mut buf = [0u8; 512];
    tokio::time::timeout(common::PATIENCE, async {
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = tcp.read(&mut buf).await.expect("응답 읽기");
            assert_ne!(n, 0, "응답 전에 닫혔다");
            head.extend_from_slice(&buf[..n]);
        }
    })
    .await
    .expect("응답 상한");
    assert!(
        head.starts_with(b"HTTP/1.1 101"),
        "{}",
        String::from_utf8_lossy(&head)
    );
    accept(&mut listener).await;
}

// ── 업그레이드 시한 · 수락 분리 (§5 12) ──

#[tokio::test]
async fn a_silent_tcp_peer_is_closed_at_the_deadline_without_holding_up_other_accepts() {
    let deadline = Duration::from_secs(2);
    let (mut listener, mut refusals) = bind(AcceptPolicy {
        upgrade_timeout: deadline,
        ..common::accept_policy()
    });
    let addr = listener.dial_address();

    // TCP 만 열고 아무것도 안 보낸다.
    let opened = Instant::now();
    let mut silent = TcpStream::connect(listener.local_addr())
        .await
        .expect("TCP");
    wait_until("침묵한 상대가 칸을 쥠", || {
        listener.permits_in_use() == 1
    })
    .await;

    // ★멈춘 상대가 있어도 둘째 수락이 선다★ — 업그레이드를 수락 자리에서 기다리면 여기가 시한까지 막힌다.
    let dialer = common::dialer();
    let (accepted, dialed) = tokio::time::timeout(common::PATIENCE, async {
        tokio::join!(listener.accept(), dialer.dial(&addr))
    })
    .await
    .expect("둘째 수락 상한");
    accepted.expect("둘째 수락");
    dialed.expect("둘째 걸기");
    let mut probe = [0u8; 1];
    assert!(
        matches!(silent.try_read(&mut probe), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
        "둘째 수락이 끝났을 때 침묵한 상대가 이미 닫혀 있었다 — 시한보다 늦게 수락한 것이다"
    );

    closed_by_peer(&mut silent, "침묵한 상대").await;
    let waited = opened.elapsed();
    assert!(
        waited >= deadline - Duration::from_millis(100),
        "시한({deadline:?}) 전에 닫혔다({waited:?}) — 시한이 아닌 다른 까닭이다"
    );
    let report = next_refusal(&mut refusals, "업그레이드 시한").await;
    assert_eq!(report.kind, RefusalKind::UpgradeTimeout);
    assert_eq!(
        report.remote.as_deref(),
        Some(silent.local_addr().unwrap().to_string().as_str())
    );
    wait_until("시한에 걸린 상대가 칸을 놓음", || {
        listener.permits_in_use() == 0
    })
    .await;
}

// ── 크기 상한 양방향 (§5 15) ──

const LIMIT: usize = 64 * 1024;

#[tokio::test]
async fn a_message_over_the_limit_breaks_the_link_into_the_listener() {
    let (mut listener, _refusals) = bind(AcceptPolicy {
        max_message_bytes: LIMIT,
        ..common::accept_policy()
    });
    // ★거는 쪽은 tungstenite 기본값이다★ — 우리 거는 쪽은 상한을 넘는 프레임을 스스로 막아 받는 쪽까지 안 간다.
    let mut client = raw_upgrade(&listener, &[]).await.expect("업그레이드");
    let (mut server_tx, mut server_rx) = accepted_ends(accept(&mut listener).await);

    client
        .send(Message::Binary(vec![1u8; LIMIT].into()))
        .await
        .expect("상한 그대로");
    match tokio::time::timeout(common::PATIENCE, server_rx.recv()).await {
        Ok(Ok(LinkRead::Frame(Frame::Binary(bytes)))) => assert_eq!(bytes.len(), LIMIT),
        other => panic!("상한 그대로가 건너오지 않았다: {other:?}"),
    }
    client
        .send(Message::Binary(vec![1u8; LIMIT + 1].into()))
        .await
        .expect("보내는 쪽은 안 막는다");
    let over = tokio::time::timeout(common::PATIENCE, server_rx.recv())
        .await
        .expect("읽기 상한")
        .expect_err("상한 + 1 을 받아들였다");
    assert!(over.message.contains("too long"), "{over}");

    // 나가는 쪽 — 같은 값을 넘는 프레임은 소켓에 닿기 전에 값으로 실패한다.
    let refused = server_tx.send(Frame::Binary(vec![0u8; LIMIT + 1])).await;
    assert!(refused.is_err(), "받는 쪽 끝이 상한 + 1 을 내보냈다");
}

#[tokio::test]
async fn a_message_over_the_limit_breaks_the_link_into_the_dialer() {
    // ★받는 쪽은 tungstenite 기본값이다★ — 우리 받는 쪽 끝은 상한을 넘는 프레임을 스스로 막는다.
    let raw = TcpListener::bind(common::loopback()).await.expect("bind");
    let addr = Address::new(format!("ws://{}/", raw.local_addr().expect("local_addr")));
    let server = tokio::spawn(async move {
        let (tcp, _) = raw.accept().await.expect("TCP 수락");
        let mut ws = tokio_tungstenite::accept_async(tcp)
            .await
            .expect("업그레이드");
        ws.send(Message::Binary(vec![2u8; LIMIT].into()))
            .await
            .expect("상한 그대로");
        ws.send(Message::Binary(vec![2u8; LIMIT + 1].into()))
            .await
            .expect("보내는 쪽은 안 막는다");
        // 거는 쪽이 끊을 때까지 쥔다 — 먼저 놓으면 끊김이 크기 때문인지 가를 수 없다.
        while ws.next().await.is_some() {}
    });

    let (_tx, mut rx) = tokio::time::timeout(common::PATIENCE, WsDialer::new(LIMIT).dial(&addr))
        .await
        .expect("걸기 상한")
        .expect("걸기");
    match tokio::time::timeout(common::PATIENCE, rx.recv()).await {
        Ok(Ok(LinkRead::Frame(Frame::Binary(bytes)))) => assert_eq!(bytes.len(), LIMIT),
        other => panic!("상한 그대로가 건너오지 않았다: {other:?}"),
    }
    let over = tokio::time::timeout(common::PATIENCE, rx.recv())
        .await
        .expect("읽기 상한")
        .expect_err("상한 + 1 을 받아들였다");
    assert!(over.message.contains("too long"), "{over}");
    server.abort();
}

// ── 업그레이드 칸 · 준비 큐 (§5 14 · §3-1) ──

/// 건 통로를 쥔 채 돌려준다 — 놓으면 받는 쪽 준비 큐의 그 통로도 곧 끊긴다.
async fn held_dial(dialer: &WsDialer, addr: &Address) -> (Box<dyn LinkTx>, Box<dyn LinkRx>) {
    tokio::time::timeout(common::PATIENCE, dialer.dial(addr))
        .await
        .expect("걸기 상한")
        .expect("걸기")
}

async fn dial(dialer: &WsDialer, addr: &Address) -> Result<(), DialFailure> {
    tokio::time::timeout(common::PATIENCE, dialer.dial(addr))
        .await
        .expect("걸기 상한")
        .map(drop)
}

#[tokio::test]
async fn full_upgrade_slots_close_new_tcp_at_once_and_a_failed_peer_frees_its_slot() {
    let (mut listener, mut refusals) = bind(AcceptPolicy {
        max_pending_upgrades: 2,
        // 침묵한 상대가 시험 동안 시한에 안 걸리게 — 칸을 놓는 것이 시한이 아니라 상대의 끊음이어야 한다.
        upgrade_timeout: Duration::from_secs(60),
        ..common::accept_policy()
    });
    let at = listener.local_addr();
    let first = TcpStream::connect(at).await.expect("TCP 1");
    let _second = TcpStream::connect(at).await.expect("TCP 2");
    wait_until("칸 둘이 참", || listener.permits_in_use() == 2).await;

    let mut third = TcpStream::connect(at).await.expect("TCP 3");
    closed_by_peer(&mut third, "칸이 없을 때 온 셋째").await;
    let report = next_refusal(&mut refusals, "칸 넘침").await;
    assert_eq!(report.kind, RefusalKind::SlotsFull);
    assert_eq!(
        report.remote.as_deref(),
        Some(third.local_addr().unwrap().to_string().as_str()),
        "넘친 **새것을** 닫아야 한다"
    );
    assert_eq!(listener.permits_in_use(), 2, "있던 상대의 칸이 흔들렸다");

    // 업그레이드 중 끊은 상대는 칸을 곧바로 놓는다.
    drop(first);
    assert_eq!(
        next_refusal(&mut refusals, "업그레이드 중 끊김").await.kind,
        RefusalKind::UpgradeFailed
    );
    wait_until("끊은 상대가 칸을 놓음", || {
        listener.permits_in_use() == 1
    })
    .await;
    let dialer = common::dialer();
    let addr = listener.dial_address();
    let (accepted, dialed) = tokio::time::timeout(common::PATIENCE, async {
        tokio::join!(listener.accept(), dialer.dial(&addr))
    })
    .await
    .expect("수락 상한");
    accepted.expect("빈 칸으로 수락");
    dialed.expect("빈 칸으로 걸기");
}

#[tokio::test]
async fn the_ready_queue_is_bounded_by_the_slots_until_the_accepted_link_is_let_go() {
    let (mut listener, mut refusals) = bind(AcceptPolicy {
        max_pending_upgrades: 2,
        ..common::accept_policy()
    });
    let addr = listener.dial_address();
    let dialer = common::dialer();

    // 소비자가 `accept` 를 안 부르는 동안 업그레이드를 마친 통로가 준비 큐에 앉는다.
    let held = [
        held_dial(&dialer, &addr).await,
        held_dial(&dialer, &addr).await,
    ];
    wait_until("준비 큐에 둘", || listener.queued() == 2).await;
    assert_eq!(listener.permits_in_use(), 2);
    assert!(
        dial(&dialer, &addr).await.is_err(),
        "준비 큐가 칸을 넘어 찼다"
    );
    assert_eq!(
        next_refusal(&mut refusals, "칸 넘침").await.kind,
        RefusalKind::SlotsFull
    );

    // ★받아 갔어도 얹기 전까지는 칸을 쥔다★ — 받는 쪽이 다음 칸을 쥐어야 놓인다(TRD 3 §5 14).
    let taken = accept(&mut listener).await;
    assert_eq!(listener.queued(), 1);
    assert!(
        dial(&dialer, &addr).await.is_err(),
        "받아 간 통로가 칸을 놓았다"
    );
    assert_eq!(
        next_refusal(&mut refusals, "칸 넘침").await.kind,
        RefusalKind::SlotsFull
    );

    drop(taken);
    let _third = held_dial(&dialer, &addr).await;
    wait_until("준비 큐에 다시 둘", || listener.queued() == 2).await;
    drop(held);
}

// ── 거절 보고 (§5 18) ──

#[tokio::test]
async fn unread_refusal_reports_are_dropped_and_counted_without_stalling_accepts() {
    let (mut listener, mut refusals) = bind(AcceptPolicy {
        refusal_queue: 1,
        ..common::accept_policy()
    });
    for _ in 0..3 {
        let refused = raw_upgrade(&listener, &[(header::ORIGIN, "http://evil.example")]).await;
        assert_eq!(status(refused), StatusCode::FORBIDDEN);
    }
    wait_until("버린 보고 둘", || refusals.dropped() == 2).await;
    // ★보고가 막혀도 수락은 선다★.
    raw_upgrade(&listener, &[]).await.expect("업그레이드");
    accept(&mut listener).await;

    assert_eq!(
        refusals.try_recv().map(|r| r.kind),
        Some(RefusalKind::OriginRejected)
    );
    assert!(refusals.try_recv().is_none(), "줄 칸보다 많이 남았다");
}

// ── 받은 통로를 명부에 얹는다 (§3-1 ① — 끝에서 끝) ──

/// 양쪽이 같은 말을 쓰는 어휘 — `TestWire` 는 거는 쪽 어휘라 받는 쪽이 요청을 못 읽는다.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Msg {
    Ask { tag: u64, body: String },
    Answer { tag: u64, body: String },
}

struct BothWays;

#[derive(Debug)]
struct Garbled;

impl DecodeClass for Garbled {
    fn class(&self) -> &'static str {
        "garbled"
    }
}

impl Wire for BothWays {
    type Out = Msg;
    type In = Msg;
    type Tag = u64;
    type StreamKey = u32;
    type DecodeError = Garbled;

    fn encode(&self, out: &Msg) -> Frame {
        match out {
            Msg::Ask { tag, body } => Frame::Text(format!("ask|{tag}|{body}")),
            Msg::Answer { tag, body } => Frame::Text(format!("ans|{tag}|{body}")),
        }
    }

    fn decode(&self, frame: Frame) -> Result<Msg, Garbled> {
        let Frame::Text(text) = frame else {
            return Err(Garbled);
        };
        let mut parts = text.splitn(3, '|');
        let (kind, tag, body) = (parts.next(), parts.next(), parts.next());
        let tag = tag.and_then(|t| t.parse().ok()).ok_or(Garbled)?;
        let body = body.ok_or(Garbled)?.to_string();
        match kind {
            Some("ask") => Ok(Msg::Ask { tag, body }),
            Some("ans") => Ok(Msg::Answer { tag, body }),
            _ => Err(Garbled),
        }
    }

    fn request_tag(&self, out: &Msg) -> Option<u64> {
        match out {
            Msg::Ask { tag, .. } => Some(*tag),
            Msg::Answer { .. } => None,
        }
    }

    fn reply_tag(&self, inn: &Msg) -> Option<u64> {
        match inn {
            Msg::Answer { tag, .. } => Some(*tag),
            Msg::Ask { .. } => None,
        }
    }

    fn stream_mark(&self, _inn: &Msg) -> Option<StreamMark<u32>> {
        None
    }

    // 스트림 조각이 없으므로 불리지 않는다.
    fn resume_request(&self, _key: &u32, _after: Option<u64>) -> Msg {
        Msg::Ask {
            tag: u64::MAX,
            body: String::new(),
        }
    }
}

/// 받는 쪽 짝 — 먼저 `welcome` 을 보내고 상대의 `hello` 를 받으면 운영 단계로.
struct Welcome;

impl Handshake for Welcome {
    fn start(&self) -> HandshakeStep {
        HandshakeStep::Send(Frame::Text(HelloHandshake::WELCOME.into()))
    }

    fn on_frame(&self, frame: &Frame) -> HandshakeStep {
        match frame {
            Frame::Text(t) if t == HelloHandshake::HELLO => HandshakeStep::Done,
            _ => HandshakeStep::Reject {
                kind: RejectKind::Terminal,
                last: None,
                reason: "expected hello".into(),
            },
        }
    }
}

async fn wait_live(peer: &Peer<BothWays>, what: &str) {
    let mut state = peer.state();
    let live = tokio::time::timeout(common::PATIENCE, async {
        loop {
            if state.borrow_and_update().is_live() {
                return true;
            }
            if state.changed().await.is_err() {
                return false;
            }
        }
    })
    .await;
    assert_eq!(
        live,
        Ok(true),
        "{what} 이 운영 단계에 안 들었다 (지금 {:?})",
        peer.peer_state()
    );
}

/// `asker` 가 묻고 `answerer` 가 자기 수신단에서 그 물음을 읽어 답한다 — 둘을 함께 폴링해야 끝난다.
async fn ask_across(
    asker: &Peer<BothWays>,
    answerer: &Peer<BothWays>,
    answerer_inbox: &mut Inbox<BothWays>,
    tag: u64,
    body: &str,
) {
    let ask = asker.request(Msg::Ask {
        tag,
        body: body.into(),
    });
    let answer = async {
        while let Some(incoming) = answerer_inbox.recv().await {
            if let Some(Msg::Ask { tag, body }) = incoming.as_message().map(|a| &a.msg) {
                answerer
                    .notify(Msg::Answer {
                        tag: *tag,
                        body: format!("re: {body}"),
                    })
                    .expect("답 보내기");
                return body.clone();
            }
        }
        panic!("물음을 읽기 전에 수신단이 끝났다");
    };
    let (reply, asked) =
        tokio::time::timeout(common::PATIENCE, async { tokio::join!(ask, answer) })
            .await
            .expect("왕복 상한");
    assert_eq!(asked, body, "물음이 그대로 건너가지 않았다");
    assert_eq!(
        reply.expect("답"),
        Msg::Answer {
            tag,
            body: format!("re: {body}")
        }
    );
}

#[tokio::test]
async fn an_accepted_socket_attaches_to_a_registry_and_requests_flow_both_ways() {
    // ★청취기와 받는 명부가 같은 시계를 쓴다★ — `upgraded_at` 과 인증 시한이 같은 시계여야 한다(`WsListener::bind`).
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let (mut listener, _refusals) =
        WsListener::bind(common::loopback(), common::accept_policy(), clock.clone()).expect("bind");
    let accepting = Registry::new(
        Arc::new(BothWays),
        Arc::new(common::dialer()),
        clock,
        Policy::default(),
    );
    let dialing = Registry::new(
        Arc::new(BothWays),
        Arc::new(common::dialer()),
        Arc::new(SystemClock),
        Policy::default(),
    );

    let (daemon, mut daemon_inbox) = dialing.add(
        PeerId::new("daemon"),
        listener.dial_address(),
        Arc::new(HelloHandshake),
    );
    let accepted = accept(&mut listener).await;
    let remote = accepted.info().remote.clone().expect("원격 주소");
    assert!(
        remote
            .parse::<SocketAddr>()
            .expect("ip:port")
            .ip()
            .is_loopback(),
        "{remote}"
    );
    assert_eq!(accepted.info().origin, None);
    let (client, mut client_inbox) =
        accepting.attach(PeerId::new("conn-1"), accepted, Arc::new(Welcome));

    wait_live(&daemon, "거는 쪽").await;
    wait_live(&client, "받는 쪽").await;
    // 운영 단계에 들면 업그레이드 칸을 놓는다.
    wait_until("받은 상대가 칸을 놓음", || {
        listener.permits_in_use() == 0
    })
    .await;

    ask_across(&daemon, &client, &mut client_inbox, 1, "from the dialer").await;
    ask_across(&client, &daemon, &mut daemon_inbox, 2, "from the listener").await;
    assert_eq!(client.peer_state(), PeerState::Live);
}
