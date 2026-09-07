//! 실소켓 ② — 상대가 안 읽을 때 쓰기 시한이 실제로 잘라내나(TRD §9-1 · ADR-0177 결정 8).
//!
//! ★인메모리로는 원리상 못 재는 유일한 성질이다★ — 하네스의 `stall_writes` 는 **우리 쪽** 시한·큐
//! 로직을 재고, 여기서 재는 것은 그 앞에 있는 **커널 send buffer** 다. 그것이 차서 쓰기가 영원히 안
//! 끝나는 상태를 만들 수 있는 것은 실소켓뿐이다.
//!
//! ★재는 자리가 둘이다★ — 통로 seam(`LinkTx::send` + `with_deadline`)과 감독의 쓰기 경로
//! (`DisconnectCause::WriteDeadline` 신고). 앞은 어댑터가 배압을 실제로 `Pending` 으로 옮기는지를,
//! 뒤는 그 `Pending` 이 정책 값에 걸려 연결을 버리고 **사유를 이름 붙여** 올리는지를 잰다.
//!
//! ## 이 파일의 모양이 왜 이런가
//!
//! - ★프레임을 **반복해서** 미는 것이 핵심이다★ — 한 프레임을 크게 만드는 방식으로는 안 멈춘다.
//!   근거(실측)의 정본은 `src/ws.rs` 헤더 「통로 성질」이고 여기 숫자를 베끼지 않는다. 소켓 버퍼 옵션도
//!   결과를 바꾸지 않으므로 이 파일은 `WsDialer`·`WsListener` 를 그대로 쓴다.
//! - ★감독 테스트가 `ping_interval`·`idle_timeout` 을 창 밖으로 밀어 두는 이유★ — 안 밀었을 때
//!   (2s/5s) 통과의 출처가 **flood 가 아니라 t=4s 의 keepalive 쓰기**였고, 그 시한 만료가
//!   `idle_timeout` 과 **같은 순간**(5.004s)에 도착했다(실측 2026-09-07). 즉 `Idle` 과 동전 던지기인
//!   테스트였다. 멀리 밀면 그 창 안에서 `WriteDeadline` 을 만들 수 있는 것이 flood 하나뿐이 된다.
//!   ★단 그 성질을 정책 값 두 줄에만 맡기지 않는다★ — 감독 테스트가 사건 도착 시각을 벽시계로 직접
//!   재므로, 그 두 줄을 지우거나 [`common::PATIENCE`] 를 올려도 동전 던지기가 조용히 돌아오지 않는다.
//! - ★세 테스트를 직렬로 돈다★ — libtest 는 한 바이너리의 테스트를 동시에 돌리는데, 아래 두 테스트는
//!   일부러 loopback 을 포화시키고 나머지 하나는 「어떤 쓰기도 시한에 안 걸렸다」를 단언한다. 같은 커널
//!   스택에서 겹치면 남의 부하가 그 단언을 빨갛게 만들 수 있다(작은 러너에서 실제 위험).
#![cfg(all(feature = "ws", feature = "test-support"))]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, MutexGuard};

use engram_dashboard_transport::link::LinkRead;
use engram_dashboard_transport::testing::{ImmediateHandshake, TestOut};
use engram_dashboard_transport::ws::{WsDialer, WsListener};
use engram_dashboard_transport::{
    with_deadline, Dialer, DisconnectCause, Elapsed, Frame, LinkRx, LinkTx, PeerId, Policy,
    SystemClock, TransportEvent,
};

/// 한 번에 미는 프레임 크기.
const FRAME: usize = 1024 * 1024;

/// 몇 프레임까지 밀어 보나. ★`ws.rs` 헤더가 적은 멈춤 지점의 스무 배 여유다★ — 다 들어가 버리면
/// 아래 테스트가 이 숫자를 말하며 실패한다. 그때 커널이 삼키는 양이 바뀐 것이고, 올릴 자리가 여기다.
const FRAME_BUDGET: usize = 64;

/// 쓰기 시한.
///
/// ★멈춘 쓰기는 상대가 읽지 않는 한 **영원히** 안 끝나므로 값이 커도 양성 신호가 흐려지지 않는다★ —
/// 이 값이 막는 것은 반대쪽, 즉 「멀쩡한 쓰기가 느려서 걸리는」 거짓 양성이다.
const DEADLINE: Duration = Duration::from_secs(2);

/// flood 전체에 걸리는 상한.
///
/// ★최악치([`FRAME_BUDGET`] × [`DEADLINE`] = 128초)를 실제로 쓰는 경로는 없다★ — 안 읽는 상대에게는 몇
/// 프레임 안에서 걸리고(`ws.rs` 헤더의 실측), 읽어 주는 상대에게는 loopback 이라 예산 전량이 금방
/// 나간다. 이 상한이 잡는 것은 그 둘 다 아닌 상태(시한도 예산도 안 걸리고 느리게 흐르는 통로)이고,
/// 공용 러너에서 두 자리 분(分)을 앉아 기다리는 대신 그 자리를 시끄럽게 만든다.
const FLOOD_CAP: Duration = Duration::from_secs(20);

/// 감독 테스트가 keepalive 를 밀어 두는 자리. ★파일 헤더의 실측이 그 사유고, 아래 벽시계 단언이 이
/// 값을 판정 기준으로 되읽는다★.
const SUPERVISOR_PING_INTERVAL: Duration = Duration::from_secs(30);

/// 감독 테스트가 침묵 시한을 밀어 두는 자리. 같은 사유다.
const SUPERVISOR_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// 감독이 첫 멈춤에 닿을 만큼 한 번에 밀어 둘 프레임 수 = 나가는 큐 길이. 커널이 삼키는 양보다 크다
/// (`ws.rs` 헤더).
const PARKED_BLOBS: usize = 16;

/// 이 바이너리의 테스트를 한 번에 하나만 돌린다 — 사유는 파일 헤더 「세 테스트를 직렬로 돈다」.
static SOCKET_LOCK: Mutex<()> = Mutex::const_new(());

/// 지금 이 파일에서 몇 개가 돌고 있나. ★잠금을 지우면 여기가 1을 넘어 그 자리가 빨개진다★ — 직렬성을
/// 구조에만 맡기면 다음 세션이 잠금을 「불필요한 장치」로 읽고 걷어도 아무도 모른다.
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

/// 시한에 걸릴 때까지 프레임을 민다. 걸린 프레임 번호를 돌려주고, [`FRAME_BUDGET`] 을 다 밀고도
/// 안 걸리면 `None`.
///
/// 패닉 조건: 시한 대신 통로 오류 · [`FLOOD_CAP`] 초과.
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

/// 핸드셰이크까지 끝난 통로 두 끝. ★거는 쪽과 받는 쪽이 서로를 기다리므로 함께 폴링해야 끝난다★.
///
/// ★상한이 붙은 이유★ — TCP 는 붙었는데 WS 업그레이드가 안 끝나는 회귀는 두 future 를 **둘 다** 영구히
/// `Pending` 으로 만든다. 그 자리는 시한도 [`common::PATIENCE`] 도 아직 시작되지 않은 지점이라, 상한이
/// 없으면 뒤따르는 flood 가 유계인 것과 무관하게 테스트가 매달린다.
async fn connected_pair(
    listener: &WsListener,
) -> (
    (Box<dyn LinkTx>, Box<dyn LinkRx>),
    (Box<dyn LinkTx>, Box<dyn LinkRx>),
) {
    let addr = listener.dial_address().expect("dial_address");
    let (accepted, dialed) = tokio::time::timeout(common::PATIENCE, async {
        tokio::join!(listener.accept(), WsDialer.dial(&addr))
    })
    .await
    .expect("핸드셰이크 상한 — TCP 는 붙었는데 WS 업그레이드가 안 끝난 것이 첫 가설이다");
    (accepted.expect("accept"), dialed.expect("dial"))
}

#[tokio::test]
async fn writes_into_a_peer_that_never_reads_are_cut_by_the_deadline() {
    let _one_at_a_time = one_at_a_time().await;
    let listener = WsListener::bind("127.0.0.1:0").await.expect("bind");

    // ★받은 통로를 붙들고 **한 번도 폴링하지 않는다**★ — 그래서 받는 쪽 커널 버퍼가 차고, 이어서
    //   보내는 쪽이 멈춘다. 떨어뜨리면 소켓이 닫혀 시한이 아니라 통로 오류가 난다.
    let (_never_read, (mut tx, _rx)) = connected_pair(&listener).await;

    let cut = flood_until_cut(&mut tx).await;
    assert!(
        cut.is_some(),
        "안 읽는 상대에게 {FRAME_BUDGET}개 × {FRAME} 바이트가 그대로 들어갔다"
    );
}

#[tokio::test]
async fn the_same_writes_into_a_peer_that_reads_are_never_cut() {
    // ★위 테스트의 반대쪽이다★ — 이것이 없으면 「이만한 쓰기는 무조건 시한에 걸린다」는 구현도 통과한다.
    //   프레임 수·크기·시한이 전부 같고 다른 것은 상대가 읽는지 하나뿐이다.
    let _one_at_a_time = one_at_a_time().await;
    let listener = WsListener::bind("127.0.0.1:0").await.expect("bind");

    let ((_server_tx, mut server_rx), (mut tx, _rx)) = connected_pair(&listener).await;

    let drain = tokio::spawn(async move {
        let mut seen = 0usize;
        while seen < FRAME_BUDGET * FRAME {
            // ★바이너리 아닌 것이 오면 그 자리에서 말한다★ — 건너뛰면 원인이 바이트 수 불일치로
            //   둔갑한다. 오늘 이 통로에 keepalive 를 보내는 것은 없지만(감독이 아니라 생 통로다)
            //   그것이 이 단언의 전제다.
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
    let listener = WsListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.dial_address().expect("dial_address");

    let policy = Policy {
        write_deadline: Duration::from_millis(500),
        // ★keepalive 와 침묵 시한을 창 밖으로 밀어 둔다★ — 파일 헤더의 실측이 그 사유다. 이 창 안에서
        //   `WriteDeadline` 을 만들 수 있는 것이 아래 flood 하나뿐이어야 판정이 선다.
        ping_interval: SUPERVISOR_PING_INTERVAL,
        idle_timeout: SUPERVISOR_IDLE_TIMEOUT,
        // 감독이 첫 멈춤에 닿을 만큼을 한 번에 밀어 둘 자리.
        outbound_queue: PARKED_BLOBS,
        ..Policy::default()
    };
    let (registry, mut inbound) = common::registry(Arc::new(WsDialer), policy);
    let peer = registry.add(PeerId::new("d1"), addr, Arc::new(ImmediateHandshake));

    // ★받고 나서 한 번도 폴링하지 않는다★ — 위 테스트와 같은 이유다.
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
        &mut inbound,
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
    // ★사유 이름만으로는 그 출처가 flood 인지 t=주기 의 keepalive 쓰기인지 안 갈린다★ — 그 갈림을 위
    //   정책 두 줄에만 맡기면, 그 줄을 지우는 사람도 [`common::PATIENCE`] 를 올리는 사람도 동전
    //   던지기를 되살린 것을 모른다. 그래서 사건이 그 주기보다 훨씬 이르게(3배 여유) 왔다는 것을 여기서
    //   직접 잰다.
    assert!(
        waited * 3 < SUPERVISOR_PING_INTERVAL,
        "WriteDeadline 이 {waited:?} 만에 왔다 — keepalive 주기({SUPERVISOR_PING_INTERVAL:?})에 가까우면 \
         사유의 출처가 flood 가 아니라 그 주기의 쓰기일 수 있다"
    );
}
