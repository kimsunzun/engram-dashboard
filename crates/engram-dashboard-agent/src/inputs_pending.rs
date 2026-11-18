//! inputs_pending — a **pending-list table** per (agent, epoch): "this incarnation's user pending list is not empty".
//!
//! ★A leaf table shaped like the turn-observation table (`turn`), separate because its users differ★: the turn table's `in_turn` must have
//!   only two users, the classifier and cleanup (ADR-0127 「the third caller」); this fact is written by the core inside the emit that
//!   reduced the registry, so no field is added there.
//! ★Reader = mail busyness (daemon adapter)★ — while true, mail does not go in, and ★there is no fail-open cap★ (user decision N8 — an
//!   intended exception to ADR-0104's 「going late < not going」).
//! ★Lock = this one table, a leaf★: nothing is called outward while holding it (ADR-0006). Entries are keyed by `AgentId`, the epoch is
//!   the value (same grounds as the `turn` module header).
// ADR-0231
// ADR-0006

use std::collections::HashMap;
use std::sync::Mutex;

use engram_dashboard_base::sync;

use crate::types::AgentId;

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
    /// ★Call it before the session becomes observable★ (owner = `AgentManager::spawn_session`, same place as the turn-observation registration).
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

    /// Record the list going empty↔full. `seq` = the output seq of the list event that produced the value.
    ///
    /// ★Drop writes with a smaller seq★: 「became empty」 is written deferred outside the replay lock and must not overwrite a later 「became
    ///   full」 another thread wrote inside the lock meanwhile.
    /// ★Drop it if the marker differs or the entry is missing (unlike the turn table)★: this fact has no cap, so a write arriving after
    ///   cleanup would revive the entry with no trigger to erase it — hence `register` alone creates entries.
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
        assert_eq!(t.get(id, 1), None, "a different epoch has no entry");
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
        assert_eq!(t.get(id, 0), Some(false), "the same seq is accepted");
    }

    #[test]
    fn writes_without_the_matching_registration_are_dropped() {
        let t = InputsPendingTable::new();
        let id = AgentId::new_v4();
        t.set(id, 0, 1, true);
        assert_eq!(
            t.get(id, 0),
            None,
            "a write without registration does not create an entry"
        );
        assert_eq!(t.len(), 0);

        t.register(id, 5);
        t.set(id, 9, 1, true);
        assert_eq!(
            t.get(id, 5),
            Some(false),
            "a write from an old marker does not overwrite the live incarnation"
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
            "a late cleanup does not erase the live incarnation"
        );
        t.forget(id, 1);
        t.set(id, 1, 2, true);
        assert_eq!(
            t.get(id, 1),
            None,
            "a late write after cleanup does not revive the entry"
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
            "the new incarnation's seq starts from 0 — it is not compared with the old seq"
        );
        assert_eq!(t.len(), 1);
    }
}
