//! busy — recipient turn-state interpretation + idle gate seam (ADR-0104 decision 3 · ADR-0113 decision 2).
//!
//! ★Role = **policy** only★: the **fact** "is it mid-turn right now" is held by the host's shared observation layer (ADR-0113
//!   decision 1), and this module **interprets that fact as a value judgment of mail**. Three pieces:
//!     ① `TurnFacts` — the port that reads the facts (implemented by the host adapter).
//!     ② `BusyPolicy` — builds the `BusyGate` answer from those facts + this module's policy (positive-knowledge-only · the `BUSY_MAX_TURN` limit),
//!        and wakes turns past the limit with a doorbell.
//!     ③ `BusyGate`/`IdleNotifier` — the door the service asks through, and the flush doorbell exit.
//!
//! ★Busy = a user-waiting list exists ∨ stopped after an error ∨ (mid-turn ∧ within the limit) (ADR-0231)★: the first two facts have no
//!   limit — user input comes first (N6·N8), and mail is not pushed in after a real error (N11). It is an intended exception to
//!   ADR-0104 「늦게 가는 것 < 안 가는 것」 (going late is better than not going).
//!
//! ★positive-knowledge-only (load-bearing — spec §5 capability fallback)★: an (id, epoch) with **no** observation is entirely
//!   **treated as idle** (= inject at once). Reading "unknown" as busy would make delivery **wait forever** for backends that cannot be observed or in the window before
//!   observation has started. busy is true **only when there is an observed fact**.
//!
//! ★The fact layer is **not mutated** (ADR-0113 decision 2 — never violate)★: neither the limit decision nor the sweep removes
//!   entries from the table. That table is a shared facility used with other consumers (UI input locking etc.), and each consumer has a
//!   different limit — if one consumer erases an entry as "old", another consumer's fact evaporates. All done here is
//!   **read and decide by one's own criterion**.
//!
//! ★busy observation targets = backends that emit turn signals★: turn events come only from agents that have a backend decoder,
//!   and the decoder exists under **exactly the same condition** as the structured-output capability.
//!   That proxy value rides on the roster entry as `LiveAgent::turn_signal` (ADR-0116 decision 7), and the send path
//!   injects at once for the kind with `turn_signal == false` **without asking this gate at all**. The day the proxy breaks
//!   (a backend appears that is structured but has no turn events), a real capability field will be added — consistent with the ADR-0104 capability
//!   principle (unobservable = immediate-injection fallback).
//!
//! ★Callback discipline (load-bearing — never violate)★: `IdleNotifier::notify_idle` is also called on **the host's output pump
//!   thread**. Its implementation must do **only one thing**, a non-blocking notification send — doing injection, taking the messaging
//!   lock or a blocking write would block the output pump behind delivery work and delay every agent's output stream.
//!   The actual flush is done by the flush worker that received the notification.
//!
//! ★Known unverified (measurement item — spec §7)★: it is unverified whether the `result` line of a nested Task subagent leaks as a **parent turn end**
//!   signal. If it does, the parent could be misjudged as idle while still mid-turn and injected early
//!   (no loss, only timing off). It is left unresolved as a real-harness measurement item.
// ADR-0104
// ADR-0110
// ADR-0113
// ADR-0231

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::PeerId;

/// ★busy limit (fail-open safety valve — round-3 finding 4, internal choice: **30 minutes**)★: a "mid-turn" observation older than this
///   from the last turn signal is judged idle, seen as **wreckage of an abnormally ended turn**.
///
/// ★Why a limit is needed★: the turn-end signal is the only in-band release. If a turn ends abnormally (parse failure · decoder
///   fault · missing end line), every delivery addressed to that recipient is folded at every doorbell and finally expires by TTL
///   ("not arriving" = the worst failure mode of messaging). Frequency of occurrence is unmeasured (spec §7 item).
/// ★Why the limit is safe (what is lost)★: the consequence of a misjudgment (seeing idle when it is actually mid-turn) is **injection mid-turn**,
///   and the claude CLI puts stdin into an internal queue mid-turn and **reads it on the next turn** (measured user-facing behavior — spec §7).
///   So the worst cost is only that "when it is read" gets blurry, and **there is no loss**. Without the limit, on the other hand, all mail
///   addressed to that recipient does not arrive until TTL — incomparably worse (ADR-0104 "going late is better than not going").
/// ★Why 30 minutes★: at human-conversation message rates a 30-minute no-output turn is far outside the normal range (any one of a tool call ·
///   delta · usage arriving makes the fact layer refresh the time, so 30 minutes means the span "output has completely stopped and there is
///   no turn-end signal either"). Shorter cuts normal long turns, and longer delays recovery.
/// ★Applied only to turn observation (`in_turn`)★ — this limit does not release `inputs_pending` or `last_end_failed` (module header).
pub const BUSY_MAX_TURN: Duration = Duration::from_secs(30 * 60);

/// ★idle gate lookup seam (ADR-0012)★ — the only door through which MessagingService asks "can this recipient receive mail right now".
/// The service calls only `busy_reason` (delivery decision = `is_some()`, hint text = that value).
///
/// ★Contract★: a pure query — no side effects, no blocking (short locks only). It must be safe to call **while holding** the messaging lock
///   (current call sites call it outside the lock, but keeping this contract means no deadlock even if future call sites grow).
pub trait BusyGate: Send + Sync {
    fn is_busy(&self, id: PeerId, epoch: u32) -> bool;

    /// The reason if busy, otherwise `None`. ★The service blocks delivery on the `is_some()` of this value and picks the hint text
    /// by that value★ — `MessagingService` does not call `is_busy`.
    /// ★`is_busy` must always equal the `is_some()` of this★: if one of the two is overridden, adjust both
    /// together so they do not diverge. An implementation that does not know the reason answers with the default (if `is_busy`, mid-turn).
    // ADR-0231
    fn busy_reason(&self, id: PeerId, epoch: u32) -> Option<BusyReason> {
        self.is_busy(id, epoch).then_some(BusyReason::InTurn)
    }
}

/// Why the recipient cannot receive mail — for choosing hint text. If two or more overlap, the one written higher wins (`BusyPolicy::busy_reason`).
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

/// ★Turn-fact lookup port (ADR-0110 decision 3 · ADR-0113 decision 1)★ — the door through which the kernel reads the host's shared observation layer **without
///   knowing it even as a type**. The production implementation is owned by the host adapter.
///
/// ★Read-only is the contract★: do not add "erase/mark" to this port (basis = module header).
/// ★Why the values are returned together★: if asked separately, a signal slips in between the two lookups and the decision is made on an impossible combination like "mid-turn but the time is old" or
///   "the turn ended but the error is not yet written".
pub trait TurnFacts: Send + Sync {
    /// The observed value of this (id, epoch). `None` = unobserved (no fact at all).
    fn turn_fact(&self, id: PeerId, epoch: u32) -> Option<TurnFact>;

    /// Everyone currently observed as mid-turn `(id, epoch, 마지막 신호 시각)` — the entry of the limit sweep.
    fn in_turn_snapshot(&self) -> Vec<(PeerId, u32, Instant)>;
}

/// ★Turn-end (idle transition) notification seam★ — the entry where the host reports "this agent's turn has ended", and the exit where the limit sweep
///   wakes a stopped turn. The production implementation does a non-blocking send to the flush worker channel.
///
/// ★Contract (load-bearing)★: **non-blocking and non-reentrant** (basis = module header callback discipline).
/// ★Idempotency dependence (accepted design)★: a notification goes out on **every turn-end signal** (whether it was a transition or not). Why:
///   narrowing it to "only when the previous state was busy" would drop the notification for a turn that ends at once with no assistant event, and parking would be
///   stranded until the next turn — we choose a **redundant notification (harmless)** over a missed delivery (fatal). The cost of a redundant notification is
///   only an empty-queue drain no-op. The upper bound on channel pressure is the host notification implementation's responsibility.
pub trait IdleNotifier: Send + Sync {
    fn notify_idle(&self, id: PeerId);
}

/// ★BusyPolicy — the gate that puts the mail policy on top of the facts (C2 · ADR-0113 decision 2)★. One is made at daemon boot and
///   shared by MessagingService (gate lookup) and the sweep task (limit fail-open).
///
/// ★The only state is the limit-decision ledger, and that is **policy** state★: all facts are beyond the port. Since the shared table cannot be
///   erased (module header), the judgment "this signal of this incarnation is seen as wreckage" is written down on this side — that is both the gate's
///   answer and the "already woken" mark.
/// ★It does not read the clock (determinism kept)★: the limit comparison happens only inside `sweep_stale_busy`, which takes `now` as an argument,
///   and the lookup (`is_busy`) just looks at that ledger. So the crate purity discipline (clock injection) holds in this
///   module too, and the decision time is pinned to the sweep period, making observable behavior deterministic.
/// ★Lock = this one ledger, notification outside the lock (ADR-0006)★: `notify_idle` is an external call, so it is not called while holding the lock.
pub struct BusyPolicy {
    facts: Arc<dyn TurnFacts>,
    notifier: Arc<dyn IdleNotifier>,
    /// The incarnation **judged wreckage** by exceeding the limit → the last signal time seen then. If the value changes (a new signal came) the judgment is
    /// void — so the moment the next signal comes it returns to busy without waiting for the sweep.
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

    /// ★Limit sweep (round-3 finding 4)★ — **judges wreckage** for "mid-turn" observations past `BUSY_MAX_TURN` and **wakes**
    ///   their owner **with a doorbell** (the waiting parking is delivered then). Called by the daemon's periodic sweep task.
    ///   Return = the number of ids woken this time.
    ///
    /// ★Waking is half of the fix★: flipping only the decision leaves the parking addressed to that recipient sitting there until the **next trigger** (appearance/turn end/new send)
    ///   — but this situation is by definition one where "that trigger does not come" (turn-end signal lost).
    /// ★One doorbell per wreckage★: since the fact cannot be erased, the same wreckage is seen again every period — if the last signal
    ///   time equals the ledger, it is wreckage already handled. If a new signal comes and stops again, that is new wreckage.
    /// ★The ledger is swapped each time for "what is wreckage now"★: entries of incarnations that revived or were reaped do not pile up.
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
                "busy 상한 초과 — 턴 종료 관측 없이 늙은 턴을 idle 로 판정하고 flush 도어벨(fail-open 안전 밸브)"
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

/// ★Harness/test-only fact source★ — plants facts by hand instead of the host observation layer to drive the policy deterministically.
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

    /// Plants this incarnation as "mid-turn, last signal = `at`" — leaves the already planted waiting-list and stopped slots as they are.
    pub fn set_in_turn(&self, id: PeerId, epoch: u32, at: Instant) {
        self.update(id, epoch, at, |f| {
            f.in_turn = true;
            f.last_signal = at;
        });
    }

    /// Plants this incarnation as "turn ended, last signal = `at`" (observed but not mid-turn). Leaves the waiting-list and stopped
    /// slots as they are.
    pub fn set_idle(&self, id: PeerId, epoch: u32, at: Instant) {
        self.update(id, epoch, at, |f| {
            f.in_turn = false;
            f.last_signal = at;
        });
    }

    /// Changes only the waiting-list slot. If there is no fact yet, plants it on top of "not a turn, last signal = `at`".
    // ADR-0231
    pub fn set_inputs_pending(&self, id: PeerId, epoch: u32, pending: bool, at: Instant) {
        self.update(id, epoch, at, |f| f.inputs_pending = pending);
    }

    /// Changes only the stopped-after-error slot. If there is no fact yet, plants it on top of "not a turn, last signal = `at`".
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
            "미관측 대상은 idle 취급(관측 불가 백엔드 즉시 주입 폴백)"
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
        assert!(!p.is_busy(id, 0), "턴 종료 관측 → idle");
    }

    #[test]
    fn state_is_keyed_by_epoch_no_leak_across_incarnations() {
        // If an old epoch's observation leaks into the new epoch's decision, mail addressed to the new incarnation waits without grounds.
        let (p, f, _n) = policy();
        let id = PeerId::new_v4();
        f.set_in_turn(id, 0, Instant::now());
        assert!(p.is_busy(id, 0));
        assert!(!p.is_busy(id, 1), "다른 epoch 은 미관측 → idle");
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
            "상한 안쪽은 여전히 busy(정상 장기 턴을 자르면 턴 중 주입이 된다)"
        );
        p.sweep_stale_busy(t0 + BUSY_MAX_TURN);
        assert!(!p.is_busy(id, 0), "상한 초과는 잔해로 보고 idle");
        assert_eq!(
            f.turn_fact(id, 0),
            Some(TurnFact {
                in_turn: true,
                last_signal: t0,
                inputs_pending: false,
                last_end_failed: false,
            }),
            "사실은 그대로 — 우편의 판정이 공용 표를 바꾸지 않는다"
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
        assert!(p.is_busy(id, 0), "새 신호가 오면 잔해 판정이 무효화된다");
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
        assert!(p.is_busy(fresh, 0), "신선한 턴 판정은 보존");
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
            "상한 미달은 잔해가 아니다"
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
        assert_eq!(n.seen(), vec![id], "도어벨 1회");

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
            "사용자 목록이 있으면 우편이 기다린다"
        );
        assert!(p.is_busy(halted, 0), "오류 뒤 멈춤 동안 우편이 기다린다");
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
            "상한은 턴 관측만 훑는다 — 목록·멈춤만 선 화신은 잔해 후보가 아니다"
        );
        assert_eq!(n.seen(), vec![both]);
        assert!(p.is_busy(pending, 0), "30 분이 지나도 목록은 바쁨(N8)");
        assert!(p.is_busy(halted, 0), "30 분이 지나도 멈춤은 바쁨(N11)");
        assert!(
            p.is_busy(both, 0),
            "늙은 턴이 잔해로 판정돼도 목록이 남았으면 바쁨"
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
        assert!(p.is_busy(id, 0), "멈춤이 남았다");
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
        assert!(!p.is_busy(id, 0), "사유 없음 = 바쁘지 않음");
    }
}
