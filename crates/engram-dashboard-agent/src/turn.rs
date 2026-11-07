//! turn — a **turn-progress observation table** per (agent, epoch). The agent "fact" layer owned by the core
//! (ADR-0113 decision 1 · ADR-0119 decision 4 — next to the registry).
//!
//! ★What this module owns = observed facts only★: "is this incarnation in a turn now" · "when was the last turn signal" ·
//!   "was the last turn end an error and is there no clean end after it yet" (stall after error — ADR-0231).
//!   **Interpretation is the consumers' job** — fallbacks, bounds, doorbells (ADR-0113 decision 2) — the cost of a misjudgment differs per consumer
//!   (for mail, "not delivered" is the worst, so it uses a fallback that collapses toward idle, and input locking wants a different bound).
//!   Putting policy here would force one consumer's value judgment on everyone.
//!
//! ★The event→signal **mapping is not here** (in the spirit of ADR-0004/0110 decision 4)★: which output event is turn progress is
//!   backend knowledge, so `backend::AgentBackend::turn_classifier` owns it. All this module knows is the signal vocabulary
//!   (`TurnSignal`) and how to accumulate those signals.
//!
//! ★The write path = `OutputCore::emit` alone, but it has **two callers** (load-bearing)★: ① the output pump thread
//!   ② the input echo path (`AgentSession::write_input_observed` — the injecting thread emits as is: the mail
//!   flush lane's blocking thread · the MCP HTTP handler · the Tauri command thread). So an argument that
//!   assumes a "single thread" does not hold. What does hold instead:
//!     - The mutex of this table is a **leaf** — never held together with another lock, and no outside call while held. So
//!       calling it from any thread creates no cycle in the lock-order rule (ADR-0006).
//!     - The critical section is only a few lines of hash-map operations. This is mandatory because one of the callers is the **output pump** — if this
//!       grows long, that agent's whole output stream backs up behind it.
//!
//! ★Lock = this table only (a structure that needs no order rule)★: key entries by `AgentId` and hold the epoch as a **value**,
//!   ending with one map. If the epoch went into the key, a second table to answer "what is this id's current epoch" would be needed,
//!   and at that moment an order rule between two locks arises.
//!
//! ★One entry per id★: there is always exactly one live incarnation. The only verb that takes that place is [`TurnObservations::register`]
//!   (owner = `AgentManager::spawn_session`), and the main cleanup path of an entry is `forget` (owner = `OutputCore::finish`).
//!   ★The incarnation marker guarantees only that markers are **different from each other** — order has no meaning★: every spawn, whatever the mode, passes
//!   the single issuing point of `AgentManager::spawn_agent` (`ProfileRegistry::epoch_for_spawn`), where
//!   a fresh random number is drawn per incarnation and guaranteed different from the previous value. **Do not scatter issuing across call sites** — in the
//!   era that did, the Fresh respawn paths reused markers and two incarnations sat here under the same key (that regression is the reason
//!   this paragraph exists).
//!   ★So marker comparison is only **equal/unequal** — never ask which is larger (do not revive it)★: once `observe_at`
//!   filtered late signals by "drop if the marker is smaller". When the marker was an ordered counter it fired 100% of the time, but
//!   after it became a random number, **half the time the dying incarnation's marker was larger** and passed as is, and that signal overwrote the
//!   live incarnation's entry. After that, the live incarnation's query answered unobserved (= not in a turn), so **mail goes in mid-turn.**
//!   Narrowing the authority to change the place to `register` alone closes that hole — a late signal is discarded because its marker does not match.
// ADR-0113
// ADR-0006

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use engram_dashboard_base::sync;

use crate::types::AgentId;

/// A turn signal read out of an output event — the entire vocabulary the table knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnSignal {
    /// The backend that emitted that event classified it as "this incarnation is in a turn".
    Progress,
    /// This turn hit an error — ★does not change `in_turn`★. The table holds it, and at that turn's `Ended`
    /// folds it into `last_end_failed` (the stall does not stand before the end is seen).
    // ADR-0231
    Failed,
    /// The backend that emitted that event classified it as "this incarnation's turn has ended" — carries the kind of end.
    Ended(TurnEndKind),
}

/// The kind of turn end — the input of the `last_end_failed` folding rule (ADR-0231).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnEndKind {
    /// A clean success end — releases the stall.
    Clean,
    /// An error end — raises the stall.
    Failed,
    /// Any other end (interrupted · unknown) — neither raises nor releases the stall.
    Other,
}

/// Turn-observation snapshot of one (agent, epoch). Absence (`Option::None`) is **unobserved**, not idle —
/// how to treat the two is decided by the consumer (module header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnObservation {
    pub in_turn: bool,
    /// The time the last turn signal was observed for this incarnation. The axis on which a consumer judges "how old is this observation" by its own
    /// standard (stuck-turn detection etc.).
    pub last_signal: Instant,
    /// This incarnation's last turn end was an error and there is no clean success end after it yet (stall after error). An end of kind 「그 밖」 (other) does
    /// not change this value. Right after registration = false.
    // ADR-0231
    pub last_end_failed: bool,
}

/// Turn observation table — one is owned by `AgentManager` and shared by all `OutputCore`s of that manager.
///
/// ★Observation target = "sessions that actually emit turn signals" (why there is no separate capability gate)★: whether to emit a signal
///   is decided by that session's backend classifier (`backend::turn_classifier` — a backend that does not declare it stays silent).
///   So we do not filter again here by the "structured output capability" — that would make the core retrace session assembly information,
///   and on the day the two criteria diverge it would be unclear which is canonical.
pub struct TurnObservations {
    entries: Mutex<HashMap<AgentId, Entry>>,
}

struct Entry {
    epoch: u32,
    in_turn: bool,
    last_signal: Instant,
    /// The **output seq** that last updated this entry (comparable only within the same epoch — when the incarnation changes, seq counts from 0
    /// again). The axis that filters out late signals when emit order and apply order diverge (`observe_at`).
    /// ★The cursor of `in_turn` · `last_signal` — the stall field is filtered separately by `last_end_seq`★.
    last_seq: u64,
    /// The seq of the last folded `Ended` — the cursor of the stall fields (`error_seq` · `last_end_failed`) (ADR-0231).
    last_end_seq: Option<u64>,
    /// The earliest seq of a `Failed` not yet folded by any `Ended` — an `Ended` with a larger seq folds it and clears it.
    error_seq: Option<u64>,
    last_end_failed: bool,
}

impl Entry {
    fn fresh(epoch: u32, at: Instant) -> Self {
        Self {
            epoch,
            in_turn: false,
            last_signal: at,
            last_seq: 0,
            last_end_seq: None,
            error_seq: None,
            last_end_failed: false,
        }
    }
}

impl Default for TurnObservations {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnObservations {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// ★Poison tolerance (the same judgment as the sessions lock in reaper.rs)★: this is a **manager-global** table, so if any holder
    ///   panics, after that the turn-signal emit of **every** agent re-panics in a chain — a different nature from the core's per-agent
    ///   locks (the ones whose `expect` justification is "does not propagate to other agents"). It holds only a hash map, so
    ///   no invariant is broken, and we recover the guard and keep going. (In the production process the only case this tolerance receives is when the contained
    ///   pump panicked while holding this lock — a holder panic elsewhere has the hook end the process · ADR-0292 ·
    ///   command execution is not contained · ADR-0293.)
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<AgentId, Entry>> {
        sync::lock(&self.entries)
    }

    /// This incarnation takes this id's place — **unconditionally** replaces whatever entry was there.
    ///
    /// ★The only caller is `AgentManager::spawn_session`, and the call point is **before the session becomes observable**
    ///   (before the sessions map insert = before pump start). This order underpins the correctness of this table★: if registration is later than that incarnation's
    ///   first signal, that signal is discarded for not matching the old entry's marker, and that incarnation answers **permanently unobserved**
    ///   (= not in a turn) until the old entry is `forget`-ed — mail injection mid-turn is that outcome.
    /// ★Ordering against the dying previous incarnation is safe either way★: if the previous incarnation's `forget` comes first, the entry is empty and we
    ///   fill it. If we come first, the previous incarnation's `forget` is a no-op because the marker differs, and a late signal is discarded for the same reason.
    /// ★The value put in is "no signal yet"★: `in_turn=false`. The table holds only facts, so registration itself does not mean a turn.
    ///   So the entry of a backend that emits no signal at all is not carried in `in_turn_snapshot`, and `finish` clears it.
    pub fn register(&self, id: AgentId, epoch: u32) {
        self.register_at(id, epoch, Instant::now());
    }

    /// Time-injected [`Self::register`].
    pub fn register_at(&self, id: AgentId, epoch: u32, at: Instant) {
        self.lock().insert(id, Entry::fresh(epoch, at));
    }

    /// Applies one turn signal (observation time = now).
    /// `seq` = that event's output sequence (canonical for emit order — the order-inversion defense axis of `observe_at`).
    pub fn observe(&self, id: AgentId, epoch: u32, seq: u64, signal: TurnSignal) {
        self.observe_at(id, epoch, seq, signal, Instant::now());
    }

    /// Time-injected — tests drive the bound boundary · time refresh · order inversion without real-time waits.
    ///
    /// ★This is where late signals of an old incarnation are filtered (secondary defense)★: even right after an incarnation swap the old `OutputCore` keeps draining the events
    ///   left in its queue for a moment (before the pump ends on EOF). Receiving such a signal revives the entry of an incarnation already destined to vanish,
    ///   and since that incarnation's end signal **never comes** (the stream is already past),
    ///   it becomes a ghost entry no one can erase.
    /// ★The range this rule blocks is narrow (no overclaiming)★: late signals with the same marker — especially an input echo arriving **after** `forget` —
    ///   cannot be blocked here. The caller (`OutputCore::emit`) blocks that with the finalize flag
    ///   (the comment there is canonical for that causality). The main line of defense is there too.
    /// ★If the marker differs, **discard unconditionally** — never ask which is larger★: the marker is a random number, so order has no meaning
    ///   (`AgentProfile::epoch`). The authority to change the place belongs to [`Self::register`] alone, and that registration coming before the live incarnation's
    ///   first signal is what makes this discarding safe (the order paragraph of that function).
    /// ★If there is no entry at all, accept it★: assemblies that skip registration (harnesses · a hand-built `OutputCore`) are a door left
    ///   so they too can be observed. In production a late signal after `finish` cleared may come in through this door, but the
    ///   caller's finalize recheck takes it back out (paragraph above).
    ///
    /// ★Within the same marker, **order is set by seq** (load-bearing)★: emit has two callers (the output pump ·
    ///   the injecting thread that issued the input echo), so when two signals run in parallel **emit order and apply order can diverge** — if the injecting
    ///   thread emits a progress signal with seq N and pauses, and meanwhile the pump applies the end signal of seq N+1,
    ///   the progress signal waking up late **turns the ended turn back to in-progress** (mail addressed to that receiver
    ///   gets parked again up to the bound). seq is issued inside the replay lock and is the canonical order of the output (output_core.rs
    ///   emit), so ignoring lower seqs closes that inversion.
    /// ★seq comparison is valid **only within the same marker**★: a new incarnation is a new `OutputCore`, so seq starts from 0
    ///   again — so when the marker differs, seq is not looked at at all (that entry is replaced by [`Self::register`]).
    /// ★The cost (honest note)★: the time is stamped outside the lock, so the displaced signal usually carries a **newer time**.
    ///   That is, this guard keeps `last_signal` slightly toward the **older side** — meaning the bound decision may
    ///   fire that much earlier, and the width is one scheduler quantum, meaningless at a 30-minute bound. This is the result of making the canonical **emit order**
    ///   the axis (making it apply order revives ended turns — paragraph above).
    ///
    /// ★The time is refreshed on every observation★: a turn that runs normally for long keeps receiving signals so it does not age, and only
    ///   one whose signals stopped ages — the consumer's "stuck turn" decision holds on that axis.
    /// ★`last_end_failed` folding (both backends — `TurnEndKind`)★: at `Ended`, true if it is an error end or it holds a `Failed` with a smaller seq ·
    ///   false if a clean end · otherwise unchanged. The held `Failed` is cleared by that `Ended`.
    /// ★The stall field is filtered by **its own cursor** (`last_end_seq`) — do not filter by the progress cursor (`last_seq`)★: if the pump
    ///   is issued `Failed` with seq N and, before it is written to the table, another thread's progress (N+1) is written first, the progress cursor
    ///   discards that `Failed`, and the later `Ended(Clean)` (N+2) folds the turn that ended in error as clean in ring order (mail addressed to
    ///   that receiver goes in without a stall after error). So `Failed` · `Ended` are discarded only if their seq is smaller than the last folded `Ended`,
    ///   and `Failed` is held (earliest seq) and folded by an `Ended` with a larger seq.
    ///   `in_turn` · `last_seq` · `last_signal` change only when that signal is the newest — progress with a later seq is the next turn.
    /// ★The remaining gap (honest note)★: a `Failed` with a seq smaller than the folded `Ended` is discarded if written **after** that `Ended` —
    ///   we hold no seq of the previous end that would tell which end that `Failed` belongs to. As long as the same
    ///   thread (the output pump) emits `Failed` and that turn's `Ended`, this apply order does not occur.
    /// ★The remaining residue and why we do not block it now★: if two incarnations' markers collide at 2^-32, this equality check is
    ///   disabled for that one incarnation's lifetime (`ProfileRegistry::epoch_for_spawn` — the adjacency guarantee
    ///   holds only within the same process). The outcome when it happens is "in a turn but looks idle, so mail goes in early", and this project
    ///   nailed down that early injection as better than "mail that never goes" (ADR-0104).
    // ADR-0007
    // ADR-0231
    pub fn observe_at(&self, id: AgentId, epoch: u32, seq: u64, signal: TurnSignal, at: Instant) {
        let mut g = self.lock();
        if g.get(&id).is_some_and(|cur| cur.epoch != epoch) {
            return;
        }
        let e = g.entry(id).or_insert_with(|| Entry::fresh(epoch, at));
        let newest = seq >= e.last_seq;
        let before_last_end = e.last_end_seq.is_some_and(|end| seq < end);
        match signal {
            TurnSignal::Progress => {
                if !newest {
                    return;
                }
                e.in_turn = true;
            }
            TurnSignal::Failed => {
                if before_last_end {
                    return;
                }
                e.error_seq = Some(e.error_seq.map_or(seq, |held| held.min(seq)));
            }
            TurnSignal::Ended(kind) => {
                if before_last_end {
                    return;
                }
                // An error issued after this end belongs to the next turn — neither fold it nor clear it.
                let held = e.error_seq.is_some_and(|err| err < seq);
                e.last_end_failed = match kind {
                    TurnEndKind::Failed => true,
                    _ if held => true,
                    TurnEndKind::Clean => false,
                    TurnEndKind::Other => e.last_end_failed,
                };
                if held {
                    e.error_seq = None;
                }
                e.last_end_seq = Some(seq);
                if newest {
                    e.in_turn = false;
                }
            }
        }
        if newest {
            // ★Do not rewind `last_signal`★: the time is stamped **outside** the lock (`observe` above), so a late-stamped
            //   signal may be applied first. If rewound, the bound decision misjudges it as wreckage that much earlier — it closes for
            //   free, so we close it.
            e.last_signal = at.max(e.last_signal);
            e.last_seq = seq;
        }
    }

    /// Observed value of this (agent, epoch). `None` = unobserved (including when only another epoch was observed).
    pub fn get(&self, id: AgentId, epoch: u32) -> Option<TurnObservation> {
        let g = self.lock();
        g.get(&id)
            .filter(|e| e.epoch == epoch)
            .map(|e| TurnObservation {
                in_turn: e.in_turn,
                last_signal: e.last_signal,
                last_end_failed: e.last_end_failed,
            })
    }

    /// Whether it is in a turn by observation. ★Unobserved is false★ — "unknown" is not turned into true here (if the fact layer
    /// guesses, the consumer cannot choose its own fallback).
    pub fn is_in_turn(&self, id: AgentId, epoch: u32) -> bool {
        self.get(id, epoch).is_some_and(|o| o.in_turn)
    }

    /// All agents observed as in a turn now. The entrance the consumer sweeps with its own bound.
    pub fn in_turn_snapshot(&self) -> Vec<(AgentId, u32, Instant)> {
        let g = self.lock();
        g.iter()
            .filter(|(_, e)| e.in_turn)
            .map(|(id, e)| (*id, e.epoch, e.last_signal))
            .collect()
    }

    /// Removes this incarnation's entry. ★Only when the epoch matches★ — keeps a late call from
    /// erasing the observation of an incarnation newly attached in between (the same principle as the reaper's sessions removal guard).
    ///
    /// ★What happens if we do not erase★: the "in a turn" mark of an agent that died mid-turn stays, and a consumer waiting
    ///   addressed to that name (mail parking etc.) is never released.
    /// ★The owner of cleanup is `OutputCore::finish`★ (the comment there is canonical for the causality — it also closes the race with a late emit after
    ///   termination there). There are **intentionally two call sites**: `finish` (the owner) and the finalize recheck
    ///   path of `emit` (it takes back what it just put in — both going through the same table mutex is the argument that
    ///   closes the race itself). **Adding a third caller** splits that causality — reading the emit side as a "forbidden
    ///   second caller" and deleting it reopens the ghost-entry hole.
    pub fn forget(&self, id: AgentId, epoch: u32) {
        let mut g = self.lock();
        if g.get(&id).is_some_and(|e| e.epoch == epoch) {
            g.remove(&id);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn unobserved_agent_has_no_entry() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        assert_eq!(t.get(id, 0), None);
        assert!(!t.is_in_turn(id, 0));
    }

    #[test]
    fn progress_then_end_flips_in_turn_and_keeps_the_entry() {
        // The consumer keeps reading "when was the last signal", so termination must not erase the entry.
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.observe(id, 0, 1, TurnSignal::Progress);
        assert!(t.is_in_turn(id, 0));
        t.observe(id, 0, 1, TurnSignal::Ended(TurnEndKind::Clean));
        assert!(!t.is_in_turn(id, 0));
        assert!(t.get(id, 0).is_some(), "종료 후에도 관측 사실은 남는다");
    }

    #[test]
    fn observation_is_keyed_by_epoch() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.observe(id, 0, 1, TurnSignal::Progress);
        assert!(t.is_in_turn(id, 0));
        assert_eq!(t.get(id, 1), None, "다른 epoch 은 미관측");
    }

    #[test]
    fn registration_replaces_the_entry_and_older_signals_are_dropped() {
        // If a late signal revives an old incarnation, no one can erase its entry (its end signal never comes).
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.observe(id, 0, 1, TurnSignal::Progress);
        t.register(id, 1);
        assert_eq!(t.get(id, 0), None, "등록이 옛 항목을 청소");
        assert_eq!(t.len(), 1, "재시작마다 죽은 항목이 쌓이지 않는다");
        assert!(!t.is_in_turn(id, 1), "등록만으로는 턴이 아니다");
        t.observe(id, 1, 1, TurnSignal::Progress);
        assert!(t.is_in_turn(id, 1));

        t.observe(id, 0, 1, TurnSignal::Progress);
        assert_eq!(t.get(id, 0), None, "옛 표식 지각 신호는 버린다");
        assert!(t.is_in_turn(id, 1), "현 화신 관측은 그대로");
    }

    // ★Regression target — that half after the marker became a random number★: the old comparison was "drop if the current marker is larger". In a permutation where the dying
    //   incarnation's marker was larger, that guard passed and the late signal **overwrote the live incarnation's entry**, and after that
    //   the live incarnation's query answered unobserved (= not in a turn), so mail went in mid-turn. So the two markers of this test are
    //   **deliberately inverted** (live incarnation 5 < dead incarnation 900) — reverting to a size comparison breaks it here.
    #[test]
    fn a_late_signal_from_a_larger_tag_cannot_evict_the_live_incarnation() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        let dying = 900;
        let live = 5;

        t.observe(id, dying, 3, TurnSignal::Progress);
        t.register(id, live);
        t.observe(id, live, 0, TurnSignal::Progress);
        assert!(t.is_in_turn(id, live), "전제: 산 화신이 턴 중으로 관측됨");

        // A dead incarnation whose draining was not finished now emits a signal.
        t.observe(id, dying, 4, TurnSignal::Ended(TurnEndKind::Clean));

        assert!(
            t.is_in_turn(id, live),
            "죽은 화신의 지각 신호가 산 화신의 관측을 덮으면 턴 중에 우편이 들어간다"
        );
        assert_eq!(
            t.get(id, dying),
            None,
            "죽은 화신의 항목은 되살아나지 않는다"
        );
        assert_eq!(t.len(), 1, "한 id 에 항목 하나");
    }

    #[test]
    fn last_signal_refreshes_on_every_observation() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        let t0 = Instant::now();
        t.observe_at(id, 0, 1, TurnSignal::Progress, t0);
        t.observe_at(
            id,
            0,
            1,
            TurnSignal::Progress,
            t0 + Duration::from_secs(20 * 60),
        );
        assert_eq!(
            t.get(id, 0).expect("관측됨").last_signal,
            t0 + Duration::from_secs(20 * 60),
            "갱신 없이 턴 시작 시각을 고정하면 정상 장기 턴이 소비자 상한에 잘린다"
        );
    }

    #[test]
    fn in_turn_snapshot_lists_only_agents_currently_in_a_turn() {
        let t = TurnObservations::new();
        let busy = AgentId::new_v4();
        let done_id = AgentId::new_v4();
        let t0 = Instant::now();
        t.observe_at(busy, 3, 1, TurnSignal::Progress, t0);
        t.observe_at(done_id, 0, 1, TurnSignal::Ended(TurnEndKind::Clean), t0);
        assert_eq!(t.in_turn_snapshot(), vec![(busy, 3, t0)]);
    }

    #[test]
    fn a_signal_that_arrives_out_of_publication_order_is_ignored() {
        // ★Order inversion regression★: if a progress signal (seq N) is applied **later** than the end signal (seq N+1), the ended turn
        //   returns to in-progress, and mail addressed to that receiver is parked again up to the bound.
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        let t0 = Instant::now();
        t.observe_at(id, 0, 5, TurnSignal::Progress, t0);
        t.observe_at(id, 0, 6, TurnSignal::Ended(TurnEndKind::Clean), t0);
        assert!(!t.is_in_turn(id, 0));

        // The progress signal of seq 5 that arrived late — must be ignored.
        t.observe_at(id, 0, 5, TurnSignal::Progress, t0 + Duration::from_secs(1));
        assert!(
            !t.is_in_turn(id, 0),
            "늦게 적용된 옛 신호가 끝난 턴을 되살리면 안 된다"
        );
        assert_eq!(
            t.get(id, 0).expect("관측됨").last_signal,
            t0,
            "밀려난 신호는 상한 축을 다시 찍지 않는다"
        );

        // The guard does not block even normal progress.
        t.observe_at(id, 0, 7, TurnSignal::Progress, t0 + Duration::from_secs(2));
        assert!(t.is_in_turn(id, 0));
    }

    #[test]
    fn an_accepted_signal_never_rewinds_last_signal() {
        // ★The only defense of `at.max(cur.last_signal)`★: the time is stamped **outside** the lock, so a late-stamped signal may
        //   be applied first — even then the bound axis must not be rewound (if rewound, it misjudges wreckage that much
        //   earlier). The seq test above sweeps only the **rejected** side, so it cannot protect this line.
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        let t0 = Instant::now();
        let later = t0 + Duration::from_secs(1);

        // seq 5 is applied first carrying a later time.
        t.observe_at(id, 0, 5, TurnSignal::Progress, later);
        // seq 6 is accepted normally (a newer emit) but its time is earlier.
        t.observe_at(id, 0, 6, TurnSignal::Ended(TurnEndKind::Clean), t0);

        assert!(!t.is_in_turn(id, 0), "수용된 신호의 상태는 반영된다");
        assert_eq!(
            t.get(id, 0).expect("관측됨").last_signal,
            later,
            "수용 경로도 상한 축을 되감지 않는다(`at.max(cur.last_signal)`)"
        );
    }

    #[test]
    fn seq_comparison_does_not_leak_across_incarnations() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        let t0 = Instant::now();
        t.observe_at(id, 0, 900, TurnSignal::Progress, t0);
        t.register_at(id, 1, t0);
        t.observe_at(id, 1, 1, TurnSignal::Progress, t0);
        assert!(
            t.is_in_turn(id, 1),
            "새 화신의 낮은 seq 가 옛 화신의 높은 seq 때문에 버려지면 그 에이전트는 영구 미관측이 된다"
        );
    }

    // Registration itself makes no fact — if a backend that emits no signal at all rose into the bound sweep, a false doorbell would ring
    // every 60 seconds until it dies.
    #[test]
    fn registration_alone_is_not_a_turn() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.register(id, 7);
        assert!(!t.is_in_turn(id, 7));
        assert!(t.in_turn_snapshot().is_empty());
        t.forget(id, 7);
        assert_eq!(t.len(), 0, "등록분도 finish 가 거둔다");
    }

    #[test]
    fn forget_removes_only_the_matching_incarnation() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.observe(id, 1, 1, TurnSignal::Progress);
        // A late reap (old epoch) — must not erase the live incarnation's observation.
        t.forget(id, 0);
        assert!(t.is_in_turn(id, 1));
        t.forget(id, 1);
        assert_eq!(t.get(id, 1), None);
        assert_eq!(t.len(), 0);
    }

    // ── Stall after error — `last_end_failed` folding (ADR-0231) ──

    fn failed_after_end(t: &TurnObservations, id: AgentId) -> bool {
        t.get(id, 0).expect("관측됨").last_end_failed
    }

    #[test]
    fn an_error_held_through_the_end_or_an_error_end_sets_the_halt() {
        let t = TurnObservations::new();
        let claude = AgentId::new_v4();
        t.register(claude, 0);
        t.observe(claude, 0, 1, TurnSignal::Progress);
        t.observe(claude, 0, 2, TurnSignal::Failed);
        t.observe(claude, 0, 3, TurnSignal::Ended(TurnEndKind::Clean));
        assert!(failed_after_end(&t, claude), "오류를 쥔 채 온 끝 = 오류 끝");

        let codex = AgentId::new_v4();
        t.register(codex, 0);
        t.observe(codex, 0, 1, TurnSignal::Progress);
        t.observe(codex, 0, 2, TurnSignal::Ended(TurnEndKind::Failed));
        assert!(failed_after_end(&t, codex));
    }

    #[test]
    fn the_error_signal_alone_neither_halts_nor_touches_in_turn() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.register(id, 0);
        t.observe(id, 0, 1, TurnSignal::Progress);
        t.observe(id, 0, 2, TurnSignal::Failed);
        let o = t.get(id, 0).expect("관측됨");
        assert!(o.in_turn, "오류 신호가 턴을 끝내지 않는다");
        assert!(!o.last_end_failed, "끝을 보기 전엔 멈춤이 서지 않는다");

        let idle = AgentId::new_v4();
        t.register(idle, 0);
        t.observe(idle, 0, 1, TurnSignal::Failed);
        assert!(!t.is_in_turn(idle, 0), "오류 신호가 턴을 켜지 않는다");
    }

    #[test]
    fn the_next_clean_end_releases_and_an_other_end_keeps_the_halt() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.register(id, 0);
        t.observe(id, 0, 1, TurnSignal::Ended(TurnEndKind::Failed));
        t.observe(id, 0, 2, TurnSignal::Ended(TurnEndKind::Other));
        assert!(failed_after_end(&t, id), "끊긴 끝은 멈춤을 풀지 않는다");
        t.observe(id, 0, 3, TurnSignal::Ended(TurnEndKind::Clean));
        assert!(!failed_after_end(&t, id), "깨끗한 끝이 푼다");
        t.observe(id, 0, 4, TurnSignal::Ended(TurnEndKind::Other));
        assert!(
            !failed_after_end(&t, id),
            "끊긴 끝은 멈춤을 세우지도 않는다"
        );
    }

    #[test]
    fn an_error_in_the_releasing_turn_keeps_the_halt_and_the_held_error_does_not_leak() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.register(id, 0);
        t.observe(id, 0, 1, TurnSignal::Ended(TurnEndKind::Failed));
        t.observe(id, 0, 2, TurnSignal::Failed);
        t.observe(id, 0, 3, TurnSignal::Ended(TurnEndKind::Clean));
        assert!(failed_after_end(&t, id), "그 턴도 오류면 참 그대로");
        // The held error is cleared by that turn's end — if it leaked to the next turn, a clean turn would raise the stall again.
        t.observe(id, 0, 4, TurnSignal::Progress);
        t.observe(id, 0, 5, TurnSignal::Ended(TurnEndKind::Clean));
        assert!(!failed_after_end(&t, id));
    }

    #[test]
    fn a_held_error_wins_over_an_other_end() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.register(id, 0);
        t.observe(id, 0, 1, TurnSignal::Failed);
        t.observe(id, 0, 2, TurnSignal::Ended(TurnEndKind::Other));
        assert!(failed_after_end(&t, id));
    }

    #[test]
    fn late_writes_with_a_smaller_seq_leave_the_halt_alone() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.register(id, 0);
        t.observe(id, 0, 5, TurnSignal::Ended(TurnEndKind::Failed));
        t.observe(id, 0, 4, TurnSignal::Ended(TurnEndKind::Clean));
        assert!(
            failed_after_end(&t, id),
            "늦은 깨끗한 끝이 멈춤을 풀면 안 된다"
        );

        t.observe(id, 0, 6, TurnSignal::Ended(TurnEndKind::Clean));
        t.observe(id, 0, 5, TurnSignal::Failed);
        t.observe(id, 0, 7, TurnSignal::Ended(TurnEndKind::Clean));
        assert!(
            !failed_after_end(&t, id),
            "버려진 오류 신호가 쥐어져 다음 끝을 오류로 만들면 안 된다"
        );
    }

    // ★Cursor regression of the stall field★: if filtered by the progress cursor, an error end is discarded just because progress with a later seq was written first.
    #[test]
    fn an_error_end_applied_behind_a_newer_progress_still_sets_the_halt() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.register(id, 0);
        t.observe(id, 0, 6, TurnSignal::Progress);
        t.observe(id, 0, 5, TurnSignal::Ended(TurnEndKind::Failed));
        let o = t.get(id, 0).expect("관측됨");
        assert!(o.last_end_failed, "링 순서상 오류로 끝난 턴이다");
        assert!(
            o.in_turn,
            "뒤 seq 의 진행은 다음 턴이다 — 늦은 끝이 끄지 않는다"
        );
    }

    #[test]
    fn an_error_applied_behind_a_newer_progress_is_held_into_the_next_end() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.register(id, 0);
        t.observe(id, 0, 6, TurnSignal::Progress);
        t.observe(id, 0, 5, TurnSignal::Failed);
        t.observe(id, 0, 7, TurnSignal::Ended(TurnEndKind::Clean));
        assert!(
            failed_after_end(&t, id),
            "진행 커서에 밀린 오류가 버려지면 오류로 끝난 턴이 깨끗하게 접힌다"
        );
    }

    // An error issued after an end is neither folded nor cleared by that end — the next end folds it.
    #[test]
    fn an_error_issued_after_a_late_end_is_left_for_the_next_end() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.register(id, 0);
        t.observe(id, 0, 7, TurnSignal::Failed);
        t.observe(id, 0, 6, TurnSignal::Ended(TurnEndKind::Clean));
        assert!(!failed_after_end(&t, id), "앞 턴은 깨끗하게 끝났다");
        t.observe(id, 0, 8, TurnSignal::Ended(TurnEndKind::Clean));
        assert!(failed_after_end(&t, id), "그 오류는 다음 턴의 끝이 접는다");
    }

    #[test]
    fn forget_drops_the_halt_and_a_new_incarnation_starts_clear() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.register(id, 0);
        t.observe(id, 0, 1, TurnSignal::Ended(TurnEndKind::Failed));
        t.forget(id, 0);
        assert_eq!(t.get(id, 0), None);
        t.register(id, 1);
        assert!(!t.get(id, 1).expect("등록됨").last_end_failed);
    }
}
