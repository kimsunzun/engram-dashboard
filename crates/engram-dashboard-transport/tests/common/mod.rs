//! 실소켓 테스트 셋이 함께 쓰는 발판.
//!
//! ★거는 쪽 기다림은 전부 「알림 + 상한」이다★ — `sleep` 으로 재는 자리가 하나도 없다. 알림이 실제
//! 동기화 지점이고, 알림이 안 오면 [`PATIENCE`] 가 매달림을 그 자리에서 시끄러운 실패로 바꾼다.
//!
//! ★상한 없는 기다림은 태스크로 띄운 **받는 쪽**에만 남아 있다★ — 손님을 기다리는 것이 그 태스크의
//! 일이라 상한이 뜻을 갖지 않는 자리다(수락 루프·`drain` 태스크가 그 꼴이다). 판정은 전부 거는 쪽이
//! 지고 그쪽엔 상한이 있어 먼저 패닉하며, 받는 쪽 태스크는 `abort()` 나 런타임 종료로 걷힌다.
//! ★거는 쪽에는 예외가 없다 — 아홉째 테스트를 더하는 자리에서 이 문장이 기준이다★.
//!
//! ★인메모리 하네스의 `settle`/`settle_until` 은 여기 못 쓴다★ — 그쪽은 협조적 양보만으로 정착을
//! 만드는데, 실소켓·실시간에서는 양보가 커널을 기다려 주지 않는다.

use std::sync::Arc;
use std::time::Duration;

use engram_dashboard_transport::testing::TestWire;
use engram_dashboard_transport::{
    Dialer, Inbound, Incoming, Peer, PeerState, Policy, Registry, SystemClock,
};

/// 실소켓·실시간 기다림의 상한. 로컬 loopback 왕복에는 과분한 값이고, 넘겼다면 무언가 매달린 것이다.
///
/// ★이 값을 **정책 창**으로 쓰는 테스트가 없어야 한다★ — 느린 러너 때문에 여기를 올리는 일은 언제든
/// 있고, 그때 「이 값이 keepalive 주기보다 작아서 성립하던 판정」이 조용히 뒤집힌다. 그 모양이던
/// `ws_write_deadline.rs` 의 감독 테스트는 사건 도착 시각을 벽시계로 따로 단언해 자기 창을 스스로
/// 좁힌다 — 새 테스트도 그렇게 한다.
pub const PATIENCE: Duration = Duration::from_secs(10);

/// 명부 하나. ★통로를 여는 쪽을 부르는 자리에서 고른다★ — 소켓 옵션을 걸어야 하는 테스트가 자기
/// [`Dialer`] 를 가져온다(`ws_write_deadline.rs`).
pub fn registry(
    dialer: Arc<dyn Dialer>,
    policy: Policy,
) -> (Registry<TestWire>, Inbound<TestWire>) {
    Registry::new(Arc::new(TestWire), dialer, Arc::new(SystemClock), policy)
}

/// 상태가 `want` 를 만족할 때까지 watch 알림으로 기다린다. 만족한 상태를 돌려준다.
///
/// 패닉 조건: [`PATIENCE`] 초과 · 그 전에 감독이 사라짐.
pub async fn wait_state(
    peer: &Peer<TestWire>,
    what: &str,
    mut want: impl FnMut(PeerState) -> bool,
) -> PeerState {
    let mut state = peer.state();
    let settled = tokio::time::timeout(PATIENCE, async {
        loop {
            let current = *state.borrow_and_update();
            if want(current) {
                return Some(current);
            }
            if state.changed().await.is_err() {
                return None;
            }
        }
    })
    .await;
    match settled {
        Ok(Some(found)) => found,
        Ok(None) => panic!(
            "{what} 을 기다리는 동안 감독이 사라졌다 (마지막 {:?})",
            peer.peer_state()
        ),
        Err(_) => panic!(
            "{PATIENCE:?} 안에 {what} 이 되지 않았다 (지금 {:?})",
            peer.peer_state()
        ),
    }
}

/// 위로 올라오는 한 줄에서 `pick` 이 값을 주는 첫 항목까지 읽는다.
///
/// 패닉 조건: [`PATIENCE`] 초과 · 그 전에 명부가 닫힘.
pub async fn wait_incoming<T>(
    inbound: &mut Inbound<TestWire>,
    what: &str,
    mut pick: impl FnMut(&Incoming<TestWire>) -> Option<T>,
) -> T {
    let seen = tokio::time::timeout(PATIENCE, async {
        while let Some(msg) = inbound.recv().await {
            if let Some(value) = pick(&msg) {
                return Some(value);
            }
        }
        None
    })
    .await;
    match seen {
        Ok(Some(value)) => value,
        Ok(None) => panic!("{what} 을 보기 전에 인바운드가 닫혔다"),
        Err(_) => panic!("{PATIENCE:?} 안에 {what} 이 올라오지 않았다"),
    }
}
