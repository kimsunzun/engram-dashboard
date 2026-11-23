//! The peer-state hub with fake connections and a fake source: who gets the broadcast, and when the app route changes.

use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use tokio::sync::mpsc;

use super::*;
use crate::test_doubles::FakeFrameSink;

struct FakeSource {
    routes: AtomicUsize,
    builds: AtomicUsize,
}

impl FakeSource {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            routes: AtomicUsize::new(0),
            builds: AtomicUsize::new(0),
        })
    }

    fn routes(&self) -> usize {
        self.routes.load(Ordering::SeqCst)
    }
}

impl PeerStateSource for FakeSource {
    fn state(&self) -> PeerState {
        let n = self.builds.fetch_add(1, Ordering::SeqCst);
        PeerState {
            state: "ready".into(),
            reason: None,
            id: Some(format!("build-{n}")),
            addresses: vec![],
            network: None,
            endpoint: "off".into(),
            endpoint_error: None,
            pcs: vec![],
            knocks: vec![],
            requests: vec![],
            actions: vec![],
        }
    }

    fn route_changed(&self) {
        self.routes.fetch_add(1, Ordering::SeqCst);
    }
}

fn conn(hub: &PeerStateHub, id: ConnId) -> mpsc::Receiver<Frame> {
    let (tx, rx) = mpsc::channel(16);
    hub.attach(id, Arc::new(FakeFrameSink::new(tx)));
    rx
}

fn states(rx: &mut mpsc::Receiver<Frame>) -> Vec<PeerState> {
    let mut out = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        let Frame::Text(text) = frame else {
            panic!("only text frames are sent");
        };
        match serde_json::from_str::<AgentEvent>(&text).expect("an event") {
            AgentEvent::PeerStateUpdated { state } => out.push(state),
            other => panic!("only peer state is sent, got {other:?}"),
        }
    }
    out
}

#[tokio::test]
async fn the_state_goes_to_subscribed_connections_only() {
    let hub = PeerStateHub::new();
    hub.bind(FakeSource::new());
    let mut subscribed = conn(&hub, 1);
    let mut other = conn(&hub, 2);
    hub.replace(1, true);
    hub.send_once().await;
    assert_eq!(states(&mut subscribed).len(), 1);
    assert!(
        states(&mut other).is_empty(),
        "an unsubscribed connection gets nothing"
    );

    // Unsubscribing stops the copies.
    hub.replace(1, false);
    hub.send_once().await;
    assert!(states(&mut subscribed).is_empty());
}

#[tokio::test]
async fn nothing_is_built_while_nobody_subscribes() {
    let hub = PeerStateHub::new();
    let source = FakeSource::new();
    hub.bind(source.clone());
    let _rx = conn(&hub, 1);
    hub.send_once().await;
    assert_eq!(source.builds.load(Ordering::SeqCst), 0);
}

/// The id of the build a frame carries (`build-<n>`).
fn build_of(frame: Frame) -> usize {
    let Frame::Text(text) = frame else {
        panic!("only text frames are sent");
    };
    match serde_json::from_str::<AgentEvent>(&text).expect("an event") {
        AgentEvent::PeerStateUpdated { state } => state
            .id
            .and_then(|id| id.strip_prefix("build-").and_then(|n| n.parse().ok()))
            .expect("a build id"),
        other => panic!("only peer state is sent, got {other:?}"),
    }
}

/// The running task sends on a subscription and on every change, in order — each copy newer than the one before.
#[tokio::test]
async fn subscribing_and_each_change_send_the_current_state() {
    let hub = PeerStateHub::new();
    hub.bind(FakeSource::new());
    let task = tokio::spawn(hub.clone().run());
    let mut rx = conn(&hub, 1);
    hub.replace(1, true);
    let first = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("the state on subscribing")
        .expect("open");
    let first = build_of(first);

    hub.nudge();
    let second = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("the state on a change")
        .expect("open");
    let second = build_of(second);
    assert!(
        second > first,
        "the copy after a change is a newer build: {first} then {second}"
    );
    task.abort();
}

/// ADR-0410: the questions go to the app while any connection has a PC list slot open — on screen or not.
#[test]
fn the_app_route_holds_while_any_connection_is_subscribed() {
    let hub = PeerStateHub::new();
    let source = FakeSource::new();
    hub.bind(source.clone());
    let _a = conn(&hub, 1);
    let _b = conn(&hub, 2);
    assert!(!hub.takes_questions());

    hub.replace(1, true);
    assert!(hub.takes_questions());
    assert_eq!(
        source.routes(),
        1,
        "the seam is told when the app starts taking the questions"
    );

    // Another connection subscribing too changes nothing; one of two closing its slot changes nothing.
    hub.replace(2, true);
    assert_eq!(source.routes(), 1);
    hub.replace(2, false);
    assert!(hub.takes_questions());
    assert_eq!(source.routes(), 1);

    // The last slot closed → the seam is told; questions go to the dialog.
    hub.replace(1, false);
    assert!(!hub.takes_questions());
    assert_eq!(source.routes(), 2);
}

#[test]
fn a_dropped_connection_that_took_the_questions_reroutes_them() {
    let hub = PeerStateHub::new();
    let source = FakeSource::new();
    hub.bind(source.clone());
    let _a = conn(&hub, 1);
    hub.replace(1, true);
    assert_eq!(source.routes(), 1);
    hub.detach(1);
    assert!(!hub.takes_questions());
    assert_eq!(source.routes(), 2);

    // A late subscription for the dropped connection does not revive it.
    hub.replace(1, true);
    assert!(!hub.takes_questions());
    assert_eq!(source.routes(), 2);
}
