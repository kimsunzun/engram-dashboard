//! codex launch materials, live — real codex, real JSON turns (TRD S25 step 6 §7-3, scenes L1–L5).
//!
//! ★Every item is `#[ignore]`★ — the codex lane runs it (`.claude/skill-bindings/qa.md`). L1 and L2 spend **2 real
//!   model turns** and create one thread in the user's real `~/.codex` (auth needed); the test prints its id and leaves
//!   it — deleting the user's codex data is the user's call. L3 and L4 spawn real codex without a model turn; L5 spends
//!   no tokens and uses a temporary `CODEX_HOME`.
//!
//! ★The manager is driven directly★ — real kits cannot reach the app before step 8, so a test [`LaunchKitSource`]
//!   writes a real kit into a temporary volatile folder, and the manager is built with a control endpoint so the MCP
//!   `-c` attachment is on the line too (B13). The endpoint's URL has no server behind it.
//! ★The stored instructions hash is set by hand★ — step 8 (not landed) records it at the first-turn event; L2 and L3
//!   stand in for that record by writing it into the test registry before each resume.
//!
//! Run: `cargo test -p engram-dashboard-agent --test codex_launch_live -- --ignored --test-threads=1 --nocapture`
//!   (`--nocapture` shows the codex thread id L1 leaves in `~/.codex`).
// ADR-0337

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use uuid::Uuid;

use engram_dashboard_agent::launch::LaunchBlocked;
use engram_dashboard_agent::launch::{
    backend_word, LaunchKit, LaunchKitSource, LaunchPlan, LaunchPorts, PrepareError, RawSetting,
    SkillRef,
};
use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::persistence::FileProfileStore;
use engram_dashboard_agent::profile::{
    AgentCommand, AgentOutputFormat, AgentProfile, ProfileRegistry, SpawnMode,
};
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, ControlChannelNeeds, ControlEndpoint,
    InputOrigin, OutputEvent, OutputFrame, OutputPayload, OutputSink, ProvisionError, SinkError,
    SinkId, StatusSink, ToolGrant, TurnOutcome, MCP_SERVER_NAME,
};
use engram_dashboard_base::testing::wait_until;

/// The `-c` line every scene's kit carries (TRD §7-3 — the raw `-c` path is exercised too).
const EFFORT_LINE: &str = "model_reasoning_effort=\"low\"";

/// The second preset `-c` line of L4's kit — the codex TUI's startup update modal would otherwise hold back the
/// composer (and the OSC 7501 idle signal) whenever the user's `~/.codex` knows of a newer version.
const UPDATE_CHECK_OFF_LINE: &str = "check_for_update_on_startup=false";

/// How long a real model turn may take before the scene gives up.
const TURN_DEADLINE: Duration = Duration::from_secs(240);

// ── Harness ─────────────────────────────────────────────────────────────────────────────────

struct NoopStatus;
impl StatusSink for NoopStatus {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

/// A control channel that hands out an endpoint (so the MCP attachment is on the line) with no server behind it.
struct LiveControl;
impl ControlChannel for LiveControl {
    fn provision(
        &self,
        id: AgentId,
        epoch: u32,
        _needs: ControlChannelNeeds,
    ) -> Result<Option<ControlEndpoint>, ProvisionError> {
        Ok(Some(ControlEndpoint {
            agent_id: id,
            epoch,
            url: "http://127.0.0.1:9/mcp".to_string(),
            token: "live-test-token".to_string(),
            config_path: None,
            send_exe: None,
            mail_allowed: false,
            priming_file: None,
            settings_file: None,
            // Without an MCP grant for our server the codex backend leaves the `-c mcp_servers.…` line off.
            grants: vec![ToolGrant::Mcp {
                server: MCP_SERVER_NAME.to_string(),
                tool: "eg_send".to_string(),
            }],
        }))
    }
    fn revoke(&self, _id: AgentId, _epoch: u32) {}
}

/// What the next launch carries — the scenes change it between launches.
#[derive(Clone)]
struct Scene {
    instructions: String,
    hash: String,
    skills: Vec<SkillRef>,
    extra_skill_roots: Vec<PathBuf>,
    folders: Vec<PathBuf>,
    /// The `key=value` lines of the kit's one raw codex preset, in order.
    raw_lines: Vec<&'static str>,
}

/// A real kit writer: `prepare` writes the merged instructions file into `<root>/vol-<id>-<epoch>` and hands out the
/// scene's skills and roots. `release` leaves the folder — the scene root's removal ([`SceneRoot`]) takes it.
struct LiveKits {
    root: PathBuf,
    scene: Mutex<Scene>,
}

impl LiveKits {
    fn volatile_dir(&self, id: AgentId, epoch: u32) -> PathBuf {
        self.root.join(format!("vol-{id}-{epoch}"))
    }
}

impl LaunchKitSource for LiveKits {
    fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked> {
        let scene = self.scene.lock().unwrap().clone();
        Ok(LaunchPlan {
            backend: backend_word(&profile.command),
            instructions: Vec::new(),
            skills: scene.skills,
            raw_settings: vec![RawSetting {
                preset: "live.yaml".to_string(),
                value: json!(scene.raw_lines),
            }],
            include_priming: true,
            notices: Vec::new(),
            real: true,
            folders: scene.folders,
            extra_skill_roots: scene.extra_skill_roots,
            stamp: None,
        })
    }

    fn prepare(
        &self,
        plan: LaunchPlan,
        id: AgentId,
        epoch: u32,
    ) -> Result<LaunchKit, PrepareError> {
        let scene = self.scene.lock().unwrap().clone();
        let vol = self.volatile_dir(id, epoch);
        std::fs::create_dir_all(&vol).map_err(|e| PrepareError::Failed(e.to_string()))?;
        let file = vol.join("instructions.md");
        std::fs::write(&file, &scene.instructions)
            .map_err(|e| PrepareError::Failed(e.to_string()))?;
        Ok(LaunchKit {
            skills: plan.skills.clone(),
            plan,
            volatile_dir: Some(vol),
            instructions_file: Some(file),
            instructions_hash: Some(scene.hash),
            last_applied_instructions_hash: None,
        })
    }

    fn release(&self, _id: AgentId, _epoch: u32) {}
}

/// Every event of one agent, in ring order (a new subscriber gets the ring replayed first).
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

    fn turn_ends_after(&self, from: usize) -> usize {
        self.seen.lock().unwrap()[from..]
            .iter()
            .filter(|e| matches!(e, OutputEvent::TurnEnd { .. }))
            .count()
    }

    fn turn_ends_failed_after(&self, from: usize) -> usize {
        self.seen.lock().unwrap()[from..]
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    OutputEvent::TurnEnd {
                        outcome: TurnOutcome::Failed { .. },
                        ..
                    }
                )
            })
            .count()
    }

    /// The structured events (the user bubble among them) after event `from`, as debug text.
    fn structured_after(&self, from: usize) -> String {
        self.seen.lock().unwrap()[from..]
            .iter()
            .filter(|e| matches!(e, OutputEvent::Structured { .. }))
            .map(|e| format!("{e:?}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The assistant text streamed after event `from`, joined.
    fn text_after(&self, from: usize) -> String {
        self.seen.lock().unwrap()[from..]
            .iter()
            .filter_map(|e| match e {
                OutputEvent::TextDelta { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Every event before `until`, as debug text.
    fn debug_before(&self, until: usize) -> String {
        self.seen.lock().unwrap()[..until]
            .iter()
            .map(|e| format!("{e:?}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn debug_after(&self, from: usize) -> String {
        self.seen.lock().unwrap()[from..]
            .iter()
            .map(|e| format!("{e:?}"))
            .collect::<Vec<_>>()
            .join("\n")
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

/// The fields of every "codex 실행 재료 적용" line (the §4-7 handshake log line), process-wide.
fn applied_lines() -> &'static Mutex<Vec<std::collections::BTreeMap<String, String>>> {
    static LINES: OnceLock<Mutex<Vec<std::collections::BTreeMap<String, String>>>> =
        OnceLock::new();
    LINES.get_or_init(|| Mutex::new(Vec::new()))
}

/// Installs the process-wide subscriber that keeps the §4-7 log line — the line is written on the transport's writer
/// thread, so a thread-local subscriber would not see it.
fn capture_applied_lines() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        tracing::subscriber::set_global_default(AppliedCapture)
            .expect("no other global subscriber in this test binary");
    });
}

struct AppliedCapture;

struct Fields<'a>(&'a mut std::collections::BTreeMap<String, String>);
impl tracing::field::Visit for Fields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
}

impl tracing::Subscriber for AppliedCapture {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() <= tracing::Level::INFO
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut fields = std::collections::BTreeMap::new();
        event.record(&mut Fields(&mut fields));
        if fields
            .get("message")
            .is_some_and(|m| m.contains("codex 실행 재료 적용"))
        {
            applied_lines().lock().unwrap().push(fields);
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

fn applied(
    api: &str,
    roots: &str,
    resumed: &str,
    context: &str,
) -> (String, String, String, String) {
    (
        api.to_string(),
        roots.to_string(),
        resumed.to_string(),
        context.to_string(),
    )
}

/// The last §4-7 line as (experimental_api, extra_roots, resume_instructions, first_turn_context).
fn last_applied() -> (String, String, String, String) {
    let lines = applied_lines().lock().unwrap();
    let last = lines.last().expect("no §4-7 log line was written");
    let field = |name: &str| last.get(name).cloned().unwrap_or_default();
    (
        field("experimental_api"),
        field("extra_roots"),
        field("resume_instructions"),
        field("first_turn_context"),
    )
}

/// A temporary root for one scene, outside this repository — everything the scene writes (the profile store too)
/// lives under it. ★Removed on drop★, so a failed scene cleans up as well — best effort: after a panic a codex child
/// may still hold files and the removal then stops partway (it only prints). Declared before the manager, it drops
/// after the manager.
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
    let root = std::env::temp_dir().join(format!("engram-live-{tag}-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&root).expect("create the scene root");
    SceneRoot(root)
}

/// A skill folder with a `SKILL.md` that names it.
fn write_skill(dir: &Path, name: &str) {
    std::fs::create_dir_all(dir).expect("create the skill folder");
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: A live-test skill named {name}. It does nothing.\n---\n\nThis skill does nothing.\n"),
    )
    .expect("write SKILL.md");
}

/// Every path under `dir` (relative), not following directory links — a link is listed, its target is not.
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

/// Removes `root` — directory links first (removing a link never touches its target), then the rest.
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

/// The manager under test — its profile store lives in the scene root (`<root>/store`), so the root's removal takes it.
fn manager_with(kits: Arc<LiveKits>) -> (AgentManager, Arc<ProfileRegistry>) {
    let store = Arc::new(FileProfileStore::new(kits.root.join("store")));
    let profiles = Arc::new(ProfileRegistry::new(store));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = AgentManager::new_with_control(
        Arc::new(NoopStatus),
        profiles.clone(),
        tracker,
        Arc::new(LiveControl),
    )
    .with_launch_ports(LaunchPorts {
        kits,
        ..LaunchPorts::noop()
    });
    (manager, profiles)
}

fn codex(output_format: AgentOutputFormat) -> AgentCommand {
    AgentCommand::Codex {
        extra_args: Vec::new(),
        output_format,
    }
}

fn kill_and_reap(manager: &AgentManager, id: AgentId) {
    manager.kill_agent(id).expect("kill_agent");
    assert!(
        wait_until(Duration::from_secs(20), || {
            !manager.list_agents().iter().any(|a| a.id == id)
        }),
        "the killed session was not reaped"
    );
}

/// Sends one user turn and waits for its end — the events from before the send.
fn one_turn(manager: &AgentManager, id: AgentId, events: &Events, prompt: &str) -> usize {
    let from = events.len();
    manager
        .write_stdin(id, prompt.as_bytes(), InputOrigin::User)
        .expect("the turn is accepted");
    assert!(
        wait_until(TURN_DEADLINE, || events.turn_ends_after(from) > 0),
        "the turn did not end within {TURN_DEADLINE:?}: {}",
        events.debug_after(from)
    );
    from
}

/// The 「기본」 priming as shipped.
fn shipped_priming() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../builtin/instructions/priming.md"
    ))
    .expect("read builtin/instructions/priming.md")
}

// ── L1 · L2 · L3 — codex JSON: fresh, changed resume, unchanged resume ──────────────────────────

/// L1 — a fresh JSON launch with a real kit: one library skill (through the junction), one folder root, the
///   instructions with a marker. One turn names the skills and prints the marker; the working folder and the folder
///   entry are unchanged; the volatile folder holds only the instructions file and the junction (B1).
/// L2 — a resume with realistic instructions (the 「기본」 priming + a few KB, the marker at the end) and a stored hash
///   that differs: the §4-7 line shows opt-in on, two roots, resume instructions on, context on; the turn is not
///   rejected and prints the new marker; the L1 turn hydrates; the context does not show in the user bubble.
/// L3 — a resume with the stored hash equal to the kit's, no model turn: opt-in off, context off, resume instructions
///   on, the roots sent again.
#[test]
#[ignore = "real codex — 2 model turns, one thread left in ~/.codex"]
fn l1_l2_l3_codex_json_fresh_then_changed_then_unchanged_resume() {
    capture_applied_lines();
    let root = scene_root("json");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let library_skill = root.join("library").join("live-lib");
    write_skill(&library_skill, "live-lib");
    let folder = root.join("folder");
    let folder_root = folder.join(".agents").join("skills");
    write_skill(&folder_root.join("live-folder"), "live-folder");
    let n = Uuid::new_v4().simple().to_string()[..8].to_string();
    let l1_marker = format!("L1-DEV-{n}");
    let l2_marker = format!("L2-DEV-{n}");

    let kits = Arc::new(LiveKits {
        root: root.to_path_buf(),
        scene: Mutex::new(Scene {
            instructions: format!(
                "You are a test agent. When asked for the marker from your instructions, print it exactly: {l1_marker}\n"
            ),
            hash: format!("l1-{n}"),
            skills: vec![SkillRef {
                name: "live-lib".to_string(),
                dir: library_skill.clone(),
            }],
            extra_skill_roots: vec![folder_root.clone()],
            folders: vec![folder.clone()],
            raw_lines: vec![EFFORT_LINE],
        }),
    });
    let (manager, profiles) = manager_with(kits.clone());
    let profile = AgentProfile::new(
        format!("live-json-{n}"),
        codex(AgentOutputFormat::StreamJson),
        work.clone(),
        Vec::new(),
        false,
    );
    let id = profile.id;
    profiles
        .try_upsert(profile.clone())
        .expect("save the profile");
    let work_before = listing(&work);
    let folder_before = listing(&folder);

    // ── L1 ──
    let info = manager
        .activate_profile(&profile, SpawnMode::Fresh)
        .expect("L1 activation");
    let events = Events::new();
    manager
        .subscribe(id, Arc::new(events.clone()))
        .expect("subscribe");
    assert_eq!(
        last_applied(),
        applied("off", "2", "false", "false"),
        "L1 §4-7 line"
    );
    let vol = kits.volatile_dir(id, info.epoch);
    let expected_vol: BTreeSet<String> = [
        "instructions.md",
        "engram/",
        "engram/skills/",
        "engram/skills/live-lib (link)",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(listing(&vol), expected_vol, "B1: the volatile folder");
    let from = one_turn(
        &manager,
        id,
        &events,
        "Without using a tool, list the skills whose names start with `live-`, then print the marker from your instructions.",
    );
    let answer = events.text_after(from);
    println!("L1 answer: {answer}");
    // Printed before the asserts: the thread exists in `~/.codex` from this turn on, even if L1 fails below.
    if let Some(thread) = profiles.get(id).and_then(|p| p.backend_session_id) {
        println!("L1 created codex thread {thread} in ~/.codex — removing it is the user's call");
    }
    assert!(answer.contains("live-lib"), "L1: {answer}");
    assert!(answer.contains("live-folder"), "L1: {answer}");
    assert!(answer.contains(&l1_marker), "L1: {answer}");
    assert_eq!(
        listing(&work),
        work_before,
        "L1: the working folder changed"
    );
    assert_eq!(
        listing(&folder),
        folder_before,
        "L1: the folder entry changed"
    );
    kill_and_reap(&manager, id);
    let thread = profiles
        .get(id)
        .and_then(|p| p.backend_session_id)
        .expect("L1's thread id was not persisted");
    println!("L1 left codex thread {thread} in ~/.codex — removing it is the user's call");

    // ── L2 ── step 8 would have recorded L1's hash at the first echo; stand in for it.
    let mut stored = profiles.get(id).expect("profile");
    stored.last_applied_instructions_hash = Some(format!("l1-{n}"));
    profiles
        .try_upsert(stored.clone())
        .expect("store L1's hash");
    {
        let mut scene = kits.scene.lock().unwrap();
        let mut text = shipped_priming();
        text.push_str("\n\n## Live test padding\n\n");
        while text.len() < 6 * 1024 {
            text.push_str("This paragraph only makes the instructions a realistic size; it asks for nothing.\n");
        }
        text.push_str(&format!(
            "\nWhen asked for the marker from your current instructions, print it exactly: {l2_marker}\n"
        ));
        scene.instructions = text;
        scene.hash = format!("l2-{n}");
    }
    manager
        .activate_profile(&stored, SpawnMode::Resume)
        .expect("L2 activation");
    let events = Events::new();
    manager
        .subscribe(id, Arc::new(events.clone()))
        .expect("subscribe");
    assert_eq!(
        last_applied(),
        applied("on", "2", "true", "true"),
        "L2 §4-7 line"
    );
    let hydrated = events.len();
    assert!(
        events.debug_before(hydrated).contains(&l1_marker),
        "L2: the L1 turn did not hydrate with the opt-in on"
    );
    let from = one_turn(
        &manager,
        id,
        &events,
        "Print the marker from your current instructions.",
    );
    let after = events.debug_after(from);
    assert!(
        events.turn_ends_failed_after(from) == 0,
        "L2: the turn failed (rejected?): {after}"
    );
    let answer = events.text_after(from);
    println!("L2 answer: {answer}");
    assert!(answer.contains(&l2_marker), "L2: {answer}");
    assert!(
        !events
            .structured_after(from)
            .contains("The instructions for this agent changed"),
        "L2: the context reached the user bubble: {after}"
    );
    kill_and_reap(&manager, id);

    // ── L3 ── the stored hash now equals the kit's.
    let mut stored = profiles.get(id).expect("profile");
    stored.last_applied_instructions_hash = Some(format!("l2-{n}"));
    profiles
        .try_upsert(stored.clone())
        .expect("store L2's hash");
    manager
        .activate_profile(&stored, SpawnMode::Resume)
        .expect("L3 activation");
    assert_eq!(
        last_applied(),
        applied("off", "2", "true", "false"),
        "L3 §4-7 line"
    );
    kill_and_reap(&manager, id);

    manager.shutdown_all();
}

// ── L4 — codex terminal ─────────────────────────────────────────────────────────────────────

/// L4 — a terminal launch with a real kit (the 「기본」 priming and two raw lines, the second turning the startup
/// update check off) and a control endpoint, no model turn: the TUI comes up (the OSC 7501 idle signal codex 0.163.0
/// sends once the composer is up — the readiness signal of `backend_contract`), the real command line carries the
/// preset `-c` lines in order before the MCP and priming `-c`, and no new file appears in the working folder or the
/// volatile folder.
#[test]
#[ignore = "real codex TUI — no model turn"]
fn l4_codex_terminal_real_kit_comes_up() {
    let root = scene_root("terminal");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let kits = Arc::new(LiveKits {
        root: root.to_path_buf(),
        scene: Mutex::new(Scene {
            instructions: shipped_priming(),
            hash: "l4".to_string(),
            skills: Vec::new(),
            extra_skill_roots: Vec::new(),
            folders: Vec::new(),
            raw_lines: vec![EFFORT_LINE, UPDATE_CHECK_OFF_LINE],
        }),
    });
    let (manager, profiles) = manager_with(kits.clone());
    let profile = AgentProfile::new(
        format!("live-terminal-{}", Uuid::new_v4().simple()),
        codex(AgentOutputFormat::Terminal),
        work.clone(),
        Vec::new(),
        false,
    );
    let id = profile.id;
    profiles
        .try_upsert(profile.clone())
        .expect("save the profile");
    let work_before = listing(&work);

    let info = manager
        .activate_profile(&profile, SpawnMode::Fresh)
        .expect("L4 activation");
    let vol = kits.volatile_dir(id, info.epoch);
    let vol_before = listing(&vol);
    let events = Events::new();
    manager
        .subscribe(id, Arc::new(events.clone()))
        .expect("subscribe");
    let ready = wait_until(Duration::from_secs(90), || {
        events
            .bytes
            .lock()
            .unwrap()
            .windows(b"\x1b]7501;state=idle".len())
            .any(|w| w == b"\x1b]7501;state=idle")
    });
    #[cfg(windows)]
    let command_line = command_line_of_cmd_holding(&work);
    kill_and_reap(&manager, id);

    assert!(ready, "L4: the TUI did not come up (no OSC 7501 idle)");
    #[cfg(windows)]
    {
        let line = command_line.expect("L4: the cmd.exe command line was not found");
        println!("L4 command line: {line}");
        let at = |needle: &str| {
            line.find(needle)
                .unwrap_or_else(|| panic!("L4: `{needle}` is not on the line: {line}"))
        };
        let effort = at("model_reasoning_effort");
        let update_check = at("check_for_update_on_startup");
        let mcp = at("mcp_servers.");
        let priming = at("developer_instructions=");
        assert!(
            effort < update_check && update_check < mcp && mcp < priming,
            "L4: the order is wrong: {line}"
        );
    }
    assert_eq!(
        listing(&work),
        work_before,
        "L4: a file appeared in the working folder"
    );
    assert_eq!(
        listing(&vol),
        vol_before,
        "L4: a file appeared in the volatile folder"
    );
    assert_eq!(
        vol_before,
        BTreeSet::from(["instructions.md".to_string()]),
        "L4: the terminal translation wrote into the volatile folder"
    );

    manager.shutdown_all();
}

/// The command line of the live `cmd.exe` whose line holds `work` (the terminal argv's `--cd`).
#[cfg(windows)]
fn command_line_of_cmd_holding(work: &Path) -> Option<String> {
    let leaf = work
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())?;
    let script = format!(
        "Get-CimInstance Win32_Process -Filter \"Name='cmd.exe'\" | Where-Object {{ $_.CommandLine -like '*{leaf}*' }} | Select-Object -First 1 -ExpandProperty CommandLine"
    );
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()?;
    let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!line.is_empty()).then_some(line)
}

// ── L5 — the odd-`"` rule and the cmd.exe line (no tokens) ─────────────────────────────────

/// One argument as portable-pty writes it on a Windows command line.
// Behaviour mirrors portable-pty 0.8.1 src/cmdbuilder.rs `CommandBuilder::append_quoted` (MIT).
#[cfg(windows)]
fn pty_quoted(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\x0b', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut pending_backslashes = 0;
    for c in arg.chars() {
        if c == '\\' {
            pending_backslashes += 1;
            continue;
        }
        let written = if c == '"' {
            2 * pending_backslashes + 1
        } else {
            pending_backslashes
        };
        out.extend(std::iter::repeat_n('\\', written));
        out.push(c);
        pending_backslashes = 0;
    }
    out.extend(std::iter::repeat_n('\\', 2 * pending_backslashes));
    out.push('"');
    out
}

/// Runs `cmd.exe /c codex app-server --stdio <args>` with the pty-style command line, under a temporary
/// `CODEX_HOME`, and returns the `sessionFlags` layer of `config/read` — `None` when the app-server never answered.
/// ★cmd runs in `cwd` (the scene root)★ — an odd-`"` line leaves `<` / `>` outside quotes, so cmd may redirect into a
/// file there instead of in the test's working folder (this crate).
#[cfg(windows)]
fn session_flags(cwd: &Path, home: &Path, args: &[&str]) -> Option<Value> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut line = String::from("/c codex app-server --stdio");
    for arg in args {
        line.push(' ');
        line.push_str(&pty_quoted(arg));
    }
    let mut child = Command::new("cmd.exe")
        .raw_arg(&line)
        .current_dir(cwd)
        .env("CODEX_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn cmd.exe");
    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                return;
            }
        }
    });
    let mut send = |v: Value| writeln!(stdin, "{v}").and_then(|_| stdin.flush());
    let _ = send(
        json!({"id": 1, "method": "initialize", "params": {"clientInfo": {"name": "engram-dashboard", "version": "0"}}}),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut result = None;
    while let Ok(text) =
        rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
    {
        let Ok(msg) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if msg["id"] == 1 {
            let _ = send(json!({"method": "initialized"}));
            let _ =
                send(json!({"id": 2, "method": "config/read", "params": {"includeLayers": true}}));
        } else if msg["id"] == 2 {
            result = msg["result"]["layers"].as_array().and_then(|layers| {
                layers
                    .iter()
                    .find(|l| l["name"]["type"] == "sessionFlags")
                    .map(|l| l["config"].clone())
            });
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    result
}

/// L5 (i)/(ii) — a preset line with an **even** number of `"` before the priming argument that holds `<` and `>`:
/// the priming and the argument after it arrive intact; with an **odd** number they do not — why `cmdline` refuses
/// an odd count (C2). (iii) — the resolved `cmd.exe` program token stays within `PROGRAM_TOKEN_BOUND` (262), and a
/// line that fits cmd's 8191 can still fail once `codex.cmd` expands `%*`, while one `SHIM_MARGIN` (300) shorter runs.
#[cfg(windows)]
#[test]
#[ignore = "real codex — no tokens, temporary CODEX_HOME"]
fn l5_quote_balance_and_the_cmd_line() {
    let root = scene_root("l5");
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let priming = "developer_instructions='keep <this> and >that intact'";

    // (i) even
    let even = session_flags(
        &root,
        &home,
        &["-c", "model=\"o3\"", "-c", priming, "-c", "p0.z=1"],
    )
    .expect("L5 (i): the app-server did not answer");
    println!("L5 (i) sessionFlags: {even}");
    assert_eq!(even["model"], "o3", "L5 (i): {even}");
    assert_eq!(
        even["developer_instructions"], "keep <this> and >that intact",
        "L5 (i): {even}"
    );
    assert_eq!(
        even["p0"]["z"], 1,
        "L5 (i): a later argument was lost: {even}"
    );

    // (ii) odd — the contrast
    let odd = session_flags(
        &root,
        &home,
        &["-c", "model=\"o3", "-c", priming, "-c", "p0.z=1"],
    );
    println!("L5 (ii) sessionFlags: {odd:?}");
    let intact = odd.as_ref().is_some_and(|v| {
        v["developer_instructions"] == "keep <this> and >that intact" && v["p0"]["z"] == 1
    });
    assert!(
        !intact,
        "L5 (ii): an odd `\"` count did not disturb the line: {odd:?}"
    );

    // (iii) the program token portable-pty resolves through PATH, and the shim line.
    let cmd = std::env::var_os("PATH")
        .and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join("cmd.exe"))
                .find(|candidate| candidate.is_file())
        })
        .expect("cmd.exe on PATH");
    let token = cmd.as_os_str().encode_wide_len() + 2;
    println!("L5 (iii) program token {} = {token} units", cmd.display());
    assert!(
        token <= 262,
        "L5 (iii): the program token exceeds PROGRAM_TOKEN_BOUND"
    );

    let version_at = |total: usize| -> bool {
        use std::os::windows::process::CommandExt;
        // `"cmd.exe" /c codex -c p0.pad=<pad> --version` — std writes the program token quoted.
        let fixed = "\"cmd.exe\" /c codex -c p0.pad= --version".len();
        let pad = "A".repeat(total - fixed);
        let output = std::process::Command::new("cmd.exe")
            .raw_arg(format!("/c codex -c p0.pad={pad} --version"))
            .env("CODEX_HOME", &home)
            .output()
            .expect("run cmd.exe");
        String::from_utf8_lossy(&output.stdout).contains("codex-cli")
    };
    let near_the_limit = version_at(8150);
    let under_the_margin = version_at(8191 - 300);
    println!("L5 (iii) 8150 → {near_the_limit}, 7891 → {under_the_margin}");
    assert!(
        !near_the_limit,
        "L5 (iii): a line under 8191 survived the shim's %* expansion — re-measure SHIM_MARGIN"
    );
    assert!(
        under_the_margin,
        "L5 (iii): a line SHIM_MARGIN under 8191 failed"
    );
}

/// UTF-16 length of an OS string.
#[cfg(windows)]
trait WideLen {
    fn encode_wide_len(&self) -> usize;
}

#[cfg(windows)]
impl WideLen for std::ffi::OsStr {
    fn encode_wide_len(&self) -> usize {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().count()
    }
}
