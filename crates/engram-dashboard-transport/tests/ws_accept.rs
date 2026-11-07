//! Real socket ④ — does the receiving side's accept policy hold in a real upgrade (TRD 3 §5 1 · 2 · 12 · 14 · 15 · 16 · 18 · 3-0 U7).
//!
//! ★Only what cannot be measured in memory is placed here★ — the header decision table · the loopback decision · loss in the report queue are measured by the `ws.rs` unit tests without a
//! socket. What is measured here is whether those decisions hold in a **real HTTP upgrade** (does the 403 reach the peer · does a refusal
//! come out as a report), whether the deadline really closes the TCP, whether the size cap went into the real tungstenite settings (both directions), whether the permit count is right in the real accept
//! order, and whether an accepted link put on the registry carries requests both ways.
//!
//! ★Part of the dialing side is not this crate's★ — our [`WsDialer`] adds no Origin, does not change Host, and blocks the outgoing
//! size itself, so the places that build those headers and sizes call `tokio-tungstenite` directly or write to TCP by hand.
#![cfg(all(feature = "ws", feature = "test-support"))]

// This suite does not use the `TestWire` scaffolding (`registry` · `wait_*` · `listen` · `accept_link`) — the other three do.
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

/// The next refusal report. Panics: when [`common::PATIENCE`] is exceeded · when the listener disappears.
async fn next_refusal(refusals: &mut AcceptRefusals, what: &str) -> AcceptRefusal {
    tokio::time::timeout(common::PATIENCE, refusals.recv())
        .await
        .unwrap_or_else(|_| panic!("{:?} 안에 {what} 보고가 안 왔다", common::PATIENCE))
        .unwrap_or_else(|| panic!("{what} 보고 전에 수신단이 끝났다"))
}

/// Re-checks briefly until the condition holds — ★used only where waiting for a count that has no notification (permits · ready queue)★. Not used to measure
/// time (`common` header). Panics: when [`common::PATIENCE`] is exceeded.
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

/// An upgrade with headers attached by hand — a header of the same name is replaced (Host too).
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

/// Whether the peer closed — 0 bytes (FIN) or an error (RST) means closed. Panics: not closed within [`common::PATIENCE`].
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
    // D21 ① — if the list is empty, every upgrade carrying an Origin is cut.
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
    // D9 — if there is no Host it is accepted. tungstenite's dialing side builds no request without a Host, so it is written to TCP by hand.
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

// ── Upgrade deadline · accept separation (§5 12) ──

#[tokio::test]
async fn a_silent_tcp_peer_is_closed_at_the_deadline_without_holding_up_other_accepts() {
    let deadline = Duration::from_secs(2);
    let (mut listener, mut refusals) = bind(AcceptPolicy {
        upgrade_timeout: deadline,
        ..common::accept_policy()
    });
    let addr = listener.dial_address();

    // Opens only TCP and sends nothing.
    let opened = Instant::now();
    let mut silent = TcpStream::connect(listener.local_addr())
        .await
        .expect("TCP");
    wait_until("침묵한 상대가 칸을 쥠", || {
        listener.permits_in_use() == 1
    })
    .await;

    // ★The second accept stands even with a stalled peer★ — if the upgrade were waited on at the accept spot, this would block until the deadline.
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

// ── Size cap in both directions (§5 15) ──

const LIMIT: usize = 64 * 1024;

#[tokio::test]
async fn a_message_over_the_limit_breaks_the_link_into_the_listener() {
    let (mut listener, _refusals) = bind(AcceptPolicy {
        max_message_bytes: LIMIT,
        ..common::accept_policy()
    });
    // ★The dialing side is tungstenite defaults★ — our dialing side blocks a frame over the cap itself, so it never reaches the receiving side.
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

    // Outgoing side — a frame over the same value fails as a value before reaching the socket.
    let refused = server_tx.send(Frame::Binary(vec![0u8; LIMIT + 1])).await;
    assert!(refused.is_err(), "받는 쪽 끝이 상한 + 1 을 내보냈다");
}

#[tokio::test]
async fn a_message_over_the_limit_breaks_the_link_into_the_dialer() {
    // ★The receiving side is tungstenite defaults★ — our receiving end blocks a frame over the cap itself.
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
        // Held until the dialing side cuts — if released first, we cannot tell whether the cut was due to size.
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

// ── Upgrade permits · ready queue (§5 14 · §3-1) ──

/// Returns the dialed link while holding it — if released, that link in the receiving side's ready queue is cut soon too.
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
        // So that the silent peer is not caught by the deadline during the test — the permit must be released by the peer's cut, not by the deadline.
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

    // A peer that cuts during the upgrade releases its permit right away.
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

    // While the consumer does not call `accept`, a link that finished its upgrade sits in the ready queue.
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

    // ★Even if taken away, the permit is held until it is put on the registry★ — it is released only once the receiving side's next stage takes hold of it (TRD 3 §5 14).
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

// ── Refusal report (§5 18) ──

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
    // ★Accepting stands even when reports are blocked★.
    raw_upgrade(&listener, &[]).await.expect("업그레이드");
    accept(&mut listener).await;

    assert_eq!(
        refusals.try_recv().map(|r| r.kind),
        Some(RefusalKind::OriginRejected)
    );
    assert!(refusals.try_recv().is_none(), "줄 칸보다 많이 남았다");
}

// ── Putting an accepted link on the registry (§3-1 ① — end to end) ──

/// A vocabulary both sides speak — `TestWire` is the dialing side's vocabulary, so the receiving side cannot read requests.
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

    // There are no stream chunks, so it is never called.
    fn resume_request(&self, _key: &u32, _after: Option<u64>) -> Msg {
        Msg::Ask {
            tag: u64::MAX,
            body: String::new(),
        }
    }
}

/// The receiving-side pair — sends `welcome` first and on receiving the peer's `hello` goes to the operating phase.
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

/// `asker` asks and `answerer` reads that question from its own receiver and answers — it ends only when both are polled together.
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
    // ★The listener and the receiving registry use the same clock★ — `upgraded_at` and the auth deadline must be on the same clock (`WsListener::bind`).
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
    // Entering the operating phase releases the upgrade permit.
    wait_until("받은 상대가 칸을 놓음", || {
        listener.permits_in_use() == 0
    })
    .await;

    ask_across(&daemon, &client, &mut client_inbox, 1, "from the dialer").await;
    ask_across(&client, &daemon, &mut daemon_inbox, 2, "from the listener").await;
    assert_eq!(client.peer_state(), PeerState::Live);
}
