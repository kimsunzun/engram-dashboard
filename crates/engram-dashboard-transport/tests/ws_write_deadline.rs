//! Real socket ② — when the peer does not read, does the write deadline really cut, and does that write path report failure **as a value** (TRD §9-1 · ADR-0177 decision 8).
//!
//! ★The one property that cannot in principle be measured in memory★ — the harness's `stall_writes` measures **our side's** deadline and queue logic; here it is the **kernel send buffer** in front of it. Only a real
//! socket can fill that buffer so a write never finishes. It measures the link seam (`LinkTx::send` + `with_deadline`: does the adapter turn backpressure into `Pending`) and the supervisor's write path (do that `Pending`
//! and the link's error each drop the connection and report **naming the reason**).
//!
//! ## Why this file has this shape
//!
//! - ★Push frames **repeatedly**★ — one big frame does not stall (evidence: `src/ws.rs` header 「Link properties」, numbers not copied). Socket buffer options do not change it, so `WsDialer` · `WsListener` are used as is.
//! - ★The supervisor test pushes `ping_interval` and `idle_timeout` out of the window★ — unpushed (2s/5s), the pass came **not from the flood but from the keepalive write at t=4s**, whose deadline expiry coincided with
//!   `idle_timeout` (5.004s): a coin-flip with `Idle` (measured). ★The property is not left to two policy lines★ — the test measures the event arrival time against the wall clock, so deleting them or raising
//!   [`common::PATIENCE`] cannot quietly revive the coin flip.
//! - ★It runs serially★ — one binary's tests run concurrently, and this file has a test saturating loopback beside one asserting 「no write hit the deadline」; overlap on one kernel stack could turn that red (a real risk on a small runner).
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

const FRAME: usize = 1024 * 1024;

/// How many frames to push at most. ★Twenty times the stall point in the `ws.rs` header★ — if all go in, the test fails citing this number: the kernel's swallow amount changed and this is where to raise it.
const FRAME_BUDGET: usize = 64;

/// The write deadline. ★A stalled write **never** finishes unless the peer reads, so a large value does not blur the positive signal★; what it guards against is the false positive 「a healthy write is slow and hits it」.
const DEADLINE: Duration = Duration::from_secs(2);

/// The cap over the whole flood. ★No path uses the worst case ([`FRAME_BUDGET`] × [`DEADLINE`] = 128 s)★ — against a non-reading peer it hits within a few frames (`ws.rs` header), against a reading one the loopback
/// sends everything quickly. It catches the neither-case (a slowly flowing link hitting neither deadline nor budget) and makes it loud instead of waiting two-digit minutes on a shared runner.
const FLOOD_CAP: Duration = Duration::from_secs(20);

/// Where the supervisor test pushes keepalive out. ★The file-header measurement is the reason, and the wall-clock assertion below reads this value back as the criterion★.
const SUPERVISOR_PING_INTERVAL: Duration = Duration::from_secs(30);

const SUPERVISOR_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Frames pushed at once so the supervisor reaches its first stall = the outgoing queue length; larger than what the kernel swallows (`ws.rs` header).
const PARKED_BLOBS: usize = 16;

/// A payload **one byte** over the receiving side's cap ([`common::MAX_MESSAGE`], the cap this suite gave the dialing side); the cap itself is legal (the adapter checks with `>`).
const OVER_LIMIT: usize = common::MAX_MESSAGE + 1;

static SOCKET_LOCK: Mutex<()> = Mutex::const_new(());

/// Tests of this file running now. ★If the lock is deleted this exceeds 1 and turns red★ — left to structure alone, the next session may take the lock for an 「unnecessary device」 and remove it unnoticed.
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
        "this file's tests ran overlapping — each other's loopback load mixes into the deadline decision"
    );
    held
}

/// Pushes frames until the deadline hits; returns the number of the frame that hit, `None` if all of [`FRAME_BUDGET`] went in. Panics: a link error instead of the deadline · exceeding [`FLOOD_CAP`].
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
                Ok(Err(err)) => panic!("at #{nth} a link error happened, not the deadline: {err}"),
                Err(Elapsed) => return Some(nth),
            }
        }
        None
    };
    match tokio::time::timeout(FLOOD_CAP, flood).await {
        Ok(cut) => cut,
        Err(_) => panic!(
            "within {FLOOD_CAP:?} neither the deadline nor the budget ({FRAME_BUDGET} × {FRAME} bytes) was hit — the link is \
              neither stalled nor drained, but flowing slowly"
        ),
    }
}

/// Both ends of a link that finished the handshake. ★The two sides wait on each other, so it finishes only when polled together★. ★Why a cap★ — TCP connected but the WS upgrade never finishing makes **both**
/// futures `Pending` forever, a point where neither the deadline nor [`common::PATIENCE`] has started, so uncapped the test hangs.
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
    .expect("handshake cap — the first hypothesis is that TCP attached but the WS upgrade did not finish");
    (accepted.expect("accept"), dialed.expect("dial"))
}

#[tokio::test]
async fn writes_into_a_peer_that_never_reads_are_cut_by_the_deadline() {
    let _one_at_a_time = one_at_a_time().await;
    let mut listener = common::listen();

    // ★Hold the accepted link and **never poll it**★ — so the receiving kernel buffer fills and the sender stalls; dropped, the socket closes and a link error arises instead of the deadline.
    let (_never_read, (mut tx, _rx)) = connected_pair(&mut listener).await;

    let cut = flood_until_cut(&mut tx).await;
    assert!(
        cut.is_some(),
        "{FRAME_BUDGET} × {FRAME} bytes went straight in to a peer that does not read"
    );
}

#[tokio::test]
async fn the_same_writes_into_a_peer_that_reads_are_never_cut() {
    // ★The opposite of the test above★ — without it an implementation 「a write this size always hits the deadline」 passes too; only whether the peer reads differs.
    let _one_at_a_time = one_at_a_time().await;
    let mut listener = common::listen();

    let ((_server_tx, mut server_rx), (mut tx, _rx)) = connected_pair(&mut listener).await;

    let drain = tokio::spawn(async move {
        let mut seen = 0usize;
        while seen < FRAME_BUDGET * FRAME {
            // ★If something non-binary arrives, say so on the spot★ — skipped, the cause is disguised as a byte-count mismatch. Nothing sends keepalive on this raw link, which is this assertion's premise.
            match server_rx.recv().await {
                Ok(LinkRead::Frame(Frame::Binary(bytes))) => seen += bytes.len(),
                other => panic!("{other:?} came to the reading side instead of a binary frame ({seen} bytes so far)"),
            }
        }
        seen
    });

    let cut = flood_until_cut(&mut tx).await;
    assert_eq!(
        cut, None,
        "a write to a reading peer hit the deadline (frame {cut:?})"
    );
    let seen = tokio::time::timeout(common::PATIENCE, drain)
        .await
        .expect("receiving-side cap")
        .expect("receiving-side task");
    assert_eq!(
        seen,
        FRAME_BUDGET * FRAME,
        "not as much crossed as was sent"
    );
}

#[tokio::test]
async fn the_supervisor_names_the_write_deadline_when_it_drops_a_parked_link() {
    let _one_at_a_time = one_at_a_time().await;
    let mut listener = common::listen();
    let addr = listener.dial_address();

    let policy = Policy {
        write_deadline: Duration::from_millis(500),
        // ★Push keepalive and the silence deadline out of the window★ (file header) — the decision stands only if the flood is the only thing that can produce `WriteDeadline` inside it.
        ping_interval: SUPERVISOR_PING_INTERVAL,
        idle_timeout: SUPERVISOR_IDLE_TIMEOUT,
        outbound_queue: PARKED_BLOBS,
        ..Policy::default()
    };
    let registry = common::registry(Arc::new(common::dialer()), policy);
    let (peer, mut inbox) = registry.add(PeerId::new("d1"), addr, Arc::new(ImmediateHandshake));

    // ★After receiving, never poll it★ — same reason as the test above.
    let _never_read = tokio::time::timeout(common::PATIENCE, listener.accept())
        .await
        .expect("accept cap")
        .expect("accept");
    common::wait_state(&peer, "operating phase", |state| state.is_live()).await;

    let flooded = Instant::now();
    for nth in 0..PARKED_BLOBS {
        peer.notify(TestOut::Blob(vec![0u8; FRAME]))
            .unwrap_or_else(|err| panic!("#{nth} could not enter the outbound queue: {err:?}"));
    }

    let cause = common::wait_incoming(
        &mut inbox,
        &format!(
            "Disconnected event (first hypothesis = at {PARKED_BLOBS} × {FRAME} bytes the kernel no longer \
              stalls)"
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
        "cut the stalled write for another reason"
    );
    // ★The reason name alone cannot tell whether the source was the flood or the keepalive write at t=period★ — so measure directly that the event came far earlier than that period (3x margin);
    //   otherwise deleting the two policy lines or raising [`common::PATIENCE`] would revive the coin flip unnoticed.
    assert!(
        waited * 3 < SUPERVISOR_PING_INTERVAL,
        "WriteDeadline came after {waited:?} — if it is close to the keepalive period ({SUPERVISOR_PING_INTERVAL:?}), \
          the source of the reason may be that period's write, not the flood"
    );
}

#[tokio::test]
async fn an_oversized_frame_is_reported_as_a_write_failure_instead_of_killing_the_supervisor() {
    // ★Measures 「does it fail as a value」★ — a panic here kills the supervisor task and nobody holds its `JoinHandle`, so the state freezes at `Live` with no disconnect event or reconnect; this test measures that the event **comes**.
    let _one_at_a_time = one_at_a_time().await;
    let mut listener = common::listen();
    let addr = listener.dial_address();

    // ★Only hold the accepted link★ — the reconnect must really connect for the second-generation assertion; not polling, as in the other tests (the frames never reach the socket).
    let server = tokio::spawn(async move {
        let mut links = Vec::new();
        while let Ok(link) = listener.accept().await {
            links.push(link);
        }
    });

    let registry = common::registry(Arc::new(common::dialer()), Policy::default());
    let (peer, mut inbox) = registry.add(PeerId::new("d1"), addr, Arc::new(ImmediateHandshake));
    common::wait_state(&peer, "operating phase", |state| state.is_live()).await;

    peer.notify(TestOut::Blob(vec![0u8; OVER_LIMIT]))
        .expect("outbound queue");

    let cause = common::wait_incoming(
        &mut inbox,
        "Disconnected event (first hypothesis = that write failed with a panic instead of a value and the supervisor died)",
        |msg| match msg.as_event() {
            Some(TransportEvent::Disconnected { cause, .. }) => Some(*cause),
            _ => None,
        },
    )
    .await;
    assert_eq!(
        cause,
        DisconnectCause::WriteFailed,
        "cut a frame over the cap for another reason"
    );

    // ★The next link is the evidence the supervisor is alive★ — a dead supervisor does not reconnect.
    let generation = common::wait_incoming(
        &mut inbox,
        "the reconnect's Connected event (first hypothesis = the supervisor died on that write)",
        |msg| match msg.as_event() {
            Some(TransportEvent::Connected { generation, .. }) => Some(*generation),
            _ => None,
        },
    )
    .await;
    assert_eq!(
        generation,
        Generation(2),
        "the next link did not attach after the disconnect"
    );

    server.abort();
}
