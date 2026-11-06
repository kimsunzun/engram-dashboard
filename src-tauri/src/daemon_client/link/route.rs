//! What one daemon's inbox items become for the shell (TRD S21 trd-3 §3-3 · §6-2 U2): the key filter, connection-state
//! edges, notice values, and the split between the frontend and the driver.
//!
//! ★Owned by the driver actor★ — [`RouteState`] is `&mut`, never behind a lock, and [`emit_broadcast`] takes no
//! routing state at all, so no routing lock can be held while a port runs (the real `AgentNamesSink` takes the layout
//! lock). ★The driver task never awaits anything that waits on an inbox★ (transport invariant 1, transitively) —
//! `Peer::request`, a `Peer::close_after_flush` future, `Registry::close_all`: when the driver lags, the pump waits on
//! it, the inbox fills and the transport stops reading that daemon, so a reply (or the end those waits need) could not
//! arrive. Run them on another task.
//! ★This module is the only place the link's connection state and lifecycle are logged★ — the transport has no
//! logging of its own. Lifecycle and refusal logging here never carries the daemon's text, and no daemon text reaches the
//! UI from here; the one exception is the moved `emit_broadcast`, which still warn-logs a daemon `Error` broadcast's
//! message as the old driver did.
//!
//! ## Key filter — every item of a key (§3-3, three rules)
//!
//! 1. Handle — an item from a handle that is not the key's current one is dropped (the old supervisor's tail after a
//!    re-add of the key).
//! 2. Generation — an item that carries a connection generation (messages · `Connected` · `Disconnected` · `Closed` ·
//!    `DecodeFailed`) older than the newest `Connected` this handle's inbox delivered is dropped. ★The watermark is
//!    what this inbox showed, never `Peer::generation()`★ — the live counter runs ahead of a lagging inbox and would
//!    drop in-order messages, including the read-ahead the transport queues just before `Disconnected`.
//! 3. Generation-less lifecycle events (`ConnectFailed` · `Rejected` · `GaveUp`) of the current handle always pass —
//!    also for a handle that never connected.
//!
//! Dropped items are logged only.
//!
//! ## Connection failures
//!
//! A handle that has not connected yet shows its first failed attempt as `Down` at once — a first connect has no
//! `Reconnecting` phase (TRD revision 9 ①); the transport keeps retrying in the background and a later `Connected` is
//! an edge. Failed attempts are logged throttled (`FailureLog` — ADR-0346 decision 5): retries no longer run out, so
//! a line per attempt would repeat for as long as the daemon is away.
//!
//! ## Replies skip this path (§3-3 B2)
//!
//! A reply wakes its `request` inside the transport and never reaches the inbox, so it can resolve before events the
//! daemon sent ahead of it are routed (pinned by a test below; the caller survey is in the TRD §6-2 U2 C note).
// ADR-0310
// ADR-0342
// ADR-0346

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use engram_dashboard_command::CommandReply as BusReply;
use engram_dashboard_protocol::{AgentCommand, AgentEvent};
use engram_dashboard_transport::{
    Arrival, Clock, CloseCode, ConnectFailure, GaveUpReason, Generation, Incoming, Peer,
    TransportEvent,
};

use super::dialer::{BUSY_UNKNOWN, NOT_REGISTERED, NO_RECORD, READ_FAILED, UNUSABLE};
use super::wire::{ShellIn, ShellWire};
use super::{DaemonKey, HandleToken};
use crate::daemon_client::events::DaemonEvents;
use crate::daemon_client::protocol_state;
use crate::daemon_client::{AgentNamesSink, ConnectionState};

/// One routed result, in the order the driver applies them.
#[cfg_attr(test, derive(Debug))]
pub enum Routed {
    /// Connection `generation` of the current handle is live — the driver opens its per-connection state.
    LinkUp { generation: Generation },
    /// Connection `generation` ended — the driver drops its per-connection state (replay flights, the usage socket).
    /// Exactly one per `LinkUp`, also when a re-add ([`RouteState::adopt`]) or an abnormal inbox end ([`route_end`])
    /// ends it.
    LinkDown { generation: Generation },
    /// A connection-state edge for the frontend: emitted only when the shown state changes. Never `Connecting`.
    /// ★Routing is the only emitter of the link's connection state★ — a driver that also emitted (e.g. `Down` from its
    /// own `close()`) would double the edges; a close shows `Down` through the `Closed` it produces.
    State(ConnectionState),
    /// A value for the connection-failure surface (`ConnectionNotice` — ADR-0342).
    Notice(LinkNotice),
    /// An event for the frontend — the driver hands it to [`emit_broadcast`].
    Broadcast(AgentEvent),
    /// An event the driver handles itself (replay · daemon requests · broken replies · usage) — the same split as the
    /// old driver's read arm (`connection.rs` `main_loop`).
    ToDriver {
        generation: Generation,
        event: AgentEvent,
    },
    /// A binary output frame for the output relay, undecoded.
    Output {
        generation: Generation,
        bytes: Vec<u8>,
    },
}

/// What the connection-failure surface gets. A value only — what the UI does with it is wired in 3-2 U4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkNotice {
    /// The daemon refused for good — `GaveUp(Rejected)` follows for a terminal refusal; for a refusal the transport
    /// would retry, the driver removes the peer instead (`Closed` follows and the registry no longer holds the peer).
    /// The UI text is [`refusal_message`]`(code)` —
    /// never the daemon's own text (ADR-0342).
    Refused { code: CloseCode },
    /// The reconnect budget ran out (`GaveUp(BudgetExhausted)`). `last` = the cause of the last failed attempt since
    /// the handle's last `Connected` (`None` = none failed). ★The shell's driver never reaches it★ — it retries network
    /// failures without limit (ADR-0346); only a registry built with a finite budget produces it.
    BudgetExhausted { last: Option<FailureCause> },
}

/// Why one connection attempt failed — the daemon's text is not kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureCause {
    Unreachable,
    Refused { code: CloseCode },
}

impl FailureCause {
    fn of(failure: &ConnectFailure) -> Self {
        match failure {
            ConnectFailure::Unreachable(_) => FailureCause::Unreachable,
            ConnectFailure::Rejected { code, .. } => FailureCause::Refused { code: *code },
        }
    }
}

/// What the failure log tells apart — [`FailureCause`] with the kind of an unreachable daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    Unreachable(UnreachableClass),
    Refused { code: CloseCode },
}

impl FailureKind {
    fn of(failure: &ConnectFailure) -> Self {
        match failure {
            ConnectFailure::Unreachable(e) => {
                FailureKind::Unreachable(UnreachableClass::of(&e.message))
            }
            ConnectFailure::Rejected { code, .. } => FailureKind::Refused { code: *code },
        }
    }
}

/// Why a daemon could not be reached, as far as a stable log key goes. ★Read from the failure text and used only to
/// throttle the log★ — a class is never a decision, and a text no rule knows is `Other`. Not the raw text: it carries
/// ports and OS detail that can change per attempt and would defeat the throttle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnreachableClass {
    /// No `daemon.json` (`link::dialer`).
    NoRecord,
    /// A `daemon.json` that cannot be used — broken · dead pid · access denied (`link::dialer`).
    RecordUnusable,
    /// The record is held by another process and no address is known (`link::dialer`).
    RecordBusy,
    /// The registration or its endpoint read is gone (`link::dialer`).
    Registration,
    /// The dial or the handshake ran out of time (the transport's texts).
    TimedOut,
    /// The daemon closed the link during the handshake (the transport's text).
    ClosedInHandshake,
    /// The ws dial itself failed — refused, reset, … (the transport's ws dialer).
    Dial,
    Other,
}

impl UnreachableClass {
    fn of(text: &str) -> Self {
        let starts = |prefix: &str| text.starts_with(prefix);
        if starts(NO_RECORD) {
            UnreachableClass::NoRecord
        } else if starts(UNUSABLE) {
            UnreachableClass::RecordUnusable
        } else if starts(BUSY_UNKNOWN) {
            UnreachableClass::RecordBusy
        } else if starts(NOT_REGISTERED) || starts(READ_FAILED) {
            UnreachableClass::Registration
        } else if matches!(
            text,
            "connect timed out" | "handshake timed out" | "handshake write timed out"
        ) {
            UnreachableClass::TimedOut
        } else if text == "peer closed during handshake" {
            UnreachableClass::ClosedInHandshake
        } else if starts("ws connect ") {
            UnreachableClass::Dial
        } else {
            UnreachableClass::Other
        }
    }
}

/// The fixed local text for a terminal refusal with `code` — ★the one place a close code becomes UI text★
/// (ADR-0342). It states the cause only — ADR-0343: the restart action lands in 3-2 U5; until then add no instruction
/// here (the tray stop does nothing on a version mismatch). ★4001 does not say why★: it is the daemon's refusal of the
/// handshake, whatever the cause (the shell never reads the daemon's text — `ShellHandshake`: token, version or
/// anything else), or a local version mismatch (`link::dialer`); 3-3 U7's identity refusal will close with it too. The
/// text for it is a pending user decision.
pub fn refusal_message(code: CloseCode) -> &'static str {
    match code {
        CloseCode::HANDSHAKE_REJECTED => "실행 중인 데몬이 이 앱과 버전이 다릅니다",
        _ => "데몬이 연결을 거절했습니다",
    }
}

/// The routing state of every key — see the module doc. `clock` times the failure log only — pass the registry's.
pub struct RouteState {
    keys: HashMap<DaemonKey, KeyRoute>,
    clock: Arc<dyn Clock>,
}

impl RouteState {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            keys: HashMap::new(),
            clock,
        }
    }

    /// `token` is now the key's current handle (one `Registry::add`): every later item of an earlier handle of the key
    /// is dropped, and the handle's generation watermark and failure memory start over.
    ///
    /// ★Call it in the same driver step as the `Registry::add` and its `spawn_pump`, before any item of `token` is
    /// routed★ — an item routed earlier is dropped as foreign. ★Apply what it returns★: the earlier handle's open
    /// connection ends here (`LinkDown` — its own `Disconnected` is now foreign and gets dropped; `Registry::add` closes
    /// the displaced supervisor only after the new one is in the roster), and a shown `Connected` turns `Reconnecting`
    /// so the new handle's `Connected` is an edge the frontend resyncs on.
    #[must_use]
    pub fn adopt(&mut self, key: DaemonKey, token: HandleToken) -> Vec<Routed> {
        let mut out = Vec::new();
        let mut shown = None;
        if let Some(old) = self.keys.remove(&key) {
            tracing::info!(peer = %key.peer_id(), "a new handle replaces the daemon link");
            if let Some(generation) = old.open {
                out.push(Routed::LinkDown { generation });
            }
            shown = old.shown;
            if shown == Some(ConnectionState::Connected) {
                shown = Some(ConnectionState::Reconnecting);
                out.push(Routed::State(ConnectionState::Reconnecting));
            }
        }
        self.keys.insert(
            key,
            KeyRoute {
                token,
                seen: None,
                open: None,
                last_failure: None,
                failures: FailureLog::default(),
                shown,
                closed: false,
            },
        );
        out
    }
}

struct KeyRoute {
    token: HandleToken,
    /// The newest `Connected` generation this handle's inbox delivered (rule ②).
    seen: Option<Generation>,
    /// The connection given to the driver as `LinkUp` and not yet as `LinkDown`.
    open: Option<Generation>,
    last_failure: Option<FailureCause>,
    failures: FailureLog,
    shown: Option<ConnectionState>,
    /// `Closed` was routed — the inbox ends cleanly after it.
    closed: bool,
}

/// Throttles the log of one handle's failed attempts (ADR-0346 decision 5): the first failure and every change of
/// cause get a line; while the same cause repeats, one summary line at most every [`FailureLog::SUMMARY_EVERY`], with
/// the count since the previous line. A `Connected` starts it over.
#[derive(Default)]
struct FailureLog {
    cause: Option<FailureKind>,
    /// Failures of `cause` since the last line.
    unlogged: u32,
    line_at: Option<Instant>,
}

/// What [`FailureLog::record`] says to log.
#[derive(Debug, PartialEq, Eq)]
enum FailureLine {
    /// The first failure, or another cause than the previous one. `unlogged` = failures of the previous cause that no
    /// line counted yet.
    New { unlogged: u32 },
    /// The same cause, [`FailureLog::SUMMARY_EVERY`] after the last line: `count` failures since that line.
    Repeated { count: u32 },
    /// The same cause, within [`FailureLog::SUMMARY_EVERY`] of the last line.
    Quiet,
}

impl FailureLog {
    const SUMMARY_EVERY: Duration = Duration::from_secs(60);

    fn record(&mut self, cause: FailureKind, now: Instant) -> FailureLine {
        if self.cause != Some(cause) {
            let unlogged = std::mem::take(&mut self.unlogged);
            self.cause = Some(cause);
            self.line_at = Some(now);
            return FailureLine::New { unlogged };
        }
        self.unlogged = self.unlogged.saturating_add(1);
        let due = self
            .line_at
            .is_none_or(|at| now.saturating_duration_since(at) >= Self::SUMMARY_EVERY);
        if !due {
            return FailureLine::Quiet;
        }
        self.line_at = Some(now);
        FailureLine::Repeated {
            count: std::mem::take(&mut self.unlogged),
        }
    }

    /// Starts over; returns the failures no line counted.
    fn reset(&mut self) -> u32 {
        std::mem::take(self).unlogged
    }
}

/// Routes one item of `key`'s handle `token` (the pump's tag). Returns nothing for a dropped or log-only item.
pub fn route(
    st: &mut RouteState,
    key: &DaemonKey,
    token: HandleToken,
    item: Incoming<ShellWire>,
) -> Vec<Routed> {
    let RouteState { keys, clock } = st;
    let Some(route) = of_current_handle(keys.get_mut(key), token) else {
        tracing::debug!(
            peer = %key.peer_id(),
            item = item_kind(&item),
            "dropped an item of a handle that is not the key's current one"
        );
        return Vec::new();
    };
    match item {
        Incoming::Message(arrival) => route.message(key, arrival),
        Incoming::Event(event) => route.event(key, event, clock.as_ref()),
    }
}

/// Routes the end of `key`'s handle `token` inbox (the pump's `Routable::Ended` — after the last item). After `Closed`
/// that is the clean end and routes nothing. ★Without `Closed` the supervisor ended abnormally★ (transport invariant 5
/// — cleanup over its bound, a panic, runtime shutdown): the open connection ends (`LinkDown`) and `Down` is shown.
/// The driver unregisters the handle's dial registration on every `Ended` — current or replaced handle — not only when
/// this returns something (3-2 U3).
pub fn route_end(st: &mut RouteState, key: &DaemonKey, token: HandleToken) -> Vec<Routed> {
    let Some(route) = of_current_handle(st.keys.get_mut(key), token) else {
        tracing::debug!(peer = %key.peer_id(), "the inbox of a replaced handle ended");
        return Vec::new();
    };
    if route.closed {
        tracing::debug!(peer = %key.peer_id(), "the daemon link inbox ended");
        return Vec::new();
    }
    tracing::warn!(
        peer = %key.peer_id(),
        open = ?route.open.map(|g| g.0),
        "the daemon link inbox ended without Closed — the transport supervisor stopped abnormally"
    );
    let mut out: Vec<Routed> = route.link_down().into_iter().collect();
    out.extend(route.show(ConnectionState::Down));
    out
}

impl RouteState {
    /// `token` is `key`'s current handle and its connection is open as routed (`LinkUp` given, `LinkDown` not yet).
    pub fn is_open(&self, key: &DaemonKey, token: HandleToken) -> bool {
        of_current_handle(self.keys.get(key), token).is_some_and(|r| r.open.is_some())
    }
}

/// Rule ①: a key's route, kept only while `token` is that key's current handle.
fn of_current_handle<R: std::borrow::Borrow<KeyRoute>>(
    route: Option<R>,
    token: HandleToken,
) -> Option<R> {
    route.filter(|r| r.borrow().token == token)
}

fn item_kind(item: &Incoming<ShellWire>) -> &'static str {
    match item {
        Incoming::Message(_) => "message",
        Incoming::Event(event) => event.kind(),
    }
}

impl KeyRoute {
    fn is_stale(&self, generation: Generation) -> bool {
        self.seen.is_some_and(|seen| generation < seen)
    }

    fn drop_stale(&self, key: &DaemonKey, what: &str, generation: Generation) -> Vec<Routed> {
        tracing::debug!(
            peer = %key.peer_id(),
            what,
            generation = generation.0,
            seen = ?self.seen.map(|g| g.0),
            "dropped an item of an older connection"
        );
        Vec::new()
    }

    fn show(&mut self, state: ConnectionState) -> Option<Routed> {
        if self.shown == Some(state) {
            return None;
        }
        self.shown = Some(state);
        Some(Routed::State(state))
    }

    fn link_down(&mut self) -> Option<Routed> {
        self.open
            .take()
            .map(|generation| Routed::LinkDown { generation })
    }

    fn message(&mut self, key: &DaemonKey, arrival: Arrival<ShellWire>) -> Vec<Routed> {
        let generation = arrival.generation;
        if self.is_stale(generation) {
            return self.drop_stale(key, "message", generation);
        }
        vec![match arrival.msg {
            ShellIn::Output(bytes) => Routed::Output { generation, bytes },
            ShellIn::Event(event) if for_driver(&event) => Routed::ToDriver { generation, event },
            ShellIn::Event(event) => Routed::Broadcast(event),
        }]
    }

    fn event(
        &mut self,
        key: &DaemonKey,
        event: TransportEvent<ShellWire>,
        clock: &dyn Clock,
    ) -> Vec<Routed> {
        let peer = key.peer_id();
        match event {
            TransportEvent::Connected { generation, .. } => {
                if self.is_stale(generation) {
                    return self.drop_stale(key, "Connected", generation);
                }
                let unlogged_failures = self.failures.reset();
                tracing::info!(
                    %peer,
                    generation = generation.0,
                    unlogged_failures,
                    "daemon link connected"
                );
                self.seen = Some(generation);
                self.open = Some(generation);
                self.last_failure = None;
                let mut out = vec![Routed::LinkUp { generation }];
                out.extend(self.show(ConnectionState::Connected));
                out
            }
            // Every loss of a live connection shows `Reconnecting`, our own as well (`cause` `Explicit`): after
            //   `reconnect_now` a redial follows, and after `close` the `Closed` that follows shows `Down` — the
            //   frontend acts on reaching or leaving `connected`, never on which non-connected state it is
            //   (`tauriTransport.ts` `applyConnectionState` · `eventBus.ts`
            //   resync), so the brief `reconnecting` before `down` changes nothing there.
            TransportEvent::Disconnected {
                generation, cause, ..
            } => {
                if self.is_stale(generation) {
                    return self.drop_stale(key, "Disconnected", generation);
                }
                tracing::info!(%peer, generation = generation.0, ?cause, "daemon link lost");
                let mut out: Vec<Routed> = self.link_down().into_iter().collect();
                out.extend(self.show(ConnectionState::Reconnecting));
                out
            }
            // No state edge per attempt — only a handle that never connected shows its first failure (module doc).
            TransportEvent::ConnectFailed {
                attempt,
                cause,
                retry_in,
                ..
            } => {
                let failure = FailureCause::of(&cause);
                let kind = FailureKind::of(&cause);
                // The failure's text can carry the daemon's words — kind and close code only.
                match self.failures.record(kind, clock.now()) {
                    FailureLine::New { unlogged } => tracing::warn!(
                        %peer,
                        attempt,
                        ?kind,
                        ?retry_in,
                        unlogged_before = unlogged,
                        "a daemon connection attempt failed"
                    ),
                    FailureLine::Repeated { count } => tracing::warn!(
                        %peer,
                        attempt,
                        ?kind,
                        ?retry_in,
                        count,
                        "daemon connection attempts keep failing with the same cause"
                    ),
                    FailureLine::Quiet => tracing::debug!(
                        %peer,
                        attempt,
                        ?kind,
                        ?retry_in,
                        "the daemon connection failed again with the same cause"
                    ),
                }
                self.last_failure = Some(failure);
                if self.seen.is_none() {
                    self.show(ConnectionState::Down).into_iter().collect()
                } else {
                    Vec::new()
                }
            }
            TransportEvent::Rejected { code, reason, .. } => {
                // The daemon's text can echo what we sent (the Auth token) — its length only.
                tracing::warn!(
                    %peer,
                    code = code.0,
                    reason_bytes = reason.len(),
                    "the daemon refused the connection for good"
                );
                vec![Routed::Notice(LinkNotice::Refused { code })]
            }
            TransportEvent::GaveUp { reason, .. } => {
                tracing::warn!(%peer, ?reason, last = ?self.last_failure, "daemon link gave up — it stays down");
                let mut out = Vec::new();
                if reason == GaveUpReason::BudgetExhausted {
                    out.push(Routed::Notice(LinkNotice::BudgetExhausted {
                        last: self.last_failure,
                    }));
                }
                out.extend(self.show(ConnectionState::Down));
                out
            }
            TransportEvent::Closed {
                generation, cause, ..
            } => {
                if self.is_stale(generation) {
                    return self.drop_stale(key, "Closed", generation);
                }
                tracing::info!(%peer, generation = generation.0, ?cause, "daemon link closed");
                self.closed = true;
                self.show(ConnectionState::Down).into_iter().collect()
            }
            TransportEvent::DecodeFailed {
                generation, reason, ..
            } => {
                if self.is_stale(generation) {
                    return self.drop_stale(key, "DecodeFailed", generation);
                }
                tracing::warn!(%peer, generation = generation.0, reason, "a daemon text frame did not decode — dropped");
                Vec::new()
            }
            // Log only, never a UI warning: the count mixes real loss with replies we bound to a connection that ended
            //   before they were written (`notify_in` after a reconnect — transport `Direction::Outbound`).
            TransportEvent::Dropped {
                direction, count, ..
            } => {
                tracing::info!(%peer, ?direction, count, "frames to the daemon were not written");
                Vec::new()
            }
            // The waiting caller already got the error.
            TransportEvent::RequestTimedOut { tag, after, .. } => {
                tracing::debug!(%peer, request_id = %tag.0, ?after, "a daemon request timed out");
                Vec::new()
            }
            TransportEvent::UnmatchedReply { tag, .. } => {
                tracing::debug!(%peer, request_id = %tag.0, "a daemon reply matched no waiting request — dropped");
                Vec::new()
            }
            TransportEvent::AmbiguousRole { .. } => {
                tracing::warn!(%peer, "the shell wire called one frame a reply and a stream fragment");
                Vec::new()
            }
            // The stream variants cannot occur (`ShellWire::StreamKey` is uninhabited); a variant the transport adds
            //   later lands here (`#[non_exhaustive]`).
            other => {
                tracing::debug!(%peer, kind = other.kind(), "an unrouted transport event");
                Vec::new()
            }
        }
    }
}

fn for_driver(event: &AgentEvent) -> bool {
    matches!(
        event,
        AgentEvent::SubscribeAck { .. }
            | AgentEvent::ReplayComplete { .. }
            | AgentEvent::SubscribeFailed { .. }
            | AgentEvent::CommandRequest { .. }
            | AgentEvent::CommandReply { .. }
            | AgentEvent::UsageLimitsUpdated { .. }
    )
}

/// Sends the outcome of a daemon's `CommandRequest` back on the connection the request arrived on — `generation` = that
/// request's `Arrival.generation`. ★Never `notify`★: after a reconnect the outcome would reach a connection that never
/// sent the request (TRD §3-0 ③ · ADR-0195). An outcome that cannot go is logged and dropped; the daemon reclaims its
/// slot by its own deadline.
// ADR-0272: the outcome travels in the raw slot.
pub fn send_command_outcome(peer: &Peer<ShellWire>, generation: Generation, reply: &BusReply) {
    let raw = match protocol_state::reply_to_wire(reply) {
        Ok(raw) => raw,
        Err(e) => {
            tracing::warn!(
                request_id = %reply.request_id,
                error = %e,
                "a command outcome did not fit the raw slot — dropped"
            );
            return;
        }
    };
    if let Err(e) = peer.notify_in(generation, AgentCommand::CommandOutcome { reply: raw }) {
        tracing::warn!(
            request_id = %reply.request_id,
            generation = generation.0,
            error = %e,
            "a command outcome was not sent — it goes only to the connection its request came on"
        );
    }
}

/// Hands one frontend event to the ports (T7c — a broadcast without `request_id` goes to every webview).
///
/// ★Which variant reaches the frontend is decided here, not in the port★ — `events.rs` knows only what happened and
/// which Tauri event name carries it. The `Error` arm logs instead of emitting, and moving it into the adapter would
/// lose that trace in the no-op assembly. Variants without a frontend effect are ignored.
/// ★Names after the frontend event, with no lock held★ (TRD S26-window-chrome §4-3) — the real `AgentNamesSink` takes
/// the layout lock, and this function takes no state.
pub(crate) fn emit_broadcast(
    events: &dyn DaemonEvents,
    names: &dyn AgentNamesSink,
    ev: &AgentEvent,
) {
    match ev {
        AgentEvent::AgentListUpdated { agents } => {
            events.agent_list_updated(agents);
            names.agents(agents);
        }
        AgentEvent::StatusChanged {
            agent_id,
            status,
            epoch,
        } => {
            events.status_changed(agent_id, status, *epoch);
        }
        AgentEvent::RestoreResult { report } => {
            events.restore_result(report);
        }
        AgentEvent::ProfileListUpdated { profiles } => {
            events.profile_list_updated(profiles);
        }
        // ADR-0061: every webview mirrors the preset list, so the broadcast is wired through even before a frontend
        //   listener consumes it.
        AgentEvent::PresetListUpdated { presets } => {
            events.preset_list_updated(presets);
        }
        // A failure outside every round trip — a reply-tagged `Error` never gets here (the reply path takes it first),
        //   so `request_id` is always `None`. Not emitted: no field says which view it belongs to.
        AgentEvent::Error {
            request_id: None,
            message,
        } => {
            tracing::warn!(%message, "데몬 error 이벤트(명령 왕복 밖)");
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use engram_dashboard_protocol::{
        AgentBackendKind, AgentId, AgentInfo, AgentProfile, AgentStatus, Preset, RequestId,
        RestoreReport, SubscribeAction, UsageLimitSnapshot, UsageVendorState,
    };
    use engram_dashboard_transport::testing::{
        settle, settle_until, ImmediateHandshake, ManualClock, MemoryNetwork,
    };
    use engram_dashboard_transport::{
        Address, Direction, DisconnectCause, Frame, LinkError, PeerId, PeerState, Policy, Registry,
        RejectKind,
    };
    use serde_json::value::RawValue;

    use super::super::test_log::capture;
    use crate::daemon_client::events::ConnectionStateEvent;
    use crate::daemon_client::replay_flight::ReplayFlightSet;
    use crate::daemon_client::tests::wire_agent;

    // ── item builders ──────────────────────────────────────────────────────────────────────────

    fn key() -> DaemonKey {
        DaemonKey::new("daemon")
    }

    fn peer_id() -> PeerId {
        key().peer_id().clone()
    }

    fn connected(generation: u64) -> Incoming<ShellWire> {
        Incoming::Event(TransportEvent::Connected {
            peer: peer_id(),
            generation: Generation(generation),
        })
    }

    fn disconnected(generation: u64, cause: DisconnectCause) -> Incoming<ShellWire> {
        Incoming::Event(TransportEvent::Disconnected {
            peer: peer_id(),
            generation: Generation(generation),
            cause,
        })
    }

    fn closed(generation: u64) -> Incoming<ShellWire> {
        Incoming::Event(TransportEvent::Closed {
            peer: peer_id(),
            generation: Generation(generation),
            cause: DisconnectCause::Explicit,
        })
    }

    fn connect_failed(cause: ConnectFailure) -> Incoming<ShellWire> {
        Incoming::Event(TransportEvent::ConnectFailed {
            peer: peer_id(),
            attempt: 1,
            of: 5,
            cause,
            retry_in: Some(Duration::from_millis(500)),
        })
    }

    fn unreachable() -> ConnectFailure {
        ConnectFailure::Unreachable(LinkError::new("connection refused"))
    }

    fn retryable_refusal() -> ConnectFailure {
        ConnectFailure::Rejected {
            kind: RejectKind::Retryable,
            code: CloseCode::HANDSHAKE_RETRYABLE,
            reason: "bad token".into(),
        }
    }

    fn rejected(reason: &str) -> Incoming<ShellWire> {
        Incoming::Event(TransportEvent::Rejected {
            peer: peer_id(),
            code: CloseCode::HANDSHAKE_REJECTED,
            reason: reason.into(),
        })
    }

    fn gave_up(reason: GaveUpReason) -> Incoming<ShellWire> {
        Incoming::Event(TransportEvent::GaveUp {
            peer: peer_id(),
            reason,
        })
    }

    fn message(generation: u64, msg: ShellIn) -> Incoming<ShellWire> {
        Incoming::Message(Arrival {
            peer: peer_id(),
            generation: Generation(generation),
            msg,
        })
    }

    fn event(generation: u64, ev: AgentEvent) -> Incoming<ShellWire> {
        message(generation, ShellIn::Event(ev))
    }

    fn list(agents: Vec<AgentInfo>) -> AgentEvent {
        AgentEvent::AgentListUpdated { agents }
    }

    fn raw(json: &str) -> Box<RawValue> {
        RawValue::from_string(json.to_owned()).unwrap()
    }

    fn command_request() -> AgentEvent {
        AgentEvent::CommandRequest {
            envelope: raw(&format!(
                r#"{{"name":"x","request_id":"{}","owner":"o","proto_ver":1,"args":null}}"#,
                uuid::Uuid::new_v4()
            )),
        }
    }

    fn states(out: &[Routed]) -> Vec<ConnectionState> {
        out.iter()
            .filter_map(|r| match r {
                Routed::State(s) => Some(*s),
                _ => None,
            })
            .collect()
    }

    fn route_state() -> RouteState {
        RouteState::new(ManualClock::new().handle())
    }

    fn adopted() -> (RouteState, HandleToken) {
        let mut st = route_state();
        let token = HandleToken::mint();
        assert!(
            st.adopt(key(), token).is_empty(),
            "a first handle ends nothing"
        );
        (st, token)
    }

    /// Routes `items` of one handle and returns everything routed, in order.
    fn run(
        st: &mut RouteState,
        token: HandleToken,
        items: impl IntoIterator<Item = Incoming<ShellWire>>,
    ) -> Vec<Routed> {
        items
            .into_iter()
            .flat_map(|item| route(st, &key(), token, item))
            .collect()
    }

    // ── handle token ───────────────────────────────────────────────────────────────────────────

    #[test]
    fn every_mint_is_a_different_handle() {
        let a = HandleToken::mint();
        let b = HandleToken::mint();
        assert_ne!(a, b);
        assert_eq!(a, a);
    }

    // ── the three filter rules ─────────────────────────────────────────────────────────────────

    #[test]
    fn rule_1_drops_every_item_of_a_handle_that_is_not_the_keys_current_one() {
        let (mut st, old) = adopted();
        assert!(!run(&mut st, old, [connected(1)]).is_empty());
        // Re-add: the new handle has not connected yet, so only the handle tells the old tail apart.
        let new = HandleToken::mint();
        let ended = st.adopt(key(), new);
        assert!(
            matches!(
                ended.as_slice(),
                [
                    Routed::LinkDown { generation },
                    Routed::State(ConnectionState::Reconnecting)
                ] if generation.0 == 1
            ),
            "the re-add ends the old connection: {ended:?}"
        );
        let tail = run(
            &mut st,
            old,
            [
                event(1, list(vec![])),
                disconnected(1, DisconnectCause::PeerClosed),
                connect_failed(unreachable()),
                gave_up(GaveUpReason::BudgetExhausted),
                closed(1),
            ],
        );
        assert!(tail.is_empty(), "the old tail must be dropped: {tail:?}");
        // A key nobody adopted routes nothing either.
        let other = DaemonKey::new("other");
        assert!(route(&mut st, &other, new, connected(1)).is_empty());
    }

    #[test]
    fn rule_2_drops_an_older_connection_and_keeps_the_read_ahead_before_disconnected() {
        let (mut st, token) = adopted();
        run(&mut st, token, [connected(2)]);
        // The read-ahead the transport queues just before `Disconnected` — routed when the transport may already be
        //   on connection 3 (a lagging inbox); only what this inbox delivered counts.
        let out = run(
            &mut st,
            token,
            [
                event(2, list(vec![])),
                disconnected(2, DisconnectCause::PeerClosed),
            ],
        );
        assert!(
            matches!(out.first(), Some(Routed::Broadcast(_))),
            "in-order messages pass: {out:?}"
        );
        run(&mut st, token, [connected(3)]);
        let old = run(
            &mut st,
            token,
            [
                event(2, list(vec![])),
                message(2, ShellIn::Output(vec![1])),
                disconnected(2, DisconnectCause::PeerClosed),
                closed(2),
            ],
        );
        assert!(old.is_empty(), "connection 2 is over: {old:?}");
    }

    #[test]
    fn rule_3_lets_the_lifecycle_of_a_handle_that_never_connected_pass() {
        let (mut st, token) = adopted();
        let out = run(
            &mut st,
            token,
            [
                connect_failed(unreachable()),
                rejected("from the daemon"),
                gave_up(GaveUpReason::Rejected),
            ],
        );
        assert!(
            matches!(
                out.as_slice(),
                [
                    Routed::State(ConnectionState::Down),
                    Routed::Notice(LinkNotice::Refused {
                        code: CloseCode::HANDSHAKE_REJECTED
                    }),
                ]
            ),
            "{out:?}"
        );

        // `Closed` of a handle that never connected carries the registry's floor — the previous handle's last
        //   generation — and still passes.
        let (mut st, first) = adopted();
        run(&mut st, first, [connected(7)]);
        let second = HandleToken::mint();
        let _ = st.adopt(key(), second);
        let out = run(&mut st, second, [closed(7)]);
        assert_eq!(states(&out), [ConnectionState::Down]);
    }

    // ── lifecycle mapping ──────────────────────────────────────────────────────────────────────

    #[test]
    fn states_are_emitted_on_edges_only() {
        let (mut st, token) = adopted();
        let out = run(
            &mut st,
            token,
            [
                connected(1),
                disconnected(1, DisconnectCause::Idle),
                connect_failed(unreachable()),
                connect_failed(unreachable()),
                connected(2),
                disconnected(2, DisconnectCause::ReadFailed),
                connect_failed(retryable_refusal()),
                connect_failed(unreachable()),
                gave_up(GaveUpReason::BudgetExhausted),
                closed(2),
            ],
        );
        use ConnectionState::*;
        assert_eq!(
            states(&out),
            [Connected, Reconnecting, Connected, Reconnecting, Down]
        );
        let links: Vec<String> = out
            .iter()
            .filter_map(|r| match r {
                Routed::LinkUp { generation } => Some(format!("up{}", generation.0)),
                Routed::LinkDown { generation } => Some(format!("down{}", generation.0)),
                _ => None,
            })
            .collect();
        assert_eq!(links, ["up1", "down1", "up2", "down2"]);
    }

    #[test]
    fn a_handle_that_never_connected_shows_its_first_failure_as_down_and_a_later_connect_as_connected(
    ) {
        use ConnectionState::*;
        let (mut st, token) = adopted();
        let out = run(
            &mut st,
            token,
            [
                connect_failed(unreachable()),
                connect_failed(unreachable()),
                connect_failed(retryable_refusal()),
                connected(1),
                disconnected(1, DisconnectCause::PeerClosed),
                connect_failed(unreachable()),
            ],
        );
        assert_eq!(
            states(&out),
            [Down, Connected, Reconnecting],
            "no Reconnecting before the first Connected, no Down after it"
        );
    }

    #[test]
    fn a_re_add_of_a_connected_key_shows_the_new_handles_first_failure_as_down() {
        use ConnectionState::*;
        let (mut st, old) = adopted();
        assert_eq!(states(&run(&mut st, old, [connected(1)])), [Connected]);

        let new = HandleToken::mint();
        assert_eq!(states(&st.adopt(key(), new)), [Reconnecting]);
        let out = run(
            &mut st,
            new,
            [
                connect_failed(unreachable()),
                connect_failed(unreachable()),
                connected(2),
            ],
        );
        assert_eq!(
            states(&out),
            [Down, Connected],
            "an explicit connect that fails shows Down, as a first connect does"
        );
    }

    #[test]
    fn repeated_failures_get_one_line_per_cause_change_and_a_summary_at_most_every_minute() {
        let mut log = FailureLog::default();
        let t0 = Instant::now();
        let at = |secs| t0 + Duration::from_secs(secs);
        let unreachable = FailureKind::Unreachable(UnreachableClass::NoRecord);
        let refused = FailureKind::Refused {
            code: CloseCode::HANDSHAKE_RETRYABLE,
        };
        assert_eq!(
            log.record(unreachable, at(0)),
            FailureLine::New { unlogged: 0 }
        );
        for secs in [10, 20, 59] {
            assert_eq!(log.record(unreachable, at(secs)), FailureLine::Quiet);
        }
        assert_eq!(
            log.record(unreachable, at(60)),
            FailureLine::Repeated { count: 4 },
            "a minute after the first line, counting every failure since it"
        );
        assert_eq!(log.record(unreachable, at(70)), FailureLine::Quiet);
        assert_eq!(
            log.record(unreachable, at(119)),
            FailureLine::Quiet,
            "the minute runs from the summary line"
        );
        assert_eq!(
            log.record(refused, at(121)),
            FailureLine::New { unlogged: 2 },
            "a new cause logs at once and carries what no line counted"
        );
        assert_eq!(
            log.record(unreachable, at(122)),
            FailureLine::New { unlogged: 0 }
        );
        assert_eq!(log.record(unreachable, at(123)), FailureLine::Quiet);
        assert_eq!(log.reset(), 1);
        assert_eq!(
            log.record(unreachable, at(124)),
            FailureLine::New { unlogged: 0 },
            "after a connect the next failure is a first one again"
        );
    }

    fn unreachable_with(text: &str) -> Incoming<ShellWire> {
        connect_failed(ConnectFailure::Unreachable(LinkError::new(text)))
    }

    #[test]
    fn a_change_in_why_the_daemon_is_unreachable_is_logged_at_once() {
        let (mut st, token) = adopted();
        let lines = Arc::new(Mutex::new(Vec::new()));
        capture(lines.clone(), || {
            run(
                &mut st,
                token,
                [
                    unreachable_with(NO_RECORD),
                    unreachable_with(&format!(
                        "{UNUSABLE}: daemon.json read failed: access denied"
                    )),
                    // The same class with other detail is the same cause.
                    unreachable_with(&format!(
                        "{UNUSABLE}: daemon.json names pid 7 which is not alive"
                    )),
                    unreachable_with("ws connect ws://127.0.0.1:5123: connection refused"),
                    unreachable_with("ws connect ws://127.0.0.1:6000: connection refused"),
                ],
            )
        });
        let lines = lines.lock().unwrap();
        let failures = failure_lines(&lines);
        assert_eq!(
            levels(&failures),
            ["WARN", "WARN", "DEBUG", "WARN", "DEBUG"],
            "{lines:?}"
        );
        assert!(
            failures[0].contains(" kind=Unreachable(NoRecord) "),
            "{failures:?}"
        );
        assert!(
            failures[1].contains(" kind=Unreachable(RecordUnusable) "),
            "{failures:?}"
        );
        assert!(
            failures[3].contains(" kind=Unreachable(Dial) "),
            "{failures:?}"
        );
    }

    /// The failure log's lines — the events with a `kind` field.
    fn failure_lines(lines: &[String]) -> Vec<&String> {
        lines
            .iter()
            .filter(|line| line.contains(" kind="))
            .collect()
    }

    fn levels<'a>(lines: &[&'a String]) -> Vec<&'a str> {
        lines
            .iter()
            .map(|line| {
                line.strip_prefix("level=")
                    .and_then(|rest| rest.split(' ').next())
                    .unwrap_or("")
            })
            .collect()
    }

    #[test]
    fn a_repeated_cause_is_debug_until_a_minute_passed_and_a_connect_reports_what_no_line_counted()
    {
        let clock = ManualClock::new();
        let mut st = RouteState::new(clock.handle());
        let token = HandleToken::mint();
        let _ = st.adopt(key(), token);
        let lines = Arc::new(Mutex::new(Vec::new()));
        capture(lines.clone(), || {
            run(&mut st, token, [unreachable_with(NO_RECORD)]);
            for _ in 0..3 {
                clock.advance(Duration::from_secs(15));
                run(&mut st, token, [unreachable_with(NO_RECORD)]);
            }
            clock.advance(Duration::from_secs(15));
            run(&mut st, token, [unreachable_with(NO_RECORD)]);
            clock.advance(Duration::from_secs(5));
            run(&mut st, token, [unreachable_with(NO_RECORD)]);
            run(&mut st, token, [connected(1)]);
            run(&mut st, token, [unreachable_with(NO_RECORD)]);
        });
        let lines = lines.lock().unwrap();
        let failures = failure_lines(&lines);
        assert_eq!(
            levels(&failures),
            ["WARN", "DEBUG", "DEBUG", "DEBUG", "WARN", "DEBUG", "WARN"],
            "within a minute the same cause is debug only: {lines:?}"
        );
        assert!(
            failures[4].contains(" count=4 "),
            "the summary a minute after the first line counts every failure since it: {failures:?}"
        );
        let connected_line = lines
            .iter()
            .find(|line| line.contains(" unlogged_failures="))
            .expect("the connect line");
        assert!(
            connected_line.starts_with("level=INFO ")
                && connected_line.contains(" unlogged_failures=1 "),
            "{connected_line}"
        );
        assert!(
            failures[6].contains(" unlogged_before=0 "),
            "after a connect the next failure is a first one again: {failures:?}"
        );
    }

    // `UnreachableClass` reads texts the transport writes — a rewording there must break this, not fall into `Other`.
    #[tokio::test]
    async fn the_transport_texts_the_failure_classes_read_are_the_ones_it_writes() {
        use engram_dashboard_transport::testing::HelloHandshake;
        use engram_dashboard_transport::ws::WsDialer;
        use engram_dashboard_transport::{DialFailure, Dialer};

        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let registry = registry(&net, &clock);
        let (peer, mut inbox) =
            registry.add(peer_id(), Address::new("mem"), Arc::new(HelloHandshake));
        settle_until("a link", || net.open_endpoints() > 0).await;
        // A close without a code during the handshake.
        net.accept().unwrap().close();
        settle_until("backoff", || peer.peer_state() == PeerState::Backoff).await;
        settle().await;
        clock.advance(Duration::from_secs(1));
        settle_until("a second link", || net.open_endpoints() > 0).await;
        // Held and never answered — the attempt runs out of time.
        let _silent = net.accept().unwrap();
        settle().await;
        clock.advance(Duration::from_secs(11));
        settle_until("backoff again", || peer.peer_state() == PeerState::Backoff).await;

        let mut classes = Vec::new();
        while let Some(item) = inbox.try_recv() {
            if let Incoming::Event(TransportEvent::ConnectFailed {
                cause: ConnectFailure::Unreachable(e),
                ..
            }) = item
            {
                classes.push(UnreachableClass::of(&e.message));
            }
        }
        assert_eq!(
            classes,
            [
                UnreachableClass::ClosedInHandshake,
                UnreachableClass::TimedOut
            ]
        );

        let failure = WsDialer::new(1024)
            .dial(&Address::new("not a url"))
            .await
            .err();
        match failure {
            Some(DialFailure::Unreachable(e)) => {
                assert_eq!(
                    UnreachableClass::of(&e.message),
                    UnreachableClass::Dial,
                    "{e}"
                )
            }
            _ => panic!("the ws dialer refuses a bad address as unreachable"),
        }
    }

    #[test]
    fn our_own_disconnect_leaves_connected_and_what_follows_decides_the_rest() {
        use ConnectionState::*;
        // `close`: `Disconnected{Explicit}` then `Closed`.
        let (mut st, token) = adopted();
        let out = run(
            &mut st,
            token,
            [
                connected(1),
                disconnected(1, DisconnectCause::Explicit),
                closed(1),
            ],
        );
        assert_eq!(states(&out), [Connected, Reconnecting, Down]);
        assert!(out
            .iter()
            .any(|r| matches!(r, Routed::LinkDown { generation } if generation.0 == 1)));

        // `reconnect_now` on a live peer: `Disconnected{Explicit}` then a redial — the new `Connected` is an edge.
        let (mut st, token) = adopted();
        let out = run(
            &mut st,
            token,
            [
                connected(1),
                disconnected(1, DisconnectCause::Explicit),
                connected(2),
            ],
        );
        assert_eq!(states(&out), [Connected, Reconnecting, Connected]);
    }

    #[test]
    fn an_inbox_end_without_closed_ends_the_open_connection_and_shows_down() {
        let (mut st, token) = adopted();
        run(&mut st, token, [connected(3)]);
        let out = route_end(&mut st, &key(), token);
        assert!(
            matches!(
                out.as_slice(),
                [
                    Routed::LinkDown { generation },
                    Routed::State(ConnectionState::Down)
                ] if generation.0 == 3
            ),
            "{out:?}"
        );

        // After `Closed` the end is clean.
        let (mut st, token) = adopted();
        run(
            &mut st,
            token,
            [
                connected(1),
                disconnected(1, DisconnectCause::Explicit),
                closed(1),
            ],
        );
        let out = route_end(&mut st, &key(), token);
        assert!(out.is_empty(), "{out:?}");

        // The end of a replaced handle routes nothing.
        let (mut st, old) = adopted();
        run(&mut st, old, [connected(1)]);
        let _ = st.adopt(key(), HandleToken::mint());
        assert!(route_end(&mut st, &key(), old).is_empty());
    }

    #[test]
    fn budget_exhaustion_carries_the_cause_of_the_last_attempt() {
        for (attempts, last) in [
            (
                vec![retryable_refusal(), unreachable()],
                Some(FailureCause::Unreachable),
            ),
            (
                vec![unreachable(), retryable_refusal()],
                Some(FailureCause::Refused {
                    code: CloseCode::HANDSHAKE_RETRYABLE,
                }),
            ),
        ] {
            let (mut st, token) = adopted();
            let mut items: Vec<_> = attempts.into_iter().map(connect_failed).collect();
            items.push(gave_up(GaveUpReason::BudgetExhausted));
            let out = run(&mut st, token, items);
            assert!(
                matches!(
                    out.as_slice(),
                    [
                        Routed::State(ConnectionState::Down),
                        Routed::Notice(LinkNotice::BudgetExhausted { last: got }),
                    ] if *got == last
                ),
                "{out:?}"
            );
        }

        // A `Connected` forgets the failures before it.
        let (mut st, token) = adopted();
        let out = run(
            &mut st,
            token,
            [
                connect_failed(retryable_refusal()),
                connected(1),
                disconnected(1, DisconnectCause::PeerClosed),
                gave_up(GaveUpReason::BudgetExhausted),
            ],
        );
        assert!(
            out.iter().any(|r| matches!(
                r,
                Routed::Notice(LinkNotice::BudgetExhausted { last: None })
            )),
            "{out:?}"
        );
    }

    #[test]
    fn a_version_refusal_after_a_reconnect_reaches_the_fixed_refusal_text() {
        let (mut st, token) = adopted();
        let out = run(
            &mut st,
            token,
            [
                connected(1),
                disconnected(1, DisconnectCause::PeerClosed),
                rejected("protocol mismatch, try version 999"),
                gave_up(GaveUpReason::Rejected),
            ],
        );
        let notices: Vec<LinkNotice> = out
            .iter()
            .filter_map(|r| match r {
                Routed::Notice(n) => Some(*n),
                _ => None,
            })
            .collect();
        assert_eq!(
            notices,
            [LinkNotice::Refused {
                code: CloseCode::HANDSHAKE_REJECTED
            }]
        );
        assert_eq!(
            states(&out),
            [
                ConnectionState::Connected,
                ConnectionState::Reconnecting,
                ConnectionState::Down
            ]
        );
        // The text is the local one for the code — it names the cause and quotes nothing the daemon sent.
        let text = refusal_message(CloseCode::HANDSHAKE_REJECTED);
        assert!(text.contains("버전"), "{text}");
        assert!(!text.contains("999"));
        assert_ne!(text, refusal_message(CloseCode(4100)));
    }

    #[test]
    fn dropped_frames_and_other_transport_reports_route_nothing() {
        let (mut st, token) = adopted();
        run(&mut st, token, [connected(1)]);
        let out = run(
            &mut st,
            token,
            [
                Incoming::Event(TransportEvent::Dropped {
                    peer: peer_id(),
                    direction: Direction::Outbound,
                    count: 3,
                }),
                Incoming::Event(TransportEvent::DecodeFailed {
                    peer: peer_id(),
                    generation: Generation(1),
                    reason: "json syntax",
                }),
                Incoming::Event(TransportEvent::UnmatchedReply {
                    peer: peer_id(),
                    tag: RequestId::new(),
                }),
                Incoming::Event(TransportEvent::RequestTimedOut {
                    peer: peer_id(),
                    tag: RequestId::new(),
                    after: Duration::from_secs(30),
                }),
                Incoming::Event(TransportEvent::AmbiguousRole { peer: peer_id() }),
            ],
        );
        assert!(out.is_empty(), "{out:?}");
    }

    // ── message split ──────────────────────────────────────────────────────────────────────────

    #[test]
    fn messages_split_between_the_frontend_the_driver_and_the_output_relay() {
        let (mut st, token) = adopted();
        run(&mut st, token, [connected(4)]);
        let agent = AgentId::new_v4();
        let for_driver = [
            AgentEvent::SubscribeAck {
                agent_id: agent,
                action: SubscribeAction::Reset,
                current_epoch: 1,
                oldest_seq: 0,
                latest_seq: 0,
                replay_from: 0,
                truncated: false,
                continues_conversation: false,
            },
            AgentEvent::ReplayComplete {
                agent_id: agent,
                epoch: 1,
            },
            AgentEvent::SubscribeFailed {
                agent_id: agent,
                reason: "gone".into(),
            },
            command_request(),
            AgentEvent::CommandReply { reply: raw("{}") },
            AgentEvent::UsageLimitsUpdated {
                snapshot: UsageLimitSnapshot {
                    vendor: AgentBackendKind::Claude,
                    account_key: "a".into(),
                    five_hour: None,
                    weekly: None,
                    model_scoped: vec![],
                    plan: None,
                    in_flight: false,
                    state: UsageVendorState::Ready,
                    revision: 1,
                },
                subscribed: vec![],
            },
        ];
        for ev in for_driver {
            let out = route(&mut st, &key(), token, event(4, ev));
            assert!(
                matches!(out.as_slice(), [Routed::ToDriver { generation, .. }] if generation.0 == 4),
                "{out:?}"
            );
        }
        for ev in [
            list(vec![]),
            AgentEvent::StatusChanged {
                agent_id: agent,
                status: AgentStatus::Running,
                epoch: 1,
            },
            AgentEvent::ProfileListUpdated { profiles: vec![] },
            AgentEvent::Error {
                request_id: None,
                message: "x".into(),
            },
        ] {
            let out = route(&mut st, &key(), token, event(4, ev));
            assert!(matches!(out.as_slice(), [Routed::Broadcast(_)]), "{out:?}");
        }
        let out = route(
            &mut st,
            &key(),
            token,
            message(4, ShellIn::Output(vec![7, 8])),
        );
        assert!(matches!(
            out.as_slice(),
            [Routed::Output { generation, bytes }] if generation.0 == 4 && bytes == &vec![7, 8]
        ));
    }

    // ── frontend ports ─────────────────────────────────────────────────────────────────────────

    /// Records both ports in one log behind one lock.
    #[derive(Default)]
    struct Recorder {
        log: Mutex<Vec<String>>,
    }

    impl Recorder {
        fn push(&self, line: String) {
            self.log.lock().unwrap().push(line);
        }

        fn lines(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    fn names_of(agents: &[AgentInfo]) -> String {
        agents
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }

    impl DaemonEvents for Recorder {
        fn connection_state(&self, _state: ConnectionStateEvent) {
            self.push("state".into());
        }
        fn agent_list_updated(&self, agents: &[AgentInfo]) {
            self.push(format!("frontend:{}", names_of(agents)));
        }
        fn status_changed(&self, _agent_id: &AgentId, _status: &AgentStatus, _epoch: u32) {
            self.push("status".into());
        }
        fn restore_result(&self, _report: &RestoreReport) {
            self.push("restore".into());
        }
        fn profile_list_updated(&self, _profiles: &[AgentProfile]) {
            self.push("profiles".into());
        }
        fn preset_list_updated(&self, _presets: &[Preset]) {
            self.push("presets".into());
        }
        fn usage_limits_updated(
            &self,
            _snapshot: &UsageLimitSnapshot,
            _labels: &[String],
            _socket_epoch: u64,
        ) {
            self.push("usage".into());
        }
    }

    impl AgentNamesSink for Recorder {
        fn agents(&self, agents: &[AgentInfo]) {
            // Re-takes the lock the frontend half used — `try_lock` fails if the caller still held it.
            let mut log = self
                .log
                .try_lock()
                .expect("no lock is held while the names sink runs");
            log.push(format!("names:{}", names_of(agents)));
        }
    }

    #[test]
    fn the_names_sink_runs_after_the_frontend_event_with_no_lock_held() {
        let rec = Recorder::default();
        let agent = wire_agent(AgentId::new_v4(), "alpha");
        emit_broadcast(&rec, &rec, &list(vec![agent]));
        assert_eq!(rec.lines(), ["frontend:alpha", "names:alpha"]);
    }

    /// The driver part this mapping test needs: a dropped link clears the replay flights, a broadcast reaches the
    /// ports (the old driver does the same per event).
    fn apply(out: Vec<Routed>, flight: &mut ReplayFlightSet, rec: &Recorder) {
        for routed in out {
            match routed {
                Routed::LinkDown { .. } => flight.on_disconnect(),
                Routed::Broadcast(ev) => emit_broadcast(rec, rec, &ev),
                _ => {}
            }
        }
    }

    #[test]
    fn a_re_add_drops_the_old_tail_after_the_new_snapshot_and_keeps_the_new_flight() {
        let mut st = route_state();
        let mut flight = ReplayFlightSet::new(Duration::from_secs(10));
        let rec = Recorder::default();
        let agent = AgentId::new_v4();
        let now = Instant::now();

        let old = HandleToken::mint();
        apply(st.adopt(key(), old), &mut flight, &rec);
        apply(
            run(
                &mut st,
                old,
                [connected(1), event(1, list(vec![wire_agent(agent, "old")]))],
            ),
            &mut flight,
            &rec,
        );

        // Re-add: the new handle connects and delivers its snapshot, then a replay starts on it.
        let new = HandleToken::mint();
        apply(st.adopt(key(), new), &mut flight, &rec);
        apply(
            run(
                &mut st,
                new,
                [connected(2), event(2, list(vec![wire_agent(agent, "new")]))],
            ),
            &mut flight,
            &rec,
        );
        assert!(
            flight.request_replay(agent, now).send_now,
            "a fresh flight starts"
        );

        // The old supervisor's tail is routed only now.
        apply(
            run(
                &mut st,
                old,
                [
                    event(1, list(vec![wire_agent(agent, "old")])),
                    disconnected(1, DisconnectCause::PeerClosed),
                    closed(1),
                ],
            ),
            &mut flight,
            &rec,
        );

        assert!(
            !flight.request_replay(agent, now).send_now,
            "the new flight is still in flight — the old tail did not clear it"
        );
        assert_eq!(
            rec.lines(),
            ["frontend:old", "names:old", "frontend:new", "names:new"],
            "the stale list overwrote neither the frontend nor the names"
        );
    }

    #[test]
    fn a_re_add_releases_the_replay_flight_of_the_old_connection() {
        let mut st = route_state();
        let mut flight = ReplayFlightSet::new(Duration::from_secs(10));
        let rec = Recorder::default();
        let agent = AgentId::new_v4();
        let now = Instant::now();

        let old = HandleToken::mint();
        apply(st.adopt(key(), old), &mut flight, &rec);
        apply(run(&mut st, old, [connected(1)]), &mut flight, &rec);
        assert!(
            flight.request_replay(agent, now).send_now,
            "a replay goes out on connection 1"
        );

        // Re-add before that replay resolved — the old handle's own `Disconnected` will be dropped as foreign.
        let new = HandleToken::mint();
        let ended = st.adopt(key(), new);
        assert_eq!(states(&ended), [ConnectionState::Reconnecting]);
        apply(ended, &mut flight, &rec);
        let up = run(&mut st, new, [connected(2)]);
        assert_eq!(
            states(&up),
            [ConnectionState::Connected],
            "an edge to resync on"
        );
        apply(up, &mut flight, &rec);
        assert!(
            flight.request_replay(agent, now).send_now,
            "the replay of connection 1 no longer holds the slot — a new one goes out"
        );
    }

    // ── on a registry ──────────────────────────────────────────────────────────────────────────

    fn text(ev: &AgentEvent) -> Frame {
        Frame::Text(serde_json::to_string(ev).unwrap())
    }

    fn registry(net: &MemoryNetwork, clock: &ManualClock) -> Registry<ShellWire> {
        Registry::new(
            Arc::new(ShellWire),
            net.dialer(),
            clock.handle(),
            Policy::default(),
        )
    }

    #[tokio::test]
    async fn a_command_outcome_goes_only_to_the_connection_its_request_came_on() {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let registry = registry(&net, &clock);
        let (peer, mut inbox) =
            registry.add(peer_id(), Address::new("mem"), Arc::new(ImmediateHandshake));
        settle_until("live", || peer.peer_state() == PeerState::Live).await;
        let first = net.accept().expect("a link was opened");

        first.push(text(&command_request()));
        settle().await;
        let mut request_generation = None;
        while let Some(item) = inbox.try_recv() {
            if let Incoming::Message(arrival) = item {
                request_generation = Some(arrival.generation);
            }
        }
        let request_generation = request_generation.expect("the daemon request arrived");

        // The connection drops and the peer reconnects before the outcome is ready.
        first.close();
        settle_until("dropped", || peer.peer_state() != PeerState::Live).await;
        for _ in 0..40 {
            if peer.peer_state() == PeerState::Live {
                break;
            }
            clock.advance(Duration::from_millis(250));
            settle().await;
        }
        assert_eq!(peer.peer_state(), PeerState::Live, "reconnected");
        let second = net.accept().expect("a second link was opened");
        assert_ne!(peer.generation(), request_generation);

        let outcome = BusReply::ok(
            engram_dashboard_command::RequestId::new(),
            serde_json::Value::Null,
        );
        send_command_outcome(&peer, request_generation, &outcome);
        settle().await;
        assert_eq!(
            second.frames(),
            Vec::<Frame>::new(),
            "an outcome of the old connection reached the new one"
        );

        // Control: the same call bound to the live connection does go out.
        send_command_outcome(&peer, peer.generation(), &outcome);
        settle().await;
        let frames = second.frames();
        assert!(
            matches!(frames.as_slice(), [Frame::Text(t)] if t.starts_with(r#"{"CommandOutcome""#)),
            "{frames:?}"
        );
    }

    // §3-3 B2 · R10: a reply skips the inbox, so it can resolve before an event the daemon sent ahead of it.
    #[tokio::test]
    async fn a_reply_wakes_its_request_while_an_event_sent_before_it_is_still_unread() {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let registry = registry(&net, &clock);
        let (peer, mut inbox) =
            registry.add(peer_id(), Address::new("mem"), Arc::new(ImmediateHandshake));
        settle_until("live", || peer.peer_state() == PeerState::Live).await;
        let endpoint = net.accept().expect("a link was opened");
        while inbox.try_recv().is_some() {}

        let rid = RequestId::new();
        let task = {
            let peer = peer.clone();
            tokio::spawn(async move {
                peer.request(AgentCommand::ListAgents { request_id: rid })
                    .await
            })
        };
        settle().await;
        endpoint.push(text(&list(vec![])));
        endpoint.push(text(&AgentEvent::Ack { request_id: rid }));
        // Nobody reads the inbox — the reply still resolves.
        settle_until("the reply wakes the request", || task.is_finished()).await;
        assert!(matches!(
            task.await.unwrap(),
            Ok(ShellIn::Event(AgentEvent::Ack { request_id })) if request_id == rid
        ));

        let first = inbox.try_recv().expect("the earlier event is still unread");
        assert!(matches!(
            first,
            Incoming::Message(Arrival {
                msg: ShellIn::Event(AgentEvent::AgentListUpdated { .. }),
                ..
            })
        ));
    }
}
