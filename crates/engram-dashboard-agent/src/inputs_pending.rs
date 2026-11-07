//! inputs_pending — a **pending-list table** per (agent, epoch): "this incarnation's user pending list is not empty".
//!
//! ★A leaf table of the same shape as the turn-observation table (`turn`), and it is separate because the place that uses it differs★: the places that use
//!   the turn-observation table's `in_turn` must be only two points, the classifier and cleanup (ADR-0127 「세 번째 호출자」 (the third caller)). This fact is
//!   written by the core inside the emit that reduced the registry, so we do not add a field to that table.
//! ★Reader = mail busyness (daemon adapter)★ — while this value is true, mail does not go in, and ★there is no fail-open cap★
//!   (user decision N8 — an intended exception to ADR-0104's 「늦게 가는 것 < 안 가는 것」 (going late < not going)).
//! ★Lock = this one table, a leaf★: we do not call outward while holding it (ADR-0006). Entries are keyed by `AgentId` and the epoch is held
//!   as the value — the grounds are the same as the `turn` module header.
// ADR-0231
// ADR-0006

use std::collections::HashMap;
use std::sync::Mutex;

use engram_dashboard_base::sync;

use crate::types::AgentId;

/// The pending-list table — owned one per `AgentManager`.
pub struct InputsPendingTable {
    entries: Mutex<HashMap<AgentId, Entry>>,
}

struct Entry {
    epoch: u32,
    pending: bool,
    /// The output seq that last updated this entry (comparable only within the same epoch).
    last_seq: u64,
}

impl Default for InputsPendingTable {
    fn default() -> Self {
        Self::new()
    }
}

impl InputsPendingTable {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Poison tolerance — it is a manager-wide table, so one holder's panic must not spread to every agent (the same judgment as `turn`).
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<AgentId, Entry>> {
        sync::lock(&self.entries)
    }

    /// This incarnation takes this id's place — it unconditionally replaces any existing entry and starts at "list empty".
    /// ★Call it before the session becomes observable★ (owner = `AgentManager::spawn_session`, the same place as the turn-observation registration).
    pub fn register(&self, id: AgentId, epoch: u32) {
        self.lock().insert(
            id,
            Entry {
                epoch,
                pending: false,
                last_seq: 0,
            },
        );
    }

    /// Record the list going empty↔full. `seq` = the output seq of the list event that produced that value.
    ///
    /// ★Drop writes with a smaller seq★: 「비었다」 (became empty) is written deferred outside the replay lock, so it must not overwrite a later
    ///   「찼다」 (became full) that another thread wrote inside the lock in between.
    /// ★Drop it if the marker differs or the entry is missing (unlike the turn-observation table)★: this fact has no cap, so if a write that arrives
    ///   late after cleanup revives the entry, there is no trigger to erase it. That is why the verb that creates an entry is `register` alone.
    pub fn set(&self, id: AgentId, epoch: u32, seq: u64, pending: bool) {
        let mut g = self.lock();
        let Some(e) = g.get_mut(&id) else {
            return;
        };
        if e.epoch != epoch || seq < e.last_seq {
            return;
        }
        e.pending = pending;
        e.last_seq = seq;
    }

    /// The value for this (agent, epoch). `None` = no entry (including the case where only another epoch exists).
    pub fn get(&self, id: AgentId, epoch: u32) -> Option<bool> {
        self.lock()
            .get(&id)
            .filter(|e| e.epoch == epoch)
            .map(|e| e.pending)
    }

    /// Remove this incarnation's entry — only when the epoch matches (so that a late call cannot erase the new incarnation's entry).
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

    #[test]
    fn a_registered_incarnation_starts_empty_and_follows_writes() {
        let t = InputsPendingTable::new();
        let id = AgentId::new_v4();
        assert_eq!(t.get(id, 0), None);
        t.register(id, 0);
        assert_eq!(t.get(id, 0), Some(false));
        t.set(id, 0, 3, true);
        assert_eq!(t.get(id, 0), Some(true));
        t.set(id, 0, 4, false);
        assert_eq!(t.get(id, 0), Some(false));
        assert_eq!(t.get(id, 1), None, "다른 epoch 은 항목 없음");
    }

    #[test]
    fn a_deferred_empty_write_with_a_smaller_seq_cannot_cover_a_later_full_one() {
        let t = InputsPendingTable::new();
        let id = AgentId::new_v4();
        t.register(id, 0);
        t.set(id, 0, 8, true);
        t.set(id, 0, 7, false);
        assert_eq!(t.get(id, 0), Some(true));
        t.set(id, 0, 8, false);
        assert_eq!(t.get(id, 0), Some(false), "같은 seq 는 받는다");
    }

    #[test]
    fn writes_without_the_matching_registration_are_dropped() {
        let t = InputsPendingTable::new();
        let id = AgentId::new_v4();
        t.set(id, 0, 1, true);
        assert_eq!(t.get(id, 0), None, "등록 없는 쓰기는 자리를 만들지 않는다");
        assert_eq!(t.len(), 0);

        t.register(id, 5);
        t.set(id, 9, 1, true);
        assert_eq!(
            t.get(id, 5),
            Some(false),
            "옛 표식의 쓰기는 산 화신을 덮지 않는다"
        );
        assert_eq!(t.get(id, 9), None);
    }

    #[test]
    fn forget_removes_only_the_matching_incarnation_and_late_writes_stay_out() {
        let t = InputsPendingTable::new();
        let id = AgentId::new_v4();
        t.register(id, 1);
        t.set(id, 1, 1, true);
        t.forget(id, 0);
        assert_eq!(
            t.get(id, 1),
            Some(true),
            "지각 정리가 산 화신을 지우지 않는다"
        );
        t.forget(id, 1);
        t.set(id, 1, 2, true);
        assert_eq!(
            t.get(id, 1),
            None,
            "정리 뒤 늦은 쓰기가 항목을 되살리지 않는다"
        );
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn registration_replaces_the_previous_incarnation() {
        let t = InputsPendingTable::new();
        let id = AgentId::new_v4();
        t.register(id, 1);
        t.set(id, 1, 5, true);
        t.register(id, 2);
        assert_eq!(t.get(id, 1), None);
        assert_eq!(t.get(id, 2), Some(false));
        t.set(id, 2, 0, true);
        assert_eq!(
            t.get(id, 2),
            Some(true),
            "새 화신의 seq 는 0 부터 — 옛 seq 와 비교하지 않는다"
        );
        assert_eq!(t.len(), 1);
    }
}
