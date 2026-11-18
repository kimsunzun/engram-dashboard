//! Integration tests for manual activation (activate_profile) — ADR-0082 (fresh-fallback removed, resume only). The real claude cannot
//!   launch in CI, so a cmd.exe batch profile simulates it; Windows only (`#[cfg(windows)]`). Single spawn, no global races.

#![cfg(windows)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use uuid::Uuid;

use engram_dashboard_agent::backend;
use engram_dashboard_agent::commands::{
    make_table, AgentBackend, AgentCommandHost, RosterChanged, UsageCommandHost, UsageVendorRow,
};
use engram_dashboard_agent::failure::AgentFailureKind;
use engram_dashboard_agent::launch::{
    notice_fingerprint, notice_row, BlockReason, LaunchArtifacts, LaunchBlocked, LaunchKit,
    LaunchKitSource, LaunchPlan, LaunchPorts, NoopLaunchKits, Notice, PrepareError,
};
use engram_dashboard_agent::manager::{Activated, AgentManager, NoticeDelivery};
use engram_dashboard_agent::persistence::FileProfileStore;
use engram_dashboard_agent::profile::{
    AgentCommand, AgentOutputFormat, AgentProfile, ProfileRegistry, ProfileStore, RestoreOutcome,
    SpawnMode,
};
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, ControlEndpoint, InputOrigin,
    NoopControlChannel, ProvisionError, PtyError, StatusSink,
};
use engram_dashboard_base::testing::wait_until;
use engram_dashboard_command::testing::block_on;
use engram_dashboard_command::ErrorCode;
use serde_json::json;

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
    make_manager_with_control(tag, Arc::new(NoopControlChannel))
}

fn make_manager_with_control(
    tag: &str,
    control: Arc<dyn ControlChannel>,
) -> (AgentManager, CountingSink, Arc<ProfileRegistry>) {
    let (manager, sink, profiles, _store) = make_manager_with_store(tag, control);
    (manager, sink, profiles)
}

fn make_manager_with_store(
    tag: &str,
    control: Arc<dyn ControlChannel>,
) -> (
    AgentManager,
    CountingSink,
    Arc<ProfileRegistry>,
    Arc<FileProfileStore>,
) {
    let sink = CountingSink::new();
    let sink_dyn: Arc<dyn StatusSink> = Arc::new(sink.clone());
    let store = Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-test-activate-{tag}-{}", Uuid::new_v4())),
    ));
    let profiles = Arc::new(ProfileRegistry::new(store.clone()));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = AgentManager::new_with_control(sink_dyn, profiles.clone(), tracker, control);
    (manager, sink, profiles, store)
}

/// Returns: (profile, batch path, count path).
///
/// ★The run-count is the ADR-0082 evidence★: each run appends one line to count and exits early (exit 1); a live old fresh-fallback would
///   spawn a second time and make count 2 — "exactly 1" proves its absence.
/// ★A batch file, not an inline compound cmd★: portable-pty `CommandBuilder` quotes `>`·`&` individually and breaks them through ConPTY
///   (measured); cmd parses a batch directly, deterministically.
fn always_early_exit_profile(tag: &str) -> (AgentProfile, PathBuf, PathBuf) {
    let uniq = Uuid::new_v4();
    let count = std::env::temp_dir().join(format!("engram-activate-count-{tag}-{uniq}.tmp"));
    let batch = std::env::temp_dir().join(format!("engram-activate-exit-{tag}-{uniq}.cmd"));

    let script = format!(
        "@echo off\r\n\
         echo x>>\"{c}\"\r\n\
         exit /b 1\r\n",
        c = count.display()
    );
    std::fs::write(&batch, script).expect("write the batch file");

    // Last argument auto_restore=true — if it is born false, the downgrade is a no-op and cannot be observed.
    let profile = AgentProfile::new(
        "activate-exit".into(),
        AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), batch.to_string_lossy().to_string()],
        },
        PathBuf::from("."),
        vec![],
        true,
    );
    (profile, batch, count)
}

/// 0 if the file does not exist (not run even once yet).
fn run_count(path: &PathBuf) -> usize {
    std::fs::read_to_string(path)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

/// A batch that outlives EARLY_EXIT_WINDOW (3 s) — 20 pings keep it alive ≈19 s so it is not misjudged as an early exit. Returns:
///   (profile, batch path, count path).
///
/// ★count comes **before** ping★ so it is 1 right after start and readable at once; a kill+replace (same epoch) regression would make it 2.
fn long_lived_profile(tag: &str) -> (AgentProfile, PathBuf, PathBuf) {
    let uniq = Uuid::new_v4();
    let count = std::env::temp_dir().join(format!("engram-activate-live-count-{tag}-{uniq}.tmp"));
    let batch = std::env::temp_dir().join(format!("engram-activate-live-{tag}-{uniq}.cmd"));
    let script = format!(
        "@echo off\r\n\
         echo x>>\"{c}\"\r\n\
         ping -n 20 127.0.0.1 >nul\r\n",
        c = count.display()
    );
    std::fs::write(&batch, script).expect("write the batch");
    let profile = AgentProfile::new(
        "activate-live".into(),
        AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), batch.to_string_lossy().to_string()],
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    (profile, batch, count)
}

/// ★ADR-0082 regression guard ①★ — an early resume exit is not replaced by an automatic fresh.
#[test]
fn activate_resume_early_exit_ends_failed_no_fresh_fallback() {
    let (manager, _sink, profiles) = make_manager("resume-no-fallback");

    let (profile, batch, count) = always_early_exit_profile("resume-no-fallback");
    let id = profile.id;
    profiles
        .try_upsert(profile.clone())
        .expect("store succeeds");

    let sid_before = profiles.get(id).and_then(|p| p.backend_session_id);
    let old_sids_before = profiles
        .get(id)
        .map(|p| p.old_session_ids.len())
        .unwrap_or(0);

    let result = manager.activate_profile(&profile, SpawnMode::Resume);

    assert!(
        result.is_err(),
        "an early resume exit must be Err (Failed corpse) without fresh-fallback — got Ok: {result:?}"
    );

    let _ = wait_until(Duration::from_secs(5), || {
        !manager.list_agents().iter().any(|a| a.id == id)
    });

    assert_eq!(
        run_count(&count),
        1,
        "the resume place must run only once (there must be no fresh-fallback) — got {}",
        run_count(&count)
    );

    assert_eq!(
        profiles.get(id).and_then(|p| p.backend_session_id),
        sid_before,
        "a resume failure must not issue a new sid (fresh-fallback removed)"
    );
    assert_eq!(
        profiles.get(id).map(|p| p.old_session_ids.len()),
        Some(old_sids_before),
        "a resume failure must not push the old sid into history (Fresh's clearing not called)"
    );

    assert!(
        profiles.get(id).is_some(),
        "the profile (corpse) must not be deleted after a resume failure — it must be kept via KeepDisableAutoRestore"
    );

    assert_eq!(
        profiles.get(id).map(|p| p.auto_restore),
        Some(false),
        "a resume failure corpse must be downgraded to auto_restore=false (not deleted)"
    );

    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// ★ADR-0082 regression guard ②★ — reactivating a live agent does not destroy that agent (a4aac1a).
#[test]
fn reactivate_running_agent_leaves_it_alive_epoch_unchanged() {
    let (manager, _sink, profiles) = make_manager("reactivate-live");

    let (profile, batch, count) = long_lived_profile("reactivate-live");
    let id = profile.id;
    profiles
        .try_upsert(profile.clone())
        .expect("store succeeds");

    let first = manager
        .activate_profile(&profile, SpawnMode::Fresh)
        .expect("the first activation must be Ok (a live session)");
    let epoch_after_first = first.epoch;

    assert!(
        wait_until(Duration::from_secs(3), || {
            manager.list_agents().iter().any(|a| {
                a.id == id
                    && !matches!(
                        a.status,
                        AgentStatus::Failed { .. }
                            | AgentStatus::Killed
                            | AgentStatus::Exited { .. }
                    )
            })
        }),
        "the first activation's session must be alive"
    );

    // The batch's append is a PTY-spawned process and can be delayed, so wait until it reaches 1.
    assert!(
        wait_until(Duration::from_secs(3), || run_count(&count) == 1),
        "the first activation must start the batch once — got {}",
        run_count(&count)
    );

    // Looking only at the returned AgentInfo.epoch cannot catch a registry map replacement, so the registry value is copied aside too.
    let reg_epoch_before = profiles.get(id).map(|p| p.epoch);

    let reactivated = manager
        .activate_profile(&profile, SpawnMode::Resume)
        .expect("reactivation must be a harmless Ok (AgentInfo of the one already running) — dying means Err/regression");

    // The batch run (append) is asynchronous, so 'absence' can only be checked by a window — give it 2s and see that count does not go up.
    //   The decisive axis is the epoch invariance below; this count is an auxiliary heuristic.
    let respawned = wait_until(Duration::from_secs(2), || run_count(&count) >= 2);
    assert!(
        !respawned,
        "reactivation must not cause a respawn — count went up to 2 (regression signal)"
    );
    assert_eq!(
        run_count(&count),
        1,
        "after reactivation the batch must have run exactly once (no respawn)"
    );

    assert_eq!(
        reactivated.id, id,
        "reactivation must point at the same agent"
    );
    assert_eq!(
        reactivated.epoch, epoch_after_first,
        "reactivating a live session does not replace the incarnation marker (map replacement = no fresh, a4aac1a regression signal)"
    );
    assert!(
        !matches!(
            reactivated.status,
            AgentStatus::Failed { .. } | AgentStatus::Killed | AgentStatus::Exited { .. }
        ),
        "if the live agent is in a terminal state after reactivation it was destroyed — it must be alive: {:?}",
        reactivated.status
    );

    assert_eq!(
        profiles.get(id).map(|p| p.epoch),
        reg_epoch_before,
        "reactivating a live session does not replace the registry marker either (map replacement = no fresh)"
    );

    // 300ms is a window to catch a regression where the asynchronous reaper reaps the old session late.
    std::thread::sleep(Duration::from_millis(300));
    let live = manager.list_agents();
    let entry = live.iter().find(|a| a.id == id);
    assert!(
        entry.is_some_and(|a| a.epoch == epoch_after_first
            && !matches!(
                a.status,
                AgentStatus::Failed { .. } | AgentStatus::Killed | AgentStatus::Exited { .. }
            )),
        "after reactivation the original session must not be killed nor its epoch changed — got {entry:?}"
    );

    assert_eq!(
        run_count(&count),
        1,
        "reactivation must not respawn a live agent (the batch start must still be once) — got {}",
        run_count(&count)
    );

    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// ★ADR-0113 wiring guard★ — spawn **registers** its incarnation in the turn observation table.
///
/// Registering before the incarnation's first output signal makes the table's "drop if the marker differs" rule safe
///   (`turn::TurnObservations::register`); missing, the previous entry would drop all of the new incarnation's signals and it would answer
///   unobserved (= not in a turn),
/// so mail goes in during a turn.
/// ★A shell gives the discriminating power★: the shell backend emits no turn signal (silent classifier), so any entry here came from the
///   registration.
/// ★Not measured (honest)★: that registration comes **before the first signal** — reversing it needs the previous incarnation's `finish`
///   blocked, and this harness has no handle; the ordering is structural (before the sessions map insert).
#[test]
fn spawn_registers_the_incarnation_in_the_turn_table() {
    let (manager, _sink, profiles) = make_manager("turn-register");

    let (profile, batch, count) = long_lived_profile("turn-register");
    let id = profile.id;
    profiles
        .try_upsert(profile.clone())
        .expect("store succeeds");

    let info = manager
        .activate_profile(&profile, SpawnMode::Fresh)
        .expect("activation is Ok");
    assert!(
        manager.turns().get(id, info.epoch).is_some(),
        "spawn must take this incarnation's place in the table"
    );
    assert!(
        !manager.turns().is_in_turn(id, info.epoch),
        "registration alone is not in a turn (signals come only from live emit)"
    );

    manager.kill_agent(id).expect("kill_agent failed");
    assert!(
        wait_until(Duration::from_secs(5), || {
            manager.turns().get(id, info.epoch).is_none()
        }),
        "termination must withdraw its own entry (if not withdrawn, parking addressed to the dead agent is not released)"
    );

    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// ★ADR-0084 regression guard★ — reactivating a corpse in the same slot changes the incarnation marker.
///
/// A shell has both session axes false, so `--resume` assembly is not reached (that is
///   `backend::claude::tests::build_command_spec_resume_emits_resume_flag_with_sid`); "reactivation = map replacement = new marker" holds
///   for any backend.
///   ★Measures only **difference**, not growth★: the marker is random per incarnation (`ProfileRegistry::epoch_for_spawn`); `assert!(new >
///   old)` would be red half the time.
#[test]
fn reactivate_after_kill_bumps_epoch() {
    let (manager, _sink, profiles) = make_manager("reactivate-epoch-bump");

    let (profile, batch, count) = long_lived_profile("reactivate-epoch-bump");
    let id = profile.id;
    profiles
        .try_upsert(profile.clone())
        .expect("store succeeds");

    let first = manager
        .activate_profile(&profile, SpawnMode::Fresh)
        .expect("the first activation must be Ok (a live session)");
    let epoch_e = first.epoch;
    assert!(
        wait_until(Duration::from_secs(3), || {
            manager.list_agents().iter().any(|a| a.id == id)
        }),
        "the first activation's session must be alive"
    );

    manager.kill_agent(id).expect("kill_agent failed");
    assert!(
        wait_until(Duration::from_secs(5), || {
            !manager.list_agents().iter().any(|a| a.id == id)
        }),
        "after a user kill the session must be reaped from the map"
    );
    assert_eq!(
        profiles.get(id).map(|p| p.epoch),
        Some(epoch_e),
        "kill alone does not replace the marker (the reactivation respawn is the issuer)"
    );

    let reactivated = manager
        .activate_profile(&profile, SpawnMode::Resume)
        .expect("reactivation must be Ok (a live session) via the resume path (the shell does not exit early)");

    assert_ne!(
        reactivated.epoch, epoch_e,
        "ADR-0084: the reactivated session's incarnation marker must differ from the dead session's (map replacement = new incarnation)"
    );
    assert_eq!(
        profiles.get(id).map(|p| p.epoch),
        Some(reactivated.epoch),
        "after reactivation the registry marker and the session marker must match (the issuing is reflected before spawn_agent reads it)"
    );

    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// ★ADR-0083 regression guard★ — a user kill that deleted the profile would break reactivation with "profile not found" (the old reaper
///   did exactly that via UserKill → DeleteProfile, taking the backend_session_id).
///
/// Proves only that **the profile lookup path is intact**; the `--resume <sid>` assembly is
///   `backend::claude::tests::build_command_spec_resume_emits_resume_flag_with_sid`.
#[test]
fn user_kill_then_reactivate_finds_profile_and_resumes() {
    let (manager, _sink, profiles) = make_manager("kill-reactivate");

    let (profile, batch, count) = long_lived_profile("kill-reactivate");
    let id = profile.id;

    // ★Pass the seeded profile to spawn/activate/kill alike★ — registry and argument being one value shows this item measures the **resume
    //   wiring**, not handle preservation (that net: `spawn_preserving_upsert_does_not_revert_a_handle_recorded_after_the_snapshot` in
    //   `profile.rs`).
    //   ★That preservation changes claude's behavior too — not 「claude stays as it was」★: reactivation's sid is read by spawn **from the
    //   registry** (not `ensure_position`), and registration no longer overwrites it with the snapshot, so **the drift sid observed by
    //   `SessionTracker`**
    //   (written unconditionally via `observe_session_id(id, None, ..)`, attached only behind the `assigns_sid` gate = claude) is what
    //   resumes — an intended improvement, **not a no-change**. Here registry = argument, so that difference is not measured.
    //   auto_restore=true observes the kill reaping's downgrade.
    let sid = Uuid::new_v4();
    let mut seeded = profile.clone();
    seeded.backend_session_id = Some(sid);
    seeded.auto_restore = true;
    profiles.try_upsert(seeded.clone()).expect("store succeeds");

    manager
        .activate_profile(&seeded, SpawnMode::Fresh)
        .expect("the first activation must be Ok (a live session)");
    assert!(
        wait_until(Duration::from_secs(3), || {
            manager.list_agents().iter().any(|a| a.id == id)
        }),
        "the first activation's session must be alive"
    );

    manager.kill_agent(id).expect("kill_agent failed");
    assert!(
        wait_until(Duration::from_secs(5), || {
            !manager.list_agents().iter().any(|a| a.id == id)
        }),
        "after a user kill the session must be reaped from the map"
    );

    assert!(
        wait_until(Duration::from_secs(2), || {
            profiles.get(id).map(|p| !p.auto_restore).unwrap_or(false)
        }),
        "a user kill corpse must keep the profile + auto_restore=false (ADR-0083 — not deleted)"
    );
    assert!(
        profiles.get(id).is_some(),
        "the profile was deleted after a user kill — it must be kept as a corpse (ADR-0083 regression)"
    );
    assert_eq!(
        profiles.get(id).and_then(|p| p.backend_session_id),
        Some(sid),
        "backend_session_id was lost by a user kill — reactivation cannot resume (ADR-0083 regression)"
    );

    let reactivated = manager.activate_profile(&seeded, SpawnMode::Resume).expect(
        "reactivation must enter the resume path without profile not found and be Ok (ADR-0083)",
    );
    assert_eq!(
        reactivated.id, id,
        "reactivation must point at the same agent (the preserved corpse profile)"
    );
    assert!(
        !matches!(
            reactivated.status,
            AgentStatus::Failed { .. } | AgentStatus::Killed | AgentStatus::Exited { .. }
        ),
        "if the reactivated session is in a terminal state, entering resume failed — it must be alive: {:?}",
        reactivated.status
    );

    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

// ── Last failure record (ADR-0172) ────────────────────────────────────────────────────

/// If a resume fails with an early exit, the kind is attached right there — by observation alone, without a prior decision.
///
/// ★A shell profile yields 「early exit right after resume」★: the phrase-recognizing knowledge is only in the claude backend (separate unit
///   test) and the shell is silent, so it falls to the context default — retryable, the concrete form of fail-open.
#[test]
fn resume_early_exit_records_a_typed_last_failure() {
    let (manager, _sink, profiles) = make_manager("record-early-exit");

    let (profile, batch, count) = always_early_exit_profile("record-early-exit");
    let id = profile.id;
    profiles
        .try_upsert(profile.clone())
        .expect("store succeeds");
    assert_eq!(
        profiles.get(id).and_then(|p| p.last_failure),
        None,
        "premise: there is no record before the attempt (no prior decision is made)"
    );

    let result = manager.activate_profile(&profile, SpawnMode::Resume);
    assert!(result.is_err(), "premise: an early exit ends in Err");

    let recorded = profiles.get(id).and_then(|p| p.last_failure);
    assert_eq!(
        recorded,
        Some(AgentFailureKind::EarlyExitAfterResume),
        "the kind must be attached at the place of failure — got {recorded:?}"
    );

    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// An activation that could not launch the process at all is recorded too (the Fresh branch — covering only resume would leak this side).
#[test]
fn an_unspawnable_profile_records_a_spawn_failure() {
    let (manager, _sink, profiles) = make_manager("record-spawn-fail");

    let profile = AgentProfile::new(
        "activate-nonexistent".into(),
        AgentCommand::Shell {
            // A nonexistent executable — PTY open fails and spawn ends in Err.
            program: format!("engram-not-a-real-program-{}.exe", Uuid::new_v4()),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    let id = profile.id;
    profiles
        .try_upsert(profile.clone())
        .expect("store succeeds");

    let result = manager.activate_profile(&profile, SpawnMode::Fresh);
    assert!(result.is_err(), "premise: a missing executable ends in Err");
    assert_eq!(
        profiles.get(id).and_then(|p| p.last_failure),
        Some(AgentFailureKind::SpawnFailed)
    );
}

/// ★Restoring an agent that is already up does not stamp a failure★ — and this path is **deterministic**.
///
/// `restore_one` skips `activate_profile`'s "already running" pre-filter, so a restore with a live agent runs straight into
///   `spawn_agent`'s double-spawn guard (`SpawnOutcome::Moot`) and enters the activation record path with no thread interleaving. The old
///   form raced two threads
/// expecting a winner and a loser; a deschedule behind the pre-filter let both pass (intermittent red) — do not revive it.
///
/// ★"The second process did not launch" is **not measured with a time window**★: file-append counting cannot tell absence from "not
///   written yet", so any wait can pass falsely under contention. Instead the **epoch**: a second spawn always raises it via
///   `epoch_for_spawn`, so an
/// unchanged value is immediate, definitive evidence.
#[test]
fn restoring_over_a_live_agent_never_stamps_it_as_failed() {
    let (manager, _sink, profiles) = make_manager("restore-over-live");

    let (profile, batch, count) = long_lived_profile("restore-over-live");
    let id = profile.id;
    profiles
        .try_upsert(profile.clone())
        .expect("store succeeds");
    manager
        .activate_profile(&profile, SpawnMode::Fresh)
        .expect("the first activation succeeds");
    assert!(
        wait_until(Duration::from_secs(5), || {
            manager.list_agents().iter().any(|a| a.id == id)
        }),
        "premise: the session is in the registry and alive"
    );
    assert_eq!(
        profiles.get(id).and_then(|p| p.last_failure),
        None,
        "premise: a successful activation leaves no record"
    );
    let epoch_before = profiles.get(id).map(|p| p.epoch);

    // spawn raises auto_restore to true, so this profile is a restore target.
    let reports = manager.restore_all();
    let handled = reports.iter().any(|r| r.agent_id == id);
    assert!(
        handled,
        "premise: the restore must scan this profile — got {reports:?}"
    );

    assert_eq!(
        profiles.get(id).and_then(|p| p.last_failure),
        None,
        "a duplicate request is not that item's failure — if the live agent gets stamped, it shows on the screen the moment it exits normally"
    );
    assert_eq!(
        profiles.get(id).map(|p| p.epoch),
        epoch_before,
        "epoch went up = a second incarnation launched (a request with nothing to do must not launch a process)"
    );
    assert_eq!(
        manager.list_agents().iter().filter(|a| a.id == id).count(),
        1,
        "there must be only one incarnation in the registry too"
    );

    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// ★What the user cut off is not an activation failure★ — a kill inside the early-exit window leaves no record.
///
/// A record 「exited right after resuming」 would make an item the user just turned off look broken (not cleared until the next activation).
///   On HEAD this path produced only a transient Err.
/// ★No false red★: the single assertion 「there is no record」 holds in **both** interleavings — kill inside the window → this branch
///   (records nothing); past it → the stand branch (clears). Which one ran is told by the activation return value, only reported below.
#[test]
fn a_kill_inside_the_resume_window_is_not_recorded_as_a_failure() {
    let (manager, _sink, profiles) = make_manager("kill-in-window");

    let (profile, batch, count) = long_lived_profile("kill-in-window");
    let id = profile.id;
    profiles
        .try_upsert(profile.clone())
        .expect("store succeeds");

    let manager = Arc::new(manager);
    let activator = {
        let m = manager.clone();
        let p = profile.clone();
        // In Resume mode it blocks polling for the early-exit window (3s) — meanwhile the main thread cuts it off.
        std::thread::spawn(move || m.activate_profile(&p, SpawnMode::Resume))
    };

    assert!(
        wait_until(Duration::from_secs(5), || {
            manager.list_agents().iter().any(|a| a.id == id)
        }),
        "premise: the session must be in the registry to be cut off"
    );
    let _ = manager.kill_agent(id);

    let outcome = activator.join().expect("activation thread");
    assert_eq!(
        profiles.get(id).and_then(|p| p.last_failure),
        None,
        "a user termination is not an activation failure — no stamp may remain (outcome: {})",
        if outcome.is_err() {
            "observed inside the window"
        } else {
            "decided as standing past the window"
        }
    );

    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

// ── Session id issuing axis (ADR-0185) ─────────────────────────────────────────────────

/// A test-bench device that makes provision fail, cutting spawn off **before the link is created** — after the sid issuing, so the spawn
///   passes the issuing point without launching a process.
struct FailingControl;

impl ControlChannel for FailingControl {
    fn provision(
        &self,
        _id: AgentId,
        _epoch: u32,
        _needs: engram_dashboard_agent::types::ControlChannelNeeds,
    ) -> Result<Option<ControlEndpoint>, ProvisionError> {
        Err(ProvisionError(
            "test bench: cut spawn off before the link is created".into(),
        ))
    }

    fn revoke(&self, _id: AgentId, _epoch: u32) {}
}

/// ★What this item **really** measures — two things★:
///   ① A Fresh spawn **does not persist** the session id for any backend (ADR-0226) — neither in the registry nor on disk (all the codex
///   row measures: registered and empty).
///   ② The old handle of a backend we issue for (claude) is not deleted but goes **into history**.
///
/// ★「ADR-0185」 cannot be measured here★ — no backend persists at spawn, so a decision split along the resume axis would leave the codex
///   row green. Those guards: `manager`'s `only_a_backend_that_assigns_ids_gets_one_handed_over` (the decision) and
///   `the_first_submission_latch_is_wired_through_the_spawn_path` (spawn uses it).
/// ★No control group for 「we drew it and handed it over」★ — the drawn value is not persisted before the first submission; counterparts:
///   `fresh_claude_spec_carries_the_minted_session_id` (in argv) and `d1_a_fresh_claude_persists_its_minted_id_only_after_the_first_submission`.
/// ★② catches this item idling★ — if spawn dies **before** the clearing, ① stays green but the old value does not enter history.
///
/// ★spawn is driven to fail **without launching a process** yet still passes the clearing point★ (clearing comes **before** link creation).
/// ★The rows stop at different places★: a backend that **uses** the control channel is cut at provision fail-closed by [`FailingControl`];
///   one that does **not** never goes through provision, so a nonexistent cwd makes link creation fail.
/// ★One cwd cannot block both rows (measured 2026-09-14 — do not revive)★: with a nonexistent cwd the ConPTY link launched anyway and the
///   real `cmd.exe /c claude` started.
// ADR-0226
#[test]
fn a_fresh_spawn_persists_no_session_id_and_moves_the_old_handle_to_history() {
    let (manager, _sink, profiles, store) =
        make_manager_with_store("mint-axis", Arc::new(FailingControl));
    let nowhere = std::env::temp_dir().join(format!("engram-no-such-dir-{}", Uuid::new_v4()));

    let codex = AgentProfile::new(
        "codex-app-server".into(),
        AgentCommand::Codex {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        },
        nowhere.clone(),
        vec![],
        false,
    );
    let mut claude = AgentProfile::new(
        "claude-terminal".into(),
        AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        },
        nowhere,
        vec![],
        false,
    );
    let claude_old = Uuid::new_v4();
    claude.backend_session_id = Some(claude_old);

    for p in [&codex, &claude] {
        if let Ok(outcome) = manager.spawn_agent(p, SpawnMode::Fresh) {
            if let Some(info) = outcome.into_started() {
                let _ = manager.kill_agent(info.id);
                panic!(
                    "{:?}: the link actually launched — the isolation premise (spawn fails with a nonexistent cwd) is broken",
                    p.command
                );
            }
        }
    }

    // ★First check that it actually got into the registry★ — `None` below cannot tell 「not persisted」 from 「no profile at all」; without
    //   this line it is green even when never registered.
    assert!(
        profiles.get(codex.id).is_some() && profiles.get(claude.id).is_some(),
        "the profile is not in the registry — the assertion below would measure absence rather than persistence"
    );
    assert_eq!(
        profiles.get(codex.id).and_then(|p| p.backend_session_id),
        None,
        "codex's Fresh spawn wrote a session id into the registry — persisting is the first submission's job (ADR-0226). This test bench \
         cuts spawn off before the link is created, so the receiving path cannot write it down either — if so, the isolation premise broke first"
    );

    let got = profiles.get(claude.id).expect("claude profile");
    assert_eq!(
        got.backend_session_id, None,
        "a Fresh spawn wrote a session id into the registry — persisting is the first submission's job (ADR-0226)"
    );
    assert!(
        got.old_session_ids.contains(&claude_old),
        "the old handle did not go into history — spawn died before the clearing point and the assertion above idles"
    );
    let on_disk = store
        .load()
        .into_iter()
        .find(|p| p.id == claude.id)
        .expect("the claude profile is on disk");
    assert_eq!(
        on_disk.backend_session_id, None,
        "a session id remained on disk — if a 0-turn session's id is persisted, the next activation fails to resume with that id"
    );
}

/// ★Measures reaching the issuing point without persistence★ — the drawn value is not in the registry before the first submission, so 「we
///   drew it and handed it over」 shows only in argv. A pure function; no process.
// ADR-0226
#[test]
fn fresh_claude_spec_carries_the_minted_session_id() {
    let minted = ProfileRegistry::mint_session_id();
    for output_format in [AgentOutputFormat::Terminal, AgentOutputFormat::StreamJson] {
        let command = AgentCommand::Claude {
            extra_args: vec![],
            output_format,
        };
        let spec = backend::build_command_spec(
            &command,
            SpawnMode::Fresh,
            Some(minted),
            None,
            std::env::temp_dir(),
            vec![],
            None,
            &LaunchArtifacts::default(),
        );
        let at = spec
            .args
            .iter()
            .position(|a| a == "--session-id")
            .unwrap_or_else(|| panic!("{command:?}: no `--session-id`: {:?}", spec.args));
        assert_eq!(
            spec.args.get(at + 1),
            Some(&minted.to_string()),
            "{command:?}: the drawn value was not put in: {:?}",
            spec.args
        );
        assert!(
            !spec.args.iter().any(|a| a == "--resume"),
            "{command:?}: Fresh, yet a resume flag was put in: {:?}",
            spec.args
        );
    }
}

/// ★The handle is read once inside spawn and that read is authoritative (ADR-0226)★ — empty, a backend that resumes with the stored handle
///   refuses **before launching the process**.
///
/// Calls `spawn_agent(Resume)` directly (it skips the entry's head decision) to reproduce 「the field became empty between the decision and
///   the read」 deterministically.
/// ★Told apart by the refusal phrase★ — this bench also cuts at provision ([`FailingControl`]), so without the refusal the same `Err`
///   changes phrase to a provision failure; in operation claude would launch without a resume flag and open a new conversation with an
///   unknown id.
/// ★shell is the old way★ — it does not resume with the stored handle and launches without one (the ADR-0082 test bench relies on that).
// ADR-0226
#[test]
fn a_resume_whose_handle_is_gone_is_refused_before_any_process() {
    let (manager, _sink, profiles) =
        make_manager_with_control("handle-gone", Arc::new(FailingControl));

    let commands = [
        AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        },
        AgentCommand::Codex {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        },
        AgentCommand::Codex {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        },
    ];
    for command in commands {
        let p = AgentProfile::new(
            "handle-gone".into(),
            command,
            std::env::temp_dir(),
            vec![],
            false,
        );
        profiles.try_upsert(p.clone()).expect("store succeeds");
        let refusal = match manager.spawn_agent(&p, SpawnMode::Resume) {
            Ok(outcome) => {
                if let Some(info) = outcome.into_started() {
                    let _ = manager.kill_agent(info.id);
                }
                panic!("{:?}: a Resume without a handle launched", p.command);
            }
            Err(e) => e.to_string(),
        };
        assert!(
            refusal.contains("the resume handle vanished during spawn"),
            "{:?}: not a refusal before launching — {refusal}",
            p.command
        );
        assert_eq!(
            profiles.get(p.id).and_then(|q| q.backend_session_id),
            None,
            "{:?}: the refusal produced a handle",
            p.command
        );
    }

    let (shell, batch, count) = always_early_exit_profile("handle-gone-shell");
    profiles.try_upsert(shell.clone()).expect("store succeeds");
    let outcome = manager
        .spawn_agent(&shell, SpawnMode::Resume)
        .expect("shell launches the old way even without a handle");
    assert!(
        outcome.into_started().is_some(),
        "shell's Resume did not make a new incarnation"
    );
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

// ── D1 runtime integration — launch a fake claude via PATH (ADR-0226) ─────────────────────────

/// Makes a folder holding a fake `claude.cmd`. Returns: (that folder, argument record file).
///
/// Putting it in front of `PATH` in the profile env makes the backend's `cmd.exe /c claude …` find it (the backend passes only the name,
///   resolved through the child's `PATH`; real claude is not launched).
/// ★Records each received argument line in the record file★ — the fake's stdout goes to the PTY and the test cannot read it.
/// ★With `--resume` it prints the real claude's refusal phrase and exits 1, otherwise lives reading input★ (phrase · exit code: the phrase
///   constant's doc in `backend::claude`).
/// ★The agent cwd must **differ** from this folder★ — cmd looks in the current folder before `PATH`, which would hide whether `PATH` took
///   effect.
fn fake_claude_dir(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("engram-fake-claude-{tag}-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("fake folder");
    let script = "@echo off\r\n\
         >>\"%~dp0args.log\" echo %*\r\n\
         echo %* | findstr /C:\"--resume\" >nul\r\n\
         if not errorlevel 1 (\r\n\
         echo No conversation found with session ID: fake\r\n\
         exit /b 1\r\n\
         )\r\n\
         :loop\r\n\
         set /p line=\r\n\
         if errorlevel 1 ping -n 2 127.0.0.1 >nul\r\n\
         goto loop\r\n";
    std::fs::write(dir.join("claude.cmd"), script).expect("write the fake claude.cmd");
    let args_log = dir.join("args.log");
    (dir, args_log)
}

/// A claude terminal profile with the fake claude in front of `PATH` — the cwd is a new folder different from the fake folder.
fn claude_on_fake_path(name: &str, fake_dir: &std::path::Path) -> (AgentProfile, PathBuf) {
    let cwd = std::env::temp_dir().join(format!("engram-fake-claude-cwd-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&cwd).expect("agent cwd");
    let path = format!(
        "{};{}",
        fake_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let profile = AgentProfile::new(
        name.into(),
        AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        },
        cwd.clone(),
        vec![("PATH".into(), path)],
        false,
    );
    (profile, cwd)
}

/// The argument lines the fake received (1 spawn = 1 line).
fn fake_spawns(args_log: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(args_log)
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The uuid right after `flag` in an argument line.
fn uuid_after(line: &str, flag: &str) -> Option<Uuid> {
    let mut words = line.split_whitespace();
    words.find(|w| *w == flag)?;
    words.next().and_then(|w| Uuid::parse_str(w).ok())
}

fn stored_on_disk(store: &FileProfileStore, id: AgentId) -> Option<Uuid> {
    store
        .load()
        .into_iter()
        .find(|p| p.id == id)
        .and_then(|p| p.backend_session_id)
}

/// ★Measures D1 with a real process — a 0-turn session leaves no id, and the first submission persists it (ADR-0226)★.
///
/// Two spawns in turn:
///   1. Resume with an old id without a conversation (garbage) — the fake gets `--resume` and dies with the refusal phrase; it now ends in
///   failure without moving to a new conversation (ADR-0082), the garbage stays in the registry.
///   2. Fresh on the same profile — the garbage goes into history, the fake launches with `--session-id <drawn value>` and lives:
///   `Ok`·Running, failure record 1 cleared. The drawn value is in neither registry nor disk **before the first submission (CR)** and
///   settles in both only after the
///      first input containing CR.
/// ★First check via the argument record that the fake launched★ — else the 「not persisted」 assertion idles.
// ADR-0226
#[test]
fn d1_a_fresh_claude_persists_its_minted_id_only_after_the_first_submission() {
    let (manager, _sink, profiles, store) =
        make_manager_with_store("d1-first-submission", Arc::new(NoopControlChannel));
    let (fake_dir, args_log) = fake_claude_dir("d1");
    let (mut profile, cwd) = claude_on_fake_path("d1-claude", &fake_dir);
    let garbage = Uuid::new_v4();
    profile.backend_session_id = Some(garbage);
    let id = profile.id;
    profiles
        .try_upsert(profile.clone())
        .expect("store succeeds");

    let resumed = manager.activate_profile(&profile, SpawnMode::Resume);
    assert!(
        resumed.is_err(),
        "resuming an id without a conversation was reported as success: {resumed:?}"
    );
    assert!(
        wait_until(Duration::from_secs(10), || manager.list_agents().is_empty()),
        "the dead resume incarnation was not reaped"
    );
    let spawns = fake_spawns(&args_log);
    assert_eq!(
        spawns.len(),
        1,
        "the fake claude must have launched exactly once (0 if PATH did not take effect): {spawns:?}"
    );
    assert_eq!(
        uuid_after(&spawns[0], "--resume"),
        Some(garbage),
        "it did not resume with the stored id: {spawns:?}"
    );
    assert_eq!(
        profiles.get(id).and_then(|p| p.backend_session_id),
        Some(garbage),
        "Phase A's resume failure does not touch the handle"
    );

    let info = manager
        .activate_profile(&profile, SpawnMode::Fresh)
        .expect("Fresh activation");
    assert!(
        matches!(info.status, AgentStatus::Running),
        "the Fresh outcome is not Running: {:?}",
        info.status
    );
    // A Fresh on a link without a connection axis returns the moment the child launches, so wait until the fake leaves its arguments.
    wait_until(Duration::from_secs(10), || {
        fake_spawns(&args_log).len() >= 2
    });
    let spawns = fake_spawns(&args_log);
    assert_eq!(
        spawns.len(),
        2,
        "Fresh must have launched the fake once more: {spawns:?}"
    );
    let minted = uuid_after(&spawns[1], "--session-id")
        .unwrap_or_else(|| panic!("Fresh did not launch with `--session-id <uuid>`: {spawns:?}"));
    assert_ne!(minted, garbage, "Fresh reused the old id (ADR-0076)");

    let got = profiles.get(id).expect("profile");
    assert_eq!(
        got.backend_session_id, None,
        "the drawn id was written into the registry before submission"
    );
    assert!(
        got.old_session_ids.contains(&garbage),
        "the garbage id did not go into history"
    );
    assert_eq!(
        got.last_failure, None,
        "the Fresh standing did not clear the preceding failure record"
    );
    assert_eq!(
        stored_on_disk(&store, id),
        None,
        "the id is on disk before submission"
    );

    // Key input without CR does not open a turn.
    manager
        .write_stdin(id, b"hello", InputOrigin::User)
        .expect("key input");
    assert_eq!(
        profiles.get(id).and_then(|p| p.backend_session_id),
        None,
        "input without CR was counted as a submission"
    );

    // The first submission — after this call returns it must have settled in both the registry and the disk.
    manager
        .write_stdin(id, b"\r", InputOrigin::User)
        .expect("submit");
    assert_eq!(
        profiles.get(id).and_then(|p| p.backend_session_id),
        Some(minted),
        "the drawn id is not in the registry even after the first submission — the latch was not put on the session or the offer was missing"
    );
    assert_eq!(
        stored_on_disk(&store, id),
        Some(minted),
        "the drawn id is not on disk even after the first submission"
    );
    // The spawn-time snapshot's Running is always true on a link without a connection axis, so check separately whether the fake is
    //   actually alive.
    assert!(
        manager
            .list_agents()
            .iter()
            .any(|a| a.id == id && matches!(a.status, AgentStatus::Running)),
        "the Fresh incarnation is not alive after the input — the fake's `--session-id` branch died"
    );

    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(10), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&fake_dir);
    let _ = std::fs::remove_dir_all(&cwd);
}

/// ★A claude resume request without a handle opens a new conversation — the consequence of user decision D1 (ADR-0226)★.
///
/// `Ok`·Running, the fake launches with `--session-id` and no `--resume`. The drawn value is not persisted before submission — the old way
///   (draw and persist, launch `--resume <new value>`) left an id without a conversation and died.
// ADR-0226
#[test]
fn a_handleless_claude_resume_opens_a_new_conversation_without_persisting() {
    let (manager, _sink, profiles, store) =
        make_manager_with_store("handleless-resume", Arc::new(NoopControlChannel));
    let (fake_dir, args_log) = fake_claude_dir("handleless");
    let (profile, cwd) = claude_on_fake_path("handleless-claude", &fake_dir);
    let id = profile.id;
    profiles
        .try_upsert(profile.clone())
        .expect("store succeeds");

    let info = manager
        .activate_profile(&profile, SpawnMode::Resume)
        .expect("a resume request without a handle stands as a new conversation");
    assert!(
        matches!(info.status, AgentStatus::Running),
        "the outcome is not Running: {:?}",
        info.status
    );

    wait_until(Duration::from_secs(10), || {
        !fake_spawns(&args_log).is_empty()
    });
    let spawns = fake_spawns(&args_log);
    assert_eq!(
        spawns.len(),
        1,
        "the fake must have launched exactly once: {spawns:?}"
    );
    assert!(
        uuid_after(&spawns[0], "--session-id").is_some() && !spawns[0].contains("--resume"),
        "not a new-conversation argv: {spawns:?}"
    );
    assert_eq!(profiles.get(id).and_then(|p| p.backend_session_id), None);
    assert_eq!(
        stored_on_disk(&store, id),
        None,
        "an id without a conversation was persisted"
    );
    assert_eq!(profiles.get(id).and_then(|p| p.last_failure), None);

    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(10), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&fake_dir);
    let _ = std::fs::remove_dir_all(&cwd);
}

// ── Notices once per agent (ADR-0341) ──

/// Plans with the notices the test sets, or the block it sets — otherwise the Noop plan and kit (today's argv).
#[derive(Default)]
struct NoticeKits {
    notices: Mutex<Vec<Notice>>,
    block: Mutex<Option<LaunchBlocked>>,
}

impl NoticeKits {
    fn set(&self, notices: Vec<Notice>) {
        *self.notices.lock().unwrap() = notices;
    }
}

impl LaunchKitSource for NoticeKits {
    fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked> {
        if let Some(blocked) = self.block.lock().unwrap().clone() {
            return Err(blocked);
        }
        let mut plan = NoopLaunchKits.check(profile)?;
        plan.notices = self.notices.lock().unwrap().clone();
        Ok(plan)
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
}

fn noticing_manager(tag: &str) -> (AgentManager, Arc<NoticeKits>, Arc<ProfileRegistry>) {
    let (manager, _sink, profiles) = make_manager(tag);
    let kits = Arc::new(NoticeKits::default());
    let manager = manager.with_launch_ports(LaunchPorts {
        kits: kits.clone(),
        ..LaunchPorts::noop()
    });
    (manager, kits, profiles)
}

fn notice(code: &str, args: serde_json::Value, line: Option<u32>) -> Notice {
    Notice {
        code: code.to_string(),
        args,
        line,
    }
}

fn delivered(manager: &AgentManager, profile: &AgentProfile, mode: SpawnMode) -> Activated {
    manager
        .activate_profile_reporting(profile, mode, NoticeDelivery::Delivered)
        .expect("the activation succeeds")
}

fn recorded(profiles: &ProfileRegistry, id: AgentId) -> Option<String> {
    profiles.get(id).and_then(|p| p.notice_fingerprint)
}

fn kill_and_reap(manager: &AgentManager, id: AgentId) {
    manager.kill_agent(id).expect("kill_agent failed");
    assert!(
        wait_until(Duration::from_secs(10), || {
            !manager.list_agents().iter().any(|a| a.id == id)
        }),
        "the killed session was not reaped"
    );
}

/// A delivering caller is shown a set once: the same set (even with a moved line) comes back empty, a changed set
/// comes back in full, and the empty set is recorded too (X → none → X shows X again).
#[test]
fn a_delivered_fresh_start_shows_each_notice_set_once_per_agent() {
    let (manager, kits, profiles) = noticing_manager("notices-fresh");
    let (profile, batch, count) = long_lived_profile("notices-fresh");
    let id = profile.id;
    profiles.try_upsert(profile.clone()).expect("save succeeds");
    let x = vec![notice("omitted", json!({ "names": ["a.md"] }), Some(1))];

    kits.set(x.clone());
    assert_eq!(
        delivered(&manager, &profile, SpawnMode::Fresh).notices,
        x,
        "first delivery"
    );
    assert_eq!(recorded(&profiles, id), Some(notice_fingerprint(&x)));
    kill_and_reap(&manager, id);

    let mut moved = x.clone();
    moved[0].line = Some(9);
    kits.set(moved);
    assert!(
        delivered(&manager, &profile, SpawnMode::Fresh)
            .notices
            .is_empty(),
        "the same set (a moved line) is not shown again"
    );
    kill_and_reap(&manager, id);

    let y = vec![x[0].clone(), notice("other", json!({}), None)];
    kits.set(y.clone());
    assert_eq!(
        delivered(&manager, &profile, SpawnMode::Fresh).notices,
        y,
        "a changed set is shown in full"
    );
    kill_and_reap(&manager, id);

    kits.set(Vec::new());
    assert!(delivered(&manager, &profile, SpawnMode::Fresh)
        .notices
        .is_empty());
    assert_eq!(
        recorded(&profiles, id),
        Some(String::new()),
        "the empty set is recorded"
    );
    kill_and_reap(&manager, id);

    kits.set(y.clone());
    assert_eq!(
        delivered(&manager, &profile, SpawnMode::Fresh).notices,
        y,
        "after an empty set the old set is new again"
    );
    kill_and_reap(&manager, id);
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// Only a caller that shows the notices marks them as seen — a withheld reply and a moot activation (already running,
/// no launch) record nothing, so the next delivering caller still gets the set.
#[test]
fn a_withheld_or_moot_activation_records_nothing() {
    let (manager, kits, profiles) = noticing_manager("notices-withheld");
    let (profile, batch, count) = long_lived_profile("notices-withheld");
    let id = profile.id;
    profiles.try_upsert(profile.clone()).expect("save succeeds");
    let x = vec![notice("omitted", json!({ "names": ["a.md"] }), None)];
    kits.set(x.clone());

    let withheld = manager
        .activate_profile_reporting(&profile, SpawnMode::Fresh, NoticeDelivery::Withheld)
        .expect("starts");
    assert_eq!(withheld.notices, x, "the notices are returned either way");
    assert_eq!(
        recorded(&profiles, id),
        None,
        "a withheld reply records nothing"
    );

    let moot = delivered(&manager, &profile, SpawnMode::Fresh);
    assert_eq!(
        moot.info.epoch, withheld.info.epoch,
        "premise: nothing was launched"
    );
    assert!(
        moot.notices.is_empty(),
        "a moot activation carries no notices"
    );
    assert_eq!(
        recorded(&profiles, id),
        None,
        "a moot activation records nothing"
    );
    kill_and_reap(&manager, id);

    assert_eq!(delivered(&manager, &profile, SpawnMode::Fresh).notices, x);
    assert_eq!(recorded(&profiles, id), Some(notice_fingerprint(&x)));
    kill_and_reap(&manager, id);
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// A caller resume that started a process records (past the early-exit window), and the next resume with the same set
/// gets an empty list.
#[test]
fn a_caller_resume_records_and_the_next_resume_with_the_same_set_is_empty() {
    let (manager, kits, profiles) = noticing_manager("notices-resume");
    let (profile, batch, count) = long_lived_profile("notices-resume");
    let id = profile.id;
    profiles.try_upsert(profile.clone()).expect("save succeeds");
    let x = vec![notice("omitted", json!({ "names": ["s"] }), None)];
    kits.set(x.clone());

    assert_eq!(delivered(&manager, &profile, SpawnMode::Resume).notices, x);
    assert_eq!(recorded(&profiles, id), Some(notice_fingerprint(&x)));
    kill_and_reap(&manager, id);

    assert!(delivered(&manager, &profile, SpawnMode::Resume)
        .notices
        .is_empty());
    kill_and_reap(&manager, id);
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// Boot restore answers no caller — it records nothing, so the first caller launch after it still shows the set.
#[test]
fn boot_restore_records_no_notice_fingerprint() {
    let (manager, kits, profiles) = noticing_manager("notices-restore");
    let (mut profile, batch, count) = long_lived_profile("notices-restore");
    profile.auto_restore = true;
    let id = profile.id;
    profiles.try_upsert(profile.clone()).expect("save succeeds");
    let x = vec![notice("omitted", json!({ "names": ["s"] }), None)];
    kits.set(x.clone());

    let reports = manager.restore_all();
    assert!(
        reports
            .iter()
            .any(|r| r.agent_id == id && matches!(r.outcome, RestoreOutcome::Started)),
        "premise: the restore started the agent — got {reports:?}"
    );
    assert_eq!(
        recorded(&profiles, id),
        None,
        "boot restore recorded the set"
    );
    kill_and_reap(&manager, id);

    assert_eq!(delivered(&manager, &profile, SpawnMode::Fresh).notices, x);
    kill_and_reap(&manager, id);
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// A launch that fails after its process started (an early exit in the resume window) answers with an error — it
/// records nothing.
#[test]
fn a_post_start_failure_records_no_notice_fingerprint() {
    let (manager, kits, profiles) = noticing_manager("notices-early-exit");
    let (profile, batch, count) = always_early_exit_profile("notices-early-exit");
    let id = profile.id;
    profiles.try_upsert(profile.clone()).expect("save succeeds");
    kits.set(vec![notice("omitted", json!({ "names": ["s"] }), None)]);

    let result =
        manager.activate_profile_reporting(&profile, SpawnMode::Resume, NoticeDelivery::Delivered);
    assert!(
        result.is_err(),
        "premise: the resume exits early — got {result:?}"
    );
    assert!(
        wait_until(Duration::from_secs(5), || run_count(&count) == 1),
        "premise: the process started"
    );
    assert_eq!(
        recorded(&profiles, id),
        None,
        "a failed launch recorded the set"
    );

    let _ = wait_until(Duration::from_secs(5), || {
        !manager.list_agents().iter().any(|a| a.id == id)
    });
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// A caller resume with no stored handle opens a new conversation (the `resume_no_fallback` head) — that start is a
/// delivered success too: the notices come back and the fingerprint is recorded.
#[test]
fn a_handleless_caller_resume_that_opens_a_new_conversation_records_the_notices() {
    let (manager, kits, profiles) = noticing_manager("notices-handleless");
    let (fake_dir, args_log) = fake_claude_dir("notices-handleless");
    let (profile, cwd) = claude_on_fake_path("notices-handleless-claude", &fake_dir);
    let id = profile.id;
    profiles.try_upsert(profile.clone()).expect("save succeeds");
    let x = vec![notice("omitted", json!({ "names": ["s"] }), None)];
    kits.set(x.clone());

    let activated = delivered(&manager, &profile, SpawnMode::Resume);
    assert!(
        wait_until(Duration::from_secs(10), || !fake_spawns(&args_log)
            .is_empty()),
        "premise: the fake claude started"
    );
    let spawns = fake_spawns(&args_log);
    assert!(
        spawns.len() == 1 && !spawns[0].contains("--resume"),
        "premise: one new-conversation spawn — got {spawns:?}"
    );
    assert_eq!(activated.notices, x);
    assert_eq!(recorded(&profiles, id), Some(notice_fingerprint(&x)));

    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(10), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&fake_dir);
    let _ = std::fs::remove_dir_all(&cwd);
}

// ── The bus `agent.spawn` over the real manager (TRD S25 step 16) ──

struct NoRoster;

impl RosterChanged for NoRoster {
    fn roster_changed(&self) {}
}

struct NoUsage;

impl UsageCommandHost for NoUsage {
    fn get(&self, _backend: &AgentBackend) -> Option<UsageVendorRow> {
        None
    }
    fn refresh(&self, _backend: &AgentBackend) -> Option<UsageVendorRow> {
        None
    }
}

/// The bus `agent.spawn` is a delivering caller: its success answer carries the notices as rows and marks the set as
/// shown (the next start with the same set answers an empty list). A blocked launch starts nothing and is a
/// `CONFLICT` carrying every reason in `args`.
#[test]
fn the_bus_spawn_delivers_the_notices_and_answers_a_block_with_its_reasons() {
    let (manager, kits, profiles) = noticing_manager("notices-bus");
    let manager = Arc::new(manager);
    let (profile, batch, count) = long_lived_profile("notices-bus");
    let id = profile.id;
    profiles.try_upsert(profile.clone()).expect("save succeeds");
    let table = make_table(
        Arc::clone(&manager) as Arc<dyn AgentCommandHost>,
        Arc::new(NoRoster),
        Arc::new(NoUsage),
    );
    let spawn = || {
        let mut args = json!({ "target": id.to_string() });
        block_on(table.call("agent.spawn", &mut args).expect("declared"))
    };

    let x = vec![notice("omitted", json!({ "names": ["a.md"] }), Some(2))];
    kits.set(x.clone());
    let first = spawn().expect("the first start");
    assert_eq!(first["notices"], json!([notice_row(&x[0])]), "{first}");
    assert_eq!(
        recorded(&profiles, id),
        Some(notice_fingerprint(&x)),
        "the bus shows the set, so the start records it"
    );
    assert!(
        wait_until(Duration::from_secs(10), || run_count(&count) == 1),
        "premise: the first process started"
    );
    kill_and_reap(&manager, id);

    let again = spawn().expect("the second start");
    assert_eq!(
        again["notices"],
        json!([]),
        "the same set is not shown twice: {again}"
    );
    assert!(
        wait_until(Duration::from_secs(10), || run_count(&count) == 2),
        "premise: the second process started"
    );
    kill_and_reap(&manager, id);

    let blocked = LaunchBlocked {
        reasons: vec![BlockReason::Broken {
            item: "rules/a.md".into(),
            via: vec!["team.yaml".into()],
            reason: notice("yaml-syntax", json!({ "path": "rules/a.md" }), Some(7)),
        }],
    };
    *kits.block.lock().unwrap() = Some(blocked.clone());
    let err = spawn().expect_err("the launch is blocked");
    assert_eq!(err.code(), ErrorCode::Conflict, "{}", err.message());
    assert_eq!(err.args(), Some(&blocked.to_args()), "{}", err.message());
    assert!(
        manager.list_agents().is_empty(),
        "a blocked launch starts nothing"
    );
    assert_eq!(run_count(&count), 2, "no third process");

    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

// ── Create-and-start over the real manager (TRD S25 step 20 — ADR-0348) ──

/// A delivering create of an unregistered profile registers it, starts it, answers the notices and marks the set as
/// shown — so a later delivering wake of the same agent (after a kill) answers no notices (TRD S25 step 20 review finding 1).
#[test]
fn a_delivered_create_registers_starts_and_shows_the_notices_once() {
    let (manager, kits, profiles) = noticing_manager("create-delivered");
    let (profile, batch, count) = long_lived_profile("create-delivered");
    let id = profile.id;
    assert!(profiles.get(id).is_none(), "premise: not registered");
    let x = vec![notice("omitted", json!({ "names": ["a.md"] }), Some(1))];
    kits.set(x.clone());

    let created = manager
        .create_and_activate(&profile, NoticeDelivery::Delivered)
        .expect("the create starts");
    assert_eq!(created.info.id, id);
    assert_eq!(created.notices, x);
    assert!(profiles.get(id).is_some(), "registered");
    assert_eq!(recorded(&profiles, id), Some(notice_fingerprint(&x)));
    assert!(
        wait_until(Duration::from_secs(10), || run_count(&count) == 1),
        "premise: the process started"
    );
    kill_and_reap(&manager, id);

    let woken = manager
        .activate_profile_reporting(&profile, SpawnMode::Fresh, NoticeDelivery::Delivered)
        .expect("the wake starts");
    assert!(
        woken.notices.is_empty(),
        "the create already showed this set: {:?}",
        woken.notices
    );
    kill_and_reap(&manager, id);
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// A withheld create returns the notices but marks nothing — the next delivering wake still gets the set.
#[test]
fn a_withheld_create_records_nothing_and_the_next_delivered_wake_shows_the_set() {
    let (manager, kits, profiles) = noticing_manager("create-withheld");
    let (profile, batch, count) = long_lived_profile("create-withheld");
    let id = profile.id;
    let x = vec![notice("omitted", json!({ "names": ["a.md"] }), None)];
    kits.set(x.clone());

    let created = manager
        .create_and_activate(&profile, NoticeDelivery::Withheld)
        .expect("the create starts");
    assert_eq!(created.notices, x, "returned either way");
    assert_eq!(
        recorded(&profiles, id),
        None,
        "a withheld create records nothing"
    );
    kill_and_reap(&manager, id);

    assert_eq!(delivered(&manager, &profile, SpawnMode::Fresh).notices, x);
    assert_eq!(recorded(&profiles, id), Some(notice_fingerprint(&x)));
    kill_and_reap(&manager, id);
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// A block found before the add leaves nothing: no roster row, no last failure, no process.
#[test]
fn a_create_blocked_before_the_add_leaves_no_row_and_no_last_failure() {
    let (manager, kits, profiles) = noticing_manager("create-blocked-early");
    let (profile, batch, count) = long_lived_profile("create-blocked-early");
    let id = profile.id;
    let blocked = LaunchBlocked {
        reasons: vec![BlockReason::Broken {
            item: "rules/a.md".into(),
            via: vec!["team.yaml".into()],
            reason: notice("yaml-syntax", json!({ "path": "rules/a.md" }), Some(7)),
        }],
    };
    *kits.block.lock().unwrap() = Some(blocked);

    let refused = manager
        .create_and_activate(&profile, NoticeDelivery::Delivered)
        .expect_err("the launch is blocked");
    assert!(!refused.created, "nothing was registered");
    assert!(
        matches!(refused.error, PtyError::LaunchBlocked(_)),
        "{:?}",
        refused.error
    );
    assert!(profiles.get(id).is_none(), "no roster row");
    assert!(manager.list_agents().is_empty(), "no process");
    assert_eq!(run_count(&count), 0, "the batch never ran");
    let _ = std::fs::remove_file(&batch);
}

/// Passes the first check (`check_before_create`) and blocks every later one (the prelaunch after the add).
struct BlocksAfterTheAdd {
    checks: AtomicUsize,
    block: LaunchBlocked,
}

impl LaunchKitSource for BlocksAfterTheAdd {
    fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked> {
        if self.checks.fetch_add(1, Ordering::SeqCst) == 0 {
            NoopLaunchKits.check(profile)
        } else {
            Err(self.block.clone())
        }
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
}

/// A block found after the add leaves the agent registered and asleep, marked `Other` (ADR-0172).
#[test]
fn a_create_blocked_after_the_add_leaves_an_asleep_row_marked_other() {
    let (manager, _sink, profiles) = make_manager("create-blocked-late");
    let kits = Arc::new(BlocksAfterTheAdd {
        checks: AtomicUsize::new(0),
        block: LaunchBlocked {
            reasons: vec![BlockReason::Broken {
                item: "rules/a.md".into(),
                via: vec!["team.yaml".into()],
                reason: notice("yaml-syntax", json!({ "path": "rules/a.md" }), Some(7)),
            }],
        },
    });
    let manager = manager.with_launch_ports(LaunchPorts {
        kits: kits.clone(),
        ..LaunchPorts::noop()
    });
    let (profile, batch, count) = long_lived_profile("create-blocked-late");
    let id = profile.id;

    let refused = manager
        .create_and_activate(&profile, NoticeDelivery::Delivered)
        .expect_err("the prelaunch is blocked");
    assert!(refused.created, "the add happened");
    assert!(
        matches!(refused.error, PtyError::LaunchBlocked(_)),
        "{:?}",
        refused.error
    );
    assert!(
        kits.checks.load(Ordering::SeqCst) >= 2,
        "premise: checked before and after the add"
    );
    assert_eq!(
        profiles.get(id).and_then(|p| p.last_failure),
        Some(AgentFailureKind::Other)
    );
    assert!(manager.list_agents().is_empty(), "asleep — no process");
    assert_eq!(run_count(&count), 0, "the batch never ran");
    let _ = std::fs::remove_file(&batch);
}

/// A folder that does not exist is a failure after the add: the row stays asleep, marked `SpawnFailed`. The profile is
/// the bus create's shape — a claude JSON agent (stdio transport, which sets the child's working folder; the PTY
/// transport was observed to start anyway with a missing folder — where it runs is unverified), with a fake claude first on `PATH` so nothing real could start.
#[test]
fn a_create_in_a_missing_folder_leaves_an_asleep_row_marked_spawn_failed() {
    let (manager, _sink, profiles) = make_manager("create-missing-cwd");
    let (fake_dir, args_log) = fake_claude_dir("create-missing-cwd");
    let (mut profile, made_cwd) = claude_on_fake_path("create-missing-cwd", &fake_dir);
    profile.command = AgentCommand::Claude {
        extra_args: vec![],
        output_format: AgentOutputFormat::StreamJson,
    };
    profile.cwd = std::env::temp_dir().join(format!("engram-missing-cwd-{}", Uuid::new_v4()));
    assert!(!profile.cwd.exists(), "premise: the folder is missing");
    let id = profile.id;

    let refused = manager
        .create_and_activate(&profile, NoticeDelivery::Delivered)
        .expect_err("the spawn fails");
    assert!(refused.created, "the add happened: {:?}", refused.error);
    assert_eq!(
        profiles.get(id).and_then(|p| p.last_failure),
        Some(AgentFailureKind::SpawnFailed)
    );
    assert!(manager.list_agents().is_empty(), "asleep — no process");
    assert!(fake_spawns(&args_log).is_empty(), "the fake never ran");
    let _ = std::fs::remove_dir_all(&fake_dir);
    let _ = std::fs::remove_dir_all(&made_cwd);
}

/// An id already in the roster is refused before anything runs — a create never turns into a wake that merges the
/// caller's copy.
#[test]
fn a_create_of_an_id_already_in_the_roster_is_refused_and_starts_nothing() {
    let (manager, _sink, profiles) = make_manager("create-existing-id");
    let (profile, batch, count) = long_lived_profile("create-existing-id");
    let id = profile.id;
    profiles.try_upsert(profile.clone()).expect("save succeeds");

    let refused = manager
        .create_and_activate(&profile, NoticeDelivery::Delivered)
        .expect_err("the id is taken");
    assert!(!refused.created, "this call registered nothing");
    assert!(
        matches!(refused.error, PtyError::Unsupported(_)),
        "{:?}",
        refused.error
    );
    assert!(manager.list_agents().is_empty(), "nothing started");
    assert_eq!(profiles.get(id).and_then(|p| p.last_failure), None);
    assert_eq!(run_count(&count), 0, "the batch never ran");
    let _ = std::fs::remove_file(&batch);
}
