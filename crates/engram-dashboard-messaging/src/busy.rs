//! busy — recipient turn-state interpretation + idle gate seam (ADR-0104 decision 3, ADR-0113 decision 2).
//!
//! ★Role = **policy** only★: the **fact** "is it mid-turn" is held by the host's shared observation layer (ADR-0113 decision 1); this
//!   module interprets it as a mail value judgment. ① `TurnFacts` — port reading the
//! facts (host adapter) ② `BusyPolicy` — builds the `BusyGate` answer from facts + policy (positive-knowledge-only, the `BUSY_MAX_TURN`
//!   limit) and wakes turns past the limit with a doorbell ③ `BusyGate`/`IdleNotifier` —
//! the door the service asks through and the flush doorbell exit.
//! ★Busy = a user-waiting list exists ∨ stopped after an error ∨ (mid-turn ∧ within the limit) (ADR-0231)★: the first two have no limit —
//!   user input comes first (N6·N8), and mail is not pushed in after a real error (N11);
//! an intended exception to ADR-0104 "going late is better than not going".
//! ★positive-knowledge-only (load-bearing — spec §5 capability fallback)★: an (id, epoch) with **no** observation is **treated as idle**
//!   (inject at once); reading "unknown" as busy would make delivery wait forever for
//! unobservable backends or before observation starts. Busy is true **only with an observed fact**.
//! ★The fact layer is not mutated (ADR-0113 decision 2 — never violate)★: neither the limit decision nor the sweep removes table entries;
//!   the table is shared with other consumers (UI input locking etc.) with different limits,
//! so one consumer erasing an "old" entry would evaporate another's fact. Here: **read and decide by one's own criterion**.
//! ★Observation targets = backends that emit turn signals★: turn events come only from agents with a backend decoder, which exists under
//!   exactly the structured-output capability condition; the proxy rides on the roster entry as
//! `LiveAgent::turn_signal` (ADR-0116 decision 7), and the send path injects at once for `turn_signal == false` **without asking this
//!   gate**. If the proxy breaks (a structured backend with no turn events), add a real
//! capability field (ADR-0104 capability principle).
//! ★Callback discipline (load-bearing)★: `IdleNotifier::notify_idle` is also called on **the host's output pump thread**; its
//!   implementation must do **only a non-blocking notification send** — injection, the messaging lock or
//! a blocking write would stall every agent's output stream behind delivery. The flush worker that receives it does the flush.
//! ★Known unverified (spec §7)★: whether the `result` line of a nested Task subagent leaks as a **parent turn end** signal; if so the
//!   parent could be judged idle mid-turn and injected early (no loss, only timing). Left as a
//! real-harness measurement item.
// ADR-0104, ADR-0110, ADR-0113, ADR-0231

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::PeerId;

/// ★busy limit (fail-open safety valve — round-3 finding 4; **30 minutes**)★: a "mid-turn" observation older than this since the last turn
///   signal is judged idle, as **wreckage of an abnormally ended turn**.
/// ★Why a limit★: the turn-end signal is the only in-band release; if a turn ends abnormally (parse failure, decoder fault, missing end
///   line) every delivery to that recipient is folded at each doorbell and finally expires by TTL
/// (the worst failure mode of messaging); frequency unmeasured (spec §7). ★Why safe★: a misjudgment means **injection mid-turn**, and the
///   claude CLI queues stdin mid-turn and reads it next turn (measured — spec §7), so the worst
/// cost is blurred read time, **no loss**; without the limit all mail waits until TTL (ADR-0104). ★Why 30 min★: at human message rates a
///   30-minute no-output turn is far outside normal (any tool call, delta or usage refreshes the
/// fact's time, so it means output stopped completely with no end signal); shorter cuts normal long turns, longer delays recovery.
///   ★Applies only to turn observation (`in_turn`)★ — not to `inputs_pending` or `last_end_failed`.
pub const BUSY_MAX_TURN: Duration = Duration::from_secs(30 * 60);

/// ★Idle gate lookup seam (ADR-0012)★ — the only door through which MessagingService asks "can this recipient receive mail now"; the
///   service calls only `busy_reason` (delivery decision = `is_some()`, hint = that value).
/// ★Contract★: a pure query — no side effects, no blocking (short locks only); safe to call **while holding** the messaging lock (current
///   sites call outside it, but this keeps future sites deadlock-free).
pub trait BusyGate: Send + Sync {
    fn is_busy(&self, id: PeerId, epoch: u32) -> bool;

    /// The reason if busy, else `None`. ★The service blocks delivery on this `is_some()` and picks the hint by the value★ (it does not
    ///   call `is_busy`). ★`is_busy` must always equal this `is_some()`★ — override both together;
    /// an implementation not knowing the reason answers the default (mid-turn if `is_busy`).
    // ADR-0231
    fn busy_reason(&self, id: PeerId, epoch: u32) -> Option<BusyReason> {
        self.is_busy(id, epoch).then_some(BusyReason::InTurn)
    }
}

/// Why the recipient cannot receive mail — for hint text. If several overlap, the one written higher wins (`BusyPolicy::busy_reason`).
// ADR-0231
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusyReason {
    /// The last turn ended in an error and there is no successful turn since.
    HaltedAfterError,
    /// Mid-turn (a turn judged wreckage by the limit does not count here).
    InTurn,
    /// Not a turn, but the user-waiting list is not empty.
    InputsPending,
}

/// Gate-unwired / unobservable fallback — **always idle** (= inject at once, spec §5 capability fallback).
pub struct AlwaysIdleGate;

impl BusyGate for AlwaysIdleGate {
    fn is_busy(&self, _id: PeerId, _epoch: u32) -> bool {
        false
    }
}

/// Turn observation facts of one incarnation — raw values with no interpretation attached (ADR-0113 decision 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnFact {
    pub in_turn: bool,
    /// Last turn signal time — read only when `in_turn` (limit decision).
    pub last_signal: Instant,
    /// That incarnation's user-waiting list is not empty.
    // ADR-0231
    pub inputs_pending: bool,
    /// That incarnation's last turn end was an error and there is no clean successful end after it (stopped after an error).
    // ADR-0231
    pub last_end_failed: bool,
}

/// ★Turn-fact lookup port (ADR-0110 decision 3, ADR-0113 decision 1)★ — the door through which the kernel reads the host's shared
///   observation layer **without knowing it even as a type**; production implementation = host adapter.
/// ★Read-only is the contract★ (no "erase/mark" — module header). ★Values returned together★: separate lookups let a signal slip in and
///   yield impossible combinations ("mid-turn but time old", "turn ended but error not written").
pub trait TurnFacts: Send + Sync {
    /// The observed value of this (id, epoch). `None` = unobserved (no fact at all).
    fn turn_fact(&self, id: PeerId, epoch: u32) -> Option<TurnFact>;

    /// Everyone currently observed as mid-turn `(id, epoch, last signal time)` — the entry of the limit sweep.
    fn in_turn_snapshot(&self) -> Vec<(PeerId, u32, Instant)>;
}

/// ★Turn-end (idle transition) notification seam★ — where the host reports "this agent's turn ended" and where the limit sweep wakes a
///   stopped turn; production = a non-blocking send to the flush worker channel.
/// ★Contract (load-bearing)★: **non-blocking and non-reentrant** (module header callback discipline). ★Idempotency dependence (accepted)★:
///   a notification goes out on **every turn-end signal**, transition or not — narrowing to
/// "only when the previous state was busy" would drop the notification for a turn ending at once with no assistant event, stranding
///   parking until the next turn; a redundant notification (an empty-queue drain no-op) is chosen over a miss.
/// Channel pressure bound is the host implementation's responsibility.
pub trait IdleNotifier: Send + Sync {
    fn notify_idle(&self, id: PeerId);
}

/// ★BusyPolicy — the gate putting mail policy on top of the facts (C2, ADR-0113 decision 2)★: one made at daemon boot, shared by
///   MessagingService (gate lookup) and the sweep task (limit fail-open).
/// ★The only state is the limit-decision ledger — **policy** state★: facts are beyond the port; since the shared table cannot be erased,
///   "this signal of this incarnation is seen as wreckage" is written down here — both the gate's answer and
/// the "already woken" mark. ★No clock reads★: the limit comparison happens only in `sweep_stale_busy` (takes `now`) and lookup just reads
///   the ledger, so clock injection holds and decision time is pinned to the sweep period (deterministic).
/// ★Lock = this one ledger, notify outside the lock (ADR-0006)★: `notify_idle` is an external call.
pub struct BusyPolicy {
    facts: Arc<dyn TurnFacts>,
    notifier: Arc<dyn IdleNotifier>,
    /// The incarnation **judged wreckage** by exceeding the limit → the last signal time seen then. If the value changes (a new signal)
    ///   the judgment is void — it returns to busy at once without waiting for the sweep.
    stale: Mutex<HashMap<(PeerId, u32), Instant>>,
}

impl BusyPolicy {
    pub fn new(facts: Arc<dyn TurnFacts>, notifier: Arc<dyn IdleNotifier>) -> Self {
        Self {
            facts,
            notifier,
            stale: Mutex::new(HashMap::new()),
        }
    }

    /// ★Limit sweep (round-3 finding 4)★ — **judges wreckage** for "mid-turn" observations past `BUSY_MAX_TURN` and **wakes** the owner
    ///   with a doorbell (waiting parking is delivered then); called by the daemon's periodic sweep task.
    /// Returns the number of ids woken. ★Waking is half the fix★: flipping only the decision leaves parking until the next trigger
    ///   (appearance/turn end/new send) — which by definition does not come here (turn-end signal lost).
    /// ★One doorbell per wreckage★: the fact cannot be erased and is seen every period; last signal time equal to the ledger = already
    ///   handled; a new signal that stops again = new wreckage. ★The ledger is swapped each
    /// time for "what is wreckage now"★: revived/reaped incarnations do not pile up.
    pub fn sweep_stale_busy(&self, now: Instant) -> usize {
        let stale_now: Vec<(PeerId, u32, Instant)> = self
            .facts
            .in_turn_snapshot()
            .into_iter()
            .filter(|(_, _, last)| now.saturating_duration_since(*last) >= BUSY_MAX_TURN)
            .collect();

        let mut to_wake: Vec<PeerId> = Vec::new();
        {
            let mut ledger = self.stale.lock().expect("busy stale ledger poisoned");
            let mut next: HashMap<(PeerId, u32), Instant> = HashMap::new();
            for (id, epoch, last) in stale_now {
                if ledger.get(&(id, epoch)) != Some(&last) && !to_wake.contains(&id) {
                    to_wake.push(id);
                }
                next.insert((id, epoch), last);
            }
            *ledger = next;
        }

        for id in &to_wake {
            tracing::warn!(
                agent = %id,
                max_secs = BUSY_MAX_TURN.as_secs(),
                "busy cap exceeded — deciding an old turn as idle without observing the turn end and ringing the flush doorbell (fail-open safety valve)"
            );
            self.notifier.notify_idle(*id);
        }
        to_wake.len()
    }

    pub fn is_busy(&self, id: PeerId, epoch: u32) -> bool {
        self.busy_reason(id, epoch).is_some()
    }

    /// Produces the busy decision and its reason in one fact lookup — `is_busy` is the `is_some()` of this.
    pub fn busy_reason(&self, id: PeerId, epoch: u32) -> Option<BusyReason> {
        let fact = self.facts.turn_fact(id, epoch)?;
        // ADR-0231: does not look at the limit ledger — these two facts are busy even after 30 minutes (module header).
        if fact.last_end_failed {
            return Some(BusyReason::HaltedAfterError);
        }
        let live_turn = fact.in_turn && {
            let ledger = self.stale.lock().expect("busy stale ledger poisoned");
            ledger.get(&(id, epoch)) != Some(&fact.last_signal)
        };
        if live_turn {
            return Some(BusyReason::InTurn);
        }
        fact.inputs_pending.then_some(BusyReason::InputsPending)
    }
}

impl BusyGate for BusyPolicy {
    fn is_busy(&self, id: PeerId, epoch: u32) -> bool {
        BusyPolicy::is_busy(self, id, epoch)
    }

    fn busy_reason(&self, id: PeerId, epoch: u32) -> Option<BusyReason> {
        BusyPolicy::busy_reason(self, id, epoch)
    }
}

/// ★Harness/test-only fact source★ — plants facts by hand in place of the host observation layer to drive the policy deterministically.
#[cfg(any(test, feature = "test-harness"))]
#[derive(Default)]
pub struct ScriptedTurnFacts {
    facts: Mutex<HashMap<(PeerId, u32), TurnFact>>,
}

#[cfg(any(test, feature = "test-harness"))]
impl ScriptedTurnFacts {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Plants this incarnation as "mid-turn, last signal = `at`" — leaves already planted waiting-list and stopped slots as they are.
    pub fn set_in_turn(&self, id: PeerId, epoch: u32, at: Instant) {
        self.update(id, epoch, at, |f| {
            f.in_turn = true;
            f.last_signal = at;
        });
    }

    /// Plants this incarnation as "turn ended, last signal = `at`" (observed, not mid-turn); leaves waiting-list and stopped slots as they are.
    pub fn set_idle(&self, id: PeerId, epoch: u32, at: Instant) {
        self.update(id, epoch, at, |f| {
            f.in_turn = false;
            f.last_signal = at;
        });
    }

    /// Changes only the waiting-list slot; with no fact yet, plants it on "not a turn, last signal = `at`".
    // ADR-0231
    pub fn set_inputs_pending(&self, id: PeerId, epoch: u32, pending: bool, at: Instant) {
        self.update(id, epoch, at, |f| f.inputs_pending = pending);
    }

    /// Changes only the stopped-after-error slot; with no fact yet, plants it on "not a turn, last signal = `at`".
    // ADR-0231
    pub fn set_last_end_failed(&self, id: PeerId, epoch: u32, failed: bool, at: Instant) {
        self.update(id, epoch, at, |f| f.last_end_failed = failed);
    }

    fn update(&self, id: PeerId, epoch: u32, at: Instant, change: impl FnOnce(&mut TurnFact)) {
        let mut g = self.facts.lock().unwrap();
        let fact = g.entry((id, epoch)).or_insert(TurnFact {
            in_turn: false,
            last_signal: at,
            inputs_pending: false,
            last_end_failed: false,
        });
        change(fact);
    }

    pub fn forget(&self, id: PeerId, epoch: u32) {
        self.facts.lock().unwrap().remove(&(id, epoch));
    }
}

#[cfg(any(test, feature = "test-harness"))]
impl TurnFacts for ScriptedTurnFacts {
    fn turn_fact(&self, id: PeerId, epoch: u32) -> Option<TurnFact> {
        self.facts.lock().unwrap().get(&(id, epoch)).copied()
    }
    fn in_turn_snapshot(&self) -> Vec<(PeerId, u32, Instant)> {
        self.facts
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, f)| f.in_turn)
            .map(|((id, epoch), f)| (*id, *epoch, f.last_signal))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    struct RecordingNotifier {
        seen: StdMutex<Vec<PeerId>>,
    }
    impl RecordingNotifier {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                seen: StdMutex::new(Vec::new()),
            })
        }
        fn seen(&self) -> Vec<PeerId> {
            self.seen.lock().unwrap().clone()
        }
    }
    impl IdleNotifier for RecordingNotifier {
        fn notify_idle(&self, id: PeerId) {
            self.seen.lock().unwrap().push(id);
        }
    }

    fn policy() -> (BusyPolicy, Arc<ScriptedTurnFacts>, Arc<RecordingNotifier>) {
        let facts = ScriptedTurnFacts::new();
        let notifier = RecordingNotifier::new();
        (
            BusyPolicy::new(facts.clone(), notifier.clone()),
            facts,
            notifier,
        )
    }

    #[test]
    fn unknown_agent_is_idle_positive_knowledge_only() {
        let (p, _f, _n) = policy();
        assert!(
            !p.is_busy(PeerId::new_v4(), 0),
            "an unobserved target is treated as idle (immediate inject fallback for backends that cannot be observed)"
        );
    }

    #[test]
    fn an_observed_turn_is_busy_and_its_end_is_idle() {
        let (p, f, _n) = policy();
        let id = PeerId::new_v4();
        let t0 = Instant::now();
        f.set_in_turn(id, 0, t0);
        assert!(p.is_busy(id, 0));
        f.set_idle(id, 0, t0);
        assert!(!p.is_busy(id, 0), "turn end observed → idle");
    }

    #[test]
    fn state_is_keyed_by_epoch_no_leak_across_incarnations() {
        // If an old epoch's observation leaks into the new epoch's decision, mail addressed to the new incarnation waits without grounds.
        let (p, f, _n) = policy();
        let id = PeerId::new_v4();
        f.set_in_turn(id, 0, Instant::now());
        assert!(p.is_busy(id, 0));
        assert!(!p.is_busy(id, 1), "another epoch is unobserved → idle");
    }

    #[test]
    fn the_ceiling_flips_the_verdict_without_touching_the_facts() {
        let (p, f, _n) = policy();
        let id = PeerId::new_v4();
        let t0 = Instant::now();
        f.set_in_turn(id, 0, t0);
        p.sweep_stale_busy(t0 + BUSY_MAX_TURN - Duration::from_secs(1));
        assert!(
            p.is_busy(id, 0),
            "inside the cap it is still busy (cutting a normal long turn would inject mid-turn)"
        );
        p.sweep_stale_busy(t0 + BUSY_MAX_TURN);
        assert!(
            !p.is_busy(id, 0),
            "past the cap it is seen as leftover and idle"
        );
        assert_eq!(
            f.turn_fact(id, 0),
            Some(TurnFact {
                in_turn: true,
                last_signal: t0,
                inputs_pending: false,
                last_end_failed: false,
            }),
            "the facts stay as they are — the mail's decision does not change the shared table"
        );
    }

    #[test]
    fn a_fresh_turn_signal_restores_busy_without_waiting_for_a_sweep() {
        let (p, f, _n) = policy();
        let id = PeerId::new_v4();
        let t0 = Instant::now();
        f.set_in_turn(id, 0, t0);
        p.sweep_stale_busy(t0 + BUSY_MAX_TURN);
        assert!(!p.is_busy(id, 0));
        f.set_in_turn(id, 0, t0 + Duration::from_secs(31 * 60));
        assert!(p.is_busy(id, 0), "a new signal voids the leftover decision");
    }

    #[test]
    fn sweep_wakes_only_stale_turns() {
        let (p, f, n) = policy();
        let stale = PeerId::new_v4();
        let fresh = PeerId::new_v4();
        let t0 = Instant::now();
        f.set_in_turn(stale, 0, t0);
        f.set_in_turn(fresh, 0, t0 + Duration::from_secs(25 * 60));
        assert_eq!(
            p.sweep_stale_busy(t0 + BUSY_MAX_TURN + Duration::from_secs(1)),
            1
        );
        assert_eq!(n.seen(), vec![stale]);
        assert!(
            p.is_busy(fresh, 0),
            "the decision of a fresh turn is preserved"
        );
    }

    #[test]
    fn sweep_does_not_wake_before_the_ceiling() {
        let (p, f, n) = policy();
        let id = PeerId::new_v4();
        let t0 = Instant::now();
        f.set_in_turn(id, 0, t0);
        assert_eq!(
            p.sweep_stale_busy(t0 + BUSY_MAX_TURN - Duration::from_secs(1)),
            0,
            "below the cap it is not leftover"
        );
        assert!(n.seen().is_empty());
    }

    #[test]
    fn the_same_stale_turn_is_woken_once_but_a_new_one_wakes_again() {
        let (p, f, n) = policy();
        let id = PeerId::new_v4();
        let t0 = Instant::now();
        f.set_in_turn(id, 0, t0);
        let late = t0 + BUSY_MAX_TURN + Duration::from_secs(1);
        assert_eq!(p.sweep_stale_busy(late), 1);
        assert_eq!(p.sweep_stale_busy(late + Duration::from_secs(60)), 0);
        assert_eq!(n.seen(), vec![id], "1 doorbell");

        let t1 = late + Duration::from_secs(120);
        f.set_in_turn(id, 0, t1);
        assert_eq!(p.sweep_stale_busy(t1 + BUSY_MAX_TURN), 1);
        assert_eq!(n.seen(), vec![id, id]);
    }

    #[test]
    fn sweep_forgets_incarnations_that_left_the_facts() {
        let (p, f, n) = policy();
        let id = PeerId::new_v4();
        let t0 = Instant::now();
        f.set_in_turn(id, 0, t0);
        let late = t0 + BUSY_MAX_TURN + Duration::from_secs(1);
        assert_eq!(p.sweep_stale_busy(late), 1);
        f.forget(id, 0);
        assert_eq!(p.sweep_stale_busy(late), 0);
        f.set_in_turn(id, 0, t0);
        assert_eq!(p.sweep_stale_busy(late), 1);
        assert_eq!(n.seen(), vec![id, id]);
    }

    // ── waiting list · stopped after error (ADR-0231) ──

    #[test]
    fn each_fact_alone_is_busy_and_all_false_is_idle() {
        let (p, f, _n) = policy();
        let t0 = Instant::now();
        let pending = PeerId::new_v4();
        f.set_inputs_pending(pending, 0, true, t0);
        let halted = PeerId::new_v4();
        f.set_last_end_failed(halted, 0, true, t0);
        let turning = PeerId::new_v4();
        f.set_in_turn(turning, 0, t0);
        let quiet = PeerId::new_v4();
        f.set_idle(quiet, 0, t0);
        assert!(
            p.is_busy(pending, 0),
            "if there is a user list, the mail waits"
        );
        assert!(
            p.is_busy(halted, 0),
            "the mail waits while stopped after an error"
        );
        assert!(p.is_busy(turning, 0));
        assert!(!p.is_busy(quiet, 0));
    }

    #[test]
    fn the_ceiling_releases_neither_the_list_nor_the_halt() {
        let (p, f, n) = policy();
        let t0 = Instant::now();
        let late = t0 + BUSY_MAX_TURN + Duration::from_secs(1);
        let pending = PeerId::new_v4();
        f.set_inputs_pending(pending, 0, true, t0);
        let halted = PeerId::new_v4();
        f.set_last_end_failed(halted, 0, true, t0);
        let both = PeerId::new_v4();
        f.set_in_turn(both, 0, t0);
        f.set_inputs_pending(both, 0, true, t0);

        assert_eq!(
            p.sweep_stale_busy(late),
            1,
            "the cap sweeps only turn observations — an incarnation with only the list·stop set is not a leftover candidate"
        );
        assert_eq!(n.seen(), vec![both]);
        assert!(
            p.is_busy(pending, 0),
            "the list is busy even after 30 minutes (N8)"
        );
        assert!(
            p.is_busy(halted, 0),
            "the stop is busy even after 30 minutes (N11)"
        );
        assert!(
            p.is_busy(both, 0),
            "even if an old turn is decided as leftover, it is busy while the list remains"
        );
    }

    #[test]
    fn clearing_the_facts_releases_the_gate() {
        let (p, f, _n) = policy();
        let t0 = Instant::now();
        let id = PeerId::new_v4();
        f.set_inputs_pending(id, 0, true, t0);
        f.set_last_end_failed(id, 0, true, t0);
        f.set_inputs_pending(id, 0, false, t0);
        assert!(p.is_busy(id, 0), "the stop remains");
        f.set_last_end_failed(id, 0, false, t0);
        assert!(!p.is_busy(id, 0));
    }

    #[test]
    fn always_idle_gate_never_reports_busy() {
        let g = AlwaysIdleGate;
        assert!(!g.is_busy(PeerId::new_v4(), 7));
        assert_eq!(g.busy_reason(PeerId::new_v4(), 7), None);
    }

    #[test]
    fn a_stale_turn_with_waiting_input_reports_the_input_not_the_turn() {
        // A turn judged wreckage is not a reason — only the waiting list holds busy up.
        let (p, f, _n) = policy();
        let id = PeerId::new_v4();
        let t0 = Instant::now();
        f.set_in_turn(id, 0, t0);
        f.set_inputs_pending(id, 0, true, t0);
        assert_eq!(p.busy_reason(id, 0), Some(BusyReason::InTurn));
        p.sweep_stale_busy(t0 + BUSY_MAX_TURN);
        assert_eq!(p.busy_reason(id, 0), Some(BusyReason::InputsPending));
        f.set_inputs_pending(id, 0, false, t0);
        assert_eq!(p.busy_reason(id, 0), None);
        assert!(!p.is_busy(id, 0), "no reason = not busy");
    }
}
