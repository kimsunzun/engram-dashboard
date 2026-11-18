//! turn — a **turn-progress observation table** per (agent, epoch): the agent "fact" layer owned by the core (ADR-0113 decision 1 ·
//!   ADR-0119 decision 4 — next to the registry).
//!
//! ★Owns observed facts only★: "is this incarnation in a turn now" · "when was the last turn signal" · "was the last turn end an error
//!   with no clean end after it yet" (stall after error — ADR-0231). **Interpretation is the consumers' job** — fallbacks, bounds, doorbells
//!   (ADR-0113 decision 2); a misjudgment costs differently per consumer (mail's worst case is "not delivered", so it falls back toward
//!   idle; input locking wants another bound), and policy here would force one consumer's judgment on all.
//!
//! ★The event→signal **mapping is not here** (ADR-0004/0110 decision 4)★: which output event is turn progress is backend knowledge
//!   (`backend::AgentBackend::turn_classifier`); this module knows only the signal vocabulary (`TurnSignal`) and accumulation.
//!
//! ★The write path = `OutputCore::emit` alone, but with **two callers** (load-bearing)★: ① the output pump thread ② the input echo path
//!   (`AgentSession::write_input_observed` — the injecting thread: the mail flush lane's blocking thread · the MCP HTTP handler · the Tauri
//!   command thread). A "single thread" argument does not hold. Instead: the table's mutex is a **leaf** (never held with another lock, no
//!   outside call while held, so no cycle in the lock order — ADR-0006), and the critical section is a few hash-map operations (mandatory:
//!   one caller is the **output pump**, whose whole output stream would back up behind a longer one).
//!
//! ★Lock = this table only★: entries are keyed by `AgentId` with the epoch as a **value**, one map; an epoch in the key would need a
//!   second table for 「this id's current epoch」 and with it an order rule between two locks.
//!
//! ★One entry per id★: there is exactly one live incarnation. The only verb that takes its place is [`TurnObservations::register`] (owner
//!   = `AgentManager::spawn_session`); the main cleanup is `forget` (owner = `OutputCore::finish`).
//!   ★Markers are only guaranteed **different from each other** — order has no meaning★: every spawn passes the single issuing point
//!   `AgentManager::spawn_agent` (`ProfileRegistry::epoch_for_spawn`), drawing a random number guaranteed different from the previous one.
//!   **Do not scatter issuing** — when it was scattered, Fresh respawns reused markers and two incarnations sat under one key.
//!   ★So comparison is **equal/unequal** only — never which is larger (do not revive it)★: `observe_at` once dropped late signals whose
//!   marker was smaller; with random markers **half the time the dying incarnation's marker was larger**, passed, and overwrote the live
//!   entry — the live
//!   incarnation then answered unobserved (= not in a turn) and **mail went in mid-turn**. Restricting the authority to change the place
//!   to `register` closes it: a late signal is discarded because its marker does not match.
// ADR-0113
// ADR-0006

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use engram_dashboard_base::sync;

use crate::types::AgentId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnSignal {
    /// The backend that emitted that event classified it as "this incarnation is in a turn".
    Progress,
    /// This turn hit an error — ★does not change `in_turn`★. The table holds it and folds it into `last_end_failed` at that turn's `Ended`
    ///   (the stall does not stand before the end is seen).
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

/// Turn-observation snapshot of one (agent, epoch). Absence (`Option::None`) is **unobserved**, not idle — how to treat the two is the
///   consumer's decision (module header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnObservation {
    pub in_turn: bool,
    /// The time the last turn signal was observed — the axis on which a consumer judges "how old is this observation" (stuck-turn
    ///   detection etc.).
    pub last_signal: Instant,
    /// This incarnation's last turn end was an error with no clean success end after it yet (stall after error). An end of kind 「other」
    ///   does not change it. Right after registration = false.
    // ADR-0231
    pub last_end_failed: bool,
}

/// Turn observation table — one owned by `AgentManager`, shared by all of its `OutputCore`s.
///
/// ★Observation target = "sessions that actually emit turn signals" (why no separate capability gate)★: whether to emit is that session's
///   backend classifier's decision (`backend::turn_classifier`; undeclared = silent). Filtering again by the "structured output
///   capability" would make
///   the core retrace session assembly information, and when the two criteria diverge it would be unclear which is canonical.
pub struct TurnObservations {
    entries: Mutex<HashMap<AgentId, Entry>>,
}

struct Entry {
    epoch: u32,
    in_turn: bool,
    last_signal: Instant,
    /// The **output seq** that last updated this entry (comparable only within the same epoch — a new incarnation counts from 0). Filters
    ///   late signals when emit order and apply order diverge (`observe_at`).
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

    /// ★Poison tolerance (same judgment as the sessions lock in reaper.rs)★: this is a **manager-global** table, so one holder panic would
    ///   re-panic the turn-signal emit of **every** agent in a chain — unlike the core's per-agent locks (whose `expect` is justified by
    ///   "does not
    ///   propagate to other agents"). It holds only a hash map (no invariant to break), so recover the guard and go on. (In production the
    ///   only case this covers is a contained pump panic while holding the lock — elsewhere the hook ends the process · ADR-0292 ·
    ///   command execution is not contained · ADR-0293.)
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<AgentId, Entry>> {
        sync::lock(&self.entries)
    }

    /// This incarnation takes this id's place — **unconditionally** replaces any entry.
    ///
    /// ★The only caller is `AgentManager::spawn_session`, **before the session becomes observable** (before the sessions map insert =
    ///   before pump start). This order underpins the table's correctness★: registered after the incarnation's first signal, that signal
    ///   is discarded for
    ///   mismatching the old entry's marker and the incarnation answers **permanently unobserved** (= not in a turn) until the old entry
    ///   is `forget`-ed — mail injected mid-turn.
    /// ★Ordering against the dying previous incarnation is safe either way★: its `forget` first → empty, we fill it; ours first → its
    ///   `forget` is a no-op (marker differs) and its late signal is discarded likewise.
    /// ★The value put in is "no signal yet"★ (`in_turn=false`): the table holds facts only, so registration is not a turn; a backend
    ///   emitting no signal is not in `in_turn_snapshot`, and `finish` clears it.
    pub fn register(&self, id: AgentId, epoch: u32) {
        self.register_at(id, epoch, Instant::now());
    }

    pub fn register_at(&self, id: AgentId, epoch: u32, at: Instant) {
        self.lock().insert(id, Entry::fresh(epoch, at));
    }

    /// Applies one turn signal (observation time = now). `seq` = that event's output sequence (canonical emit order — the order-inversion
    ///   defense axis of `observe_at`).
    pub fn observe(&self, id: AgentId, epoch: u32, seq: u64, signal: TurnSignal) {
        self.observe_at(id, epoch, seq, signal, Instant::now());
    }

    /// Time-injected — tests drive the bound boundary · time refresh · order inversion without real-time waits.
    ///
    /// ★Filters late signals of an old incarnation (secondary defense)★: right after a swap the old `OutputCore` drains queued events for
    ///   a moment; accepting such a signal revives an entry destined to vanish, whose end signal **never comes** — a ghost entry no one
    ///   can erase.
    /// ★The blocked range is narrow (no overclaiming)★: late signals with the **same** marker — notably an input echo arriving **after**
    ///   `forget` — cannot be blocked here; the caller (`OutputCore::emit`) blocks them with the finalize flag (its comment is canonical),
    ///   the main defense.
    /// ★Marker differs → **discard unconditionally**, never compare size★ (random number; `AgentProfile::epoch`). Only [`Self::register`]
    ///   changes the place, and registering before the live incarnation's first signal is what makes this safe (that function's order
    ///   paragraph).
    /// ★No entry at all → accept★: assemblies skipping registration (harnesses · hand-built `OutputCore`) stay observable. In production a
    ///   late signal after `finish` may enter here, and the caller's finalize recheck takes it back out.
    ///
    /// ★Within the same marker, **seq sets the order** (load-bearing)★: emit has two callers (pump · the injecting thread of the input
    ///   echo), so **emit order and apply order can diverge** — the injecting thread emits progress seq N and pauses, the pump applies the
    ///   end seq N+1, and
    ///   the late progress would **turn the ended turn back to in-progress** (mail to that receiver parked again up to the bound). seq is
    ///   issued inside the replay lock (output_core.rs emit) and is the canonical order, so lower seqs are ignored.
    /// ★seq is valid **only within the same marker**★: a new incarnation is a new `OutputCore` counting from 0, so seq is not looked at
    ///   when markers differ (the entry is replaced by [`Self::register`]).
    /// ★Cost (honest)★: time is stamped outside the lock, so a displaced signal usually carries a **newer time**; the guard keeps
    ///   `last_signal` slightly **older**, so the bound may fire earlier by one scheduler quantum — meaningless at a 30-minute bound — the
    ///   price of making
    ///   **emit order** the axis (apply order would revive ended turns).
    ///
    /// ★Time is refreshed on every observation★: a long normal turn keeps receiving signals and does not age; only a stopped one ages (the
    ///   consumer's "stuck turn" axis).
    /// ★`last_end_failed` folding (both backends — `TurnEndKind`)★: at `Ended`, true if an error end or a held `Failed` with a smaller seq
    ///   · false if a clean end · else unchanged; the held `Failed` is cleared by that `Ended`.
    /// ★The stall field is filtered by **its own cursor** (`last_end_seq`), not the progress cursor (`last_seq`)★: with `Failed` seq N and
    ///   another thread's progress N+1 written first, the progress cursor would discard that `Failed` and the later `Ended(Clean)` (N+2)
    ///   would fold an
    ///   error-ended turn as clean (mail goes in without a stall after error). So `Failed` · `Ended` are discarded only if their seq is
    ///   below the last folded `Ended`; `Failed` is held (earliest seq) and folded by a larger `Ended`.
    ///   `in_turn` · `last_seq` · `last_signal` change only for the newest signal — later-seq progress is the next turn.
    /// ★Remaining gap (honest)★: a `Failed` with a seq below the folded `Ended` is discarded if written **after** it — no previous-end seq
    ///   tells which end it belongs to. It does not occur while one thread (the output pump) emits `Failed` and that turn's `Ended`.
    /// ★Residue not blocked now★: two incarnations' markers colliding at 2^-32 disable this equality check for that incarnation's lifetime
    ///   (`ProfileRegistry::epoch_for_spawn` — adjacency holds only within one process); the outcome is "in a turn but looks idle, mail goes in
    ///   early", which the project chose over "mail that never goes" (ADR-0104).
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
            // ★Do not rewind `last_signal`★: time is stamped **outside** the lock (`observe` above), so a late-stamped signal may apply
            //   first; rewinding would make the bound misjudge wreckage earlier — free to close.
            e.last_signal = at.max(e.last_signal);
            e.last_seq = seq;
        }
    }

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

    /// Whether it is in a turn by observation. ★Unobserved is false★ — the fact layer does not turn 「unknown」 into true (the consumer
    ///   picks its own fallback).
    pub fn is_in_turn(&self, id: AgentId, epoch: u32) -> bool {
        self.get(id, epoch).is_some_and(|o| o.in_turn)
    }

    pub fn in_turn_snapshot(&self) -> Vec<(AgentId, u32, Instant)> {
        let g = self.lock();
        g.iter()
            .filter(|(_, e)| e.in_turn)
            .map(|(id, e)| (*id, e.epoch, e.last_signal))
            .collect()
    }

    /// Removes this incarnation's entry. ★Only when the epoch matches★ — a late call must not erase the observation of an incarnation
    ///   attached in between (as the reaper's sessions removal guard).
    ///
    /// ★If not erased★: the "in a turn" mark of an agent that died mid-turn stays and a consumer waiting on that name (mail parking etc.)
    ///   is never released.
    /// ★Owner = `OutputCore::finish`★ (its comment is canonical, including the race with a late emit after termination). **Intentionally
    ///   two call sites**: `finish` and the finalize recheck path of `emit` (taking back what it just put in; both through the same table
    ///   mutex is what closes the race).
    /// **A third caller** would split that causality — deleting the emit side as a "forbidden second caller" reopens the ghost-entry hole.
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
        assert!(t.get(id, 0).is_some(), "the observed fact stays after exit");
    }

    #[test]
    fn observation_is_keyed_by_epoch() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.observe(id, 0, 1, TurnSignal::Progress);
        assert!(t.is_in_turn(id, 0));
        assert_eq!(t.get(id, 1), None, "a different epoch is unobserved");
    }

    #[test]
    fn registration_replaces_the_entry_and_older_signals_are_dropped() {
        // If a late signal revives an old incarnation, no one can erase its entry (its end signal never comes).
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.observe(id, 0, 1, TurnSignal::Progress);
        t.register(id, 1);
        assert_eq!(t.get(id, 0), None, "registration cleans the old entry");
        assert_eq!(t.len(), 1, "dead entries do not pile up with each restart");
        assert!(!t.is_in_turn(id, 1), "registration alone is not a turn");
        t.observe(id, 1, 1, TurnSignal::Progress);
        assert!(t.is_in_turn(id, 1));

        t.observe(id, 0, 1, TurnSignal::Progress);
        assert_eq!(
            t.get(id, 0),
            None,
            "a late signal with an old marker is discarded"
        );
        assert!(
            t.is_in_turn(id, 1),
            "the current incarnation's observation stays as is"
        );
    }

    // ★Regression target — the half after the marker became a random number★: the old guard "drop if the current marker is larger" passed
    //   when the dying incarnation's marker was larger, so the late signal **overwrote the live entry** (mail went in mid-turn). Hence the
    //   two markers
    //   here are **deliberately inverted** (live 5 < dead 900) — reverting to a size comparison breaks it.
    #[test]
    fn a_late_signal_from_a_larger_tag_cannot_evict_the_live_incarnation() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        let dying = 900;
        let live = 5;

        t.observe(id, dying, 3, TurnSignal::Progress);
        t.register(id, live);
        t.observe(id, live, 0, TurnSignal::Progress);
        assert!(
            t.is_in_turn(id, live),
            "premise: the live incarnation is observed as in a turn"
        );

        t.observe(id, dying, 4, TurnSignal::Ended(TurnEndKind::Clean));

        assert!(
            t.is_in_turn(id, live),
            "if a dead incarnation's late signal overwrites the live incarnation's observation, mail goes in during a turn"
        );
        assert_eq!(
            t.get(id, dying),
            None,
            "a dead incarnation's entry is not revived"
        );
        assert_eq!(t.len(), 1, "one entry per id");
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
            t.get(id, 0).expect("observed").last_signal,
            t0 + Duration::from_secs(20 * 60),
            "if the turn start time is fixed without updates, a normal long turn is cut off at the consumer cap"
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
        // ★Order inversion regression★: a progress signal (seq N) applied **after** the end signal (seq N+1) would return the ended turn
        //   to in-progress, parking mail to that receiver again up to the bound.
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        let t0 = Instant::now();
        t.observe_at(id, 0, 5, TurnSignal::Progress, t0);
        t.observe_at(id, 0, 6, TurnSignal::Ended(TurnEndKind::Clean), t0);
        assert!(!t.is_in_turn(id, 0));

        t.observe_at(id, 0, 5, TurnSignal::Progress, t0 + Duration::from_secs(1));
        assert!(
            !t.is_in_turn(id, 0),
            "an old signal applied late must not revive a finished turn"
        );
        assert_eq!(
            t.get(id, 0).expect("observed").last_signal,
            t0,
            "a superseded signal does not stamp the cap axis again"
        );

        t.observe_at(id, 0, 7, TurnSignal::Progress, t0 + Duration::from_secs(2));
        assert!(t.is_in_turn(id, 0));
    }

    #[test]
    fn an_accepted_signal_never_rewinds_last_signal() {
        // ★The only defense of `at.max(cur.last_signal)`★: time is stamped **outside** the lock, so a late-stamped signal may apply first
        //   and must not rewind the bound axis. The seq test above sweeps only the **rejected** side and cannot protect this line.
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        let t0 = Instant::now();
        let later = t0 + Duration::from_secs(1);

        t.observe_at(id, 0, 5, TurnSignal::Progress, later);
        t.observe_at(id, 0, 6, TurnSignal::Ended(TurnEndKind::Clean), t0);

        assert!(
            !t.is_in_turn(id, 0),
            "the state of an accepted signal is applied"
        );
        assert_eq!(
            t.get(id, 0).expect("observed").last_signal,
            later,
            "the acceptance path does not rewind the cap axis either (`at.max(cur.last_signal)`)"
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
            "if a new incarnation's low seq is discarded because of an old incarnation's high seq, that agent becomes permanently unobserved"
        );
    }

    // Registration itself makes no fact — a backend that emits no signal rising into the bound sweep would ring a false doorbell every 60
    //   s until it dies.
    #[test]
    fn registration_alone_is_not_a_turn() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.register(id, 7);
        assert!(!t.is_in_turn(id, 7));
        assert!(t.in_turn_snapshot().is_empty());
        t.forget(id, 7);
        assert_eq!(t.len(), 0, "finish also collects registered entries");
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
        t.get(id, 0).expect("observed").last_end_failed
    }

    #[test]
    fn an_error_held_through_the_end_or_an_error_end_sets_the_halt() {
        let t = TurnObservations::new();
        let claude = AgentId::new_v4();
        t.register(claude, 0);
        t.observe(claude, 0, 1, TurnSignal::Progress);
        t.observe(claude, 0, 2, TurnSignal::Failed);
        t.observe(claude, 0, 3, TurnSignal::Ended(TurnEndKind::Clean));
        assert!(
            failed_after_end(&t, claude),
            "an end that came while holding an error = an error end"
        );

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
        let o = t.get(id, 0).expect("observed");
        assert!(o.in_turn, "an error signal does not end the turn");
        assert!(
            !o.last_end_failed,
            "the stop does not stand before the end is seen"
        );

        let idle = AgentId::new_v4();
        t.register(idle, 0);
        t.observe(idle, 0, 1, TurnSignal::Failed);
        assert!(
            !t.is_in_turn(idle, 0),
            "an error signal does not turn a turn on"
        );
    }

    #[test]
    fn the_next_clean_end_releases_and_an_other_end_keeps_the_halt() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.register(id, 0);
        t.observe(id, 0, 1, TurnSignal::Ended(TurnEndKind::Failed));
        t.observe(id, 0, 2, TurnSignal::Ended(TurnEndKind::Other));
        assert!(
            failed_after_end(&t, id),
            "a cut-off end does not clear the stop"
        );
        t.observe(id, 0, 3, TurnSignal::Ended(TurnEndKind::Clean));
        assert!(!failed_after_end(&t, id), "a clean end clears it");
        t.observe(id, 0, 4, TurnSignal::Ended(TurnEndKind::Other));
        assert!(
            !failed_after_end(&t, id),
            "a cut-off end does not set the stop either"
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
        assert!(
            failed_after_end(&t, id),
            "if that turn is also an error, it stays true"
        );
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
            "a late clean end must not clear the stop"
        );

        t.observe(id, 0, 6, TurnSignal::Ended(TurnEndKind::Clean));
        t.observe(id, 0, 5, TurnSignal::Failed);
        t.observe(id, 0, 7, TurnSignal::Ended(TurnEndKind::Clean));
        assert!(
            !failed_after_end(&t, id),
            "a discarded error signal must not be held and make the next end an error"
        );
    }

    // ★Cursor regression of the stall field★: if filtered by the progress cursor, an error end is discarded just because progress with a
    //   later seq was written first.
    #[test]
    fn an_error_end_applied_behind_a_newer_progress_still_sets_the_halt() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.register(id, 0);
        t.observe(id, 0, 6, TurnSignal::Progress);
        t.observe(id, 0, 5, TurnSignal::Ended(TurnEndKind::Failed));
        let o = t.get(id, 0).expect("observed");
        assert!(
            o.last_end_failed,
            "in ring order it is a turn that ended with an error"
        );
        assert!(
            o.in_turn,
            "progress at a later seq is the next turn — a late end does not turn it off"
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
            "if an error pushed back by the progress cursor is discarded, a turn that ended with an error folds cleanly"
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
        assert!(!failed_after_end(&t, id), "the previous turn ended cleanly");
        t.observe(id, 0, 8, TurnSignal::Ended(TurnEndKind::Clean));
        assert!(
            failed_after_end(&t, id),
            "that error is folded by the next turn's end"
        );
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
        assert!(!t.get(id, 1).expect("registered").last_end_failed);
    }
}
