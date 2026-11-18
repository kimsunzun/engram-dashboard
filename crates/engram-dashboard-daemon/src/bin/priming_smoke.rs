//! priming-smoke — ADR-0092 priming acceptance baseline smoke driver (verification-only bin).
//!
//! Sends a natural 1:1 teammate message to one real primed claude over the real control path and prints the reply.
//!
//! ★No pass/fail assertion★ — whether it was accepted is decided qualitatively by the orchestrator reading the reply (the ADR-0092 acceptance decision is qualitative).
// ADR-0092

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::persistence::{FilePresetStore, FileProfileStore};
use engram_dashboard_agent::preset::PresetRegistry;
use engram_dashboard_agent::profile::{
    AgentCommand, AgentOutputFormat, AgentProfile, ProfileRegistry, SpawnMode,
};
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, OutputEvent, OutputFrame, OutputPayload,
    OutputSink, SinkError, SinkId, StatusSink,
};

use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server_with_help, CommandTableSlot, ManagerSlot, MessagingSlot, RosterBroadcastSlot,
};
use engram_dashboard_daemon::control::priming::{FilePrimingProvider, PrimingProvider};
use engram_dashboard_daemon::control::registry::{BoundIdentity, ControlRegistry};
use engram_dashboard_daemon::control::DaemonControlChannel;
use engram_dashboard_daemon::messaging_host::messaging_for_manager;
use engram_dashboard_messaging::envelope::Entrance;

const SPAWN_APPEAR_TIMEOUT: Duration = Duration::from_secs(10);
const TURN_WAIT_CAP: Duration = Duration::from_secs(180);

/// ★A natural 1:1 teammate message (ADR-0092 — not a codeword/recall report)★: watches whether the primed agent isolates it as an injection or naturally accepts it as a teammate message.
const NATURAL_MESSAGE: &str =
    "Finished reviewing the auth module. Found 2 issues on the login path — please take a look.";

/// The original task keeping the agent busy — creates the collaboration context the natural message arrives in.
const TASK_PROMPT: &str =
    "You are now in charge of work on the auth module. When you are ready to start, tell me in one line.";

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    std::process::exit(rt.block_on(run()));
}

/// ★Loud skip★: if claude cannot be spawned, exit 0 but leave the SKIPPED label on both stdout and stderr (no silent skip).
fn skip_no_claude(reason: &str) -> i32 {
    let line =
        format!("SKIPPED [priming-smoke]: {reason} — priming acceptance cannot be measured (claude missing/auth).");
    println!("{line}");
    eprintln!("{line}");
    0
}

async fn run() -> i32 {
    let repo_root = repo_root_from_manifest();
    let priming = FilePrimingProvider::new(repo_root.clone());
    let priming_path = priming.priming_file();
    match &priming_path {
        Some(p) => eprintln!("[smoke] priming file = {}", p.display()),
        None => eprintln!(
            "[smoke] WARNING: priming file not found (base={}) — spawned without priming (the acceptance baseline is meaningless)",
            repo_root.display()
        ),
    }

    let registry = Arc::new(ControlRegistry::new());
    let slot = Arc::new(ManagerSlot::new());
    let messaging_slot = Arc::new(MessagingSlot::new());
    // ★No client to attach★ — the roster notification fanout slot is handed to the table empty (skipping notifications is normal here, ADR-0132). The table itself is filled — left empty, `/control/agent` would only give 503.
    let broadcast_slot = Arc::new(RosterBroadcastSlot::new());
    let command_slot = Arc::new(CommandTableSlot::new());
    // ★Hold the reaper★ — if dropped, the seat table closes and every later round trip is bounced.
    // ★Attach the stage-1 table to the same copy as the slot below — do not revert to `without_commands()`★: the decider's and the bus's tables would differ and `/control/call` would not recognize `agent.*` (production `lib.rs` assembles it this way).
    let (relay_bus, _relay_sweeper) = engram_dashboard_daemon::command_delivery::CommandBus::new(
        engram_dashboard_daemon::command_roster::CommandRoster::new(),
        engram_dashboard_daemon::command_delivery::CommandDeliveries::new(),
        Arc::new(
            engram_dashboard_daemon::control::commands::DaemonLocalCommands::new(
                command_slot.clone(),
            ),
        ),
    );
    let handle = match start_mcp_server_with_help(
        registry.clone(),
        slot.clone(),
        messaging_slot.clone(),
        command_slot.clone(),
        relay_bus,
        // ADR-0285: a real primed agent may type `engram help`; without the source, that help is bounced and the measurement polluted. Same anchor as the production assembly.
        engram_dashboard_daemon::control::help::HelpSource::from_install_root(),
    )
    .await
    {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[smoke] MCP server startup failed: {e}");
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
        engram_dashboard_daemon::control::mcp_config::McpDir::new(data_dir.join("mcp-config")),
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
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = Arc::new(AgentManager::new_with_control(
        sink, profiles, presets, tracker, control,
    ));
    slot.set(manager.clone());
    command_slot.set(Arc::new(
        engram_dashboard_daemon::control::commands::make_daemon_table(
            manager.clone(),
            broadcast_slot.clone(),
            Arc::new(engram_dashboard_daemon::control::commands::NoInputLeases),
            Arc::new(engram_dashboard_daemon::control::commands::NoUsageLimits),
        ),
    ));
    let messaging = Arc::new(messaging_for_manager(manager.clone(), registry.clone()));
    messaging_slot.set(messaging.clone());

    // Model pin — override with the first argument, default is sonnet (fast and cheap, same family as the pilot).
    let model = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sonnet".to_string());

    let profile = AgentProfile::new(
        format!("smoke-{}", &AgentId::new_v4().to_string()[..8]),
        AgentCommand::Claude {
            extra_args: vec!["--model".to_string(), model.clone()],
            output_format: AgentOutputFormat::StreamJson,
        },
        workspace.clone(),
        vec![],
        false,
    );
    let agent = match manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .map(|o| o.into_started())
    {
        Ok(None) => {
            return skip_no_claude(
                "spawn_agent: already starting (moot) — unreachable in this harness",
            )
        }
        Ok(Some(info)) => {
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
                return skip_no_claude("the agent did not appear in the list after spawn");
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
            return skip_no_claude(&format!("spawn_agent failed: {e}"));
        }
    };
    eprintln!(
        "[smoke] spawned primed agent id={} model={}",
        agent.id, model
    );

    let obs = Arc::new(TurnObserver::new());
    let sink_id = manager.subscribe(agent.id, obs.clone()).ok();

    if !send_and_wait(&manager, agent.id, &obs, TASK_PROMPT) {
        eprintln!("[smoke] WARNING: the task turn timed out with no reply (continuing)");
    } else {
        eprintln!(
            "[smoke] --- task turn response ---\n{}\n--- end ---",
            obs.response_text().trim()
        );
    }

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, format!("smoke-sender-{sender}"), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    obs.begin_turn();
    let baseline = obs.done_snapshot();
    let cmd = ControlCommand {
        from,
        to: vec![agent.id.to_string()], // avoid name collisions — target by id.
        body: NATURAL_MESSAGE.to_string(),
        contract: Default::default(),
    };
    let ack = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
    eprintln!("[smoke] inject ACK = {}", ack.to_json());

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

/// ★Does not use cwd★ — points to the repo deterministically from the manifest wherever cargo run's cwd is.
fn repo_root_from_manifest() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")); // .../crates/engram-dashboard-daemon
    manifest
        .parent() // .../crates
        .and_then(|p| p.parent()) // .../engram-dashboard (repo root)
        .map(|p| p.to_path_buf())
        .unwrap_or(manifest)
}

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
