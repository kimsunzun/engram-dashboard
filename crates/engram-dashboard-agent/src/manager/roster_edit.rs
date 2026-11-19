//! `agent.setWorkspace` and `agent.duplicate` — the two roster edits whose bodies are the manager's.
//!
//! ## Rules (invariants)
//!
//! - **setWorkspace order**: the folder stat (through the launch-kit port, holding no lock) → the shared lock decision
//!   without the reservation → the agent's reservation as a working-folder holder → the decision again → the
//!   name-allocation gate and the name → one conditional registry write. A call the pre-check refuses returns before it
//!   holds the reservation, so it never turns a concurrent start into a refused one; a refusal found after (the re-check,
//!   the write, a store failure) holds it only until it returns.
//! - **No deadlock**: taking a reservation never blocks, and no holder of the name-allocation gate waits on a reservation
//!   (launches take reservation → gate in the same order).
//! - **duplicate order**: the name-allocation gate → one roster read (ceiling + name) → one locked registry section that
//!   reads the original and inserts the copy.
//! - **A refusal writes nothing** — both registry writes are conditional (`ProfileRegistry::try_set_workspace` ·
//!   `try_insert_copy`), so a refusal saves nothing of its own and a store error cannot replace it.
//! - Neither calls the library (no library lock).
// ADR-0120
// ADR-0132
// ADR-0321
// ADR-0386

use std::path::PathBuf;

use engram_dashboard_base::path::normalize_spelling;
use uuid::Uuid;

use super::{check_roster_capacity, AgentManager, ReservationHolder, SpawnReservation};
use crate::profile::{AgentProfile, StoreError, WorkspaceWrite};
use crate::profile_edit::{ProfileEditError, ProfileEditPort, WorkspaceLock};
use crate::types::{AgentId, PtyError};

/// The `why` of a `cwd` that is not an absolute path — refused before the stat, as `library.setRoot` refuses a
/// relative root (a relative working folder would follow whatever folder the daemon runs in).
// ADR-0386
pub const NOT_ABSOLUTE: &str = "not absolute";

/// What `agent.setWorkspace` set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSet {
    /// The agent's name — pinned as its display name when it had none, so the folder change does not rename it.
    pub name: String,
    /// The folder as stored (the normalized spelling).
    pub cwd: String,
}

/// Why `agent.setWorkspace` changed nothing.
#[derive(Debug)]
pub enum WorkspaceRefusal {
    /// The folder is not absolute ([`NOT_ABSOLUTE`]), missing, not a folder, unreadable, or its stat did not answer in
    /// time ([`crate::launch::STAT_TIMED_OUT`]) — `why` says which.
    FolderNotFound { cwd: String, why: String },
    /// The working folder is locked — the shared decision's reason.
    Locked(WorkspaceLock),
    /// Another working-folder change holds the agent's reservation.
    Changing,
    /// The agent is not in the roster (or was deleted meanwhile).
    NotFound,
    /// The store did not accept the write — nothing changed.
    Store(StoreError),
}

impl AgentManager {
    /// `agent.setWorkspace` — sets the working folder of an asleep agent that has never conversed (module rules).
    // ADR-0321
    // ADR-0386
    pub fn try_set_workspace(
        &self,
        id: AgentId,
        cwd: &str,
    ) -> Result<WorkspaceSet, WorkspaceRefusal> {
        let cwd = normalize_spelling(cwd);
        let folder = PathBuf::from(&cwd);
        if !folder.is_absolute() {
            return Err(WorkspaceRefusal::FolderNotFound {
                cwd,
                why: NOT_ABSOLUTE.to_owned(),
            });
        }
        // ADR-0360: outside every lock — production bounds it with the library's read budget.
        if let Err(why) = self.launch.kits.stat_folder(&folder) {
            return Err(WorkspaceRefusal::FolderNotFound { cwd, why });
        }
        if let Some(lock) = self.workspace_locked(id)? {
            return Err(WorkspaceRefusal::Locked(lock));
        }
        let _reservation = self.reserve_for_working_folder(id)?;
        // Called before the registry write, never inside it — it reads the registry, whose lock is not re-entrant.
        if let Some(lock) = self.workspace_locked(id)? {
            return Err(WorkspaceRefusal::Locked(lock));
        }
        // ADR-0120: the name is derived and pinned under the gate, so a concurrent rename cannot take it in between.
        let _gate = self.lock_name_allocation();
        // Asleep here (no launch can start while the reservation is held), so the asleep derivation is the roster's.
        let name = self
            .profiles
            .get(id)
            .ok_or(WorkspaceRefusal::NotFound)?
            .canonical_name_when_live();
        let written = self
            .profiles
            .try_set_workspace(id, &folder, &name)
            .map_err(WorkspaceRefusal::Store)?;
        workspace_answer(written, name, cwd)
    }

    /// The agent's reservation as a working-folder holder. Held by another change = `Changing`; held by a launch =
    /// `Locked` — `Running` when its session is live, else `Preparing` (the shared decision, running first).
    // ADR-0386
    fn reserve_for_working_folder(
        &self,
        id: AgentId,
    ) -> Result<SpawnReservation, WorkspaceRefusal> {
        match SpawnReservation::reserve(self.spawning.clone(), id, ReservationHolder::WorkingFolder)
        {
            Ok(reservation) => Ok(reservation),
            Err(ReservationHolder::WorkingFolder) => Err(WorkspaceRefusal::Changing),
            Err(ReservationHolder::Launch) => {
                let lock = match self.workspace_locked(id)? {
                    Some(WorkspaceLock::Running) => WorkspaceLock::Running,
                    _ => WorkspaceLock::Preparing,
                };
                Err(WorkspaceRefusal::Locked(lock))
            }
        }
    }

    /// The shared lock decision ([`ProfileEditPort::workspace_lock`]) as a refusal.
    fn workspace_locked(&self, id: AgentId) -> Result<Option<WorkspaceLock>, WorkspaceRefusal> {
        self.workspace_lock(id).map_err(|e| match e {
            ProfileEditError::NotFound(_) => WorkspaceRefusal::NotFound,
            ProfileEditError::Store(error) => WorkspaceRefusal::Store(error),
        })
    }

    /// `agent.duplicate` — registers a copy of `id` beside it (same parent), asleep, under the next name of its family
    /// (`bob` → `bob(1)`). The caller asks the backend policy first.
    /// `Err` = `NotFound` (absent, or deleted before the insert — nothing saved for this request) · `RosterFull` · name
    /// space exhausted (`Unsupported`) · [`PtyError::Store`] (nothing registered).
    // ADR-0120
    // ADR-0132
    // ADR-0386
    pub fn try_duplicate_agent(&self, id: AgentId) -> Result<AgentProfile, PtyError> {
        let _gate = self.lock_name_allocation();
        let roster = self.roster();
        let desired = roster
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.canonical_name.clone())
            .ok_or(PtyError::NotFound(id))?;
        check_roster_capacity(&roster)?;
        let copy_id = Uuid::new_v4();
        let name = self.assign_new_name(&roster, copy_id, desired)?;
        self.profiles
            .try_insert_copy(id, |original| duplicate_of(original, copy_id, name))
            .map_err(PtyError::Store)?
            .ok_or(PtyError::NotFound(id))
    }
}

/// The registry write's outcome as `agent.setWorkspace`'s answer.
fn workspace_answer(
    written: WorkspaceWrite,
    name: String,
    cwd: String,
) -> Result<WorkspaceSet, WorkspaceRefusal> {
    match written {
        WorkspaceWrite::Written => Ok(WorkspaceSet { name, cwd }),
        WorkspaceWrite::NotFound => Err(WorkspaceRefusal::NotFound),
        WorkspaceWrite::Conversed => Err(WorkspaceRefusal::Locked(WorkspaceLock::Conversed)),
    }
}

/// The copy of `original` — the constructor's values plus what a duplicate carries: the command (backend, mode,
/// `extra_args`), the working folder, env, the Default slot, the attached presets and the parent. Everything a launch,
/// a conversation or a failure wrote stays at the constructor value (session id and history, incarnation marker, last
/// failure, restart fields, timestamps, the instructions hash, the notice fingerprint, `restart_policy`), and it is not
/// a boot-restore target.
// ADR-0341: a duplicate starts without the notice fingerprint, so its first launch shows its notices.
// ADR-0386
fn duplicate_of(original: &AgentProfile, id: AgentId, name: String) -> AgentProfile {
    let mut copy = AgentProfile::new(
        original.name.clone(),
        original.command.clone(),
        original.cwd.clone(),
        original.env.clone(),
        false,
    );
    copy.id = id;
    copy.display_name = Some(name);
    copy.parent_id = original.parent_id;
    copy.default_preset = original.default_preset;
    copy.presets = original.presets.clone();
    copy
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use uuid::Uuid;

    use super::super::tests::{
        agent_profile, capacity_manager, fill_roster_to, put_live_session, scripted_manager,
    };
    use super::super::MAX_ROSTER_SIZE;
    use super::*;
    use crate::failure::AgentFailureKind;
    use crate::launch::{
        LaunchBlocked, LaunchKit, LaunchKitSource, LaunchPlan, LaunchPorts, NoopLaunchKits,
        PrepareError,
    };
    use crate::profile::scripted_store::{Outcome, ScriptedStore, FAILURES};
    use crate::profile::{AgentCommand, AttachedPreset, RestartPolicy};

    /// A folder that exists — the stat passes — in the spelling the manager stores (`\` → `/`).
    fn existing_folder() -> String {
        normalize_spelling(&std::env::temp_dir().to_string_lossy())
    }

    /// A manager with one asleep, never-conversed agent — `display_name` as given.
    fn seeded(display_name: Option<&str>) -> (AgentManager, Arc<ScriptedStore>, AgentId) {
        let (manager, store) = scripted_manager();
        let p = agent_profile("C:/old/alpha", display_name);
        let id = p.id;
        manager.profiles.upsert(p);
        (manager, store, id)
    }

    fn converse(manager: &AgentManager, id: AgentId) {
        assert!(manager
            .profiles
            .update_with(id, |p| p.backend_session_id = Some(Uuid::new_v4())));
    }

    fn cwd_of(manager: &AgentManager, id: AgentId) -> std::path::PathBuf {
        manager.profiles.get(id).expect("in the roster").cwd
    }

    /// The refusal of a reserve step expected to refuse (`SpawnReservation` has no `Debug`, so no `unwrap_err`).
    fn refused(step: Result<SpawnReservation, WorkspaceRefusal>) -> WorkspaceRefusal {
        match step {
            Ok(_) => panic!("the reserve step took the reservation"),
            Err(refusal) => refusal,
        }
    }

    fn locked(refusal: WorkspaceRefusal) -> WorkspaceLock {
        match refusal {
            WorkspaceRefusal::Locked(lock) => lock,
            other => panic!("not a lock refusal: {other:?}"),
        }
    }

    /// The Noop kits with a folder stat that answers `why` for every folder.
    struct StatAnswers(&'static str);

    impl LaunchKitSource for StatAnswers {
        fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked> {
            NoopLaunchKits.check(profile)
        }
        fn prepare(
            &self,
            plan: LaunchPlan,
            id: AgentId,
            epoch: u32,
        ) -> Result<LaunchKit, PrepareError> {
            NoopLaunchKits.prepare(plan, id, epoch)
        }
        fn release(&self, _id: AgentId, _epoch: u32) {}
        fn stat_folder(&self, _folder: &Path) -> Result<(), String> {
            Err(self.0.to_owned())
        }
    }

    // ── agent.setWorkspace ──

    #[test]
    fn set_workspace_writes_the_folder_and_pins_the_name_only_when_none() {
        let target = existing_folder();
        let (manager, store, id) = seeded(None);
        let set = manager
            .try_set_workspace(id, &format!("\"{target}\""))
            .expect("an asleep agent that never conversed");
        assert_eq!(
            set,
            WorkspaceSet {
                name: "alpha".into(),
                cwd: target.clone()
            },
            "the quoted spelling is normalized and the old folder's name is kept"
        );
        let stored = manager.profiles.get(id).unwrap();
        assert_eq!(stored.cwd, Path::new(&target));
        assert_eq!(stored.display_name.as_deref(), Some("alpha"));
        assert_eq!(store.disk(id).unwrap().cwd, Path::new(&target), "saved");

        let (manager, _store, id) = seeded(Some("keep"));
        let set = manager.try_set_workspace(id, &target).unwrap();
        assert_eq!(set.name, "keep");
        assert_eq!(
            manager.profiles.get(id).unwrap().display_name.as_deref(),
            Some("keep")
        );
    }

    #[test]
    fn set_workspace_refuses_a_folder_that_does_not_stat_and_names_why() {
        let (manager, store, id) = seeded(Some("a"));
        let gone = std::env::temp_dir().join(format!("engram-no-such-{}", Uuid::new_v4()));
        let attempts = store.attempts();
        let err = manager
            .try_set_workspace(id, &gone.to_string_lossy())
            .unwrap_err();
        assert!(
            matches!(&err, WorkspaceRefusal::FolderNotFound { cwd, why }
                if Path::new(cwd) == gone && why == "missing"),
            "{err:?}"
        );
        assert_eq!(cwd_of(&manager, id), Path::new("C:/old/alpha"));
        assert_eq!(store.attempts(), attempts, "nothing saved");
    }

    /// A stat that timed out (the library's bounded IO answers its timeout word) refuses `workspace-not-found` with that
    /// word, before any lock or reservation.
    #[test]
    fn set_workspace_refuses_a_timed_out_stat_with_the_timeout_in_why() {
        let (manager, _store, id) = seeded(Some("a"));
        let manager = manager.with_launch_ports(LaunchPorts {
            kits: Arc::new(StatAnswers("timeout")),
            ..LaunchPorts::noop()
        });
        let err = manager
            .try_set_workspace(id, &existing_folder())
            .unwrap_err();
        assert!(
            matches!(&err, WorkspaceRefusal::FolderNotFound { why, .. } if why == "timeout"),
            "{err:?}"
        );
        assert!(manager.reservation_holder(id).is_none());
    }

    #[test]
    fn set_workspace_refuses_each_lock_reason_and_writes_nothing() {
        let target = existing_folder();

        let (manager, store, id) = seeded(Some("a"));
        put_live_session(&manager, id, 1);
        let attempts = store.attempts();
        let err = manager.try_set_workspace(id, &target).unwrap_err();
        assert_eq!(locked(err), WorkspaceLock::Running);
        assert_eq!(store.attempts(), attempts, "running: nothing saved");
        assert_eq!(cwd_of(&manager, id), Path::new("C:/old/alpha"));

        let (manager, store, id) = seeded(Some("a"));
        {
            let _launch =
                SpawnReservation::reserve(manager.spawning.clone(), id, ReservationHolder::Launch)
                    .expect("reserve");
            let attempts = store.attempts();
            let err = manager.try_set_workspace(id, &target).unwrap_err();
            assert_eq!(locked(err), WorkspaceLock::Preparing);
            assert_eq!(store.attempts(), attempts, "preparing: nothing saved");
            assert_eq!(
                manager.reservation_holder(id),
                Some(ReservationHolder::Launch),
                "the launch keeps its reservation"
            );
            assert_eq!(cwd_of(&manager, id), Path::new("C:/old/alpha"));
        }

        let (manager, store, id) = seeded(Some("a"));
        converse(&manager, id);
        let attempts = store.attempts();
        let err = manager.try_set_workspace(id, &target).unwrap_err();
        assert_eq!(locked(err), WorkspaceLock::Conversed);
        assert_eq!(store.attempts(), attempts, "conversed: nothing saved");
        assert_eq!(cwd_of(&manager, id), Path::new("C:/old/alpha"));
    }

    /// Another working-folder change holding the reservation refuses `changing` and leaves that holder in place.
    #[test]
    fn set_workspace_refuses_changing_while_another_change_holds_the_reservation() {
        let (manager, store, id) = seeded(Some("a"));
        let _other = SpawnReservation::reserve(
            manager.spawning.clone(),
            id,
            ReservationHolder::WorkingFolder,
        )
        .expect("reserve");
        let attempts = store.attempts();
        let err = manager
            .try_set_workspace(id, &existing_folder())
            .unwrap_err();
        assert!(matches!(err, WorkspaceRefusal::Changing), "{err:?}");
        assert_eq!(store.attempts(), attempts, "nothing saved");
        assert_eq!(
            manager.reservation_holder(id),
            Some(ReservationHolder::WorkingFolder)
        );
        assert_eq!(cwd_of(&manager, id), Path::new("C:/old/alpha"));
    }

    #[test]
    fn set_workspace_of_an_agent_not_in_the_roster_is_not_found() {
        let (manager, store, _id) = seeded(Some("a"));
        let attempts = store.attempts();
        let err = manager
            .try_set_workspace(Uuid::new_v4(), &existing_folder())
            .unwrap_err();
        assert!(matches!(err, WorkspaceRefusal::NotFound), "{err:?}");
        assert_eq!(store.attempts(), attempts, "nothing saved");
    }

    /// A blank override names nothing (`canonical_name_when_live`), so the name derived from the old folder is pinned
    /// over it — else the folder change would rename the agent.
    #[test]
    fn set_workspace_pins_the_name_over_a_blank_override() {
        let (manager, _store, id) = seeded(Some("  "));
        let set = manager.try_set_workspace(id, &existing_folder()).unwrap();
        assert_eq!(set.name, "alpha");
        assert_eq!(
            manager.profiles.get(id).unwrap().display_name.as_deref(),
            Some("alpha")
        );
    }

    /// A relative `cwd` refuses `not absolute` before the stat runs (the kits here would answer every stat with their
    /// own word).
    #[test]
    fn set_workspace_refuses_a_relative_folder_before_the_stat() {
        for relative in ["sub", "."] {
            let (manager, store, id) = seeded(Some("a"));
            let manager = manager.with_launch_ports(LaunchPorts {
                kits: Arc::new(StatAnswers("the stat ran")),
                ..LaunchPorts::noop()
            });
            let attempts = store.attempts();
            let err = manager.try_set_workspace(id, relative).unwrap_err();
            assert!(
                matches!(&err, WorkspaceRefusal::FolderNotFound { cwd, why }
                    if cwd == relative && why == NOT_ABSOLUTE),
                "{relative}: {err:?}"
            );
            assert_eq!(store.attempts(), attempts, "{relative}: nothing saved");
            assert_eq!(cwd_of(&manager, id), Path::new("C:/old/alpha"));
        }
    }

    /// The reserve step on its own: free → held as a working-folder change; held by another change → `Changing`; held by
    /// a launch → `Running` when the session is live, else `Preparing`; held by a launch for an agent gone from the
    /// roster → `NotFound`.
    #[test]
    fn the_reserve_step_classifies_who_holds_the_reservation() {
        let (manager, _store, id) = seeded(Some("a"));
        {
            let _mine = manager.reserve_for_working_folder(id).expect("free");
            assert_eq!(
                manager.reservation_holder(id),
                Some(ReservationHolder::WorkingFolder)
            );
            let err = refused(manager.reserve_for_working_folder(id));
            assert!(matches!(err, WorkspaceRefusal::Changing), "{err:?}");
        }
        assert!(manager.reservation_holder(id).is_none(), "released on drop");

        let launch = |manager: &AgentManager, id| {
            SpawnReservation::reserve(manager.spawning.clone(), id, ReservationHolder::Launch)
                .expect("reserve")
        };
        let _launch = launch(&manager, id);
        assert_eq!(
            locked(refused(manager.reserve_for_working_folder(id))),
            WorkspaceLock::Preparing
        );
        put_live_session(&manager, id, 1);
        assert_eq!(
            locked(refused(manager.reserve_for_working_folder(id))),
            WorkspaceLock::Running
        );
        assert_eq!(
            manager.reservation_holder(id),
            Some(ReservationHolder::Launch),
            "the launch keeps it"
        );

        let gone = Uuid::new_v4();
        let _gone = launch(&manager, gone);
        let err = refused(manager.reserve_for_working_folder(gone));
        assert!(matches!(err, WorkspaceRefusal::NotFound), "{err:?}");
    }

    /// The registry write's outcome maps onto the answer: written → set, gone → `NotFound`, conversed (a session id
    /// written after the manager's re-check) → `Locked(Conversed)`.
    #[test]
    fn the_registry_outcome_maps_onto_the_answer() {
        let set = workspace_answer(WorkspaceWrite::Written, "a".into(), "C:/w".into()).unwrap();
        assert_eq!(
            set,
            WorkspaceSet {
                name: "a".into(),
                cwd: "C:/w".into()
            }
        );
        let gone = workspace_answer(WorkspaceWrite::NotFound, "a".into(), "C:/w".into());
        assert!(matches!(gone, Err(WorkspaceRefusal::NotFound)), "{gone:?}");
        let conversed = workspace_answer(WorkspaceWrite::Conversed, "a".into(), "C:/w".into());
        assert_eq!(locked(conversed.unwrap_err()), WorkspaceLock::Conversed);
    }

    /// The reservation is released on every exit — a later launch is not refused by a finished change.
    #[test]
    fn set_workspace_releases_the_reservation_on_success_and_refusal() {
        let (manager, _store, id) = seeded(Some("a"));
        manager.try_set_workspace(id, &existing_folder()).unwrap();
        assert!(manager.reservation_holder(id).is_none());
        converse(&manager, id);
        manager
            .try_set_workspace(id, &existing_folder())
            .unwrap_err();
        assert!(manager.reservation_holder(id).is_none());
    }

    #[test]
    fn a_store_failure_of_the_write_refuses_store_and_changes_nothing() {
        for failure in FAILURES {
            let (manager, store, id) = seeded(None);
            store.set(failure);
            let err = manager
                .try_set_workspace(id, &existing_folder())
                .unwrap_err();
            assert!(
                matches!(
                    (failure, &err),
                    (Outcome::FailsIo, WorkspaceRefusal::Store(StoreError::Io(_)))
                        | (
                            Outcome::Refuses(_),
                            WorkspaceRefusal::Store(StoreError::Refused(_))
                        )
                ),
                "{failure:?} → {err:?}"
            );
            let stored = manager.profiles.get(id).unwrap();
            assert_eq!(stored.cwd, Path::new("C:/old/alpha"), "{failure:?}");
            assert_eq!(stored.display_name, None, "{failure:?}: not pinned");
            assert!(!manager.profiles.is_dirty(), "{failure:?}");
        }
    }

    /// ★Pins the order in the source★ — the tests above run one call at a time, so they stay green with the stat moved
    /// under a lock, the reservation taken before the pre-check (a refused call would hold it and refuse a concurrent
    /// start), or the name derived outside the gate (a concurrent rename could take it — ADR-0120).
    #[test]
    fn set_workspace_runs_stat_precheck_reservation_recheck_gate_write_in_that_order() {
        let src = include_str!("roster_edit.rs");
        let production = src.split("mod tests {").next().expect("production section");
        let body: String = production
            .split("pub fn try_set_workspace(")
            .nth(1)
            .expect("the body")
            .split("fn reserve_for_working_folder(")
            .next()
            .expect("up to the next function")
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("//"))
            .collect::<String>()
            .split_whitespace()
            .collect();
        // The body asks the decision exactly twice — before the reservation and after it; the reserve branch's own
        //   check is in `reserve_for_working_folder`, outside this body.
        assert_eq!(
            body.matches("self.workspace_locked(id)?").count(),
            2,
            "{body}"
        );
        let mut last = 0;
        let mut from = 0;
        for needle in [
            "if!folder.is_absolute()",
            "self.launch.kits.stat_folder(&folder)",
            "self.workspace_locked(id)?",
            "self.reserve_for_working_folder(id)?",
            "self.workspace_locked(id)?",
            "self.lock_name_allocation()",
            ".canonical_name_when_live()",
            ".try_set_workspace(id,&folder,&name)",
        ] {
            let at = body[from..]
                .find(needle)
                .map(|i| i + from)
                .unwrap_or_else(|| panic!("`{needle}` is not after the step before it: {body}"));
            assert!(at >= last);
            last = at;
            from = at + needle.len();
        }
    }

    // ── the conditional registry write ──

    /// A session id written after the manager's re-check is caught inside the write: it refuses, writes nothing and —
    /// with no pending change — attempts no save.
    #[test]
    fn the_registry_write_refuses_a_conversed_agent_without_a_save_attempt() {
        let (manager, store, id) = seeded(None);
        converse(&manager, id);
        let attempts = store.attempts();
        let outcome = manager
            .profiles
            .try_set_workspace(id, Path::new("C:/new"), "alpha")
            .unwrap();
        assert_eq!(outcome, WorkspaceWrite::Conversed);
        assert_eq!(store.attempts(), attempts);
        let stored = manager.profiles.get(id).unwrap();
        assert_eq!(stored.cwd, Path::new("C:/old/alpha"));
        assert_eq!(stored.display_name, None);
    }

    /// An agent deleted meanwhile is `NotFound` with no save attempt — and a failing store does not replace it.
    #[test]
    fn the_registry_write_of_a_deleted_agent_is_not_found_without_a_save_attempt() {
        let (manager, store, _id) = seeded(None);
        store.set(Outcome::FailsIo);
        let attempts = store.attempts();
        let outcome = manager
            .profiles
            .try_set_workspace(Uuid::new_v4(), Path::new("C:/new"), "x")
            .expect("a refusal is not a store error");
        assert_eq!(outcome, WorkspaceWrite::NotFound);
        assert_eq!(store.attempts(), attempts);
    }

    /// The same folder with the name already pinned changes nothing and saves nothing.
    #[test]
    fn the_registry_write_of_the_same_folder_saves_nothing() {
        let (manager, store, id) = seeded(Some("a"));
        let attempts = store.attempts();
        let outcome = manager
            .profiles
            .try_set_workspace(id, Path::new("C:/old/alpha"), "a")
            .unwrap();
        assert_eq!(outcome, WorkspaceWrite::Written);
        assert_eq!(store.attempts(), attempts);
    }

    // ── agent.duplicate ──

    /// An original with every runtime-written field filled — what a duplicate must not carry.
    fn worn_original(manager: &AgentManager, parent: Option<AgentId>) -> AgentProfile {
        let mut p = AgentProfile::new(
            "raw-name".into(),
            AgentCommand::Codex {
                extra_args: vec!["--x".into()],
                output_format: crate::profile::AgentOutputFormat::Terminal,
            },
            "C:/work/bob".into(),
            vec![("K".into(), "v".into())],
            true,
        );
        p.display_name = Some("bob".into());
        p.parent_id = parent;
        p.default_preset = false;
        p.presets = vec![AttachedPreset {
            path: "/team/a.yaml".into(),
        }];
        p.backend_session_id = Some(Uuid::new_v4());
        p.old_session_ids = vec![Uuid::new_v4()];
        p.epoch = 7;
        p.last_applied_instructions_hash = Some("hash".into());
        p.notice_fingerprint = Some("fp".into());
        p.restart_policy = RestartPolicy::Never;
        p.restart_count = 3;
        p.failed_reason = Some("crashed".into());
        p.created_at = 1;
        p.last_active = 1;
        p.last_start_at = Some(1);
        manager.profiles.upsert(p.clone());
        manager
            .profiles
            .set_last_failure(p.id, None, Some(AgentFailureKind::SpawnFailed));
        p
    }

    #[test]
    fn a_duplicate_carries_the_setup_and_starts_fresh() {
        let (manager, store, _) = seeded(Some("lead"));
        let lead = manager
            .roster()
            .into_iter()
            .find(|e| e.canonical_name == "lead")
            .unwrap()
            .id;
        let original = worn_original(&manager, Some(lead));

        let copy = manager
            .try_duplicate_agent(original.id)
            .expect("duplicated");

        assert_ne!(copy.id, original.id);
        assert_eq!(copy.display_name.as_deref(), Some("bob(1)"));
        assert_eq!(copy.name, original.name);
        assert_eq!(copy.command, original.command);
        assert_eq!(copy.cwd, original.cwd);
        assert_eq!(copy.env, original.env);
        assert_eq!(copy.default_preset, original.default_preset);
        assert_eq!(copy.presets, original.presets);
        assert_eq!(copy.parent_id, Some(lead), "a sibling of the original");
        let fresh = AgentProfile::new(
            String::new(),
            copy.command.clone(),
            "".into(),
            vec![],
            false,
        );
        assert_eq!(copy.backend_session_id, None);
        assert!(copy.old_session_ids.is_empty());
        assert_eq!(copy.epoch, fresh.epoch);
        assert_eq!(copy.last_failure, None);
        assert_eq!(copy.failed_reason, None);
        assert_eq!(copy.restart_count, 0);
        assert_eq!(copy.restart_policy, fresh.restart_policy);
        assert_eq!(copy.last_start_at, None);
        assert!(copy.created_at > original.created_at);
        assert!(!copy.auto_restore, "not a boot-restore target");
        assert_eq!(copy.last_applied_instructions_hash, None);
        assert_eq!(copy.notice_fingerprint, None);

        let stored = manager.profiles.get(copy.id).expect("registered");
        assert_eq!(stored.display_name.as_deref(), Some("bob(1)"));
        assert_eq!(stored.last_failure, None);
        assert!(store.disk(copy.id).is_some(), "saved");
        assert_eq!(
            manager.profiles.get(original.id).unwrap().last_failure,
            Some(AgentFailureKind::SpawnFailed),
            "the original is untouched"
        );
    }

    #[test]
    fn duplicates_take_the_next_name_of_the_family_and_a_top_level_original_stays_top_level() {
        let (manager, _store, _) = seeded(Some("other"));
        let original = worn_original(&manager, None);
        let first = manager.try_duplicate_agent(original.id).unwrap();
        let second = manager.try_duplicate_agent(original.id).unwrap();
        assert_eq!(first.display_name.as_deref(), Some("bob(1)"));
        assert_eq!(second.display_name.as_deref(), Some("bob(2)"));
        assert_eq!((first.parent_id, second.parent_id), (None, None));
    }

    #[test]
    fn a_duplicate_at_the_roster_ceiling_is_roster_full_and_registers_nothing() {
        let manager = capacity_manager();
        fill_roster_to(&manager, MAX_ROSTER_SIZE);
        let original = manager.roster()[0].id;
        let err = manager.try_duplicate_agent(original).unwrap_err();
        assert!(
            matches!(
                err,
                PtyError::RosterFull {
                    current: MAX_ROSTER_SIZE,
                    limit: MAX_ROSTER_SIZE
                }
            ),
            "{err}"
        );
        assert_eq!(manager.roster().len(), MAX_ROSTER_SIZE);
    }

    /// An original that is not there is `NotFound`; with no pending change no save is attempted, and a failing store
    /// does not mask it.
    #[test]
    fn duplicating_a_missing_original_is_not_found_and_saves_nothing() {
        let (manager, store, _) = seeded(Some("a"));
        store.set(Outcome::FailsIo);
        let attempts = store.attempts();
        let gone = Uuid::new_v4();
        let err = manager.try_duplicate_agent(gone).unwrap_err();
        assert!(matches!(err, PtyError::NotFound(id) if id == gone), "{err}");
        assert_eq!(store.attempts(), attempts);

        let outcome = manager
            .profiles
            .try_insert_copy(gone, |_| unreachable!("no original to copy"))
            .expect("a missing original is not a store error");
        assert!(outcome.is_none());
        assert_eq!(
            store.attempts(),
            attempts,
            "the registry method saves nothing"
        );
        assert_eq!(manager.roster().len(), 1);
    }

    #[test]
    fn a_store_failure_of_the_insert_registers_nothing() {
        for failure in FAILURES {
            let (manager, store, id) = seeded(Some("a"));
            store.set(failure);
            let err = manager.try_duplicate_agent(id).unwrap_err();
            assert!(matches!(err, PtyError::Store(_)), "{failure:?}: {err}");
            assert_eq!(manager.roster().len(), 1, "{failure:?}");
            assert!(!manager.profiles.is_dirty(), "{failure:?}");
        }
    }

    /// A rewrite after the duplicate reaches the copy too — it rewrites the current roster.
    #[test]
    fn a_preset_path_rewrite_after_a_duplicate_rewrites_the_copy() {
        use crate::profile_edit::PresetPathRename;
        let (manager, _store, _) = seeded(Some("x"));
        let original = worn_original(&manager, None);
        let copy = manager.try_duplicate_agent(original.id).unwrap();
        let mut changed = manager
            .rewrite_preset_paths(&[PresetPathRename {
                from: "/team/a.yaml".into(),
                to: "/team/b.yaml".into(),
            }])
            .unwrap();
        changed.sort_unstable();
        let mut expected = vec![original.id, copy.id];
        expected.sort_unstable();
        assert_eq!(changed, expected);
    }

    /// ★Pins the one locked section in the source★ — a copy read with `get` and inserted with `try_upsert` would be two
    /// locked sections, and a relink's rewrite landing between them would leave the copy on the old path; one thread
    /// cannot show that window.
    #[test]
    fn the_copy_is_read_and_inserted_in_one_locked_section() {
        let profile = include_str!("../profile.rs");
        let insert = profile
            .split("pub(crate) fn try_insert_copy(")
            .nth(1)
            .expect("`try_insert_copy`")
            .split("Ok(inserted)")
            .next()
            .expect("its body");
        let section = insert
            .split("self.try_mutate_if(|m| {")
            .nth(1)
            .unwrap_or_else(|| panic!("not one conditional locked section: {insert}"));
        let read = section
            .find("m.get(&original)")
            .expect("reads the original inside");
        let write = section.find("m.insert(").expect("inserts inside");
        assert!(read < write, "{insert}");

        let src = include_str!("roster_edit.rs");
        let production = src.split("mod tests {").next().expect("production section");
        let body = production
            .split("pub fn try_duplicate_agent(")
            .nth(1)
            .expect("the body")
            .split("fn duplicate_of(")
            .next()
            .expect("up to the next function");
        assert!(body.contains(".try_insert_copy("), "{body}");
        for split in ["self.profiles.get(", "try_upsert"] {
            assert!(
                !body.contains(split),
                "`{split}` splits the section: {body}"
            );
        }
    }
}
