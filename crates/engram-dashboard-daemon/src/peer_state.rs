//! The peer-state broadcast to the app and the app's route for questions.
//!
//! Each shell connection says with [`AgentCommand::PeerSubscribe`](engram_dashboard_protocol::AgentCommand::PeerSubscribe) whether it
//!   has a PC list slot open (`subscribed` — the shell's reading of its layout, any tab of any window). This module keeps that flag per
//!   connection and:
//! - ★broadcasts★ the peer state — the same shape as `peer.list` — to the subscribed connections when one subscribes and whenever the
//!   peer link or the waiting requests change ([`PeerStateHub::nudge`]; the daemon assembly forwards `PeerLink::changes` and
//!   `Approvals::changes` here). One task builds and sends ([`PeerStateHub::run`]), so a connection's copies go out in order and the
//!   last one it got is the current state; nudges that come while a build runs fold into one more build;
//! - ★is the app route★ of the "ask the user" seam ([`AppRoute`]): questions go to the app while any connection is subscribed, else
//!   to the OS dialog — whether the slot is on screen does not matter (ADR-0410). Every change of that answer calls the source's
//!   `route_changed` (outside this module's lock), so the seam reroutes at once — the last connection that drops or closes its slot
//!   sends the open questions to the dialog.
//!
//! Locks: one short mutex over the per-connection entries. Nothing is built, sent or called back under it; [`AppRoute::takes_questions`]
//!   reads an atomic only (the seam calls it under its own lock).
//! The state is built on the blocking pool ([`PeerStateSource::state`] reads the peer store and the interface addresses).

// ADR-0394, ADR-0410

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use engram_dashboard_base::sync::lock;
use engram_dashboard_protocol::{AgentEvent, PeerState};
use tokio::sync::Notify;

use crate::ask_user::AppRoute;
use crate::conn_port::{ConnId, Frame, FrameSink};
use crate::connection_core::event_json;

/// Where the state comes from and whom a route change is told — production = the approvals (`link::peer::approvals`).
pub(crate) trait PeerStateSource: Send + Sync {
    /// The whole peer state as `peer.list` shows it. ★Blocking★ — called on the blocking pool.
    fn state(&self) -> PeerState;
    /// The app started or stopped taking the questions. Called outside this module's lock.
    fn route_changed(&self);
}

#[derive(Default)]
struct Entry {
    /// Released on detach (the connection's frame outlet must not outlive its `on_disconnect`).
    frames: Option<Arc<dyn FrameSink>>,
    subscribed: bool,
}

/// The per-connection peer-state subscriptions (module head).
#[derive(Default)]
pub struct PeerStateHub {
    entries: Mutex<HashMap<ConnId, Entry>>,
    /// Any connection subscribed — what the seam reads.
    present: AtomicBool,
    source: OnceLock<Arc<dyn PeerStateSource>>,
    wake: Notify,
}

impl PeerStateHub {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Plugs the source in once the approvals exist (they are made after the connection layer). A second call is ignored (logged).
    pub(crate) fn bind(&self, source: Arc<dyn PeerStateSource>) {
        if self.source.set(source).is_err() {
            tracing::error!("the peer-state source was bound twice — the first one stays");
        }
        self.nudge();
    }

    /// A connection attached — not subscribed. The only place an entry is made.
    pub(crate) fn attach(&self, conn: ConnId, frames: Arc<dyn FrameSink>) {
        lock(&self.entries).insert(
            conn,
            Entry {
                frames: Some(frames),
                ..Entry::default()
            },
        );
    }

    /// A connection dropped — its entry and frame outlet go before this returns; if it was the last one subscribed, the questions go
    ///   to the dialog. The only place an entry is erased.
    pub(crate) fn detach(&self, conn: ConnId) {
        let removed = lock(&self.entries).remove(&conn);
        drop(removed);
        self.reroute_if_changed();
    }

    /// `PeerSubscribe` from connection `conn` — replaces its flag. A connection already gone is not revived. A new subscription gets
    ///   the state at once (every subscriber gets a copy — the same state twice is harmless).
    pub(crate) fn replace(&self, conn: ConnId, subscribed: bool) {
        let newly = {
            let mut entries = lock(&self.entries);
            let Some(entry) = entries.get_mut(&conn) else {
                drop(entries);
                tracing::debug!(
                    conn,
                    "a peer subscription arrived after its connection dropped — ignored"
                );
                return;
            };
            let newly = subscribed && !entry.subscribed;
            entry.subscribed = subscribed;
            newly
        };
        self.reroute_if_changed();
        if newly {
            self.nudge();
        }
    }

    /// The peer state may have changed — one more build and send (folded with others that come meanwhile).
    pub(crate) fn nudge(&self) {
        self.wake.notify_one();
    }

    /// Builds and sends the state after each nudge, until the task is aborted (daemon shutdown). Nothing is built while no connection
    ///   is subscribed or no source is bound.
    pub(crate) async fn run(self: Arc<Self>) {
        loop {
            self.wake.notified().await;
            self.send_once().await;
        }
    }

    async fn send_once(&self) {
        let Some(source) = self.source.get().cloned() else {
            return;
        };
        if self.targets().is_empty() {
            return;
        }
        let state = match tokio::task::spawn_blocking(move || source.state()).await {
            Ok(state) => state,
            Err(e) => {
                tracing::warn!("the peer state could not be built: {e}");
                return;
            }
        };
        let Some(text) = event_json(&AgentEvent::PeerStateUpdated { state }) else {
            return;
        };
        // Read after the build: a connection that subscribed meanwhile gets this copy too.
        for frames in self.targets() {
            // A saturated queue closes that connection (the frame port's rule); the others still get theirs.
            if frames.try_send(Frame::Text(text.clone())).is_err() {
                tracing::debug!(
                    "a peer-state copy was not queued — that connection is saturated or closing"
                );
            }
        }
    }

    fn targets(&self) -> Vec<Arc<dyn FrameSink>> {
        lock(&self.entries)
            .values()
            .filter(|entry| entry.subscribed)
            .filter_map(|entry| entry.frames.clone())
            .collect()
    }

    /// Recomputes "subscribed somewhere"; on a change tells the source, outside the lock.
    fn reroute_if_changed(&self) {
        let (changed, now) = {
            let entries = lock(&self.entries);
            let now = entries.values().any(|entry| entry.subscribed);
            // Swapped under the entries lock, so two recomputes cannot store their answers out of order.
            (self.present.swap(now, Ordering::SeqCst) != now, now)
        };
        if changed {
            if now {
                tracing::info!("approval questions now go to the app — a PC list slot is open");
            } else {
                tracing::info!(
                    "approval questions now go to the OS dialog — no PC list slot is open"
                );
            }
            if let Some(source) = self.source.get() {
                source.route_changed();
            }
        }
    }
}

/// Nudges the hub on every tick of `changes` (the peer link's or the approvals' change counter) until its sender is gone or the task
///   is aborted.
pub(crate) async fn forward(
    hub: Arc<PeerStateHub>,
    mut changes: tokio::sync::watch::Receiver<u64>,
) {
    while changes.changed().await.is_ok() {
        hub.nudge();
    }
}

impl AppRoute for PeerStateHub {
    fn takes_questions(&self) -> bool {
        self.present.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests;
