//! Real socket ③ — version-rejection round trip (TRD §9-1).
//!
//! ★Measures **how a close that crossed a real socket is classified**★ — whether the close **code and message** survive the socket and arrive as [`LinkRead::Closed`], and whether that classification **burns the
//! reconnect budget** (ADR-0180 decision 5); without the former 「a terminal refusal does not spend the budget」 has no material to decide on and does not hold in code (the [`LinkRead`] rustdoc in `link.rs`). One stimulus per
//! branch: a terminal-refusal close · a redial-refusal close · a code-carrying close that is **not** a refusal · a peer that vanished without a word.
//!
//! ★Do not count the suite size★ — a hand-written count goes stale with every added test. ★Why measure again though memory did★ — the harness's `reject()` hands the close over **as a value**; here a real WS close
//! frame crosses the kernel, and whether code and message survive is an adapter property. ★This paragraph is the canonical source `lib.rs` 「Standalone verification」 names — do not delete it★.
//!
//! ★Different arms gather under one failure-to-connect result★ — 「the peer spoke and cut」 and 「the read failed」 classify the same ([`ConnectFailure::Unreachable`]) with the same budget and endpoint, so the asserting
//! place pins **which arm ran** via the `LinkError` message; unpinned, the decision stays green even if the stimulus disappears (close-frame loss · RST · dependency change).
#![cfg(all(feature = "ws", feature = "test-support"))]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use engram_dashboard_transport::policy::MIN_BACKOFF;
use engram_dashboard_transport::testing::ListeningHandshake;
use engram_dashboard_transport::{
    Close, CloseCode, ConnectFailure, Dialer, GaveUpReason, LinkRead, Listener, PeerId, PeerState,
    Policy, ReconnectPolicy, RejectKind, TransportEvent,
};

const REASON: &str = "version 3 != 4";

const CLOSED_DURING_HANDSHAKE: &str = "peer closed during handshake";

const READ_ERROR_PREFIX: &str = "ws read:";

const LATCHED_READ: &str = "ws read: link already failed";

/// The policy of the budget-exhaustion test. ★It does not wait in real time for the default backoff (500ms→1s→2s)★ — the **schedule** is measured in memory with the injected clock (`policy.rs` · `peer.rs`
/// unit suites); here only classification and budget consumption. `max_attempts = 1` is the **minimum** reaching exhaustion (initial attempt + one retry = two links), which the 「more than one link」 assertion counts.
fn spends_budget_without_waiting() -> Policy {
    Policy {
        reconnect: ReconnectPolicy {
            max_attempts: 1,
            // ★[`MIN_BACKOFF`] is the floor of `delay()`★ — a shorter value is folded up to it, so it cannot go faster.
            base: MIN_BACKOFF,
            cap: MIN_BACKOFF,
            multiplier: 1.0,
            jitter: 0.0,
        },
        ..Policy::default()
    }
}

#[tokio::test]
async fn a_close_code_from_the_peer_is_a_rejection_that_spends_no_reconnect_budget() {
    let mut listener = common::listen();
    let addr = listener.dial_address();
    let accepted = Arc::new(AtomicUsize::new(0));

    // The receiving side sees no frame and closes with code+message (what ADR-0178's version rejection does). ★Counting connected links is the material for the budget decision★ — a refusal burning budget connects a second link after the backoff.
    let server = tokio::spawn({
        let accepted = accepted.clone();
        async move {
            while let Ok((mut tx, rx)) = common::accept_link(&mut listener).await {
                accepted.fetch_add(1, Ordering::AcqRel);
                tx.close(Close::new(CloseCode::HANDSHAKE_REJECTED, REASON))
                    .await;
                // ★The socket closes only when both ends are released★ (`split_link` in `ws.rs`).
                drop((tx, rx));
            }
        }
    });

    // ★The only test here using the default policy★ — 「a refusal does not spend the budget」 is measurable only with a budget to spend; an implementation burning it would connect a second link after 500ms and the count exceeds 1.
    let registry = common::registry(Arc::new(common::dialer()), Policy::default());
    let (peer, mut inbox) = registry.add(PeerId::new("d1"), addr, Arc::new(ListeningHandshake));

    let (code, reason) =
        common::wait_incoming(&mut inbox, "Rejected event", |msg| match msg.as_event() {
            Some(TransportEvent::Rejected { code, reason, .. }) => Some((*code, reason.clone())),
            _ => None,
        })
        .await;
    assert_eq!(
        code,
        CloseCode::HANDSHAKE_REJECTED,
        "the close code did not cross the socket"
    );
    assert_eq!(reason, REASON, "the close wording did not cross the socket");

    let settled = common::wait_state(&peer, "give up", |state| {
        matches!(state, PeerState::GaveUp(_))
    })
    .await;
    assert_eq!(
        settled,
        PeerState::GaveUp(GaveUpReason::Rejected),
        "the reject turned into budget exhaustion"
    );
    assert_eq!(
        accepted.load(Ordering::Acquire),
        1,
        "the reject burned the reconnect budget and reattached"
    );

    server.abort();
}

#[tokio::test]
async fn a_retryable_close_code_from_the_peer_spends_budget_and_dials_again() {
    // ★The kind is carried by the code★ (TRD 3 §3-0 ⑤) — does a redial-refusal code cross the socket and read as the budget-spending branch; the test above alone passes an implementation 「anything is terminal if a code is carried」.
    let mut listener = common::listen();
    let addr = listener.dial_address();
    let accepted = Arc::new(AtomicUsize::new(0));

    let server = tokio::spawn({
        let accepted = accepted.clone();
        async move {
            while let Ok((mut tx, rx)) = common::accept_link(&mut listener).await {
                accepted.fetch_add(1, Ordering::AcqRel);
                tx.close(Close::new(CloseCode::HANDSHAKE_RETRYABLE, REASON))
                    .await;
                drop((tx, rx));
            }
        }
    });

    let registry = common::registry(Arc::new(common::dialer()), spends_budget_without_waiting());
    let (peer, mut inbox) = registry.add(PeerId::new("d1"), addr, Arc::new(ListeningHandshake));

    let picked =
        common::wait_incoming(
            &mut inbox,
            "ConnectFailed or Rejected event",
            |msg| match msg.as_event() {
                Some(TransportEvent::ConnectFailed { cause, .. }) => Some(Ok(cause.clone())),
                Some(TransportEvent::Rejected { code, reason, .. }) => {
                    Some(Err((*code, reason.clone())))
                }
                _ => None,
            },
        )
        .await;
    let cause = picked.unwrap_or_else(|(code, reason)| {
        panic!("a reject that redials came up as a terminal reject: code={code:?} reason={reason}")
    });
    assert_eq!(
        cause,
        ConnectFailure::Rejected {
            kind: RejectKind::Retryable,
            code: CloseCode::HANDSHAKE_RETRYABLE,
            reason: REASON.into(),
        },
        "the code must cross the socket and be read as a reject that redials"
    );

    let settled = common::wait_state(&peer, "give up", |state| {
        matches!(state, PeerState::GaveUp(_))
    })
    .await;
    assert_eq!(
        settled,
        PeerState::GaveUp(GaveUpReason::BudgetExhausted),
        "a reject that redials gives up only when the budget runs out"
    );
    assert!(
        accepted.load(Ordering::Acquire) > 1,
        "a budget-using branch but there was no retry"
    );

    server.abort();
}

#[tokio::test]
async fn a_peer_that_vanishes_without_speaking_is_unreachable_and_does_spend_budget() {
    // ★A silent exit is **not** a refusal★ — the test above alone passes 「any close is a refusal」. ★The pinned spot is not `RejectKind::of`★: a WS peer that just dropped the link is a tungstenite **read error** (measured:
    //   the `Ok(Err(err))` arm of `handshake_once`), never passing through [`LinkRead::Closed`]; the [`READ_ERROR_PREFIX`] assertion holds that measurement, so a dependency reporting EOF turns it red instead of this comment going
    //   false. The `RejectKind::of` opposite is the `going_away` test below.
    let mut listener = common::listen();
    let addr = listener.dial_address();
    let accepted = Arc::new(AtomicUsize::new(0));

    let server = tokio::spawn({
        let accepted = accepted.clone();
        async move {
            while let Ok(link) = listener.accept().await {
                accepted.fetch_add(1, Ordering::AcqRel);
                drop(link);
            }
        }
    });

    let registry = common::registry(Arc::new(common::dialer()), spends_budget_without_waiting());
    let (peer, mut inbox) = registry.add(PeerId::new("d1"), addr, Arc::new(ListeningHandshake));

    // ★Also pick up the refusal event★ — unpicked, a regression 「reads this stimulus as a refusal」 shows as exceeding the cap (10 s) and the cause is disguised as 「the event never comes」.
    let picked =
        common::wait_incoming(
            &mut inbox,
            "ConnectFailed or Rejected event",
            |msg| match msg.as_event() {
                Some(TransportEvent::ConnectFailed { cause, .. }) => Some(Ok(cause.clone())),
                Some(TransportEvent::Rejected { code, reason, .. }) => {
                    Some(Err((*code, reason.clone())))
                }
                _ => None,
            },
        )
        .await;
    let cause = picked.unwrap_or_else(|(code, reason)| {
        panic!("came up as a reject, not a failure to attach: code={code:?} reason={reason}")
    });
    let ConnectFailure::Unreachable(err) = &cause else {
        panic!("read a peer that vanished silently as a reject: {cause:?}");
    };
    assert!(
        err.message.starts_with(READ_ERROR_PREFIX),
        "another arm ran, not the read error arm — the first hypothesis is 「tungstenite started reporting this \
          stimulus as EOF」, and then this test's measurement comment is stale too: {err}"
    );

    let reason = common::wait_incoming(&mut inbox, "GaveUp event", |msg| match msg.as_event() {
        Some(TransportEvent::GaveUp { reason, .. }) => Some(*reason),
        _ => None,
    })
    .await;
    assert_eq!(
        reason,
        GaveUpReason::BudgetExhausted,
        "read a silent drop as a reject"
    );
    assert!(
        accepted.load(Ordering::Acquire) > 1,
        "a budget-using branch but there was no retry"
    );
    let settled = common::wait_state(&peer, "give up", |state| {
        matches!(state, PeerState::GaveUp(_))
    })
    .await;
    assert_eq!(settled, PeerState::GaveUp(GaveUpReason::BudgetExhausted));

    server.abort();
}

#[tokio::test]
async fn a_graceful_going_away_is_not_a_rejection_and_does_spend_budget() {
    // ★The opposite of `RejectKind::of`★ — a close **carrying a code** that is 「normal exit」 is not a refusal and spends budget (a restarting daemon connects next time); without this branch 「anything with a code is a refusal」
    //   passes the earlier branches.
    let mut listener = common::listen();
    let addr = listener.dial_address();
    let accepted = Arc::new(AtomicUsize::new(0));

    let server = tokio::spawn({
        let accepted = accepted.clone();
        async move {
            while let Ok((mut tx, rx)) = common::accept_link(&mut listener).await {
                accepted.fetch_add(1, Ordering::AcqRel);
                tx.close(Close::going_away()).await;
                drop((tx, rx));
            }
        }
    });

    // ① ★Premise: this server's close code really crosses the socket★ — unmeasured, ② cannot be told from 「the frame was lost and the read-error arm ran」 (same classification, budget and endpoint), and ② stays green even if
    //    `RejectKind::of`'s normal-exit exception is deleted.
    let (probe_tx, mut probe_rx) =
        tokio::time::timeout(common::PATIENCE, common::dialer().dial(&addr))
            .await
            .expect("direct dial cap — the first hypothesis is that TCP attached but the WS upgrade did not finish")
            .expect("direct dial");
    let read = tokio::time::timeout(common::PATIENCE, probe_rx.recv())
        .await
        .expect("close cap")
        .expect("close read");
    match read {
        LinkRead::Closed(Some(close)) => {
            assert_eq!(
                close.code,
                CloseCode::GOING_AWAY,
                "the normal close code did not cross the socket"
            );
            assert_eq!(
                close.reason,
                Close::going_away().reason,
                "the normal close wording did not cross the socket"
            );
        }
        other => panic!("{other:?} came up instead of a close carrying the code"),
    }
    drop((probe_tx, probe_rx));
    let probe = accepted.load(Ordering::Acquire);

    // ② That close is not a refusal and spends the budget.
    let registry = common::registry(Arc::new(common::dialer()), spends_budget_without_waiting());
    let (peer, mut inbox) = registry.add(PeerId::new("d1"), addr, Arc::new(ListeningHandshake));

    // ★Also pick up the refusal event★ — see the same comment above.
    let picked =
        common::wait_incoming(
            &mut inbox,
            "ConnectFailed or Rejected event",
            |msg| match msg.as_event() {
                Some(TransportEvent::ConnectFailed { cause, .. }) => Some(Ok(cause.clone())),
                Some(TransportEvent::Rejected { code, reason, .. }) => {
                    Some(Err((*code, reason.clone())))
                }
                _ => None,
            },
        )
        .await;
    let cause = picked.unwrap_or_else(|(code, reason)| {
        panic!("came up as a reject, not a failure to attach: code={code:?} reason={reason}")
    });
    let ConnectFailure::Unreachable(err) = &cause else {
        panic!("read a normal close as a reject: {cause:?}");
    };
    // ★The message separates 「spoke and was cut」 from 「read error」★ — ① already measured that the code crosses, so the residue is 「an exit with no code gives the same message」, where ① turns red first.
    assert_eq!(
        err.message, CLOSED_DURING_HANDSHAKE,
        "the read error arm ran, not the arm where the peer said goodbye and cut — it means the close frame ① measured was lost on the supervisor's \
          link: {err}"
    );

    let reason = common::wait_incoming(&mut inbox, "GaveUp event", |msg| match msg.as_event() {
        Some(TransportEvent::GaveUp { reason, .. }) => Some(*reason),
        _ => None,
    })
    .await;
    assert_eq!(
        reason,
        GaveUpReason::BudgetExhausted,
        "read a normal close as a reject"
    );
    assert!(
        accepted.load(Ordering::Acquire) > probe + 1,
        "a budget-using branch but there was no retry"
    );
    let settled = common::wait_state(&peer, "give up", |state| {
        matches!(state, PeerState::GaveUp(_))
    })
    .await;
    assert_eq!(settled, PeerState::GaveUp(GaveUpReason::BudgetExhausted));

    server.abort();
}

#[tokio::test]
async fn a_read_after_a_failed_read_is_still_an_error_never_a_spoken_close() {
    // ★[`LinkRead::Closed`] means 「the peer **said** it closed」★ — a close nobody spoke under that name would make a consumer swap 「rejected / left politely」 for 「cut off」. `vanishes_without_speaking` measures the supervisor's
    //   classification; here the floor under it — the adapter's **second** read.
    let mut listener = common::listen();
    let addr = listener.dial_address();

    let dialer = common::dialer();
    let (accepted, dialed) = tokio::time::timeout(common::PATIENCE, async {
        tokio::join!(common::accept_link(&mut listener), dialer.dial(&addr))
    })
    .await
    .expect("handshake cap — the first hypothesis is that TCP attached but the WS upgrade did not finish");
    let server = accepted.expect("accept");
    let (_client_tx, mut client_rx) = dialed.expect("dial");

    // ★Vanishes without a close frame★ — the socket closes only when both ends are released (`split_link` in `ws.rs`).
    drop(server);

    let first = tokio::time::timeout(common::PATIENCE, client_rx.recv())
        .await
        .expect("first read cap")
        .expect_err("the first read of a peer that vanished silently must be an error");
    assert!(
        first.message.starts_with(READ_ERROR_PREFIX),
        "the first read is not the adapter's read error: {first}"
    );
    // ★If the first failure is the latch message, the latch fired first, not the stimulus★ — then the assertion below would measure itself; separate them here.
    assert_ne!(
        first.message, LATCHED_READ,
        "the latch was set before the stimulus reached the link"
    );

    match tokio::time::timeout(common::PATIENCE, client_rx.recv())
        .await
        .expect("second read cap")
    {
        Err(again) => assert_eq!(
            again.message, LATCHED_READ,
            "the second read raised an error other than the latch — the first hypothesis is that the link was polled again without the latch"
        ),
        Ok(read) => panic!(
            "after the read failed, a second read came up as a value — `Closed` would mean it reported a close \
              nobody said: {read:?}"
        ),
    }
}
