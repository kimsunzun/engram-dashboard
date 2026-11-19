//! Transport-neutral connection core (ConnectionCore) — ADR-0020 Stage 1.
//!
//! Input is `AgentCommand`; output leaves only as `Outbound` through an `OutboundSink`. The module knows nothing of
//! TcpStream/tungstenite/the frame codec; an adapter (`agent_conn::FrameOutboundSink`) implements the sink.
//!
//! ★Invariants R1~R7 (ADR-0020) must hold★: R1 Ack→replay→ReplayComplete FIFO (kept by `handle_subscribe`); R6 close on
//! saturation is carrier-specific, so the sink implementation interprets `SinkError`.
//!
//! The status fanout (lease/profile broadcast) goes through `crate::conn_port::FrameFanout`, not the per-conn reply (ADR-0129).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use engram_dashboard_agent::commands::{
    create_and_start, created_but_not_started, AgentBackend, CreateRefused, NewAgent,
};
use engram_dashboard_agent::launch::{notice_row, Notice};
use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::manager::RenameOutcome as CoreRenameOutcome;
use engram_dashboard_agent::manager::{Activated, NoticeDelivery};
use engram_dashboard_agent::profile::RestoreReport as CoreRestoreReport;
use engram_dashboard_agent::profile::SpawnMode;
use engram_dashboard_agent::queued_input::{ListedRow, ListedState, QueuedListing};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo as CoreAgentInfo, AgentStatus as CoreStatus, CancelError, InputOrigin,
    OutputSink, PtyError, ReplayKind, SinkId, SubscribeReply,
};
// Production `SpawnByCwd` has no default backend (an empty field is refused with `MISSING_BACKEND`); only test fixtures spawn a shell.
#[cfg(test)]
use engram_dashboard_platform::shell::default_shell;

use engram_dashboard_agent::backend::usage_probe_for;
use engram_dashboard_agent::commands::llm_creation_refusal;
use engram_dashboard_agent::commands::{store_command_error, store_error_reason};
use engram_dashboard_agent::failure::AgentFailureKind as CoreFailureKind;
use engram_dashboard_agent::persistence::PROFILES_FILE;
use engram_dashboard_agent::profile::StoreError;
use engram_dashboard_agent::profile::{
    AgentCommand as CoreSpawnCommand, AgentOutputFormat as CoreAgentOutputFormat,
    AgentProfile as CoreProfile, RestartPolicy as CoreRestartPolicy,
    RestoreOutcome as CoreRestoreOutcome,
};
use engram_dashboard_agent::types::{
    Capabilities as CoreCaps, DeliveredCopy as CoreDeliveredCopy, DropCause as CoreDropCause,
    OutputChunk as CoreOutputChunk, OutputEvent as CoreOutputEvent,
    QueuedInputEvent as CoreQueuedInputEvent, ToolCategory as CoreToolCategory,
    ToolOutcome as CoreToolOutcome, TurnOutcome as CoreTurnOutcome,
};
use engram_dashboard_agent::usage::UsageVendorKey;

use engram_dashboard_protocol::{
    AgentBackendKind as WireBackendKind, AgentCommand, AgentEvent,
    AgentFailureKind as WireFailureKind, AgentInfo as WireAgentInfo,
    AgentOutputFormat as WireAgentOutputFormat, AgentProfile as WireProfile,
    AgentSpawnCommand as WireSpawnCommand, Capabilities as WireCaps,
    ControlCaps as WireControlCaps, DeliveredCopy as WireDeliveredCopy, DropCause as WireDropCause,
    EnvelopeFormat as WireEnvelopeFormat, InputCaps as WireInputCaps, ModelCaps as WireModelCaps,
    OutputCaps as WireOutputCaps, OwnerToken as WireOwnerToken,
    QueuedInputCancel as WireQueuedInputCancel, QueuedInputEvent as WireQueuedInputEvent,
    QueuedInputRow as WireQueuedInputRow, RestartPolicy as WireRestartPolicy,
    RestoreOutcome as WireRestoreOutcome, RestoreReport, SessionCaps as WireSessionCaps,
    SnapshotChunk as WireSnapshotChunk, StructuredEvent as WireStructuredEvent, SubscribeAction,
    ToolCategory as WireToolCategory, ToolOutcome as WireToolOutcome,
    TurnOutcome as WireTurnOutcome, PROTOCOL_VERSION,
};

use tokio::sync::watch;

use crate::command_delivery::{CommandDeliveries, LocalCommands, OutcomeLanding};
use crate::command_roster::CommandRoster;
use crate::conn_port::{ConnId, FrameFanout};
use crate::control::registry::ControlRegistry;
use crate::usage_service::book::RequestKind;
use crate::usage_service::UsageService;
use engram_dashboard_command::{
    CommandDecl, CommandEnvelope, CommandError, CommandReply, ErrorCode, OwnerToken,
};
use engram_dashboard_messaging::envelope::EnvelopeFormat as CoreEnvelopeFormat;
use serde_json::value::RawValue;

// ── OutboundSink seam (follows the grain of the ADR-0003 OutputSink) ──────────────────────────────

/// The carrier-neutral unit of sending (Text/Binary/Close); control is still an unencoded `AgentEvent` — the adapter encodes.
///
/// `Box<AgentEvent>` avoids clippy `large_enum_variant` (~272B vs 24B); the control path is not hot. Build via `Outbound::event()`.
#[derive(Debug)]
pub enum Outbound {
    Event(Box<AgentEvent>),
    Binary(Vec<u8>),
    Close(String),
}

impl Outbound {
    pub fn event(ev: AgentEvent) -> Self {
        Outbound::Event(Box::new(ev))
    }
}

/// Sink enqueue failure (queue full / closed). The adapter interprets it per carrier (WS = close the connection, R6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SinkError;

impl std::fmt::Display for SinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "outbound sink enqueue failed")
    }
}

impl std::error::Error for SinkError {}

pub trait OutboundSink: Send + Sync {
    fn enqueue(&self, out: Outbound) -> Result<(), SinkError>;

    /// Builds the output sink + replay drop flag to hand to the core `subscribe_from`.
    ///
    /// Encoding differs per carrier (WS = binary frame, embedded = base64 PtyEvent). The returned `AtomicBool` says whether a
    /// frame drop (try_send full) happened during the replay phase.
    fn make_output_sink(&self) -> (Arc<dyn OutputSink>, Arc<AtomicBool>);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchFlow {
    Continue,
    /// Close the connection — produced only by `StopDaemon`.
    Close,
}

// ── what stays in the arrival-order queue and what is detached (head-of-line blocking) ──────────────────────────

/// May one command be detached from the connection's arrival-order lane and run as a separate task?
///
/// The network layer lifts frames in arrival order, one consumer per connection (`conn_port::ConnectionHandler` contract), which
/// keeps the ordering invariants for free but lets a slow command delay the next. Only commands whose order has no meaning leave.
/// ★Default is [`DispatchOrder::InOrder`] — if in doubt, keep it in the lane★: wrongly detaching is a silent ordering defect, wrongly keeping is merely slow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DispatchOrder {
    InOrder,
    /// Detached and run as a separate task — a later command can finish first.
    Detached,
}

/// A second classification used only when the inbound lane is saturated — may that frame skip the lane?
///
/// ★Not the same axis as [`DispatchOrder`] — never merge them★ ("does it take long" vs "is it meant to unblock a blocked lane").
/// ★When not saturated it is never called★, so `Kill` never overtakes an earlier command in normal times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InboundLane {
    /// Stands in the arrival-order lane; refused when the lane is full.
    Ordered,
    /// Skips the lane — runs before and overlapping with what is already queued.
    Bypass,
}

/// Wording of the inbound-lane saturation refusal — a spoken refusal, not a silent loss, so the receiver can retry or give up
/// (same disposition as the codex transport's `INPUT_QUEUE_LIMIT`/ADR-0190).
pub(crate) const SATURATED_REFUSAL: &str =
    "daemon inbound queue for this connection is saturated; this command was refused, not applied — \
     retry it, or send Kill if an agent is wedged";

/// The [`InboundLane`] decision. ★Only `Kill` skips — before growing the list, pass both checks below★:
/// 1. Is it in no ordering group? `Kill` is in none of the five; it may overtake an earlier `WriteStdin`(X), and the lost bytes are what killing means.
/// 2. Does it run on a different path from what blocks the lane? In-order lane blockers today: an earlier `Kill` awaiting
///    `AgentManager::kill_agent` on the blocking pool (shutdown + pump join, 5 s ceiling) and the synchronous file IO of profile CRUD.
///    A bypassing `Kill` goes down to `child.kill()` + the Job terminate (`PtyTransport::shutdown`). A second `Kill` of an agent whose
///    kill is still running waits on that transport's master lock, so it does not end the first kill.
///
/// ★`Interrupt` is deliberately not here★: it falls in ordering group ① (`WriteStdin`(X) ↔ `Interrupt`(X)), and `interrupt()` returns
/// immediately on every channel (PTY refuses with `Unsupported` — ADR-0245; claude JSON queues behind pending input — ADR-0238;
/// codex uses the control queue), so skipping gains nothing. It is refused on saturation with [`SATURATED_REFUSAL`]; the user's only
/// handle under pressure is `Kill`. `StopDaemon` is not here either (ordering group ⑤).
// ADR-0206
pub(crate) fn inbound_lane(cmd: &AgentCommand) -> InboundLane {
    match cmd {
        AgentCommand::Kill { .. } => InboundLane::Bypass,
        // ★`_ =>` is allowed only here★ (unlike [`dispatch_order`]): the omission leans to "stand in the lane" = order preserved = the safe
        // side, so a new variant silently taking this default is the desired behavior.
        _ => InboundLane::Ordered,
    }
}

/// The `request_id` to carry in the `Error` when refusing a received command without a reply (today: inbound lane saturation,
/// `on_inbound_saturated` in `agent_conn`).
///
/// Branches match the shell's pairing (`daemon_client::protocol_state::command_request_id`): the refusal wakes the sender's wait slot
/// only with that slot's key. `None` = the sender sets no wait slot (`Error{request_id: None}`); a broken envelope is also `None`
/// (ADR-0272 decision 6). ★No `_ =>`★: a silent `None` for a new variant would hang the sender until the deadline.
// ADR-0272
pub(crate) fn command_request_id(
    cmd: &AgentCommand,
) -> Option<engram_dashboard_protocol::RequestId> {
    match cmd {
        AgentCommand::Spawn { request_id, .. }
        | AgentCommand::Kill { request_id, .. }
        | AgentCommand::Interrupt { request_id, .. }
        | AgentCommand::WriteStdin { request_id, .. }
        | AgentCommand::AcquireInput { request_id, .. }
        | AgentCommand::ReleaseInput { request_id, .. }
        | AgentCommand::ListAgents { request_id }
        | AgentCommand::StopDaemon { request_id, .. }
        | AgentCommand::SpawnByCwd { request_id, .. }
        | AgentCommand::ListProfiles { request_id }
        | AgentCommand::CreateProfile { request_id, .. }
        | AgentCommand::DeleteProfile { request_id, .. }
        | AgentCommand::SpawnProfile { request_id, .. }
        | AgentCommand::SetProfileAutoRestore { request_id, .. }
        | AgentCommand::RenameProfile { request_id, .. }
        | AgentCommand::ReparentProfile { request_id, .. }
        | AgentCommand::GetSnapshot { request_id, .. }
        | AgentCommand::SetEnvelopeFormat { request_id, .. }
        | AgentCommand::RegisterCommands { request_id, .. }
        | AgentCommand::UpdateCommands { request_id, .. }
        | AgentCommand::ListCommands { request_id }
        | AgentCommand::ListQueuedInputs { request_id, .. }
        | AgentCommand::CancelQueuedInput { request_id, .. }
        | AgentCommand::RefreshUsageLimits { request_id, .. } => Some(*request_id),
        // The correlation key is inside the envelope (ADR-0272 decision 4).
        AgentCommand::Command { envelope } => envelope_from_wire(envelope)
            .ok()
            .map(|envelope| engram_dashboard_protocol::RequestId(envelope.request_id.0)),
        AgentCommand::Resize { .. }
        | AgentCommand::Subscribe { .. }
        | AgentCommand::Unsubscribe { .. }
        | AgentCommand::UsageSubscribe { .. }
        // The sender's reply: the key inside belongs to a request this daemon issued, so the sender has no wait slot.
        | AgentCommand::CommandOutcome { .. } => None,
    }
}

/// The [`DispatchOrder`] decision.
///
/// Only the activation three and ⟳ (`RefreshUsageLimits`) are detached: they wait inside dispatch for an outcome that may take
/// seconds (activation: typically 2 s, backstop 15 s, ~20 s with teardown on a failure verdict; ⟳: until the query ends, cap
/// `usage_service::REPLY_WAIT_MAX`). Detaching opens no new window — duplicate activation is blocked by the manager's reservation
/// (`SpawnReservation`) and `activation_in_flight`, which do not know the connection (residual window = ADR-0082).
/// ★Order among these and later commands is not guaranteed★ (`SpawnProfile(X)` then `WriteStdin(X)` writes to an agent not
/// yet up). The sender waits for `Spawned`/`Error` (both carry `request_id`); that contract lives in the activation-set doc of
/// `engram_dashboard_protocol::AgentCommand`, and our frontend already waits through the `request_id` table (`sendCommand` in
/// `src/api/protocolClient.ts`).
/// ★No `_ =>`★: a new variant must be a compile error asking "must this stay in the lane".
// ADR-0206
pub(crate) fn dispatch_order(cmd: &AgentCommand) -> DispatchOrder {
    match cmd {
        // ── outside the lane: the three where activation waits for the outcome ──────────────────────────────────
        AgentCommand::Spawn { .. }
        | AgentCommand::SpawnProfile { .. }
        // `SpawnByCwd` belongs here too: it waits until the real process creation (after `spawn_blocking`) ends. Order carries no meaning —
        //   it is an ad-hoc creation that makes a new uuid every time, so there is no pair to overlap with.
        | AgentCommand::SpawnByCwd { .. }
        // ⟳ is outside the lane too: a probe launch waits up to the cap (`usage_service::REPLY_WAIT_MAX`). Order has no meaning — two ⟳
        // of the same vendor are merged by the service into one probe, and a subscription replacement reaches by broadcast either way.
        | AgentCommand::RefreshUsageLimits { .. } => DispatchOrder::Detached,

        // ── inside the lane: the five groups below hang on arrival order ────────────────────────────
        // ① Inputs to the same agent (`WriteStdin`↔`WriteStdin`/`Interrupt`) — nothing below the channel queue re-establishes order.
        //    `Interrupt`: refused on a PTY (ADR-0245), same FIFO as input on claude JSON (ADR-0238), acceptance depends on turn state on codex.
        //    Either way `Interrupt` is also bound by ②, so it stays in the lane.
        // ② Input lease acquire/return and the input after it — both write the table `check_input` reads.
        // ③ `Subscribe`↔`Subscribe`/`Unsubscribe` of one connection — `ReplayComplete` is queued after the core lock is released, so overlap
        //    would mix Ack/replay/Complete (R1).
        // ④ `RegisterCommands`↔`UpdateCommands` — full replacement vs a diff against it.
        // ⑤ `StopDaemon` must be the connection's last command (the only arm returning `Close`); detached, its answer to end the
        //    connection would come after the next command had already passed.
        AgentCommand::Kill { .. }
        | AgentCommand::Interrupt { .. }
        | AgentCommand::WriteStdin { .. }
        | AgentCommand::Resize { .. }
        | AgentCommand::Subscribe { .. }
        | AgentCommand::Unsubscribe { .. }
        | AgentCommand::AcquireInput { .. }
        | AgentCommand::ReleaseInput { .. }
        | AgentCommand::ListAgents { .. }
        | AgentCommand::StopDaemon { .. }
        | AgentCommand::ListProfiles { .. }
        | AgentCommand::CreateProfile { .. }
        | AgentCommand::DeleteProfile { .. }
        | AgentCommand::SetProfileAutoRestore { .. }
        | AgentCommand::RenameProfile { .. }
        | AgentCommand::ReparentProfile { .. }
        | AgentCommand::GetSnapshot { .. }
        | AgentCommand::SetEnvelopeFormat { .. }
        | AgentCommand::RegisterCommands { .. }
        | AgentCommand::UpdateCommands { .. }
        | AgentCommand::ListCommands { .. }
        | AgentCommand::Command { .. }
        | AgentCommand::CommandOutcome { .. }
        | AgentCommand::ListQueuedInputs { .. }
        // ADR-0231: a cancel belongs to ① — overtaking an earlier `WriteStdin` of the same connection, the roster would not yet know that text.
        | AgentCommand::CancelQueuedInput { .. }
        // A usage subscription is a full replacement: of two in a row the last must win. No await, so it does not hold the lane.
        | AgentCommand::UsageSubscribe { .. } => DispatchOrder::InOrder,
    }
}

// ── per-conn lifetime state (ConnectionSession) ─────────────────────────────────────────

/// The lifetime state of one connection, shared by dispatch (`on_text`) and cleanup (`on_disconnect`).
pub struct ConnectionSession {
    pub conn_id: ConnId,
    /// (agent_id → sink_id) registered by this connection — unsubscribed in cleanup.
    pub subs: Arc<Mutex<HashMap<AgentId, SinkId>>>,
    /// (agent_id, viewport_id) pairs registered by this connection — cleaned from the viewport negotiation map in cleanup.
    pub owned_viewports: Arc<Mutex<Vec<(AgentId, String)>>>,
    claimed_owner_warned: AtomicBool,
    /// Whether this connection already warned once about a command outcome with no round trip to attach to (the `CommandOutcome` arm).
    ///
    /// The client can send this frame any number of times and the release daemon's file sink is always on
    /// (`docs/reference/logging-conventions.md`); a per-frame `warn!` would hand log size to the other side. Same as `claimed_owner_warned`.
    stray_outcome_warned: AtomicBool,
    /// Whether this connection already warned once about an inbound-lane saturation refusal (`AgentConnection::on_inbound_saturated`).
    ///
    /// Saturation is decided per frame, so without a latch it would flood the file log. A cancel-family command passed through is
    /// logged every time outside the latch: rare, and the only trace of what unblocked the lane.
    pub(crate) saturated_warned: AtomicBool,
}

impl ConnectionSession {
    pub fn new(conn_id: ConnId) -> Self {
        Self {
            conn_id,
            subs: Arc::new(Mutex::new(HashMap::new())),
            owned_viewports: Arc::new(Mutex::new(Vec::new())),
            claimed_owner_warned: AtomicBool::new(false),
            stray_outcome_warned: AtomicBool::new(false),
            saturated_warned: AtomicBool::new(false),
        }
    }

    /// This connection's owner token in the command roster.
    ///
    /// ★Derived from the connection id — the registration packet's `owner` field is not trusted★: it would let any connection take
    /// another's names (registration overwrites even a live owner's names — `Roster::register`). Identity is the connection the envelope
    /// came from (TRD §4-⑧ · `CommandEnvelope::owner`; TRD §3-7).
    /// Provisional: a reconnect gets a new `ConnId` and the old registration is erased on disconnect (ADR-0150 decision 3). Slice B
    /// replaces this with a client-made identifier sent in the first greeting, stable across reconnects (ADR-0150 decisions 1·2 · TRD §3-7
    /// clauses 1·5); this derivation then remains only as the fail-open branch for a connection that sends no identifier.
    /// The token is not on the wire, so a registration `owner` differing from this is normal, not a refusal (`note_claimed_owner`).
    /// That field stays ignored even after slice B — removing it is a contract change with nothing to gain (ADR-0150 Impact).
    ///
    /// ★Not the authority★: the roster uses the value `attach` stored ([`CommandRoster::attached_owner`], ADR-0154); they agree only
    /// because `attach` derives by the same rule ([`CommandRoster::owner_of`]). No production consumer remains — only the test that
    /// checks what `attach` stores.
    // ADR-0150
    pub fn owner_token(&self) -> OwnerToken {
        CommandRoster::owner_of(self.conn_id)
    }
}

// ── multi-viewer negotiation state (MultiViewState) ───────────────────────────────────────────────
//
// Several connections may attach to one agent at once. Two policies live on the daemon side (the core receives only the final size
// and whether to pass): resize negotiation (tmux smallest: the PTY fits the min cols/rows of all the agent's viewports) and the
// input lease (Zellij explicit lease: one connection holds one agent's input authority).
//
// Concurrency: Arc<Mutex>. ★No manager.resize/await call while holding the lock★ — compute, release, then call the manager (core §10 lock order).

#[derive(Clone, Default)]
pub struct MultiViewState {
    inner: Arc<Mutex<MultiViewInner>>,
}

#[derive(Default)]
struct MultiViewInner {
    viewports: HashMap<AgentId, HashMap<String, (u16, u16)>>,
    leases: HashMap<AgentId, ConnId>,
}

enum LeasePass {
    Allow,
    Denied,
}

/// Refusal wording of [`LeasePass::Denied`]; the WS commands that affect input (`Interrupt`·`WriteStdin`·`CancelQueuedInput`)
/// refuse with the same characters.
// ADR-0231
pub(crate) const INPUT_LOCKED_REFUSAL: &str = "input locked by another viewer; acquire first";

impl MultiViewState {
    pub fn new() -> Self {
        Self::default()
    }

    /// `None` = no registered viewport (theoretically always `Some`, one was just inserted).
    fn set_viewport(
        &self,
        agent_id: AgentId,
        viewport_id: String,
        cols: u16,
        rows: u16,
    ) -> Option<(u16, u16)> {
        let mut g = self.inner.lock().expect("multiview poisoned");
        g.viewports
            .entry(agent_id)
            .or_default()
            .insert(viewport_id, (cols, rows));
        smallest(g.viewports.get(&agent_id))
    }

    /// Returns (agent_id, `Some(min)` = smallest of the remaining viewports / `None` = no viewport now).
    pub fn remove_conn_viewports(
        &self,
        owned: &[(AgentId, String)],
    ) -> Vec<(AgentId, Option<(u16, u16)>)> {
        let mut g = self.inner.lock().expect("multiview poisoned");
        let mut affected: Vec<AgentId> = Vec::new();
        for (agent_id, viewport_id) in owned {
            if let Some(m) = g.viewports.get_mut(agent_id) {
                m.remove(viewport_id);
                if m.is_empty() {
                    g.viewports.remove(agent_id);
                }
                if !affected.contains(agent_id) {
                    affected.push(*agent_id);
                }
            }
        }
        affected
            .into_iter()
            .map(|a| (a, smallest(g.viewports.get(&a))))
            .collect()
    }

    /// Ok(true)=newly acquired (state change), Ok(false)=this conn already holds (idempotent), Err=another conn holds.
    fn acquire(&self, agent_id: AgentId, conn_id: ConnId) -> Result<bool, ()> {
        let mut g = self.inner.lock().expect("multiview poisoned");
        match g.leases.get(&agent_id) {
            None => {
                g.leases.insert(agent_id, conn_id);
                Ok(true)
            }
            Some(&holder) if holder == conn_id => Ok(false),
            Some(_) => Err(()),
        }
    }

    /// Ok(true)=released (state change), Ok(false)=it was empty to begin with, Err=another conn holds
    /// (only the holder can release).
    fn release(&self, agent_id: AgentId, conn_id: ConnId) -> Result<bool, ()> {
        let mut g = self.inner.lock().expect("multiview poisoned");
        match g.leases.get(&agent_id) {
            Some(&holder) if holder == conn_id => {
                g.leases.remove(&agent_id);
                Ok(true)
            }
            None => Ok(false),
            Some(_) => Err(()),
        }
    }

    fn check_input(&self, agent_id: AgentId, conn_id: ConnId) -> LeasePass {
        self.lease_pass(agent_id, Some(conn_id))
    }

    /// The only place of the input lease decision — WS (`check_input`) and the bus·CLI (`InputLease`) both come here.
    /// `caller == None` is a caller with no connection, so it cannot be the holder (refused if anyone holds it).
    // ADR-0231
    fn lease_pass(&self, agent_id: AgentId, caller: Option<ConnId>) -> LeasePass {
        let g = self.inner.lock().expect("multiview poisoned");
        match g.leases.get(&agent_id) {
            None => LeasePass::Allow,
            Some(&holder) if Some(holder) == caller => LeasePass::Allow,
            Some(_) => LeasePass::Denied,
        }
    }

    /// Prevents a zombie lock. The returned agents now have an empty lease and are the targets for broadcasting InputLeaseChanged{held:false}.
    pub fn release_all_for_conn(&self, conn_id: ConnId) -> Vec<AgentId> {
        let mut g = self.inner.lock().expect("multiview poisoned");
        let freed: Vec<AgentId> = g
            .leases
            .iter()
            .filter(|(_, &h)| h == conn_id)
            .map(|(a, _)| *a)
            .collect();
        for a in &freed {
            g.leases.remove(a);
        }
        freed
    }
}

// ADR-0231: the lease port of the command table's common entrance — decision is the same `lease_pass` as WS.
impl crate::control::commands::InputLease for MultiViewState {
    fn permits(&self, agent_id: AgentId, caller: Option<ConnId>) -> bool {
        matches!(self.lease_pass(agent_id, caller), LeasePass::Allow)
    }
}

fn smallest(views: Option<&HashMap<String, (u16, u16)>>) -> Option<(u16, u16)> {
    let m = views?;
    let mut it = m.values();
    let &(mut c, mut r) = it.next()?;
    for &(vc, vr) in it {
        c = c.min(vc);
        r = r.min(vr);
    }
    Some((c, r))
}

// ── type conversion (core → wire) ─────────────────────────────────────────────────────
//
// ★No reflection round trip (do not revive)★: core↔wire through serde_json::to_value→from_value compiles when a field/tag drifts and
// silently drops (None) at runtime. Fields are mapped explicitly so core changes give a compile error.
// The conversion lives in the daemon crate (the core stays protocol-free — a §1 invariant); the orphan rule forbids `impl From`
// between two external types.

fn caps_to_wire(c: &CoreCaps) -> WireCaps {
    WireCaps {
        input: WireInputCaps {
            raw: c.input.raw,
            message: c.input.message,
            attachment: c.input.attachment,
        },
        output: WireOutputCaps {
            terminal_bytes: c.output.terminal_bytes,
            structured: c.output.structured,
            markdown: c.output.markdown,
            tool_events: c.output.tool_events,
            usage: c.output.usage,
        },
        control: WireControlCaps {
            resize: c.control.resize,
            interrupt: c.control.interrupt,
            cancel: c.control.cancel,
            graceful_shutdown: c.control.graceful_shutdown,
        },
        session: WireSessionCaps {
            resume: c.session.resume,
            snapshot: c.session.snapshot,
            cwd_env: c.session.cwd_env,
        },
        model: WireModelCaps {
            select: c.model.select,
            temperature: c.model.temperature,
            max_tokens: c.model.max_tokens,
        },
    }
}

fn status_to_wire(status: &CoreStatus) -> engram_dashboard_protocol::AgentStatus {
    use engram_dashboard_protocol::AgentStatus as W;
    match status {
        CoreStatus::Running => W::Running,
        CoreStatus::Exiting => W::Exiting,
        CoreStatus::Exited { code } => W::Exited { code: *code },
        CoreStatus::Failed { message } => W::Failed {
            message: message.clone(),
        },
        CoreStatus::Killed => W::Killed,
    }
}

pub(crate) fn agent_info_to_wire(a: &CoreAgentInfo) -> WireAgentInfo {
    WireAgentInfo {
        id: a.id,
        name: a.name.clone(),
        cwd: a.cwd.clone(),
        status: status_to_wire(&a.status),
        cols: a.cols,
        rows: a.rows,
        epoch: a.epoch,
        capabilities: caps_to_wire(&a.capabilities),
    }
}

/// Launch notices → the `Spawned.notices` rows — `None` for none (the wire never carries an empty list).
// TRD S25 §5-2 16
fn notices_to_wire(notices: &[Notice]) -> Option<Vec<serde_json::Value>> {
    (!notices.is_empty()).then(|| notices.iter().map(notice_row).collect())
}

pub(crate) fn restore_outcome_to_wire(outcome: &CoreRestoreOutcome) -> WireRestoreOutcome {
    match outcome {
        CoreRestoreOutcome::Resumed => WireRestoreOutcome::Resumed,
        CoreRestoreOutcome::Started => WireRestoreOutcome::Started,
        CoreRestoreOutcome::FreshFallback {
            old_sid,
            new_sid,
            reason,
        } => WireRestoreOutcome::FreshFallback {
            old_sid: old_sid.map(|u| u.to_string()),
            new_sid: new_sid.to_string(),
            reason: reason.clone(),
        },
        CoreRestoreOutcome::Blocked { reason } => WireRestoreOutcome::Blocked {
            reason: reason.clone(),
        },
        CoreRestoreOutcome::Failed { reason } => WireRestoreOutcome::Failed {
            reason: reason.clone(),
        },
    }
}

pub(crate) fn core_agents_to_wire(agents: Vec<CoreAgentInfo>) -> Vec<WireAgentInfo> {
    agents.iter().map(agent_info_to_wire).collect()
}

/// The wording returned when the backend field of a spawn packet is empty. It names the field to fill, so the next spawn entrance
/// builder can fix it from this one reply.
const MISSING_BACKEND: &str = "the backend field is empty — write \"claude\" or \"codex\" in `backend` of the spawn packet (SpawnByCwd·CreateProfile). There is no default (a decision to tell apart not choosing from choosing claude — a different backend does not come up quietly).";

/// The wire backend choice → the core run command. `None` = the field is empty (the caller refuses with [`MISSING_BACKEND`]).
///
/// ★Both arms carry the `output_format` they were given — this function fills no default★: filling one here would
/// split the requested mode from the launched one, unseen until the agent behaves differently. `CreateProfile` passes
/// the packet's field; `SpawnByCwd`, which has no mode field, does not come here — the agent crate's creation body
/// (`commands::create_and_start`) holds that default. ★No wildcard★: a new wire word stops the build at this match.
fn spawn_command_for(
    backend: Option<WireBackendKind>,
    extra_args: Vec<String>,
    output_format: CoreAgentOutputFormat,
) -> Option<CoreSpawnCommand> {
    match backend? {
        WireBackendKind::Claude => Some(CoreSpawnCommand::Claude {
            extra_args,
            output_format,
        }),
        WireBackendKind::Codex => Some(CoreSpawnCommand::Codex {
            extra_args,
            output_format,
        }),
    }
}

/// Backend-creation policy shape — takes one word, `None` = create, `Some(reason)` = do not create.
type BackendPolicyFn = fn(&str) -> Option<&'static str>;

/// The wire backend's word, for the `SpawnByCwd` refusal text only — the policy itself is asked by the creation body
/// with the declared spelling. Taken from the enum's serialization (always lowercase), not written by hand.
fn wire_backend_word(kind: WireBackendKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The refusal text when the policy closes a `SpawnByCwd` backend — `reason` is the policy's.
///
/// ★Must stand alone★: the daemon `Error` text reaches the bus caller with no prefix (the shell's `agent.spawnInto` carries it as
/// `CONFLICT`). The front is the `agent.new` policy-refusal frame (agent `verb_new`) plus "Not spawned."; this side's word is the
/// lowercase wire serialization (that side: `Claude`) and the code is `CONFLICT` here vs `INVALID_ARGUMENT` there. Kept apart from the
/// typo net's "unknown backend" — lumped, the caller would fix a typo that is not there.
// ADR-0279
fn by_cwd_refusal(kind: WireBackendKind, reason: &str) -> String {
    let word = wire_backend_word(kind);
    format!(
        "backend '{word}' is a known word, but this surface does not create it now — {reason} Not spawned."
    )
}

fn output_format_to_wire(f: CoreAgentOutputFormat) -> WireAgentOutputFormat {
    match f {
        CoreAgentOutputFormat::Terminal => WireAgentOutputFormat::Terminal,
        CoreAgentOutputFormat::StreamJson => WireAgentOutputFormat::StreamJson,
    }
}

fn spawn_command_to_wire(cmd: &CoreSpawnCommand) -> WireSpawnCommand {
    match cmd {
        CoreSpawnCommand::Claude {
            extra_args,
            output_format,
        } => WireSpawnCommand::Claude {
            extra_args: extra_args.clone(),
            output_format: output_format_to_wire(*output_format),
        },
        CoreSpawnCommand::Shell { program, args } => WireSpawnCommand::Shell {
            program: program.clone(),
            args: args.clone(),
        },
        CoreSpawnCommand::Codex {
            extra_args,
            output_format,
        } => WireSpawnCommand::Codex {
            extra_args: extra_args.clone(),
            output_format: output_format_to_wire(*output_format),
        },
    }
}

fn restart_policy_to_wire(p: CoreRestartPolicy) -> WireRestartPolicy {
    match p {
        CoreRestartPolicy::Never => WireRestartPolicy::Never,
        CoreRestartPolicy::OnCrash => WireRestartPolicy::OnCrash,
        CoreRestartPolicy::Always => WireRestartPolicy::Always,
    }
}

// ADR-0172
fn failure_kind_to_wire(k: CoreFailureKind) -> WireFailureKind {
    match k {
        CoreFailureKind::NoConversationToResume => WireFailureKind::NoConversationToResume,
        CoreFailureKind::SpawnFailed => WireFailureKind::SpawnFailed,
        CoreFailureKind::EarlyExitAfterResume => WireFailureKind::EarlyExitAfterResume,
        CoreFailureKind::Other => WireFailureKind::Other,
    }
}

fn profile_to_wire(p: &CoreProfile) -> WireProfile {
    WireProfile {
        id: p.id,
        name: p.name.clone(),
        display_name: p.display_name.clone(),
        // The core AgentId and the wire ProfileId are the same Uuid alias, so they are copied without conversion.
        parent_id: p.parent_id,
        command: spawn_command_to_wire(&p.command),
        cwd: p.cwd.to_string_lossy().into_owned(),
        env: p.env.clone(),
        backend_session_id: p.backend_session_id.map(|u| u.to_string()),
        old_session_ids: p.old_session_ids.iter().map(|u| u.to_string()).collect(),
        epoch: p.epoch,
        auto_restore: p.auto_restore,
        restart_policy: restart_policy_to_wire(p.restart_policy),
        restart_count: p.restart_count,
        failed_reason: p.failed_reason.clone(),
        last_failure: p.last_failure.map(failure_kind_to_wire),
        created_at: p.created_at,
        last_active: p.last_active,
        last_start_at: p.last_start_at,
    }
}

fn core_profiles_to_wire(profiles: Vec<CoreProfile>) -> Vec<WireProfile> {
    profiles.iter().map(profile_to_wire).collect()
}

fn snapshot_chunk_to_wire(c: &CoreOutputChunk) -> WireSnapshotChunk {
    WireSnapshotChunk {
        seq: c.seq,
        data: c.data.clone(),
    }
}

/// Returns `Option` as a TerminalBytes defense: `OutputEvent::TerminalBytes` should never reach this conversion (console raw bytes are
/// split at the sink into a tag0 frame; only the `OutputPayload::Event` arm calls this) and the wire `StructuredEvent` has no such
/// variant (ADR-0045). `None` instead of a panic: a tag0/tag1 misclassification is an upstream wiring bug, not worth killing the
/// connection (call site: `agent_conn::FrameOutputSink::send`).
pub(crate) fn output_event_to_wire(ev: &CoreOutputEvent) -> Option<WireStructuredEvent> {
    match ev {
        CoreOutputEvent::TerminalBytes(_) => None,
        CoreOutputEvent::TextDelta {
            text,
            turn_id,
            message_id,
        } => Some(WireStructuredEvent::TextDelta {
            text: text.clone(),
            turn_id: turn_id.clone(),
            message_id: message_id.clone(),
        }),
        CoreOutputEvent::ToolCall {
            name,
            args_json,
            id,
            turn_id,
            message_id,
            category,
        } => Some(WireStructuredEvent::ToolCall {
            name: name.clone(),
            args_json: args_json.clone(),
            id: id.clone(),
            turn_id: turn_id.clone(),
            message_id: message_id.clone(),
            category: Some(tool_category_to_wire(*category)),
        }),
        CoreOutputEvent::ToolResult { id, outcome } => Some(WireStructuredEvent::ToolResult {
            id: id.clone(),
            outcome: tool_outcome_to_wire(*outcome),
        }),
        CoreOutputEvent::Usage {
            input_tokens,
            output_tokens,
            turn_id,
        } => Some(WireStructuredEvent::Usage {
            input_tokens: *input_tokens,
            output_tokens: *output_tokens,
            turn_id: turn_id.clone(),
        }),
        CoreOutputEvent::MessageDone {
            turn_id,
            message_id,
        } => Some(WireStructuredEvent::MessageDone {
            turn_id: turn_id.clone(),
            message_id: message_id.clone(),
        }),
        CoreOutputEvent::TurnEnd { turn_id, outcome } => Some(WireStructuredEvent::TurnEnd {
            turn_id: turn_id.clone(),
            outcome: turn_outcome_to_wire(outcome),
        }),
        CoreOutputEvent::Error(message) => Some(WireStructuredEvent::Error {
            message: message.clone(),
        }),
        CoreOutputEvent::Structured { kind, json } => Some(WireStructuredEvent::Structured {
            kind: kind.clone(),
            json: json.clone(),
        }),
        CoreOutputEvent::QueuedInput(op) => Some(WireStructuredEvent::QueuedInput {
            op: queued_input_to_wire(op),
        }),
    }
}

/// A roster-event domain → wire. ★No `_` branch★: new events/causes must be a compile error so none silently folds into another
/// (the frontend accumulator reduces like the roster only if the event sequence crosses as is).
// ADR-0231
fn queued_input_to_wire(op: &CoreQueuedInputEvent) -> WireQueuedInputEvent {
    match op {
        CoreQueuedInputEvent::Queued { id, text } => WireQueuedInputEvent::Queued {
            id: id.clone(),
            text: text.clone(),
        },
        CoreQueuedInputEvent::CancelRequested { id } => {
            WireQueuedInputEvent::CancelRequested { id: id.clone() }
        }
        CoreQueuedInputEvent::CancelAnswered { id, removed } => {
            WireQueuedInputEvent::CancelAnswered {
                id: id.clone(),
                removed: *removed,
            }
        }
        CoreQueuedInputEvent::CancelFailed { id } => {
            WireQueuedInputEvent::CancelFailed { id: id.clone() }
        }
        CoreQueuedInputEvent::HandedOver { id, sent } => WireQueuedInputEvent::HandedOver {
            id: id.clone(),
            sent: *sent,
        },
        CoreQueuedInputEvent::Delivered { id } => {
            WireQueuedInputEvent::Delivered { id: id.clone() }
        }
        CoreQueuedInputEvent::Dropped { id, cause } => WireQueuedInputEvent::Dropped {
            id: id.clone(),
            cause: drop_cause_to_wire(*cause),
        },
        CoreQueuedInputEvent::AckUnavailable { delivered } => {
            WireQueuedInputEvent::AckUnavailable {
                delivered: delivered
                    .iter()
                    .map(|CoreDeliveredCopy { id, text }| WireDeliveredCopy {
                        id: id.clone(),
                        text: text.clone(),
                    })
                    .collect(),
            }
        }
    }
}

fn drop_cause_to_wire(cause: CoreDropCause) -> WireDropCause {
    match cause {
        CoreDropCause::Withdrawn => WireDropCause::Withdrawn,
        CoreDropCause::Interrupted => WireDropCause::Interrupted,
        CoreDropCause::AgentEnded => WireDropCause::AgentEnded,
        CoreDropCause::Rejected => WireDropCause::Rejected,
        CoreDropCause::Unknown => WireDropCause::Unknown,
    }
}

/// The answer of one list lookup → wire, using the `as_str` words of the bus `agent.listQueuedInputs`. Destructured so a new
/// manager field is a compile error.
// ADR-0231
fn queued_listing_to_wire(
    request_id: engram_dashboard_protocol::RequestId,
    agent_id: AgentId,
    listing: QueuedListing,
) -> AgentEvent {
    let QueuedListing {
        rows,
        as_of_seq,
        epoch,
        stopped_after_error,
    } = listing;
    let inputs = rows
        .into_iter()
        .map(|ListedRow { id, text, state }| WireQueuedInputRow {
            id,
            text,
            state: state.as_str().to_string(),
            cancel: match state {
                ListedState::Cancelling {
                    answer,
                    vendor_closed,
                } => Some(WireQueuedInputCancel {
                    answer: answer.as_str().to_string(),
                    vendor_closed,
                }),
                ListedState::Queued | ListedState::Sent | ListedState::Unconfirmed => None,
            },
        })
        .collect();
    AgentEvent::QueuedInputs {
        request_id,
        agent_id,
        inputs,
        as_of_seq,
        epoch,
        stopped_after_error,
    }
}

/// List lookup failure → `Error` wording (`CODE: wording`); the manager's failure is the single "no live session".
fn list_error_text(agent_id: AgentId, e: PtyError) -> String {
    match e {
        PtyError::NotFound(_) => CommandError::not_found(format!(
            "agent {agent_id} is not running, so it holds no queued inputs — an asleep agent has nothing waiting"
        )),
        other => CommandError::internal(format!(
            "listing the queued inputs of agent {agent_id} failed: {other}"
        )),
    }
    .to_string()
}

/// Cancel failure → `Error` wording (`CODE: wording`) — codes match the bus `agent.cancelQueuedInput`.
fn cancel_error_text(agent_id: AgentId, input_id: &str, e: CancelError) -> String {
    match e {
        // "Already settled" and "went dormant" are not told apart — the caller does the same thing (re-read the list).
        CancelError::NotFound => CommandError::not_found(format!(
            "agent {agent_id} has no waiting input '{input_id}' — it was already delivered, cancelled or dropped, or the agent is not running; list its queued inputs to see what is still waiting"
        )),
        // No resend instruction: the item is still waiting to be cancelled, so a re-request writes nothing and is `requested`.
        CancelError::Write(e) => CommandError::internal(format!(
            "the cancel request for '{input_id}' could not be written to agent {agent_id}: {e} — the item stays in the cancelling state and sending it again will not resend it; its fate now follows the agent's own lifecycle, so watch it in the queued-input list"
        )),
    }
    .to_string()
}

/// The store did not accept this request's change → `Error` wording (`CODE: wording`). The code split (refusal = `CONFLICT`, write
/// failure = `INTERNAL`) and reason wording match the command bus (agent `store_command_error` · `store_error_reason`); the WS wording
/// carries no retry instruction, so "try again shortly" comes from that reason. `what` = what failed · `file` = the file that did not accept.
/// This is the one line recording the refusal on the request side (the store's log records the file side). Caller-sent values
/// (name · path) are not carried.
// ADR-0291 R17
fn store_error_text(
    conn_id: ConnId,
    verb: &'static str,
    file: &'static str,
    what: &str,
    error: &StoreError,
) -> String {
    tracing::warn!(
        conn = conn_id,
        verb,
        file,
        %error,
        "the store did not accept it; returning this request as an error (WS)"
    );
    store_command_error(
        error,
        format!("{what}: {}", store_error_reason(file, error)),
    )
    .to_string()
}

/// Failure of a profile-side request (the manager's `try_` version · activation) → `Error` wording. Only the profile file's save `Err`
/// goes through [`store_error_text`]; the rest is the `PtyError` Display as is.
// ADR-0291 R17
fn profile_error_text(conn_id: ConnId, verb: &'static str, what: &str, e: PtyError) -> String {
    match e {
        PtyError::Store(error) => store_error_text(conn_id, verb, PROFILES_FILE, what, &error),
        other => other.to_string(),
    }
}

/// The failure reply of a wire activation (`Spawn` · `SpawnByCwd` · `SpawnProfile`) — [`profile_error_text`] as the
/// message, plus a launch block's reasons as `launch_blocked` (the `LaunchBlocked::to_args` object — what the bus
/// `agent.spawn` wake refusal carries as `args`; the bus after-create refusal adds `agent_id`).
// TRD S25 §1-8 · §5-2 16
fn activation_error(
    conn_id: ConnId,
    verb: &'static str,
    what: &str,
    request_id: engram_dashboard_protocol::RequestId,
    e: PtyError,
) -> AgentEvent {
    AgentEvent::Error {
        request_id: Some(request_id),
        launch_blocked: launch_blocked_args(&e),
        message: profile_error_text(conn_id, verb, what, e),
        // Only the `SpawnByCwd` created-asleep arm names an agent — every activation failure here created nothing.
        // ADR-0348
        created_agent_id: None,
    }
}

/// A launch block's reasons as the `Error.launch_blocked` object (`LaunchBlocked::to_args`); `None` for any other
/// failure.
fn launch_blocked_args(e: &PtyError) -> Option<serde_json::Map<String, serde_json::Value>> {
    match e {
        PtyError::LaunchBlocked(blocked) => Some(blocked.to_args()),
        _ => None,
    }
}

/// `SpawnByCwd.deliver_notices` → the creation body's notice delivery: `true` = the sender shows the reply's notices
/// to a reader, so the set may be marked as shown.
// ADR-0348
fn delivery_for(deliver_notices: bool) -> NoticeDelivery {
    if deliver_notices {
        NoticeDelivery::Delivered
    } else {
        NoticeDelivery::Withheld
    }
}

/// Tool kind domain → wire. ★No `_` branch★: a new kind must be a compile error so it never silently folds into another kind.
/// The daemon does not know vendor tool names — each backend translator already decided.
// ADR-0239
fn tool_category_to_wire(category: CoreToolCategory) -> WireToolCategory {
    match category {
        CoreToolCategory::Read => WireToolCategory::Read,
        CoreToolCategory::Search => WireToolCategory::Search,
        CoreToolCategory::List => WireToolCategory::List,
        CoreToolCategory::Edit => WireToolCategory::Edit,
        CoreToolCategory::Command => WireToolCategory::Command,
        CoreToolCategory::Web => WireToolCategory::Web,
        CoreToolCategory::Agent => WireToolCategory::Agent,
        CoreToolCategory::Mcp => WireToolCategory::Mcp,
        CoreToolCategory::Other => WireToolCategory::Other,
    }
}

/// Tool-end outcome domain → wire. ★No `_` branch★: a denial folded into a failure would make "N errors" false.
// ADR-0241
fn tool_outcome_to_wire(outcome: CoreToolOutcome) -> WireToolOutcome {
    match outcome {
        CoreToolOutcome::Completed => WireToolOutcome::Completed,
        CoreToolOutcome::Failed => WireToolOutcome::Failed,
        CoreToolOutcome::Declined => WireToolOutcome::Declined,
        CoreToolOutcome::Refused => WireToolOutcome::Refused,
    }
}

/// Turn outcome domain → wire. ★No `_` branch★: a new outcome must not silently fold into `Unknown`.
fn turn_outcome_to_wire(outcome: &CoreTurnOutcome) -> WireTurnOutcome {
    match outcome {
        CoreTurnOutcome::Completed => WireTurnOutcome::Completed,
        CoreTurnOutcome::Failed { detail } => WireTurnOutcome::Failed {
            detail: detail.clone(),
        },
        CoreTurnOutcome::Interrupted => WireTurnOutcome::Interrupted,
        CoreTurnOutcome::Unknown => WireTurnOutcome::Unknown,
    }
}

pub(crate) fn core_report_to_wire(report: CoreRestoreReport) -> RestoreReport {
    RestoreReport {
        agent_id: report.agent_id,
        epoch: report.epoch,
        outcome: restore_outcome_to_wire(&report.outcome),
    }
}

pub(crate) fn core_status_to_wire(status: CoreStatus) -> engram_dashboard_protocol::AgentStatus {
    status_to_wire(&status)
}

// ADR-0310: the owner mark of a registration packet is a protocol copy, moved here into the tool-crate type the roster uses.
fn owner_from_wire(owner: WireOwnerToken) -> OwnerToken {
    OwnerToken(owner.0)
}

/// Unpacks the envelope field (raw JSON) of a command packet into the tool-crate envelope (ADR-0272 decision 4).
///
/// `Err` = the envelope shape is wrong (ADR-0272 decision 6) — the receiver refuses with `Error{request_id: None}` (the number could not be read).
// ADR-0272
pub(crate) fn envelope_from_wire(raw: &RawValue) -> Result<CommandEnvelope, serde_json::Error> {
    serde_json::from_str(raw.get())
}

/// Re-wraps the envelope to deliver into the packet's raw field (ADR-0272 decision 6). The arguments the relay fixed (ADR-0308
/// `resolve_agent_args`) ride in these bytes — passing the received raw as is would lose that fix.
// ADR-0272
pub(crate) fn envelope_to_wire(
    envelope: &CommandEnvelope,
) -> Result<Box<RawValue>, serde_json::Error> {
    serde_json::value::to_raw_value(envelope)
}

/// Unpacks the reply field (raw JSON) of the `CommandOutcome` packet into the tool-crate reply (ADR-0272 decision 4).
///
/// `Err` = the reply shape is wrong (ADR-0272 decision 6) — the number cannot be read, so the receiver refuses with `Error{request_id: None}`.
// ADR-0272
pub(crate) fn reply_from_wire(raw: &RawValue) -> Result<CommandReply, serde_json::Error> {
    serde_json::from_str(raw.get())
}

/// Wraps the reply into the raw field of `AgentEvent::CommandReply`. The retry instruction the relay issued
/// (`command_delivery::send_reply`) rides in these bytes — passing the owner's raw as is would lose it.
// ADR-0272
pub(crate) fn reply_to_wire(reply: &CommandReply) -> Result<Box<RawValue>, serde_json::Error> {
    serde_json::value::to_raw_value(reply)
}

/// Unpacks the item-list field (raw JSON) of a registration packet (`RegisterCommands.decls` · `UpdateCommands.added`) into the
/// tool-crate registration items (ADR-0272 decision 3). `agent_args` is put on the roster here and read by the pre-relay rewrite
/// (`command_delivery::resolve_agent_args`); an old item with no such field is an empty list (ADR-0308).
///
/// `Err` = the list shape is wrong (ADR-0272 decision 6) — the receiver refuses the packet, roster untouched, with the out-of-field `request_id` (`INVALID_ARGUMENT`).
// ADR-0272
// ADR-0308
pub(crate) fn decls_from_wire(raw: &RawValue) -> Result<Vec<CommandDecl>, serde_json::Error> {
    serde_json::from_str(raw.get())
}

/// The refusal of a registration packet whose item-list field is broken (ADR-0272 decision 6). ★The number is outside the field, so
/// it answers with that packet's `request_id`★ and the sender's wait wakes at once (the shell's registration waits have no cap, so `None` would hang it until the connection drops).
/// Code is `INVALID_ARGUMENT`; the whole packet is rejected, so the roster stays as it was.
fn broken_decls_refusal(e: &serde_json::Error) -> CommandError {
    CommandError::invalid_argument(format!(
        "invalid command declarations: {e} — this packet was not applied"
    ))
}

/// wire vendor → the vendor key of the usage entry. `None` = that vendor has no probe.
///
/// The key is the probe's key, not the incoming word: the bus entrance (uppercase word) and this one must converge on the same
/// probe key, so it goes through the wire word rather than matching the vendor.
// ADR-0004
fn usage_vendor_key(vendor: WireBackendKind) -> Option<UsageVendorKey> {
    let word = serde_json::to_value(vendor).ok()?;
    usage_probe_for(word.as_str()?).map(|probe| probe.key())
}

fn no_usage_text(vendor: WireBackendKind) -> String {
    let word = serde_json::to_value(vendor)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    CommandError::not_found(format!(
        "this daemon keeps no usage limits for backend '{word}'"
    ))
    .to_string()
}

/// `None` = serialization failed (already logged here).
pub(crate) fn event_json(ev: &AgentEvent) -> Option<String> {
    match serde_json::to_string(ev) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::error!("AgentEvent serialization failed: {e}");
            None
        }
    }
}

fn kind_to_action(kind: ReplayKind) -> SubscribeAction {
    match kind {
        ReplayKind::FromOldest => SubscribeAction::Reset,
        ReplayKind::Truncated => SubscribeAction::TruncatedReplay,
        ReplayKind::Resumed => SubscribeAction::Resume,
    }
}

// ── ConnectionCore ────────────────────────────────────────────────────────────────

/// Created anew per connection (`agent_conn::AgentConnections::handler_for`). What is one per server is that factory; every field
/// here except `llm_policy` (a stateless function pointer) is a clone of a handle shared by all connections.
/// ★A new field must be a shared handle★, or it quietly becomes per-connection state; connection-specific state belongs in `ConnectionSession`.
pub struct ConnectionCore {
    manager: Arc<AgentManager>,
    multiview: MultiViewState,
    // ADR-0129
    fanout: Arc<dyn FrameFanout>,
    // ADR-0096
    control_registry: Arc<ControlRegistry>,
    /// An empty slot = this assembly (experiment bin · some tests) has no messaging, so `DeleteProfile` has nothing to clean up.
    // ADR-0116
    messaging: Arc<crate::control::mcp_server::MessagingSlot>,
    // ADR-0155
    commands: CommandRoster,
    // ADR-0154
    deliveries: CommandDeliveries,
    /// The commands this daemon answers by itself — delivery stage 1 and one side of the discovery list (`command_delivery`).
    // ADR-0155
    locals: Arc<dyn LocalCommands>,
    usage: Arc<UsageService>,
    shutdown_tx: watch::Sender<bool>,
    /// Set by the first `StopDaemon`; a later `StopDaemon` joins instead of running `shutdown_all` again.
    stopping: AtomicBool,
    #[cfg(test)]
    shutdowns_spawned: std::sync::atomic::AtomicUsize,
    /// The backend creation policy the `SpawnByCwd` arm passes to the one creation body (agent
    /// `commands::create_and_start`). Production is always agent `commands::llm_creation_refusal`, filled by
    /// [`ConnectionCore::new`]; only tests swap in a closing fake (today's production table closes nothing, so real
    /// input cannot reach the refusal arm · ADR-0012).
    /// A stateless function pointer, so a fresh copy per connection cannot diverge.
    // ADR-0279
    llm_policy: BackendPolicyFn,
}

impl ConnectionCore {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        manager: Arc<AgentManager>,
        multiview: MultiViewState,
        fanout: Arc<dyn FrameFanout>,
        control_registry: Arc<ControlRegistry>,
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
            stopping: AtomicBool::new(false),
            #[cfg(test)]
            shutdowns_spawned: std::sync::atomic::AtomicUsize::new(0),
            llm_policy: llm_creation_refusal,
        }
    }

    #[cfg(test)]
    fn with_llm_policy(mut self, policy: BackendPolicyFn) -> Self {
        self.llm_policy = policy;
        self
    }

    pub fn usage(&self) -> &UsageService {
        &self.usage
    }

    pub fn multiview(&self) -> &MultiViewState {
        &self.multiview
    }

    pub fn commands(&self) -> &CommandRoster {
        &self.commands
    }

    pub fn deliveries(&self) -> &CommandDeliveries {
        &self.deliveries
    }

    pub fn fanout(&self) -> &dyn FrameFanout {
        self.fanout.as_ref()
    }

    pub fn manager(&self) -> &Arc<AgentManager> {
        &self.manager
    }

    /// A name the daemon answers by itself cannot be registered by a client — a collision is rejected with `CONFLICT`.
    ///
    /// Accepting it would report success while giving no traffic: delivery is answered first by the daemon (stage 1,
    /// `command_delivery::deliver`), so no envelope reaches that owner. ★The whole packet is rejected, not just the colliding name★
    /// (all-or-nothing like the roster's own collisions — `Roster::update` doc), decided before touching the roster. `CONFLICT`, not
    /// `INVALID_ARGUMENT`: the packet is fine, the daemon's state already holds the name.
    // ADR-0155
    fn refuse_names_i_answer(&self, decls: &[CommandDecl]) -> Result<(), CommandError> {
        // Count every colliding name: registration is once at attach (TRD §3-7 clause 1), so naming only the first costs one reconnect per name.
        // The log sanitizer is used though the destination is a response wording (canonical split: `control::agent::preview` doc): the
        // receiver is a program that flows the value into its logs/UI, so raw control characters would plant a forged entry in its log; there
        // is no original-value comparison to preserve, and truncation is marked (`…`). The quoted-name shape matches the roster disconnect
        // log (`logged_names` of `command_roster`).
        let clashing: Vec<String> = decls
            .iter()
            .filter(|decl| self.locals.claim(&decl.name).is_some())
            .map(|decl| format!("'{}'", sanitize_for_log(&decl.name)))
            .collect();
        if clashing.is_empty() {
            return Ok(());
        }
        Err(CommandError::of(
            ErrorCode::Conflict,
            format!(
                "answered by this daemon itself, so they cannot be registered: {} — pick different names; this packet was not applied",
                clashing.join(", ")
            ),
        ))
    }

    /// sink.enqueue failure (SinkError) is ignored: the adapter's enqueue is non-blocking (try_send), so on a saturated queue it
    /// drops and signals close at once; the end point (connection close) is the same either way.
    pub async fn dispatch(
        &self,
        cmd: AgentCommand,
        session: &ConnectionSession,
        sink: &dyn OutboundSink,
    ) -> DispatchFlow {
        use engram_dashboard_protocol::RequestId;

        let manager = &self.manager;
        let multiview = &self.multiview;
        let fanout = self.fanout.as_ref();
        let conn_id = session.conn_id;
        let subs = &session.subs;
        let owned_viewports = &session.owned_viewports;

        fn reply(sink: &dyn OutboundSink, request_id: RequestId, result: Result<(), String>) {
            let ev = match result {
                Ok(()) => AgentEvent::Ack { request_id },
                Err(message) => AgentEvent::Error {
                    request_id: Some(request_id),
                    message,
                    launch_blocked: None,
                    created_agent_id: None,
                },
            };
            let _ = sink.enqueue(Outbound::event(ev));
        }

        /// Turns a roster-operation failure into a reply, carrying the code too.
        ///
        /// `Error` has no typed field, so it goes out as the `CommandError` Display (`CODE: wording`); without the code the caller must
        /// pattern-match wording (TRD §4-⑦). No reply at all would hang the sender's `request_id` wait.
        /// A refusal is logged on the server too (the reply alone leaves the operator no trace); the normal path is quiet. The wording quotes a
        /// client string, so it goes through `sanitize_for_log`.
        fn reply_roster(
            sink: &dyn OutboundSink,
            conn_id: ConnId,
            verb: &'static str,
            request_id: RequestId,
            result: Result<(), CommandError>,
        ) {
            if let Err(e) = &result {
                tracing::warn!(
                    conn = conn_id,
                    verb,
                    code = %e.code(),
                    "command roster refusal: {}",
                    sanitize_for_log(e.message())
                );
            }
            reply(sink, request_id, result.map_err(|e| e.to_string()));
        }

        match cmd {
            AgentCommand::Spawn {
                profile_id,
                request_id,
            } => {
                // ★Do not call `spawn_agent` directly — the activation entrance must be one★: it neither records nor clears the "last failure"
                // (ADR-0172) and has no reactivation guard. Only difference from `SpawnProfile`: no mode derivation (always Fresh).
                // ADR-0172
                // The success is an `Ack`, which has no place for launch notices — so `activate_profile` (drops them,
                //   records nothing). A launch block still answers with its reasons.
                // TRD S25 §5-2 16
                match manager.agent_snapshot(profile_id) {
                    Some(profile) => {
                        // Hand this to the blocking pool, never call it here: `activate_profile` blocks until an outcome on a connection-declaring transport
                        // (backstop 15 s) and, on a failure verdict, runs the teardown `session.kill` (pump join bounded by 5 s) — ~20 s with no yield point,
                        // tying up an async worker (N stuck activations take N workers and stall other connections). `Kill` and `StopDaemon` do the same;
                        // the bus sibling is covered by `blocking_handler` (`agent::commands::make_table`).
                        let mgr = manager.clone();
                        let started = tokio::task::spawn_blocking(move || {
                            mgr.activate_profile(&profile, SpawnMode::Fresh).map(|_| ())
                        })
                        .await
                        .unwrap_or_else(|e| {
                            Err(PtyError::SpawnFailed(format!(
                                "activation task failed: {e}"
                            )))
                        });
                        // ★Push regardless of the outcome, even a store refusal★: an activation failure records that entry's "last failure" (ADR-0172 · ADR-0291 E2).
                        broadcast_profile_list(fanout, manager);
                        match started {
                            Ok(()) => reply(sink, request_id, Ok(())),
                            Err(e) => {
                                let _ = sink.enqueue(Outbound::event(activation_error(
                                    conn_id,
                                    "Spawn",
                                    &format!("agent {profile_id} was not started"),
                                    request_id,
                                    e,
                                )));
                            }
                        }
                    }
                    None => reply(
                        sink,
                        request_id,
                        Err(format!("profile not found: {profile_id}")),
                    ),
                }
            }

            AgentCommand::Kill {
                agent_id,
                request_id,
            } => {
                // Off the async worker: `kill_agent` blocks until the transport shuts down, and a blocked worker starves tasks in its unstealable
                // LIFO slot, this connection's writer included (T-79). Awaited, so the reply still follows the kill.
                let mgr = manager.clone();
                let result = tokio::task::spawn_blocking(move || mgr.kill_agent(agent_id))
                    .await
                    .map_err(|e| format!("kill task failed: {e}"))
                    .and_then(|killed| killed.map_err(|e| e.to_string()));
                reply(sink, request_id, result);
            }

            AgentCommand::Interrupt {
                agent_id,
                request_id,
            } => {
                let result = match multiview.check_input(agent_id, conn_id) {
                    LeasePass::Allow => manager.interrupt(agent_id).map_err(|e| e.to_string()),
                    LeasePass::Denied => Err(INPUT_LOCKED_REFUSAL.to_string()),
                };
                reply(sink, request_id, result);
            }

            AgentCommand::WriteStdin {
                agent_id,
                data,
                request_id,
            } => {
                // ADR-0231: input typed by a viewer = human input (`User`) — mid-turn the only entrance that can reach the pending list.
                let result = match multiview.check_input(agent_id, conn_id) {
                    LeasePass::Allow => manager
                        .write_stdin(agent_id, &data, InputOrigin::User)
                        .map_err(|e| e.to_string()),
                    LeasePass::Denied => Err(INPUT_LOCKED_REFUSAL.to_string()),
                };
                reply(sink, request_id, result);
            }

            AgentCommand::Resize {
                agent_id,
                cols,
                rows,
                viewport_id,
            } => {
                let target = match viewport_id {
                    Some(v) => {
                        {
                            let mut owned =
                                owned_viewports.lock().expect("owned_viewports poisoned");
                            if !owned.iter().any(|(a, vid)| *a == agent_id && vid == &v) {
                                owned.push((agent_id, v.clone()));
                            }
                        }
                        multiview
                            .set_viewport(agent_id, v, cols, rows)
                            .unwrap_or((cols, rows))
                    }
                    // No viewport_id (v1 frontend) = bypass negotiation, resize directly to that size.
                    None => (cols, rows),
                };
                if let Err(e) = manager.resize(agent_id, target.0, target.1) {
                    send_error(sink, None, format!("resize failed: {e}"));
                }
            }

            AgentCommand::Subscribe {
                agent_id,
                epoch,
                after_seq,
            } => {
                if self.handle_subscribe(agent_id, epoch, after_seq, subs, sink)
                    == DispatchFlow::Close
                {
                    return DispatchFlow::Close;
                }
            }

            AgentCommand::Unsubscribe { agent_id } => {
                let sink_id = subs.lock().expect("subs poisoned").remove(&agent_id);
                if let Some(sid) = sink_id {
                    let _ = manager.unsubscribe(agent_id, sid);
                }
            }

            AgentCommand::AcquireInput {
                agent_id,
                request_id,
            } => match multiview.acquire(agent_id, conn_id) {
                Ok(true) => {
                    broadcast_lease_changed(fanout, agent_id, true);
                    reply(sink, request_id, Ok(()));
                }
                Ok(false) => reply(sink, request_id, Ok(())),
                Err(()) => reply(
                    sink,
                    request_id,
                    Err("input held by another viewer".to_string()),
                ),
            },

            AgentCommand::ReleaseInput {
                agent_id,
                request_id,
            } => match multiview.release(agent_id, conn_id) {
                Ok(true) => {
                    broadcast_lease_changed(fanout, agent_id, false);
                    reply(sink, request_id, Ok(()));
                }
                Ok(false) => reply(sink, request_id, Ok(())),
                Err(()) => reply(
                    sink,
                    request_id,
                    Err("input lease held by another viewer".to_string()),
                ),
            },

            AgentCommand::ListAgents { request_id } => {
                let _ = sink.enqueue(Outbound::event(AgentEvent::AgentList {
                    request_id,
                    agents: core_agents_to_wire(manager.list_agents()),
                }));
            }

            AgentCommand::StopDaemon {
                force,
                kill_agents,
                request_id,
            } => {
                // ── M4: force policy ──────────────────────────────────────────────────
                // A force=false refusal prevents accidentally killing every live PTY session. Only truly active sessions count (not Exited/Killed/Failed/Exiting).
                let active_count = manager
                    .list_agents()
                    .iter()
                    .filter(|a| matches!(a.status, CoreStatus::Running))
                    .count();
                if !force && active_count > 0 {
                    send_error(
                        sink,
                        Some(request_id),
                        format!(
                            "active agents present ({active_count}); use force=true to stop the daemon"
                        ),
                    );
                    return DispatchFlow::Continue;
                }

                // `kill_agents` is ignored in v1 (always kill): children sit in the daemon's KILL_ON_JOB_CLOSE Job and die with it, so detach is
                // impossible in the current Job model. The flag stays in protocol for a future detach.
                let _ = kill_agents;
                // The shutdown runs in its own task: the network layer (`link::server`) aborts this connection's dispatch task when the client closes,
                // and an awaited `shutdown_all` would never reach `shutdown_tx` — the daemon survived with its agents killed (TRD S21 trd-3-2-u5 §3-4).
                // A second `StopDaemon` only joins. `run()`'s `select!` watches `shutdown_tx`.
                if !self.stopping.swap(true, Ordering::SeqCst) {
                    #[cfg(test)]
                    self.shutdowns_spawned.fetch_add(1, Ordering::SeqCst);
                    let mgr = manager.clone();
                    let shutdown_tx = self.shutdown_tx.clone();
                    tokio::spawn(async move {
                        let _ = tokio::task::spawn_blocking(move || mgr.shutdown_all()).await;
                        let _ = shutdown_tx.send(true);
                    });
                }

                // `Close` flushes queued frames before closing (`link::server` → `close_after_flush`), so this Ack goes out first — still best
                // effort: bounded by the 2 s flush and refused if the outbound queue is full.
                reply(sink, request_id, Ok(()));
                return DispatchFlow::Close;
            }

            // ── profile CRUD + ad-hoc spawn (phase4 stage 1) ───────────────────────────────
            // An unchosen backend is refused, not defaulted: a quiet default would bring up an agent other than the one requested. The wording
            // names the field to fill.
            AgentCommand::SpawnByCwd {
                cwd,
                backend,
                request_id,
                deliver_notices,
            } => {
                let Some(kind) = backend else {
                    reply(sink, request_id, Err(MISSING_BACKEND.to_string()));
                    return DispatchFlow::Continue;
                };
                // No wildcard: a new wire word must stop the build here instead of inheriting a backend unasked.
                let declared = match kind {
                    WireBackendKind::Claude => AgentBackend::Claude,
                    WireBackendKind::Codex => AgentBackend::Codex,
                };
                // The body asks the policy before anything is registered, so a refusal leaves no profile and writes no
                //   last failure (ADR-0172). The gate does not tell callers apart — the person's path sees the same
                //   table (ADR-0279 "accepted cost").
                // ADR-0270 · ADR-0279
                let policy = self.llm_policy;
                let roster = RosterFanout::new(self.fanout.clone(), manager.clone());
                // ★Hand it to the blocking pool★ — this verb also runs the launch-material check (it can wait on the library lock) and the real
                //   process creation synchronously with no yield. The canonical reason is the same comment on the `Spawn` arm above.
                // TRD S25 §5-2 4
                let mgr = manager.clone();
                // ADR-0348
                let joined = tokio::task::spawn_blocking(move || {
                    create_and_start(
                        mgr.as_ref(),
                        &roster,
                        NewAgent {
                            cwd,
                            name: None,
                            backend: declared,
                            policy,
                            delivery: delivery_for(deliver_notices),
                        },
                    )
                })
                .await;
                let event = match joined {
                    Ok(Ok(Activated { info, notices })) => AgentEvent::Spawned {
                        request_id,
                        agent: agent_info_to_wire(&info),
                        notices: notices_to_wire(&notices),
                    },
                    Ok(Err(CreateRefused::Policy { reason })) => AgentEvent::Error {
                        request_id: Some(request_id),
                        message: by_cwd_refusal(kind, reason),
                        launch_blocked: None,
                        created_agent_id: None,
                    },
                    Ok(Err(CreateRefused::NotCreated(e))) => activation_error(
                        conn_id,
                        "SpawnByCwd",
                        "the agent was not created or started",
                        request_id,
                        e,
                    ),
                    // The agent stays registered and asleep — `INTERNAL` (the bus `agent.spawn` code for the same
                    //   end) and its id, so the caller wakes it instead of creating another.
                    // ADR-0348 · TRD S25 step 20 (U3)
                    Ok(Err(CreateRefused::CreatedAsleep { id, name, error })) => {
                        AgentEvent::Error {
                            request_id: Some(request_id),
                            launch_blocked: launch_blocked_args(&error),
                            message: CommandError::internal(created_but_not_started(
                                &name, id, &error,
                            ))
                            .to_string(),
                            created_agent_id: Some(id),
                        }
                    }
                    Err(e) => activation_error(
                        conn_id,
                        "SpawnByCwd",
                        "the agent was not created or started",
                        request_id,
                        PtyError::SpawnFailed(format!("spawn task failed: {e}")),
                    ),
                };
                let _ = sink.enqueue(Outbound::event(event));
            }

            AgentCommand::ListProfiles { request_id } => {
                let _ = sink.enqueue(Outbound::event(AgentEvent::ProfileList {
                    request_id,
                    profiles: core_profiles_to_wire(manager.agent_snapshots()),
                }));
            }

            AgentCommand::CreateProfile {
                name,
                cwd,
                extra_args,
                env,
                auto_restore,
                output_format,
                backend,
                request_id,
            } => {
                let core_output_format = match output_format {
                    WireAgentOutputFormat::Terminal => CoreAgentOutputFormat::Terminal,
                    WireAgentOutputFormat::StreamJson => CoreAgentOutputFormat::StreamJson,
                };
                let Some(command) = spawn_command_for(backend, extra_args, core_output_format)
                else {
                    reply(sink, request_id, Err(MISSING_BACKEND.to_string()));
                    return DispatchFlow::Continue;
                };
                let profile = CoreProfile::new(
                    name,
                    command,
                    std::path::PathBuf::from(cwd),
                    env,
                    auto_restore,
                );
                // ★A version with a caller — if the store does not accept it, do not register and answer an error (no broadcast)★.
                // ADR-0291 R17 (D3)
                match manager.try_create_agent(profile) {
                    Ok(stored) => {
                        let wire = profile_to_wire(&stored);
                        // Reply with one Created — no Ack (prevents a duplicate resolve).
                        let _ = sink.enqueue(Outbound::event(AgentEvent::Created {
                            request_id,
                            profile: wire,
                        }));
                        broadcast_profile_list(fanout, manager);
                    }
                    Err(e) => reply(
                        sink,
                        request_id,
                        Err(profile_error_text(
                            conn_id,
                            "CreateProfile",
                            "the profile was not created",
                            e,
                        )),
                    ),
                }
            }

            AgentCommand::DeleteProfile {
                profile_id,
                request_id,
            } => {
                // Delete cleanup hook (ADR-0116 decision 3): the only profile removal point. Lock-order obligation and gate axis are canonical in
                // `handle_profile_deleted`; decision and cleanup are by profile id, the pre-removal name is for logs only (ADR-0309 decision 6).
                // The trigger (roster absence) is the kernel's DeliveryPort decision; checking here would split it.
                let deleted_name = manager
                    .agent_snapshot(profile_id)
                    .map(|p| p.canonical_name_when_live());
                // ★If the store does not accept it nothing was deleted★: answer an error with no `Ack` · broadcast · cleanup hook, or a live profile's
                // parked mail · contracts would be cleaned.
                // ADR-0291 R17 (D3)
                if let Err(e) = manager.try_delete_agent(profile_id) {
                    let message = profile_error_text(
                        conn_id,
                        "DeleteProfile",
                        &format!("profile {profile_id} was not deleted"),
                        e,
                    );
                    reply(sink, request_id, Err(message));
                    return DispatchFlow::Continue;
                }
                reply(sink, request_id, Ok(()));
                broadcast_profile_list(fanout, manager);
                if let (Some(name), Some(messaging)) = (deleted_name, self.messaging.get()) {
                    // No child stdin blocking write on this path (queue cleanup + ledger transition + a short roster snapshot), so it runs in this
                    // async context without spawn_blocking.
                    let out = messaging.handle_profile_deleted(profile_id, &name);
                    tracing::debug!(
                        profile = %profile_id,
                        // The name is a client-chosen string checked only for non-empty (core `profile.rs`), so shape · length are bounded here
                        // ([`sanitize_for_log`]). The hook above gets the original.
                        name = %sanitize_for_log(&name),
                        skipped_live = out.skipped_live,
                        parked_failed = out.failed_parked,
                        contracts_failed = out.failed_contracts,
                        "profile deleted — running the messaging delete-cleanup hook (ADR-0116 decision 3)"
                    );
                }
            }

            AgentCommand::SpawnProfile {
                profile_id,
                resume,
                request_id,
            } => {
                // Mode derives from whether a session exists (ADR-0076): "agent activation = resume the existing session; for fresh, make a new
                // agent". A stored session means Resume regardless of the wire `resume` flag (the frontend sends false); resume=true is respected even
                // without a session.
                // Without a handle that request does not resume (ADR-0226): for backends resuming by stored handle (claude·codex)
                // `AgentManager::resume_no_fallback` hands it to the new-conversation path and reports `RestoreOutcome::Started` (canonical:
                // `opens_a_new_conversation`).
                match manager.agent_snapshot(profile_id) {
                    Some(profile) => {
                        //   The sid branch asks the backend (ADR-0185) whether the stored sid is resumable by that command; an explicit `resume` does not ride that predicate.
                        let mode = if resume
                            || engram_dashboard_agent::backend::can_resume_profile(&profile)
                        {
                            SpawnMode::Resume
                        } else {
                            SpawnMode::Fresh
                        };
                        // Blocking pool — reason as in the `Spawn` branch. Fresh falls here too: a connection-declaring path waits for the outcome regardless of mode (ADR-0201).
                        // `Withheld`: the reply carries the notices, but the webview does not show them yet, so this
                        //   path must not mark the set as seen for the agent (a later delivering caller still gets it).
                        // ADR-0341
                        // TRD S25 §5-2 16
                        // TODO(TRD S25 §5-2 11): when the webview shows `Spawned.notices`, switch the WS activation
                        //   paths to `Delivered` and make the webview's `SpawnByCwd` senders send
                        //   `deliver_notices: true` — or "once per agent" breaks. (By-cwd already records through the
                        //   one creation body — ADR-0348.)
                        let mgr = manager.clone();
                        let profile_for_task = profile.clone();
                        let started = tokio::task::spawn_blocking(move || {
                            mgr.activate_profile_reporting(
                                &profile_for_task,
                                mode,
                                NoticeDelivery::Withheld,
                            )
                        })
                        .await
                        .unwrap_or_else(|e| {
                            Err(engram_dashboard_agent::types::PtyError::SpawnFailed(
                                format!("activation task failed: {e}"),
                            ))
                        });
                        // ★Push the profile list whichever way the outcome goes★ (ADR-0172/0162): activation clears the entry's "last failure" on success and
                        //   records it on failure (a store refusal too — ADR-0291 E2), and that axis flows only through the profile list. The bus `agent.spawn`
                        //   must do the same (CLAUDE.md 「LLM-우선 제어」). // hangul-ok
                        //   Its pair is `core::agent::commands::{wake_existing, create_and_start}`.
                        match started {
                            Ok(activated) => {
                                let _ = sink.enqueue(Outbound::event(AgentEvent::Spawned {
                                    request_id,
                                    agent: agent_info_to_wire(&activated.info),
                                    notices: notices_to_wire(&activated.notices),
                                }));
                            }
                            Err(e) => {
                                let _ = sink.enqueue(Outbound::event(activation_error(
                                    conn_id,
                                    "SpawnProfile",
                                    &format!("agent {profile_id} was not started"),
                                    request_id,
                                    e,
                                )));
                            }
                        }
                        broadcast_profile_list(fanout, manager);
                    }
                    None => reply(
                        sink,
                        request_id,
                        Err(format!("profile not found: {profile_id}")),
                    ),
                }
            }

            // The three below are versions with a caller — if the store does not accept it nothing changes and an
            //   error is answered with no broadcast.
            // ADR-0291 R17 (D3)
            AgentCommand::SetProfileAutoRestore {
                profile_id,
                auto_restore,
                request_id,
            } => match manager.try_set_agent_auto_restore(profile_id, auto_restore) {
                Ok(true) => {
                    reply(sink, request_id, Ok(()));
                    broadcast_profile_list(fanout, manager);
                }
                Ok(false) => reply(
                    sink,
                    request_id,
                    Err(format!("profile not found: {profile_id}")),
                ),
                Err(e) => reply(
                    sink,
                    request_id,
                    Err(profile_error_text(
                        conn_id,
                        "SetProfileAutoRestore",
                        &format!("the auto-restore flag of profile {profile_id} was not changed"),
                        e,
                    )),
                ),
            },

            AgentCommand::RenameProfile {
                profile_id,
                name,
                request_id,
            } => match manager.try_rename_agent(profile_id, name) {
                Ok(CoreRenameOutcome::Renamed(_)) => {
                    reply(sink, request_id, Ok(()));
                    broadcast_profile_list(fanout, manager);
                    // For a live agent the list name (`AgentInfo.name`) also changes (a dormant profile's list stays as is) — the shell's tab auto
                    //   name · window title read the name only from this list (TRD S26-window-chrome §4-3).
                    broadcast_agent_list(fanout, manager);
                }
                Ok(CoreRenameOutcome::Unchanged(_)) => {
                    reply(sink, request_id, Ok(()));
                    broadcast_profile_list(fanout, manager);
                }
                Ok(CoreRenameOutcome::NotFound) => reply(
                    sink,
                    request_id,
                    Err(format!("profile not found: {profile_id}")),
                ),
                Ok(CoreRenameOutcome::Exhausted) => reply(
                    sink,
                    request_id,
                    Err(format!(
                        "name suffix space exhausted — cannot assign a unique name: {profile_id}"
                    )),
                ),
                Err(e) => reply(
                    sink,
                    request_id,
                    Err(profile_error_text(
                        conn_id,
                        "RenameProfile",
                        &format!("profile {profile_id} was not renamed"),
                        e,
                    )),
                ),
            },

            AgentCommand::ReparentProfile {
                child_id,
                parent_id,
                request_id,
            } => match manager.try_reparent_agent(child_id, parent_id) {
                Ok(true) => {
                    reply(sink, request_id, Ok(()));
                    broadcast_profile_list(fanout, manager);
                }
                Ok(false) => reply(
                    sink,
                    request_id,
                    Err(format!(
                        "reparent rejected (missing/self-parent/cycle/2-level): child={child_id}"
                    )),
                ),
                Err(e) => reply(
                    sink,
                    request_id,
                    Err(profile_error_text(
                        conn_id,
                        "ReparentProfile",
                        &format!("profile {child_id} was not moved"),
                        e,
                    )),
                ),
            },

            AgentCommand::GetSnapshot {
                agent_id,
                request_id,
            } => {
                // Reply with one Snapshot — no separate Ack (prevents a duplicate resolve).
                match manager.get_snapshot(agent_id) {
                    Ok(chunks) => {
                        let _ = sink.enqueue(Outbound::event(AgentEvent::Snapshot {
                            request_id,
                            agent_id,
                            chunks: chunks.iter().map(snapshot_chunk_to_wire).collect(),
                        }));
                    }
                    Err(e) => reply(sink, request_id, Err(e.to_string())),
                }
            }

            AgentCommand::SetEnvelopeFormat { format, request_id } => {
                // No broadcast — global state is observed on the next message, not a list push target.
                let core_format = match format {
                    WireEnvelopeFormat::Colon => CoreEnvelopeFormat::Colon,
                    WireEnvelopeFormat::Xml => CoreEnvelopeFormat::Xml,
                };
                self.control_registry.set_envelope_format(core_format);
                reply(sink, request_id, Ok(()));
            }

            // ── command bus registration wire (ADR-0155/0156, TRD §3-7) ──────────────────────────────
            //
            // ★No `_ =>`★: this exhaustive match gives a compile error per new variant, the only point asking "was the wiring forgotten".
            AgentCommand::RegisterCommands {
                owner: claimed,
                decls,
                catalog_version,
                request_id,
            } => {
                // ★Do not revert `attached_owner` to `session.owner_token()` — no test catches it★: today both give the same log, and a diverging
                // state (stored ≠ derived) cannot be made at this layer (the roster table is private to `command_roster`). Reverting would
                // quietly let impersonation through in slice B (cause = the `note_claimed_owner` comment; the three tests calling that function
                // directly bite the decision).
                // ADR-0154
                note_claimed_owner(
                    session,
                    &owner_from_wire(claimed),
                    self.commands.attached_owner(conn_id).as_ref(),
                );
                let decls = match decls_from_wire(&decls) {
                    Ok(decls) => decls,
                    Err(e) => {
                        reply_roster(
                            sink,
                            conn_id,
                            "register",
                            request_id,
                            Err(broken_decls_refusal(&e)),
                        );
                        return DispatchFlow::Continue;
                    }
                };
                // ★Do not refuse by `catalog_version`★: the generation number is per crate, so comparing it with the receiver's own is wrong —
                // it is for diagnostics (TRD §4-①).
                tracing::debug!(
                    conn = conn_id,
                    catalog_version,
                    names = decls.len(),
                    "command registration"
                );
                reply_roster(
                    sink,
                    conn_id,
                    "register",
                    request_id,
                    self.refuse_names_i_answer(&decls)
                        .and_then(|()| self.commands.register(conn_id, decls)),
                );
            }

            AgentCommand::UpdateCommands {
                owner: claimed,
                added,
                removed,
                request_id,
            } => {
                // Do not revert `attached_owner` to `session.owner_token()` (see the same note in the registration arm).
                // ADR-0154
                note_claimed_owner(
                    session,
                    &owner_from_wire(claimed),
                    self.commands.attached_owner(conn_id).as_ref(),
                );
                let added = match decls_from_wire(&added) {
                    Ok(added) => added,
                    Err(e) => {
                        reply_roster(
                            sink,
                            conn_id,
                            "update",
                            request_id,
                            Err(broken_decls_refusal(&e)),
                        );
                        return DispatchFlow::Continue;
                    }
                };
                tracing::debug!(
                    conn = conn_id,
                    added = added.len(),
                    removed = removed.len(),
                    "command diff"
                );
                reply_roster(
                    sink,
                    conn_id,
                    "update",
                    request_id,
                    self.refuse_names_i_answer(&added)
                        .and_then(|()| self.commands.update(conn_id, added, removed)),
                );
            }

            AgentCommand::ListCommands { request_id } => {
                // The owner token is not passed down — the declarer is the owner, so the grade field is gone (TRD §3-7 amendment ㉠).
                //
                // The rule for merging the two sources rides the same function as the HTTP discovery route (`/control/commands`); writing it
                // back here would split the two surfaces' lists (canonical: `control::catalog::merge`).
                let entries =
                    crate::control::catalog::merge(self.locals.decls(), self.commands.entries());
                let _ = sink.enqueue(Outbound::event(AgentEvent::CommandList {
                    request_id,
                    entries,
                }));
            }

            // The reply is not given here: after the round trip the delivery task sends it to the original connection (`command_delivery::deliver`)
            // through the frame outlet the roster holds (same single-writer queue). `sink` is used only for the broken-envelope refusal below.
            // ★Waiting for the round trip here is forbidden★: a connection's frames are handled one at a time, so when the owner is this connection
            // itself the `CommandOutcome` would sit in the inbound queue behind this command and never be taken out (self-deadlock, ADR-0081
            // decision 3 amendment). Splitting socket reads from handling does not remove it (one handling lane — [`dispatch_order`]), and moving
            // this to [`DispatchOrder::Detached`] would make classification do delivery's job (TRD §3-6).
            // ADR-0154
            AgentCommand::Command { envelope } => {
                // ADR-0272 decision 6: a broken envelope means the number cannot be read — answer like the parse-failure branch
                //   (`Error{request_id: None}`) and keep the connection alive.
                let envelope = match envelope_from_wire(&envelope) {
                    Ok(envelope) => envelope,
                    Err(e) => {
                        tracing::warn!(conn = conn_id, "cannot unpack the command envelope: {e}");
                        let _ = sink.enqueue(Outbound::event(AgentEvent::Error {
                            request_id: None,
                            message: format!("invalid command envelope: {e}"),
                            launch_blocked: None,
                            created_agent_id: None,
                        }));
                        return DispatchFlow::Continue;
                    }
                };
                tracing::debug!(
                    conn = conn_id,
                    request_id = %envelope.request_id,
                    name = %sanitize_for_log(&envelope.name),
                    "command request — looking up the owner in the roster and delivering"
                );
                tokio::spawn(crate::command_delivery::deliver(
                    self.commands.clone(),
                    self.deliveries.clone(),
                    Arc::clone(&self.locals),
                    conn_id,
                    envelope,
                    crate::command_delivery::ENTRANCE_SOCKET,
                ));
            }

            // Attaches the owner's outcome to its round trip; the delivery task then sends the reply to the asking connection (`command_delivery`).
            // ★With no place to attach it, still do not stay silent★, or the sender burns its whole deadline: answer `Error{request_id: None}` — this frame
            //   has no correlation key of a request I issued (`command_request_id` gives `None`), and `CommandReply` would send that key's reply twice (TRD §4-⑤).
            // The two causes (a late outcome whose seat was reaped, an outcome nobody asked for) are not told apart: the table keeps no trace of
            //   passed keys (a growing list — why ADR-0150 dropped roster traces).
            // ADR-0154
            AgentCommand::CommandOutcome { reply } => {
                // ADR-0272 decision 6: a broken reply cannot be attached to any round trip — same disposition as the broken-envelope branch
                //   (`Error{request_id: None}` · connection lives); that round trip answers the caller at its own deadline.
                let reply = match reply_from_wire(&reply) {
                    Ok(reply) => reply,
                    Err(e) => {
                        tracing::warn!(
                            conn = conn_id,
                            "cannot unpack the reply of the command outcome: {e}"
                        );
                        let _ = sink.enqueue(Outbound::event(AgentEvent::Error {
                            request_id: None,
                            message: format!("invalid command reply: {e}"),
                            launch_blocked: None,
                            created_agent_id: None,
                        }));
                        return DispatchFlow::Continue;
                    }
                };
                let request_id = reply.request_id;
                // The two causes differ in weight: "no place to attach" is a normal race (deadline passed first); "never delegated" is an impossible
                //   frame that would drown in the former's noise.
                let refusal = match self.deliveries.complete(reply) {
                    OutcomeLanding::Attached => {
                        tracing::debug!(conn = conn_id, %request_id, "attached the command outcome to its round trip");
                        return DispatchFlow::Continue;
                    }
                    OutcomeLanding::NoSeat => "no round trip is waiting for this outcome — it may have passed its deadline, or nothing asked for it",
                    OutcomeLanding::NotDelegated => "this daemon answers that command itself, so it never handed it to anyone and takes no outcome for it",
                };
                // Only the first is a warn, the rest debug (latch: `stray_outcome_warned` doc).
                if !session.stray_outcome_warned.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        conn = conn_id,
                        %request_id,
                        refusal,
                        "command outcome refused (logged once per connection)"
                    );
                } else {
                    tracing::debug!(conn = conn_id, %request_id, refusal, "command outcome refused (repeat)");
                }
                let _ = sink.enqueue(Outbound::event(AgentEvent::Error {
                    request_id: None,
                    message: refusal.into(),
                    launch_blocked: None,
                    created_agent_id: None,
                }));
            }

            // Both branches call the same manager verbs as the bus `agent.listQueuedInputs` · `agent.cancelQueuedInput`; a failure puts the
            //   error code at the head of the `Error` wording (`CODE: wording`). A list lookup is a read, so it skips the input lease.
            // ADR-0231
            AgentCommand::ListQueuedInputs {
                agent_id,
                request_id,
            } => match manager.list_queued_inputs(agent_id) {
                Ok(listing) => {
                    let _ = sink.enqueue(Outbound::event(queued_listing_to_wire(
                        request_id, agent_id, listing,
                    )));
                }
                Err(e) => send_error(sink, Some(request_id), list_error_text(agent_id, e)),
            },

            // The lease check is the same as `WriteStdin`: a non-holder is refused with the same wording and never reaches the manager.
            // ADR-0231
            AgentCommand::CancelQueuedInput {
                agent_id,
                input_id,
                request_id,
            } => match multiview.check_input(agent_id, conn_id) {
                LeasePass::Denied => {
                    send_error(sink, Some(request_id), INPUT_LOCKED_REFUSAL.to_string())
                }
                LeasePass::Allow => match manager.cancel_queued_input(agent_id, &input_id) {
                    Ok(outcome) => {
                        let _ = sink.enqueue(Outbound::event(AgentEvent::QueuedInputCancelReply {
                            request_id,
                            agent_id,
                            input_id,
                            outcome: outcome.as_str().to_string(),
                        }));
                    }
                    Err(e) => send_error(
                        sink,
                        Some(request_id),
                        cancel_error_text(agent_id, &input_id, e),
                    ),
                },
            },

            // ── usage limits (TRD S21 usage-limit-slot §1-6) ───────────────────────────────────
            // The first sheet is sent by the service straight out this connection's outlet — it is not this line's answer.
            AgentCommand::UsageSubscribe { vendors } => {
                let vendors = vendors.into_iter().filter_map(usage_vendor_key).collect();
                self.usage.replace_subscription(conn_id, vendors);
            }

            // ★No entry is an `Error`, not an `Ack`★: a quiet `Ack` would be read by the shell as "looked up" and it would wait forever for a value.
            AgentCommand::RefreshUsageLimits { vendor, request_id } => {
                let answered = match usage_vendor_key(vendor) {
                    Some(key) => self
                        .usage
                        .request(key, RequestKind::Refresh)
                        .await
                        .is_some(),
                    None => false,
                };
                if answered {
                    reply(sink, request_id, Ok(()));
                } else {
                    send_error(sink, Some(request_id), no_usage_text(vendor));
                }
            }
        }
        DispatchFlow::Continue
    }

    /// ★Do not predict SubscribeAck from get_snapshot★: an evict between the predicted snapshot and the one subscribe_from replays would
    /// make Ack.replay_from/latest diverge from the first sent seq without the client noticing the loss. Every Ack field comes from
    /// the single snapshot outcome of subscribe_from.
    ///
    /// Two planes: the sink handed to subscribe_from (`agent_conn::FrameOutputSink`) is the output frame plane; only control
    /// (Ack/ReplayComplete/Error) goes through dispatch's `OutboundSink`. Both join the same frame outlet (one single-writer queue per
    /// connection), so FIFO holds — R1 hangs on this.
    ///
    /// A returned `Close` = close this connection; it arises only when `ReplayComplete` could not be queued.
    fn handle_subscribe(
        &self,
        agent_id: AgentId,
        requested_epoch: Option<u32>,
        after_seq: Option<u64>,
        subs: &Arc<Mutex<HashMap<AgentId, SinkId>>>,
        sink: &dyn OutboundSink,
    ) -> DispatchFlow {
        let manager = &self.manager;

        let (out_sink, replay_dropped) = sink.make_output_sink();

        // The incarnation marker of the Ack and `ReplayComplete`, and the resume marker of the Ack, come only from the `subscribe_from`
        //   response: a separate lookup lets the incarnation change before the replay, so the marker would name a different incarnation.
        // An enqueue failure is swallowed: control is small, and if the queue is full the replay using it is blocked and caught as truncated.
        // ADR-0226
        let on_ready = |reply: &SubscribeReply| {
            let outcome = &reply.outcome;
            let _ = sink.enqueue(Outbound::event(AgentEvent::SubscribeAck {
                agent_id,
                action: kind_to_action(outcome.kind),
                current_epoch: reply.incarnation.epoch,
                oldest_seq: outcome.oldest_seq,
                latest_seq: outcome.latest_seq,
                replay_from: outcome.replay_from,
                truncated: outcome.kind == ReplayKind::Truncated,
                continues_conversation: reply.incarnation.continues_conversation,
            }));
        };

        let reply = match manager.subscribe_from(
            agent_id,
            out_sink,
            after_seq,
            requested_epoch,
            on_ready,
        ) {
            Ok(r) => r,
            Err(e) => {
                // A nonexistent agent comes here — `subscribe_from` does not call `on_ready` when the session lookup fails, so no Ack goes out
                //   (contract of `AgentManager::subscribe_from`, pinned by core test `subscribe_from_err_never_invokes_on_ready`).
                // ★The refusal goes out as `SubscribeFailed`, not `Error` (load-bearing)★: `Subscribe` has no request_id, so `Error{request_id: None}`
                //   cannot identify the owner and the client's single-flight slot stayed a zombie — that agent's Subscribe never went out again. This
                //   point returns before the Ack, so "no Ack/Complete follows a refusal" holds and the client relies on it (see the AgentEvent docs).
                refuse_subscribe(sink, agent_id, format!("subscribe failed: {e}"));
                return DispatchFlow::Continue;
            }
        };
        let outcome = reply.outcome;

        let old = subs
            .lock()
            .expect("subs poisoned")
            .insert(agent_id, outcome.sink_id);
        if let Some(old_sid) = old {
            if old_sid != outcome.sink_id {
                let _ = manager.unsubscribe(agent_id, old_sid);
            }
        }

        // The kind-based truncated already went out in the Ack; this notifies only kind != Truncated with an actual drop during the
        //   synchronous replay send.
        if outcome.kind != ReplayKind::Truncated && replay_dropped.load(Ordering::Acquire) {
            send_error(
                sink,
                None,
                format!("replay truncated for agent {agent_id}: output dropped during replay; please refresh"),
            );
        }

        // ★This one cannot slip — if it cannot be queued the connection closes (load-bearing)★: after the Ack the client's single-flight slot is
        //   `acked`, and only `ReplayComplete` or a disconnect releases it (decisions of the shell's `daemon_client::replay_flight`). Slipping
        //   would block that agent's replay until the connection drops. Closing forces the disconnect and the client re-requests everything.
        // The queue can fill because Ack → replay frames → this event share one single-writer queue. Cost: the connection's other agents'
        //   subscriptions drop together — accepted, since keeping it alive would be a silent permanent block with no signal.
        if sink
            .enqueue(Outbound::event(AgentEvent::ReplayComplete {
                agent_id,
                epoch: reply.incarnation.epoch,
            }))
            .is_err()
        {
            tracing::error!(
                %agent_id,
                "could not put ReplayComplete in the queue (saturated) — closing the connection (otherwise that agent's replay is permanently blocked)"
            );
            return DispatchFlow::Close;
        }
        DispatchFlow::Continue
    }
}

/// The owner token a registration packet writes is only an advertisement, not authority — the roster's owner is the value stored when
/// that connection attached (`actual`), sourced only from [`CommandRoster::attached_owner`].
///
/// ★Do not derive `actual` here★: against a derived value, once the stored value leaves the derivation an honest client is caught while
/// impersonation passes (a connection writing the old derived form `conn-<id>` slips by the format branch; ADR-0154). `None` (not
/// attached) has nothing to compare and goes to the format branch.
/// It does not refuse: the client cannot know its token (not on the wire), so refusing on mismatch would block every normal
/// registration; the mismatch itself is not raised either. Only a client writing our token format (`conn-<id>`, which arises only
/// inside the daemon) is raised, one line per connection with its own latch (not shared with the debug branch, or one harmless packet
/// would lock out the later warning). A blank field is not a claim. The printed length is truncated (unverified client string, no frame size cap).
// ADR-0155
fn note_claimed_owner(
    session: &ConnectionSession,
    claimed: &OwnerToken,
    actual: Option<&OwnerToken>,
) {
    if claimed.as_str().is_empty() || Some(claimed) == actual {
        return;
    }
    let conn_id = session.conn_id;
    if claimed
        .as_str()
        .starts_with(CommandRoster::OWNER_TOKEN_PREFIX)
    {
        if !session.claimed_owner_warned.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                conn = conn_id,
                claimed = %sanitize_for_log(claimed.as_str()),
                "registration wrote another connection's owner token — ignoring it and putting it up with this connection's token (logged once per connection)"
            );
        }
    } else {
        tracing::debug!(
            conn = conn_id,
            claimed = %sanitize_for_log(claimed.as_str()),
            "the owner token of a registration packet is not used"
        );
    }
}

/// Trims a client-given string for a log — both length and shape.
///
/// Length: the wire has no frame size cap; cuts at a character boundary. Shape: escapes control characters — the log layer uses the
/// Display value as is, so a newline in a name would split a line and forge an entry. Range: `char::is_control` (C0/C1); U+2028 passes
/// and the fmt layer does not split on it.
/// The width fits one piece of a name; a place carrying more calls [`sanitize_within`] with its own width. ★Do not widen this one★.
///
/// Scope is this crate's log fields; do not widen this to "everything" (three earlier editions did and each had counterexamples):
/// - Guards: an unverified peer's string in a log field — the socket layer (this file), delivery (`command_delivery::deliver`), roster
///   disconnect (`logged_names` of `command_roster`), the control routes (`control::mcp_server` · `control::registry`), the messaging host.
/// - Exception 1, values the daemon made (like the `conn-<id>` owner token, `previous_owner` of `command_roster::CommandRoster::attach`):
///   that place carries a trigger — the commit changing the stored value must fix that line too.
/// - Exception 2, typed values (`Uuid` · `ErrorCode` · `http::Method` · `Path`): the type carries the shape.
/// - Exception 3, outside this crate: the `Origin` · `Host` of a refused WS upgrade are masked by the transport (`AcceptRefusal`, logged by `link::server`).
/// - Not covered: the `: {e}` interpolation at the end of a message string (the logging convention allows only that).
///
/// `control::agent::preview` is for response wording only and lets control characters through.
pub(crate) fn sanitize_for_log(text: &str) -> String {
    const MAX_CHARS: usize = 64;
    sanitize_within(text, MAX_CHARS)
}

/// The branch where the caller decides the width of [`sanitize_for_log`]; trimming rules are the same.
pub(crate) fn sanitize_within(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in text.chars().take(max_chars) {
        if ch.is_control() {
            out.extend(ch.escape_debug());
        } else {
            out.push(ch);
        }
    }
    if text.chars().nth(max_chars).is_some() {
        out.push('…');
    }
    out
}

/// The fanout port requires a non-blocking implementation, so it is safe to call from any context (pump/cleanup).
pub(crate) fn broadcast_lease_changed(fanout: &dyn FrameFanout, agent_id: AgentId, held: bool) {
    let ev = AgentEvent::InputLeaseChanged { agent_id, held };
    if let Some(text) = event_json(&ev) {
        fanout.broadcast_text(text);
    }
}

/// The real adapter of the roster notification port the `control/agent` route calls (ADR-0132).
///
/// It lives here because which event goes in which wire shape is knowledge this module owns; `control/` knows only "the roster changed"
/// (ADR-0236 dropped the ADR-0130 leaf property).
/// ★No mismatched pair★: the constructor binds only a fanout and a manager **from one assembly** — the production
///   construction points are `DaemonWiring::roster_broadcast` (the rule in that struct's comment) and the WS
///   `SpawnByCwd` arm, which builds one from the core's own `fanout` + `manager` (the same assembly).
/// One notification = `ProfileListUpdated` then `AgentListUpdated`: the port does not know which verb called, and the agent list must
/// always go so a bus · CLI `agent.rename` of a live agent reaches the shell's tab auto name (TRD S26-window-chrome §4-3).
pub struct RosterFanout {
    fanout: Arc<dyn FrameFanout>,
    manager: Arc<AgentManager>,
}

impl RosterFanout {
    pub fn new(fanout: Arc<dyn FrameFanout>, manager: Arc<AgentManager>) -> Self {
        Self { fanout, manager }
    }
}

impl crate::control::agent::RosterBroadcast for RosterFanout {
    fn roster_changed(&self) {
        broadcast_profile_list(self.fanout.as_ref(), &self.manager);
        broadcast_agent_list(self.fanout.as_ref(), &self.manager);
    }
}

/// The agent crate's roster port — the same two frames as [`crate::control::agent::RosterBroadcast`].
// ADR-0348
// Named by path, not imported — two `roster_changed` methods in scope would make a plain `.roster_changed()` ambiguous.
impl engram_dashboard_agent::commands::RosterChanged for RosterFanout {
    fn roster_changed(&self) {
        crate::control::agent::RosterBroadcast::roster_changed(self);
    }
}

fn broadcast_profile_list(fanout: &dyn FrameFanout, manager: &Arc<AgentManager>) {
    let ev = AgentEvent::ProfileListUpdated {
        profiles: core_profiles_to_wire(manager.agent_snapshots()),
    };
    if let Some(text) = event_json(&ev) {
        fanout.broadcast_text(text);
    }
}

fn broadcast_agent_list(fanout: &dyn FrameFanout, manager: &Arc<AgentManager>) {
    if let Some(text) = event_json(&agent_list_event(manager)) {
        fanout.broadcast_text(text);
    }
}

/// Emits the `Subscribe` refusal notice ([`AgentEvent::SubscribeFailed`]) — only caller: the `Err` branch of `subscribe_from` in
/// [`ConnectionCore::handle_subscribe`].
/// ★Do not add a pre-lookup refusal asking separately whether the agent exists★: the incarnation marker would come from a different
/// lookup than the replay (ADR-0226). ★Do not swallow an enqueue failure★: this frame is the only signal releasing the client's
/// single-flight slot; if saturation blocks it, that agent's Subscribe never goes out again until the adapter cuts the connection (R6).
fn refuse_subscribe(sink: &dyn OutboundSink, agent_id: AgentId, reason: String) {
    tracing::debug!(agent = %agent_id, %reason, "Subscribe refused");
    if let Err(e) = sink.enqueue(Outbound::event(AgentEvent::SubscribeFailed {
        agent_id,
        reason,
    })) {
        tracing::warn!(
            agent = %agent_id,
            "could not enqueue the Subscribe refusal notice — the client cannot release its subscription slot: {e}"
        );
    }
}

fn send_error(
    sink: &dyn OutboundSink,
    request_id: Option<engram_dashboard_protocol::RequestId>,
    message: String,
) {
    let _ = sink.enqueue(Outbound::event(AgentEvent::Error {
        request_id,
        message,
        launch_blocked: None,
        created_agent_id: None,
    }));
}

pub fn hello_event(daemon_version: String) -> AgentEvent {
    AgentEvent::Hello {
        protocol_version: PROTOCOL_VERSION,
        daemon_version,
        capabilities: None,
    }
}

pub fn agent_list_event(manager: &Arc<AgentManager>) -> AgentEvent {
    AgentEvent::AgentListUpdated {
        agents: core_agents_to_wire(manager.list_agents()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn_port as frame_port;
    use engram_dashboard_protocol::{CommandListEntry, RequestId};
    use std::sync::atomic::AtomicU8;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    fn rid() -> RequestId {
        RequestId(uuid::Uuid::new_v4())
    }

    /// A wire word (lowercase) and a bus word (capitalized) hit one entry — the key is the probe's key, not the incoming spelling.
    /// The bus side uses the function the real `usage.*` calls (agent `usage_vendor_of`).
    #[test]
    fn wire_and_bus_words_reach_the_same_usage_key() {
        use engram_dashboard_agent::commands::{usage_vendor_of, AgentBackend};
        for (wire, bus) in [
            (WireBackendKind::Claude, AgentBackend::Claude),
            (WireBackendKind::Codex, AgentBackend::Codex),
        ] {
            let from_wire = usage_vendor_key(wire).expect("every wire vendor has a probe");
            let from_bus = usage_vendor_of(&bus).expect("every bus word has a probe");
            assert_eq!(from_wire, from_bus, "{wire:?}");
        }
    }

    /// Decision table of [`inbound_lane`] — what skips a full lane.
    ///
    /// `Interrupt` being `Ordered` is the core assertion: "it's a cancel so it skips" would break ordering group ① (`WriteStdin`(X) ↔
    /// `Interrupt`(X)); the reason is in the [`inbound_lane`] comment, here it is only pinned.
    #[test]
    fn only_kill_jumps_the_queue_when_it_is_saturated() {
        let agent_id = uuid::Uuid::new_v4();
        assert_eq!(
            inbound_lane(&AgentCommand::Kill {
                agent_id,
                request_id: rid()
            }),
            InboundLane::Bypass,
            "Kill is the only handle that actually unblocks a blocked lane"
        );
        let ordered = vec![
            AgentCommand::Interrupt {
                agent_id,
                request_id: rid(),
            },
            AgentCommand::WriteStdin {
                agent_id,
                data: "x".into(),
                request_id: rid(),
            },
            AgentCommand::StopDaemon {
                force: true,
                kill_agents: true,
                request_id: rid(),
            },
            AgentCommand::ListAgents { request_id: rid() },
            AgentCommand::Spawn {
                profile_id: uuid::Uuid::new_v4(),
                request_id: rid(),
            },
            AgentCommand::UsageSubscribe { vendors: vec![] },
            AgentCommand::RefreshUsageLimits {
                vendor: WireBackendKind::Codex,
                request_id: rid(),
            },
        ];
        for cmd in &ordered {
            assert_eq!(
                inbound_lane(cmd),
                InboundLane::Ordered,
                "if this skips the lane the ordering group breaks: {cmd:?}"
            );
        }
    }

    /// Decision table of [`dispatch_order`] — a wrong classification shows only in production, so both sides are pinned. The detached
    /// side must be exhaustive (growth there is the dangerous direction); the in-lane side uses one representative per ordering group,
    /// since an omitted variant is caught by `dispatch_order`'s exhaustive match.
    #[test]
    fn only_the_activations_and_the_usage_refresh_leave_the_arrival_queue() {
        let detached = vec![
            AgentCommand::Spawn {
                profile_id: uuid::Uuid::new_v4(),
                request_id: rid(),
            },
            AgentCommand::SpawnProfile {
                profile_id: uuid::Uuid::new_v4(),
                resume: false,
                request_id: rid(),
            },
            AgentCommand::SpawnByCwd {
                cwd: "C:/tmp".into(),
                backend: None,
                request_id: rid(),
                deliver_notices: false,
            },
            AgentCommand::RefreshUsageLimits {
                vendor: WireBackendKind::Claude,
                request_id: rid(),
            },
        ];
        for cmd in &detached {
            assert_eq!(
                dispatch_order(cmd),
                DispatchOrder::Detached,
                "a command that waits to the end must be outside the lane: {cmd:?}"
            );
        }

        let agent_id = uuid::Uuid::new_v4();
        let in_order = vec![
            // ① input to the same agent
            AgentCommand::WriteStdin {
                agent_id,
                data: "x".into(),
                request_id: rid(),
            },
            AgentCommand::Interrupt {
                agent_id,
                request_id: rid(),
            },
            AgentCommand::CancelQueuedInput {
                agent_id,
                input_id: "q1".into(),
                request_id: rid(),
            },
            // A pending-list lookup also stays in the lane — it must not read ahead of earlier input · cancels of the same connection.
            AgentCommand::ListQueuedInputs {
                agent_id,
                request_id: rid(),
            },
            // ② input lease
            AgentCommand::AcquireInput {
                agent_id,
                request_id: rid(),
            },
            AgentCommand::ReleaseInput {
                agent_id,
                request_id: rid(),
            },
            // ③ subscription
            AgentCommand::Subscribe {
                agent_id,
                epoch: None,
                after_seq: None,
            },
            AgentCommand::Unsubscribe { agent_id },
            // ④ command roster
            AgentCommand::RegisterCommands {
                owner: WireOwnerToken::new("shell"),
                decls: RawValue::from_string("[]".to_string()).unwrap(),
                catalog_version: 1,
                request_id: rid(),
            },
            AgentCommand::UpdateCommands {
                owner: WireOwnerToken::new("shell"),
                added: RawValue::from_string("[]".to_string()).unwrap(),
                removed: vec![],
                request_id: rid(),
            },
            // ⑤ the command that ends the connection
            AgentCommand::StopDaemon {
                force: true,
                kill_agents: true,
                request_id: rid(),
            },
            // A usage subscription — a full replacement, so the last must win.
            AgentCommand::UsageSubscribe {
                vendors: vec![WireBackendKind::Claude],
            },
        ];
        for cmd in &in_order {
            assert_eq!(
                dispatch_order(cmd),
                DispatchOrder::InOrder,
                "a command whose order has meaning must stay in the lane: {cmd:?}"
            );
        }
    }

    /// Do not call activation directly on an async worker — it ties up that worker for up to 20 seconds.
    ///
    /// `AgentManager::activate_profile` blocks until an outcome on a connection-declaring transport (backstop 15 s) and on a failure verdict
    /// runs the teardown `session.kill` (pump join bounded by 5 s), with no yield point. Checked from source because worker exhaustion
    /// shows only under load and this repo avoids clock assertions. The bus sibling is covered by `blocking_handler`.
    /// Blind spots: `include_str!` is one file, so a third activation place in another daemon file is out of range; the window is 5 lines,
    /// so a handover further above is caught only by axis 1 (the `mgr` receiver).
    #[test]
    fn both_ws_activation_sites_hand_the_blocking_call_to_the_pool() {
        let src = include_str!("connection_core.rs");
        let production = src.split("mod tests {").next().expect("production section");

        let lines: Vec<&str> = production.lines().collect();
        let call_sites: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                (l.contains(".activate_profile(") || l.contains(".activate_profile_reporting("))
                    && !l.trim_start().starts_with("//")
            })
            .map(|(i, _)| i)
            .collect();
        assert!(
            !call_sites.is_empty(),
            "the activation call place is gone — the premise of this item is stale"
        );
        // Two axes, since each alone leaks: (1) the call receiver is the pool-moved copy (`mgr`) — catches a bare call at any distance; (2) a
        //   handover within a few lines above — catches a disguise that only renamed to `mgr`. No count: an unrelated `spawn_blocking` would false-red.
        for i in call_sites {
            let line = lines[i];
            assert!(
                line.contains("mgr.activate_profile"),
                "activation is called directly on an async worker — that worker is tied up for up to 20 seconds, and N blocked activations take N workers and stall other connections' commands too: {line}"
            );
            let window = lines[i.saturating_sub(4)..=i].join("\n");
            assert!(
                window.contains("spawn_blocking"),
                "only the receiver has the pool-copy name and there is no handover — the name is right but it still runs on an async worker:\n{window}"
            );
        }
    }

    /// ★The ad-hoc create's start (`SpawnByCwd`) is not called on an async worker either★ — the launch-material check
    /// can wait on the library lock and the real process spawn runs without yielding (TRD S25 §5-2 4). Why it is
    /// measured from the source, the two axes and what it cannot see are the same as the activation sibling above.
    /// The call is the one creation body (`create_and_start(`, ADR-0348), and it waits for the codex link verdict too.
    #[test]
    fn the_by_cwd_spawn_hands_the_blocking_call_to_the_pool() {
        let src = include_str!("connection_core.rs");
        let production = src.split("mod tests {").next().expect("production section");

        let lines: Vec<&str> = production.lines().collect();
        let call_sites: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                (l.contains("create_and_start(") || l.contains(".spawn_agent("))
                    && !l.trim_start().starts_with("//")
            })
            .map(|(i, _)| i)
            .collect();
        assert!(
            !call_sites.is_empty(),
            "the ad-hoc create call site is gone — this item's premise is stale"
        );
        for i in call_sites {
            let line = lines[i];
            assert!(
                line.trim_start().starts_with("create_and_start("),
                "the by-cwd create does not go through the one creation body: {line}"
            );
            let window = lines[i.saturating_sub(4)..=(i + 1).min(lines.len() - 1)].join("\n");
            assert!(
                window.contains("spawn_blocking") && window.contains("mgr.as_ref()"),
                "only the receiver has the pool-copy name and there is no handover — the name is right but it still runs on an async worker:\n{window}"
            );
        }
    }

    /// The output plane is not mocked — it flows to the real frame outlet (conn_tx), riding the same single-writer queue as control so FIFO order is observable.
    struct MockOutboundSink {
        events: Arc<StdMutex<Vec<AgentEvent>>>,
        conn_tx: tokio::sync::mpsc::Sender<frame_port::Frame>,
    }

    impl MockOutboundSink {
        fn new(conn_tx: tokio::sync::mpsc::Sender<frame_port::Frame>) -> Self {
            Self {
                events: Arc::new(StdMutex::new(Vec::new())),
                conn_tx,
            }
        }
        fn events(&self) -> Vec<AgentEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    impl OutboundSink for MockOutboundSink {
        fn enqueue(&self, out: Outbound) -> Result<(), SinkError> {
            match out {
                Outbound::Event(ev) => {
                    if let Some(text) = event_json(&ev) {
                        let _ = self.conn_tx.try_send(frame_port::Frame::Text(text));
                    }
                    self.events.lock().unwrap().push(*ev);
                    Ok(())
                }
                Outbound::Binary(b) => {
                    let _ = self.conn_tx.try_send(frame_port::Frame::Binary(b));
                    Ok(())
                }
                Outbound::Close(r) => {
                    let _ = self.conn_tx.try_send(frame_port::Frame::Close(r));
                    Ok(())
                }
            }
        }
        fn make_output_sink(&self) -> (Arc<dyn OutputSink>, Arc<AtomicBool>) {
            let sink = Arc::new(crate::agent_conn::FrameOutputSink::new(Arc::new(
                crate::test_doubles::FakeFrameSink::new(self.conn_tx.clone()),
            )));
            let flag = sink.replay_dropped_flag();
            (sink, flag)
        }
    }

    /// The flush trigger is not plugged in on purpose: production empties the parking on appearance (MessagingFlushSink), but this test needs
    ///   parked items to remain after the spawn to verify the delete cleanup kills them. The diff wiring is guarded by the messaging_host.rs · control_send tests.
    fn test_core_with_messaging() -> (
        ConnectionCore,
        Arc<engram_dashboard_messaging::service::MessagingService>,
    ) {
        let (core, _rx) = test_core();
        let messaging = Arc::new(crate::messaging_host::messaging_for_manager(
            core.manager.clone(),
            core.control_registry.clone(),
        ));
        core.messaging.set(messaging.clone());
        (core, messaging)
    }

    fn test_core() -> (ConnectionCore, watch::Receiver<bool>) {
        let (core, rx, _fanout) = test_core_with(CommandDeliveries::new());
        (core, rx)
    }

    /// Also returns the fanout record — for tests measuring the broadcast.
    fn test_core_recording() -> (ConnectionCore, Arc<crate::test_doubles::RecordingFanout>) {
        let (core, _rx, fanout) = test_core_with(CommandDeliveries::new());
        (core, fanout)
    }

    /// Plugs the correlation table in from outside — a test of the deadline branch needs a clock pushed by hand.
    fn test_core_with(
        deliveries: CommandDeliveries,
    ) -> (
        ConnectionCore,
        watch::Receiver<bool>,
        Arc<crate::test_doubles::RecordingFanout>,
    ) {
        test_core_built(deliveries, &|_, _| {
            Arc::new(crate::command_delivery::NoLocalCommands)
        })
    }

    /// An assembly where the daemon holds its own `agent.*` table (from `control::commands::make_daemon_table`, the production assembler),
    /// so delivery stage 1 and discovery's own part really run.
    fn test_core_with_own_agent_table() -> (ConnectionCore, watch::Receiver<bool>) {
        // The fanout record is not verified here, so it is dropped (see `test_core_recording`). The lease port is the core's `MultiViewState`,
        // as in the production assembly (`lib.rs`).
        let (core, rx, _fanout) =
            test_core_built(CommandDeliveries::new(), &|manager, multiview| {
                let slot = Arc::new(crate::control::mcp_server::CommandTableSlot::new());
                slot.set(Arc::new(crate::control::commands::make_daemon_table(
                    manager.clone(),
                    Arc::new(crate::control::mcp_server::RosterBroadcastSlot::new()),
                    Arc::new(multiview.clone()),
                    Arc::new(crate::control::commands::NoUsageLimits),
                    Arc::new(crate::control::commands::NoLibrary),
                )));
                Arc::new(crate::control::commands::DaemonLocalCommands::new(slot))
            });
        (core, rx)
    }

    /// An assembly that fills the table later — slot injection can follow the connection factory assembly, and a registration in that window
    /// is kept in the roster as is. The one state where an overlap can really occur.
    fn test_core_with_late_agent_table() -> (
        ConnectionCore,
        watch::Receiver<bool>,
        Arc<crate::control::mcp_server::CommandTableSlot>,
    ) {
        let slot = Arc::new(crate::control::mcp_server::CommandTableSlot::new());
        let for_locals = slot.clone();
        // The fanout record is not what this branch verifies so it is dropped (same reason as the sibling above).
        let (core, rx, _fanout) = test_core_built(CommandDeliveries::new(), &move |_, _| {
            Arc::new(crate::control::commands::DaemonLocalCommands::new(
                for_locals.clone(),
            ))
        });
        (core, rx, slot)
    }

    /// The assembly body the three branches above share — only the way the own-part `agent.*` table is plugged in differs.
    ///
    /// Returns the fanout record as the third value too: `test_core_recording` has no other way to hold that handle.
    fn test_core_built(
        deliveries: CommandDeliveries,
        locals: &dyn Fn(&Arc<AgentManager>, &MultiViewState) -> Arc<dyn LocalCommands>,
    ) -> (
        ConnectionCore,
        watch::Receiver<bool>,
        Arc<crate::test_doubles::RecordingFanout>,
    ) {
        test_core_on_stores(deliveries, locals, Arc::default(), None)
    }

    /// The store switch of [`test_core_refusable`] — accept · refuse on load (new version) · write failure; a failure leaves nothing either way.
    const STORE_SAVES: u8 = 0;
    const STORE_REFUSES: u8 = 1;
    const STORE_FAILS_IO: u8 = 2;

    /// An assembly that changes the store outcome — the switch ([`STORE_SAVES`] · [`STORE_REFUSES`] · [`STORE_FAILS_IO`]) hangs on the
    /// profile store. Returns the fanout record and the switch.
    // ADR-0291 R17
    fn test_core_refusable() -> (
        ConnectionCore,
        Arc<crate::test_doubles::RecordingFanout>,
        Arc<AtomicU8>,
    ) {
        let refusing = Arc::new(AtomicU8::new(STORE_SAVES));
        let (core, _rx, fanout) = test_core_on_stores(
            CommandDeliveries::new(),
            &|_, _| Arc::new(crate::command_delivery::NoLocalCommands),
            refusing.clone(),
            None,
        );
        (core, fanout, refusing)
    }

    /// A core whose manager checks launches with `kits` (the running daemon plugs none yet, and the Noop source
    /// neither blocks nor carries notices), and its fanout record.
    fn test_core_with_kits(
        kits: Arc<dyn engram_dashboard_agent::launch::LaunchKitSource>,
    ) -> (ConnectionCore, Arc<crate::test_doubles::RecordingFanout>) {
        let (core, _rx, fanout) = test_core_on_stores(
            CommandDeliveries::new(),
            &|_, _| Arc::new(crate::command_delivery::NoLocalCommands),
            Arc::default(),
            Some(kits),
        );
        (core, fanout)
    }

    /// The body of [`test_core_built`] — while `refusing` is not [`STORE_SAVES`] the profile store fails to save.
    /// `kits` = the manager's launch-kit source (`None` = the Noop default).
    fn test_core_on_stores(
        deliveries: CommandDeliveries,
        locals: &dyn Fn(&Arc<AgentManager>, &MultiViewState) -> Arc<dyn LocalCommands>,
        refusing: Arc<AtomicU8>,
        kits: Option<Arc<dyn engram_dashboard_agent::launch::LaunchKitSource>>,
    ) -> (
        ConnectionCore,
        watch::Receiver<bool>,
        Arc<crate::test_doubles::RecordingFanout>,
    ) {
        use engram_dashboard_agent::profile::{ProfileRegistry, ProfileStore, Refusal};
        use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};

        fn refused(switch: &AtomicU8) -> Result<(), StoreError> {
            match switch.load(Ordering::SeqCst) {
                STORE_REFUSES => Err(StoreError::ReadOnly(Refusal::Newer { found: 2 })),
                STORE_FAILS_IO => Err(StoreError::Io(std::io::Error::other("disk full"))),
                _ => Ok(()),
            }
        }

        #[derive(Default)]
        struct MemStore {
            saved: StdMutex<Vec<engram_dashboard_agent::profile::AgentProfile>>,
            refusing: Arc<AtomicU8>,
        }
        impl ProfileStore for MemStore {
            fn save(
                &self,
                p: &[engram_dashboard_agent::profile::AgentProfile],
            ) -> Result<(), StoreError> {
                refused(&self.refusing)?;
                *self.saved.lock().unwrap() = p.to_vec();
                Ok(())
            }
            fn load(&self) -> Vec<engram_dashboard_agent::profile::AgentProfile> {
                self.saved.lock().unwrap().clone()
            }
        }

        let recording = Arc::new(crate::test_doubles::RecordingFanout::new());
        let fanout: Arc<dyn FrameFanout> = recording.clone();
        let store: Arc<dyn ProfileStore> = Arc::new(MemStore {
            refusing,
            ..MemStore::default()
        });
        let status_sink = Arc::new(crate::status_fanout::DaemonStatusSink::new(fanout.clone()));
        let profiles = Arc::new(ProfileRegistry::new(store));
        let tracker = Arc::new(SessionTracker::new(
            TrackerConfig::default(),
            Arc::new(|_aid, _sid| {}),
        ));
        let mut manager = AgentManager::new(status_sink, profiles, tracker);
        if let Some(kits) = kits {
            manager = manager.with_launch_ports(engram_dashboard_agent::launch::LaunchPorts {
                kits,
                ..engram_dashboard_agent::launch::LaunchPorts::noop()
            });
        }
        let manager = Arc::new(manager);
        let manager_for_locals = manager.clone();
        let multiview = MultiViewState::new();
        let locals = locals(&manager_for_locals, &multiview);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let control_registry = Arc::new(ControlRegistry::new());
        let core = ConnectionCore::new(
            manager,
            multiview,
            fanout,
            control_registry,
            Arc::new(crate::control::mcp_server::MessagingSlot::new()),
            CommandRoster::new(),
            deliveries,
            locals,
            crate::usage_service::fakes::idle_service(),
            shutdown_tx,
        );
        (core, shutdown_rx, recording)
    }

    /// A human click (WS) and the LLM (command bus) shake the same thing — the WS half (the bus half: two tests in `core::agent::commands`).
    /// Activation changes the entry's "last failure" (ADR-0172), which flows only through the profile list broadcast. Measured on the
    /// failure branch: deterministic without a real claude, and there the notification is the only thing reaching the screen.
    // ADR-0172
    #[tokio::test]
    async fn ws_spawn_profile_announces_the_profile_list_even_when_activation_fails() {
        let (core, fanout) = test_core_recording();
        let profile = engram_dashboard_agent::profile::AgentProfile::new(
            "ws-parity".into(),
            engram_dashboard_agent::profile::AgentCommand::Shell {
                // A nonexistent executable — activation fails deterministically.
                program: format!("engram-not-a-real-program-{}.exe", uuid::Uuid::new_v4()),
                args: vec![],
            },
            std::env::temp_dir(),
            vec![],
            false,
        );
        let created = core
            .manager
            .try_create_agent(profile)
            .expect("registration");
        let before = fanout.texts().len();

        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        core.dispatch(
            AgentCommand::SpawnProfile {
                profile_id: created.id,
                resume: false,
                request_id: engram_dashboard_protocol::RequestId(uuid::Uuid::new_v4()),
            },
            &session,
            &mock,
        )
        .await;

        let announced = fanout.texts()[before..]
            .iter()
            .any(|t| t.contains("ProfileListUpdated"));
        assert!(
            announced,
            "the activation outcome must go out via the profile list — without it a failure launched via WS does not show on screen: {:?}",
            fanout.texts()
        );
    }

    /// Do not derive Resume from a stored sid alone (ADR-0185) — bus counterpart:
    /// `waking_does_not_resume_a_profile_whose_backend_cannot_resume` in `engram_dashboard_agent::commands`.
    /// The wire has no "did it resume" field, so the mode is told apart by outcome: with a program that ends immediately, Fresh emits
    /// `Spawned`, while Resume has `resume_no_fallback` see the exit in the early-exit window and fail it, so no `Spawned`.
    // ADR-0185
    #[tokio::test]
    async fn ws_spawn_profile_does_not_resume_a_profile_whose_backend_cannot_resume() {
        let (core, _rx) = test_core();
        let mut profile = engram_dashboard_agent::profile::AgentProfile::new(
            "ws-no-resume".into(),
            engram_dashboard_agent::profile::AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec!["/c".into(), "exit".into()],
            },
            std::env::temp_dir(),
            vec![],
            false,
        );
        // A sid remaining on a command that cannot resume — a hand-edited profile file makes it today.
        profile.backend_session_id = Some(uuid::Uuid::new_v4());
        let created = core
            .manager
            .try_create_agent(profile)
            .expect("registration");
        assert!(
            core.manager
                .agent_snapshot(created.id)
                .and_then(|p| p.backend_session_id)
                .is_some(),
            "premise: the sid must be planted for this item to measure that branch"
        );

        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        core.dispatch(
            AgentCommand::SpawnProfile {
                profile_id: created.id,
                resume: false,
                request_id: engram_dashboard_protocol::RequestId(uuid::Uuid::new_v4()),
            },
            &session,
            &mock,
        )
        .await;

        assert!(
            mock.events()
                .iter()
                .any(|e| matches!(e, AgentEvent::Spawned { .. })),
            "even with a sid, a command that cannot resume must come up Fresh: {:?}",
            mock.events()
        );
        let _ = core.manager.kill_agent(created.id);
    }

    /// `resume: true` deliberately bypasses that predicate — this asymmetry is pinned (same program · sid · command, differing only by the
    /// flag). It does not justify the bypass (that decision is open): a session "harmonizing" the rule and breaking this is changing a decision.
    // ADR-0185
    #[tokio::test]
    async fn ws_spawn_profile_still_honours_an_explicit_resume_request() {
        let (core, _rx) = test_core();
        let mut profile = engram_dashboard_agent::profile::AgentProfile::new(
            "ws-explicit-resume".into(),
            engram_dashboard_agent::profile::AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec!["/c".into(), "exit".into()],
            },
            std::env::temp_dir(),
            vec![],
            false,
        );
        profile.backend_session_id = Some(uuid::Uuid::new_v4());
        let created = core
            .manager
            .try_create_agent(profile)
            .expect("registration");

        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        core.dispatch(
            AgentCommand::SpawnProfile {
                profile_id: created.id,
                resume: true,
                request_id: engram_dashboard_protocol::RequestId(uuid::Uuid::new_v4()),
            },
            &session,
            &mock,
        )
        .await;

        // Evidence of Resume: the early-exit window sees the exit and fails the activation (Fresh emits `Spawned`). "No Spawned" alone also holds
        //   for unrelated reasons, so it also checks the failure came from the resume branch.
        let events = mock.events();
        assert!(
            events.iter().any(|e| matches!(
                e,
                AgentEvent::Error { message, .. } if message.contains("resume")
            )),
            "an explicit resume request was blocked by the resume axis — that bypass is intended so changing it needs a decision first: {events:?}"
        );
        let _ = core.manager.kill_agent(created.id);
    }

    fn test_core_with_control_registry() -> (ConnectionCore, Arc<ControlRegistry>) {
        let (core, _rx) = test_core();
        let control_registry = core.control_registry.clone();
        (core, control_registry)
    }

    // ── R1: Subscribe ────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn subscribe_emits_ack_then_replay_then_complete_in_order() {
        let (core, _rx) = test_core();
        let profile = engram_dashboard_agent::profile::AgentProfile::new(
            "t".into(),
            engram_dashboard_agent::profile::AgentCommand::Shell {
                program: default_shell().to_string(),
                args: vec![],
            },
            std::env::temp_dir(),
            vec![],
            false,
        );
        let info = core
            .manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("spawn")
            .into_started()
            .expect("this call really launches (not a duplicate request)");
        let agent_id = info.id;
        let mut waited = 0;
        loop {
            if let Ok(chunks) = core.manager.get_snapshot(agent_id) {
                if !chunks.is_empty() {
                    break;
                }
            }
            if waited > 50 {
                break; // Even with no output the Ack/Complete order is verified.
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
            waited += 1;
        }

        let (tx, mut conn_rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(4608);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);

        core.dispatch(
            AgentCommand::Subscribe {
                agent_id,
                epoch: None,
                after_seq: None,
            },
            &session,
            &mock,
        )
        .await;

        let mut items = Vec::new();
        while let Ok(item) = conn_rx.try_recv() {
            items.push(item);
        }
        assert!(
            items.len() >= 2,
            "at least Ack+ReplayComplete: {}",
            items.len()
        );
        match &items[0] {
            frame_port::Frame::Text(s) => {
                assert!(s.contains("SubscribeAck"), "the 1st is SubscribeAck: {s}")
            }
            other => panic!("the 1st must be Text(SubscribeAck): {other:?}"),
        }
        match items.last().unwrap() {
            frame_port::Frame::Text(s) => {
                assert!(
                    s.contains("ReplayComplete"),
                    "the last is ReplayComplete: {s}"
                )
            }
            other => panic!("the last must be Text(ReplayComplete): {other:?}"),
        }
        for mid in &items[1..items.len() - 1] {
            assert!(
                matches!(mid, frame_port::Frame::Binary(_)),
                "only replay Binary between Ack and ReplayComplete: {mid:?}"
            );
        }

        let evs = mock.events();
        assert!(
            matches!(evs.first(), Some(AgentEvent::SubscribeAck { .. })),
            "control first event=SubscribeAck"
        );
        assert!(
            matches!(evs.last(), Some(AgentEvent::ReplayComplete { .. })),
            "control last=ReplayComplete"
        );

        let _ = core.manager.kill_agent(agent_id);
    }

    // Regression targeted: slipping `ReplayComplete` leaves the client's acked slot unreleasable (replay blocked until disconnect); if it
    //   cannot be queued the connection must close.
    #[tokio::test]
    async fn subscribe_closes_the_connection_when_replay_complete_cannot_be_queued() {
        // Pass Ack · replay and drop only ReplayComplete by queue saturation (the production order: replay fills the single-writer queue).
        struct DropsReplayComplete(MockOutboundSink);
        impl OutboundSink for DropsReplayComplete {
            fn enqueue(&self, out: Outbound) -> Result<(), SinkError> {
                if matches!(&out, Outbound::Event(ev) if matches!(**ev, AgentEvent::ReplayComplete { .. }))
                {
                    return Err(SinkError);
                }
                self.0.enqueue(out)
            }
            fn make_output_sink(&self) -> (Arc<dyn OutputSink>, Arc<AtomicBool>) {
                self.0.make_output_sink()
            }
        }

        let (core, _rx) = test_core();
        let profile = engram_dashboard_agent::profile::AgentProfile::new(
            "t".into(),
            engram_dashboard_agent::profile::AgentCommand::Shell {
                program: default_shell().to_string(),
                args: vec![],
            },
            std::env::temp_dir(),
            vec![],
            false,
        );
        let info = core
            .manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("spawn")
            .into_started()
            .expect("this call really launches (not a duplicate request)");
        let agent_id = info.id;

        let (tx, _conn_rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(4608);
        let sink = DropsReplayComplete(MockOutboundSink::new(tx));
        let session = ConnectionSession::new(1);

        let flow = core
            .dispatch(
                AgentCommand::Subscribe {
                    agent_id,
                    epoch: None,
                    after_seq: None,
                },
                &session,
                &sink,
            )
            .await;
        assert_eq!(
            flow,
            DispatchFlow::Close,
            "if ReplayComplete cannot be put in the connection must close (otherwise that agent's replay is permanently blocked)"
        );
        assert!(
            matches!(
                sink.0.events().first(),
                Some(AgentEvent::SubscribeAck { .. })
            ),
            "premise: the Ack went out (so the client slot is acked)"
        );

        let _ = core.manager.kill_agent(agent_id);
    }

    // ── Subscribe: a nonexistent agent ─────────────────────────────────────────────────────
    // The refusal carries the `agent_id` (regression target): an ownerless `Error{request_id:None}` left the client's single-flight slot a
    //   zombie. The absence of an Ack is pinned too (the client's immediate release relies on it).
    #[tokio::test]
    async fn subscribe_unknown_agent_emits_subscribe_failed_no_ack() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let missing = uuid::Uuid::new_v4();
        core.dispatch(
            AgentCommand::Subscribe {
                agent_id: missing,
                epoch: None,
                after_seq: None,
            },
            &session,
            &mock,
        )
        .await;
        let evs = mock.events();
        assert_eq!(evs.len(), 1, "only 1 refusal");
        match &evs[0] {
            AgentEvent::SubscribeFailed { agent_id, .. } => assert_eq!(*agent_id, missing),
            other => panic!("must be SubscribeFailed: {other:?}"),
        }
    }

    // ── Subscribe: the refusal notice body ────────────────────────────────────────────────────
    // The dispatch case above sees only the variant and owner; here the body runs directly to pin the notice content (agent_id · reason).
    #[test]
    fn refuse_subscribe_carries_agent_id_and_reason() {
        let (tx, _rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let agent_id = uuid::Uuid::new_v4();
        refuse_subscribe(&mock, agent_id, "subscribe failed: agent not found".into());
        match mock.events().as_slice() {
            [AgentEvent::SubscribeFailed {
                agent_id: got,
                reason,
            }] => {
                assert_eq!(
                    *got, agent_id,
                    "the refusal must carry the owner (no correlation = a zombie)"
                );
                assert!(
                    reason.contains("subscribe failed"),
                    "the reason is conveyed: {reason}"
                );
            }
            other => panic!("must be 1 SubscribeFailed: {other:?}"),
        }
    }

    // Even with a saturated queue and an unsendable refusal, dispatch must survive (no panic · unwrap). The log is not asserted (no
    //   subscriber-capture harness).
    #[test]
    fn refuse_subscribe_survives_full_queue() {
        struct FullSink;
        impl OutboundSink for FullSink {
            fn enqueue(&self, _out: Outbound) -> Result<(), SinkError> {
                Err(SinkError)
            }
            fn make_output_sink(&self) -> (Arc<dyn OutputSink>, Arc<AtomicBool>) {
                unreachable!("this test runs only the enqueue")
            }
        }
        refuse_subscribe(&FullSink, uuid::Uuid::new_v4(), "full".into());
    }

    /// The incarnation marker of the Ack and `ReplayComplete` comes only from the one `subscribe_from` response. Checked from source: the
    /// window is an incarnation swap between two lookups, not reproducible through dispatch. Pair: core `subscribe_from_reports_the_incarnation_it_replayed_from`.
    // ADR-0226
    #[test]
    fn handle_subscribe_takes_the_incarnation_only_from_the_subscribe_reply() {
        // Align to LF to find the column-0 `}` boundary below regardless of the checkout's line endings (CRLF).
        let src = include_str!("connection_core.rs").replace("\r\n", "\n");
        let production = src.split("mod tests {").next().expect("production section");
        assert_eq!(
            production.matches("fn handle_subscribe(").count(),
            1,
            "`handle_subscribe` must be exactly one in the production section for this item to pin down the body"
        );
        let rest = production
            .split("fn handle_subscribe(")
            .nth(1)
            .expect("the body of `handle_subscribe`");
        // This method is the last of `impl ConnectionCore`, so the body runs to the first column-0 `}` closing the impl.
        let body = &rest[..rest.find("\n}\n").expect("the brace closing the impl")];
        assert!(
            !body.contains("fn "),
            "the cut-out chunk contains another function — the boundary overflowed"
        );
        assert!(
            !body.contains("agent_epoch("),
            "the marker is looked up separately — it can say a different incarnation from the replay: {body}"
        );
        assert_eq!(
            body.matches("reply.incarnation.epoch").count(),
            2,
            "both the Ack and `ReplayComplete` places must use the response's marker: {body}"
        );
        assert_eq!(
            body.matches("continues_conversation: reply.incarnation.continues_conversation")
                .count(),
            1,
            "the Ack's resume marker must also come from the response: {body}"
        );
    }

    /// The Ack's resume marker carries as is whether the spawn carried a handle to resume with. Measured with a shell: on Resume, spawn
    /// reads the roster's handle regardless of backend and the shell discards it, so a true-marker incarnation stands up without a real claude·codex.
    // ADR-0226
    #[tokio::test]
    async fn subscribe_ack_carries_whether_the_incarnation_continues_a_conversation() {
        let (core, _rx) = test_core();

        // (the Ack's resume marker, the Ack's incarnation marker, the Complete's incarnation marker, the incarnation markers of the drained binary frames)
        async fn subscribe_once(
            core: &ConnectionCore,
            agent_id: AgentId,
        ) -> (bool, u32, u32, Vec<u32>) {
            // Wait until the shell prints something — otherwise the replay is empty and the frame marker assertion below spins on nothing.
            let mut printed = false;
            for _ in 0..125 {
                if core
                    .manager
                    .get_snapshot(agent_id)
                    .is_ok_and(|chunks| !chunks.is_empty())
                {
                    printed = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
            assert!(
                printed,
                "the shell output did not reach the replay ring within 5 seconds — a test premise failure (shell startup delay or output path), not a subscription replay defect"
            );
            let (tx, mut conn_rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(4608);
            let mock = MockOutboundSink::new(tx);
            core.dispatch(
                AgentCommand::Subscribe {
                    agent_id,
                    epoch: None,
                    after_seq: None,
                },
                &ConnectionSession::new(1),
                &mock,
            )
            .await;
            let evs = mock.events();
            let (continues, ack_epoch) = match evs.first() {
                Some(AgentEvent::SubscribeAck {
                    continues_conversation,
                    current_epoch,
                    ..
                }) => (*continues_conversation, *current_epoch),
                other => panic!("the first event must be SubscribeAck: {other:?}"),
            };
            let complete_epoch = match evs.last() {
                Some(AgentEvent::ReplayComplete { epoch, .. }) => *epoch,
                other => panic!("the last event must be ReplayComplete: {other:?}"),
            };
            // The drained binary frames may include live frames that entered the replay meanwhile; they must carry that incarnation's marker too.
            let mut drained_epochs = Vec::new();
            while let Ok(frame) = conn_rx.try_recv() {
                if let frame_port::Frame::Binary(bytes) = frame {
                    let decoded = engram_dashboard_protocol::decode_frame(&bytes)
                        .expect("the output frame is read with the codec");
                    drained_epochs.push(decoded.epoch);
                }
            }
            (continues, ack_epoch, complete_epoch, drained_epochs)
        }

        let shell = || {
            engram_dashboard_agent::profile::AgentProfile::new(
                "t".into(),
                engram_dashboard_agent::profile::AgentCommand::Shell {
                    program: default_shell().to_string(),
                    args: vec![],
                },
                std::env::temp_dir(),
                vec![],
                false,
            )
        };

        let fresh = core
            .manager
            .spawn_agent(&shell(), SpawnMode::Fresh)
            .expect("spawn")
            .into_started()
            .expect("this call really launches (not a duplicate request)");
        let mut resumable = shell();
        resumable.backend_session_id = Some(uuid::Uuid::new_v4());
        let resumed = core
            .manager
            .spawn_agent(&resumable, SpawnMode::Resume)
            .expect("spawn")
            .into_started()
            .expect("this call really launches (not a duplicate request)");

        for (spawned, expect_continues) in [(&fresh, false), (&resumed, true)] {
            let (continues, ack_epoch, complete_epoch, drained_epochs) =
                subscribe_once(&core, spawned.id).await;
            assert_eq!(
                continues, expect_continues,
                "marker = whether the spawn carried a handle (Fresh false · Resume with a handle true)"
            );
            assert_eq!(
                ack_epoch, spawned.epoch,
                "the Ack names the spawned incarnation"
            );
            assert_eq!(
                complete_epoch, spawned.epoch,
                "Complete is the same incarnation"
            );
            assert!(
                !drained_epochs.is_empty(),
                "the shell output is in the snapshot so at least one replay frame must come"
            );
            assert!(
                drained_epochs.iter().all(|e| *e == spawned.epoch),
                "the drained output frames are the same incarnation: {drained_epochs:?}"
            );
        }

        let _ = core.manager.kill_agent(fresh.id);
        let _ = core.manager.kill_agent(resumed.id);
    }

    // ── Spawn: a nonexistent profile ──────────────────────────────────────────────────────
    #[tokio::test]
    async fn spawn_missing_profile_errors() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let req = rid();
        core.dispatch(
            AgentCommand::Spawn {
                profile_id: uuid::Uuid::new_v4(),
                request_id: req,
            },
            &session,
            &mock,
        )
        .await;
        let evs = mock.events();
        match evs.as_slice() {
            [AgentEvent::Error {
                request_id: Some(r),
                ..
            }] => assert_eq!(*r, req, "the request_id is attached to the Error"),
            other => panic!("expected 1 Error(request_id): {other:?}"),
        }
    }

    // ── ReparentProfile: refused (false) (ADR-0072) ──────────────────────────────────
    //    The mock sink is not behind the fanout port, so the absence of an Ack corroborates that the broadcast branch was skipped.
    #[tokio::test]
    async fn reparent_rejected_emits_error_no_ack_no_broadcast() {
        let (core, _rx) = test_core();
        let child = engram_dashboard_agent::profile::AgentProfile::new(
            "c".into(),
            engram_dashboard_agent::profile::AgentCommand::Shell {
                program: default_shell().to_string(),
                args: vec![],
            },
            std::env::temp_dir(),
            vec![],
            false,
        );
        let cid = child.id;
        core.manager
            .try_create_agent(child)
            .expect("registration succeeded");

        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let req = rid();
        core.dispatch(
            AgentCommand::ReparentProfile {
                child_id: cid,
                parent_id: Some(uuid::Uuid::new_v4()), // a nonexistent parent → refused.
                request_id: req,
            },
            &session,
            &mock,
        )
        .await;

        match mock.events().as_slice() {
            [AgentEvent::Error {
                request_id: Some(r),
                ..
            }] => assert_eq!(*r, req, "the request_id is attached to the refusal Error"),
            other => panic!("expected only 1 Error on refusal (no Ack/broadcast): {other:?}"),
        }
        assert_eq!(
            core.manager.agent_snapshot(cid).unwrap().parent_id,
            None,
            "a refused reparent must not change the state"
        );
    }

    // ── Kill: a nonexistent agent ─────────────────────────────────────────────────────────
    #[tokio::test]
    async fn kill_unknown_agent_errors() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        core.dispatch(
            AgentCommand::Kill {
                agent_id: uuid::Uuid::new_v4(),
                request_id: rid(),
            },
            &session,
            &mock,
        )
        .await;
        assert!(
            matches!(mock.events().as_slice(), [AgentEvent::Error { .. }]),
            "killing a nonexistent agent is an Error"
        );
    }

    // ── WriteStdin: the lease held by another conn ─────────────────────────────────────────
    #[tokio::test]
    async fn write_stdin_denied_when_lease_held_by_other() {
        let (core, _rx) = test_core();
        let agent_id = uuid::Uuid::new_v4();
        let _ = core.multiview.acquire(agent_id, 2);
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        core.dispatch(
            AgentCommand::WriteStdin {
                agent_id,
                data: b"x".to_vec(),
                request_id: rid(),
            },
            &session,
            &mock,
        )
        .await;
        match mock.events().as_slice() {
            [AgentEvent::Error { message, .. }] => {
                assert!(
                    message.contains("input locked"),
                    "lease refusal message: {message}"
                )
            }
            other => panic!("expected a Denied Error: {other:?}"),
        }
    }

    // ── the two WS commands for the pending input list (ADR-0231) ───────────────────────────────────────
    mod queued_seam {
        use std::sync::atomic::AtomicU8;
        use std::sync::Arc;

        use engram_dashboard_agent::backend::InputEncoder;
        use engram_dashboard_agent::manager::AgentManager;
        use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
        use engram_dashboard_agent::session::AgentSession;
        use engram_dashboard_agent::transport::AgentTransport;
        use engram_dashboard_agent::types::{
            AgentId, AgentInfo, AgentStatus, BackendCaps, ControlCaps, InputCaps, InputEvent,
            ModelCaps, OutputCaps, OutputEvent, PtyError, QueuedInputEvent, SessionCaps,
            StatusSink, TransportCaps,
        };

        struct NoopStatus;
        impl StatusSink for NoopStatus {
            fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        /// A channel that answers only ids it holds as acceptance unknown — writes are swallowed.
        struct SeamTransport {
            unconfirmed: Vec<String>,
            /// Runs inside `shutdown()` — lets a test hold the kill path on whichever thread runs it.
            on_shutdown: Option<Box<dyn Fn() + Send + Sync>>,
        }
        impl AgentTransport for SeamTransport {
            fn start(&self, _core: Arc<OutputCore>) {}
            fn send_input(&self, _input: InputEvent) -> Result<(), PtyError> {
                Ok(())
            }
            fn unconfirmed_inputs(&self) -> Vec<String> {
                self.unconfirmed.clone()
            }
            fn resize(&self, _c: u16, _r: u16) -> Result<(), PtyError> {
                Ok(())
            }
            fn interrupt(&self) -> Result<(), PtyError> {
                Ok(())
            }
            fn shutdown(&self) {
                if let Some(on_shutdown) = &self.on_shutdown {
                    on_shutdown();
                }
            }
            fn capabilities(&self) -> TransportCaps {
                TransportCaps {
                    input: InputCaps {
                        raw: true,
                        message: false,
                        attachment: false,
                    },
                    output: OutputCaps {
                        terminal_bytes: false,
                        structured: true,
                        markdown: false,
                        tool_events: false,
                        usage: false,
                    },
                    control: ControlCaps {
                        resize: false,
                        interrupt: false,
                        cancel: false,
                        graceful_shutdown: false,
                    },
                }
            }
        }

        /// Plugs in a live session whose roster is filled with `events`. The mid-turn input policy is `None` (no public seam carries a policy
        /// on an injected session), so cancelling is always NOT_FOUND like every backend today.
        pub(super) fn insert(
            manager: &Arc<AgentManager>,
            epoch: u32,
            events: Vec<QueuedInputEvent>,
            unconfirmed: &[&str],
        ) -> AgentId {
            insert_seam(
                manager,
                epoch,
                events,
                SeamTransport {
                    unconfirmed: unconfirmed.iter().map(|id| id.to_string()).collect(),
                    on_shutdown: None,
                },
            )
        }

        /// A live session whose transport runs `on_shutdown` when it is shut down (killed).
        pub(super) fn insert_with_shutdown(
            manager: &Arc<AgentManager>,
            on_shutdown: impl Fn() + Send + Sync + 'static,
        ) -> AgentId {
            insert_seam(
                manager,
                0,
                Vec::new(),
                SeamTransport {
                    unconfirmed: Vec::new(),
                    on_shutdown: Some(Box::new(on_shutdown)),
                },
            )
        }

        fn insert_seam(
            manager: &Arc<AgentManager>,
            epoch: u32,
            events: Vec<QueuedInputEvent>,
            transport: SeamTransport,
        ) -> AgentId {
            let id = AgentId::new_v4();
            let core = Arc::new(OutputCore::new(
                id,
                epoch,
                Arc::new(NoopStatus),
                TurnWiring::detached(),
            ));
            for event in events {
                core.emit(OutputEvent::QueuedInput(event));
            }
            let session = Arc::new(AgentSession::new(
                id,
                std::path::PathBuf::from("seam-root"),
                epoch,
                80,
                24,
                Arc::new(AtomicU8::new(0)),
                BackendCaps {
                    session: SessionCaps {
                        resume: true,
                        snapshot: false,
                        cwd_env: true,
                    },
                    model: ModelCaps {
                        select: false,
                        temperature: false,
                        max_tokens: false,
                    },
                },
                InputEncoder::ClaudeStreamJson,
                true,
                core,
                Box::new(transport),
            ));
            manager.insert_test_session(session);
            id
        }
    }

    async fn dispatch_one(
        core: &ConnectionCore,
        conn_id: ConnId,
        cmd: AgentCommand,
    ) -> Vec<AgentEvent> {
        let (tx, _rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(conn_id);
        core.dispatch(cmd, &session, &mock).await;
        mock.events()
    }

    /// A kill must not hold the async worker running the connection's dispatch (T-79): other tasks on that runtime must run during shutdown.
    /// The fake shutdown signals "entered" and waits up to 2 s for a release only the canary task sends; an inline kill on a current-thread
    /// runtime would block the canary until the timeout. The canary waits for "entered" first, so `join!` poll order does not matter.
    #[tokio::test]
    async fn a_kill_in_progress_leaves_the_runtime_free() {
        use engram_dashboard_protocol::RequestId;
        use std::sync::atomic::AtomicBool;
        use std::sync::mpsc::{self, TryRecvError};

        let (core, _rx) = test_core();
        let (entered_tx, entered_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = StdMutex::new(release_rx);
        let released = Arc::new(AtomicBool::new(false));
        let released_in_shutdown = released.clone();
        let agent_id = queued_seam::insert_with_shutdown(&core.manager, move || {
            let _ = entered_tx.send(());
            let got = release_rx
                .lock()
                .expect("release poisoned")
                .recv_timeout(Duration::from_secs(2));
            released_in_shutdown.store(got.is_ok(), Ordering::SeqCst);
        });

        let canary = async move {
            loop {
                match entered_rx.try_recv() {
                    Ok(()) => break,
                    Err(TryRecvError::Empty) => tokio::time::sleep(Duration::from_millis(1)).await,
                    Err(TryRecvError::Disconnected) => panic!("the kill never reached shutdown"),
                }
            }
            let _ = release_tx.send(());
        };
        let req = RequestId(uuid::Uuid::new_v4());
        let ((events, released_at_reply), ()) = tokio::join!(
            async {
                let events = dispatch_one(
                    &core,
                    1,
                    AgentCommand::Kill {
                        agent_id,
                        request_id: req,
                    },
                )
                .await;
                (events, released.load(Ordering::SeqCst))
            },
            canary
        );

        assert!(
            released_at_reply,
            "the kill had not been released when its reply returned — Kill must run off the async worker and \
             reply only after the kill finished"
        );
        match events.as_slice() {
            [AgentEvent::Ack { request_id }] => assert_eq!(*request_id, req),
            other => panic!("the kill must be answered with its Ack: {other:?}"),
        }
    }

    /// A list lookup does not look at the lease — an agent whose lease another holds is read as is. The response carries the roster's reduced
    /// state in full (the two fields of a row waiting to be cancelled · the acceptance-unknown mark on `Queued` rows · `as_of_seq` ·
    /// the incarnation marker · the turn observation table's pause field).
    #[tokio::test]
    async fn list_queued_inputs_skips_the_lease_and_carries_the_whole_listing() {
        use engram_dashboard_agent::turn::{TurnEndKind, TurnSignal};
        use engram_dashboard_agent::types::QueuedInputEvent as Q;

        let (core, _rx) = test_core();
        let agent_id = queued_seam::insert(
            &core.manager,
            7,
            vec![
                Q::Queued {
                    id: "q1".into(),
                    text: "first".into(),
                },
                Q::Queued {
                    id: "q2".into(),
                    text: "second".into(),
                },
                Q::Queued {
                    id: "q3".into(),
                    text: "third".into(),
                },
                Q::CancelRequested { id: "q3".into() },
                Q::CancelAnswered {
                    id: "q3".into(),
                    removed: false,
                },
            ],
            &["q2", "q3"],
        );
        let turns = core.manager.turns();
        turns.register(agent_id, 7);
        turns.observe(agent_id, 7, 1, TurnSignal::Ended(TurnEndKind::Failed));
        let _ = core.multiview.acquire(agent_id, 2);

        let req = rid();
        let events = dispatch_one(
            &core,
            1,
            AgentCommand::ListQueuedInputs {
                agent_id,
                request_id: req,
            },
        )
        .await;
        let [AgentEvent::QueuedInputs {
            request_id,
            agent_id: answered_for,
            inputs,
            as_of_seq,
            epoch,
            stopped_after_error,
        }] = events.as_slice()
        else {
            panic!("expected 1 QueuedInputs (it must not be caught by the lease): {events:?}");
        };
        assert_eq!((*request_id, *answered_for), (req, agent_id));
        let row = |id: &str, text: &str, state: &str, cancel| WireQueuedInputRow {
            id: id.into(),
            text: text.into(),
            state: state.into(),
            cancel,
        };
        assert_eq!(
            inputs,
            &vec![
                row("q1", "first", "queued", None),
                row("q2", "second", "unconfirmed", None),
                row(
                    "q3",
                    "third",
                    "cancelling",
                    Some(WireQueuedInputCancel {
                        answer: "not_removed".into(),
                        vendor_closed: false,
                    })
                ),
            ]
        );
        assert_eq!(*epoch, 7);
        assert!(
            *stopped_after_error,
            "the error end of the turn observation table as is"
        );
        // The same seq as the manager value the bus carries (rows = the reduced value of the ring prefix `as_of_seq`).
        let listing = core
            .manager
            .list_queued_inputs(agent_id)
            .expect("a live session");
        assert!(
            as_of_seq.is_some(),
            "the incarnation that reduced the list events"
        );
        assert_eq!(*as_of_seq, listing.as_of_seq);
    }

    #[tokio::test]
    async fn list_queued_inputs_of_an_agent_without_a_live_session_is_not_found() {
        let (core, _rx) = test_core();
        let req = rid();
        let events = dispatch_one(
            &core,
            1,
            AgentCommand::ListQueuedInputs {
                agent_id: uuid::Uuid::new_v4(),
                request_id: req,
            },
        )
        .await;
        match events.as_slice() {
            [AgentEvent::Error {
                request_id: Some(r),
                message,
                ..
            }] => {
                assert_eq!(*r, req);
                assert!(
                    message.starts_with("NOT_FOUND: "),
                    "the code is carried as a value: {message}"
                );
            }
            other => panic!("expected an Error with a correlation key: {other:?}"),
        }
    }

    /// A non-holder is refused with the same characters as `WriteStdin` — two commands to the same agent, answers compared. The refusal
    /// comes before the manager (otherwise a nonexistent agent would give NOT_FOUND).
    #[tokio::test]
    async fn cancel_queued_input_refuses_a_non_holder_exactly_like_write_stdin() {
        let (core, _rx) = test_core();
        let agent_id = uuid::Uuid::new_v4();
        let _ = core.multiview.acquire(agent_id, 2);

        let (write_req, cancel_req) = (rid(), rid());
        let write = dispatch_one(
            &core,
            1,
            AgentCommand::WriteStdin {
                agent_id,
                data: b"x".to_vec(),
                request_id: write_req,
            },
        )
        .await;
        let cancel = dispatch_one(
            &core,
            1,
            AgentCommand::CancelQueuedInput {
                agent_id,
                input_id: "q1".into(),
                request_id: cancel_req,
            },
        )
        .await;
        let (
            [AgentEvent::Error {
                request_id: Some(w),
                message: write_message,
                ..
            }],
            [AgentEvent::Error {
                request_id: Some(c),
                message: cancel_message,
                ..
            }],
        ) = (write.as_slice(), cancel.as_slice())
        else {
            panic!("expected both to be Errors with a correlation key: {write:?} / {cancel:?}");
        };
        assert_eq!((*w, *c), (write_req, cancel_req));
        assert_eq!(cancel_message, write_message, "the same refusal wording");
        assert_eq!(cancel_message, INPUT_LOCKED_REFUSAL);
    }

    /// Today always NOT_FOUND — every backend's mid-turn input policy is `None`, so even a listed id is not a cancel target. The lease
    /// holder (passes) and an agent nobody holds (passes) both reach the manager's NOT_FOUND.
    #[tokio::test]
    async fn cancel_queued_input_is_not_found_today() {
        use engram_dashboard_agent::types::QueuedInputEvent as Q;

        let (core, _rx) = test_core();
        let live = queued_seam::insert(
            &core.manager,
            0,
            vec![Q::Queued {
                id: "q1".into(),
                text: "first".into(),
            }],
            &[],
        );
        let _ = core.multiview.acquire(live, 1);
        for (conn_id, agent_id) in [(1, live), (1, uuid::Uuid::new_v4())] {
            let req = rid();
            let events = dispatch_one(
                &core,
                conn_id,
                AgentCommand::CancelQueuedInput {
                    agent_id,
                    input_id: "q1".into(),
                    request_id: req,
                },
            )
            .await;
            match events.as_slice() {
                [AgentEvent::Error {
                    request_id: Some(r),
                    message,
                    ..
                }] => {
                    assert_eq!(*r, req);
                    assert!(
                        message.starts_with("NOT_FOUND: "),
                        "the lease passes and the manager says NOT_FOUND: {message}"
                    );
                }
                other => panic!("expected a NOT_FOUND with a correlation key: {other:?}"),
            }
        }
        let listing = core
            .manager
            .list_queued_inputs(live)
            .expect("a live session");
        assert_eq!(
            listing.rows.len(),
            1,
            "a refused cancel does not touch the roster"
        );
    }

    // ── AcquireInput ─────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn acquire_input_acks_and_broadcasts() {
        let (core, _rx) = test_core();
        let agent_id = uuid::Uuid::new_v4();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let req = rid();
        core.dispatch(
            AgentCommand::AcquireInput {
                agent_id,
                request_id: req,
            },
            &session,
            &mock,
        )
        .await;
        match mock.events().as_slice() {
            [AgentEvent::Ack { request_id }] => assert_eq!(*request_id, req),
            other => panic!("expected an Ack: {other:?}"),
        }
        let (tx2, _r) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock2 = MockOutboundSink::new(tx2);
        core.dispatch(
            AgentCommand::AcquireInput {
                agent_id,
                request_id: rid(),
            },
            &session,
            &mock2,
        )
        .await;
        assert!(
            matches!(mock2.events().as_slice(), [AgentEvent::Ack { .. }]),
            "re-acquiring is an idempotent Ack"
        );
    }

    // ── ListAgents ───────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn list_agents_returns_agent_list() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let req = rid();
        core.dispatch(
            AgentCommand::ListAgents { request_id: req },
            &session,
            &mock,
        )
        .await;
        match mock.events().as_slice() {
            [AgentEvent::AgentList { request_id, .. }] => assert_eq!(*request_id, req),
            other => panic!("expected an AgentList: {other:?}"),
        }
    }

    // ── command bus registration wire (ADR-0155/0156 · TRD §3-7) ────────────────────────────
    use engram_dashboard_command::{CommandDecl, ErrorCode, Roster};

    fn decl(name: &str) -> CommandDecl {
        CommandDecl {
            name: name.to_string(),
            help: format!("{{\"name\":\"{name}\"}}"),
            agent_args: Vec::new(),
            deadline: engram_dashboard_command::DeadlineClass::Ordinary,
        }
    }

    use crate::log_capture::capture_levels as capture_logs;

    // ── judgment of the claimed owner token (ADR-0154) ────────────────────────────────────────
    //
    // All three bite whether the comparison uses the roster's stored value (an implementation re-deriving from the session flips all
    // three); the stored value is an argument, so that state can be made without the roster or `attach`. Every judgment is driven to the
    // warn callsite on purpose: only these three tests hit it, so no other test can press tracing's global callsite-interest cache first.
    // The debug branch is not a basis for the judgment: the registration wire tests hit it without a subscriber.

    /// Impersonation — wrote our format while the real owner is not that. A re-deriving implementation sees `conn-7` equal to its derived value and lets it through.
    #[test]
    fn a_claim_of_our_token_shape_is_warned_when_the_real_owner_is_not_derived() {
        let session = ConnectionSession::new(7);
        let actual = OwnerToken::new("shell-a1");

        let logged = capture_logs(|| {
            note_claimed_owner(&session, &OwnerToken::new("conn-7"), Some(&actual))
        });

        assert_eq!(logged.len(), 1, "must be one line: {logged:?}");
        assert_eq!(
            logged[0].0,
            tracing::Level::WARN,
            "impersonation is a warn: {logged:?}"
        );
        assert!(
            session.claimed_owner_warned.load(Ordering::Relaxed),
            "the once-per-connection latch must stand"
        );
    }

    /// Honest — wrote its own stored token as is. A re-deriving implementation sees a difference and warns the honest side as an impersonator.
    ///
    /// The stored token is in our format (`conn-9`) so the flip comes out as a warn; a shape like `shell-a1` would fall to debug and fail the basis condition above.
    #[test]
    fn a_claim_that_matches_the_real_owner_is_silent() {
        let session = ConnectionSession::new(7);
        let actual = OwnerToken::new("conn-9");

        let logged = capture_logs(|| note_claimed_owner(&session, &actual, Some(&actual)));

        assert!(
            logged.is_empty(),
            "silent when it equals the real owner: {logged:?}"
        );
    }

    /// A connection not attached — no value to compare. A re-deriving implementation makes `conn-7` here too and lets it pass.
    #[test]
    fn a_claim_of_our_token_shape_is_warned_when_nothing_is_attached() {
        let session = ConnectionSession::new(7);

        let logged =
            capture_logs(|| note_claimed_owner(&session, &OwnerToken::new("conn-7"), None));

        assert_eq!(logged.len(), 1, "must be one line: {logged:?}");
        assert_eq!(logged[0].0, tracing::Level::WARN, "{logged:?}");
    }

    /// The tail re-packing carries the inner string as is, and the two types share wire bytes, so unpacking gives the original value.
    #[test]
    fn the_wire_owner_crosses_into_the_bus_type_unchanged() {
        for raw in [
            "",
            "engram-dashboard-shell",
            "conn-7",
            "owner \"quote\"\n\u{7f}",
        ] {
            let wire = WireOwnerToken::new(raw);
            let owner = owner_from_wire(wire.clone());
            assert_eq!(owner.as_str(), raw);

            let bytes = serde_json::to_string(&wire).unwrap();
            assert_eq!(bytes, serde_json::to_string(&owner).unwrap(), "{raw:?}");
            assert_eq!(serde_json::from_str::<OwnerToken>(&bytes).unwrap(), owner);
            assert_eq!(
                serde_json::from_str::<WireOwnerToken>(&bytes).unwrap(),
                wire
            );
        }
    }

    /// Deliberately carries a token in the owner field that is not the derived value — the wire's contract is that the roster's owner
    /// comes from the connection, not the packet (`ConnectionSession::owner_token`).
    fn register(decls: Vec<CommandDecl>, request_id: RequestId) -> AgentCommand {
        AgentCommand::RegisterCommands {
            owner: WireOwnerToken::new("whatever-the-client-thinks"),
            decls: decls_slot(&decls),
            catalog_version: 7,
            request_id,
        }
    }

    fn update(added: Vec<CommandDecl>, removed: Vec<&str>, request_id: RequestId) -> AgentCommand {
        AgentCommand::UpdateCommands {
            owner: WireOwnerToken::new("whatever-the-client-thinks"),
            added: decls_slot(&added),
            removed: removed.into_iter().map(str::to_string).collect(),
            request_id,
        }
    }

    /// Wraps the registration item list into the packet's raw field — production wraps in the shell (the daemon only unpacks).
    fn decls_slot(decls: &[CommandDecl]) -> Box<RawValue> {
        serde_json::value::to_raw_value(decls).expect("registration item serialization")
    }

    /// Same field bytes as the protocol registration golden (`MIXED_DECLS` in `messages.rs`) · the shell `protocol_state` test.
    const MIXED_DECLS: &str = r#"[{"name":"slot.focus","help":"{}","agent_args":["agent","after_tab"]},{"name":"tab.focus","help":"{}"}]"#;

    /// A session with a connection standing — the roster accepts registrations only from attached connections; this line is production's `on_connect` (`CommandRoster::attach`).
    fn attached(core: &ConnectionCore, conn_id: ConnId) -> ConnectionSession {
        // Only the place is filled (measuring that table is `command_roster`'s side).
        // The receiving side dies with this function: nothing can go out through the outlet here, so an assertion on delivery would come up
        //   empty — change this helper to hand the receiver back first.
        let (tx, _rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(1);
        let frames: Arc<dyn frame_port::FrameSink> =
            Arc::new(crate::test_doubles::FakeFrameSink::new(tx));
        core.commands().attach(conn_id, &frames);
        ConnectionSession::new(conn_id)
    }

    #[tokio::test]
    async fn register_commands_acks_and_fills_the_roster() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = attached(&core, 1);
        let req = rid();

        core.dispatch(register(vec![decl("tab.create")], req), &session, &mock)
            .await;

        match mock.events().as_slice() {
            [AgentEvent::Ack { request_id }] => assert_eq!(*request_id, req),
            other => panic!("a registration is answered with an Ack: {other:?}"),
        }
        let entries = core.commands().entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "tab.create");
        assert_eq!(
            entries[0].owner,
            session.owner_token(),
            "the owner is derived from the connection — not the token the packet wrote"
        );
    }

    #[tokio::test]
    async fn update_commands_acks_and_applies_the_delta() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = attached(&core, 1);
        core.dispatch(
            register(vec![decl("tab.create"), decl("tab.close")], rid()),
            &session,
            &mock,
        )
        .await;

        let req = rid();
        core.dispatch(
            update(vec![decl("tab.split")], vec!["tab.close"], req),
            &session,
            &mock,
        )
        .await;

        match mock.events().as_slice() {
            [_, AgentEvent::Ack { request_id }] => assert_eq!(*request_id, req),
            other => panic!("a diff is answered with an Ack too: {other:?}"),
        }
        let names: Vec<String> = core
            .commands()
            .entries()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(
            names,
            vec!["tab.create".to_string(), "tab.split".to_string()],
            "a removed name vanishes with its place (ADR-0150) and a name not in the diff stays as is"
        );
    }

    /// The agent reference field goes to the roster from unpacking the raw registration field (ADR-0308 · TRD trd-3 §6-1 U4's A4) — the
    /// table the pre-relay rewrite (`command_delivery::resolve_agent_args`) reads is filled by this. Registration and diff share it.
    #[tokio::test]
    async fn the_register_slot_hands_agent_args_to_the_roster() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = attached(&core, 1);

        let registered = rid();
        core.dispatch(
            AgentCommand::RegisterCommands {
                owner: WireOwnerToken::new("shell"),
                decls: RawValue::from_string(MIXED_DECLS.to_string()).unwrap(),
                catalog_version: 1,
                request_id: registered,
            },
            &session,
            &mock,
        )
        .await;
        let updated = rid();
        core.dispatch(
            AgentCommand::UpdateCommands {
                owner: WireOwnerToken::new("shell"),
                added: RawValue::from_string(
                    r#"[{"name":"tab.dock","help":"{}","agent_args":["agent"]}]"#.to_string(),
                )
                .unwrap(),
                removed: vec![],
                request_id: updated,
            },
            &session,
            &mock,
        )
        .await;

        match mock.events().as_slice() {
            [AgentEvent::Ack { request_id: a }, AgentEvent::Ack { request_id: b }] => {
                assert_eq!((*a, *b), (registered, updated));
            }
            other => panic!("both registration · diff are an Ack: {other:?}"),
        }
        let commands = core.commands();
        assert_eq!(
            commands.agent_args_of("slot.focus"),
            vec!["agent".to_string(), "after_tab".to_string()],
            "in declaration order"
        );
        assert!(commands.agent_args_of("tab.focus").is_empty());
        assert_eq!(
            commands.agent_args_of("tab.dock"),
            vec!["agent".to_string()]
        );
    }

    /// ADR-0369: a registration slot carries the deadline class into the roster (the seat reads it from there); an old
    /// item without the field registers as ordinary.
    #[test]
    fn a_registered_deadline_class_reaches_the_roster_and_an_old_item_is_ordinary() {
        use engram_dashboard_command::DeadlineClass;
        let raw = RawValue::from_string(
            r#"[{"name":"agent.spawnInto","help":"{}","deadline":"Launch"},{"name":"tab.focus","help":"{}"}]"#
                .to_string(),
        )
        .unwrap();
        let decls = decls_from_wire(&raw).expect("the list unpacks");
        let commands = CommandRoster::new();
        let (sink, _inbox) = {
            let (tx, rx) = tokio::sync::mpsc::channel::<crate::conn_port::Frame>(4);
            let sink: Arc<dyn crate::conn_port::FrameSink> =
                Arc::new(crate::test_doubles::FakeFrameSink::new(tx));
            (sink, rx)
        };
        commands.attach(1, &sink);
        commands.register(1, decls).expect("register");
        assert_eq!(commands.class_of("agent.spawnInto"), DeadlineClass::Launch);
        assert_eq!(commands.class_of("tab.focus"), DeadlineClass::Ordinary);
    }

    /// An old item missing the field unpacks to an empty `agent_args` (ADR-0308); protocol `decl_agent_args_are_optional_on_the_wire` measures the packet side.
    #[test]
    fn an_old_register_slot_unpacks_to_no_agent_args() {
        let old =
            RawValue::from_string(r#"[{"name":"tab.focus","help":"{}"}]"#.to_string()).unwrap();
        match decls_from_wire(&old)
            .expect("an old item unpacks too")
            .as_slice()
        {
            [decl] => {
                assert_eq!(decl.name, "tab.focus");
                assert!(decl.agent_args.is_empty(), "no field means an empty list");
            }
            other => panic!("must be one item: {other:?}"),
        }

        let mixed = RawValue::from_string(MIXED_DECLS.to_string()).unwrap();
        let decls = decls_from_wire(&mixed).expect("a list with the field");
        assert_eq!(
            decls
                .iter()
                .map(|decl| (decl.name.as_str(), decl.agent_args.clone()))
                .collect::<Vec<_>>(),
            vec![
                (
                    "slot.focus",
                    vec!["agent".to_string(), "after_tab".to_string()]
                ),
                ("tab.focus", Vec::new()),
            ]
        );
    }

    /// A broken item list is refused with that packet's number (ADR-0272 decision 6 · `broken_decls_refusal`): the whole packet is
    /// rejected (the diff's `removed` is not applied either) and the connection lives.
    #[tokio::test]
    async fn a_broken_register_slot_is_refused_with_its_request_id_and_applies_nothing() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = attached(&core, 1);
        core.dispatch(register(vec![decl("tab.create")], rid()), &session, &mock)
            .await;

        let registered = rid();
        let flow = core
            .dispatch(
                AgentCommand::RegisterCommands {
                    owner: WireOwnerToken::new("shell"),
                    decls: RawValue::from_string(r#"[{"name":1}]"#.to_string()).unwrap(),
                    catalog_version: 1,
                    request_id: registered,
                },
                &session,
                &mock,
            )
            .await;
        assert_eq!(flow, DispatchFlow::Continue, "the connection lives");
        let updated = rid();
        let flow = core
            .dispatch(
                AgentCommand::UpdateCommands {
                    owner: WireOwnerToken::new("shell"),
                    added: RawValue::from_string(r#"{"name":"tab.split"}"#.to_string()).unwrap(),
                    removed: vec!["tab.create".to_string()],
                    request_id: updated,
                },
                &session,
                &mock,
            )
            .await;
        assert_eq!(flow, DispatchFlow::Continue, "the connection lives");

        match mock.events().as_slice() {
            [AgentEvent::Ack { .. }, AgentEvent::Error {
                request_id: Some(a),
                message: first,
                ..
            }, AgentEvent::Error {
                request_id: Some(b),
                message: second,
                ..
            }] => {
                assert_eq!(
                    (*a, *b),
                    (registered, updated),
                    "answers with that packet's number"
                );
                for message in [first, second] {
                    assert!(
                        message.starts_with("INVALID_ARGUMENT: invalid command declarations"),
                        "{message}"
                    );
                }
            }
            other => {
                panic!("a broken registration · diff is an Error carrying the number: {other:?}")
            }
        }
        let names: Vec<String> = core
            .commands()
            .entries()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(
            names,
            vec!["tab.create".to_string()],
            "a broken packet does not touch the roster"
        );
    }

    /// An unknown deadline class breaks the item, so the whole packet is refused with its own number — the valid item
    /// beside it and the diff's `removed` are not applied, and the connection lives.
    // ADR-0272
    #[tokio::test]
    async fn an_unknown_deadline_class_refuses_the_whole_packet_with_its_request_id() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = attached(&core, 1);
        core.dispatch(register(vec![decl("tab.create")], rid()), &session, &mock)
            .await;

        let registered = rid();
        let flow = core
            .dispatch(
                AgentCommand::RegisterCommands {
                    owner: WireOwnerToken::new("shell"),
                    decls: RawValue::from_string(
                        r#"[{"name":"tab.focus","help":"{}"},{"name":"agent.spawnInto","help":"{}","deadline":"Someday"}]"#
                            .to_string(),
                    )
                    .unwrap(),
                    catalog_version: 1,
                    request_id: registered,
                },
                &session,
                &mock,
            )
            .await;
        assert_eq!(flow, DispatchFlow::Continue, "the connection lives");
        let updated = rid();
        let flow = core
            .dispatch(
                AgentCommand::UpdateCommands {
                    owner: WireOwnerToken::new("shell"),
                    added: RawValue::from_string(
                        r#"[{"name":"agent.spawnInto","help":"{}","deadline":"Someday"}]"#
                            .to_string(),
                    )
                    .unwrap(),
                    removed: vec!["tab.create".to_string()],
                    request_id: updated,
                },
                &session,
                &mock,
            )
            .await;
        assert_eq!(flow, DispatchFlow::Continue, "the connection lives");

        match mock.events().as_slice() {
            [AgentEvent::Ack { .. }, AgentEvent::Error {
                request_id: Some(a),
                message: first,
                ..
            }, AgentEvent::Error {
                request_id: Some(b),
                message: second,
                ..
            }] => {
                assert_eq!(
                    (*a, *b),
                    (registered, updated),
                    "answered with that packet's number"
                );
                for message in [first, second] {
                    assert!(
                        message.starts_with("INVALID_ARGUMENT: invalid command declarations"),
                        "{message}"
                    );
                }
            }
            other => panic!("an unknown deadline class is an Error carrying the number: {other:?}"),
        }
        let names: Vec<String> = core
            .commands()
            .entries()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(
            names,
            vec!["tab.create".to_string()],
            "the refused packets leave the roster as it was"
        );
    }

    /// `help` is one opaque string to the daemon — registration and lookup must succeed even with a non-JSON value; parsing or
    /// validation here would break it (TRD §3-7 hard constraint · §7 seam table).
    #[tokio::test]
    async fn list_commands_returns_the_roster_projection_with_the_help_bytes_intact() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = attached(&core, 1);
        let opaque = "not json at all — arbitrary bytes {[(";
        core.dispatch(
            register(
                vec![CommandDecl {
                    name: "tab.create".to_string(),
                    help: opaque.to_string(),
                    agent_args: Vec::new(),
                    deadline: engram_dashboard_command::DeadlineClass::Ordinary,
                }],
                rid(),
            ),
            &session,
            &mock,
        )
        .await;

        let req = rid();
        core.dispatch(
            AgentCommand::ListCommands { request_id: req },
            &session,
            &mock,
        )
        .await;

        match mock.events().as_slice() {
            [_, AgentEvent::CommandList {
                request_id,
                entries,
            }] => {
                assert_eq!(*request_id, req);
                assert_eq!(
                    entries,
                    &vec![CommandListEntry {
                        name: "tab.create".to_string(),
                        help: opaque.to_string(),
                        available: true,
                    }]
                );
            }
            other => panic!("a lookup is answered with a CommandList: {other:?}"),
        }
    }

    /// The discovery list merges two sources — attached owners' roster and the daemon's own table. Leaving out the own table would deliver
    /// `agent.*` yet hide it, so an LLM trusting the list never calls that family. On overlap the daemon's wins (delivery order own table → roster,
    /// `command_delivery::deliver`).
    #[tokio::test]
    async fn list_commands_merges_the_daemons_own_names_and_wins_on_a_clash() {
        // Does not occur in an assembled server — this measures the port contract with an assembly plugging the table in late (production fills
        //   it before accepting connections); afterwards registration itself is rejected with `CONFLICT`.
        let (core, _rx, slot) = test_core_with_late_agent_table();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = attached(&core, 1);
        core.dispatch(
            register(
                vec![
                    decl("tab.create"),
                    CommandDecl {
                        name: "agent.list".to_string(),
                        help: "stolen".to_string(),
                        agent_args: Vec::new(),
                        deadline: engram_dashboard_command::DeadlineClass::Ordinary,
                    },
                ],
                rid(),
            ),
            &session,
            &mock,
        )
        .await;
        assert_eq!(
            core.commands().entries().len(),
            2,
            "premise: the table was in its empty window so that registration passed"
        );

        slot.set(Arc::new(crate::control::commands::make_daemon_table(
            core.manager().clone(),
            Arc::new(crate::control::mcp_server::RosterBroadcastSlot::new()),
            Arc::new(core.multiview().clone()),
            Arc::new(crate::control::commands::NoUsageLimits),
            Arc::new(crate::control::commands::NoLibrary),
        )));

        let req = rid();
        core.dispatch(
            AgentCommand::ListCommands { request_id: req },
            &session,
            &mock,
        )
        .await;

        match mock.events().as_slice() {
            [_, AgentEvent::CommandList {
                request_id,
                entries,
            }] => {
                assert_eq!(*request_id, req);
                let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
                assert_eq!(
                    names,
                    vec![
                        "agent.cancelQueuedInput",
                        "agent.interrupt",
                        "agent.list",
                        "agent.listQueuedInputs",
                        "agent.move",
                        "agent.new",
                        "agent.rename",
                        "agent.resolve",
                        "agent.spawn",
                        "library.folders",
                        "library.list",
                        "library.newFile",
                        "library.newFolder",
                        "library.read",
                        "library.refresh",
                        "library.relink",
                        "library.setRoot",
                        "library.status",
                        "preset.add",
                        "preset.check",
                        "preset.get",
                        "preset.previewFolder",
                        "preset.remove",
                        "preset.reorder",
                        "preset.save",
                        "preset.setBackend",
                        "tab.create",
                        "usage.get",
                        "usage.refresh"
                    ],
                    "the two sources merge into one list in name order"
                );
                let clashed = entries
                    .iter()
                    .find(|e| e.name == "agent.list")
                    .expect("the overlapping name");
                assert_ne!(
                    clashed.help, "stolen",
                    "the shape of an overlapping name is given by the side that actually runs it (the daemon)"
                );
                assert!(
                    serde_json::from_str::<serde_json::Value>(&clashed.help)
                        .is_ok_and(|item| item["name"] == "agent.list"),
                    "the daemon table's declaration is carried as is: {}",
                    clashed.help
                );
            }
            other => panic!("a lookup is answered with a CommandList: {other:?}"),
        }
    }

    /// Registering a name the daemon holds must not succeed: the roster has no such entry, but delivery is answered first by the daemon
    /// (stage 1), so no envelope reaches that owner. Same all-or-nothing and `CONFLICT` as `Roster::update`.
    #[tokio::test]
    async fn registering_a_name_this_daemon_answers_is_refused_as_a_conflict() {
        let (core, _rx) = test_core_with_own_agent_table();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = attached(&core, 1);

        core.dispatch(
            register(
                vec![decl("agent.list"), decl("tab.create"), decl("agent.new")],
                rid(),
            ),
            &session,
            &mock,
        )
        .await;

        match mock.events().as_slice() {
            [AgentEvent::Error { message, .. }] => {
                assert!(message.contains("CONFLICT"), "{message}");
                // Point at every colliding name — registration happens once at attach, so finding the rest would cost one reconnect per name.
                assert!(
                    message.contains("agent.list") && message.contains("agent.new"),
                    "all colliding names must be carried: {message}"
                );
            }
            other => panic!("a colliding name must be refused: {other:?}"),
        }
        assert!(
            core.commands().entries().is_empty(),
            "the whole packet is refused — a non-colliding name must not get in either"
        );

        // A non-colliding packet passes as is (the refusal does not block the whole family).
        core.dispatch(register(vec![decl("tab.create")], rid()), &session, &mock)
            .await;
        assert_eq!(core.commands().entries().len(), 1);

        // A diff is the same decision — blocking only one side lets the same state in through the unblocked side.
        core.dispatch(
            update(vec![decl("agent.new")], vec![], rid()),
            &session,
            &mock,
        )
        .await;
        match mock.events().last() {
            Some(AgentEvent::Error { message, .. }) => {
                assert!(message.contains("CONFLICT"), "{message}");
                assert!(message.contains("agent.new"), "{message}");
            }
            other => panic!("a diff must be refused too: {other:?}"),
        }
        assert_eq!(
            core.commands().entries().len(),
            1,
            "a refusal does not touch the roster"
        );
    }

    // ── the two arms of command delivery (ADR-0154) ────────────────────────────────────────────
    //
    // Per-branch assertions are in `command_delivery`; here: does dispatch connect to that delivery, and the wire answers of the two arms.
    // Tests hitting the `CommandOutcome` refusal warn run inside `capture_logs`: a hit outside registers that callsite's interest on a thread
    // with no subscriber and other captures come up empty (canonical: the `log_capture` module header). dispatch is async, so it runs in a
    // current-thread runtime — `with_default` is thread-local.

    fn bus_command(name: &str, request_id: engram_dashboard_command::RequestId) -> AgentCommand {
        bus_command_with(name, serde_json::json!({ "window": "main" }), request_id)
    }

    fn bus_command_with(
        name: &str,
        args: serde_json::Value,
        request_id: engram_dashboard_command::RequestId,
    ) -> AgentCommand {
        // The destination field is whatever the caller writes — targeting is the daemon's own roster (ADR-0154).
        let envelope = engram_dashboard_command::CommandEnvelope {
            name: name.to_string(),
            request_id,
            owner: OwnerToken::new("whatever-the-caller-thinks"),
            proto_ver: 7,
            args,
        };
        AgentCommand::Command {
            envelope: envelope_to_wire(&envelope).expect("envelope serialization"),
        }
    }

    fn bus_outcome(reply: engram_dashboard_command::CommandReply) -> AgentCommand {
        AgentCommand::CommandOutcome {
            reply: reply_to_wire(&reply).expect("reply serialization"),
        }
    }

    fn on_current_thread<T>(body: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime")
            .block_on(body)
    }

    fn warns_matching(logged: &[(tracing::Level, String)], needle: &str) -> usize {
        logged
            .iter()
            .filter(|(level, line)| *level == tracing::Level::WARN && line.contains(needle))
            .count()
    }

    fn total_warns(logged: &[(tracing::Level, String)]) -> usize {
        logged
            .iter()
            .filter(|(level, _)| *level == tracing::Level::WARN)
            .count()
    }

    /// A fragment catching only the first warning of a refused outcome — fix this constant with the wording.
    const STRAY_OUTCOME_WARN: &str = "logged once per connection";

    /// A session attached to the roster that can receive the frames sent out to that connection. [`attached`] drops the receiver at once,
    /// so delivery assertions would come up empty.
    fn attached_with_inbox(
        core: &ConnectionCore,
        conn_id: ConnId,
    ) -> (
        ConnectionSession,
        tokio::sync::mpsc::Receiver<frame_port::Frame>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let frames: Arc<dyn frame_port::FrameSink> =
            Arc::new(crate::test_doubles::FakeFrameSink::new(tx));
        core.commands().attach(conn_id, &frames);
        (ConnectionSession::new(conn_id), rx)
    }

    /// Cap the wait so delivery to the wrong connection is a failure, not a hang (the same helper as in `command_delivery`).
    async fn next_frame_event(
        rx: &mut tokio::sync::mpsc::Receiver<frame_port::Frame>,
    ) -> AgentEvent {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
            .await
            .expect("exceeded the cap waiting for a frame — the delivery does not come to this connection")
            .expect("one frame");
        match frame {
            frame_port::Frame::Text(text) => serde_json::from_str(&text).expect("event decode"),
            other => panic!("expected Text: {other:?}"),
        }
    }

    /// The request goes to the owner, the reply to the connection that originally asked — one dispatch pair measures the round trip. Also
    /// pins that the reply does not go out via `dispatch`'s return path (an owner that is this connection could not take its own answer out — TRD §3-6).
    #[tokio::test]
    async fn a_command_is_delivered_to_its_owner_and_its_reply_returns_to_the_caller() {
        let (core, _rx) = test_core();
        let (owner, mut owner_inbox) = attached_with_inbox(&core, 1);
        let (caller, mut caller_inbox) = attached_with_inbox(&core, 2);
        let (tx, _spill) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let owner_mock = MockOutboundSink::new(tx.clone());
        let caller_mock = MockOutboundSink::new(tx);
        core.dispatch(
            register(vec![decl("tab.create")], rid()),
            &owner,
            &owner_mock,
        )
        .await;
        let req = engram_dashboard_command::RequestId::new();

        core.dispatch(bus_command("tab.create", req), &caller, &caller_mock)
            .await;

        match next_frame_event(&mut owner_inbox).await {
            AgentEvent::CommandRequest { envelope } => {
                let envelope = envelope_from_wire(&envelope)
                    .expect("the envelope the daemon re-wrapped unpacks");
                assert_eq!(envelope.request_id, req, "the id is the same throughout");
                assert_eq!(
                    Some(envelope.owner),
                    core.commands().attached_owner(1),
                    "the owner the roster stored is carried on the outer envelope — not the value the caller wrote"
                );
                assert_eq!(
                    envelope.args,
                    serde_json::json!({ "window": "main" }),
                    "the inside passes through without parsing"
                );
            }
            other => panic!("the owner receives a CommandRequest: {other:?}"),
        }

        core.dispatch(
            bus_outcome(engram_dashboard_command::CommandReply::ok(
                req,
                serde_json::json!({ "tab": 4 }),
            )),
            &owner,
            &owner_mock,
        )
        .await;

        match next_frame_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = reply_from_wire(&reply).expect("the reply the daemon wrapped unpacks");
                assert_eq!(reply.request_id, req);
                assert_eq!(
                    reply.outcome,
                    Ok(serde_json::json!({ "tab": 4 })),
                    "the owner's outcome arrives as is"
                );
            }
            other => panic!("the connection that asked receives a CommandReply: {other:?}"),
        }
        assert!(
            owner_inbox.try_recv().is_err(),
            "the reply must not leak to the owner side"
        );
        assert!(
            caller_mock.events().is_empty(),
            "the request arm gives no synchronous answer: {:?}",
            caller_mock.events()
        );
        assert!(
            matches!(owner_mock.events().as_slice(), [AgentEvent::Ack { .. }]),
            "the outcome arm is quiet too (only the one registration Ack): {:?}",
            owner_mock.events()
        );
        assert_eq!(
            core.deliveries().in_flight(),
            0,
            "a finished round trip leaves the table"
        );
    }

    /// A broken envelope is a refusal with no number (ADR-0272 decision 6): `Error{request_id: None}` like the parse-failure branch, no round trip, the connection lives.
    #[tokio::test]
    async fn a_broken_envelope_is_refused_without_a_request_id_and_opens_no_round_trip() {
        let (core, _rx) = test_core();
        let (owner, mut owner_inbox) = attached_with_inbox(&core, 1);
        let (caller, _caller_inbox) = attached_with_inbox(&core, 2);
        let (tx, _spill) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let owner_mock = MockOutboundSink::new(tx.clone());
        let caller_mock = MockOutboundSink::new(tx);
        core.dispatch(
            register(vec![decl("tab.create")], rid()),
            &owner,
            &owner_mock,
        )
        .await;

        let broken = AgentCommand::Command {
            envelope: RawValue::from_string(r#"{"name":"tab.create"}"#.to_string()).unwrap(),
        };
        assert_eq!(
            command_request_id(&broken),
            None,
            "a saturation refusal has no number to carry either"
        );
        let flow = core.dispatch(broken, &caller, &caller_mock).await;

        assert_eq!(flow, DispatchFlow::Continue, "the connection lives");
        match caller_mock.events().as_slice() {
            [AgentEvent::Error {
                request_id: None,
                message,
                ..
            }] => assert!(message.starts_with("invalid command envelope"), "{message}"),
            other => panic!("must be one Error with no number: {other:?}"),
        }
        // If a regression delivered in a spawned task, give it a gap to run so the negative assertion below sees it.
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert!(
            owner_inbox.try_recv().is_err(),
            "a broken envelope does not go to the owner"
        );
        assert_eq!(core.deliveries().in_flight(), 0, "no round trip is opened");
    }

    /// The refusal number of a received command is the uuid inside the envelope (ADR-0081 decision 4); the sender's reply (`CommandOutcome`) has none.
    #[test]
    fn the_refusal_reads_the_request_id_inside_the_envelope() {
        let req = engram_dashboard_command::RequestId::new();
        assert_eq!(
            command_request_id(&bus_command("tab.create", req)),
            Some(RequestId(req.0))
        );
        assert_eq!(
            command_request_id(&bus_outcome(engram_dashboard_command::CommandReply::ok(
                req,
                serde_json::Value::Null,
            ))),
            None
        );
    }

    /// The relay-re-wrapped packet has the same bytes as the protocol golden (`command_request_json_golden_and_roundtrip`, ADR-0272
    /// decision 6): a changed envelope shape · field order would change the bytes the owner receives. Those bytes also unpack to the same envelope.
    #[test]
    fn the_relay_repacks_the_envelope_into_the_golden_bytes() {
        let envelope = engram_dashboard_command::CommandEnvelope {
            name: "tab.create".to_string(),
            request_id: engram_dashboard_command::RequestId(uuid::Uuid::nil()),
            owner: OwnerToken::new("shell"),
            proto_ver: 1,
            args: serde_json::json!({ "window": "main" }),
        };
        let packet = event_json(&AgentEvent::CommandRequest {
            envelope: envelope_to_wire(&envelope).unwrap(),
        })
        .unwrap();
        assert_eq!(
            packet,
            r#"{"CommandRequest":{"envelope":{"name":"tab.create","request_id":"00000000-0000-0000-0000-000000000000","owner":"shell","proto_ver":1,"args":{"window":"main"}}}}"#
        );
        let AgentEvent::CommandRequest { envelope: raw } = serde_json::from_str(&packet).unwrap()
        else {
            panic!("must be a CommandRequest");
        };
        assert_eq!(envelope_from_wire(&raw).unwrap(), envelope);
    }

    /// A broken reply is a refusal with no number (ADR-0272 decision 6): `Error{request_id: None}` goes to the connection that sent the
    /// outcome, attaching to no round trip. A matching number in the raw is not picked out (ADR-0272 "rejected alternatives": partial
    /// parsing). The round trip stays open until an intact outcome comes.
    #[tokio::test]
    async fn a_broken_outcome_is_refused_without_a_request_id_and_settles_nothing() {
        let (core, _rx) = test_core();
        let (owner, mut owner_inbox) = attached_with_inbox(&core, 1);
        let (caller, mut caller_inbox) = attached_with_inbox(&core, 2);
        let (tx, _spill) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let owner_mock = MockOutboundSink::new(tx.clone());
        let caller_mock = MockOutboundSink::new(tx);
        core.dispatch(
            register(vec![decl("tab.create")], rid()),
            &owner,
            &owner_mock,
        )
        .await;
        let req = engram_dashboard_command::RequestId::new();
        core.dispatch(bus_command("tab.create", req), &caller, &caller_mock)
            .await;
        assert!(matches!(
            next_frame_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        let broken = AgentCommand::CommandOutcome {
            reply: RawValue::from_string(format!(r#"{{"request_id":"{req}"}}"#)).unwrap(),
        };
        let flow = core.dispatch(broken, &owner, &owner_mock).await;

        assert_eq!(flow, DispatchFlow::Continue, "the connection lives");
        match owner_mock.events().as_slice() {
            [.., AgentEvent::Error {
                request_id: None,
                message,
                ..
            }] => assert!(message.starts_with("invalid command reply"), "{message}"),
            other => panic!("the last must be an Error with no number to the side that sent the outcome: {other:?}"),
        }
        assert_eq!(
            core.deliveries().in_flight(),
            1,
            "a broken reply does not close the round trip"
        );
        assert!(
            caller_inbox.try_recv().is_err(),
            "nothing goes to the asker"
        );

        core.dispatch(
            bus_outcome(engram_dashboard_command::CommandReply::ok(
                req,
                serde_json::json!({ "tab": 4 }),
            )),
            &owner,
            &owner_mock,
        )
        .await;
        match next_frame_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => assert_eq!(
                reply_from_wire(&reply)
                    .expect("the reply the daemon wrapped unpacks")
                    .request_id,
                req
            ),
            other => panic!("an intact outcome closes that round trip: {other:?}"),
        }
    }

    /// The relay-wrapped reply packet has the same bytes as the protocol golden (`command_reply_json_golden_and_roundtrip`, ADR-0272);
    /// those bytes also unpack to the same reply.
    #[test]
    fn the_relay_repacks_the_reply_into_the_golden_bytes() {
        let nil = engram_dashboard_command::RequestId(uuid::Uuid::nil());
        for (reply, golden) in [
            (
                CommandReply::err(nil, CommandError::of(ErrorCode::NotFound, "no view 'x'")),
                r#"{"CommandReply":{"reply":{"request_id":"00000000-0000-0000-0000-000000000000","outcome":{"Err":{"code":"NOT_FOUND","message":"no view 'x'","retry":"never"}}}}}"#,
            ),
            (
                CommandReply::ok(nil, serde_json::json!({ "view_id": "v1" })),
                r#"{"CommandReply":{"reply":{"request_id":"00000000-0000-0000-0000-000000000000","outcome":{"Ok":{"view_id":"v1"}}}}}"#,
            ),
        ] {
            let packet = event_json(&AgentEvent::CommandReply {
                reply: reply_to_wire(&reply).unwrap(),
            })
            .unwrap();
            assert_eq!(packet, golden);
            let AgentEvent::CommandReply { reply: raw } = serde_json::from_str(&packet).unwrap()
            else {
                panic!("must be a CommandReply");
            };
            assert_eq!(reply_from_wire(&raw).unwrap(), reply);
        }
    }

    /// Pins the tolerance advertised by the hand-written TS reply field (protocol `AgentCommand::CommandOutcome` · `AgentEvent::CommandReply`
    /// `#[ts(type = …)]`) at this unpacking — protocol cannot see the raw reply shape (ADR-0272). If the reply type is later tightened, the
    /// advertisement silently becomes false; the golden above sees only reply → bytes, so this reverses the direction.
    #[test]
    fn the_outcome_slot_accepts_everything_its_ts_type_advertises() {
        let nil = "00000000-0000-0000-0000-000000000000";
        // All three fields absent + a field outside the contract.
        let sparse = RawValue::from_string(format!(
            r#"{{"request_id":"{nil}","outcome":{{"Err":{{"detail":{{"n":1}}}}}}}}"#
        ))
        .unwrap();
        let reply = reply_from_wire(&sparse)
            .expect("accepted even without the three fields (the TS field advertises so)");
        assert!(reply.outcome.is_err());
        // A field outside the contract must survive the way back — an additive extension does not evaporate at the relay hop.
        assert!(
            reply_to_wire(&reply).unwrap().get().contains("\"detail\""),
            "a field outside the contract vanished in the relay"
        );

        // All three fields `null`.
        let nulls = RawValue::from_string(format!(
            r#"{{"request_id":"{nil}","outcome":{{"Err":{{"code":null,"message":null,"retry":null}}}}}}"#
        ))
        .unwrap();
        reply_from_wire(&nulls).expect("three nulls are accepted too");
    }

    /// After the deadline the reply goes out exactly once; a late outcome finds no place to attach, falls to the sibling arm's refusal, and
    /// that refusal goes to the connection that sent the outcome. The clock is pushed by hand.
    #[test]
    fn a_command_whose_owner_never_answers_is_timed_out_exactly_once() {
        let clock = Arc::new(engram_dashboard_base::time::ManualClock::new());
        let deliveries = crate::command_delivery::tests::deliveries_with(clock.clone());
        let mut seen = None;
        capture_logs(|| {
            seen = Some(on_current_thread(async {
                let (core, _rx, _fanout) = test_core_with(deliveries.clone());
                let (owner, mut owner_inbox) = attached_with_inbox(&core, 1);
                let (caller, mut caller_inbox) = attached_with_inbox(&core, 2);
                let (tx, _spill) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
                let mock = MockOutboundSink::new(tx);
                core.dispatch(register(vec![decl("tab.create")], rid()), &owner, &mock)
                    .await;
                let req = engram_dashboard_command::RequestId::new();

                core.dispatch(bus_command("tab.create", req), &caller, &mock)
                    .await;
                assert!(matches!(
                    next_frame_event(&mut owner_inbox).await,
                    AgentEvent::CommandRequest { .. }
                ));

                clock.advance(std::time::Duration::from_secs(11));
                assert_eq!(deliveries.expire(), 1, "after the deadline it is reaped");
                let timed_out = next_frame_event(&mut caller_inbox).await;

                // The owner answers late — there is no place to attach.
                core.dispatch(
                    bus_outcome(engram_dashboard_command::CommandReply::ok(
                        req,
                        serde_json::json!({ "tab": 4 }),
                    )),
                    &owner,
                    &mock,
                )
                .await;

                (
                    timed_out,
                    mock.events(),
                    caller_inbox.try_recv().is_err(),
                    deliveries.in_flight(),
                )
            }));
        });

        let (timed_out, events, caller_quiet, in_flight) = seen.expect("the body ran");
        match timed_out {
            AgentEvent::CommandReply { reply } => {
                let reply = reply_from_wire(&reply).expect("the reply the daemon wrapped unpacks");
                let err = reply.outcome.expect_err("deadline exceeded");
                assert_eq!(err.code(), ErrorCode::Timeout);
                assert_eq!(
                    err.retry(),
                    engram_dashboard_command::RetryMode::Never,
                    "there is no dedup store so a re-query becomes a re-execution — this path does not instruct a retry\
                     (basis = `command_delivery::no_retry`)"
                );
            }
            other => panic!("the deadline answers with a CommandReply: {other:?}"),
        }
        assert!(
            caller_quiet,
            "a late outcome must not become a second reply (TRD §4-⑤)"
        );
        assert_eq!(in_flight, 0, "a reaped place must not remain");
        match events.as_slice() {
            [AgentEvent::Ack { .. }, AgentEvent::Error {
                request_id,
                message,
                ..
            }] => {
                assert_eq!(*request_id, None, "this frame has no correlation key");
                assert!(message.contains("deadline"), "{message}");
            }
            other => panic!("a late outcome is answered with a refusal to the connection that sent the outcome: {other:?}"),
        }
    }

    /// With no round trip to attach to it does not stay silent, but warns once per connection (the client can send this frame any number
    /// of times; latch = `stray_outcome_warned` doc).
    #[test]
    fn a_stray_outcome_is_refused_and_warns_once_per_connection() {
        let mut seen = None;
        let logged = capture_logs(|| {
            seen = Some(on_current_thread(async {
                let (core, _rx) = test_core();
                let (tx, _spill) = tokio::sync::mpsc::channel::<frame_port::Frame>(32);
                let mock = MockOutboundSink::new(tx);
                let first = ConnectionSession::new(1);
                let second = ConnectionSession::new(2);

                for session in [&first, &second, &first] {
                    core.dispatch(
                        bus_outcome(engram_dashboard_command::CommandReply::ok(
                            engram_dashboard_command::RequestId::new(),
                            serde_json::json!({}),
                        )),
                        session,
                        &mock,
                    )
                    .await;
                }
                (
                    mock.events(),
                    first.stray_outcome_warned.load(Ordering::Relaxed),
                )
            }));
        });

        let (events, latched) = seen.expect("the body ran");
        assert!(latched, "the first refusal must raise the latch");
        assert_eq!(
            events.len(),
            3,
            "a refusal is an answer too — not swallowed: {events:?}"
        );
        for event in &events {
            match event {
                AgentEvent::Error { request_id, .. } => assert_eq!(*request_id, None),
                other => panic!("a refusal is an Error with no correlation key: {other:?}"),
            }
        }
        // The two assertions measure different things; with only one each has an escape route.
        // 1. Counting by wording pins that this arm emitted the warning.
        assert_eq!(
            warns_matching(&logged, STRAY_OUTCOME_WARN),
            2,
            "one line per connection so two for two connections: {logged:?}"
        );
        // 2. Counting in total catches the repeat branch being raised from `debug!` to `warn!` with the wording changed too (a wording-only
        //    count would pass vacuously).
        assert_eq!(
            total_warns(&logged),
            2,
            "the WARNs in this capture must be only the two first warnings: {logged:?}"
        );
    }

    /// A registration landing after cleanup (overlap basis = the `CommandRoster` header). Per-branch assertions are on the `command_roster`
    /// side; this checks that dispatch passes the connection id to that decision — otherwise this refusal never stands.
    #[tokio::test]
    async fn a_registration_dispatched_after_the_disconnect_is_refused() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = attached(&core, 1);
        core.commands().detach(1);
        let req = rid();

        core.dispatch(register(vec![decl("tab.create")], req), &session, &mock)
            .await;

        match mock.events().as_slice() {
            [AgentEvent::Error {
                request_id,
                message,
                ..
            }] => {
                assert_eq!(*request_id, Some(req));
                assert!(
                    message.starts_with(ErrorCode::Conflict.as_str()),
                    "the registration of a disconnected connection is a roster **state** refusal: {message}"
                );
            }
            other => panic!("a refusal is answered with an Error: {other:?}"),
        }
        assert!(
            core.commands().entries().is_empty(),
            "not a single name may get on"
        );
    }

    /// Do not swallow the failure — carry the code too. The code rides at the front of the wording because `AgentEvent::Error` has no
    /// typed code field (`CommandError` `Display`, `CODE: wording`); this guards only that the code does not vanish there. Removing the
    /// parsing cost would need a typed field (a wire contract change).
    #[tokio::test]
    async fn a_refused_registration_replies_with_the_error_code() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = attached(&core, 1);
        let req = rid();
        let too_long = CommandDecl {
            name: "n".repeat(Roster::MAX_NAME_BYTES + 1),
            help: "{}".to_string(),
            agent_args: Vec::new(),
            deadline: engram_dashboard_command::DeadlineClass::Ordinary,
        };

        core.dispatch(register(vec![too_long], req), &session, &mock)
            .await;

        match mock.events().as_slice() {
            [AgentEvent::Error {
                request_id,
                message,
                ..
            }] => {
                assert_eq!(*request_id, Some(req));
                assert!(
                    message.starts_with(ErrorCode::InvalidArgument.as_str()),
                    "the code is carried before the wording: {message}"
                );
            }
            other => panic!("a refusal is answered with an Error: {other:?}"),
        }
        assert!(
            core.commands().entries().is_empty(),
            "a failed registration does not touch the roster"
        );
    }

    /// The owner differs per connection, so another's live registration cannot be taken — also asserts two sessions see the same roster (if sharing breaks, conn 2 sees an empty roster and succeeds).
    #[tokio::test]
    async fn an_update_from_another_connection_cannot_take_a_live_name() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let first = attached(&core, 1);
        let second = attached(&core, 2);
        let mock = MockOutboundSink::new(tx.clone());
        core.dispatch(register(vec![decl("tab.create")], rid()), &first, &mock)
            .await;

        let intruder = MockOutboundSink::new(tx);
        let req = rid();
        core.dispatch(
            update(vec![decl("tab.create")], vec![], req),
            &second,
            &intruder,
        )
        .await;

        match intruder.events().as_slice() {
            [AgentEvent::Error {
                request_id,
                message,
                ..
            }] => {
                assert_eq!(*request_id, Some(req));
                assert!(
                    message.starts_with(ErrorCode::Conflict.as_str()),
                    "taking another's live registration is a roster **state** refusal: {message}"
                );
            }
            other => panic!("a refusal is answered with an Error: {other:?}"),
        }
        let entries = core.commands().entries();
        assert_eq!(
            entries.len(),
            1,
            "if a refusal ended after erasing another's name a live command would quietly vanish"
        );
        assert_eq!(
            entries[0].owner,
            first.owner_token(),
            "the connection attached first remains the owner"
        );
    }

    // ── ADR-0096: SetEnvelopeFormat ──────────────────────────────────────────────
    #[tokio::test]
    async fn set_envelope_format_acks_and_mutates_send_path_state() {
        let (core, control_registry) = test_core_with_control_registry();
        assert_eq!(
            control_registry.envelope_format(),
            CoreEnvelopeFormat::Xml,
            "the initial envelope format is xml (default, ADR-0103)"
        );
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let req = rid();
        core.dispatch(
            AgentCommand::SetEnvelopeFormat {
                format: WireEnvelopeFormat::Colon,
                request_id: req,
            },
            &session,
            &mock,
        )
        .await;
        match mock.events().as_slice() {
            [AgentEvent::Ack { request_id }] => assert_eq!(*request_id, req),
            other => panic!("SetEnvelopeFormat must return an Ack: {other:?}"),
        }
        assert_eq!(
            control_registry.envelope_format(),
            CoreEnvelopeFormat::Colon,
            "after dispatch the global envelope format state must change to colon (the send path reads this value)"
        );

        core.dispatch(
            AgentCommand::SetEnvelopeFormat {
                format: WireEnvelopeFormat::Xml,
                request_id: rid(),
            },
            &session,
            &mock,
        )
        .await;
        assert_eq!(
            control_registry.envelope_format(),
            CoreEnvelopeFormat::Xml,
            "switching back to xml is reflected too"
        );
    }

    // ── CreateProfile ────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn create_profile_returns_created() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let req = rid();
        core.dispatch(
            AgentCommand::CreateProfile {
                name: "p".into(),
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                extra_args: vec![],
                env: vec![],
                auto_restore: false,
                output_format: WireAgentOutputFormat::Terminal,
                backend: Some(WireBackendKind::Claude),
                request_id: req,
            },
            &session,
            &mock,
        )
        .await;
        match mock.events().as_slice() {
            [AgentEvent::Created { request_id, .. }] => assert_eq!(*request_id, req),
            other => panic!("expected Created: {other:?}"),
        }
        assert_eq!(
            core.manager.agent_snapshots().len(),
            1,
            "1 profile registered"
        );
    }

    // ── ADR-0044 M2: CreateProfile(output_format=StreamJson) ─────────────────────
    #[tokio::test]
    async fn create_profile_stream_json_stores_json_mode() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        core.dispatch(
            AgentCommand::CreateProfile {
                name: "json".into(),
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                extra_args: vec![],
                env: vec![],
                auto_restore: false,
                output_format: WireAgentOutputFormat::StreamJson,
                backend: Some(WireBackendKind::Claude),
                request_id: rid(),
            },
            &session,
            &mock,
        )
        .await;
        let profiles = core.manager.agent_snapshots();
        assert_eq!(profiles.len(), 1, "1 profile registered");
        assert!(
            matches!(
                &profiles[0].command,
                CoreSpawnCommand::Claude {
                    output_format: CoreAgentOutputFormat::StreamJson,
                    ..
                }
            ),
            "a profile made as StreamJson must be in json mode"
        );
    }

    // ── backend choice field (user decision 2026-09-07) ────────────────────────────────────
    //
    // Absence is an error, not a default — without these two items a revert to "claude if not chosen" would pass quietly (it shows only on screen as the wrong agent coming up).
    #[tokio::test]
    async fn create_profile_without_a_backend_is_refused() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let req = rid();
        core.dispatch(
            AgentCommand::CreateProfile {
                name: "no-backend".into(),
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                extra_args: vec![],
                env: vec![],
                auto_restore: false,
                output_format: WireAgentOutputFormat::Terminal,
                backend: None,
                request_id: req,
            },
            &session,
            &mock,
        )
        .await;
        match mock.events().as_slice() {
            [AgentEvent::Error {
                request_id,
                message,
                ..
            }] => {
                assert_eq!(*request_id, Some(req));
                assert!(
                    message.contains("backend"),
                    "must say which field: {message}"
                );
                assert!(
                    message.contains("claude") && message.contains("codex"),
                    "must say what to write: {message}"
                );
            }
            other => panic!("expected an Error: {other:?}"),
        }
        assert_eq!(
            core.manager.agent_snapshots().len(),
            0,
            "a refusal does not register"
        );
    }

    /// A nonexistent folder as cwd for the refusal-branch tests: that branch ignores cwd, and a regression opening it would end in an OS error instead of launching a real claude·codex
    /// (that OS-error path is unverified).
    fn missing_cwd() -> String {
        std::env::temp_dir()
            .join(format!("engram-no-such-dir-{}", uuid::Uuid::new_v4()))
            .to_string_lossy()
            .into_owned()
    }

    /// An empty field is refused with [`MISSING_BACKEND`] without asking the policy — a counting fake that closes every word measures
    /// 0 policy calls and the wording; asking the policy would hide the which-field-to-fill wording behind the policy refusal.
    #[tokio::test]
    async fn spawn_by_cwd_without_a_backend_is_refused() {
        // The policy field is a `fn` pointer, so the count lives here — process-global, as in the sibling's `CLOSED`:
        //   the creation body asks the policy on the blocking-pool thread, where a thread-local would not count.
        static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        fn counts_and_closes_everything(_: &str) -> Option<&'static str> {
            CALLS.fetch_add(1, Ordering::SeqCst);
            Some("The test closed every word.")
        }
        let (core, _rx) = test_core();
        let core = core.with_llm_policy(counts_and_closes_everything);
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let req = rid();
        core.dispatch(
            AgentCommand::SpawnByCwd {
                cwd: missing_cwd(),
                backend: None,
                request_id: req,
                deliver_notices: false,
            },
            &session,
            &mock,
        )
        .await;
        match mock.events().as_slice() {
            [AgentEvent::Error {
                request_id,
                message,
                ..
            }] => {
                assert_eq!(*request_id, Some(req));
                assert_eq!(message.as_str(), MISSING_BACKEND);
            }
            other => panic!("expected an Error: {other:?}"),
        }
        assert_eq!(
            CALLS.load(Ordering::SeqCst),
            0,
            "the policy was asked for an empty field"
        );
        assert_eq!(
            core.manager.list_agents().len(),
            0,
            "a refusal does not spawn"
        );
        // The counting side really counts — without this the 0 above is green even with a counter that never increments.
        assert!((core.llm_policy)("claude").is_some());
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    }

    /// Every wire backend and its word — the word comes from the function the refusal text names it with
    /// ([`wire_backend_word`]; the creation body asks the policy with the declared spelling, compared ignoring case).
    /// ★Filled by hand, but a new variant makes `slot`'s `match` stop the build here★ — add it to the array then. An
    /// edit that only leaves the array short is not caught by the assert below (the slot count comes from the same array).
    fn every_wire_backend() -> Vec<(WireBackendKind, String)> {
        const ALL: [WireBackendKind; 2] = [WireBackendKind::Claude, WireBackendKind::Codex];
        fn slot(kind: WireBackendKind) -> usize {
            match kind {
                WireBackendKind::Claude => 0,
                WireBackendKind::Codex => 1,
            }
        }
        let mut seen = [false; ALL.len()];
        let backends = ALL
            .into_iter()
            .map(|kind| {
                seen[slot(kind)] = true;
                (kind, wire_backend_word(kind))
            })
            .collect();
        assert!(seen.iter().all(|s| *s), "a slot passed by empty: {seen:?}");
        backends
    }

    /// All wire backends are declared in the policy table. A missing word closes fail-closed, but that is "not yet decided", not "decided
    /// to close". Measured here because agent (policy table) does not depend on protocol (wire vocabulary) — this handler sees both.
    #[test]
    fn every_wire_backend_declares_an_llm_policy() {
        use engram_dashboard_agent::commands::LLM_BACKEND_POLICY;
        for (_, word) in every_wire_backend() {
            assert!(
                LLM_BACKEND_POLICY
                    .iter()
                    .any(|policy| policy.word.eq_ignore_ascii_case(&word)),
                "no policy is declared for the wire backend '{word}'"
            );
        }
    }

    /// The production assembly plugs in the real policy — a missing wiring opens the wall silently (ADR-0279).
    /// Feeding a word outside the table is the core: today's table opens every wire word (`None`), so an "always open" fake and the real
    /// policy differ only on a word outside the table (closed fail-closed, `Some`).
    #[test]
    fn the_production_core_asks_the_agent_backend_policy() {
        use engram_dashboard_agent::commands::llm_creation_refusal;
        let (core, _rx) = test_core();
        let outside = "no-such-backend";
        assert!(
            llm_creation_refusal(outside).is_some(),
            "a word outside the table must be closed for this test to catch an 'always open' fake"
        );
        let mut words: Vec<String> = every_wire_backend()
            .into_iter()
            .map(|(_, word)| word)
            .collect();
        words.push(outside.to_string());
        for word in &words {
            assert_eq!(
                (core.llm_policy)(word),
                llm_creation_refusal(word),
                "'{word}'"
            );
        }
    }

    /// ★A backend the policy closes is refused with no profile, no roster row and no spawn★ — the invariant that the
    /// verdict comes before anything is registered (ADR-0279), measured at dispatch level. Today's production table
    /// closes nothing, so real input cannot reach this arm; a closing fake is plugged in (ADR-0012).
    /// ★Run per wire backend, with a fake that closes that one word only★ — one backend alone lets "only that backend
    /// asks the policy" pass green, and a fake that closes everything stays green when the arm asks the wrong word.
    #[tokio::test]
    async fn spawn_by_cwd_refuses_a_backend_the_policy_closes() {
        // The word the fake closes. A process-global, not a thread-local: the creation body asks the policy on the
        //   blocking-pool thread. The body asks with the declared spelling (`Codex`) and the refusal text names the
        //   wire word (`codex`), so the fake compares ignoring case.
        static CLOSED: StdMutex<String> = StdMutex::new(String::new());
        fn closes_the_chosen_word(word: &str) -> Option<&'static str> {
            CLOSED
                .lock()
                .unwrap()
                .eq_ignore_ascii_case(word)
                .then_some("The test closed this backend.")
        }
        for (kind, word) in every_wire_backend() {
            *CLOSED.lock().unwrap() = word.clone();
            let (core, _rx) = test_core();
            let core = core.with_llm_policy(closes_the_chosen_word);
            let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
            let mock = MockOutboundSink::new(tx);
            let session = ConnectionSession::new(1);
            let req = rid();
            core.dispatch(
                AgentCommand::SpawnByCwd {
                    cwd: missing_cwd(),
                    backend: Some(kind),
                    request_id: req,
                    deliver_notices: false,
                },
                &session,
                &mock,
            )
            .await;
            match mock.events().as_slice() {
                [AgentEvent::Error {
                    request_id,
                    message,
                    ..
                }] => {
                    assert_eq!(*request_id, Some(req), "{word}");
                    assert!(
                        !message.contains("unknown"),
                        "must differ from the typo net's wording: {message}"
                    );
                    // Exactly the text the bus caller receives — daemon wording, no prefix.
                    assert_eq!(
                        message.as_str(),
                        format!(
                            "backend '{word}' is a known word, but this surface does not create it now — The test closed this backend. Not spawned."
                        )
                    );
                }
                other => panic!("{word}: expected one Error: {other:?}"),
            }
            assert!(
                core.manager.agent_snapshots().is_empty(),
                "{word}: a refusal does not register a profile"
            );
            assert!(
                core.manager.list_agents().is_empty(),
                "{word}: a refusal does not spawn"
            );
        }
    }

    /// The chosen word goes all the way to the run command on disk — a break here launches claude when codex was chosen.
    /// The chosen mode rides the same chain: a default filled at this boundary would bring up an interactive TUI behind a node that chose "codex JSON", invisible until it behaves differently.
    #[tokio::test]
    async fn create_profile_with_codex_stores_the_codex_command() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        core.dispatch(
            AgentCommand::CreateProfile {
                name: "codex-p".into(),
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                extra_args: vec!["--foo".into()],
                env: vec![],
                auto_restore: false,
                output_format: WireAgentOutputFormat::StreamJson,
                backend: Some(WireBackendKind::Codex),
                request_id: rid(),
            },
            &session,
            &mock,
        )
        .await;
        let profiles = core.manager.agent_snapshots();
        assert_eq!(profiles.len(), 1);
        match &profiles[0].command {
            CoreSpawnCommand::Codex {
                extra_args,
                output_format,
            } => {
                assert_eq!(extra_args, &vec!["--foo".to_string()]);
                assert_eq!(
                    *output_format,
                    CoreAgentOutputFormat::StreamJson,
                    "the chosen mode must sit as is — if a default were filled Terminal would come out here"
                );
            }
            other => panic!("expected Codex: {other:?}"),
        }
    }

    /// The half the item above cannot measure — an unchosen `StreamJson` does not intervene (red if this boundary raised codex wholesale to
    /// app-server). Both must stand together, or fixing that one value stays green.
    #[tokio::test]
    async fn create_profile_with_codex_terminal_is_not_upgraded_to_app_server() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        core.dispatch(
            AgentCommand::CreateProfile {
                name: "codex-term".into(),
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                extra_args: vec![],
                env: vec![],
                auto_restore: false,
                output_format: WireAgentOutputFormat::Terminal,
                backend: Some(WireBackendKind::Codex),
                request_id: rid(),
            },
            &session,
            &mock,
        )
        .await;
        let profiles = core.manager.agent_snapshots();
        assert_eq!(profiles.len(), 1);
        match &profiles[0].command {
            CoreSpawnCommand::Codex { output_format, .. } => {
                assert_eq!(*output_format, CoreAgentOutputFormat::Terminal);
            }
            other => panic!("expected Codex: {other:?}"),
        }
    }

    /// The returning direction — the mode is carried when the roster goes out. No frontend code reads it today (nothing looks at
    /// `profile.command`), but dropping it would remove from the wire the means of telling a "codex JSON" node from an interactive TUI node.
    #[test]
    fn codex_spawn_command_carries_the_mode_back_to_wire() {
        for (core_mode, wire_mode) in [
            (
                CoreAgentOutputFormat::StreamJson,
                WireAgentOutputFormat::StreamJson,
            ),
            (
                CoreAgentOutputFormat::Terminal,
                WireAgentOutputFormat::Terminal,
            ),
        ] {
            let wire = spawn_command_to_wire(&CoreSpawnCommand::Codex {
                extra_args: vec!["--foo".into()],
                output_format: core_mode,
            });
            match wire {
                WireSpawnCommand::Codex {
                    extra_args,
                    output_format,
                } => {
                    assert_eq!(extra_args, vec!["--foo".to_string()]);
                    assert_eq!(output_format, wire_mode);
                }
                other => panic!("expected Codex: {other:?}"),
            }
        }
    }

    // ── StopDaemon (force=false, 0 active) ──────────────────────────────────────────
    #[tokio::test]
    async fn stop_daemon_no_active_closes_and_signals() {
        let (core, mut rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let flow = core
            .dispatch(
                AgentCommand::StopDaemon {
                    force: false,
                    kill_agents: true,
                    request_id: rid(),
                },
                &session,
                &mock,
            )
            .await;
        assert_eq!(flow, DispatchFlow::Close, "0 active → Close");
        assert!(
            matches!(mock.events().as_slice(), [AgentEvent::Ack { .. }]),
            "1 Ack"
        );
        // The shutdown runs in its own task — the signal follows it, not the handler's return.
        tokio::time::timeout(Duration::from_secs(10), rx.changed())
            .await
            .expect("the shutdown signal fires")
            .expect("the sender is alive");
        assert!(*rx.borrow_and_update());
    }

    /// TRD S21 trd-3-2-u5 §3-4: a client closing right after `StopDaemon` makes `net` abort the dispatch task — here the handler is dropped
    /// after one poll. The shutdown still runs and still signals.
    #[tokio::test]
    async fn stop_daemon_survives_a_dispatch_dropped_after_one_poll() {
        use futures_util::FutureExt;

        let (core, mut rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let polled_once = core
            .dispatch(
                AgentCommand::StopDaemon {
                    force: true,
                    kill_agents: true,
                    request_id: rid(),
                },
                &session,
                &mock,
            )
            .now_or_never();
        assert_eq!(
            polled_once,
            Some(DispatchFlow::Close),
            "the handler awaits nothing"
        );
        tokio::time::timeout(Duration::from_secs(10), rx.changed())
            .await
            .expect("the shutdown signal fires although the dispatch was dropped")
            .expect("the sender is alive");
        assert!(*rx.borrow_and_update());
    }

    /// TRD S21 trd-3-2-u5 §3-4: two back-to-back `StopDaemon`s — the second arrives before the first has signalled and starts no second shutdown.
    #[tokio::test]
    async fn a_stop_daemon_before_the_first_shutdown_signals_joins_it() {
        let (core, mut rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let stop = || AgentCommand::StopDaemon {
            force: true,
            kill_agents: true,
            request_id: rid(),
        };
        let first = core
            .dispatch(stop(), &ConnectionSession::new(1), &mock)
            .await;
        let second = core
            .dispatch(stop(), &ConnectionSession::new(2), &mock)
            .await;
        assert_eq!((first, second), (DispatchFlow::Close, DispatchFlow::Close));
        assert!(!*rx.borrow(), "the first shutdown had not signalled");
        assert_eq!(
            core.shutdowns_spawned.load(Ordering::SeqCst),
            1,
            "one shutdown_all"
        );
        tokio::time::timeout(Duration::from_secs(10), rx.changed())
            .await
            .expect("the shutdown signals")
            .expect("the sender is alive");
        assert!(*rx.borrow_and_update());
    }

    /// A `StopDaemon` once the daemon is stopping only joins — answers and closes but does not run `shutdown_all` or signal again. It comes
    /// after the first signalled, so this checks the latch, not an overlap with a running `shutdown_all`.
    // Known limit: the negative half waits a bounded real time; a second run slower than that would pass unseen.
    #[tokio::test]
    async fn a_stop_daemon_after_the_shutdown_signalled_only_answers_and_closes() {
        let (core, mut rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let stop = || AgentCommand::StopDaemon {
            force: true,
            kill_agents: true,
            request_id: rid(),
        };
        let first = core
            .dispatch(stop(), &ConnectionSession::new(1), &mock)
            .await;
        assert_eq!(first, DispatchFlow::Close);
        tokio::time::timeout(Duration::from_secs(10), rx.changed())
            .await
            .expect("the first shutdown signals")
            .expect("the sender is alive");
        rx.borrow_and_update();

        let second = core
            .dispatch(stop(), &ConnectionSession::new(2), &mock)
            .await;
        assert_eq!(
            second,
            DispatchFlow::Close,
            "the joiner's connection still ends"
        );
        assert_eq!(
            mock.events()
                .iter()
                .filter(|e| matches!(e, AgentEvent::Ack { .. }))
                .count(),
            2,
            "both get their Ack"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(500), rx.changed())
                .await
                .is_err(),
            "a joiner must not run the shutdown again"
        );
    }

    // ── kind_to_action mapping (all 3 variants) ──────────────────────────────────────
    #[test]
    fn kind_to_action_maps_all_variants() {
        assert_eq!(
            kind_to_action(ReplayKind::FromOldest),
            SubscribeAction::Reset
        );
        assert_eq!(
            kind_to_action(ReplayKind::Truncated),
            SubscribeAction::TruncatedReplay
        );
        assert_eq!(kind_to_action(ReplayKind::Resumed), SubscribeAction::Resume);
    }

    // ── core→wire AgentInfo conversion roundtrip ───────────────────────────────────────
    #[test]
    fn core_agent_info_converts_to_wire() {
        use engram_dashboard_agent::types::{
            AgentInfo as Ci, Capabilities, ControlCaps, InputCaps, ModelCaps, OutputCaps,
            SessionCaps,
        };
        let core = Ci {
            id: uuid::Uuid::new_v4(),
            name: "t".into(),
            cwd: "/tmp".into(),
            status: CoreStatus::Running,
            cols: 80,
            rows: 24,
            epoch: 3,
            capabilities: Capabilities {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: true,
                    structured: false,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: true,
                    interrupt: true,
                    cancel: false,
                    graceful_shutdown: true,
                },
                session: SessionCaps {
                    resume: true,
                    snapshot: true,
                    cwd_env: true,
                },
                model: ModelCaps {
                    select: false,
                    temperature: false,
                    max_tokens: false,
                },
            },
            reads_messages: true,
        };
        let wire = core_agents_to_wire(vec![core.clone()]);
        assert_eq!(wire.len(), 1, "conversion succeeded (JSON shapes match)");
        assert_eq!(wire[0].name, "t");
        assert_eq!(wire[0].epoch, 3);
    }

    // ── (M3) does every core::AgentStatus variant roundtrip to wire ────────────────
    #[test]
    fn all_core_status_variants_roundtrip_to_wire() {
        use engram_dashboard_agent::types::{
            AgentInfo as Ci, Capabilities, ControlCaps, InputCaps, ModelCaps, OutputCaps,
            SessionCaps,
        };
        use engram_dashboard_protocol::AgentStatus as WireStatus;

        let caps = Capabilities {
            input: InputCaps {
                raw: true,
                message: false,
                attachment: false,
            },
            output: OutputCaps {
                terminal_bytes: true,
                structured: false,
                markdown: false,
                tool_events: false,
                usage: false,
            },
            control: ControlCaps {
                resize: true,
                interrupt: true,
                cancel: false,
                graceful_shutdown: true,
            },
            session: SessionCaps {
                resume: true,
                snapshot: true,
                cwd_env: true,
            },
            model: ModelCaps {
                select: false,
                temperature: false,
                max_tokens: false,
            },
        };

        // (core status, expected wire status) pairs — all variants.
        let cases: Vec<(CoreStatus, WireStatus)> = vec![
            (CoreStatus::Running, WireStatus::Running),
            (CoreStatus::Exiting, WireStatus::Exiting),
            (
                CoreStatus::Exited { code: Some(0) },
                WireStatus::Exited { code: Some(0) },
            ),
            (
                CoreStatus::Exited { code: None },
                WireStatus::Exited { code: None },
            ),
            (
                CoreStatus::Failed {
                    message: "boom".into(),
                },
                WireStatus::Failed {
                    message: "boom".into(),
                },
            ),
            (CoreStatus::Killed, WireStatus::Killed),
        ];

        for (core_status, expected_wire) in cases {
            let core = Ci {
                id: uuid::Uuid::new_v4(),
                name: "v".into(),
                cwd: "/tmp".into(),
                status: core_status.clone(),
                cols: 80,
                rows: 24,
                epoch: 0,
                capabilities: caps.clone(),
                reads_messages: true,
            };
            let wire = core_agents_to_wire(vec![core]);
            assert_eq!(
                wire.len(),
                1,
                "variant {core_status:?} was dropped in core→wire (tag/field mismatch)"
            );
            assert_eq!(
                wire[0].status, expected_wire,
                "variant {core_status:?} was converted to a different wire status"
            );
            let direct = core_status_to_wire(core_status.clone());
            assert_eq!(
                direct, expected_wire,
                "the direct conversion path must match too"
            );
        }
    }

    // ── (application 1) every core::RestoreOutcome variant → explicit wire conversion ──────────────────────
    #[test]
    fn all_restore_outcomes_convert_to_wire() {
        use engram_dashboard_agent::profile::RestoreOutcome as Co;
        use engram_dashboard_protocol::RestoreOutcome as Wo;

        let old = uuid::Uuid::new_v4();
        let new = uuid::Uuid::new_v4();

        assert_eq!(restore_outcome_to_wire(&Co::Resumed), Wo::Resumed);
        assert_eq!(restore_outcome_to_wire(&Co::Started), Wo::Started);

        match restore_outcome_to_wire(&Co::FreshFallback {
            old_sid: Some(old),
            new_sid: new,
            reason: "r".into(),
        }) {
            Wo::FreshFallback {
                old_sid,
                new_sid,
                reason,
            } => {
                assert_eq!(old_sid, Some(old.to_string()), "old_sid Uuid→String");
                assert_eq!(new_sid, new.to_string(), "new_sid Uuid→String");
                assert_eq!(reason, "r");
            }
            other => panic!("expected FreshFallback, got {other:?}"),
        }

        match restore_outcome_to_wire(&Co::FreshFallback {
            old_sid: None,
            new_sid: new,
            reason: "r2".into(),
        }) {
            Wo::FreshFallback { old_sid, .. } => assert_eq!(old_sid, None, "None preserved"),
            other => panic!("expected FreshFallback, got {other:?}"),
        }

        assert_eq!(
            restore_outcome_to_wire(&Co::Blocked { reason: "b".into() }),
            Wo::Blocked { reason: "b".into() }
        );
        assert_eq!(
            restore_outcome_to_wire(&Co::Failed { reason: "f".into() }),
            Wo::Failed { reason: "f".into() }
        );
    }

    // ── S15 B7: output_event_to_wire ─────────────────────────────────────────────
    #[tokio::test]
    async fn output_event_to_wire_maps_all_variants_preserving_fields() {
        use engram_dashboard_protocol::StructuredEvent as W;

        assert_eq!(
            output_event_to_wire(&CoreOutputEvent::TextDelta {
                text: "hi".into(),
                turn_id: Some("t1".into()),
                message_id: None,
            }),
            Some(W::TextDelta {
                text: "hi".into(),
                turn_id: Some("t1".into()),
                message_id: None,
            })
        );

        assert_eq!(
            output_event_to_wire(&CoreOutputEvent::ToolCall {
                name: "read".into(),
                args_json: r#"{"p":1}"#.into(),
                id: Some("c1".into()),
                turn_id: None,
                message_id: Some("m1".into()),
                category: CoreToolCategory::Read,
            }),
            Some(W::ToolCall {
                name: "read".into(),
                args_json: r#"{"p":1}"#.into(),
                id: Some("c1".into()),
                turn_id: None,
                message_id: Some("m1".into()),
                category: Some(WireToolCategory::Read),
            })
        );

        assert_eq!(
            output_event_to_wire(&CoreOutputEvent::Usage {
                input_tokens: 7,
                output_tokens: 11,
                turn_id: Some("t2".into()),
            }),
            Some(W::Usage {
                input_tokens: 7,
                output_tokens: 11,
                turn_id: Some("t2".into()),
            })
        );

        assert_eq!(
            output_event_to_wire(&CoreOutputEvent::MessageDone {
                turn_id: Some("t3".into()),
                message_id: Some("m2".into()),
            }),
            Some(W::MessageDone {
                turn_id: Some("t3".into()),
                message_id: Some("m2".into()),
            })
        );

        assert_eq!(
            output_event_to_wire(&CoreOutputEvent::Error("boom".into())),
            Some(W::Error {
                message: "boom".into()
            })
        );

        assert_eq!(
            output_event_to_wire(&CoreOutputEvent::Structured {
                kind: "k".into(),
                json: r#"{"a":1}"#.into(),
            }),
            Some(W::Structured {
                kind: "k".into(),
                json: r#"{"a":1}"#.into(),
            })
        );

        assert_eq!(
            output_event_to_wire(&CoreOutputEvent::QueuedInput(
                CoreQueuedInputEvent::Queued {
                    id: "u1".into(),
                    text: "hi".into(),
                }
            )),
            Some(W::QueuedInput {
                op: WireQueuedInputEvent::Queued {
                    id: "u1".into(),
                    text: "hi".into(),
                }
            })
        );

        assert_eq!(
            output_event_to_wire(&CoreOutputEvent::TerminalBytes(vec![1, 2, 3])),
            None,
            "TerminalBytes (tag0 only) is not mapped to a wire StructuredEvent"
        );
    }

    /// No tool kind folds, all cross over — the bundle head counts per kind. The domain side always carries and the wire side is always
    /// `Some` (no field = only an old daemon).
    // ADR-0239
    #[tokio::test]
    async fn tool_call_maps_every_category_one_to_one() {
        use engram_dashboard_protocol::StructuredEvent as W;
        for (core, want) in [
            (CoreToolCategory::Read, WireToolCategory::Read),
            (CoreToolCategory::Search, WireToolCategory::Search),
            (CoreToolCategory::List, WireToolCategory::List),
            (CoreToolCategory::Edit, WireToolCategory::Edit),
            (CoreToolCategory::Command, WireToolCategory::Command),
            (CoreToolCategory::Web, WireToolCategory::Web),
            (CoreToolCategory::Agent, WireToolCategory::Agent),
            (CoreToolCategory::Mcp, WireToolCategory::Mcp),
            (CoreToolCategory::Other, WireToolCategory::Other),
        ] {
            match output_event_to_wire(&CoreOutputEvent::ToolCall {
                name: "t".into(),
                args_json: "{}".into(),
                id: None,
                turn_id: None,
                message_id: None,
                category: core,
            }) {
                Some(W::ToolCall { category, .. }) => assert_eq!(category, Some(want), "{core:?}"),
                other => panic!("expected ToolCall, got {other:?}"),
            }
        }
    }

    /// No tool-end outcome folds — a denial dropping to a failure falsifies "N errors"; our refusal dropping to a vendor denial makes the
    /// reason line wrong. `id` goes as is (the accumulator finds the row with it).
    // ADR-0241
    #[tokio::test]
    async fn tool_result_maps_every_outcome_one_to_one() {
        use engram_dashboard_protocol::StructuredEvent as W;
        for (core, want) in [
            (CoreToolOutcome::Completed, WireToolOutcome::Completed),
            (CoreToolOutcome::Failed, WireToolOutcome::Failed),
            (CoreToolOutcome::Declined, WireToolOutcome::Declined),
            (CoreToolOutcome::Refused, WireToolOutcome::Refused),
        ] {
            assert_eq!(
                output_event_to_wire(&CoreOutputEvent::ToolResult {
                    id: "i-1".into(),
                    outcome: core,
                }),
                Some(W::ToolResult {
                    id: "i-1".into(),
                    outcome: want,
                }),
                "{core:?}"
            );
        }
    }

    /// No roster event · cause folds — the accumulator reduces like the roster only if the event sequence goes as is (one cause dropping
    /// to another diverges revive and bubble clearing).
    // ADR-0231
    #[tokio::test]
    async fn queued_input_maps_every_event_and_cause_without_collapsing_any() {
        use engram_dashboard_protocol::StructuredEvent as W;
        let wire = |op| match output_event_to_wire(&CoreOutputEvent::QueuedInput(op)) {
            Some(W::QueuedInput { op }) => op,
            other => panic!("expected QueuedInput, got {other:?}"),
        };
        let id = || "u1".to_owned();
        assert_eq!(
            wire(CoreQueuedInputEvent::CancelRequested { id: id() }),
            WireQueuedInputEvent::CancelRequested { id: id() }
        );
        for removed in [true, false] {
            assert_eq!(
                wire(CoreQueuedInputEvent::CancelAnswered { id: id(), removed }),
                WireQueuedInputEvent::CancelAnswered { id: id(), removed }
            );
        }
        assert_eq!(
            wire(CoreQueuedInputEvent::CancelFailed { id: id() }),
            WireQueuedInputEvent::CancelFailed { id: id() }
        );
        for sent in [true, false] {
            assert_eq!(
                wire(CoreQueuedInputEvent::HandedOver { id: id(), sent }),
                WireQueuedInputEvent::HandedOver { id: id(), sent }
            );
        }
        assert_eq!(
            wire(CoreQueuedInputEvent::Delivered { id: id() }),
            WireQueuedInputEvent::Delivered { id: id() }
        );
        for (core, want) in [
            (CoreDropCause::Withdrawn, WireDropCause::Withdrawn),
            (CoreDropCause::Interrupted, WireDropCause::Interrupted),
            (CoreDropCause::AgentEnded, WireDropCause::AgentEnded),
            (CoreDropCause::Rejected, WireDropCause::Rejected),
            (CoreDropCause::Unknown, WireDropCause::Unknown),
        ] {
            assert_eq!(
                wire(CoreQueuedInputEvent::Dropped {
                    id: id(),
                    cause: core,
                }),
                WireQueuedInputEvent::Dropped {
                    id: id(),
                    cause: want,
                }
            );
        }
        assert_eq!(
            wire(CoreQueuedInputEvent::AckUnavailable {
                delivered: vec![
                    CoreDeliveredCopy {
                        id: "a".into(),
                        text: "one".into(),
                    },
                    CoreDeliveredCopy {
                        id: "b".into(),
                        text: "two".into(),
                    },
                ],
            }),
            WireQueuedInputEvent::AckUnavailable {
                delivered: vec![
                    WireDeliveredCopy {
                        id: "a".into(),
                        text: "one".into(),
                    },
                    WireDeliveredCopy {
                        id: "b".into(),
                        text: "two".into(),
                    },
                ],
            }
        );
    }

    /// No four-way outcome folds — two dropping to one value would lose the means to tell "aborted" from "done", or "unknown" from "done".
    ///
    /// This does not measure that no backend vocabulary goes up on the wire: `detail` carries the other side's sentence as is (that field's
    /// contract). Only the discriminator is neutral, pinned by the protocol golden (ADR-0004).
    #[tokio::test]
    async fn turn_end_maps_every_outcome_without_collapsing_any() {
        use engram_dashboard_protocol::StructuredEvent as W;

        let cases = [
            (CoreTurnOutcome::Completed, WireTurnOutcome::Completed),
            (CoreTurnOutcome::Interrupted, WireTurnOutcome::Interrupted),
            (CoreTurnOutcome::Unknown, WireTurnOutcome::Unknown),
            (
                CoreTurnOutcome::Failed {
                    detail: Some("model refused".into()),
                },
                WireTurnOutcome::Failed {
                    detail: Some("model refused".into()),
                },
            ),
            (
                CoreTurnOutcome::Failed { detail: None },
                WireTurnOutcome::Failed { detail: None },
            ),
        ];
        for (core, wire) in cases {
            assert_eq!(
                output_event_to_wire(&CoreOutputEvent::TurnEnd {
                    turn_id: Some("t9".into()),
                    outcome: core.clone(),
                }),
                Some(W::TurnEnd {
                    turn_id: Some("t9".into()),
                    outcome: wire,
                }),
                "{core:?}"
            );
        }
    }

    // ══════════════════════════════════════════════════════════════════════════════════════════
    // Review fix D1 — the wiring integration test of the `DeleteProfile` delete-cleanup hook (ADR-0116 decision 3)
    // ══════════════════════════════════════════════════════════════════════════════════════════

    /// Daemon integration, since a fake-port unit test cannot catch it: the defect was in the order · axis of two wirings — (1) the hook must
    /// take the snapshot (name for logs) before removing the profile, (2) the trigger gate must hang on the id. A kernel unit test scripts
    /// the name · gate itself. Dispatch runs here with real profiles · spawn · the real `ManagerDeliveryPort`. Also checks that deleting a
    /// profile renamed while dormant sweeps by id the items piled under the old name (ADR-0309 decision 6).
    #[tokio::test]
    async fn deleting_a_renamed_profile_whose_session_is_live_keeps_its_mail_and_contracts() {
        use engram_dashboard_messaging::envelope::Entrance;
        use engram_dashboard_messaging::service::SendMeta;
        use engram_dashboard_messaging::SenderIdentity;

        let (core, messaging) = test_core_with_messaging();
        let (tx, _rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(64);
        let sink = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);

        // (1) 2 profiles — boss (soon a live session) · sleepy (stays dormant = the parking recipient).
        let mk = |name: &str| {
            engram_dashboard_agent::profile::AgentProfile::new(
                name.into(),
                engram_dashboard_agent::profile::AgentCommand::Shell {
                    program: default_shell().to_string(),
                    args: vec![],
                },
                std::env::temp_dir(),
                vec![],
                false,
            )
        };
        let boss = mk("boss-raw");
        let sleepy = {
            let mut p = mk("sleepy-raw");
            p.display_name = Some("sleepy".into());
            p
        };
        core.manager
            .try_create_agent(boss.clone())
            .expect("registration succeeded");
        core.manager
            .try_create_agent(sleepy.clone())
            .expect("registration succeeded");

        // (2) rename (RenameProfile)
        core.dispatch(
            AgentCommand::RenameProfile {
                profile_id: boss.id,
                name: Some("boss".into()),
                request_id: engram_dashboard_protocol::RequestId(uuid::Uuid::new_v4()),
            },
            &session,
            &sink,
        )
        .await;
        assert_eq!(
            core.manager
                .agent_snapshot(boss.id)
                .expect("profile")
                .canonical_name_when_live(),
            "boss",
            "the rename changed the canonical name (this equation is this test's premise)"
        );

        // (3) while boss is dormant: 1 item parked to boss, and 1 contract where boss is the requester.
        let sender = SenderIdentity {
            peer_id: uuid::Uuid::new_v4(),
            epoch: 0,
        };
        let req = SendMeta {
            request: true,
            reply_by: None,
            reply_by_raw: None,
            reply_to: None,
            to_attr: None,
        };
        let rows = messaging
            .handle_send(
                "m-in",
                sender,
                "outsider",
                &["boss".to_string()],
                "pile it up",
                Entrance::Cli,
                &SendMeta::default(),
            )
            .expect("row response");
        assert_eq!(
            rows[0].status,
            engram_dashboard_messaging::service::SendStatus::Pending,
            "parked to the dormant profile name: {rows:?}"
        );
        assert_eq!(messaging.parked_len(boss.id), 1);
        messaging
            .handle_send(
                "m-out",
                SenderIdentity {
                    peer_id: boss.id,
                    epoch: 0,
                },
                "boss",
                &["sleepy".to_string()],
                "do it",
                Entrance::Cli,
                &req,
            )
            .expect("row response");
        assert_eq!(
            messaging.contract_outcome_for_test("m-out", sleepy.id),
            Some("awaiting_reply"),
            "the contract where boss is the requester opened"
        );

        // (4) boss spawn — now a live session.
        core.dispatch(
            AgentCommand::Spawn {
                profile_id: boss.id,
                request_id: engram_dashboard_protocol::RequestId(uuid::Uuid::new_v4()),
            },
            &session,
            &sink,
        )
        .await;
        assert!(
            core.manager.list_agents().iter().any(|a| a.id == boss.id),
            "the spawned session must be on the list (this test's premise)"
        );

        // (5) DeleteProfile — real dispatch. The session does not die (kill is a separate command).
        core.dispatch(
            AgentCommand::DeleteProfile {
                profile_id: boss.id,
                request_id: engram_dashboard_protocol::RequestId(uuid::Uuid::new_v4()),
            },
            &session,
            &sink,
        )
        .await;

        // (6) protection assertions.
        assert_eq!(
            messaging.parked_len(boss.id),
            1,
            "★D1★ the live session's parked mail died in the delete cleanup (if the gate were on the name axis it would be 0 here)"
        );
        assert_eq!(
            messaging.contract_outcome_for_test("m-out", sleepy.id),
            Some("awaiting_reply"),
            "★D1★ the contract where the live session is the requester was terminated as failed (an ADR-0118 decision 2 violation)"
        );

        // (7) The opposite direction: the snapshot must be taken before removal for the hook to run. Park mail before sleepy and a contract
        //   where sleepy is the requester, rename sleepy to drowsy, and the new profile heir takes the name sleepy. Deletion must sweep by id —
        //   the hook passes drowsy, so by-name misses the contract opened as sleepy, and the old name kills heir's part (ADR-0309 decision 6).
        let helper = {
            let mut p = mk("helper-raw");
            p.display_name = Some("helper".into());
            p
        };
        core.manager
            .try_create_agent(helper.clone())
            .expect("registration succeeded");
        let rows2 = messaging
            .handle_send(
                "m-sleep",
                sender,
                "outsider",
                &["sleepy".to_string()],
                "to the dormant one",
                Entrance::Cli,
                &SendMeta::default(),
            )
            .expect("row response");
        assert_eq!(
            rows2[0].status,
            engram_dashboard_messaging::service::SendStatus::Pending
        );
        messaging
            .handle_send(
                "m-sleep-req",
                SenderIdentity {
                    peer_id: sleepy.id,
                    epoch: 0,
                },
                "sleepy",
                &["helper".to_string()],
                "do it",
                Entrance::Cli,
                &req,
            )
            .expect("row response");
        assert_eq!(
            messaging.contract_outcome_for_test("m-sleep-req", helper.id),
            Some("awaiting_reply"),
            "the contract where sleepy is the requester opened"
        );
        core.dispatch(
            AgentCommand::RenameProfile {
                profile_id: sleepy.id,
                name: Some("drowsy".into()),
                request_id: engram_dashboard_protocol::RequestId(uuid::Uuid::new_v4()),
            },
            &session,
            &sink,
        )
        .await;
        assert_eq!(
            core.manager
                .agent_snapshot(sleepy.id)
                .expect("profile")
                .canonical_name_when_live(),
            "drowsy",
            "renamed while dormant (this step's premise)"
        );
        let heir = {
            let mut p = mk("heir-raw");
            p.display_name = Some("sleepy".into());
            p
        };
        core.manager
            .try_create_agent(heir.clone())
            .expect("registration succeeded");
        let rows3 = messaging
            .handle_send(
                "m-heir",
                sender,
                "outsider",
                &["sleepy".to_string()],
                "to the new sleepy",
                Entrance::Cli,
                &SendMeta::default(),
            )
            .expect("row response");
        assert_eq!(
            rows3[0].status,
            engram_dashboard_messaging::service::SendStatus::Pending
        );
        assert_eq!(
            messaging.parked_len(heir.id),
            1,
            "new mail to the old name is entrusted to that name's new owner"
        );
        core.dispatch(
            AgentCommand::DeleteProfile {
                profile_id: sleepy.id,
                request_id: engram_dashboard_protocol::RequestId(uuid::Uuid::new_v4()),
            },
            &session,
            &sink,
        )
        .await;
        assert_eq!(
            messaging.parked_len(sleepy.id),
            0,
            "deleting a dormant profile must trigger the cleanup (evidence that the snapshot was taken before removal)"
        );
        let view = messaging
            .message_state("m-sleep", std::time::Instant::now())
            .expect("lookup");
        assert_eq!(
            view.rows[0].code,
            Some("RECIPIENT_DELETED"),
            "the ledger end point + reason code remain (not quietly discarded): {view:?}"
        );
        assert_eq!(
            messaging.contract_outcome_for_test("m-sleep-req", helper.id),
            Some("reply_failed"),
            "a contract opened under the pre-rename name is also closed by id"
        );
        assert_eq!(
            messaging.parked_len(heir.id),
            1,
            "the new owner's part under the old name stays as is"
        );

        core.manager.kill_agent(boss.id).ok();
    }

    // ══════════════════════════════════════════════════════════════════════════════════════════
    // Store refusal — the WS caller-side change (ADR-0291 R17 · D3 · E2)
    // ══════════════════════════════════════════════════════════════════════════════════════════

    /// A shell profile that fails immediately when run (a nonexistent program) — if the store refusal regressed and reached launching, nothing comes up.
    fn unlaunchable_profile(name: &str) -> CoreProfile {
        let mut profile = CoreProfile::new(
            format!("{name}-raw"),
            CoreSpawnCommand::Shell {
                program: format!("engram-not-a-real-program-{}.exe", uuid::Uuid::new_v4()),
                args: vec![],
            },
            std::env::temp_dir(),
            vec![],
            false,
        );
        profile.display_name = Some(name.to_string());
        profile
    }

    /// A `DeleteProfile` the store did not accept answers an error and emits neither the cleanup hook nor a broadcast (D3) — cleaning up a
    ///   profile that was not deleted breaks live messaging (ADR-0116 decision 3). The hook not having run shows as the parked mail
    ///   remaining; the control (saved: the same request cleans that mail) shows the observation is not vacuous.
    // ADR-0291 R17 (D3)
    #[tokio::test]
    async fn a_delete_profile_the_store_refuses_answers_an_error_without_the_hook_or_a_broadcast() {
        use engram_dashboard_messaging::envelope::Entrance;
        use engram_dashboard_messaging::service::{SendMeta, SendStatus};
        use engram_dashboard_messaging::SenderIdentity;

        let (core, fanout, refusing) = test_core_refusable();
        let messaging = Arc::new(crate::messaging_host::messaging_for_manager(
            core.manager.clone(),
            core.control_registry.clone(),
        ));
        core.messaging.set(messaging.clone());
        let (tx, _rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(64);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let sleepy = unlaunchable_profile("sleepy");
        core.manager
            .try_create_agent(sleepy.clone())
            .expect("registered when saved");
        let rows = messaging
            .handle_send(
                "m-park",
                SenderIdentity {
                    peer_id: uuid::Uuid::new_v4(),
                    epoch: 0,
                },
                "outsider",
                &["sleepy".to_string()],
                "pile it up",
                Entrance::Cli,
                &SendMeta::default(),
            )
            .expect("row response");
        assert_eq!(
            rows[0].status,
            SendStatus::Pending,
            "premise: parked to the dormant profile"
        );

        refusing.store(STORE_REFUSES, Ordering::SeqCst);
        let before = fanout.texts().len();
        let req = rid();
        core.dispatch(
            AgentCommand::DeleteProfile {
                profile_id: sleepy.id,
                request_id: req,
            },
            &session,
            &mock,
        )
        .await;

        match mock.events().as_slice() {
            [AgentEvent::Error {
                request_id: Some(r),
                message,
                ..
            }] => {
                assert_eq!(*r, req);
                assert!(
                    message.starts_with("CONFLICT:") && message.contains(PROFILES_FILE),
                    "{message}"
                );
            }
            other => panic!("expected 1 Error, not an Ack: {other:?}"),
        }
        assert!(
            core.manager.agent_snapshot(sleepy.id).is_some(),
            "it was not deleted"
        );
        assert_eq!(
            messaging.parked_len(sleepy.id),
            1,
            "the cleanup hook did not run"
        );
        assert_eq!(fanout.texts().len(), before, "broadcast 0");

        // Control — when saved the same request deletes and the hook cleans up that mail.
        refusing.store(STORE_SAVES, Ordering::SeqCst);
        core.dispatch(
            AgentCommand::DeleteProfile {
                profile_id: sleepy.id,
                request_id: rid(),
            },
            &session,
            &mock,
        )
        .await;
        assert!(
            matches!(mock.events().last(), Some(AgentEvent::Ack { .. })),
            "{:?}",
            mock.events()
        );
        assert_eq!(
            messaging.parked_len(sleepy.id),
            0,
            "when saved the hook runs"
        );
        assert!(
            fanout.texts().len() > before,
            "when saved the roster is pushed"
        );
    }

    /// Every roster change the store did not accept answers with an error and emits no broadcast (D3). The wording names the code
    ///   (`CONFLICT`, same split as the bus) and the file (the profile file).
    // ADR-0291 R17 (D3)
    #[tokio::test]
    async fn every_ws_roster_change_the_store_refuses_answers_an_error_and_broadcasts_nothing() {
        let (core, fanout, refusing) = test_core_refusable();
        let session = ConnectionSession::new(1);
        let alpha = unlaunchable_profile("alpha");
        let lead = unlaunchable_profile("lead");
        core.manager
            .try_create_agent(alpha.clone())
            .expect("registered when saved");
        core.manager
            .try_create_agent(lead.clone())
            .expect("registered when saved");
        let profiles = || {
            let mut rows: Vec<_> = core
                .manager
                .agent_snapshots()
                .into_iter()
                .map(|p| (p.id, p.display_name, p.parent_id, p.auto_restore))
                .collect();
            rows.sort();
            rows
        };
        let profiles_before = profiles();
        refusing.store(STORE_REFUSES, Ordering::SeqCst);
        let before = fanout.texts().len();
        let temp = std::env::temp_dir().to_string_lossy().into_owned();

        let cases = [
            (
                "CreateProfile",
                PROFILES_FILE,
                AgentCommand::CreateProfile {
                    name: "new".into(),
                    cwd: temp,
                    extra_args: vec![],
                    env: vec![],
                    auto_restore: false,
                    output_format: WireAgentOutputFormat::Terminal,
                    backend: Some(WireBackendKind::Claude),
                    request_id: rid(),
                },
            ),
            (
                "RenameProfile",
                PROFILES_FILE,
                AgentCommand::RenameProfile {
                    profile_id: alpha.id,
                    name: Some("beta".into()),
                    request_id: rid(),
                },
            ),
            (
                "ReparentProfile",
                PROFILES_FILE,
                AgentCommand::ReparentProfile {
                    child_id: alpha.id,
                    parent_id: Some(lead.id),
                    request_id: rid(),
                },
            ),
            (
                "SetProfileAutoRestore",
                PROFILES_FILE,
                AgentCommand::SetProfileAutoRestore {
                    profile_id: alpha.id,
                    auto_restore: true,
                    request_id: rid(),
                },
            ),
        ];
        for (label, file, cmd) in cases {
            let (tx, _rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
            let mock = MockOutboundSink::new(tx);
            core.dispatch(cmd, &session, &mock).await;
            match mock.events().as_slice() {
                [AgentEvent::Error {
                    request_id: Some(_),
                    message,
                    ..
                }] => assert!(
                    message.starts_with("CONFLICT:") && message.contains(file),
                    "{label}: {message}"
                ),
                other => panic!("{label}: expected 1 Error: {other:?}"),
            }
        }

        assert_eq!(fanout.texts().len(), before, "broadcast 0");
        assert_eq!(profiles(), profiles_before, "roster unchanged");
    }

    /// WS activation pushes the profile list even on a store refusal (E2) — the "last failure" axis reaches the screen only through
    ///   the profile list (ADR-0172). The answer is an error carrying the code.
    // ADR-0291 R17 (E2)
    #[tokio::test]
    async fn a_ws_spawn_profile_the_store_refuses_answers_conflict_and_still_announces_the_list() {
        let (core, fanout, refusing) = test_core_refusable();
        let (tx, _rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let profile = unlaunchable_profile("waker");
        core.manager
            .try_create_agent(profile.clone())
            .expect("registered when saved");
        refusing.store(STORE_REFUSES, Ordering::SeqCst);
        let before = fanout.texts().len();

        core.dispatch(
            AgentCommand::SpawnProfile {
                profile_id: profile.id,
                resume: false,
                request_id: rid(),
            },
            &session,
            &mock,
        )
        .await;

        match mock.events().as_slice() {
            [AgentEvent::Error { message, .. }] => assert!(
                message.starts_with("CONFLICT:") && message.contains(PROFILES_FILE),
                "{message}"
            ),
            other => panic!("expected 1 Error: {other:?}"),
        }
        assert!(core.manager.list_agents().is_empty(), "nothing came up");
        assert_eq!(
            fanout.texts().len(),
            before + 1,
            "pushed once regardless of the outcome"
        );
        assert_eq!(
            core.manager
                .agent_snapshot(profile.id)
                .and_then(|p| p.last_failure),
            Some(CoreFailureKind::Other)
        );
    }

    /// WS `Spawn{profile_id}` is also `CONFLICT` on a store refusal with nothing coming up — same activation entrance, so same answer · notification as `SpawnProfile`.
    // ADR-0291 R17 (E2)
    #[tokio::test]
    async fn a_ws_spawn_by_profile_id_the_store_refuses_answers_conflict_and_still_announces_the_list(
    ) {
        let (core, fanout, refusing) = test_core_refusable();
        let (tx, _rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let profile = unlaunchable_profile("fresh-waker");
        core.manager
            .try_create_agent(profile.clone())
            .expect("registered when saved");
        refusing.store(STORE_REFUSES, Ordering::SeqCst);
        let before = fanout.texts().len();

        core.dispatch(
            AgentCommand::Spawn {
                profile_id: profile.id,
                request_id: rid(),
            },
            &session,
            &mock,
        )
        .await;

        match mock.events().as_slice() {
            [AgentEvent::Error { message, .. }] => assert!(
                message.starts_with("CONFLICT:")
                    && message.contains(PROFILES_FILE)
                    && message.contains("restart the daemon"),
                "{message}"
            ),
            other => panic!("expected 1 Error: {other:?}"),
        }
        assert!(core.manager.list_agents().is_empty(), "nothing came up");
        assert_eq!(
            fanout.texts().len(),
            before + 1,
            "pushed once regardless of the outcome"
        );
    }

    /// WS `SpawnByCwd` is `CONFLICT` on a store refusal with neither registration nor launching (an ad-hoc profile is registered before launching).
    ///   A nonexistent cwd ensures no process stands if the refusal regressed and reached launching.
    // ADR-0291 R17
    #[tokio::test]
    async fn a_ws_spawn_by_cwd_the_store_refuses_answers_conflict_and_registers_nothing() {
        let (core, _fanout, refusing) = test_core_refusable();
        let (tx, _rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        refusing.store(STORE_REFUSES, Ordering::SeqCst);
        let missing =
            std::env::temp_dir().join(format!("engram-no-such-dir-{}", uuid::Uuid::new_v4()));

        core.dispatch(
            AgentCommand::SpawnByCwd {
                cwd: missing.to_string_lossy().into_owned(),
                backend: Some(WireBackendKind::Claude),
                request_id: rid(),
                deliver_notices: false,
            },
            &session,
            &mock,
        )
        .await;

        match mock.events().as_slice() {
            [AgentEvent::Error {
                message,
                created_agent_id,
                ..
            }] => {
                assert!(
                    message.starts_with(&format!(
                        "CONFLICT: the agent was not created or started: {PROFILES_FILE}"
                    )),
                    "{message}"
                );
                assert_eq!(created_agent_id, &None, "nothing was created");
            }
            other => panic!("expected 1 Error: {other:?}"),
        }
        assert!(core.manager.list_agents().is_empty(), "nothing came up");
        assert!(
            core.manager.agent_snapshots().is_empty(),
            "nothing got onto the roster"
        );
    }

    /// A WS store write failure is `INTERNAL:` — same wording shape as a refusal (`CONFLICT`) but a different code (same split as the bus).
    // ADR-0291 R17
    #[tokio::test]
    async fn a_ws_change_whose_store_write_fails_answers_internal() {
        let (core, fanout, refusing) = test_core_refusable();
        let (tx, _rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let alpha = unlaunchable_profile("alpha");
        core.manager
            .try_create_agent(alpha.clone())
            .expect("registered when saved");
        refusing.store(STORE_FAILS_IO, Ordering::SeqCst);
        let before = fanout.texts().len();

        core.dispatch(
            AgentCommand::RenameProfile {
                profile_id: alpha.id,
                name: Some("beta".into()),
                request_id: rid(),
            },
            &session,
            &mock,
        )
        .await;

        match mock.events().as_slice() {
            [AgentEvent::Error { message, .. }] => assert!(
                message.starts_with("INTERNAL:")
                    && message.contains(&format!("writing {PROFILES_FILE} failed: disk full")),
                "{message}"
            ),
            other => panic!("expected 1 Error: {other:?}"),
        }
        assert_eq!(fanout.texts().len(), before, "broadcast 0");
        assert_eq!(
            core.manager
                .agent_snapshot(alpha.id)
                .and_then(|p| p.display_name),
            Some("alpha".to_string()),
            "name as is"
        );
    }

    // ══ the agent list after a rename (TRD S26-window-chrome §4-3) ══

    /// The form names (`AgentListUpdated` etc.) of broadcasts emitted after `before`, in order.
    fn broadcast_kinds(
        fanout: &crate::test_doubles::RecordingFanout,
        before: usize,
    ) -> Vec<String> {
        fanout.texts()[before..]
            .iter()
            .map(|text| {
                let value: serde_json::Value = serde_json::from_str(text).expect("wire is JSON");
                value
                    .as_object()
                    .and_then(|o| o.keys().next().cloned())
                    .expect("one external tag")
            })
            .collect()
    }

    /// The (id, name) list of each `AgentListUpdated` emitted after `before`, in order.
    fn broadcast_agent_lists(
        fanout: &crate::test_doubles::RecordingFanout,
        before: usize,
    ) -> Vec<Vec<(AgentId, String)>> {
        fanout.texts()[before..]
            .iter()
            .filter_map(|text| match serde_json::from_str::<AgentEvent>(text) {
                Ok(AgentEvent::AgentListUpdated { agents }) => {
                    Some(agents.into_iter().map(|a| (a.id, a.name)).collect())
                }
                _ => None,
            })
            .collect()
    }

    fn live_profile(raw: &str) -> CoreProfile {
        CoreProfile::new(
            raw.into(),
            CoreSpawnCommand::Shell {
                program: default_shell().to_string(),
                args: vec![],
            },
            std::env::temp_dir(),
            vec![],
            false,
        )
    }

    /// Renaming a live agent changes the name in `list_agents` (what the shell's tab auto name · window title follow), and `AgentListUpdated` goes out carrying it.
    #[tokio::test]
    async fn renaming_a_live_agent_broadcasts_the_agent_list_with_its_new_name() {
        let (core, fanout) = test_core_recording();
        let (tx, _rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let profile = live_profile("live-raw");
        core.manager
            .try_create_agent(profile.clone())
            .expect("registration succeeded");
        core.manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("spawn")
            .into_started()
            .expect("this call really launches (not a duplicate request)");
        let before = fanout.texts().len();

        core.dispatch(
            AgentCommand::RenameProfile {
                profile_id: profile.id,
                name: Some("renamed-live".into()),
                request_id: rid(),
            },
            &session,
            &mock,
        )
        .await;

        let listed = core
            .manager
            .list_agents()
            .into_iter()
            .find(|a| a.id == profile.id)
            .map(|a| a.name);
        let lists = broadcast_agent_lists(&fanout, before);
        // Turning off also emits the list — it comes before turning off.
        let kinds: Vec<String> = broadcast_kinds(&fanout, before)
            .into_iter()
            .filter(|kind| kind != "StatusChanged")
            .collect();
        core.manager.kill_agent(profile.id).ok();

        assert_eq!(
            listed.as_deref(),
            Some("renamed-live"),
            "the source of the list name"
        );
        assert_eq!(
            lists,
            vec![vec![(profile.id, "renamed-live".to_string())]],
            "one agent list per rename"
        );
        assert_eq!(kinds, ["ProfileListUpdated", "AgentListUpdated"]);
    }

    /// The bus · CLI `agent.rename` does not pass the WS arm; the roster notification port (`RosterFanout`) emits the agent list with the new name after the profile list.
    #[tokio::test]
    async fn the_roster_port_broadcasts_the_agent_list_after_the_profile_list() {
        use crate::control::agent::RosterBroadcast;

        let (core, fanout) = test_core_recording();
        let profile = live_profile("bus-raw");
        core.manager
            .try_create_agent(profile.clone())
            .expect("registration succeeded");
        core.manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("spawn")
            .into_started()
            .expect("this call really launches (not a duplicate request)");
        let port = RosterFanout::new(fanout.clone(), core.manager.clone());
        assert!(
            matches!(
                core.manager
                    .try_rename_agent(profile.id, Some("renamed-by-bus".into())),
                Ok(CoreRenameOutcome::Renamed(_))
            ),
            "rename confirmed (this test's premise)"
        );
        let before = fanout.texts().len();

        port.roster_changed();

        let lists = broadcast_agent_lists(&fanout, before);
        // Turning off also emits the list — it comes before turning off.
        let kinds: Vec<String> = broadcast_kinds(&fanout, before)
            .into_iter()
            .filter(|kind| kind != "StatusChanged")
            .collect();
        core.manager.kill_agent(profile.id).ok();

        assert_eq!(
            lists,
            vec![vec![(profile.id, "renamed-by-bus".to_string())]],
            "one agent list per notification"
        );
        assert_eq!(kinds, ["ProfileListUpdated", "AgentListUpdated"]);
    }

    /// A no-change rename (another holds the name, so the family name stays) emits only the profile list.
    #[tokio::test]
    async fn an_unchanged_rename_does_not_broadcast_the_agent_list() {
        let (core, fanout) = test_core_recording();
        let (tx, _rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let bob = unlaunchable_profile("bob");
        let twin = unlaunchable_profile("bob(1)");
        for profile in [&bob, &twin] {
            core.manager
                .try_create_agent(profile.clone())
                .expect("registration succeeded");
        }
        let before = fanout.texts().len();

        core.dispatch(
            AgentCommand::RenameProfile {
                profile_id: twin.id,
                name: Some("bob".into()),
                request_id: rid(),
            },
            &session,
            &mock,
        )
        .await;

        assert_eq!(
            core.manager
                .agent_snapshot(twin.id)
                .and_then(|p| p.display_name),
            Some("bob(1)".to_string()),
            "no change (this test's premise)"
        );
        assert_eq!(broadcast_kinds(&fanout, before), ["ProfileListUpdated"]);
    }

    // ── 6. does Subscribe put SubscribeAck → ReplayComplete into conn_tx in that order ──
    //    (no mock manager: the real AgentManager's empty snapshot path gives NotFound, so the control message order logic is reproduced
    //     directly. The real manager's replay synchronous send is covered by the output_core.rs unit tests.)
    #[tokio::test]
    async fn subscribe_control_order_ack_then_complete() {
        use crate::conn_port::Frame;
        use engram_dashboard_protocol::{encode_terminal_frame, SubscribeAction};
        use tokio::sync::mpsc;
        let (tx, mut rx) = mpsc::channel::<Frame>(16);
        let agent_id = uuid::Uuid::new_v4();

        let ack = event_json(&AgentEvent::SubscribeAck {
            agent_id,
            action: SubscribeAction::Reset,
            current_epoch: 0,
            oldest_seq: 0,
            latest_seq: 0,
            replay_from: 0,
            truncated: false,
            continues_conversation: false,
        })
        .unwrap();
        tx.send(Frame::Text(ack)).await.unwrap();
        tx.send(Frame::Binary(encode_terminal_frame(agent_id, 0, 0, b"r")))
            .await
            .unwrap();
        let complete = event_json(&AgentEvent::ReplayComplete { agent_id, epoch: 0 }).unwrap();
        tx.send(Frame::Text(complete)).await.unwrap();

        let first = rx.recv().await.unwrap();
        let second = rx.recv().await.unwrap();
        let third = rx.recv().await.unwrap();

        match first {
            Frame::Text(s) => assert!(s.contains("SubscribeAck")),
            _ => panic!("the 1st must be a SubscribeAck Text"),
        }
        assert!(
            matches!(second, Frame::Binary(_)),
            "the 2nd is replay binary"
        );
        match third {
            Frame::Text(s) => assert!(s.contains("ReplayComplete")),
            _ => panic!("the 3rd must be a ReplayComplete Text"),
        }
    }

    // ── the daemon's own table answers by the bus (delivery stage 1) ──────────────────────────────────

    // hangul-ok-begin
    /// The whole path registered without a dialog: `agent.new` on the bus is answered by the daemon's own table and shows in `agent.list`
    /// at once. If this breaks, the LLM's only means of creating an agent is a click (CLAUDE.md 「LLM-우선 제어」).
    // hangul-ok-end
    #[tokio::test]
    async fn the_daemons_own_agent_verbs_answer_over_the_bus() {
        let (core, _rx) = test_core_with_own_agent_table();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let (session, mut inbox) = attached_with_inbox(&core, 2);

        let made = engram_dashboard_command::RequestId::new();
        core.dispatch(
            bus_command_with(
                "agent.new",
                serde_json::json!({ "cwd": "C:/work/bus", "name": "from-bus", "backend": "Claude" }),
                made,
            ),
            &session,
            &mock,
        )
        .await;

        let agent_id = match next_frame_event(&mut inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = reply_from_wire(&reply).expect("the reply the daemon wrapped unpacks");
                assert_eq!(reply.request_id, made);
                let ok = reply.outcome.expect("registration succeeded");
                assert_eq!(ok["state"], "sleeping", "creating does not launch: {ok}");
                assert_eq!(ok["name"], "from-bus", "{ok}");
                ok["agent_id"].as_str().expect("id string").to_string()
            }
            other => panic!("a reply must come: {other:?}"),
        };

        let listed = engram_dashboard_command::RequestId::new();
        core.dispatch(
            bus_command_with("agent.list", serde_json::json!({}), listed),
            &session,
            &mock,
        )
        .await;

        match next_frame_event(&mut inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = reply_from_wire(&reply).expect("the reply the daemon wrapped unpacks");
                assert_eq!(reply.request_id, listed);
                let ok = reply.outcome.expect("lookup succeeded");
                let agents = ok["agents"].as_array().expect("array");
                assert_eq!(agents.len(), 1, "the one just made is on the roster: {ok}");
                assert_eq!(agents[0]["id"], agent_id, "{ok}");
            }
            other => panic!("a reply must come: {other:?}"),
        }
    }

    /// The bus entrance gets the same check (ADR-0157): without it a typo blocked at the control route would pass through the bus and become
    /// a different action (a lone `parnet` running as a hierarchy release). One check place for both (`control::commands::call_daemon_command`).
    #[tokio::test]
    async fn a_typo_in_an_argument_is_refused_on_the_bus_too() {
        let (core, _rx) = test_core_with_own_agent_table();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let (session, mut inbox) = attached_with_inbox(&core, 2);

        let req = engram_dashboard_command::RequestId::new();
        core.dispatch(
            bus_command_with(
                "agent.new",
                serde_json::json!({ "cwdd": "C:/work/bus" }),
                req,
            ),
            &session,
            &mock,
        )
        .await;

        match next_frame_event(&mut inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = reply_from_wire(&reply).expect("the reply the daemon wrapped unpacks");
                let err = reply.outcome.expect_err("an unknown field is refused");
                assert_eq!(
                    err.code(),
                    engram_dashboard_command::ErrorCode::InvalidArgument
                );
                assert!(
                    err.message().contains("cwdd"),
                    "must point at which field: {}",
                    err.message()
                );
            }
            other => panic!("a reply must come: {other:?}"),
        }
        assert_eq!(
            core.manager().roster().len(),
            0,
            "a refusal creates nothing"
        );
    }

    /// The bus cancel gets the same lease decision as WS (ADR-0231) — with the real wiring: the production lease state (`MultiViewState`) is the
    /// command table's lease port and delivery passes the sending connection as the caller. The holder passes and gets the verb body's
    /// answer (today always NOT_FOUND — mid-turn input policy `None`).
    #[tokio::test]
    async fn a_bus_cancel_from_a_non_holder_is_refused_with_the_ws_text() {
        use engram_dashboard_agent::types::QueuedInputEvent as Q;

        let (core, _rx) = test_core_with_own_agent_table();
        let live = queued_seam::insert(
            core.manager(),
            0,
            vec![Q::Queued {
                id: "q1".into(),
                text: "first".into(),
            }],
            &[],
        );
        let _ = core.multiview.acquire(live, 7);
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let (stranger, mut stranger_inbox) = attached_with_inbox(&core, 2);
        let (holder, mut holder_inbox) = attached_with_inbox(&core, 7);
        let args = serde_json::json!({ "target": live.to_string(), "input_id": "q1" });

        core.dispatch(
            bus_command_with(
                "agent.cancelQueuedInput",
                args.clone(),
                engram_dashboard_command::RequestId::new(),
            ),
            &stranger,
            &mock,
        )
        .await;
        match next_frame_event(&mut stranger_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = reply_from_wire(&reply).expect("the reply the daemon wrapped unpacks");
                let err = reply.outcome.expect_err("a non-holder is refused");
                assert_eq!(err.code(), engram_dashboard_command::ErrorCode::Conflict);
                assert_eq!(
                    err.message(),
                    INPUT_LOCKED_REFUSAL,
                    "the same characters as WS"
                );
            }
            other => panic!("a reply must come: {other:?}"),
        }

        core.dispatch(
            bus_command_with(
                "agent.cancelQueuedInput",
                args,
                engram_dashboard_command::RequestId::new(),
            ),
            &holder,
            &mock,
        )
        .await;
        match next_frame_event(&mut holder_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = reply_from_wire(&reply).expect("the reply the daemon wrapped unpacks");
                let err = reply.outcome.expect_err("today the verb is NOT_FOUND");
                assert_eq!(
                    err.code(),
                    engram_dashboard_command::ErrorCode::NotFound,
                    "the holder passes the lease and reaches the body: {}",
                    err.message()
                );
            }
            other => panic!("a reply must come: {other:?}"),
        }
        assert_eq!(
            core.manager()
                .list_queued_inputs(live)
                .expect("a live session")
                .rows
                .len(),
            1,
            "a refused cancel does not touch the roster"
        );
    }

    // ── Wire activation replies: launch block reasons · launch notices (TRD S25 §5-2 16) ──

    mod launch_replies {
        use super::*;
        use engram_dashboard_agent::launch::{
            notice_text, BlockReason, LaunchBlocked, LaunchKit, LaunchKitSource, LaunchPlan,
            NoopLaunchKits, PrepareError, Reason,
        };

        /// Blocks every launch with the reasons it holds.
        struct BlockingKits(LaunchBlocked);

        impl LaunchKitSource for BlockingKits {
            fn check(&self, _profile: &CoreProfile) -> Result<LaunchPlan, LaunchBlocked> {
                Err(self.0.clone())
            }
            fn prepare(
                &self,
                plan: LaunchPlan,
                id: AgentId,
                epoch: u32,
            ) -> Result<LaunchKit, PrepareError> {
                NoopLaunchKits.prepare(plan, id, epoch)
            }
            fn release(&self, _id: AgentId, _epoch: u32) {}
        }

        /// Passes every launch with the Noop plan plus these notices.
        struct NoticingKits(Vec<Notice>);

        impl LaunchKitSource for NoticingKits {
            fn check(&self, profile: &CoreProfile) -> Result<LaunchPlan, LaunchBlocked> {
                let mut plan = NoopLaunchKits.check(profile)?;
                plan.notices = self.0.clone();
                Ok(plan)
            }
            fn prepare(
                &self,
                plan: LaunchPlan,
                id: AgentId,
                epoch: u32,
            ) -> Result<LaunchKit, PrepareError> {
                NoopLaunchKits.prepare(plan, id, epoch)
            }
            fn release(&self, _id: AgentId, _epoch: u32) {}
        }

        fn blocked() -> LaunchBlocked {
            LaunchBlocked {
                reasons: vec![BlockReason::Missing {
                    item: "rules/a.md".into(),
                    via: vec!["team.yaml".into()],
                    reason: Reason {
                        code: "not-found".into(),
                        args: serde_json::json!({ "path": "rules/a.md" }),
                        line: Some(3),
                    },
                }],
            }
        }

        async fn dispatch_events(core: &ConnectionCore, cmd: AgentCommand) -> Vec<AgentEvent> {
            let (tx, _rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
            let mock = MockOutboundSink::new(tx);
            core.dispatch(cmd, &ConnectionSession::new(1), &mock).await;
            mock.events()
        }

        /// Every wire activation that the launch check blocks answers with the reasons as structured
        /// `launch_blocked` — the `LaunchBlocked::to_args` object — and starts nothing.
        #[tokio::test]
        async fn a_blocked_ws_activation_answers_with_its_reasons_and_starts_nothing() {
            let expected = serde_json::json!({
                "reasons": [{
                    "kind": "missing", "item": "rules/a.md", "via": ["team.yaml"],
                    "code": "not-found", "args": { "path": "rules/a.md" }, "line": 3
                }]
            });
            for verb in ["Spawn", "SpawnProfile", "SpawnByCwd"] {
                let (core, _fanout) = test_core_with_kits(Arc::new(BlockingKits(blocked())));
                // The policy is not under test — an open one lets `SpawnByCwd` reach the launch check.
                let core = core.with_llm_policy(|_| None);
                let stored = core
                    .manager
                    .try_create_agent(live_profile("blocked"))
                    .expect("register");
                let req = rid();
                let cmd = match verb {
                    "Spawn" => AgentCommand::Spawn {
                        profile_id: stored.id,
                        request_id: req,
                    },
                    "SpawnProfile" => AgentCommand::SpawnProfile {
                        profile_id: stored.id,
                        resume: false,
                        request_id: req,
                    },
                    _ => AgentCommand::SpawnByCwd {
                        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                        backend: Some(WireBackendKind::Claude),
                        request_id: req,
                        deliver_notices: false,
                    },
                };
                match dispatch_events(&core, cmd).await.as_slice() {
                    [AgentEvent::Error {
                        request_id,
                        message,
                        launch_blocked: Some(reasons),
                        created_agent_id,
                    }] => {
                        assert_eq!(*request_id, Some(req), "{verb}");
                        assert_eq!(
                            serde_json::Value::Object(reasons.clone()),
                            expected,
                            "{verb}"
                        );
                        assert!(!message.is_empty(), "{verb}: the text stays");
                        assert_eq!(created_agent_id, &None, "{verb}: nothing was created");
                    }
                    other => panic!("{verb}: one Error with the reasons expected: {other:?}"),
                }
                assert!(
                    core.manager.list_agents().is_empty(),
                    "{verb}: nothing started"
                );
                assert_eq!(
                    core.manager.agent_snapshots().len(),
                    1,
                    "{verb}: nothing created"
                );
            }
        }

        /// A failure that is not a launch block carries no `launch_blocked` — the field is not "any failure".
        #[tokio::test]
        async fn a_ws_activation_failure_that_is_not_a_block_carries_no_reasons() {
            for verb in ["Spawn", "SpawnProfile"] {
                let (core, _rx) = test_core();
                let stored = core
                    .manager
                    .try_create_agent(unlaunchable_profile("broken"))
                    .expect("register");
                let req = rid();
                let cmd = if verb == "Spawn" {
                    AgentCommand::Spawn {
                        profile_id: stored.id,
                        request_id: req,
                    }
                } else {
                    AgentCommand::SpawnProfile {
                        profile_id: stored.id,
                        resume: false,
                        request_id: req,
                    }
                };
                match dispatch_events(&core, cmd).await.as_slice() {
                    [AgentEvent::Error {
                        request_id,
                        launch_blocked,
                        ..
                    }] => {
                        assert_eq!(*request_id, Some(req), "{verb}");
                        assert_eq!(launch_blocked, &None, "{verb}");
                    }
                    other => panic!("{verb}: one Error expected: {other:?}"),
                }
            }
        }

        // ── WS create-and-start through the one creation body (ADR-0348 · TRD S25 step 20) ──

        /// Passes the first launch check of a profile id (the pre-create check) and blocks every later one (the
        /// prelaunch after the add) — a block after the add, with no process started.
        struct BlocksAfterTheAdd {
            blocked: LaunchBlocked,
            passed_once: StdMutex<std::collections::HashSet<AgentId>>,
        }

        impl LaunchKitSource for BlocksAfterTheAdd {
            fn check(&self, profile: &CoreProfile) -> Result<LaunchPlan, LaunchBlocked> {
                if self.passed_once.lock().unwrap().insert(profile.id) {
                    NoopLaunchKits.check(profile)
                } else {
                    Err(self.blocked.clone())
                }
            }
            fn prepare(
                &self,
                plan: LaunchPlan,
                id: AgentId,
                epoch: u32,
            ) -> Result<LaunchKit, PrepareError> {
                NoopLaunchKits.prepare(plan, id, epoch)
            }
            fn release(&self, _id: AgentId, _epoch: u32) {}
        }

        fn by_cwd(req: RequestId) -> AgentCommand {
            AgentCommand::SpawnByCwd {
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                backend: Some(WireBackendKind::Claude),
                request_id: req,
                deliver_notices: false,
            }
        }

        /// A WS create blocked after the add answers `INTERNAL` with the shared created-asleep wording, the block's
        /// reasons and the id of the row it left — registered, asleep, marked `Other` — and announces the roster once.
        #[tokio::test]
        async fn a_ws_create_blocked_after_the_add_names_the_asleep_agent() {
            let (core, fanout) = test_core_with_kits(Arc::new(BlocksAfterTheAdd {
                blocked: blocked(),
                passed_once: StdMutex::default(),
            }));
            let core = core.with_llm_policy(|_| None);
            let before = fanout.texts().len();
            let req = rid();

            let events = dispatch_events(&core, by_cwd(req)).await;

            let rows = core.manager.agent_snapshots();
            let [row] = rows.as_slice() else {
                panic!("one registered row expected: {rows:?}");
            };
            match events.as_slice() {
                [AgentEvent::Error {
                    request_id,
                    message,
                    launch_blocked: Some(reasons),
                    created_agent_id,
                }] => {
                    assert_eq!(*request_id, Some(req));
                    assert_eq!(*created_agent_id, Some(row.id), "the row it left");
                    assert!(
                        message.starts_with("INTERNAL: agent '")
                            && message
                                .contains(&format!("({}) was created but did not start: ", row.id))
                            && message.contains("fix the listed items first"),
                        "{message}"
                    );
                    assert_eq!(reasons, &blocked().to_args());
                }
                other => panic!("one created-asleep Error expected: {other:?}"),
            }
            assert_eq!(row.last_failure, Some(CoreFailureKind::Other));
            assert!(core.manager.list_agents().is_empty(), "nothing started");
            assert_eq!(
                broadcast_kinds(&fanout, before),
                ["ProfileListUpdated", "AgentListUpdated"],
                "the roster is announced once"
            );
        }

        /// A WS create blocked before the add names no agent, leaves no row and announces nothing.
        #[tokio::test]
        async fn a_ws_create_blocked_before_the_add_names_no_agent_and_announces_nothing() {
            let (core, fanout) = test_core_with_kits(Arc::new(BlockingKits(blocked())));
            let core = core.with_llm_policy(|_| None);
            let before = fanout.texts().len();

            let events = dispatch_events(&core, by_cwd(rid())).await;

            match events.as_slice() {
                [AgentEvent::Error {
                    launch_blocked: Some(_),
                    created_agent_id: None,
                    message,
                    ..
                }] => assert!(!message.starts_with("INTERNAL:"), "{message}"),
                other => panic!("one pre-add block expected: {other:?}"),
            }
            assert!(core.manager.agent_snapshots().is_empty(), "no row");
            assert!(broadcast_kinds(&fanout, before).is_empty(), "no announce");
        }

        #[test]
        fn the_delivery_request_maps_to_the_notice_delivery() {
            assert_eq!(delivery_for(true), NoticeDelivery::Delivered);
            assert_eq!(delivery_for(false), NoticeDelivery::Withheld);
        }

        /// The production text up to the test module.
        fn production_source() -> &'static str {
            include_str!("connection_core.rs")
                .split("mod tests {")
                .next()
                .expect("production part")
        }

        /// The production text of the `SpawnByCwd` arm — from its pattern to the next arm.
        fn by_cwd_arm_source() -> &'static str {
            let production = production_source();
            let start = production
                .find("AgentCommand::SpawnByCwd {\n                cwd,")
                .or_else(|| production.find("AgentCommand::SpawnByCwd {\r\n                cwd,"))
                .expect("the SpawnByCwd arm");
            let end = start
                + production[start..]
                    .find("AgentCommand::ListProfiles")
                    .expect("the next arm");
            &production[start..end]
        }

        /// ★The arm passes the sender's request into the body★ — a literal `NoticeDelivery::` there would answer
        /// every sender the same way (withheld never records the notices; delivered marks as shown a set the webview
        /// does not show).
        // ADR-0348 · TRD S25 step 20 (M13)
        #[test]
        fn the_by_cwd_arm_passes_the_sender_delivery_request_into_the_body() {
            let arm = by_cwd_arm_source();
            assert!(
                arm.contains("delivery: delivery_for(deliver_notices)"),
                "{arm}"
            );
            assert!(!arm.contains("NoticeDelivery::"), "{arm}");
        }

        /// ★Only the `SpawnByCwd` created-asleep arm names a created agent★ — every other `Error` (incl. every
        /// [`activation_error`]) created nothing, and a stray id would send a caller to wake an agent that is not
        /// there.
        // ADR-0348 · TRD S25 step 20 (U3)
        #[test]
        fn only_the_by_cwd_created_asleep_arm_names_a_created_agent() {
            let production = production_source();
            assert_eq!(production.matches("created_agent_id: Some(").count(), 1);
            let arm = by_cwd_arm_source();
            assert!(
                arm.contains("CreateRefused::CreatedAsleep")
                    && arm.contains("created_agent_id: Some(id)"),
                "{arm}"
            );
            let body = &production[production
                .find("fn activation_error(")
                .expect("activation_error")..];
            let body = &body[..body.find("\n}").expect("end of activation_error")];
            assert!(body.contains("created_agent_id: None"), "{body}");
        }

        /// `SpawnProfile`'s success carries the launch notices as rows (absent when there are none) and leaves the
        /// agent's notice fingerprint unrecorded — the webview shows no notices yet.
        #[tokio::test]
        async fn a_ws_spawn_profile_success_carries_the_launch_notices_and_records_nothing() {
            let notices = vec![
                Reason {
                    code: "skipped-field".into(),
                    args: serde_json::json!({ "field": "model" }),
                    line: Some(4),
                },
                Reason {
                    code: "plain".into(),
                    args: serde_json::json!({}),
                    line: None,
                },
            ];
            let rows = serde_json::json!([
                {
                    "code": "skipped-field", "args": { "field": "model" }, "line": 4,
                    "text": notice_text(&notices[0])
                },
                { "code": "plain", "args": {}, "text": notice_text(&notices[1]) }
            ]);
            for (kits, expected) in [(notices.clone(), Some(rows)), (Vec::new(), None)] {
                let (core, _fanout) = test_core_with_kits(Arc::new(NoticingKits(kits)));
                let stored = core
                    .manager
                    .try_create_agent(live_profile("noticed"))
                    .expect("register");
                let req = rid();
                let events = dispatch_events(
                    &core,
                    AgentCommand::SpawnProfile {
                        profile_id: stored.id,
                        resume: false,
                        request_id: req,
                    },
                )
                .await;
                let _ = core.manager.kill_agent(stored.id);
                match events.as_slice() {
                    [AgentEvent::Spawned {
                        request_id,
                        agent,
                        notices,
                    }] => {
                        assert_eq!(*request_id, req);
                        assert_eq!(agent.id, stored.id);
                        assert_eq!(
                            notices.clone().map(serde_json::Value::Array),
                            expected,
                            "the notice rows"
                        );
                    }
                    other => panic!("one Spawned expected: {other:?}"),
                }
                assert_eq!(
                    core.manager
                        .agent_snapshot(stored.id)
                        .expect("still registered")
                        .notice_fingerprint,
                    None,
                    "the WS records no notice fingerprint"
                );
            }
        }

        /// `Spawn` answers `Ack`, which carries no notices — so a success with notices leaves the fingerprint
        /// unrecorded.
        #[tokio::test]
        async fn a_ws_spawn_success_acks_and_records_no_notice_fingerprint() {
            let notices = vec![Reason {
                code: "plain".into(),
                args: serde_json::json!({}),
                line: None,
            }];
            let (core, _fanout) = test_core_with_kits(Arc::new(NoticingKits(notices)));
            let stored = core
                .manager
                .try_create_agent(live_profile("noticed"))
                .expect("register");
            let req = rid();
            let events = dispatch_events(
                &core,
                AgentCommand::Spawn {
                    profile_id: stored.id,
                    request_id: req,
                },
            )
            .await;
            let started = !core.manager.list_agents().is_empty();
            let _ = core.manager.kill_agent(stored.id);
            match events.as_slice() {
                [AgentEvent::Ack { request_id }] => assert_eq!(*request_id, req),
                other => panic!("one Ack expected: {other:?}"),
            }
            assert!(started, "the agent started");
            assert_eq!(
                core.manager
                    .agent_snapshot(stored.id)
                    .expect("still registered")
                    .notice_fingerprint,
                None,
                "the WS records no notice fingerprint"
            );
        }
    }
}
