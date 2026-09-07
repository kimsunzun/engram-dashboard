//! 실소켓 ③ — 버전 거절 왕복(TRD §9-1).
//!
//! ★이 파일이 재는 것 = 실 소켓을 건너온 닫기를 **어떻게 분류하나**★ — 상대가 실은 닫기 **코드와
//! 문구**가 소켓을 건너와 [`LinkRead::Closed`] 로 올라오나, 그리고 그 분류가 **재연결 예산을 태우나**
//! (ADR-0180 결정 5). 앞의 것이 없으면 뒤의 것은 판정 재료가 없어서 「거절은 예산을 안 쓴다」가 코드로
//! 성립하지 않는다 — `link.rs` 의 [`LinkRead`] rustdoc 이 못박은 그 자리다. 자극은 갈래마다 세운다:
//! 거절인 닫기 · 코드를 실었지만 거절이 **아닌** 닫기 · 말 없이 사라진 상대.
//!
//! ★스위트 크기를 세지 말 것★ — 손으로 적은 개수는 테스트가 하나 늘 때마다 조용히 어긋나고, 낡은 개수는
//! 아예 없는 것보다 나쁘다(다음 읽는 사람이 그것을 믿는다). 무엇을 재는 파일인지는 위 문단이 말한다.
//!
//! ★인메모리로도 재던 것이지만 여기서 다시 재는 이유★ — 하네스의 `reject()` 는 닫기를 **값으로** 건네고,
//! 여기서는 실 WS 닫기 프레임이 커널을 건너간다. 코드·문구가 그 왕복에서 살아남는지는 어댑터의 성질이다.
//! ★이 문단이 `lib.rs` 「단독 검증」이 이 파일을 가리켜 부르는 그 정본이다 — 지우지 말 것★.
//!
//! ★못 붙었다는 결과 하나에 서로 다른 팔이 모인다★ — 「상대가 말하고 끊었다」와 「읽기가 실패했다」는
//! 분류([`ConnectFailure::Unreachable`])·예산 소비·종점이 전부 같다. 그래서 그 결과를 단언하는 자리는
//! `LinkError` 문구로 **어느 팔이 돌았는지**까지 못 박는다 — 안 박으면 자극이 사라져도(닫기 프레임
//! 유실·RST·의존성 변경) 판정이 그대로 초록이다.
#![cfg(all(feature = "ws", feature = "test-support"))]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use engram_dashboard_transport::policy::MIN_BACKOFF;
use engram_dashboard_transport::testing::ListeningHandshake;
use engram_dashboard_transport::ws::{WsDialer, WsListener};
use engram_dashboard_transport::{
    Close, CloseCode, ConnectFailure, Dialer, GaveUpReason, LinkRead, PeerId, PeerState, Policy,
    ReconnectPolicy, TransportEvent,
};

/// ★상대가 실제로 이 문자열을 실어 보내고 거는 쪽이 그것을 되읽는지가 이 파일의 요점이다★.
const REASON: &str = "version 3 != 4";

/// 상대가 **말하고** 끊었을 때 핸드셰이크가 붙이는 문구(`peer.rs`).
const CLOSED_DURING_HANDSHAKE: &str = "peer closed during handshake";

/// 읽기가 실패했을 때 어댑터가 붙이는 접두(`ws.rs`).
const READ_ERROR_PREFIX: &str = "ws read:";

/// 이미 실패한 통로를 다시 읽었을 때 어댑터의 걸쇠가 내는 문구(`ws.rs` 의 `WsRx::recv`).
const LATCHED_READ: &str = "ws read: link already failed";

/// 예산 소진을 재는 테스트가 쓰는 정책.
///
/// ★기본 백오프(500ms→1s→2s = 3.5초)를 실시간으로 앉아 기다리지 않는다★ — 백오프 **일정**은 주입
/// 시계로 인메모리에서 재는 몫이고(`policy.rs`·`peer.rs` 단위 스위트), 이 파일이 실소켓으로 재는 것은
/// 분류와 예산 소비다. `max_attempts = 1` 은 「초기 시도 + 재시도 하나 = 통로 둘」로 소진에 닿는
/// **최소값**이고, 아래 「통로가 하나보다 많다」 단언이 그 둘을 센다.
fn spends_budget_without_waiting() -> Policy {
    Policy {
        reconnect: ReconnectPolicy {
            max_attempts: 1,
            // ★[`MIN_BACKOFF`] 는 `delay()` 의 바닥값이다★ — 더 짧게 적어도 그 함수가 여기까지 접어
            //   올리므로 이보다 빠르게 만들 수는 없다.
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
    let listener = WsListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.dial_address().expect("dial_address");
    let accepted = Arc::new(AtomicUsize::new(0));

    // 받는 쪽: 프레임을 하나도 안 보고 코드+문구로 닫는다(= ADR-0178 의 버전 거절이 하는 일).
    // ★붙은 통로를 세는 것이 예산 판정의 재료다★ — 거절이 예산을 태우면 백오프 뒤 둘째 통로가 붙는다.
    let server = tokio::spawn({
        let accepted = accepted.clone();
        async move {
            while let Ok((mut tx, rx)) = listener.accept().await {
                accepted.fetch_add(1, Ordering::AcqRel);
                tx.close(Close::new(CloseCode::HANDSHAKE_REJECTED, REASON))
                    .await;
                // ★양 끝을 다 놓아야 소켓이 닫힌다★(`ws.rs` 의 `split_link`).
                drop((tx, rx));
            }
        }
    });

    // ★이 파일에서 여기만 기본 정책이다★ — 「거절은 예산을 안 쓴다」는 쓸 예산이 실제로 있을 때만
    //   재진다. 태우는 구현이면 500ms 뒤 둘째 통로가 붙어 아래 계수가 1을 넘는다.
    let (registry, mut inbound) = common::registry(Arc::new(WsDialer), Policy::default());
    let peer = registry.add(PeerId::new("d1"), addr, Arc::new(ListeningHandshake));

    let (code, reason) = common::wait_incoming(&mut inbound, "Rejected 사건", |msg| {
        match msg.as_event() {
            Some(TransportEvent::Rejected { code, reason, .. }) => Some((*code, reason.clone())),
            _ => None,
        }
    })
    .await;
    assert_eq!(
        code,
        Some(CloseCode::HANDSHAKE_REJECTED),
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
async fn a_peer_that_vanishes_without_speaking_is_unreachable_and_does_spend_budget() {
    // ★말 없는 종료는 거절이 **아니다**★ — 위 테스트만 있으면 「닫기면 무엇이든 거절」인 구현도 통과한다.
    //
    // ★이 갈래가 짚는 자리는 `is_rejection` 이 아니다★ — 통로를 그냥 버린 WS 상대는 tungstenite 에서
    //   **읽기 오류**로 올라오므로(실측 2026-09-07: `handshake_once` 의 `Ok(Err(err))` 팔이 잡는다)
    //   [`LinkRead::Closed`] 를 아예 안 지난다. 그 실측을 아래 [`READ_ERROR_PREFIX`] 단언이 붙들고 있어,
    //   의존성이 EOF 로 신고하기 시작하면 이 주석이 조용히 거짓이 되는 대신 그 자리가 빨개진다.
    //   `is_rejection` 쪽 반대편은 아래 `going_away` 테스트다.
    let listener = WsListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.dial_address().expect("dial_address");
    let accepted = Arc::new(AtomicUsize::new(0));

    let server = tokio::spawn({
        let accepted = accepted.clone();
        async move {
            while let Ok(link) = listener.accept().await {
                accepted.fetch_add(1, Ordering::AcqRel);
                // 아무 말 없이 통로를 버린다.
                drop(link);
            }
        }
    });

    let (registry, mut inbound) =
        common::registry(Arc::new(WsDialer), spends_budget_without_waiting());
    let peer = registry.add(PeerId::new("d1"), addr, Arc::new(ListeningHandshake));

    // ★거절 사건도 함께 집는다★ — 안 집으면 「이 자극을 거절로 읽는」 회귀가 상한 초과(10초)로
    //   신고돼 원인이 「사건이 안 온다」로 둔갑한다.
    let picked = common::wait_incoming(
        &mut inbound,
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

    let reason = common::wait_incoming(&mut inbound, "GaveUp 사건", |msg| match msg.as_event() {
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
    // ★`is_rejection` 의 반대편이다★ — 상대가 **코드를 실어** 닫았지만 그 코드가 「정상 종료」면 거절이
    //   아니고 예산을 쓴다(데몬 재시작 중이면 다음 시도에 붙는다). 이 갈래가 없으면 「코드가 실려
    //   있으면 무엇이든 거절」인 구현이 앞의 갈래들을 그대로 통과한다.
    let listener = WsListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.dial_address().expect("dial_address");
    let accepted = Arc::new(AtomicUsize::new(0));

    let server = tokio::spawn({
        let accepted = accepted.clone();
        async move {
            while let Ok((mut tx, rx)) = listener.accept().await {
                accepted.fetch_add(1, Ordering::AcqRel);
                tx.close(Close::going_away()).await;
                drop((tx, rx));
            }
        }
    });

    // ① ★전제: 이 서버의 닫기 코드가 실제로 소켓을 건너온다★ — 안 재면 아래 ②가 「프레임이 유실돼
    //    읽기 오류 팔이 돌았다」와 구별되지 않는다. 두 팔은 분류·예산 소비·종점이 전부 같아서 어느
    //    쪽이든 ②가 초록이고, 그러면 `is_rejection` 의 정상 종료 예외를 지워도 초록이다.
    let (probe_tx, mut probe_rx) = tokio::time::timeout(common::PATIENCE, WsDialer.dial(&addr))
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

    // ② 그 닫기가 거절이 아니고 예산을 쓴다.
    let (registry, mut inbound) =
        common::registry(Arc::new(WsDialer), spends_budget_without_waiting());
    let peer = registry.add(PeerId::new("d1"), addr, Arc::new(ListeningHandshake));

    // ★거절 사건도 함께 집는다★ — 안 집으면 「이 자극을 거절로 읽는」 회귀가 상한 초과(10초)로
    //   신고돼 원인이 「사건이 안 온다」로 둔갑한다.
    let picked = common::wait_incoming(
        &mut inbound,
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
    // ★문구가 가르는 것은 「말하고 끊김」과 「읽기 오류」다★ — ①이 코드가 건너오는 것을 이미 쟀으므로
    //   여기 남는 잔여는 「코드 없는 종료도 같은 문구를 낸다」 하나뿐이고, 그 경우 ①이 먼저 빨개진다.
    assert_eq!(
        err.message, CLOSED_DURING_HANDSHAKE,
        "상대가 말하고 끊은 팔이 아니라 읽기 오류 팔이 돌았다 — ①이 잰 닫기 프레임이 감독 쪽 \
         통로에서는 유실됐다는 뜻이다: {err}"
    );

    let reason = common::wait_incoming(&mut inbound, "GaveUp 사건", |msg| match msg.as_event() {
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
    // ★[`LinkRead::Closed`] 는 「상대가 닫았다고 **말했다**」는 뜻이다★ — 아무도 말하지 않은 닫기가 그
    //   이름으로 올라오면, 그 값으로 분기하는 소비자가 「거절당했다/곱게 갔다」를 「끊겼다」와 맞바꾼다.
    //   위 `vanishes_without_speaking` 은 그 자극에 대한 **감독의 분류**를 재고, 여기서는 그 분류가 앉아
    //   있는 바닥 — 어댑터에서 읽기를 **두 번** 하는 자리 — 을 잰다.
    let listener = WsListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.dial_address().expect("dial_address");

    // 거는 쪽과 받는 쪽이 서로를 기다리므로 함께 폴링해야 핸드셰이크가 끝난다.
    let (accepted, dialed) = tokio::time::timeout(common::PATIENCE, async {
        tokio::join!(listener.accept(), WsDialer.dial(&addr))
    })
    .await
    .expect("핸드셰이크 상한 — TCP 는 붙었는데 WS 업그레이드가 안 끝난 것이 첫 가설이다");
    let server = accepted.expect("accept");
    let (_client_tx, mut client_rx) = dialed.expect("dial");

    // ★닫기 프레임 없이 사라진다★ — 양 끝을 다 놓아야 소켓이 닫힌다(`ws.rs` 의 `split_link`).
    drop(server);

    let first = tokio::time::timeout(common::PATIENCE, client_rx.recv())
        .await
        .expect("첫 읽기 상한")
        .expect_err("말 없이 사라진 상대의 첫 읽기는 오류여야 한다");
    assert!(
        first.message.starts_with(READ_ERROR_PREFIX),
        "첫 읽기가 어댑터의 읽기 오류가 아니다: {first}"
    );
    // ★첫 실패가 걸쇠 문구면 자극이 아니라 걸쇠가 먼저 걸린 것이다★ — 그러면 아래 단언이 자기 자신을
    //   재게 되므로 여기서 갈라 둔다.
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
