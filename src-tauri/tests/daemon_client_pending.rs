//! Round trips that share one request id on the shell's daemon link — the transport's M1 rule (transport TRD §4-1)
//! seen through the shell's wire (`link::wire::ShellWire`) and its caller texts (`link::errors`). No daemon, no Tauri:
//! the daemon's side is a memory link.
//!
//! Run: `cargo test -p engram-dashboard --test daemon_client_pending` (no child process, so no `-- --test-threads=4` —
//! the rule is `.claude/skill-bindings/qa.md` 「모듈별 명령」). CI runs this target in its own step
//! (`.github/workflows/ci.yml`).
//!
//! ## ★What it guards — an overlapping request id fails both and kills nothing★
//! The request id of `AgentCommand::Command { envelope }` is the outside caller's (the webview forwards it through
//! `forward_daemon_command`), so two round trips can share one. A reply cannot tell them apart, so the transport fails
//! both: the older waiter reads [`PENDING_SUPERSEDED`], the newer one a send failure, and the id is retired. The link
//! stays up — a crash here once cut every command until a restart (debug GUI measurement 2026-08-18).

use std::sync::Arc;

use engram_dashboard_lib::daemon_client::link::errors::{
    request_error_text, PENDING_SUPERSEDED, SEND_FAILED_PREFIX,
};
use engram_dashboard_lib::daemon_client::link::wire::{ShellIn, ShellWire};
use engram_dashboard_protocol::{AgentCommand, AgentEvent, RequestId};
use engram_dashboard_transport::testing::{
    settle, settle_until, ImmediateHandshake, ManualClock, MemoryEndpoint, MemoryNetwork,
};
use engram_dashboard_transport::{
    Address, Frame, Inbox, Peer, PeerId, PeerState, Policy, Registry, RequestError,
};
use tokio::task::JoinHandle;

/// A live connection on the shell wire — `endpoint` is the daemon's side; the other fields keep the link alive (a
/// dropped inbox ends the peer).
struct Link {
    peer: Peer<ShellWire>,
    endpoint: Arc<MemoryEndpoint>,
    _inbox: Inbox<ShellWire>,
    _registry: Registry<ShellWire>,
    _net: MemoryNetwork,
    _clock: ManualClock,
}

async fn live() -> Link {
    let net = MemoryNetwork::new();
    let clock = ManualClock::new();
    let registry = Registry::new(
        Arc::new(ShellWire),
        net.dialer(),
        clock.handle(),
        Policy::default(),
    );
    let (peer, inbox) = registry.add(
        PeerId::new("daemon"),
        Address::new("mem"),
        Arc::new(ImmediateHandshake),
    );
    settle_until("live", || peer.peer_state() == PeerState::Live).await;
    let endpoint = net.accept().expect("a link was opened");
    Link {
        peer,
        endpoint,
        _inbox: inbox,
        _registry: registry,
        _net: net,
        _clock: clock,
    }
}

fn list(request_id: RequestId) -> AgentCommand {
    AgentCommand::ListAgents { request_id }
}

fn ack(request_id: RequestId) -> Frame {
    Frame::Text(serde_json::to_string(&AgentEvent::Ack { request_id }).unwrap())
}

/// A round trip on its own task — as the shell's callers make it.
fn round_trip(link: &Link, cmd: AgentCommand) -> JoinHandle<Result<ShellIn, RequestError>> {
    let peer = link.peer.clone();
    tokio::spawn(async move { peer.request(cmd).await })
}

/// Awaits a task once it finished — the manual clock never fires the request timeout, so a bare await could hang.
async fn joined<T>(what: &str, task: JoinHandle<T>) -> T {
    settle_until(what, || task.is_finished()).await;
    task.await.expect("the task ran to its end")
}

/// The text the shell's caller gets for a failed round trip.
fn caller_text(result: Result<ShellIn, RequestError>) -> String {
    match result {
        Ok(reply) => panic!("expected a failure, got {reply:?}"),
        Err(e) => request_error_text(&e),
    }
}

fn is_ack(result: &Result<ShellIn, RequestError>, rid: RequestId) -> bool {
    matches!(result, Ok(ShellIn::Event(AgentEvent::Ack { request_id })) if *request_id == rid)
}

/// ★Core regression★ — two round trips on one request id: the older reads the superseded text, the newer a send
/// failure, and the link takes the next round trip.
#[tokio::test]
async fn a_duplicate_request_id_fails_both_round_trips_and_the_link_lives() {
    let link = live().await;
    let rid = RequestId::new();

    let old = round_trip(&link, list(rid));
    settle().await;
    let new = round_trip(&link, list(rid));

    assert_eq!(
        caller_text(joined("the older round trip", old).await),
        PENDING_SUPERSEDED
    );
    let newer = caller_text(joined("the newer round trip", new).await);
    assert!(
        newer.starts_with(&format!("{SEND_FAILED_PREFIX}: ")),
        "{newer}"
    );

    let fresh = RequestId::new();
    let next = round_trip(&link, list(fresh));
    settle_until("the next round trip is written", || {
        link.endpoint
            .frames()
            .iter()
            .any(|frame| matches!(frame, Frame::Text(t) if t.contains(&fresh.0.to_string())))
    })
    .await;
    link.endpoint.push(ack(fresh));
    assert!(is_ack(&joined("the next round trip", next).await, fresh));
}

/// The collided id is retired: the daemon's late reply to it wakes no one, and a round trip that reuses it is refused
/// at once.
#[tokio::test]
async fn a_collided_request_id_stays_retired_and_its_late_reply_wakes_no_one() {
    let link = live().await;
    let rid = RequestId::new();
    let old = round_trip(&link, list(rid));
    settle().await;
    let new = round_trip(&link, list(rid));
    joined("the older round trip", old).await.unwrap_err();
    joined("the newer round trip", new).await.unwrap_err();

    link.endpoint.push(ack(rid));
    settle().await;
    assert_eq!(link.peer.peer_state(), PeerState::Live);

    let again = joined("a reuse of the id", round_trip(&link, list(rid))).await;
    let text = caller_text(again);
    assert!(
        text.starts_with(&format!("{SEND_FAILED_PREFIX}: ")),
        "{text}"
    );
}

/// Different ids do not touch each other — each round trip is woken by its own reply only.
#[tokio::test]
async fn distinct_request_ids_coexist() {
    let link = live().await;
    let (a, b) = (RequestId::new(), RequestId::new());
    let wait_a = round_trip(&link, list(a));
    let wait_b = round_trip(&link, list(b));
    let mut written = 0;
    settle_until("both are written", || {
        written += link.endpoint.frames().len();
        written >= 2
    })
    .await;

    link.endpoint.push(ack(a));
    assert!(is_ack(&joined("a", wait_a).await, a));
    settle().await;
    assert!(!wait_b.is_finished(), "b still waits for its own reply");
    link.endpoint.push(ack(b));
    assert!(is_ack(&joined("b", wait_b).await, b));
}
