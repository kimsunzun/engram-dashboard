//! The daemon connection driver on the transport crate (TRD S21 trd-3 §6-2 U3 — working name `connection_v2`).
//!
//! ★Not in production yet★ — only the test constructor `DaemonClient::new_on_link` builds it, so the module is
//! compiled for tests only; 3-2 U4 switches the production constructor over. The old driver (`connection.rs`) and its
//! suite stay as they are.
//!
//! One actor task per client owns the routing: the registry's dialer registrations, [`RouteState`] and the inbox pumps
//! all meet on it, and it is the only place that applies what [`route`] returns (connection state · frontend
//! broadcasts · replay events · output frames). Callers' round trips run on the callers' tasks through a registry
//! clone, never on the actor.
//!
//! - ★The actor never awaits anything that waits on an inbox★ (transport invariant 1, transitively — see `link::pump`):
//!   no `Peer::request`, no `Peer::close_after_flush` future, no `Registry::close_all`. Its awaits are its two
//!   channels and the replay deadline timer on the injected clock; it writes with `notify_in` only.
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
//!   actor delayed past the first backoff lets one more dial out before the stop lands. A first connect shows its first
//!   failure as `Down` at once and keeps retrying in the background (`link::route`).
//! - ★Every decision that a close must order runs on the actor★ — "already connected" and "was this connect overtaken
//!   by a close" are answered there, in channel order after any earlier `Close`. A close bumps the close epoch at once;
//!   a connect / ensure carries the epoch it began under, and an add older than the last close builds nothing (a close
//!   during `connect`'s daemon search wins).
//! - `connect` / `ensure` resolve on their handle's first answer — `Connected`, or its first failure. A connect that a
//!   re-add overtakes gets the new handle's answer; a close before the answer ends it with `ClosedBeforeHello`.
//! - ★Subscribe · Unsubscribe · RequestReplay go through this one actor, in order, onto one peer queue★ (TRD §6-2 U3 —
//!   master re-check A1). A replay request checks the window routing when the actor takes it, so an `Unsubscribe`
//!   queued before it is written first and the check sees the routing that `Unsubscribe` came from. The `Subscribe`
//!   of a merged request is checked again when the one before it resolves.
//! - ★Replay flights belong to the routed connection★ (`LinkUp` … `LinkDown`): every write goes out with `notify_in`
//!   on that connection only, and `LinkDown` clears the flights. A replay `Subscribe` that cannot be written, or an
//!   output frame too short to read, drops that connection (`Peer::drop_link_in` — bound to it, back to the backoff);
//!   the actor clears its flights at once and handles none of its later output frames or driver events (its
//!   broadcasts and lifecycle events still pass).
//!
//! ★Not wired yet★ (3-2 U3c and the rest of U3): daemon command requests and their outcomes, broken command replies,
//! command registration on connect, the usage interest. Their routed items are dropped here (logged at debug); a
//! client built on this driver is for driver tests only until then.
// ADR-0046
// ADR-0310
// ADR-0346

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use engram_dashboard_protocol::{AgentCommand, AgentEvent, AgentId};
use engram_dashboard_transport::{
    Address, Clock, ConnectFailure, Dialer, DisconnectCause, Generation, Incoming, Peer, Policy,
    Registry, RejectKind, TransportEvent,
};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, watch};

use super::connection::{
    apply_replay_event, known_epoch, HandshakeError, ReplayFollowUp, REPLAY_DEADLINE,
    REPLAY_DEADLINE_TICK, SEND_FAILED_PREFIX,
};
use super::events::{ConnectionStateEvent, DaemonEvents};
use super::frame_relay::{relay_binary_frame, FrameRelay, UnknownTagLog};
use super::link::dialer::LocalDaemonDialer;
use super::link::endpoint::EndpointSource;
use super::link::errors::{request_error_text, require_live, send_error_text};
use super::link::pump::{spawn_pump, Routable};
use super::link::route::{
    emit_broadcast, refusal_message, route, route_end, LinkNotice, RouteState, Routed,
};
use super::link::wire::{ShellIn, ShellWire};
use super::link::{DaemonKey, HandleToken};
use super::protocol_state::{self, CommandFailure, SubState};
use super::replay_flight::{encode_marker_frame, ReplayFlightSet};
use super::{
    AgentNamesSink, ConnectionState, DEFAULT_CONNECT_TIMEOUT, NOT_CONNECTED, NO_REQUEST_ID,
    REPLAY_UNSENT,
};
use crate::output_channel::{send_to_windows, WindowChannelRegistry};
use crate::output_router::{OutputRouter, WindowLabel};

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

/// Where the actor hands what it routes.
pub(crate) struct Ports {
    pub events: Arc<dyn DaemonEvents>,
    pub names: Arc<dyn AgentNamesSink>,
    /// Which windows show which agent — the replay request check and every output frame read it.
    pub router: Arc<OutputRouter>,
    /// The windows' output channels. Output frames and replay markers go through the same channel so the frontend
    /// sees them in arrival order (ADR-0046).
    pub windows: WindowChannelRegistry,
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
    /// The pumps' channel into the actor — tests feed a replaced handle's tail through it.
    #[cfg(test)]
    routable_tx: mpsc::Sender<Routable>,
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
    /// Write `Unsubscribe` on the open connection. With none open there is nothing to undo — the daemon drops a
    /// connection's subscriptions with it.
    Unsubscribe {
        agent_id: AgentId,
    },
    /// Start the agent's replay or join the one in flight (single flight — ADR-0046 M1); `reply` gets the generation
    /// assigned (a joined request writes nothing and still gets one), or the old driver's text when no generation can
    /// be served: no open connection, no window shows the agent, or its `Subscribe` could not be written.
    RequestReplay {
        agent_id: AgentId,
        reply: oneshot::Sender<Result<u64, String>>,
    },
    /// The key's current handle.
    #[cfg(test)]
    CurrentHandle {
        reply: oneshot::Sender<Option<HandleToken>>,
    },
}

impl LinkClient {
    /// Starts the actor on `rt`. `inner` dials the resolved daemon address (the transport's ws dialer in production,
    /// a memory network in tests); `clock` drives the registry, the failure log and the replay deadlines.
    pub(crate) fn start(
        rt: &Handle,
        inner: Arc<dyn Dialer>,
        source: Arc<dyn EndpointSource>,
        clock: Arc<dyn Clock>,
        ports: Ports,
    ) -> Self {
        let (client, driver, cmd_rx, routable_rx) = Self::build(rt, inner, source, clock, ports);
        rt.spawn(driver.run(cmd_rx, routable_rx));
        client
    }

    /// The client and its actor, not yet running.
    fn build(
        rt: &Handle,
        inner: Arc<dyn Dialer>,
        source: Arc<dyn EndpointSource>,
        clock: Arc<dyn Clock>,
        ports: Ports,
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
        let Ports {
            events,
            names,
            router,
            windows,
        } = ports;
        let client = Self {
            key: key.clone(),
            rt: rt.clone(),
            source: source.clone(),
            registry: registry.clone(),
            #[cfg(test)]
            dialer: dialer.clone(),
            #[cfg(test)]
            routable_tx: routable_tx.clone(),
            closes: closes.clone(),
            cmd_tx,
            state_rx,
        };
        let driver = Driver {
            key,
            source,
            registry,
            dialer,
            route: RouteState::new(clock.clone()),
            clock,
            routable_tx,
            registrations: HashMap::new(),
            current: None,
            first: None,
            closes,
            state_tx,
            events,
            names,
            router,
            windows,
            open: None,
            subs: HashMap::new(),
            flight: ReplayFlightSet::new(REPLAY_DEADLINE),
            unknown_tags: UnknownTagLog::default(),
            dropping: None,
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

    /// Queues the unsubscribe on the actor, behind every earlier unsubscribe and replay request. Never blocks — the
    /// layout calls it under its lock.
    pub(crate) fn unsubscribe(&self, agent_id: AgentId) {
        let _ = self.cmd_tx.send(DriverCmd::Unsubscribe { agent_id });
    }

    /// The generation the frontend fences this replay with (ADR-0046 M1). `Err` = no open connection
    /// ([`NOT_CONNECTED`]), or no window shows the agent / its `Subscribe` could not be written ([`REPLAY_UNSENT`]).
    pub(crate) async fn request_replay(&self, agent_id: AgentId) -> Result<u64, String> {
        let (reply, answer) = oneshot::channel();
        if self
            .cmd_tx
            .send(DriverCmd::RequestReplay { agent_id, reply })
            .is_err()
        {
            return Err(NOT_CONNECTED.to_owned());
        }
        answer
            .await
            .unwrap_or_else(|_| Err(REPLAY_UNSENT.to_owned()))
    }
}

/// The actor — see the module doc.
struct Driver {
    key: DaemonKey,
    source: Arc<dyn EndpointSource>,
    registry: Registry<ShellWire>,
    dialer: Arc<LocalDaemonDialer>,
    route: RouteState,
    clock: Arc<dyn Clock>,
    /// Cloned into every pump.
    routable_tx: mpsc::Sender<Routable>,
    /// The dial address of every handle whose inbox has not ended.
    registrations: HashMap<HandleToken, Address>,
    /// The newest handle — the key's current one. `None` after a user close or a refusal stop, so a late event of
    /// that handle cannot act on the key again.
    current: Option<HandleToken>,
    /// The current handle and the callers waiting for its first answer — callers of a handle it replaced included.
    first: Option<(
        HandleToken,
        Vec<oneshot::Sender<Result<(), HandshakeError>>>,
    )>,
    closes: Arc<AtomicU64>,
    state_tx: watch::Sender<ConnectionState>,
    events: Arc<dyn DaemonEvents>,
    names: Arc<dyn AgentNamesSink>,
    router: Arc<OutputRouter>,
    windows: WindowChannelRegistry,
    /// The connection routed as `LinkUp` and not yet as `LinkDown` — the one replay flights and callers' writes
    /// belong to.
    open: Option<Generation>,
    /// The last known epoch of each agent (ADR-0046 T5). Kept across connections, as the old driver kept it across
    /// its reconnects.
    subs: HashMap<AgentId, SubState>,
    /// Single-flight replay bookkeeping. Its generation counters live as long as the actor; its flights end with
    /// their connection.
    flight: ReplayFlightSet,
    /// Unknown output tags already warned about on the open connection.
    unknown_tags: UnknownTagLog,
    /// A connection this actor decided to drop and whose `LinkDown` has not come yet — its later output frames and
    /// driver events are not handled (`drop_connection`); its broadcasts and lifecycle events still pass.
    dropping: Option<Generation>,
}

impl Driver {
    async fn run(
        mut self,
        mut cmd_rx: mpsc::UnboundedReceiver<DriverCmd>,
        mut routable_rx: mpsc::Receiver<Routable>,
    ) {
        let mut sweep = self.clock.sleep(REPLAY_DEADLINE_TICK);
        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => match cmd {
                    Some(DriverCmd::IsLive { reply }) => {
                        let _ = reply.send(self.is_live());
                    }
                    Some(DriverCmd::Add { closes, first }) => self.add(closes, first),
                    Some(DriverCmd::Close) => self.close(),
                    Some(DriverCmd::Unsubscribe { agent_id }) => self.unsubscribe(agent_id),
                    Some(DriverCmd::RequestReplay { agent_id, reply }) => {
                        let _ = reply.send(self.request_replay(agent_id));
                    }
                    #[cfg(test)]
                    Some(DriverCmd::CurrentHandle { reply }) => {
                        let _ = reply.send(self.current);
                    }
                    None => break,
                },
                // Never `None` — `self.routable_tx` is a sender.
                Some(routable) = routable_rx.recv() => self.on_routable(routable),
                () = &mut sweep => {
                    self.sweep_replay_deadlines();
                    sweep = self.clock.sleep(REPLAY_DEADLINE_TICK);
                }
            }
        }
        tracing::debug!(peer = %self.key.peer_id(), "the daemon client is gone — closing its link");
        // Dropping `routable_rx` with the actor makes every pump close its inbox and drain it (`link::pump`).
        self.registry.remove(self.key.peer_id());
        for address in self.registrations.values() {
            self.dialer.unregister(address);
        }
    }

    /// The current handle's peer is live, or its connection is still open as routed. Routing trails the peer (the
    /// supervisor publishes `Live` before it queues `Connected`), so a connection that ended but whose `Disconnected`
    /// is not routed yet still counts — a connect then answers `Ok` instead of re-adding behind its loss.
    fn is_live(&self) -> bool {
        self.current
            .is_some_and(|token| self.route.is_open(&self.key, token))
            || self
                .registry
                .get(self.key.peer_id())
                .is_some_and(|peer| peer.peer_state().is_live())
    }

    /// The link does not reconnect; a late item of the handle no longer counts as the current one's (no refusal
    /// stops it, no notice follows the close). Callers still waiting for the first answer get `ClosedBeforeHello` now —
    /// a later handle's answer is not theirs.
    fn close(&mut self) {
        self.current = None;
        self.registry.remove(self.key.peer_id());
        if let Some((_, waiting)) = self.first.take() {
            for caller in waiting {
                let _ = caller.send(Err(HandshakeError::ClosedBeforeHello));
            }
        }
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
        // Callers of the replaced handle had no answer yet (no close came — it would have answered them): this handle
        // carries on their connect.
        let mut waiting = self
            .first
            .take()
            .map(|(_, waiting)| waiting)
            .unwrap_or_default();
        waiting.push(first);
        self.first = Some((token, waiting));
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
                for caller in waiting {
                    // The caller may have stopped waiting.
                    let _ = caller.send(answer.clone());
                }
            }
        }
    }

    /// The open connection and the peer to write on — `None` before `LinkUp`, after `LinkDown`, and once a close
    /// removed the peer.
    fn open_peer(&self) -> Option<(Peer<ShellWire>, Generation)> {
        let generation = self.open?;
        let peer = self.registry.get(self.key.peer_id())?;
        Some((peer, generation))
    }

    fn unsubscribe(&self, agent_id: AgentId) {
        let Some((peer, generation)) = self.open_peer() else {
            tracing::debug!(
                peer = %self.key.peer_id(),
                %agent_id,
                "unsubscribe: no open daemon connection — nothing to undo"
            );
            return;
        };
        if let Err(e) = peer.notify_in(generation, AgentCommand::Unsubscribe { agent_id }) {
            tracing::debug!(
                peer = %self.key.peer_id(),
                %agent_id,
                generation = generation.0,
                error = %e,
                "unsubscribe: not written"
            );
        }
    }

    fn request_replay(&mut self, agent_id: AgentId) -> Result<u64, String> {
        let Some((peer, generation)) = self.open_peer() else {
            return Err(NOT_CONNECTED.to_owned());
        };
        // The window check — its reason is the old driver's (`connection.rs` `RequestReplay` arm); running it here,
        //   when the actor takes the request, is what orders it after an earlier `Unsubscribe` (module doc).
        if self.router.targets(agent_id).is_empty() {
            tracing::debug!(
                peer = %self.key.peer_id(),
                %agent_id,
                "replay request: no window shows the agent — no Subscribe"
            );
            return Err(REPLAY_UNSENT.to_owned());
        }
        let epoch = known_epoch(&self.subs, agent_id);
        let outcome = self.flight.request_replay(agent_id, self.clock.now());
        tracing::debug!(
            peer = %self.key.peer_id(),
            %agent_id,
            gen = outcome.generation,
            send_now = outcome.send_now,
            "replay request"
        );
        if outcome.send_now && !self.write_subscribe(&peer, generation, agent_id, epoch) {
            return Err(REPLAY_UNSENT.to_owned());
        }
        Ok(outcome.generation)
    }

    /// Writes a replay `Subscribe` on connection `generation`; returns whether it went into the peer queue. Its
    /// generation is already in flight, and a `Subscribe` that never went out gets no answer — the slot would stay
    /// taken and every later request of the agent would merge into it. So a failed write drops the connection, which
    /// clears every flight (`drop_connection` — the old driver's rule, `connection.rs` `RequestReplay` arm).
    fn write_subscribe(
        &mut self,
        peer: &Peer<ShellWire>,
        generation: Generation,
        agent_id: AgentId,
        epoch: Option<u32>,
    ) -> bool {
        let subscribe = AgentCommand::Subscribe {
            agent_id,
            epoch,
            after_seq: None,
        };
        match peer.notify_in(generation, subscribe) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(
                    peer = %self.key.peer_id(),
                    %agent_id,
                    generation = generation.0,
                    error = %e,
                    "a replay Subscribe was not written"
                );
                self.drop_connection(generation, DisconnectCause::WriteFailed);
                false
            }
        }
    }

    /// Ends connection `generation` as a lost link (`Peer::drop_link_in`): the handle stays, `Reconnecting` follows
    /// from the `Disconnected`, and the next dial waits the usual backoff — a daemon that keeps sending what made us
    /// drop cannot drive a reconnect loop. ★Bound to that connection★: when it already ended, the transport does nothing
    /// (its `LinkDown` is on the way) and never drops a newer connection.
    ///
    /// ★The connection is over for this actor at once★, as the old driver left its loop there: its flights are cleared
    /// now and `open` goes, and its output frames and replay events still in the inbox (or salvaged before its
    /// `Disconnected`) are not relayed until its `LinkDown` — a later `ReplayComplete` would mark a replay complete
    /// that is missing the frame we could not read.
    ///
    /// The actor-side part runs with or without a peer — after a close the registry no longer holds one, but the tail of
    /// the connection is still routed until its `LinkDown`.
    // A drop lost in the transport would leave a live connection blacked out here; `link_policy` keeps
    //   `QueueFullPolicy::Disconnect`, under which a drop that meets a full queue drops the link anyway.
    fn drop_connection(&mut self, generation: Generation, cause: DisconnectCause) {
        tracing::info!(
            peer = %self.key.peer_id(),
            generation = generation.0,
            ?cause,
            "dropping the daemon connection if it is still the live one — the link reconnects after its backoff"
        );
        if let Some(peer) = self.registry.get(self.key.peer_id()) {
            peer.drop_link_in(generation, cause);
        }
        self.dropping = Some(generation);
        if self.open == Some(generation) {
            self.open = None;
            self.flight.on_disconnect();
        }
    }

    /// An item of a connection being dropped — not relayed.
    fn is_dropping(&self, generation: Generation, what: &'static str) -> bool {
        let dropping = self.dropping == Some(generation);
        if dropping {
            tracing::debug!(
                peer = %self.key.peer_id(),
                generation = generation.0,
                what,
                "an item of a daemon connection being dropped — not relayed"
            );
        }
        dropping
    }

    /// The `Subscribe` of a merged replay request, released by the resolution of the one before it. ★The window check
    /// again★ — the agent's last window may have closed since the request merged, and its `Unsubscribe` is already
    /// written: a `Subscribe` now would leak the subscription (the A1 shape). Then nothing is written and the slot is
    /// released as a refusal releases it — no answer can come for a `Subscribe` that never went out.
    fn write_follow_up(&mut self, generation: Generation, agent_id: AgentId, epoch: Option<u32>) {
        if self.router.targets(agent_id).is_empty() {
            tracing::debug!(
                peer = %self.key.peer_id(),
                %agent_id,
                "no window shows the agent any more — its merged replay request is dropped"
            );
            // Nothing is merged behind it — the resolution just promoted it; its failure marker has no window to go to.
            let _ = self.flight.on_refused(agent_id, self.clock.now());
            return;
        }
        match self.registry.get(self.key.peer_id()) {
            Some(peer) => {
                self.write_subscribe(&peer, generation, agent_id, epoch);
            }
            // Closed — the `LinkDown` before its `Closed` clears the flight.
            None => tracing::debug!(
                peer = %self.key.peer_id(),
                generation = generation.0,
                "the next replay Subscribe has no peer to go to — the link was closed"
            ),
        }
    }

    /// A replay event of connection `generation` (`SubscribeAck` · `ReplayComplete` · `SubscribeFailed`); the
    /// follow-up `Subscribe` goes out on that connection.
    fn on_driver_event(&mut self, generation: Generation, event: AgentEvent) {
        let windows = &self.windows;
        let mut deliver =
            |labels: &[WindowLabel], bytes: &[u8]| send_to_windows(windows, labels, bytes);
        let follow_up = apply_replay_event(
            &event,
            &mut self.flight,
            &mut self.subs,
            &self.router,
            self.clock.now(),
            &mut deliver,
        );
        match follow_up {
            ReplayFollowUp::Handled(None) => {}
            ReplayFollowUp::Handled(Some(AgentCommand::Subscribe {
                agent_id, epoch, ..
            })) => self.write_follow_up(generation, agent_id, epoch),
            ReplayFollowUp::Handled(Some(other)) => tracing::warn!(
                peer = %self.key.peer_id(),
                ?other,
                "a replay follow-up that is not a Subscribe — not written"
            ),
            ReplayFollowUp::NotHandled => tracing::debug!(
                peer = %self.key.peer_id(),
                event = event_kind(&event),
                "a daemon event this driver does not handle yet — dropped"
            ),
        }
    }

    /// One output frame of connection `generation` to the windows that show its agent (`frame_relay`).
    fn relay_output(&mut self, generation: Generation, bytes: &[u8]) {
        let windows = &self.windows;
        let mut deliver =
            |labels: &[WindowLabel], bytes: &[u8]| send_to_windows(windows, labels, bytes);
        let relayed = relay_binary_frame(
            bytes,
            &mut self.flight,
            &mut self.subs,
            &self.router,
            &mut self.unknown_tags,
            self.clock.now(),
            &mut deliver,
        );
        if relayed == FrameRelay::Disconnect {
            self.drop_connection(generation, DisconnectCause::ReadFailed);
        }
    }

    /// Fails every replay that made no progress within [`REPLAY_DEADLINE`] — the old driver's sweep: one failure
    /// marker each, the slot kept (why = `ReplayFlightSet::check_deadlines`).
    fn sweep_replay_deadlines(&mut self) {
        for (agent_id, marker) in self.flight.check_deadlines(self.clock.now()) {
            tracing::warn!(
                peer = %self.key.peer_id(),
                %agent_id,
                gen = marker.generation,
                "a replay made no progress before its deadline — failure marker sent; its Subscribe slot stays taken \
                 until a late answer or the connection ends"
            );
            // Best effort — a failure has no Ack to take the epoch from; the view does not fence failure markers.
            let epoch = known_epoch(&self.subs, agent_id).unwrap_or(0);
            let frame = encode_marker_frame(agent_id, epoch, marker);
            send_to_windows(&self.windows, &self.router.targets(agent_id), &frame);
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
                Routed::LinkUp { generation } => self.open = Some(generation),
                // The old driver's per-socket reset: flights cleared without markers (the frontend re-requests on its
                //   `connected` edge), unknown tags warned again on the next connection.
                Routed::LinkDown { generation } => {
                    self.open = None;
                    self.flight.on_disconnect();
                    self.unknown_tags = UnknownTagLog::default();
                    if self.dropping == Some(generation) {
                        self.dropping = None;
                    }
                }
                Routed::ToDriver { generation, .. } if self.is_dropping(generation, "event") => {}
                Routed::ToDriver { generation, event } => self.on_driver_event(generation, event),
                Routed::Output { generation, .. } if self.is_dropping(generation, "output") => {}
                Routed::Output { generation, bytes } => self.relay_output(generation, &bytes),
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
