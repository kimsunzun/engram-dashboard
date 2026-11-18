//! Real socket ④ — does the receiving side's accept policy hold in a real upgrade (TRD 3 §5 1 · 2 · 12 · 14 · 15 · 16 · 18 · 3-0 U7).
//!
//! ★Only what memory cannot measure lives here★ — the header decision table · loopback decision · report-queue loss are `ws.rs` unit tests. Here: do those decisions hold in a **real HTTP upgrade** (403 reaches the peer ·
//! a refusal comes out as a report), does the deadline really close the TCP, does the size cap reach the real tungstenite settings (both directions), is the permit count right in real accept order, does an accepted link on
//! the registry carry requests both ways. ★Part of the dialing side is not this crate's★ — our [`WsDialer`] adds no Origin, keeps Host, and blocks outgoing size itself, so those headers and sizes are built with
//! `tokio-tungstenite` directly or by hand on TCP.
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

// ── Scaffolding ──

fn bind(policy: AcceptPolicy) -> (WsListener, AcceptRefusals) {
    WsListener::bind(common::loopback(), policy, Arc::new(SystemClock)).expect("bind")
}

async fn next_refusal(refusals: &mut AcceptRefusals, what: &str) -> AcceptRefusal {
    tokio::time::timeout(common::PATIENCE, refusals.recv())
        .await
        .unwrap_or_else(|_| {
            panic!(
                "the {what} report did not come within {:?}",
                common::PATIENCE
            )
        })
        .unwrap_or_else(|| panic!("the inbox ended before the {what} report"))
}

/// Re-checks briefly until the condition holds — ★only where waiting for a count with no notification (permits · ready queue)★; not for measuring time (`common` header). Panics: [`common::PATIENCE`] exceeded.
async fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    let waited = tokio::time::timeout(common::PATIENCE, async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(
        waited.is_ok(),
        "{what} did not happen within {:?}",
        common::PATIENCE
    );
}

async fn raw_upgrade(
    listener: &WsListener,
    headers: &[(header::HeaderName, &str)],
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, WsError> {
    let mut request = format!("ws://{}/", listener.local_addr())
        .into_client_request()
        .expect("request");
    for (name, value) in headers {
        request.headers_mut().insert(
            name.clone(),
            HeaderValue::from_str(value).expect("head value"),
        );
    }
    tokio::time::timeout(common::PATIENCE, tokio_tungstenite::connect_async(request))
        .await
        .expect("upgrade cap")
        .map(|(ws, _response)| ws)
}

fn status(result: Result<WebSocketStream<MaybeTlsStream<TcpStream>>, WsError>) -> StatusCode {
    match result {
        Err(WsError::Http(response)) => response.status(),
        Err(other) => panic!("a failure that is not an HTTP reject: {other}"),
        Ok(_) => panic!("the upgrade was accepted"),
    }
}

async fn accept(listener: &mut WsListener) -> Accepted {
    tokio::time::timeout(common::PATIENCE, listener.accept())
        .await
        .expect("accept cap")
        .expect("accept")
}

async fn closed_by_peer(stream: &mut TcpStream, what: &str) {
    let mut buf = [0u8; 64];
    match tokio::time::timeout(common::PATIENCE, stream.read(&mut buf)).await {
        Ok(Ok(0)) | Ok(Err(_)) => {}
        Ok(Ok(n)) => panic!("{what}: {n} bytes came instead of a close"),
        Err(_) => panic!("{what} did not close within {:?}", common::PATIENCE),
    }
}

// ── loopback (§5 1) ──

#[tokio::test]
async fn a_listener_opens_only_on_loopback() {
    let (listener, _refusals) = bind(common::accept_policy());
    let local = listener.local_addr();
    assert!(local.ip().is_loopback(), "{local}");
    assert_ne!(
        local.port(),
        0,
        "must read back the number the kernel chose"
    );

    let anywhere: SocketAddr = "0.0.0.0:0".parse().unwrap();
    let refused = WsListener::bind(anywhere, common::accept_policy(), Arc::new(SystemClock))
        .err()
        .expect("opened plaintext ws:// on all addresses");
    assert!(refused.message.contains("loopback"), "{refused}");
}

// ── Origin (§5 2) ──

#[tokio::test]
async fn an_origin_outside_the_list_gets_403_and_a_report_while_listed_or_absent_ones_pass() {
    let (mut listener, mut refusals) = bind(AcceptPolicy {
        allowed_origins: vec!["http://localhost:1420".into()],
        ..common::accept_policy()
    });

    raw_upgrade(&listener, &[]).await.expect("no Origin");
    assert_eq!(accept(&mut listener).await.info().origin, None);

    raw_upgrade(&listener, &[(header::ORIGIN, "http://localhost:1420")])
        .await
        .expect("an Origin in the list");
    assert_eq!(
        accept(&mut listener).await.info().origin.as_deref(),
        Some("http://localhost:1420"),
        "the received Origin did not cross to the consumer"
    );

    let refused = raw_upgrade(&listener, &[(header::ORIGIN, "http://evil.example")]).await;
    assert_eq!(status(refused), StatusCode::FORBIDDEN);
    let report = next_refusal(&mut refusals, "Origin reject").await;
    assert_eq!(report.kind, RefusalKind::OriginRejected);
    assert_eq!(report.origin.as_deref(), Some("http://evil.example"));
    assert!(report.remote.is_some());
    assert_eq!(
        listener.queued(),
        0,
        "a rejected link sat in the ready queue"
    );
    assert_eq!(
        listener.permits_in_use(),
        0,
        "a rejected link holds a permit"
    );
}

#[tokio::test]
async fn the_release_value_refuses_any_origin() {
    // D21 ① — if the list is empty, every upgrade carrying an Origin is cut.
    let (listener, mut refusals) = bind(common::accept_policy());
    let refused = raw_upgrade(&listener, &[(header::ORIGIN, "http://tauri.localhost")]).await;
    assert_eq!(status(refused), StatusCode::FORBIDDEN);
    assert_eq!(
        next_refusal(&mut refusals, "Origin reject").await.kind,
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
        .expect("receiving name");
    accept(&mut listener).await;

    for bad in [
        format!("evil.example:{port}"),
        format!("127.0.0.1:{}", port.wrapping_add(1)),
    ] {
        let refused = raw_upgrade(&listener, &[(header::HOST, &bad)]).await;
        assert_eq!(status(refused), StatusCode::FORBIDDEN, "{bad}");
        let report = next_refusal(&mut refusals, "Host reject").await;
        assert_eq!(report.kind, RefusalKind::HostRejected, "{bad}");
        assert_eq!(report.host.as_deref(), Some(bad.as_str()));
    }
}

#[tokio::test]
async fn an_upgrade_without_a_host_header_is_accepted() {
    // D9 — no Host is accepted. tungstenite's dialing side builds no request without a Host, so it is written to TCP by hand.
    let (mut listener, _refusals) = bind(common::accept_policy());
    let mut tcp = TcpStream::connect(listener.local_addr())
        .await
        .expect("TCP");
    tcp.write_all(
        b"GET / HTTP/1.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
          Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
    )
    .await
    .expect("request write");
    let mut head = Vec::new();
    let mut buf = [0u8; 512];
    tokio::time::timeout(common::PATIENCE, async {
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = tcp.read(&mut buf).await.expect("response read");
            assert_ne!(n, 0, "closed before the response");
            head.extend_from_slice(&buf[..n]);
        }
    })
    .await
    .expect("response cap");
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

    let opened = Instant::now();
    let mut silent = TcpStream::connect(listener.local_addr())
        .await
        .expect("TCP");
    wait_until("the silent peer holds a permit", || {
        listener.permits_in_use() == 1
    })
    .await;

    // ★The second accept stands even with a stalled peer★ — waiting for the upgrade at the accept spot would block until the deadline.
    let dialer = common::dialer();
    let (accepted, dialed) = tokio::time::timeout(common::PATIENCE, async {
        tokio::join!(listener.accept(), dialer.dial(&addr))
    })
    .await
    .expect("second accept cap");
    accepted.expect("second accept");
    dialed.expect("second dial");
    let mut probe = [0u8; 1];
    assert!(
        matches!(silent.try_read(&mut probe), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
        "when the second accept finished, the silent peer was already closed — it was accepted later than the deadline"
    );

    closed_by_peer(&mut silent, "silent peer").await;
    let waited = opened.elapsed();
    assert!(
        waited >= deadline - Duration::from_millis(100),
        "closed ({waited:?}) before the deadline ({deadline:?}) — the reason is something other than the deadline"
    );
    let report = next_refusal(&mut refusals, "upgrade deadline").await;
    assert_eq!(report.kind, RefusalKind::UpgradeTimeout);
    assert_eq!(
        report.remote.as_deref(),
        Some(silent.local_addr().unwrap().to_string().as_str())
    );
    wait_until(
        "the peer caught by the deadline releases the permit",
        || listener.permits_in_use() == 0,
    )
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
    // ★The dialing side is tungstenite defaults★ — our dialer blocks a frame over the cap itself, so it never reaches the receiver.
    let mut client = raw_upgrade(&listener, &[]).await.expect("upgrade");
    let (mut server_tx, mut server_rx) = accepted_ends(accept(&mut listener).await);

    client
        .send(Message::Binary(vec![1u8; LIMIT].into()))
        .await
        .expect("exactly the cap");
    match tokio::time::timeout(common::PATIENCE, server_rx.recv()).await {
        Ok(Ok(LinkRead::Frame(Frame::Binary(bytes)))) => assert_eq!(bytes.len(), LIMIT),
        other => panic!("exactly the cap did not cross: {other:?}"),
    }
    client
        .send(Message::Binary(vec![1u8; LIMIT + 1].into()))
        .await
        .expect("the sending side is not blocked");
    let over = tokio::time::timeout(common::PATIENCE, server_rx.recv())
        .await
        .expect("read cap")
        .expect_err("accepted cap + 1");
    assert!(over.message.contains("too long"), "{over}");

    // Outgoing side — a frame over the same value fails as a value before reaching the socket.
    let refused = server_tx.send(Frame::Binary(vec![0u8; LIMIT + 1])).await;
    assert!(refused.is_err(), "the receiving end emitted cap + 1");
}

#[tokio::test]
async fn a_message_over_the_limit_breaks_the_link_into_the_dialer() {
    // ★The receiving side is tungstenite defaults★ — our receiving end blocks a frame over the cap itself.
    let raw = TcpListener::bind(common::loopback()).await.expect("bind");
    let addr = Address::new(format!("ws://{}/", raw.local_addr().expect("local_addr")));
    let server = tokio::spawn(async move {
        let (tcp, _) = raw.accept().await.expect("TCP accept");
        let mut ws = tokio_tungstenite::accept_async(tcp).await.expect("upgrade");
        ws.send(Message::Binary(vec![2u8; LIMIT].into()))
            .await
            .expect("exactly the cap");
        ws.send(Message::Binary(vec![2u8; LIMIT + 1].into()))
            .await
            .expect("the sending side is not blocked");
        // Held until the dialing side cuts — released first, we could not tell whether the cut was due to size.
        while ws.next().await.is_some() {}
    });

    let (_tx, mut rx) = tokio::time::timeout(common::PATIENCE, WsDialer::new(LIMIT).dial(&addr))
        .await
        .expect("dial cap")
        .expect("dial");
    match tokio::time::timeout(common::PATIENCE, rx.recv()).await {
        Ok(Ok(LinkRead::Frame(Frame::Binary(bytes)))) => assert_eq!(bytes.len(), LIMIT),
        other => panic!("exactly the cap did not cross: {other:?}"),
    }
    let over = tokio::time::timeout(common::PATIENCE, rx.recv())
        .await
        .expect("read cap")
        .expect_err("accepted cap + 1");
    assert!(over.message.contains("too long"), "{over}");
    server.abort();
}

// ── Upgrade permits · ready queue (§5 14 · §3-1) ──

/// Returns the dialed link while holding it — if released, that link in the receiving side's ready queue is cut soon too.
async fn held_dial(dialer: &WsDialer, addr: &Address) -> (Box<dyn LinkTx>, Box<dyn LinkRx>) {
    tokio::time::timeout(common::PATIENCE, dialer.dial(addr))
        .await
        .expect("dial cap")
        .expect("dial")
}

async fn dial(dialer: &WsDialer, addr: &Address) -> Result<(), DialFailure> {
    tokio::time::timeout(common::PATIENCE, dialer.dial(addr))
        .await
        .expect("dial cap")
        .map(drop)
}

#[tokio::test]
async fn full_upgrade_slots_close_new_tcp_at_once_and_a_failed_peer_frees_its_slot() {
    let (mut listener, mut refusals) = bind(AcceptPolicy {
        max_pending_upgrades: 2,
        // The silent peer must not be caught by the deadline during the test — the permit must be released by the peer's cut, not the deadline.
        upgrade_timeout: Duration::from_secs(60),
        ..common::accept_policy()
    });
    let at = listener.local_addr();
    let first = TcpStream::connect(at).await.expect("TCP 1");
    let _second = TcpStream::connect(at).await.expect("TCP 2");
    wait_until("two permits taken", || listener.permits_in_use() == 2).await;

    let mut third = TcpStream::connect(at).await.expect("TCP 3");
    closed_by_peer(&mut third, "the third that came when there was no permit").await;
    let report = next_refusal(&mut refusals, "permit overflow").await;
    assert_eq!(report.kind, RefusalKind::SlotsFull);
    assert_eq!(
        report.remote.as_deref(),
        Some(third.local_addr().unwrap().to_string().as_str()),
        "must close the **new one** that overflowed"
    );
    assert_eq!(
        listener.permits_in_use(),
        2,
        "the existing peer's permit wobbled"
    );

    // A peer that cuts during the upgrade releases its permit right away.
    drop(first);
    assert_eq!(
        next_refusal(&mut refusals, "disconnect during upgrade")
            .await
            .kind,
        RefusalKind::UpgradeFailed
    );
    wait_until("the disconnected peer releases the permit", || {
        listener.permits_in_use() == 1
    })
    .await;
    let dialer = common::dialer();
    let addr = listener.dial_address();
    let (accepted, dialed) = tokio::time::timeout(common::PATIENCE, async {
        tokio::join!(listener.accept(), dialer.dial(&addr))
    })
    .await
    .expect("accept cap");
    accepted.expect("accept with an empty permit");
    dialed.expect("dial with an empty permit");
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
    wait_until("two in the ready queue", || listener.queued() == 2).await;
    assert_eq!(listener.permits_in_use(), 2);
    assert!(
        dial(&dialer, &addr).await.is_err(),
        "the ready queue filled beyond the permits"
    );
    assert_eq!(
        next_refusal(&mut refusals, "permit overflow").await.kind,
        RefusalKind::SlotsFull
    );

    // ★Even taken away, the permit is held until the link is put on the registry★ — released only once the receiving side's next stage holds it (TRD 3 §5 14).
    let taken = accept(&mut listener).await;
    assert_eq!(listener.queued(), 1);
    assert!(
        dial(&dialer, &addr).await.is_err(),
        "the taken link released the permit"
    );
    assert_eq!(
        next_refusal(&mut refusals, "permit overflow").await.kind,
        RefusalKind::SlotsFull
    );

    drop(taken);
    let _third = held_dial(&dialer, &addr).await;
    wait_until("two in the ready queue again", || listener.queued() == 2).await;
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
    wait_until("two dropped reports", || refusals.dropped() == 2).await;
    // ★Accepting stands even when reports are blocked★.
    raw_upgrade(&listener, &[]).await.expect("upgrade");
    accept(&mut listener).await;

    assert_eq!(
        refusals.try_recv().map(|r| r.kind),
        Some(RefusalKind::OriginRejected)
    );
    assert!(
        refusals.try_recv().is_none(),
        "more remained than the queue slots"
    );
}

// ── Putting an accepted link on the registry (§3-1 ① — end to end) ──

/// A vocabulary both sides speak — `TestWire` is the dialing side's, so the receiving side could not read requests.
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
        "{what} did not enter the operating phase (now {:?})",
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
                    .expect("send answer");
                return body.clone();
            }
        }
        panic!("the inbox ended before reading the question");
    };
    let (reply, asked) =
        tokio::time::timeout(common::PATIENCE, async { tokio::join!(ask, answer) })
            .await
            .expect("round trip cap");
    assert_eq!(asked, body, "the question did not cross as is");
    assert_eq!(
        reply.expect("answer"),
        Msg::Answer {
            tag,
            body: format!("re: {body}")
        }
    );
}

#[tokio::test]
async fn an_accepted_socket_attaches_to_a_registry_and_requests_flow_both_ways() {
    // ★The listener and the receiving registry use one clock★ — `upgraded_at` and the auth deadline must share it (`WsListener::bind`).
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
    let remote = accepted.info().remote.clone().expect("remote address");
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

    wait_live(&daemon, "dialing side").await;
    wait_live(&client, "receiving side").await;
    // Entering the operating phase releases the upgrade permit.
    wait_until("the accepted peer releases the permit", || {
        listener.permits_in_use() == 0
    })
    .await;

    ask_across(&daemon, &client, &mut client_inbox, 1, "from the dialer").await;
    ask_across(&client, &daemon, &mut daemon_inbox, 2, "from the listener").await;
    assert_eq!(client.peer_state(), PeerState::Live);
}
