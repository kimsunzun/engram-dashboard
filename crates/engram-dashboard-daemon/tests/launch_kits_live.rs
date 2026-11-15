//! Real launch materials, live — the library service as the manager's launch-kit source, over the repo `builtin\`,
//! with real claude and codex (scenes L1–L4).
//!
//! ★Every scene is `#[ignore]`★ — the claude and codex lanes run them (`.claude/skill-bindings/qa.md`). L1, L2 and L4
//!   spend **one real claude turn each**; L3 spawns codex twice (JSON and terminal) and spends **one real model turn**
//!   on the JSON one. Auth needed.
//! ★The service is built here, not through the daemon assembly★ — the same values as production (the repo built-in
//!   folder, the production folder waits), but a temporary launch root, home and profile store under one scene root
//!   (removed on drop), so the launch writes nothing outside it and codex's own skill roots under the home are empty.
//!   L4 adds a temporary library root. A real MCP server and control channel stand behind the agents, so claude's
//!   engram tools are really attached.
//! ★The CLIs still keep their own records outside it★ — each claude turn leaves a session transcript under the real
//!   `~/.claude/projects/`, and L3's JSON turn leaves one thread in the real `~/.codex` (its id is printed). Removing
//!   them is the user's call.
//!
//! Run: `cargo test -p engram-dashboard-daemon --test launch_kits_live -- --ignored --test-threads=1 --nocapture`
//!   (`claude_` / `codex_` select one lane).
// ADR-0311
// ADR-0316
// ADR-0327
// ADR-0328
// ADR-0337
// ADR-0341

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use uuid::Uuid;

use engram_dashboard_agent::launch::{LaunchPorts, StdLaunchFs};
use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::persistence::FileProfileStore;
use engram_dashboard_agent::profile::{
    AgentCommand, AgentOutputFormat, AgentProfile, AttachedPreset, ProfileRegistry, SpawnMode,
};
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, InputOrigin, OutputEvent, OutputFrame,
    OutputPayload, OutputSink, SinkError, SinkId, StatusSink,
};
use engram_dashboard_asset_library::backends::AgentBackends;
use engram_dashboard_asset_library::folder::{
    BoundedIo, LaunchWaits, StdFolderOps, ThreadSpawn, IO_LIMIT, LIST_CAP,
};
use engram_dashboard_asset_library::launch::merge_instructions;
use engram_dashboard_asset_library::store::FsStore;
use engram_dashboard_asset_library::{LibraryService, LibraryValues};
use engram_dashboard_base::testing::wait_until;
use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server_without_help, CommandTableSlot, ManagerSlot, McpServerHandle, MessagingSlot,
};
use engram_dashboard_daemon::control::registry::ControlRegistry;
use engram_dashboard_daemon::control::DaemonControlChannel;

/// How long a real model turn may take before the scene gives up.
const TURN_DEADLINE: Duration = Duration::from_secs(240);

/// The question of every claude turn — what the stream-json `init` lists, asked of the model.
const TOOLS_QUESTION: &str = "Without calling any tool, list the names of every tool you have whose name starts with mcp__engram, comma-separated, and nothing else.";

// ── Harness ─────────────────────────────────────────────────────────────────────────────────

struct NoopStatus;
impl StatusSink for NoopStatus {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

/// Every event and byte of one agent, in ring order.
#[derive(Clone)]
struct Events {
    id: SinkId,
    seen: Arc<Mutex<Vec<OutputEvent>>>,
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl Events {
    fn new() -> Self {
        Events {
            id: Uuid::new_v4(),
            seen: Arc::new(Mutex::new(Vec::new())),
            bytes: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn len(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    /// A turn ended after event `from` — codex ends one with `TurnEnd`, claude with `MessageDone` (an interrupted
    /// claude turn with `TurnEnd`).
    fn turn_ended_after(&self, from: usize) -> bool {
        self.seen.lock().unwrap()[from..].iter().any(|e| {
            matches!(
                e,
                OutputEvent::TurnEnd { .. } | OutputEvent::MessageDone { .. }
            )
        })
    }

    fn text_after(&self, from: usize) -> String {
        self.seen.lock().unwrap()[from..]
            .iter()
            .filter_map(|e| match e {
                OutputEvent::TextDelta { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn debug_after(&self, from: usize) -> String {
        self.seen.lock().unwrap()[from..]
            .iter()
            .map(|e| format!("{e:?}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn saw_bytes(&self, needle: &[u8]) -> bool {
        self.bytes
            .lock()
            .unwrap()
            .windows(needle.len())
            .any(|w| w == needle)
    }
}

impl OutputSink for Events {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        match frame.payload {
            OutputPayload::Event(e) => self.seen.lock().unwrap().push(e.clone()),
            OutputPayload::Bytes(b) => self.bytes.lock().unwrap().extend_from_slice(b),
        }
        Ok(())
    }
    fn sink_id(&self) -> SinkId {
        self.id
    }
}

/// A temporary root for one scene, outside this repository. ★Removed on drop★ (directory links first, so a link's
/// target survives) — best effort; it only prints what it could not remove.
struct SceneRoot(PathBuf);

impl std::ops::Deref for SceneRoot {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for SceneRoot {
    fn drop(&mut self) {
        remove_root(&self.0);
    }
}

fn scene_root(tag: &str) -> SceneRoot {
    let root = std::env::temp_dir().join(format!("engram-kits-{tag}-{}", Uuid::new_v4().simple()));
    for folder in ["launch", "home", "store", "mcp"] {
        std::fs::create_dir_all(root.join(folder)).expect("create the scene root");
    }
    SceneRoot(root)
}

fn remove_root(root: &Path) {
    fn unlink(dir: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                if let Err(e) = std::fs::remove_dir(&path).or_else(|_| std::fs::remove_file(&path))
                {
                    eprintln!("could not remove the link {}: {e}", path.display());
                }
            } else if meta.is_dir() {
                unlink(&path);
            }
        }
    }
    unlink(root);
    if let Err(e) = std::fs::remove_dir_all(root) {
        eprintln!("could not remove {}: {e}", root.display());
    }
}

/// Every path under `dir` (relative), not following directory links.
fn listing(dir: &Path) -> BTreeSet<String> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeSet<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let kind = std::fs::symlink_metadata(&path)
                .expect("metadata")
                .file_type();
            if kind.is_symlink() {
                out.insert(format!("{rel} (link)"));
            } else if kind.is_dir() {
                out.insert(format!("{rel}/"));
                walk(base, &path, out);
            } else {
                out.insert(rel);
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(dir, dir, &mut out);
    out
}

fn put(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).expect("create the parent folder");
    std::fs::write(path, text).expect("write the file");
}

fn repo_builtin() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../builtin")
}

/// The 「기본」 instructions file as a launch writes it: the shipped priming, merged on its own.
fn merged_priming() -> String {
    let priming = std::fs::read_to_string(repo_builtin().join("instructions/priming.md"))
        .expect("read builtin/instructions/priming.md");
    merge_instructions(Some(&priming), &[])
}

/// One scene's manager: the library service (production values over the scene root; `library` = its library root)
/// plugged in as the launch-kit source, a real MCP server and control channel, a profile store in the scene root.
struct Live {
    manager: Arc<AgentManager>,
    profiles: Arc<ProfileRegistry>,
    control: Arc<ControlRegistry>,
    mcp: McpServerHandle,
}

async fn live(root: &Path, library: Option<PathBuf>) -> Live {
    let control_registry = Arc::new(ControlRegistry::new());
    let manager_slot = Arc::new(ManagerSlot::new());
    let (relay_bus, relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    // Held for the process — dropping it closes the seat table (`control_send.rs` `hold_for_the_process`).
    std::mem::forget(relay_sweeper);
    let mcp = start_mcp_server_without_help(
        control_registry.clone(),
        manager_slot.clone(),
        Arc::new(MessagingSlot::new()),
        Arc::new(CommandTableSlot::new()),
        relay_bus,
    )
    .await
    .expect("start the MCP server");
    let control: Arc<dyn ControlChannel> = Arc::new(DaemonControlChannel::new(
        control_registry.clone(),
        mcp.url.clone(),
        engram_dashboard_daemon::control::mcp_config::McpDir::new(root.join("mcp")),
        None,
        Arc::new(engram_dashboard_daemon::control::priming::NoopPrimingProvider),
    ));
    let profiles = Arc::new(ProfileRegistry::new(Arc::new(FileProfileStore::new(
        root.join("store"),
    ))));
    let library = Arc::new(LibraryService::new(
        LibraryValues {
            registry: profiles.clone(),
            backends: Arc::new(AgentBackends),
            root: library,
            builtin_dir: Some(repo_builtin()),
            launch_root: root.join("launch"),
            home: Some(root.join("home")),
            env: Arc::new(|name| std::env::var(name).ok()),
        },
        Arc::new(FsStore),
        BoundedIo::new(
            IO_LIMIT,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::new(StdFolderOps),
            Arc::new(ThreadSpawn),
        ),
    ));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = Arc::new(
        AgentManager::new_with_control(Arc::new(NoopStatus), profiles.clone(), tracker, control)
            .with_launch_ports(LaunchPorts {
                kits: library,
                fs: Arc::new(StdLaunchFs),
            }),
    );
    manager_slot.set(manager.clone());
    Live {
        manager,
        profiles,
        control: control_registry,
        mcp,
    }
}

impl Live {
    /// Saves `profile` and starts it fresh — the launch folder of that incarnation.
    fn start(&self, profile: &AgentProfile) -> (AgentInfo, Events) {
        self.profiles
            .try_upsert(profile.clone())
            .expect("save the profile");
        let info = self
            .manager
            .activate_profile(profile, SpawnMode::Fresh)
            .expect("activation");
        let events = Events::new();
        self.manager
            .subscribe(profile.id, Arc::new(events.clone()))
            .expect("subscribe");
        (info, events)
    }

    /// Sends one user turn and waits for its end — the events from before the send.
    fn one_turn(&self, id: AgentId, events: &Events, prompt: &str) -> usize {
        let from = events.len();
        self.manager
            .write_stdin(id, prompt.as_bytes(), InputOrigin::User)
            .expect("the turn is accepted");
        assert!(
            wait_until(TURN_DEADLINE, || events.turn_ended_after(from)),
            "the turn did not end within {TURN_DEADLINE:?}: {}",
            events.debug_after(from)
        );
        from
    }

    fn kill_and_reap(&self, id: AgentId) {
        self.manager.kill_agent(id).expect("kill_agent");
        assert!(
            wait_until(Duration::from_secs(20), || {
                !self.manager.list_agents().iter().any(|a| a.id == id)
            }),
            "the killed session was not reaped"
        );
    }

    async fn end(self) {
        let manager = self.manager.clone();
        tokio::task::spawn_blocking(move || manager.shutdown_all())
            .await
            .expect("shutdown_all");
        self.mcp.shutdown().await;
    }
}

fn agent(name: &str, command: AgentCommand, cwd: &Path) -> AgentProfile {
    AgentProfile::new(
        format!("{name}-{}", Uuid::new_v4().simple()),
        command,
        cwd.to_path_buf(),
        Vec::new(),
        false,
    )
}

fn claude() -> AgentCommand {
    AgentCommand::Claude {
        extra_args: Vec::new(),
        output_format: AgentOutputFormat::StreamJson,
    }
}

/// A codex agent. The terminal one turns the startup update check off (a pass-through `-c`), as the codex lane's
/// terminal scene does: the TUI's update modal would otherwise hold back the composer and its idle signal whenever
/// `~/.codex` knows of a newer version — an environment state, not the launch under test.
fn codex(output_format: AgentOutputFormat) -> AgentCommand {
    let extra_args = match output_format {
        AgentOutputFormat::Terminal => vec![
            "-c".to_string(),
            "check_for_update_on_startup=false".to_string(),
        ],
        AgentOutputFormat::StreamJson => Vec::new(),
    };
    AgentCommand::Codex {
        extra_args,
        output_format,
    }
}

/// The command line of the live `cmd.exe` whose line holds the full path `needle` — the whole path, so a `cmd.exe`
/// left by an aborted earlier run (another scene root) cannot answer.
#[cfg(windows)]
fn command_line_holding(needle: &Path) -> Option<String> {
    let path = needle.to_string_lossy().replace('\'', "''");
    let script = format!(
        "Get-CimInstance Win32_Process -Filter \"Name='cmd.exe'\" | Where-Object {{ $_.CommandLine -and $_.CommandLine.Contains('{path}') }} | Select-Object -First 1 -ExpandProperty CommandLine"
    );
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()?;
    let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!line.is_empty()).then_some(line)
}

/// The developer texts codex recorded for `thread` — every `input_text` of a `developer` message in the thread's
/// rollout (`<codex home>/sessions/…/rollout-…-<thread>.jsonl`; the home = `CODEX_HOME`, else `~/.codex`). ★The
/// instructions an app-server thread opens with land there verbatim, as one item★ — the observation point of what
/// `thread/start`'s `developerInstructions` carried, written by the real CLI. Codex writes the file from the first turn
/// on: a thread without a turn has none (both measured, codex-cli 0.167.0).
fn rollout_developer_texts(thread: &str) -> Vec<String> {
    fn find(dir: &Path, suffix: &str) -> Option<PathBuf> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = find(&path, suffix) {
                    return Some(found);
                }
            } else if path.to_string_lossy().ends_with(suffix) {
                return Some(path);
            }
        }
        None
    }
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| engram_dashboard_platform::env::home_dir().map(|home| home.join(".codex")))
        .expect("the codex home");
    let rollout = find(&home.join("sessions"), &format!("-{thread}.jsonl"))
        .unwrap_or_else(|| panic!("no rollout of thread {thread} under {}", home.display()));
    std::fs::read_to_string(&rollout)
        .expect("read the rollout")
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|record| {
            record["type"] == "response_item"
                && record["payload"]["type"] == "message"
                && record["payload"]["role"] == "developer"
        })
        .flat_map(|record| {
            record["payload"]["content"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter(|item| item["type"] == "input_text")
        .filter_map(|item| item["text"].as_str().map(str::to_owned))
        .collect()
}

/// One claude turn asking for the engram tools: the answer names them and the agent (the only live one — a kill
/// revokes its sessions) bound an MCP session with our server.
fn assert_engram_tools(live: &Live, id: AgentId, events: &Events, scene: &str) {
    let from = live.one_turn(id, events, TOOLS_QUESTION);
    let answer = events.text_after(from);
    println!("{scene} answer: {answer}");
    assert!(
        answer.contains("mcp__engram__"),
        "{scene}: the engram tools are not listed: {answer}"
    );
    assert!(
        live.control.bound_session_count() >= 1,
        "{scene}: claude bound no MCP session with our server"
    );
}

// ── L1 · L2 — claude: the 「기본」 materials, and the `allowedMcpServers` merge ─────────────────────

/// L1 — a fresh claude (stream-json) launch with real kits and no library root: the volatile folder holds the merged
///   priming as `instructions.md` and the merged built-in settings as `settings.json`; the command line loads them
///   (`--append-system-prompt-file <vol>\instructions.md` · `--system-prompt-snapshot off` · one
///   `--settings <vol>\settings.json`); one turn lists the engram MCP tools (the launch did not die on the snapshot
///   flag — K3).
/// L2 — the same in a working folder whose project `.claude/settings.json` holds `allowedMcpServers: []`: the engram
///   tools are still listed (the merged `--settings` layer allows our server).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real claude — 2 model turns"]
async fn claude_l1_l2_real_kits_load_the_builtin_materials() {
    let root = scene_root("claude");
    let live = live(&root, None).await;

    // ── L1 ──
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let profile = agent("live-claude", claude(), &work);
    let id = profile.id;
    let (info, events) = live.start(&profile);
    let vol = root.join("launch").join(format!("{id}-{}", info.epoch));
    assert_eq!(
        listing(&vol),
        BTreeSet::from(["instructions.md".to_string(), "settings.json".to_string()]),
        "L1: the volatile folder"
    );
    assert_eq!(
        std::fs::read_to_string(vol.join("instructions.md")).unwrap(),
        merged_priming(),
        "L1: the instructions file is not the merged priming"
    );
    let settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(vol.join("settings.json")).unwrap()).unwrap();
    assert_eq!(
        settings["allowedMcpServers"],
        serde_json::json!([{ "serverName": "engram" }]),
        "L1: {settings}"
    );
    assert_eq!(
        settings["env"]["DISABLE_AUTOUPDATER"], "1",
        "L1: {settings}"
    );
    #[cfg(windows)]
    {
        let line = command_line_holding(&vol).expect("L1: the claude command line was not found");
        println!("L1 command line: {line}");
        let instructions = vol.join("instructions.md").to_string_lossy().into_owned();
        let settings_file = vol.join("settings.json").to_string_lossy().into_owned();
        assert!(
            line.contains(&format!("--append-system-prompt-file {instructions}"))
                || line.contains(&format!("--append-system-prompt-file \"{instructions}\"")),
            "L1: {line}"
        );
        assert!(line.contains("--system-prompt-snapshot off"), "L1: {line}");
        assert_eq!(line.matches("--settings ").count(), 1, "L1: {line}");
        assert!(line.contains(&settings_file), "L1: {line}");
    }
    assert_engram_tools(&live, id, &events, "L1");
    live.kill_and_reap(id);
    assert!(
        wait_until(Duration::from_secs(10), || !vol.exists()),
        "L1: the volatile folder outlived the reap"
    );

    // ── L2 ──
    let narrowed = root.join("narrowed");
    put(
        &narrowed.join(".claude").join("settings.json"),
        "{ \"allowedMcpServers\": [] }\n",
    );
    let profile = agent("live-claude-narrowed", claude(), &narrowed);
    let id = profile.id;
    let (_info, events) = live.start(&profile);
    assert_engram_tools(&live, id, &events, "L2");
    live.kill_and_reap(id);

    live.end().await;
}

// ── L4 — claude: folder entries and a library skill ───────────────────────────────────────────

/// L4 — a library root with a preset that attaches a library skill and two folder entries: folder A holds a
///   `CLAUDE.md` and a rule, folder B a skill. The command line repeats `--add-dir` for both folders and carries the
///   plugin folder (the skill junction); one turn prints the markers of A's `CLAUDE.md` and rule and lists the skills
///   of the library and of folder B.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real claude — 1 model turn"]
async fn claude_l4_folder_entries_and_a_library_skill_load() {
    let root = scene_root("claude-folders");
    let n = Uuid::new_v4().simple().to_string()[..8].to_string();
    let (memory_marker, rule_marker) = (format!("MEMO-{n}"), format!("RULE-{n}"));
    let a = root.join("folder-a");
    let b = root.join("folder-b");
    put(
        &a.join("CLAUDE.md"),
        &format!("When asked for markers, print the memory marker {memory_marker}.\n"),
    );
    put(
        &a.join(".claude").join("rules").join("live.md"),
        &format!("When asked for markers, also print the rule marker {rule_marker}.\n"),
    );
    let skill = |dir: &Path, name: &str| {
        put(
            &dir.join("SKILL.md"),
            &format!("---\nname: {name}\ndescription: A live-test skill named {name}. It does nothing.\n---\n\nThis skill does nothing.\n"),
        );
    };
    skill(
        &b.join(".claude").join("skills").join("live-folder"),
        "live-folder",
    );
    let lib = root.join("lib");
    skill(&lib.join("skills").join("live-lib"), "live-lib");
    put(
        &lib.join("P.yaml"),
        &format!(
            "kind: preset\nversion: 1\nskills:\n  - skills/live-lib\nfolders:\n  - '{}'\n  - '{}'\n",
            a.display(),
            b.display()
        ),
    );
    let live = live(&root, Some(lib)).await;
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let mut profile = agent("live-claude-folders", claude(), &work);
    profile.presets = vec![AttachedPreset {
        path: "P.yaml".to_string(),
    }];
    let id = profile.id;
    let (info, events) = live.start(&profile);
    let vol = root.join("launch").join(format!("{id}-{}", info.epoch));
    #[cfg(windows)]
    {
        let line = command_line_holding(&vol).expect("L4: the claude command line was not found");
        println!("L4 command line: {line}");
        assert_eq!(line.matches("--add-dir ").count(), 2, "L4: {line}");
        assert!(line.contains("--plugin-dir "), "L4: {line}");
    }
    let from = live.one_turn(
        id,
        &events,
        "Without using a tool: print every marker your instructions ask you to print, then list the names of the skills whose names contain live-.",
    );
    let answer = events.text_after(from);
    println!("L4 answer: {answer}");
    live.kill_and_reap(id);
    live.end().await;

    assert!(answer.contains(&memory_marker), "L4: CLAUDE.md: {answer}");
    assert!(answer.contains(&rule_marker), "L4: the rule: {answer}");
    assert!(
        answer.contains("live-lib"),
        "L4: the library skill: {answer}"
    );
    assert!(
        answer.contains("live-folder"),
        "L4: the folder skill: {answer}"
    );
}

// ── L3 — codex JSON and terminal ──────────────────────────────────────────────────────────────

/// L3 — codex JSON and codex terminal, one fresh launch each with real kits and no library root: each comes up (a
///   JSON turn ends · the TUI's OSC 7501 idle signal), the volatile folder's `instructions.md` is the merged priming,
///   the instructions codex received are that text — JSON: a `developer` message of the thread's rollout
///   ([`rollout_developer_texts`]) · terminal: `-c developer_instructions=…` on the command line — and nothing appears
///   in the working folder. One model turn (JSON): codex writes the rollout from the first turn on.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real codex — 1 model turn, one thread left in ~/.codex"]
async fn codex_l3_real_kits_carry_the_builtin_priming() {
    let root = scene_root("codex");
    let live = live(&root, None).await;
    for (scene, folder, mode) in [
        ("L3 JSON", "work-json", AgentOutputFormat::StreamJson),
        ("L3 terminal", "work-terminal", AgentOutputFormat::Terminal),
    ] {
        let work = root.join(folder);
        std::fs::create_dir_all(&work).unwrap();
        let work_before = listing(&work);
        let profile = agent("live-codex", codex(mode), &work);
        let id = profile.id;
        let (info, events) = live.start(&profile);
        let vol = root.join("launch").join(format!("{id}-{}", info.epoch));
        assert_eq!(
            std::fs::read_to_string(vol.join("instructions.md")).ok(),
            Some(merged_priming()),
            "{scene}: the instructions file"
        );
        if mode == AgentOutputFormat::Terminal {
            let ready = wait_until(Duration::from_secs(90), || {
                events.saw_bytes(b"\x1b]7501;state=idle")
            });
            // `--cd` carries the working folder as the launch decided it — canonical (`working_folder`), which can
            //   differ from `temp_dir()`'s spelling (an 8.3 short name, another case).
            #[cfg(windows)]
            let line = command_line_holding(
                &dunce::canonicalize(&work).expect("canonicalize the working folder"),
            );
            live.kill_and_reap(id);
            let screen = String::from_utf8_lossy(&events.bytes.lock().unwrap()).into_owned();
            let tail: String = screen
                .chars()
                .rev()
                .take(1500)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            assert!(
                ready,
                "{scene}: the TUI did not come up (no OSC 7501 idle): {tail:?}"
            );
            #[cfg(windows)]
            {
                let line = line.expect("L3 terminal: the cmd.exe command line was not found");
                assert!(line.contains("developer_instructions="), "{scene}: {line}");
            }
        } else {
            live.one_turn(id, &events, "Reply with the single word OK.");
            let thread = live
                .profiles
                .get(id)
                .and_then(|p| p.backend_session_id)
                .expect("L3 JSON: the codex thread id was not recorded");
            println!("L3 JSON created codex thread {thread} in ~/.codex — removing it is the user's call");
            live.kill_and_reap(id);
            let developer = rollout_developer_texts(&thread.to_string());
            assert!(
                developer.contains(&merged_priming()),
                "{scene}: codex did not receive the merged priming as developer instructions — the developer texts \
                 begin: {:?}",
                developer
                    .iter()
                    .map(|text| text.chars().take(120).collect::<String>())
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(
            listing(&work),
            work_before,
            "{scene}: something was written in the working folder"
        );
        assert!(
            wait_until(Duration::from_secs(10), || !vol.exists()),
            "{scene}: the volatile folder outlived the reap"
        );
    }
    live.end().await;
}
