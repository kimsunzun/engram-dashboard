//! Real socket ⑤ — the iroh adapter between real endpoints in one process (ADR-0388).
//!
//! ★Only what memory cannot measure lives here★ — the header, dial-address and close-code tables are unit tests in `src/iroh.rs`. Here: does a close code
//! survive a real QUIC `CONNECTION_CLOSE`, does iroh's TLS really bind the dialed key, is the accept gate called before any stream, does a silent or broken peer
//! end where the file header says, and does a peer that stops answering end as a read error once QUIC gives up on it. ★Every endpoint binds 127.0.0.1 with
//! relay, address lookup and mDNS off★ — nothing leaves the machine. Raw-protocol stimuli (a silent dialer, broken frames) are built with `iroh` directly,
//! because our dialer cannot produce them.
#![cfg(all(feature = "iroh", feature = "test-support"))]

// This suite uses only the registry and wait helpers of the shared scaffolding.
#[allow(dead_code)]
mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use engram_dashboard_transport::iroh::{
    dial_address, public_key, Helper, IrohAcceptPolicy, IrohConfig, IrohEndpoint, IrohListener,
    FIN_CLOSE_GRACE, PREAMBLE, QUIC_IDLE_TIMEOUT,
};
use engram_dashboard_transport::policy::MIN_BACKOFF;
use engram_dashboard_transport::testing::{
    accepted_ends, ImmediateHandshake, ListeningHandshake, TestIn, TestOut,
};
use engram_dashboard_transport::{
    Accepted, Address, Close, CloseCode, ConnectFailure, DialFailure, Dialer, DisconnectCause,
    Frame, GaveUpReason, Handshake, HandshakeStep, LinkError, LinkRead, LinkRx, LinkTx, Listener,
    PeerId, PeerState, Policy, ReconnectPolicy, RejectKind, SystemClock, TransportEvent,
};

const ALPN: &[u8] = b"engram/peer/1";

/// The PeerWire frame cap (512 KiB: a 64 KiB body under worst-case JSON escaping, plus headroom) — the value the peer link will use.
const MAX_PAYLOAD: usize = 512 << 10;

/// A stream-accept window no test reaches — for suites that do not measure it.
const LONG: Duration = Duration::from_secs(60);

type Ends = (Box<dyn LinkTx>, Box<dyn LinkRx>);

fn config(seed: u8) -> IrohConfig {
    IrohConfig {
        secret_key: [seed; 32],
        alpn: ALPN.to_vec(),
        listen_port: 0,
        loopback_only: true,
        relay: Helper::Disabled,
        address_lookup: Helper::Disabled,
        mdns: false,
        max_payload: MAX_PAYLOAD,
    }
}

/// An accept policy that counts its calls and answers `verdict`.
fn gate(
    verdict: bool,
    calls: &Arc<AtomicUsize>,
    timeout: Duration,
    max_pending: usize,
) -> IrohAcceptPolicy {
    let calls = calls.clone();
    IrohAcceptPolicy {
        accept_if: Arc::new(move |_key: &[u8; 32]| {
            calls.fetch_add(1, Ordering::AcqRel);
            verdict
        }),
        handshake_timeout: common::PATIENCE,
        stream_accept_timeout: timeout,
        max_pending,
    }
}

fn open_gate() -> IrohAcceptPolicy {
    gate(true, &Arc::new(AtomicUsize::new(0)), LONG, 8)
}

async fn node_with(config: IrohConfig, accept: IrohAcceptPolicy) -> (IrohEndpoint, IrohListener) {
    IrohEndpoint::bind(config, accept, Arc::new(SystemClock))
        .await
        .expect("bind")
}

async fn node(seed: u8, accept: IrohAcceptPolicy) -> (IrohEndpoint, IrohListener) {
    node_with(config(seed), accept).await
}

fn address_of(node: &IrohEndpoint) -> Address {
    dial_address(&node.id(), &node.bound_addrs())
}

async fn dial(from: &IrohEndpoint, to: &Address) -> Result<Ends, DialFailure> {
    tokio::time::timeout(common::PATIENCE, from.dialer().dial(to))
        .await
        .expect("dial cap")
}

async fn accept(listener: &mut IrohListener) -> Accepted {
    tokio::time::timeout(common::PATIENCE, listener.accept())
        .await
        .expect("accept cap")
        .expect("accept")
}

async fn recv(rx: &mut Box<dyn LinkRx>) -> Result<LinkRead, LinkError> {
    tokio::time::timeout(common::PATIENCE, rx.recv())
        .await
        .expect("read cap")
}

async fn recv_text(rx: &mut Box<dyn LinkRx>) -> String {
    match recv(rx).await {
        Ok(LinkRead::Frame(Frame::Text(text))) => text,
        other => panic!("expected a text frame, got {other:?}"),
    }
}

/// Re-checks briefly until the condition holds — only for counts with no notification (permits · ready queue). Panics: [`common::PATIENCE`] exceeded.
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

/// The budget-exhaustion policy of `ws_reject.rs` — classification and budget only; the schedule is measured in memory.
fn spends_budget_without_waiting() -> Policy {
    Policy {
        reconnect: ReconnectPolicy {
            max_attempts: 1,
            base: MIN_BACKOFF,
            cap: MIN_BACKOFF,
            multiplier: 1.0,
            jitter: 0.0,
        },
        ..Policy::default()
    }
}

/// Accepts every link and runs `handle` on its ends, counting links.
fn serve(
    mut listener: IrohListener,
    accepted: &Arc<AtomicUsize>,
    handle: impl Fn(Ends) -> futures_util::future::BoxFuture<'static, ()> + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    let accepted = accepted.clone();
    tokio::spawn(async move {
        while let Ok(link) = listener.accept().await {
            accepted.fetch_add(1, Ordering::AcqRel);
            handle(accepted_ends(link)).await;
        }
    })
}

// ── Raw iroh — stimuli our dialer cannot produce ──

async fn raw_endpoint(seed: u8) -> iroh::Endpoint {
    iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .expect("bind address")
        .secret_key(iroh::SecretKey::from_bytes(&[seed; 32]))
        .bind()
        .await
        .expect("raw bind")
}

async fn raw_connect(raw: &iroh::Endpoint, to: &IrohEndpoint) -> iroh::endpoint::Connection {
    let mut addr = iroh::EndpointAddr::new(iroh::PublicKey::from_bytes(&to.id()).expect("key"));
    for one in to.bound_addrs() {
        addr = addr.with_ip_addr(one);
    }
    tokio::time::timeout(common::PATIENCE, raw.connect(addr, ALPN))
        .await
        .expect("raw connect cap")
        .expect("raw connect")
}

/// The application close code that ended `conn`. Panics: no close within [`common::PATIENCE`] · a close that is not an application close.
async fn closed_with(conn: &iroh::endpoint::Connection, what: &str) -> u64 {
    match tokio::time::timeout(common::PATIENCE, conn.closed()).await {
        Ok(iroh::endpoint::ConnectionError::ApplicationClosed(close)) => {
            close.error_code.into_inner()
        }
        Ok(other) => panic!("{what}: ended without an application close: {other}"),
        Err(_) => panic!("{what}: not closed within {:?}", common::PATIENCE),
    }
}

// ── Accept + attach ──

#[tokio::test]
async fn an_accepted_link_attaches_and_frames_cross_both_ways() {
    let (a, _a_listener) = node(1, open_gate()).await;
    let (b, mut b_listener) = node(2, open_gate()).await;

    let a_registry = common::registry(Arc::new(a.dialer()), Policy::default());
    let (a_peer, mut a_inbox) = a_registry.add(
        PeerId::new("b"),
        address_of(&b),
        Arc::new(ImmediateHandshake),
    );
    let b_registry = common::registry(Arc::new(b.dialer()), Policy::default());
    let (b_peer, mut b_inbox) = b_registry.attach(
        PeerId::new("conn-1"),
        accept(&mut b_listener).await,
        Arc::new(ImmediateHandshake),
    );

    common::wait_state(&a_peer, "dialer live", |s| s == PeerState::Live).await;
    common::wait_state(&b_peer, "acceptor live", |s| s == PeerState::Live).await;
    a_peer
        .notify(TestOut::Notice("from-a".into()))
        .expect("notify a");
    b_peer
        .notify(TestOut::Notice("from-b".into()))
        .expect("notify b");
    let notice = |msg: &engram_dashboard_transport::Incoming<
        engram_dashboard_transport::testing::TestWire,
    >| match msg.as_message() {
        Some(arrival) => match &arrival.msg {
            TestIn::Notice(body) => Some(body.clone()),
            _ => None,
        },
        None => None,
    };
    assert_eq!(
        common::wait_incoming(&mut b_inbox, "a's notice", notice).await,
        "from-a"
    );
    assert_eq!(
        common::wait_incoming(&mut a_inbox, "b's notice", notice).await,
        "from-b"
    );
}

#[tokio::test]
async fn the_public_key_of_a_secret_is_the_id_its_endpoint_binds_with() {
    let (a, _a_listener) = node(7, open_gate()).await;
    assert_eq!(public_key(&[7; 32]), a.id());
    assert_ne!(public_key(&[8; 32]), a.id());
}

#[tokio::test]
async fn an_accepted_link_names_the_dialers_authenticated_key() {
    let (a, _a_listener) = node(3, open_gate()).await;
    let (b, mut b_listener) = node(4, open_gate()).await;
    let _ends = dial(&a, &address_of(&b)).await.expect("dial");
    let accepted = accept(&mut b_listener).await;
    assert_eq!(accepted.info().peer_key, Some(a.id().to_vec()));
    assert_ne!(a.id(), b.id());
    let remote = accepted.info().remote.clone().expect("observed address");
    assert!(remote.starts_with("127.0.0.1:"), "{remote}");
}

#[tokio::test]
async fn dialing_a_key_at_another_nodes_address_fails() {
    let (a, _a_listener) = node(5, open_gate()).await;
    let (b, _b_listener) = node(6, open_gate()).await;
    let c_calls = Arc::new(AtomicUsize::new(0));
    let (c, mut c_listener) = node(7, gate(true, &c_calls, LONG, 8)).await;

    let wrong = dial_address(&b.id(), &c.bound_addrs());
    match dial(&a, &wrong).await {
        Err(DialFailure::Unreachable(err)) => {
            assert!(err.message.contains("iroh connect"), "{err}")
        }
        Err(other) => panic!("not unreachable: {other}"),
        Ok(_) => panic!("a link opened to a node that does not hold the dialed key"),
    }
    assert_eq!(
        c_calls.load(Ordering::Acquire),
        0,
        "C's gate saw a connection that should not exist"
    );
    assert_eq!(c_listener.queued(), 0);
    // The right key at the same address opens — the failure above came from the key, not the address.
    let right = dial_address(&c.id(), &c.bound_addrs());
    let _ends = dial(&a, &right).await.expect("the right key dials");
    let _ = accept(&mut c_listener).await;
}

/// A transparent UDP relay on its own thread that counts the dialer's datagrams — the only way to know a dialer's first packet has left.
struct CountingRelay {
    addr: std::net::SocketAddr,
    from_dialer: Arc<AtomicUsize>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl CountingRelay {
    fn start(server: std::net::SocketAddr) -> Self {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("relay bind");
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .expect("relay timeout");
        let addr = socket.local_addr().expect("relay addr");
        let from_dialer = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread = {
            let (from_dialer, stop) = (from_dialer.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut dialer = None;
                let mut buf = vec![0u8; 65536];
                while !stop.load(Ordering::Acquire) {
                    let Ok((n, from)) = socket.recv_from(&mut buf) else {
                        continue;
                    };
                    if from == server {
                        if let Some(to) = dialer {
                            let _ = socket.send_to(&buf[..n], to);
                        }
                    } else {
                        dialer = Some(from);
                        from_dialer.fetch_add(1, Ordering::AcqRel);
                        let _ = socket.send_to(&buf[..n], server);
                    }
                }
            })
        };
        Self {
            addr,
            from_dialer,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for CountingRelay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// What became of a dialer that froze mid-handshake ([`after_a_frozen_handshake`]).
#[derive(Debug, PartialEq, Eq)]
enum FrozenHandshake {
    /// The server finished its side and called `accept_if`.
    Admitted,
    /// The dialer's connect failed, or its connection was closed before the server admitted it.
    Dropped(String),
    /// Neither within [`common::PATIENCE`] after the thaw.
    Undecided,
}

/// A raw dialer that freezes right after its first packet left — for `freeze`, on its own runtime — then thaws. ★Judged by the server★ (`accept_if` calls):
/// the dialer finishes its own side of the handshake before the server does, so its `connect` succeeding proves nothing.
async fn after_a_frozen_handshake(
    server: &IrohEndpoint,
    server_calls: &Arc<AtomicUsize>,
    seed: u8,
    freeze: Duration,
) -> FrozenHandshake {
    let relay = CountingRelay::start(server.bound_addrs()[0]);
    let (target, relay_addr) = (server.id(), relay.addr);
    let sent = relay.from_dialer.clone();
    let calls = server_calls.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async move {
            let raw = raw_endpoint(seed).await;
            let addr = iroh::EndpointAddr::new(iroh::PublicKey::from_bytes(&target).expect("key"))
                .with_ip_addr(relay_addr);
            let connecting = tokio::spawn(async move {
                let connected = raw.connect(addr, ALPN).await;
                (raw, connected)
            });
            let started = Instant::now();
            while sent.load(Ordering::Acquire) == 0 {
                assert!(
                    started.elapsed() < common::PATIENCE,
                    "the first packet never left"
                );
                tokio::task::yield_now().await;
            }
            // ★Blocks this runtime's only thread★ — the dialer neither reads the server's answer nor sends its next flight.
            std::thread::sleep(freeze);
            let decided = tokio::time::timeout(common::PATIENCE, async {
                let (_raw, connected) = connecting.await.expect("connect task");
                let conn = match connected {
                    Ok(conn) => conn,
                    Err(err) => return FrozenHandshake::Dropped(format!("connect: {err}")),
                };
                tokio::select! {
                    lost = conn.closed() => FrozenHandshake::Dropped(format!("closed: {lost}")),
                    () = async {
                        while calls.load(Ordering::Acquire) == 0 {
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                    } => FrozenHandshake::Admitted,
                }
            })
            .await;
            decided.unwrap_or(FrozenHandshake::Undecided)
        })
    })
    .await
    .expect("frozen dialer thread");
    drop(relay);
    outcome
}

/// ★A dialer that stops mid-handshake is dropped at the handshake deadline★ — QUIC itself would wait as long as packets keep coming. The same frozen dialer is
/// admitted by a server whose deadline is longer than the freeze (the control), so the drop comes from the deadline.
#[tokio::test]
async fn a_handshake_that_stalls_past_the_handshake_deadline_is_dropped() {
    let freeze = Duration::from_secs(1);
    let patient_calls = Arc::new(AtomicUsize::new(0));
    let (patient, _patient_listener) = node(58, gate(true, &patient_calls, LONG, 8)).await;
    assert_eq!(
        after_a_frozen_handshake(&patient, &patient_calls, 59, freeze).await,
        FrozenHandshake::Admitted,
        "the control: a handshake frozen within the deadline must complete"
    );

    let strict_calls = Arc::new(AtomicUsize::new(0));
    let strict_gate = IrohAcceptPolicy {
        handshake_timeout: Duration::from_millis(300),
        ..gate(true, &strict_calls, LONG, 8)
    };
    let (strict, strict_listener) = node(60, strict_gate).await;
    let outcome = after_a_frozen_handshake(&strict, &strict_calls, 61, freeze).await;
    assert!(
        matches!(outcome, FrozenHandshake::Dropped(_)),
        "a handshake frozen past the deadline was not dropped: {outcome:?}"
    );
    assert_eq!(strict_calls.load(Ordering::Acquire), 0);
    assert_eq!(strict_listener.queued(), 0);
}

// ── Accept gate ──

#[tokio::test]
async fn a_refused_key_gives_the_dialer_a_terminal_rejection_and_reaches_no_stream() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (a, _a_listener) = node(8, open_gate()).await;
    let (b, b_listener) = node(9, gate(false, &calls, LONG, 8)).await;

    // ★The default budget★ — a refusal that spent it would dial again after 500 ms and call the gate twice.
    let registry = common::registry(Arc::new(a.dialer()), Policy::default());
    let (peer, mut inbox) = registry.add(
        PeerId::new("b"),
        address_of(&b),
        Arc::new(ListeningHandshake),
    );
    let code = common::wait_incoming(&mut inbox, "Rejected event", |msg| match msg.as_event() {
        Some(TransportEvent::Rejected { code, .. }) => Some(*code),
        _ => None,
    })
    .await;
    assert_eq!(code, CloseCode::HANDSHAKE_REJECTED);
    assert_eq!(
        common::wait_state(&peer, "give up", |s| matches!(s, PeerState::GaveUp(_))).await,
        PeerState::GaveUp(GaveUpReason::Rejected)
    );
    assert_eq!(
        calls.load(Ordering::Acquire),
        1,
        "the refusal spent budget and dialed again"
    );
    assert_eq!(b_listener.queued(), 0);
    assert_eq!(
        b_listener.permits_in_use(),
        0,
        "a refused key took an accept permit"
    );
}

// ★The gate is before any stream★ — this dialer never opens one, yet it is closed with the refusal code long before the stream-accept window.
#[tokio::test]
async fn a_refused_key_is_closed_without_waiting_for_a_stream() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (b, b_listener) = node(10, gate(false, &calls, LONG, 8)).await;
    let raw = raw_endpoint(11).await;
    let conn = raw_connect(&raw, &b).await;
    let started = Instant::now();
    assert_eq!(
        closed_with(&conn, "refused silent dialer").await,
        u64::from(CloseCode::HANDSHAKE_REJECTED.0)
    );
    assert!(
        started.elapsed() < LONG / 2,
        "closed only at {:?}",
        started.elapsed()
    );
    assert_eq!(calls.load(Ordering::Acquire), 1);
    assert_eq!(b_listener.permits_in_use(), 0);
}

#[tokio::test]
async fn a_silent_dialer_is_cut_at_the_stream_accept_deadline_and_frees_its_permit() {
    let window = Duration::from_millis(300);
    let (b, mut b_listener) = node(12, gate(true, &Arc::new(AtomicUsize::new(0)), window, 1)).await;
    let raw = raw_endpoint(13).await;
    // ★The acceptor's window starts after the handshake this call begins★, so the cut cannot come earlier than `window` from here.
    let dialed = Instant::now();
    let silent = raw_connect(&raw, &b).await;
    wait_until("the silent dialer takes the only permit", || {
        b_listener.permits_in_use() == 1
    })
    .await;
    assert_eq!(
        closed_with(&silent, "silent dialer").await,
        u64::from(CloseCode::PROTOCOL_ERROR.0)
    );
    assert!(
        dialed.elapsed() >= window,
        "cut after {:?}, before the window",
        dialed.elapsed()
    );
    wait_until("the permit returns", || b_listener.permits_in_use() == 0).await;
    assert_eq!(b_listener.queued(), 0);

    // ★The freed permit is the only one★ — a real dial now goes through.
    let (a, _a_listener) = node(14, open_gate()).await;
    let _ends = dial(&a, &address_of(&b)).await.expect("dial");
    let accepted = accept(&mut b_listener).await;
    assert_eq!(accepted.info().peer_key, Some(a.id().to_vec()));
}

// ── Close codes across a real QUIC close ──

#[tokio::test]
async fn a_terminal_reject_code_crosses_and_spends_no_budget() {
    let (a, _a_listener) = node(15, open_gate()).await;
    let (b, b_listener) = node(16, open_gate()).await;
    let accepted = Arc::new(AtomicUsize::new(0));
    let server = serve(b_listener, &accepted, |(mut tx, rx)| {
        Box::pin(async move {
            tx.close(Close::new(CloseCode::HANDSHAKE_REJECTED, "version 1 != 2"))
                .await;
            drop((tx, rx));
        })
    });
    let registry = common::registry(Arc::new(a.dialer()), Policy::default());
    let (peer, mut inbox) = registry.add(
        PeerId::new("b"),
        address_of(&b),
        Arc::new(ListeningHandshake),
    );
    let (code, reason) =
        common::wait_incoming(&mut inbox, "Rejected event", |msg| match msg.as_event() {
            Some(TransportEvent::Rejected { code, reason, .. }) => Some((*code, reason.clone())),
            _ => None,
        })
        .await;
    assert_eq!(
        (code, reason.as_str()),
        (CloseCode::HANDSHAKE_REJECTED, "version 1 != 2")
    );
    assert_eq!(
        common::wait_state(&peer, "give up", |s| matches!(s, PeerState::GaveUp(_))).await,
        PeerState::GaveUp(GaveUpReason::Rejected)
    );
    assert_eq!(accepted.load(Ordering::Acquire), 1);
    server.abort();
}

#[tokio::test]
async fn a_retryable_reject_code_crosses_and_spends_budget() {
    let (a, _a_listener) = node(17, open_gate()).await;
    let (b, b_listener) = node(18, open_gate()).await;
    let accepted = Arc::new(AtomicUsize::new(0));
    let server = serve(b_listener, &accepted, |(mut tx, rx)| {
        Box::pin(async move {
            tx.close(Close::new(CloseCode::HANDSHAKE_RETRYABLE, "stale"))
                .await;
            drop((tx, rx));
        })
    });
    let registry = common::registry(Arc::new(a.dialer()), spends_budget_without_waiting());
    let (peer, mut inbox) = registry.add(
        PeerId::new("b"),
        address_of(&b),
        Arc::new(ListeningHandshake),
    );
    let cause = common::wait_incoming(&mut inbox, "ConnectFailed event", |msg| {
        match msg.as_event() {
            Some(TransportEvent::ConnectFailed { cause, .. }) => Some(cause.clone()),
            Some(TransportEvent::Rejected { code, .. }) => {
                panic!("a retryable code read as terminal: {code:?}")
            }
            _ => None,
        }
    })
    .await;
    assert_eq!(
        cause,
        ConnectFailure::Rejected {
            kind: RejectKind::Retryable,
            code: CloseCode::HANDSHAKE_RETRYABLE,
            reason: "stale".into(),
        }
    );
    assert_eq!(
        common::wait_state(&peer, "give up", |s| matches!(s, PeerState::GaveUp(_))).await,
        PeerState::GaveUp(GaveUpReason::BudgetExhausted)
    );
    assert!(accepted.load(Ordering::Acquire) > 1, "no retry");
    server.abort();
}

// ★The implicit close★ — an acceptor that just drops the link sends QUIC code 0 (the last handle closing); the dialer must read it as "could not connect".
#[tokio::test]
async fn an_implicit_close_is_not_a_rejection() {
    let (a, _a_listener) = node(19, open_gate()).await;
    let (b, b_listener) = node(20, open_gate()).await;
    let accepted = Arc::new(AtomicUsize::new(0));
    let server = serve(b_listener, &accepted, |ends| {
        Box::pin(async move {
            drop(ends);
        })
    });
    let registry = common::registry(Arc::new(a.dialer()), spends_budget_without_waiting());
    let (peer, mut inbox) = registry.add(
        PeerId::new("b"),
        address_of(&b),
        Arc::new(ListeningHandshake),
    );
    let cause = common::wait_incoming(&mut inbox, "ConnectFailed event", |msg| {
        match msg.as_event() {
            Some(TransportEvent::ConnectFailed { cause, .. }) => Some(cause.clone()),
            Some(TransportEvent::Rejected { code, .. }) => {
                panic!("an implicit close read as a rejection: {code:?}")
            }
            _ => None,
        }
    })
    .await;
    // ★This message is the `LinkRead::Closed(None)` arm of `handshake_once`★ — a read error would carry the adapter's own text.
    assert_eq!(
        cause,
        ConnectFailure::Unreachable(LinkError::new("peer closed during handshake"))
    );
    assert_eq!(
        common::wait_state(&peer, "give up", |s| matches!(s, PeerState::GaveUp(_))).await,
        PeerState::GaveUp(GaveUpReason::BudgetExhausted)
    );
    server.abort();
}

#[tokio::test]
async fn a_peer_sent_code_zero_reads_as_a_close_without_a_code() {
    let (a, _a_listener) = node(21, open_gate()).await;
    let (b, mut b_listener) = node(22, open_gate()).await;
    let (_tx, mut rx) = dial(&a, &address_of(&b)).await.expect("dial");
    let (mut b_tx, _b_rx) = accepted_ends(accept(&mut b_listener).await);
    b_tx.close(Close::new(CloseCode(0), "bye")).await;
    assert!(matches!(recv(&mut rx).await, Ok(LinkRead::Closed(None))));
}

#[tokio::test]
async fn a_spoken_close_carries_its_code_and_reason_to_the_dialer() {
    let (a, _a_listener) = node(23, open_gate()).await;
    let (b, mut b_listener) = node(24, open_gate()).await;
    let (_tx, mut rx) = dial(&a, &address_of(&b)).await.expect("dial");
    let (mut b_tx, _b_rx) = accepted_ends(accept(&mut b_listener).await);
    b_tx.close(Close::new(CloseCode::GOING_AWAY, "shutting down"))
        .await;
    match recv(&mut rx).await {
        Ok(LinkRead::Closed(Some(close))) => {
            assert_eq!(close.code, CloseCode::GOING_AWAY);
            assert_eq!(close.reason, "shutting down");
        }
        other => panic!("{other:?}"),
    }
}

// ── Liveness ──

/// ★A peer that stops answering★: its endpoint runs on its own runtime, whose only thread is then blocked — the process is alive, its QUIC stack silent.
/// ★The window is QUIC's connection idle timeout★ (`QUIC_IDLE_TIMEOUT`, measured 30.0 s — `src/iroh.rs` 「link properties」), so this waits that long. The supervisor's
/// own ping / silence cut are off with values `Policy::check` refuses while they are on (write deadline over the ping interval) and a silence cut far shorter
/// than iroh's — had they still armed, the link would end as `Idle` within 2 s.
#[tokio::test]
async fn a_peer_that_stops_answering_ends_the_link_as_a_read_error_at_the_connection_idle_timeout()
{
    let (a, mut a_listener) = node(25, open_gate()).await;
    let a_address = address_of(&a);

    let (dialed_tx, dialed_rx) = std::sync::mpsc::channel::<()>();
    let (frozen_tx, frozen_rx) = std::sync::mpsc::channel::<()>();
    let (thaw_tx, thaw_rx) = std::sync::mpsc::channel::<()>();
    let frozen = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async move {
            let (b, _b_listener) = node(26, open_gate()).await;
            let (tx, mut rx) = dial(&b, &a_address).await.expect("dial");
            dialed_tx.send(()).expect("signal");
            // ★Freeze only after A's first frame arrived★ — a dial returns once the preamble is buffered, not sent; frozen earlier, A never sees the stream.
            assert!(matches!(recv(&mut rx).await, Ok(LinkRead::Frame(_))));
            frozen_tx.send(()).expect("signal");
            // ★Blocks this runtime's only thread★ — B's QUIC stack stops reading and sending.
            thaw_rx.recv().expect("thaw");
            drop((tx, rx));
            b.close().await;
        });
    });
    tokio::task::spawn_blocking(move || dialed_rx.recv().expect("dialed"))
        .await
        .expect("join");

    let policy = Policy {
        supervisor_keepalive: false,
        ping_interval: Duration::from_secs(1),
        idle_timeout: Duration::from_secs(2),
        write_deadline: Duration::from_secs(5),
        ..Policy::default()
    };
    assert_eq!(policy.check(), Ok(()));
    let registry = common::registry(Arc::new(a.dialer()), policy);
    let (peer, mut inbox) = registry.attach(
        PeerId::new("conn-1"),
        accept(&mut a_listener).await,
        Arc::new(ImmediateHandshake),
    );
    common::wait_state(&peer, "live", |s| s == PeerState::Live).await;
    peer.notify(TestOut::Notice("freeze".into()))
        .expect("notify");
    tokio::task::spawn_blocking(move || frozen_rx.recv().expect("frozen"))
        .await
        .expect("join");
    let started = Instant::now();
    let cause = tokio::time::timeout(Duration::from_secs(90), async {
        while let Some(msg) = inbox.recv().await {
            if let Some(TransportEvent::Disconnected { cause, .. }) = msg.as_event() {
                return Some(*cause);
            }
        }
        None
    })
    .await
    .expect("the link did not end within 90 s")
    .expect("the inbox ended without a disconnect");
    let waited = started.elapsed();

    thaw_tx.send(()).expect("thaw");
    frozen.join().expect("frozen peer thread");

    assert_eq!(
        cause,
        DisconnectCause::ReadFailed,
        "not a read error (after {waited:?})"
    );
    // ★Above iroh's 15 s path idle and near the 30 s connection idle timeout★ — tells the two apart.
    assert!(
        waited >= QUIC_IDLE_TIMEOUT * 3 / 4,
        "ended at {waited:?} — before the connection idle timeout, so not the idle-out"
    );
    assert!(
        waited <= QUIC_IDLE_TIMEOUT + common::PATIENCE,
        "ended at {waited:?} — long after the connection idle timeout"
    );
}

/// ★The supervisor's ping is still on by default★ (only the peer link turns it off), and over this adapter it arrives as a real keepalive frame.
#[tokio::test]
async fn the_supervisor_ping_stays_on_by_default_and_arrives_as_a_keepalive_frame() {
    let (a, _a_listener) = node(27, open_gate()).await;
    let (b, mut b_listener) = node(28, open_gate()).await;
    let policy = Policy {
        ping_interval: Duration::from_millis(200),
        idle_timeout: Duration::from_secs(30),
        write_deadline: Duration::from_millis(100),
        ..Policy::default()
    };
    assert!(policy.supervisor_keepalive);
    let registry = common::registry(Arc::new(a.dialer()), policy);
    let (_peer, _inbox) = registry.add(
        PeerId::new("b"),
        address_of(&b),
        Arc::new(ImmediateHandshake),
    );
    let (_b_tx, mut b_rx) = accepted_ends(accept(&mut b_listener).await);
    assert!(matches!(
        recv(&mut b_rx).await,
        Ok(LinkRead::Frame(Frame::Keepalive))
    ));
}

// ── Links ──

#[tokio::test]
async fn concurrent_links_between_one_pair_stay_distinct() {
    let (a, mut a_listener) = node(29, open_gate()).await;
    let (b, mut b_listener) = node(30, open_gate()).await;
    // Two links A → B and one B → A, all open at once.
    let (mut l1_tx, mut l1_rx) = dial(&a, &address_of(&b)).await.expect("dial 1");
    let (mut l1b_tx, mut l1b_rx) = accepted_ends(accept(&mut b_listener).await);
    let (mut l2_tx, mut l2_rx) = dial(&a, &address_of(&b)).await.expect("dial 2");
    let (mut l2b_tx, mut l2b_rx) = accepted_ends(accept(&mut b_listener).await);
    let (mut l3_tx, mut l3_rx) = dial(&b, &address_of(&a)).await.expect("dial 3");
    let l3a = accept(&mut a_listener).await;
    assert_eq!(l3a.info().peer_key, Some(b.id().to_vec()));
    let (mut l3a_tx, mut l3a_rx) = accepted_ends(l3a);

    for (tx, text) in [
        (&mut l2_tx, "l2 a->b"),
        (&mut l3_tx, "l3 b->a"),
        (&mut l1_tx, "l1 a->b"),
        (&mut l2b_tx, "l2 b->a"),
        (&mut l1b_tx, "l1 b->a"),
        (&mut l3a_tx, "l3 a->b"),
    ] {
        tx.send(Frame::Text(text.into())).await.expect("send");
    }
    assert_eq!(recv_text(&mut l1b_rx).await, "l1 a->b");
    assert_eq!(recv_text(&mut l2b_rx).await, "l2 a->b");
    assert_eq!(recv_text(&mut l3a_rx).await, "l3 b->a");
    assert_eq!(recv_text(&mut l1_rx).await, "l1 b->a");
    assert_eq!(recv_text(&mut l2_rx).await, "l2 b->a");
    assert_eq!(recv_text(&mut l3_rx).await, "l3 a->b");
}

#[tokio::test]
async fn a_frame_at_the_cap_crosses_and_one_over_it_fails_as_a_value() {
    let (a, _a_listener) = node(31, open_gate()).await;
    let (b, mut b_listener) = node(32, open_gate()).await;
    let (mut tx, _rx) = dial(&a, &address_of(&b)).await.expect("dial");
    let (_b_tx, mut b_rx) = accepted_ends(accept(&mut b_listener).await);

    let err = tx
        .send(Frame::Binary(vec![1; MAX_PAYLOAD + 1]))
        .await
        .expect_err("over the cap");
    assert!(err.message.contains("cap"), "{err}");
    tx.send(Frame::Binary(vec![7; MAX_PAYLOAD]))
        .await
        .expect("at the cap");
    match recv(&mut b_rx).await {
        Ok(LinkRead::Frame(Frame::Binary(bytes))) => {
            assert_eq!(bytes.len(), MAX_PAYLOAD);
            assert!(bytes.iter().all(|b| *b == 7));
        }
        other => panic!("{other:?}"),
    }
}

/// Each stimulus on a fresh raw connection after a correct preamble: the acceptor's read fails and the dialer is closed with `PROTOCOL_ERROR`.
#[tokio::test]
async fn malformed_and_oversized_frames_close_with_protocol_error() {
    let (b, mut b_listener) = node(33, open_gate()).await;
    let raw = raw_endpoint(34).await;
    let over = u32::try_from(MAX_PAYLOAD + 1).expect("fits");
    let mut oversized = vec![2u8];
    oversized.extend_from_slice(&over.to_be_bytes());
    let cases: Vec<(&str, Vec<u8>, Ending)> = vec![
        ("unknown kind", vec![0xff, 0, 0, 0, 0], Ending::Open),
        (
            "keepalive with a payload",
            vec![3, 0, 0, 0, 1, 0],
            Ending::Open,
        ),
        (
            "text that is not UTF-8",
            vec![1, 0, 0, 0, 2, 0xff, 0xfe],
            Ending::Open,
        ),
        ("over the 512 KiB cap", oversized, Ending::Open),
        ("stream ended inside a frame", vec![1, 0], Ending::Finish),
        ("stream reset", vec![1, 0, 0, 0, 2], Ending::Reset),
    ];
    for (what, bytes, ending) in cases {
        let conn = raw_connect(&raw, &b).await;
        let (mut send, _recv) = conn.open_bi().await.expect("open");
        send.write_all(&[PREAMBLE]).await.expect("preamble");
        // ★Accepted before the stimulus★ — a reset discards unsent data, so a preamble still buffered would be lost with it.
        let (_b_tx, mut b_rx) = accepted_ends(accept(&mut b_listener).await);
        send.write_all(&bytes).await.expect("stimulus");
        ending.apply(&mut send);
        assert!(
            recv(&mut b_rx).await.is_err(),
            "{what}: the acceptor read did not fail"
        );
        assert_eq!(
            closed_with(&conn, what).await,
            u64::from(CloseCode::PROTOCOL_ERROR.0),
            "{what}"
        );
    }
}

/// How a raw stimulus leaves its stream.
#[derive(Clone, Copy)]
enum Ending {
    Open,
    Finish,
    Reset,
}

impl Ending {
    fn apply(self, send: &mut iroh::endpoint::SendStream) {
        match self {
            Ending::Open => {}
            Ending::Finish => send.finish().expect("finish"),
            Ending::Reset => send
                .reset(iroh::endpoint::VarInt::from_u32(7))
                .expect("reset"),
        }
    }
}

/// ★A FIN at a frame boundary means "closing" — but only with a close after it★; a peer that ends its stream and never closes is cut after the grace.
#[tokio::test]
async fn a_stream_end_at_a_frame_boundary_without_a_close_is_cut_after_the_grace() {
    let (b, mut b_listener) = node(43, open_gate()).await;
    let raw = raw_endpoint(44).await;
    let conn = raw_connect(&raw, &b).await;
    let (mut send, _recv) = conn.open_bi().await.expect("open");
    send.write_all(&[PREAMBLE, 1, 0, 0, 0, 2, b'h', b'i'])
        .await
        .expect("preamble and one frame");
    Ending::Finish.apply(&mut send);
    let (_b_tx, mut b_rx) = accepted_ends(accept(&mut b_listener).await);
    assert_eq!(recv_text(&mut b_rx).await, "hi");
    // ★The grace starts inside this read★ (when it meets the FIN), so the cut cannot come earlier than the grace from here.
    let reading = Instant::now();
    let read = tokio::time::timeout(FIN_CLOSE_GRACE + common::PATIENCE, b_rx.recv())
        .await
        .expect("read cap");
    assert!(read.is_err(), "{read:?}");
    assert!(
        reading.elapsed() >= FIN_CLOSE_GRACE,
        "cut after {:?}",
        reading.elapsed()
    );
    assert_eq!(
        closed_with(&conn, "FIN without a close").await,
        u64::from(CloseCode::PROTOCOL_ERROR.0)
    );
}

#[tokio::test]
async fn a_stream_that_ends_or_resets_before_the_preamble_is_closed_with_protocol_error() {
    let (b, b_listener) = node(45, open_gate()).await;
    let raw = raw_endpoint(46).await;
    for (what, ending) in [("FIN", Ending::Finish), ("RESET", Ending::Reset)] {
        let conn = raw_connect(&raw, &b).await;
        let (mut send, _recv) = conn.open_bi().await.expect("open");
        ending.apply(&mut send);
        assert_eq!(
            closed_with(&conn, what).await,
            u64::from(CloseCode::PROTOCOL_ERROR.0),
            "{what}"
        );
        assert_eq!(b_listener.queued(), 0, "{what}");
    }
    wait_until("the permits return", || b_listener.permits_in_use() == 0).await;
}

// ── Closing delivers the tail ──

/// ★QUIC drops unsent stream data at `CONNECTION_CLOSE`★ — a burst still in the send buffer when the close is asked for must arrive whole, then the code.
#[tokio::test]
async fn close_after_flush_delivers_every_frame_and_then_the_close_code() {
    const FRAMES: usize = 64;
    let (a, _a_listener) = node(47, open_gate()).await;
    let (b, mut b_listener) = node(48, open_gate()).await;
    let registry = common::registry(Arc::new(a.dialer()), Policy::default());
    let (peer, mut inbox) = registry.add(
        PeerId::new("b"),
        address_of(&b),
        Arc::new(ImmediateHandshake),
    );
    let (_b_tx, mut b_rx) = accepted_ends(accept(&mut b_listener).await);
    common::wait_state(&peer, "live", |s| s == PeerState::Live).await;
    let drain = tokio::spawn(async move { while inbox.recv().await.is_some() {} });

    let pad = "x".repeat(16 << 10);
    for i in 0..FRAMES {
        peer.notify(TestOut::Notice(format!("{i}|{pad}")))
            .expect("notify");
    }
    let flushed = peer.close_after_flush(CloseCode(CloseCode::CONSUMER_BASE), "bye");
    for i in 0..FRAMES {
        let text = recv_text(&mut b_rx).await;
        assert!(text.starts_with(&format!("not|{i}|")), "frame {i} missing");
    }
    match recv(&mut b_rx).await {
        Ok(LinkRead::Closed(Some(close))) => {
            assert_eq!(close.code, CloseCode(CloseCode::CONSUMER_BASE));
            assert_eq!(close.reason, "bye");
        }
        other => panic!("{other:?}"),
    }
    tokio::time::timeout(common::PATIENCE, flushed)
        .await
        .expect("flush cap");
    drain.await.expect("drain");
}

/// Rejects at once, with a last frame for the dialer to read.
struct RejectWithLast;

impl Handshake for RejectWithLast {
    fn start(&self) -> HandshakeStep {
        HandshakeStep::Reject {
            kind: RejectKind::Terminal,
            last: Some(Frame::Text("why: version 1 != 2".into())),
            reason: "version mismatch".into(),
        }
    }

    fn on_frame(&self, _frame: &Frame) -> HandshakeStep {
        self.start()
    }
}

#[tokio::test]
async fn a_rejections_last_frame_reaches_the_dialer_before_the_close() {
    let (a, _a_listener) = node(49, open_gate()).await;
    let (b, mut b_listener) = node(50, open_gate()).await;
    let (_tx, mut rx) = dial(&a, &address_of(&b)).await.expect("dial");
    let b_registry = common::registry(Arc::new(b.dialer()), Policy::default());
    let (_peer, mut b_inbox) = b_registry.attach(
        PeerId::new("conn-1"),
        accept(&mut b_listener).await,
        Arc::new(RejectWithLast),
    );
    assert_eq!(recv_text(&mut rx).await, "why: version 1 != 2");
    match recv(&mut rx).await {
        Ok(LinkRead::Closed(Some(close))) => {
            assert_eq!(close.code, CloseCode::HANDSHAKE_REJECTED);
            assert_eq!(close.reason, "version mismatch");
        }
        other => panic!("{other:?}"),
    }
    while b_inbox.recv().await.is_some() {}
}

// ── Other endings ──

#[tokio::test]
async fn a_full_permit_pool_closes_with_going_away_and_the_dialer_spends_budget() {
    let (a, _a_listener) = node(51, open_gate()).await;
    let (b, b_listener) = node(52, gate(true, &Arc::new(AtomicUsize::new(0)), LONG, 1)).await;
    // The only permit stays held by a link left in the ready queue.
    let _held = dial(&a, &address_of(&b)).await.expect("dial");
    wait_until("the queued link holds the permit", || {
        b_listener.queued() == 1
    })
    .await;

    let raw = raw_endpoint(53).await;
    let conn = raw_connect(&raw, &b).await;
    assert_eq!(
        closed_with(&conn, "no permit").await,
        u64::from(CloseCode::GOING_AWAY.0)
    );

    let registry = common::registry(Arc::new(a.dialer()), spends_budget_without_waiting());
    let (peer, mut inbox) = registry.add(
        PeerId::new("b"),
        address_of(&b),
        Arc::new(ListeningHandshake),
    );
    common::wait_incoming(&mut inbox, "GaveUp event", |msg| match msg.as_event() {
        Some(TransportEvent::GaveUp { .. }) => Some(()),
        Some(TransportEvent::Rejected { code, .. }) => {
            panic!("a full permit pool read as a rejection: {code:?}")
        }
        _ => None,
    })
    .await;
    assert_eq!(
        peer.peer_state(),
        PeerState::GaveUp(GaveUpReason::BudgetExhausted)
    );
}

#[tokio::test]
async fn closing_the_endpoint_closes_its_links_and_ends_the_listener_after_its_queue() {
    let (a, _a_listener) = node(54, open_gate()).await;
    let (b, mut b_listener) = node(55, open_gate()).await;
    let (_l1_tx, mut l1_rx) = dial(&a, &address_of(&b)).await.expect("dial 1");
    let (_l1b_tx, mut l1b_rx) = accepted_ends(accept(&mut b_listener).await);
    let _l2 = dial(&a, &address_of(&b)).await.expect("dial 2");
    wait_until("the second link waits in the queue", || {
        b_listener.queued() == 1
    })
    .await;

    tokio::time::timeout(common::PATIENCE, b.close())
        .await
        .expect("close cap");
    assert!(matches!(recv(&mut l1_rx).await, Ok(LinkRead::Closed(None))));
    assert!(
        recv(&mut l1b_rx).await.is_err(),
        "our own end must fail, not read a close"
    );
    let _queued = accept(&mut b_listener).await;
    let ended = tokio::time::timeout(common::PATIENCE, b_listener.accept())
        .await
        .expect("accept cap");
    assert!(
        ended.is_err(),
        "the listener accepted after its endpoint closed"
    );
}

/// ★A write after the peer closed with a code is `Ok`★ — the code reaches the supervisor through the read side (`src/iroh.rs` 「close mapping」).
#[tokio::test]
async fn a_write_after_the_peers_close_succeeds_and_the_reader_keeps_the_code() {
    let (a, _a_listener) = node(56, open_gate()).await;
    let (b, mut b_listener) = node(57, open_gate()).await;
    let (mut tx, mut rx) = dial(&a, &address_of(&b)).await.expect("dial");
    let (mut b_tx, _b_rx) = accepted_ends(accept(&mut b_listener).await);
    b_tx.close(Close::new(CloseCode::HANDSHAKE_REJECTED, "not paired"))
        .await;
    let code = |read: Result<LinkRead, LinkError>| match read {
        Ok(LinkRead::Closed(Some(close))) => close.code,
        other => panic!("{other:?}"),
    };
    // The close has landed once the reader sees it.
    assert_eq!(code(recv(&mut rx).await), CloseCode::HANDSHAKE_REJECTED);
    tx.send(Frame::Text("late".into()))
        .await
        .expect("a write into a peer-closed link");
    assert_eq!(code(recv(&mut rx).await), CloseCode::HANDSHAKE_REJECTED);
}

#[tokio::test]
async fn a_wrong_preamble_is_closed_before_the_link_is_queued() {
    let (b, b_listener) = node(35, open_gate()).await;
    let raw = raw_endpoint(36).await;
    let conn = raw_connect(&raw, &b).await;
    let (mut send, _recv) = conn.open_bi().await.expect("open");
    send.write_all(&[PREAMBLE.wrapping_add(1)])
        .await
        .expect("preamble");
    assert_eq!(
        closed_with(&conn, "wrong preamble").await,
        u64::from(CloseCode::PROTOCOL_ERROR.0)
    );
    assert_eq!(b_listener.queued(), 0);
    wait_until("the permit returns", || b_listener.permits_in_use() == 0).await;
}

#[tokio::test]
async fn an_alpn_mismatch_is_refused_by_the_quic_handshake() {
    let (a, _a_listener) = node_with(
        IrohConfig {
            alpn: b"engram/shell/1".to_vec(),
            ..config(37)
        },
        open_gate(),
    )
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let (b, b_listener) = node(38, gate(true, &calls, LONG, 8)).await;
    match dial(&a, &address_of(&b)).await {
        Err(DialFailure::Unreachable(err)) => {
            assert!(err.message.contains("iroh connect"), "{err}")
        }
        Err(other) => panic!("not unreachable: {other}"),
        Ok(_) => panic!("a link opened across two ALPNs"),
    }
    assert_eq!(
        calls.load(Ordering::Acquire),
        0,
        "the gate saw a connection the handshake should have refused"
    );
    assert_eq!(b_listener.queued(), 0);
}

/// The dialer's first frame names a version; the acceptor rejects another as terminal.
struct Hello(&'static str);

impl Handshake for Hello {
    fn start(&self) -> HandshakeStep {
        HandshakeStep::Send(Frame::Text(self.0.into()))
    }

    fn on_frame(&self, _frame: &Frame) -> HandshakeStep {
        HandshakeStep::Done
    }
}

struct VersionGate(&'static str);

impl Handshake for VersionGate {
    fn start(&self) -> HandshakeStep {
        HandshakeStep::Await
    }

    fn on_frame(&self, frame: &Frame) -> HandshakeStep {
        match frame {
            Frame::Text(text) if text == self.0 => HandshakeStep::Done,
            _ => HandshakeStep::Reject {
                kind: RejectKind::Terminal,
                last: None,
                reason: "version mismatch".into(),
            },
        }
    }
}

#[tokio::test]
async fn a_hello_version_mismatch_is_terminal() {
    let (a, _a_listener) = node(39, open_gate()).await;
    let (b, b_listener) = node(40, open_gate()).await;
    let accepted = Arc::new(AtomicUsize::new(0));
    let b_registry = Arc::new(common::registry(Arc::new(b.dialer()), Policy::default()));
    let server = {
        let accepted = accepted.clone();
        let b_registry = b_registry.clone();
        let mut listener = b_listener;
        tokio::spawn(async move {
            let mut inboxes = Vec::new();
            while let Ok(link) = listener.accept().await {
                let n = accepted.fetch_add(1, Ordering::AcqRel);
                let (_peer, inbox) = b_registry.attach(
                    PeerId::new(format!("conn-{n}")),
                    link,
                    Arc::new(VersionGate("hello v2")),
                );
                inboxes.push(inbox);
            }
        })
    };
    let registry = common::registry(Arc::new(a.dialer()), Policy::default());
    let (peer, mut inbox) = registry.add(
        PeerId::new("b"),
        address_of(&b),
        Arc::new(Hello("hello v1")),
    );
    let (code, reason) =
        common::wait_incoming(&mut inbox, "Rejected event", |msg| match msg.as_event() {
            Some(TransportEvent::Rejected { code, reason, .. }) => Some((*code, reason.clone())),
            _ => None,
        })
        .await;
    assert_eq!(
        (code, reason.as_str()),
        (CloseCode::HANDSHAKE_REJECTED, "version mismatch")
    );
    assert_eq!(
        common::wait_state(&peer, "give up", |s| matches!(s, PeerState::GaveUp(_))).await,
        PeerState::GaveUp(GaveUpReason::Rejected)
    );
    assert_eq!(accepted.load(Ordering::Acquire), 1);
    server.abort();
}

/// ★`accept` keeps losing races★ — a timer arm cancels it again and again while three links land; a version that lost a link on cancel never reaches three.
#[tokio::test]
async fn accept_is_cancel_safe() {
    let (a, _a_listener) = node(41, open_gate()).await;
    let (b, mut b_listener) = node(42, open_gate()).await;
    let target = address_of(&b);
    let dialer = a.dialer();
    let dials = tokio::spawn(async move {
        let mut held = Vec::new();
        for _ in 0..3 {
            held.push(
                dialer
                    .dial(&target)
                    .await
                    .map_err(|e| e.to_string())
                    .expect("dial"),
            );
        }
        held
    });
    let mut got = 0;
    let mut cancelled = 0usize;
    tokio::time::timeout(common::PATIENCE, async {
        while got < 3 {
            tokio::select! {
                link = b_listener.accept() => {
                    link.expect("accept");
                    got += 1;
                }
                () = tokio::time::sleep(Duration::from_micros(200)) => cancelled += 1,
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("accepted {got} of 3 links after {cancelled} cancelled accepts"));
    assert!(
        cancelled > 0,
        "no accept was ever cancelled — the test measured nothing"
    );
    let _held = dials.await.expect("dials");
}
