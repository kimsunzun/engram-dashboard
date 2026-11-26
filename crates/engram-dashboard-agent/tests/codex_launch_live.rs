//! codex launch materials, live — real codex, real JSON turns (TRD S25 step 6 §7-3, scenes L1–L5), and the hook
//! components lane (ADR-0420, scenes H-a – H-d — no tokens, temporary `CODEX_HOME`s).
//!
//! ★Every item is `#[ignore]`★ — the codex lane runs it (`.claude/skill-bindings/qa.md`). L1 and L2 spend **2 real
//!   model turns** and create one thread in the user's real `~/.codex` (auth needed); the test prints its id and leaves
//!   it — deleting the user's codex data is the user's call. L3 and L4 spawn real codex without a model turn; L5 spends
//!   no tokens and uses a temporary `CODEX_HOME`.
//!
//! ★The manager is driven directly★ — a test [`LaunchKitSource`] writes a scripted kit (skills, roots, raw lines,
//!   instructions with a marker) into a temporary volatile folder, and the manager is built with a control endpoint so
//!   the MCP `-c` attachment is on the line too (B13). The endpoint's URL has no server behind it.
//! ★The stored instructions hash is the manager's own record★ — the first echo of a delivered turn records the kit's
//!   hash; L2 and L3 read it before each resume instead of writing it.
//!
//! ★The hook scenes★ (ADR-0420) — Engram's own hook output against codex's `hooks/list`: H-a/H-d through the manager,
//!   Engram's transport and `AppServerOpen` (the post-open check's verdicts, start and resume, `experimentalApi` off);
//!   H-b Engram's terminal lines on an app-server line; H-c a user-layer state fixture beside Engram's thread config.
//!   Every hook scene uses its own temporary `CODEX_HOME` with a provider that points at a closed local port, so a turn
//!   reaches no model.
//!
//! Run: `cargo test -p engram-dashboard-agent --test codex_launch_live -- --ignored --test-threads=1 --nocapture`
//!   (`--nocapture` shows the codex thread id L1 leaves in `~/.codex`).
// ADR-0337
// ADR-0420

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use uuid::Uuid;

use engram_dashboard_agent::backend;
use engram_dashboard_agent::backend::hooks::HookRef;
use engram_dashboard_agent::launch::LaunchBlocked;
use engram_dashboard_agent::launch::{
    backend_word, HookTrust, LaunchArtifacts, LaunchKit, LaunchKitSource, LaunchPlan, LaunchPorts,
    PrepareError, RawSetting, SkillRef, StdLaunchFs, CODEX_HOOKS_UNTRUSTED,
};
use engram_dashboard_agent::manager::{AgentManager, NoticeDelivery};
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
use engram_dashboard_platform::spawn::{prepare_tree_root, TreeRoot};

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
    /// The plan's hook components.
    hooks: Vec<HookRef>,
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
                source: "live.yaml".to_string(),
                line: None,
                value: json!(scene.raw_lines),
            }],
            hooks: scene.hooks,
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

/// The fields of every "codex launch materials applied" line (the §4-7 handshake log line), process-wide.
fn applied_lines() -> &'static Mutex<Vec<std::collections::BTreeMap<String, String>>> {
    static LINES: OnceLock<Mutex<Vec<std::collections::BTreeMap<String, String>>>> =
        OnceLock::new();
    LINES.get_or_init(|| Mutex::new(Vec::new()))
}

/// The fields of every "codex hooks checked" line (the post-open check's counts), process-wide.
fn hooks_checked_lines() -> &'static Mutex<Vec<std::collections::BTreeMap<String, String>>> {
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
            .is_some_and(|m| m.contains("codex launch materials applied"))
        {
            applied_lines().lock().unwrap().push(fields);
        } else if fields
            .get("message")
            .is_some_and(|m| m.contains("codex hooks checked"))
        {
            hooks_checked_lines().lock().unwrap().push(fields);
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
    applied_fields(last)
}

/// One §4-7 line as (experimental_api, extra_roots, resume_instructions, first_turn_context).
fn applied_fields(
    line: &std::collections::BTreeMap<String, String>,
) -> (String, String, String, String) {
    let field = |name: &str| line.get(name).cloned().unwrap_or_default();
    (
        field("experimental_api"),
        field("extra_roots"),
        field("resume_instructions"),
        field("first_turn_context"),
    )
}

/// The one line of `lines` written after `mark` (its length before an activation) — panics unless exactly one was
/// written, so an activation that skipped the step cannot pass on an earlier activation's line.
fn one_line_since(
    lines: &Mutex<Vec<std::collections::BTreeMap<String, String>>>,
    what: &str,
    scene: &str,
    mark: usize,
) -> std::collections::BTreeMap<String, String> {
    let lines = lines.lock().unwrap();
    let new = &lines[mark.min(lines.len())..];
    assert_eq!(
        new.len(),
        1,
        "{scene}: {} \"{what}\" lines since the activation began, not one: {new:?}",
        new.len()
    );
    new[0].clone()
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

/// The 「Default」 priming as shipped.
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
/// L2 — a resume with realistic instructions (the 「Default」 priming + a few KB, the marker at the end) and a stored hash
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
            hooks: Vec::new(),
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

    // ── L2 ── L1's first echo recorded L1's hash.
    let stored = profiles.get(id).expect("profile");
    assert_eq!(
        stored.last_applied_instructions_hash,
        Some(format!("l1-{n}")),
        "L2: L1's delivered first turn did not record its hash"
    );
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

    // ── L3 ── L2's first echo recorded L2's hash, which now equals the kit's.
    let stored = profiles.get(id).expect("profile");
    assert_eq!(
        stored.last_applied_instructions_hash,
        Some(format!("l2-{n}")),
        "L3: L2's delivered first turn did not record its hash"
    );
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

/// L4 — a terminal launch with a real kit (the 「Default」 priming and two raw lines, the second turning the startup
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
            hooks: Vec::new(),
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
    let mut command = Command::new("cmd.exe");
    command
        .raw_arg(&line)
        .current_dir(cwd)
        .env("CODEX_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let (mut child, tree) = spawn_tree(&mut command, "cmd.exe /c codex app-server");
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
    drop(stdin);
    end_tree(&tree, &mut child, "cmd.exe /c codex app-server");
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

// ── H-a … H-d — hook components against codex's hooks/list (no tokens) ───────────────────────────

/// A temporary `CODEX_HOME`'s `config.toml` head: a provider at a closed local port, so a turn reaches no model.
const DEAD_PROVIDER: &str = "model_provider = \"engram-dead\"\n\n[model_providers.engram-dead]\nname = \"engram-dead\"\nbase_url = \"http://127.0.0.1:9/v1\"\nwire_api = \"responses\"\n";

/// A raw codex settings hooks line the scenes fold beside the component (a `PermissionRequest` group).
const RAW_HOOKS_LINE: &str =
    "hooks.PermissionRequest=[{matcher='Bash',hooks=[{type='command',command='echo engram-live-raw'}]}]";

/// A hook component whose handlers cover every hash rule of ADR-0420: the `command_windows` alias and
/// `commandWindows`, `timeout: 0` → 1, the `SessionEnd` clamp and the `Interrupt` default, an async `SessionEnd` hashed
/// as configured, the matcher dropped for `UserPromptSubmit` / `Stop`, `additionalContextLimit` kept per event, dropped
/// on another and at its default, an unknown field, a `statusMessage`, and the placeholder. Every command only echoes.
fn hash_rule_hook(dir: &Path) -> HookRef {
    let Value::Object(definition) = json!({
        "SessionStart": [{"matcher": "startup", "hooks": [
            {"type": "command", "command": "echo engram-live-start ${CLAUDE_PLUGIN_ROOT}", "timeout": 0, "statusMessage": "starting"}
        ]}],
        "PreToolUse": [{"matcher": "Bash", "hooks": [
            {"type": "command", "command": "echo engram-live-pre", "commandWindows": "echo engram-live-pre-win", "additionalContextLimit": 100, "unknownField": 1}
        ]}],
        "PostToolUse": [{"hooks": [
            {"type": "command", "command": "echo engram-live-post", "additionalContextLimit": 2500}
        ]}],
        "UserPromptSubmit": [{"matcher": "dropped", "hooks": [
            {"type": "command", "command": "echo engram-live-prompt", "async": true}
        ]}],
        "Stop": [{"matcher": "dropped", "hooks": [
            {"type": "command", "command": "echo engram-live-stop", "additionalContextLimit": 7}
        ]}],
        "SessionEnd": [{"hooks": [
            {"type": "command", "command": "echo engram-live-end", "async": true, "timeout": 9}
        ]}],
        "Interrupt": [{"hooks": [{"type": "command", "command": "echo engram-live-interrupt"}]}],
        "SubagentStart": [{"hooks": [
            {"type": "command", "command": "echo engram-live-sub", "command_windows": "echo engram-live-sub-win"}
        ]}],
    }) else {
        unreachable!()
    };
    std::fs::create_dir_all(dir).expect("create the hook folder");
    HookRef {
        path: "live/hooks/rules".to_string(),
        dir: dir.to_path_buf(),
        definition,
    }
}

/// A real kit for `backend::materialize` — the hook component and the raw lines, nothing else (no instructions file,
/// no skills, so nothing is read or written).
fn hook_kit(command: &AgentCommand, hooks: Vec<HookRef>, raw_lines: &[&str]) -> LaunchKit {
    LaunchKit {
        plan: LaunchPlan {
            backend: backend_word(command),
            instructions: Vec::new(),
            skills: Vec::new(),
            raw_settings: vec![RawSetting {
                preset: "live.yaml".to_string(),
                source: "live.yaml".to_string(),
                line: None,
                value: json!(raw_lines),
            }],
            hooks,
            include_priming: false,
            notices: Vec::new(),
            real: true,
            folders: Vec::new(),
            extra_skill_roots: Vec::new(),
            stamp: None,
        },
        volatile_dir: None,
        instructions_file: None,
        skills: Vec::new(),
        instructions_hash: None,
        last_applied_instructions_hash: None,
    }
}

/// Engram's artifacts for `kit` in `output_format`, through the public dispatch.
fn hook_artifacts(
    output_format: AgentOutputFormat,
    kit: &LaunchKit,
    work: &Path,
) -> LaunchArtifacts {
    backend::materialize(&codex(output_format), kit, work, &StdLaunchFs).expect("materialize")
}

/// A raw JSON-RPC client on `codex app-server --stdio <args>`, spawned the way Engram wraps the CLI
/// (`console_command`), under `home` as `CODEX_HOME`. ★Its whole process tree ends on drop★ ([`end_tree`]) — the
/// wrapper's codex descendants would otherwise outlive it and race the scene root's removal.
struct RawAppServer {
    child: std::process::Child,
    tree: TreeRoot,
    stdin: std::process::ChildStdin,
    lines: std::sync::mpsc::Receiver<String>,
    next_id: i64,
}

impl RawAppServer {
    fn spawn(cwd: &Path, home: &Path, args: &[String]) -> Self {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};

        let mut all = vec!["app-server".to_string(), "--stdio".to_string()];
        all.extend(args.iter().cloned());
        let (program, args) = engram_dashboard_platform::shell::console_command("codex", all);
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(cwd)
            .env("CODEX_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let (mut child, tree) = spawn_tree(&mut command, "codex app-server");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
        let mut server = RawAppServer {
            child,
            tree,
            stdin,
            lines,
            next_id: 1,
        };
        server.request(
            "initialize",
            json!({"clientInfo": {"name": "engram-dashboard", "version": "0"}}),
        );
        server.notify("initialized");
        server
    }

    fn notify(&mut self, method: &str) {
        use std::io::Write;
        writeln!(self.stdin, "{}", json!({ "method": method })).expect("write");
        self.stdin.flush().expect("flush");
    }

    /// The `result` of one request — panics on an error answer or no answer in 60 s.
    fn request(&mut self, method: &str, params: Value) -> Value {
        use std::io::Write;
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({ "id": id, "method": method, "params": params })
        )
        .expect("write");
        self.stdin.flush().expect("flush");
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let text = self
                .lines
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("{method}: no answer within 60 s"));
            let Ok(message) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if message["id"] == id {
                if let Some(error) = message.get("error") {
                    panic!("{method}: {error}");
                }
                return message["result"].clone();
            }
        }
    }

    /// Every hook `hooks/list` lists for `params`, flattened over its entries.
    fn hooks(&mut self, params: Value) -> Vec<Value> {
        let answer = self.request("hooks/list", params);
        answer["data"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|entry| entry["hooks"].as_array().cloned().unwrap_or_default())
            .collect()
    }
}

impl Drop for RawAppServer {
    fn drop(&mut self) {
        end_tree(&self.tree, &mut self.child, "codex app-server");
    }
}

/// Spawns `command` as the root of a process tree (`platform::spawn`) and wakes it — a failure cuts what started
/// and panics.
fn spawn_tree(command: &mut std::process::Command, what: &str) -> (std::process::Child, TreeRoot) {
    prepare_tree_root(command);
    let mut child = command
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {what}: {e}"));
    let tree = match TreeRoot::attach(&child) {
        Ok(tree) => tree,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("tie {what} to a process tree: {e}");
        }
    };
    if let Err(e) = tree.start(&child) {
        end_tree(&tree, &mut child, what);
        panic!("wake {what}: {e}");
    }
    (child, tree)
}

/// Cuts `tree` and waits until the child and every member have exited, so the scene root's removal does not race a
/// codex descendant. A tree still alive after 10 s panics (only prints while already panicking).
fn end_tree(tree: &TreeRoot, child: &mut std::process::Child, what: &str) {
    let _ = tree.kill(child);
    let _ = child.wait();
    if !wait_until(Duration::from_secs(10), || tree.members_gone()) {
        let message = format!("{what}: its process tree outlived the kill for 10 s");
        if std::thread::panicking() {
            eprintln!("{message}");
        } else {
            panic!("{message}");
        }
    }
}

/// The listed hook of `key`, or a panic naming what was listed.
fn listed<'a>(hooks: &'a [Value], key: &str) -> &'a Value {
    hooks
        .iter()
        .find(|hook| hook["key"] == key)
        .unwrap_or_else(|| panic!("{key} is not listed: {hooks:#?}"))
}

/// ★Engram's hash is codex's★ — every trust entry is listed under its key with `currentHash` = Engram's, trusted and
/// enabled.
fn assert_trusted_as_written(scene: &str, expected: &[HookTrust], hooks: &[Value]) {
    assert!(!expected.is_empty(), "{scene}: nothing was written");
    for entry in expected {
        let hook = listed(hooks, &entry.key);
        assert_eq!(hook["currentHash"], entry.hash.as_str(), "{scene}: {hook}");
        assert_eq!(hook["trustStatus"], "trusted", "{scene}: {hook}");
        assert_eq!(hook["enabled"], true, "{scene}: {hook}");
        assert_eq!(hook["source"], "sessionFlags", "{scene}: {hook}");
    }
}

/// No post-open verdict among an activation's notices.
fn assert_no_hook_verdict(scene: &str, notices: &[engram_dashboard_agent::launch::Notice]) {
    let verdicts: Vec<_> = notices
        .iter()
        .filter(|n| n.code == CODEX_HOOKS_UNTRUSTED)
        .collect();
    assert!(verdicts.is_empty(), "{scene}: {verdicts:?}");
}

/// The marks one activation's own log lines are read after — (hooks checked, materials applied) lines so far.
fn log_marks() -> (usize, usize) {
    (
        hooks_checked_lines().lock().unwrap().len(),
        applied_lines().lock().unwrap().len(),
    )
}

/// The one "codex hooks checked" line written since `marks` as (expected, verdicts).
fn hooks_checked_since(scene: &str, marks: (usize, usize)) -> (String, String) {
    let line = one_line_since(hooks_checked_lines(), "codex hooks checked", scene, marks.0);
    let field = |name: &str| line.get(name).cloned().unwrap_or_default();
    (field("expected"), field("verdicts"))
}

/// The one §4-7 line written since `marks` ([`applied_fields`]).
fn applied_since(scene: &str, marks: (usize, usize)) -> (String, String, String, String) {
    applied_fields(&one_line_since(
        applied_lines(),
        "codex launch materials applied",
        scene,
        marks.1,
    ))
}

/// H-a + H-d — a codex JSON agent with the hash-rule component and a raw hooks line, through the manager, Engram's
/// transport and `AppServerOpen`: on start the post-open check (`hooks/list {threadId}`) finds every handler Engram
/// wrote trusted and enabled — no verdict notice, the check's line counts every handler and no verdict — and the
/// handshake runs without `experimentalApi` (H-d); one turn (to a closed port) records the thread; the resume sends the
/// same config and the check passes again.
#[test]
#[ignore = "real codex — no tokens, temporary CODEX_HOME"]
fn h_a_codex_json_hooks_are_trusted_as_written_on_start_and_resume() {
    capture_applied_lines();
    let root = scene_root("hooks-json");
    let work = root.join("work");
    let home = root.join("home");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), DEAD_PROVIDER).unwrap();
    let hook = hash_rule_hook(&root.join("library").join("rules"));
    let written = hook_artifacts(
        AgentOutputFormat::StreamJson,
        &hook_kit(
            &codex(AgentOutputFormat::StreamJson),
            vec![hook.clone()],
            &[RAW_HOOKS_LINE],
        ),
        &work,
    )
    .hook_trust
    .len();

    let kits = Arc::new(LiveKits {
        root: root.to_path_buf(),
        scene: Mutex::new(Scene {
            instructions: "You are a test agent.\n".to_string(),
            hash: "h-a".to_string(),
            skills: Vec::new(),
            extra_skill_roots: Vec::new(),
            folders: Vec::new(),
            raw_lines: vec![RAW_HOOKS_LINE],
            hooks: vec![hook],
        }),
    });
    let (manager, profiles) = manager_with(kits);
    let profile = AgentProfile::new(
        format!("live-hooks-{}", Uuid::new_v4().simple()),
        codex(AgentOutputFormat::StreamJson),
        work.clone(),
        vec![(
            "CODEX_HOME".to_string(),
            home.to_string_lossy().into_owned(),
        )],
        false,
    );
    let id = profile.id;
    profiles
        .try_upsert(profile.clone())
        .expect("save the profile");

    let marks = log_marks();
    let started = manager
        .activate_profile_reporting(&profile, SpawnMode::Fresh, NoticeDelivery::Delivered)
        .expect("H-a start");
    assert_no_hook_verdict("H-a start", &started.notices);
    assert_eq!(
        hooks_checked_since("H-a start", marks),
        (written.to_string(), "0".to_string()),
        "H-a start: the check's line"
    );
    assert_eq!(
        applied_since("H-d start", marks).0,
        "off",
        "H-d: experimentalApi"
    );

    let events = Events::new();
    manager
        .subscribe(id, Arc::new(events.clone()))
        .expect("subscribe");
    manager
        .write_stdin(id, b"hello", InputOrigin::User)
        .expect("the turn is accepted");
    assert!(
        wait_until(Duration::from_secs(90), || {
            profiles
                .get(id)
                .and_then(|p| p.backend_session_id)
                .is_some()
        }),
        "H-a: the thread id was not recorded: {}",
        events.debug_after(0)
    );
    kill_and_reap(&manager, id);

    let stored = profiles.get(id).expect("profile");
    let marks = log_marks();
    let resumed = manager
        .activate_profile_reporting(&stored, SpawnMode::Resume, NoticeDelivery::Delivered)
        .expect("H-a resume");
    assert_no_hook_verdict("H-a resume", &resumed.notices);
    assert_eq!(
        hooks_checked_since("H-a resume", marks),
        (written.to_string(), "0".to_string()),
        "H-a resume: the check's line"
    );
    assert_eq!(
        applied_since("H-d resume", marks).0,
        "off",
        "H-d: experimentalApi on resume"
    );
    kill_and_reap(&manager, id);
    manager.shutdown_all();
}

/// H-b — Engram's codex terminal lines (the hash-rule component and a raw hooks line) on an app-server line, through
/// the same wrapper Engram launches codex with: `hooks/list {cwds}` lists every handler under Engram's key with
/// Engram's hash, trusted and enabled — the terminal route reads as written.
#[test]
#[ignore = "real codex — no tokens, temporary CODEX_HOME"]
fn h_b_codex_terminal_hook_lines_are_trusted_as_written() {
    let root = scene_root("hooks-terminal");
    let work = root.join("work");
    let home = root.join("home");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), DEAD_PROVIDER).unwrap();
    let hook = hash_rule_hook(&root.join("library").join("rules"));
    let artifacts = hook_artifacts(
        AgentOutputFormat::Terminal,
        &hook_kit(
            &codex(AgentOutputFormat::Terminal),
            vec![hook],
            &[RAW_HOOKS_LINE],
        ),
        &work,
    );
    assert!(!artifacts.hook_args.is_empty());
    let mut server = RawAppServer::spawn(&root, &home, &artifacts.hook_args);
    let hooks = server.hooks(json!({ "cwds": [work] }));
    assert_trusted_as_written("H-b", &artifacts.hook_trust, &hooks);
}

/// H-c — a user-layer state fixture beside Engram's thread config (F1, measure-4 M-B): three user hooks trusted in the
/// user layer — H1 enabled, H2 and H3 `enabled = false` — and a codex settings asset carrying `{<H3> = {enabled = true}}`.
/// With Engram's config on `thread/start`: H1 stays trusted and enabled, H2 stays off, H3 is on (the carried entry keeps
/// its effect), and Engram's own handlers are trusted as written.
#[test]
#[ignore = "real codex — no tokens, temporary CODEX_HOME"]
fn h_c_a_user_layer_state_keeps_its_effect_beside_engrams_hooks() {
    let root = scene_root("hooks-user-layer");
    let work = root.join("work");
    let home = root.join("home");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let user_hooks = "\n[[hooks.SessionStart]]\n[[hooks.SessionStart.hooks]]\ntype = \"command\"\ncommand = \"echo engram-live-user-1\"\n\n[[hooks.Stop]]\n[[hooks.Stop.hooks]]\ntype = \"command\"\ncommand = \"echo engram-live-user-2\"\n\n[[hooks.PreCompact]]\n[[hooks.PreCompact.hooks]]\ntype = \"command\"\ncommand = \"echo engram-live-user-3\"\n";
    let config = home.join("config.toml");
    std::fs::write(&config, format!("{DEAD_PROVIDER}{user_hooks}")).unwrap();

    // The user hooks' keys and hashes, as codex lists them before any state.
    let (h1, h2, h3) = {
        let mut server = RawAppServer::spawn(&root, &home, &[]);
        let hooks = server.hooks(json!({ "cwds": [work] }));
        let user = |event: &str| -> (String, String) {
            let hook = hooks
                .iter()
                .find(|h| {
                    h["source"] == "user" && h["key"].as_str().is_some_and(|k| k.contains(event))
                })
                .unwrap_or_else(|| panic!("no user {event} hook: {hooks:#?}"));
            (
                hook["key"].as_str().unwrap().to_string(),
                hook["currentHash"].as_str().unwrap().to_string(),
            )
        };
        (
            user(":session_start:"),
            user(":stop:"),
            user(":pre_compact:"),
        )
    };
    let mut state = String::new();
    for ((key, hash), enabled) in [(&h1, None), (&h2, Some(false)), (&h3, Some(false))] {
        state.push_str(&format!(
            "\n[hooks.state.'{key}']\ntrusted_hash = \"{hash}\"\n"
        ));
        if let Some(enabled) = enabled {
            state.push_str(&format!("enabled = {enabled}\n"));
        }
    }
    std::fs::write(&config, format!("{DEAD_PROVIDER}{user_hooks}{state}")).unwrap();

    let carried = format!("hooks.state={{'{}'={{enabled=true}}}}", h3.0);
    let hook = hash_rule_hook(&root.join("library").join("rules"));
    let artifacts = hook_artifacts(
        AgentOutputFormat::StreamJson,
        &hook_kit(
            &codex(AgentOutputFormat::StreamJson),
            vec![hook],
            &[&carried],
        ),
        &work,
    );
    let hook_config = artifacts.hook_config.clone().expect("a thread config");
    assert_eq!(
        hook_config["state"][&h3.0],
        json!({ "enabled": true }),
        "the carried entry"
    );

    let mut server = RawAppServer::spawn(&root, &home, &[]);
    let opened = server.request(
        "thread/start",
        json!({
            "cwd": work,
            "approvalPolicy": "never",
            "sandbox": "read-only",
            "ephemeral": true,
            "config": { "hooks": hook_config },
        }),
    );
    let thread = opened["thread"]["id"]
        .as_str()
        .expect("a thread id")
        .to_string();
    let hooks = server.hooks(json!({ "threadId": thread }));
    for (key, enabled) in [(&h1.0, true), (&h2.0, false), (&h3.0, true)] {
        let hook = listed(&hooks, key);
        assert_eq!(hook["trustStatus"], "trusted", "H-c {key}: {hook}");
        assert_eq!(hook["enabled"], enabled, "H-c {key}: {hook}");
    }
    assert_trusted_as_written("H-c", &artifacts.hook_trust, &hooks);
}
