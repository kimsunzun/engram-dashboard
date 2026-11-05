//! The daemon connection driver on the transport crate (TRD S21 trd-3 §6-2 U3 — working name `connection_v2`).
//!
//! ★Not in production yet★ — only the test constructor `DaemonClient::new_on_link` builds it, so the module is
//! compiled for tests only; 3-2 U4 switches the production constructor over. The old driver (`connection.rs`) and its
//! suite stay as they are.
//!
//! One actor task per client owns the routing: the registry's dialer registrations, [`RouteState`] and the inbox pumps
//! all meet on it, and it is the only place that applies what [`route`] returns (connection state · frontend
//! broadcasts). Callers' round trips run on the callers' tasks through a registry clone, never on the actor.
//!
//! - ★The actor never awaits anything that waits on an inbox★ (transport invariant 1, transitively — see `link::pump`):
//!   no `Peer::request`, no `Peer::close_after_flush` future, no `Registry::close_all`. Its awaits are its two
//!   channels only.
//! - ★Every add is one actor step★ — `register` (a fresh dial address) + `adopt` + `Registry::add` + `spawn_pump`,
//!   with `adopt`'s output applied; a re-add of the key takes the same step. Every `Ended` gives its dial registration
//!   back, current or replaced handle.
//! - ★Routing is the only source of connection state★ — `close` removes the peer and shows nothing itself; the
//!   `Closed` that follows shows `Down`.
//! - ★Retries never run out★ — network failures retry without limit on the capped backoff ([`link_policy`] —
//!   ADR-0346). ★No refusal is retried★: terminal ones stop in the transport (the dialer's version check ·
//!   `ShellHandshake`), and a refusal the transport would retry (a refusal close code other than 4001 — `RejectKind::of`;
//!   1000 · 1001 · 4000 are not refusals and retry as network failures) stops here: the actor removes the peer when it
//!   routes that `ConnectFailed`. ★Best effort★ — the transport has already scheduled its next attempt by then, so an
//!   actor delayed past the first backoff lets one more dial out before the stop lands. A first connect shows its first failure as `Down` at once
//!   and keeps retrying in the background (`link::route`).
//! - ★Every decision that a close must order runs on the actor★ — "already connected" and "was this connect overtaken
//!   by a close" are answered there, in channel order after any earlier `Close`. A close bumps the close epoch at once;
//!   a connect / ensure carries the epoch it began under, and an add older than the last close builds nothing (a close
//!   during `connect`'s daemon search wins).
//! - `connect` / `ensure` resolve on their handle's first answer — `Connected`, or its first failure.
//!
//! ★Not wired yet★ (3-2 U3b and the rest of U3): the subscribe / replay actor and the output relay, daemon command
//! requests and their outcomes, command registration on connect, the usage interest. Their routed items are dropped
//! here (logged at debug); a client built on this driver is for driver tests only until then.
// ADR-0310
// ADR-0346

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use engram_dashboard_protocol::{AgentCommand, AgentEvent};
use engram_dashboard_transport::{
    Address, Clock, ConnectFailure, Dialer, Incoming, Policy, Registry, RejectKind, TransportEvent,
};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, watch};

use super::connection::{HandshakeError, SEND_FAILED_PREFIX};
use super::events::{ConnectionStateEvent, DaemonEvents};
use super::link::dialer::LocalDaemonDialer;
use super::link::endpoint::EndpointSource;
use super::link::errors::{request_error_text, require_live, send_error_text};
use super::link::pump::{spawn_pump, Routable};
use super::link::route::{
    emit_broadcast, refusal_message, route, route_end, LinkNotice, RouteState, Routed,
};
use super::link::wire::{ShellIn, ShellWire};
use super::link::{DaemonKey, HandleToken};
use super::protocol_state::{self, CommandFailure};
use super::{
    AgentNamesSink, ConnectionState, DEFAULT_CONNECT_TIMEOUT, NOT_CONNECTED, NO_REQUEST_ID,
};

/// The one daemon of stage 3 (D4 ① — the registry is built for N).
const DAEMON: &str = "local";

/// Room in the pumps → actor channel. A full channel is backpressure, not loss (`link::pump`).
const ROUTABLE_QUEUE: usize = 256;

/// The transport policy of the shell's daemon link: the crate defaults, except that network-class failures retry
/// without limit (ADR-0346 decision 1 — production never reaches `GaveUp(BudgetExhausted)`). The rest of the TRD §7
/// values land with 3-2 U4.
pub(crate) fn link_policy() -> Policy {
    let mut policy = Policy::default();
    policy.reconnect.max_attempts = u32::MAX;
    policy
}

/// The callers' side of the driver — `DaemonClient` holds one.
pub(crate) struct LinkClient {
    key: DaemonKey,
    rt: Handle,
    source: Arc<dyn EndpointSource>,
    registry: Registry<ShellWire>,
    /// The actor's dialer — held here for tests of its registrations.
    #[cfg(test)]
    dialer: Arc<LocalDaemonDialer>,
    /// Bumped by every `close` before it queues `Close` (module doc).
    closes: Arc<AtomicU64>,
    cmd_tx: mpsc::UnboundedSender<DriverCmd>,
    state_rx: watch::Receiver<ConnectionState>,
}

enum DriverCmd {
    /// Is the key's current peer live?
    IsLive {
        reply: oneshot::Sender<bool>,
    },
    /// Start a handle for the key — the first one or a re-add — unless the current peer is live (`first` gets `Ok`)
    /// or a close came after `closes` was read (`first` gets an error). Otherwise `first` gets the new handle's first
    /// answer.
    Add {
        closes: u64,
        first: oneshot::Sender<Result<(), HandshakeError>>,
    },
    Close,
}

impl LinkClient {
    /// Starts the actor on `rt`. `inner` dials the resolved daemon address (the transport's ws dialer in production,
    /// a memory network in tests); `clock` drives the registry and the failure log.
    pub(crate) fn start(
        rt: &Handle,
        inner: Arc<dyn Dialer>,
        source: Arc<dyn EndpointSource>,
        clock: Arc<dyn Clock>,
        events: Arc<dyn DaemonEvents>,
        names: Arc<dyn AgentNamesSink>,
    ) -> Self {
        let (client, driver, cmd_rx, routable_rx) =
            Self::build(rt, inner, source, clock, events, names);
        rt.spawn(driver.run(cmd_rx, routable_rx));
        client
    }

    /// The client and its actor, not yet running.
    fn build(
        rt: &Handle,
        inner: Arc<dyn Dialer>,
        source: Arc<dyn EndpointSource>,
        clock: Arc<dyn Clock>,
        events: Arc<dyn DaemonEvents>,
        names: Arc<dyn AgentNamesSink>,
    ) -> (
        Self,
        Driver,
        mpsc::UnboundedReceiver<DriverCmd>,
        mpsc::Receiver<Routable>,
    ) {
        let key = DaemonKey::new(DAEMON);
        let dialer = Arc::new(LocalDaemonDialer::new(inner));
        let registry = Registry::new(
            Arc::new(ShellWire),
            dialer.clone(),
            clock.clone(),
            link_policy(),
        );
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (state_tx, state_rx) = watch::channel(ConnectionState::Down);
        let (routable_tx, routable_rx) = mpsc::channel(ROUTABLE_QUEUE);
        let closes = Arc::new(AtomicU64::new(0));
        let driver = Driver {
            key: key.clone(),
            source: source.clone(),
            registry: registry.clone(),
            dialer: dialer.clone(),
            route: RouteState::new(clock),
            routable_tx,
            registrations: HashMap::new(),
            current: None,
            first: None,
            closes: closes.clone(),
            state_tx,
            events,
            names,
        };
        let client = Self {
            key,
            rt: rt.clone(),
            source,
            registry,
            #[cfg(test)]
            dialer,
            closes,
            cmd_tx,
            state_rx,
        };
        (client, driver, cmd_rx, routable_rx)
    }

    /// The connection state routing last showed. ★Never `Connecting`★ — routing does not emit it.
    pub(crate) fn state_rx(&self) -> watch::Receiver<ConnectionState> {
        self.state_rx.clone()
    }

    /// Explicit connect (ADR-0021 — may start a daemon). A failed search still starts the handle — unless a close
    /// overtook this connect, then nothing is built: it reads `daemon.json` on every attempt and connects once a
    /// daemon is there, and the caller gets the search's error.
    pub(crate) async fn connect(&self) -> Result<(), HandshakeError> {
        let closes = self.closes.load(Ordering::SeqCst);
        if self.is_live().await {
            return Ok(());
        }
        tracing::info!(peer = %self.key.peer_id(), "connecting to the daemon (may start it)");
        let source = self.source.clone();
        let searched = match self
            .rt
            .spawn_blocking(move || source.ensure(DEFAULT_CONNECT_TIMEOUT))
            .await
        {
            Ok(found) => found.map(|_| ()),
            Err(e) => Err(format!("the daemon search task failed: {e}")),
        };
        let first = self.add(closes);
        match searched {
            Ok(()) => first.await,
            Err(e) => {
                tracing::warn!(
                    peer = %self.key.peer_id(),
                    error = %e,
                    "finding or starting the daemon failed — retrying daemon.json in the background"
                );
                Err(HandshakeError::Discovery(e))
            }
        }
    }

    /// Attach-only connect (ADR-0021 — never starts a daemon).
    pub(crate) async fn ensure(&self) -> Result<(), HandshakeError> {
        let closes = self.closes.load(Ordering::SeqCst);
        tracing::info!(peer = %self.key.peer_id(), "attaching to a running daemon");
        self.add(closes).await
    }

    /// The actor's answer — ordered after every `close` called before.
    async fn is_live(&self) -> bool {
        let (reply, answer) = oneshot::channel();
        if self.cmd_tx.send(DriverCmd::IsLive { reply }).is_err() {
            return false;
        }
        answer.await.unwrap_or(false)
    }

    /// Sends the add now and returns the wait for its first answer.
    fn add(&self, closes: u64) -> impl std::future::Future<Output = Result<(), HandshakeError>> {
        let (first, answer) = oneshot::channel();
        let sent = self.cmd_tx.send(DriverCmd::Add { closes, first });
        async move {
            if sent.is_err() {
                return Err(HandshakeError::TaskGone);
            }
            answer.await.unwrap_or(Err(HandshakeError::TaskGone))
        }
    }

    /// Explicit close — the link does not reconnect. Returns at once; the state follows through routing.
    ///
    /// ★Bump the epoch before queueing `Close`★ — the actor reads the epoch after it takes `Close`, so an `Add` that
    /// captured the old epoch is seen as stale. The other order lets an `Add` queued behind `Close` still read the old
    /// value and revive the link.
    pub(crate) fn close(&self) {
        tracing::info!(peer = %self.key.peer_id(), "closing the daemon link");
        self.closes.fetch_add(1, Ordering::SeqCst);
        let _ = self.cmd_tx.send(DriverCmd::Close);
    }

    /// One command round trip on the caller's task; failure texts are the old driver's (`link::errors`).
    pub(crate) async fn send_command(
        &self,
        cmd: AgentCommand,
    ) -> Result<AgentEvent, CommandFailure> {
        use CommandFailure::Local;

        if protocol_state::command_request_id(&cmd).is_none() {
            return Err(Local(NO_REQUEST_ID.to_owned()));
        }
        let Some(peer) = self.registry.get(self.key.peer_id()) else {
            return Err(Local(NOT_CONNECTED.to_owned()));
        };
        require_live(peer.peer_state()).map_err(Local)?;
        match peer.request(cmd).await {
            Ok(ShellIn::Event(event)) => protocol_state::classify_reply(Ok(event)),
            // `ShellWire::reply_tag` never tags an output frame, so none answers a request.
            Ok(ShellIn::Output(bytes)) => Err(Local(format!(
                "{SEND_FAILED_PREFIX}: an output frame answered the request ({} bytes)",
                bytes.len()
            ))),
            Err(e) => Err(Local(request_error_text(&e))),
        }
    }

    /// A command without a reply. Not live → dropped, as the old driver's closed command window (ADR-0195).
    pub(crate) fn send_fire_and_forget(&self, cmd: AgentCommand) {
        let peer = self
            .registry
            .get(self.key.peer_id())
            .filter(|peer| require_live(peer.peer_state()).is_ok());
        let Some(peer) = peer else {
            tracing::debug!(peer = %self.key.peer_id(), "fire-and-forget: the daemon link is not live — dropped");
            return;
        };
        if let Err(e) = peer.notify(cmd) {
            tracing::debug!(
                peer = %self.key.peer_id(),
                error = %send_error_text(&e),
                "fire-and-forget: not sent"
            );
        }
    }
}

/// The actor — see the module doc.
struct Driver {
    key: DaemonKey,
    source: Arc<dyn EndpointSource>,
    registry: Registry<ShellWire>,
    dialer: Arc<LocalDaemonDialer>,
    route: RouteState,
    /// Cloned into every pump.
    routable_tx: mpsc::Sender<Routable>,
    /// The dial address of every handle whose inbox has not ended.
    registrations: HashMap<HandleToken, Address>,
    /// The newest handle — the key's current one. `None` after a user close or a refusal stop, so a late event of
    /// that handle cannot act on the key again.
    current: Option<HandleToken>,
    /// The newest handle and the caller waiting for its first answer.
    first: Option<(HandleToken, oneshot::Sender<Result<(), HandshakeError>>)>,
    closes: Arc<AtomicU64>,
    state_tx: watch::Sender<ConnectionState>,
    events: Arc<dyn DaemonEvents>,
    names: Arc<dyn AgentNamesSink>,
}

impl Driver {
    async fn run(
        mut self,
        mut cmd_rx: mpsc::UnboundedReceiver<DriverCmd>,
        mut routable_rx: mpsc::Receiver<Routable>,
    ) {
        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => match cmd {
                    Some(DriverCmd::IsLive { reply }) => {
                        let _ = reply.send(self.is_live());
                    }
                    Some(DriverCmd::Add { closes, first }) => self.add(closes, first),
                    Some(DriverCmd::Close) => self.close(),
                    None => break,
                },
                // Never `None` — `self.routable_tx` is a sender.
                Some(routable) = routable_rx.recv() => self.on_routable(routable),
            }
        }
        tracing::debug!(peer = %self.key.peer_id(), "the daemon client is gone — closing its link");
        // Dropping `routable_rx` with the actor makes every pump close its inbox and drain it (`link::pump`).
        self.registry.remove(self.key.peer_id());
        for address in self.registrations.values() {
            self.dialer.unregister(address);
        }
    }

    /// The current handle is open as routed (the supervisor publishes `PeerState` after it queued `Connected`, so the
    /// routed state can be ahead), or its peer is live.
    fn is_live(&self) -> bool {
        self.current
            .is_some_and(|token| self.route.is_open(&self.key, token))
            || self
                .registry
                .get(self.key.peer_id())
                .is_some_and(|peer| peer.peer_state().is_live())
    }

    /// The link does not reconnect; a late item of the handle no longer counts as the current one's (no refusal
    /// stops it, no notice follows the close).
    fn close(&mut self) {
        self.current = None;
        self.registry.remove(self.key.peer_id());
    }

    fn add(&mut self, closes: u64, first: oneshot::Sender<Result<(), HandshakeError>>) {
        if closes != self.closes.load(Ordering::SeqCst) {
            tracing::info!(
                peer = %self.key.peer_id(),
                "a close came after this connect began — not connecting"
            );
            let _ = first.send(Err(HandshakeError::TaskGone));
            return;
        }
        if self.is_live() {
            let _ = first.send(Ok(()));
            return;
        }
        let token = HandleToken::mint();
        let (address, handshake) = self.dialer.register(&self.key, self.source.clone());
        let ended = self.route.adopt(self.key.clone(), token);
        let (_peer, inbox) =
            self.registry
                .add(self.key.peer_id().clone(), address.clone(), handshake);
        spawn_pump(inbox, self.key.clone(), token, self.routable_tx.clone());
        self.registrations.insert(token, address);
        self.current = Some(token);
        if let Some((_, replaced)) = self.first.replace((token, first)) {
            let _ = replaced.send(Err(HandshakeError::TaskGone));
        }
        self.apply(ended);
    }

    fn on_routable(&mut self, routable: Routable) {
        match routable {
            Routable::Item { key, token, item } => {
                let out = self.route_item(&key, token, item);
                self.apply(out);
            }
            Routable::Ended { key, token } => {
                if let Some(address) = self.registrations.remove(&token) {
                    self.dialer.unregister(&address);
                }
                self.answer_first_with(token, Err(HandshakeError::TaskGone));
                let out = route_end(&mut self.route, &key, token);
                self.apply(out);
            }
        }
    }

    /// What one inbox item becomes. A refusal the transport would retry stops the peer here, its notice ahead of what
    /// routing returns — the order of a terminal refusal (`Notice`, then `Down`).
    fn route_item(
        &mut self,
        key: &DaemonKey,
        token: HandleToken,
        item: Incoming<ShellWire>,
    ) -> Vec<Routed> {
        self.answer_first(token, &item);
        let mut out = Vec::new();
        if let Some(code) = self.retryable_refusal(token, &item) {
            self.stop_refused(code);
            out.push(Routed::Notice(LinkNotice::Refused { code }));
        }
        out.extend(route(&mut self.route, key, token, item));
        out
    }

    /// The close code of a refusal the transport would retry, on the current handle (ADR-0346 — none is retried here).
    fn retryable_refusal(
        &self,
        token: HandleToken,
        item: &Incoming<ShellWire>,
    ) -> Option<engram_dashboard_transport::CloseCode> {
        match item {
            Incoming::Event(TransportEvent::ConnectFailed {
                cause:
                    ConnectFailure::Rejected {
                        kind: RejectKind::Retryable,
                        code,
                        ..
                    },
                ..
            }) if self.current == Some(token) => Some(*code),
            _ => None,
        }
    }

    /// Ends the current peer after a refusal, as the transport ends a terminal one; the `Closed` the removal produces
    /// shows `Down` (the caller sends the notice).
    fn stop_refused(&mut self, code: engram_dashboard_transport::CloseCode) {
        tracing::warn!(
            peer = %self.key.peer_id(),
            code = code.0,
            "the daemon refused the connection — not retrying"
        );
        self.close();
    }

    /// The first answer of the newest handle. Only that handle is the key's current one (`adopt` in the same step as
    /// the add), so the handle check is the routing's rule ①; items before it carry no older generation of it.
    fn answer_first(&mut self, token: HandleToken, item: &Incoming<ShellWire>) {
        let Incoming::Event(event) = item else {
            return;
        };
        let answer = match event {
            TransportEvent::Connected { .. } => Ok(()),
            TransportEvent::ConnectFailed {
                cause: ConnectFailure::Unreachable(e),
                ..
            } => Err(HandshakeError::Connect(e.to_string())),
            // The refusal's own text can be the daemon's — the fixed local text for its code instead (ADR-0342).
            TransportEvent::ConnectFailed {
                cause: ConnectFailure::Rejected { code, .. },
                ..
            }
            | TransportEvent::Rejected { code, .. } => Err(HandshakeError::AuthRejected(
                refusal_message(*code).to_owned(),
            )),
            TransportEvent::GaveUp { .. } | TransportEvent::Closed { .. } => {
                Err(HandshakeError::ClosedBeforeHello)
            }
            _ => return,
        };
        self.answer_first_with(token, answer);
    }

    fn answer_first_with(&mut self, token: HandleToken, answer: Result<(), HandshakeError>) {
        if self
            .first
            .as_ref()
            .is_some_and(|(newest, _)| *newest == token)
        {
            if let Some((_, waiting)) = self.first.take() {
                // The caller may have stopped waiting.
                let _ = waiting.send(answer);
            }
        }
    }

    fn apply(&mut self, out: Vec<Routed>) {
        for routed in out {
            match routed {
                Routed::State(state) => {
                    self.state_tx.send_replace(state);
                    match frontend_state(state) {
                        Some(event) => self.events.connection_state(event),
                        None => tracing::debug!(?state, "a state the frontend has no word for"),
                    }
                }
                Routed::Broadcast(event) => {
                    emit_broadcast(self.events.as_ref(), self.names.as_ref(), &event)
                }
                // The connection-failure surface is wired in 3-2 U4; routing already logged the refusal.
                Routed::Notice(notice) => {
                    tracing::debug!(peer = %self.key.peer_id(), ?notice, "daemon link notice")
                }
                // No per-connection state yet (module doc).
                Routed::LinkUp { .. } | Routed::LinkDown { .. } => {}
                Routed::ToDriver { event, .. } => tracing::debug!(
                    peer = %self.key.peer_id(),
                    event = event_kind(&event),
                    "a daemon event this driver does not handle yet — dropped"
                ),
                // Per frame — not logged.
                Routed::Output { .. } => {}
            }
        }
    }
}

fn frontend_state(state: ConnectionState) -> Option<ConnectionStateEvent> {
    match state {
        ConnectionState::Connected => Some(ConnectionStateEvent::Connected),
        ConnectionState::Reconnecting => Some(ConnectionStateEvent::Reconnecting),
        ConnectionState::Down => Some(ConnectionStateEvent::Down),
        ConnectionState::Connecting => None,
    }
}

fn event_kind(event: &AgentEvent) -> &'static str {
    match event {
        AgentEvent::SubscribeAck { .. } => "SubscribeAck",
        AgentEvent::ReplayComplete { .. } => "ReplayComplete",
        AgentEvent::SubscribeFailed { .. } => "SubscribeFailed",
        AgentEvent::CommandRequest { .. } => "CommandRequest",
        AgentEvent::CommandReply { .. } => "CommandReply",
        AgentEvent::UsageLimitsUpdated { .. } => "UsageLimitsUpdated",
        _ => "other",
    }
}

#[cfg(test)]
mod tests;
