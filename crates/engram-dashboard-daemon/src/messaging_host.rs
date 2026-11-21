//! messaging_host — the host adapter + assembly room of the messaging kernel (`engram-dashboard-messaging`) (ADR-0110).
//!
//! Plugs the daemon's real objects into the port traits the kernel owns. The kernel does not know `AgentManager`,
//! `TurnObservations` or `ControlRegistry` even as types (ADR-0110 decision 2), so all code knowing them lives here.
//!
//! Seam ledger — every resident of this file (the admission check of invariant 2). Form: `type/function` — kernel port/seam →
//! daemon object. Private translation helpers used by one entry (`to_live_agent`, `receipt`, `sort_key`, ...) belong to that entry.
//!   - `ManagerDeliveryPort` — `DeliveryPort` (inject · roster · name) → `AgentManager`.
//!   - `is_live` — roster predicate (adapter over core `AgentStatus::is_live`); listed because it spans the port above and `RosterDiff`.
//!   - `ManagerTurnFacts` — `TurnFacts` → the core's pending-list table + turn observation table (ADR-0113 decision 1 · ADR-0231).
//!   - the `ControlPlanePort` implementation of `ControlRegistry` — envelope format lookup + delivery observation recording.
//!   - `messaging_for_manager`/`messaging_for_manager_gated`/`busy_gate_for_manager` — `MessagingService`/`BusyPolicy` construction
//!     seam → manager + control registry wiring.
//!   - `ChannelIdleNotifier` — `IdleNotifier` · `FlushTrigger` → the sending end of the flush channel (`IdleCoalescer` is its internal state).
//!   - `MessagingFlushSink` — no kernel port; binds to the core `StatusSink` (decorator), diffs appearance/incarnation changes
//!     (`RosterDiff`) and triggers a flush on the same channel.
//!   - `spawn_flush_worker`/`run_flush_worker`/`run_flush_lane`/`FlushWorkerHandles`/`FlushWiring` — the consuming end of that channel →
//!     `flush_for` of `MessagingSlot` (the late-injected `MessagingService`); the delivery lane internals (`DeliveryLanes`,
//!     `flush_recipient`, `spawn_delivery`, `advance_lane`, `DeliveryDone`) count as belonging here.
//!
//! Invariant 1 — never re-implement kernel decisions (load-bearing, ADR-0110 "ports thin, policy in the lib"): the busy decision
//! (positive-knowledge-only · cap), the three send branches and the parking TTL are kernel-owned. This applies to the ports, not to
//! the whole file: the flush pipeline holds host-owned policy — idle coalescing (`IdleCoalescer`), roster diff sequencing (`RosterDiff`,
//! enqueue under a lock, an intended ADR-0006 deviation), blocking isolation (`spawn_blocking` in `run_flush_lane`) — decided on facts
//! the kernel cannot know. Each rationale is canonical in that type's header.
//! Invariant 2 — admission check: a new type enters only if it can be written as one line in the seam ledger (which kernel port/seam,
//! which daemon object); adding without listing is also a violation. "Related to messaging" is not a criterion — the erosion ADR-0129
//! recorded (control plane / messaging seams flowing into `ws.rs`) was topically related at every step. A listed place is a seam, not daemon housekeeping.
//! The flush pipeline lives here (ADR-0129) because it implements kernel ports (`IdleNotifier`/`FlushTrigger`), consumes a kernel
//! service (`MessagingSlot`) and shares the `is_live` predicate with the sending side.
// ADR-0110
// ADR-0129

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_agent::inputs_pending::InputsPendingTable;
use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::profile::RestoreReport as CoreRestoreReport;
use engram_dashboard_agent::turn::{TurnObservation, TurnObservations};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo as CoreAgentInfo, AgentStatus as CoreStatus, StatusSink,
};
use engram_dashboard_messaging::busy::{BusyGate, BusyPolicy, IdleNotifier, TurnFact, TurnFacts};
use engram_dashboard_messaging::envelope::{DeliveryObservation, EnvelopeFormat};
use engram_dashboard_messaging::service::{
    AddressingSources, ControlPlanePort, DeliveryPort, DormantAgent, InjectReceipt, LiveAgent,
    MessagingService,
};
use engram_dashboard_messaging::PeerId;

use tokio::sync::mpsc;

use crate::control::registry::ControlRegistry;
use crate::usage_service::observe::UsageObserveSink;

// ── delivery adapter ────────────────────────────────────────────────────────────────────────────

/// Production DeliveryPort — a thin wrapper over Arc<AgentManager>, calling only its public API (ADR-0006: each call clones the
///   sessions lock internally and releases it at once).
pub struct ManagerDeliveryPort {
    manager: Arc<AgentManager>,
}

impl ManagerDeliveryPort {
    pub fn new(manager: Arc<AgentManager>) -> Self {
        Self { manager }
    }
}

/// Roster predicate — the only membership condition in this file (load-bearing).
///
/// Defined by agent `AgentStatus::is_live` (ADR-0119 decision 4); the roster (`AgentManager::roster`) must use the same predicate but the
/// core cannot depend on the daemon, so this is the daemon-side adapter taking an `AgentInfo`.
// ADR-0119
pub(crate) fn is_live(a: &engram_dashboard_agent::types::AgentInfo) -> bool {
    a.status.is_live()
}

fn to_live_agent(a: engram_dashboard_agent::types::AgentInfo) -> LiveAgent {
    LiveAgent {
        id: a.id,
        name: a.name,
        epoch: a.epoch,
        turn_signal: a.capabilities.output.structured,
    }
}

/// Sort key for `@all` determinism (see `live_agents`). Both lookups (`live_agents` · `addressing_sources`) share it, or the decision would see a different order.
fn sort_key(a: &LiveAgent, b: &LiveAgent) -> std::cmp::Ordering {
    a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id))
}

fn receipt(o: engram_dashboard_agent::types::WriteOutcome) -> InjectReceipt {
    InjectReceipt {
        bytes_requested: o.bytes_requested,
        bytes_written: o.bytes_written,
        msg_uuid: o.msg_uuid,
        epoch: o.epoch,
    }
}

impl DeliveryPort for ManagerDeliveryPort {
    /// Must be `submit_stdin_observed` (do not go back to `write_stdin_observed`): with only envelope bytes written, a terminal (TUI)
    ///   recipient keeps them in its input box and never starts a turn — delivery means "the recipient took it as a turn"
    ///   (`DeliveryPort::inject` contract).
    /// The receipt must mean landing, not acceptance: the bounded input queue makes `send_input`'s `Ok` mean "received", so recording it
    ///   would log an unsent envelope as delivered. Only `submit_stdin_observed` confirms the landing (`AgentSession::submit_input_observed`
    ///   → `AgentTransport::flush_input`). Whether a backend needs submission is the backend's decision (ADR-0004).
    /// The source is fixed to `Mail`: mail never reaches the user's pending list even mid-turn (ADR-0231); `write_stdin` takes a source
    ///   and would make the caller choose again.
    fn inject(&self, to_id: PeerId, bytes: &[u8]) -> Result<InjectReceipt, String> {
        self.manager
            .submit_stdin_observed(to_id, bytes)
            .map(receipt)
            .map_err(|e| e.to_string())
    }

    /// Sort = (name, id) ascending (load-bearing — `@all` determinism).
    ///
    /// The `@all` list uses this roster verbatim (groups.rs contract axis 1 — the resolver must not sort), but the manager's session store
    /// is a HashMap, so unsorted the same broadcast yields different injection and `results[]` orders per run. Kernel unit tests use a Vec
    /// fake and cannot catch it, so determinism is pinned on the production implementation. The sort sits before the source, so the
    /// resolver still returns what it received; `DeliveryPort` promises only a live, reachable roster. The second key is id so same-name
    /// entries (dup-name) stay stable.
    /// Only those that read mail are listed (user decision 2026-08-17): besides `is_live`, `reads_messages` removes the shell — an envelope
    /// reaching a shell is run as a command, not read. This is not a regression to ADR-0116 decisions 1·7 (they rejected "cannot deliver
    /// because the turn cannot be observed"; this is about what the input is interpreted as).
    /// Must be the same predicate as `addressing_sources`, or a shell is resolved as `@all` and then injected.
    /// Eligibility comes from a snapshot field (do not re-ask the manager): per-item re-query brings TOCTOU, non-atomicity and per-session
    /// locks (`AgentInfo::reads_messages` doc).
    fn live_agents(&self) -> Vec<LiveAgent> {
        let mut live: Vec<LiveAgent> = self
            .manager
            .list_agents()
            .into_iter()
            .filter(|a| is_live(a) && a.reads_messages)
            .map(to_live_agent)
            .collect();
        live.sort_by(sort_key);
        live
    }

    /// No set-difference computation here (ADR-0119 decision 2): the dormant names are synthesized in the manager alone and this forwards.
    /// A copy of that synthesis existed in the frontend and drifted; do not merge them again here.
    /// The dormant side is sorted by (name, id) too (as `live_agents` and `sort_key`).
    // ADR-0119 (single entry of the roster — this function is a forwarder)
    // ADR-0309 (decision 3 — the dormant side is an (id, name) pair)
    fn addressing_sources(&self) -> AddressingSources {
        let mut roster: Vec<LiveAgent> = Vec::new();
        let mut dormant: Vec<DormantAgent> = Vec::new();
        for entry in self.manager.roster() {
            match entry.live {
                // The live list must use the same predicate as `live_agents`, or a shell gets resolved at entry yet is absent from the delivery list.
                Some(info) if info.reads_messages => roster.push(to_live_agent(info)),
                // Dormant names are not filtered (known residual, unrelated to delivery safety): mail to a dormant shell name can be parked, but after waking
                //   the shell does not enter the live list, so nothing is injected (it expires by TTL). Filtering would break existing integration tests
                //   that use a shell profile as a stand-in for a live agent.
                Some(_) => {}
                None => dormant.push(DormantAgent {
                    id: entry.id,
                    name: entry.canonical_name,
                }),
            }
        }
        roster.sort_by(sort_key);
        dormant.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        // ADR-0391: no other PC in the snapshot yet — the peer module does not feed messaging, so every address resolves locally.
        AddressingSources {
            roster,
            dormant,
            pcs: Vec::new(),
        }
    }

    fn is_agent_live(&self, id: PeerId) -> bool {
        self.manager
            .list_agents()
            .into_iter()
            .any(|a| a.id == id && is_live(&a))
    }
}

// ── control plane adapter ────────────────────────────────────────────────────────────────────────────

impl ControlPlanePort for ControlRegistry {
    // Self-recursion trap (load-bearing): in the `ControlRegistry::…(self)` path below an inherent method resolves ahead of a trait method.
    //   Renaming/removing the inherent method silently rebinds the path to this trait method itself (no compile error) and the delivery hot
    //   path recurses until stack overflow. Rename on the registry side together.
    fn envelope_format(&self) -> EnvelopeFormat {
        ControlRegistry::envelope_format(self)
    }
    fn record_delivery(&self, obs: DeliveryObservation) {
        ControlRegistry::record_delivery(self, obs)
    }
}

// ── turn observation adapter ──────────────────────────────────────────────────────────────────────────────

/// Production `TurnFacts` — a read-only window translating the core's turn observation tables (ADR-0113 facts layer) into kernel vocabulary.
///
/// Thin by contract: the cap and fallback belong to the kernel `BusyPolicy` (ADR-0110); a condition like "hide it if old" here would
/// leak mail policy into the daemon.
/// Holds the tables directly, not the manager: no sessions lock on this delivery-decision path; the tables are leaf locks (ADR-0006).
/// Read order = pending-list table first, turn observation table second (load-bearing): it mirrors the core's write order (progress →
/// "emptied"), so a read seeing "emptied" also sees the progress before it. Reversed, mail arriving in the gap is read as idle and folded into the user's turn.
// ADR-0231
pub struct ManagerTurnFacts {
    pending: Arc<dyn PendingRead>,
    turns: Arc<dyn TurnRead>,
}

/// The window through which the adapter reads the two tables — where a test plugs in a fake recording the read order.
trait PendingRead: Send + Sync {
    fn inputs_pending(&self, id: AgentId, epoch: u32) -> Option<bool>;
}

trait TurnRead: Send + Sync {
    fn observation(&self, id: AgentId, epoch: u32) -> Option<TurnObservation>;
    fn in_turn_snapshot(&self) -> Vec<(AgentId, u32, Instant)>;
}

impl PendingRead for InputsPendingTable {
    fn inputs_pending(&self, id: AgentId, epoch: u32) -> Option<bool> {
        self.get(id, epoch)
    }
}

impl TurnRead for TurnObservations {
    fn observation(&self, id: AgentId, epoch: u32) -> Option<TurnObservation> {
        self.get(id, epoch)
    }
    fn in_turn_snapshot(&self) -> Vec<(AgentId, u32, Instant)> {
        TurnObservations::in_turn_snapshot(self)
    }
}

impl ManagerTurnFacts {
    pub fn new(manager: &Arc<AgentManager>) -> Self {
        Self {
            pending: manager.inputs_pending(),
            turns: manager.turns(),
        }
    }
}

impl TurnFacts for ManagerTurnFacts {
    fn turn_fact(&self, id: PeerId, epoch: u32) -> Option<TurnFact> {
        let inputs_pending = self.pending.inputs_pending(id, epoch);
        match (self.turns.observation(id, epoch), inputs_pending) {
            (Some(o), pending) => Some(TurnFact {
                in_turn: o.in_turn,
                last_signal: o.last_signal,
                inputs_pending: pending.unwrap_or(false),
                last_end_failed: o.last_end_failed,
            }),
            // A full list without a turn observation: the fact of being busy is not discarded. `last_signal` is read only when `in_turn`.
            (None, Some(true)) => Some(TurnFact {
                in_turn: false,
                last_signal: Instant::now(),
                inputs_pending: true,
                last_end_failed: false,
            }),
            (None, _) => None,
        }
    }

    fn in_turn_snapshot(&self) -> Vec<(PeerId, u32, Instant)> {
        self.turns.in_turn_snapshot()
    }
}

// ── assembly helpers ──────────────────────────────────────────────────────────────────────────────────

/// No gate (immediate injection) — for assemblies without the idle gate (experiment bins). Daemon boot uses `messaging_for_manager_gated`.
pub fn messaging_for_manager(
    manager: Arc<AgentManager>,
    registry: Arc<ControlRegistry>,
) -> MessagingService {
    MessagingService::new(Arc::new(ManagerDeliveryPort::new(manager)), registry)
}

/// Idle gate injected — for daemon boot and integration tests.
pub fn messaging_for_manager_gated(
    manager: Arc<AgentManager>,
    registry: Arc<ControlRegistry>,
    busy: Arc<dyn BusyGate>,
) -> MessagingService {
    MessagingService::new_gated(Arc::new(ManagerDeliveryPort::new(manager)), registry, busy)
}

pub fn busy_gate_for_manager(
    manager: Arc<AgentManager>,
    notifier: Arc<dyn IdleNotifier>,
) -> BusyPolicy {
    BusyPolicy::new(Arc::new(ManagerTurnFacts::new(&manager)), notifier)
}

// ── MessagingFlushSink (appearance/epoch flush trigger, ADR-0104 — C1/C2) ──────────────────────────

/// The unit of work flowing to the flush worker (C1 appearance flush + C2 idle gate doorbell).
///
/// One channel, one consumer: separate channels would let the appearance and the turn end of one agent overtake each other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlushMsg {
    /// Roster appearance/incarnation change — flush the parked mail of that agent (queue key = id, ADR-0309).
    Appear { id: AgentId },
    /// Turn end (idle transition) observed — inject that id's parked mail oldest-first (C2 idle gate, ADR-0104 decision 3).
    Idle { id: AgentId },
}

/// Idle notification coalescer (C2): a new enqueue is folded away if an unprocessed Idle of the same id is already queued.
///
/// Needed because every MessageDone notifies (busy.rs `IdleNotifier`) and back-to-back tool calls would pile up items in the unbounded
/// channel. A flush drains the whole queue, so N Idles of one id equal one.
/// No lost wakeup (load-bearing): the core updates the turn observation table before notifying (output_core.rs emit), so the state a
/// folded notification points at is represented by the still-unprocessed Idle; the consumer removes the id from the set when it picks
/// up (before looking at the gate), so a turn end arriving mid-processing is enqueued again.
/// Appear is never folded: an appearance is unique and must be lossless — Idle only.
#[derive(Debug, Default)]
pub struct IdleCoalescer {
    pending: Mutex<std::collections::HashSet<AgentId>>,
}

impl IdleCoalescer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Must an Idle of this id be enqueued (no unprocessed item yet)? If true the caller sends.
    fn claim(&self, id: AgentId) -> bool {
        self.pending
            .lock()
            .expect("idle coalescer poisoned")
            .insert(id)
    }

    /// The consumer picked up an Idle of this id — later notifications must be enqueued again.
    fn taken(&self, id: AgentId) {
        self.pending
            .lock()
            .expect("idle coalescer poisoned")
            .remove(&id);
    }
}

/// The kernel → flush worker notification implementation — `IdleNotifier` (waking the cap sweep) and `FlushTrigger` (service doorbell)
///   share one channel (C2); both mean "flush that id's parking queue".
///
/// Non-blocking contract (load-bearing): `notify_idle` runs on the pump thread and `request_flush` on the sender (MCP/HTTP) thread;
/// an unbounded `send` is non-blocking and await-free. A closed channel (worker ended) is ignored — the daemon is going down and
/// parking is in memory (spec §0 no persistence).
pub struct ChannelIdleNotifier {
    tx: mpsc::UnboundedSender<FlushMsg>,
    coalescer: Arc<IdleCoalescer>,
}

impl ChannelIdleNotifier {
    pub fn new(tx: mpsc::UnboundedSender<FlushMsg>, coalescer: Arc<IdleCoalescer>) -> Self {
        Self { tx, coalescer }
    }

    fn enqueue(&self, id: AgentId) {
        if self.coalescer.claim(id) {
            let _ = self.tx.send(FlushMsg::Idle { id });
        }
    }
}

impl engram_dashboard_messaging::busy::IdleNotifier for ChannelIdleNotifier {
    fn notify_idle(&self, id: AgentId) {
        self.enqueue(id);
    }
}

impl engram_dashboard_messaging::service::FlushTrigger for ChannelIdleNotifier {
    fn request_flush(&self, id: AgentId) {
        self.enqueue(id);
    }
}

/// Parking flush trigger (ADR-0104 · S18 messaging v1 C1): wraps the status sink to observe roster changes on the daemon side and
/// flush parked mail of an agent (id) that newly came alive or whose incarnation changed.
///
/// Wrapping needs no agent seam change (ADR-0028/0104: the core must not know messaging): the manager's status sink already pushes a
/// roster snapshot via `agent_list_updated(Vec<AgentInfo>)`, which is diffed here; the existing broadcast is delegated as is.
///
/// Flush work is split off the status-sink callback (load-bearing): that callback is a no-blocking path called by the manager/reaper thread,
/// and a synchronous `flush_for` blocked roster forwarding through several blocking writes. The sink does only a cheap diff, pushes a
/// `FlushMsg` onto the unbounded channel and forwards the status event at once; the flush worker flushes.
///
/// Lock discipline (ADR-0006) — an intended deviation: the prev-snapshot Mutex stays held through the channel enqueue
/// (`RosterDiff::dispatch`, rationale canonical there). Do not go back to "release the lock then send".
/// With the lock held there is exactly one outbound call — the unbounded flush channel send (everything else is pure data handling:
/// the `is_live` predicate · HashMap handling). Safety = non-reentrant (the receiver is the flush worker, which cannot re-enter
/// `RosterDiff::inner`) and non-blocking. Adding a log, another lock or a port call in this section breaks that exhaustiveness. The
/// actual flush (messaging lock + port call + blocking write) runs on the worker, outside this lock.
pub struct MessagingFlushSink {
    /// `Box<dyn>`: production is `UsageObserveSink(DaemonStatusSink)`; integration tests wrap a NoopSink — the diff/flush logic is the same whatever is wrapped.
    inner: Box<dyn StatusSink>,
    /// Channel sending flush work (FlushMsg) to the flush worker (unbounded — never blocks the status callback). A send failure (worker
    ///   not running/dropped) is ignored: parking is retried on the next appearance.
    flush_tx: mpsc::UnboundedSender<FlushMsg>,
    /// Roster diff sequencing state (snapshot + enqueue serialization).
    diff: RosterDiff,
    /// The outlet moving the turn-end push (core `StatusSink::turn_ended`) to the doorbell — shares the coalescer with the notifying side.
    idle: ChannelIdleNotifier,
}

/// Roster diff sequencing state — keeps the previous snapshot under a lock and finishes the channel enqueue while holding it.
///
/// Why send under the lock (load-bearing): with the lock released before send, two concurrent `agent_list_updated` callbacks (the core does
/// not serialize them) can let snapshot update order and enqueue order diverge — an Appear from the old snapshot arriving after the new
/// one's and flushing an incarnation that is gone. Enqueue inside the lock guarantees snapshot order = channel order; the unbounded
/// send is non-blocking, so the section stays short.
#[derive(Debug, Default)]
pub struct RosterDiff {
    inner: Mutex<RosterSnapshots>,
}

#[derive(Debug, Default)]
struct RosterSnapshots {
    /// Live agents of the previous snapshot — id → incarnation marker. Keyed by id so a rename is invisible to the diff (ADR-0309).
    prev: HashMap<AgentId, u32>,
}

impl RosterDiff {
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles one roster update — computes the diff and enqueues in order while holding the lock.
    fn dispatch(&self, agents: &[CoreAgentInfo], flush_tx: &mpsc::UnboundedSender<FlushMsg>) {
        let mut st = self.inner.lock().expect("flush roster diff poisoned");

        // Collects all live (Running|Exiting) agents as id → incarnation marker. No structured condition applies (ADR-0116 decision 7, 4th
        //   amendment): capability was dropped from roster eligibility, so a session with no turn signal can hold parked mail too (via
        //   injection failure, spec §5 branch 3); reviving the condition would leave that mail without a reappearance flush until the 24h TTL.
        // Key = id, the name is not looked at (ADR-0309 decision 1 · TRD S29 X4): the queue is keyed by id, so same-name agents each flush their own
        //   queue and a rename (same id · same incarnation) is not an appearance. The old "flush only when the name is unique" skip existed
        //   because a name-keyed queue could go to the wrong namesake — reviving it ties up both twins' mail until the next trigger.
        // ADR-0309
        let next: HashMap<AgentId, u32> = agents
            .iter()
            .filter(|a| crate::messaging_host::is_live(a))
            .map(|a| (a.id, a.epoch))
            .collect();
        for (id, epoch) in &next {
            // Flush trigger = an id not there before (new agent · woke from dormancy) OR a different incarnation of the same id (respawn/reactivation).
            //   Incarnation is mismatch, not order: the marker is random per incarnation (`AgentProfile::epoch`), so an order test misses an
            //   appearance when the new marker is smaller and that agent's mail is tied up until the next respawn.
            if st.prev.get(id) != Some(epoch) {
                let _ = flush_tx.send(FlushMsg::Appear { id: *id });
            }
        }
        st.prev = next;
    }
}

impl MessagingFlushSink {
    /// Production constructor — `idle` is the Idle coalescer shared with the flush lane. The production chain is the single
    /// `MessagingFlushSink(UsageObserveSink(DaemonStatusSink))`; taking the middle link by type makes a chain without usage observation unbuildable.
    pub fn new(
        inner: UsageObserveSink,
        flush_tx: mpsc::UnboundedSender<FlushMsg>,
        idle: Arc<IdleCoalescer>,
    ) -> Self {
        Self::new_boxed(Box::new(inner), flush_tx, idle)
    }

    /// Test constructor — wraps an arbitrary inner StatusSink (NoopSink, etc.) to verify only the flush logic.
    pub fn new_test(
        inner: Box<dyn StatusSink>,
        flush_tx: mpsc::UnboundedSender<FlushMsg>,
        idle: Arc<IdleCoalescer>,
    ) -> Self {
        Self::new_boxed(inner, flush_tx, idle)
    }

    fn new_boxed(
        inner: Box<dyn StatusSink>,
        flush_tx: mpsc::UnboundedSender<FlushMsg>,
        idle: Arc<IdleCoalescer>,
    ) -> Self {
        Self {
            idle: ChannelIdleNotifier::new(flush_tx.clone(), idle),
            inner,
            flush_tx,
            diff: RosterDiff::new(),
        }
    }
}

/// 2-lane pipeline (head-of-line blocking isolation, load-bearing):
///   - main lane (this function): only forwards from the channel to the delivery lane (non-blocking), so work from the status callback keeps being absorbed.
///   - delivery lane (`run_flush_lane`): its own task + channel, where `spawn_blocking` can stick on child stdin writes and submit pacing;
///     serial per recipient (AgentId), parallel across recipients (split axis canonical in that function's header).
///
/// The lane task is owned by the caller (boot) — this function does not spawn it: a JoinHandle held in a local and dropped when shutdown
/// aborts the main lane only detaches the lane, so the 5s join belt in lib.rs would watch just the main lane (no blocking work) and a
/// runtime drop could hang. Both tasks are held by the caller and brought down by abort + belt (`spawn_flush_worker` / `FlushWorkerHandles::shutdown`).
/// When the main lane ends or is aborted `lane_tx` drops and the lane exits after handling the deliveries left in its queue.
pub async fn run_flush_worker(
    mut flush_rx: mpsc::UnboundedReceiver<FlushMsg>,
    lane_tx: mpsc::UnboundedSender<FlushMsg>,
) {
    while let Some(msg) = flush_rx.recv().await {
        match msg {
            other @ (FlushMsg::Appear { .. } | FlushMsg::Idle { .. }) => {
                // Do not swallow the send failure: if the lane died (panic, etc.) a silent `let _ =` hides that all deliveries stopped permanently
                //   (parking piles up and expires by TTL, with no log). No lane restart here — the lane isolates an individual flush's panic only in test
                //   builds without the hook (in production a panic outside the marker ends the daemon — ADR-0292), so this failure is either
                //   shutdown or a real bug, and the log is the only clue.
                if let Err(e) = lane_tx.send(other) {
                    tracing::warn!("flush lane forward failed (lane ended/panicked — later deliveries stop): {e}");
                }
            }
        }
    }
    tracing::debug!("flush worker (main lane) ended (channel closed)");
}

/// The 2-lane bundle handle of the flush worker — boot (production `run()` / test server) owns both tasks together. A detached lane makes
/// the shutdown belt meaningless (see `run_flush_worker`), so one type brings down both sides.
pub struct FlushWorkerHandles {
    /// Receiving lane — takes from the channel and passes to the delivery lane (no blocking work).
    main: tokio::task::JoinHandle<()>,
    /// Delivery lane (Appear/Idle) — its `spawn_blocking` deliveries can stick on child stdin writes and submit pacing (at most 1 per
    ///   recipient, several across recipients).
    lane: tokio::task::JoinHandle<()>,
}

impl FlushWorkerHandles {
    /// Brings both lanes down — `shutdown_all` (child kill · pipe close) must have finished before calling (load-bearing order: the shutdown
    ///   comment in lib.rs). Each abort's join has a 5s belt so an unforeseen blocking cannot hang the daemon shutdown — it warns and
    ///   detaches (process exit reclaims the threads).
    ///
    /// Accepted residual — abort and the belt cannot cut the body of `spawn_blocking`: abort acts only at `.await` points, so a blocking-pool
    /// thread inside a syscall keeps running. The shutdown does not hang because of the call order, not the belt: (1) `shutdown_all` has
    /// finished, so the child is killed and the stdin pipe closed, and the blocked `inject` (write_all+flush) is released with an error;
    /// (2) there is no blocking work outside the delivery lane. Kill-first is the real guarantee; abort + belt is an observation device
    /// (a firing belt means a premise broke, and needs a pathological OS where a killed child's pipe write neither errors nor returns). No stronger
    /// cancellation (separate process, forced thread termination) is introduced.
    ///
    /// Closing the pipe does not release the submit pacing wait: a delivery is body write → `agent::backend::SUBMIT_PACING` (0.5s) wait →
    /// submit write; (1) errors the write but the wait sleeps on, so an in-flight closure runs up to ~0.5s (+ write) after the abort.
    /// The daemon process exit waits that ~0.5s: the bin is `#[tokio::main]` and the shutdown path has no `shutdown_timeout`/`shutdown_background`,
    /// so the runtime drop waits for started blocking tasks. Removing it would need touching the pacing, which breaks submission for a TUI recipient (`SUBMIT_PACING` doc).
    /// The "belt + ~0.5s" cap holds under 4 premises (if one breaks it is not a cap):
    ///   1. concurrent deliveries ≤ blocking pool capacity (tokio default 512) — below it the sleeps are concurrent; above it they queue
    ///      and `shutdown_all` shares the same pool, delaying kill-first itself. 512 is out of reach for one person's agents.
    ///   2. belt time = 5s × 2 (`shutdown` belts the main and delivery lanes sequentially).
    ///   3. remaining injections fail fast (pipe closed, body write cut off before the pacing); a batch that keeps succeeding yields remaining count × 0.5s.
    ///   4. it waits only on the normal (Ok) exit path — the error path in `main.rs` is `std::process::exit` and skips the runtime drop.
    pub async fn shutdown(self) {
        // main first — on abort `lane_tx` is dropped so the lane accepts no new work.
        self.main.abort();
        if tokio::time::timeout(FLUSH_JOIN_BELT, self.main)
            .await
            .is_err()
        {
            tracing::warn!(
                "flush worker (main lane) shutdown timed out after {FLUSH_JOIN_BELT:?} — detach"
            );
        }
        // then the delivery lane; deliveries left here are discarded (parking is in memory — spec §0).
        self.lane.abort();
        if tokio::time::timeout(FLUSH_JOIN_BELT, self.lane)
            .await
            .is_err()
        {
            tracing::warn!("flush lane shutdown timed out after {FLUSH_JOIN_BELT:?} — detach (prevents a shutdown hang)");
        }
    }
}

/// The lane channel is unbounded: the forward must be non-blocking, or head-of-line blocking returns (`run_flush_worker` header).
pub fn spawn_flush_worker(
    flush_rx: mpsc::UnboundedReceiver<FlushMsg>,
    wiring: FlushWiring,
) -> FlushWorkerHandles {
    let (lane_tx, lane_rx) = mpsc::unbounded_channel::<FlushMsg>();
    let lane = tokio::spawn(run_flush_lane(lane_rx, wiring.messaging, wiring.idle));
    let main = tokio::spawn(run_flush_worker(flush_rx, lane_tx));
    FlushWorkerHandles { main, lane }
}

/// Shutdown join belt — if not finished this long after abort, warn and detach (prevents a daemon shutdown hang).
const FLUSH_JOIN_BELT: Duration = Duration::from_secs(5);

/// Per-recipient delivery wait queue — an entry's existence = a delivery to that recipient is running now; the value = work waiting behind it.
///
/// There is no separate running flag: the same fact in two places can diverge ("running but no entry" overlaps the next delivery;
/// "only the entry remains" stops that recipient's mail permanently) without a compile error.
// ADR-0142
type DeliveryLanes = HashMap<AgentId, VecDeque<FlushMsg>>;

/// The recipient this work is addressed to — the split key of delivery serialization.
fn flush_recipient(msg: &FlushMsg) -> AgentId {
    match msg {
        FlushMsg::Appear { id } | FlushMsg::Idle { id } => *id,
    }
}

/// The end report of a delivery closure (normal exit and panic unwind) — Drop tells the dispatcher completion.
///
/// Drop rather than awaiting the JoinHandle (load-bearing): awaiting inline would tie the dispatcher to that delivery and bring back
/// head-of-line blocking. Being Drop it fires on panic unwind too; without it that AgentId's queue never reopens and its mail stops silently.
/// Panic isolation exists only in test builds without the hook: the delivery closure is outside the containment marker, so in production a
/// panic ends the process before unwinding (this Drop does not run either — ADR-0292). Do not read it as "the lane survives a panic in production".
struct DeliveryDone {
    id: AgentId,
    tx: mpsc::UnboundedSender<AgentId>,
    stage: DeliveryStage,
}

/// How far the delivery closure got — decides what the `Drop` of `DeliveryDone` says.
///
/// The two incomplete branches must stay separate: merged, it would always lie in production, where a delivery panic ends the process
/// before unwinding (ADR-0292), so the only incomplete Drop is `NotStarted`; logging "panicked" would report a nonexistent bug at every shutdown.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DeliveryStage {
    /// The blocking pool never ran the closure — cancelled from the queue during runtime shutdown.
    NotStarted,
    /// Entered execution but did not finish = unwinding (possible only in test builds without the hook).
    Started,
    Finished,
}

impl Drop for DeliveryDone {
    fn drop(&mut self) {
        match self.stage {
            DeliveryStage::Finished => {}
            DeliveryStage::NotStarted => tracing::debug!(
                agent = %self.id,
                "flush delivery was cancelled before it started (runtime shutting down) — releasing the lane only"
            ),
            DeliveryStage::Started => tracing::warn!(
                agent = %self.id,
                "flush delivery unwound before completing (that recipient's lane continues — this branch is reached only in runs without the panic hook)"
            ),
        }
        // If the channel is closed (dispatcher ended) discard — the daemon is going down.
        let _ = self.tx.send(self.id);
    }
}

/// Launches this work's delivery onto the blocking pool. `false` = could not launch (service not injected — the work was consumed and no
///   completion notice will come, so the caller must continue that recipient's lane itself).
fn spawn_delivery(
    msg: FlushMsg,
    messaging: &Arc<crate::control::mcp_server::MessagingSlot>,
    idle: &IdleCoalescer,
    done_tx: &mpsc::UnboundedSender<AgentId>,
) -> bool {
    let id = flush_recipient(&msg);
    if matches!(msg, FlushMsg::Idle { .. }) {
        // Remove from the coalescing set first when picking up (lost wakeup prevention): a MessageDone arriving during this flush must be
        //   enqueued again. The removal moment is when this delivery actually starts — earlier (taken from the channel / queued) lets a
        //   waiting item's turn end pile up as a new item (duplicates); later folds away the turn end in between.
        idle.taken(id);
    }
    let Some(svc) = messaging.get() else {
        // Service not injected (early boot) — no parking, skip; it comes again on the next appearance/turn-end event.
        return false;
    };
    let svc = svc.clone();
    let report = DeliveryDone {
        id,
        tx: done_tx.clone(),
        stage: DeliveryStage::NotStarted,
    };
    tokio::task::spawn_blocking(move || {
        let mut report = report;
        report.stage = DeliveryStage::Started;
        match msg {
            FlushMsg::Appear { id } => svc.flush_for(id),
            FlushMsg::Idle { id } => svc.flush_for_agent(id),
        }
        report.stage = DeliveryStage::Finished;
    });
    true
}

/// Starts the next waiting work for this recipient — if none, removes the entry (bounded state).
fn advance_lane(
    id: AgentId,
    lanes: &mut DeliveryLanes,
    messaging: &Arc<crate::control::mcp_server::MessagingSlot>,
    idle: &IdleCoalescer,
    done_tx: &mpsc::UnboundedSender<AgentId>,
) {
    loop {
        // A skipped work (service not injected) gets no completion notice, so look at the next one here.
        let Some(next) = lanes.get_mut(&id).and_then(|q| q.pop_front()) else {
            lanes.remove(&id);
            return;
        };
        if spawn_delivery(next, messaging, idle, done_tx) {
            return;
        }
    }
}

/// Delivery lane — a per-recipient serial dispatcher for delivery work (Appear/Idle); a stuck blocking write here leaves the receiving lane running.
///
/// Split key = doorbell AgentId (load-bearing — do not widen): one delivery per doorbell id at a time. The aim is throughput — only terminal
/// (Raw) recipients need the 0.5s submit pacing (structured input has no submit sequence); one queue would make everyone pay it. Order within a
/// doorbell id ("oldest first" batch — ADR-0104) rides on this too.
/// Injection target = doorbell id: the kernel `MessagingService::drain_queue` injects only into the roster entry whose id equals the queue id
/// (ADR-0309), so two lanes with different doorbell ids never converge on one session. The kernel's target-axis in-flight guard (ADR-0142)
/// remains though the trigger that set it up no longer arises — removal is up to `docs/tracking.md` T-30.
/// The dispatcher itself does not block: it does not await the blocking task inline; completion returns on a separate channel (`DeliveryDone`)
/// while it keeps emptying `lane_rx`. Breaking that brings head-of-line blocking straight back.
/// Appear is not folded: queueing is lossless (a flush drains the recipient's parked mail whole), while dropping it would strand that id's
/// mail until the TTL. Coalescing is for the notifying-side Idle only (`IdleCoalescer`).
/// spawn_blocking isolation (executor starvation): the `inject` inside `flush_for` is transport.send_input = a synchronous blocking
/// write_all+flush to the child stdin pipe (`PtyTransport`/`StdioTransport::send_input`). Called directly in an async task it occupies a
/// runtime worker; on a current-thread/single-worker runtime that starves other tasks (`shutdown_all`, the 5s join belt). So each flush runs on
/// `spawn_blocking`: the write occupies the blocking pool, the dispatcher stays at the select's `.await` so abort acts at once, and the belt
/// and shutdown task keep being polled. This fixes only executor starvation — the dependence on shutdown order remains (`FlushWorkerHandles::shutdown`).
// ADR-0142
async fn run_flush_lane(
    mut lane_rx: mpsc::UnboundedReceiver<FlushMsg>,
    messaging: Arc<crate::control::mcp_server::MessagingSlot>,
    idle: Arc<IdleCoalescer>,
) {
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<AgentId>();
    let mut lanes: DeliveryLanes = DeliveryLanes::new();
    // Even after the receiving channel closes it ends only after draining in-flight deliveries and waiting (lifetime contract = the
    //   `run_flush_worker` header). This scope holds done_tx so the completion channel never closes first.
    let mut open = true;

    while open || !lanes.is_empty() {
        tokio::select! {
            incoming = lane_rx.recv(), if open => match incoming {
                Some(msg) => {
                    let id = flush_recipient(&msg);
                    match lanes.entry(id) {
                        std::collections::hash_map::Entry::Occupied(mut e) => e.get_mut().push_back(msg),
                        std::collections::hash_map::Entry::Vacant(e) => {
                            e.insert(VecDeque::new());
                            if !spawn_delivery(msg, &messaging, &idle, &done_tx) {
                                advance_lane(id, &mut lanes, &messaging, &idle, &done_tx);
                            }
                        }
                    }
                }
                None => open = false,
            },
            Some(id) = done_rx.recv() => advance_lane(id, &mut lanes, &messaging, &idle, &done_tx),
            // Both branches are closed (the loop condition blocks this, but select panics when every branch is inactive).
            else => break,
        }
    }
    tracing::debug!("flush lane ended (channel closed)");
}

/// The flush worker wiring bundle (assembled at boot) — fewer arguments, and what is shared is visible at a glance.
#[derive(Clone)]
pub struct FlushWiring {
    pub messaging: Arc<crate::control::mcp_server::MessagingSlot>,
    /// Idle coalescing set — shared with the notifying side (`MessagingFlushSink`), released on pick-up.
    pub idle: Arc<IdleCoalescer>,
}

impl StatusSink for MessagingFlushSink {
    fn status_changed(&self, id: AgentId, status: CoreStatus, epoch: u32) {
        self.inner.status_changed(id, status, epoch);
    }

    fn agent_list_updated(&self, agents: Vec<CoreAgentInfo>) {
        self.diff.dispatch(&agents, &self.flush_tx);
        self.inner.agent_list_updated(agents);
    }

    fn restore_result(&self, report: CoreRestoreReport) {
        self.inner.restore_result(report);
    }

    // Usage is only passed through, not a flush trigger — it must not be omitted (decorator contract of `turn_ended`).
    fn usage_observed(&self, obs: engram_dashboard_agent::usage::UsageObservation) {
        self.inner.usage_observed(obs);
    }

    /// Turn-end push → flush doorbell (ADR-0113 decision 3 — the daemon only relays): the coalescing decision + a non-blocking channel send
    ///   (contract: `StatusSink::turn_ended`).
    // ADR-0113
    fn turn_ended(&self, id: AgentId, epoch: u32) {
        // The epoch is not put on the doorbell: a flush opens the per-agent queue, and whichever incarnation ended, delivering to the current
        //   one is right (mail is addressed to the logical agent — ADR-0086 §F5).
        self.idle.enqueue(id);
        // Always pass it to the wrapped sink too (decorator contract): this wrapper is the only StatusSink the daemon installs and the hook is a
        //   no-op by default, so omitting it dies silently when the inner one implements it. A path exporting turn state to the frontend/LLM
        //   control surface is planned inside it (ADR-0113 §Impact — aligned with §5).
        self.inner.turn_ended(id, epoch);
    }

    /// Empty-list push → the same flush doorbell: the flush lane asks again whether busy, so ringing during a pause after an error lets no mail in.
    ///   Passed to the wrapped sink as in `turn_ended` (decorator contract).
    // ADR-0231
    fn inputs_drained(&self, id: AgentId, epoch: u32) {
        self.idle.enqueue(id);
        self.inner.inputs_drained(id, epoch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_agent::types::AgentId;

    // ── turn fact adapter ──────────────────────────────────────────────────────────────────────

    fn facts(pending: Arc<InputsPendingTable>, turns: Arc<TurnObservations>) -> ManagerTurnFacts {
        ManagerTurnFacts { pending, turns }
    }

    #[test]
    fn turn_facts_forwards_the_core_observation_verbatim() {
        use engram_dashboard_agent::turn::{TurnEndKind, TurnSignal};
        let turns = Arc::new(TurnObservations::new());
        let f = facts(Arc::new(InputsPendingTable::new()), turns.clone());
        let id = AgentId::new_v4();
        assert_eq!(
            f.turn_fact(id, 0),
            None,
            "unobserved is passed on as unobserved"
        );

        let t0 = Instant::now();
        turns.observe_at(id, 0, 1, TurnSignal::Progress, t0);
        assert_eq!(
            f.turn_fact(id, 0),
            Some(TurnFact {
                in_turn: true,
                last_signal: t0,
                inputs_pending: false,
                last_end_failed: false,
            })
        );
        assert_eq!(f.in_turn_snapshot(), vec![(id, 0, t0)]);

        turns.observe_at(id, 0, 2, TurnSignal::Ended(TurnEndKind::Clean), t0);
        assert_eq!(
            f.turn_fact(id, 0),
            Some(TurnFact {
                in_turn: false,
                last_signal: t0,
                inputs_pending: false,
                last_end_failed: false,
            })
        );
        assert!(
            f.in_turn_snapshot().is_empty(),
            "only those in a turn reach the sweep entrance"
        );
    }

    #[test]
    fn turn_facts_carry_the_list_and_the_halt_from_the_two_tables() {
        use engram_dashboard_agent::turn::{TurnEndKind, TurnSignal};
        let pending = Arc::new(InputsPendingTable::new());
        let turns = Arc::new(TurnObservations::new());
        let f = facts(pending.clone(), turns.clone());
        let id = AgentId::new_v4();
        let t0 = Instant::now();
        pending.register(id, 4);
        turns.register_at(id, 4, t0);
        pending.set(id, 4, 1, true);
        turns.observe_at(id, 4, 2, TurnSignal::Ended(TurnEndKind::Failed), t0);
        assert_eq!(
            f.turn_fact(id, 4),
            Some(TurnFact {
                in_turn: false,
                last_signal: t0,
                inputs_pending: true,
                last_end_failed: true,
            })
        );
        assert!(
            f.in_turn_snapshot().is_empty(),
            "a list or a pause does not reach the cap sweep entrance"
        );

        let other = AgentId::new_v4();
        pending.register(other, 0);
        pending.set(other, 0, 1, true);
        let fact = f
            .turn_fact(other, 0)
            .expect("a list filled without a turn observation is also a fact of being busy");
        assert!(fact.inputs_pending && !fact.in_turn && !fact.last_end_failed);
        pending.set(other, 0, 2, false);
        assert_eq!(
            f.turn_fact(other, 0),
            None,
            "no fact in either table means unobserved"
        );
    }

    /// Two fake tables that write the read order on one line.
    struct Recorded {
        log: Arc<Mutex<Vec<&'static str>>>,
    }
    impl PendingRead for Recorded {
        fn inputs_pending(&self, _id: AgentId, _epoch: u32) -> Option<bool> {
            self.log.lock().unwrap().push("pending");
            Some(false)
        }
    }
    impl TurnRead for Recorded {
        fn observation(&self, _id: AgentId, _epoch: u32) -> Option<TurnObservation> {
            self.log.lock().unwrap().push("turn");
            None
        }
        fn in_turn_snapshot(&self) -> Vec<(AgentId, u32, Instant)> {
            Vec::new()
        }
    }

    // ADR-0231
    #[test]
    fn turn_facts_read_the_list_table_before_the_turn_table() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let f = ManagerTurnFacts {
            pending: Arc::new(Recorded { log: log.clone() }),
            turns: Arc::new(Recorded { log: log.clone() }),
        };
        let _ = f.turn_fact(AgentId::new_v4(), 0);
        assert_eq!(
            *log.lock().unwrap(),
            vec!["pending", "turn"],
            "mirror of the write order (progress → emptied) — reversed, there is a gap where the two facts are read together as idle"
        );
    }

    // ══════════════════════════════════════════════════════════════════════════════════════════
    // Review fix D9-a — sealing the real adapter of the roster predicate (ADR-0116 decisions 1·7 · spec §7)
    // ══════════════════════════════════════════════════════════════════════════════════════════

    /// The platform gate applies only to tests that spawn: the dormancy-axis seal is only `upsert` (no OS dependency), and `#[cfg(windows)]`
    /// on the whole module would let that regression pass green elsewhere.
    mod roster_predicate {
        use super::*;
        use engram_dashboard_agent::persistence::{FilePresetStore, FileProfileStore};
        use engram_dashboard_agent::preset::PresetRegistry;
        #[cfg(windows)]
        use engram_dashboard_agent::profile::SpawnMode;
        use engram_dashboard_agent::profile::{AgentCommand, AgentProfile, ProfileRegistry};
        use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
        use engram_dashboard_agent::types::{AgentInfo, AgentStatus, StatusSink};
        #[cfg(windows)]
        use engram_dashboard_base::testing::wait_until;
        use engram_dashboard_messaging::service::DeliveryPort;
        use std::time::Duration;

        struct NoopSink;
        impl StatusSink for NoopSink {
            fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        fn manager(tag: &str) -> Arc<AgentManager> {
            let sink: Arc<dyn StatusSink> = Arc::new(NoopSink);
            let dir = |k: &str| {
                std::env::temp_dir()
                    .join(format!("engram-roster-{k}-{tag}-{}", uuid::Uuid::new_v4()))
            };
            Arc::new(AgentManager::new(
                sink,
                Arc::new(ProfileRegistry::new(Arc::new(FileProfileStore::new(dir(
                    "prof",
                ))))),
                Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(dir(
                    "preset",
                ))))),
                Arc::new(SessionTracker::new(
                    TrackerConfig {
                        enabled: false,
                        poll_interval: Duration::from_secs(1),
                    },
                    Arc::new(|_, _| {}),
                )),
            ))
        }

        /// A fixture whose base name (`AgentProfile::name`) and canonical name (`display_name`) deliberately differ: if equal, a regression
        ///   picking the dormant name from the trap field (`p.name`) passes (ADR-0116 Impact — "picking the dormant name from that field silently diverges").
        fn profile(base: &str, canonical: &str) -> AgentProfile {
            let mut p = AgentProfile::new(
                base.to_string(),
                AgentCommand::Shell {
                    program: engram_dashboard_platform::shell::default_shell().to_string(),
                    args: vec![],
                },
                std::env::temp_dir(),
                vec![],
                false,
            );
            p.display_name = Some(canonical.to_string());
            p
        }

        /// A live session with no turn signal (shell = structured false) — the core population of the 4th-amendment roster.
        #[cfg(windows)]
        fn shell(name: &str) -> AgentProfile {
            profile(name, name)
        }

        /// A shell is not one that reads mail (user decision 2026-08-17): an envelope reaching a shell is run as a command, so it is removed from
        /// the recipient list (this also closes automatic `@all` inclusion).
        ///
        /// The delete-cleanup gate (`is_agent_live`) is not applied together: it asks "is this session alive right now", not "does it receive mail";
        /// applying both would fire the cleanup of another axis when deleting a shell profile — pinned here.
        /// The capability-independence of the membership predicate (ADR-0116 decisions 1·7) is guarded by `tests/control_send.rs`
        /// `roster_includes_a_terminal_agent_without_a_turn_signal_no_claude`; this shell fixture cannot speak for it (a terminal claude needs a
        /// real binary, and the session injection seam is on the integration side).
        #[cfg(windows)]
        #[test]
        fn a_live_shell_is_excluded_from_the_mail_roster_but_still_counts_as_live() {
            let manager = manager("live");
            let port = ManagerDeliveryPort::new(manager.clone());
            let info = manager
                .spawn_agent(&shell("sheller"), SpawnMode::Fresh)
                .expect("shell spawn")
                .into_started()
                .expect("this call really launches (not a duplicate request)");
            assert!(wait_until(Duration::from_secs(3), || manager
                .list_agents()
                .iter()
                .any(|a| a.id == info.id)));

            assert!(
                !port.live_agents().iter().any(|a| a.id == info.id),
                "a shell must not get on the delivery list (the envelope would be run as a command)"
            );
            let sources = port.addressing_sources();
            assert!(
                !sources.roster.iter().any(|a| a.id == info.id),
                "the entry-decision source must use the same predicate (applying it to one side only splits interpretation and delivery): {sources:?}"
            );
            assert!(
                port.is_agent_live(info.id),
                "the delete-cleanup gate is the 'is it alive' axis so it must stay true (review fix D1)"
            );

            // A hole that was really open: `DeleteProfile` does not kill a live session. A decision drawn from the profile would be "unknown" here and the
            //   shell would return to the list — the decision basis must be in the session.
            manager.try_delete_agent(info.id).expect("save succeeded");
            assert!(
                !port.live_agents().iter().any(|a| a.id == info.id),
                "even when the profile is gone a live shell must still be off the delivery list"
            );
            assert!(
                !port
                    .addressing_sources()
                    .roster
                    .iter()
                    .any(|a| a.id == info.id),
                "the entry-decision source likewise"
            );

            manager.kill_agent(info.id).ok();
        }

        // The seal of ADR-0116 decisions 1·7 (capability ≠ membership) holds only if the adapter's two output functions are exercised, so the
        // integration test with the session injection seam takes it (`tests/control_send.rs`
        // `roster_includes_a_terminal_agent_without_a_turn_signal_no_claude`). Do not substitute a test calling only `is_live`: that predicate is a
        // single status line that never reads capability, so it is always true (a tautology — written that way once and caught in review).

        #[test]
        fn two_dormant_profiles_sharing_a_name_are_both_reported() {
            let manager = manager("dormant-dup");
            let port = ManagerDeliveryPort::new(manager.clone());
            // Plant it via the harness seam (ADR-0120): the normal path (`try_create_agent`) enforces registry-wide name uniqueness and cannot create two
            //   same names; before uniqueness applied to all data (an existing agents.json) this state can really exist.
            let twin_a = profile("raw-twin-a", "twin");
            let twin_b = profile("raw-twin-b", "twin");
            let mut expected = vec![
                DormantAgent {
                    id: twin_a.id,
                    name: "twin".to_string(),
                },
                DormantAgent {
                    id: twin_b.id,
                    name: "twin".to_string(),
                },
            ];
            expected.sort_by_key(|d| d.id);
            manager.seed_agent_bypassing_uniqueness(twin_a);
            manager.seed_agent_bypassing_uniqueness(twin_b);

            let sources = port.addressing_sources();
            assert!(
                sources.roster.is_empty(),
                "no spawn so the roster must be empty: {sources:?}"
            );
            assert_eq!(
                sources.dormant, expected,
                "two same-name dormant entries must come up as 2 with their own ids and canonical names (name · id order): {sources:?}"
            );
        }

        /// Renaming a dormant agent keeps its id (ADR-0309 decision 1): pending mail · contracts are bound to that id, so an id changing with a
        ///   rename would make the agent lose its own mail and obligations.
        #[test]
        fn a_renamed_dormant_profile_is_reported_with_the_same_id_and_the_new_name() {
            let manager = manager("dormant-rename");
            let port = ManagerDeliveryPort::new(manager.clone());
            let sleeper = profile("raw-sleeper", "before");
            let id = sleeper.id;
            manager.seed_agent_bypassing_uniqueness(sleeper);
            assert_eq!(
                port.addressing_sources().dormant,
                vec![DormantAgent {
                    id,
                    name: "before".to_string()
                }],
                "premise: a dormant pair"
            );

            let outcome = manager
                .try_rename_agent(id, Some("after".to_string()))
                .expect("save succeeded");
            assert!(
                matches!(&outcome, engram_dashboard_agent::manager::RenameOutcome::Renamed(n) if n == "after"),
                "premise: the rename is confirmed: {outcome:?}"
            );
            assert_eq!(
                port.addressing_sources().dormant,
                vec![DormantAgent {
                    id,
                    name: "after".to_string()
                }],
                "must be the same id · a new name"
            );
        }

        #[cfg(windows)]
        #[test]
        fn a_live_namesake_does_not_hide_a_dormant_profile_with_the_same_name() {
            // By name a live namesake wins, so the dormancy axis never reaches the outcome; `@all` expands by id so a dormant namesake gets its own share
            //   (`service.rs::a_live_agent_wins_over_a_dormant_namesake` · ADR-0309) — a regression removing by the name axis silently loses it.
            let manager = manager("live-namesake");
            let port = ManagerDeliveryPort::new(manager.clone());
            let info = manager
                .spawn_agent(&profile("raw-live-twin", "twin"), SpawnMode::Fresh)
                .expect("shell spawn")
                .into_started()
                .expect("this call really launches (not a duplicate request)");
            assert!(wait_until(Duration::from_secs(3), || manager
                .list_agents()
                .iter()
                .any(|a| a.id == info.id)));
            // Harness seam: the live `twin` is already in the registry, so the normal path would rename it to `twin(1)` (ADR-0120); the set-difference
            //   axis being id needs equal names.
            let dormant_twin = profile("raw-dormant-twin", "twin");
            let dormant_id = dormant_twin.id;
            manager.seed_agent_bypassing_uniqueness(dormant_twin);

            let sources = port.addressing_sources();
            // The live side is a shell, so it is not on the delivery list (`reads_messages`). The axis is unchanged: the set difference is finished by
            //   agent `roster()` by id and the adapter only filters after, so a regression removing by name makes the live `twin` swallow the dormant one and the list empty.
            assert_eq!(
                sources.dormant,
                vec![DormantAgent {
                    id: dormant_id,
                    name: "twin".to_string()
                }],
                "the live side drops out by id and the dormant same-name must remain (removing by the name axis would empty it): {sources:?}"
            );

            manager.kill_agent(info.id).ok();
        }

        // The absence of a terminated session from the roster is not asserted here: the reaper removes a really terminated session from the map at once
        // (reaper.rs), so "terminal status yet in the list" cannot be made from a real session. The seal is the integration test with the session injection
        // seam (`tests/control_send.rs` `roster_excludes_a_terminal_session_still_in_the_map`).
    }

    // ── 7. MessagingFlushSink diff/enqueue logic — pure diff verification without the worker ──────────────────────
    use engram_dashboard_agent::types::{
        AgentInfo as TAgentInfo, Capabilities, ControlCaps, InputCaps, ModelCaps, OutputCaps,
        SessionCaps,
    };

    fn flush_info(
        id: AgentId,
        name: &str,
        epoch: u32,
        structured: bool,
        status: CoreStatus,
    ) -> TAgentInfo {
        TAgentInfo {
            id,
            name: name.to_string(),
            cwd: ".".to_string(),
            status,
            cols: 80,
            rows: 24,
            epoch,
            capabilities: Capabilities {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: !structured,
                    structured,
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
                session: SessionCaps {
                    resume: false,
                    snapshot: false,
                    cwd_env: false,
                },
                model: ModelCaps {
                    select: false,
                    temperature: false,
                    max_tokens: false,
                },
            },
            reads_messages: true,
        }
    }

    /// Test no-op inner StatusSink — broadcast is irrelevant (only the diff is verified).
    struct TestNoopSink;
    impl StatusSink for TestNoopSink {
        fn status_changed(&self, _: AgentId, _: CoreStatus, _: u32) {}
        fn agent_list_updated(&self, _: Vec<CoreAgentInfo>) {}
        fn restore_result(&self, _: CoreRestoreReport) {}
    }

    /// Builds a sink that extracts only flush work + the receiving end of its channel (worker not wired — only the diff is observed).
    fn flush_sink() -> (MessagingFlushSink, mpsc::UnboundedReceiver<FlushMsg>) {
        let (tx, rx) = mpsc::unbounded_channel::<FlushMsg>();
        let sink = MessagingFlushSink::new_test(
            Box::new(TestNoopSink),
            tx,
            Arc::new(IdleCoalescer::new()),
        );
        (sink, rx)
    }

    fn drain_msgs(rx: &mut mpsc::UnboundedReceiver<FlushMsg>) -> Vec<FlushMsg> {
        let mut out = Vec::new();
        while let Ok(t) = rx.try_recv() {
            out.push(t);
        }
        out
    }

    /// The ids of appearance flush targets (Appear) — one snapshot's order is HashMap iteration, so sorted before returning.
    fn drain_targets(rx: &mut mpsc::UnboundedReceiver<FlushMsg>) -> Vec<AgentId> {
        let mut ids: Vec<AgentId> = drain_msgs(rx)
            .into_iter()
            .filter_map(|m| match m {
                FlushMsg::Appear { id } => Some(id),
                _ => None,
            })
            .collect();
        ids.sort();
        ids
    }

    #[test]
    fn flush_sink_enqueues_a_newly_live_agent() {
        let (sink, mut rx) = flush_sink();
        let id = AgentId::new_v4();
        sink.agent_list_updated(vec![flush_info(id, "alice", 0, true, CoreStatus::Running)]);
        assert_eq!(
            drain_targets(&mut rx),
            vec![id],
            "flush for a newly live agent"
        );
    }

    /// Same-name twins each flush their own queue (ADR-0309 · TRD S29 X4): the queue is keyed by id; reviving the old "only when the name is unique" skip would drop both.
    #[test]
    fn flush_sink_flushes_each_same_named_twin() {
        let (sink, mut rx) = flush_sink();
        let a = AgentId::new_v4();
        let b = AgentId::new_v4();
        sink.agent_list_updated(vec![
            flush_info(a, "dup", 0, true, CoreStatus::Running),
            flush_info(b, "dup", 0, true, CoreStatus::Running),
        ]);
        let mut both = vec![a, b];
        both.sort();
        assert_eq!(drain_targets(&mut rx), both, "both twins appear");
        sink.agent_list_updated(vec![flush_info(a, "dup", 0, true, CoreStatus::Running)]);
        assert!(
            drain_targets(&mut rx).is_empty(),
            "even if one drops out and the name becomes unique, the remaining one is the same incarnation so it is not an appearance"
        );
    }

    /// A rename is not an appearance — only a new id or a different incarnation of the same id is (ADR-0309 · TRD S29 §6-3 U4). A by-name diff
    ///   would raise a needless flush on rename and see an agent that took the old name as the same agent.
    #[test]
    fn flush_sink_appears_by_id_and_tag_and_ignores_a_rename() {
        let (sink, mut rx) = flush_sink();
        let x = AgentId::new_v4();
        sink.agent_list_updated(vec![flush_info(x, "a", 4, true, CoreStatus::Running)]);
        assert_eq!(drain_targets(&mut rx), vec![x]);

        sink.agent_list_updated(vec![flush_info(x, "b", 4, true, CoreStatus::Running)]);
        assert!(
            drain_targets(&mut rx).is_empty(),
            "a rename with the same id · the same incarnation is not an appearance"
        );

        let y = AgentId::new_v4();
        sink.agent_list_updated(vec![
            flush_info(x, "b", 4, true, CoreStatus::Running),
            flush_info(y, "a", 4, true, CoreStatus::Running),
        ]);
        assert_eq!(
            drain_targets(&mut rx),
            vec![y],
            "a different agent that newly took the old name appears only as itself (the new id)"
        );

        sink.agent_list_updated(vec![
            flush_info(x, "b", 7, true, CoreStatus::Running),
            flush_info(y, "a", 4, true, CoreStatus::Running),
        ]);
        assert_eq!(
            drain_targets(&mut rx),
            vec![x],
            "a new incarnation of the same id is an appearance"
        );
    }

    /// Decided by mismatch, not order: the incarnation marker is random per incarnation, so a new value can be smaller (`AgentProfile::epoch`);
    /// an order test misses the appearance in half of respawns and ties up that agent's mail until the next respawn.
    #[test]
    fn flush_sink_enqueues_on_any_different_tag_but_not_the_same_one() {
        let (sink, mut rx) = flush_sink();
        let id = AgentId::new_v4();
        sink.agent_list_updated(vec![flush_info(id, "a", 5, true, CoreStatus::Running)]);
        assert_eq!(drain_targets(&mut rx), vec![id]);
        sink.agent_list_updated(vec![flush_info(id, "a", 5, true, CoreStatus::Running)]);
        assert!(
            drain_targets(&mut rx).is_empty(),
            "re-pushing the same marker does not flush"
        );
        sink.agent_list_updated(vec![flush_info(id, "a", 9, true, CoreStatus::Running)]);
        assert_eq!(
            drain_targets(&mut rx),
            vec![id],
            "a differing marker flushes (respawn/reactivation)"
        );
        sink.agent_list_updated(vec![flush_info(id, "a", 2, true, CoreStatus::Running)]);
        assert_eq!(
            drain_targets(&mut rx),
            vec![id],
            "there are respawns where the marker gets **smaller** even with the same id — that is also an appearance"
        );
    }

    #[test]
    fn flush_sink_enqueues_when_same_name_different_id_lower_epoch() {
        let (sink, mut rx) = flush_sink();
        let old = AgentId::new_v4();
        let new = AgentId::new_v4();
        sink.agent_list_updated(vec![flush_info(old, "svc", 3, true, CoreStatus::Running)]);
        assert_eq!(drain_targets(&mut rx), vec![old]);
        sink.agent_list_updated(vec![flush_info(new, "svc", 0, true, CoreStatus::Running)]);
        assert_eq!(
            drain_targets(&mut rx),
            vec![new],
            "a different agent with the same name (new id) flushes even with a lower epoch (finding 3)"
        );
        sink.agent_list_updated(vec![flush_info(new, "svc", 0, true, CoreStatus::Running)]);
        assert!(
            drain_targets(&mut rx).is_empty(),
            "re-pushing the same id+epoch does not flush"
        );
    }

    #[test]
    fn flush_sink_appears_for_a_turn_signal_less_agent_and_ignores_the_dead() {
        let (sink, mut rx) = flush_sink();
        let tui = AgentId::new_v4();
        let dead = AgentId::new_v4();
        sink.agent_list_updated(vec![
            flush_info(tui, "tui", 0, false, CoreStatus::Running), // non-structured = no turn signal
            flush_info(dead, "dead", 0, true, CoreStatus::Killed), // terminal
        ]);
        assert_eq!(
            drain_msgs(&mut rx),
            vec![FlushMsg::Appear { id: tui }],
            "a live session with no turn signal = Appear · terminal is nothing"
        );
    }

    // ── 7b. turn-end push → doorbell (ADR-0113 — the daemon only relays) ─────────────────────────────

    #[test]
    fn a_turn_end_push_from_the_core_becomes_an_idle_doorbell() {
        let (sink, mut rx) = flush_sink();
        let id = AgentId::new_v4();
        sink.turn_ended(id, 3);
        assert_eq!(drain_msgs(&mut rx), vec![FlushMsg::Idle { id }]);
    }

    /// The inner that records whether it was passed to the wrapped sink.
    struct DrainedInner {
        seen: Arc<Mutex<Vec<(AgentId, u32)>>>,
    }
    impl StatusSink for DrainedInner {
        fn status_changed(&self, _: AgentId, _: CoreStatus, _: u32) {}
        fn agent_list_updated(&self, _: Vec<CoreAgentInfo>) {}
        fn inputs_drained(&self, id: AgentId, epoch: u32) {
            self.seen.lock().unwrap().push((id, epoch));
        }
    }

    // ADR-0231
    #[test]
    fn a_drained_list_push_rings_the_same_coalesced_doorbell_and_is_forwarded() {
        let (tx, mut rx) = mpsc::unbounded_channel::<FlushMsg>();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = MessagingFlushSink::new_test(
            Box::new(DrainedInner { seen: seen.clone() }),
            tx,
            Arc::new(IdleCoalescer::new()),
        );
        let id = AgentId::new_v4();
        sink.inputs_drained(id, 3);
        sink.turn_ended(id, 3);
        assert_eq!(
            drain_msgs(&mut rx),
            vec![FlushMsg::Idle { id }],
            "same doorbell as the turn end — if there is an unprocessed one they fold into each other"
        );
        assert_eq!(*seen.lock().unwrap(), vec![(id, 3)], "decorator contract");
    }

    #[test]
    fn turn_end_pushes_are_coalesced_per_agent_until_taken() {
        let (sink, mut rx) = flush_sink();
        let id = AgentId::new_v4();
        sink.turn_ended(id, 0);
        sink.turn_ended(id, 0);
        sink.turn_ended(id, 1);
        assert_eq!(
            drain_msgs(&mut rx),
            vec![FlushMsg::Idle { id }],
            "folds if there is an unprocessed one (reopens when the consumer picks it up — IdleCoalescer)"
        );
    }

    #[test]
    fn idle_coalescer_folds_pending_notifications_until_taken() {
        use engram_dashboard_messaging::busy::IdleNotifier;
        let (tx, mut rx) = mpsc::unbounded_channel::<FlushMsg>();
        let coalescer = Arc::new(IdleCoalescer::new());
        let notifier = ChannelIdleNotifier::new(tx, coalescer.clone());
        let a = AgentId::new_v4();
        let b = AgentId::new_v4();
        notifier.notify_idle(a);
        notifier.notify_idle(a);
        notifier.notify_idle(a);
        notifier.notify_idle(b);
        assert_eq!(
            drain_msgs(&mut rx),
            vec![FlushMsg::Idle { id: a }, FlushMsg::Idle { id: b }],
            "only 1 unprocessed per id (different ids do not fold into each other)"
        );
        coalescer.taken(a);
        notifier.notify_idle(a);
        assert_eq!(drain_msgs(&mut rx), vec![FlushMsg::Idle { id: a }]);
    }

    #[test]
    fn service_doorbell_shares_the_idle_channel_and_coalescing() {
        use engram_dashboard_messaging::service::FlushTrigger;
        let (tx, mut rx) = mpsc::unbounded_channel::<FlushMsg>();
        let coalescer = Arc::new(IdleCoalescer::new());
        let notifier = ChannelIdleNotifier::new(tx, coalescer.clone());
        let id = AgentId::new_v4();
        notifier.request_flush(id);
        notifier.request_flush(id);
        assert_eq!(drain_msgs(&mut rx), vec![FlushMsg::Idle { id }]);
    }

    // ── 7c. usage observation — flows only through the wrapped sink, without the doorbell ─────────────────────────────

    struct UsageRecordingInner {
        seen: Arc<Mutex<Vec<engram_dashboard_agent::usage::UsageObservation>>>,
    }
    impl StatusSink for UsageRecordingInner {
        fn status_changed(&self, _: AgentId, _: CoreStatus, _: u32) {}
        fn agent_list_updated(&self, _: Vec<CoreAgentInfo>) {}
        fn usage_observed(&self, obs: engram_dashboard_agent::usage::UsageObservation) {
            self.seen.lock().unwrap().push(obs);
        }
    }

    #[test]
    fn a_usage_observation_is_forwarded_to_the_wrapped_sink_and_rings_no_doorbell() {
        use engram_dashboard_agent::usage::{
            UsageObservation, UsageSource, UsageVendorKey, WindowObs,
        };
        let (tx, mut rx) = mpsc::unbounded_channel::<FlushMsg>();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = MessagingFlushSink::new_test(
            Box::new(UsageRecordingInner { seen: seen.clone() }),
            tx,
            Arc::new(IdleCoalescer::new()),
        );
        let obs = UsageObservation {
            vendor: UsageVendorKey::new("test-vendor"),
            five_hour: Some(WindowObs {
                used_pct: Some(12.5),
                resets_at: Some(1_900_000_000),
            }),
            weekly: None,
            model_scoped: None,
            plan: Some("pro".to_string()),
            source: UsageSource::Passive,
            limits_unavailable: None,
        };

        sink.usage_observed(obs.clone());

        assert_eq!(
            *seen.lock().unwrap(),
            vec![obs],
            "decorator contract — the received observation as is, once"
        );
        assert!(
            drain_msgs(&mut rx).is_empty(),
            "usage is not a flush trigger"
        );
    }

    // ── 9b. flush worker: 2-lane ownership/shutdown (round-3 finding 1) ────────────────────────────────

    fn lane_wiring() -> FlushWiring {
        FlushWiring {
            messaging: Arc::new(crate::control::mcp_server::MessagingSlot::new()),
            idle: Arc::new(IdleCoalescer::new()),
        }
    }

    #[tokio::test]
    async fn flush_worker_handles_shutdown_stops_both_lanes() {
        let (tx, rx) = mpsc::unbounded_channel::<FlushMsg>();
        let handles = spawn_flush_worker(rx, lane_wiring());
        // Put in one delivery work so the lane actually runs (messaging slot not injected, so it skips at once = deterministic).
        tx.send(FlushMsg::Idle {
            id: AgentId::new_v4(),
        })
        .expect("receiving lane receive");
        tokio::time::timeout(Duration::from_secs(6), handles.shutdown())
            .await
            .expect("shutdown must return within the belt");
        assert!(
            tx.send(FlushMsg::Idle {
                id: AgentId::new_v4()
            })
            .is_err(),
            "after shutdown the receiving lane no longer receives"
        );
    }

    // ── 9c. delivery lane: serial per recipient · parallel across recipients ─────────────────────────────────────────

    use engram_dashboard_messaging::envelope::Entrance;
    use engram_dashboard_messaging::service::{SendMeta, SendStatus};
    use engram_dashboard_messaging::SenderIdentity;

    /// Entry/exit marker of a delivery closure. The roster-lookup marker has no recipient; the injection marker has one (`LaneProbe` header).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Probe {
        EnterDrain,
        ExitDrain,
        EnterInject(AgentId),
        ExitInject(AgentId),
    }

    /// A fake DeliveryPort observing entry/exit of delivery closures with test-controlled timing.
    ///
    /// Two observation points (external facts fixed by kernel code):
    ///   - Roster lookup (`live_agents`): if that recipient's queue has mail, Appear (`flush_for`) or Idle (`flush_for_agent`) calls this port once at
    ///     the head of the drain. An Idle for an empty queue never calls it (the empty-queue early return of `flush_for_agent`); Appear calls it
    ///     regardless — the order of two deliveries to one recipient is told apart by this.
    ///   - Injection (`inject`): an awake recipient's drain calls it with that id; the only marker carrying the recipient.
    ///
    /// Mail is parked while asleep (`park_while_asleep`): a send to an awake recipient drains at once on the test thread (the marker is stamped
    /// outside the lane); a send to a dormant recipient only parks.
    struct LaneProbe {
        agents: Vec<(AgentId, String)>,
        /// False = everyone dormant (roster empty, seen only on the dormant side) · true = everyone awake.
        awake: std::sync::atomic::AtomicBool,
        events: mpsc::UnboundedSender<Probe>,
        /// A closure that entered with this marker blocks until it gets a permit.
        blocked: Vec<Probe>,
        /// Permit channel — each test send releases one blocked closure. The Mutex is because `Receiver` is `!Sync` (the port must be `Sync`); the
        ///   lock is held while waiting, and the marker send is before it so a blocked closure's entry is observed too.
        release: Mutex<std::sync::mpsc::Receiver<()>>,
        inflight: std::sync::atomic::AtomicUsize,
        peak_inflight: std::sync::atomic::AtomicUsize,
    }

    impl LaneProbe {
        fn agents(&self) -> Vec<LiveAgent> {
            if !self.awake.load(std::sync::atomic::Ordering::SeqCst) {
                return Vec::new();
            }
            self.agents
                .iter()
                .map(|(id, name)| LiveAgent {
                    id: *id,
                    name: name.clone(),
                    epoch: 0,
                    turn_signal: false,
                })
                .collect()
        }

        fn wake(&self) {
            self.awake.store(true, std::sync::atomic::Ordering::SeqCst);
        }

        fn checkpoint(&self, enter: Probe, exit: Probe) {
            use std::sync::atomic::Ordering::SeqCst;
            let n = self.inflight.fetch_add(1, SeqCst) + 1;
            self.peak_inflight.fetch_max(n, SeqCst);
            let _ = self.events.send(enter);
            if self.blocked.contains(&enter) {
                self.release
                    .lock()
                    .expect("permit channel poisoned")
                    .recv()
                    .expect("the permit channel must be alive");
            }
            self.inflight.fetch_sub(1, SeqCst);
            let _ = self.events.send(exit);
        }

        fn peak(&self) -> usize {
            self.peak_inflight.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl DeliveryPort for LaneProbe {
        fn inject(&self, to_id: PeerId, bytes: &[u8]) -> Result<InjectReceipt, String> {
            self.checkpoint(Probe::EnterInject(to_id), Probe::ExitInject(to_id));
            Ok(InjectReceipt {
                bytes_requested: bytes.len(),
                bytes_written: bytes.len(),
                msg_uuid: PeerId::new_v4(),
                epoch: 0,
            })
        }
        fn live_agents(&self) -> Vec<LiveAgent> {
            self.checkpoint(Probe::EnterDrain, Probe::ExitDrain);
            self.agents()
        }
        fn addressing_sources(&self) -> AddressingSources {
            let roster = self.agents();
            let dormant = if roster.is_empty() {
                self.agents
                    .iter()
                    .map(|(id, name)| DormantAgent {
                        id: *id,
                        name: name.clone(),
                    })
                    .collect()
            } else {
                Vec::new()
            };
            AddressingSources {
                roster,
                dormant,
                pcs: Vec::new(),
            }
        }
        fn is_agent_live(&self, id: PeerId) -> bool {
            self.agents().iter().any(|a| a.id == id)
        }
    }

    struct LaneHarness {
        handles: FlushWorkerHandles,
        tx: mpsc::UnboundedSender<FlushMsg>,
        events: mpsc::UnboundedReceiver<Probe>,
        release: std::sync::mpsc::Sender<()>,
        probe: Arc<LaneProbe>,
        svc: Arc<MessagingService>,
    }

    impl LaneHarness {
        /// Parks one letter for a dormant recipient — only parks, no flush (`LaneProbe` header).
        fn park_while_asleep(&self, name: &str) {
            let rows = self
                .svc
                .handle_send(
                    &format!("m-{name}"),
                    SenderIdentity {
                        peer_id: PeerId::new_v4(),
                        epoch: 0,
                        node: None,
                    },
                    "sender",
                    &[name.to_string()],
                    "hi",
                    Entrance::Mcp,
                    &SendMeta::default(),
                )
                .expect("send accepted");
            assert_eq!(
                rows[0].status,
                SendStatus::Pending,
                "dormant recipient → parked: {rows:?}"
            );
        }
    }

    /// A real `MessagingService` (fake port) + a real 2-lane worker assembly — only lane scheduling is observed. Recipients start dormant (woken with `LaneProbe::wake`).
    fn lane_harness(agents: Vec<(AgentId, String)>, blocked: Vec<Probe>) -> LaneHarness {
        let (ev_tx, events) = mpsc::unbounded_channel::<Probe>();
        let (release, release_rx) = std::sync::mpsc::channel::<()>();
        let probe = Arc::new(LaneProbe {
            agents,
            awake: std::sync::atomic::AtomicBool::new(false),
            events: ev_tx,
            blocked,
            release: Mutex::new(release_rx),
            inflight: std::sync::atomic::AtomicUsize::new(0),
            peak_inflight: std::sync::atomic::AtomicUsize::new(0),
        });
        let svc = Arc::new(MessagingService::new(
            probe.clone(),
            Arc::new(ControlRegistry::new()),
        ));
        let slot = Arc::new(crate::control::mcp_server::MessagingSlot::new());
        slot.set(svc.clone());
        let (tx, rx) = mpsc::unbounded_channel::<FlushMsg>();
        let handles = spawn_flush_worker(
            rx,
            FlushWiring {
                messaging: slot,
                idle: Arc::new(IdleCoalescer::new()),
            },
        );
        LaneHarness {
            handles,
            tx,
            events,
            release,
            probe,
            svc,
        }
    }

    /// Receives the next marker with a bounded wait — the timeout is a failure backstop, not synchronization (a missing marker is itself a regression).
    async fn next_probe(events: &mut mpsc::UnboundedReceiver<Probe>) -> Probe {
        tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("the delivery marker must arrive within 5s — if it does not, that delivery could not even start")
            .expect("the marker channel must be alive")
    }

    /// A blocked recipient does not block another's delivery: one delivery is held over 0.5s by submit pacing, and a single queue would make
    /// even a recipient needing no pacing pay it. Block a's injection and see whether b reaches injection.
    #[tokio::test]
    async fn a_stalled_recipient_does_not_hold_up_another_recipients_delivery() {
        let a = AgentId::new_v4();
        let b = AgentId::new_v4();
        let mut h = lane_harness(
            vec![(a, "alpha".to_string()), (b, "bravo".to_string())],
            vec![Probe::EnterInject(a)],
        );
        h.park_while_asleep("alpha");
        h.park_while_asleep("bravo");
        h.probe.wake();

        h.tx.send(FlushMsg::Idle { id: a })
            .expect("receiving lane receive");
        assert_eq!(next_probe(&mut h.events).await, Probe::EnterDrain);
        assert_eq!(next_probe(&mut h.events).await, Probe::ExitDrain);
        assert_eq!(next_probe(&mut h.events).await, Probe::EnterInject(a));
        h.tx.send(FlushMsg::Idle { id: b })
            .expect("receiving lane receive");

        assert_eq!(
            next_probe(&mut h.events).await,
            Probe::EnterDrain,
            "b's delivery must start even while a is blocked"
        );
        assert_eq!(next_probe(&mut h.events).await, Probe::ExitDrain);
        assert_eq!(next_probe(&mut h.events).await, Probe::EnterInject(b));
        assert_eq!(
            next_probe(&mut h.events).await,
            Probe::ExitInject(b),
            "b must run through while a is still blocked"
        );
        assert!(
            h.probe.peak() >= 2,
            "the two recipients' deliveries must actually have run overlapping (if they flow sequentially this test is meaningless)"
        );

        h.release.send(()).expect("permit");
        assert_eq!(next_probe(&mut h.events).await, Probe::ExitInject(a));
        h.handles.shutdown().await;
    }

    /// Deliveries of one doorbell id do not overlap and run in dispatch order — this underpins the batch's "oldest first". (Two deliveries to
    /// one session not overlapping is not this axis: it is the kernel's target-axis guard noted in the `run_flush_lane` header, tested in the kernel.)
    ///
    /// Telling the order: put Appear then Idle with the queue empty. Appear reads the roster even for an empty queue (the first marker), and a
    /// letter is parked while it is blocked. The second marker can then come only from an Idle that started after the queue filled — an Idle run
    /// first would end without a marker, and overlapping runs would show the second before the first is released.
    #[tokio::test]
    async fn same_recipient_deliveries_are_serialized_in_dispatch_order() {
        let a = AgentId::new_v4();
        let mut h = lane_harness(vec![(a, "alpha".to_string())], vec![Probe::EnterDrain]);

        // Put both in first — without serialization both deliveries would be in flight together and the assertion below catches it.
        h.tx.send(FlushMsg::Appear { id: a })
            .expect("receiving lane receive");
        h.tx.send(FlushMsg::Idle { id: a })
            .expect("receiving lane receive");

        assert_eq!(
            next_probe(&mut h.events).await,
            Probe::EnterDrain,
            "the Appear dispatched first starts first (an Idle on an empty queue emits no marker)"
        );
        h.park_while_asleep("alpha");
        h.release.send(()).expect("permit");
        assert_eq!(next_probe(&mut h.events).await, Probe::ExitDrain);
        assert_eq!(
            next_probe(&mut h.events).await,
            Probe::EnterDrain,
            "the next delivery (Idle — the queue is full) starts only after the previous one ends"
        );
        h.release.send(()).expect("permit");
        assert_eq!(next_probe(&mut h.events).await, Probe::ExitDrain);

        assert_eq!(
            h.probe.peak(),
            1,
            "deliveries to the same recipient never overlap (prevents envelopes merging)"
        );
        h.handles.shutdown().await;
    }
}
