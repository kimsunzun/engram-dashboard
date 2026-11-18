//! Scaffolding shared by the real-socket tests. ★Every dialing-side wait is 「notification + cap」★ — nothing measures with `sleep`; the notification is the real synchronization point and [`PATIENCE`] turns a hang into a loud
//! failure. ★Uncapped waits remain only on the **receiving side** spawned as a task★ (waiting for a guest is that task's job; the accept loop and `drain` task are that shape) — the dialing side carries every decision and panics
//! first, and the receiving-side task is reaped by `abort()` or runtime shutdown. ★The dialing side has no exceptions★: when adding a wait ask **who decides** — the deciding side gets a cap; only a task whose job is to wait for a
//! guest goes without. Do not describe that place as 「the Nth test」 (counts go stale). ★The in-memory `settle`/`settle_until` cannot be used here★ — they yield cooperatively, which does not wait for the kernel on a real socket.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use engram_dashboard_transport::testing::{accepted_ends, TestWire};
use engram_dashboard_transport::ws::{AcceptPolicy, WsDialer, WsListener};
use engram_dashboard_transport::{
    Dialer, Inbox, Incoming, LinkError, LinkRx, LinkTx, Listener, Peer, PeerState, Policy,
    Registry, SystemClock,
};

/// The byte cap of one message and one frame — the production value (TRD 3 §5 15); the same value goes to [`dialer`] and [`accept_policy`].
pub const MAX_MESSAGE: usize = 16 << 20;

/// The accept policy of suites not measuring accepting — production-shaped values (reject upgrades with an Origin · loopback Host · 16 MiB); ★the crate has no allow-everything value★, so tests use real ones.
/// ★The upgrade deadline is separate from [`PATIENCE`]★ (that constant is not a policy window — its rustdoc).
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

pub fn loopback() -> SocketAddr {
    "127.0.0.1:0".parse().expect("address syntax")
}

pub fn listen() -> WsListener {
    WsListener::bind(loopback(), accept_policy(), Arc::new(SystemClock))
        .expect("bind")
        .0
}

pub fn dialer() -> WsDialer {
    WsDialer::new(MAX_MESSAGE)
}

pub async fn accept_link(
    listener: &mut WsListener,
) -> Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), LinkError> {
    listener.accept().await.map(accepted_ends)
}

/// The cap on real-socket, real-time waits — generous for a loopback round trip; exceeding it means something hung. ★No test may use it as a **policy window**★ — raising it for a slow runner would silently flip 「a decision that
/// held because this value was smaller than the keepalive interval」; `ws_write_deadline.rs`'s supervisor test narrows its own window by asserting the event arrival time against the wall clock — new tests do the same.
pub const PATIENCE: Duration = Duration::from_secs(10);

/// One registry. ★The dialer is chosen at the call site★ — a test setting socket options brings its own [`Dialer`] (`ws_write_deadline.rs`).
pub fn registry(dialer: Arc<dyn Dialer>, policy: Policy) -> Registry<TestWire> {
    Registry::new(Arc::new(TestWire), dialer, Arc::new(SystemClock), policy)
}

/// Waits through watch notifications until the state satisfies `want`; returns it. Panics: [`PATIENCE`] exceeded · the supervisor disappears first.
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
            "the supervisor vanished while waiting for {what} (last {:?})",
            peer.peer_state()
        ),
        Err(_) => panic!(
            "{what} did not happen within {PATIENCE:?} (now {:?})",
            peer.peer_state()
        ),
    }
}

/// Reads one handle's receiver up to the first item for which `pick` gives a value. Panics: [`PATIENCE`] exceeded · the receiver ends first.
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
        Ok(None) => panic!("the inbox ended before seeing {what}"),
        Err(_) => panic!("{what} did not come up within {PATIENCE:?}"),
    }
}
