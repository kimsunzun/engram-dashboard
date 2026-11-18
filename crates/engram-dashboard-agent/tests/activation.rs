//! Integration tests for manual activation (activate_profile) — ADR-0082 (fresh-fallback removed, resume only). The real claude cannot
//!   launch in CI, so a cmd.exe batch profile simulates it; Windows only (`#[cfg(windows)]`). Single spawn, no global races.

#![cfg(windows)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use uuid::Uuid;

use engram_dashboard_agent::backend;
use engram_dashboard_agent::failure::AgentFailureKind;
use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::persistence::{FilePresetStore, FileProfileStore};
use engram_dashboard_agent::preset::PresetRegistry;
use engram_dashboard_agent::profile::{
    AgentCommand, AgentOutputFormat, AgentProfile, ProfileRegistry, ProfileStore, SpawnMode,
};
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, ControlEndpoint, InputOrigin,
    NoopControlChannel, ProvisionError, StatusSink,
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
    let preset_store = Arc::new(FilePresetStore::new(std::env::temp_dir().join(format!(
        "engram-test-activate-preset-{tag}-{}",
        Uuid::new_v4()
    ))));
    let presets = Arc::new(PresetRegistry::new(preset_store));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager =
        AgentManager::new_with_control(sink_dyn, profiles.clone(), presets, tracker, control);
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
