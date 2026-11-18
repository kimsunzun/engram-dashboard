//! `AgentManager`'s implementation of the profile edit port.
//!
//! An attachment write takes the roster lock once each (saving included — the order is `ProfileRegistry`'s as is); the lock read
//! briefly takes and releases the reservations · sessions · roster **one at a time** (never held together — ADR-0006).
// ADR-0311
// TRD S25 §1-5 「profile edit port」

use super::AgentManager;
use crate::profile_edit::{
    Attachments, PresetPathRename, ProfileEditError, ProfileEditPort, WorkspaceLock,
};
use crate::types::AgentId;

impl ProfileEditPort for AgentManager {
    fn set_attachments(&self, id: AgentId, next: Attachments) -> Result<(), ProfileEditError> {
        if self.profiles.try_set_attachments(id, next)? {
            Ok(())
        } else {
            Err(ProfileEditError::NotFound(id))
        }
    }

    fn rewrite_preset_paths(
        &self,
        renames: &[PresetPathRename],
    ) -> Result<Vec<AgentId>, ProfileEditError> {
        Ok(self.profiles.try_rewrite_preset_paths(renames)?)
    }

    fn remove_preset_paths(&self, paths: &[String]) -> Result<Vec<AgentId>, ProfileEditError> {
        Ok(self.profiles.try_remove_preset_paths(paths)?)
    }

    // ADR-0321
    fn workspace_lock(&self, id: AgentId) -> Result<Option<WorkspaceLock>, ProfileEditError> {
        // Read order = reservations → sessions → roster. A launch releases its reservation **after** raising the session, so a launch
        //   overlapping this call is always caught by one of the two (reading in reverse misses a launch that finished in between). The
        //   session id is written by that launch's first submission, so it is read last.
        // TODO(TRD S25 10a `agent.setWorkspace`): a change that calls this decision while holding its own reservation sees that
        //   reservation as `Preparing` — fix together when the reservation table tells holders apart.
        let preparing = self.activation_in_flight(id);
        // ADR-0116: a corpse (before the reaper reaps it) is not running — the same predicate as the roster.
        let running = self
            .get_session(id)
            .is_ok_and(|session| session.status().is_live());
        let conversed = self
            .profiles
            .get(id)
            .ok_or(ProfileEditError::NotFound(id))?
            .backend_session_id
            .is_some();
        Ok(if running {
            Some(WorkspaceLock::Running)
        } else if preparing {
            Some(WorkspaceLock::Preparing)
        } else if conversed {
            Some(WorkspaceLock::Conversed)
        } else {
            None
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use super::super::tests::{agent_profile, put_live_session, scripted_manager};
    use super::super::SpawnReservation;
    use super::*;
    use crate::profile::scripted_store::{Outcome, FAILURES};
    use crate::profile::{AttachedPreset, StoreError};
    use crate::types::TerminalReason;

    fn attachments(default_preset: bool, paths: &[&str]) -> Attachments {
        Attachments {
            default_preset,
            presets: paths
                .iter()
                .map(|p| AttachedPreset {
                    path: (*p).to_string(),
                })
                .collect(),
        }
    }

    /// A manager with one profile seeded in the roster — the test decides the save result.
    fn seeded() -> (
        AgentManager,
        Arc<crate::profile::scripted_store::ScriptedStore>,
        AgentId,
    ) {
        let (manager, store) = scripted_manager();
        let p = agent_profile("C:/edit/a", Some("a"));
        let id = p.id;
        manager.profiles.upsert(p);
        (manager, store, id)
    }

    fn converse(manager: &AgentManager, id: AgentId) {
        assert!(manager
            .profiles
            .update_with(id, |p| p.backend_session_id = Some(Uuid::new_v4())));
    }

    // ── attachment writes ──

    #[test]
    fn set_attachments_writes_through_and_tells_a_missing_agent_apart() {
        let (manager, store, id) = seeded();
        let next = attachments(false, &["/b.md", "/a.md"]);

        manager.set_attachments(id, next.clone()).unwrap();
        assert_eq!(manager.profiles.attachments_of(id), Some(next.clone()));
        assert_eq!(store.disk(id).unwrap().presets, next.presets);
        manager
            .set_attachments(id, next.clone())
            .expect("the same value is not an error");

        let missing = Uuid::new_v4();
        assert!(matches!(
            manager.set_attachments(missing, next),
            Err(ProfileEditError::NotFound(got)) if got == missing
        ));
    }

    #[test]
    fn rewrite_preset_paths_returns_the_changed_agents() {
        let (manager, _store, id) = seeded();
        manager
            .set_attachments(id, attachments(true, &["/a.md"]))
            .unwrap();

        let changed = manager
            .rewrite_preset_paths(&[PresetPathRename {
                from: "/a.md".into(),
                to: "/b.md".into(),
            }])
            .unwrap();

        assert_eq!(changed, vec![id]);
        assert_eq!(
            manager.profiles.attachments_of(id),
            Some(attachments(true, &["/b.md"]))
        );
    }

    #[test]
    fn remove_preset_paths_returns_the_changed_agents() {
        let (manager, _store, id) = seeded();
        manager
            .set_attachments(id, attachments(true, &["/a.md", "/b.md"]))
            .unwrap();

        let changed = manager.remove_preset_paths(&["/a.md".into()]).unwrap();

        assert_eq!(changed, vec![id]);
        assert_eq!(
            manager.profiles.attachments_of(id),
            Some(attachments(true, &["/b.md"]))
        );
    }

    #[test]
    fn a_store_failure_surfaces_as_store_and_changes_nothing() {
        for failure in FAILURES {
            let (manager, store, id) = seeded();
            manager
                .set_attachments(id, attachments(true, &["/a.md"]))
                .unwrap();
            let before = manager.profiles.attachments_of(id);
            store.set(failure);

            let set = manager
                .set_attachments(id, attachments(false, &[]))
                .unwrap_err();
            let rewrite = manager
                .rewrite_preset_paths(&[PresetPathRename {
                    from: "/a.md".into(),
                    to: "/b.md".into(),
                }])
                .unwrap_err();
            let remove = manager.remove_preset_paths(&["/a.md".into()]).unwrap_err();

            for err in [set, rewrite, remove] {
                assert!(
                    matches!(
                        (failure, &err),
                        (Outcome::FailsIo, ProfileEditError::Store(StoreError::Io(_)))
                            | (
                                Outcome::Refuses(_),
                                ProfileEditError::Store(StoreError::Refused(_))
                            )
                    ),
                    "{failure:?} → {err}"
                );
            }
            assert_eq!(manager.profiles.attachments_of(id), before, "{failure:?}");
            assert!(!manager.profiles.is_dirty(), "{failure:?}");
        }
    }

    // ── working folder lock ──

    #[test]
    fn workspace_lock_reports_each_reason_alone() {
        let (manager, _store, id) = seeded();
        assert_eq!(manager.workspace_lock(id).unwrap(), None);

        {
            let _held = SpawnReservation::reserve(manager.spawning.clone(), id).expect("reserve");
            assert_eq!(
                manager.workspace_lock(id).unwrap(),
                Some(WorkspaceLock::Preparing)
            );
        }
        assert_eq!(
            manager.workspace_lock(id).unwrap(),
            None,
            "reservation released"
        );

        let (manager, _store, id) = seeded();
        put_live_session(&manager, id, 1);
        assert_eq!(
            manager.workspace_lock(id).unwrap(),
            Some(WorkspaceLock::Running)
        );

        let (manager, _store, id) = seeded();
        converse(&manager, id);
        assert_eq!(
            manager.workspace_lock(id).unwrap(),
            Some(WorkspaceLock::Conversed)
        );
    }

    #[test]
    fn workspace_lock_reports_the_first_of_running_preparing_conversed() {
        let (manager, _store, id) = seeded();
        converse(&manager, id);
        let _held = SpawnReservation::reserve(manager.spawning.clone(), id).expect("reserve");
        assert_eq!(
            manager.workspace_lock(id).unwrap(),
            Some(WorkspaceLock::Preparing),
            "preparing > conversed"
        );

        put_live_session(&manager, id, 1);
        assert_eq!(
            manager.workspace_lock(id).unwrap(),
            Some(WorkspaceLock::Running),
            "running > preparing · conversed"
        );
    }

    /// Exiting (`Exiting`) is still running — the same predicate as the roster (ADR-0116).
    #[test]
    fn an_exiting_session_still_locks_as_running() {
        let (manager, _store, id) = seeded();
        let core = put_live_session(&manager, id, 1);
        assert!(core.enter_exiting());

        assert_eq!(
            manager.workspace_lock(id).unwrap(),
            Some(WorkspaceLock::Running)
        );
    }

    /// ★Pins the read order reservations → sessions → roster in the source★ — the items above measure in turn on one thread, so they
    /// stay green with the order reversed. Reversed, an overlapping launch (raises the session, then releases the reservation) is missed by both reads and the lock looks open.
    #[test]
    fn workspace_lock_reads_reservation_then_session_then_roster() {
        let body = squashed_production_body("fn workspace_lock(", "#[cfg(test)]");
        let at = |call: &str| {
            body.find(call).unwrap_or_else(|| {
                panic!("`{call}` is not read — this item's premise is stale: {body}")
            })
        };
        let reservation = at("self.activation_in_flight(id)");
        let session = at("self.get_session(id)");
        let roster = at("self.profiles.get(id)");
        assert!(
            reservation < session && session < roster,
            "the read order is not reservations → sessions → roster: {body}"
        );
    }

    /// Cuts the production section between `start` and `end`, drops comment lines and removes all whitespace — the same shape as
    /// the helper of that name in the `manager.rs` tests (reads this file).
    fn squashed_production_body(start: &str, end: &str) -> String {
        let src = include_str!("profile_edit.rs");
        let production = src.split("mod tests {").next().expect("production section");
        let body = production
            .split(start)
            .nth(1)
            .unwrap_or_else(|| {
                panic!("`{start}` is not in the production section — this item's premise is stale")
            })
            .split(end)
            .next()
            .expect("up to the next section");
        body.lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("//"))
            .collect::<String>()
            .split_whitespace()
            .collect::<String>()
    }

    #[test]
    fn a_dead_session_awaiting_the_reaper_does_not_lock() {
        let (manager, _store, id) = seeded();
        let core = put_live_session(&manager, id, 1);
        core.finish(TerminalReason::Killed);

        assert_eq!(manager.workspace_lock(id).unwrap(), None);
        converse(&manager, id);
        assert_eq!(
            manager.workspace_lock(id).unwrap(),
            Some(WorkspaceLock::Conversed)
        );
    }

    #[test]
    fn workspace_lock_of_an_agent_not_in_the_roster_is_not_found() {
        let (manager, _store, _id) = seeded();
        let stranger = Uuid::new_v4();
        assert!(matches!(
            manager.workspace_lock(stranger),
            Err(ProfileEditError::NotFound(got)) if got == stranger
        ));

        // Not in the roster = answers absent even with a running session.
        put_live_session(&manager, stranger, 1);
        assert!(matches!(
            manager.workspace_lock(stranger),
            Err(ProfileEditError::NotFound(_))
        ));
    }
}
