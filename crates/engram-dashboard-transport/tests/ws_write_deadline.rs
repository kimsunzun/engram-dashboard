//! Real socket ② — when the peer does not read, does the write deadline really cut, and does that write path report failure **as a value**
//! (TRD §9-1 · ADR-0177 decision 8).
//!
//! ★The only property that in principle cannot be measured in memory★ — the harness's `stall_writes` measures **our side's** deadline and queue
//! logic, and what is measured here is the **kernel send buffer** in front of it. Only a real socket can create the state where
//! that buffer fills and a write never finishes.
//!
//! ★Where it measures★ — the link seam (`LinkTx::send` + `with_deadline`) and the supervisor's write path. The first measures whether the adapter
//! really turns backpressure into `Pending`, the second whether that `Pending` and the error the link gave each drop the connection and report **naming the
//! reason**.
//!
//! ## Why this file has this shape
//!
//! - ★The key is pushing frames **repeatedly**★ — making one frame big does not stop it.
//!   The canonical source of the evidence (measured) is the `src/ws.rs` header 「통로 성질」 and the numbers are not copied here. Socket buffer options also
//!   do not change the result, so this file uses `WsDialer` and `WsListener` as they are.
//! - ★Why the supervisor test pushes `ping_interval` and `idle_timeout` out of the window★ — when not pushed
//!   (2s/5s) the source of the pass was **not the flood but the keepalive write at t=4s**, and that deadline expiry
//!   arrived at **the same moment** as `idle_timeout` (5.004s) (measured 2026-09-07). That is, it was a
//!   coin-flip with `Idle`. Pushed far away, the only thing that can produce `WriteDeadline` inside that window is the flood.
//!   ★But that property is not left to two policy-value lines alone★ — the supervisor test directly measures the event arrival time against the wall clock,
//!   so even if those two lines are deleted or [`common::PATIENCE`] is raised, the coin flip does not quietly return.
//! - ★It runs serially★ — libtest runs the tests of one binary concurrently, and this file deliberately has a test that
//!   saturates loopback together with a test that asserts 「어떤 쓰기도 시한에 안 걸렸다」 (no write hit the deadline). If they overlap on the same kernel
//!   stack, someone else's load can turn that assertion red (a real risk on a small runner).
#![cfg(all(feature = "ws", feature = "test-support"))]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, MutexGuard};

use engram_dashboard_transport::link::LinkRead;
use engram_dashboard_transport::testing::{ImmediateHandshake, TestOut};
use engram_dashboard_transport::ws::WsListener;
use engram_dashboard_transport::{
    with_deadline, Dialer, DisconnectCause, Elapsed, Frame, Generation, LinkRx, LinkTx, Listener,
    PeerId, Policy, SystemClock, TransportEvent,
};

/// The frame size pushed at one time.
const FRAME: usize = 1024 * 1024;

/// How many frames to push up to. ★Twenty times the stall point recorded in the `ws.rs` header★ — if all of them go in,
/// the test below fails citing this number. That means the amount the kernel swallows changed, and this is the place to raise it.
const FRAME_BUDGET: usize = 64;

/// The write deadline.
///
/// ★A stalled write **never** finishes unless the peer reads, so a large value does not blur the positive signal★ —
/// what this value guards against is the opposite side, the false positive 「멀쩡한 쓰기가 느려서 걸리는」 (a healthy write is slow and hits it).
const DEADLINE: Duration = Duration::from_secs(2);

/// The cap over the whole flood.
///
/// ★No path actually uses the worst case ([`FRAME_BUDGET`] × [`DEADLINE`] = 128 seconds)★ — against a peer that does not read it hits within a few
/// frames (the measurement in the `ws.rs` header), and against a peer that reads, it is loopback so the whole budget
/// goes out quickly. What this cap catches is the state that is neither of those (a link flowing slowly that hits neither deadline nor budget),
/// and instead of sitting and waiting two-digit minutes on a shared runner it makes that spot loud.
const FLOOD_CAP: Duration = Duration::from_secs(20);

/// Where the supervisor test pushes keepalive out. ★The measurement in the file header is the reason, and the wall-clock assertion below reads this
/// value back as the decision criterion★.
const SUPERVISOR_PING_INTERVAL: Duration = Duration::from_secs(30);

/// Where the supervisor test pushes the silence deadline out. Same reason.
const SUPERVISOR_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// The number of frames to push at once so the supervisor reaches its first stall = the outgoing queue length. It is larger than what the kernel swallows
/// (`ws.rs` header).
const PARKED_BLOBS: usize = 16;

/// A payload **one byte** over the receiving side's cap. The cap itself is legal (the adapter checks with `>`).
///
/// The threshold is the cap this suite gave the dialing side ([`common::MAX_MESSAGE`]).
const OVER_LIMIT: usize = common::MAX_MESSAGE + 1;

/// Runs only one test of this binary at a time — the reason is 「직렬로 돈다」 in the file header.
static SOCKET_LOCK: Mutex<()> = Mutex::const_new(());

/// How many in this file are running right now. ★If the lock is deleted this exceeds 1 and that spot turns red★ — if seriality is left
/// to structure alone, the next session may read the lock as an 「불필요한 장치」 (unnecessary device) and remove it, and nobody would know.
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

struct OneAtATime {
    _lock: MutexGuard<'static, ()>,
}

impl Drop for OneAtATime {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

async fn one_at_a_time() -> OneAtATime {
    let held = OneAtATime {
        _lock: SOCKET_LOCK.lock().await,
    };
    assert_eq!(
        IN_FLIGHT.fetch_add(1, Ordering::AcqRel) + 1,
        1,
        "이 파일의 테스트가 겹쳐 돌았다 — 서로의 loopback 부하가 시한 판정에 섞인다"
    );
    held
}

/// Pushes frames until the deadline hits. Returns the number of the frame that hit, and `None` if it pushed all of [`FRAME_BUDGET`] and
/// none hit.
///
/// Panics: a link error instead of the deadline · exceeding [`FLOOD_CAP`].
async fn flood_until_cut(tx: &mut Box<dyn LinkTx>) -> Option<usize> {
    let flood = async {
        for nth in 0..FRAME_BUDGET {
            match with_deadline(
                &SystemClock,
                DEADLINE,
                tx.send(Frame::Binary(vec![0u8; FRAME])),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(err)) => panic!("{nth}번째에서 시한이 아니라 통로 오류가 났다: {err}"),
                Err(Elapsed) => return Some(nth),
            }
        }
        None
    };
    match tokio::time::timeout(FLOOD_CAP, flood).await {
        Ok(cut) => cut,
        Err(_) => panic!(
            "{FLOOD_CAP:?} 안에 시한도 예산({FRAME_BUDGET}개 × {FRAME} 바이트)도 안 걸렸다 — 통로가 \
             멈추지도 비지도 않고 느리게 흐르고 있다"
        ),
    }
}

/// Both ends of a link that finished the handshake. ★The dialing side and the receiving side wait on each other, so it finishes only when polled together★.
///
/// ★Why there is a cap★ — a regression where TCP connected but the WS upgrade never finishes makes **both** futures `Pending` forever.
/// That spot is a point where neither the deadline nor [`common::PATIENCE`] has started yet, so without a cap the test hangs regardless of the
/// flood that follows being bounded.
async fn connected_pair(
    listener: &mut WsListener,
) -> (
    (Box<dyn LinkTx>, Box<dyn LinkRx>),
    (Box<dyn LinkTx>, Box<dyn LinkRx>),
) {
    let addr = listener.dial_address();
    let dialer = common::dialer();
    let (accepted, dialed) = tokio::time::timeout(common::PATIENCE, async {
        tokio::join!(common::accept_link(listener), dialer.dial(&addr))
    })
    .await
    .expect("핸드셰이크 상한 — TCP 는 붙었는데 WS 업그레이드가 안 끝난 것이 첫 가설이다");
    (accepted.expect("accept"), dialed.expect("dial"))
}

#[tokio::test]
async fn writes_into_a_peer_that_never_reads_are_cut_by_the_deadline() {
    let _one_at_a_time = one_at_a_time().await;
    let mut listener = common::listen();

    // ★Hold the accepted link and **never poll it**★ — so the receiving side's kernel buffer fills, and then
    //   the sending side stalls. If dropped, the socket closes and a link error arises instead of the deadline.
    let (_never_read, (mut tx, _rx)) = connected_pair(&mut listener).await;

    let cut = flood_until_cut(&mut tx).await;
    assert!(
        cut.is_some(),
        "안 읽는 상대에게 {FRAME_BUDGET}개 × {FRAME} 바이트가 그대로 들어갔다"
    );
}

#[tokio::test]
async fn the_same_writes_into_a_peer_that_reads_are_never_cut() {
    // ★This is the opposite side of the test above★ — without it an implementation of 「이만한 쓰기는 무조건 시한에 걸린다」 (a write this size always hits the deadline) passes too.
    //   Frame count, size and deadline are all the same and the only difference is whether the peer reads.
    let _one_at_a_time = one_at_a_time().await;
    let mut listener = common::listen();

    let ((_server_tx, mut server_rx), (mut tx, _rx)) = connected_pair(&mut listener).await;

    let drain = tokio::spawn(async move {
        let mut seen = 0usize;
        while seen < FRAME_BUDGET * FRAME {
            // ★If something non-binary arrives, say so on the spot★ — if skipped, the cause is disguised as a byte-count
            //   mismatch. Today nothing sends keepalive on this link (it is a raw link, not the supervisor)
            //   and that is the premise of this assertion.
            match server_rx.recv().await {
                Ok(LinkRead::Frame(Frame::Binary(bytes))) => seen += bytes.len(),
                other => panic!("읽어 주는 쪽에 바이너리 프레임 대신 {other:?} 가 왔다 (여기까지 {seen} 바이트)"),
            }
        }
        seen
    });

    let cut = flood_until_cut(&mut tx).await;
    assert_eq!(
        cut, None,
        "읽어 주는 상대에게 보내는 쓰기가 시한에 걸렸다 (프레임 {cut:?})"
    );
    let seen = tokio::time::timeout(common::PATIENCE, drain)
        .await
        .expect("받는 쪽 상한")
        .expect("받는 쪽 태스크");
    assert_eq!(seen, FRAME_BUDGET * FRAME, "보낸 만큼 건너오지 않았다");
}

#[tokio::test]
async fn the_supervisor_names_the_write_deadline_when_it_drops_a_parked_link() {
    let _one_at_a_time = one_at_a_time().await;
    let mut listener = common::listen();
    let addr = listener.dial_address();

    let policy = Policy {
        write_deadline: Duration::from_millis(500),
        // ★Push keepalive and the silence deadline out of the window★ — the measurement in the file header is the reason. The decision stands only if the flood below is the only thing that can produce
        //   `WriteDeadline` inside this window.
        ping_interval: SUPERVISOR_PING_INTERVAL,
        idle_timeout: SUPERVISOR_IDLE_TIMEOUT,
        // Where to push, at once, enough for the supervisor to reach its first stall.
        outbound_queue: PARKED_BLOBS,
        ..Policy::default()
    };
    let registry = common::registry(Arc::new(common::dialer()), policy);
    let (peer, mut inbox) = registry.add(PeerId::new("d1"), addr, Arc::new(ImmediateHandshake));

    // ★After receiving, never poll it★ — same reason as the test above.
    let _never_read = tokio::time::timeout(common::PATIENCE, listener.accept())
        .await
        .expect("수락 상한")
        .expect("accept");
    common::wait_state(&peer, "운영 단계", |state| state.is_live()).await;

    let flooded = Instant::now();
    for nth in 0..PARKED_BLOBS {
        peer.notify(TestOut::Blob(vec![0u8; FRAME]))
            .unwrap_or_else(|err| panic!("{nth}번째가 나가는 큐에 못 들었다: {err:?}"));
    }

    let cause = common::wait_incoming(
        &mut inbox,
        &format!(
            "Disconnected 사건(첫 가설 = {PARKED_BLOBS}개 × {FRAME} 바이트에서는 커널이 더는 멈추지 \
             않는다)"
        ),
        |msg| match msg.as_event() {
            Some(TransportEvent::Disconnected { cause, .. }) => Some(*cause),
            _ => None,
        },
    )
    .await;
    let waited = flooded.elapsed();
    assert_eq!(
        cause,
        DisconnectCause::WriteDeadline,
        "멈춘 쓰기를 다른 사유로 끊었다"
    );
    // ★The reason name alone cannot tell whether the source was the flood or the keepalive write at t=period★ — if that distinction is left
    //   to the two policy lines above alone, neither the person who deletes those lines nor the person who raises
    //   [`common::PATIENCE`] would know they revived the coin flip. So here we directly measure that the event came
    //   far earlier than that period (with a 3x margin).
    assert!(
        waited * 3 < SUPERVISOR_PING_INTERVAL,
        "WriteDeadline 이 {waited:?} 만에 왔다 — keepalive 주기({SUPERVISOR_PING_INTERVAL:?})에 가까우면 \
         사유의 출처가 flood 가 아니라 그 주기의 쓰기일 수 있다"
    );
}

#[tokio::test]
async fn an_oversized_frame_is_reported_as_a_write_failure_instead_of_killing_the_supervisor() {
    // ★What is measured is 「값으로 실패하나」 (does it fail as a value)★ — if this spot panics, the supervisor task dies inside it, and nobody holds that
    //   `JoinHandle`, so there is **no place to observe the panic**: the emitted state freezes at `Live`
    //   forever and neither a disconnect event nor a reconnect happens. So what this test measures is whether the event **comes**.
    let _one_at_a_time = one_at_a_time().await;
    let mut listener = common::listen();
    let addr = listener.dial_address();

    // ★Only hold the accepted link★ — the reconnect must actually connect for the second-generation assertion below to stand. Not polling
    //   is for the same reason as the other tests in this file, and the frames here never reach the socket in the first place.
    let server = tokio::spawn(async move {
        let mut links = Vec::new();
        while let Ok(link) = listener.accept().await {
            links.push(link);
        }
    });

    let registry = common::registry(Arc::new(common::dialer()), Policy::default());
    let (peer, mut inbox) = registry.add(PeerId::new("d1"), addr, Arc::new(ImmediateHandshake));
    common::wait_state(&peer, "운영 단계", |state| state.is_live()).await;

    peer.notify(TestOut::Blob(vec![0u8; OVER_LIMIT]))
        .expect("나가는 큐");

    let cause = common::wait_incoming(
        &mut inbox,
        "Disconnected 사건(첫 가설 = 그 쓰기가 값 대신 패닉으로 실패해 감독이 죽었다)",
        |msg| match msg.as_event() {
            Some(TransportEvent::Disconnected { cause, .. }) => Some(*cause),
            _ => None,
        },
    )
    .await;
    assert_eq!(
        cause,
        DisconnectCause::WriteFailed,
        "상한을 넘은 프레임을 다른 사유로 끊었다"
    );

    // ★The evidence that the supervisor is alive is the next link★ — a dead supervisor does not reconnect.
    let generation = common::wait_incoming(
        &mut inbox,
        "재연결의 Connected 사건(첫 가설 = 감독이 그 쓰기에서 죽었다)",
        |msg| match msg.as_event() {
            Some(TransportEvent::Connected { generation, .. }) => Some(*generation),
            _ => None,
        },
    )
    .await;
    assert_eq!(generation, Generation(2), "끊김 뒤 다음 통로가 안 붙었다");

    server.abort();
}
