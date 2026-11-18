//! OutputRouter — agent_id → window-label routing table (lock-free hot path) + subscription union diff. Pure logic, zero Tauri runtime dependency (headless test possible).
//!
//! Routing table for fanning out a daemon output frame (`DecodedFrame{agent_id, ...}`) to all windows (labels) where that agent is currently visible. The authoritative source is `ViewManager` (ADR-0035/0057): across the split trees of all tabs of each window (hidden tabs too) it finds the Slot holding the agent and reverse-indexes `agent_id → [window_label]`.
//!
//! Routing reversal (ADR-0057/0056 keep-alive): all tabs of each window are walked (hidden tabs receive too, switch lossless — ADR-0056); the `active_view_id` · `MAIN_WINDOW_LABEL` routing branch is gone — the active flag no longer enters `by_agent`. With unique ownership (ADR-0057 invariant 2) a View is in one window only, so no double counting.
//!
//! Invariants:
//! - Hot path (`targets`) takes no lock (ADR-0006): `ArcSwap::load()` only — called per frame. Allocation · recomputation belong to the low-frequency `rebuild`.
//! - rebuild-always (D2): layout changes are rare, so every change recomputes the whole snapshot and swaps it atomically with `ArcSwap::store`; no version-cache branch (no gain against complexity).
//! - AgentKey = `AgentId` (`uuid::Uuid`): the frame's `agent_id` is already one, so zero conversion on the hot path. A Slot stores `SlotContent::Agent{agent_id: String}` (ADR-0060), so String→Uuid parsing happens at rebuild (low-frequency); an unparseable slot can never match a real frame and is ignored. Consistent with `protocol_state`'s `HashMap<AgentId, SubState>` key.
//! - F-B subscription union = layout-derived (ADR-0035, spike §8 F-B): with no separate ref-count map, the snapshot's agent set is "the set that should be subscribed now"; rebuild diffs it against the previous set and produces the 0→1 (Subscribe) · 1→0 (Unsubscribe) deltas in the same tree walk.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use arc_swap::ArcSwap;

use crate::layout::manager::ViewManager;
use crate::layout::types::{LayoutNode, SlotContent};

// Cached empty routing result (returned on a miss): `Arc::from(Vec::new())` allocates an Arc header on every call, so frequent hot-path misses would accumulate cost. Built once and returned via `Arc::clone` (refcount +1, zero allocation) — a truly zero-alloc miss path (FIX-2).
static EMPTY_TARGETS: LazyLock<Arc<[WindowLabel]>> = LazyLock::new(|| Arc::from(Vec::new()));

pub type AgentKey = uuid::Uuid;

// Tauri window label (e.g. "main", "slot-popup-0a1b2c3d-3"); same type as the `ViewManager.windows` key, so no separate numeric registry (spike §8 D1).
pub type WindowLabel = String;

// Values are shared as `Arc<[WindowLabel]>` — cloning on the hot path is a pointer copy. An agent not in `by_agent` is visible in no window → `targets` returns an empty slice.
#[derive(Debug, Default)]
pub struct RoutingSnapshot {
    pub by_agent: HashMap<AgentKey, Arc<[WindowLabel]>>,
}

// Subscription delta of one rebuild (F-B): the agent-union diff against the previous snapshot in a single tree walk.
// - `to_subscribe`: newly visible this time (0→1). ADR-0046: layout does not send a wire Subscribe with this; it is still produced for the diff-preservation invariant test / future diagnostics.
// - `to_unsubscribe`: no longer visible in any window (1→0) → the router emits a wire `Unsubscribe` (cleanup).
// ADR-0046 axis B removed: the old slot=(window,agent) cursor deltas (`slots_to_replay`/`slots_to_drop`) went with the mirror buffer; remount/new window is a full re-replay of the daemon ring (view-driven).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SubscriptionDelta {
    pub to_subscribe: Vec<AgentKey>,
    pub to_unsubscribe: Vec<AgentKey>,
}

impl SubscriptionDelta {
    pub fn is_empty(&self) -> bool {
        self.to_subscribe.is_empty() && self.to_unsubscribe.is_empty()
    }
}

pub struct OutputRouter {
    table: ArcSwap<RoutingSnapshot>,
}

impl Default for OutputRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputRouter {
    pub fn new() -> Self {
        Self {
            table: ArcSwap::from_pointee(RoutingSnapshot::default()),
        }
    }

    // Hot path: all window labels where the agent is currently visible. Consume the returned Arc immediately (carrying a `load()` Guard across an `.await` exhausts slots — here the Guard lives only inside the function and only the Arc is returned; spike §7 Pitfall).
    pub fn targets(&self, agent_id: AgentKey) -> Arc<[WindowLabel]> {
        let snap = self.table.load();
        match snap.by_agent.get(&agent_id) {
            Some(labels) => Arc::clone(labels),
            None => Arc::clone(&EMPTY_TARGETS),
        }
    }

    // `pub(crate)` so other test modules in the crate (daemon_client tests) can call it. ADR-0046: the old eager resubscribe (iterating current_agents) was deleted; the helper stays because routing (targets) tests can reuse it (currently unused → allow).
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn set_visible_agents_for_test(&self, agents: &[AgentKey]) {
        let mut by_agent: HashMap<AgentKey, Arc<[WindowLabel]>> = HashMap::new();
        for a in agents {
            by_agent.insert(*a, Arc::from(vec!["main".to_string()]));
        }
        self.table.store(Arc::new(RoutingSnapshot { by_agent }));
    }

    // Low-frequency: recompute the whole routing table from a ViewManager snapshot and swap it atomically.
    // Call contract (serialization — FIX-1): call it while holding the ViewManager lock (inside the same critical section as the layout mutation). Inside is the RMW `load(prev) → delta computation → store(new)`; called concurrently outside the lock (Tauri thread pool) the deltas diverge — duplicate Subscribe · missing Unsubscribe · ABA (a stale store overwriting a new one). The lock serializes the RMW and guarantees `&mgr` is the current state.
    // ADR-0006: calling inside the lock is not a violation — the body is pure computation + lock-free `ArcSwap::store` with no external calls (emit / DaemonClient / network I/O). Enqueueing the returned delta is also inside the same lock (wiring = the `layout::apply::SubscriptionSync::resync` implementation + the two cleanup paths of `commands::popout` — no exceptions); deferred outside, a re-add between compute and fire lets a stale unsubscribe kill a live subscription.
    pub fn rebuild(&self, mgr: &ViewManager) -> SubscriptionDelta {
        let mut by_agent: HashMap<AgentKey, Vec<WindowLabel>> = HashMap::new();

        for (label, wt) in &mgr.windows {
            let windows = [label.as_str()];
            for vid in &wt.tabs {
                // By the unique ownership invariant each vid is a real View (no orphans). Defensively, a lookup failure is skipped.
                if let Some(view) = mgr.views.get(vid) {
                    collect_agents(&view.layout, &windows, &mut by_agent);
                }
            }
        }

        let mut snapshot = RoutingSnapshot {
            by_agent: HashMap::with_capacity(by_agent.len()),
        };
        let mut new_set: HashSet<AgentKey> = HashSet::with_capacity(by_agent.len());
        for (agent, mut labels) in by_agent {
            // Label sort — diff/test determinism (HashMap iteration order is nondeterministic). Irrelevant to hot-path semantics.
            labels.sort();
            labels.dedup();
            new_set.insert(agent);
            snapshot.by_agent.insert(agent, Arc::from(labels));
        }

        // Diff against the previous snapshot (F-B). Read the old set before the store.
        let prev = self.table.load();
        let prev_set: HashSet<AgentKey> = prev.by_agent.keys().copied().collect();
        // `HashSet::difference` order is nondeterministic — sort like `labels.sort()` to fix delta order (test reproducibility + stable send order, FIX-4).
        let mut to_subscribe: Vec<AgentKey> = new_set.difference(&prev_set).copied().collect();
        let mut to_unsubscribe: Vec<AgentKey> = prev_set.difference(&new_set).copied().collect();
        to_subscribe.sort();
        to_unsubscribe.sort();

        let delta = SubscriptionDelta {
            to_subscribe,
            to_unsubscribe,
        };

        self.table.store(Arc::new(snapshot));
        delta
    }
}

// Window cleanup core (Tauri-free — G1 headless required, TRD §8 stage 1): drops all tab Views of window `label` (views + view_owner + windows entry), runs `rebuild` once and returns that delta. The caller fires the returned `to_unsubscribe` (agents remaining in no window) to the daemon inside the lock (F1).
// Knows nothing of Tauri · DaemonClient · registry — only `ViewManager` (model) + `OutputRouter` (routing). `commands::popout::drop_window_in_model` (close request · `Destroyed` cleanup) calls it inside the ViewManager lock and handles firing the delta · releasing the closing window's number in the same lock; detaching the registry (the Tauri part) is `cleanup_popup_window`'s job.
// - `label == MAIN_WINDOW_LABEL`: not a cleanup target (invariant 4) — no-op, empty delta (defensive; the caller blocks it too).
// - `label` not in `windows` (the normal path: close_tab · window close (`apply::close_window`) or a popout close request deleted the model first): skip close, `rebuild` once (the table is recomputed by contract); Destroyed is an OS event after that.
// - `label` in `windows` (a popout close request · a `Destroyed` without a close request — multi-tab residual risk, G1): drop all tabs then `rebuild`. The window number is decided by `cause` — on a close request the OS window still exists so it is held until its `Destroyed` (`close_window`); on `Destroyed` it is released at once (`drop_window_gone` — TRD S26-window-chrome §4-1).
pub fn cleanup_window_core(
    mgr: &mut ViewManager,
    router: &OutputRouter,
    label: &str,
    cause: DropCause,
) -> SubscriptionDelta {
    use crate::layout::manager::MAIN_WINDOW_LABEL;
    if label == MAIN_WINDOW_LABEL {
        return SubscriptionDelta::default();
    }
    if mgr.windows.contains_key(label) {
        // It could give a main-rejection Err, but main was already filtered above, so only success here.
        let _ = match cause {
            DropCause::CloseRequested => mgr.close_window(label),
            DropCause::Destroyed => mgr.drop_window_gone(label),
        };
    }
    router.rebuild(mgr)
}

/// TRD S26-window-chrome §4-1
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropCause {
    CloseRequested,
    Destroyed,
}

// Load-bearing routing invariant (ADR-0041/0042/0046 · ADR-0060): only an assigned slot (`SlotContent::Agent`) is a routing target — `SlotContent::Empty` is explicitly ignored. Content kind (SlotContent) and byte routing cross only here, extracting only the Agent's binding (agent_id).
fn collect_agents(
    node: &LayoutNode,
    windows: &[&str],
    by_agent: &mut HashMap<AgentKey, Vec<WindowLabel>>,
) {
    match node {
        LayoutNode::Slot { content, .. } => {
            if let SlotContent::Agent { agent_id } = content {
                match agent_id.parse::<AgentKey>() {
                    Ok(key) => {
                        let entry = by_agent.entry(key).or_default();
                        for w in windows {
                            entry.push((*w).to_string());
                        }
                    }
                    // Dropped silently, debugging clues vanish (ADR-0038); rebuild is low-frequency so the debug log costs the hot path 0 (logging-conventions).
                    Err(_) => {
                        tracing::debug!(agent_id = %agent_id, "rebuild: slot agent_id is not a UUID — skipping routing");
                    }
                }
            }
        }
        LayoutNode::Split { a, b, .. } => {
            collect_agents(a, windows, by_agent);
            collect_agents(b, windows, by_agent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::manager::MAIN_WINDOW_LABEL;
    use crate::layout::types::SplitDir;
    use uuid::Uuid;

    // ── helpers ─────────────────────────────────────────────────────────────

    fn agent() -> (Uuid, String) {
        let id = Uuid::new_v4();
        (id, id.to_string())
    }

    fn main_active(mgr: &ViewManager) -> Uuid {
        mgr.windows.get(MAIN_WINDOW_LABEL).unwrap().active
    }

    fn first_slot(mgr: &ViewManager, view_id: Uuid) -> Uuid {
        let v = mgr.views.get(&view_id).unwrap();
        crate::layout::tree::first_slot_id(&v.layout)
    }

    fn assign_to_main(mgr: &mut ViewManager, agent_str: &str) -> Uuid {
        let view_id = main_active(mgr);
        let slot = first_slot(mgr, view_id);
        mgr.assign_agent(view_id, slot, agent_str.to_string())
            .unwrap();
        slot
    }

    fn targets_set(router: &OutputRouter, agent: Uuid) -> Vec<String> {
        let mut v: Vec<String> = router.targets(agent).iter().cloned().collect();
        v.sort();
        v
    }

    // ── routing: targets returns the right window label ──────────────────────

    #[test]
    fn agent_in_main_tab_routes_to_main() {
        let mut mgr = ViewManager::new();
        let (aid, astr) = agent();
        assign_to_main(&mut mgr, &astr);

        let router = OutputRouter::new();
        router.rebuild(&mgr);

        assert_eq!(targets_set(&router, aid), vec!["main".to_string()]);
    }

    #[test]
    fn agent_absent_returns_empty() {
        let mgr = ViewManager::new(); // Only empty slots — assignments 0.
        let router = OutputRouter::new();
        router.rebuild(&mgr);
        let (aid, _) = agent();
        assert!(
            router.targets(aid).is_empty(),
            "an unassigned agent gives an empty result"
        );
    }

    #[test]
    fn agent_in_hidden_tab_still_routes_keep_alive() {
        let mut mgr = ViewManager::new();
        let hidden = main_active(&mgr);
        let (aid, astr) = agent();
        let hslot = first_slot(&mgr, hidden);
        mgr.assign_agent(hidden, hslot, astr).unwrap();
        let _active = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();

        let router = OutputRouter::new();
        router.rebuild(&mgr);
        assert_eq!(
            targets_set(&router, aid),
            vec!["main".to_string()],
            "an agent in a hidden tab is routed too (keep-alive)"
        );
    }

    #[test]
    fn agent_in_popup_window_routes_to_popup_label() {
        let mut mgr = ViewManager::new();
        let pv = mgr.create_window("slot-popup-1").unwrap();
        let (aid, astr) = agent();
        let slot = first_slot(&mgr, pv);
        mgr.assign_agent(pv, slot, astr).unwrap();

        let router = OutputRouter::new();
        router.rebuild(&mgr);
        assert_eq!(targets_set(&router, aid), vec!["slot-popup-1".to_string()]);
    }

    #[test]
    fn same_agent_in_two_windows_routes_to_both() {
        let mut mgr = ViewManager::new();
        let (aid, astr) = agent();
        assign_to_main(&mut mgr, &astr);
        let pv = mgr.create_window("slot-popup-1").unwrap();
        let slot2 = first_slot(&mgr, pv);
        mgr.assign_agent(pv, slot2, astr.clone()).unwrap();

        let router = OutputRouter::new();
        router.rebuild(&mgr);
        assert_eq!(
            targets_set(&router, aid),
            vec!["main".to_string(), "slot-popup-1".to_string()],
            "an agent visible in two windows is routed to both"
        );
    }

    #[test]
    fn split_view_with_two_agents_routes_each() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let slot = first_slot(&mgr, view_id);
        let slot2 = mgr.split_slot(view_id, slot, SplitDir::LeftRight).unwrap();
        let (a1, a1s) = agent();
        let (a2, a2s) = agent();
        mgr.assign_agent(view_id, slot, a1s).unwrap();
        mgr.assign_agent(view_id, slot2, a2s).unwrap();

        let router = OutputRouter::new();
        router.rebuild(&mgr);
        assert_eq!(targets_set(&router, a1), vec!["main".to_string()]);
        assert_eq!(targets_set(&router, a2), vec!["main".to_string()]);
    }

    #[test]
    fn invalid_uuid_slot_string_is_skipped() {
        let mut mgr = ViewManager::new();
        assign_to_main(&mut mgr, "not-a-uuid");
        let router = OutputRouter::new();
        let delta = router.rebuild(&mgr);
        assert!(delta.is_empty(), "an unparsable slot is not a subscription target either");
        let (probe, _) = agent();
        assert!(router.targets(probe).is_empty());
    }

    // ── lock-free reads (structural): no Mutex in OutputRouter ─────────────────
    // These tests verify only the safety of concurrent reads (targets/load) and store. Concurrent rebuild (two writers doing load→delta→store with no lock) is a contract violation (FIX-1: rebuild must be called only inside the ViewManager-lock critical section), so there is no concurrent-writer test; the serialized-sequence invariant is covered by `delta_conservation_over_sequence`.

    #[test]
    fn targets_is_lock_free_concurrent_reads() {
        // Structural assertion: targets takes no lock, so many threads read concurrently without blocking (the only field of OutputRouter is one ArcSwap — no Mutex/RwLock).
        use std::sync::Arc as StdArc;
        use std::thread;

        let mut mgr = ViewManager::new();
        let (aid, astr) = agent();
        assign_to_main(&mut mgr, &astr);
        let router = StdArc::new(OutputRouter::new());
        router.rebuild(&mgr);

        let mut handles = Vec::new();
        for _ in 0..8 {
            let r = StdArc::clone(&router);
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    let t = r.targets(aid);
                    assert_eq!(&*t, &["main".to_string()][..]);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn rebuild_concurrent_with_reads_is_safe() {
        // While rebuild (store) runs, another thread's targets (load) always sees a consistent snapshot (old or new), no data race — ArcSwap semantics: either empty or ["main"], no torn value.
        use std::sync::Arc as StdArc;
        use std::thread;

        let (aid, astr) = agent();
        let router = StdArc::new(OutputRouter::new());

        let reader = {
            let r = StdArc::clone(&router);
            let aid = aid;
            thread::spawn(move || {
                for _ in 0..5000 {
                    let t = r.targets(aid);
                    assert!(t.is_empty() || &*t == &["main".to_string()][..]);
                }
            })
        };

        for _ in 0..200 {
            let mut mgr = ViewManager::new();
            assign_to_main(&mut mgr, &astr);
            router.rebuild(&mgr);
            // Also swap once with an empty table (0→1→0 toggle) to widen the concurrency surface.
            router.rebuild(&ViewManager::new());
        }
        reader.join().unwrap();
    }

    // ── F-B subscription union diff ─────────────────────────────────────────

    #[test]
    fn diff_zero_to_one_subscribes() {
        let mut mgr = ViewManager::new();
        let (aid, astr) = agent();
        assign_to_main(&mut mgr, &astr);

        let router = OutputRouter::new();
        let delta = router.rebuild(&mgr);
        assert_eq!(delta.to_subscribe, vec![aid], "0→1 = Subscribe");
        assert!(delta.to_unsubscribe.is_empty());
    }

    #[test]
    fn diff_one_to_zero_unsubscribes() {
        let mut mgr = ViewManager::new();
        let (aid, astr) = agent();
        let view = main_active(&mgr);
        let slot = assign_to_main(&mut mgr, &astr);

        let router = OutputRouter::new();
        router.rebuild(&mgr);
        mgr.close_slot(view, slot).unwrap();
        let delta = router.rebuild(&mgr);
        assert!(delta.to_subscribe.is_empty());
        assert_eq!(delta.to_unsubscribe, vec![aid], "1→0 = Unsubscribe");
    }

    #[test]
    fn diff_no_change_is_empty() {
        let mut mgr = ViewManager::new();
        let (_aid, astr) = agent();
        assign_to_main(&mut mgr, &astr);

        let router = OutputRouter::new();
        router.rebuild(&mgr);
        let delta = router.rebuild(&mgr);
        assert!(
            delta.is_empty(),
            "an unchanged layout gives subscription delta 0 (prevents duplicate Subscribe)"
        );
    }

    #[test]
    fn diff_agent_in_two_windows_one_closes_stays_subscribed() {
        let mut mgr = ViewManager::new();
        let (aid, astr) = agent();
        assign_to_main(&mut mgr, &astr);
        let pv = mgr.create_window("slot-popup-1").unwrap();
        let slot2 = first_slot(&mgr, pv);
        mgr.assign_agent(pv, slot2, astr.clone()).unwrap();

        let router = OutputRouter::new();
        let d1 = router.rebuild(&mgr);
        assert_eq!(d1.to_subscribe, vec![aid], "first 0→1 Subscribe, once");

        mgr.close_window("slot-popup-1").unwrap();
        let d2 = router.rebuild(&mgr);
        assert!(
            d2.is_empty(),
            "even if one window closes, it stays in main, 1→1 — no Unsubscribe (still visible)"
        );
        assert_eq!(targets_set(&router, aid), vec!["main".to_string()]);
    }

    #[test]
    fn diff_switch_tab_is_noop_keep_alive() {
        // ★G5★: the active flag is excluded from by_agent entirely (the old "switch → A leaves and B enters" is a direct violation).
        let mut mgr = ViewManager::new();
        let v1 = main_active(&mgr);
        let (a1, a1s) = agent();
        let s1 = first_slot(&mgr, v1);
        mgr.assign_agent(v1, s1, a1s).unwrap();

        let v2 = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap(); // active = v2
        let (b1, b1s) = agent();
        let s2 = first_slot(&mgr, v2);
        mgr.assign_agent(v2, s2, b1s).unwrap();

        let router = OutputRouter::new();
        let d0 = router.rebuild(&mgr);
        // The order is sorted, so check as a set.
        let subs: std::collections::HashSet<_> = d0.to_subscribe.iter().copied().collect();
        assert_eq!(subs, std::collections::HashSet::from([a1, b1]));
        assert_eq!(targets_set(&router, a1), vec!["main".to_string()]);
        assert_eq!(targets_set(&router, b1), vec!["main".to_string()]);

        mgr.switch_tab(MAIN_WINDOW_LABEL, v1).unwrap();
        let d1 = router.rebuild(&mgr);
        assert!(d1.is_empty(), "switch_tab is keep-alive, so the delta is a no-op");
        assert_eq!(targets_set(&router, a1), vec!["main".to_string()]);
        assert_eq!(targets_set(&router, b1), vec!["main".to_string()]);
    }

    #[test]
    fn close_tab_removes_agent_and_unsubscribes() {
        let mut mgr = ViewManager::new();
        let v1 = main_active(&mgr);
        let (a1, a1s) = agent();
        let s1 = first_slot(&mgr, v1);
        mgr.assign_agent(v1, s1, a1s).unwrap();

        let v2 = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let (b1, b1s) = agent();
        let s2 = first_slot(&mgr, v2);
        mgr.assign_agent(v2, s2, b1s).unwrap();

        let router = OutputRouter::new();
        router.rebuild(&mgr);

        mgr.close_tab(MAIN_WINDOW_LABEL, v2).unwrap();
        let d = router.rebuild(&mgr);
        assert_eq!(d.to_unsubscribe, vec![b1], "Unsubscribe the closed tab's agent");
        assert!(d.to_subscribe.is_empty());
        assert!(router.targets(b1).is_empty());
        assert_eq!(targets_set(&router, a1), vec!["main".to_string()]);
    }

    #[test]
    fn diff_assign_then_clear_agent() {
        let mut mgr = ViewManager::new();
        let router = OutputRouter::new();
        let d0 = router.rebuild(&mgr);
        assert!(d0.is_empty());

        let (aid, astr) = agent();
        let view = main_active(&mgr);
        let slot = assign_to_main(&mut mgr, &astr);
        let d1 = router.rebuild(&mgr);
        assert_eq!(d1.to_subscribe, vec![aid]);

        mgr.close_slot(view, slot).unwrap();
        let d2 = router.rebuild(&mgr);
        assert_eq!(d2.to_unsubscribe, vec![aid]);
    }

    // Non-vacuity guard: a no-op diff (always an empty delta) fails this; it asserts the exact set difference in a sequence where two different agents come and go.
    #[test]
    fn diff_non_vacuous_distinct_deltas() {
        let mut mgr = ViewManager::new();
        let router = OutputRouter::new();

        let (a, as_) = agent();
        assign_to_main(&mut mgr, &as_);
        let d1 = router.rebuild(&mgr);
        assert_eq!(d1.to_subscribe, vec![a]);
        assert!(d1.to_unsubscribe.is_empty());

        let view_id = main_active(&mgr);
        let slot = first_slot(&mgr, view_id);
        let (b, bs) = agent();
        mgr.assign_agent(view_id, slot, bs).unwrap();
        let d2 = router.rebuild(&mgr);
        assert_eq!(d2.to_subscribe, vec![b], "B newly visible");
        assert_eq!(d2.to_unsubscribe, vec![a], "A was replaced and vanished");
    }

    // ADR-0046: the axis B (slot=(window,agent) cursor delta) test was deleted — the rust check is the single-flight state machine = `daemon_client::replay_flight` unit tests. The axis A (per-agent) diff · preservation invariants below remain.

    // Delta preservation invariant guard: over a whole serialized rebuild sequence (calls holding the ViewManager lock, FIX-1), `(union of all to_subscribe) \ (union of all to_unsubscribe)` must exactly equal the agent set of the final table — every agent visible at the end net-subscribed (none missing), none net-unsubscribed. Catches leaks (subscribed but not in the table) and losses (in the table but not subscribed) at once.
    #[test]
    fn delta_conservation_over_sequence() {
        let mut mgr = ViewManager::new();
        let router = OutputRouter::new();

        let mut subscribed: HashSet<Uuid> = HashSet::new();
        let mut unsubscribed: HashSet<Uuid> = HashSet::new();

        let fold = |router: &OutputRouter,
                    mgr: &ViewManager,
                    sub: &mut HashSet<Uuid>,
                    unsub: &mut HashSet<Uuid>| {
            let d = router.rebuild(mgr);
            for a in d.to_subscribe {
                sub.insert(a);
            }
            for a in d.to_unsubscribe {
                unsub.insert(a);
            }
        };

        // step1: A in the first slot of main's active tab, split and B in the second slot (both 0→1).
        let view_id = main_active(&mgr);
        let slot_a = first_slot(&mgr, view_id);
        let slot_b = mgr
            .split_slot(view_id, slot_a, SplitDir::LeftRight)
            .unwrap();
        let (a, as_) = agent();
        let (b, bs) = agent();
        mgr.assign_agent(view_id, slot_a, as_).unwrap();
        mgr.assign_agent(view_id, slot_b, bs).unwrap();
        fold(&router, &mgr, &mut subscribed, &mut unsubscribed);

        // step2: close the B slot (B 1→0). A stays.
        mgr.close_slot(view_id, slot_b).unwrap();
        fold(&router, &mgr, &mut subscribed, &mut unsubscribed);

        // step3: reassign C to the slot where A was (A 1→0, C 0→1 at once).
        let slot_a2 = first_slot(&mgr, view_id);
        let (c, cs) = agent();
        mgr.assign_agent(view_id, slot_a2, cs).unwrap();
        fold(&router, &mgr, &mut subscribed, &mut unsubscribed);

        let net: HashSet<Uuid> = subscribed.difference(&unsubscribed).copied().collect();
        let final_set: HashSet<Uuid> = router.table.load().by_agent.keys().copied().collect();
        assert_eq!(
            net, final_set,
            "net subscription set of the serialized rebuild sequence = the final table's agent set (no leak or loss)"
        );
        assert_eq!(final_set, HashSet::from([c]), "only C in the final table");
        assert!(net.contains(&c) && !net.contains(&a) && !net.contains(&b));
    }

    // ADR-0046: the resync_filter_*(slots_for_window_agent) tests were deleted — that method served the old per-slot mount replay filter; resync_output is now absorbed into per-agent request_replay (view-driven full re-replay), so no window · slot filter is needed (`targets()` handles routing, single-flight handles marker order).

    // TRD §8

    fn assign_to_window(mgr: &mut ViewManager, label: &str, agent_str: &str) -> Uuid {
        let view_id = mgr.windows.get(label).unwrap().active;
        let slot = first_slot(mgr, view_id);
        mgr.assign_agent(view_id, slot, agent_str.to_string())
            .unwrap();
        slot
    }

    #[test]
    fn cleanup_core_multitab_popup_drops_all_and_unsubscribes() {
        let mut mgr = ViewManager::new();
        let p0 = mgr.create_window("slot-popup-1").unwrap();
        let (a_id, a_str) = agent();
        assign_to_window(&mut mgr, "slot-popup-1", &a_str);
        let p1 = mgr.create_tab("slot-popup-1", None).unwrap();
        let (b_id, b_str) = agent();
        let p1_slot = first_slot(&mgr, p1);
        mgr.assign_agent(p1, p1_slot, b_str).unwrap();

        let router = OutputRouter::new();
        let d0 = router.rebuild(&mgr);
        let subs: HashSet<_> = d0.to_subscribe.iter().copied().collect();
        assert_eq!(subs, HashSet::from([a_id, b_id]));

        // Forced Destroyed cleanup core (the window is still in the model — titlebar/forced-exit path).
        let delta = cleanup_window_core(&mut mgr, &router, "slot-popup-1", DropCause::Destroyed);

        for v in [p0, p1] {
            assert!(!mgr.views.contains_key(&v), "no View left");
            assert!(!mgr.view_owner.contains_key(&v), "no view_owner left");
        }
        assert!(!mgr.windows.contains_key("slot-popup-1"), "window entry removed");
        assert!(
            mgr.closing_numbers().is_empty(),
            "Destroyed releases the number right away"
        );
        mgr.assert_number_invariants();
        let unsub: HashSet<_> = delta.to_unsubscribe.iter().copied().collect();
        assert_eq!(
            unsub,
            HashSet::from([a_id, b_id]),
            "Unsubscribe all window-only agents"
        );
        assert!(router.targets(a_id).is_empty());
        assert!(router.targets(b_id).is_empty());
    }

    #[test]
    fn cleanup_core_on_a_close_request_holds_the_number_until_destroyed() {
        let mut mgr = ViewManager::new();
        mgr.create_window("slot-popup-1").unwrap();
        let router = OutputRouter::new();
        router.rebuild(&mgr);

        cleanup_window_core(&mut mgr, &router, "slot-popup-1", DropCause::CloseRequested);

        assert!(!mgr.windows.contains_key("slot-popup-1"));
        assert_eq!(mgr.closing_numbers(), [2], "the OS window remains and holds the number");
        mgr.assert_number_invariants();
    }

    #[test]
    fn cleanup_core_keeps_agent_shared_with_main() {
        let mut mgr = ViewManager::new();
        let (shared, shared_str) = agent();
        assign_to_main(&mut mgr, &shared_str);
        mgr.create_window("slot-popup-1").unwrap();
        assign_to_window(&mut mgr, "slot-popup-1", &shared_str);

        let router = OutputRouter::new();
        router.rebuild(&mgr);

        let delta = cleanup_window_core(&mut mgr, &router, "slot-popup-1", DropCause::Destroyed);
        assert!(
            delta.to_unsubscribe.is_empty(),
            "no Unsubscribe for a shared agent that remains in main"
        );
        assert_eq!(targets_set(&router, shared), vec!["main".to_string()]);
    }

    #[test]
    fn cleanup_core_main_is_noop() {
        let mut mgr = ViewManager::new();
        let (aid, astr) = agent();
        assign_to_main(&mut mgr, &astr);
        let router = OutputRouter::new();
        router.rebuild(&mgr);

        let before_views = mgr.views.len();
        let delta = cleanup_window_core(&mut mgr, &router, MAIN_WINDOW_LABEL, DropCause::Destroyed);
        assert!(delta.is_empty(), "a main cleanup request gives an empty delta (no-op)");
        assert_eq!(mgr.views.len(), before_views, "main View unchanged");
        assert!(mgr.windows.contains_key(MAIN_WINDOW_LABEL), "main window kept");
        assert_eq!(targets_set(&router, aid), vec!["main".to_string()]);
    }

    #[test]
    fn cleanup_core_already_removed_window_only_rebuilds() {
        // Normal path: window close (`apply::close_window`) deletes the model first and the OS Destroyed arrives late → label not in windows → close skipped, only rebuild (empty delta — it already left routing).
        let mut mgr = ViewManager::new();
        let (aid, astr) = agent();
        mgr.create_window("slot-popup-1").unwrap();
        assign_to_window(&mut mgr, "slot-popup-1", &astr);
        let router = OutputRouter::new();
        router.rebuild(&mgr);

        mgr.close_window("slot-popup-1").unwrap();
        let d_cmd = router.rebuild(&mgr);
        assert_eq!(d_cmd.to_unsubscribe, vec![aid]);

        let d_late = cleanup_window_core(&mut mgr, &router, "slot-popup-1", DropCause::Destroyed);
        assert!(d_late.is_empty(), "re-cleaning an already cleaned window gives an empty delta");
    }

    // ── move_slot_to_window phase-C model level (F4 edge + target vanished) ────────────────────────────

    #[test]
    fn move_phase_c_target_vanished_drops_detached_and_keeps_source() {
        // Move phase C: the existing-window target vanished during phase B (unlocked) → `insert_tab_into` Err → `drop_detached_view`, source slot kept (the user does not lose a slot). Routing is source only.
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot(&mgr, src);
        let (aid, astr) = agent();
        mgr.assign_agent(src, slot, astr).unwrap();

        // phase A: reserve a temporary View.
        let (tmp, _src_content) = mgr.prepare_detached_view(src, slot).unwrap();
        // phase C: the target window is gone (vanished model) → Err.
        let err = mgr.insert_tab_into("gone-window", tmp).unwrap_err();
        assert!(matches!(
            err,
            crate::layout::manager::LayoutError::WindowNotFound(_)
        ));
        mgr.drop_detached_view(tmp);

        let router = OutputRouter::new();
        router.rebuild(&mgr);
        assert_eq!(mgr.slot_agent(src, slot).unwrap(), Some(aid.to_string()));
        assert_eq!(targets_set(&router, aid), vec!["main".to_string()]);
        assert!(!mgr.views.contains_key(&tmp), "temporary View removed");
    }

    #[test]
    fn move_phase_c_source_reassigned_becomes_copy_invariant5_benign() {
        // F4 edge: if during MOVE the source slot is reassigned to other content in phase B (unlocked), phase C's `still_ours` guard (model level: `slot_content(src,slot) == the moved content`, ADR-0064) fails → source close skipped. The moved content is only at the target and the source holds unrelated new content — "preventing deletion of unrelated content" takes priority (load-bearing); the original content remains at the target, so no data loss. Contrasts with "source unchanged → MOVE (delete source)" below; verified at model level.
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot(&mgr, src);
        let (moved, moved_str) = agent();
        mgr.assign_agent(src, slot, moved_str.clone()).unwrap();

        // phase A: the content to move becomes a temporary View (src_content = the SlotContent taken out — the still_ours comparison base).
        let (tmp, src_content) = mgr.prepare_detached_view(src, slot).unwrap();
        mgr.create_window("slot-popup-1").unwrap();

        // ★During phase B the source slot is reassigned to another agent (gap race simulation)★.
        let (other, other_str) = agent();
        mgr.assign_agent(src, slot, other_str).unwrap();

        // phase C: insertion into the target succeeds.
        mgr.insert_tab_into("slot-popup-1", tmp).unwrap();
        // still_ours decision (reproducing the command layer logic, ADR-0064): is the source slot content == the moved content?
        let still_ours = matches!(
            mgr.slot_content(src, slot),
            Ok(ref c) if *c == src_content
        );
        assert!(!still_ours, "reassigned, so still_ours=false → close skipped");
        assert_eq!(mgr.slot_agent(src, slot).unwrap(), Some(other.to_string()));

        let router = OutputRouter::new();
        router.rebuild(&mgr);
        assert_eq!(
            targets_set(&router, moved),
            vec!["slot-popup-1".to_string()]
        );
        assert_eq!(targets_set(&router, other), vec!["main".to_string()]);
    }

    #[test]
    fn move_phase_c_source_intact_move_removes_source() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot(&mgr, src);
        let (moved, moved_str) = agent();
        mgr.assign_agent(src, slot, moved_str.clone()).unwrap();

        let (tmp, src_content) = mgr.prepare_detached_view(src, slot).unwrap();
        mgr.create_window("slot-popup-1").unwrap();
        mgr.insert_tab_into("slot-popup-1", tmp).unwrap();

        let still_ours = matches!(
            mgr.slot_content(src, slot),
            Ok(ref c) if *c == src_content
        );
        assert!(still_ours, "if the source is unchanged, still_ours=true");
        mgr.close_slot(src, slot).unwrap(); // MOVE close

        let router = OutputRouter::new();
        router.rebuild(&mgr);
        assert_eq!(
            targets_set(&router, moved),
            vec!["slot-popup-1".to_string()]
        );
    }

    #[test]
    fn move_non_agent_content_produces_no_subscription_delta() {
        // ADR-0064: over the whole process of moving an agent_list slot to a popup the subscription delta must be 0 — non-agent content has no backend output subscription (`collect_agents` routes only Agent), so rebuild produces no Subscribe/Unsubscribe. Pins that the delta is meaningful only for Agent content (the agent case logic is covered by the tests above).
        use crate::layout::types::SlotContent;
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot(&mgr, src);
        mgr.set_slot_content(src, slot, SlotContent::AgentList)
            .unwrap();

        let router = OutputRouter::new();
        assert!(
            router.rebuild(&mgr).is_empty(),
            "an agent_list placement gives subscription delta 0"
        );

        // phase A: take agent_list content out as a temporary View.
        let (tmp, src_content) = mgr.prepare_detached_view(src, slot).unwrap();
        assert_eq!(src_content, SlotContent::AgentList);
        assert!(
            router.rebuild(&mgr).is_empty(),
            "phase A rebuild gives delta 0 too (non-agent)"
        );

        // phase C: new window insertion + source close (a true MOVE — still_ours=true).
        mgr.attach_view_as_new_window("slot-popup-1", tmp).unwrap();
        let still_ours = matches!(
            mgr.slot_content(src, slot),
            Ok(ref c) if *c == src_content
        );
        assert!(still_ours);
        mgr.close_slot(src, slot).unwrap();
        assert!(
            router.rebuild(&mgr).is_empty(),
            "phase C rebuild gives delta 0 too (the whole non-agent move is subscription-neutral)"
        );
        assert!(router.table.load().by_agent.is_empty(), "no agent routing");
    }
}
