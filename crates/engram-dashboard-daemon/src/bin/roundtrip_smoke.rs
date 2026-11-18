//! roundtrip-smoke — A→B→A round trip (reply round-trip) measurement driver (verification-only bin).
//!
//! ## What it measures
//! Spawns two real primed claudes (A = alice · B = bob, stream-json, Fresh) through the launch materials
//! (`LibraryService` over `--builtin-dir`) and, on top of the A→B receipt that priming-smoke proved (ADR-0092
//! decision 4 baseline — that bin is gone, its measurement carries on in this A→B half, ADR-0370), looks at two new
//! pieces only: ① does B call the **sending half** (MCP `eg_send` or the `engram mail send` CLI) **on its own**
//! ② does A accept that reply naturally. Two observation axes — mechanical = the registry `DeliveryObservation`
//! (from = B, to = A) plus the entrance B chose, qualitative = A's turn text. The orchestrator gives the final reading
//! from the stdout markers below.
//!
//! ## Not misattributing negative results
//! `B_SENT=false` is a valid experimental negative only when setup fully succeeded and both A and B are alive. Missing infrastructure · wrong argument combinations · process death end with different labels (SETUP-SKIP / SETUP-FAIL); mixing them pollutes the data.
//!
//! ## Known unverified · uncontrolled
//! - The call-site wiring of the gate deciders is not reached by unit tests — the gates are inside `run()`, which spawns 2 real agents, so the suite stays green even if a gate is turned off with `if false && …`. Only the pure deciders are covered; this applies to every gate in this file.
//! - `--seed-reply-by` only renders the `reply-by` attribute on the envelope — this one-shot run does not run the daemon's 60-second sweep, so overdue timeouts · notices never fire.
//! - A wrong duration notation is bounced by `validate_contract` only at seed send time, after both agents are spawned — classified SETUP-FAIL, but the spawn cost is spent.
//!
//! ## Flags (test-only knobs — not production switches)
//! - `--builtin-dir <dir>` — the built-in folder the launch reads (absolute as is, relative under the repo root).
//!   Unset = the repo `builtin\`. A priming variant is a separately copied folder with its own
//!   `instructions\priming.md` (ADR-0370); a no-priming run = a copy with an empty priming file. The library reads
//!   the folder once, when the service is built, so a variant is fixed for the run.
//! - `--model <name>` — default sonnet.
//! - `--disallow-mcp` — removes only the MCP `eg_send` grant. ★It cannot create CLI routing (measured: 6/6 still sent through MCP)★ — the spawn stays MCP-capable, so the daemon refuses that credential's CLI mail (ADR-0133). ★No mode measures the CLI entrance★ — canonical mail path is MCP, the CLI is a mirror (ADR-0209 decision 3). Keeping or discarding the knob awaits a user decision.
//! - `--seed-request` (+ `--seed-reply-by <dur>`, e.g. `5m`) — sends the seed A→B as a reply contract.
//! - `--b-task <text|@path>` — replaces B's original task prompt (`@` = file reference, relative from the repo root).
//! - Misuse of `--b-task` · `--seed-reply-by` (missing value · next token starts with `--` · blank) and `--seed-reply-by` without `--seed-request` bounce with exit 1 before spawn. Other knobs are lenient (unspecified = today's behavior).
//!
//! ## Exit labels (stdout and stderr — no silent skip)
//! - `SKIPPED` exit 0 — `claude --version` could not start or exited unsuccessfully (the claude CLI is absent) — asked
//!   once before any spawn (after the config gates), and again when an agent does not start.
//! - `SETUP-SKIP` exit 1 — infrastructure the case requires is missing (a CLI path but the control plane CLI is not built).
//! - `SETUP-FAIL` exit 1 — an argument error (before any spawn) · a launch that would not carry the built-in priming:
//!   a blocked launch check, any `builtin-unavailable` notice (a missing or unreadable priming file, or a partial
//!   `--builtin-dir` that would launch unprimed or without its settings — ADR-0361 stand-ins), or a plan without the
//!   priming · `claude --version` not exiting within `CLAUDE_PROBE_DEADLINE` (the CLI is there but misbehaves) · an
//!   agent that did not start while the claude CLI is there (a launch-kit prepare, store or folder failure)
//!   · an A/B output subscription failure · a failed B task turn · A or B process death · a refused seed ACK. An empty
//!   but present priming file is not a setup failure.
//! - Otherwise exit 0 — a valid negative is exit 0 too.
//!
//! ## stdout markers (the orchestrator parses them)
//! The `ROUNDTRIP CASE= B_SENT= ENTRANCE=` banner (`CASE` = the `--builtin-dir` value as given, `repo-builtin` when
//! unset) · `[model]` · `[priming]` (the effective priming file under `--builtin-dir`, which the run read) ·
//! `SEED_KIND=` · `B_TASK=` are printed **always, even when unset** — the signal that makes a silently ignored typo
//! visible (so "unset = byte-identical to today" is a claim about the spawn, priming and send logic, not about the
//! whole stdout). With `--seed-request` these follow:
//! - `REPLY_MATCHES_SEED` — whether a delivered (is_delivered) B→A record has `in_reply_to == seed msg_id` (strict).
//! - `REPLY_IN_REPLY_TO` — the matching record's value first, else the first delivered record's `in_reply_to`; tells "wrong/hallucinated id" (matches=false, value present) from "no in-reply-to" (none).
//! - `REPLY_POLL=matched|timeout|skipped-no-budget` · `REPLY_POLL_BUDGET_MS=<ms>` — reply-polling budget state. 0 = no budget (skipped) or under 1ms left (truncated); the label, not the number, is the source of truth for the skip decision.
//! `[delivery-census]` dumps all captured delivery records in arrival order with no axis filter.
//!
//! ## Running (the orchestrator runs it; this file is only compiled)
//! The control plane CLI binary must be built first — `cargo run` does not build another package's (`engram-dashboard-cli`) bin, and the harness looks for `engram` (Win: `.exe`) among its own exe siblings, so both must share the profile/target.
//! ```text
//! cargo build -p engram-dashboard-cli
//! cargo run   -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke -- <flags>
//! ```
//! ## Core invariants
//! - `required-features = ["test-harness"]` — production/release builds do not compile this bin.
//! - B's reply goes only through the real entrance — the harness never calls `handle_send` on B's behalf; that is what it verifies.
//! - **No experiment uses the CLI entrance** (ADR-0128 single channel · CLI-mirror priming withdrawn 2026-09-19) —
//!   the daemon refuses CLI mail from an MCP-capable spawn by its credential (ADR-0133), so a `--builtin-dir` whose
//!   priming teaches the CLI is refused before any spawn. That judgment reads the **effective priming file's text**,
//!   not the folder name.
// ADR-0092
// ADR-0370

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_agent::launch::{
    notice_text, LaunchKitSource, LaunchPorts, StdLaunchFs, BUILTIN_UNAVAILABLE,
};
use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::persistence::FileProfileStore;
use engram_dashboard_agent::profile::{
    AgentCommand, AgentOutputFormat, AgentProfile, ProfileRegistry, SpawnMode,
};
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, OutputEvent, OutputFrame, OutputPayload,
    OutputSink, SinkError, SinkId, StatusSink, CLI_EXE_NAME,
};

use engram_dashboard_asset_library::builtin::file_of;
use engram_dashboard_asset_library::path::BuiltinId;
use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, SendContract};
use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server_with_help, CommandTableSlot, ManagerSlot, MessagingSlot, RosterBroadcastSlot,
};
use engram_dashboard_daemon::control::priming::mentions_mail_cli_surface;
use engram_dashboard_daemon::control::registry::{BoundIdentity, ControlRegistry};
use engram_dashboard_daemon::control::DaemonControlChannel;
use engram_dashboard_daemon::messaging_host::messaging_for_manager;
use engram_dashboard_daemon::{build_library_service, LibraryHost, LibraryStart};
use engram_dashboard_messaging::envelope::{DeliveryObservation, DeliveryObserver, Entrance};
use engram_dashboard_platform::shell::console_command;
use engram_dashboard_platform::spawn::{prepare_tree_root, TreeRoot};

const SPAWN_APPEAR_TIMEOUT: Duration = Duration::from_secs(10);
/// How long `claude --version` may take before the probe cuts it and calls the CLI misbehaving — a cold node start of
/// the CLI takes a few seconds.
const CLAUDE_PROBE_DEADLINE: Duration = Duration::from_secs(10);
/// How long a cut probe child may take to end before it is left behind with a warning.
const PROBE_REAP_DEADLINE: Duration = Duration::from_secs(2);
const TURN_WAIT_CAP: Duration = Duration::from_secs(180);
/// B reply wait cap — exceeding it is not an error but a negative result (B did not send).
const REPLY_WAIT_CAP: Duration = Duration::from_secs(180);

/// B learns this name from the envelope and replies with `to=alice`.
const NAME_A: &str = "alice";
const NAME_B: &str = "bob";

/// The original task that sets up the "working teammate" context before the seed — without it, whether a reply is natural cannot be decided.
const TASK_PROMPT_B: &str =
    "You are currently working on the auth module (login/session). When you're ready to start, reply in one line.";

/// ★Seed A→B (ADR-0092 — a natural teammate question, not a mechanical "use tool X")★: an ordinary collaboration
///   question in which A asks B for status, so that handing the answer back to A is the natural reaction. How to send
///   (tool / CLI) is taught by the **priming variant**, not the body (default = the repo
///   `builtin\instructions\priming.md`, which teaches only eg_send since ADR-0126 decision 1).
const SEED_A_TO_B: &str =
    "Can you share the status of the auth module? If you're stuck anywhere on the login path, tell me what you need too.";

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    std::process::exit(rt.block_on(run()));
}

fn skip_no_claude(reason: &str) -> i32 {
    let line =
        format!("SKIPPED [roundtrip-smoke]: {reason} — the A→B→A round trip cannot be measured (claude missing/auth).");
    println!("{line}");
    eprintln!("{line}");
    0
}

fn setup_skip(reason: &str) -> i32 {
    let line = format!("SETUP-SKIP [roundtrip-smoke]: {reason}");
    println!("{line}");
    eprintln!("{line}");
    1
}

fn setup_fail(reason: &str) -> i32 {
    let line = format!("SETUP-FAIL [roundtrip-smoke]: {reason}");
    println!("{line}");
    eprintln!("{line}");
    1
}

fn reply_poll_label(matched: bool, poll_ran: bool) -> &'static str {
    if matched {
        "matched"
    } else if poll_ran {
        "timeout"
    } else {
        "skipped-no-budget"
    }
}

fn seed_reply_by_without_request_is_invalid(
    seed_request: bool,
    seed_reply_by: &Option<String>,
) -> bool {
    !seed_request && seed_reply_by.is_some()
}

/// Does no existence check — the caller judges by trying to read it.
fn resolve_b_task_file_path(value: &str, repo_root: &std::path::Path) -> Option<PathBuf> {
    let rel = value.strip_prefix('@')?;
    Some(under_root(rel, repo_root))
}

/// `value` as is when absolute, else joined under `repo_root`.
fn under_root(value: &str, repo_root: &Path) -> PathBuf {
    let p = PathBuf::from(value);
    if p.is_absolute() {
        p
    } else {
        repo_root.join(p)
    }
}

/// The built-in folder of this run: `--builtin-dir` resolved by [`under_root`], unset = the repo `builtin\`.
fn resolve_builtin_dir(selector: Option<&str>, repo_root: &Path) -> PathBuf {
    under_root(selector.unwrap_or("builtin"), repo_root)
}

/// What the run's library service starts with: `library.json` read from `<data_dir>\state` (none there — the root
/// stays unset; the run reads the built-ins only and runs no first start), the default place `<data_dir>\library`, a
/// run-only identity and a root generation base drawn as production draws it.
fn library_start(data_dir: &Path) -> LibraryStart {
    LibraryStart {
        config: engram_dashboard_asset_library::config::load(&data_dir.join("state")),
        default_root: data_dir.join("library"),
        daemon_id: uuid::Uuid::new_v4().to_string(),
        root_generation_base: engram_dashboard_daemon::root_generation_base(),
    }
}

/// The priming file under `builtin_dir` — the file the library reads it from.
fn priming_file(builtin_dir: &Path) -> PathBuf {
    file_of(BuiltinId::Priming)
        .split('/')
        .fold(builtin_dir.to_path_buf(), |dir, part| dir.join(part))
}

/// Whether the launch materials of `profiles` carry the built-in priming, checked before anything is spawned — `Err`
/// = why the run cannot pass as a primed one (a SETUP-FAIL reason).
///
/// `Err` when `kits` blocks any of `profiles`; when a plan carries a `builtin-unavailable` notice (a built-in the
/// library stood in for — ADR-0361: a missing or unreadable priming file, a missing default preset or settings); or
/// when a plan is not real or does not include the priming (a default preset that does not list `engram:/priming`
/// tells nothing). An empty priming file passes.
fn check_builtin(kits: &dyn LaunchKitSource, profiles: &[&AgentProfile]) -> Result<(), String> {
    for profile in profiles {
        let plan = kits
            .check(profile)
            .map_err(|blocked| format!("the launch check of {} failed: {blocked}", profile.name))?;
        let told: Vec<String> = plan
            .notices
            .iter()
            .filter(|notice| notice.code == BUILTIN_UNAVAILABLE)
            .map(notice_text)
            .collect();
        if !told.is_empty() {
            return Err(format!(
                "the launch of {} reports {BUILTIN_UNAVAILABLE}: {} — it would not run as a primed one",
                profile.name,
                told.join(" / ")
            ));
        }
        if !plan.real || !plan.include_priming {
            return Err(format!(
                "the launch of {} would not carry the built-in priming (real={}, include_priming={}) — it would not run as a primed one",
                profile.name, plan.real, plan.include_priming
            ));
        }
    }
    Ok(())
}

/// What `claude --version` says about the claude CLI, bounded by [`CLAUDE_PROBE_DEADLINE`].
fn claude_probe() -> ProbeVerdict {
    probe_verdict(probe(
        "claude",
        vec!["--version".to_string()],
        CLAUDE_PROBE_DEADLINE,
    ))
}

/// What a probe of the claude CLI means for the run.
#[derive(Debug, PartialEq, Eq)]
enum ProbeVerdict {
    Runs,
    /// The CLI could not start or exited unsuccessfully — the environment's (SKIPPED). Carries why.
    NoClaude(String),
    /// The CLI exists but misbehaves (no exit within the deadline), or the probe itself failed — SETUP-FAIL. Carries
    /// why.
    Broken(String),
}

/// How one probe ended.
#[derive(Debug)]
enum ProbeEnd {
    Exited {
        success: bool,
    },
    NotStarted(String),
    /// The child could not be tied to or woken as a tree root; it was ended.
    TreeFailed(String),
    /// Waiting on the child failed; the tree was ended.
    WaitFailed(String),
    /// No exit within `deadline`; the tree was ended. `reaped` = the child ended within [`PROBE_REAP_DEADLINE`] after.
    TimedOut {
        deadline: Duration,
        reaped: bool,
    },
}

fn probe_verdict(end: ProbeEnd) -> ProbeVerdict {
    match end {
        ProbeEnd::Exited { success: true } => ProbeVerdict::Runs,
        ProbeEnd::Exited { success: false } => {
            ProbeVerdict::NoClaude("`claude --version` exited unsuccessfully".to_string())
        }
        ProbeEnd::NotStarted(e) => {
            ProbeVerdict::NoClaude(format!("`claude --version` could not start: {e}"))
        }
        ProbeEnd::TreeFailed(e) => ProbeVerdict::Broken(format!(
            "the `claude --version` probe could not run as a tree root: {e}"
        )),
        ProbeEnd::WaitFailed(e) => {
            ProbeVerdict::Broken(format!("waiting on `claude --version` failed: {e}"))
        }
        ProbeEnd::TimedOut { deadline, .. } => ProbeVerdict::Broken(format!(
            "`claude --version` timed out after {deadline:?} and was ended — the CLI is there but does not answer"
        )),
    }
}

/// Runs `program args` through the console wrapping a launch uses, as the root of a process tree (ADR-0275 — the
/// usage probes do the same), and waits up to `deadline` for it to exit. On a timeout the whole tree is cut — never by
/// walking parent pids (`kill_tree`'s `/T`), which reaches unrelated trees through stale parent pids (ADR-0291).
// ADR-0275
// ADR-0291
fn probe(program: &str, args: Vec<String>, deadline: Duration) -> ProbeEnd {
    let (program, args) = console_command(program, args);
    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    prepare_tree_root(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => return ProbeEnd::NotStarted(e.to_string()),
    };
    let tree = match TreeRoot::attach(&child) {
        Ok(tree) => tree,
        Err(e) => {
            // The child is left suspended outside any tree (Windows) — it is cut alone.
            if let Err(kill) = child.kill() {
                eprintln!("[roundtrip] probe: could not kill the untied child: {kill}");
            }
            reap_probe(&mut child);
            return ProbeEnd::TreeFailed(e.to_string());
        }
    };
    if let Err(e) = tree.start(&child) {
        end_tree(&tree, &mut child);
        return ProbeEnd::TreeFailed(format!("could not wake it: {e}"));
    }
    let waited = wait_bounded(deadline, || {
        child.try_wait().map(|status| status.map(|s| s.success()))
    });
    match waited {
        Ok(Some(success)) => ProbeEnd::Exited { success },
        Ok(None) => ProbeEnd::TimedOut {
            deadline,
            reaped: end_tree(&tree, &mut child),
        },
        Err(e) => {
            end_tree(&tree, &mut child);
            ProbeEnd::WaitFailed(e.to_string())
        }
    }
}

/// Cuts the probe's tree and reaps the child — `true` = it ended within [`PROBE_REAP_DEADLINE`].
fn end_tree(tree: &TreeRoot, child: &mut std::process::Child) -> bool {
    if let Err(e) = tree.kill(child) {
        eprintln!("[roundtrip] probe: the tree kill failed (the child itself was cut): {e}");
    }
    reap_probe(child)
}

/// Reaps a cut probe child within [`PROBE_REAP_DEADLINE`] — a kill's `Ok` does not mean it ended (ADR-0291 decision
/// 13); one that does not end is left behind with a warning.
fn reap_probe(child: &mut std::process::Child) -> bool {
    let reaped = wait_bounded(PROBE_REAP_DEADLINE, || {
        child.try_wait().map(|status| status.map(|_| true))
    });
    match reaped {
        Ok(Some(_)) => true,
        Ok(None) => {
            eprintln!(
                "[roundtrip] probe: pid {} did not end within {PROBE_REAP_DEADLINE:?} after the kill — left behind",
                child.id()
            );
            false
        }
        Err(e) => {
            eprintln!(
                "[roundtrip] probe: could not reap pid {}: {e} — left behind",
                child.id()
            );
            false
        }
    }
}

/// Polls `poll` (`Some(success)` = the child exited) until it answers or `deadline` passes — `Ok(None)` = timed out.
fn wait_bounded(
    deadline: Duration,
    mut poll: impl FnMut() -> std::io::Result<Option<bool>>,
) -> std::io::Result<Option<bool>> {
    let until = Instant::now() + deadline;
    loop {
        if let Some(success) = poll()? {
            return Ok(Some(success));
        }
        if Instant::now() >= until {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// How a run that could not start an agent ends.
#[derive(Debug, PartialEq, Eq)]
enum SpawnFailure {
    /// The claude CLI does not run — the environment's (SKIPPED).
    NoClaude,
    /// Anything else — a launch-kit check or prepare, the profile store, the working folder, a misbehaving CLI
    /// (SETUP-FAIL).
    Setup,
}

/// `verdict` = [`claude_probe`] asked at the failure. Only a CLI that does not run at all is the environment's.
fn classify_spawn_failure(verdict: &ProbeVerdict) -> SpawnFailure {
    match verdict {
        ProbeVerdict::NoClaude(_) => SpawnFailure::NoClaude,
        ProbeVerdict::Runs | ProbeVerdict::Broken(_) => SpawnFailure::Setup,
    }
}

fn spawn_failure_exit(why: &str) -> i32 {
    let verdict = claude_probe();
    let why = match &verdict {
        ProbeVerdict::Runs => why.to_string(),
        ProbeVerdict::NoClaude(probe) | ProbeVerdict::Broken(probe) => format!("{why} — {probe}"),
    };
    match classify_spawn_failure(&verdict) {
        SpawnFailure::NoClaude => skip_no_claude(&why),
        SpawnFailure::Setup => setup_fail(&why),
    }
}

/// `*_error` fields only record the fact of misuse; `run()` decides whether and when to bounce (fail fast with SETUP-FAIL before spawning a real claude).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    builtin_dir: Option<String>,
    model: String,
    disallow_mcp: bool,
    seed_request: bool,
    seed_reply_by: Option<String>,
    seed_reply_by_error: Option<String>,
    b_task: Option<String>,
    b_task_error: Option<String>,
}

struct CapturingObserver {
    records: Mutex<Vec<DeliveryObservation>>,
}

impl CapturingObserver {
    fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
        }
    }
    fn record_count(&self) -> usize {
        self.records.lock().unwrap().len()
    }

    /// ★Baseline cut★: the observer is installed before B's original task turn; if B sends one message to A in that turn, the pre-seed record would be mistaken for "the reply" (false `B_SENT=true`). So scan only after the `record_count` taken right before the seed injection.
    fn find_delivery_after(
        &self,
        baseline: usize,
        from_id: AgentId,
        to_id: AgentId,
    ) -> Option<DeliveryObservation> {
        let recs = self.records.lock().unwrap();
        recs.get(baseline..)?
            .iter()
            .find(|r| r.from.peer_id == from_id && r.to_id == to_id)
            .cloned()
    }

    /// Reply content verification (REPLY_IN_REPLY_TO/REPLY_MATCHES_SEED) must not be first-wins, so this returns all records — an unrelated "ack" first would otherwise cause a false negative. The b_sent/entrance decisions use the first arrival (`find_delivery_after`).
    fn records_after(
        &self,
        baseline: usize,
        from_id: AgentId,
        to_id: AgentId,
    ) -> Vec<DeliveryObservation> {
        let recs = self.records.lock().unwrap();
        recs.get(baseline..)
            .map(|slice| {
                slice
                    .iter()
                    .filter(|r| r.from.peer_id == from_id && r.to_id == to_id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn all_records(&self) -> Vec<DeliveryObservation> {
        self.records.lock().unwrap().clone()
    }
}

impl DeliveryObserver for CapturingObserver {
    fn observe(&self, obs: DeliveryObservation) {
        self.records.lock().unwrap().push(obs);
    }
}

async fn run() -> i32 {
    let args = parse_args(std::env::args().skip(1));
    // The 3 argument guards run before priming resolution and MCP server startup so no real claude is spawned in vain. Value syntax errors come before meaning checks.
    if let Some(reason) = &args.seed_reply_by_error {
        return setup_fail(reason);
    }
    if seed_reply_by_without_request_is_invalid(args.seed_request, &args.seed_reply_by) {
        return setup_fail(
            "--seed-reply-by requires --seed-request (reply_by is only meaningful as the deadline of a request contract) — add --seed-request or drop --seed-reply-by",
        );
    }
    if let Some(reason) = &args.b_task_error {
        return setup_fail(reason);
    }

    let repo_root = repo_root_from_manifest();
    let builtin_dir = resolve_builtin_dir(args.builtin_dir.as_deref(), &repo_root);
    let case_label = args.builtin_dir.as_deref().unwrap_or("repo-builtin");
    // Nothing has been spawned yet, so a failure return has nothing to clean up.
    let (task_prompt_b, b_task_kind): (String, String) = match &args.b_task {
        None => (TASK_PROMPT_B.to_string(), "default".to_string()),
        Some(v) => match resolve_b_task_file_path(v, &repo_root) {
            Some(path) => match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let kind = format!("file:{}", path.display());
                    (content, kind)
                }
                Err(e) => {
                    return setup_fail(&format!(
                        "--b-task file read failed: {} : {e} — cannot proceed without B's original task prompt",
                        path.display()
                    ));
                }
            },
            None => {
                let kind = format!("inline({} bytes)", v.len());
                (v.clone(), kind)
            }
        },
    };
    let data_dir = std::env::temp_dir().join(format!("engram-roundtrip-{}", AgentId::new_v4()));
    let ws_a = std::env::temp_dir().join(format!("engram-roundtrip-ws-a-{}", AgentId::new_v4()));
    let ws_b = std::env::temp_dir().join(format!("engram-roundtrip-ws-b-{}", AgentId::new_v4()));
    let _ = std::fs::create_dir_all(&ws_a);
    let _ = std::fs::create_dir_all(&ws_b);
    let profile_dir =
        std::env::temp_dir().join(format!("engram-roundtrip-prof-{}", AgentId::new_v4()));
    let dirs = [&data_dir, &ws_a, &ws_b, &profile_dir];
    let profiles = Arc::new(ProfileRegistry::new(Arc::new(FileProfileStore::new(
        profile_dir.clone(),
    ))));
    let library = Arc::new(build_library_service(
        profiles.clone(),
        Some(builtin_dir.clone()),
        data_dir.join("launch"),
        LibraryHost::process(),
        library_start(&data_dir),
    ));
    let profile_a = agent_profile(NAME_A, &args.model, &ws_a);
    let profile_b = agent_profile(NAME_B, &args.model, &ws_b);
    // Until the MCP server starts, a failure has only these folders to remove.
    if let Err(reason) = check_builtin(library.as_ref(), &[&profile_a, &profile_b]) {
        remove_dirs(&dirs);
        return setup_fail(&format!("{reason} (case={case_label})"));
    }
    let priming_path = priming_file(&builtin_dir);
    // The text the launches merge in — read once, by the service, when it was built.
    let priming_text = library.builtin_priming_text().to_owned();
    eprintln!(
        "[roundtrip] priming = {} (case={case_label})",
        priming_path.display()
    );

    let send_exe = sibling_send_exe();
    match &send_exe {
        Some(p) => eprintln!("[roundtrip] {CLI_EXE_NAME} = {}", p.display()),
        None => {
            eprintln!(
                "[roundtrip] no {CLI_EXE_NAME} sibling binary — CLI entrance disabled (MCP only)."
            )
        }
    }
    // ★One broad signal (block when in doubt)★: every spawn is MCP-capable, so priming that touches the CLI surface is refused without exception. ★The paired 「requirement direction」 gate (`teaches_mail_cli`) does not exist★ — no mode requires CLI send teaching (ADR-0209 decision 3). To revive it, set up that mode first, or every run becomes a false SETUP-FAIL.
    let mentions_cli_surface = mentions_mail_cli_surface(&priming_text);
    if mentions_cli_surface {
        remove_dirs(&dirs);
        return setup_fail(&format!(
            "CLI-teaching priming on an MCP-capable spawn (case={case_label}): the daemon refuses CLI mail requests from an MCP-capable spawn by credential (ADR-0133), so the command B learned does not work — a channel taught but unusable = a send freeze (ADR-0099 measured ~6/7 not sent) would be misattributed as a normal negative. There is no mode that measures the CLI entrance — the canonical mail path is MCP and the CLI is a mirror (ADR-0209 decision 3). ※The decision in this prohibition direction looks **broadly** (`mentions_mail_cli_surface`) — it is caught if **even one** of a mail CLI call form (`engram mail …`/`$ENGRAM_CLI_EXE mail …`) · the env variable name itself · a CLI-only flag (`--to`·`--body`·`--body-stdin`·`--request`·`--reply-by`·`--reply-to`) appears. Every arm compares at **word boundaries** after the same normalization (lowercase · whitespace/newline collapse · markdown emphasis/escape symbol removal · HTML comment removal · zero-width character removal), so disarray at the level of case · whitespace/newlines · markdown emphasis · HTML comments · zero-width characters is folded, and plain words like `engram mailbox`·`--token` are not caught. **Shell grammar in general (variable rebinding · aliases) and sentence negation are not decided** — the `teaches_mail_cli` comment is the source of truth for that limit. A negative sentence (\"do not use that command\") is still caught in this prohibition direction by the **bare mention** of the variable (an intended fail-closed: refusing is safer than misattributing — though it is not a 'taught' decision). To turn it into an MCP-only file, remove the mention itself",
        ));
    }
    // ★This skip guards only the consistency of the harness's SETUP labels★, not "a CLI grant exists" — this mode's send grant is 0 whether or not the binary exists. Keeping or discarding the knob awaits a user decision.
    if args.disallow_mcp && send_exe.is_none() {
        remove_dirs(&dirs);
        return setup_skip(
            "--disallow-mcp: SETUP-SKIP because the control plane CLI sibling binary is missing. ※Note (ADR-0133) — this mode leaves the spawn MCP-capable, so if the binary is built the command runs but **the daemon refuses that request** (the CLI mail entrance of an MCP-capable credential is closed). Moreover, in the auto permission mode the grant is a NO-OP, so the agent sends through MCP anyway (measured 6/6). That is, building is only a procedure to get past this skip and does not create CLI routing — there is no mode that does that. To get past the skip: `cargo build -p engram-dashboard-cli`",
        );
    }
    // After the config gates (a config error keeps its label without claude), before any spawn: under the Windows `cmd.exe`
    //   wrapper a missing claude does not fail the spawn itself.
    match claude_probe() {
        ProbeVerdict::Runs => {}
        ProbeVerdict::NoClaude(why) => {
            remove_dirs(&dirs);
            return skip_no_claude(&why);
        }
        ProbeVerdict::Broken(why) => {
            remove_dirs(&dirs);
            return setup_fail(&why);
        }
    }
    // Set before the control channel exists, so both agents (A · B) are provisioned with the same grant set.
    if args.disallow_mcp {
        std::env::set_var("ENGRAM_DISALLOW_MCP_SEND", "1");
        eprintln!("[roundtrip] --disallow-mcp → MCP send grant removed (CLI-only measurement, ENGRAM_DISALLOW_MCP_SEND=1)");
    }
    let registry = Arc::new(ControlRegistry::new());
    let observer = Arc::new(CapturingObserver::new());
    registry.set_delivery_observer(observer.clone());

    let slot = Arc::new(ManagerSlot::new());
    // MessagingService late-injection slot — send/flush (filled after the manager is assembled).
    let messaging_slot = Arc::new(MessagingSlot::new());
    // ★No client to attach★ — the roster notification fanout slot is handed to the table empty (skipping notifications is normal here, ADR-0132). The table itself is filled — left empty, `/control/agent` would only give 503.
    let broadcast_slot = Arc::new(RosterBroadcastSlot::new());
    let command_slot = Arc::new(CommandTableSlot::new());
    // ★Hold on to the reaper★ — if dropped, the seat table closes and every later round trip is bounced.
    // ★Attach the stage-1 table to the same copy as the slot below — do not revert to `without_commands()`★: the decider's table and the bus's table would differ, and `/control/call` would not recognize `agent.*` (production `lib.rs` assembles it this way).
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
            eprintln!("[roundtrip] MCP server startup failed: {e}");
            remove_dirs(&dirs);
            return 1;
        }
    };
    let url = handle.url.clone();
    let control: Arc<dyn ControlChannel> = Arc::new(DaemonControlChannel::new(
        registry.clone(),
        url,
        engram_dashboard_daemon::control::mcp_config::McpDir::new(data_dir.join("mcp-config")),
        send_exe,
    ));
    let sink: Arc<dyn StatusSink> = Arc::new(NoopStatus);
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = Arc::new(
        AgentManager::new_with_control(sink, profiles, tracker, control).with_launch_ports(
            LaunchPorts {
                kits: library,
                fs: Arc::new(StdLaunchFs),
            },
        ),
    );
    slot.set(manager.clone());
    command_slot.set(Arc::new(
        engram_dashboard_daemon::control::commands::make_daemon_table(
            manager.clone(),
            broadcast_slot.clone(),
            Arc::new(engram_dashboard_daemon::control::commands::NoInputLeases),
            Arc::new(engram_dashboard_daemon::control::commands::NoUsageLimits),
        ),
    ));
    // MessagingService assembly (3-way send branch · flush), wrapping the manager as a DeliveryPort. No flush sink is wired (NoopStatus), so parking scenarios take only the direct path; the seed goes to a live recipient (delivered).
    let messaging = Arc::new(messaging_for_manager(manager.clone(), registry.clone()));
    messaging_slot.set(messaging.clone());

    // ── A·B spawn (both real primed claude, stream-json, Fresh) ─────────────────────────
    let agent_a = match spawn_named(&manager, &profile_a) {
        Ok(a) => a,
        Err(why) => {
            cleanup(&manager, &[], &dirs).await;
            handle.shutdown().await;
            return spawn_failure_exit(&why);
        }
    };
    let agent_b = match spawn_named(&manager, &profile_b) {
        Ok(b) => b,
        Err(why) => {
            cleanup(&manager, &[agent_a.id], &dirs).await;
            handle.shutdown().await;
            return spawn_failure_exit(&why);
        }
    };
    eprintln!(
        "[roundtrip] spawned A(alice)={} B(bob)={} model={}",
        agent_a.id, agent_b.id, args.model
    );

    let obs_a = Arc::new(TurnObserver::new());
    let obs_b = Arc::new(TurnObserver::new());
    let sink_a = manager.subscribe(agent_a.id, obs_a.clone()).ok();
    let sink_b = manager.subscribe(agent_b.id, obs_b.clone()).ok();

    macro_rules! fail_setup {
        ($reason:expr) => {{
            print_delivery_census(&observer);
            if let Some(id) = sink_a {
                let _ = manager.unsubscribe(agent_a.id, id);
            }
            if let Some(id) = sink_b {
                let _ = manager.unsubscribe(agent_b.id, id);
            }
            cleanup(&manager, &[agent_a.id, agent_b.id], &dirs).await;
            handle.shutdown().await;
            return setup_fail($reason);
        }};
    }

    if sink_a.is_none() {
        fail_setup!("A output subscribe failed (sink_a=None) — A turns cannot be observed, the qualitative result is meaningless (setup failure)");
    }
    if sink_b.is_none() {
        fail_setup!("B output subscribe failed (sink_b=None) — B turns cannot be observed, setup cannot be decided (setup failure)");
    }

    // ── 1) B original task turn (working teammate context) ────────────────────────────────────────────
    // ★Do not revert to warn-and-continue★: it would report `B_SENT=false` as a normal negative without the teammate context, mistaking a setup failure for an experiment result.
    if !send_and_wait(&manager, agent_b.id, &obs_b, &task_prompt_b) {
        if !is_agent_alive(&manager, agent_b.id) {
            fail_setup!("B exited during the original task turn (process death) — teammate context setup failed");
        }
        fail_setup!(
            "B original task turn gave no end signal within the cap — teammate context setup failed (not a valid negative)"
        );
    }
    eprintln!(
        "[roundtrip] --- B task turn ---\n{}\n--- end ---",
        obs_b.response_text().trim()
    );
    // Recheck that B is alive right before the seed — it may have died after the task turn.
    if !is_agent_alive(&manager, agent_b.id) {
        fail_setup!("B exited before the seed injection — setup failed");
    }

    // ── 2) seed A→B (real control path, from = A's real issued identity) ────────────────────────
    // ★from = token-derived (ADR-0086)★: A is a Fresh spawn (epoch 0) and provision already issued a token for that (id,0); the sender is this BoundIdentity, not a body string.
    let from_a = BoundIdentity {
        agent_id: agent_a.id,
        epoch: 0,
    };
    obs_a.begin_turn();
    let baseline_a = obs_a.done_snapshot();
    let reply_baseline = observer.record_count();
    obs_b.begin_turn();
    let baseline_b = obs_b.done_snapshot();

    let seed_contract = if args.seed_request {
        SendContract {
            request: true,
            reply_by: args.seed_reply_by.clone(),
            reply_to: None,
        }
    } else {
        SendContract::default()
    };
    let seed = ControlCommand {
        from: from_a,
        to: vec![NAME_B.to_string()],
        body: SEED_A_TO_B.to_string(),
        contract: seed_contract,
    };
    let ack = handle_send(&manager, &registry, &messaging, Entrance::Cli, seed);
    eprintln!("[roundtrip] seed A→B ACK = {}", ack.to_json());
    // B is a live recipient, so it must be accepted (delivered), not parked — `is_accepted` filters only bounces.
    if !ack.is_accepted() {
        fail_setup!(&format!(
            "seed A→B ACK was not accepted (bounced): {}",
            ack.to_json()
        ));
    }
    // The ACK JSON `id` is this seed's logical msg_id (spec §6) — the reference for the reply match (REPLY_MATCHES_SEED).
    let seed_msg_id: Option<String> = ack
        .to_json()
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    // ★ACK shape-drift guard — cannot fire today★ (`ControlResult::Ok { id: String, .. }`, ingress.rs). Kept because if the shape changed and a None came out silently, it would be misclassified as REPLY_POLL=skipped-no-budget although the real cause is a missing reference value.
    if args.seed_request && seed_msg_id.is_none() {
        fail_setup!(&format!(
            "seed ACK missing id — request judgment impossible (ACK={})",
            ack.to_json()
        ));
    }

    // ── 3) wait for B's reply through B's own send path (the harness does not call handle_send) ──────
    // ★REPLY_WAIT_CAP measured from here shares one wall clock with the following A turn wait (`wait_turn_end`, at most TURN_WAIT_CAP)★ (user decision) instead of a new timeout constant. If the A turn is long, the later `wait_for_matching_reply` budget becomes 0/negative and polling does not run.
    let reply_wait_started = Instant::now();
    let reply_obs = wait_for_reply(
        &observer,
        reply_baseline,
        agent_b.id,
        agent_a.id,
        REPLY_WAIT_CAP,
    );
    let b_sent = reply_obs.is_some();
    // ★Check A's liveness too — do not revert to a B-only gate★: if A dies, B's reply has no target, no observation appears while B is alive, and a B-only gate misclassifies that as a normal negative.
    if !b_sent && !is_agent_alive(&manager, agent_b.id) {
        fail_setup!("B exited while waiting for the reply (process death) — not a valid negative (setup failure)");
    }
    if !b_sent && !is_agent_alive(&manager, agent_a.id) {
        fail_setup!(
            "A exited while waiting for the reply (process death) — no target for B's reply to reach, not a valid negative (setup failure)"
        );
    }
    let entrance_label = match &reply_obs {
        Some(o) => entrance_str(o.entrance),
        None => "none",
    };
    // ── 4) wait for the text A produced while handling B's reply (qualitative observation) ───────────────────────────
    let a_responded = if b_sent {
        obs_a.wait_turn_end(baseline_a, TURN_WAIT_CAP)
    } else {
        // B did not send, so no A turn is expected — it already saw nothing during REPLY_WAIT_CAP; look without waiting.
        obs_a.done_snapshot() > baseline_a
    };
    let a_response = obs_a.response_text();

    // ★Wait for the real contract reply here too★: `wait_for_reply` stops at the first B→A record after the baseline, which may be an unrelated "ack". Only for seed-request — a plain seed has no `in_reply_to` to poll for, and the default path must not slow down.
    let mut reply_poll_ran = false;
    let mut reply_poll_budget_ms: u64 = 0;
    if args.seed_request {
        if let Some(sid) = seed_msg_id.as_deref() {
            let remaining = REPLY_WAIT_CAP.saturating_sub(reply_wait_started.elapsed());
            reply_poll_budget_ms = remaining.as_millis() as u64;
            if !remaining.is_zero() {
                reply_poll_ran = true;
                wait_for_matching_reply(
                    &observer,
                    reply_baseline,
                    agent_b.id,
                    agent_a.id,
                    sid,
                    remaining,
                );
            }
        }
    }

    // ── 5) structured stdout markers (for the orchestrator's decision) ────────────────────────────────
    println!("\n===== ROUNDTRIP CASE={case_label} B_SENT={b_sent} ENTRANCE={entrance_label} =====");
    println!("[model] {}", args.model);
    println!("[priming] {}", priming_path.display());
    println!("[seed A->B body] {SEED_A_TO_B}");
    let seed_kind = if args.seed_request {
        "request"
    } else {
        "plain"
    };
    println!("SEED_KIND={seed_kind}");
    println!("B_TASK={b_task_kind}");
    if args.seed_request {
        println!(
            "SEED_REPLY_BY={}",
            args.seed_reply_by.as_deref().unwrap_or("none")
        );
        // ★Only delivered (is_delivered) records count as evidence★ — a write-failed record did not arrive, even if its `in_reply_to` equals the seed id. If only such records remain, honestly give none/false.
        let after_seed = observer.records_after(reply_baseline, agent_b.id, agent_a.id);
        let matched = after_seed.iter().find(|r| {
            r.is_delivered()
                && seed_msg_id
                    .as_deref()
                    .is_some_and(|s| r.in_reply_to.as_deref() == Some(s))
        });
        let reply_matches_seed = matched.is_some();
        let first_reply_with_id = after_seed
            .iter()
            .find(|r| r.is_delivered() && r.in_reply_to.is_some());
        // ★Do not revert to unconditional first-wins★: a wrong id first and the right id later would give `REPLY_IN_REPLY_TO=<wrong-id>` with `REPLY_MATCHES_SEED=true`. Prefer the matching record so both markers derive from the same record.
        let reply_in_reply_to: Option<String> = match matched {
            Some(m) => m.in_reply_to.clone(),
            None => first_reply_with_id.and_then(|r| r.in_reply_to.clone()),
        };
        println!(
            "REPLY_IN_REPLY_TO={}",
            reply_in_reply_to.as_deref().unwrap_or("none")
        );
        println!("REPLY_MATCHES_SEED={reply_matches_seed}");
        let reply_poll = reply_poll_label(reply_matches_seed, reply_poll_ran);
        println!("REPLY_POLL={reply_poll}");
        println!("REPLY_POLL_BUDGET_MS={reply_poll_budget_ms}");
    }
    println!("[B sent reply to A] {b_sent}");
    println!("[B chosen entrance] {entrance_label}");
    if let Some(o) = &reply_obs {
        println!(
            "[B->A delivery] msg_id={} bytes={} to_epoch={:?}",
            o.msg_id, o.bytes_requested, o.to_epoch
        );
    }
    println!("[A responded within cap] {a_responded}");
    println!("[A full response text]\n{}", a_response.trim());
    // Diagnosis: B's response in its turn after the seed (unrelated to send routing — B_SENT=false with text = "answered but did not send it with send"; empty = "did not react to the seed").
    let b_turn_ended = obs_b.done_snapshot() > baseline_b;
    let b_seed_response = obs_b.response_text();
    println!("[B post-seed turn ended] {b_turn_ended}");
    println!("[B post-seed turn text]\n{}", b_seed_response.trim());
    print_delivery_census(&observer);
    println!("===== END ROUNDTRIP (orchestrator judges qualitatively) =====\n");

    // ── cleanup ──────────────────────────────────────────────────────────────────────
    if let Some(id) = sink_a {
        let _ = manager.unsubscribe(agent_a.id, id);
    }
    if let Some(id) = sink_b {
        let _ = manager.unsubscribe(agent_b.id, id);
    }
    cleanup(&manager, &[agent_a.id, agent_b.id], &dirs).await;
    handle.shutdown().await;
    0
}

/// Takes an `iter` to drop the std::env dependency (pure, so unit tests reach it).
/// A value that is another flag (starts with `--`) is not taken (FIX round-2 #7) — taking it would eat `--model` in
///   `--builtin-dir --model opus` as the folder and leave the model silently at its default.
fn parse_args(iter: impl Iterator<Item = String>) -> Args {
    let mut builtin_dir = None;
    let mut model = "sonnet".to_string();
    let mut disallow_mcp = false;
    let mut seed_request = false;
    let mut seed_reply_by = None;
    let mut seed_reply_by_error: Option<String> = None;
    let mut b_task = None;
    let mut b_task_error: Option<String> = None;
    let mut it = iter.peekable();
    while let Some(tok) = it.next() {
        match tok.as_str() {
            "--builtin-dir" => {
                if let Some(v) = take_flag_value(&mut it) {
                    builtin_dir = Some(v);
                }
            }
            "--model" => {
                if let Some(v) = take_flag_value(&mut it) {
                    model = v;
                }
            }
            "--disallow-mcp" => disallow_mcp = true,
            "--seed-request" => seed_request = true,
            "--seed-reply-by" => match it.peek() {
                None => {
                    seed_reply_by_error = Some(
                        "--seed-reply-by requires a value (duration text like '5m') but none was given."
                            .to_string(),
                    );
                }
                Some(next) if next.starts_with("--") => {
                    seed_reply_by_error = Some(format!(
                        "--seed-reply-by requires a value (duration text like '5m'); '{next}' looks like another flag, not a duration value."
                    ));
                }
                Some(_) => {
                    let v = it.next().expect("peeked Some");
                    if v.trim().is_empty() {
                        seed_reply_by_error = Some(
                            "--seed-reply-by value is empty or whitespace-only; provide a duration like '5m'."
                                .to_string(),
                        );
                    } else {
                        seed_reply_by = Some(v);
                    }
                }
            },
            "--b-task" => match it.peek() {
                None => {
                    b_task_error = Some(
                        "--b-task requires a value (text or @path) but none was given.".to_string(),
                    );
                }
                Some(next) if next.starts_with("--") => {
                    b_task_error = Some(format!(
                        "--b-task requires a value (text or @path); '{next}' looks like another flag, not a task value."
                    ));
                }
                Some(_) => {
                    let v = it.next().expect("peeked Some");
                    if v.trim().is_empty() {
                        b_task_error = Some(
                            "--b-task value is empty or whitespace-only; provide task text or an @path reference."
                                .to_string(),
                        );
                    } else {
                        b_task = Some(v);
                    }
                }
            },
            _ => {}
        }
    }
    Args {
        builtin_dir,
        model,
        disallow_mcp,
        seed_request,
        seed_reply_by,
        seed_reply_by_error,
        b_task,
        b_task_error,
    }
}

fn take_flag_value<I: Iterator<Item = String>>(it: &mut std::iter::Peekable<I>) -> Option<String> {
    match it.peek() {
        Some(next) if next.starts_with("--") => None,
        Some(_) => it.next(),
        None => None,
    }
}

fn entrance_str(e: Entrance) -> &'static str {
    match e {
        Entrance::Mcp => "mcp",
        Entrance::Cli => "cli",
        Entrance::Daemon => "daemon",
    }
}

/// `.simple()` is 32 hex chars without hyphens, so the first 8-char slice is all ASCII hex (byte-slice safe).
fn short_peer_id(id: uuid::Uuid) -> String {
    id.simple().to_string()[..8].to_string()
}

/// ★Why print everything without an axis filter★: [B->A delivery] prints only the reply-axis record (B→A); group (@all) broadcasts or sends during B's original task turn showed up nowhere in stdout, making acceptance measurements depend on participants' self-reports.
/// ★The delivered/err axis cannot be omitted (F1)★: only delivered records are evidence — printed without a status axis, write-failed records read like delivery evidence and the census reproduces the self-report dependency.
fn print_delivery_census(observer: &CapturingObserver) {
    for (idx, obs) in observer.all_records().iter().enumerate() {
        println!(
            "[delivery-census] #{idx} from={} to={}({}) entrance={} msg_id={} in_reply_to={} bytes={} delivered={} err={}",
            short_peer_id(obs.from.peer_id),
            short_peer_id(obs.to_id),
            obs.to_name,
            entrance_str(obs.entrance),
            obs.msg_id,
            obs.in_reply_to.as_deref().unwrap_or("-"),
            obs.bytes_requested,
            obs.is_delivered(),
            obs.error.as_deref().unwrap_or("-")
        );
    }
}

/// This harness always runs inside the source tree via `cargo run`, so `CARGO_MANIFEST_DIR` can be trusted (unlike the production daemon's exe walk-up).
fn repo_root_from_manifest() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")); // .../crates/engram-dashboard-daemon
    manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or(manifest)
}

fn sibling_send_exe() -> Option<PathBuf> {
    engram_dashboard_platform::env::sibling_exe(CLI_EXE_NAME).filter(|cand| cand.is_file())
}

fn is_agent_alive(manager: &Arc<AgentManager>, id: AgentId) -> bool {
    manager
        .list_agents()
        .iter()
        .find(|a| a.id == id)
        .map(|a| matches!(a.status, AgentStatus::Running | AgentStatus::Exiting))
        .unwrap_or(false)
}

fn agent_profile(name: &str, model: &str, workspace: &Path) -> AgentProfile {
    // Name the agents alice and bob explicitly via display_name: a cwd-derived name would be the workspace
    // folder's basename (`engram-roundtrip-ws-a-<uuid>`), not the name the agents address each other by
    // (ADR-0101 WYSIWYA).
    let mut profile = AgentProfile::new(
        name.to_string(),
        AgentCommand::Claude {
            extra_args: vec!["--model".to_string(), model.to_string()],
            output_format: AgentOutputFormat::StreamJson,
        },
        workspace.to_path_buf(),
        vec![],
        false,
    );
    profile.display_name = Some(name.to_string());
    profile
}

/// The started agent, or why it did not start (kept for the SKIPPED / SETUP-FAIL line).
fn spawn_named(manager: &Arc<AgentManager>, profile: &AgentProfile) -> Result<AgentInfo, String> {
    let info = manager
        .spawn_agent(profile, SpawnMode::Fresh)
        .map_err(|e| format!("the spawn of {} failed: {e}", profile.name))?
        .into_started()
        .ok_or_else(|| {
            format!(
                "the spawn of {} did nothing (already running or starting)",
                profile.name
            )
        })?;
    let deadline = Instant::now() + SPAWN_APPEAR_TIMEOUT;
    while Instant::now() < deadline {
        if manager.list_agents().iter().any(|a| a.id == info.id) {
            return Ok(info);
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    Err(format!(
        "{} did not appear in the agent list within {SPAWN_APPEAR_TIMEOUT:?}",
        profile.name
    ))
}

/// Polling: the relay depends on B's real claude judgment with nondeterministic delay and there is no cv signal source, so short sleep polling is enough.
fn wait_for_reply(
    observer: &Arc<CapturingObserver>,
    baseline: usize,
    from_b: AgentId,
    to_a: AgentId,
    cap: Duration,
) -> Option<DeliveryObservation> {
    let deadline = Instant::now() + cap;
    loop {
        if let Some(rec) = observer.find_delivery_after(baseline, from_b, to_a) {
            return Some(rec);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn wait_for_matching_reply(
    observer: &Arc<CapturingObserver>,
    baseline: usize,
    from_b: AgentId,
    to_a: AgentId,
    seed_msg_id: &str,
    cap: Duration,
) -> Option<DeliveryObservation> {
    let deadline = Instant::now() + cap;
    loop {
        if let Some(rec) = observer
            .records_after(baseline, from_b, to_a)
            .into_iter()
            .find(|r| r.is_delivered() && r.in_reply_to.as_deref() == Some(seed_msg_id))
        {
            return Some(rec);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
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

async fn cleanup(manager: &Arc<AgentManager>, agent_ids: &[AgentId], dirs: &[&PathBuf]) {
    for id in agent_ids {
        let _ = manager.kill_agent(*id);
    }
    if !agent_ids.is_empty() {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !manager.list_agents().is_empty() {
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    remove_dirs(dirs);
}

fn remove_dirs(dirs: &[&PathBuf]) {
    for d in dirs {
        let _ = std::fs::remove_dir_all(d);
    }
}

struct NoopStatus;
impl StatusSink for NoopStatus {
    fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
    fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
}

/// ★Lost-wakeup prevention (FIX round-2 #3)★: with `done_count` as an `AtomicU64` incremented · notified outside the mutex, a completion signal between the waiter's predicate check and `wait_timeout` registration is lost (the cv does not remember an earlier notify), giving a false timeout. So `done_count` lives inside the very mutex the cv uses, and signaler and waiter update · recheck under that lock.
struct TurnState {
    text: String,
    done_count: u64,
}

struct TurnObserver {
    id: SinkId,
    state: Mutex<TurnState>,
    cv: Condvar,
}

impl TurnObserver {
    fn new() -> Self {
        Self {
            id: SinkId::new_v4(),
            state: Mutex::new(TurnState {
                text: String::new(),
                done_count: 0,
            }),
            cv: Condvar::new(),
        }
    }
    fn begin_turn(&self) {
        self.state.lock().unwrap().text.clear();
    }
    fn done_snapshot(&self) -> u64 {
        self.state.lock().unwrap().done_count
    }
    fn wait_turn_end(&self, baseline: u64, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut g = self.state.lock().unwrap();
        loop {
            if g.done_count > baseline {
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
        self.state.lock().unwrap().text.clone()
    }
}

impl OutputSink for TurnObserver {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        let OutputPayload::Event(ev) = frame.payload else {
            return Ok(());
        };
        match ev {
            OutputEvent::TextDelta { text, .. } => {
                self.state.lock().unwrap().text.push_str(text);
            }
            OutputEvent::MessageDone { .. } => {
                let mut g = self.state.lock().unwrap();
                g.done_count += 1;
                drop(g);
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

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_agent::launch::{
        LaunchBlocked, LaunchKit, LaunchPlan, NoopLaunchKits, PrepareError,
    };
    use engram_dashboard_asset_library::LibraryService;
    use engram_dashboard_daemon::control::priming::teaches_mail_cli;

    fn s(v: &[&str]) -> impl Iterator<Item = String> {
        v.iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn parse_args_defaults() {
        let a = parse_args(s(&[]));
        assert_eq!(a.builtin_dir, None);
        assert_eq!(a.model, "sonnet");
        assert!(!a.disallow_mcp, "default allows MCP (today's behavior)");
        assert!(
            !a.seed_request,
            "default has no seed contract (plain notification, today's behavior)"
        );
        assert_eq!(a.seed_reply_by, None);
        assert_eq!(
            a.seed_reply_by_error, None,
            "default has no --seed-reply-by misuse"
        );
        assert_eq!(
            a.b_task, None,
            "default has no B original task override (today's behavior)"
        );
        assert_eq!(a.b_task_error, None, "default has no --b-task misuse");
    }

    #[test]
    fn parse_args_disallow_mcp_flag_is_boolean() {
        let a = parse_args(s(&["--disallow-mcp", "--model", "opus"]));
        assert!(a.disallow_mcp, "--disallow-mcp present → on");
        assert_eq!(
            a.model, "opus",
            "--model after --disallow-mcp parses normally"
        );
        assert_eq!(a.builtin_dir, None);
    }

    #[test]
    fn parse_args_disallow_mcp_absent_is_false() {
        let a = parse_args(s(&["--builtin-dir", "some/builtin", "--model", "haiku"]));
        assert!(!a.disallow_mcp);
    }

    #[test]
    fn parse_args_builtin_dir_and_model() {
        let a = parse_args(s(&["--builtin-dir", "some/builtin", "--model", "opus"]));
        assert_eq!(a.builtin_dir.as_deref(), Some("some/builtin"));
        assert_eq!(a.model, "opus");
    }

    #[test]
    fn parse_args_order_independent_and_ignores_unknown() {
        let a = parse_args(s(&[
            "--model",
            "haiku",
            "junk",
            "--builtin-dir",
            "other/builtin",
        ]));
        assert_eq!(a.builtin_dir.as_deref(), Some("other/builtin"));
        assert_eq!(a.model, "haiku");
    }

    #[test]
    fn parse_args_flag_without_value_is_ignored() {
        let a = parse_args(s(&["--builtin-dir"]));
        assert_eq!(a.builtin_dir, None);
        assert_eq!(a.model, "sonnet");
    }

    #[test]
    fn parse_args_flag_does_not_consume_next_flag_as_value() {
        let a = parse_args(s(&["--builtin-dir", "--model", "opus"]));
        assert_eq!(
            a.builtin_dir, None,
            "a following flag is not taken as the --builtin-dir value"
        );
        assert_eq!(a.model, "opus", "--model must parse normally");
    }

    #[test]
    fn parse_args_trailing_flag_flags_both_ignored_cleanly() {
        let a = parse_args(s(&["--model", "--builtin-dir"]));
        assert_eq!(a.builtin_dir, None);
        assert_eq!(
            a.model, "sonnet",
            "a flag after --model means no value → the default stays"
        );
    }

    #[test]
    fn resolve_builtin_dir_unset_is_the_repo_builtin() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        assert_eq!(resolve_builtin_dir(None, &root), root.join("builtin"));
    }

    #[test]
    fn resolve_explicit_absolute_path_passthrough() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let abs = if cfg!(windows) {
            "C:\\custom\\my-builtin"
        } else {
            "/custom/my-builtin"
        };
        let got = resolve_builtin_dir(Some(abs), &root);
        assert_eq!(got, PathBuf::from(abs), "an absolute folder passes as is");
    }

    #[test]
    fn resolve_explicit_relative_path_joined_under_root() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let got = resolve_builtin_dir(Some("sub/custom"), &root);
        assert!(got.is_absolute());
        assert!(
            got.ends_with("sub/custom") || got.ends_with("sub\\custom"),
            "a relative folder is joined under the repo root: {got:?}"
        );
    }

    // ── check_builtin (the pre-spawn setup gate) ───────────────────────────────────────────────

    /// A temporary folder outside the repository, removed on drop.
    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch() -> Scratch {
        let root =
            std::env::temp_dir().join(format!("engram-roundtrip-test-{}", AgentId::new_v4()));
        for folder in ["ws", "home"] {
            std::fs::create_dir_all(root.join(folder)).expect("create the scratch folder");
        }
        Scratch(root)
    }

    fn repo_builtin() -> PathBuf {
        repo_root_from_manifest().join("builtin")
    }

    /// A built-in folder under `root` holding the repo built-ins named in `copied` and the priming text `priming`
    /// (`None` = no priming file).
    fn builtin_copy(root: &Path, copied: &[&str], priming: Option<&str>) -> PathBuf {
        let dir = root.join("builtin");
        for rel in copied {
            let to = dir.join(rel);
            std::fs::create_dir_all(to.parent().unwrap()).expect("create the parent folder");
            std::fs::copy(repo_builtin().join(rel), &to).expect("copy the built-in");
        }
        if let Some(text) = priming {
            std::fs::create_dir_all(dir.join("instructions")).expect("create instructions");
            std::fs::write(priming_file(&dir), text).expect("write the priming");
        }
        dir
    }

    /// The run's library service over `builtin_dir` — hermetic: the scratch home and an empty environment, so the
    /// user's own skill roots are not read.
    fn library_over(root: &Path, builtin_dir: &Path) -> LibraryService {
        let profiles = Arc::new(ProfileRegistry::new(Arc::new(FileProfileStore::new(
            root.join("profiles"),
        ))));
        build_library_service(
            profiles,
            Some(builtin_dir.to_path_buf()),
            root.join("launch"),
            LibraryHost {
                home: Some(root.join("home")),
                env: Arc::new(|_| None),
            },
            library_start(root),
        )
    }

    /// `check_builtin` over `kits` for the run's two agents, as `run` calls it.
    fn check_with(root: &Path, kits: &dyn LaunchKitSource) -> Result<(), String> {
        let alice = agent_profile(NAME_A, "sonnet", &root.join("ws"));
        let bob = agent_profile(NAME_B, "sonnet", &root.join("ws"));
        check_builtin(kits, &[&alice, &bob])
    }

    #[test]
    fn check_builtin_over_the_repo_builtin_passes_with_the_shipped_priming() {
        let root = scratch();
        let library = library_over(&root.0, &repo_builtin());
        check_with(&root.0, &library).expect("the repo builtin passes");
        let shipped = std::fs::read_to_string(priming_file(&repo_builtin()))
            .expect("read the shipped priming");
        assert_eq!(library.builtin_priming_text(), shipped);
    }

    #[test]
    fn check_builtin_without_the_priming_file_is_a_setup_failure() {
        let root = scratch();
        let dir = builtin_copy(
            &root.0,
            &["presets/default.yaml", "settings/default.yaml"],
            None,
        );
        let err = check_with(&root.0, &library_over(&root.0, &dir))
            .err()
            .expect("a missing priming fails");
        assert!(err.contains(BUILTIN_UNAVAILABLE), "{err}");
        assert!(err.contains("engram:/priming"), "{err}");
    }

    #[test]
    fn check_builtin_with_the_priming_but_no_default_preset_is_a_setup_failure() {
        let root = scratch();
        let dir = builtin_copy(
            &root.0,
            &["settings/default.yaml"],
            Some("Teammates mail you."),
        );
        let err = check_with(&root.0, &library_over(&root.0, &dir))
            .err()
            .expect("a stood-in default preset fails");
        assert!(err.contains(BUILTIN_UNAVAILABLE), "{err}");
        assert!(err.contains("engram:/default"), "{err}");
    }

    #[test]
    fn check_builtin_with_a_default_preset_that_does_not_list_the_priming_is_a_setup_failure() {
        let root = scratch();
        let dir = builtin_copy(
            &root.0,
            &["settings/default.yaml"],
            Some("Teammates mail you."),
        );
        std::fs::create_dir_all(dir.join("presets")).expect("create presets");
        std::fs::write(
            dir.join("presets").join("default.yaml"),
            "kind: preset\nversion: 1\nsettings:\n  - engram:/settings/default\n",
        )
        .expect("write the preset");
        let err = check_with(&root.0, &library_over(&root.0, &dir))
            .err()
            .expect("a launch without the priming fails");
        assert!(err.contains("include_priming=false"), "{err}");
    }

    #[test]
    fn check_builtin_with_an_empty_priming_file_is_not_a_setup_failure() {
        let root = scratch();
        let dir = builtin_copy(
            &root.0,
            &["presets/default.yaml", "settings/default.yaml"],
            Some(""),
        );
        let library = library_over(&root.0, &dir);
        check_with(&root.0, &library).expect("an empty priming passes");
        assert_eq!(library.builtin_priming_text(), "");
    }

    /// A launch-kit source whose every check is refused.
    struct Refusing;

    impl LaunchKitSource for Refusing {
        fn check(&self, _profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked> {
            Err(LaunchBlocked {
                reasons: Vec::new(),
            })
        }
        fn prepare(
            &self,
            _plan: LaunchPlan,
            _id: AgentId,
            _epoch: u32,
        ) -> Result<LaunchKit, PrepareError> {
            unreachable!("check_builtin only checks")
        }
        fn release(&self, _id: AgentId, _epoch: u32) {}
    }

    #[test]
    fn check_builtin_with_a_refused_launch_check_is_a_setup_failure() {
        let root = scratch();
        let err = check_with(&root.0, &Refusing)
            .err()
            .expect("a refused check fails");
        assert!(err.contains("launch check of alice failed"), "{err}");
    }

    #[test]
    fn check_builtin_over_noop_kits_is_a_setup_failure() {
        let root = scratch();
        let err = check_with(&root.0, &NoopLaunchKits)
            .err()
            .expect("a Noop plan fails");
        assert!(err.contains("real=false"), "{err}");
    }

    #[test]
    fn a_spawn_failure_is_skipped_only_when_the_claude_cli_does_not_run() {
        assert_eq!(
            classify_spawn_failure(&ProbeVerdict::NoClaude("absent".into())),
            SpawnFailure::NoClaude
        );
        assert_eq!(
            classify_spawn_failure(&ProbeVerdict::Runs),
            SpawnFailure::Setup,
            "with a working CLI a spawn or prepare failure is the harness's"
        );
        assert_eq!(
            classify_spawn_failure(&ProbeVerdict::Broken("timed out".into())),
            SpawnFailure::Setup,
            "a CLI that is there but misbehaves is not an absent one"
        );
    }

    #[test]
    fn a_probe_that_never_exits_times_out() {
        let mut polls = 0;
        let waited = wait_bounded(Duration::from_millis(120), || {
            polls += 1;
            Ok(None)
        });
        assert_eq!(waited.unwrap(), None);
        assert!(polls > 1, "it polls until the deadline, not once");
    }

    #[test]
    fn a_probe_that_exits_answers_with_its_status() {
        let mut polls = 0;
        let waited = wait_bounded(Duration::from_secs(60), || {
            polls += 1;
            Ok((polls == 3).then_some(false))
        });
        assert_eq!(waited.unwrap(), Some(false));
        assert_eq!(polls, 3);
    }

    #[test]
    fn a_failed_poll_ends_the_wait_with_its_error() {
        let waited = wait_bounded(Duration::from_secs(60), || {
            Err(std::io::Error::other("gone"))
        });
        assert_eq!(waited.unwrap_err().to_string(), "gone");
    }

    #[test]
    fn only_an_absent_or_failing_cli_is_skipped_and_a_timeout_is_a_setup_failure() {
        assert_eq!(
            probe_verdict(ProbeEnd::Exited { success: true }),
            ProbeVerdict::Runs
        );
        assert!(matches!(
            probe_verdict(ProbeEnd::Exited { success: false }),
            ProbeVerdict::NoClaude(_)
        ));
        assert!(matches!(
            probe_verdict(ProbeEnd::NotStarted("not found".into())),
            ProbeVerdict::NoClaude(_)
        ));
        assert!(matches!(
            probe_verdict(ProbeEnd::TreeFailed("no job".into())),
            ProbeVerdict::Broken(_)
        ));
        assert!(matches!(
            probe_verdict(ProbeEnd::WaitFailed("gone".into())),
            ProbeVerdict::Broken(_)
        ));
        let ProbeVerdict::Broken(why) = probe_verdict(ProbeEnd::TimedOut {
            deadline: Duration::from_secs(10),
            reaped: true,
        }) else {
            panic!("a timeout is a misbehaving CLI, not an absent one");
        };
        assert!(why.contains("timed out after 10s"), "{why}");
    }

    /// A real long-lived child (through the same console wrapping — on Windows `ping` is a grandchild under `cmd.exe`)
    /// is cut as a tree on the timeout and reaped within the reap window.
    #[test]
    fn a_probe_that_outlives_its_deadline_is_cut_and_reaped() {
        let (program, args): (&str, &[&str]) = if cfg!(windows) {
            ("ping", &["-n", "60", "127.0.0.1"])
        } else {
            ("sleep", &["60"])
        };
        let deadline = Duration::from_millis(500);
        let started = Instant::now();
        let end = probe(
            program,
            args.iter().map(|a| a.to_string()).collect(),
            deadline,
        );
        let took = started.elapsed();
        assert!(
            matches!(end, ProbeEnd::TimedOut { reaped: true, .. }),
            "{end:?}"
        );
        assert!(
            took < deadline + PROBE_REAP_DEADLINE + Duration::from_secs(1),
            "the probe returned only after {took:?}"
        );
    }

    #[test]
    fn a_probe_of_a_missing_program_does_not_run() {
        let end = probe(
            "engram-roundtrip-no-such-program",
            Vec::new(),
            Duration::from_secs(10),
        );
        assert!(
            matches!(probe_verdict(end), ProbeVerdict::NoClaude(_)),
            "a missing program is an absent CLI"
        );
    }

    #[test]
    fn teaches_mail_cli_true_for_cli_command_mention() {
        let text =
            "To reply, run in your shell: `$ENGRAM_CLI_EXE mail send --to alice --body ...`\n\
                    i.e. run the engram mail send command with the recipient name.";
        assert!(
            teaches_mail_cli(text),
            "mention of a mail CLI command → CLI instruction"
        );
    }

    /// ★A bare mention is not teaching (it must be a call form)★ — a sentence saying "do not use" the variable must not satisfy the requirement gate. The rule's source of truth and full cases are the tests on the `control::priming` side.
    #[test]
    fn env_var_mention_is_surface_evidence_and_invocation_is_teaching() {
        let mention = "Invoke the binary referenced by ENGRAM_CLI_EXE to deliver your message.";
        assert!(!teaches_mail_cli(mention), "a bare mention is not teaching");
        assert!(
            mentions_mail_cli_surface(mention),
            "still a CLI surface trace, so it is caught in the prohibition direction"
        );
        assert!(
            teaches_mail_cli("run `$ENGRAM_CLI_EXE mail send --to alice`"),
            "a variable call form is teaching"
        );
    }

    #[test]
    fn teaches_mail_cli_false_for_mcp_only() {
        let text = "To reply, call the MCP tool `eg_send` with the recipient and body.";
        assert!(
            !teaches_mail_cli(text),
            "only MCP eg_send → not a CLI instruction"
        );
    }

    #[test]
    fn teaches_mail_cli_false_for_empty() {
        assert!(!teaches_mail_cli(""), "empty body → not a CLI instruction");
    }

    #[test]
    fn teaches_mail_cli_case_insensitive() {
        assert!(
            teaches_mail_cli("Reply via ENGRAM MAIL SEND right away."),
            "uppercase notation → CLI instruction"
        );
        assert!(
            teaches_mail_cli("Use Engram Mail Send to deliver."),
            "mixed notation → CLI instruction"
        );
        assert!(
            teaches_mail_cli("Invoke $Engram_Cli_Exe Mail Pending to check."),
            "mixed-notation variable call → CLI instruction"
        );
    }

    #[test]
    fn teaches_mail_cli_negation_is_intentionally_true() {
        assert!(
            teaches_mail_cli("Do NOT use engram mail send; use MCP instead."),
            "a negative sentence is true too if call-form adjacency holds — sentence negation is intentionally not decided (known limit)"
        );
    }

    // ── REPLY_POLL label decision (pure) ──────────────────────────────────────────────────────────────
    #[test]
    fn reply_poll_label_matched_wins_regardless_of_poll_ran() {
        assert_eq!(reply_poll_label(true, true), "matched");
        assert_eq!(
            reply_poll_label(true, false),
            "matched",
            "matched takes top priority regardless of poll_ran (wins over skipped-no-budget)"
        );
    }

    #[test]
    fn reply_poll_label_timeout_when_ran_but_not_matched() {
        assert_eq!(reply_poll_label(false, true), "timeout");
    }

    #[test]
    fn reply_poll_label_skipped_no_budget_when_not_ran_and_not_matched() {
        assert_eq!(reply_poll_label(false, false), "skipped-no-budget");
    }

    // ── --seed-request/--seed-reply-by (pure parsing) ─────────────────────────────────────────────────
    #[test]
    fn parse_args_seed_request_flag_is_boolean() {
        let a = parse_args(s(&["--seed-request", "--model", "opus"]));
        assert!(a.seed_request, "--seed-request present → on");
        assert_eq!(
            a.model, "opus",
            "--model after --seed-request parses normally"
        );
        assert_eq!(a.seed_reply_by, None);
    }

    #[test]
    fn parse_args_seed_request_absent_is_false() {
        let a = parse_args(s(&["--builtin-dir", "some/builtin", "--model", "haiku"]));
        assert!(!a.seed_request);
    }

    #[test]
    fn parse_args_seed_reply_by_value() {
        let a = parse_args(s(&["--seed-request", "--seed-reply-by", "5m"]));
        assert!(a.seed_request);
        assert_eq!(a.seed_reply_by.as_deref(), Some("5m"));
    }

    #[test]
    fn parse_args_seed_reply_by_does_not_consume_next_flag_as_value() {
        let a = parse_args(s(&["--seed-reply-by", "--model", "opus"]));
        assert_eq!(
            a.seed_reply_by, None,
            "if the next token is a flag, it is not swallowed as the seed_reply_by value"
        );
        assert!(
            a.seed_reply_by_error.is_some(),
            "if the next token looks like a flag, a misuse error must be recorded (F5)"
        );
        assert_eq!(
            a.model, "opus",
            "--model must parse normally (recording the error does not block later parsing)"
        );
    }

    #[test]
    fn parse_args_seed_reply_by_missing_value_at_end_is_an_error() {
        let a = parse_args(s(&["--seed-request", "--seed-reply-by"]));
        assert_eq!(a.seed_reply_by, None);
        assert!(
            a.seed_reply_by_error.is_some(),
            "--seed-reply-by ending without a value must record a misuse error (F5)"
        );
    }

    #[test]
    fn parse_args_seed_reply_by_empty_value_is_an_error() {
        let a = parse_args(s(&["--seed-request", "--seed-reply-by", ""]));
        assert_eq!(a.seed_reply_by, None);
        assert!(
            a.seed_reply_by_error.is_some(),
            "an empty value must record a misuse error (F5)"
        );
    }

    #[test]
    fn parse_args_seed_reply_by_whitespace_only_value_is_an_error() {
        let a = parse_args(s(&["--seed-request", "--seed-reply-by", "   "]));
        assert_eq!(a.seed_reply_by, None);
        assert!(
            a.seed_reply_by_error.is_some(),
            "a whitespace-only value must record a misuse error (F5)"
        );
    }

    #[test]
    fn parse_args_seed_reply_by_valid_value_has_no_error() {
        let a = parse_args(s(&["--seed-request", "--seed-reply-by", "5m"]));
        assert_eq!(a.seed_reply_by.as_deref(), Some("5m"));
        assert_eq!(
            a.seed_reply_by_error, None,
            "a valid value records no error"
        );
    }

    #[test]
    fn seed_reply_by_without_request_is_rejected() {
        assert!(
            seed_reply_by_without_request_is_invalid(false, &Some("5m".to_string())),
            "seed_request=false but seed_reply_by=Some → bounce"
        );
    }

    #[test]
    fn seed_reply_by_with_request_is_valid() {
        assert!(!seed_reply_by_without_request_is_invalid(
            true,
            &Some("5m".to_string())
        ));
    }

    #[test]
    fn seed_reply_by_absent_is_always_valid() {
        assert!(!seed_reply_by_without_request_is_invalid(false, &None));
        assert!(!seed_reply_by_without_request_is_invalid(true, &None));
    }

    // ── --b-task (pure parsing + file reference resolution) ───────────────────────────────────────────────────────
    #[test]
    fn parse_args_b_task_inline_text() {
        let a = parse_args(s(&["--b-task", "You are on billing. Reply in one line."]));
        assert_eq!(
            a.b_task.as_deref(),
            Some("You are on billing. Reply in one line.")
        );
    }

    #[test]
    fn parse_args_b_task_file_reference_kept_raw() {
        let a = parse_args(s(&["--b-task", "@prompts/experiments/b-task.md"]));
        assert_eq!(a.b_task.as_deref(), Some("@prompts/experiments/b-task.md"));
    }

    #[test]
    fn parse_args_b_task_next_looks_like_flag_is_an_error() {
        let a = parse_args(s(&["--b-task", "--model", "opus"]));
        assert_eq!(
            a.b_task, None,
            "if the next token is a flag, it is not swallowed as the b_task value (the value stays unset)"
        );
        assert!(
            a.b_task_error.is_some(),
            "if the next token looks like a flag, a misuse error must be recorded (F5)"
        );
        assert_eq!(
            a.model, "opus",
            "--model must parse normally (recording the error does not block later parsing)"
        );
    }

    #[test]
    fn parse_args_b_task_missing_value_at_end_is_an_error() {
        let a = parse_args(s(&["--b-task"]));
        assert_eq!(a.b_task, None);
        assert!(
            a.b_task_error.is_some(),
            "--b-task ending without a value must record a misuse error (F5)"
        );
    }

    #[test]
    fn parse_args_b_task_empty_value_is_an_error() {
        let a = parse_args(s(&["--b-task", ""]));
        assert_eq!(a.b_task, None);
        assert!(
            a.b_task_error.is_some(),
            "an empty value must record a misuse error (F5)"
        );
    }

    #[test]
    fn parse_args_b_task_whitespace_only_value_is_an_error() {
        let a = parse_args(s(&["--b-task", "   "]));
        assert_eq!(a.b_task, None);
        assert!(
            a.b_task_error.is_some(),
            "a whitespace-only value must record a misuse error (F5)"
        );
    }

    #[test]
    fn parse_args_b_task_valid_value_has_no_error() {
        let a = parse_args(s(&["--b-task", "You are on billing."]));
        assert_eq!(a.b_task.as_deref(), Some("You are on billing."));
        assert_eq!(a.b_task_error, None, "a valid value records no error");
    }

    #[test]
    fn resolve_b_task_file_path_absolute_passthrough() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let abs = if cfg!(windows) {
            "@C:\\custom\\task.md"
        } else {
            "@/custom/task.md"
        };
        let got = resolve_b_task_file_path(abs, &root).expect("@ prefix → file reference");
        let expected = if cfg!(windows) {
            "C:\\custom\\task.md"
        } else {
            "/custom/task.md"
        };
        assert_eq!(
            got,
            PathBuf::from(expected),
            "an absolute path passes as is"
        );
    }

    #[test]
    fn resolve_b_task_file_path_relative_joined_under_root() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let got = resolve_b_task_file_path("@sub/task.md", &root)
            .expect("@ prefix → relative file reference");
        assert!(got.is_absolute());
        assert!(
            got.ends_with("sub/task.md") || got.ends_with("sub\\task.md"),
            "a relative path is joined from the repo root: {got:?}"
        );
    }

    #[test]
    fn resolve_b_task_file_path_none_for_inline_text() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        assert_eq!(
            resolve_b_task_file_path("plain inline text", &root),
            None,
            "without the @ prefix, None (treated as inline text)"
        );
    }
}
