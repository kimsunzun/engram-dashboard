//! OutputRouter — agent_id → window-label routing table (lock-free hot path) + subscription union diff.
//! **Pure logic — zero Tauri runtime dependency** (headless standalone test possible).
//!
//! ## Role
//! Provides the routing table for fanning out an output frame from the daemon (`DecodedFrame{agent_id, ...}`) to
//! **all windows (window label)** where that agent is currently visible on screen. The authoritative source of the table is
//! `ViewManager` (ADR-0035/0057) — in the split trees of **all tabs of each window** (hidden tabs too, not only the active one) it
//! finds the Slot where the agent is placed and reverse-indexes into `agent_id → [window_label]`.
//!
//! ## ★Routing reversal (ADR-0057/0056 keep-alive)★
//! Walks **all tabs of each window** (hidden tabs too, receive ·
//! switch lossless, ADR-0056). The `active_view_id` · `MAIN_WINDOW_LABEL` routing branch is **removed entirely** —
//! the active flag no longer enters `by_agent`. With unique ownership (ADR-0057 invariant 2) a View is in only one window
//! → no "one View in two windows" double counting.
//!
//! ## Core invariants
//! - **Hot path (`targets`) lock 0** (ADR-0006): `ArcSwap::load()` only — called per frame, so no Mutex.
//!   Allocation · recomputation are all done by the low-frequency path (`rebuild`).
//! - **rebuild-always** (D2): layout changes are low-frequency, so on every change the whole snapshot is recomputed and
//!   swapped atomically with `ArcSwap::store`. The version-cache branch is not adopted (no gain against complexity).
//! - **AgentKey = `AgentId` (= `uuid::Uuid`)**: the frame's `agent_id` is already an `AgentId`, so zero conversion on the hot path.
//!   A Slot stores `SlotContent::Agent{agent_id: String}` (ADR-0060), so **String→Uuid parsing happens at rebuild (low-frequency)**
//!   and is normalized at the boundary. A slot that fails to parse can never match a real frame anyway, so it is ignored. This choice is
//!   also consistent with the existing `HashMap<AgentId, SubState>` key of `protocol_state` (the connection task dedups by the same key).
//! - **F-B subscription union = layout-derived** (ADR-0035, spike §8 F-B): with no separate ref-count map, the snapshot's
//!   agent set itself is "the set that should be subscribed now". On rebuild it diffs against the previous set and produces the 0→1 (Subscribe) ·
//!   1→0 (Unsubscribe) deltas **piggybacking on the same tree walk** (single pass).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use arc_swap::ArcSwap;

use crate::layout::manager::ViewManager;
use crate::layout::types::{LayoutNode, SlotContent};

// Cached empty routing result (returned on a miss). `Arc::from(Vec::new())` allocates an Arc header (refcount block) on the
// heap every time — if misses are frequent on the hot path, that is the accumulated cost. Build it once and return it via `Arc::clone` (refcount
// +1, zero allocation) to make the miss path truly zero-alloc (FIX-2).
static EMPTY_TARGETS: LazyLock<Arc<[WindowLabel]>> = LazyLock::new(|| Arc::from(Vec::new()));

pub type AgentKey = uuid::Uuid;

// Tauri window label (e.g. "main", "slot-popup-0a1b2c3d-3"). Same type as the `ViewManager.windows` key → no separate
// numeric registry needed (spike §8 D1).
pub type WindowLabel = String;

// Values are shared as `Arc<[WindowLabel]>` — cloning on the hot path is only a pointer copy (no element copy).
//
// An agent not in `by_agent` = currently visible in no window → `targets` returns an empty slice (send targets 0).
#[derive(Debug, Default)]
pub struct RoutingSnapshot {
    pub by_agent: HashMap<AgentKey, Arc<[WindowLabel]>>,
}

// Subscription delta of one rebuild (F-B). Produces the agent-union diff against the previous snapshot in a single tree walk.
//
// ## Per-agent diff
// - `to_subscribe`: newly started to be visible this time (0→1 agent). ★ADR-0046: layout does **not send** a wire Subscribe with this★.
//   The delta is still produced and used for the diff-preservation invariant test / future diagnostics.
// - `to_unsubscribe`: no longer visible in any window (1→0 agent) → the router emits a wire `Unsubscribe` (cleanup).
//
// ★ADR-0046 — axis B removed★: the old slot=(window,agent) unit cursor deltas (`slots_to_replay`/`slots_to_drop`)
//   were deleted together with the mirror buffer (cursor). Remount/new window is replaced by a full re-replay of the daemon ring (view-driven).
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

// App-level shared (lives beyond the lifetime of the reconnect task) → manage/inject it as `Arc<OutputRouter>`.
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

    // ★Hot path★ All window labels where the agent is currently visible.
    //
    // ★Caution (spike §7 Pitfall)★: iterate and consume the returned Arc
    // immediately. (Carrying a `load()` Guard across an `.await` exhausts slots — here the Guard is used only inside the function and
    // only the Arc is returned, so it is safe.)
    pub fn targets(&self, agent_id: AgentKey) -> Arc<[WindowLabel]> {
        let snap = self.table.load();
        match snap.by_agent.get(&agent_id) {
            Some(labels) => Arc::clone(labels),
            None => Arc::clone(&EMPTY_TARGETS),
        }
    }

    // `pub(crate)` 라 crate 내 다른 테스트 모듈(daemon_client 의 시험들)에서도 호출 가능.
    // ★ADR-0046★: 옛 eager resubscribe(current_agents 순회)는 삭제됐다 — 헬퍼는 라우팅(targets) 테스트가
    //   다시 쓸 수 있어 남긴다(현재는 미사용 → allow).
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn set_visible_agents_for_test(&self, agents: &[AgentKey]) {
        let mut by_agent: HashMap<AgentKey, Arc<[WindowLabel]>> = HashMap::new();
        for a in agents {
            by_agent.insert(*a, Arc::from(vec!["main".to_string()]));
        }
        self.table.store(Arc::new(RoutingSnapshot { by_agent }));
    }

    // ★Low-frequency★ Recomputes the whole routing table from a ViewManager snapshot and swaps it atomically.
    //
    // ★Call contract (serialization — FIX-1)★: this function must be called **while holding the ViewManager lock (inside the same
    //  critical section as the layout mutation)**. Its inside is the RMW of `load(prev) → delta 계산 → store(new)`,
    //  so if called concurrently outside the lock (Tauri thread pool) the deltas diverge — duplicate Subscribe · missing
    //  Unsubscribe · ABA (a stale store overwrites a new store). The lock serializes the RMW and guarantees that `&mgr` is the
    //  current state.
    //
    // ★ADR-0006★: calling inside the lock is not a violation — the body is only **pure computation + lock-free `ArcSwap::store`**, and
    //  external calls this function makes while holding the lock (emit / DaemonClient / network I/O) are 0. The enqueue of the returned delta is also
    //  **inside the same lock** (wiring = the `layout::apply::SubscriptionSync::resync` implementation +
    //  the two cleanup paths of `commands::popout` — no exceptions). If deferred outside the lock, a re-add between compute and fire lets a stale
    //  unsubscribe kill a live subscription.
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
        // `HashSet::difference` order is nondeterministic — sort with the same determinism policy as labels.sort() to fix the delta order
        // (test reproducibility + stable send order, FIX-4).
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

// ★Window cleanup core (Tauri-free — G1 headless required, TRD §8 stage 1)★. **Drops all tab Views** of window `label`
// (views + view_owner + windows entry) and then runs `rebuild` once → returns that delta. The returned
// `to_unsubscribe` (agents that remain in no window) is fired to the daemon **by the caller inside the lock** (F1).
//
// ★This function knows nothing of Tauri · DaemonClient · registry★ — it takes only `ViewManager` (model) + `OutputRouter` (routing)
//   and purely updates only the model · routing table (headless standalone test possible). `commands::popout::drop_window_in_model`
//   (called by the close request · `Destroyed` cleanup) calls this core inside the ViewManager lock and handles firing the delta · releasing the closing window's number
//   in the same lock. Detaching the registry (the Tauri part) is the job of `cleanup_popup_window`.
//
// - `label == MAIN_WINDOW_LABEL`: not a cleanup target (invariant 4) — no-op, empty delta (defensive, the caller also blocks it first).
// - `label` not in `windows` (the normal path where close_tab · window close (`apply::close_window`) or a popout close request deleted the model first):
//   skip close, only `rebuild` once (recompute the table by contract). Destroyed is an OS event after that, so it is already not in the model.
// - `label` in `windows` (a popout close request · a `Destroyed` that came without a close request — multi-tab residual risk, G1):
//   drop all tabs by iterating and then `rebuild`. The window number is decided by `cause` — for a close request the OS window still exists, so it is held until its `Destroyed`
//   (`close_window`), and for `Destroyed` it is released at once (`drop_window_gone` — TRD S26-window-chrome §4-1).
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

/// The OS event that called the popout window cleanup — tells apart whether that OS window still exists (TRD S26-window-chrome §4-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropCause {
    /// Close request — the OS window will emit `Destroyed` soon.
    CloseRequested,
    /// Destroyed — the OS window is already gone.
    Destroyed,
}

// ★Load-bearing routing invariant (ADR-0041/0042/0046 · ADR-0060)★: **only an assigned slot (SlotContent::Agent)**
// is a routing target — `SlotContent::Empty` is explicitly ignored (no output is sent to an empty slot). The content
// kind (SlotContent) and byte routing cross only at this point, and here only the Agent's binding (agent_id) is extracted.
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
                    // If dropped silently, debugging clues vanish (ADR-0038). rebuild is low-frequency, so the debug log
                    // costs the hot path 0 (logging-conventions).
                    Err(_) => {
                        tracing::debug!(agent_id = %agent_id, "rebuild: 슬롯 agent_id 가 UUID 아님 — 라우팅 스킵");
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
            "배정 안 된 agent 는 빈 결과"
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
            "숨은 탭의 agent 도 라우팅(keep-alive)"
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
            "같은 agent 가 두 창에 보이면 둘 다 라우팅"
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
        assert!(delta.is_empty(), "파싱 불가 slot 은 구독 대상도 아님");
        let (probe, _) = agent();
        assert!(router.targets(probe).is_empty());
    }

    // ── lock-free reads (structural): no Mutex in OutputRouter ─────────────────
    //
    // ★Caution (FIX-1)★: the tests below verify only the safety of concurrent *reads* (targets/load) and store. Concurrent *rebuild*
    // (two writers doing load→delta→store with no lock) is a **contract violation** — rebuild must be called only inside the ViewManager-lock-holding
    // critical section (serialization guaranteed), so there is no concurrent-writer test
    // (it would be testing misuse, so meaningless). The serialized-sequence invariant is covered by delta_conservation_over_sequence.

    #[test]
    fn targets_is_lock_free_concurrent_reads() {
        // Structural assertion: targets takes no lock, so many threads read concurrently without blocking.
        // (The only field of OutputRouter is one ArcSwap — no Mutex/RwLock. Confirmed by compiling + concurrent reads.)
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
        // While rebuild (store) is in progress, another thread doing targets (load) always sees only a consistent
        // snapshot (the old or the new one) with no data race — ArcSwap semantics. Either an empty result or ["main"] (no torn value).
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
            "레이아웃 불변이면 구독 델타 0(중복 Subscribe 방지)"
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
        assert_eq!(d1.to_subscribe, vec![aid], "처음 0→1 Subscribe 한 번");

        mgr.close_window("slot-popup-1").unwrap();
        let d2 = router.rebuild(&mgr);
        assert!(
            d2.is_empty(),
            "한 창 닫혀도 main 에 남아 1→1 — Unsubscribe 금지(아직 보임)"
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
        assert!(d1.is_empty(), "switch_tab 은 keep-alive 라 델타 no-op");
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
        assert_eq!(d.to_unsubscribe, vec![b1], "닫힌 탭 agent Unsubscribe");
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

    // ★non-vacuity guard★: if the diff logic is a no-op (always returns an empty delta), this test fails.
    // It asserts the exact set difference in a sequence where two different agents come and go — it cannot pass with an empty delta.
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
        assert_eq!(d2.to_subscribe, vec![b], "B 새로 보임");
        assert_eq!(d2.to_unsubscribe, vec![a], "A 교체돼 사라짐");
    }

    // ★ADR-0046: axis B (slot=(window,agent) cursor delta) test deleted★ — the rust check is the single-flight
    //   state machine = the daemon_client::replay_flight unit tests. The axis A (per-agent) diff · preservation invariants below remain.

    // ★Delta preservation invariant guard★: over the whole **serialized** rebuild sequence (= calls holding the ViewManager lock, FIX-1),
    // `(모든 to_subscribe 합집합) \ (모든 to_unsubscribe 합집합)` must exactly match the agent set of the final table — that is, every agent visible at the end is
    // net-subscribed (none missing), and none was net-unsubscribed. A regression net that catches leaks (subscribed but not in the table) ·
    // losses (in the table but not subscribed) at once.
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
            "직렬화 rebuild 시퀀스의 net-구독 집합 = 최종 테이블 agent 집합(누수·유실 0)"
        );
        assert_eq!(final_set, HashSet::from([c]), "최종 테이블엔 C 만");
        assert!(net.contains(&c) && !net.contains(&a) && !net.contains(&b));
    }

    // ★ADR-0046: resync_filter_*(slots_for_window_agent) tests deleted★ — that method was for the old per-slot mount
    //   replay filter. resync_output is now absorbed into per-agent request_replay (view-driven full re-replay), so
    //   no window · slot filter is needed (targets() handles routing, single-flight handles marker order).

    // ── cleanup_window_core (G1 multi-tab cleanup headless required, TRD §8 stage 1) ────────────────────

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
            assert!(!mgr.views.contains_key(&v), "View 잔류 0");
            assert!(!mgr.view_owner.contains_key(&v), "view_owner 잔류 0");
        }
        assert!(!mgr.windows.contains_key("slot-popup-1"), "창 엔트리 제거");
        assert!(
            mgr.closing_numbers().is_empty(),
            "Destroyed 는 번호를 곧바로 푼다"
        );
        mgr.assert_number_invariants();
        let unsub: HashSet<_> = delta.to_unsubscribe.iter().copied().collect();
        assert_eq!(
            unsub,
            HashSet::from([a_id, b_id]),
            "창 전용 agent 전부 Unsubscribe"
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
        assert_eq!(mgr.closing_numbers(), [2], "OS 창이 남아 번호를 쥔다");
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
            "main 에 남은 shared agent 는 Unsubscribe 금지"
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
        assert!(delta.is_empty(), "main 정리 요청은 빈 델타(no-op)");
        assert_eq!(mgr.views.len(), before_views, "main View 불변");
        assert!(mgr.windows.contains_key(MAIN_WINDOW_LABEL), "main 창 유지");
        assert_eq!(targets_set(&router, aid), vec!["main".to_string()]);
    }

    #[test]
    fn cleanup_core_already_removed_window_only_rebuilds() {
        // Normal path: window close (`apply::close_window`) deletes the model first and then the OS Destroyed arrives late →
        //   label not in windows → close skipped, only rebuild (empty delta — since it already left routing).
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
        assert!(d_late.is_empty(), "이미 정리된 창 재정리는 빈 델타");
    }

    // ── move_slot_to_window phase-C model level (F4 edge + target vanished) ────────────────────────────

    #[test]
    fn move_phase_c_target_vanished_drops_detached_and_keeps_source() {
        // move phase-C: the existing-window target vanished during phase B (unlocked) → insert_tab_into Err → drop_detached_view,
        //   source slot kept (the user does not lose a slot). Routing is source only.
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
        assert!(!mgr.views.contains_key(&tmp), "임시 View 제거");
    }

    #[test]
    fn move_phase_c_source_reassigned_becomes_copy_invariant5_benign() {
        // ★F4 edge★: if during MOVE the source slot is **reassigned to other content** during phase B (unlocked), the phase C
        //   still_ours guard (model level: whether slot_content(src,slot) == the moved content, ADR-0064) fails → source
        //   close skipped. Result: the moved content is only at the target, and the source has unrelated new content — that is, "preventing deletion of unrelated content" takes
        //   priority (load-bearing). From the original content's view it remains only at the target, so data loss 0. Below, in contrast with "if the source is unchanged
        //   then MOVE (delete source)", verify at model level that close is skipped on reassignment.
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
        assert!(!still_ours, "재배정됐으니 still_ours=false → close 스킵");
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
        assert!(still_ours, "소스 그대로면 still_ours=true");
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
        // ★ADR-0064★: over the whole process of moving an agent_list slot to a popup the subscription delta must be 0 —
        //   non-agent content has no backend output subscription (collect_agents routes only Agent), so rebuild produces
        //   no Subscribe/Unsubscribe at all. That is, the fact that "the subscription delta is meaningful only for Agent content" is
        //   structurally guaranteed is pinned as a regression (the agent case logic is covered as is by the tests above = no regression).
        use crate::layout::types::SlotContent;
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot(&mgr, src);
        mgr.set_slot_content(src, slot, SlotContent::AgentList)
            .unwrap();

        let router = OutputRouter::new();
        assert!(
            router.rebuild(&mgr).is_empty(),
            "agent_list 배치는 구독 델타 0"
        );

        // phase A: take agent_list content out as a temporary View.
        let (tmp, src_content) = mgr.prepare_detached_view(src, slot).unwrap();
        assert_eq!(src_content, SlotContent::AgentList);
        assert!(
            router.rebuild(&mgr).is_empty(),
            "phase A rebuild 도 델타 0(비-에이전트)"
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
            "phase C rebuild 도 델타 0(비-에이전트 이동 전체가 구독 무관)"
        );
        assert!(router.table.load().by_agent.is_empty(), "agent 라우팅 0");
    }
}
