//! 실소켓 테스트 셋이 함께 쓰는 발판.
//!
//! ★거는 쪽 기다림은 전부 「알림 + 상한」이다★ — `sleep` 으로 재는 자리가 하나도 없다. 알림이 실제
//! 동기화 지점이고, 알림이 안 오면 [`PATIENCE`] 가 매달림을 그 자리에서 시끄러운 실패로 바꾼다.
//!
//! ★상한 없는 기다림은 태스크로 띄운 **받는 쪽**에만 남아 있다★ — 손님을 기다리는 것이 그 태스크의
//! 일이라 상한이 뜻을 갖지 않는 자리다(수락 루프·`drain` 태스크가 그 꼴이다). 판정은 전부 거는 쪽이
//! 지고 그쪽엔 상한이 있어 먼저 패닉하며, 받는 쪽 태스크는 `abort()` 나 런타임 종료로 걷힌다.
//! ★거는 쪽에는 예외가 없다★ — 새 기다림을 더하는 자리에서 가르는 것은 **누가 판정하나** 하나다:
//! 판정을 지는 쪽의 기다림에는 전부 상한이 붙고, 손님을 기다리는 것이 그 자체로 일인 태스크에만 안
//! 붙는다. ★그 자리를 「몇 번째 테스트」로 세지 말 것★ — 세는 문장은 테스트가 하나 늘 때마다 조용히
//! 어긋난다.
//!
//! ★인메모리 하네스의 `settle`/`settle_until` 은 여기 못 쓴다★ — 그쪽은 협조적 양보만으로 정착을
//! 만드는데, 실소켓·실시간에서는 양보가 커널을 기다려 주지 않는다.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use engram_dashboard_transport::testing::{accepted_ends, TestWire};
use engram_dashboard_transport::ws::{AcceptPolicy, WsDialer, WsListener};
use engram_dashboard_transport::{
    Dialer, Inbox, Incoming, LinkError, LinkRx, LinkTx, Listener, Peer, PeerState, Policy,
    Registry, SystemClock,
};

/// 한 메시지 · 한 프레임의 바이트 상한 — 운영 값(TRD 3 §5 15)과 같다. 거는 쪽([`dialer`]) · 받는 쪽([`accept_policy`])에
/// 같은 값을 준다.
pub const MAX_MESSAGE: usize = 16 << 20;

/// 수락을 재지 않는 스위트가 쓰는 수락 정책 — 운영과 같은 모양의 값이다(Origin 단 업그레이드 거절 · loopback Host ·
/// 16 MiB). ★허용 전부를 뜻하는 값이 crate 에 없어서★ 시험도 실제 값으로 돈다.
///
/// ★업그레이드 시한은 [`PATIENCE`] 와 따로 둔다★ — 그 상수를 정책 창으로 쓰지 않는다(그 rustdoc).
pub fn accept_policy() -> AcceptPolicy {
    AcceptPolicy {
        allowed_origins: Vec::new(),
        allowed_host_names: vec!["127.0.0.1".into(), "localhost".into()],
        upgrade_timeout: Duration::from_secs(5),
        max_pending_upgrades: 8,
        max_message_bytes: MAX_MESSAGE,
        refusal_queue: 16,
    }
}

/// 포트 0 의 loopback — 커널이 고른 번호는 청취기에서 되읽는다.
pub fn loopback() -> SocketAddr {
    "127.0.0.1:0".parse().expect("주소 문법")
}

/// [`accept_policy`] 로 연 청취기. 거절 보고는 버린다 — 그것을 재는 것은 `ws_accept.rs` 다.
pub fn listen() -> WsListener {
    WsListener::bind(loopback(), accept_policy(), Arc::new(SystemClock))
        .expect("bind")
        .0
}

pub fn dialer() -> WsDialer {
    WsDialer::new(MAX_MESSAGE)
}

/// 받은 통로의 양 끝 — 받는 쪽을 감독 없이 손으로 모는 테스트가 쓴다.
pub async fn accept_link(
    listener: &mut WsListener,
) -> Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), LinkError> {
    listener.accept().await.map(accepted_ends)
}

/// 실소켓·실시간 기다림의 상한. 로컬 loopback 왕복에는 과분한 값이고, 넘겼다면 무언가 매달린 것이다.
///
/// ★이 값을 **정책 창**으로 쓰는 테스트가 없어야 한다★ — 느린 러너 때문에 여기를 올리는 일은 언제든
/// 있고, 그때 「이 값이 keepalive 주기보다 작아서 성립하던 판정」이 조용히 뒤집힌다. 그 모양이던
/// `ws_write_deadline.rs` 의 감독 테스트는 사건 도착 시각을 벽시계로 따로 단언해 자기 창을 스스로
/// 좁힌다 — 새 테스트도 그렇게 한다.
pub const PATIENCE: Duration = Duration::from_secs(10);

/// 명부 하나. ★통로를 여는 쪽을 부르는 자리에서 고른다★ — 소켓 옵션을 걸어야 하는 테스트가 자기
/// [`Dialer`] 를 가져온다(`ws_write_deadline.rs`).
pub fn registry(dialer: Arc<dyn Dialer>, policy: Policy) -> Registry<TestWire> {
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

/// 손잡이 하나의 수신단에서 `pick` 이 값을 주는 첫 항목까지 읽는다.
///
/// 패닉 조건: [`PATIENCE`] 초과 · 그 전에 수신단이 끝남.
pub async fn wait_incoming<T>(
    inbox: &mut Inbox<TestWire>,
    what: &str,
    mut pick: impl FnMut(&Incoming<TestWire>) -> Option<T>,
) -> T {
    let seen = tokio::time::timeout(PATIENCE, async {
        while let Some(msg) = inbox.recv().await {
            if let Some(value) = pick(&msg) {
                return Some(value);
            }
        }
        None
    })
    .await;
    match seen {
        Ok(Some(value)) => value,
        Ok(None) => panic!("{what} 을 보기 전에 수신단이 끝났다"),
        Err(_) => panic!("{PATIENCE:?} 안에 {what} 이 올라오지 않았다"),
    }
}
