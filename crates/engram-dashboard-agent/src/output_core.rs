//! OutputCore — a shared struct that separates an agent's output-side core state (seq/replay/subscribers/status)
//! and the behavior on top of it (emit/finish/subscribe/...) out of transport · session.
//!
//! Why separate: output fanout · terminal transitions · subscription are identical for PTY and API,
//! regardless of the transport kind. A transport only needs to produce bytes/events and hand them to `emit`/`finish`.
//!
//! The transport (pump) hands output and termination over via emit/finish, and manager/AgentSession query via subscribe/
//! status/snapshot.
//!
//! tauri import 0. The S9 drain/session lock discipline and invariants are preserved literally.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use engram_dashboard_base::panic::uncontained;

use crate::backend::TurnClassifier;
use crate::inputs_pending::InputsPendingTable;
use crate::queued_input::{QueuedInputs, RowPhase};
use crate::turn::{TurnObservations, TurnSignal};
use crate::types::{
    AgentId, AgentStatus, DropCause, OutputChunk, OutputEvent, OutputFrame, OutputPayload,
    OutputSink, QueuedInputEvent, ReplayKind, SinkId, StatusSink, SubscribeOutcome, TerminalReason,
    TurnOutcome,
};

type OnTerminalHook = Box<dyn Fn(TerminalReason) + Send + Sync>;

/// The cap of the diagnostic stream buffer — it holds only this much of this incarnation's stderr, counted **from the end**.
///
/// ★It is a different thing from the output ring — do not merge them★: the ring is a stream that the terminal screen and the stream-json parser consume **together**,
///   so mixing stderr in there wedges a non-JSON line in the middle of NDJSON, breaking the parser and leaking diagnostics
///   onto the screen too (source of record for the grounds = the ADR-0044 comment in the `transport::stdio` module). So it is a separate small buffer.
/// ★Size grounds★: the only consumer reading this buffer is activation-failure classification (the early decision in `AgentManager`),
///   and what it looks for is the one-line diagnostic text claude emits. A few KB keeps that line from being pushed out by later output.
///   ★The cap must not be removed★ — this buffer grows for as long as the session lives, so if unbounded, an agent that spews
///   progress logs leaks memory as is.
// ADR-0172
const DIAGNOSTIC_CAP_BYTES: usize = 8 * 1024;

/// An agent's output-side core state. An independent Mutex per field (same as the split motive in the session.rs module comment):
/// while emit holds only the replay/subscribers lock briefly, it can run in parallel with other paths (status, etc.) without deadlock.
pub struct OutputCore {
    // ── Immutable (no change after creation) ──────────────────
    id: AgentId,
    epoch: u32,

    // ── Output sequence / status ──────────────────────────────
    seq: AtomicU64,
    status: Mutex<AgentStatus>,
    finalized: AtomicBool,

    // ── Output subscription ───────────────────────────────────
    subscribers: Mutex<Vec<Arc<dyn OutputSink>>>,

    // ── Replay buffer ─────────────────────────────────────────
    replay: Mutex<Ring>,

    // ── Diagnostic stream (ADR-0172 classification entrance) ──
    /// The tail (bounded) of this incarnation's **stderr text**. A separate buffer **split** from the output ring, and its reason and
    /// size grounds are in `DIAGNOSTIC_CAP_BYTES`. In a PTY session stderr arrives merged into the console stream, so this is
    /// always empty here (normal — the evidence on that side is carried by `terminal_tail`).
    diagnostics: Mutex<String>,

    // ── Status notification ───────────────────────────────────
    status_sink: Arc<dyn StatusSink>,

    // ── Pump thread control ───────────────────────────────────
    drain_handle: Mutex<Option<JoinHandle<()>>>,
    drain_done_rx: Mutex<Option<Receiver<()>>>,

    // ── Finalize once hook (ADR-0019 reaper) ──────────────────
    /// Unit tests use only OutputCore::new and do not inject the hook, so it is an Option (None = no-op).
    on_terminal: Mutex<Option<OnTerminalHook>>,

    // ── ADR-0113 turn observation ─────────────────────────────
    /// Immutable after creation, so there is neither an atomic load nor a lock on the hot path.
    turn: TurnWiring,

    // ── ADR-0231 pending-input registry ───────────────────────
    /// This incarnation's registry — the session reads through this Arc. The only way to change the registry is the reduction this core feeds inside the replay lock
    /// (so registry = the reduced value of the ring prefix).
    queued_inputs: Arc<QueuedInputs>,
    /// The pending-list table that mail busyness reads. `None` = an assembly where the registry is not visible to mail (harness) — no table, no doorbell either.
    inputs_pending: Option<Arc<InputsPendingTable>>,
    /// 「봉인됨」 (sealed) — set by the terminal synthesis of `finish`. A `Queued` after it stands in the ring as `Dropped{AgentEnded}`.
    /// ★Read and written only inside the replay lock★ — the order (whether it is after the synthesis swept) is carried by that lock and the atomic value is only interior mutability, so it is
    /// `Relaxed`. If a spot that reads outside the lock is made, a `Queued` slips out between the synthesis and the seal.
    sealed: AtomicBool,
}

/// Pending-input registry wiring (ADR-0231) — the registry and the pending-list table are always plugged in together. With a registry but no table, mail reads as idle even while the user
/// list is full and reaches stdin before the user's text.
///
/// ★`registry` is a new registry dedicated to that incarnation★ — created anew for each incarnation (the tombstone and 「받음 불가 판명」 (acceptance-impossible finding) are incarnation facts).
/// ★`pending` must be the table on which that incarnation was `register`ed★ — the table drops writes without registration, so if missed, the list
/// never shows to mail (and there is no error).
// ADR-0231
pub struct QueuedWiring {
    pub registry: Arc<QueuedInputs>,
    pub pending: Arc<InputsPendingTable>,
}

/// What one list event left inside the replay lock — used by the steps after the lock is released (turn observation · 「비었다」 (became empty) · fanout).
struct ListStep {
    /// Lines that stood in the ring (in issue order) — through rewriting, one event can become two lines.
    numbered: Vec<(u64, OutputEvent)>,
    /// The slot for the original event to hand to the classifier. `None` = the original event was rewritten as a seal.
    classify_at: Option<usize>,
    /// This event emptied the registry — with that seq write 「비었다」 (became empty) and ring the doorbell (outside the lock, after turn observation).
    drained_at: Option<u64>,
    /// This span was the first to reduce this incarnation's 「받음 불가 판명」 (acceptance-impossible finding) — leave the decision log once outside the lock.
    ack_unavailable_judged: bool,
}

/// One set of turn-observation wiring (ADR-0113) — the table (where to accumulate) and the classifier (what is a signal) are always plugged in together.
/// Keeping them separate would create a half state of "a table exists but no classifier".
///
/// ★The point of this type is that it is a **required argument** of `OutputCore::new` (ADR-0002 §0 — lay down early
///   the boundaries that are cheap and long-lived)★: with post-hoc injection (a setter), even if a new spawn path misses it, it compiles, and that agent is
///   **reported as idle forever** and mail gets inserted mid-turn — with neither error nor log. Making it an argument turns that mistake into a
///   compile error. An assembly that needs no observation is **stated explicitly** with `detached()` (distinct from a miss).
pub struct TurnWiring {
    table: Arc<TurnObservations>,
    classify: TurnClassifier,
}

impl TurnWiring {
    /// Production wiring — the manager's shared table + that session backend's classifier (`backend::turn_classifier`).
    pub fn new(table: Arc<TurnObservations>, classify: TurnClassifier) -> Self {
        Self { table, classify }
    }

    /// ★Only for assemblies that do not use observation (harness · standalone smoke)★ — its own table that nobody reads + a silent classifier.
    ///   It produces no signal at all, so from the observation consumer's viewpoint it equals "that agent has never been observed".
    ///
    /// ★The production spawn path never uses it★: if used, that agent's turn gate is silently turned off
    ///   (= mid-turn injection). Production must always use `new(manager 표, backend 분류자)` — that is why the names
    ///   differ. ★There is **no** device that enforces that distinction★: unlike the `use tauri` isolation or messaging isolation, there is
    ///   no grep gate that blocks this call, so this is a boundary kept by convention, not by the compiler (whether production calls are 0 is
    ///   checked in review).
    ///
    /// ★Why it was not locked behind the `test-harness` feature (a recorded trade-off)★: doing so would make core's own integration
    ///   tests (`tests/*.rs`) link the lib **as an external crate** with that feature off, so a
    ///   self-dev-dependency would have to be added to core too. That pattern already exists in this workspace (the ADR-0088 trick in daemon Cargo.toml)
    ///   — so it is not because it is impossible. What weighs is the cost that file documented together:
    ///   cargo takes the **union** of one crate's features within a single invocation, so building the lib and the
    ///   test targets together with `--all-targets` unifies that feature into the shared lib build and **embeds the harness
    ///   surface in the shipped artifact**. We judged the price of widening that risk to core now to be
    ///   larger than the defense a "constructor with a different name" already gives. If the decision changes to locking it, the release build rule (`--all-targets` forbidden) will apply to core too.
    #[doc(hidden)]
    pub fn detached() -> Self {
        Self {
            table: Arc::new(TurnObservations::new()),
            classify: crate::backend::no_turn_signals,
        }
    }
}

impl OutputCore {
    pub fn new(
        id: AgentId,
        epoch: u32,
        status_sink: Arc<dyn StatusSink>,
        turn: TurnWiring,
    ) -> Self {
        Self {
            id,
            epoch,
            seq: AtomicU64::new(0),
            status: Mutex::new(AgentStatus::Running),
            finalized: AtomicBool::new(false),
            subscribers: Mutex::new(Vec::new()),
            replay: Mutex::new(Ring::new()),
            diagnostics: Mutex::new(String::new()),
            status_sink,
            drain_handle: Mutex::new(None),
            drain_done_rx: Mutex::new(None),
            on_terminal: Mutex::new(None),
            turn,
            queued_inputs: Arc::new(QueuedInputs::new()),
            inputs_pending: None,
            sealed: AtomicBool::new(false),
        }
    }

    /// Plugs in the pending-input registry — if not called (default), there is a registry of its own that nobody reads, and no table. Sealing · rewriting · copy
    /// filling run the same either way (the ring's shape is independent of wiring).
    /// ★Call it right after creation, before the first event★ — if swapped in later, the registry is no longer the reduced value of the ring prefix.
    /// ★Production spawn must always call it★ — it compiles even if missed, and that incarnation's user list is not visible to mail.
    // ADR-0231
    pub fn with_queued(mut self, wiring: QueuedWiring) -> Self {
        self.queued_inputs = wiring.registry;
        self.inputs_pending = Some(wiring.pending);
        self
    }

    /// This incarnation's registry. ★Do not emit into this core while holding the guard★ — the lock order is replay → registry, so you would take them backwards.
    // ADR-0231
    pub fn queued_inputs(&self) -> &Arc<QueuedInputs> {
        &self.queued_inputs
    }

    pub fn id(&self) -> AgentId {
        self.id
    }

    /// Inject the finalize-time hook — spawn_session calls it once before registering in the sessions map.
    /// The closure is called only on the finalized.swap winner path (= exactly once). ★Race prevention★:
    /// inside the closure, intent · shutting_down must be snapshotted at that moment to build and send the ReapMsg.
    pub fn set_on_terminal(&self, hook: OnTerminalHook) {
        *self.on_terminal.lock().expect("on_terminal poisoned") = Some(hook);
    }

    /// ADR-0079: on a resume spawn, **seed** the replay buffer with past events restored from the `.jsonl` transcript.
    ///
    /// ★seed-before-publish invariant (ADR-0079, load-bearing — seed before publish: closes
    ///   empty-ring-replay + seq-interleave window, cross-family review 2026-07-13)★: manager.
    ///   spawn_session calls this **before inserting this core into the sessions map** (= before the session becomes observable).
    ///   Both the subscribe and emit paths go through the sessions map lookup (get_session), so before the insert no other
    ///   thread can reach this core → the windows are blocked at the source where a reconnecting subscription during seeding replays an empty Ring (seed does not
    ///   fanout → past is lost permanently), or a concurrent emit scrambles seq (Ring order [0,2,1]).
    ///   Being before the insert, it is naturally also before the pump (live emit) starts — so there are no subscribers or live events yet.
    ///
    /// ★No fanout (a property of seed time)★: unlike emit, it does not send to subscribers — there are no
    ///   subscribers yet so it is meaningless, and even if there were, seed is a "buffer pre-load" and must be
    ///   delivered only through the replay path (subscribe→replay) (it is not a live fanout). So it takes only the replay lock briefly, pushes to the Ring, and only advances
    ///   seq (ADR-0006 lock discipline kept — the subscribers lock is not taken).
    ///
    /// ★seq continuity★: for each seeded event it issues seq via `seq.fetch_add(1)`, same as emit. So
    ///   the seq of the first live emit after the seed naturally continues as the last seed seq + 1 (no gap or duplicate).
    ///   It does not touch finalize · status (ADR-0005 — seed is not a terminal transition).
    ///
    /// ★It does not touch turn observation either — do not revive it (ADR-0113)★: the transcript is a **past record**, so
    ///   it may have been cut mid-turn (killed incarnation), in which case it ends with progress signals and no end signal. If that is
    ///   fed as observation, the new incarnation is marked "in turn" and that turn's end **never comes** — consumers
    ///   waiting on that state (mail parking, etc.) stall with no way to wake them. Observation starts only from live emit.
    pub fn seed(&self, events: Vec<OutputEvent>) {
        let mut replay = self.replay.lock().expect("replay poisoned");
        for event in events {
            debug_assert_not_list_event(&event);
            let seq = self.seq.fetch_add(1, Ordering::Relaxed);
            let cost_bytes = estimate_cost_bytes(&event);
            replay.push(StoredOutput {
                seq,
                event,
                cost_bytes,
            });
        }
    }

    /// **payload-generic (S15 B4)** — it takes every OutputEvent variant (console bytes + structured),
    /// stores it in the Ring, and fans out payload-generically (ADR-0002, no assumption on output kind).
    ///
    /// ★Core invariants (ADR-0006 §10 rule 3 — only the payload changes, the lock structure is unchanged)★
    /// - No lock is held when `sink.send()` is called. Take a snapshot of the subscribers by clone,
    ///   release the lock immediately, then loop over the copy and send.
    /// - The replay lock and the subscribers lock are not held at the same time (each briefly).
    ///   Taking both locks at once is an exception for the subscribe function alone; it is strictly forbidden in emit.
    pub fn emit(&self, event: OutputEvent) {
        self.emit_inner(event, true);
    }

    /// Emit the same event **exactly the same**, but ★do not count this line as 「이 화신이 턴 중이라는 증거」 (evidence that this incarnation is in a turn)★
    /// (nothing is written to ADR-0113's fact layer).
    ///
    /// ★Reason to exist★: 「화면에 보여야 한다」 (it must be visible on screen) and 「이 화신이 턴 중이다」 (this incarnation is in a turn) are not the same fact. If the peer
    ///   leaks a line of a **turn we never opened**, its content must be visible on screen (dropping it is a silent loss), but
    ///   if we record it as a progress signal, the end of that turn, not being ours, is blocked by the attribution gate — and with nobody able to
    ///   undo that fact, mail stays blocked until the 30-minute fail-open valve. This door closes that asymmetry.
    /// ★So an end that must move the turn table is not sent through this door★ — this door also skips the doorbell
    ///   (`StatusSink::turn_ended`), so if our turn's end is sent here, the turn table stays at 「턴 중」 (in turn)
    ///   and the consumer waiting for that end is not woken. ★The one exception is a boundary for the screen only★ — a `TurnEnd{turn_id:
    ///   None}` (③ below) that turns off only the frontend's wait indicator at a place where the turn table is
    ///   already idle, so there is nothing to write and no consumer to wake. If that were emitted through the normal door, the classifier would fold it into a clean end and erase the earlier error end
    ///   (`last_end_failed`) — mail that should stay stopped would flow. One-line decision: **this door is a place that writes
    ///   nothing to the turn table — neither progress nor end. A line that must move the turn table is not sent through this door.**
    /// ★Do not add cleanup callers (ADR-0127)★ — it writes no observation at all, so there is nothing to erase. So
    ///   the two places, `finish` + the finalize recheck in `emit`, stay as they are.
    /// ★There are three callers and **all in the same file**★ (`backend/codex/transport.rs`):
    ///   ① unattributed lines the peer leaks (including a late receipt that arrived outside our turn, and its bubble) ② the user bubble the path synthesizes when idle
    ///   (ADR-0127 — if emitted through the normal door, the classifier records it as progress, so when that text's `turn/start` is
    ///   rejected, a turn that was never opened remains 「진행 중」 (in progress)) ③ the screen-only
    ///   boundary set after a receipt that arrived when there was no open turn (the exception above — ADR-0231).
    ///   ★The past screen fetched as a page right after resume (ADR-0203) comes not through this door but through
    ///   [`Self::emit_batch_without_turn_observation`]★ — it must be loaded as one chunk so it is not split in the ring.
    ///   That the chunk comes through a fanout door rather than `seed` is **because the timing differs** — `seed` is right only
    ///   before the session is on the registry (there are no subscribers even without fanout), and that chunk comes after the handshake, so there may already be
    ///   attached subscribers. If a door without fanout were used there, that subscriber would have replayed an empty ring and would **never see**
    ///   the past screen.
    // ADR-0113
    // ADR-0127
    pub(crate) fn emit_without_turn_observation(&self, event: OutputEvent) {
        self.emit_inner(event, false);
    }

    /// Emit **several items as one chunk** through the same door — ★no other emit can slip in between★.
    ///
    /// ★The reason to exist = 「끼어들 수 없다」 (cannot slip in), just that★: if called singly, each call takes the replay lock separately, so
    ///   at each gap the pump's live emit can take a seq. A new line stuck **in the middle** of the restored conversation
    ///   is the result (this is why ADR-0203's history restore uses this door). Issuing seq consecutively
    ///   within one lock span makes that chunk always contiguous in the ring.
    /// ★★That 「끼어들 수 없다」 (cannot slip in) is true **only inside the ring** — do not read it as extending to delivery★★: the implementation below
    ///   does fanout after releasing the lock (the grounds for that discipline = [`Self::emit`]), so **already attached** subscribers
    ///   can receive them in the order [chunk seq N] → [live seq N+k] → [chunk seq N+1]. The outcome is not discarding but
    ///   **reordering on the frontend** — the frontend (`src/api/protocolClient.ts`) always passes only `마지막+1` to the view, holds frames
    ///   after a gap, and passes them in seq order once the gap is filled (ADR-0231 — past the hold cap it returns to the buffer phase and
    ///   asks for a full replay again). So on screen they stand in ring order — only the wire arrival order gets scrambled.
    ///   ★**Later-attached** subscribers are unaffected★ (they replay from the ring, so the order is intact).
    /// ★And it cannot block what comes **before the chunk** either★ — a live line that arrived before this chunk still stands ahead.
    /// ★So what this door sells is just 「**링 안에서** 이력이 쪼개지지 않는다」 (history is not split inside the ring)★ — delivery order and 「이력이
    ///   맨 앞이다」 (history is at the very front) are not what this door gives, and the only way to close both is to hold the live emit at the
    ///   producer side up to the gate (that queue's cap and overflow handling are prerequisites so it is a separate matter — that separate matter was **closed as not
    ///   to be built**: ADR-0205. The limitation above stays open).
    /// ★The lock discipline is the same as [`Self::emit`] (ADR-0006)★ — issue · push inside the replay lock, fanout without the lock using
    ///   the subscribers snapshot. The two locks are not held together.
    /// ★Not touching observation · status · finalize is also the same as the single-item door★ (ADR-0005/0113/0127).
    // ADR-0203
    // ADR-0205
    pub(crate) fn emit_batch_without_turn_observation(&self, events: Vec<OutputEvent>) {
        if events.is_empty() {
            return;
        }
        // 1. Issue + push within **one** replay lock — this span is the actual form of 「끼어들 수 없다」 (cannot slip in).
        let numbered: Vec<(u64, OutputEvent)> = {
            let mut replay = self.replay.lock().expect("replay poisoned");
            events
                .into_iter()
                .map(|event| {
                    debug_assert_not_list_event(&event);
                    let seq = self.seq.fetch_add(1, Ordering::Relaxed);
                    let cost_bytes = estimate_cost_bytes(&event);
                    replay.push(StoredOutput {
                        seq,
                        event: event.clone(),
                        cost_bytes,
                    });
                    (seq, event)
                })
                .collect()
        };

        // 2. Fanout after releasing the lock.
        self.fan_out(&numbered);
    }

    /// Send several lines through the subscribers snapshot without a lock — dead sinks are reaped in one go
    /// the same way as the single-item door. The caller calls it after releasing the replay lock (ADR-0006).
    fn fan_out(&self, numbered: &[(u64, OutputEvent)]) {
        if numbered.is_empty() {
            return;
        }
        let sinks = self
            .subscribers
            .lock()
            .expect("subscribers poisoned")
            .clone();
        let mut dead = Vec::new();
        for sink in sinks {
            for (seq, event) in numbered {
                let payload = match event {
                    OutputEvent::TerminalBytes(v) => OutputPayload::Bytes(v),
                    other => OutputPayload::Event(other),
                };
                if sink
                    .send(OutputFrame {
                        agent_id: self.id,
                        epoch: self.epoch,
                        seq: *seq,
                        payload,
                    })
                    .is_err()
                {
                    dead.push(sink.sink_id());
                    break;
                }
            }
        }
        if !dead.is_empty() {
            self.subscribers
                .lock()
                .expect("subscribers poisoned")
                .retain(|s| !dead.contains(&s.sink_id()));
        }
    }

    /// Pass the usage observation the decoder yielded to the status sink — the output pump hands over the decoder's `take_usage` result as is
    /// ([`crate::transport::OutputDecoder::take_usage`]).
    /// ★It does not touch the ring · seq · subscribers · turn observation★ — usage is not this incarnation's output but account-level state.
    /// ★Called holding no lock at all★ — this is where the side that trusts the sink contract (non-blocking, non-reentrant) sits.
    pub(crate) fn report_usage(&self, observations: Vec<crate::usage::UsageObservation>) {
        // ADR-0292 decision 3: the observation fixes the account-level shared ledger — it turns off the indicator even when called inside the pump's containment.
        uncontained(|| {
            for obs in observations {
                self.status_sink.usage_observed(obs);
            }
        });
    }

    fn emit_inner(&self, event: OutputEvent, observe_turn: bool) {
        let event = match event {
            OutputEvent::QueuedInput(op) => return self.emit_list_event(op, observe_turn),
            other => other,
        };
        let cost_bytes = estimate_cost_bytes(&event);

        // 3~4. ★Issue seq + push to replay atomically inside the replay lock★ — a brief lock (lock order stage 1,
        //    ADR-0006). **Order matters (gap prevention)**: replay.push must come before fanout, so that even if subscribe
        //    slips in between, the new sink receives this seq from replay (worst case dup, absorbed by the frontend's seq dedup).
        //    In reverse order a gap occurs.
        //
        // ★ADR-0079: issue seq inside the replay lock right before the push (why it must not be outside the lock)★
        //    The pump thread (transport stdio/pty) and write_input's synthetic user-echo (session.rs)
        //    call emit concurrently. If seq is fetch_add'ed outside the lock, two callers get N/N+1 yet
        //    the lock-entry order can flip, so N+1 may be pushed to the ring before N → the ring's seq ordering
        //    breaks. subscribe_from cuts the replay slice with `partition_point(|c| c.seq <= s)` (assuming seq ascending),
        //    so if the ring is non-monotonic, replay slicing collapses. Binding issue+push into the same lock span
        //    to make it atomic means lock acquisition order is the seq order = the ring is always monotonic. (Found in the cross-family review 2026-07-13
        //    — seed() also issues inside the lock the same way, so the two paths agree.)
        //    Ordering::Relaxed kept: the lock provides ordering (happens-before), so the atomic is responsible only for uniqueness.
        let seq;
        {
            let mut replay = self.replay.lock().expect("replay poisoned");
            seq = self.seq.fetch_add(1, Ordering::Relaxed);
            replay.push(StoredOutput {
                seq,
                event: event.clone(),
                cost_bytes,
            });
        }

        if observe_turn {
            self.record_turn_signal(seq, &event);
        }

        let payload = match &event {
            OutputEvent::TerminalBytes(v) => OutputPayload::Bytes(v),
            other => OutputPayload::Event(other),
        };

        // 5. ★Invariant 1 (ADR-0006 §10 rule 3)★ send may be blocking/try_send, so sending while holding a lock
        //    deadlocks or stalls with subscribe/other sends.
        let frame = OutputFrame {
            agent_id: self.id,
            epoch: self.epoch,
            seq,
            payload,
        };
        let sinks = self
            .subscribers
            .lock()
            .expect("subscribers poisoned")
            .clone();

        let mut dead = Vec::new();
        for sink in sinks {
            if sink.send(frame).is_err() {
                dead.push(sink.sink_id());
            }
        }

        if !dead.is_empty() {
            self.subscribers
                .lock()
                .expect("subscribers poisoned")
                .retain(|s| !dead.contains(&s.sink_id()));
        }
    }

    /// The door for a list event (`QueuedInput`) — the same stages as the single-item door, with registry reduction and the pending-list table write inserted among them.
    ///
    /// ★Inside the replay lock (one span)★: seal → rewrite after decision → fill the `AckUnavailable` copy → issue · push · reduce →
    ///   「찼다」 (became full). Since the registry is fed in this span, registry = the reduced value of the ring prefix, and the seal · rewrite decisions also follow the ring
    ///   order (they do not read the atomic `DeliveryAck` — if read, ring replay and the registry would see different orders).
    /// ★Outside the lock★: turn observation (only the original event that was not rewritten) → 「비었다」 (became empty) + doorbell → fanout.
    ///   Why 「비었다」 is written **after** turn observation: the claude `Delivered` that empties the list is progress of that turn, so if written inside
    ///   the lock, there is a moment before the progress reaches the table when both facts are 「한가」 (idle), and mail that asked for busyness then
    ///   gets folded into the user's turn. 「찼다」 is the opposite, inside the lock — if delayed, mail would read as idle even after `Queued` stands in the ring
    ///   and could reach stdin before the user's text. That the 「비었다」 deferred outside the lock does not overwrite a later
    ///   「찼다」 written by another thread in between is carried by the table's seq rule.
    // ADR-0231
    fn emit_list_event(&self, op: QueuedInputEvent, observe_turn: bool) {
        let step = {
            let mut replay = self.replay.lock().expect("replay poisoned");
            self.record_list_event(&mut replay, op)
        };
        self.finish_list_step(step, observe_turn);
    }

    /// The door for a cancel request — the registry check and the `CancelRequested` record are **one replay span**.
    ///
    /// ★The pump's outcome (`Delivered`, etc.) cannot slip between check and record★: the only way to change the registry is the reduction inside the replay lock,
    ///   so checking outside the lock and emitting separately would answer `Requested` for an item that closed in that gap and even send the
    ///   cancel line to the vendor. Here we record only when the check saw `Queued`, so the answer does not diverge from the registry.
    // ADR-0231
    pub(crate) fn emit_cancel_request(&self, id: &str) -> CancelRequest {
        let step = {
            let mut replay = self.replay.lock().expect("replay poisoned");
            let phase = self
                .queued_inputs
                .lock()
                .rows()
                .iter()
                .find(|row| row.id == id)
                .map(|row| row.phase);
            match phase {
                None => return CancelRequest::NotListed,
                Some(RowPhase::Cancelling { .. }) => return CancelRequest::AlreadyCancelling,
                Some(RowPhase::Queued) => self.record_list_event(
                    &mut replay,
                    QueuedInputEvent::CancelRequested { id: id.to_owned() },
                ),
            }
        };
        self.finish_list_step(step, true);
        CancelRequest::Recorded
    }

    /// The busyness the session classification reads — the pending-list table says 「찼다」 (full) or turn observation says in a turn. It does not read the registry.
    ///
    /// ★Table first, turn observation later★: the emit of the `Delivered` that empties the list writes 「비었다」 (became empty) **after** recording turn observation's progress,
    ///   so a read in this order that saw 「비었다」 also sees that progress. Reading in reverse or reading the registry lets input arriving in the middle of that emit
    ///   see 「빔 + 턴 없음」 (empty + no turn) and be classified as idle.
    /// ★An assembly without the table (harness) reads the list as always empty★ — only turn observation remains.
    // ADR-0231
    pub(crate) fn classified_input_busy(&self) -> bool {
        pending_then_in_turn(self)
    }

    /// Finish one span of a list event outside the lock — turn observation → 「비었다」 (became empty) + doorbell → fanout.
    fn finish_list_step(&self, step: ListStep, observe_turn: bool) {
        if step.ack_unavailable_judged {
            // ADR-0231: the old-CLI decision is once per incarnation (one span where the registry's finding marker goes false → true).
            tracing::info!(
                agent = %self.id,
                epoch = self.epoch,
                "받음 알림 불가 판정 — 이 화신은 대기 목록 없이 오늘 경로"
            );
        }
        if observe_turn {
            if let Some(at) = step.classify_at {
                let (seq, event) = &step.numbered[at];
                self.record_turn_signal(*seq, event);
            }
        }
        if let Some(seq) = step.drained_at {
            self.write_drained(seq);
        }
        self.fan_out(&step.numbered);
    }

    /// The list stage inside the replay lock span. The caller calls it holding the replay lock.
    // ADR-0231: lock order = (session `input_order` →) replay → registry → pending-list table. The registry guard is taken only inside this replay span,
    //   the table is a leaf, and `StatusSink` is called holding no lock (`write_drained`). If you make a place that takes the registry first
    //   and waits for replay, the two wait on each other — the side that reads only the registry (list query · cancel validation of the path policy)
    //   takes only the registry lock and does not emit, and the session classification's cancel validation reads under the replay lock
    //   (`emit_cancel_request`).
    fn record_list_event(&self, replay: &mut Ring, op: QueuedInputEvent) -> ListStep {
        let mut registry = self.queued_inputs.lock();
        let was_empty = registry.is_empty();
        let was_judged = registry.ack_unavailable_seen();
        // The seal comes before the rewrite — a `Queued` after termination is not a receipt but a drop.
        let (events, classify_at) = match op {
            QueuedInputEvent::Queued { id, .. } if self.sealed.load(Ordering::Relaxed) => {
                // This is a place where the user's text is dropped without delivery, so we leave a trace — the body is not put in (id only).
                tracing::debug!(
                    agent = %self.id,
                    epoch = self.epoch,
                    id = %id,
                    "봉인 뒤의 Queued 를 Dropped(AgentEnded) 로 바꿔 적음"
                );
                (
                    vec![QueuedInputEvent::Dropped {
                        id,
                        cause: DropCause::AgentEnded,
                    }],
                    None,
                )
            }
            // A `Queued` after the finding is closed on the spot as a receipt — the reason we chain an `AckUnavailable` carrying the copy rather than a `Delivered`:
            //   even if the ring cap cuts between the two lines, the closing line holds the body.
            queued @ QueuedInputEvent::Queued { .. } if registry.ack_unavailable_seen() => (
                vec![
                    queued,
                    QueuedInputEvent::AckUnavailable {
                        delivered: Vec::new(),
                    },
                ],
                Some(0),
            ),
            other => (vec![other], Some(0)),
        };
        let mut numbered = Vec::with_capacity(events.len());
        for mut op in events {
            // ★The core always fills the copy★ (the decoder emits it empty) — the open item **before** the reduction is the item that reduction
            //   closes with a receipt.
            if let QueuedInputEvent::AckUnavailable { delivered } = &mut op {
                *delivered = registry.open_copies();
            }
            let seq = self.seq.fetch_add(1, Ordering::Relaxed);
            registry.reduce(seq, &op);
            let event = OutputEvent::QueuedInput(op);
            let cost_bytes = estimate_cost_bytes(&event);
            replay.push(StoredOutput {
                seq,
                event: event.clone(),
                cost_bytes,
            });
            numbered.push((seq, event));
        }
        let now_empty = registry.is_empty();
        let ack_unavailable_judged = !was_judged && registry.ack_unavailable_seen();
        drop(registry);

        let last_seq = numbered.last().map(|(seq, _)| *seq).expect("한 줄 이상");
        let mut drained_at = None;
        match (was_empty, now_empty) {
            (true, false) => {
                if let Some(pending) = &self.inputs_pending {
                    pending.set(self.id, self.epoch, last_seq, true);
                }
            }
            (false, true) => drained_at = Some(last_seq),
            // A rewrite after the decision is empty → empty — nobody sees the moment the list was full, so there is nothing to write or ring.
            _ => {}
        }
        ListStep {
            numbered,
            classify_at,
            drained_at,
            ack_unavailable_judged,
        }
    }

    /// Write 「비었다」 (became empty) and ring the doorbell — holding no lock. An assembly without the table does neither.
    /// The table comes first: the side that gets the doorbell asks for busyness again right away.
    // ADR-0231
    fn write_drained(&self, seq: u64) {
        if let Some(pending) = &self.inputs_pending {
            pending.set(self.id, self.epoch, seq, false);
            self.status_sink.inputs_drained(self.id, self.epoch);
        }
    }

    /// Write the turn signal the classifier produced into the table, and ring the doorbell if it is an end. The caller calls it after releasing the replay lock.
    fn record_turn_signal(&self, seq: u64, event: &OutputEvent) {
        // ★ADR-0113 turn observation★: update the table **before fanout · notification**. The consumer that got the notification
        //   queries the table right away (doorbell→flush, etc.), so if the order flips, that query sees the pre-update value.
        //   Lock discipline: the table update takes only its own lock briefly (not holding the core lock), and the notification is made after releasing that lock.
        if let Some(signal) = (self.turn.classify)(event) {
            // ★Carry the **output order (seq)** on the signal★: there are two emit callers (the pump · the
            //   injection thread that produced the input echo), so if the two emits run in parallel, the table's apply order can flip against the emission order. seq is issued inside the replay
            //   lock and is the **canonical order of output**, so the table filters late signals with it (turn.rs).
            self.turn.table.observe(self.id, self.epoch, seq, signal);
            // ★So that a late emit after termination cannot revive a ghost entry (load-bearing)★: while the injection thread is blocked on the transport
            //   write, the pump can pass EOF→`finish` and empty the table, and the echo that wakes up afterwards
            //   recreates the entry with the **same epoch** (the table-side rule that drops only smaller epochs cannot stop it).
            //   That entry never gets an end signal, so nobody can erase it.
            //   ★What creates the order = **the table's mutex** (do not delete this argument)★: that lock puts our insert and
            //   `finish`'s forget in a **total order**. In the order where forget comes first, forget's unlock
            //   (release) synchronizes-with our lock (acquire), so the `finalized` swap before forget
            //   happens-before our load → the load cannot read false. In the opposite order our insert comes
            //   first, so the following forget erases it. Either way no ghost remains.
            //   ★So the `Acquire` of this load is not the grounds★ — the conclusion is the same with `Relaxed`, and what carries the order
            //   is the mutex. If the table update is "optimized" outside the lock (lock-free), this proof silently collapses.
            if self.finalized.load(Ordering::Acquire) {
                self.turn.table.forget(self.id, self.epoch);
            }
            // ★The notification does not apply epoch · finalize · seq gates (intentionally asymmetric with the table update above)★: the table is
            //   **state**, so if a dead/old incarnation writes it, the live incarnation's facts get polluted, but the doorbell is a **one-shot stimulus**,
            //   so a surplus is absorbed as an empty-queue no-op (the consumer re-verifies at run time) and an omission creates a wait
            //   ("omission < surplus" — the StatusSink::turn_ended contract).
            if matches!(signal, TurnSignal::Ended(_)) {
                self.status_sink.turn_ended(self.id, self.epoch);
            }
        }
    }

    /// Terminal transition — called once by the pump after it leaves the loop. The finalize exactly-once gate absorbs duplicate calls.
    ///
    /// The terminal notification emitter is the pump (= here) alone. The reason→AgentStatus mapping is exactly the impl-spec table.
    /// ★The remaining list items' `Dropped{AgentEnded}` go out before the terminal transition (ADR-0231)★ — subscribers see the terminal
    ///   after the list is closed.
    pub fn finish(&self, reason: TerminalReason) {
        if self.finalized.swap(true, Ordering::AcqRel) {
            return;
        }

        // ★ADR-0231 terminal synthesis + seal = one replay lock span★: single emits take the lock separately per call, so a `Queued` standing between the sweep and
        //   the seal slips past both and becomes a permanent item (= mail blocked with no cap). This chunk does not pass through turn
        //   observation (we do not add observation cleanup points — ADR-0127).
        let synthesized = {
            let mut replay = self.replay.lock().expect("replay poisoned");
            self.synthesize_ended_and_seal(&mut replay)
        };
        if !synthesized.is_empty() {
            tracing::info!(
                agent = %self.id,
                epoch = self.epoch,
                count = synthesized.len(),
                "종료 합성: 남은 대기 입력을 Dropped(AgentEnded) 로 닫음"
            );
        }
        // ★Even when terminal synthesis empties the list, it neither writes 「비었다」 (became empty) nor rings the doorbell★: this incarnation's entry in the pending-list table
        //   is reaped below together with the turn entry, so writing it would just be erased soon, and the doorbell would be a stimulus to flow mail into an incarnation
        //   about to make its terminal transition. Parked mail is carried by the next incarnation's appearance flush.
        // ADR-0231
        self.fan_out(&synthesized);

        // Adding AgentStatus variants is forbidden (impl-spec table).
        // ★reason must also be passed to the reaper hook, so clone it before mapping (preserve before consuming).
        let new_status = match reason.clone() {
            TerminalReason::Exited { code } => AgentStatus::Exited { code },
            TerminalReason::Killed => AgentStatus::Killed,
            TerminalReason::Interrupted => AgentStatus::Killed,
            TerminalReason::StreamClosed => AgentStatus::Exited { code: None },
            TerminalReason::Cancelled => AgentStatus::Killed,
            TerminalReason::Error(s) => AgentStatus::Failed { message: s },
        };

        {
            let mut status = self.status.lock().expect("status poisoned");
            *status = new_status.clone();
        }

        // ★ADR-0113: reap this incarnation's turn observation here★ — it is the finalize winner path so **exactly once** (ADR-0005),
        //   and an incarnation that has moved to terminal no longer produces facts. If not erased, an agent that died mid-turn stays
        //   "in turn" and consumers waiting on it (mail parking, etc.) are never released.
        //   ★Why here and not the reaper★: the reaper wakes only from the ReapMsg this finish sends, so it is **always later**,
        //   and above all it must be at the same point as the `finalized` flag so the race with the late insert on the emit side is closed by order
        //   (the finalize recheck comment in emit is the other half of that causality). The epoch-match check is done by forget.
        // ADR-0113
        self.turn.table.forget(self.id, self.epoch);
        // The pending-list table is also reaped at the same place. ★The emit-side finalize recheck is not needed for this table★ — the table's `set` does not
        //   create entries (only `register` does), so a late write cannot revive a reaped entry.
        // ADR-0231
        if let Some(pending) = &self.inputs_pending {
            pending.forget(self.id, self.epoch);
        }

        // External calls after releasing the status lock (§10: no external calls while holding the status lock).
        self.status_sink
            .status_changed(self.id, new_status, self.epoch);

        // ★ADR-0019 reaper hook★: take the on_terminal lock briefly and call it immediately without clone — safe because it is a span where no other core
        //   lock is held.
        if let Some(hook) = self
            .on_terminal
            .lock()
            .expect("on_terminal poisoned")
            .as_ref()
        {
            hook(reason);
        }
    }

    /// For each open item issue · push · reduce `Dropped{AgentEnded}` and set the seal. The caller calls it holding the replay lock
    /// (`finish` only). Returns = the lines that stood in the ring.
    // ADR-0231
    fn synthesize_ended_and_seal(&self, replay: &mut Ring) -> Vec<(u64, OutputEvent)> {
        let mut registry = self.queued_inputs.lock();
        let open: Vec<String> = registry.rows().iter().map(|row| row.id.clone()).collect();
        let mut numbered = Vec::with_capacity(open.len());
        for id in open {
            let op = QueuedInputEvent::Dropped {
                id,
                cause: DropCause::AgentEnded,
            };
            let seq = self.seq.fetch_add(1, Ordering::Relaxed);
            registry.reduce(seq, &op);
            let event = OutputEvent::QueuedInput(op);
            let cost_bytes = estimate_cost_bytes(&event);
            replay.push(StoredOutput {
                seq,
                event: event.clone(),
                cost_bytes,
            });
            numbered.push((seq, event));
        }
        debug_assert!(registry.is_empty(), "종료 합성 뒤 열린 항목이 남았다");
        self.sealed.store(true, Ordering::Relaxed);
        numbered
    }

    /// Transitional Exiting transition — for manager kill stage 0.5. This path is the Exiting notification emitter.
    /// Returns false if already finalized or terminal (no overwrite). Only when non-terminal such as Running,
    /// records Exiting + emits status_changed(Exiting), then true.
    ///
    /// **Protection scope (precisely):** what the finalized/terminal check below protects is the **status field value**
    /// (preventing terminal from being overwritten by Exiting and sticking). It does not protect the **notification order**
    /// going out to status_sink — it can race with finish's status_changed outside the lock.
    /// However, the normal kill path calls transport.shutdown() only after the manager completes enter_exiting()
    /// (only then does the pump wake and finish), so the Exiting notification completes before the terminal notification and the order is guaranteed.
    /// The notification inversion window is only "the moment a natural process exit overlaps with kill", and this is existing behavior that exists identically in the S9 original
    /// (manager.kill_agent's Exiting emission vs drain.transition's terminal emission).
    /// The frontend decides terminal not by status_changed but by agent-list-updated (the list)
    /// (`docs/reference/architecture.md` 「런타임 불변식」 (runtime invariants) I4), so even receiving a late Exiting it does not stick — mitigated by design.
    pub fn enter_exiting(&self) -> bool {
        // If termination has already been handled, do not write Exiting (fast path — the real status field protection is the lock span below).
        if self.finalized.load(Ordering::Acquire) {
            return false;
        }

        {
            let mut status = self.status.lock().expect("status poisoned");
            if matches!(
                *status,
                AgentStatus::Exited { .. } | AgentStatus::Killed | AgentStatus::Failed { .. }
            ) {
                return false;
            }
            *status = AgentStatus::Exiting;
        }

        // External calls after releasing the status lock.
        self.status_sink
            .status_changed(self.id, AgentStatus::Exiting, self.epoch);
        true
    }

    /// Wait for pump termination — kill stage 6. Discard the result so it can be ignored even if the receiver
    /// is already gone (detach after timeout).
    pub fn join_pump(&self, timeout: Duration) {
        let rx = self
            .drain_done_rx
            .lock()
            .expect("drain_done_rx poisoned")
            .take();
        if let Some(rx) = rx {
            let _ = rx.recv_timeout(timeout);
        }
    }

    /// Load the pump handle/done_rx — called after transport.start (stage 3) has launched the pump thread.
    pub fn attach_pump(&self, handle: JoinHandle<()>, done_rx: Receiver<()>) {
        *self.drain_handle.lock().expect("drain_handle poisoned") = Some(handle);
        *self.drain_done_rx.lock().expect("drain_done_rx poisoned") = Some(done_rx);
    }

    /// Subscriber registration + replay send. Returns a SinkId (for unsubscribe).
    ///
    /// **C4 (LLD §7, strictly observed):** the replay is sent while holding the subscribers lock.
    /// This way emit's live send and this replay send are serialized by the same subscribers lock,
    /// so a replay→live order inversion is blocked at the source. emit briefly waits trying to take the subscribers
    /// lock at step 5, but the replay send is one-shot so it is allowed.
    ///
    /// **Lock order rule 3 exception (LLD §10):** only the subscribe function takes both the subscribers→replay locks
    /// at once (always in this order). emit must never hold both locks at once.
    pub fn subscribe(&self, sink: Arc<dyn OutputSink>) -> SinkId {
        let sink_id = sink.sink_id();

        let mut subscribers_guard = self.subscribers.lock().expect("subscribers poisoned");
        subscribers_guard.push(sink.clone());

        let snapshot = {
            let replay_guard = self.replay.lock().expect("replay poisoned");
            replay_guard.snapshot()
        };

        // The snapshot's seq and the following live chunk's seq are not broken, so the frontend can dedup/sort by seq.
        // It is a just-registered sink, so a send failure is unlikely → ignore (§7).
        for stored in &snapshot {
            let payload = match &stored.event {
                OutputEvent::TerminalBytes(v) => OutputPayload::Bytes(v),
                other => OutputPayload::Event(other),
            };
            let frame = OutputFrame {
                agent_id: self.id,
                epoch: self.epoch,
                seq: stored.seq,
                payload,
            };
            let _ = sink.send(frame);
        }

        drop(subscribers_guard);

        sink_id
    }

    /// Selective replay subscription based on after_seq/epoch. It follows `subscribe`'s C4 pattern (sending the replay while holding the subscribers lock)
    /// as is, but branches the range to send.
    ///
    /// Branches:
    /// - epoch mismatch (epoch_matches=false) or after_seq=None → FromOldest (everything).
    /// - epoch match & after_seq=Some(s):
    ///     - if the buffer is empty → Resumed (send 0).
    ///     - s < oldest → Truncated (everything from oldest).
    ///     - s >= oldest → Resumed (only the tail with seq>s).
    ///
    /// `on_ready`: called once after the branch · meta (oldest/latest/kind/replay_from) are fixed, **right before the replay is
    ///   sent to the sink**. The daemon queues the SubscribeAck first inside it,
    ///   guaranteeing the "Ack→replay binary" FIFO order (invariant 2) while filling the Ack fields with the outcome
    ///   based on this single snapshot, eliminating TOCTOU (snapshot A/B mismatch). **No blocking inside on_ready**
    ///   (called while holding the subscribers lock — only non-blocking try_send).
    pub fn subscribe_from(
        &self,
        sink: Arc<dyn OutputSink>,
        after_seq: Option<u64>,
        epoch_matches: bool,
        on_ready: impl FnOnce(&SubscribeOutcome),
    ) -> SubscribeOutcome {
        let sink_id = sink.sink_id();
        let mut subscribers_guard = self.subscribers.lock().expect("subscribers poisoned");
        subscribers_guard.push(sink.clone());
        // ★Read the next issue seq under the same replay lock as the snapshot★ — all issuing is inside that lock (emit ·
        //   seed · batch · list event · terminal synthesis), so this value is the first seq that will come right after the snapshot.
        let (snapshot, next_seq) = {
            let replay_guard = self.replay.lock().expect("replay poisoned");
            (replay_guard.snapshot(), self.seq.load(Ordering::Relaxed))
        };

        let oldest = snapshot.first().map(|c| c.seq).unwrap_or(0);
        let latest = snapshot.last().map(|c| c.seq).unwrap_or(0);

        let (kind, start_idx) = match after_seq {
            _ if !epoch_matches => (ReplayKind::FromOldest, 0usize),
            None => (ReplayKind::FromOldest, 0usize),
            Some(s) => {
                if snapshot.is_empty() {
                    (ReplayKind::Resumed, 0usize)
                } else if s < oldest {
                    (ReplayKind::Truncated, 0usize)
                } else {
                    // Skip the prefix with seq<=s (seq continuity guaranteed → partition_point is safe).
                    let idx = snapshot.partition_point(|c| c.seq <= s);
                    (ReplayKind::Resumed, idx)
                }
            }
        };

        let to_send = &snapshot[start_idx..];

        // ★An empty ring is the next issue seq (ADR-0231)★ — the old `latest+1` read an empty ring's `latest` as 0, so on a new
        //   incarnation it became 1 (the first issue is 0). The view uses this value as the flush start point of an empty replay, so if it is one larger
        //   it skips a seq 0 that arrives late behind the marker. A non-empty ring keeps the old value.
        let replay_from = to_send.first().map(|c| c.seq).unwrap_or_else(|| {
            if snapshot.is_empty() {
                next_seq
            } else {
                match after_seq {
                    Some(s) => s.saturating_add(1),
                    None => latest.saturating_add(1),
                }
            }
        });

        let outcome = SubscribeOutcome {
            kind,
            sink_id,
            oldest_seq: oldest,
            latest_seq: latest,
            replay_from,
            replayed: to_send.len(),
        };

        on_ready(&outcome);

        for stored in to_send {
            let payload = match &stored.event {
                OutputEvent::TerminalBytes(v) => OutputPayload::Bytes(v),
                other => OutputPayload::Event(other),
            };
            let frame = OutputFrame {
                agent_id: self.id,
                epoch: self.epoch,
                seq: stored.seq,
                payload,
            };
            let _ = sink.send(frame);
        }
        drop(subscribers_guard);

        outcome
    }

    pub fn unsubscribe(&self, sink_id: SinkId) {
        self.subscribers
            .lock()
            .expect("subscribers poisoned")
            .retain(|s| s.sink_id() != sink_id);
    }

    /// Replay snapshot — for the initial restore of a late-attaching window (get_snapshot → wire SnapshotChunk path).
    ///
    /// ★Contract kept (B5)★: this getter still returns `Vec<OutputChunk>` (a byte-only wire mirror) —
    /// the callers (manager.get_snapshot → daemon snapshot_chunk_to_wire) depend on this shape. The Ring is
    /// payload-generic (StoredOutput), but this path's wire type (OutputChunk{seq,data}) is still byte-only, so it converts **only TerminalBytes**.
    /// The snapshot wire mapping of structured events belongs to B7 (daemon adapter), so it is skipped here — there is currently no structured event producer
    /// (B3 unwired), so no runtime loss.
    pub fn snapshot(&self) -> Vec<OutputChunk> {
        self.replay
            .lock()
            .expect("replay poisoned")
            .snapshot()
            .into_iter()
            .filter_map(|s| match s.event {
                OutputEvent::TerminalBytes(data) => Some(OutputChunk { seq: s.seq, data }),
                // ★Silent-loss observation hook★: the subscribe_from replay path is payload-generic and delivers normally, so
                // the two restore paths are asymmetric — it would become a silent loss the moment B3 is wired, so we leave the drop
                // observable as a warn.
                ref other => {
                    // Log only the variant tag (payload content is not put in the log — avoids sensitive/large data).
                    let kind = match other {
                        OutputEvent::TerminalBytes(_) => "TerminalBytes", // Handled by the arm above — never reached
                        OutputEvent::TextDelta { .. } => "TextDelta",
                        OutputEvent::ToolCall { .. } => "ToolCall",
                        OutputEvent::ToolResult { .. } => "ToolResult",
                        OutputEvent::Usage { .. } => "Usage",
                        OutputEvent::MessageDone { .. } => "MessageDone",
                        OutputEvent::TurnEnd { .. } => "TurnEnd",
                        OutputEvent::Error(_) => "Error",
                        OutputEvent::Structured { .. } => "Structured",
                        OutputEvent::QueuedInput(_) => "QueuedInput",
                    };
                    tracing::warn!(
                        seq = s.seq,
                        kind,
                        "structured event dropped from wire snapshot (B7 미배선 — get_snapshot 경로 무음 유실)"
                    );
                    None
                }
            })
            .collect()
    }

    /// Return the **console bytes** this incarnation last emitted, up to `max_bytes` (counted from the end).
    ///
    /// ★Not a sibling of `snapshot()` — do not use it instead★: `snapshot()` clones the whole ring
    ///   (documented max 2MB) and prints a warn for every item that is not console bytes. The caller of this getter looks at only a few lines of a dead session's
    ///   exit text, so there is no reason to pay that cost and log flood.
    /// ★The cap is kept exactly★: if the last chunk exceeds the cap, only the **end part** of that chunk is cut and kept.
    /// ★An empty result is normal★: the ring of a structured (NDJSON) session has no console bytes at all.
    // ADR-0172
    pub fn terminal_tail(&self, max_bytes: usize) -> Vec<u8> {
        self.replay
            .lock()
            .expect("replay poisoned")
            .terminal_tail(max_bytes)
    }

    /// Hold one line of this incarnation's diagnostic (stderr) — called by the transport's stderr drain thread.
    ///
    /// ★It must be **independent** of the log (load-bearing)★: the same line also goes out through `tracing::debug!(target: "agent_stderr")`
    ///   but the default log filter is `warn`, so that line is normally **discarded**. Hanging classification on that log makes
    ///   the log level switch a feature on and off (= a feature that works only on the developer's machine). So what accumulates here
    ///   happens regardless of level, and the caller calls this **before** the log macro.
    /// ★It takes masked text (caller's duty)★: it is external process output so credentials may be mixed in, and this
    ///   buffer, besides being read as classification grounds, is a place that may be carried into logs and reports as the failure reason.
    /// ★The cap is kept by discarding from the front★ — the latest lines are the evidence, so keep the back.
    // ADR-0172
    pub fn push_diagnostic(&self, line: &str) {
        let mut buf = self.diagnostics.lock().expect("diagnostics poisoned");
        buf.push_str(line);
        buf.push('\n');
        if buf.len() > DIAGNOSTIC_CAP_BYTES {
            // ★Push the cut position up to a **char boundary**★: cutting by bytes alone splits a
            //   multibyte character, breaks the `String` invariant, and `drain` panics — it actually hits when Korean · emoji
            //   are mixed into claude's diagnostics.
            let cut = buf.ceil_char_boundary(buf.len() - DIAGNOSTIC_CAP_BYTES);
            buf.drain(..cut);
        }
    }

    /// All the diagnostic text held so far (at most `DIAGNOSTIC_CAP_BYTES`).
    ///
    /// ★An empty result is normal★: a PTY session emits stderr merged into the console stream so it does not use this buffer
    ///   — that side's evidence is carried by `terminal_tail`. The caller passes the two **combined** to classification.
    // ADR-0172
    pub fn diagnostic_tail(&self) -> String {
        self.diagnostics
            .lock()
            .expect("diagnostics poisoned")
            .clone()
    }

    pub fn status(&self) -> AgentStatus {
        self.status.lock().expect("status poisoned").clone()
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use crate::types::AgentId;

    struct NoopStatus;
    impl StatusSink for NoopStatus {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
        fn agent_list_updated(&self, _agents: Vec<crate::types::AgentInfo>) {}
    }

    fn core() -> OutputCore {
        OutputCore::new(
            AgentId::new_v4(),
            1,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        )
    }

    /// The diagnostic buffer is **separate and empty by default** — nothing is leaked into the ring.
    ///
    /// ★What this assertion guards★: if a change merging this buffer into the output ring arrives, this goes red. Merging would wedge a non-JSON line
    ///   into the middle of the NDJSON parser and break the structured session, and diagnostics would leak onto the terminal screen too.
    #[test]
    fn diagnostics_never_leak_into_the_output_ring() {
        let core = core();
        core.push_diagnostic("No conversation found with session ID: 8b1c");

        assert!(
            core.diagnostic_tail().contains("No conversation found"),
            "진단은 진단 버퍼에 남는다"
        );
        assert!(
            core.terminal_tail(4096).is_empty(),
            "진단이 출력 링에 들어가면 안 된다 — 링은 화면과 stream-json 파서가 함께 먹는다"
        );
        assert!(core.snapshot().is_empty(), "링에 이벤트가 생겨서도 안 된다");
    }

    /// Even past the cap the **latest line stays**, and no multibyte character is split.
    ///
    /// ★The latter is the panic regression★: cutting by byte position alone breaks the `String` invariant and `drain` panics —
    ///   it actually hits when Korean · emoji are mixed into claude's diagnostics. That panic happens on the stderr drain thread (outside the containment marker),
    ///   so in the production process a hook ends the daemon (ADR-0292) — in a test build without the hook only diagnostic capture
    ///   silently dies.
    #[test]
    fn the_diagnostic_buffer_is_bounded_and_utf8_safe() {
        let core = core();
        // A line is purely multibyte, so wherever it is cut it lands on a boundary.
        let noisy = "가".repeat(512); // 3 bytes each = 1536B/line
        for _ in 0..32 {
            core.push_diagnostic(&noisy);
        }
        core.push_diagnostic("No conversation found with session ID: 8b1c");

        let tail = core.diagnostic_tail();
        assert!(
            tail.len() <= DIAGNOSTIC_CAP_BYTES,
            "상한이 안 지켜지면 세션이 사는 내내 자라는 누수가 된다 — got {}B",
            tail.len()
        );
        assert!(
            tail.contains("No conversation found"),
            "앞에서부터 버리므로 **최신** 줄이 증거로 남아야 한다"
        );
    }

    /// If the cut position is in the middle of a character, it moves up to the **next** boundary — discarding the least without exceeding the cap.
    ///
    /// The test above does not decide where the cut lands. Here we pin the number of bytes to cut at 2
    /// and drop it in the middle of `'가'`(0..3): going down (0) exceeds the cap, and cutting at 2 as is makes `drain` panic.
    #[test]
    fn a_cut_inside_a_character_moves_up_to_the_next_boundary() {
        let core = core();
        core.push_diagnostic("가");
        let filler = "a".repeat(DIAGNOSTIC_CAP_BYTES - 3);
        core.push_diagnostic(&filler);
        // "가\n" 4 + filler + "\n" = cap + 2 → cut position 2.
        assert_eq!(core.diagnostic_tail(), format!("\n{filler}\n"));
    }
}

// ── S15 B4: payload-generic replay buffer ─────────────────────────────────────────
//
// ADR-0002: no assumption on output kind — the buffer does not discriminate bytes/events.

#[derive(Debug, Clone)]
pub struct StoredOutput {
    pub seq: u64,
    pub event: OutputEvent,
    /// Approximate size for the eviction byte budget (not the exact wire size — see estimate_cost_bytes below).
    pub cost_bytes: usize,
}

/// A list event enters the ring only through the single-item door that feeds the registry (the `emit` family) — if it enters via `seed` or the chunk door, the registry is no longer the ring prefix's
/// reduced value (that line stands only in the ring without the registry knowing it).
// ADR-0231
fn debug_assert_not_list_event(event: &OutputEvent) {
    debug_assert!(
        !matches!(event, OutputEvent::QueuedInput(_)),
        "목록 사건이 명부를 거치지 않는 문으로 들었다: {event:?}"
    );
}

/// The answer of [`OutputCore::emit_cancel_request`] — the result of seeing the registry check and record in one replay span.
// ADR-0231
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancelRequest {
    /// It was `Queued` — `CancelRequested` was put in the ring. The vendor cancel line is written by the caller.
    Recorded,
    /// It is already waiting for cancel — nothing was put in (idempotent).
    AlreadyCancelling,
    /// It is not in the list (unknown · already resolved) — nothing was put in.
    NotListed,
}

/// The two tables the session classification reads — one name per read. The order is decided in one place, [`pending_then_in_turn`], not by the caller
/// (removing any room to swap the two reads at the call site).
// ADR-0231
trait BusyReads {
    /// Did the pending-list table record this incarnation as 「찼다」 (full). An assembly without the table (harness) is `false`.
    fn read_pending_listed(&self) -> bool;
    /// Did the turn-observation table record this incarnation as in a turn.
    fn read_in_turn(&self) -> bool;
}

impl BusyReads for OutputCore {
    fn read_pending_listed(&self) -> bool {
        self.inputs_pending
            .as_ref()
            .and_then(|table| table.get(self.id, self.epoch))
            .unwrap_or(false)
    }

    fn read_in_turn(&self) -> bool {
        self.turn.table.is_in_turn(self.id, self.epoch)
    }
}

/// Do the session classification's two reads **in this order** — if the pending-list table is true, turn observation is not read. The grounds for the order are
/// [`OutputCore::classified_input_busy`].
// ADR-0231
fn pending_then_in_turn(reads: &impl BusyReads) -> bool {
    reads.read_pending_listed() || reads.read_in_turn()
}

/// An **eviction-budget** size approximation of an OutputEvent.
///
/// ★Why an approximation (TRD core)★: the core cannot serialize (ADR-0003 — wire conversion belongs to the daemon adapter).
/// So it cannot compute the exact wire byte count. But if large `args_json` (tool args) · `json` (Structured)
/// events were counted only as "count 1", they would bypass the max_bytes (2MB) cap and the buffer would grow without bound. To prevent this,
/// we structurally approximate the **sum of byte lengths of the payload string fields** and reflect it in the budget. This value is for eviction
/// decisions only and is not the exact serialized size (tag · delimiter · escape overhead ignored).
/// ★`pub(crate)` because **someone outside the ring must be able to count 「이만큼이면 링을 채운다」 (this much fills the ring)**★:
/// a backend that **requests** the restored history from the peer (codex app-server) must decide by itself how many items to fetch,
/// and the ceiling is [`REPLAY_MAX_BYTES`] · [`REPLAY_MAX_EVENTS`], and the axis that counts weight is this function. If estimated on another
/// axis, that backend would either fetch more of what the ring will discard (pure cost) or fetch less of what can be loaded
/// (screen loss). (ADR-0203)
pub(crate) fn estimate_cost_bytes(event: &OutputEvent) -> usize {
    match event {
        OutputEvent::TerminalBytes(v) => v.len(),
        OutputEvent::TextDelta {
            text,
            turn_id,
            message_id,
        } => text.len() + opt_len(turn_id) + opt_len(message_id),
        // The kind is a fixed-size enum so it has no weight.
        OutputEvent::ToolCall {
            name,
            args_json,
            id,
            turn_id,
            message_id,
            category: _,
        } => name.len() + args_json.len() + opt_len(id) + opt_len(turn_id) + opt_len(message_id),
        // The outcome is a fixed-size enum so it has no weight.
        OutputEvent::ToolResult { id, outcome: _ } => id.len(),
        // Usage is fixed-size numeric fields — only the turn_id string is counted (the two u64s are ignored).
        OutputEvent::Usage { turn_id, .. } => opt_len(turn_id),
        OutputEvent::MessageDone {
            turn_id,
            message_id,
        } => opt_len(turn_id) + opt_len(message_id),
        // Only the failure reason of the outcome has a length decided by the peer — the other three branches have no weight.
        OutputEvent::TurnEnd { turn_id, outcome } => {
            opt_len(turn_id)
                + match outcome {
                    TurnOutcome::Failed { detail } => opt_len(detail),
                    TurnOutcome::Completed | TurnOutcome::Interrupted | TurnOutcome::Unknown => 0,
                }
        }
        OutputEvent::Error(s) => s.len(),
        OutputEvent::Structured { kind, json } => kind.len() + json.len(),
        // The two that carry body text — `Queued`'s text and the bubble copy of the acceptance-impossible finding.
        // ADR-0231
        OutputEvent::QueuedInput(ev) => match ev {
            QueuedInputEvent::Queued { id, text } => id.len() + text.len(),
            QueuedInputEvent::AckUnavailable { delivered } => delivered
                .iter()
                .map(|copy| copy.id.len() + copy.text.len())
                .sum(),
            QueuedInputEvent::CancelRequested { id }
            | QueuedInputEvent::CancelAnswered { id, .. }
            | QueuedInputEvent::CancelFailed { id }
            | QueuedInputEvent::HandedOver { id, .. }
            | QueuedInputEvent::Delivered { id }
            | QueuedInputEvent::Dropped { id, .. } => id.len(),
        },
    }
}

fn opt_len(s: &Option<String>) -> usize {
    s.as_deref().map(str::len).unwrap_or(0)
}

/// Output replay ring buffer for late-attaching windows — a 2MB cap **and** an event-count cap.
/// Kept as a concrete type dedicated to StoredOutput (a generic `Ring<T>` is excessive since this project has no other storage target
/// — the simpler side).
///
/// ★Reason for the event-count cap (S12 consult, GPT-only catch): with only a byte cap, when 1-byte chunks
/// run away the event count balloons into the millions, and when a new subscriber receives the replay it fills the bounded mpsc
/// instantly, creating a permanent loop where every reconnect is cut as a slow-consumer. If either of the two
/// is exceeded, evict from the front.
pub struct Ring {
    items: VecDeque<StoredOutput>,
    total_bytes: usize,
    max_bytes: usize,
    max_events: usize,
}

/// The byte ceiling of the ring. ★It is pulled out as a constant for the same reason as [`estimate_cost_bytes`]★ — a backend that requests history and
/// receives it decides 「여기까지만 받으면 된다」 (this much is enough to receive) with this value (ADR-0203). The only place the ring itself uses it is
/// [`Ring::new`] below.
pub(crate) const REPLAY_MAX_BYTES: usize = 2 * 1024 * 1024;

/// The count ceiling of the ring — the reason is the same as [`REPLAY_MAX_BYTES`].
pub(crate) const REPLAY_MAX_EVENTS: usize = 4096;

impl Ring {
    pub fn new() -> Self {
        Self {
            items: VecDeque::new(),
            total_bytes: 0,
            max_bytes: REPLAY_MAX_BYTES,
            // 4096: set at or below the daemon WS send queue cap (e.g. 4608) − control_slack (512) so that
            // replay alone does not overflow a new subscriber's queue.
            max_events: REPLAY_MAX_EVENTS,
        }
    }

    /// Append a StoredOutput at the back, and evict from the front when the dual cap (sum of cost_bytes OR count) is exceeded.
    /// total_bytes accumulates cost_bytes on each push and is subtracted on each evict, so it always matches the sum of items.
    pub fn push(&mut self, item: StoredOutput) {
        self.total_bytes += item.cost_bytes;
        self.items.push_back(item);
        // ★Keep-the-latest-one invariant (len() > 1 guard)★: if the cost_bytes of the single event just pushed
        // alone exceeds max_bytes (2MB) (e.g. a large args_json/Structured.json), without the len() > 1 guard
        // the eviction loop would pop_front even that latest event and the buffer would be empty → a late-attaching subscriber
        // **misses the latest seq entirely**. So only older ones are evicted and the last one is always kept.
        // Trade-off: if a single event exceeds the budget, memory can exceed it by up to "the size of the largest single event"
        // (a temporary violation of the byte cap), but replay consistency (a late subscriber always sees the latest) > the strict byte cap.
        while self.items.len() > 1
            && (self.total_bytes > self.max_bytes || self.items.len() > self.max_events)
        {
            if let Some(oldest) = self.items.pop_front() {
                self.total_bytes -= oldest.cost_bytes;
            } else {
                break;
            }
        }
    }

    /// From the back, collect only `TerminalBytes` up to `max_bytes` and return them in **chronological** order.
    ///
    /// ★The cap is hard★: a chunk straddling the boundary is cut and only its **end part** is kept, so the returned length does not exceed `max_bytes`
    ///   (if a chunk came through whole, the cap would stop being a cap).
    /// ★No clone★: only slices are collected and joined once at the end — the difference from copying the whole ring (`snapshot`)
    ///   is the reason this method exists.
    // ADR-0172
    pub fn terminal_tail(&self, max_bytes: usize) -> Vec<u8> {
        let mut parts: Vec<&[u8]> = Vec::new();
        let mut total = 0usize;
        for item in self.items.iter().rev() {
            if total >= max_bytes {
                break;
            }
            if let OutputEvent::TerminalBytes(data) = &item.event {
                let take = data.len().min(max_bytes - total);
                parts.push(&data[data.len() - take..]);
                total += take;
            }
        }
        parts.reverse();
        parts.concat()
    }

    /// Clone and return the whole current buffer (seq ascending). The caller filters after_seq with partition_point.
    /// An owned snapshot is needed so the caller (subscribe) can borrow it outside the lock (minimizing lock hold time).
    pub fn snapshot(&self) -> Vec<StoredOutput> {
        self.items.iter().cloned().collect()
    }
}

impl Default for Ring {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolCategory;
    use std::sync::Mutex;

    /// A mock OutputSink that collects received output in order as (seq, bytes, is_event).
    struct MockSink {
        id: SinkId,
        events: Mutex<Vec<(u64, Vec<u8>, bool)>>,
    }

    impl MockSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                id: uuid::Uuid::new_v4(),
                events: Mutex::new(Vec::new()),
            })
        }

        fn seqs(&self) -> Vec<u64> {
            self.events.lock().unwrap().iter().map(|e| e.0).collect()
        }

        fn len(&self) -> usize {
            self.events.lock().unwrap().len()
        }
    }

    impl OutputSink for MockSink {
        fn send(&self, frame: OutputFrame<'_>) -> Result<(), crate::types::SinkError> {
            let (bytes, is_event) = match frame.payload {
                OutputPayload::Bytes(b) => (b.to_vec(), false),
                OutputPayload::Event(e) => (format!("{e:?}").into_bytes(), true),
            };
            self.events
                .lock()
                .unwrap()
                .push((frame.seq, bytes, is_event));
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    struct MockStatusSink {
        statuses: Mutex<Vec<AgentStatus>>,
        turn_ends: Mutex<Vec<(AgentId, u32)>>,
    }

    impl MockStatusSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                statuses: Mutex::new(Vec::new()),
                turn_ends: Mutex::new(Vec::new()),
            })
        }

        fn statuses(&self) -> Vec<AgentStatus> {
            self.statuses.lock().unwrap().clone()
        }

        fn turn_ends(&self) -> Vec<(AgentId, u32)> {
            self.turn_ends.lock().unwrap().clone()
        }
    }

    impl StatusSink for MockStatusSink {
        fn status_changed(&self, _id: AgentId, status: AgentStatus, _epoch: u32) {
            self.statuses.lock().unwrap().push(status);
        }
        fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
        fn turn_ended(&self, id: AgentId, epoch: u32) {
            self.turn_ends.lock().unwrap().push((id, epoch));
        }
    }

    use crate::types::AgentInfo;

    fn new_core(status_sink: Arc<dyn StatusSink>) -> OutputCore {
        OutputCore::new(uuid::Uuid::new_v4(), 0, status_sink, TurnWiring::detached())
    }

    // ── ADR-0292: usage observation reaches the sink outside the containment marker ─────────────────────────

    #[test]
    fn report_usage_reaches_the_sink_outside_the_contain_mark() {
        use crate::usage::{UsageObservation, UsageSource, UsageVendorKey};
        use engram_dashboard_base::panic::{contain, is_contained};

        struct MarkAtUsage(Mutex<Vec<bool>>);
        impl StatusSink for MarkAtUsage {
            fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
            fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
            fn usage_observed(&self, _obs: UsageObservation) {
                self.0.lock().unwrap().push(is_contained());
            }
        }

        let sink = Arc::new(MarkAtUsage(Mutex::new(Vec::new())));
        let core = new_core(sink.clone());
        let observation = || UsageObservation {
            vendor: UsageVendorKey::new("test-vendor"),
            five_hour: None,
            weekly: None,
            model_scoped: None,
            plan: None,
            source: UsageSource::Passive,
            limits_unavailable: None,
        };

        let after = contain(std::panic::AssertUnwindSafe(|| {
            core.report_usage(vec![observation(), observation()]);
            is_contained()
        }))
        .expect("panic 없음");

        assert_eq!(*sink.0.lock().unwrap(), vec![false, false]);
        assert!(after, "관측을 넘긴 뒤 펌프의 표시가 되살아나지 않았다");
    }

    // ── ADR-0113: emit → turn observation table + turn end push ───────────────────────────────────

    /// A core with the table + claude classifier plugged in (same shape as the production spawn_session wiring — taken through the same dispatch).
    fn core_with_turns(
        status_sink: Arc<dyn StatusSink>,
        epoch: u32,
    ) -> (OutputCore, Arc<TurnObservations>, AgentId) {
        use crate::profile::{AgentCommand, AgentOutputFormat};
        let id = uuid::Uuid::new_v4();
        let turns = Arc::new(TurnObservations::new());
        let core = OutputCore::new(
            id,
            epoch,
            status_sink,
            TurnWiring::new(
                turns.clone(),
                crate::backend::turn_classifier(&AgentCommand::Claude {
                    extra_args: vec![],
                    output_format: AgentOutputFormat::StreamJson,
                }),
            ),
        );
        (core, turns, id)
    }

    fn delta() -> OutputEvent {
        OutputEvent::TextDelta {
            text: "x".into(),
            turn_id: None,
            message_id: None,
        }
    }

    fn message_done() -> OutputEvent {
        OutputEvent::MessageDone {
            turn_id: None,
            message_id: None,
        }
    }

    #[test]
    fn emit_records_turn_signals_into_the_shared_table() {
        let (core, turns, id) = core_with_turns(MockStatusSink::new(), 7);
        core.emit(delta());
        assert!(turns.is_in_turn(id, 7), "진행 신호 → 턴 중");
        core.emit(message_done());
        assert!(!turns.is_in_turn(id, 7), "종료 신호 → 턴 아님");
    }

    /// A sink that reads the table at the doorbell moment — an observation window on the order 「표 갱신 → 통지」 (table update → notification).
    struct HaltAtDoorbell {
        turns: Arc<TurnObservations>,
        seen: Mutex<Vec<bool>>,
    }
    impl StatusSink for HaltAtDoorbell {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
        fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
        fn turn_ended(&self, id: AgentId, epoch: u32) {
            let halted = self.turns.get(id, epoch).expect("관측됨").last_end_failed;
            self.seen.lock().unwrap().push(halted);
        }
    }

    /// A classifier that carries an error end — the production classifier does not yet emit an error end, so the test builds it directly.
    fn classify_outcome(event: &OutputEvent) -> Option<TurnSignal> {
        use crate::turn::TurnEndKind;
        use crate::types::TurnOutcome;
        match event {
            OutputEvent::TurnEnd { outcome, .. } => Some(TurnSignal::Ended(match outcome {
                TurnOutcome::Completed => TurnEndKind::Clean,
                TurnOutcome::Failed { .. } => TurnEndKind::Failed,
                _ => TurnEndKind::Other,
            })),
            _ => None,
        }
    }

    // ADR-0231
    #[test]
    fn the_turn_end_doorbell_rings_after_the_halt_is_written() {
        use crate::types::TurnOutcome;
        let id = uuid::Uuid::new_v4();
        let turns = Arc::new(TurnObservations::new());
        turns.register(id, 2);
        let sink = Arc::new(HaltAtDoorbell {
            turns: turns.clone(),
            seen: Mutex::new(Vec::new()),
        });
        let core = OutputCore::new(
            id,
            2,
            sink.clone(),
            TurnWiring::new(turns.clone(), classify_outcome),
        );
        core.emit(OutputEvent::TurnEnd {
            turn_id: None,
            outcome: TurnOutcome::Failed { detail: None },
        });
        core.emit(OutputEvent::TurnEnd {
            turn_id: None,
            outcome: TurnOutcome::Completed,
        });
        assert_eq!(
            *sink.seen.lock().unwrap(),
            vec![true, false],
            "초인종을 받은 쪽이 곧바로 바쁨을 묻는다 — 그때 표가 이미 멈춤·풀림을 보여야 한다"
        );
    }

    type Park = (std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>);
    /// A spot that holds that thread at the moment it classifies an error line — a gap where seq is issued and the table is not yet written (test only).
    static PARK_ERROR: Mutex<Option<Park>> = Mutex::new(None);

    /// Error line = `Failed` (caught) · input echo = progress · turn end = clean end.
    fn classify_parking_error(event: &OutputEvent) -> Option<TurnSignal> {
        use crate::turn::TurnEndKind;
        match event {
            OutputEvent::Error(_) => {
                let parked = PARK_ERROR.lock().unwrap().take();
                if let Some((entered, release)) = parked {
                    entered.send(()).expect("entered");
                    release
                        .recv_timeout(Duration::from_secs(5))
                        .expect("release");
                }
                Some(TurnSignal::Failed)
            }
            OutputEvent::Structured { .. } => Some(TurnSignal::Progress),
            OutputEvent::MessageDone { .. } => Some(TurnSignal::Ended(TurnEndKind::Clean)),
            _ => None,
        }
    }

    /// ★Cursor regression of the stuck field (real emit path)★: the injection thread's progress
    /// (N+1) is written first, before the pump's error line (seq N) is written to the table — filtering by the progress cursor drops that error, so the turn end (N+2) folds cleanly.
    // ADR-0231
    #[test]
    fn an_error_written_behind_a_newer_progress_still_halts_at_the_turn_end() {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        *PARK_ERROR.lock().unwrap() = Some((entered_tx, release_rx));
        let id = uuid::Uuid::new_v4();
        let turns = Arc::new(TurnObservations::new());
        turns.register(id, 4);
        let core = Arc::new(OutputCore::new(
            id,
            4,
            MockStatusSink::new(),
            TurnWiring::new(turns.clone(), classify_parking_error),
        ));

        // The output pump — a turn end line after the error line (caught in classification).
        let pump = {
            let core = core.clone();
            std::thread::spawn(move || {
                core.emit(OutputEvent::Error("boom".into()));
                core.emit(message_done());
            })
        };
        entered_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("오류 줄의 seq 가 발급된 채 붙잡혔다");
        // The injection thread's input echo.
        core.emit(OutputEvent::Structured {
            kind: "user".into(),
            json: "{}".into(),
        });
        release_tx.send(()).expect("release");
        pump.join().expect("pump");

        let order: Vec<&str> = core
            .replay
            .lock()
            .unwrap()
            .snapshot()
            .iter()
            .map(|s| match s.event {
                OutputEvent::Error(_) => "Error",
                OutputEvent::Structured { .. } => "Structured",
                OutputEvent::MessageDone { .. } => "MessageDone",
                _ => "other",
            })
            .collect();
        assert_eq!(
            order,
            ["Error", "Structured", "MessageDone"],
            "링 순서 전제"
        );
        let o = turns.get(id, 4).expect("관측됨");
        assert!(
            o.last_end_failed,
            "링 순서상 오류로 끝난 턴이 깨끗하게 접혔다 — 뒤 seq 의 진행에 밀려 오류가 버려졌다"
        );
        assert!(!o.in_turn, "턴 끝이 가장 새 신호다");
    }

    /// ★A door that writes no progress signal★ — the same event goes to the screen (fanout) · replay as is, but nothing is left in the fact
    /// layer. If this property breaks, an unattributed line turns on the in-turn indicator and its end is blocked by the attribution
    /// gate, so mail is blocked until the 30-minute fail-open valve (the very mechanism ADR-0127 closed).
    #[test]
    fn the_unobserved_door_fans_out_and_replays_but_writes_no_turn_fact() {
        let (core, turns, id) = core_with_turns(MockStatusSink::new(), 3);
        let sink = MockSink::new();
        core.subscribe(sink.clone());

        core.emit_without_turn_observation(delta());
        assert_eq!(turns.get(id, 3), None, "미귀속 줄이 사실 계층을 켰다");
        assert_eq!(sink.len(), 1, "화면으로는 나가야 한다");
        assert_eq!(
            core.replay
                .lock()
                .expect("replay poisoned")
                .snapshot()
                .len(),
            1,
            "replay 에도 남아야 한다"
        );

        // The same core's normal door still writes — the doors were split, the table was not turned off.
        core.emit(delta());
        assert!(turns.is_in_turn(id, 3));
    }

    #[test]
    fn emit_of_non_turn_events_leaves_the_table_untouched() {
        let (core, turns, id) = core_with_turns(MockStatusSink::new(), 0);
        core.emit(OutputEvent::TerminalBytes(b"raw".to_vec()));
        core.emit(OutputEvent::Usage {
            input_tokens: 1,
            output_tokens: 2,
            turn_id: None,
        });
        core.emit(OutputEvent::Error("stream hiccup".into()));
        assert_eq!(
            turns.get(id, 0),
            None,
            "턴 신호가 아니면 항목을 만들지 않는다"
        );
    }

    #[test]
    fn every_message_done_pushes_a_turn_end_notification() {
        let sink = MockStatusSink::new();
        let (core, _turns, id) = core_with_turns(sink.clone(), 2);
        core.emit(delta());
        core.emit(message_done());
        core.emit(message_done());
        assert_eq!(sink.turn_ends(), vec![(id, 2), (id, 2)]);
    }

    #[test]
    fn progress_events_do_not_push_turn_end() {
        let sink = MockStatusSink::new();
        let (core, _turns, _id) = core_with_turns(sink.clone(), 0);
        core.emit(delta());
        core.emit(OutputEvent::Structured {
            kind: "user".into(),
            json: "{}".into(),
        });
        assert!(sink.turn_ends().is_empty());
    }

    #[test]
    fn a_late_echo_after_finish_cannot_resurrect_the_entry() {
        let (core, turns, id) = core_with_turns(MockStatusSink::new(), 0);
        core.emit(delta());
        assert!(turns.is_in_turn(id, 0), "전제: 턴 중으로 관측됨");

        core.finish(TerminalReason::Exited { code: Some(0) });
        assert_eq!(turns.get(id, 0), None, "finish 가 자기 항목을 거둔다");

        // The injection thread that was blocked now emits the echo (same epoch).
        core.emit(OutputEvent::Structured {
            kind: "user".into(),
            json: "{}".into(),
        });
        assert_eq!(
            turns.get(id, 0),
            None,
            "종료된 화신은 항목을 되살리지 못한다"
        );
        assert!(turns.in_turn_snapshot().is_empty());
    }

    #[test]
    fn a_dead_incarnations_late_echo_cannot_delete_the_live_ones_observation() {
        use crate::profile::{AgentCommand, AgentOutputFormat};
        let id = uuid::Uuid::new_v4();
        let turns = Arc::new(TurnObservations::new());
        let classify = crate::backend::turn_classifier(&AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        });
        let wiring = |t: &Arc<TurnObservations>| TurnWiring::new(t.clone(), classify);

        // The earlier incarnation (epoch 0) — runs for a while and then terminates.
        let dead = OutputCore::new(id, 0, MockStatusSink::new(), wiring(&turns));
        for _ in 0..5 {
            dead.emit(delta());
        }
        dead.finish(TerminalReason::Killed);
        assert_eq!(turns.get(id, 0), None, "종료가 자기 항목을 거둔다(전제)");

        // The new incarnation (epoch 1) — same AgentId, a new core, so seq counts from 0 again.
        let live = OutputCore::new(id, 1, MockStatusSink::new(), wiring(&turns));
        live.emit(delta());
        assert!(turns.is_in_turn(id, 1), "산 화신이 턴 중으로 관측됨(전제)");

        // The dead incarnation's injection thread wakes up only now and emits an echo — its seq is **larger** than the live incarnation's.
        dead.emit(OutputEvent::Structured {
            kind: "user".into(),
            json: "{}".into(),
        });
        assert!(
            turns.is_in_turn(id, 1),
            "죽은 화신의 지각 emit 이 산 화신의 관측을 덮거나 지우면 안 된다"
        );
        assert_eq!(
            turns.get(id, 0),
            None,
            "죽은 화신의 항목이 되살아나지도 않는다"
        );
    }

    #[test]
    fn a_backend_without_a_turn_mapping_never_records_a_signal() {
        use crate::profile::AgentCommand;
        let id = uuid::Uuid::new_v4();
        let turns = Arc::new(TurnObservations::new());
        let core = OutputCore::new(
            id,
            0,
            MockStatusSink::new(),
            TurnWiring::new(
                turns.clone(),
                crate::backend::turn_classifier(&AgentCommand::Shell {
                    program: "cmd.exe".into(),
                    args: vec![],
                }),
            ),
        );
        core.emit(delta());
        core.emit(OutputEvent::Structured {
            kind: "session_meta".into(),
            json: "{}".into(),
        });
        core.emit(message_done());
        assert_eq!(turns.get(id, 0), None, "분류자가 침묵하면 관측도 없다");
    }

    #[test]
    fn an_unwired_core_records_nothing_at_all() {
        let sink = MockStatusSink::new();
        let core = new_core(sink.clone());
        core.emit(delta());
        core.emit(message_done());
        assert!(sink.turn_ends().is_empty(), "배선 없으면 통지도 없다");
    }

    #[test]
    fn seed_never_bootstraps_turn_observation_from_a_transcript() {
        let sink = MockStatusSink::new();
        let (core, turns, id) = core_with_turns(sink.clone(), 0);
        core.seed(vec![delta(), message_done(), delta()]);
        assert_eq!(turns.get(id, 0), None, "seed 는 관측이 아니다");
        assert!(sink.turn_ends().is_empty(), "seed 는 통지도 내지 않는다");
        // Observation starts from live emit (it is not that seed blocks observation either).
        core.emit(delta());
        assert!(turns.is_in_turn(id, 0));
    }

    /// The chunk door's basic contract — it enters the ring, goes out **in order** to attached subscribers, and seq continues.
    #[test]
    fn a_batch_lands_in_the_ring_and_fans_out_in_order() {
        let core = new_core(MockStatusSink::new());
        let sink = MockSink::new();
        core.subscribe(sink.clone());

        core.emit_batch_without_turn_observation(vec![
            OutputEvent::TerminalBytes(b"one".to_vec()),
            OutputEvent::TerminalBytes(b"two".to_vec()),
            OutputEvent::TerminalBytes(b"three".to_vec()),
        ]);

        assert_eq!(sink.seqs(), vec![0, 1, 2]);
        // A late-attaching subscriber also receives the same set from the ring.
        let late = MockSink::new();
        core.subscribe(late.clone());
        assert_eq!(late.seqs(), vec![0, 1, 2]);
    }

    /// An empty chunk uses no seq — otherwise every session with nothing to restore would shift the numbering by one.
    #[test]
    fn an_empty_batch_consumes_no_seq() {
        let core = new_core(MockStatusSink::new());
        core.emit_batch_without_turn_observation(Vec::new());
        let sink = MockSink::new();
        core.subscribe(sink.clone());
        core.emit(OutputEvent::TerminalBytes(b"first".to_vec()));
        assert_eq!(sink.seqs(), vec![0]);
    }

    /// Like the single-item door, the chunk door **writes no observation** — if a past record made the new incarnation 「턴 중」 (in a turn),
    /// the end of that turn would never come (ADR-0113/0127 · the same discipline `seed` kept).
    #[test]
    fn a_batch_never_bootstraps_turn_observation() {
        let sink = MockStatusSink::new();
        let (core, turns, id) = core_with_turns(sink.clone(), 0);
        core.emit_batch_without_turn_observation(vec![delta(), message_done(), delta()]);
        assert_eq!(turns.get(id, 0), None, "덩이 문이 관측을 적었다");
        assert!(sink.turn_ends().is_empty(), "덩이 문이 통지를 냈다");
    }

    /// ★★No other emit can slip **into** the chunk★★ — this is the reason this door exists (ADR-0203).
    /// An implementation called singly would let a competing thread take a middle seq and split the restored conversation.
    #[test]
    fn nothing_can_interleave_inside_a_batch() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let core = Arc::new(new_core(MockStatusSink::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let racer = {
            let core = core.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    core.emit(OutputEvent::TerminalBytes(b"live".to_vec()));
                }
            })
        };

        let batch: Vec<OutputEvent> = (0..200)
            .map(|i| OutputEvent::Structured {
                kind: "history".to_string(),
                json: format!("{i}"),
            })
            .collect();
        core.emit_batch_without_turn_observation(batch);
        stop.store(true, Ordering::Relaxed);
        racer.join().unwrap();

        // Pull the seqs of the chunk items from the ring and check they are **contiguous**.
        let stored = core.replay.lock().unwrap().snapshot();
        let seqs: Vec<u64> = stored
            .iter()
            .filter(
                |c| matches!(&c.event, OutputEvent::Structured { kind, .. } if kind == "history"),
            )
            .map(|c| c.seq)
            .collect();
        assert_eq!(seqs.len(), 200, "덩이가 링에서 잘렸다");
        for pair in seqs.windows(2) {
            assert_eq!(
                pair[1],
                pair[0] + 1,
                "덩이 한가운데에 다른 줄이 끼어들었다: {seqs:?}"
            );
        }
    }

    #[test]
    fn emit_increments_seq_and_fans_out() {
        let core = new_core(MockStatusSink::new());
        let sink = MockSink::new();
        core.subscribe(sink.clone());

        core.emit(OutputEvent::TerminalBytes(b"hello".to_vec()));
        core.emit(OutputEvent::TerminalBytes(b"world".to_vec()));

        assert_eq!(sink.seqs(), vec![0, 1]);
        assert_eq!(sink.len(), 2);
        {
            let ev = sink.events.lock().unwrap();
            assert_eq!(ev[0].1, b"hello");
            assert_eq!(ev[1].1, b"world");
            assert!(!ev[0].2, "TerminalBytes 는 OutputPayload::Bytes 로 와야 함");
            assert!(!ev[1].2);
        }
        let snap = core.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].seq, 0);
        assert_eq!(snap[1].seq, 1);
        assert_eq!(snap[0].data, b"hello");
        assert_eq!(snap[1].data, b"world");
    }

    #[test]
    fn emit_structured_event_fans_out_as_event_and_replays() {
        let core = new_core(MockStatusSink::new());

        let live = MockSink::new();
        core.subscribe(live.clone());
        core.emit(OutputEvent::TextDelta {
            text: "hi".into(),
            turn_id: None,
            message_id: None,
        });
        core.emit(OutputEvent::TerminalBytes(b"raw".to_vec()));

        {
            let ev = live.events.lock().unwrap();
            assert_eq!(ev.len(), 2);
            assert!(
                ev[0].2,
                "구조화 이벤트는 OutputPayload::Event 로 fanout 돼야 함"
            );
            assert!(
                String::from_utf8_lossy(&ev[0].1).contains("TextDelta"),
                "Event payload 가 해당 이벤트를 담아야 함"
            );
            assert!(!ev[1].2, "TerminalBytes 는 Bytes 로 fanout");
            assert_eq!(ev[1].1, b"raw");
        }

        let late = MockSink::new();
        core.subscribe(late.clone());
        {
            let ev = late.events.lock().unwrap();
            assert_eq!(ev.len(), 2);
            assert_eq!(ev[0].0, 0);
            assert!(ev[0].2, "replay 된 구조화 이벤트도 Event payload");
            assert_eq!(ev[1].0, 1);
            assert!(!ev[1].2);
            assert_eq!(ev[1].1, b"raw");
        }
    }

    #[test]
    fn seed_pushes_to_ring_in_order_without_fanout_then_live_seq_continues() {
        let core = new_core(MockStatusSink::new());

        // (1) Seed 3 past events with no subscribers (= imitating the .jsonl restore on resume).
        core.seed(vec![
            OutputEvent::Structured {
                kind: "user".into(),
                json: r#"{"type":"text","text":"과거 질문"}"#.into(),
            },
            OutputEvent::TextDelta {
                text: "과거 답변".into(),
                turn_id: None,
                message_id: None,
            },
            OutputEvent::MessageDone {
                turn_id: None,
                message_id: None,
            },
        ]);

        // Check they were loaded into the Ring in order as seq 0,1,2 (snapshot converts only TerminalBytes, so seq is verified through
        // the late subscriber replay below — snapshot() skips structured events).
        let late = MockSink::new();
        core.subscribe(late.clone());
        assert_eq!(
            late.seqs(),
            vec![0, 1, 2],
            "seed 한 과거 3건이 Ring 에 seq 0,1,2 로 순서대로 있어야 하고 replay 로 전달됨"
        );

        core.emit(OutputEvent::TextDelta {
            text: "새 라이브 토큰".into(),
            turn_id: None,
            message_id: None,
        });
        assert_eq!(
            late.seqs(),
            vec![0, 1, 2, 3],
            "seed 뒤 라이브 emit 의 seq 가 seed 마지막+1 로 이어져야 함(gap·중복 없음)"
        );
    }

    #[test]
    fn seed_does_not_fanout_to_existing_subscriber() {
        let core = new_core(MockStatusSink::new());
        // (An abnormal order, but for defensive verification) attach a subscriber first and then seed.
        let sink = MockSink::new();
        core.subscribe(sink.clone());

        core.seed(vec![OutputEvent::TextDelta {
            text: "seed".into(),
            turn_id: None,
            message_id: None,
        }]);

        assert_eq!(sink.len(), 0, "seed 는 기존 구독자로 fanout 하지 않아야 함");

        core.emit(OutputEvent::TextDelta {
            text: "live".into(),
            turn_id: None,
            message_id: None,
        });
        assert_eq!(
            sink.seqs(),
            vec![1],
            "라이브 emit 만 fanout, seq 는 seed 다음"
        );
    }

    #[test]
    fn subscribe_replays_then_lives_without_seq_gap() {
        let core = new_core(MockStatusSink::new());

        // Emit 2 events before subscribing → they accumulate only in replay.
        core.emit(OutputEvent::TerminalBytes(b"a".to_vec()));
        core.emit(OutputEvent::TerminalBytes(b"b".to_vec()));

        let sink = MockSink::new();
        core.subscribe(sink.clone());
        assert_eq!(sink.seqs(), vec![0, 1]);

        core.emit(OutputEvent::TerminalBytes(b"c".to_vec()));
        assert_eq!(sink.seqs(), vec![0, 1, 2]);
    }

    // ── S15 B4 Ring unit tests (isolation) ──────────────────────────────────────────────

    fn stored(seq: u64, event: OutputEvent) -> StoredOutput {
        let cost_bytes = estimate_cost_bytes(&event);
        StoredOutput {
            seq,
            event,
            cost_bytes,
        }
    }

    // ── ADR-0172: the tail that failure classification reads (is the cap hard · is it console bytes only) ────────────────

    #[test]
    fn terminal_tail_returns_the_last_console_bytes_in_order() {
        let mut ring = Ring::new();
        ring.push(stored(0, OutputEvent::TerminalBytes(b"first ".to_vec())));
        ring.push(stored(1, OutputEvent::TerminalBytes(b"second".to_vec())));
        assert_eq!(ring.terminal_tail(64), b"first second".to_vec());
    }

    #[test]
    fn terminal_tail_is_a_hard_bound_even_when_one_chunk_exceeds_it() {
        // ★If a chunk is kept whole, the cap is not a cap★ — a chunk straddling the boundary must be cut at its end part only.
        let mut ring = Ring::new();
        ring.push(stored(
            0,
            OutputEvent::TerminalBytes(b"xxxxxxxxxx".to_vec()),
        ));
        let tail = ring.terminal_tail(4);
        assert_eq!(tail.len(), 4);
        assert_eq!(tail, b"xxxx".to_vec());

        let mut ring = Ring::new();
        ring.push(stored(0, OutputEvent::TerminalBytes(b"OLD".to_vec())));
        ring.push(stored(
            1,
            OutputEvent::TerminalBytes(b"0123456789".to_vec()),
        ));
        assert_eq!(
            ring.terminal_tail(4),
            b"6789".to_vec(),
            "뒤에서부터 채우므로 남는 건 가장 최근 바이트다"
        );
    }

    #[test]
    fn terminal_tail_skips_structured_events_and_can_be_empty() {
        // The ring of a structured (NDJSON) session has no console bytes at all — an empty value is the normal result.
        let mut ring = Ring::new();
        ring.push(stored(0, OutputEvent::Error("boom".into())));
        ring.push(stored(
            1,
            OutputEvent::TextDelta {
                text: "hi".into(),
                turn_id: None,
                message_id: None,
            },
        ));
        assert!(ring.terminal_tail(64).is_empty());

        ring.push(stored(2, OutputEvent::TerminalBytes(b"raw".to_vec())));
        assert_eq!(
            ring.terminal_tail(64),
            b"raw".to_vec(),
            "사이에 낀 구조화 이벤트는 건너뛰고 콘솔 바이트만 잇는다"
        );
    }

    #[test]
    fn ring_push_snapshot_preserves_order_and_seq() {
        let mut ring = Ring::new();
        ring.push(stored(0, OutputEvent::TerminalBytes(b"a".to_vec())));
        ring.push(stored(1, OutputEvent::TerminalBytes(b"bb".to_vec())));
        ring.push(stored(2, OutputEvent::Error("boom".into())));

        let snap = ring.snapshot();
        assert_eq!(
            snap.iter().map(|s| s.seq).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(matches!(&snap[0].event, OutputEvent::TerminalBytes(v) if v == b"a"));
        assert!(matches!(&snap[2].event, OutputEvent::Error(s) if s == "boom"));
    }

    #[test]
    fn ring_evicts_on_byte_budget_independent_of_count() {
        let mut ring = Ring::new();
        // 3 events of ~1MB args_json each → cost sum ~3MB > 2MB → the oldest is evicted.
        let big = "x".repeat(1024 * 1024);
        ring.push(stored(
            0,
            OutputEvent::ToolCall {
                name: "t".into(),
                args_json: big.clone(),
                id: None,
                turn_id: None,
                message_id: None,
                category: ToolCategory::Other,
            },
        ));
        ring.push(stored(
            1,
            OutputEvent::ToolCall {
                name: "t".into(),
                args_json: big.clone(),
                id: None,
                turn_id: None,
                message_id: None,
                category: ToolCategory::Other,
            },
        ));
        ring.push(stored(
            2,
            OutputEvent::ToolCall {
                name: "t".into(),
                args_json: big.clone(),
                id: None,
                turn_id: None,
                message_id: None,
                category: ToolCategory::Other,
            },
        ));
        let snap = ring.snapshot();
        assert!(
            snap.len() < 3,
            "cost_bytes 합이 2MB 초과 시 건수 상한과 무관하게 evict 돼야 함(len={})",
            snap.len()
        );
        assert_eq!(snap.last().unwrap().seq, 2);
    }

    #[test]
    fn ring_evicts_on_event_count_cap() {
        // 5000 small events → far below the byte budget but capped by the count cap (4096).
        let mut ring = Ring::new();
        for seq in 0..5000u64 {
            ring.push(stored(seq, OutputEvent::TerminalBytes(vec![b'x'])));
        }
        let snap = ring.snapshot();
        assert_eq!(snap.len(), 4096);
        // Evict from the oldest → the first remaining seq = 5000-4096 = 904.
        assert_eq!(snap.first().unwrap().seq, 904);
        assert_eq!(snap.last().unwrap().seq, 4999);
    }

    #[test]
    fn ring_preserves_latest_when_single_event_exceeds_byte_budget() {
        // FIX-A: even if the cost_bytes of the single event just pushed exceeds max_bytes (2MB) alone,
        // by the keep-the-latest-one invariant (len() > 1 guard) that event must remain in the replay buffer.
        let mut ring = Ring::new();
        let huge = "y".repeat(3 * 1024 * 1024); // > 2MB (max_bytes)
        ring.push(stored(
            42,
            OutputEvent::Structured {
                kind: "big".into(),
                json: huge,
            },
        ));
        let snap = ring.snapshot();
        assert_eq!(snap.len(), 1, "단일 초과 이벤트라도 최신 1건은 보존돼야 함");
        assert_eq!(snap[0].seq, 42, "보존된 이벤트는 방금 push 한 최신 seq");
    }

    #[test]
    fn ring_evicts_only_old_when_latest_exceeds_budget() {
        // FIX-A: old small events + a large latest event that alone exceeds the budget.
        let mut ring = Ring::new();
        ring.push(stored(0, OutputEvent::TerminalBytes(b"old0".to_vec())));
        ring.push(stored(1, OutputEvent::TerminalBytes(b"old1".to_vec())));
        let huge = "z".repeat(3 * 1024 * 1024); // > 2MB → this alone exceeds the budget
        ring.push(stored(
            2,
            OutputEvent::Structured {
                kind: "big".into(),
                json: huge,
            },
        ));
        let snap = ring.snapshot();
        assert_eq!(snap.len(), 1, "오래된 것만 빠지고 최신 1건 남아야 함");
        assert_eq!(snap[0].seq, 2, "남은 것은 최신 이벤트");
    }

    #[test]
    fn ring_cost_bytes_reflects_terminal_and_structured() {
        // TerminalBytes → v.len().
        assert_eq!(
            estimate_cost_bytes(&OutputEvent::TerminalBytes(vec![0u8; 100])),
            100
        );
        // Structured (ToolCall) → sum of name + args_json + optional strings.
        let cost = estimate_cost_bytes(&OutputEvent::ToolCall {
            name: "read".into(),           // 4
            args_json: "{\"p\":1}".into(), // 7
            id: Some("abc".into()),        // 3
            turn_id: None,
            message_id: None,
            category: ToolCategory::Read,
        });
        assert_eq!(cost, 4 + 7 + 3);
        // TextDelta → text + optional.
        let cost2 = estimate_cost_bytes(&OutputEvent::TextDelta {
            text: "hello".into(),       // 5
            turn_id: Some("t1".into()), // 2
            message_id: None,
        });
        assert_eq!(cost2, 5 + 2);
        // ToolResult → id only (the outcome is fixed-size).
        let cost3 = estimate_cost_bytes(&OutputEvent::ToolResult {
            id: "exec-1".into(), // 6
            outcome: crate::types::ToolOutcome::Failed,
        });
        assert_eq!(cost3, 6);
    }

    /// Registry events count the body (text · bubble copy) as ring weight — so a long text does not bypass the ring cap as 「건수 1」 (count 1).
    #[test]
    fn ring_cost_bytes_counts_queued_text_and_ack_copies() {
        use crate::types::{DeliveredCopy, DropCause};
        let queued = |ev| estimate_cost_bytes(&OutputEvent::QueuedInput(ev));
        assert_eq!(
            queued(QueuedInputEvent::Queued {
                id: "u1".into(),      // 2
                text: "hello".into(), // 5
            }),
            2 + 5
        );
        assert_eq!(
            queued(QueuedInputEvent::AckUnavailable {
                delivered: vec![
                    DeliveredCopy {
                        id: "a".into(),     // 1
                        text: "xyz".into(), // 3
                    },
                    DeliveredCopy {
                        id: "bb".into(),     // 2
                        text: "wxyz".into(), // 4
                    },
                ],
            }),
            1 + 3 + 2 + 4
        );
        assert_eq!(
            queued(QueuedInputEvent::Dropped {
                id: "u12".into(),
                cause: DropCause::Rejected,
            }),
            3
        );
    }

    #[test]
    fn finish_finalizes_exactly_once() {
        let status_sink = MockStatusSink::new();
        let core = new_core(status_sink.clone());

        core.finish(TerminalReason::Killed);
        core.finish(TerminalReason::Killed);

        let statuses = status_sink.statuses();
        assert_eq!(statuses.len(), 1);
        assert!(matches!(statuses[0], AgentStatus::Killed));
        assert!(matches!(core.status(), AgentStatus::Killed));
    }

    /// ADR-0019 reaper hook (on_terminal) exactly-once guarantee: assert that on the finalize winner path (passing finalized.swap)
    /// the hook is called exactly once, and 0 times on a duplicate finish (swap loser).
    #[test]
    fn on_terminal_hook_fires_exactly_once() {
        let core = new_core(MockStatusSink::new());
        let calls = Arc::new(AtomicU64::new(0));

        let c = calls.clone();
        core.set_on_terminal(Box::new(move |_reason: TerminalReason| {
            c.fetch_add(1, Ordering::SeqCst);
        }));

        core.finish(TerminalReason::Exited { code: Some(0) });
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "finalize 승자 경로에서 on_terminal hook 이 정확히 1회 호출돼야 함"
        );

        core.finish(TerminalReason::Killed);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "중복 finish(finalize 패자)에서 on_terminal hook 이 다시 호출됨(1회 위반)"
        );
    }

    #[test]
    fn subscribe_from_resume_sends_only_tail() {
        let core = new_core(MockStatusSink::new());
        for i in 0..5u8 {
            core.emit(OutputEvent::TerminalBytes(vec![b'a' + i]));
        }
        let sink = MockSink::new();
        let out = core.subscribe_from(sink.clone(), Some(2), true, |_| {});

        assert_eq!(sink.seqs(), vec![3, 4]);
        assert_eq!(out.kind, ReplayKind::Resumed);
        assert_eq!(out.replayed, 2);
        assert_eq!(out.replay_from, 3);
    }

    #[test]
    fn subscribe_from_truncated_when_after_below_oldest() {
        let core = new_core(MockStatusSink::new());
        // Emit 5000 1-byte chunks → the event cap (4096) is exceeded so evict up to oldest=904.
        for _ in 0..5000u64 {
            core.emit(OutputEvent::TerminalBytes(vec![b'x']));
        }
        let sink = MockSink::new();
        let out = core.subscribe_from(sink.clone(), Some(10), true, |_| {});

        assert_eq!(out.kind, ReplayKind::Truncated);
        assert_eq!(out.oldest_seq, 904);
        assert_eq!(sink.seqs().first().copied(), Some(904));
        assert_eq!(out.replay_from, 904);
    }

    #[test]
    fn subscribe_from_epoch_mismatch_is_from_oldest() {
        let core = new_core(MockStatusSink::new());
        for i in 0..3u8 {
            core.emit(OutputEvent::TerminalBytes(vec![b'a' + i]));
        }
        let sink = MockSink::new();
        let out = core.subscribe_from(sink.clone(), Some(1), false, |_| {});

        assert_eq!(out.kind, ReplayKind::FromOldest);
        assert_eq!(sink.seqs(), vec![0, 1, 2]);
        assert_eq!(out.replay_from, 0);
    }

    #[test]
    fn subscribe_from_caught_up_sends_nothing() {
        let core = new_core(MockStatusSink::new());
        for i in 0..3u8 {
            core.emit(OutputEvent::TerminalBytes(vec![b'a' + i]));
        }
        let sink = MockSink::new();
        // after_seq=2 (=latest) → no tail to send.
        let out = core.subscribe_from(sink.clone(), Some(2), true, |_| {});
        assert_eq!(out.kind, ReplayKind::Resumed);
        assert_eq!(out.replayed, 0);
        assert_eq!(out.replay_from, 3);
        assert_eq!(sink.len(), 0);

        core.emit(OutputEvent::TerminalBytes(b"d".to_vec()));
        assert_eq!(sink.seqs(), vec![3]);
    }

    // ★The replay head of an empty ring = the next issue seq (ADR-0231)★ — 0 for a new incarnation. With the old value (`latest+1` = 1) the view
    //   would start the flush of an empty replay from 1 and skip a seq 0 that arrives late behind the marker. All three branches (full · incarnation marker
    //   mismatch · resume of the same incarnation) carry the same value.
    #[test]
    fn subscribe_from_empty_ring_replay_from_is_the_next_seq() {
        for (after_seq, epoch_matches) in [(None, true), (Some(5), false), (Some(5), true)] {
            let core = new_core(MockStatusSink::new());
            let sink = MockSink::new();
            let out = core.subscribe_from(sink.clone(), after_seq, epoch_matches, |_| {});
            assert_eq!(out.replayed, 0);
            assert_eq!(
                out.replay_from, 0,
                "빈 ring 은 첫 발급 seq({after_seq:?}, {epoch_matches})"
            );

            core.emit(OutputEvent::TerminalBytes(b"a".to_vec()));
            assert_eq!(
                sink.seqs(),
                vec![out.replay_from],
                "머리 = 실제로 처음 오는 seq"
            );
        }
    }

    #[test]
    fn subscribe_from_none_after_seq_is_from_oldest() {
        let core = new_core(MockStatusSink::new());
        for i in 0..3u8 {
            core.emit(OutputEvent::TerminalBytes(vec![b'a' + i]));
        }
        let sink = MockSink::new();
        let out = core.subscribe_from(sink.clone(), None, true, |_| {});

        assert_eq!(out.kind, ReplayKind::FromOldest);
        assert_eq!(sink.seqs(), vec![0, 1, 2]);
        assert_eq!(out.replay_from, 0);
    }

    /// M-A fix verification: confirm the on_ready callback is (1) called **before** the replay send, (2) called exactly once,
    /// and (3) the outcome the callback receives is the same as the returned outcome (single-snapshot basis).
    #[test]
    fn subscribe_from_calls_on_ready_before_replay() {
        struct OrderSink {
            id: SinkId,
            replay_started: Arc<AtomicBool>,
        }
        impl OutputSink for OrderSink {
            fn send(&self, _frame: OutputFrame<'_>) -> Result<(), crate::types::SinkError> {
                self.replay_started.store(true, Ordering::SeqCst);
                Ok(())
            }
            fn sink_id(&self) -> SinkId {
                self.id
            }
        }

        let core = new_core(MockStatusSink::new());
        for i in 0..3u8 {
            core.emit(OutputEvent::TerminalBytes(vec![b'a' + i]));
        }

        let replay_started = Arc::new(AtomicBool::new(false));
        let sink = Arc::new(OrderSink {
            id: uuid::Uuid::new_v4(),
            replay_started: replay_started.clone(),
        });

        let call_count = Arc::new(AtomicU64::new(0));
        // Capture the outcome the callback saw and compare it with the returned outcome (SubscribeOutcome is Copy).
        let seen: Arc<Mutex<Option<SubscribeOutcome>>> = Arc::new(Mutex::new(None));

        let cc = call_count.clone();
        let started = replay_started.clone();
        let seen_cb = seen.clone();
        let out = core.subscribe_from(sink, Some(1), true, move |outcome| {
            assert!(
                !started.load(Ordering::SeqCst),
                "on_ready 는 replay 전송 전에 호출돼야 함"
            );
            cc.fetch_add(1, Ordering::SeqCst);
            *seen_cb.lock().unwrap() = Some(*outcome);
        });

        assert_eq!(call_count.load(Ordering::SeqCst), 1);
        // Whether the replay was really sent (after_seq=1 → seq 2 sent) → started true.
        assert!(replay_started.load(Ordering::SeqCst));
        let seen = seen.lock().unwrap().expect("콜백이 호출됨");
        assert_eq!(seen.kind, out.kind);
        assert_eq!(seen.oldest_seq, out.oldest_seq);
        assert_eq!(seen.latest_seq, out.latest_seq);
        assert_eq!(seen.replay_from, out.replay_from);
        assert_eq!(out.kind, ReplayKind::Resumed);
    }

    #[test]
    fn enter_exiting_true_when_running_false_after_terminal() {
        let status_sink = MockStatusSink::new();
        let core = new_core(status_sink.clone());

        assert!(core.enter_exiting());
        assert!(matches!(core.status(), AgentStatus::Exiting));
        assert!(matches!(
            status_sink.statuses().last().unwrap(),
            AgentStatus::Exiting
        ));

        core.finish(TerminalReason::Exited { code: Some(0) });
        assert!(!core.enter_exiting());
    }

    /// ADR-0079 regression guard: whether the replay ring keeps seq ascending (monotonic) even under concurrent emit.
    ///
    /// ★Why this test holds (hermetic)★: the real situation where the pump thread and the write_input synthetic echo call emit
    ///   concurrently is reproduced with emit loops on several threads. It is probabilistic, but with large threads×iterations it
    ///   almost surely steps into the issue/push inversion window and catches the regression (the iteration count is generous so it is not flaky).
    #[test]
    fn concurrent_emit_keeps_replay_ring_monotonic() {
        let core = Arc::new(new_core(MockStatusSink::new()));
        const THREADS: u64 = 4;
        const PER_THREAD: u64 = 500; // 4*500 = 2000 < max_events(4096) → no eviction.

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let core = core.clone();
                std::thread::spawn(move || {
                    for _ in 0..PER_THREAD {
                        // Small payload — does not touch even the 2MB max_bytes. emit is the only seq consumer.
                        core.emit(OutputEvent::TerminalBytes(vec![b'x']));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("emit thread panicked");
        }

        let stored = core.replay.lock().expect("replay poisoned").snapshot();
        let seqs: Vec<u64> = stored.iter().map(|s| s.seq).collect();
        assert_eq!(
            seqs.len() as u64,
            THREADS * PER_THREAD,
            "eviction 없이 전량 저장돼야(검사 완전성)"
        );
        assert!(
            seqs.windows(2).all(|w| w[0] < w[1]),
            "replay ring 은 seq 로 엄격 오름차순이어야 한다(동시 emit 원자성): {seqs:?}"
        );
    }
}

// ── ADR-0231 pending-input registry — core observation (registry reduction · terminal synthesis · seal · rewriting · pending-list table order) ──────────
#[cfg(test)]
mod queued_tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::mpsc;

    use crate::queued_input::{CancelAnswer, QueuedRow, Registry, RowPhase};
    use crate::turn::TurnEndKind;
    use crate::types::{AgentInfo, DeliveredCopy, SinkError};

    const EPOCH: u32 = 5;
    /// The cap for test threads waiting on each other — past it, the test fails instead of hanging.
    const WAIT: Duration = Duration::from_secs(5);

    fn queued(id: &str, text: &str) -> OutputEvent {
        OutputEvent::QueuedInput(QueuedInputEvent::Queued {
            id: id.into(),
            text: text.into(),
        })
    }
    fn delivered(id: &str) -> OutputEvent {
        OutputEvent::QueuedInput(QueuedInputEvent::Delivered { id: id.into() })
    }
    fn dropped(id: &str, cause: DropCause) -> OutputEvent {
        OutputEvent::QueuedInput(QueuedInputEvent::Dropped {
            id: id.into(),
            cause,
        })
    }
    fn cancel_requested(id: &str) -> OutputEvent {
        OutputEvent::QueuedInput(QueuedInputEvent::CancelRequested { id: id.into() })
    }
    fn cancel_answered(id: &str, removed: bool) -> OutputEvent {
        OutputEvent::QueuedInput(QueuedInputEvent::CancelAnswered {
            id: id.into(),
            removed,
        })
    }
    fn ack_unavailable() -> OutputEvent {
        OutputEvent::QueuedInput(QueuedInputEvent::AckUnavailable {
            delivered: Vec::new(),
        })
    }
    fn copy(id: &str, text: &str) -> DeliveredCopy {
        DeliveredCopy {
            id: id.into(),
            text: text.into(),
        }
    }
    fn ended(id: &str) -> QueuedInputEvent {
        QueuedInputEvent::Dropped {
            id: id.into(),
            cause: DropCause::AgentEnded,
        }
    }
    fn delta() -> OutputEvent {
        OutputEvent::TextDelta {
            text: "x".into(),
            turn_id: None,
            message_id: None,
        }
    }

    struct Quiet;
    impl StatusSink for Quiet {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
        fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
    }

    /// Write the two tables at the moment two doorbells ring — an observation window on the orders 「표 갱신 → 통지」 (table update → notification) · 「턴 관측 → 비었다」 (turn observation → became empty).
    struct DoorbellProbe {
        id: AgentId,
        pending: Arc<InputsPendingTable>,
        turns: Arc<TurnObservations>,
        /// The pending-list table at the turn-end doorbell moment.
        at_turn_end: Mutex<Vec<Option<bool>>>,
        /// (pending-list table, in turn) at the became-empty doorbell moment.
        at_drain: Mutex<Vec<(Option<bool>, bool)>>,
    }
    impl DoorbellProbe {
        fn drains(&self) -> Vec<(Option<bool>, bool)> {
            self.at_drain.lock().unwrap().clone()
        }
    }
    impl StatusSink for DoorbellProbe {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
        fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
        fn turn_ended(&self, id: AgentId, epoch: u32) {
            assert_eq!((id, epoch), (self.id, EPOCH));
            let pending = self.pending.get(id, epoch);
            self.at_turn_end.lock().unwrap().push(pending);
        }
        fn inputs_drained(&self, id: AgentId, epoch: u32) {
            assert_eq!((id, epoch), (self.id, EPOCH));
            let pending = self.pending.get(id, epoch);
            let in_turn = self.turns.is_in_turn(id, epoch);
            self.at_drain.lock().unwrap().push((pending, in_turn));
        }
    }

    /// A sink that records the received frames' seq and list events.
    struct Recorder {
        id: SinkId,
        got: Mutex<Vec<(u64, Option<QueuedInputEvent>)>>,
    }
    impl Recorder {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                id: uuid::Uuid::new_v4(),
                got: Mutex::new(Vec::new()),
            })
        }
        fn seqs(&self) -> Vec<u64> {
            self.got.lock().unwrap().iter().map(|(s, _)| *s).collect()
        }
    }
    impl OutputSink for Recorder {
        fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
            let op = match frame.payload {
                OutputPayload::Event(OutputEvent::QueuedInput(op)) => Some(op.clone()),
                _ => None,
            };
            self.got.lock().unwrap().push((frame.seq, op));
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    fn claude_classifier() -> TurnClassifier {
        use crate::profile::{AgentCommand, AgentOutputFormat};
        crate::backend::turn_classifier(&AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        })
    }

    /// A classifier where the receipt that empties the list is also a turn end — built to tell whether the turn-end doorbell comes before 「비었다」 (became empty).
    fn delivered_ends_the_turn(event: &OutputEvent) -> Option<TurnSignal> {
        matches!(
            event,
            OutputEvent::QueuedInput(QueuedInputEvent::Delivered { .. })
        )
        .then_some(TurnSignal::Ended(TurnEndKind::Clean))
    }

    /// The two tables of one incarnation — the core is built in the same shape as the production spawn (two table registrations + a new registry).
    struct Fixture {
        id: AgentId,
        turns: Arc<TurnObservations>,
        pending: Arc<InputsPendingTable>,
    }
    impl Fixture {
        fn new() -> Self {
            Self {
                id: uuid::Uuid::new_v4(),
                turns: Arc::new(TurnObservations::new()),
                pending: Arc::new(InputsPendingTable::new()),
            }
        }
        fn core(
            &self,
            status_sink: Arc<dyn StatusSink>,
            classify: TurnClassifier,
        ) -> Arc<OutputCore> {
            self.turns.register(self.id, EPOCH);
            self.pending.register(self.id, EPOCH);
            Arc::new(
                OutputCore::new(
                    self.id,
                    EPOCH,
                    status_sink,
                    TurnWiring::new(self.turns.clone(), classify),
                )
                .with_queued(QueuedWiring {
                    registry: Arc::new(QueuedInputs::new()),
                    pending: self.pending.clone(),
                }),
            )
        }
        fn probe(&self) -> Arc<DoorbellProbe> {
            Arc::new(DoorbellProbe {
                id: self.id,
                pending: self.pending.clone(),
                turns: self.turns.clone(),
                at_turn_end: Mutex::new(Vec::new()),
                at_drain: Mutex::new(Vec::new()),
            })
        }
        fn pending(&self) -> Option<bool> {
            self.pending.get(self.id, EPOCH)
        }
    }

    fn ring(core: &OutputCore) -> Vec<StoredOutput> {
        core.replay.lock().unwrap().snapshot()
    }

    /// Only the ring's list events (in seq order).
    fn ring_ops(core: &OutputCore) -> Vec<(u64, QueuedInputEvent)> {
        ring(core)
            .into_iter()
            .filter_map(|s| match s.event {
                OutputEvent::QueuedInput(op) => Some((s.seq, op)),
                _ => None,
            })
            .collect()
    }

    fn rows(core: &OutputCore) -> Vec<QueuedRow> {
        core.queued_inputs().snapshot().0
    }

    // ── Registry = the reduced value of the ring prefix ────────────────────────────────────────────────────────────────

    /// While two threads emit list events interleaved, a third takes a registry snapshot — the taken (row, S) must equal the result of feeding the ring prefix `seq <= S` into
    /// a fresh reducer. If the registry is fed outside the replay lock, the two threads' reduction order diverges from the ring order
    /// and this comparison breaks.
    #[test]
    fn the_registry_snapshot_is_the_reduction_of_the_ring_prefix_it_names() {
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        let stop = Arc::new(AtomicBool::new(false));

        let reader = {
            let (core, stop) = (core.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut samples = Vec::new();
                while !stop.load(Ordering::Acquire) {
                    samples.push(core.queued_inputs().snapshot());
                    std::thread::yield_now();
                }
                samples
            })
        };
        let emitters: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|tag| {
                let core = core.clone();
                std::thread::spawn(move || {
                    for i in 0..150 {
                        let id = format!("{tag}{i}");
                        core.emit(queued(&id, "t"));
                        core.emit(delta());
                        match i % 4 {
                            0 => core.emit(delivered(&id)),
                            1 => core.emit(cancel_requested(&id)),
                            2 => core.emit(dropped(&format!("{tag}{}", i - 1), DropCause::Unknown)),
                            _ => {}
                        }
                    }
                })
            })
            .collect();
        for e in emitters {
            e.join().expect("emitter");
        }
        stop.store(true, Ordering::Release);
        let mut samples = reader.join().expect("reader");
        samples.push(core.queued_inputs().snapshot());

        let ops = ring_ops(&core);
        assert!(
            ring(&core).len() < REPLAY_MAX_EVENTS,
            "축출 없이 전량 남아야 대조가 완전하다"
        );
        samples.sort_by_key(|(_, s)| *s);
        let mut fresh = Registry::new();
        let mut next = ops.iter().peekable();
        for (rows, s) in samples {
            let Some(s) = s else {
                assert!(rows.is_empty(), "목록 사건 없이 행이 있다");
                continue;
            };
            while let Some((seq, op)) = next.next_if(|(seq, _)| *seq <= s) {
                fresh.reduce(*seq, op);
            }
            assert_eq!(fresh.as_of_seq(), Some(s), "S 가 목록 사건의 seq 가 아니다");
            assert_eq!(
                fresh.rows(),
                &rows[..],
                "S={s} 의 행이 링 접두의 환원값과 다르다"
            );
        }
        assert!(
            next.next().is_none(),
            "마지막 스냅숏이 링 끝까지 환원하지 않았다"
        );
    }

    #[test]
    fn without_list_events_the_snapshot_has_no_seq() {
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        assert_eq!(core.queued_inputs().snapshot(), (vec![], None));
        core.emit(delta());
        assert_eq!(core.queued_inputs().snapshot(), (vec![], None));
        core.emit(queued("a", "A"));
        assert_eq!(core.queued_inputs().snapshot().1, Some(1));
    }

    // ── Terminal synthesis + seal ─────────────────────────────────────────────────────────────────────

    /// The synthesized `Dropped{AgentEnded}` comes before the terminal transition in both the ring and the subscribers · the pending-list table is also reaped.
    #[test]
    fn finish_drops_every_open_row_before_the_terminal_transition() {
        struct Log {
            id: SinkId,
            lines: Mutex<Vec<String>>,
        }
        impl OutputSink for Log {
            fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
                if let OutputPayload::Event(OutputEvent::QueuedInput(op)) = frame.payload {
                    self.lines.lock().unwrap().push(format!("{op:?}"));
                }
                Ok(())
            }
            fn sink_id(&self) -> SinkId {
                self.id
            }
        }
        impl StatusSink for Log {
            fn status_changed(&self, _id: AgentId, status: AgentStatus, _epoch: u32) {
                self.lines
                    .lock()
                    .unwrap()
                    .push(format!("status {status:?}"));
            }
            fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
        }
        let log = Arc::new(Log {
            id: uuid::Uuid::new_v4(),
            lines: Mutex::new(Vec::new()),
        });
        let fx = Fixture::new();
        let core = fx.core(log.clone(), claude_classifier());
        core.emit(queued("a", "A"));
        core.emit(queued("b", "B"));
        core.emit(cancel_requested("b"));
        core.subscribe(log.clone());
        log.lines.lock().unwrap().clear();
        assert_eq!(fx.pending(), Some(true));

        core.finish(TerminalReason::Exited { code: Some(0) });

        assert_eq!(
            *log.lines.lock().unwrap(),
            vec![
                format!("{:?}", ended("a")),
                format!("{:?}", ended("b")),
                format!("status {:?}", AgentStatus::Exited { code: Some(0) }),
            ],
            "남은 항목이 종점 전이보다 먼저 닫혀야 한다(목록 순)"
        );
        let tail: Vec<_> = ring_ops(&core)
            .into_iter()
            .skip(3)
            .map(|(_, op)| op)
            .collect();
        assert_eq!(tail, vec![ended("a"), ended("b")]);
        let registry = core.queued_inputs().lock();
        assert!(registry.is_empty());
        assert_eq!(
            registry.tombstone("a"),
            Some(false),
            "에이전트 종료는 되살림 불가"
        );
        drop(registry);
        assert_eq!(
            fx.pending(),
            None,
            "대기 목록 표가 이 화신의 항목을 거두지 않았다"
        );
    }

    /// Seal: a `Queued` after terminal synthesis stands in the ring as `Dropped{AgentEnded}` (→ tombstone) — it cannot make a permanent item.
    #[test]
    fn a_queued_after_finish_lands_in_the_ring_as_agent_ended() {
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        core.finish(TerminalReason::Killed);
        let sink = Recorder::new();
        core.subscribe(sink.clone());

        core.emit(queued("late", "L"));

        assert_eq!(ring_ops(&core), vec![(0, ended("late"))]);
        assert_eq!(*sink.got.lock().unwrap(), vec![(0, Some(ended("late")))]);
        assert!(rows(&core).is_empty());
        assert_eq!(core.queued_inputs().lock().tombstone("late"), Some(false));
    }

    /// `finish` while another thread emits `Queued` nonstop — there is no `Queued` between the synthesis chunk and the seal, the synthesis
    /// closes every item open at that moment in one chunk, and no non-terminal item remains in the registry.
    #[test]
    fn finish_synthesis_and_seal_are_one_lock_section() {
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        let (ready_tx, ready_rx) = mpsc::channel();
        let emitter = {
            let core = core.clone();
            std::thread::spawn(move || {
                // Cap = a size that stays whole without ring eviction (`Queued` + synthesized `Dropped` < `REPLAY_MAX_EVENTS`). If scheduling
                //   keeps it from seeing termination up to the cap, this run cannot measure the overlap — the seal alone is measured by the test above.
                let mut after_final = 0;
                for i in 0..1800 {
                    core.emit(queued(&format!("q{i}"), "t"));
                    if i == 100 {
                        ready_tx.send(()).expect("ready");
                    }
                    if core.finalized.load(Ordering::Acquire) {
                        after_final += 1;
                        if after_final == 100 {
                            break;
                        }
                    }
                }
            })
        };
        ready_rx.recv_timeout(WAIT).expect("emitter ready");
        core.finish(TerminalReason::Exited { code: Some(0) });
        emitter.join().expect("emitter");

        let ops: Vec<QueuedInputEvent> = ring_ops(&core).into_iter().map(|(_, op)| op).collect();
        let first_drop = ops
            .iter()
            .position(|op| matches!(op, QueuedInputEvent::Dropped { .. }))
            .expect("합성 Dropped 가 없다");
        let listed: Vec<&String> = ops[..first_drop]
            .iter()
            .map(|op| match op {
                QueuedInputEvent::Queued { id, .. } => id,
                other => panic!("합성 앞에 Queued 아닌 사건: {other:?}"),
            })
            .collect();
        let dropped_after: Vec<&String> = ops[first_drop..]
            .iter()
            .map(|op| match op {
                QueuedInputEvent::Dropped {
                    id,
                    cause: DropCause::AgentEnded,
                } => id,
                other => panic!("합성 뒤에 AgentEnded 아닌 사건 — 봉인을 빠져나갔다: {other:?}"),
            })
            .collect();
        assert_eq!(
            &dropped_after[..listed.len()],
            &listed[..],
            "합성 덩이가 그때 열린 항목 전부를 목록 순으로 잇달아 닫지 않았다"
        );
        assert!(rows(&core).is_empty(), "명부에 비종결 항목이 남았다");
    }

    // ── Rewriting after the decision · copy ─────────────────────────────────────────────────────────────

    /// A `Queued` that stood before the finding is written as is and the finding moves it · a `Queued` after the finding is a `Queued` + an `AckUnavailable`
    /// carrying one copy of that item (two seqs) — the registry result is a receipt and the list was never full.
    #[test]
    fn a_queued_after_the_ack_verdict_is_closed_by_its_own_copy_in_the_same_lock() {
        let fx = Fixture::new();
        let probe = fx.probe();
        let core = fx.core(probe.clone(), claude_classifier());
        core.emit(queued("a", "A"));
        core.emit(ack_unavailable());
        assert_eq!(fx.pending(), Some(false));

        core.emit(queued("b", "B"));

        let queued_op = |id: &str, text: &str| QueuedInputEvent::Queued {
            id: id.into(),
            text: text.into(),
        };
        let ack_op = |copies| QueuedInputEvent::AckUnavailable { delivered: copies };
        assert_eq!(
            ring_ops(&core),
            vec![
                (0, queued_op("a", "A")),
                (1, ack_op(vec![copy("a", "A")])),
                (2, queued_op("b", "B")),
                (3, ack_op(vec![copy("b", "B")])),
            ]
        );
        let registry = core.queued_inputs().lock();
        assert!(registry.is_empty());
        assert_eq!(registry.tombstone("b"), Some(false), "받음으로 닫혔다");
        assert_eq!(registry.as_of_seq(), Some(3));
        drop(registry);
        assert_eq!(fx.pending(), Some(false));
        assert_eq!(
            probe.drains().len(),
            1,
            "빔 → 빔인 바꿔 적기는 초인종을 울리지 않는다"
        );
    }

    #[test]
    fn the_seal_comes_before_the_ack_rewrite() {
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        core.emit(ack_unavailable());
        core.finish(TerminalReason::Killed);

        core.emit(queued("c", "C"));

        let ops = ring_ops(&core);
        assert_eq!(ops.len(), 2);
        assert_eq!(
            ops[1],
            (1, ended("c")),
            "봉인 뒤의 Queued 는 받음이 아니라 버림이다"
        );
    }

    /// Into a finding the decoder emitted empty, the core fills in the copies of the items that reduction closes with a receipt (`Queued` · cancel-pending could not be removed · no
    /// response) in list order — none is closed by cancel · what the decoder filled in is overwritten · the ring weight counts
    /// the copy body.
    #[test]
    fn the_core_fills_the_ack_copies_from_the_rows_it_closes_as_delivered() {
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        core.emit(queued("a", "AAAA"));
        core.emit(queued("b", "BBBB"));
        core.emit(cancel_requested("b"));
        core.emit(queued("c", "CCCC"));
        core.emit(cancel_requested("c"));
        core.emit(cancel_answered("c", false));
        core.emit(queued("d", "DDDD"));
        core.emit(cancel_requested("d"));
        core.emit(cancel_answered("d", true));
        assert_eq!(
            rows(&core)
                .iter()
                .map(|r| (r.id.as_str(), r.phase))
                .collect::<Vec<_>>(),
            vec![
                ("a", RowPhase::Queued),
                (
                    "b",
                    RowPhase::Cancelling {
                        answer: CancelAnswer::Unanswered,
                        vendor_closed: false
                    }
                ),
                (
                    "c",
                    RowPhase::Cancelling {
                        answer: CancelAnswer::NotRemoved,
                        vendor_closed: false
                    }
                ),
            ]
        );

        core.emit(OutputEvent::QueuedInput(QueuedInputEvent::AckUnavailable {
            delivered: vec![copy("zz", "디코더가 채운 것")],
        }));

        let stored = ring(&core).pop().expect("판명");
        let filled = QueuedInputEvent::AckUnavailable {
            delivered: vec![copy("a", "AAAA"), copy("b", "BBBB"), copy("c", "CCCC")],
        };
        assert!(
            matches!(&stored.event, OutputEvent::QueuedInput(op) if *op == filled),
            "사본이 닫힌 항목의 목록 순 본문이 아니다: {:?}",
            stored.event
        );
        assert_eq!(
            stored.cost_bytes,
            estimate_cost_bytes(&OutputEvent::QueuedInput(filled))
        );
        assert!(stored.cost_bytes > estimate_cost_bytes(&ack_unavailable()));
        let registry = core.queued_inputs().lock();
        assert!(registry.is_empty());
        assert_eq!(
            registry.tombstone("zz"),
            None,
            "덮인 디코더 사본은 환원되지 않는다"
        );
        assert!(registry.ack_unavailable_seen());
    }

    // ── Turn observation guard ─────────────────────────────────────────────────────────────────────────

    thread_local! {
        static CLASSIFIED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    fn record_classified(event: &OutputEvent) -> Option<TurnSignal> {
        let label = match event {
            OutputEvent::QueuedInput(QueuedInputEvent::Queued { id, .. }) => format!("Queued {id}"),
            OutputEvent::QueuedInput(QueuedInputEvent::AckUnavailable { .. }) => {
                "AckUnavailable".into()
            }
            other => format!("{other:?}"),
        };
        CLASSIFIED.with(|c| c.borrow_mut().push(label));
        None
    }

    /// Events the core rewrote or built (the seal's `Dropped` · the `AckUnavailable` chained after a decision · terminal synthesis) do not pass through
    /// the classifier — they are not facts of vendor output. The original event (including the decoder's finding) passes.
    #[test]
    fn events_the_core_writes_itself_never_reach_the_turn_classifier() {
        let classified = || CLASSIFIED.with(|c| std::mem::take(&mut *c.borrow_mut()));
        classified();

        // Terminal synthesis (open items x) · seal (y).
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), record_classified);
        core.emit(queued("x", "X"));
        core.finish(TerminalReason::Killed);
        core.emit(queued("y", "Y"));
        assert_eq!(
            ring_ops(&core).len(),
            3,
            "합성 Dropped x · 봉인 Dropped y 가 링에 섰다"
        );
        assert_eq!(
            classified(),
            vec!["Queued x"],
            "종료 합성·봉인이 분류기를 지났다"
        );

        // Rewriting after the decision — only the original event (including the decoder's finding) passes.
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), record_classified);
        core.emit(queued("a", "A"));
        core.emit(ack_unavailable());
        core.emit(queued("b", "B"));
        assert_eq!(ring_ops(&core).len(), 4);
        assert_eq!(
            classified(),
            vec!["Queued a", "AckUnavailable", "Queued b"],
            "판정 뒤 잇는 AckUnavailable 이 분류기를 지났다"
        );
    }

    /// With the production claude classifier — except for the receipt (`Delivered`), list events do not turn on turn observation (including rewritten events).
    #[test]
    fn list_events_other_than_delivered_never_turn_on_the_turn_observation() {
        let fx = Fixture::new();
        let probe = fx.probe();
        let core = fx.core(probe.clone(), claude_classifier());
        core.emit(queued("a", "A"));
        core.emit(cancel_requested("a"));
        core.emit(cancel_answered("a", false));
        core.emit(queued("b", "B"));
        core.emit(dropped("b", DropCause::Rejected));
        core.emit(ack_unavailable());
        core.emit(queued("c", "C"));
        assert!(!fx.turns.is_in_turn(fx.id, EPOCH), "목록 사건이 턴을 켰다");
        assert!(
            probe.at_turn_end.lock().unwrap().is_empty(),
            "목록 사건이 턴을 끝냈다"
        );
    }

    // ── Write order of the pending-list table ─────────────────────────────────────────────────────────────

    /// 「찼다」 (became full) is inside the replay lock that reduced it — the side that received that emit's fanout already reads true.
    #[test]
    fn filled_is_written_inside_the_lock_that_reduced_it() {
        struct ReadsPendingOnQueued {
            id: SinkId,
            agent: AgentId,
            pending: Arc<InputsPendingTable>,
            seen: Mutex<Vec<Option<bool>>>,
        }
        impl OutputSink for ReadsPendingOnQueued {
            fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
                if let OutputPayload::Event(OutputEvent::QueuedInput(QueuedInputEvent::Queued {
                    ..
                })) = frame.payload
                {
                    let v = self.pending.get(self.agent, EPOCH);
                    self.seen.lock().unwrap().push(v);
                }
                Ok(())
            }
            fn sink_id(&self) -> SinkId {
                self.id
            }
        }
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        let sink = Arc::new(ReadsPendingOnQueued {
            id: uuid::Uuid::new_v4(),
            agent: fx.id,
            pending: fx.pending.clone(),
            seen: Mutex::new(Vec::new()),
        });
        core.subscribe(sink.clone());
        assert_eq!(fx.pending(), Some(false));
        core.emit(queued("a", "A"));
        assert_eq!(*sink.seen.lock().unwrap(), vec![Some(true)]);
    }

    /// 「비었다」 (became empty) is **after** that emit's turn observation — if the receipt that empties the list is also a turn end, the table is
    /// still true at the turn-end doorbell moment and false at the became-empty doorbell moment.
    #[test]
    fn drained_is_written_after_the_turn_observation_of_the_same_emit() {
        let fx = Fixture::new();
        let probe = fx.probe();
        let core = fx.core(probe.clone(), delivered_ends_the_turn);
        core.emit(queued("a", "A"));
        core.emit(delivered("a"));
        assert_eq!(*probe.at_turn_end.lock().unwrap(), vec![Some(true)]);
        assert_eq!(probe.drains(), vec![(Some(false), false)]);
    }

    /// The claude `Delivered` that empties the list (progress of the drain turn) — at the moment the table turns false, turn observation is already progress.
    /// An observer reading nonstop in the same order as the adapter (table first, turn observation later) never sees 「둘 다 한가」 (both idle) even once.
    #[test]
    fn a_draining_claude_delivery_is_never_observed_idle_on_both_facts() {
        let fx = Fixture::new();
        let probe = fx.probe();
        let core = fx.core(probe.clone(), claude_classifier());
        let stop = Arc::new(AtomicBool::new(false));
        let (filled_tx, filled_rx) = mpsc::channel();
        let observer = {
            let (pending, turns, id, stop) =
                (fx.pending.clone(), fx.turns.clone(), fx.id, stop.clone());
            std::thread::spawn(move || {
                let mut filled_seen = false;
                let mut idle_after_filled = 0usize;
                while !stop.load(Ordering::Acquire) {
                    let p = pending.get(id, EPOCH);
                    let in_turn = turns.is_in_turn(id, EPOCH);
                    if p == Some(true) && !filled_seen {
                        filled_seen = true;
                        filled_tx.send(()).expect("filled");
                    }
                    if filled_seen && p == Some(false) && !in_turn {
                        idle_after_filled += 1;
                    }
                }
                idle_after_filled
            })
        };
        core.emit(queued("a", "A"));
        filled_rx.recv_timeout(WAIT).expect("관찰자가 찼다를 봤다");
        core.emit(delivered("a"));
        stop.store(true, Ordering::Release);
        assert_eq!(observer.join().expect("observer"), 0);
        assert_eq!(probe.drains(), vec![(Some(false), true)]);
    }

    /// A door that does not pass through turn observation also, when it empties the list, writes and rings right after releasing the lock · a terminal synthesis chunk
    /// does not ring even if it empties the list and only reaps from the table.
    #[test]
    fn unobserved_emits_drain_and_ring_but_the_finish_batch_only_forgets() {
        let fx = Fixture::new();
        let probe = fx.probe();
        let core = fx.core(probe.clone(), claude_classifier());
        core.emit_without_turn_observation(queued("a", "A"));
        assert_eq!(fx.pending(), Some(true));
        core.emit_without_turn_observation(delivered("a"));
        assert_eq!(probe.drains(), vec![(Some(false), false)]);

        core.emit(queued("b", "B"));
        assert_eq!(fx.pending(), Some(true));
        core.finish(TerminalReason::Killed);
        assert!(rows(&core).is_empty(), "종료 합성이 목록을 비웠다(전제)");
        // ADR-0231: terminal synthesis does not ring the doorbell — the pending-list table entry is reaped right away, and the doorbell would be a stimulus to flow mail into an incarnation about to make its
        //   terminal transition (parked mail is carried by the next incarnation's appearance flush).
        assert_eq!(
            probe.drains().len(),
            1,
            "종료 합성이 죽어가는 화신 앞으로 초인종을 울렸다"
        );
        assert_eq!(fx.pending(), None, "표에서 거둔다");
    }

    /// The table drops writes with a smaller seq — a 「비었다」 (became empty) (seq N) deferred outside the lock does not overwrite a 「찼다」 (became full) (seq N+1) that another thread
    /// wrote inside the lock in between.
    #[test]
    fn a_deferred_drain_never_overwrites_a_later_fill() {
        /// Catch at the first turn-end doorbell — a spot that holds the thread right before it writes 「비었다」 (became empty) (test only).
        struct HoldFirstTurnEnd {
            hold: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
        }
        impl StatusSink for HoldFirstTurnEnd {
            fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
            fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
            fn turn_ended(&self, _id: AgentId, _epoch: u32) {
                let taken = self.hold.lock().unwrap().take();
                if let Some((entered, release)) = taken {
                    entered.send(()).expect("entered");
                    release.recv_timeout(WAIT).expect("release");
                }
            }
        }
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let fx = Fixture::new();
        let core = fx.core(
            Arc::new(HoldFirstTurnEnd {
                hold: Mutex::new(Some((entered_tx, release_rx))),
            }),
            delivered_ends_the_turn,
        );
        core.emit(queued("a", "A"));
        let drainer = {
            let core = core.clone();
            std::thread::spawn(move || core.emit(delivered("a")))
        };
        entered_rx.recv_timeout(WAIT).expect("비우는 스레드가 섰다");
        assert!(rows(&core).is_empty(), "명부는 이미 비었다(락 안)");
        core.emit(queued("b", "B"));
        release_tx.send(()).expect("release");
        drainer.join().expect("drainer");

        assert_eq!(
            fx.pending(),
            Some(true),
            "늦게 쓴 더 작은 seq 의 「비었다」가 「찼다」를 덮었다"
        );
        assert_eq!(rows(&core).len(), 1);
    }

    /// The doorbell rings once at the moment of emptying — including when only the list empties without a turn end, and it does not ring while it stays full.
    #[test]
    fn the_drain_doorbell_rings_once_per_drain() {
        let fx = Fixture::new();
        let probe = fx.probe();
        let core = fx.core(probe.clone(), claude_classifier());
        core.emit(queued("a", "A"));
        core.emit(queued("b", "B"));
        core.emit(delivered("a"));
        assert!(probe.drains().is_empty());
        core.emit(delivered("b"));
        assert_eq!(probe.drains().len(), 1);
        core.emit(queued("c", "C"));
        core.emit(dropped("c", DropCause::Withdrawn));
        assert_eq!(probe.drains().len(), 2);
        core.emit(delivered("c"));
        assert_eq!(
            probe.drains().len(),
            2,
            "빈 목록의 묘비 사건은 울리지 않는다"
        );
        assert!(
            probe.at_turn_end.lock().unwrap().is_empty(),
            "턴 끝은 없었다"
        );
    }

    /// A core without the registry plugged in has the same ring shape (seal · rewriting · copy) — only table writes and the doorbell are missing.
    #[test]
    fn a_core_without_queued_wiring_shapes_the_ring_the_same_but_rings_nothing() {
        struct CountsDrains(AtomicU64);
        impl StatusSink for CountsDrains {
            fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
            fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
            fn inputs_drained(&self, _id: AgentId, _epoch: u32) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let drains = Arc::new(CountsDrains(AtomicU64::new(0)));
        let core = OutputCore::new(
            uuid::Uuid::new_v4(),
            EPOCH,
            drains.clone(),
            TurnWiring::detached(),
        );
        core.emit(queued("a", "A"));
        core.emit(ack_unavailable());
        core.emit(queued("b", "B"));
        core.emit(queued("open", "O"));
        core.finish(TerminalReason::Killed);
        core.emit(queued("c", "C"));
        let kinds: Vec<String> = ring_ops(&core)
            .into_iter()
            .map(|(_, op)| match op {
                QueuedInputEvent::Queued { id, .. } => format!("Queued {id}"),
                QueuedInputEvent::AckUnavailable { delivered } => format!(
                    "Ack {:?}",
                    delivered.iter().map(|c| c.id.as_str()).collect::<Vec<_>>()
                ),
                QueuedInputEvent::Dropped { id, cause } => format!("Dropped {id} {cause:?}"),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "Queued a",
                "Ack [\"a\"]",
                "Queued b",
                "Ack [\"b\"]",
                "Queued open",
                "Ack [\"open\"]",
                "Dropped c AgentEnded",
            ]
        );
        assert_eq!(drains.0.load(Ordering::SeqCst), 0);
    }

    // ── Live delivery (§5-7 premise) ───────────────────────────────────────────────────────────────

    /// Two threads emit and the test sink holds one side's send to create an inversion (test only — the production sink is not
    /// blocked) → that sink receives N+1 before N but receives **both** · a new sink attached in between also receives every issued seq
    /// without a miss (replay or live). Grounds = the ring push is before fanout and subscribe takes the replay while holding the subscribers lock
    /// — if either flips, the new sink loses N.
    #[test]
    fn a_held_send_reorders_delivery_but_every_sink_gets_every_seq() {
        struct HoldsSeq {
            id: SinkId,
            hold_seq: u64,
            entered: Mutex<Option<mpsc::Sender<()>>>,
            release: Mutex<Option<mpsc::Receiver<()>>>,
            got: Mutex<Vec<u64>>,
        }
        impl OutputSink for HoldsSeq {
            fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
                if frame.seq == self.hold_seq {
                    if let Some(tx) = self.entered.lock().unwrap().take() {
                        tx.send(()).expect("entered");
                    }
                    let rx = self.release.lock().unwrap().take();
                    if let Some(rx) = rx {
                        rx.recv_timeout(WAIT).expect("release");
                    }
                }
                self.got.lock().unwrap().push(frame.seq);
                Ok(())
            }
            fn sink_id(&self) -> SinkId {
                self.id
            }
        }
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let held = Arc::new(HoldsSeq {
            id: uuid::Uuid::new_v4(),
            hold_seq: 0,
            entered: Mutex::new(Some(entered_tx)),
            release: Mutex::new(Some(release_rx)),
            got: Mutex::new(Vec::new()),
        });
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        core.subscribe(held.clone());

        let first = {
            let core = core.clone();
            std::thread::spawn(move || core.emit(queued("n", "N")))
        };
        entered_rx
            .recv_timeout(WAIT)
            .expect("seq 0 의 send 가 붙잡혔다");
        core.emit(queued("n1", "N+1"));
        let late = Recorder::new();
        core.subscribe(late.clone());
        core.emit(delivered("n"));
        release_tx.send(()).expect("release");
        first.join().expect("first emitter");

        assert_eq!(
            held.got.lock().unwrap().clone(),
            vec![1, 2, 0],
            "붙잡힌 N 이 N+1 뒤에 도착한다 — 역전은 있다"
        );
        assert_eq!(
            late.seqs(),
            vec![0, 1, 2],
            "새 sink 는 replay + 라이브로 전부 받는다"
        );
    }

    // TRD §7-1: classification reads in the order pending-list table → turn observation (the read order is recorded) · if the table is true, turn observation is not read.
    #[test]
    fn the_classification_reads_the_pending_table_before_the_turn_table() {
        for (pending, in_turn, busy, reads) in [
            (true, false, true, vec!["pending"]),
            (true, true, true, vec!["pending"]),
            (false, true, true, vec!["pending", "turn"]),
            (false, false, false, vec!["pending", "turn"]),
        ] {
            struct Scripted {
                pending: bool,
                in_turn: bool,
                log: RefCell<Vec<&'static str>>,
            }
            impl BusyReads for Scripted {
                fn read_pending_listed(&self) -> bool {
                    self.log.borrow_mut().push("pending");
                    self.pending
                }
                fn read_in_turn(&self) -> bool {
                    self.log.borrow_mut().push("turn");
                    self.in_turn
                }
            }
            let reads_of = Scripted {
                pending,
                in_turn,
                log: RefCell::new(Vec::new()),
            };
            let got = pending_then_in_turn(&reads_of);
            assert_eq!(got, busy, "({pending}, {in_turn})");
            assert_eq!(reads_of.log.into_inner(), reads, "({pending}, {in_turn})");
        }
    }

    /// The two reads each read only their own table — if the table the name points to and the table actually read are swapped, it breaks here (the order
    /// assertion above leans on the names, so this is its pair).
    // ADR-0231
    #[test]
    fn each_busy_reader_reads_only_its_own_table() {
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        assert!(!core.read_pending_listed() && !core.read_in_turn());

        fx.pending.set(fx.id, EPOCH, 100, true);
        assert!(core.read_pending_listed(), "표만 찼다");
        assert!(!core.read_in_turn(), "턴 관측은 표를 읽지 않는다");

        fx.pending.set(fx.id, EPOCH, 101, false);
        core.emit(OutputEvent::TextDelta {
            text: "thinking".into(),
            turn_id: None,
            message_id: None,
        });
        assert!(core.read_in_turn(), "턴 관측만 턴 중");
        assert!(
            !core.read_pending_listed(),
            "목록 읽기는 턴 관측을 읽지 않는다"
        );
    }

    /// The condition of the old-CLI decision log — stands only in the one span where the registry's finding marker changes false → true. Neither a second decision nor the `AckUnavailable`
    /// the core chains onto a `Queued` after the finding is raised again.
    // ADR-0231
    #[test]
    fn the_ack_unavailable_verdict_is_flagged_once_per_incarnation() {
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        let judged = |op: QueuedInputEvent| {
            let mut replay = core.replay.lock().unwrap();
            core.record_list_event(&mut replay, op)
                .ack_unavailable_judged
        };
        let unavailable = || QueuedInputEvent::AckUnavailable {
            delivered: Vec::new(),
        };
        let queued_op = |id: &str| QueuedInputEvent::Queued {
            id: id.into(),
            text: "t".into(),
        };

        assert!(!judged(queued_op("q1")));
        assert!(judged(unavailable()), "첫 판정");
        assert!(!judged(unavailable()), "둘째 판정은 다시 세우지 않는다");
        assert!(!judged(queued_op("q2")), "판명 뒤 바꿔 적기");
    }

    #[test]
    fn classified_input_busy_reads_the_two_tables_of_this_incarnation() {
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        assert!(!core.classified_input_busy(), "빈 목록 · 턴 없음 = 한가");
        core.emit(queued("q1", "one"));
        assert!(core.classified_input_busy(), "목록이 찼다");
        core.emit(delivered("q1"));
        assert_eq!(fx.pending(), Some(false));
        assert!(core.classified_input_busy(), "받음 = 진행 — 턴 중");

        let detached = OutputCore::new(fx.id, EPOCH, Arc::new(Quiet), TurnWiring::detached());
        detached.emit(queued("q1", "one"));
        assert!(
            !detached.classified_input_busy(),
            "표가 없는 조립은 목록을 빈 것으로 읽는다"
        );
    }

    // The door for a cancel request — sets `CancelRequested` only on a `Queued` row, and otherwise does not touch the ring.
    #[test]
    fn a_cancel_request_is_recorded_only_for_a_queued_row() {
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        core.emit(queued("q1", "one"));
        core.emit(queued("done", "two"));
        core.emit(delivered("done"));
        let before = ring_ops(&core).len();

        assert_eq!(core.emit_cancel_request("never"), CancelRequest::NotListed);
        assert_eq!(core.emit_cancel_request("done"), CancelRequest::NotListed);
        assert_eq!(
            ring_ops(&core).len(),
            before,
            "목록 밖 id 는 링에 아무것도 안 세운다"
        );

        assert_eq!(core.emit_cancel_request("q1"), CancelRequest::Recorded);
        let ops = ring_ops(&core);
        assert_eq!(ops.len(), before + 1);
        assert_eq!(
            ops.last().unwrap().1,
            QueuedInputEvent::CancelRequested { id: "q1".into() }
        );
        assert!(matches!(
            core.queued_inputs().lock().rows()[0].phase,
            RowPhase::Cancelling { .. }
        ));

        assert_eq!(
            core.emit_cancel_request("q1"),
            CancelRequest::AlreadyCancelling
        );
        assert_eq!(
            ring_ops(&core).len(),
            before + 1,
            "멱등 — 둘째 요청은 링을 안 건드린다"
        );
        assert_eq!(fx.pending(), Some(true), "취소 대기도 목록이다");
    }
}
