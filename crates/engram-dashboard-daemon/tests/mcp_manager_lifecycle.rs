//! ADR-0086 스텝 1 — manager-level 제어 채널 생명주기 통합 테스트(FIX 10).
//!
//! 실제 `DaemonControlChannel` + `AgentManager`(new_with_control)를 배선해, unit·mcp_control.rs 가 못
//! 보는 spawn/kill 경로를 단언한다.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::preset::PresetRegistry;
use engram_dashboard_core::agent::profile::{
    AgentCommand, AgentProfile, ProfileRegistry, SpawnMode,
};
use engram_dashboard_core::agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, StatusSink,
};
use engram_dashboard_core::persistence::{FilePresetStore, FileProfileStore};

use engram_dashboard_daemon::control::mcp_config;
use engram_dashboard_daemon::control::mcp_server::{start_mcp_server, ManagerSlot, MessagingSlot};
use engram_dashboard_daemon::control::priming::NoopPrimingProvider;
use engram_dashboard_daemon::control::registry::ControlRegistry;
use engram_dashboard_daemon::control::DaemonControlChannel;

struct NoopSink;
impl StatusSink for NoopSink {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

fn wait_until<F: Fn() -> bool>(timeout: Duration, cond: F) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    cond()
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
    let handle = start_mcp_server(
        registry.clone(),
        Arc::new(ManagerSlot::new()),
        Arc::new(MessagingSlot::new()),
    )
    .await
    .expect("start mcp server");
    let data_dir = std::env::temp_dir().join(format!("engram-mcp-mgr-{tag}-{}", AgentId::new_v4()));

    let control: Arc<dyn ControlChannel> = Arc::new(DaemonControlChannel::new(
        registry.clone(),
        handle.url.clone(),
        data_dir.clone(),
        None, // send_exe: 이 파일의 테스트는 CLI 입구를 쓰지 않는다.
        Arc::new(NoopPrimingProvider),
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
    let handle = start_mcp_server(
        registry.clone(),
        Arc::new(ManagerSlot::new()),
        Arc::new(MessagingSlot::new()),
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
    let preset_store = Arc::new(FilePresetStore::new(
        std::env::temp_dir().join(format!("engram-mcp-mgr-preset-{tag}-{}", AgentId::new_v4())),
    ));
    let presets = Arc::new(PresetRegistry::new(preset_store));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            sessions_dir: None,
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = AgentManager::new_with_control(sink, profiles, presets, tracker, control);

    (manager, registry, data_dir, handle)
}

// ── round-2 F3: claude(제어 채널 소비) spawn ─────────────────────────────────────────
// ★claude 바이너리 불요★: provision Err 가 `?` 로 조기 반환하므로 실제 프로세스 spawn 에 닿지 않는다 —
//   is_err() 가 "claude 미설치" 로 우연히 초록이 되는 경로가 없다.
#[tokio::test]
async fn claude_spawn_fails_closed_when_provision_errors() {
    use engram_dashboard_core::agent::types::{
        AgentId as CoreAgentId, ControlEndpoint, ProvisionError,
    };

    struct FailingControl;
    impl ControlChannel for FailingControl {
        fn provision(
            &self,
            _id: CoreAgentId,
            _epoch: u32,
            _accepts_mcp_config: bool,
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
            output_format: engram_dashboard_core::agent::profile::ClaudeOutputFormat::Terminal,
        },
        PathBuf::from("."),
        vec![],
        false,
    );

    let res = manager.spawn_agent(&profile, SpawnMode::Fresh);
    assert!(
        res.is_err(),
        "claude 는 제어 채널을 소비하므로 provision Err 에 fail-closed(스폰 중단)"
    );
    assert!(
        manager.list_agents().is_empty(),
        "fail-closed spawn 은 세션을 등록하지 않아야"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── round-2 F3: shell(제어 채널 미소비) spawn ────────────────────────────────────────
#[tokio::test]
async fn shell_spawn_succeeds_with_failing_control_channel() {
    use engram_dashboard_core::agent::types::{
        AgentId as CoreAgentId, ControlEndpoint, ProvisionError,
    };

    struct FailingControl;
    impl ControlChannel for FailingControl {
        fn provision(
            &self,
            _id: CoreAgentId,
            _epoch: u32,
            _accepts_mcp_config: bool,
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
            program: engram_dashboard_core::agent::manager::default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        false,
    );

    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("shell 스폰은 provision 실패와 무관하게 성공해야(F3)");
    assert!(
        wait_until(Duration::from_secs(3), || manager.list_agents().len() == 1),
        "셸 spawn 직후 세션 존재"
    );
    assert_eq!(
        registry.live_token_count(),
        0,
        "셸은 provision 을 안 부르므로 registry 미접촉(산 토큰 0)"
    );

    manager.kill_agent(info.id).expect("kill ok");
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── FIX 3: ProvisionGuard 회수 자원 ──────────────────────────────────────────────────
// ★drop 경로 자체는 여기서 검증되지 않는다★: cmd/c claude(또는 잘못된 cwd)로도 Windows ConPTY spawn 이
//   실패하지 않아 "transport open 실패 → guard drop revoke" 를 통합 경로에서 결정적으로 재현할 수 없다.
//   여기선 guard 가 drop 시 부르는 것과 **같은** revoke 가 실존 자원을 회수하는지만 본다.
#[tokio::test]
async fn provision_guard_revoke_reclaims_real_token_and_config_file() {
    use engram_dashboard_core::agent::types::ControlChannel;

    let (_manager, registry, data_dir, handle) =
        make_manager_with_control("provision-reclaim").await;
    let channel = DaemonControlChannel::new(
        registry.clone(),
        handle.url.clone(),
        data_dir.clone(),
        None,
        Arc::new(NoopPrimingProvider),
    );
    let id = AgentId::new_v4();

    let ep = channel
        .provision(id, 0, true)
        .expect("provision ok")
        .expect("endpoint");
    assert_eq!(registry.live_token_count(), 1, "provision 후 산 토큰 1개");
    let cfg = ep
        .config_path
        .clone()
        .expect("MCP-capable → config_path Some");
    assert!(cfg.exists(), "provision 이 실제 config 파일을 씀");

    channel.revoke(id, 0);
    assert_eq!(
        registry.live_token_count(),
        0,
        "revoke 후 산 토큰 0(FIX 3 가 회수하는 자원 = 실 토큰)"
    );
    assert!(
        !cfg.exists(),
        "revoke 후 config 파일 삭제(FIX 3 가 회수하는 자원 = 실 파일)"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── FIX 4: revoke-before-kill ────────────────────────────────────────────────────────
// ★backend 무관★: kill_agent 는 세션 backend 와 무관하게 revoke 를 부르므로, 셸 세션으로 spawn 하되
//   registry 에 (id, epoch=0) 토큰을 직접 심어 "provision 된 claude" 를 모사한다.
#[tokio::test]
async fn kill_revokes_token_before_pump_join() {
    let (manager, registry, data_dir, handle) = make_manager_with_control("kill-revoke").await;

    let profile = AgentProfile::new(
        "kill-revoke".into(),
        AgentCommand::Shell {
            program: engram_dashboard_core::agent::manager::default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn ok");
    assert!(
        wait_until(Duration::from_secs(3), || manager.list_agents().len() == 1),
        "spawn 직후 세션 존재"
    );
    registry.issue(info.id, 0, "simulated-live-token".to_string());
    assert_eq!(registry.live_token_count(), 1, "심은 산 토큰 1개");

    manager.kill_agent(info.id).expect("kill ok");
    assert_eq!(
        registry.live_token_count(),
        0,
        "kill_agent 반환 직후 토큰이 이미 폐기(revoke-before-kill — reaper backstop 대기 불요)"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── FIX 5: 부팅 스윕 ─────────────────────────────────────────────────────────────────
#[test]
fn boot_sweep_removes_stale_configs() {
    let data_dir = std::env::temp_dir().join(format!("engram-mcp-sweep-{}", AgentId::new_v4()));
    let sub = data_dir.join("mcp-config");
    std::fs::create_dir_all(&sub).expect("mkdir");

    let f1 = sub.join(format!("{}-0.json", AgentId::new_v4()));
    let f2 = sub.join(format!("{}-3.json", AgentId::new_v4()));
    std::fs::write(&f1, "{\"stale\":1}").unwrap();
    std::fs::write(&f2, "{\"stale\":2}").unwrap();
    assert!(f1.exists() && f2.exists(), "사전 stale 파일 존재");

    mcp_config::sweep_stale_configs(&data_dir);
    assert!(!f1.exists(), "스윕 후 stale 파일 1 삭제");
    assert!(!f2.exists(), "스윕 후 stale 파일 2 삭제");

    let fresh = std::env::temp_dir().join(format!("engram-mcp-sweep-none-{}", AgentId::new_v4()));
    mcp_config::sweep_stale_configs(&fresh); // panic 없이 통과해야.

    let _ = std::fs::remove_dir_all(&data_dir);
}
