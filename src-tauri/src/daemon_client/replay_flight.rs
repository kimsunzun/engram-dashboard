//! Single-flight replay numbering/fence state machine + replay boundary marker encoding (ADR-0046 M1).
//!
//! src-tauri dropped the mirror buffer (ADR-0046) and became a stateless pass-through router. A remount/reload/new window gets the daemon ring fully replayed again; so the view can know that replay's end, a single flight per agent maps the wire `Subscribe`↔`ReplayComplete` 1:1 and stamps a `gen`. The view flushes with sort+dedup only on a success marker whose gen is at least what its own `requestReplay` got back (gen fence — blocks an early flush by someone else's / an old generation's replay).
//!
//! ## No count-based coupling
//! Inferring gen↔Complete by counting/last-value stamping desyncs (last value = early flush loses data; FIFO = a failure path missing Complete desyncs permanently). So exactly one in-flight is enforced: every request arriving during an in-flight merges into "one next Subscribe", sent when the current in-flight resolves.
//!
//! ## Lifetime · invariants (TRD rev4 §2 + FIX round — zombie semantics)
//! - in_flight lifetime = sent → (SubscribeAck) → acked → (ReplayComplete) → success marker. The slot has three release paths only: resolution (Complete after Ack) · refusal ([`ReplayFlightSet::on_refused`]) · disconnect. Exceeding the deadline does not release it.
//! - A success marker may follow a failure marker on the same gen safely: a view that re-requested holds a higher myGen and ignores it, and a view still waiting flushes the complete buffer (it keeps its buffer on a failure marker). Under the progress-based deadline a late resolution implies an empty/near-empty replay whose frames the view already buffered.
//! - agent-gone releases the slot: the daemon emits that refusal as `AgentEvent::SubscribeFailed` ([`ReplayFlightSet::on_refused`]); a bare `Error` carries no owner field, so the client could not know which slot to release (do not revive that wiring).
//! - One zombie shape remains: the daemon gives only `Ack` and never `ReplayComplete` or a refusal (a response may still come, so expiry cannot release the slot). The slot stays until disconnect; the failure marker already went out at the first expiry and the UX is handled by the view's bounded re-request ladder (accepted). The daemon dropping `Complete` on a saturated queue is not this case — it closes the connection on the spot and ends in a disconnect (the ReplayComplete enqueue failure branch of the daemon's `handle_subscribe`).
//!
//! ## Single outstanding — a premise kept outside this file
//! [`ReplayFlightSet::on_ack`] · [`ReplayFlightSet::on_complete`] · [`ReplayFlightSet::on_refused`] do not check the arriving response's generation (the wire has no generation field — the daemon does not know gen); they rely on at most one Subscribe of that agent being on the wire and being exactly the current `in_flight`. So this file has no entry point that produces a Subscribe that went on the wire with its tracking lost — one such function would misstamp an old generation's Ack/Complete/refusal onto the new slot and attach a success marker to a gen whose replay never ran (gen fence collapse). The caller-side discipline is the replay Subscribe send-failure handling in `connection.rs` (it drops the connection).
//!
//! ## Purity ① test isolation — zero socket · tokio · Tauri (ADR-0012 · ADR-0175 decision 3)
//! It lives in the shell package but touches none of its wiring. Time comes as an `Instant` argument so unit tests are deterministic (side effects — sending the marker · the wire Subscribe — are the caller's = the connection task's).
//!
//! ## Purity ② ignorant of the wire contract — zero protocol dependencies (ADR-0046 decisions 3·4)
//! The boundary marker tag 255 stamped here is a shell↔webview Channel-internal contract absent from the daemon codec (see `MARKER_TAG`), and the only state ADR-0046 decision 3 allows the shell is request bookkeeping (epoch · single-flight). Importing protocol would let later sessions use wire types here and make this module know a contract it takes no part in. It is not "because that crate is heavy" — protocol is a pure data crate with no runtime, so its value for ①'s determinism is 0; explaining the clause as a reason of ① is wrong.
//!
//! ## Keeping the two above — a gate, not prose
//! `.github/workflows/ci.yml`'s `Gate: replay_flight purity — no transport/wire ingress` scans this file and FAILs if a `tauri` · `tokio` · `std::net` · `engram_dashboard_*` path appears outside comments, including inline full paths and spaced `::` (`tokio :: spawn`) — unlike the sibling-crate isolation gate there is no compiler backstop (the shell package depends on all three, so a single `tokio::spawn(..)` just compiles). The gate's authority (expected values · known limits) = that step's comment in `ci.yml` (ADR-0175 §Impact).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use uuid::Uuid;

/// Agent identifier — the same `uuid::Uuid` as `engram_dashboard_protocol::AgentId`, so it passes straight through at call sites. A duplicate kept knowingly: using that alias would import protocol and make this file know the wire contract (「Purity ②」); not a cost matter.
type AgentId = Uuid;

/// Wire tag of the replay boundary marker — a src-tauri↔webview Channel-internal contract value absent from the daemon codec (tag0/tag1). The frontend decodeOutputFrame skips it as an unknown tag (forward compatible, M0); M2 consumes it formally.
pub const MARKER_TAG: u8 = 255;

/// Total marker frame length = tag(1) + agentId(16) + epoch(4) + gen(8) + flags(1) + replay_from(8) = 38 bytes. Shell and webview ship in one build, so the webview rejects a shorter marker (ADR-0231 — including the old 30 bytes).
pub const MARKER_FRAME_LEN: usize = 1 + 16 + 4 + 8 + 1 + 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    pub generation: u64,
    pub truncated: bool,
    /// Failure termination (progress-based deadline exceeded): the view must not flush and climbs the re-request ladder (M2).
    pub failed: bool,
    /// The incarnation of this replay came up to resume a stored conversation (propagates SubscribeAck.continues_conversation — not whether the resume succeeded). Can be true only on a success marker: the two failure-marker builders ([`ReplayFlightSet::check_deadlines`] · [`ReplayFlightSet::on_refused`]) set it `false` and [`encode_marker_frame`] also clears bit2 of a failure marker (the Ack may not have come). View-side contract (ADR-0226 A4): read only on a success flush.
    pub continues_conversation: bool,
    /// The head of this replay — that generation's `SubscribeAck.replay_from` (the seq the daemon actually sent first; for an empty ring the next seq to issue). The view starts the success flush at `max(last+1, replay_from)` (ADR-0231). Received rather than derived from the buffer's minimum seq, which is wrong for an empty replay. 0 before the Ack; meaningless on a failure marker (no flush).
    pub replay_from: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestOutcome {
    pub generation: u64,
    /// `true` = send the wire `Subscribe{after_seq:None}` now (idle). `false` = merged because an in-flight runs; the Subscribe fires through [`Resolution::send_next`] when the current in-flight resolves.
    pub send_now: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalOutcome {
    Ignore,
    Released {
        /// Failure marker. `None` if it was already emitted once as a zombie (no duplicate emission — it would push the view's re-request ladder twice).
        marker: Option<Marker>,
        send_next: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// A Complete that arrived before Ack (orphan of the previous generation) or a stray with no in_flight.
    Ignore,
    Emit {
        marker: Marker,
        send_next: bool,
    },
}

#[derive(Default)]
struct AgentFlight {
    gen_counter: u64,
    in_flight: Option<InFlight>,
    /// The gen of the next Subscribe, shared by requests that arrived during an in-flight. Invariant: `Some` ⟹ `in_flight` is `Some`.
    next_gen: Option<u64>,
}

struct InFlight {
    generation: u64,
    acked: bool,
    truncated: bool,
    continues_conversation: bool,
    replay_from: u64,
    /// Zombie flag (FIX-1): the failure marker was already emitted once (blocks re-emission); rationale in [`ReplayFlightSet::check_deadlines`].
    failed: bool,
    deadline: Instant,
}

impl InFlight {
    fn fresh(generation: u64, now: Instant, deadline: Duration) -> Self {
        InFlight {
            generation,
            acked: false,
            truncated: false,
            continues_conversation: false,
            replay_from: 0,
            failed: false,
            deadline: now + deadline,
        }
    }
}

/// Owned by the connection task (actor) through `&mut` and driven serially — no internal lock.
pub struct ReplayFlightSet {
    agents: HashMap<AgentId, AgentFlight>,
    deadline: Duration,
}

impl ReplayFlightSet {
    pub fn new(deadline: Duration) -> Self {
        Self {
            agents: HashMap::new(),
            deadline,
        }
    }

    /// Entry point of the contract that always terminates with exactly one marker.
    pub fn request_replay(&mut self, agent: AgentId, now: Instant) -> RequestOutcome {
        let deadline = self.deadline;
        let f = self.agents.entry(agent).or_default();
        if f.in_flight.is_none() {
            f.gen_counter += 1;
            let generation = f.gen_counter;
            f.in_flight = Some(InFlight::fresh(generation, now, deadline));
            RequestOutcome {
                generation,
                send_now: true,
            }
        } else {
            // Coalesce safety: a request always arrives after the view's SubState registration, so merging into a later-fired Subscribe is safe (N views remounting at once ≤ 2 replays). A zombie slot is merged here too — when the zombie is released by a late Complete/disconnect this next_gen goes out.
            let generation = match f.next_gen {
                Some(g) => g,
                None => {
                    f.gen_counter += 1;
                    f.next_gen = Some(f.gen_counter);
                    f.gen_counter
                }
            };
            RequestOutcome {
                generation,
                send_now: false,
            }
        }
    }

    // There is deliberately no send-failure rollback entry point (do not revive it): the old `abort_in_flight` emptied the slot when the `send_now` Subscribe's wire send failed and kept polling the connection. That puts the single-outstanding premise (module header) on an assumption about the caller — safe only if "send failure = the peer did not receive it", which is tungstenite/TCP error semantics, not something we enforce. Instead the caller drops the connection on the spot (the `connection.rs` Subscribe(replay) send-failure branch) and [`Self::on_disconnect`] cleans everything — broader than a rollback, but the premise becomes code.
    // One sanctioned exception (the driver's `write_follow_up`): a merged request promoted by `on_complete` / `on_refused` whose agent no window shows any more is released with `on_refused` before its Subscribe is attempted — it never reached the wire, so the premise holds.

    /// Being single-flight, an arriving Ack is always the one outstanding in-flight's (a zombie's late Ack too), so it stamps onto the current slot with no gen comparison. `replay_from` = that Ack's replay head, carried by this generation's success marker ([`Marker::replay_from`]).
    pub fn on_ack(
        &mut self,
        agent: AgentId,
        truncated: bool,
        continues_conversation: bool,
        replay_from: u64,
        now: Instant,
    ) {
        let deadline = self.deadline;
        if let Some(f) = self.agents.get_mut(&agent) {
            if let Some(inf) = f.in_flight.as_mut() {
                inf.acked = true;
                inf.truncated = truncated;
                inf.continues_conversation = continues_conversation;
                inf.replay_from = replay_from;
                inf.deadline = now + deadline;
            }
        }
    }

    pub fn note_progress(&mut self, agent: AgentId, now: Instant) {
        let deadline = self.deadline;
        if let Some(f) = self.agents.get_mut(&agent) {
            if let Some(inf) = f.in_flight.as_mut() {
                inf.deadline = now + deadline;
            }
        }
    }

    /// Stamps a success marker only on an in_flight in acked state. Wire order is `[Ack_k]…[Complete_k][Ack_{k+1}]`, so a Complete arriving before Ack is provably an orphan of the previous generation → Ignore (misattribution blocked at the source). `now` = deadline baseline of the new in_flight.
    /// A zombie is released here too (FIX-1): a slot that became a zombie (`failed=true`) via the deadline also passes the acked gate on a late Ack→late Complete and is released with a success marker of the same gen (evidence the replay really completed).
    pub fn on_complete(&mut self, agent: AgentId, now: Instant) -> Resolution {
        let deadline = self.deadline;
        let Some(f) = self.agents.get_mut(&agent) else {
            return Resolution::Ignore;
        };
        let acked = matches!(&f.in_flight, Some(inf) if inf.acked);
        if !acked {
            return Resolution::Ignore;
        }
        let inf = f.in_flight.take().expect("if acked, in_flight is Some");
        let marker = Marker {
            generation: inf.generation,
            truncated: inf.truncated,
            failed: false,
            continues_conversation: inf.continues_conversation,
            replay_from: inf.replay_from,
        };
        let send_next = advance_next(f, now, deadline);
        Resolution::Emit { marker, send_next }
    }

    /// Progress-based deadline sweep (FIX-1 — zombie semantics). An in-flight expired with no progress is emitted once as a failure marker and marked a zombie (`failed=true`), but the slot is kept and the queue is not advanced. Returns `(agent, failure marker)` pairs; the caller sends only the markers — the next Subscribe never goes out from here.
    /// Why the queue is not advanced (cross-family reviewer — load-bearing): moving straight to the next Subscribe and replacing the slot would misstamp a late Ack/Complete of the expired generation onto the new in-flight and attach a success marker to a new gen whose replay has not run (gen fence collapse). With the zombie holding the slot an Ack/Complete can structurally point only at the one outstanding Subscribe (this zombie), so misattribution is impossible. The slot is released only by resolution (late Complete) · refusal ([`Self::on_refused`]) · disconnect.
    /// Why a refusal may release and a timeout may not: only whether a response can still come. A timeout is merely "quiet for 10 seconds", so a late Ack/Complete can still arrive and the slot must be held; a refusal (`AgentEvent::SubscribeFailed`) is the daemon's notice that it finished by deciding not to process that Subscribe — neither Ack nor Complete follows (the daemon's `handle_subscribe` returns early before emitting Ack; its only failure is the `get_session` lookup, structurally before `on_ready`), so nothing remains to misattribute. Hence the easy variant "release the expired zombie" was not chosen.
    pub fn check_deadlines(&mut self, now: Instant) -> Vec<(AgentId, Marker)> {
        let mut out = Vec::new();
        for (agent, f) in self.agents.iter_mut() {
            if let Some(inf) = f.in_flight.as_mut() {
                if !inf.failed && now >= inf.deadline {
                    inf.failed = true;
                    out.push((
                        *agent,
                        Marker {
                            generation: inf.generation,
                            truncated: inf.truncated,
                            failed: true,
                            continues_conversation: false,
                            replay_from: inf.replay_from,
                        },
                    ));
                }
            }
        }
        out
    }

    /// Whether a replay still waits for its deadline — one [`Self::check_deadlines`] could still fail; with none, no sweep is needed.
    pub fn awaits_deadline(&self) -> bool {
        self.agents
            .values()
            .any(|f| f.in_flight.as_ref().is_some_and(|inf| !inf.failed))
    }

    /// The daemon refused this agent's `Subscribe` (`AgentEvent::SubscribeFailed`): releases the slot, emits the failure marker now if not yet emitted (so the view does not wait for the 10 s deadline), and advances the merged queue.
    /// Why the release is safe: see "why a refusal may release" in [`Self::check_deadlines`] — a refused subscription has no Ack/Complete to follow, so no response can be misattributed.
    /// Why the generation is not compared: the wire refusal has no generation field (the daemon does not know gen), yet releasing the current `in_flight` is right because that agent has at most one Subscribe on the wire (module header's single-outstanding premise). A new Subscribe goes out only via (a) an idle request · (b) the `send_next` of [`Self::on_complete`] / this function, both after the previous generation resolved — so "a refusal for an already resolved old generation while the slot is another generation" cannot occur. The only shape breaking the premise is a Subscribe left on the wire with its tracking lost, and no such entry point exists (see "no send-failure rollback entry point" above).
    /// An acked slot is not released (defense): a subscription that got an Ack was accepted by the daemon and cannot coexist with a refusal; releasing would make the following `ReplayComplete` meet an empty slot and fall to `Ignore` at [`Self::on_complete`]'s acked gate, so the view would never get the success marker — one misfired refusal killing a normal replay, closed cheaply.
    /// Releasing is right even when the refusal reason is transient: whether a retry is worthwhile is not decided here (the view's bounded ladder drives re-requests); releasing only lets that re-request go on the wire, while holding the slot would turn a transient failure into a permanent cutoff (this defect's actual shape).
    pub fn on_refused(&mut self, agent: AgentId, now: Instant) -> RefusalOutcome {
        let deadline = self.deadline;
        let Some(f) = self.agents.get_mut(&agent) else {
            return RefusalOutcome::Ignore;
        };
        match f.in_flight.as_ref() {
            None => return RefusalOutcome::Ignore,
            Some(inf) if inf.acked => return RefusalOutcome::Ignore,
            Some(_) => {}
        }
        let inf = f.in_flight.take().expect("Some confirmed just above");
        let marker = if inf.failed {
            None
        } else {
            Some(Marker {
                generation: inf.generation,
                truncated: inf.truncated,
                failed: true,
                continues_conversation: false,
                replay_from: inf.replay_from,
            })
        };
        let send_next = advance_next(f, now, deadline);
        RefusalOutcome::Released { marker, send_next }
    }

    /// Connection lost — internal clear only, no marker emitted (the re-request driver is the frontend connected transition alone). A zombie left until disconnect due to agent-gone is also cleaned here.
    pub fn on_disconnect(&mut self) {
        for f in self.agents.values_mut() {
            f.in_flight = None;
            f.next_gen = None;
        }
    }
}

fn advance_next(f: &mut AgentFlight, now: Instant, deadline: Duration) -> bool {
    match f.next_gen.take() {
        Some(g) => {
            f.in_flight = Some(InFlight::fresh(g, now, deadline));
            true
        }
        None => false,
    }
}

/// Encode a replay boundary marker into wire frame bytes (Channel-internal contract, ADR-0046; M2 parser contract, paired with the webview `decodeReplayMarker`): `[tag=255:1][agentId:16][epoch:4 BE][gen:8 BE][flags:1][replay_from:8 BE]` — fixed 38 bytes.
/// Endianness (FIX-4 — whole marker frame BE): agentId is RFC4122 network order (same as the frame header); epoch · gen · replay_from are big-endian (the binary frame header is uniformly BE, so the M2 parser reads with one convention). `replay_from` follows flags at the very end (ADR-0231) so earlier field offsets stay put and old readers do not shift. flags bit0=truncated, bit1=failed, bit2=continues_conversation; bit2 is always 0 on a failure marker (bit1) — cleared here even if the input `Marker` carries true (ADR-0226).
pub fn encode_marker_frame(agent_id: AgentId, epoch: u32, marker: Marker) -> Vec<u8> {
    let mut buf = Vec::with_capacity(MARKER_FRAME_LEN);
    buf.push(MARKER_TAG);
    buf.extend_from_slice(agent_id.as_bytes());
    buf.extend_from_slice(&epoch.to_be_bytes());
    buf.extend_from_slice(&marker.generation.to_be_bytes());
    let mut flags = 0u8;
    if marker.truncated {
        flags |= 0b0000_0001;
    }
    if marker.failed {
        flags |= 0b0000_0010;
    }
    if marker.continues_conversation && !marker.failed {
        flags |= 0b0000_0100;
    }
    buf.push(flags);
    // ADR-0231
    buf.extend_from_slice(&marker.replay_from.to_be_bytes());
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aid(n: u128) -> AgentId {
        Uuid::from_u128(n)
    }

    fn t0() -> Instant {
        Instant::now()
    }

    fn dl() -> Duration {
        Duration::from_secs(10)
    }

    // ── gen monotonic + idle request = Subscribe once ───────────────────────────────────────────
    #[test]
    fn idle_request_allocates_gen_and_sends_subscribe() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        let out = fs.request_replay(a, now);
        assert_eq!(out.generation, 1, "first gen=1");
        assert!(out.send_now, "when idle, Subscribe is sent immediately");
    }

    #[test]
    fn gen_counter_monotonic_across_cycles_never_resets() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        assert_eq!(fs.request_replay(a, now).generation, 1);
        fs.on_ack(a, false, false, 0, now);
        match fs.on_complete(a, now) {
            Resolution::Emit { marker, send_next } => {
                assert_eq!(marker.generation, 1);
                assert!(!send_next, "no queue means no next Subscribe");
            }
            other => panic!("should be a success marker: {other:?}"),
        }
        assert_eq!(
            fs.request_replay(a, now).generation,
            2,
            "gen is monotonic (no reset to 0)"
        );
        fs.on_ack(a, false, false, 0, now);
        assert!(matches!(
            fs.on_complete(a, now),
            Resolution::Emit {
                marker: Marker { generation: 2, .. },
                ..
            }
        ));
    }

    // ── coalescing ────────────────────────────────────────────────────────────────────
    #[test]
    fn coalesces_waiters_to_single_next_gen() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        let first = fs.request_replay(a, now);
        assert_eq!((first.generation, first.send_now), (1, true));
        let w1 = fs.request_replay(a, now);
        let w2 = fs.request_replay(a, now);
        assert_eq!(w1.generation, 2, "first waiter = next gen 2");
        assert_eq!(w2.generation, 2, "second waiter = same next gen 2 (shared)");
        assert!(
            !w1.send_now && !w2.send_now,
            "in flight, so not fired immediately"
        );
        fs.on_ack(a, false, false, 0, now);
        match fs.on_complete(a, now) {
            Resolution::Emit { marker, send_next } => {
                assert_eq!(
                    marker.generation, 1,
                    "what resolves is the current in-flight (gen1)"
                );
                assert!(send_next, "merged queue → one next Subscribe fired");
            }
            other => panic!("should be a success marker: {other:?}"),
        }
        fs.on_ack(a, false, false, 0, now);
        match fs.on_complete(a, now) {
            Resolution::Emit { marker, send_next } => {
                assert_eq!(marker.generation, 2);
                assert!(!send_next, "queue drained — no more Subscribe");
            }
            other => panic!("should be a success marker: {other:?}"),
        }
    }

    // ── acked gate ──────────────────────────────────────────────────────────────────
    #[test]
    fn complete_before_ack_is_orphan_ignored() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now);
        assert_eq!(
            fs.on_complete(a, now),
            Resolution::Ignore,
            "Complete before Ack is ignored"
        );
        fs.on_ack(a, false, false, 0, now);
        assert!(matches!(
            fs.on_complete(a, now),
            Resolution::Emit {
                marker: Marker {
                    generation: 1,
                    failed: false,
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn complete_without_inflight_is_ignored() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        assert_eq!(
            fs.on_complete(a, t0()),
            Resolution::Ignore,
            "with no in-flight, a stray Complete is ignored"
        );
    }

    // ── FIX-1 (a): zombie late-resolution sequence ────────────────────────────────────────
    #[test]
    fn zombie_late_ack_complete_emits_success_same_gen_then_advances() {
        let mut fs = ReplayFlightSet::new(Duration::from_millis(100));
        let a = aid(1);
        let start = t0();
        assert_eq!(fs.request_replay(a, start).generation, 1);
        let w = fs.request_replay(a, start);
        assert_eq!((w.generation, w.send_now), (2, false), "gen2 queue merged");
        let expired = fs.check_deadlines(start + Duration::from_millis(200));
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].0, a);
        assert_eq!(expired[0].1.generation, 1, "failure marker gen1");
        assert!(expired[0].1.failed, "no-progress timeout = failure marker");
        assert!(
            fs.check_deadlines(start + Duration::from_millis(300))
                .is_empty(),
            "a zombie does not reissue a failure marker"
        );
        fs.on_ack(a, false, false, 0, start + Duration::from_millis(400));
        match fs.on_complete(a, start + Duration::from_millis(500)) {
            Resolution::Emit { marker, send_next } => {
                assert_eq!(
                    marker.generation, 1,
                    "★the success marker is gen1 — not gen2★"
                );
                assert!(
                    !marker.failed,
                    "late Complete = success (replay actually completed)"
                );
                assert!(send_next, "only now is the merged gen2 Subscribe fired");
            }
            other => panic!("a late Complete is a success marker: {other:?}"),
        }
        fs.on_ack(a, false, false, 0, start + Duration::from_millis(600));
        match fs.on_complete(a, start + Duration::from_millis(700)) {
            Resolution::Emit { marker, send_next } => {
                assert_eq!(marker.generation, 2);
                assert!(!marker.failed);
                assert!(!send_next, "queue drained");
            }
            other => panic!("should be a success marker: {other:?}"),
        }
    }

    // ── FIX-1 (b): zombie not released + disconnect ───────────────────────────────────────────
    #[test]
    fn zombie_unresolved_then_disconnect_clears_no_markers_counter_monotonic() {
        let mut fs = ReplayFlightSet::new(Duration::from_millis(100));
        let a = aid(1);
        let start = t0();
        fs.request_replay(a, start); // gen1
        fs.request_replay(a, start); // gen2 queue
        let expired = fs.check_deadlines(start + Duration::from_millis(200));
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].1.generation, 1);
        assert!(expired[0].1.failed);
        let w = fs.request_replay(a, start + Duration::from_millis(300));
        assert!(
            !w.send_now,
            "a zombie holds the slot → new request merged (not fired immediately)"
        );
        fs.on_disconnect();
        let after = fs.request_replay(a, start + Duration::from_millis(400));
        assert_eq!(
            after.generation, 3,
            "gen_counter is monotonic (no reset to 0 even after a zombie)"
        );
        assert!(
            after.send_now,
            "disconnect emptied the slot; fired immediately"
        );
    }

    // ── progress-based deadline ────────────────────────────────────────────────────────────
    #[test]
    fn missing_complete_expires_to_failure_marker_then_zombie_resolves() {
        let mut fs = ReplayFlightSet::new(Duration::from_millis(100));
        let a = aid(1);
        let start = t0();
        fs.request_replay(a, start); // gen1 in-flight
        let expired = fs.check_deadlines(start + Duration::from_millis(200));
        assert_eq!(expired.len(), 1);
        let (agent, marker) = expired[0];
        assert_eq!(agent, a);
        assert_eq!(marker.generation, 1);
        assert!(marker.failed, "no-progress timeout = failure marker");
        assert!(
            !fs.request_replay(a, start + Duration::from_millis(250))
                .send_now,
            "a zombie holds the slot, so the new request is merged"
        );
        fs.on_ack(a, false, false, 0, start + Duration::from_millis(300));
        assert!(matches!(
            fs.on_complete(a, start + Duration::from_millis(300)),
            Resolution::Emit {
                marker: Marker {
                    generation: 1,
                    failed: false,
                    ..
                },
                send_next: true
            }
        ));
    }

    #[test]
    fn progress_resets_deadline_healthy_slow_replay_no_false_positive() {
        let mut fs = ReplayFlightSet::new(Duration::from_millis(100));
        let a = aid(1);
        let start = t0();
        fs.request_replay(a, start);
        fs.on_ack(a, false, false, 0, start);
        let mut now = start;
        for _ in 0..5 {
            now += Duration::from_millis(80); // progress before the deadline (100ms).
            fs.note_progress(a, now);
            assert!(
                fs.check_deadlines(now).is_empty(),
                "while progressing, no expiry"
            );
        }
        assert!(matches!(
            fs.on_complete(a, now),
            Resolution::Emit {
                marker: Marker { failed: false, .. },
                ..
            }
        ));
    }

    // ── refusal (SubscribeFailed) — slot release ──────────────────────────────────────────────
    // Regression target: right after a daemon restart there are no sessions (boot auto-restore OFF), so the reconnect replay's Subscribe is refused as "agent not found". A refusal has no Ack/Complete, so the slot stayed a zombie and that agent's Subscribe could never go out again (every request merged) — even after a re-spawn output stayed cut off in all windows. The first case below breaks exactly that "never again".
    #[test]
    fn refusal_releases_slot_so_next_request_sends_again() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        let first = fs.request_replay(a, now);
        assert_eq!((first.generation, first.send_now), (1, true));

        match fs.on_refused(a, now) {
            RefusalOutcome::Released { marker, send_next } => {
                let m = marker.expect(
                    "the first refusal yields a failure marker (so the view does not wait 10 s)",
                );
                assert_eq!(m.generation, 1);
                assert!(m.failed);
                assert!(!send_next, "no queue means no next Subscribe");
            }
            other => panic!("a refusal should release the slot: {other:?}"),
        }

        // The heart of the defect — before the fix `send_now=false` repeated forever.
        let next = fs.request_replay(a, now);
        assert_eq!(next.generation, 2, "gen is monotonic");
        assert!(
            next.send_now,
            "released, so the new Subscribe goes out immediately"
        );
    }

    #[test]
    fn refusal_advances_coalesced_waiter() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now); // gen1 in-flight
        let w = fs.request_replay(a, now); // gen2 merged
        assert_eq!((w.generation, w.send_now), (2, false));

        match fs.on_refused(a, now) {
            RefusalOutcome::Released { marker, send_next } => {
                assert_eq!(
                    marker.expect("failure marker").generation,
                    1,
                    "what is released is gen1"
                );
                assert!(send_next, "fire the merged gen2 Subscribe now");
            }
            other => panic!("should be a release: {other:?}"),
        }
        // gen2 has risen to in-flight — it goes through the normal Ack/Complete up to the success marker.
        fs.on_ack(a, false, false, 0, now);
        assert!(matches!(
            fs.on_complete(a, now),
            Resolution::Emit {
                marker: Marker {
                    generation: 2,
                    failed: false,
                    ..
                },
                ..
            }
        ));
    }

    // Pins the single-outstanding premise: a refusal has no generation field, so "what is released is the generation just refused" holds only by order — consecutive refusals each yield the generation on the wire at that time. If this breaks the gen fence collapses (module header · `on_refused`).
    #[test]
    fn consecutive_refusals_release_the_then_outstanding_generation() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now); // gen1 = went out on the wire
        let w = fs.request_replay(a, now); // gen2 merged (not sent yet)
        assert_eq!((w.generation, w.send_now), (2, false));

        match fs.on_refused(a, now) {
            RefusalOutcome::Released { marker, send_next } => {
                assert_eq!(
                    marker.expect("failure marker").generation,
                    1,
                    "first refusal = gen1"
                );
                assert!(send_next, "now gen2 goes out on the wire");
            }
            other => panic!("should be a release: {other:?}"),
        }
        // The second refusal arrives — at that point gen2 is the only outstanding one.
        match fs.on_refused(a, now) {
            RefusalOutcome::Released { marker, send_next } => {
                assert_eq!(
                    marker.expect("failure marker").generation,
                    2,
                    "second refusal = gen2"
                );
                assert!(!send_next, "queue drained");
            }
            other => panic!("should be a release: {other:?}"),
        }
        assert!(
            fs.request_replay(a, now).send_now,
            "the slot is empty, so it can fire again"
        );
    }

    // If a zombie (a failure marker was emitted once through expiry) receives a refusal late: it releases but **does not emit the marker again**.
    #[test]
    fn refusal_on_expired_zombie_releases_without_duplicate_marker() {
        let mut fs = ReplayFlightSet::new(Duration::from_millis(100));
        let a = aid(1);
        let start = t0();
        fs.request_replay(a, start);
        let expired = fs.check_deadlines(start + Duration::from_millis(200));
        assert_eq!(expired.len(), 1, "expiry failure marker once");

        match fs.on_refused(a, start + Duration::from_millis(300)) {
            RefusalOutcome::Released { marker, send_next } => {
                assert!(
                    marker.is_none(),
                    "a gen whose failure marker already went out — no duplicate issue"
                );
                assert!(!send_next);
            }
            other => panic!("should be a release: {other:?}"),
        }
        assert!(
            fs.request_replay(a, start + Duration::from_millis(400))
                .send_now,
            "a zombie is also released by a refusal"
        );
    }

    // Defense: a slot that received an Ack (accepted by the daemon) is not released by a refusal — released, the following `ReplayComplete` would meet an empty slot and fall to Ignore and that replay's success marker would never go out.
    #[test]
    fn refusal_never_releases_an_acked_healthy_slot() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now);
        fs.on_ack(a, false, false, 0, now);

        assert_eq!(
            fs.on_refused(a, now),
            RefusalOutcome::Ignore,
            "an acked slot is not released by a refusal"
        );
        // The normal completion path must stay alive as is.
        assert!(matches!(
            fs.on_complete(a, now),
            Resolution::Emit {
                marker: Marker {
                    generation: 1,
                    failed: false,
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn refusal_without_inflight_is_ignored() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        assert_eq!(
            fs.on_refused(a, now),
            RefusalOutcome::Ignore,
            "unknown agent"
        );
        fs.request_replay(a, now);
        fs.on_refused(a, now);
        assert_eq!(
            fs.on_refused(a, now),
            RefusalOutcome::Ignore,
            "a second refusal on an already released slot is ignored (no duplicate marker or wrong release)"
        );
    }

    // ── send failure = cleaned by disconnect ────────────────────────
    // Regression target: the old code emptied the slot on a wire send failure and kept polling the connection — a Subscribe that lost tracking can remain on the wire and a late refusal/Ack is misstamped onto the new-generation slot (gen fence collapse). Now the caller drops the connection on the spot and this path cleans everything; a re-request can still go out again.
    #[test]
    fn send_failure_path_clears_via_disconnect_and_next_request_sends() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        let first = fs.request_replay(a, now);
        assert_eq!((first.generation, first.send_now), (1, true));
        fs.on_disconnect(); // Send failure → the caller drops the connection.
        let next = fs.request_replay(a, now);
        assert_eq!(next.generation, 2, "gen is monotonic (1 used up → 2)");
        assert!(
            next.send_now,
            "disconnect cleanup emptied the slot; fired again immediately"
        );
        fs.on_ack(a, false, false, 0, now);
        assert!(matches!(
            fs.on_complete(a, now),
            Resolution::Emit {
                marker: Marker { generation: 2, .. },
                ..
            }
        ));
    }

    // ── disconnect ──────────────────────────────────────────────────────────────────────────
    #[test]
    fn disconnect_clears_inflight_and_waiters_keeps_counter() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now); // gen1 in-flight
        fs.request_replay(a, now); // gen2 queue
        fs.on_disconnect();
        let out = fs.request_replay(a, now);
        assert_eq!(
            out.generation, 3,
            "gen_counter stays monotonic (2 used up → 3)"
        );
        assert!(
            out.send_now,
            "disconnect emptied in-flight; fired immediately"
        );
    }

    // ── truncated flag propagation ──────────────────────────────────────────────────────────
    #[test]
    fn truncated_flag_propagates_to_success_marker() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now);
        fs.on_ack(a, true, false, 0, now); // The daemon cut the past beyond the lower bound.
        match fs.on_complete(a, now) {
            Resolution::Emit { marker, .. } => {
                assert!(
                    marker.truncated,
                    "SubscribeAck.truncated propagates to the marker"
                );
                assert!(!marker.failed);
            }
            other => panic!("should be a success marker: {other:?}"),
        }
    }

    // ── marker encoding spec ──────────────────────────────────────────────────────────────
    #[test]
    fn marker_frame_layout_is_fixed() {
        let a = aid(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10);
        let marker = Marker {
            generation: 0x1122_3344_5566_7788,
            truncated: true,
            failed: false,
            continues_conversation: false,
            replay_from: 0x99aa_bbcc_ddee_ff01,
        };
        let buf = encode_marker_frame(a, 7, marker);
        assert_eq!(
            MARKER_FRAME_LEN, 38,
            "pairs with the length in the webview's decodeReplayMarker"
        );
        assert_eq!(buf.len(), MARKER_FRAME_LEN, "38 bytes");
        assert_eq!(buf[0], MARKER_TAG, "tag=255");
        assert_eq!(&buf[1..17], a.as_bytes(), "agentId 16 bytes");
        assert_eq!(&buf[17..21], &7u32.to_be_bytes(), "epoch BE");
        assert_eq!(
            &buf[21..29],
            &0x1122_3344_5566_7788u64.to_be_bytes(),
            "gen BE (FIX-4 — whole frame unified to BE)"
        );
        assert_eq!(buf[29], 0b0000_0001, "flags: truncated=bit0");
        assert_eq!(
            &buf[30..38],
            &0x99aa_bbcc_ddee_ff01u64.to_be_bytes(),
            "replay_from BE — last position after flags (ADR-0231)"
        );
    }

    #[test]
    fn marker_failed_flag_encodes_bit1() {
        let a = aid(1);
        let buf = encode_marker_frame(
            a,
            0,
            Marker {
                generation: 1,
                truncated: false,
                failed: true,
                continues_conversation: false,
                replay_from: 0,
            },
        );
        assert_eq!(buf[29], 0b0000_0010, "flags: failed=bit1");
    }

    // ── resume marker (ADR-0226) — from Ack to success marker bit2 ─────────────────────────
    #[test]
    fn continues_conversation_encodes_bit2_on_success_and_is_cleared_on_failure() {
        let a = aid(1);
        let success = encode_marker_frame(
            a,
            0,
            Marker {
                generation: 1,
                truncated: false,
                failed: false,
                continues_conversation: true,
                replay_from: 0,
            },
        );
        assert_eq!(success.len(), MARKER_FRAME_LEN, "length stays 38");
        assert_eq!(
            success[29], 0b0000_0100,
            "flags: continues_conversation=bit2"
        );
        let failure = encode_marker_frame(
            a,
            0,
            Marker {
                generation: 1,
                truncated: true,
                failed: true,
                continues_conversation: true,
                replay_from: 0,
            },
        );
        assert_eq!(failure.len(), MARKER_FRAME_LEN, "length stays 38");
        assert_eq!(
            failure[29], 0b0000_0011,
            "a failure marker has bit2 = 0 even when the input is true — bit0 and bit1 stay as they are"
        );
    }

    #[test]
    fn continues_conversation_flag_propagates_to_success_marker() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now);
        fs.on_ack(a, false, true, 0, now);
        match fs.on_complete(a, now) {
            Resolution::Emit { marker, .. } => {
                assert!(
                    marker.continues_conversation,
                    "the Ack's flag goes into the success marker"
                );
                assert!(
                    !marker.truncated,
                    "does not leak into the neighboring field"
                );
                assert!(!marker.failed);
            }
            other => panic!("should be a success marker: {other:?}"),
        }
    }

    // A failure marker does not carry the resume marker even if the Ack said true, even when expiry comes after the Ack; the later Complete's success marker carries that same Ack's resume marker.
    #[test]
    fn a_deadline_failure_marker_never_carries_continues_conversation() {
        let mut fs = ReplayFlightSet::new(Duration::from_millis(100));
        let a = aid(1);
        let start = t0();
        fs.request_replay(a, start);
        fs.on_ack(a, false, true, 0, start);
        let expired = fs.check_deadlines(start + Duration::from_millis(200));
        assert_eq!(expired.len(), 1);
        assert!(expired[0].1.failed);
        assert!(
            !expired[0].1.continues_conversation,
            "a failure marker has flag 0"
        );
        match fs.on_complete(a, start + Duration::from_millis(300)) {
            Resolution::Emit { marker, .. } => {
                assert!(!marker.failed);
                assert!(
                    marker.continues_conversation,
                    "a late success marker carries the Ack's flag"
                );
            }
            other => panic!("a late Complete is a success marker: {other:?}"),
        }
    }

    // If expiry is **before** the Ack (zombie), the resume marker comes from the late Ack — failure marker 0, late success marker true.
    #[test]
    fn a_zombies_late_ack_still_lands_its_flag_on_the_success_marker() {
        let mut fs = ReplayFlightSet::new(Duration::from_millis(100));
        let a = aid(1);
        let start = t0();
        fs.request_replay(a, start);
        let expired = fs.check_deadlines(start + Duration::from_millis(200));
        assert!(
            !expired[0].1.continues_conversation,
            "expiry before Ack = flag 0"
        );
        fs.on_ack(a, false, true, 0, start + Duration::from_millis(300));
        assert!(matches!(
            fs.on_complete(a, start + Duration::from_millis(400)),
            Resolution::Emit {
                marker: Marker {
                    failed: false,
                    continues_conversation: true,
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn a_refusal_marker_never_carries_continues_conversation() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now);
        match fs.on_refused(a, now) {
            RefusalOutcome::Released { marker, .. } => {
                let m = marker.expect("the first refusal is a failure marker");
                assert!(m.failed);
                assert!(!m.continues_conversation, "a refusal marker has flag 0");
            }
            other => panic!("should be a release: {other:?}"),
        }
    }

    // The resume marker is each generation's own Ack value: even if the previous generation's Ack was true, the success marker of a later merged generation carries its own Ack's (false). This catches the previous generation's true sticking and carrying over — a false Ack not overwriting the field (OR accumulation · write only when true) with the field surviving across generations (missing reset · one field per agent). Either alone does not leak: every success marker is preceded by its own Ack, so correct overwriting never leaks wherever the field lives, and building it fresh per generation leaves nothing to accumulate.
    #[test]
    fn the_next_generation_does_not_inherit_the_previous_acks_flag() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now); // gen1
        fs.request_replay(a, now); // gen2 merged
        fs.on_ack(a, false, true, 0, now);
        match fs.on_complete(a, now) {
            Resolution::Emit { marker, send_next } => {
                assert_eq!(marker.generation, 1);
                assert!(marker.continues_conversation);
                assert!(send_next, "gen2 goes out now");
            }
            other => panic!("should be a success marker: {other:?}"),
        }
        fs.on_ack(a, false, false, 0, now);
        match fs.on_complete(a, now) {
            Resolution::Emit { marker, .. } => {
                assert_eq!(marker.generation, 2);
                assert!(
                    !marker.continues_conversation,
                    "gen2 carries the value of its own Ack (false)"
                );
            }
            other => panic!("should be a success marker: {other:?}"),
        }
    }

    // ── replay head (ADR-0231) — from Ack to success marker ───────────────────────────────
    #[test]
    fn replay_from_propagates_from_the_ack_to_the_success_marker() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now);
        fs.on_ack(a, true, false, 40, now);
        match fs.on_complete(a, now) {
            Resolution::Emit { marker, .. } => {
                assert_eq!(
                    marker.replay_from, 40,
                    "the Ack's replay_from goes into the success marker"
                );
                assert!(marker.truncated, "the neighboring field stays as it is");
            }
            other => panic!("should be a success marker: {other:?}"),
        }
    }

    // The head 0 of an empty replay goes through as is — a new incarnation (empty ring) receives head 0, and the view starts its flush with it and catches the seq 0 arriving late after the marker. 0 is not special-cased as "unknown" (equal to the pre-Ack default, but a success marker always stands only after an Ack).
    #[test]
    fn an_empty_replays_head_zero_reaches_the_marker_bytes() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now);
        fs.on_ack(a, false, false, 0, now);
        let Resolution::Emit { marker, .. } = fs.on_complete(a, now) else {
            panic!("should be a success marker");
        };
        let buf = encode_marker_frame(a, 3, marker);
        assert_eq!(&buf[30..38], &0u64.to_be_bytes());
    }

    // The head is each generation's own Ack value — a generation merged and sent afterward does not inherit the previous generation's head.
    #[test]
    fn the_next_generation_carries_its_own_acks_replay_from() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now); // gen1
        fs.request_replay(a, now); // gen2 merged
        fs.on_ack(a, false, false, 40, now);
        let Resolution::Emit { marker, send_next } = fs.on_complete(a, now) else {
            panic!("gen1 should be a success marker");
        };
        assert_eq!((marker.generation, marker.replay_from), (1, 40));
        assert!(send_next);
        fs.on_ack(a, false, false, 55, now);
        let Resolution::Emit { marker, .. } = fs.on_complete(a, now) else {
            panic!("gen2 should be a success marker");
        };
        assert_eq!((marker.generation, marker.replay_from), (2, 55));
    }
}
