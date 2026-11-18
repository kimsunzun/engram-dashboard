//! ADR-0086 step 1 — manager-level control channel lifecycle integration test (FIX 10).
//!
//! Wires the real `DaemonControlChannel` + `AgentManager` (new_with_control) to assert the spawn/kill paths that unit tests and mcp_control.rs cannot see.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::persistence::FileProfileStore;
use engram_dashboard_agent::profile::{AgentCommand, AgentProfile, ProfileRegistry, SpawnMode};
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, ControlChannelNeeds, StatusSink,
};
use engram_dashboard_base::testing::wait_until;

/// The three axes an MCP-capable spawn passes — these files measure only the channel physics (token · config file); where production reads those values from is measured by `mail_gate.rs`.
///
/// ★The config file measured here is that of the row with the file axis on★ — set that field to false and the assertions collapse. That combination is measured by the test in the daemon's `control/mod.rs` (ADR-0209).
fn mcp_needs() -> ControlChannelNeeds {
    ControlChannelNeeds {
        accepts_mcp_config: true,
        writes_mcp_config_file: true,
        uses_mail: true,
    }
}

use engram_dashboard_daemon::control::mcp_config;
use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server_without_help, CommandTableSlot, ManagerSlot, MessagingSlot,
};
use engram_dashboard_daemon::control::registry::ControlRegistry;
use engram_dashboard_daemon::control::DaemonControlChannel;

struct NoopSink;
impl StatusSink for NoopSink {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

/// ★Ties the relay reaper to the process lifetime★
///
/// If dropped, the reaper empties and closes the seat table (`CommandDeliveries::drain`) and every later relay is bounced as 「shutting down」 — tied to a local of the two harnesses below, that happens when the function returns (this file was green in that state because its tests skipped the seat table).
/// ★No extra axis in the return tuple★: no test reads it, and the server is reclaimed at test end = process end. The sibling `control_send.rs` has the same shape.
static RELAY_SWEEPERS: std::sync::Mutex<
    Vec<engram_dashboard_daemon::command_delivery::BusSweeper>,
> = std::sync::Mutex::new(Vec::new());

fn hold_for_the_process(sweeper: engram_dashboard_daemon::command_delivery::BusSweeper) {
    RELAY_SWEEPERS
        .lock()
        .expect("relay sweepers poisoned")
        .push(sweeper);
}

async fn make_manager_with_control(
    tag: &str,
) -> (
    AgentManager,
    Arc<ControlRegistry>,
    PathBuf,
    engram_dashboard_daemon::control::mcp_server::McpServerHandle,
) {
    let registry = Arc::new(ControlRegistry::new());
    // ★Hold the reaper★ — tied to a local, the seat table closes when this function returns ([`hold_for_the_process`]).
    let (relay_bus, relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    hold_for_the_process(relay_sweeper);
    let handle = start_mcp_server_without_help(
        registry.clone(),
        Arc::new(ManagerSlot::new()),
        Arc::new(MessagingSlot::new()),
        // No control verbs are called here — an empty command table only makes that route 503.
        Arc::new(CommandTableSlot::new()),
        relay_bus,
    )
    .await
    .expect("start mcp server");
    let data_dir = std::env::temp_dir().join(format!("engram-mcp-mgr-{tag}-{}", AgentId::new_v4()));

    let control: Arc<dyn ControlChannel> = Arc::new(DaemonControlChannel::new(
        registry.clone(),
        handle.url.clone(),
        engram_dashboard_daemon::control::mcp_config::McpDir::new(data_dir.join("mcp-config")),
        None, // send_exe: the tests in this file do not use the CLI entry.
    ));

    let (manager, registry, data_dir, handle) =
        make_manager_with_injected(tag, registry, control, data_dir, handle);
    (manager, registry, data_dir, handle)
}

async fn make_manager_with_control_channel(
    tag: &str,
    control: Arc<dyn ControlChannel>,
) -> (
    AgentManager,
    Arc<ControlRegistry>,
    PathBuf,
    engram_dashboard_daemon::control::mcp_server::McpServerHandle,
) {
    let registry = Arc::new(ControlRegistry::new());
    // ★Hold the reaper★ — tied to a local, the seat table closes when this function returns ([`hold_for_the_process`]).
    let (relay_bus, relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    hold_for_the_process(relay_sweeper);
    let handle = start_mcp_server_without_help(
        registry.clone(),
        Arc::new(ManagerSlot::new()),
        Arc::new(MessagingSlot::new()),
        // No control verbs are called here — an empty command table only makes that route 503.
        Arc::new(CommandTableSlot::new()),
        relay_bus,
    )
    .await
    .expect("start mcp server");
    let data_dir = std::env::temp_dir().join(format!("engram-mcp-mgr-{tag}-{}", AgentId::new_v4()));
    make_manager_with_injected(tag, registry, control, data_dir, handle)
}

fn make_manager_with_injected(
    tag: &str,
    registry: Arc<ControlRegistry>,
    control: Arc<dyn ControlChannel>,
    data_dir: PathBuf,
    handle: engram_dashboard_daemon::control::mcp_server::McpServerHandle,
) -> (
    AgentManager,
    Arc<ControlRegistry>,
    PathBuf,
    engram_dashboard_daemon::control::mcp_server::McpServerHandle,
) {
    let sink: Arc<dyn StatusSink> = Arc::new(NoopSink);
    let profile_store = Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-mcp-mgr-prof-{tag}-{}", AgentId::new_v4())),
    ));
    let profiles = Arc::new(ProfileRegistry::new(profile_store));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = AgentManager::new_with_control(sink, profiles, tracker, control);

    (manager, registry, data_dir, handle)
}

// ── round-2 F3: claude (consumes the control channel) spawn ──
// ★No claude binary needed★: provision Err returns early via `?` before the real process spawn — is_err() cannot turn green by accident as "claude not installed".
#[tokio::test]
async fn claude_spawn_fails_closed_when_provision_errors() {
    use engram_dashboard_agent::types::{AgentId as CoreAgentId, ControlEndpoint, ProvisionError};

    struct FailingControl;
    impl ControlChannel for FailingControl {
        fn provision(
            &self,
            _id: CoreAgentId,
            _epoch: u32,
            _needs: engram_dashboard_agent::types::ControlChannelNeeds,
        ) -> Result<Option<ControlEndpoint>, ProvisionError> {
            Err(ProvisionError("injected provision failure".to_string()))
        }
        fn revoke(&self, _id: CoreAgentId, _epoch: u32) {}
    }

    let (manager, _registry, data_dir, handle) =
        make_manager_with_control_channel("claude-fail-closed", Arc::new(FailingControl)).await;
    let profile = AgentProfile::new(
        "claude-fail-closed".into(),
        AgentCommand::Claude {
            extra_args: vec![],
            output_format: engram_dashboard_agent::profile::AgentOutputFormat::Terminal,
        },
        PathBuf::from("."),
        vec![],
        false,
    );

    let res = manager.spawn_agent(&profile, SpawnMode::Fresh);
    assert!(
        res.is_err(),
        "claude consumes the control channel, so it fails closed on provision Err (spawn aborted)"
    );
    assert!(
        manager.list_agents().is_empty(),
        "a fail-closed spawn must not register a session"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── round-2 F3: shell (does not consume the control channel) spawn ────────────────────────────────────────
#[tokio::test]
async fn shell_spawn_succeeds_with_failing_control_channel() {
    use engram_dashboard_agent::types::{AgentId as CoreAgentId, ControlEndpoint, ProvisionError};

    struct FailingControl;
    impl ControlChannel for FailingControl {
        fn provision(
            &self,
            _id: CoreAgentId,
            _epoch: u32,
            _needs: engram_dashboard_agent::types::ControlChannelNeeds,
        ) -> Result<Option<ControlEndpoint>, ProvisionError> {
            Err(ProvisionError("must not be called for shell".to_string()))
        }
        fn revoke(&self, _id: CoreAgentId, _epoch: u32) {}
    }

    let (manager, registry, data_dir, handle) =
        make_manager_with_control_channel("shell-succeeds", Arc::new(FailingControl)).await;
    let profile = AgentProfile::new(
        "shell-succeeds".into(),
        AgentCommand::Shell {
            program: engram_dashboard_platform::shell::default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        false,
    );

    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("a shell spawn must succeed regardless of provision failure (F3)")
        .into_started()
        .expect("this call really launches (not a duplicate request)");
    assert!(
        wait_until(Duration::from_secs(3), || manager.list_agents().len() == 1),
        "session exists right after the shell spawn"
    );
    assert_eq!(
        registry.live_token_count(),
        0,
        "a shell does not call provision, so the registry is untouched (0 live tokens)"
    );

    manager.kill_agent(info.id).expect("kill ok");
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── FIX 3: resources ProvisionGuard reclaims ──
// ★The drop path itself is not verified here★: even with cmd/c claude (or a wrong cwd) the Windows ConPTY spawn does not fail, so "transport open failed → guard drop revoke" cannot be reproduced deterministically on the integration path. This only checks that the same revoke the guard calls on drop reclaims the real resources.
#[tokio::test]
async fn provision_guard_revoke_reclaims_real_token_and_config_file() {
    use engram_dashboard_agent::types::ControlChannel;

    let (_manager, registry, data_dir, handle) =
        make_manager_with_control("provision-reclaim").await;
    let channel = DaemonControlChannel::new(
        registry.clone(),
        handle.url.clone(),
        engram_dashboard_daemon::control::mcp_config::McpDir::new(data_dir.join("mcp-config")),
        None,
    );
    let id = AgentId::new_v4();

    let ep = channel
        .provision(id, 0, mcp_needs())
        .expect("provision ok")
        .expect("endpoint");
    assert_eq!(
        registry.live_token_count(),
        1,
        "one live token after provision"
    );
    let cfg = ep
        .config_path
        .clone()
        .expect("MCP-capable → config_path Some");
    assert!(cfg.exists(), "provision writes a real config file");

    channel.revoke(id, 0);
    assert_eq!(
        registry.live_token_count(),
        0,
        "0 live tokens after revoke (the resource FIX 3 reclaims = the real token)"
    );
    assert!(
        !cfg.exists(),
        "config file deleted after revoke (the resource FIX 3 reclaims = the real file)"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── FIX 4: revoke-before-kill ──
// ★Backend-independent★: kill_agent calls revoke whatever the session backend, so spawn with a shell session and plant that session's (id, incarnation marker) token directly in the registry to mimic "a provisioned claude".
//   ★Use the marker spawn returned — do not hardcode a constant★: the marker is a random number per incarnation (not 0 even for the first spawn); a mismatched planted value is caught by revoke's epoch-guard and the token is not deleted (the planting side would be wrong, not the revoke-before-kill under test).
#[tokio::test]
async fn kill_revokes_token_before_pump_join() {
    let (manager, registry, data_dir, handle) = make_manager_with_control("kill-revoke").await;

    let profile = AgentProfile::new(
        "kill-revoke".into(),
        AgentCommand::Shell {
            program: engram_dashboard_platform::shell::default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn ok")
        .into_started()
        .expect("this call really launches (not a duplicate request)");
    assert!(
        wait_until(Duration::from_secs(3), || manager.list_agents().len() == 1),
        "session exists right after spawn"
    );
    registry.issue(
        info.id,
        info.epoch,
        "simulated-live-token".to_string(),
        true,
    );
    assert_eq!(registry.live_token_count(), 1, "one planted live token");

    manager.kill_agent(info.id).expect("kill ok");
    assert_eq!(
        registry.live_token_count(),
        0,
        "the token is already revoked right after kill_agent returns (revoke-before-kill — no need to wait for the reaper backstop)"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── FIX 5: boot sweep ─────────────────────────────────────────────────────────────────
#[test]
fn boot_sweep_removes_stale_configs() {
    let data_dir = std::env::temp_dir().join(format!("engram-mcp-sweep-{}", AgentId::new_v4()));
    let sub = data_dir.join("mcp-config");
    std::fs::create_dir_all(&sub).expect("mkdir");

    let f1 = sub.join(format!("{}-0.json", AgentId::new_v4()));
    let f2 = sub.join(format!("{}-3.json", AgentId::new_v4()));
    std::fs::write(&f1, "{\"stale\":1}").unwrap();
    std::fs::write(&f2, "{\"stale\":2}").unwrap();
    assert!(f1.exists() && f2.exists(), "stale files exist beforehand");

    mcp_config::sweep_stale_configs(&mcp_config::McpDir::new(&sub));
    assert!(!f1.exists(), "stale file 1 deleted after the sweep");
    assert!(!f2.exists(), "stale file 2 deleted after the sweep");

    let fresh = std::env::temp_dir().join(format!("engram-mcp-sweep-none-{}", AgentId::new_v4()));
    mcp_config::sweep_stale_configs(&mcp_config::McpDir::new(fresh.join("mcp-config"))); // must pass without panic.

    let _ = std::fs::remove_dir_all(&data_dir);
}
