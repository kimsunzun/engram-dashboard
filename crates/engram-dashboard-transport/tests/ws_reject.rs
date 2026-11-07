//! Real socket ③ — version-rejection round trip (TRD §9-1).
//!
//! ★What this file measures = **how a close that crossed a real socket is classified**★ — whether the close **code and
//! message** the peer carried cross the socket and come up as [`LinkRead::Closed`], and whether that classification **burns the reconnect budget**
//! (ADR-0180 decision 5). Without the former the latter has no material to decide on, so 「종결 거절은 예산을 안 쓴다」 (a terminal refusal does not spend the budget) does not
//! hold in code — exactly the place pinned down by the [`LinkRead`] rustdoc in `link.rs`. A stimulus is set up per branch:
//! a close that is a terminal refusal · a close that is a redial refusal · a close that carries a code but is **not** a refusal · a peer that vanished without a word.
//!
//! ★Do not count the suite size★ — a hand-written count silently goes stale each time a test is added, and a stale count is
//! worse than none (the next reader believes it). What this file measures is said by the paragraph above.
//!
//! ★It was also measured in memory, but the reason to measure again here★ — the harness's `reject()` hands over the close **as a value**,
//! while here a real WS close frame crosses the kernel. Whether the code and message survive that round trip is a property of the adapter.
//! ★This paragraph is the canonical source that `lib.rs` 「단독 검증」 points to by name — do not delete it★.
//!
//! ★Different arms gather under the one result of failing to connect★ — 「상대가 말하고 끊었다」 (the peer spoke and cut) and 「읽기가 실패했다」 (the read failed) have the same
//! classification ([`ConnectFailure::Unreachable`]), budget consumption and endpoint. So the place that asserts that result
//! pins down **which arm ran** through the `LinkError` message — if not pinned, the decision stays green even if the stimulus disappears (close frame
//! loss · RST · dependency change).
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

/// ★The point of this file is that the peer really sends this string and the dialing side reads it back★.
const REASON: &str = "version 3 != 4";

/// The message the handshake attaches when the peer **spoke** and cut (`peer.rs`).
const CLOSED_DURING_HANDSHAKE: &str = "peer closed during handshake";

/// The prefix the adapter attaches when a read failed (`ws.rs`).
const READ_ERROR_PREFIX: &str = "ws read:";

/// The message the adapter's latch gives when a link that already failed is read again (`WsRx::recv` in `ws.rs`).
const LATCHED_READ: &str = "ws read: link already failed";

/// The policy used by the test that measures budget exhaustion.
///
/// ★It does not sit waiting in real time for the default backoff (500ms→1s→2s = 3.5 seconds)★ — the backoff **schedule** is the job of
/// measuring in memory with the injected clock (the `policy.rs`·`peer.rs` unit suites), and what this file measures on a real socket is
/// classification and budget consumption. `max_attempts = 1` is the **minimum** that reaches exhaustion with 「초기 시도 + 재시도 하나 = 통로 둘」 (initial attempt + one retry = two links),
/// and the assertion 「통로가 하나보다 많다」 (more than one link) below counts those two.
fn spends_budget_without_waiting() -> Policy {
    Policy {
        reconnect: ReconnectPolicy {
            max_attempts: 1,
            // ★[`MIN_BACKOFF`] is the floor of `delay()`★ — even if written shorter, that function folds it up
            //   to here, so it cannot be made faster than this.
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

    // The receiving side: sees no frame at all and closes with code+message (= what the version rejection of ADR-0178 does).
    // ★Counting the links that connected is the material for the budget decision★ — if a refusal burns the budget, a second link connects after the backoff.
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

    // ★This is the only place in this file that uses the default policy★ — 「거절은 예산을 안 쓴다」 (a refusal does not spend the budget) is measured
    //   only when there is a budget to spend. An implementation that burns it would connect a second link after 500ms, and the count below exceeds 1.
    let registry = common::registry(Arc::new(common::dialer()), Policy::default());
    let (peer, mut inbox) = registry.add(PeerId::new("d1"), addr, Arc::new(ListeningHandshake));

    let (code, reason) =
        common::wait_incoming(&mut inbox, "Rejected 사건", |msg| match msg.as_event() {
            Some(TransportEvent::Rejected { code, reason, .. }) => Some((*code, reason.clone())),
            _ => None,
        })
        .await;
    assert_eq!(
        code,
        CloseCode::HANDSHAKE_REJECTED,
        "닫기 코드가 소켓을 건너오지 않았다"
    );
    assert_eq!(reason, REASON, "닫기 문구가 소켓을 건너오지 않았다");

    let settled = common::wait_state(&peer, "포기", |state| {
        matches!(state, PeerState::GaveUp(_))
    })
    .await;
    assert_eq!(
        settled,
        PeerState::GaveUp(GaveUpReason::Rejected),
        "거절이 예산 소진으로 둔갑했다"
    );
    assert_eq!(
        accepted.load(Ordering::Acquire),
        1,
        "거절이 재연결 예산을 태워 다시 붙었다"
    );

    server.abort();
}

#[tokio::test]
async fn a_retryable_close_code_from_the_peer_spends_budget_and_dials_again() {
    // ★The kind is carried by the code★ (TRD 3 §3-0 ⑤) — does a redial refusal code cross the socket and read as the branch that spends the budget. With the test
    //   above alone, an implementation of 「코드가 실리면 무엇이든 종결」 (anything is terminal if a code is carried) passes too.
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
            "ConnectFailed 또는 Rejected 사건",
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
        panic!("다시 거는 거절이 종결 거절로 올라왔다: code={code:?} reason={reason}")
    });
    assert_eq!(
        cause,
        ConnectFailure::Rejected {
            kind: RejectKind::Retryable,
            code: CloseCode::HANDSHAKE_RETRYABLE,
            reason: REASON.into(),
        },
        "코드가 소켓을 건너와 다시 거는 거절로 읽혀야 한다"
    );

    let settled = common::wait_state(&peer, "포기", |state| {
        matches!(state, PeerState::GaveUp(_))
    })
    .await;
    assert_eq!(
        settled,
        PeerState::GaveUp(GaveUpReason::BudgetExhausted),
        "다시 거는 거절은 예산이 바닥나야 포기한다"
    );
    assert!(
        accepted.load(Ordering::Acquire) > 1,
        "예산을 쓰는 갈래인데 재시도가 없었다"
    );

    server.abort();
}

#[tokio::test]
async fn a_peer_that_vanishes_without_speaking_is_unreachable_and_does_spend_budget() {
    // ★A silent exit is **not** a refusal★ — with the test above alone, an implementation of 「닫기면 무엇이든 거절」 (any close is a refusal) passes too.
    //
    // ★The spot this branch pins down is not `RejectKind::of`★ — a WS peer that simply dropped the link comes up in tungstenite as
    //   a **read error** (measured 2026-09-07: the `Ok(Err(err))` arm of `handshake_once` catches it), so it never passes through
    //   [`LinkRead::Closed`] at all. The [`READ_ERROR_PREFIX`] assertion below holds that measurement, so if
    //   a dependency starts reporting it as EOF, that spot turns red instead of this comment silently becoming false.
    //   The opposite side on the `RejectKind::of` end is the `going_away` test below.
    let mut listener = common::listen();
    let addr = listener.dial_address();
    let accepted = Arc::new(AtomicUsize::new(0));

    let server = tokio::spawn({
        let accepted = accepted.clone();
        async move {
            while let Ok(link) = listener.accept().await {
                accepted.fetch_add(1, Ordering::AcqRel);
                // Drops the link without a word.
                drop(link);
            }
        }
    });

    let registry = common::registry(Arc::new(common::dialer()), spends_budget_without_waiting());
    let (peer, mut inbox) = registry.add(PeerId::new("d1"), addr, Arc::new(ListeningHandshake));

    // ★Also pick up the refusal event★ — if not picked up, a regression that 「이 자극을 거절로 읽는」 (reads this stimulus as a refusal) is reported as exceeding the cap (10 seconds)
    //   and the cause is disguised as 「사건이 안 온다」 (the event never comes).
    let picked =
        common::wait_incoming(
            &mut inbox,
            "ConnectFailed 또는 Rejected 사건",
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
        panic!("못 붙은 것이 아니라 거절로 올라왔다: code={code:?} reason={reason}")
    });
    let ConnectFailure::Unreachable(err) = &cause else {
        panic!("말 없이 사라진 상대를 거절로 읽었다: {cause:?}");
    };
    assert!(
        err.message.starts_with(READ_ERROR_PREFIX),
        "읽기 오류 팔이 아니라 다른 팔이 돌았다 — 첫 가설은 「tungstenite 가 이 자극을 EOF 로 \
         신고하기 시작했다」이고, 그러면 이 테스트의 실측 주석도 함께 낡은 것이다: {err}"
    );

    let reason = common::wait_incoming(&mut inbox, "GaveUp 사건", |msg| match msg.as_event() {
        Some(TransportEvent::GaveUp { reason, .. }) => Some(*reason),
        _ => None,
    })
    .await;
    assert_eq!(
        reason,
        GaveUpReason::BudgetExhausted,
        "말 없이 끊긴 것을 거절로 읽었다"
    );
    assert!(
        accepted.load(Ordering::Acquire) > 1,
        "예산을 쓰는 갈래인데 재시도가 없었다"
    );
    let settled = common::wait_state(&peer, "포기", |state| {
        matches!(state, PeerState::GaveUp(_))
    })
    .await;
    assert_eq!(settled, PeerState::GaveUp(GaveUpReason::BudgetExhausted));

    server.abort();
}

#[tokio::test]
async fn a_graceful_going_away_is_not_a_rejection_and_does_spend_budget() {
    // ★This is the opposite side of `RejectKind::of`★ — if the peer closed **carrying a code** but that code is 「정상 종료」 (normal exit), it is not a refusal
    //   and it spends the budget (if the daemon is restarting, the next attempt connects). Without this branch an implementation
    //   of 「코드가 실려 있으면 무엇이든 거절」 (anything is a refusal if a code is carried) passes the earlier branches as they are.
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

    // ① ★Premise: this server's close code really crosses the socket★ — if not measured, ② below cannot be told apart from 「프레임이 유실돼 읽기 오류 팔이 돌았다」
    //    (the frame was lost and the read-error arm ran). The two arms have the same classification, budget consumption and endpoint, so
    //    ② is green either way, and then it is green even if the normal-exit exception of `RejectKind::of` is deleted.
    let (probe_tx, mut probe_rx) =
        tokio::time::timeout(common::PATIENCE, common::dialer().dial(&addr))
            .await
            .expect("직접 dial 상한 — TCP 는 붙었는데 WS 업그레이드가 안 끝난 것이 첫 가설이다")
            .expect("직접 dial");
    let read = tokio::time::timeout(common::PATIENCE, probe_rx.recv())
        .await
        .expect("닫기 상한")
        .expect("닫기 읽기");
    match read {
        LinkRead::Closed(Some(close)) => {
            assert_eq!(
                close.code,
                CloseCode::GOING_AWAY,
                "정상 종료 코드가 소켓을 건너오지 않았다"
            );
            assert_eq!(
                close.reason,
                Close::going_away().reason,
                "정상 종료 문구가 소켓을 건너오지 않았다"
            );
        }
        other => panic!("코드를 실은 닫기 대신 {other:?} 가 올라왔다"),
    }
    drop((probe_tx, probe_rx));
    let probe = accepted.load(Ordering::Acquire);

    // ② That close is not a refusal and spends the budget.
    let registry = common::registry(Arc::new(common::dialer()), spends_budget_without_waiting());
    let (peer, mut inbox) = registry.add(PeerId::new("d1"), addr, Arc::new(ListeningHandshake));

    // ★Also pick up the refusal event★ — if not picked up, a regression that 「이 자극을 거절로 읽는」 (reads this stimulus as a refusal) is reported as exceeding the cap (10 seconds)
    //   and the cause is disguised as 「사건이 안 온다」 (the event never comes).
    let picked =
        common::wait_incoming(
            &mut inbox,
            "ConnectFailed 또는 Rejected 사건",
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
        panic!("못 붙은 것이 아니라 거절로 올라왔다: code={code:?} reason={reason}")
    });
    let ConnectFailure::Unreachable(err) = &cause else {
        panic!("정상 종료를 거절로 읽었다: {cause:?}");
    };
    // ★What the message tells apart is 「말하고 끊김」 (spoke and was cut) versus 「읽기 오류」 (read error)★ — ① already measured that the code crosses,
    //   so the only residue left here is 「코드 없는 종료도 같은 문구를 낸다」 (an exit with no code gives the same message), and in that case ① turns red first.
    assert_eq!(
        err.message, CLOSED_DURING_HANDSHAKE,
        "상대가 말하고 끊은 팔이 아니라 읽기 오류 팔이 돌았다 — ①이 잰 닫기 프레임이 감독 쪽 \
         통로에서는 유실됐다는 뜻이다: {err}"
    );

    let reason = common::wait_incoming(&mut inbox, "GaveUp 사건", |msg| match msg.as_event() {
        Some(TransportEvent::GaveUp { reason, .. }) => Some(*reason),
        _ => None,
    })
    .await;
    assert_eq!(
        reason,
        GaveUpReason::BudgetExhausted,
        "정상 종료를 거절로 읽었다"
    );
    assert!(
        accepted.load(Ordering::Acquire) > probe + 1,
        "예산을 쓰는 갈래인데 재시도가 없었다"
    );
    let settled = common::wait_state(&peer, "포기", |state| {
        matches!(state, PeerState::GaveUp(_))
    })
    .await;
    assert_eq!(settled, PeerState::GaveUp(GaveUpReason::BudgetExhausted));

    server.abort();
}

#[tokio::test]
async fn a_read_after_a_failed_read_is_still_an_error_never_a_spoken_close() {
    // ★[`LinkRead::Closed`] means 「상대가 닫았다고 **말했다**」 (the peer said it closed)★ — if a close nobody spoke comes up under
    //   that name, a consumer that branches on that value swaps 「거절당했다/곱게 갔다」 (rejected / left politely) for 「끊겼다」 (cut off).
    //   `vanishes_without_speaking` above measures **the supervisor's classification** of that stimulus, and here we measure the floor
    //   that classification sits on — the place in the adapter where reading happens **twice**.
    let mut listener = common::listen();
    let addr = listener.dial_address();

    // The dialing side and the receiving side wait on each other, so the handshake finishes only when they are polled together.
    let dialer = common::dialer();
    let (accepted, dialed) = tokio::time::timeout(common::PATIENCE, async {
        tokio::join!(common::accept_link(&mut listener), dialer.dial(&addr))
    })
    .await
    .expect("핸드셰이크 상한 — TCP 는 붙었는데 WS 업그레이드가 안 끝난 것이 첫 가설이다");
    let server = accepted.expect("accept");
    let (_client_tx, mut client_rx) = dialed.expect("dial");

    // ★It vanishes without a close frame★ — the socket closes only when both ends are released (`split_link` in `ws.rs`).
    drop(server);

    let first = tokio::time::timeout(common::PATIENCE, client_rx.recv())
        .await
        .expect("첫 읽기 상한")
        .expect_err("말 없이 사라진 상대의 첫 읽기는 오류여야 한다");
    assert!(
        first.message.starts_with(READ_ERROR_PREFIX),
        "첫 읽기가 어댑터의 읽기 오류가 아니다: {first}"
    );
    // ★If the first failure is the latch message, then the latch fired first, not the stimulus★ — then the assertion below would measure itself,
    //   so we separate them here.
    assert_ne!(
        first.message, LATCHED_READ,
        "자극이 통로에 닿기 전에 걸쇠가 걸려 있었다"
    );

    match tokio::time::timeout(common::PATIENCE, client_rx.recv())
        .await
        .expect("두 번째 읽기 상한")
    {
        Err(again) => assert_eq!(
            again.message, LATCHED_READ,
            "두 번째 읽기가 걸쇠가 아닌 다른 오류를 냈다 — 걸쇠 없이 통로를 다시 폴링한 것이 첫 가설이다"
        ),
        Ok(read) => panic!(
            "읽기가 실패한 뒤 두 번째 읽기가 값으로 올라왔다 — `Closed` 면 아무도 말하지 않은 닫기를 \
             보고한 것이다: {read:?}"
        ),
    }
}
