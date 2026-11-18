//! ③ reaper exit classification integration test — asserts the ADR-0019 disposition with real shell spawns.
//!
//! The first four shell-launching tests are single spawns (no named-mutex/global contention), left as default.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use uuid::Uuid;

use engram_dashboard_agent::backend::{AgentBackend, ShellBackend};
use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
use engram_dashboard_agent::persistence::FileProfileStore;
use engram_dashboard_agent::profile::{AgentCommand, AgentProfile, ProfileRegistry, SpawnMode};
use engram_dashboard_agent::reaper::ReaperDeps;
use engram_dashboard_agent::session::AgentSession;
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::transport::api::ApiTransport;
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, ReapMsg, StatusSink, TerminalReason, TerminationIntent,
};
use engram_dashboard_base::testing::wait_until;

#[derive(Clone)]
struct CountingSink {
    list_updates: Arc<AtomicUsize>,
    statuses: Arc<Mutex<Vec<AgentStatus>>>,
}

impl CountingSink {
    fn new() -> Self {
        Self {
            list_updates: Arc::new(AtomicUsize::new(0)),
            statuses: Arc::new(Mutex::new(Vec::new())),
        }
    }
    fn list_update_count(&self) -> usize {
        self.list_updates.load(Ordering::SeqCst)
    }
}

impl StatusSink for CountingSink {
    fn status_changed(&self, _id: AgentId, status: AgentStatus, _epoch: u32) {
        self.statuses.lock().unwrap().push(status);
    }
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {
        self.list_updates.fetch_add(1, Ordering::SeqCst);
    }
}

fn make_manager(tag: &str) -> (AgentManager, CountingSink, Arc<ProfileRegistry>) {
    let sink = CountingSink::new();
    let sink_dyn: Arc<dyn StatusSink> = Arc::new(sink.clone());
    let store = Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-test-reaper-{tag}-{}", Uuid::new_v4())),
    ));
    let profiles = Arc::new(ProfileRegistry::new(store));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = AgentManager::new(sink_dyn, profiles.clone(), tracker);
    (manager, sink, profiles)
}

#[cfg(windows)]
fn exit_profile(code: i32) -> AgentProfile {
    AgentProfile::new(
        "reaper-test".into(),
        AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), format!("exit {code}")],
        },
        PathBuf::from("."),
        vec![],
        false,
    )
}

#[cfg(not(windows))]
fn exit_profile(code: i32) -> AgentProfile {
    AgentProfile::new(
        "reaper-test".into(),
        AgentCommand::Shell {
            program: "sh".into(),
            args: vec!["-c".into(), format!("exit {code}")],
        },
        PathBuf::from("."),
        vec![],
        false,
    )
}

// ── Natural exit (exit 0) ────────────────────────────────────────────────────────
#[test]
fn natural_exit_zero_reaps_and_keeps_profile_corpse() {
    let (manager, sink, profiles) = make_manager("exit0");
    let profile = exit_profile(0);
    let id = profile.id;

    let updates_before = sink.list_update_count();
    manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn failed")
        .into_started()
        .expect("this call actually launches (not a duplicate request)");

    let removed = wait_until(Duration::from_secs(15), || manager.list_agents().is_empty());
    if !removed {
        let agents = manager.list_agents();
        eprintln!(
            "PROBE exit0 still present: {:?}",
            agents.iter().map(|a| &a.status).collect::<Vec<_>>()
        );
    }
    assert!(
        removed,
        "exit0: the reaper did not remove the session from the map"
    );
    assert!(
        wait_until(Duration::from_secs(2), || {
            profiles.get(id).map(|p| !p.auto_restore).unwrap_or(false)
        }),
        "exit0: a corpse of a normal exit must keep the profile + auto_restore=false (ADR-0083 — not deleted)"
    );
    assert!(
        profiles.get(id).is_some(),
        "exit0: a normal exit but the profile was deleted — it must be kept as a corpse (ADR-0083)"
    );
    assert!(
        sink.list_update_count() > updates_before,
        "exit0: the reaper did not notify agent_list_updated"
    );
}

// ── Crash (exit 1) ───────────────────────────────────────────────────────────
#[test]
fn crash_exit_one_keeps_profile_disables_auto_restore() {
    let (manager, _sink, profiles) = make_manager("exit1");
    let profile = exit_profile(1);
    let id = profile.id;

    manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn failed")
        .into_started()
        .expect("this call actually launches (not a duplicate request)");

    assert!(
        wait_until(Duration::from_secs(5), || manager.list_agents().is_empty()),
        "exit1: the reaper did not remove the session"
    );
    assert!(
        wait_until(Duration::from_secs(2), || {
            profiles.get(id).map(|p| !p.auto_restore).unwrap_or(false)
        }),
        "exit1: after a crash it is not profile kept + auto_restore=false (it must exist and be false)"
    );
    assert!(
        profiles.get(id).is_some(),
        "exit1: a crash but the profile was deleted (it must be kept)"
    );
}

// ── User kill ────────────────────────────────────────────────────────────────
#[test]
fn user_kill_keeps_profile_corpse_with_session_id() {
    let (manager, _sink, profiles) = make_manager("userkill");
    let profile = AgentProfile::new(
        "reaper-kill".into(),
        AgentCommand::Shell {
            program: engram_dashboard_platform::shell::default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        true, // start with auto_restore=true → can assert whether the kill collection downgrades it to false.
    );
    let id = profile.id;
    let sid = Uuid::new_v4();
    let mut seeded = profile.clone();
    seeded.backend_session_id = Some(sid);
    profiles.try_upsert(seeded.clone()).expect("save succeeds");

    let info = manager
        .spawn_agent(&seeded, SpawnMode::Fresh)
        .expect("spawn failed")
        .into_started()
        .expect("this call actually launches (not a duplicate request)");

    assert!(
        wait_until(Duration::from_secs(2), || manager.list_agents().len() == 1),
        "userkill: no session right after spawn"
    );

    manager.kill_agent(info.id).expect("kill_agent failed");

    assert!(
        wait_until(Duration::from_secs(5), || manager.list_agents().is_empty()),
        "userkill: the reaper did not remove the session from the map"
    );
    assert!(
        wait_until(Duration::from_secs(2), || {
            profiles.get(id).map(|p| !p.auto_restore).unwrap_or(false)
        }),
        "userkill: a corpse of a user kill must keep the profile + auto_restore=false (ADR-0083 — not deleted)"
    );
    assert!(
        profiles.get(id).is_some(),
        "userkill: a user kill but the profile was deleted — it must be kept as a corpse (ADR-0083)"
    );
    assert_eq!(
        profiles.get(id).and_then(|p| p.backend_session_id),
        Some(sid),
        "userkill: backend_session_id was lost — reactivation cannot resume (ADR-0083 regression)"
    );
}

// ── Exit during shutdown_all ─────────────────────────────────────────────────────
#[test]
fn shutdown_all_keeps_profiles_for_boot_restore() {
    let (manager, _sink, profiles) = make_manager("shutdown");
    let profile = AgentProfile::new(
        "reaper-shutdown".into(),
        AgentCommand::Shell {
            program: engram_dashboard_platform::shell::default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    let id = profile.id;
    manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn failed")
        .into_started()
        .expect("this call actually launches (not a duplicate request)");
    assert!(
        wait_until(Duration::from_secs(2), || manager.list_agents().len() == 1),
        "shutdown: no session right after spawn"
    );

    assert!(
        profiles.get(id).map(|p| p.auto_restore).unwrap_or(false),
        "shutdown: auto_restore is not true after spawn (activation rule)"
    );

    manager.shutdown_all();

    assert!(
        wait_until(Duration::from_secs(5), || manager.list_agents().is_empty()),
        "shutdown: session map removal failed"
    );
    let p = profiles
        .get(id)
        .expect("shutdown: the profile was deleted (it must be kept)");
    assert!(
        p.auto_restore,
        "shutdown: auto_restore dropped to false — dropped from boot restore targets (KeepAsIs violation)"
    );
}

// ── Deterministic reap_one assertions (timing-independent) ─────────────────────────────────────────────

fn make_reaper_deps(tag: &str) -> (Arc<ProfileRegistry>, CountingSink, ReaperDeps) {
    let sink = CountingSink::new();
    let sink_dyn: Arc<dyn StatusSink> = Arc::new(sink.clone());
    let store = Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-test-reaper-{tag}-{}", Uuid::new_v4())),
    ));
    let profiles = Arc::new(ProfileRegistry::new(store));
    let sessions: Arc<RwLock<HashMap<AgentId, Arc<AgentSession>>>> =
        Arc::new(RwLock::new(HashMap::new()));
    let deps = ReaperDeps {
        sessions,
        profiles: profiles.clone(),
        status_sink: sink_dyn,
        control: Arc::new(engram_dashboard_agent::types::NoopControlChannel),
    };
    (profiles, sink, deps)
}

/// A session put into the map with an ApiTransport (stub) plugged in, without real resources or a pump — start/kill are not called.
fn make_test_session(
    id: AgentId,
    epoch: u32,
    status_sink: Arc<dyn StatusSink>,
) -> Arc<AgentSession> {
    let core = Arc::new(OutputCore::new(
        id,
        epoch,
        status_sink,
        TurnWiring::detached(),
    ));
    let intent = Arc::new(AtomicU8::new(TerminationIntent::None as u8));
    // It is ApiTransport, so the caps content is irrelevant — the shell caps are put in as a dummy satisfying the composition path.
    let shell_cmd = engram_dashboard_agent::profile::AgentCommand::Shell {
        program: "cmd.exe".into(),
        args: vec![],
    };
    Arc::new(AgentSession::new(
        id,
        PathBuf::from("."),
        epoch,
        80,
        24,
        intent,
        ShellBackend.capabilities(&shell_cmd),
        engram_dashboard_agent::backend::InputEncoder::Raw,
        // ★Do not write it as a literal★: "live session value == backend-derived value" is the premise of the mail eligibility defense, so
        //   a harness holding the opposite of the real thing would break that premise in-tree first. Derived from the same command.
        engram_dashboard_agent::backend::reads_messages(&shell_cmd),
        core,
        Box::new(ApiTransport::new()),
    ))
}

// ── epoch race ───────────────────────────────────────────────────────────────
#[test]
fn epoch_mismatch_does_not_reap_current_session() {
    let (profiles, sink, deps) = make_reaper_deps("epoch-race");
    let id = Uuid::new_v4();

    // epoch=1 = the "current" session whose marker changed by respawn (the value itself has no meaning — it only needs to differ from 0).
    let status_dyn: Arc<dyn StatusSink> = Arc::new(sink.clone());
    let session = make_test_session(id, 1, status_dyn);
    deps.sessions.write().unwrap().insert(id, session);

    // The profile is kept under any disposition, so existence cannot tell them apart — set auto_restore=true and decide by whether a wrong
    //   downgrade happens.
    let mut profile = exit_profile(0);
    profile.id = id;
    profile.auto_restore = true;
    profiles.try_upsert(profile).expect("save succeeds");

    let updates_before = sink.list_update_count();

    let stale = ReapMsg {
        id,
        epoch: 0,
        reason: TerminalReason::Exited { code: Some(0) },
        intent_at_finish: TerminationIntent::None,
        shutting_down_at_finish: false,
    };
    deps.reap_one(stale);

    assert!(
        deps.sessions.read().unwrap().contains_key(&id),
        "epoch race: an epoch-mismatched done wrongly removed the current (epoch=1) session"
    );
    assert!(
        profiles.get(id).map(|p| p.auto_restore).unwrap_or(false),
        "epoch race: an epoch mismatch but the disposition (auto_restore downgrade) was applied"
    );
    assert_eq!(
        sink.list_update_count(),
        updates_before,
        "epoch race: an epoch mismatch but an agent_list_updated notification happened"
    );
}

// ── idempotency ──────────────────────────────────────────────────────────────
#[test]
fn duplicate_reap_processes_exactly_once() {
    let (profiles, sink, deps) = make_reaper_deps("idempotency");
    let id = Uuid::new_v4();

    let status_dyn: Arc<dyn StatusSink> = Arc::new(sink.clone());
    let session = make_test_session(id, 0, status_dyn);
    deps.sessions.write().unwrap().insert(id, session);

    // auto_restore=true is needed for the first downgrade to be observed.
    let mut profile = exit_profile(0);
    profile.id = id;
    profile.auto_restore = true;
    profiles.try_upsert(profile).expect("save succeeds");

    let updates_before = sink.list_update_count();

    let done = ReapMsg {
        id,
        epoch: 0,
        reason: TerminalReason::Exited { code: Some(0) },
        intent_at_finish: TerminationIntent::None,
        shutting_down_at_finish: false,
    };

    deps.reap_one(done.clone());
    assert!(
        !deps.sessions.read().unwrap().contains_key(&id),
        "idempotency: on the first round the session was not removed from the map"
    );
    assert!(
        profiles.get(id).map(|p| !p.auto_restore).unwrap_or(false),
        "idempotency: on the first round the disposition (profile kept + auto_restore=false) was not applied"
    );
    assert_eq!(
        sink.list_update_count(),
        updates_before + 1,
        "idempotency: the first round's notification is not exactly once"
    );

    // The real ground of the second round's no-op is the map lookup failure — it returns as is at the epoch check section.
    deps.reap_one(done);
    assert_eq!(
        sink.list_update_count(),
        updates_before + 1,
        "idempotency: the second duplicate reap caused an extra notification (violates exactly once)"
    );
    assert!(
        profiles.get(id).map(|p| !p.auto_restore).unwrap_or(false),
        "idempotency: on the second round the profile state wavered (must be kept + auto_restore=false)"
    );
}

// ── ADR-0084 apply_disposition epoch-guard ───────────────────────────────────
//
// ★Do not call reap_one with the map empty★ — the epoch=0 session must stay in the map to pass sessions.remove and reach
//   apply_disposition. The remove guard looks at the **session** epoch and the disposition guard at the **profile** epoch, so the session
//   matches and only the profile is raised to 1.
#[test]
fn stale_disposition_does_not_downgrade_reactivated_live_session() {
    let (profiles, _sink, deps) = make_reaper_deps("disp-epoch-guard");
    let id = Uuid::new_v4();

    let status_dyn: Arc<dyn StatusSink> = Arc::new(_sink.clone());
    let dead = make_test_session(id, 0, status_dyn);
    deps.sessions.write().unwrap().insert(id, dead);

    // Imitates a live session where reactivation happened — auto_restore=true is needed for a wrong downgrade to show.
    let mut profile = exit_profile(0);
    profile.id = id;
    profile.epoch = 1; // the state where the incarnation marker changed by reactivation (mismatching reaped_epoch=0).
    profile.auto_restore = true;
    profiles.try_upsert(profile).expect("save succeeds");

    let stale = ReapMsg {
        id,
        epoch: 0,
        reason: TerminalReason::Exited { code: Some(0) },
        intent_at_finish: TerminationIntent::None,
        shutting_down_at_finish: false,
    };
    deps.reap_one(stale);

    assert!(
        profiles.get(id).map(|p| p.auto_restore).unwrap_or(false),
        "ADR-0084: an epoch-mismatched (reactivated) stale reap must not downgrade the live session's auto_restore"
    );
}

// ── ADR-0084 control group (epoch match) ───────────────────────────────────────────────
#[test]
fn matching_epoch_disposition_downgrades_as_before() {
    let (profiles, _sink, deps) = make_reaper_deps("disp-epoch-match");
    let id = Uuid::new_v4();

    let status_dyn: Arc<dyn StatusSink> = Arc::new(_sink.clone());
    let dead = make_test_session(id, 0, status_dyn);
    deps.sessions.write().unwrap().insert(id, dead);

    let mut profile = exit_profile(0);
    profile.id = id;
    profile.epoch = 0; // no reactivation → matches reaped_epoch=0.
    profile.auto_restore = true;
    profiles.try_upsert(profile).expect("save succeeds");

    let done = ReapMsg {
        id,
        epoch: 0,
        reason: TerminalReason::Exited { code: Some(0) },
        intent_at_finish: TerminationIntent::None,
        shutting_down_at_finish: false,
    };
    deps.reap_one(done);

    assert!(
        profiles.get(id).map(|p| !p.auto_restore).unwrap_or(false),
        "ADR-0084: on an epoch match a normal exit must downgrade auto_restore to false (the guard does not block the normal path)"
    );
}
