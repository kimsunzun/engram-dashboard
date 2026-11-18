//! Adapter the agent system plugs into the network frame port (ADR-0129).
//!
//! Owns the connection lifetime hook (`AgentConnection` — Hello/list push · command dispatch · connection cleanup) and all wire
//! encoding (`AgentEvent`→JSON text, `OutputFrame`→codec binary); the network layer receives only the opaque frames produced here.
//!
//! Single-writer merge (FIFO): the control plane (`FrameOutboundSink`), output plane (`FrameOutputSink`) and usage plane
//! (`ConnUsageOutlet`) all go out through the same `FrameSink`, so a SubscribeAck enqueued before the replay binary stays before it.
//! No blocking: the three outlets can be called from pump/manager sync threads, so only the non-blocking `FrameSink::try_send` is used;
//! raising the close signal on a saturated queue is up to the `FrameSink` implementation (network layer).

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::types::{
    AgentId, OutputEvent, OutputFrame, OutputPayload, OutputSink, SinkError, SinkId,
};
use engram_dashboard_agent::usage::UsageVendorKey;
use engram_dashboard_base::sync;
use engram_dashboard_protocol::{
    encode_structured_frame, encode_terminal_frame, placeholder_error_frame, AgentCommand,
    AgentEvent, UsageLimitSnapshot,
};

use futures_util::future::BoxFuture;
use tokio::sync::watch;

use crate::command_delivery::{CommandDeliveries, LocalCommands};
use crate::command_roster::CommandRoster;
use crate::conn_port::{
    ConnFlow, ConnId, ConnectionHandler, ConnectionHandlerFactory, Frame, FrameFanout, FrameSink,
    Saturated,
};
use crate::connection_core::{
    agent_list_event, broadcast_lease_changed, command_request_id, dispatch_order, event_json,
    hello_event, inbound_lane, output_event_to_wire, ConnectionCore, ConnectionSession,
    DispatchFlow, DispatchOrder, InboundLane, MultiViewState, Outbound,
    OutboundSink as CoreOutboundSink, SinkError as CoreSinkError, SATURATED_REFUSAL,
};
use crate::usage_service::book::wire_vendor;
use crate::usage_service::watch::{UsageEncoder, UsageFrame, UsageOutlet};
use crate::usage_service::UsageService;

// ── output plane sink ────────────────────────────────────────────────────────────

/// The `OutputSink` for one agent subscription of one connection; the pump thread calls `send`.
/// Returns `SinkError` when the queue is full/closed (the core removes it as a dead sink).
pub struct FrameOutputSink {
    frames: Arc<dyn FrameSink>,
    /// Whether a try_send failure (frame drop) happened during the replay phase. handle_subscribe checks it just before ReplayComplete and
    /// corrects SubscribeAck.truncated after the fact; it matters only for replay-phase accuracy (live, the core removes a dead sink).
    replay_dropped: Arc<AtomicBool>,
    sink_id: SinkId,
}

impl FrameOutputSink {
    pub(crate) fn new(frames: Arc<dyn FrameSink>) -> Self {
        Self {
            frames,
            replay_dropped: Arc::new(AtomicBool::new(false)),
            sink_id: uuid::Uuid::new_v4(),
        }
    }

    pub(crate) fn replay_dropped_flag(&self) -> Arc<AtomicBool> {
        self.replay_dropped.clone()
    }

    /// Fills in for an event of this seq that could not be carried — a tag1 `Error` placeholder with the same agent, incarnation marker and seq
    /// (`engram_dashboard_protocol::placeholder_error_frame`).
    ///
    /// Not dropped (ADR-0231): a live view holds `seq > last+1` indefinitely, so a silently skipped seq piles later frames up to the hold cap
    /// and stalls the view. Every other loss is closed by closing the connection and a full replay, so an unfilled hole could only arise here.
    /// Must not return `SinkError` — the core would reap this sink as dead and that agent's output would stop silently while the connection
    /// lives. The return equals a normal send (`SinkError` only when the queue is full).
    // ADR-0231
    pub(crate) fn send_placeholder(
        &self,
        agent_id: AgentId,
        epoch: u32,
        seq: u64,
    ) -> Result<(), SinkError> {
        self.enqueue(placeholder_error_frame(agent_id, epoch, seq))
    }

    fn enqueue(&self, buf: Vec<u8>) -> Result<(), SinkError> {
        match self.frames.try_send(Frame::Binary(buf)) {
            Ok(()) => Ok(()),
            Err(_) => {
                self.replay_dropped.store(true, Ordering::Release);
                Err(SinkError)
            }
        }
    }
}

/// agent event → tag1 payload (JSON of the wire `StructuredEvent`). `None` = cannot be carried — the caller sends a placeholder
/// ([`FrameOutputSink::send_placeholder`]), never drops.
fn structured_payload(ev: &OutputEvent, agent_id: AgentId) -> Option<Vec<u8>> {
    // (1) agent→wire conversion. TerminalBytes should never arrive here (the normal path routes it to tag0 — an upstream wiring bug): it cannot
    //     be mapped (None) → debug catches it early, release warns and sends a placeholder (connection kept).
    let Some(wire) = output_event_to_wire(ev) else {
        debug_assert!(
            false,
            "TerminalBytes (tag0 only) reached the Event (tag1) arm — upstream payload branch bug"
        );
        tracing::warn!(
            agent = %agent_id,
            "cannot encode as tag1 (TerminalBytes reached the Event arm) — sending a placeholder instead"
        );
        return None;
    };
    // (2) JSON serialization. Failure is nearly impossible (string/number fields only); then only this frame warns and a placeholder goes out
    //     — a serialization failure is a data problem, not a slow consumer, so no `SinkError` kills the connection.
    match serde_json::to_vec(&wire) {
        Ok(p) => Some(p),
        Err(e) => {
            tracing::warn!(
                agent = %agent_id,
                "StructuredEvent serialization failed — sending a placeholder instead: {e}"
            );
            None
        }
    }
}

impl OutputSink for FrameOutputSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        // Payload branch (ADR-0045): console bytes → a tag0 terminal frame, structured events → a tag1 structured frame. The sink owns the wire
        //   encoding (the core knows no wire, ADR-0003).
        let buf = match frame.payload {
            OutputPayload::Bytes(b) => {
                encode_terminal_frame(frame.agent_id, frame.epoch, frame.seq, b)
            }
            // tag1 encoding: the codec is opaque to the payload schema — the serialization format (JSON) and event types are the daemon's (ADR-0045
            //   self-describing). Header layout is the same as tag0, tag=1.
            OutputPayload::Event(ev) => match structured_payload(ev, frame.agent_id) {
                Some(payload) => {
                    encode_structured_frame(frame.agent_id, frame.epoch, frame.seq, &payload)
                }
                None => return self.send_placeholder(frame.agent_id, frame.epoch, frame.seq),
            },
        };
        self.enqueue(buf)
    }

    fn sink_id(&self) -> SinkId {
        self.sink_id
    }
}

// ── control plane sink (response path of ConnectionCore.dispatch) ─────────────────

pub struct FrameOutboundSink {
    frames: Arc<dyn FrameSink>,
}

impl FrameOutboundSink {
    pub(crate) fn new(frames: Arc<dyn FrameSink>) -> Self {
        Self { frames }
    }
}

impl CoreOutboundSink for FrameOutboundSink {
    fn enqueue(&self, out: Outbound) -> Result<(), CoreSinkError> {
        let frame = match out {
            // A serialization failure is a drop.
            Outbound::Event(ev) => match event_json(&ev) {
                Some(text) => Frame::Text(text),
                None => return Ok(()),
            },
            Outbound::Binary(b) => Frame::Binary(b),
            Outbound::Close(reason) => Frame::Close(reason),
        };
        self.frames.try_send(frame).map_err(|_| CoreSinkError)
    }

    fn make_output_sink(&self) -> (Arc<dyn OutputSink>, Arc<AtomicBool>) {
        let sink = Arc::new(FrameOutputSink::new(self.frames.clone()));
        let flag = sink.replay_dropped_flag();
        (sink, flag)
    }
}

// ── usage plane (TRD S21 usage-limit-slot §1-4) ────────────────────────────────────

/// One connection's usage outlet — `on_connect` creates a new one per connection and hands it to the usage roster.
///
/// No `Clone`: a copy sharing the lock would let one copy's `revoke` silence another entry's outlet (contract of `UsageWatch::attach`).
/// When saturated this connection ends: a `try_send` failure is a close signal for the frame outlet (network layer), as with the output plane sink.
pub(crate) struct ConnUsageOutlet {
    frames: Mutex<Option<Arc<dyn FrameSink>>>,
}

impl ConnUsageOutlet {
    pub(crate) fn new(frames: Arc<dyn FrameSink>) -> Self {
        Self {
            frames: Mutex::new(Some(frames)),
        }
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Arc<dyn FrameSink>>> {
        // Only copying and taking out happen inside, so the guard stays intact after a poison.
        sync::lock(&self.frames)
    }
}

impl UsageOutlet for ConnUsageOutlet {
    fn send(&self, frame: &UsageFrame) {
        let Some(frames) = self.slot().clone() else {
            return;
        };
        let _ = frames.try_send(Frame::Text(frame.json.to_string()));
    }

    fn revoke(&self) {
        let taken = self.slot().take();
        drop(taken);
    }
}

/// One usage sheet → one [`AgentEvent::UsageLimitsUpdated`] JSON line. `subscribed` becomes a wire vendor by the same conversion as the
/// snapshot's `vendor` (an unconvertible key is dropped).
pub(crate) struct UsageEventEncoder;

impl UsageEncoder for UsageEventEncoder {
    fn encode(
        &self,
        snapshot: &UsageLimitSnapshot,
        subscribed: &BTreeSet<UsageVendorKey>,
    ) -> Option<Arc<str>> {
        let ev = AgentEvent::UsageLimitsUpdated {
            snapshot: snapshot.clone(),
            subscribed: subscribed.iter().copied().filter_map(wire_vendor).collect(),
        };
        event_json(&ev).map(Arc::from)
    }
}

// ── connection lifetime hook ────────────────────────────────────────────────────

/// The agent-side handler for one connection: binds `ConnectionCore` (dispatch) and that connection's lifetime state.
pub struct AgentConnection {
    core: Arc<ConnectionCore>,
    session: Arc<ConnectionSession>,
}

/// Called only after `on_text` failed to decode a command (ADR-0129 0-4).
///
/// The network layer already authenticated on the first frame, so a later handshake is a protocol violation; that frame is not a command
/// variant (protocol `handshake::AuthFrame`), so it fails command decoding and is caught here.
/// Decided by the tag, not by being intact: an Auth frame with a missing field or mismatched body also counts. Otherwise an old client would get
/// ``unknown variant `Auth` ``, which misleads debugging since Auth is the only first frame this server accepts.
/// The upper layer's vocabulary does not grow: only one top-level key is looked at and the body is skipped with `IgnoredAny` (no value tree) —
/// a peer can reach this path on every frame, so no allocation is added.
fn is_handshake_frame(text: &str) -> bool {
    /// The canonical tag string is `engram_dashboard_protocol::handshake::AuthFrame`; `handshake_frame_is_recognized_after_auth` below runs this
    /// check on that type's serialized bytes, so it breaks if the two sides diverge.
    #[derive(serde::Deserialize)]
    enum HandshakeTag {
        Auth(serde::de::IgnoredAny),
    }
    serde_json::from_str::<HandshakeTag>(text).is_ok()
}

impl ConnectionHandler for AgentConnection {
    fn on_connect<'a>(
        &'a self,
        conn_id: ConnId,
        frames: &'a Arc<dyn FrameSink>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            // Two frames, fixed: the queue room is shared with the status fanout, so no safe frame count can be named (port contract).
            if let Some(text) = event_json(&hello_event(env!("CARGO_PKG_VERSION").into())) {
                let _ = frames.send(Frame::Text(text)).await;
            }
            if let Some(text) = event_json(&agent_list_event(self.core.manager())) {
                let _ = frames.send(Frame::Text(text)).await;
            }
            // Registered after the greeting: the greeting is queued before the attach, and a failed greeting closes the connection and `on_disconnect`
            //   removes the entry. Still before any command — the network layer spawns the read task only after this call returns (`link/server.rs`),
            //   so a connection that did not get here cannot send a frame.
            // ADR-0154
            self.core.commands().attach(conn_id, frames);
            // The usage entry is created after the greeting too: once it exists the first sheet of a subscription replacement goes straight out this
            //   outlet and must not precede the two greeting frames. The set is empty, so nothing goes out before subscribing.
            self.core
                .usage()
                .attach(conn_id, ConnUsageOutlet::new(frames.clone()));
        })
    }

    /// These calls arrive in arrival order, never overlapping (port contract) — that serialization embodies the ordering invariant, so holding one
    /// long delays the same connection's next command. Which commands detach from the lane, and why only those, is canonical in [`dispatch_order`].
    fn on_text<'a>(
        &'a self,
        conn_id: ConnId,
        text: &'a str,
        frames: &'a Arc<dyn FrameSink>,
    ) -> BoxFuture<'a, ConnFlow> {
        let sink = FrameOutboundSink::new(frames.clone());
        Box::pin(async move {
            match serde_json::from_str::<AgentCommand>(text) {
                Ok(cmd) if dispatch_order(&cmd) == DispatchOrder::Detached => {
                    // Sent out of the lane so it does not block this connection's next command.
                    // The reply still goes out on this connection's own frame outlet: the sink holds a copy of the connection's single outbound queue, so a late
                    //   end of this task cannot leak the reply elsewhere. The copy may outlive `on_disconnect` (⟳ holds it up to `REPLY_WAIT_MAX`); it keeps only
                    //   the transport handle alive, not the link — the network layer closes the peer in every teardown branch (`link/server.rs`), so a late send just fails.
                    // This task keeps running after the connection is cleaned up (the network layer's abort does not reach it — port contract), but does not revive
                    //   connection state: a detached command touches only the global manager, the usage service and all-connection / subscriber broadcasts, never the
                    //   roster, subscriptions or leases (the roster side is blocked once more by `refuse_if_detached`).
                    let core = Arc::clone(&self.core);
                    let session = Arc::clone(&self.session);
                    tokio::spawn(async move {
                        // A detached command does not emit `Close` (it originates only from `StopDaemon` and replay-queue saturation, both in the lane). If a
                        //   misclassification sent one here it becomes an in-queue marker rather than a silently unclosed connection.
                        if core.dispatch(cmd, &session, &sink).await == DispatchFlow::Close {
                            let _ =
                                sink.enqueue(Outbound::Close("dispatch requested close".into()));
                        }
                    });
                    ConnFlow::Continue
                }
                Ok(cmd) => match self.core.dispatch(cmd, &self.session, &sink).await {
                    DispatchFlow::Close => ConnFlow::Close,
                    DispatchFlow::Continue => ConnFlow::Continue,
                },
                // Do not close the connection: emit one Error line and keep it alive (regression guard `tests/ws_e2e.rs` case21 also checks later commands are answered).
                // Command parsing comes first: the tags do not overlap, so the result is order-independent, but this adds no extra parsing per frame on the normal path.
                Err(_) if is_handshake_frame(text) => {
                    tracing::warn!(
                        conn = conn_id,
                        "second handshake on an authenticated connection — refused"
                    );
                    let _ = sink.enqueue(Outbound::event(AgentEvent::Error {
                        request_id: None,
                        message: "already authenticated".into(),
                    }));
                    ConnFlow::Continue
                }
                Err(e) => {
                    tracing::warn!(conn = conn_id, "command parse failed: {e}");
                    let _ = sink.enqueue(Outbound::event(AgentEvent::Error {
                        request_id: None,
                        message: format!("invalid command: {e}"),
                    }));
                    ConnFlow::Continue
                }
            }
        })
    }

    fn on_binary<'a>(
        &'a self,
        conn_id: ConnId,
        _payload: &'a [u8],
        frames: &'a Arc<dyn FrameSink>,
    ) -> BoxFuture<'a, ConnFlow> {
        let sink = FrameOutboundSink::new(frames.clone());
        Box::pin(async move {
            // The protocol has no client→daemon binary.
            tracing::warn!(conn = conn_id, "unexpected binary frame — close");
            let _ = sink.enqueue(Outbound::event(AgentEvent::Error {
                request_id: None,
                message: "unexpected binary frame".into(),
            }));
            let _ = frames.send(Frame::Close("protocol error".into())).await;
            ConnFlow::Close
        })
    }

    /// Called only when saturated — it only classifies and does not process (a sync branch on top of the read loop).
    ///
    /// A refusal is also a reply: before choosing `Refused`, put in an `Error` carrying that command's `request_id`; otherwise the sender gets neither
    /// reply nor error until the deadline (obligation 2 of the port contract).
    /// That put-in can fail: returning `Refused` then would claim a reply was sent, the network layer drops the original command, and the sender gets
    /// neither. So the result splits into [`Saturated::Unanswered`] (meaning and disposition — close the connection — canonical in that variant);
    /// this occurs only when both queues are saturated at once (the lane was full, hence we got here, and the send queue too).
    /// A parse failure also comes here with no `request_id` to carry; it answers in the same shape as the `on_text` parse-failure branch (`Error{request_id: None}`).
    fn on_inbound_saturated(
        &self,
        conn_id: ConnId,
        text: &str,
        frames: &Arc<dyn FrameSink>,
    ) -> Saturated {
        let cmd = serde_json::from_str::<AgentCommand>(text).ok();
        if let Some(cmd) = &cmd {
            if inbound_lane(cmd) == InboundLane::Bypass {
                tracing::warn!(
                    conn = conn_id,
                    "inbound lane saturated — letting cancel-family commands through outside the lane"
                );
                return Saturated::Bypass;
            }
        }
        // Warn only the first time — saturation happens on every frame, so without a latch this one event would swamp the log (as `stray_outcome_warned`).
        if !self.session.saturated_warned.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                conn = conn_id,
                "inbound lane saturated — refusing this connection's command (logged once per connection)"
            );
        }
        let sink = FrameOutboundSink::new(frames.clone());
        let refusal = sink.enqueue(Outbound::event(AgentEvent::Error {
            request_id: cmd.as_ref().and_then(command_request_id),
            message: SATURATED_REFUSAL.to_string(),
        }));
        match refusal {
            Ok(()) => Saturated::Refused,
            Err(_) => {
                // No latch: the sibling warning (`saturated_warned`) is a common per-frame event, but this one is followed by the connection ending — at most one line per connection.
                tracing::warn!(
                    conn = conn_id,
                    "could not enqueue the saturation refusal on the send queue either — this command vanishes without a reply"
                );
                Saturated::Unanswered
            }
        }
    }

    fn on_disconnect(&self, conn_id: ConnId) {
        // ── remove this connection's name from the command roster (ADR-0150) — must be the first line of this cleanup ────
        // If pushed back, this connection reads as attached while the earlier cleanup runs, and an overlapping registration passes (overlap basis: the
        //   `CommandRoster` header): a dead connection would take back the name a live one put up, then this line later erases it — a healthy
        //   connection's command silently disappears. At the very front the window does not exist (nor a panic-skippable stretch).
        // This line also releases the frame outlet copy the roster holds (put in by `on_connect`) — one of the copies the port contract requires released
        //   by return (`conn_port::ConnectionHandler::on_disconnect`). Put-in and take-out are one place each, inside the roster's one lock, so no race of missing it.
        // ADR-0154
        self.core.commands().detach(conn_id);
        // The usage roster cell goes at the same place: the frame outlet copy it holds is released inside this call (port duty above). A copy taken
        //   briefly while the outlet sends one frame may outlive this call — harmless (a single `try_send`). Nothing revives a cell, so a late
        //   subscription swap overlapping this cleanup is dropped (`UsageWatch::replace`).
        self.core.usage().detach(conn_id);

        // ── reap the command round trips this connection issued (ADR-0154) ───────────────────────
        // After the roster cleanup: reversed, a released round trip replying while this connection is still in the roster writes a frame to a dead outlet (harmless, causality blurs).
        // The correlation table and the roster are different tables (different lifetime units: one round trip vs one connection — `command_delivery`
        //   header), so the cleanup is two lines. Without this line an entry with nowhere to take its reply sits until the deadline and its round-trip task does not end.
        let dropped = self.core.deliveries().drop_origin(conn_id);
        if dropped > 0 {
            tracing::debug!(
                conn = conn_id,
                dropped,
                "disconnected — reaped the command round trips this connection issued"
            );
        }

        let manager = self.core.manager();

        // ── subscription leak prevention ──────────────────────────────────────────────────
        // Without it a zombie sink that try_sends to a dead queue stays in the core subscribers until the next emit removes it — so cut it explicitly.
        //
        // Known race — this snapshot can miss one subscription: `handle_subscribe` registers the sink with the core and then records it in `subs` with no
        //   `.await` between, so cancellation cannot intervene, but (1) a read_task on another worker still in that window while this cleanup snapshots
        //   (the abort-overlap race in the `ConnectionHandler::on_disconnect` doc), or (2) a panic in that window (a test build without the hook; in
        //   production the hook ends the daemon — ADR-0292) leaves it only in the core.
        // Two outcomes: (a) a zombie sink stays until the next emit (the core's dead-sink removal recovers it); (b) it holds a copy of the connection's
        //   frame outlet — the transport handle outlives the connection, not the link: the network layer closes the peer and aborts the losing tasks
        //   (`link/server.rs`), which is why those aborts must not be removed.
        let leftovers: Vec<(AgentId, SinkId)> = {
            let guard = self.session.subs.lock().expect("subs poisoned");
            guard.iter().map(|(a, s)| (*a, *s)).collect()
        };
        for (agent_id, sink_id) in leftovers {
            let _ = manager.unsubscribe(agent_id, sink_id);
        }

        // ── multi-viewer cleanup ──────────────────────────────────────────────────────
        // (a) viewport renegotiation — resize to the smallest of the remaining viewers (as tmux growing back after a detach).
        //     Lock order: remove_conn_viewports returns only the negotiated value computed inside the multiview lock; manager.resize runs with the lock released (never call the core while holding the lock).
        let owned: Vec<(AgentId, String)> = {
            let g = self
                .session
                .owned_viewports
                .lock()
                .expect("owned_viewports poisoned");
            g.clone()
        };
        if !owned.is_empty() {
            for (agent_id, negotiated) in self.core.multiview().remove_conn_viewports(&owned) {
                if let Some((cols, rows)) = negotiated {
                    let _ = manager.resize(agent_id, cols, rows);
                }
                // None = no viewers left → leave as is (keep the last size).
            }
        }
        // (b) automatic input lease release.
        for agent_id in self.core.multiview().release_all_for_conn(conn_id) {
            broadcast_lease_changed(self.core.fanout(), agent_id, false);
        }
    }
}

pub struct AgentConnections {
    manager: Arc<AgentManager>,
    multiview: MultiViewState,
    // ADR-0129
    fanout: Arc<dyn FrameFanout>,
    // ADR-0096
    control_registry: Arc<crate::control::registry::ControlRegistry>,
    // ADR-0116
    messaging: Arc<crate::control::mcp_server::MessagingSlot>,
    // ADR-0155
    commands: CommandRoster,
    // ADR-0154
    deliveries: CommandDeliveries,
    // ADR-0155
    locals: Arc<dyn LocalCommands>,
    usage: Arc<UsageService>,
    shutdown_tx: watch::Sender<bool>,
}

impl AgentConnections {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        manager: Arc<AgentManager>,
        multiview: MultiViewState,
        fanout: Arc<dyn FrameFanout>,
        control_registry: Arc<crate::control::registry::ControlRegistry>,
        messaging: Arc<crate::control::mcp_server::MessagingSlot>,
        commands: CommandRoster,
        deliveries: CommandDeliveries,
        locals: Arc<dyn LocalCommands>,
        usage: Arc<UsageService>,
        shutdown_tx: watch::Sender<bool>,
    ) -> Self {
        Self {
            manager,
            multiview,
            fanout,
            control_registry,
            messaging,
            commands,
            deliveries,
            locals,
            usage,
            shutdown_tx,
        }
    }
}

impl AgentConnections {
    /// The body of `handler_for`, yielding the concrete type: tests checking cleanup order must hold the connection's lifetime state (`session`),
    /// which a `dyn ConnectionHandler` hides. The production path is only `handler_for`, so assembly does not diverge.
    fn build(&self, conn_id: ConnId) -> Arc<AgentConnection> {
        let core = Arc::new(ConnectionCore::new(
            self.manager.clone(),
            self.multiview.clone(),
            self.fanout.clone(),
            self.control_registry.clone(),
            self.messaging.clone(),
            self.commands.clone(),
            self.deliveries.clone(),
            Arc::clone(&self.locals),
            Arc::clone(&self.usage),
            self.shutdown_tx.clone(),
        ));
        Arc::new(AgentConnection {
            core,
            session: Arc::new(ConnectionSession::new(conn_id)),
        })
    }
}

impl ConnectionHandlerFactory for AgentConnections {
    fn handler_for(&self, conn_id: ConnId) -> Arc<dyn ConnectionHandler> {
        self.build(conn_id)
    }

    fn handshake_error_frame(&self, message: &str) -> Option<String> {
        event_json(&AgentEvent::Error {
            request_id: None,
            message: message.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_doubles::FakeFrameSink;
    use tokio::sync::mpsc;

    /// The network layer's real outlet (`link::server::ConnFrameSink`) needs a live transport peer that only that module can build (ADR-0129), so
    /// these tests use a double depending only on the port contract (`conn_port`).
    fn frame_sink(tx: mpsc::Sender<Frame>) -> Arc<dyn FrameSink> {
        Arc::new(FakeFrameSink::new(tx))
    }

    // ── 1. does FrameOutputSink put a binary frame into conn_tx ─────────────────
    #[tokio::test]
    async fn frame_output_sink_encodes_and_sends_binary() {
        let (tx, mut rx) = mpsc::channel::<Frame>(8);
        let sink = FrameOutputSink::new(frame_sink(tx));
        let agent_id = uuid::Uuid::new_v4();
        let data = b"abc";
        let frame = OutputFrame {
            agent_id,
            epoch: 7,
            seq: 42,
            payload: OutputPayload::Bytes(data),
        };
        sink.send(frame).expect("send ok");

        match rx.recv().await.expect("one item") {
            Frame::Binary(buf) => {
                let decoded = engram_dashboard_protocol::decode_frame(&buf).expect("decode");
                assert_eq!(decoded.agent_id, agent_id);
                assert_eq!(decoded.epoch, 7);
                assert_eq!(decoded.seq, 42);
                assert_eq!(decoded.payload, b"abc");
            }
            _ => panic!("not Binary"),
        }
    }

    // ── 1b. (S15 B7) does it encode an Event (structured) payload as a tag1 frame ──────
    #[tokio::test]
    async fn frame_output_sink_encodes_event_as_tag1_structured_frame() {
        use engram_dashboard_agent::types::{
            OutputEvent as CoreOutputEvent, ToolCategory as CoreToolCategory,
        };
        use engram_dashboard_protocol::{
            decode_frame, StructuredEvent as WireStructuredEvent, ToolCategory as WireToolCategory,
            FRAME_TAG_STRUCTURED_EVENT,
        };

        let (tx, mut rx) = mpsc::channel::<Frame>(8);
        let sink = FrameOutputSink::new(frame_sink(tx));
        let agent_id = uuid::Uuid::new_v4();
        let ev = CoreOutputEvent::ToolCall {
            name: "read".into(),
            args_json: r#"{"path":"/x"}"#.into(),
            id: Some("call_1".into()),
            turn_id: Some("t9".into()),
            message_id: None,
            category: CoreToolCategory::Read,
        };
        let frame = OutputFrame {
            agent_id,
            epoch: 3,
            seq: 100,
            payload: OutputPayload::Event(&ev),
        };
        sink.send(frame).expect("Event send ok");

        match rx.recv().await.expect("one item") {
            Frame::Binary(buf) => {
                let decoded = decode_frame(&buf).expect("decode");
                assert_eq!(decoded.tag, FRAME_TAG_STRUCTURED_EVENT, "must be tag1");
                assert_eq!(decoded.agent_id, agent_id);
                assert_eq!(decoded.epoch, 3);
                assert_eq!(decoded.seq, 100);
                let parsed: WireStructuredEvent =
                    serde_json::from_slice(decoded.payload).expect("payload JSON parse");
                assert_eq!(
                    parsed,
                    WireStructuredEvent::ToolCall {
                        name: "read".into(),
                        args_json: r#"{"path":"/x"}"#.into(),
                        id: Some("call_1".into()),
                        turn_id: Some("t9".into()),
                        message_id: None,
                        category: Some(WireToolCategory::Read),
                    },
                    "tag1 payload restored losslessly as the wire StructuredEvent"
                );
            }
            other => panic!("expected Binary(tag1): {other:?}"),
        }
    }

    // ── 2. full → SinkError + replay_dropped ──────────────────────────────────
    #[tokio::test]
    async fn frame_output_sink_full_returns_error_and_marks_replay_dropped() {
        // The out-of-band close signal on queue saturation is not this test's concern: it is the contract of the frame outlet implementation, kept next
        //   to `impl FrameSink for ConnFrameSink` (the test double does not raise it).
        let (tx, mut rx) = mpsc::channel::<Frame>(1);
        let sink = FrameOutputSink::new(frame_sink(tx));
        let replay_dropped = sink.replay_dropped_flag();
        let agent_id = uuid::Uuid::new_v4();
        let frame = |seq: u64| OutputFrame {
            agent_id,
            epoch: 0,
            seq,
            payload: OutputPayload::Bytes(b"x"),
        };
        sink.send(frame(0)).expect("first ok");
        assert!(sink.send(frame(1)).is_err(), "SinkError when full");
        assert!(
            replay_dropped.load(Ordering::Acquire),
            "replay_dropped set on drop"
        );

        assert!(matches!(rx.recv().await.unwrap(), Frame::Binary(_)));
    }

    // ── 2a. placeholder for discard branches (ADR-0231) ─────────────────────────────
    // Measures the placeholder both branches call, not the branches: a serialization failure cannot be produced with current event shapes, and a
    //   stray `TerminalBytes` stops at `debug_assert` first in debug. Checked: a tag1 `Error` of the same seq replaces the discarded seq (no hole in
    //   the view's seq stream), the return is `Ok` so the sink lives and the next seq goes out, and the bytes equal the shell's placeholder.
    #[tokio::test]
    async fn a_dropped_event_goes_out_as_a_placeholder_at_the_same_seq() {
        use engram_dashboard_protocol::{
            decode_frame, placeholder_error_frame, StructuredEvent as WireStructuredEvent,
            FRAME_TAG_STRUCTURED_EVENT, FRAME_TAG_TERMINAL_BYTES, PLACEHOLDER_ERROR_MESSAGE,
        };

        let (tx, mut rx) = mpsc::channel::<Frame>(8);
        let sink = FrameOutputSink::new(frame_sink(tx));
        let agent_id = uuid::Uuid::new_v4();
        let bytes = |seq: u64| OutputFrame {
            agent_id,
            epoch: 4,
            seq,
            payload: OutputPayload::Bytes(b"x"),
        };
        sink.send(bytes(5)).expect("the preceding frame");
        sink.send_placeholder(agent_id, 4, 6)
            .expect("a placeholder is Ok — with SinkError the core would reap the sink");
        sink.send(bytes(7))
            .expect("the sink is alive and the next seq goes out too");

        let mut got = Vec::new();
        for _ in 0..3 {
            match rx.recv().await.expect("three frames") {
                Frame::Binary(buf) => got.push(buf),
                other => panic!("expected Binary: {other:?}"),
            }
        }
        let seqs: Vec<u64> = got
            .iter()
            .map(|b| decode_frame(b).expect("decode").seq)
            .collect();
        assert_eq!(seqs, vec![5, 6, 7], "seq stream without holes");

        let placeholder = decode_frame(&got[1]).expect("decode");
        assert_eq!(placeholder.tag, FRAME_TAG_STRUCTURED_EVENT, "tag1");
        assert_eq!(placeholder.agent_id, agent_id);
        assert_eq!(placeholder.epoch, 4);
        let parsed: WireStructuredEvent =
            serde_json::from_slice(placeholder.payload).expect("payload JSON");
        assert_eq!(
            parsed,
            WireStructuredEvent::Error {
                message: PLACEHOLDER_ERROR_MESSAGE.to_owned()
            }
        );
        assert_eq!(
            got[1],
            placeholder_error_frame(agent_id, 4, 6),
            "bytes equal the shell relay's placeholder (a single set)"
        );
        assert_eq!(
            decode_frame(&got[2]).expect("decode").tag,
            FRAME_TAG_TERMINAL_BYTES
        );
    }

    // ── 2b. second-handshake detection (ADR-0129 0-4) ────────────────────────────────
    #[test]
    fn handshake_frame_is_recognized_after_auth() {
        let frame = engram_dashboard_protocol::handshake::AuthFrame::Auth {
            token: engram_dashboard_protocol::Secret::new("deadbeef"),
            protocol_version: 3,
        };
        assert!(is_handshake_frame(&serde_json::to_string(&frame).unwrap()));
        // The same frame even if field order or whitespace differ (each sender has its own serializer, even a hand-assembled JS one).
        assert!(is_handshake_frame(
            r#"{ "Auth": { "protocol_version": 3, "token": "deadbeef" } }"#
        ));
    }

    #[test]
    fn malformed_handshake_frames_are_recognized_too() {
        for text in [
            r#"{"Auth":{"token":"deadbeef"}}"#, // protocol_version missing
            r#"{"Auth":123}"#,                  // body is not an object
            r#"{"Auth":null}"#,
        ] {
            assert!(
                is_handshake_frame(text),
                "an Auth-tagged mismatched frame is also a handshake: {text}"
            );
        }
    }

    #[test]
    fn non_handshake_text_is_not_recognized() {
        // Must not eat into the ordinary parse-error path: otherwise case23 (broken JSON → "invalid command") silently becomes "already authenticated".
        for text in [
            r#"{"NotACommand":true}"#,
            r#"{"ListAgents":{"request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            r#""Auth""#,                 // only the tag, no body = not a frame
            r#"{"Auth":1,"Kill":2}"#,    // the top-level key is not a single one
            r#"{"auth":{"token":"x"}}"#, // the tag is a wire contract down to the case
            "not json at all",
            "",
        ] {
            assert!(
                !is_handshake_frame(text),
                "accepted as a handshake though it is not: {text}"
            );
        }
    }

    // ── 3. control plane: do the 3 Outbound kinds encode as 3 frame kinds ───────────────
    #[tokio::test]
    async fn frame_outbound_sink_maps_outbound_to_frames() {
        let (tx, mut rx) = mpsc::channel::<Frame>(8);
        let sink = FrameOutboundSink::new(frame_sink(tx));
        sink.enqueue(Outbound::event(AgentEvent::Error {
            request_id: None,
            message: "boom".into(),
        }))
        .expect("event ok");
        sink.enqueue(Outbound::Binary(vec![1, 2, 3]))
            .expect("binary ok");
        sink.enqueue(Outbound::Close("bye".into()))
            .expect("close ok");

        match rx.recv().await.unwrap() {
            Frame::Text(s) => assert!(s.contains("boom"), "Error as JSON text"),
            other => panic!("expected Text: {other:?}"),
        }
        assert!(matches!(rx.recv().await.unwrap(), Frame::Binary(_)));
        match rx.recv().await.unwrap() {
            Frame::Close(r) => assert_eq!(r, "bye"),
            other => panic!("expected Close: {other:?}"),
        }
    }

    // ── 4. command roster: does connection cleanup erase its owner's name (ADR-0155/0150) ──

    /// Also checks that connections made by one factory see the same roster — a per-connection roster would give conn 2 an empty list.
    fn test_factory() -> AgentConnections {
        test_factory_with(
            Arc::new(crate::test_doubles::RecordingFanout::new()),
            CommandRoster::new(),
            CommandDeliveries::new(),
            crate::usage_service::fakes::idle_service(),
        )
    }

    fn test_factory_with(
        fanout: Arc<dyn FrameFanout>,
        commands: CommandRoster,
        deliveries: CommandDeliveries,
        usage: Arc<UsageService>,
    ) -> AgentConnections {
        use engram_dashboard_agent::preset::{Preset, PresetRegistry, PresetStore};
        use engram_dashboard_agent::profile::{
            AgentProfile, ProfileRegistry, ProfileStore, StoreError,
        };
        use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};

        struct NoStore;
        impl ProfileStore for NoStore {
            fn save(&self, _: &[AgentProfile]) -> Result<(), StoreError> {
                Ok(())
            }
            fn load(&self) -> Vec<AgentProfile> {
                Vec::new()
            }
        }
        impl PresetStore for NoStore {
            fn save(&self, _: &[Preset]) -> Result<(), StoreError> {
                Ok(())
            }
            fn load(&self) -> Vec<Preset> {
                Vec::new()
            }
        }

        let manager = Arc::new(AgentManager::new(
            Arc::new(crate::status_fanout::DaemonStatusSink::new(fanout.clone())),
            Arc::new(ProfileRegistry::new(Arc::new(NoStore))),
            Arc::new(PresetRegistry::new(Arc::new(NoStore))),
            Arc::new(SessionTracker::new(
                TrackerConfig::default(),
                Arc::new(|_agent_id, _sid| {}),
            )),
        ));
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);
        AgentConnections::new(
            manager,
            MultiViewState::new(),
            fanout,
            Arc::new(crate::control::registry::ControlRegistry::new()),
            Arc::new(crate::control::mcp_server::MessagingSlot::new()),
            commands,
            deliveries,
            // An assembly with no commands of its own — what this harness looks at is the roster and cleanup order.
            Arc::new(crate::command_delivery::NoLocalCommands),
            usage,
            shutdown_tx,
        )
    }

    fn command_json(cmd: &AgentCommand) -> String {
        serde_json::to_string(cmd).expect("command serialization")
    }

    /// Cap the wait so a frame that never reaches this connection is a failure, not a hang.
    async fn next_event(rx: &mut mpsc::Receiver<Frame>) -> AgentEvent {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
            .await
            .expect("exceeded the cap waiting for a frame — it does not arrive on this connection")
            .expect("one frame");
        match frame {
            Frame::Text(text) => serde_json::from_str(&text).expect("event decode"),
            other => panic!("expected Text: {other:?}"),
        }
    }

    /// On disconnect that connection's name disappears from the roster — a trace would make the owner token fresh per connection, so entries pile up on
    /// every reconnect with no expiry or reclaim path until the cap (ADR-0150 decision 3). The looking-up side is another connection, which also shows
    /// that connections of one factory see the same roster.
    /// Daemon-side regression net: the activation command runs outside the arrival-order lane, so a later command does not wait for its outcome (once,
    /// one activation — typically 2s, backstop 15s — had to finish before the next command of the connection started).
    ///
    /// Measured with a nonexistent profile: it ends deterministically with no real process while the reply path is the same (classification ends before
    /// the `manager` lookup). The observation relies on the current-thread runtime of `#[tokio::test]`: a detached task is not polled until this test
    /// awaits again, so the inline `ListAgents` reply lands first; on a multithreaded runtime that order would be probability.
    #[tokio::test]
    async fn a_later_command_is_answered_before_an_earlier_activation() {
        use engram_dashboard_protocol::RequestId;

        let factory = test_factory();
        let conn = factory.handler_for(1);
        let (tx, mut rx) = mpsc::channel::<Frame>(8);
        let frames = frame_sink(tx);
        conn.on_connect(1, &frames).await;
        for _ in 0..2 {
            let _ = next_event(&mut rx).await;
        }

        let spawn = command_json(&AgentCommand::Spawn {
            profile_id: uuid::Uuid::new_v4(),
            request_id: RequestId(uuid::Uuid::new_v4()),
        });
        let list = command_json(&AgentCommand::ListAgents {
            request_id: RequestId(uuid::Uuid::new_v4()),
        });

        assert_eq!(
            conn.on_text(1, &spawn, &frames).await,
            ConnFlow::Continue,
            "activation does not close the connection"
        );
        conn.on_text(1, &list, &frames).await;

        match next_event(&mut rx).await {
            AgentEvent::AgentList { .. } => {}
            other => panic!(
                "the later ListAgents waited for the activation — head-of-line blocking regression: {other:?}"
            ),
        }
        // Even when detached, the reply comes through this connection's same outlet.
        match next_event(&mut rx).await {
            AgentEvent::Error { message, .. } => assert!(
                message.contains("profile not found"),
                "the detached branch's reply must come to this connection: {message}"
            ),
            other => panic!("expected Error: {other:?}"),
        }
    }

    /// A command staying in the lane puts its reply in the queue before `on_text` returns — the counterpart of the test above; only together do "all detached" and "none detached" each get caught.
    #[tokio::test]
    async fn an_in_order_command_is_answered_before_on_text_returns() {
        use engram_dashboard_protocol::RequestId;

        let factory = test_factory();
        let conn = factory.handler_for(1);
        let (tx, mut rx) = mpsc::channel::<Frame>(8);
        let frames = frame_sink(tx);
        conn.on_connect(1, &frames).await;
        for _ in 0..2 {
            let _ = next_event(&mut rx).await;
        }

        let kill = command_json(&AgentCommand::Kill {
            agent_id: uuid::Uuid::new_v4(),
            request_id: RequestId(uuid::Uuid::new_v4()),
        });
        conn.on_text(1, &kill, &frames).await;
        assert!(
            rx.try_recv().is_ok(),
            "the reply of an in-lane command must already be in the queue by the time on_text returns"
        );
    }

    /// The daemon-side counterpart of the saturation branch: the network layer's net (`ws::tests::a_cancel_still_gets_through_a_saturated_inbound_queue`)
    /// measures the wiring; this measures what this handler lets through, what it refuses and how. A wordless refusal is indistinguishable from a
    /// silent loss, so the `request_id` is checked too.
    #[tokio::test]
    async fn a_saturated_queue_passes_kill_and_refuses_the_rest_out_loud() {
        use engram_dashboard_protocol::RequestId;

        let factory = test_factory();
        let conn = factory.handler_for(1);
        let (tx, mut rx) = mpsc::channel::<Frame>(8);
        let frames = frame_sink(tx);
        conn.on_connect(1, &frames).await;
        for _ in 0..2 {
            let _ = next_event(&mut rx).await;
        }

        let kill = command_json(&AgentCommand::Kill {
            agent_id: uuid::Uuid::new_v4(),
            request_id: RequestId::new(),
        });
        assert_eq!(
            conn.on_inbound_saturated(1, &kill, &frames),
            Saturated::Bypass,
            "a cancel skips the blocked lane"
        );
        assert!(
            rx.try_recv().is_err(),
            "Bypass is only a classification — it must not be processed or replied to here"
        );

        let write_req = RequestId::new();
        let write = command_json(&AgentCommand::WriteStdin {
            agent_id: uuid::Uuid::new_v4(),
            data: "x".into(),
            request_id: write_req,
        });
        assert_eq!(
            conn.on_inbound_saturated(1, &write, &frames),
            Saturated::Refused,
            "a command whose order has meaning is not skipped"
        );
        match next_event(&mut rx).await {
            AgentEvent::Error {
                request_id,
                message,
            } => {
                assert_eq!(
                    request_id,
                    Some(write_req),
                    "the refusal must say whose request it is so the sender decides on a retry"
                );
                assert!(
                    message.contains("refused, not applied"),
                    "the fact that it was not applied must be in the wording: {message}"
                );
            }
            other => panic!("expected Error: {other:?}"),
        }

        // Even a frame that cannot be parsed does not stay silent (only the request_id to carry is missing).
        assert_eq!(
            conn.on_inbound_saturated(1, "{\"NotACommand\":1}", &frames),
            Saturated::Refused
        );
        match next_event(&mut rx).await {
            AgentEvent::Error { request_id, .. } => assert_eq!(request_id, None),
            other => panic!("expected Error: {other:?}"),
        }
    }

    /// If the refusal could not be put in, it reports so: folding it into `Refused` makes the network layer believe in a nonexistent reply, drop the
    /// original command, and leave the sender with neither (obligation 2 of the port contract).
    ///
    /// Both queues saturated at once: the full inbound lane is told by this branch being called, and the send queue is saturated by pre-filling a
    /// capacity-1 one — the only condition yielding this value. Deterministic: no deadline, no thread.
    #[tokio::test]
    async fn a_refusal_that_cannot_be_enqueued_is_reported_as_unanswered() {
        use engram_dashboard_protocol::RequestId;

        let factory = test_factory();
        let conn = factory.handler_for(1);
        // Capacity 1 — once one frame is put in beforehand, every later `try_send` fails.
        let (tx, _rx) = mpsc::channel::<Frame>(1);
        tx.try_send(Frame::Text("preempt".into()))
            .expect("fill one entry");
        let frames = frame_sink(tx);

        let write = command_json(&AgentCommand::WriteStdin {
            agent_id: uuid::Uuid::new_v4(),
            data: "x".into(),
            request_id: RequestId::new(),
        });
        assert_eq!(
            conn.on_inbound_saturated(1, &write, &frames),
            Saturated::Unanswered,
            "if Refused is returned though the refusal reply failed, the network layer believes a lie"
        );
    }

    #[tokio::test]
    async fn a_disconnect_removes_that_connections_commands_from_the_roster() {
        use engram_dashboard_command::CommandDecl;
        use engram_dashboard_protocol::{OwnerToken, RequestId};

        let factory = test_factory();
        let owner = factory.handler_for(1);
        let onlooker = factory.handler_for(2);
        let (owner_tx, mut owner_rx) = mpsc::channel::<Frame>(8);
        let (onlooker_tx, mut onlooker_rx) = mpsc::channel::<Frame>(8);
        let owner_frames = frame_sink(owner_tx);
        let onlooker_frames = frame_sink(onlooker_tx);
        // Exactly the production order — registration is accepted only after `on_connect` put the connection on the list.
        owner.on_connect(1, &owner_frames).await;
        onlooker.on_connect(2, &onlooker_frames).await;
        for _ in 0..2 {
            let _ = next_event(&mut owner_rx).await;
            let _ = next_event(&mut onlooker_rx).await;
        }

        let registration = command_json(&AgentCommand::RegisterCommands {
            owner: OwnerToken::new("shell"),
            decls: serde_json::value::to_raw_value(&[CommandDecl {
                name: "tab.create".to_string(),
                help: "{}".to_string(),
                agent_args: Vec::new(),
            }])
            .unwrap(),
            catalog_version: 1,
            request_id: RequestId(uuid::Uuid::new_v4()),
        });
        owner.on_text(1, &registration, &owner_frames).await;
        assert!(
            matches!(next_event(&mut owner_rx).await, AgentEvent::Ack { .. }),
            "registration is Ack"
        );

        let list = command_json(&AgentCommand::ListCommands {
            request_id: RequestId(uuid::Uuid::new_v4()),
        });
        onlooker.on_text(2, &list, &onlooker_frames).await;
        match next_event(&mut onlooker_rx).await {
            AgentEvent::CommandList { entries, .. } => {
                assert_eq!(
                    entries.len(),
                    1,
                    "another connection sees the same roster too"
                );
                assert_eq!(entries[0].name, "tab.create");
            }
            other => panic!("expected CommandList: {other:?}"),
        }

        owner.on_disconnect(1);

        onlooker.on_text(2, &list, &onlooker_frames).await;
        match next_event(&mut onlooker_rx).await {
            AgentEvent::CommandList { entries, .. } => {
                assert!(
                    entries.is_empty(),
                    "disappears without a trace: {entries:?}"
                )
            }
            other => panic!("expected CommandList: {other:?}"),
        }

        // A frame arriving after cleanup finished (the tail of the overlap) is the easy side; the overlap during cleanup is
        //   `..._detached_before_the_rest_of_the_cleanup` below.
        owner.on_text(1, &registration, &owner_frames).await;
        match next_event(&mut owner_rx).await {
            AgentEvent::Error { message, .. } => assert!(
                message.contains(crate::command_roster::DETACHED_REFUSAL),
                "it must be what the connection lifetime net caught: {message}"
            ),
            other => panic!("expected Error: {other:?}"),
        }
        onlooker.on_text(2, &list, &onlooker_frames).await;
        match next_event(&mut onlooker_rx).await {
            AgentEvent::CommandList { entries, .. } => assert!(
                entries.is_empty(),
                "a late registration must not revive the name: {entries:?}"
            ),
            other => panic!("expected CommandList: {other:?}"),
        }
    }

    /// Also reaps the command round trips issued by the disconnected connection.
    ///
    /// The roster and the correlation table are different tables (different lifetime units — `command_delivery` header), so cleanup is two lines. Without the
    /// correlation-table line an entry with nowhere to take its reply sits until the deadline with its round-trip task, and the runtime gives no signal — so
    /// this test is that line's only net (per-branch reaping assertions are in `command_delivery`).
    // ADR-0154
    #[tokio::test]
    async fn a_disconnect_also_reclaims_that_connections_command_round_trips() {
        use engram_dashboard_command::{CommandDecl, CommandEnvelope, OwnerToken, RequestId};
        use engram_dashboard_protocol::OwnerToken as WireOwnerToken;

        let commands = CommandRoster::new();
        let deliveries = CommandDeliveries::new();
        let factory = test_factory_with(
            Arc::new(crate::test_doubles::RecordingFanout::new()),
            commands,
            deliveries.clone(),
            crate::usage_service::fakes::idle_service(),
        );
        let owner = factory.handler_for(1);
        let caller = factory.handler_for(2);
        let (owner_tx, mut owner_rx) = mpsc::channel::<Frame>(8);
        let (caller_tx, mut caller_rx) = mpsc::channel::<Frame>(8);
        let owner_frames = frame_sink(owner_tx);
        let caller_frames = frame_sink(caller_tx);
        owner.on_connect(1, &owner_frames).await;
        caller.on_connect(2, &caller_frames).await;
        for _ in 0..2 {
            let _ = next_event(&mut owner_rx).await;
            let _ = next_event(&mut caller_rx).await;
        }
        let registration = command_json(&AgentCommand::RegisterCommands {
            owner: WireOwnerToken::new("shell"),
            decls: serde_json::value::to_raw_value(&[CommandDecl {
                name: "tab.create".to_string(),
                help: "{}".to_string(),
                agent_args: Vec::new(),
            }])
            .unwrap(),
            catalog_version: 1,
            request_id: engram_dashboard_protocol::RequestId(uuid::Uuid::new_v4()),
        });
        owner.on_text(1, &registration, &owner_frames).await;
        assert!(matches!(
            next_event(&mut owner_rx).await,
            AgentEvent::Ack { .. }
        ));

        let envelope = CommandEnvelope {
            name: "tab.create".to_string(),
            request_id: RequestId::new(),
            owner: OwnerToken::new("whatever-the-caller-thinks"),
            proto_ver: 1,
            args: serde_json::json!({}),
        };
        let request = command_json(&AgentCommand::Command {
            envelope: serde_json::value::to_raw_value(&envelope).expect("envelope serialization"),
        });
        caller.on_text(2, &request, &caller_frames).await;
        // From the moment the envelope is seen to go out to the owner, that round trip is waiting for a reply.
        assert!(matches!(
            next_event(&mut owner_rx).await,
            AgentEvent::CommandRequest { .. }
        ));
        assert_eq!(
            deliveries.in_flight(),
            1,
            "the round trip sits in the table"
        );

        caller.on_disconnect(2);

        assert_eq!(
            deliveries.in_flight(),
            0,
            "cleanup must reap this connection's round trips"
        );
    }

    /// Pins that the first line of cleanup is `detach`: if it slips back one position, this connection reads as attached while the preceding steps run, and an
    /// overlapping registration passes — a dead connection takes back a live one's name.
    ///
    /// Observation: the only double pluggable into cleanup is the lease-release notice, but that is the last step (looking there shows only "absent just
    /// before the end"; a middle placement passes). So the test first holds the lock the first step takes (`session.subs`) and runs cleanup on another
    /// thread — it stops at that lock, so if this connection is already off the list while stopped, `detach` is ahead of that lock.
    /// No deadlock: the cleanup thread's roster lock ends inside `detach`, and the test takes the roster briefly while holding its lock. The probe does
    /// not touch the roster (an empty diff passes through).
    #[test]
    fn the_command_roster_is_detached_before_the_first_cleanup_step() {
        use std::time::{Duration, Instant};

        let commands = CommandRoster::new();
        let factory = test_factory_with(
            Arc::new(crate::test_doubles::RecordingFanout::new()),
            commands.clone(),
            CommandDeliveries::new(),
            crate::usage_service::fakes::idle_service(),
        );
        let conn = factory.build(1);
        let session = conn.session.clone();
        let (tx, _rx) = mpsc::channel::<Frame>(1);
        commands.attach(1, &frame_sink(tx));

        let held = session.subs.lock().expect("subs poisoned");
        let cleanup = std::thread::spawn(move || conn.on_disconnect(1));

        // Observe while cleanup is stopped at the lock. The thread may not have started yet, so repeat briefly; if `detach` is behind the lock a refusal
        //   never appears in this window, so the deadline is the verdict.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut refused = None;
        while Instant::now() < deadline {
            if let Err(e) = commands.update(1, vec![], vec![]) {
                refused = Some(e);
                break;
            }
            std::thread::yield_now();
        }

        drop(held);
        cleanup.join().expect("cleanup thread");

        let refused = refused.expect("while cleanup is stopped at the first step this connection must already be off the list");
        assert_eq!(
            refused.message(),
            crate::command_roster::DETACHED_REFUSAL,
            "it must be what the connection lifetime net caught"
        );
    }

    // ── 5. usage plane (TRD S21 usage-limit-slot §4) ──

    mod usage {
        use super::*;
        use crate::usage_service::clock::ManualUsageClock;
        use crate::usage_service::fakes;
        use crate::usage_service::watch::{deliver, Target};
        use engram_dashboard_agent::backend::usage_probe_for;
        use engram_dashboard_agent::usage::{UsageObservation, UsageProbe, UsageSource, WindowObs};
        use engram_dashboard_protocol::{AgentBackendKind, RequestId, UsageVendorState};
        use std::time::Duration;

        use AgentBackendKind::{Claude, Codex};

        const T0: i64 = 1_900_000_000;
        const FAR_RESET: i64 = T0 + 5 * 3_600;
        /// 5-hour window utilization the fake probe returns.
        const PROBED_PCT: f64 = 37.5;

        fn probe_of(kind: AgentBackendKind) -> &'static dyn UsageProbe {
            let word = serde_json::to_value(kind).expect("wire word");
            usage_probe_for(word.as_str().expect("string")).expect("probe")
        }

        fn key_of(kind: AgentBackendKind) -> UsageVendorKey {
            probe_of(kind).key()
        }

        fn keys(kinds: &[AgentBackendKind]) -> BTreeSet<UsageVendorKey> {
            kinds.iter().map(|&k| key_of(k)).collect()
        }

        fn passive(pct: f64, resets_at: i64) -> UsageObservation {
            UsageObservation {
                vendor: key_of(Claude),
                five_hour: Some(WindowObs {
                    used_pct: Some(pct),
                    resets_at: Some(resets_at),
                }),
                weekly: None,
                model_scoped: None,
                plan: None,
                source: UsageSource::Passive,
                limits_unavailable: None,
            }
        }

        fn pct(sheet: &UsageLimitSnapshot) -> Option<f64> {
            sheet.five_hour.as_ref().and_then(|w| w.used_pct)
        }

        struct Rig {
            factory: AgentConnections,
            service: Arc<UsageService>,
            clock: Arc<ManualUsageClock>,
            gate: fakes::ProbeGate,
        }

        /// Entry = one claude (fake probe — blocks until the test releases it). codex has a probe so it joins the subscription set, but this service has no
        /// entry for it, so no sheet. No scheduler — the test drives the time.
        fn rig() -> Rig {
            rig_waiting(None)
        }

        /// `reply_wait` = ⟳ reply wait cap; `None` means the production cap.
        fn rig_waiting(reply_wait: Option<Duration>) -> Rig {
            let clock = Arc::new(ManualUsageClock::new(Duration::from_secs(100), T0));
            let (probe, gate) = fakes::gated_probe(probe_of(Claude), PROBED_PCT, FAR_RESET);
            let service = match reply_wait {
                Some(wait) => fakes::service_with_wait(vec![probe], clock.clone(), wait),
                None => fakes::service(vec![probe], clock.clone()),
            };
            let factory = test_factory_with(
                Arc::new(crate::test_doubles::RecordingFanout::new()),
                CommandRoster::new(),
                CommandDeliveries::new(),
                service.clone(),
            );
            Rig {
                factory,
                service,
                clock,
                gate,
            }
        }

        struct Conn {
            id: ConnId,
            handler: Arc<dyn ConnectionHandler>,
            frames: Arc<dyn FrameSink>,
            rx: mpsc::Receiver<Frame>,
        }

        impl Conn {
            async fn open(rig: &Rig, id: ConnId) -> Self {
                let handler = rig.factory.handler_for(id);
                let (tx, rx) = mpsc::channel::<Frame>(64);
                let frames = frame_sink(tx);
                handler.on_connect(id, &frames).await;
                Self {
                    id,
                    handler,
                    frames,
                    rx,
                }
            }

            /// Attaches and drains the two greeting frames.
            async fn greeted(rig: &Rig, id: ConnId) -> Self {
                let mut conn = Self::open(rig, id).await;
                let events = conn.events();
                assert!(
                    matches!(
                        events.as_slice(),
                        [
                            AgentEvent::Hello { .. },
                            AgentEvent::AgentListUpdated { .. }
                        ]
                    ),
                    "{events:?}"
                );
                conn
            }

            async fn send(&self, cmd: AgentCommand) {
                let flow = self
                    .handler
                    .on_text(self.id, &command_json(&cmd), &self.frames)
                    .await;
                assert_eq!(flow, ConnFlow::Continue);
            }

            async fn subscribe(&self, vendors: &[AgentBackendKind]) {
                self.send(AgentCommand::UsageSubscribe {
                    vendors: vendors.to_vec(),
                })
                .await;
            }

            /// Every event currently in the queue — does not wait.
            fn events(&mut self) -> Vec<AgentEvent> {
                let mut out = Vec::new();
                while let Ok(frame) = self.rx.try_recv() {
                    match frame {
                        Frame::Text(text) => {
                            out.push(serde_json::from_str(&text).expect("event decode"))
                        }
                        other => panic!("expected Text: {other:?}"),
                    }
                }
                out
            }

            /// The usage sheets currently in the queue — a failure if another event is mixed in.
            fn sheets(&mut self) -> Vec<(UsageLimitSnapshot, Vec<AgentBackendKind>)> {
                self.events()
                    .into_iter()
                    .map(|ev| match ev {
                        AgentEvent::UsageLimitsUpdated {
                            snapshot,
                            subscribed,
                        } => (snapshot, subscribed),
                        other => panic!("expected a usage sheet: {other:?}"),
                    })
                    .collect()
            }
        }

        /// A line built by the real encoder goes through the real outlet into that connection's frame outlet once. A reaped outlet is silent and has released its frame-outlet copy.
        #[tokio::test]
        async fn the_encoded_line_reaches_the_frame_sink_once_through_the_outlet() {
            let (tx, mut rx) = mpsc::channel::<Frame>(8);
            let frames = frame_sink(tx);
            let outlet: Arc<dyn UsageOutlet> = Arc::new(ConnUsageOutlet::new(frames.clone()));
            let snapshot = UsageLimitSnapshot {
                vendor: Claude,
                account_key: "default".into(),
                five_hour: None,
                weekly: None,
                model_scoped: vec![],
                plan: None,
                in_flight: false,
                state: UsageVendorState::Ready,
                revision: 4,
            };
            let subscribed = Arc::new(keys(&[Codex, Claude]));
            let target = || Target {
                outlet: outlet.clone(),
                subscribed: subscribed.clone(),
            };

            deliver(&snapshot, vec![target()], &UsageEventEncoder);
            let Ok(Frame::Text(text)) = rx.try_recv() else {
                panic!("expected one Text frame");
            };
            assert!(rx.try_recv().is_err(), "enters only once");
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&text).unwrap(),
                serde_json::to_value(AgentEvent::UsageLimitsUpdated {
                    snapshot: snapshot.clone(),
                    subscribed: vec![Claude, Codex],
                })
                .unwrap()
            );

            outlet.revoke();
            assert_eq!(Arc::strong_count(&frames), 1, "reaping releases the copy");
            deliver(&snapshot, vec![target()], &UsageEventEncoder);
            assert!(rx.try_recv().is_err(), "a reaped outlet is silent");
        }

        /// The first sheet goes after the two greeting frames, and only to subscribed connections.
        #[tokio::test]
        async fn a_first_sheet_follows_the_greeting_and_reaches_only_the_subscriber() {
            let rig = rig();
            let mut a = Conn::open(&rig, 1).await;
            let mut b = Conn::greeted(&rig, 2).await;

            a.subscribe(&[Claude]).await;

            let events = a.events();
            let [AgentEvent::Hello { .. }, AgentEvent::AgentListUpdated { .. }, AgentEvent::UsageLimitsUpdated {
                snapshot,
                subscribed,
            }] = events.as_slice()
            else {
                panic!("expected two greetings → first sheet: {events:?}");
            };
            assert_eq!(snapshot.vendor, Claude);
            assert_eq!(subscribed, &[Claude]);
            assert!(b.events().is_empty());
            assert_eq!(fakes::union(&rig.service), keys(&[Claude]));
        }

        /// The reply to ⟳ is a valueless `Ack` sent after the probe ends, and the value goes by broadcast to subscribed connections — a sending connection that
        /// is not subscribed gets no sheet.
        #[tokio::test]
        async fn a_refresh_answers_a_bare_ack_and_the_value_goes_to_the_subscriber() {
            // Set the wait cap long: during the blocked probe "B has no reply yet" is asserted, and the production cap (5s) ending first on a slow runner would fail falsely.
            let mut rig = rig_waiting(Some(Duration::from_secs(60)));
            let mut a = Conn::greeted(&rig, 1).await;
            let mut b = Conn::greeted(&rig, 2).await;
            a.subscribe(&[Claude]).await;
            let first = a.sheets();
            assert_eq!(first.len(), 1);

            let r = RequestId::new();
            b.send(AgentCommand::RefreshUsageLimits {
                vendor: Claude,
                request_id: r,
            })
            .await;
            // While the probe is blocked: the "refreshing" sheet went out before the probe launched, the reply not yet.
            tokio::time::timeout(Duration::from_secs(10), rig.gate.entered.recv())
                .await
                .expect("the probe must start");
            let started = a.sheets();
            assert!(
                matches!(started.as_slice(), [(sheet, _)] if sheet.in_flight),
                "{started:?}"
            );
            assert!(
                b.events().is_empty(),
                "the reply waits for the probe to end"
            );

            rig.gate.release.send(()).expect("blocked probe");
            // Receiving B's reply is the synchronization — the end-of-probe sheet goes out before waking the waiting request.
            match next_event(&mut b.rx).await {
                AgentEvent::Ack { request_id } => assert_eq!(request_id, r),
                other => panic!("expected Ack: {other:?}"),
            }
            assert!(
                b.events().is_empty(),
                "no value for an unsubscribed connection"
            );

            let ended = a.sheets();
            let [(done, subscribed)] = ended.as_slice() else {
                panic!("expected the end-of-probe sheet: {ended:?}");
            };
            assert!(!done.in_flight);
            assert_eq!(pct(done), Some(PROBED_PCT));
            assert_eq!(subscribed, &[Claude]);
            assert!(first[0].0.revision < started[0].0.revision);
            assert!(started[0].0.revision < done.revision);
        }

        /// A ⟳ for a vendor with no entry is not a silent `Ack` but an `Error` of that number.
        #[tokio::test]
        async fn a_refresh_for_a_vendor_without_a_slot_is_an_error_with_its_request_id() {
            let rig = rig();
            let mut b = Conn::greeted(&rig, 2).await;

            let r = RequestId::new();
            b.send(AgentCommand::RefreshUsageLimits {
                vendor: Codex,
                request_id: r,
            })
            .await;
            match next_event(&mut b.rx).await {
                AgentEvent::Error {
                    request_id,
                    message,
                } => {
                    assert_eq!(request_id, Some(r));
                    assert!(message.starts_with("NOT_FOUND: "), "{message}");
                }
                other => panic!("expected Error: {other:?}"),
            }
        }

        #[tokio::test]
        async fn a_passive_change_reaches_only_the_subscriber() {
            let rig = rig();
            let mut a = Conn::greeted(&rig, 1).await;
            let mut b = Conn::greeted(&rig, 2).await;
            a.subscribe(&[Claude]).await;
            a.events();

            rig.service.observe(&passive(40.0, FAR_RESET));

            let sheets = a.sheets();
            assert_eq!(sheets.len(), 1);
            assert_eq!(pct(&sheets[0].0), Some(40.0));
            assert!(b.events().is_empty());
        }

        /// On disconnect the entry is erased (dropped from the union) and the frame-outlet copy the outlet holds is released; a late replacement overlapping after disconnect does not revive it.
        #[tokio::test]
        async fn a_disconnect_removes_the_slot_and_a_late_subscription_does_not_revive_it() {
            let rig = rig();
            let mut a = Conn::greeted(&rig, 1).await;
            let b = Conn::greeted(&rig, 2).await;
            a.subscribe(&[Claude]).await;
            b.subscribe(&[Codex]).await;
            a.events();

            a.handler.on_disconnect(1);
            assert_eq!(fakes::union(&rig.service), keys(&[Codex]));
            assert_eq!(
                Arc::strong_count(&a.frames),
                1,
                "the frame-outlet copy held by the two rosters is released inside cleanup"
            );

            a.subscribe(&[Claude]).await;
            assert_eq!(fakes::union(&rig.service), keys(&[Codex]));
            rig.service.observe(&passive(40.0, FAR_RESET));
            assert!(a.events().is_empty());
        }

        /// One connection's replacement takes effect before `on_text` returns, so it is arrival-ordered — the last one wins.
        #[tokio::test]
        async fn replacements_on_one_connection_apply_in_arrival_order() {
            let rig = rig();
            let mut a = Conn::greeted(&rig, 1).await;

            a.subscribe(&[Claude, Codex]).await;
            assert_eq!(fakes::union(&rig.service), keys(&[Claude, Codex]));
            a.subscribe(&[]).await;
            assert!(fakes::union(&rig.service).is_empty());
            a.subscribe(&[Codex]).await;
            assert_eq!(fakes::union(&rig.service), keys(&[Codex]));

            let sheets = a.sheets();
            assert_eq!(
                sheets.len(),
                1,
                "only the first replacement carrying a vendor with an entry gives one sheet: {sheets:?}"
            );
            assert_eq!(sheets[0].1, [Claude, Codex]);
        }

        /// The shell resends the same set without suppression — that does not create a feedback loop.
        #[tokio::test]
        async fn a_same_set_replacement_sends_nothing() {
            let rig = rig();
            let mut a = Conn::greeted(&rig, 1).await;
            let mut b = Conn::greeted(&rig, 2).await;
            a.subscribe(&[Claude]).await;
            rig.service.observe(&passive(40.0, FAR_RESET));
            assert_eq!(a.sheets().len(), 2);

            a.subscribe(&[Claude]).await;
            a.subscribe(&[Claude]).await;
            assert!(a.events().is_empty());
            assert!(b.events().is_empty());
        }

        /// Latch exception — a window whose reset has passed at replacement time goes out as exactly one sheet carrying the pre-replacement set; repeating gives 0.
        #[tokio::test]
        async fn a_latch_due_at_replacement_goes_out_once_with_the_set_before_it() {
            let rig = rig();
            let mut a = Conn::greeted(&rig, 1).await;
            a.subscribe(&[Claude, Codex]).await;
            rig.service.observe(&passive(40.0, T0 + 60));
            assert_eq!(a.sheets().len(), 2);
            rig.clock.set_wall(T0 + 120);

            a.subscribe(&[Claude]).await;
            let sheets = a.sheets();
            let [(latched, subscribed)] = sheets.as_slice() else {
                panic!("expected one latch sheet: {sheets:?}");
            };
            assert!(latched.five_hour.as_ref().expect("window").expired);
            assert_eq!(subscribed, &[Claude, Codex], "pre-replacement set");

            a.subscribe(&[Claude]).await;
            a.subscribe(&[Claude]).await;
            assert!(a.events().is_empty());
        }
    }
}
