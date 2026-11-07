//! The consumer of one handle's inbox (TRD S21 trd-3 §6-2 U2 · D24 ②): forwards every item, tagged with its key and
//! handle, to the driver's channel, drains the inbox to its end (transport invariant 5) and then reports the end.
//!
//! - ★Forward only★ — routing runs on the driver's task ([`super::route`]), one inbox per handle, so items of one
//!   handle keep their order and an old handle's tail can be told apart by its token.
//! - ★Backpressure is the point★ — while the driver lags, `send` waits, the inbox fills and the transport stops reading
//!   this daemon instead of losing frames. So the driver task never awaits anything that needs this inbox drained
//!   (transport invariant 1): `Peer::request` (the reply could be stuck behind this pump), a `Peer::close_after_flush`
//!   future or `Registry::close_all` (their end waits for the drain).
//! - When the driver's receiver is gone — also while the inbox is quiet — the pump closes the inbox (the supervisor
//!   folds at once, though the registry still holds a `Peer` clone) and discards what is left until the end.
// ADR-0310

use engram_dashboard_transport::{Inbox, Incoming};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::wire::ShellWire;
use super::{DaemonKey, HandleToken};

/// What the pump hands the driver.
pub enum Routable {
    /// One inbox item and the key and handle it came from — for [`super::route::route`].
    Item {
        key: DaemonKey,
        token: HandleToken,
        item: Incoming<ShellWire>,
    },
    /// The handle's inbox ended (`None`); always the last one of the handle — for [`super::route::route_end`].
    Ended { key: DaemonKey, token: HandleToken },
}

/// Spawns the pump on the current tokio runtime. ★Spawn it in the same driver step as the `Registry::add` and
/// `RouteState::adopt` of `token`★ (see `adopt`). The task ends after the inbox's end, or once the driver is gone.
pub fn spawn_pump(
    mut inbox: Inbox<ShellWire>,
    key: DaemonKey,
    token: HandleToken,
    tx: mpsc::Sender<Routable>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let item = tokio::select! {
                item = inbox.recv() => item,
                () = tx.closed() => break,
            };
            let Some(item) = item else {
                // A gone driver needs no end report.
                let _ = tx.send(Routable::Ended { key, token }).await;
                return;
            };
            let item = Routable::Item {
                key: key.clone(),
                token,
                item,
            };
            if tx.send(item).await.is_err() {
                break;
            }
        }
        tracing::debug!(
            peer = %key.peer_id(),
            "the link driver is gone — closing this daemon's inbox"
        );
        inbox.close();
        while inbox.recv().await.is_some() {}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use engram_dashboard_protocol::AgentEvent;
    use engram_dashboard_transport::testing::{
        settle, settle_until, ImmediateHandshake, ManualClock, MemoryNetwork,
    };
    use engram_dashboard_transport::{Address, Frame, PeerState, Policy, Registry};

    use crate::daemon_client::link::wire::ShellIn;

    fn registry(net: &MemoryNetwork, clock: &ManualClock) -> Registry<ShellWire> {
        Registry::new(
            Arc::new(ShellWire),
            net.dialer(),
            clock.handle(),
            Policy::default(),
        )
    }

    fn list_frame() -> Frame {
        Frame::Text(
            serde_json::to_string(&AgentEvent::AgentListUpdated { agents: vec![] }).unwrap(),
        )
    }

    #[tokio::test]
    async fn the_pump_forwards_every_item_in_order_and_then_the_end() {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let registry = registry(&net, &clock);
        let key = DaemonKey::new("daemon");
        let token = HandleToken::mint();
        let (peer, inbox) = registry.add(
            key.peer_id().clone(),
            Address::new("mem"),
            Arc::new(ImmediateHandshake),
        );
        // Room for one item only — the pump has to wait on the driver and still lose nothing.
        let (tx, mut rx) = mpsc::channel(1);
        let pump = spawn_pump(inbox, key.clone(), token, tx);
        settle_until("live", || peer.peer_state() == PeerState::Live).await;
        let endpoint = net.accept().expect("a link was opened");
        for _ in 0..3 {
            endpoint.push(list_frame());
        }
        settle().await;
        peer.close();

        let mut kinds = Vec::new();
        for _ in 0..64 {
            settle().await;
            while let Ok(routable) = rx.try_recv() {
                let (k, t, kind) = match routable {
                    Routable::Item { key, token, item } => (
                        key,
                        token,
                        match item {
                            Incoming::Message(arrival) => match arrival.msg {
                                ShellIn::Event(_) => "event",
                                ShellIn::Output(_) => "output",
                            },
                            Incoming::Event(event) => event.kind(),
                        },
                    ),
                    Routable::Ended { key, token } => (key, token, "ended"),
                };
                assert_eq!((&k, t), (&key, token));
                kinds.push(kind);
            }
            if pump.is_finished() {
                break;
            }
        }
        assert!(
            pump.is_finished(),
            "the pump ends with the inbox: {kinds:?}"
        );
        assert_eq!(
            kinds,
            [
                "Connected",
                "event",
                "event",
                "event",
                "Disconnected",
                "Closed",
                "ended"
            ]
        );
    }

    #[tokio::test]
    async fn a_quiet_inbox_is_closed_when_the_driver_goes_away() {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let registry = registry(&net, &clock);
        let key = DaemonKey::new("daemon");
        let (peer, inbox) = registry.add(
            key.peer_id().clone(),
            Address::new("mem"),
            Arc::new(ImmediateHandshake),
        );
        let (tx, mut rx) = mpsc::channel(8);
        let pump = spawn_pump(inbox, key, HandleToken::mint(), tx);
        settle_until("live", || peer.peer_state() == PeerState::Live).await;
        settle_until("Connected was forwarded", || rx.try_recv().is_ok()).await;

        // Nothing arrives any more — the pump waits on an empty inbox when the driver goes.
        drop(rx);
        settle_until("the pump ends without another item", || pump.is_finished()).await;
        settle_until("the supervisor folds once its inbox is closed", || {
            peer.peer_state() == PeerState::Closed
        })
        .await;
    }

    #[tokio::test]
    async fn a_pump_blocked_on_a_full_channel_ends_when_the_driver_goes_away() {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let registry = registry(&net, &clock);
        let key = DaemonKey::new("daemon");
        let (peer, inbox) = registry.add(
            key.peer_id().clone(),
            Address::new("mem"),
            Arc::new(ImmediateHandshake),
        );
        // One slot, never read: `Connected` fills it and the pump waits in `send` with the next item.
        let (tx, rx) = mpsc::channel(1);
        let pump = spawn_pump(inbox, key, HandleToken::mint(), tx);
        settle_until("live", || peer.peer_state() == PeerState::Live).await;
        let endpoint = net.accept().expect("a link was opened");
        endpoint.push(list_frame());
        settle().await;
        assert!(!pump.is_finished(), "the pump waits on the full channel");

        drop(rx);
        settle_until("the pump ends once the driver is gone", || {
            pump.is_finished()
        })
        .await;
        settle_until("the supervisor folds once its inbox is closed", || {
            peer.peer_state() == PeerState::Closed
        })
        .await;
    }
}
