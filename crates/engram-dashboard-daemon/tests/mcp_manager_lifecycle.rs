//! ADR-0086 스텝 1 — manager-level 제어 채널 생명주기 통합 테스트(FIX 10).
//!
//! 실제 `DaemonControlChannel` + `AgentManager`(new_with_control)를 배선해, 통합 테스트에서 다음을
//! 단언한다(unit·mcp_control.rs 가 못 보는 spawn/kill 경로):
//!   - spawn 실패(exe 부재) 경로에서 발급된 토큰/config 가 revoke 된다(FIX 3 leak 방지).
//!   - kill 이 pump join(최대 5s) 을 기다리지 않고 즉시 토큰을 폐기한다(FIX 4 revoke-before-kill).
//!   - 부팅 스윕이 stale mcp-config 파일을 청소한다(FIX 5).
//!
//! start_mcp_server 로 in-process MCP 서버를 띄우고 그 registry 를 DaemonControlChannel·AgentManager 가
//! 공유한다(start_test_server idiom 미러 — 실서버 조립과 같은 경로).

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
use engram_dashboard_daemon::control::mcp_server::{start_mcp_server, ManagerSlot};
use engram_dashboard_daemon::control::priming::NoopPrimingProvider;
use engram_dashboard_daemon::control::registry::ControlRegistry;
use engram_dashboard_daemon::control::DaemonControlChannel;

/// 통지를 삼키는 no-op status sink(생명주기 테스트는 토큰/파일만 본다).
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

/// (manager, registry, data_dir) 구성 — 실 DaemonControlChannel 을 끼운 AgentManager. tag 로 임시
/// 디렉토리를 격리한다. 반환한 handle 은 shutdown 을 위해 호출자가 붙잡는다.
async fn make_manager_with_control(
    tag: &str,
) -> (
    AgentManager,
    Arc<ControlRegistry>,
    PathBuf,
    engram_dashboard_daemon::control::mcp_server::McpServerHandle,
) {
    let registry = Arc::new(ControlRegistry::new());
    let handle = start_mcp_server(registry.clone(), Arc::new(ManagerSlot::new()))
        .await
        .expect("start mcp server");
    let data_dir = std::env::temp_dir().join(format!("engram-mcp-mgr-{tag}-{}", AgentId::new_v4()));

    let control: Arc<dyn ControlChannel> = Arc::new(DaemonControlChannel::new(
        registry.clone(),
        handle.url.clone(),
        data_dir.clone(),
        None,                          // send_exe: 이 테스트는 CLI 입구 불요.
        Arc::new(NoopPrimingProvider), // ADR-0092: 프라이밍 무관 테스트.
    ));

    let (manager, registry, data_dir, handle) =
        make_manager_with_injected(tag, registry, control, data_dir, handle);
    (manager, registry, data_dir, handle)
}

/// 커스텀 ControlChannel 주입형(F3 테스트용) — 실패하는 mock 을 끼워 backend-conditional provisioning 을
/// 검증한다. MCP 서버·registry 는 여전히 띄우되(핸들 반환), manager 엔 주어진 control 을 배선한다.
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
    let handle = start_mcp_server(registry.clone(), Arc::new(ManagerSlot::new()))
        .await
        .expect("start mcp server");
    let data_dir = std::env::temp_dir().join(format!("engram-mcp-mgr-{tag}-{}", AgentId::new_v4()));
    make_manager_with_injected(tag, registry, control, data_dir, handle)
}

/// 공통 조립부 — registry·control·data_dir·handle 을 받아 나머지 레지스트리(profile/preset/tracker)를
/// 격리 구성하고 AgentManager 를 배선한다.
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

// ── round-2 F3: claude(제어 채널 소비) spawn 은 provision Err 에 fail-closed ────────────────
// provision 이 Err 를 돌려주면 claude 스폰은 transport open 전에 중단된다(제어 채널 없이 도는 에이전트
// 금지). ★claude 바이너리 불요★: provision Err 가 `?` 로 조기 반환하므로 실제 프로세스 spawn 에 닿지
// 않는다. 실패하는 ControlChannel 을 주입해 이 fail-closed 계약을 격리 검증한다(F3 — provision 을
// **부르는** backend 에만 fail-closed 가 적용됨을 shell 대조군과 함께 본다).
#[tokio::test]
async fn claude_spawn_fails_closed_when_provision_errors() {
    use engram_dashboard_core::agent::types::{
        AgentId as CoreAgentId, ControlEndpoint, ProvisionError,
    };

    // 항상 provision 을 실패시키는 ControlChannel(CSPRNG/파일 write 실패 모사).
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

    // claude 프로필 — supports_control_channel=true 라 manager 가 provision 을 부른다(→ Err → 중단).
    //   ★claude 미설치 무관★: provision Err 가 transport open 전에 `?` 로 반환하므로 바이너리 불요.
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

// ── round-2 F3: shell(제어 채널 미소비) spawn 은 provision 실패와 무관하게 성공 ─────────────────
// shell 은 supports_control_channel=false 라 manager 가 provision 을 **아예 부르지 않는다** — 따라서
// 실패하는 ControlChannel 을 주입해도 셸 스폰은 성공한다(config-write 실패가 MCP 불필요한 스폰을
// 중단시키던 round-2 F3 회귀 차단). registry 도 전혀 건드리지 않는다(산 토큰 0 유지).
#[tokio::test]
async fn shell_spawn_succeeds_with_failing_control_channel() {
    use engram_dashboard_core::agent::types::{
        AgentId as CoreAgentId, ControlEndpoint, ProvisionError,
    };

    // 불리면 반드시 실패하는 ControlChannel — 셸 경로에선 애초에 불리지 않아야 한다.
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

    // 정리: kill(셸 세션엔 revoke 가 idempotent no-op).
    manager.kill_agent(info.id).expect("kill ok");
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── FIX 3(유지): ProvisionGuard 가 회수하는 자원 = 실 토큰 + 실 config 파일(provision→revoke 생명주기) ─
// ProvisionGuard(FIX 3)는 provision 성공 후 세션 등록 전 어느 실패에서든 발급된 토큰+config 를 revoke
// (drop-time)한다. ★Windows spawn 실패 유도 불가★: cmd/c claude(또는 잘못된 cwd)로도 ConPTY spawn 이
// 실패하지 않아 통합 경로에서 "transport open 실패 → guard drop revoke" 를 결정적으로 재현할 수 없다
// (자세한 사유는 회수 보고 §deviations). guard 의 회수 **동작 자체**(armed→drop revoke)는 단순 RAII 라
// core 배선(arm=endpoint Some, disarm=등록 성공, drop=revoke)으로 자명하고, 여기선 guard 가 drop 시 부르는
// 것과 동일한 revoke 가 **실존 자원**(registry 토큰 + 디스크 config 파일)을 실제로 회수하는지를 실
// DaemonControlChannel provision→revoke 로 결정적으로 단언한다.
#[tokio::test]
async fn provision_guard_revoke_reclaims_real_token_and_config_file() {
    use engram_dashboard_core::agent::types::ControlChannel;

    let (_manager, registry, data_dir, handle) =
        make_manager_with_control("provision-reclaim").await;
    // guard drop 이 부르는 그 경로(control.revoke)를 실 DaemonControlChannel 로 직접 돌린다.
    let channel = DaemonControlChannel::new(
        registry.clone(),
        handle.url.clone(),
        data_dir.clone(),
        None,
        Arc::new(NoopPrimingProvider), // ADR-0092: revoke 경로 테스트 — 프라이밍 무관.
    );
    let id = AgentId::new_v4();

    // provision — 실 토큰 발급 + 실 config 파일 write(guard 가 arm 되는 시점의 상태와 동일).
    //   ADR-0099: MCP-capable(true)로 provision 해야 mcp-config 파일이 실제로 쓰인다(이 테스트가 검증하는 상태).
    let ep = channel
        .provision(id, 0, true)
        .expect("provision ok")
        .expect("endpoint");
    assert_eq!(registry.live_token_count(), 1, "provision 후 산 토큰 1개");
    // ADR-0099: config_path 는 Option — MCP-capable(true) → Some(실파일).
    let cfg = ep
        .config_path
        .clone()
        .expect("MCP-capable → config_path Some");
    assert!(cfg.exists(), "provision 이 실제 config 파일을 씀");

    // ProvisionGuard::drop 이 부르는 것과 동일한 revoke — 발급된 토큰/config 를 회수한다.
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

// ── FIX 4(유지): kill 이 pump join 전에(즉시) 토큰을 폐기 ─────────────────────────────────
// kill_agent 는 blocking session.kill(최대 5s join) **전에** control.revoke 를 부른다(revoke-before-kill).
// ★backend 무관★: kill_agent 는 세션 backend 와 무관하게 revoke 를 부르므로, 셸 세션으로 spawn 하되
//   registry 에 (id, epoch=0) 토큰을 직접 심어 "provision 된 claude" 를 모사한다. kill_agent 반환 직후
//   그 토큰이 폐기돼 있어야 한다(그 5s 창 동안 토큰이 유효하지 않았음 — reaper backstop 대기 불요).
#[tokio::test]
async fn kill_revokes_token_before_pump_join() {
    let (manager, registry, data_dir, handle) = make_manager_with_control("kill-revoke").await;

    // 오래 사는 셸(즉시 종료 X) — kill 로만 끝나게.
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
    // "provision 된 claude" 모사: 이 세션의 (id, epoch=0) 에 산 토큰을 직접 심는다(셸은 provision 을
    //   안 부르므로 registry 는 비어 있음). kill_agent 가 이 토큰을 blocking join 전에 revoke 해야 한다.
    registry.issue(info.id, 0, "simulated-live-token".to_string());
    assert_eq!(registry.live_token_count(), 1, "심은 산 토큰 1개");

    // kill 을 부른다. ★FIX 4★: kill_agent 는 blocking session.kill(최대 5s join) **전에** revoke 를
    //   부른다 — kill_agent 반환 시점(=join 완료 후)엔 이미 폐기돼 있고, 그 5s 창 동안 토큰이 유효하지
    //   않았다. kill_agent 자체가 revoke 를 동기 완료하므로 반환 직후 산 토큰이 0 이어야 한다.
    manager.kill_agent(info.id).expect("kill ok");
    assert_eq!(
        registry.live_token_count(),
        0,
        "kill_agent 반환 직후 토큰이 이미 폐기(revoke-before-kill — reaper backstop 대기 불요)"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── FIX 5: 부팅 스윕이 stale mcp-config 파일을 청소 ────────────────────────────────────
#[test]
fn boot_sweep_removes_stale_configs() {
    let data_dir = std::env::temp_dir().join(format!("engram-mcp-sweep-{}", AgentId::new_v4()));
    let sub = data_dir.join("mcp-config");
    std::fs::create_dir_all(&sub).expect("mkdir");

    // 이전 데몬이 남긴 것처럼 파일 몇 개를 심는다(dead credential).
    let f1 = sub.join(format!("{}-0.json", AgentId::new_v4()));
    let f2 = sub.join(format!("{}-3.json", AgentId::new_v4()));
    std::fs::write(&f1, "{\"stale\":1}").unwrap();
    std::fs::write(&f2, "{\"stale\":2}").unwrap();
    assert!(f1.exists() && f2.exists(), "사전 stale 파일 존재");

    // 부팅 스윕 → 전부 청소.
    mcp_config::sweep_stale_configs(&data_dir);
    assert!(!f1.exists(), "스윕 후 stale 파일 1 삭제");
    assert!(!f2.exists(), "스윕 후 stale 파일 2 삭제");

    // 디렉토리 부재(첫 부팅) no-op 안전.
    let fresh = std::env::temp_dir().join(format!("engram-mcp-sweep-none-{}", AgentId::new_v4()));
    mcp_config::sweep_stale_configs(&fresh); // panic 없이 통과해야.

    let _ = std::fs::remove_dir_all(&data_dir);
}
