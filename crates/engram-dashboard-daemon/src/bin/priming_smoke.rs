//! priming-smoke — ADR-0092 프라이밍 수용 baseline 스모크 드라이버(검증 전용 bin).
//!
//! ## 역할
//! 실 **primed** claude 에이전트 1개(stream-json, Fresh)를 스폰하고 — 프라이밍(#1 FilePrimingProvider +
//! #2 스폰 배선)이 `--append-system-prompt-file` 로 주입된 상태 — **자연스러운 1:1 팀원 메시지**를
//! 실 control 경로(handle_send → wrap_message → write_stdin_observed)로 보낸 뒤, 에이전트의 응답
//! 텍스트를 캡처해 **PRINT** 한다. 오케스트레이터가 그 응답을 읽고 수용 여부를 **정성 판정**한다
//! (pass/fail 단언 아님 — ADR-0092 수용 판정은 qualitative).
//!
//! ## 핵심 불변식(ADR-0092)
//! - **required-features = ["test-harness"]** — 운영/릴리즈 빌드는 이 bin 을 컴파일하지 않는다.
//! - **프라이밍은 실물 파일에서** — 하드코딩 금지. FilePrimingProvider 가 `prompts/agent-priming.md` 를
//!   (또는 ENGRAM_PRIMING_FILE override) 해석해 주입한다.
//! - **자연 메시지** — 코드워드/기억-보고 프로브가 아니라 진짜 팀원 메시지(수용 confound 회피, ADR-0092).
//! - **skip_no_claude loud-skip** — claude 부재/인증 실패면 요란하게 스킵(silent skip 금지).
// ADR-0092

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::preset::PresetRegistry;
use engram_dashboard_core::agent::profile::{
    AgentCommand, AgentProfile, ClaudeOutputFormat, ProfileRegistry, SpawnMode,
};
use engram_dashboard_core::agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, OutputEvent, OutputFrame, OutputPayload,
    OutputSink, SinkError, SinkId, StatusSink,
};
use engram_dashboard_core::persistence::{FilePresetStore, FileProfileStore};

use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, Entrance};
use engram_dashboard_daemon::control::mcp_server::{start_mcp_server, ManagerSlot};
use engram_dashboard_daemon::control::priming::{
    FilePrimingProvider, PrimingProvider, PrimingVariant,
};
use engram_dashboard_daemon::control::registry::{BoundIdentity, ControlRegistry};
use engram_dashboard_daemon::control::DaemonControlChannel;

/// 스폰 후 목록 등장 대기.
const SPAWN_APPEAR_TIMEOUT: Duration = Duration::from_secs(10);
/// 턴 종료(MessageDone) 대기 상한.
const TURN_WAIT_CAP: Duration = Duration::from_secs(180);

/// ★자연 1:1 팀원 메시지(ADR-0092 — 코드워드/기억-보고 아님)★: 동료가 코드 리뷰 결과를 알려주는
///   평범한 협업 메시지. primed 에이전트가 이걸 인젝션으로 격리하는지, 팀원 메시지로 자연 수용하는지를 본다.
const NATURAL_MESSAGE: &str = "모듈 auth 리뷰 끝냈어. 로그인 경로에 이슈 2개 발견 — 확인 부탁.";

/// 원과제(에이전트가 뭔가 작업 중이도록) — 자연스러운 협업 맥락을 만든다.
const TASK_PROMPT: &str =
    "너는 지금 auth 모듈 관련 작업을 맡고 있다. 시작 준비가 됐으면 한 줄로 알려줘.";

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    std::process::exit(rt.block_on(run()));
}

/// ★loud skip(saturation_pilot / control_send 이식)★: claude 스폰 불가면 요란하게 스킵(exit 0 이되
/// SKIPPED 라벨을 stdout+stderr 에 남긴다 — silent skip 금지).
fn skip_no_claude(reason: &str) -> i32 {
    let line =
        format!("SKIPPED [priming-smoke]: {reason} — 프라이밍 수용 실측 불가(claude 부재/인증).");
    println!("{line}");
    eprintln!("{line}");
    0
}

async fn run() -> i32 {
    // 프라이밍 provider — repo 의 prompts/agent-priming.md 를 해석한다. base = 이 크레이트 매니페스트
    //   기준 repo 루트(cargo run 은 크레이트 dir 이 cwd 일 수 있어 명시). ENGRAM_PRIMING_FILE override 존중.
    let repo_root = repo_root_from_manifest();
    let priming = FilePrimingProvider::new(repo_root.clone());
    // ADR-0099: 이 진단 bin 은 claude(MCP-capable) 를 스폰하므로 McpPrimary(both-teaching) 변형을 본다.
    let priming_path = priming.priming_file(PrimingVariant::McpPrimary);
    match &priming_path {
        Some(p) => eprintln!("[smoke] priming file = {}", p.display()),
        None => eprintln!(
            "[smoke] WARNING: 프라이밍 파일을 못 찾음(base={}) — 프라이밍 없이 스폰됨(수용 baseline 무의미)",
            repo_root.display()
        ),
    }

    // 배선(control_send.rs / saturation_pilot wire() 미러) — 실 FilePrimingProvider 를 채널에 끼운다.
    let registry = Arc::new(ControlRegistry::new());
    let slot = Arc::new(ManagerSlot::new());
    let handle = match start_mcp_server(registry.clone(), slot.clone()).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[smoke] MCP 서버 기동 실패: {e}");
            return 1;
        }
    };
    let url = handle.url.clone();
    let data_dir = std::env::temp_dir().join(format!("engram-priming-smoke-{}", AgentId::new_v4()));
    let workspace =
        std::env::temp_dir().join(format!("engram-priming-smoke-ws-{}", AgentId::new_v4()));
    let _ = std::fs::create_dir_all(&workspace);

    let priming_provider: Arc<dyn PrimingProvider> = Arc::new(FilePrimingProvider::new(repo_root));
    let control: Arc<dyn ControlChannel> = Arc::new(DaemonControlChannel::new(
        registry.clone(),
        url,
        data_dir.clone(),
        None,
        priming_provider,
    ));
    let sink: Arc<dyn StatusSink> = Arc::new(NoopStatus);
    let profile_dir =
        std::env::temp_dir().join(format!("engram-priming-smoke-prof-{}", AgentId::new_v4()));
    let preset_dir =
        std::env::temp_dir().join(format!("engram-priming-smoke-preset-{}", AgentId::new_v4()));
    let profiles = Arc::new(ProfileRegistry::new(Arc::new(FileProfileStore::new(
        profile_dir.clone(),
    ))));
    let presets = Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(
        preset_dir.clone(),
    ))));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            sessions_dir: None,
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = Arc::new(AgentManager::new_with_control(
        sink, profiles, presets, tracker, control,
    ));
    slot.set(manager.clone());

    // 모델 핀 — 인자 없으면 sonnet(빠르고 저렴, 파일럿과 동일 계열). 첫 인자로 override 가능.
    let model = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sonnet".to_string());

    // 실 primed claude 스폰(stream-json, Fresh).
    let profile = AgentProfile::new(
        format!("smoke-{}", &AgentId::new_v4().to_string()[..8]),
        AgentCommand::Claude {
            extra_args: vec!["--model".to_string(), model.clone()],
            output_format: ClaudeOutputFormat::StreamJson,
        },
        workspace.clone(),
        vec![],
        false,
    );
    let agent = match manager.spawn_agent(&profile, SpawnMode::Fresh) {
        Ok(info) => {
            let deadline = Instant::now() + SPAWN_APPEAR_TIMEOUT;
            let mut appeared = false;
            while Instant::now() < deadline {
                if manager.list_agents().iter().any(|a| a.id == info.id) {
                    appeared = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
            if !appeared {
                cleanup(
                    &manager,
                    None,
                    &data_dir,
                    &workspace,
                    &profile_dir,
                    &preset_dir,
                )
                .await;
                handle.shutdown().await;
                return skip_no_claude("스폰 후 에이전트가 목록에 안 나타남");
            }
            info
        }
        Err(e) => {
            cleanup(
                &manager,
                None,
                &data_dir,
                &workspace,
                &profile_dir,
                &preset_dir,
            )
            .await;
            handle.shutdown().await;
            return skip_no_claude(&format!("spawn_agent 실패: {e}"));
        }
    };
    eprintln!(
        "[smoke] spawned primed agent id={} model={}",
        agent.id, model
    );

    // 출력 관측 sink 부착.
    let obs = Arc::new(TurnObserver::new());
    let sink_id = manager.subscribe(agent.id, obs.clone()).ok();

    // Turn 1 = 원과제(협업 맥락 형성). 실패해도 계속 — 수용 관측이 목적.
    if !send_and_wait(&manager, agent.id, &obs, TASK_PROMPT) {
        eprintln!("[smoke] WARNING: task 턴이 응답 없이 타임아웃(계속 진행)");
    } else {
        eprintln!(
            "[smoke] --- task turn response ---\n{}\n--- end ---",
            obs.response_text().trim()
        );
    }

    // 발신자 신원 발급(팀원 A) — 자연 메시지의 from.
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, format!("smoke-sender-{sender}"));
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    // ★자연 1:1 메시지를 실 control 경로로 주입★: handle_send → wrap_message → write_stdin_observed.
    obs.begin_turn();
    let baseline = obs.done_snapshot();
    let cmd = ControlCommand {
        from,
        to: agent.id.to_string(), // 정확한 AgentId 로 지목(이름 충돌 회피).
        body: NATURAL_MESSAGE.to_string(),
    };
    let ack = handle_send(&manager, &registry, Entrance::Cli, cmd);
    eprintln!("[smoke] inject ACK = {}", ack.to_json());

    // 응답 대기(자연 수용이면 에이전트가 이 메시지에 팀원처럼 반응한다).
    let responded = obs.wait_turn_end(baseline, TURN_WAIT_CAP);
    let response = obs.response_text();

    println!("\n===== PRIMED AGENT RESPONSE TO NATURAL 1:1 MESSAGE =====");
    println!("[injected message] {NATURAL_MESSAGE}");
    println!("[responded within cap] {responded}");
    println!("[response text]\n{}", response.trim());
    println!("===== END RESPONSE (orchestrator judges acceptance qualitatively) =====\n");

    if let Some(id) = sink_id {
        let _ = manager.unsubscribe(agent.id, id);
    }
    cleanup(
        &manager,
        Some(agent.id),
        &data_dir,
        &workspace,
        &profile_dir,
        &preset_dir,
    )
    .await;
    handle.shutdown().await;
    0
}

/// 이 크레이트 매니페스트(`crates/engram-dashboard-daemon`)에서 두 단계 위로 올라간 repo 루트.
///   `prompts/agent-priming.md` 가 그 아래 산다. cargo run 의 cwd 가 어디든 결정적으로 repo 를 가리킨다.
fn repo_root_from_manifest() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")); // .../crates/engram-dashboard-daemon
    manifest
        .parent() // .../crates
        .and_then(|p| p.parent()) // .../engram-dashboard (repo 루트)
        .map(|p| p.to_path_buf())
        .unwrap_or(manifest)
}

/// 프롬프트를 유저 턴으로 보내고 이번 턴 종료(MessageDone)까지 대기. 응답 도달=true, 타임아웃=false.
fn send_and_wait(
    manager: &Arc<AgentManager>,
    id: AgentId,
    obs: &Arc<TurnObserver>,
    prompt: &str,
) -> bool {
    obs.begin_turn();
    let baseline = obs.done_snapshot();
    if manager.write_stdin_observed(id, prompt.as_bytes()).is_err() {
        return false;
    }
    obs.wait_turn_end(baseline, TURN_WAIT_CAP)
}

#[allow(clippy::too_many_arguments)]
async fn cleanup(
    manager: &Arc<AgentManager>,
    agent_id: Option<AgentId>,
    data_dir: &std::path::Path,
    workspace: &std::path::Path,
    profile_dir: &std::path::Path,
    preset_dir: &std::path::Path,
) {
    if let Some(id) = agent_id {
        let _ = manager.kill_agent(id);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !manager.list_agents().is_empty() {
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    let _ = std::fs::remove_dir_all(data_dir);
    let _ = std::fs::remove_dir_all(workspace);
    let _ = std::fs::remove_dir_all(profile_dir);
    let _ = std::fs::remove_dir_all(preset_dir);
}

struct NoopStatus;
impl StatusSink for NoopStatus {
    fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
    fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
}

/// 턴 관측기 — MessageDone 카운트(턴 종료 신호) + TextDelta 누적(응답 텍스트). saturation_pilot 축소판.
struct TurnObserver {
    id: SinkId,
    inner: Mutex<String>,
    done_count: AtomicU64,
    cv: Condvar,
}

impl TurnObserver {
    fn new() -> Self {
        Self {
            id: SinkId::new_v4(),
            inner: Mutex::new(String::new()),
            done_count: AtomicU64::new(0),
            cv: Condvar::new(),
        }
    }
    fn begin_turn(&self) {
        self.inner.lock().unwrap().clear();
    }
    fn done_snapshot(&self) -> u64 {
        self.done_count.load(Ordering::Acquire)
    }
    fn wait_turn_end(&self, baseline: u64, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut g = self.inner.lock().unwrap();
        loop {
            if self.done_count.load(Ordering::Acquire) > baseline {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let (ng, _to) = self.cv.wait_timeout(g, deadline - now).unwrap();
            g = ng;
        }
    }
    fn response_text(&self) -> String {
        self.inner.lock().unwrap().clone()
    }
}

impl OutputSink for TurnObserver {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        let OutputPayload::Event(ev) = frame.payload else {
            return Ok(());
        };
        match ev {
            OutputEvent::TextDelta { text, .. } => {
                self.inner.lock().unwrap().push_str(text);
            }
            OutputEvent::MessageDone { .. } => {
                self.done_count.fetch_add(1, Ordering::Release);
                self.cv.notify_all();
            }
            _ => {}
        }
        Ok(())
    }
    fn sink_id(&self) -> SinkId {
        self.id
    }
}
