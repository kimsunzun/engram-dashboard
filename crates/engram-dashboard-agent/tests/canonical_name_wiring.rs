//! Integration test of canonical name **wiring** (ADR-0101 WYSIWYA — regression net of the production path).
//!
//! ★Why a separate file (mutation probe D9-b)★: `src/name.rs`'s derivation tests call the helpers **directly**, so they stay green even if
//!   production **stops using** them. These three mutations in `manager::resolve_canonical_name` still passed every existing agent and
//!   daemon suite:
//!   ① `profile.cwd` instead of `session.cwd` ② `resolve_display_name` instead of the guarded `canonical_name_or_id_fallback`
//!   ③ skipping derivation and returning the stored `AgentProfile.name`. ③ is the very bug ADR-0101 fixed (stored name = often the full
//!   cwd string as the routing address → tree display ≠ routing address).
//!
//! ★So the helpers are not called here★: a real session is spawned and only **the manager's public projections** (`AgentInfo.name` from
//!   `spawn_agent` · `list_agents()` · `canonical_name(id)`) are looked at; cases are split so each mutation is caught by a different case.
//!
//! ★Cases ①·② seed the profile into the roster before spawning — do not drop the seeding★: spawning an id not in the roster
//!   registers it and pins `display_name` (TRD S25 §1-5 「display_name pinned」). A pinned name makes the live-axis derivation return
//!   it first, so neither the cwd axis nor the guard axis is reached and ①·② leak green. Spawning an existing id assigns no name, so
//!   `display_name` stays as seeded (`None` · blank) — both cases assert that first after spawning.
//!
//! ★Relation to the asleep parking key (ADR-0116 decision 1)★: an asleep profile's name is derived by
//!   `AgentProfile::canonical_name_when_live()` and **must equal the live name asserted here** (else the parking key and the post-restore
//!   name diverge and mail silently expires after the 24h TTL). Each case also pins that the two derivations give **the same string**.
//!
//! Windows only — spawns cmd.exe.
// ADR-0101 / ADR-0116

#![cfg(windows)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::persistence::FileProfileStore;
use engram_dashboard_agent::profile::{AgentCommand, AgentProfile, ProfileRegistry, SpawnMode};
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::types::{AgentId, AgentInfo, AgentStatus, StatusSink};

struct NoopSink;
impl StatusSink for NoopSink {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

fn make_manager(tag: &str) -> (AgentManager, Arc<ProfileRegistry>) {
    let sink: Arc<dyn StatusSink> = Arc::new(NoopSink);
    let profiles = Arc::new(ProfileRegistry::new(Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-canon-{tag}-{}", Uuid::new_v4())),
    ))));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    (AgentManager::new(sink, profiles.clone(), tracker), profiles)
}

/// Spawns an existing id — registration assigns no name, so the seeded `display_name` stays (file head items ①·②).
fn spawn_seeded(
    manager: &AgentManager,
    profiles: &ProfileRegistry,
    profile: &AgentProfile,
) -> AgentInfo {
    profiles
        .try_upsert(profile.clone())
        .expect("save the roster seed");
    let info = manager
        .spawn_agent(profile, SpawnMode::Fresh)
        .expect("spawn")
        .into_started()
        .expect("this call actually launches (not a duplicate request)");
    assert_eq!(
        profiles.get(profile.id).expect("in the roster").display_name,
        profile.display_name,
        "spawning an existing id pinned a name — the pinned name hides the live-axis derivation, so this case catches nothing"
    );
    info
}

/// A harmless child that does not die at once (interactive cmd.exe — a prompt under the PTY). Name derivation ignores process behavior;
///   only "a live session exists" is needed.
fn idle_shell(name: &str, cwd: PathBuf) -> AgentProfile {
    AgentProfile::new(
        name.to_string(),
        AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        },
        cwd,
        vec![],
        false,
    )
}

fn projected_name(manager: &AgentManager, spawned: &AgentInfo) -> String {
    let listed = manager
        .list_agents()
        .into_iter()
        .find(|a| a.id == spawned.id)
        .expect("the spawned session is in the list");
    let by_id = manager.canonical_name(spawned.id).expect("id → name");
    assert_eq!(
        spawned.name, listed.name,
        "the names of the spawn return and list_agents diverged (they must use the same core)"
    );
    assert_eq!(
        spawned.name, by_id,
        "agent_info and canonical_name diverged (the envelope sender ≠ routing address — ADR-0101)"
    );
    spawned.name.clone()
}

#[test]
fn the_stored_profile_name_field_is_never_used_as_the_routing_address() {
    // ★Regression net of the bug ADR-0101 fixed (mutation ③)★: `AgentProfile.name` is the original string from CreateProfile, so it is
    //   **often a path**; using it splits the tree display name (basename) from the routing address.
    let (manager, _) = make_manager("namefield");
    let cwd = std::env::temp_dir();
    let raw_name = "C:\\some\\raw\\name-field-must-not-be-the-address";
    let profile = idle_shell(raw_name, cwd.clone());

    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn")
        .into_started()
        .expect("this call actually launches (not a duplicate request)");
    let name = projected_name(&manager, &info);
    manager.kill_agent(info.id).ok();

    assert_ne!(
        name, raw_name,
        "the stored name field leaked out as the routing address (ADR-0101 regression)"
    );
    let expected = dunce::canonicalize(&cwd)
        .expect("temp_dir exists")
        .file_name()
        .expect("basename")
        .to_string_lossy()
        .to_string();
    assert_eq!(name, expected, "name = basename(canonicalize(cwd))");
}

#[test]
fn a_relative_cwd_derives_the_name_from_the_canonicalized_session_cwd() {
    // ★Catching mutation ① (session.cwd → profile.cwd)★: the profile can hold a raw path (`"."`·`".."`· symlinks · differently-cased
    //   Windows paths) that spawn canonicalizes into session.cwd. The tree draws `basename(AgentInfo.cwd)` = basename(session.cwd), so
    //   deriving from profile.cwd would split the tree display (`engram-dashboard-agent`) from the routing address (`"."`).
    let (manager, profiles) = make_manager("relcwd");
    let profile = idle_shell("rel", PathBuf::from("."));

    let info = spawn_seeded(&manager, &profiles, &profile);
    let name = projected_name(&manager, &info);
    manager.kill_agent(info.id).ok();

    assert_ne!(
        name, ".",
        "the raw cwd basename must not become the address"
    );
    let expected = dunce::canonicalize(".")
        .expect("the cwd exists")
        .file_name()
        .expect("basename")
        .to_string_lossy()
        .to_string();
    assert_eq!(
        name, expected,
        "must be the basename of the canonicalized cwd"
    );
    assert_eq!(
        profile.canonical_name_when_live(),
        name,
        "if the asleep parking key and the live name diverge, mail cannot meet its owner"
    );
}

#[test]
fn a_blank_display_name_override_is_ignored_by_the_production_path() {
    // ★Catching mutation ② (canonical_name_or_id_fallback → resolve_display_name)★: the latter uses an empty/whitespace-only override **as
    //   the name as is** — a whitespace address cannot be pointed at and draws an empty tree entry. The production path must use the
    //   guarded helper.
    let (manager, profiles) = make_manager("blankoverride");
    let cwd = std::env::temp_dir();
    let mut profile = idle_shell("blank", cwd.clone());
    profile.display_name = Some("   ".into());

    let info = spawn_seeded(&manager, &profiles, &profile);
    let name = projected_name(&manager, &info);
    manager.kill_agent(info.id).ok();

    assert!(
        !name.trim().is_empty(),
        "a whitespace-only override leaked out as the address: {name:?}"
    );
    let expected = dunce::canonicalize(&cwd)
        .expect("temp_dir exists")
        .file_name()
        .expect("basename")
        .to_string_lossy()
        .to_string();
    assert_eq!(
        name, expected,
        "a whitespace override is ignored, cwd basename"
    );
    assert_eq!(
        profile.canonical_name_when_live(),
        name,
        "the asleep parking key must pass the same guard too (one rule for the two derivations)"
    );
}

#[test]
fn a_real_display_name_override_wins_over_the_cwd_basename() {
    // ★The premise of deletion cleanup (ADR-0116 decision 3)★: if this equality breaks, the cleanup points at the wrong queue.
    let (manager, _) = make_manager("override");
    let mut profile = idle_shell("ovr", std::env::temp_dir());
    profile.display_name = Some("Renamed-Boss".into());

    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn")
        .into_started()
        .expect("this call actually launches (not a duplicate request)");
    let name = projected_name(&manager, &info);
    manager.kill_agent(info.id).ok();

    assert_eq!(name, "Renamed-Boss");
    assert_eq!(profile.canonical_name_when_live(), name);
}
