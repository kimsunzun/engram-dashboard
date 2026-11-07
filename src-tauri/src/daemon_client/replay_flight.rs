//! Single-flight replay numbering/fence state machine + replay boundary marker encoding (ADR-0046 M1).
//!
//! ## What / why (load-bearing)
//! src-tauri dropped the mirror buffer (ADR-0046) and became a stateless pass-through router. A remount/reload/new window gets the daemon ring
//! **fully replayed again**, and so that the view can know the boundary (end) of that replay, a **single-flight per agent** maps the
//! wire `Subscribe`↔`ReplayComplete` 1:1 and stamps a `gen` (generation). The view flushes with sort+dedup only on a success marker whose gen is **at least** what its own `requestReplay`
//! got back (gen fence — blocks early flush by someone else's / an old generation's replay).
//!
//! ## ★No count-based coupling (re-verified in practice)★
//! Inferring gen↔Complete by counting/last-value stamping desyncs (last value = early flush loses data, FIFO = a failure path
//! that misses Complete desyncs permanently). So **exactly 1 concurrent in-flight** is enforced — every request that arrives
//! during an in-flight is merged (coalesced) into "one next Subscribe", and that Subscribe is sent when the current in-flight resolves.
//!
//! ## ★Lifetime · invariants (TRD rev4 §2 + FIX round — zombie semantics)★
//! - in_flight lifetime = sent → (SubscribeAck) → acked → (ReplayComplete) → success marker. **There are only three
//!   release paths for the slot — resolution (Complete after Ack) · refusal ([`ReplayFlightSet::on_refused`]) · disconnect.** Exceeding the
//!   deadline does not release the slot.
//! - A success marker may follow a failure marker on the same gen safely: a view that re-requested holds a higher myGen and
//!   ignores it, and a view still waiting flushes the complete buffer (the view keeps its buffer on a failure marker). Under the progress-based
//!   deadline a late resolution implies an empty/near-empty replay, and the frames that flowed were already buffered by the view.
//! - **agent-gone is no longer a zombie:** the daemon emits that refusal as `AgentEvent::SubscribeFailed`
//!   ([`ReplayFlightSet::on_refused`]) and the slot is released on the spot. The old wiring had only an `Error` with no field
//!   to identify the owner, so the client did not know which slot to release, and it was written here as an "accepted limitation" (do not revive it).
//! - **One shape where a zombie still remains:** the daemon gives only `Ack` and afterwards never sends `ReplayComplete` or a refusal
//!   (a response may still come, so expiry cannot release the slot). That slot stays until disconnect. The failure marker had
//!   already gone out at the first expiry, and the UX is handled by the view's bounded re-request ladder (accepted). ★The branch where the daemon
//!   **drops `Complete` because the queue is saturated** is not included here★ — there the daemon closes the connection on the spot and it
//!   ends in a disconnect (the ReplayComplete enqueue failure branch of the daemon's `handle_subscribe`).
//!
//! ## ★Single outstanding — a premise kept outside this file (load-bearing)★
//! [`ReplayFlightSet::on_ack`] · [`ReplayFlightSet::on_complete`] · [`ReplayFlightSet::on_refused`] do **not check** the generation of the
//! arriving response (the wire has no generation field — the daemon does not know gen). Instead they rely on the premise that "at most one Subscribe of that agent
//! is on the wire and it is exactly the current `in_flight`". So this file
//! **has no entry point that produces a Subscribe that went out on the wire with its tracking lost** — if even one such function exists,
//! an old generation's Ack/Complete/refusal gets misstamped onto the new generation's slot and a success marker is attached to a gen whose replay never ran
//! (gen fence collapse). The matching discipline on the caller side is the replay Subscribe send-failure
//! handling in the sibling module `connection.rs` (it drops the connection).
//!
//! ## ★Purity ① test isolation — zero socket · tokio · Tauri dependencies (ADR-0012 · ADR-0175 decision 3)★
//! It lives inside the shell package but touches none of its wiring. Time is taken as an `Instant` argument so deterministic unit
//! tests stand (side effects = actually sending the marker · the wire Subscribe are done by the caller = the connection task).
//!
//! ## ★Purity ② ignorant of the wire contract — zero protocol dependencies (ADR-0046 decisions 3·4)★
//! The boundary marker tag 255 that this module stamps is a **shell↔webview Channel-internal contract** absent from the daemon codec (see
//! `MARKER_TAG` below), and the only state ADR-0046 decision 3 allows the shell is request bookkeeping (epoch · single-flight). Once protocol is
//! brought in, a later session starts using wire types here and this module ends up knowing a contract it takes no part in.
//! ★It is not "because that crate is heavy"★ — protocol is a pure data crate with no runtime, so its value for the determinism of ① is 0. That is,
//! explaining this clause as a reason of ① is wrong.
//!
//! ## ★Keeping the two above — a gate, not prose★
//! `.github/workflows/ci.yml`'s `Gate: replay_flight purity — no transport/wire ingress` scans this one file
//! and FAILs if a `tauri` · `tokio` · `std::net` · `engram_dashboard_*` path appears **outside comments**. It catches not only
//! `use` lines but also inline full paths, and the shape with spaces around `::` (`tokio :: spawn`) — because, unlike the sibling-crate isolation gate, there is **no compiler backstop**
//! (the shell package depends on all three, so a single `tokio::spawn(..)` line just compiles).
//! The gate's authority (expected values · known limits) = that step's comment in `.github/workflows/ci.yml`
//! (ADR-0175 §영향).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use uuid::Uuid;

/// Agent identifier. It is the same `uuid::Uuid` as `engram_dashboard_protocol::AgentId`, so it passes straight through at call sites
/// — ★a duplicate kept knowingly★. Switching to that alias would require importing protocol, and the moment that happens this file
/// knows the wire contract (「순수성 ②」 (Purity ②) above). It is not a cost matter — that crate has no runtime.
type AgentId = Uuid;

/// Wire tag of the replay boundary marker. A **src-tauri↔webview Channel-internal contract** value absent from the daemon codec (tag0/tag1) —
/// the frontend decodeOutputFrame quietly skips it as an unknown tag (forward compatible, M0) and M2 consumes it formally.
pub const MARKER_TAG: u8 = 255;

/// Total marker frame length = tag(1) + agentId(16) + epoch(4) + gen(8) + flags(1) + replay_from(8) = 38 bytes.
/// ★The shell and webview ship in one build, so the webview does not accept a marker shorter than this★ (ADR-0231 — including the old 30 bytes).
pub const MARKER_FRAME_LEN: usize = 1 + 16 + 4 + 8 + 1 + 8;

/// Logical content of the replay boundary marker (gen fence + flags). agentId · epoch are attached at encode time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    pub generation: u64,
    /// The daemon cut the past beyond the ring lower bound (propagates SubscribeAck.truncated) — the view surfaces a warning.
    pub truncated: bool,
    /// Failure termination (progress-based deadline exceeded) — the view must not flush, re-request ladder (M2).
    pub failed: bool,
    /// The incarnation of this replay came up to resume a stored conversation (propagates SubscribeAck.continues_conversation —
    /// not whether the resume succeeded). ★Can be true only on a success marker★ — the two places that build a failure marker
    /// ([`ReplayFlightSet::check_deadlines`] · [`ReplayFlightSet::on_refused`]) set it to `false`,
    /// and [`encode_marker_frame`] also clears bit2 of a failure marker. On a failure the Ack may not have come, so it is a spot with no value.
    /// The view-side contract (ADR-0226 A4): this bit is read only on a success flush.
    pub continues_conversation: bool,
    /// The head of this replay — that generation's `SubscribeAck.replay_from` (the seq the daemon actually sent first · for an empty ring the
    /// next seq to be issued). The view starts the success flush at `max(마지막+1, replay_from)` (ADR-0231).
    /// ★Received because it cannot be replaced by the buffer's minimum seq★ — it is wrong when the replay is empty. Before the Ack it is 0, and on a failure marker
    /// it is meaningless (the view does not flush).
    pub replay_from: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestOutcome {
    /// The generation assigned to this request — the caller returns it to the frontend.
    pub generation: u64,
    /// `true` = send the wire `Subscribe{after_seq:None}` now (idle, so fire immediately).
    /// `false` = merged because an in-flight is running (the Subscribe fires through [`Resolution::send_next`] when the current in-flight resolves).
    pub send_now: bool,
}

/// [`ReplayFlightSet::on_refused`] 결정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalOutcome {
    /// No pool slot — already cleaned up by a disconnect, or a refusal of a subscription we never issued (stray).
    Ignore,
    /// Slot release done.
    Released {
        /// Failure marker. `None` if it was already emitted once as a zombie (no duplicate emission — it would push the view's re-request ladder twice).
        marker: Option<Marker>,
        /// `true` = send the merged next request's Subscribe now.
        send_next: bool,
    },
}

/// [`ReplayFlightSet::on_complete`] 결정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// A Complete that arrived before Ack (orphan of the previous generation) or a stray with no in_flight.
    Ignore,
    Emit {
        marker: Marker,
        /// `true` = send the merged next request's Subscribe now (the current in-flight has been replaced with that gen).
        send_next: bool,
    },
}

/// Single-flight bookkeeping for one agent. `gen_counter` is never reset (monotonic — prevents mistaking an old-generation marker).
#[derive(Default)]
struct AgentFlight {
    gen_counter: u64,
    in_flight: Option<InFlight>,
    /// The gen of the "next one Subscribe" shared by the requests that arrived during an in-flight. `Some` ⟹ in_flight `Some` (invariant).
    next_gen: Option<u64>,
}

struct InFlight {
    generation: u64,
    acked: bool,
    truncated: bool,
    continues_conversation: bool,
    replay_from: u64,
    /// ★Zombie flag (FIX-1)★: the failure marker has already been emitted once (a mark against re-emission). The rationale is
    ///   [`ReplayFlightSet::check_deadlines`].
    failed: bool,
    /// Progress-based deadline — updated by every frame/Ack.
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

/// The connection task (actor) owns it by `&mut` and operates it serially — no internal lock.
pub struct ReplayFlightSet {
    agents: HashMap<AgentId, AgentFlight>,
    deadline: Duration,
}

impl ReplayFlightSet {
    /// `deadline` = progress-based no-progress limit (around 10s in production).
    pub fn new(deadline: Duration) -> Self {
        Self {
            agents: HashMap::new(),
            deadline,
        }
    }

    /// Entry point of the contract that always terminates with exactly 1 marker.
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
            // ★Coalesce safety condition★: a request always arrives *after* the view's SubState registration, so merging it into a Subscribe fired later
            //   is safe (the rev2 safety condition is met by this structure). N views remounting at once ≤ 2 replays.
            //   ★A zombie slot is merged here too★ — when the zombie is released by a late Complete/disconnect this next_gen goes out.
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

    // ★There is deliberately no send-failure rollback entry point (do not revive it)★: the old `abort_in_flight` only emptied the slot when the wire send of the send_now
    //   Subscribe failed and **kept polling the connection**. That combination puts this file's single
    //   outstanding premise (module header) on top of an assumption about the caller — it is safe only if "send failure = the peer did not receive it"
    //   is true, and that is the error semantics of tungstenite/TCP, not a property we enforce. Instead the
    //   caller **drops the connection** on the spot (the `connection.rs` Subscribe(replay) send-failure branch) and
    //   [`Self::on_disconnect`] cleans everything up — it erases more broadly than a rollback, but it makes the premise into code.
    //   One sanctioned exception (the connection driver's `write_follow_up`): a merged request promoted by
    //   `on_complete` / `on_refused` whose agent no window shows any more is released with `on_refused` *before*
    //   its Subscribe is attempted — it never reached the wire, so the single-outstanding premise holds.

    /// Being single-flight, an arriving Ack is always that of the one outstanding in-flight (zombie included — a late Ack
    /// of an expired generation also points at that slot). So it stamps onto the current slot as is, with no gen comparison.
    /// `replay_from` = the replay head of that Ack — carried by this generation's success marker ([`Marker::replay_from`]).
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

    /// Progress signal of that agent such as a binary frame — resets the deadline (no false positive for a healthy-slow replay).
    pub fn note_progress(&mut self, agent: AgentId, now: Instant) {
        let deadline = self.deadline;
        if let Some(f) = self.agents.get_mut(&agent) {
            if let Some(inf) = f.in_flight.as_mut() {
                inf.deadline = now + deadline;
            }
        }
    }

    /// Stamps a success marker **only on an in_flight in acked state**. The wire order is `[Ack_k]…[Complete_k][Ack_{k+1}]`,
    /// so a Complete that arrives before Ack is provably an orphan of the previous generation → Ignore (misattribution blocked at the source).
    /// `now` = deadline baseline of the new in_flight.
    ///
    /// ★A zombie is released here too (FIX-1)★: a slot that became a zombie (`failed=true`) through the deadline also passes the acked gate
    /// on a late Ack→late Complete and is released with a **success marker of the same gen** (evidence that the replay actually completed).
    pub fn on_complete(&mut self, agent: AgentId, now: Instant) -> Resolution {
        let deadline = self.deadline;
        let Some(f) = self.agents.get_mut(&agent) else {
            return Resolution::Ignore;
        };
        let acked = matches!(&f.in_flight, Some(inf) if inf.acked);
        if !acked {
            return Resolution::Ignore;
        }
        let inf = f.in_flight.take().expect("acked 이면 in_flight 는 Some");
        // ★Same-gen success marker★: whether it was a zombie (late Complete) or normal, replay completed = success (failed:false).
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

    /// ★Progress-based deadline sweep (FIX-1 — zombie semantics)★. An in-flight that expired with no progress is **emitted once as a failure
    /// marker** and marked as a zombie (`failed=true`), but **the slot is kept and the queue is not advanced**. Returns:
    /// a list of `(agent, 실패 마커)` (the caller sends only the markers — the next Subscribe never goes out from here).
    ///
    /// ★Why the queue is not advanced (found by the cross-family reviewer — load-bearing)★: if a timeout moved straight to the next Subscribe and
    /// replaced the slot, then when a *late* Ack/Complete of the expired generation arrives it would be misstamped onto the new in-flight and a
    /// **success marker would be attached to a new gen whose replay has not run yet** (gen fence collapse). If the zombie holds the slot,
    /// an Ack/Complete can structurally point only at the one outstanding Subscribe (= this zombie), so misattribution is impossible.
    /// The slot is released only by resolution (late Complete) · **refusal** ([`Self::on_refused`]) · disconnect.
    ///
    /// ★Why a refusal may release and a timeout may not (the rationale that resolves the hazard above)★: the difference between the two is
    /// **whether a response can still come**, and nothing else. A timeout is merely the observation "it has been quiet for 10 seconds", so a late Ack/Complete
    /// can still come, which is why the slot must be held to block misattribution structurally. A refusal
    /// (`AgentEvent::SubscribeFailed`), on the other hand, is **a notice that the daemon has finished by deciding not to process that Subscribe**,
    /// and after it neither Ack nor Complete is emitted for that subscription (the daemon's `handle_subscribe` returns early before emitting Ack — its failure is only
    /// the `get_session` lookup failure, so it is structurally before `on_ready`).
    /// Since there is no response left that could be misattributed, releasing at once is safe.
    /// Because of this difference we **did not choose** the easy variant "release the expired zombie" (measured 2026-08-19).
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

    /// Whether a replay still waits for its deadline — one [`Self::check_deadlines`] could still fail. With none, no
    /// sweep is needed.
    pub fn awaits_deadline(&self) -> bool {
        self.agents
            .values()
            .any(|f| f.in_flight.as_ref().is_some_and(|inf| !inf.failed))
    }

    /// The daemon **refused** this agent's `Subscribe` (`AgentEvent::SubscribeFailed`). It releases the slot, and
    /// if the failure marker has not been emitted yet it emits it now (so the view does not wait for the 10-second deadline), and advances the merged queue.
    ///
    /// ★Why the release is safe★ = the paragraph "why a refusal may release" of [`Self::check_deadlines`] (the rationale that resolves
    /// that hazard is there). Summary: a refused subscription has no Ack/Complete to follow, so no response can be misattributed.
    ///
    /// ★Why the generation is not compared — the rationale that "the slot being released is the very slot that was refused"★: the wire refusal
    /// (`AgentEvent::SubscribeFailed`) has no generation field (the daemon does not know gen). Even so, releasing the current `in_flight`
    /// is fine because **that agent has at most one Subscribe on the wire** — the module header's single
    /// outstanding premise. A new Subscribe goes out only via (a) an idle request · (b) the `send_next` of [`Self::on_complete`] / this function,
    /// and both go out **after the previous generation has been resolved**. So the state "a refusal arrived but its target is an already
    /// resolved old generation and the current slot is another generation" cannot hold. ★The only shape that breaks this premise =
    /// a Subscribe left on the wire with its tracking lost★, and no such entry point is created (the comment "there is deliberately no
    /// send-failure rollback entry point" above).
    ///
    /// ★An acked slot is not released (defense)★: a subscription that received an Ack was **accepted** by the daemon, so it cannot
    /// coexist with a refusal. The reason to block it anyway = if it were released, the `ReplayComplete` that follows would meet an empty slot and
    /// fall to `Ignore` at the acked gate of [`Self::on_complete`], and the view waiting for that replay would never
    /// receive the success marker. It is a path where one misfired refusal **kills a normal replay**, so it is closed cheaply.
    ///
    /// ★Releasing is right even when the refusal reason is transient★: this function does not decide "is a retry worthwhile" — the re-request
    /// driver is the view's bounded ladder. Releasing only makes that re-request **able to go out on the wire**, and holding
    /// the slot would turn even a transient failure into a permanent cutoff (the actual shape of this defect).
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
        let inf = f.in_flight.take().expect("바로 위에서 Some 확인");
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

    /// Connection lost — internal clear only, **no marker is emitted** (the re-request driver is the frontend connected transition alone).
    /// A zombie left until disconnect due to agent-gone is also finally cleaned here.
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

/// Encode a replay boundary marker into wire frame bytes (Channel-internal contract, ADR-0046). Layout (M2 parser contract ·
/// a pair with the webview `decodeReplayMarker`):
/// `[tag=255:1][agentId:16][epoch:4 BE][gen:8 BE][flags:1][replay_from:8 BE]` — fixed 38 bytes.
///
/// ★Endianness (FIX-4 — the whole marker frame unified to BE)★: agentId is RFC4122 network order (same form as the frame header), epoch · gen ·
/// replay_from are all **big-endian** (the same convention as the binary frame header being uniformly BE — so the M2 parser
/// reads with one convention). `replay_from` is attached at the very end after flags (ADR-0231) — the earlier field offsets stay put, so old readers do not shift.
/// flags bit0=truncated, bit1=failed, bit2=continues_conversation.
/// ★bit2 is always 0 on a failure marker (bit1) — even if the input `Marker` carries true, it is cleared here (ADR-0226)★.
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
        assert_eq!(out.generation, 1, "첫 gen=1");
        assert!(out.send_now, "idle 이면 즉시 Subscribe 송신");
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
                assert!(!send_next, "대기열 없으면 다음 Subscribe 없음");
            }
            other => panic!("성공 마커여야: {other:?}"),
        }
        assert_eq!(fs.request_replay(a, now).generation, 2, "gen 단조(리셋 0)");
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
        assert_eq!(w1.generation, 2, "첫 대기자 = 다음 gen 2");
        assert_eq!(w2.generation, 2, "둘째 대기자 = 같은 다음 gen 2(공유)");
        assert!(
            !w1.send_now && !w2.send_now,
            "in-flight 중이라 즉시 발사 안 함"
        );
        fs.on_ack(a, false, false, 0, now);
        match fs.on_complete(a, now) {
            Resolution::Emit { marker, send_next } => {
                assert_eq!(marker.generation, 1, "해소되는 건 현 in-flight(gen1)");
                assert!(send_next, "병합된 대기열 → 다음 Subscribe 1회 발사");
            }
            other => panic!("성공 마커여야: {other:?}"),
        }
        fs.on_ack(a, false, false, 0, now);
        match fs.on_complete(a, now) {
            Resolution::Emit { marker, send_next } => {
                assert_eq!(marker.generation, 2);
                assert!(!send_next, "대기열 소진 — 더는 Subscribe 없음");
            }
            other => panic!("성공 마커여야: {other:?}"),
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
            "Ack 전 Complete 무시"
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
            "in-flight 없으면 stray Complete 무시"
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
        assert_eq!((w.generation, w.send_now), (2, false), "gen2 대기열 병합");
        let expired = fs.check_deadlines(start + Duration::from_millis(200));
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].0, a);
        assert_eq!(expired[0].1.generation, 1, "실패 마커 gen1");
        assert!(expired[0].1.failed, "무진행 초과 = 실패 마커");
        assert!(
            fs.check_deadlines(start + Duration::from_millis(300))
                .is_empty(),
            "좀비는 실패 마커 재발행 없음"
        );
        fs.on_ack(a, false, false, 0, start + Duration::from_millis(400));
        match fs.on_complete(a, start + Duration::from_millis(500)) {
            Resolution::Emit { marker, send_next } => {
                assert_eq!(marker.generation, 1, "★성공 마커는 gen1 — gen2 아님★");
                assert!(!marker.failed, "late Complete = 성공(replay 실제 완료)");
                assert!(send_next, "이제서야 병합된 gen2 Subscribe 발사");
            }
            other => panic!("late Complete 는 성공 마커: {other:?}"),
        }
        fs.on_ack(a, false, false, 0, start + Duration::from_millis(600));
        match fs.on_complete(a, start + Duration::from_millis(700)) {
            Resolution::Emit { marker, send_next } => {
                assert_eq!(marker.generation, 2);
                assert!(!marker.failed);
                assert!(!send_next, "대기열 소진");
            }
            other => panic!("성공 마커여야: {other:?}"),
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
                                     // agent-gone scenario: afterwards neither Ack nor Complete ever comes.
        let expired = fs.check_deadlines(start + Duration::from_millis(200));
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].1.generation, 1);
        assert!(expired[0].1.failed);
        let w = fs.request_replay(a, start + Duration::from_millis(300));
        assert!(!w.send_now, "좀비가 슬롯 점유 → 새 요청 병합(즉시 발사 X)");
        fs.on_disconnect();
        let after = fs.request_replay(a, start + Duration::from_millis(400));
        assert_eq!(after.generation, 3, "gen_counter 단조(좀비 후에도 리셋 0)");
        assert!(after.send_now, "disconnect 로 슬롯 비어 즉시 발사");
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
        assert!(marker.failed, "무진행 초과 = 실패 마커");
        assert!(
            !fs.request_replay(a, start + Duration::from_millis(250))
                .send_now,
            "좀비 슬롯 점유로 새 요청은 병합"
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
            assert!(fs.check_deadlines(now).is_empty(), "진행 중이면 만료 0");
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
    // ★Regression target (measured 2026-08-19)★: right after a daemon restart there are no sessions (boot auto-restore OFF), so the reconnect replay's
    //   Subscribe is refused as "agent not found". A refusal has no Ack/Complete, so the slot stayed a zombie, and
    //   after that **that agent's Subscribe could never go out again** (every request merged), so even after a re-spawn output was
    //   permanently cut off in all windows. The first case below breaks exactly that "never again".
    #[test]
    fn refusal_releases_slot_so_next_request_sends_again() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        let first = fs.request_replay(a, now);
        assert_eq!((first.generation, first.send_now), (1, true));

        match fs.on_refused(a, now) {
            RefusalOutcome::Released { marker, send_next } => {
                let m = marker.expect("첫 거절은 실패 마커를 낸다(뷰가 10초를 안 기다리게)");
                assert_eq!(m.generation, 1);
                assert!(m.failed);
                assert!(!send_next, "대기열이 없으면 다음 Subscribe 없음");
            }
            other => panic!("거절은 슬롯을 해제해야: {other:?}"),
        }

        // ★This assertion is the heart of the defect★ — before the fix send_now=false repeated forever.
        let next = fs.request_replay(a, now);
        assert_eq!(next.generation, 2, "gen 단조");
        assert!(next.send_now, "해제됐으니 새 Subscribe 가 즉시 나간다");
    }

    #[test]
    fn refusal_advances_coalesced_waiter() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now); // gen1 in-flight
        let w = fs.request_replay(a, now); // gen2 병합
        assert_eq!((w.generation, w.send_now), (2, false));

        match fs.on_refused(a, now) {
            RefusalOutcome::Released { marker, send_next } => {
                assert_eq!(marker.expect("실패 마커").generation, 1, "해제되는 건 gen1");
                assert!(send_next, "병합된 gen2 Subscribe 를 지금 발사");
            }
            other => panic!("해제여야: {other:?}"),
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

    // ★Pinning the single-outstanding premise★: a refusal has no generation field, so "what is released now is the very generation just refused" holds
    //   only by order — consecutive refusals each yield the generation **that was on the wire at that time**. If this goes wrong the gen fence
    //   collapses (module header · on_refused paragraph).
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
                assert_eq!(marker.expect("실패 마커").generation, 1, "첫 거절 = gen1");
                assert!(send_next, "이제 gen2 가 wire 로 나간다");
            }
            other => panic!("해제여야: {other:?}"),
        }
        // The second refusal arrives — at that point gen2 is the only outstanding one.
        match fs.on_refused(a, now) {
            RefusalOutcome::Released { marker, send_next } => {
                assert_eq!(marker.expect("실패 마커").generation, 2, "둘째 거절 = gen2");
                assert!(!send_next, "대기열 소진");
            }
            other => panic!("해제여야: {other:?}"),
        }
        assert!(
            fs.request_replay(a, now).send_now,
            "슬롯이 비어 재발사 가능"
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
        assert_eq!(expired.len(), 1, "만료 실패 마커 1회");

        match fs.on_refused(a, start + Duration::from_millis(300)) {
            RefusalOutcome::Released { marker, send_next } => {
                assert!(
                    marker.is_none(),
                    "이미 실패 마커가 나간 gen — 중복 발행 금지"
                );
                assert!(!send_next);
            }
            other => panic!("해제여야: {other:?}"),
        }
        assert!(
            fs.request_replay(a, start + Duration::from_millis(400))
                .send_now,
            "좀비도 거절로 풀린다"
        );
    }

    // ★Defense★: a slot that received an Ack (= accepted by the daemon) is not released by a refusal. If released, the following
    //   ReplayComplete would meet an empty slot and fall to Ignore, and that replay's success marker would never go out.
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
            "acked 슬롯은 거절로 안 풀린다"
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
        assert_eq!(fs.on_refused(a, now), RefusalOutcome::Ignore, "미지 agent");
        fs.request_replay(a, now);
        fs.on_refused(a, now);
        assert_eq!(
            fs.on_refused(a, now),
            RefusalOutcome::Ignore,
            "이미 해제된 슬롯의 두 번째 거절은 무시(중복 마커·오해제 금지)"
        );
    }

    // ── send failure = cleaned by disconnect (replaces the old abort_in_flight rollback) ────────────────────────
    // ★Regression target★: the old code only emptied the slot on a wire send failure and kept polling the connection — if a Subscribe that lost tracking
    //   can remain on the wire, a late refusal/Ack is misstamped onto the new-generation slot (gen fence collapse). Now the caller
    //   drops the connection on the spot and this path cleans everything up. The result that a re-request must be able to go out again is the same.
    #[test]
    fn send_failure_path_clears_via_disconnect_and_next_request_sends() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        let first = fs.request_replay(a, now);
        assert_eq!((first.generation, first.send_now), (1, true));
        fs.on_disconnect(); // Send failure → the caller drops the connection.
        let next = fs.request_replay(a, now);
        assert_eq!(next.generation, 2, "gen 단조(1 소진 → 2)");
        assert!(next.send_now, "단절 청소로 슬롯 비어 즉시 재발사");
        fs.on_ack(a, false, false, 0, now);
        assert!(matches!(
            fs.on_complete(a, now),
            Resolution::Emit {
                marker: Marker { generation: 2, .. },
                ..
            }
        ));
    }

    // ── 단절 ──────────────────────────────────────────────────────────────────────────
    #[test]
    fn disconnect_clears_inflight_and_waiters_keeps_counter() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now); // gen1 in-flight
        fs.request_replay(a, now); // gen2 queue
        fs.on_disconnect();
        let out = fs.request_replay(a, now);
        assert_eq!(out.generation, 3, "gen_counter 단조 유지(2까지 소진 → 3)");
        assert!(out.send_now, "단절로 in-flight 비어 즉시 발사");
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
                assert!(marker.truncated, "SubscribeAck.truncated 가 마커로 전파");
                assert!(!marker.failed);
            }
            other => panic!("성공 마커여야: {other:?}"),
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
            "웹뷰 decodeReplayMarker 의 길이와 한 쌍"
        );
        assert_eq!(buf.len(), MARKER_FRAME_LEN, "38바이트");
        assert_eq!(buf[0], MARKER_TAG, "tag=255");
        assert_eq!(&buf[1..17], a.as_bytes(), "agentId 16바이트");
        assert_eq!(&buf[17..21], &7u32.to_be_bytes(), "epoch BE");
        assert_eq!(
            &buf[21..29],
            &0x1122_3344_5566_7788u64.to_be_bytes(),
            "gen BE(FIX-4 — 프레임 전체 BE 통일)"
        );
        assert_eq!(buf[29], 0b0000_0001, "flags: truncated=bit0");
        assert_eq!(
            &buf[30..38],
            &0x99aa_bbcc_ddee_ff01u64.to_be_bytes(),
            "replay_from BE — flags 뒤 끝자리(ADR-0231)"
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
        assert_eq!(success.len(), MARKER_FRAME_LEN, "길이 38 불변");
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
        assert_eq!(failure.len(), MARKER_FRAME_LEN, "길이 38 불변");
        assert_eq!(
            failure[29], 0b0000_0011,
            "실패 마커는 입력이 참이어도 bit2 0 — bit0·bit1 은 그대로"
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
                assert!(marker.continues_conversation, "Ack 의 표식이 성공 마커로");
                assert!(!marker.truncated, "옆 칸으로 새지 않는다");
                assert!(!marker.failed);
            }
            other => panic!("성공 마커여야: {other:?}"),
        }
    }

    // ★A failure marker does not carry the resume marker even if the Ack said true★ — even when expiry comes after the Ack. The success marker of the later
    //   Complete after that carries the same Ack's resume marker.
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
        assert!(!expired[0].1.continues_conversation, "실패 마커는 표식 0");
        match fs.on_complete(a, start + Duration::from_millis(300)) {
            Resolution::Emit { marker, .. } => {
                assert!(!marker.failed);
                assert!(
                    marker.continues_conversation,
                    "늦은 성공 마커는 Ack 의 표식"
                );
            }
            other => panic!("늦은 Complete 는 성공 마커: {other:?}"),
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
        assert!(!expired[0].1.continues_conversation, "Ack 전 만료 = 표식 0");
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
                let m = marker.expect("첫 거절은 실패 마커");
                assert!(m.failed);
                assert!(!m.continues_conversation, "거절 마커는 표식 0");
            }
            other => panic!("해제여야: {other:?}"),
        }
    }

    // ★The resume marker is each generation's own Ack value★ — even if the previous generation's Ack was true, the success marker of a generation that was merged and went out
    //   afterwards carries its own Ack's (false) value. What this catches is the shape where the previous generation's true **sticks** and carries over —
    //   a case where a false Ack does not overwrite the field (OR accumulation · write only when true) and that field survives across generations (missing
    //   reset · one field per agent). Either one alone does not leak — every success marker is preceded by its own Ack,
    //   so as long as overwriting is done right it does not leak wherever the field is placed, and if it is built fresh per generation there is nothing to accumulate.
    #[test]
    fn the_next_generation_does_not_inherit_the_previous_acks_flag() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now); // gen1
        fs.request_replay(a, now); // gen2 병합
        fs.on_ack(a, false, true, 0, now);
        match fs.on_complete(a, now) {
            Resolution::Emit { marker, send_next } => {
                assert_eq!(marker.generation, 1);
                assert!(marker.continues_conversation);
                assert!(send_next, "gen2 가 이제 나간다");
            }
            other => panic!("성공 마커여야: {other:?}"),
        }
        fs.on_ack(a, false, false, 0, now);
        match fs.on_complete(a, now) {
            Resolution::Emit { marker, .. } => {
                assert_eq!(marker.generation, 2);
                assert!(
                    !marker.continues_conversation,
                    "gen2 는 자기 Ack(거짓)의 값을 싣는다"
                );
            }
            other => panic!("성공 마커여야: {other:?}"),
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
                assert_eq!(marker.replay_from, 40, "Ack 의 replay_from 이 성공 마커로");
                assert!(marker.truncated, "옆 칸은 그대로");
            }
            other => panic!("성공 마커여야: {other:?}"),
        }
    }

    // ★The head 0 of an empty replay goes through as is★ — a new incarnation (empty ring) receives head 0, and the view starts its flush with that value
    //   and catches the seq 0 that arrives late after the marker. 0 is not special-cased as 「0 = 모름」 (0 = unknown) (even if it is the same number as the default before Ack,
    //   a success marker always stands only after an Ack).
    #[test]
    fn an_empty_replays_head_zero_reaches_the_marker_bytes() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now);
        fs.on_ack(a, false, false, 0, now);
        let Resolution::Emit { marker, .. } = fs.on_complete(a, now) else {
            panic!("성공 마커여야");
        };
        let buf = encode_marker_frame(a, 3, marker);
        assert_eq!(&buf[30..38], &0u64.to_be_bytes());
    }

    // ★The head is each generation's own Ack value★ — a generation merged and sent afterward does not inherit the previous generation's head.
    #[test]
    fn the_next_generation_carries_its_own_acks_replay_from() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now); // gen1
        fs.request_replay(a, now); // gen2 병합
        fs.on_ack(a, false, false, 40, now);
        let Resolution::Emit { marker, send_next } = fs.on_complete(a, now) else {
            panic!("gen1 성공 마커여야");
        };
        assert_eq!((marker.generation, marker.replay_from), (1, 40));
        assert!(send_next);
        fs.on_ack(a, false, false, 55, now);
        let Resolution::Emit { marker, .. } = fs.on_complete(a, now) else {
            panic!("gen2 성공 마커여야");
        };
        assert_eq!((marker.generation, marker.replay_from), (2, 55));
    }
}
