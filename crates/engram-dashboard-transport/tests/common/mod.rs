//! Scaffolding shared by the three real-socket tests.
//!
//! ★Every wait on the dialing side is 「알림 + 상한」 (notification + cap)★ — there is no place that measures with `sleep`. The notification is the real
//! synchronization point, and if no notification comes, [`PATIENCE`] turns the hang into a loud failure on the spot.
//!
//! ★Waits without a cap remain only on the **receiving side** spawned as a task★ — waiting for a guest is the job of that task,
//! so a cap carries no meaning there (the accept loop and the `drain` task are of that shape). The dialing side
//! carries every decision and has a cap, so it panics first, and the receiving-side task is reaped by `abort()` or runtime shutdown.
//! ★The dialing side has no exceptions★ — the one thing that decides where to draw the line when adding a new wait is **who decides**:
//! every wait on the side that carries the decision gets a cap, and only a task whose job is itself to wait for a guest
//! goes without. ★Do not count that place as 「몇 번째 테스트」 (the Nth test)★ — a sentence that counts silently
//! goes stale each time a test is added.
//!
//! ★The in-memory harness's `settle`/`settle_until` cannot be used here★ — that side builds settling from cooperative yielding
//! alone, and on a real socket in real time yielding does not wait for the kernel.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use engram_dashboard_transport::testing::{accepted_ends, TestWire};
use engram_dashboard_transport::ws::{AcceptPolicy, WsDialer, WsListener};
use engram_dashboard_transport::{
    Dialer, Inbox, Incoming, LinkError, LinkRx, LinkTx, Listener, Peer, PeerState, Policy,
    Registry, SystemClock,
};

/// The byte cap of one message and one frame — the same as the production value (TRD 3 §5 15). The
/// same value goes to the dialing side ([`dialer`]) and the receiving side ([`accept_policy`]).
pub const MAX_MESSAGE: usize = 16 << 20;

/// The accept policy used by suites that do not measure accepting — values of the same shape as production (reject upgrades with an Origin · loopback Host ·
/// 16 MiB). ★Because the crate has no value meaning allow-everything★, tests also run with real values.
///
/// ★The upgrade deadline is kept separate from [`PATIENCE`]★ — that constant is not used as a policy window (its rustdoc).
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

/// Loopback on port 0 — the number the kernel picked is read back from the listener.
pub fn loopback() -> SocketAddr {
    "127.0.0.1:0".parse().expect("주소 문법")
}

/// A listener opened with [`accept_policy`]. Refusal reports are dropped — measuring those is `ws_accept.rs`'s job.
pub fn listen() -> WsListener {
    WsListener::bind(loopback(), accept_policy(), Arc::new(SystemClock))
        .expect("bind")
        .0
}

pub fn dialer() -> WsDialer {
    WsDialer::new(MAX_MESSAGE)
}

/// Both ends of an accepted link — used by tests that drive the receiving side by hand without a supervisor.
pub async fn accept_link(
    listener: &mut WsListener,
) -> Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), LinkError> {
    listener.accept().await.map(accepted_ends)
}

/// The cap on real-socket, real-time waits. It is generous for a local loopback round trip, and if it was exceeded something hung.
///
/// ★No test may use this value as a **policy window**★ — raising it because of a slow runner can happen at any time, and
/// then 「이 값이 keepalive 주기보다 작아서 성립하던 판정」 (a decision that held because this value was smaller than the keepalive interval) silently flips. The supervisor test in
/// `ws_write_deadline.rs`, which had that shape, narrows its own window by separately asserting the event arrival time against the wall clock —
/// new tests do the same.
pub const PATIENCE: Duration = Duration::from_secs(10);

/// One registry. ★The one that opens the link is chosen at the call site★ — a test that must set socket options brings its own
/// [`Dialer`] (`ws_write_deadline.rs`).
pub fn registry(dialer: Arc<dyn Dialer>, policy: Policy) -> Registry<TestWire> {
    Registry::new(Arc::new(TestWire), dialer, Arc::new(SystemClock), policy)
}

/// Waits through watch notifications until the state satisfies `want`. Returns the state that satisfied it.
///
/// Panics: when [`PATIENCE`] is exceeded · when the supervisor disappears before that.
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

/// Reads from one handle's receiver up to the first item for which `pick` gives a value.
///
/// Panics: when [`PATIENCE`] is exceeded · when the receiver ends before that.
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
