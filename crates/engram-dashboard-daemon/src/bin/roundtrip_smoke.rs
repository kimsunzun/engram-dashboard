//! roundtrip-smoke — A→B→A round trip (reply round-trip) measurement driver (verification-only bin).
//!
//! Spawns 2 real primed claudes (A=alice · B=bob, stream-json/Fresh) and, on top of the A→B receiving that priming-smoke proved, looks at two things: ① does B call the sending half (MCP `eg_send` or the `engram mail send` CLI) on its own ② does A naturally accept that reply. Mechanical axis = registry `DeliveryObservation` (from=B, to=A) + the entrance B chose; qualitative axis = A's turn text. The orchestrator interprets the stdout markers below.
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
//! - `--priming <path|C0>` — priming file (absolute as is, relative from the repo root). Unspecified · `C0` = production A `prompts/agent-priming.md`. The old C1~C3 aliases were removed (ADR-0099): such a value is read as a file path and fails as missing.
//! - `--model <name>` — default sonnet.
//! - `--disallow-mcp` — removes only the MCP `eg_send` grant. ★It cannot create CLI routing (measured: 6/6 still sent through MCP)★ — the spawn stays MCP-capable, so the daemon refuses that credential's CLI mail (ADR-0133). ★No mode measures the CLI entrance★ — canonical mail path is MCP, the CLI is a mirror (ADR-0209 decision 3). Keeping or discarding the knob awaits a user decision.
//! - `--seed-request` (+ `--seed-reply-by <dur>`, e.g. `5m`) — sends the seed A→B as a reply contract.
//! - `--b-task <text|@path>` — replaces B's original task prompt (`@` = file reference, relative from the repo root).
//! - Misuse of `--b-task` · `--seed-reply-by` (missing value · next token starts with `--` · blank) and `--seed-reply-by` without `--seed-request` bounce with exit 1 before spawn. Other knobs are lenient (unspecified = today's behavior).
//!
//! ## Exit labels (stdout and stderr — no silent skip)
//! - `SKIPPED` exit 0 — claude missing/auth failure.
//! - `SETUP-SKIP` exit 1 — infrastructure the case requires is missing (a CLI path but the control plane CLI is not built).
//! - `SETUP-FAIL` exit 1 — argument error (before spawn) · priming file missing/unreadable · A/B output subscribe failure · B task turn failure · A·B process death · seed ACK bounced.
//! - Otherwise exit 0 — a valid negative is exit 0 too.
//!
//! ## stdout markers (the orchestrator parses them)
//! The `ROUNDTRIP CASE= B_SENT= ENTRANCE=` banner · `[model]` · `[priming]` (the real in-effect path that passed the existence check) · `SEED_KIND=` · `B_TASK=` are printed always, even when unspecified, so a silently ignored typo stands out. With `--seed-request`:
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
//! - No experiment uses the CLI entrance (ADR-0128 · CLI mirror priming abolished) — the daemon refuses an MCP-capable spawn's CLI mail by credential (ADR-0133), so `--priming` combinations with CLI-instructing priming are refused before spawn. The decision reads the resolved priming file body, not the selector or basename.
// ADR-0092

use std::path::PathBuf;
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
    OutputSink, SinkError, SinkId, StatusSink, CLI_EXE_NAME,
};

use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, SendContract};
use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server_with_help, CommandTableSlot, ManagerSlot, MessagingSlot, RosterBroadcastSlot,
};
use engram_dashboard_daemon::control::priming::{
    mentions_mail_cli_surface, FilePrimingProvider, PrimingProvider,
};
use engram_dashboard_daemon::control::registry::{BoundIdentity, ControlRegistry};
use engram_dashboard_daemon::control::DaemonControlChannel;
use engram_dashboard_daemon::messaging_host::messaging_for_manager;
use engram_dashboard_messaging::envelope::{DeliveryObservation, DeliveryObserver, Entrance};

const SPAWN_APPEAR_TIMEOUT: Duration = Duration::from_secs(10);
const TURN_WAIT_CAP: Duration = Duration::from_secs(180);
/// B reply wait cap — exceeding it is not an error but a negative result (B did not send).
const REPLY_WAIT_CAP: Duration = Duration::from_secs(180);

/// B learns this name from the envelope and replies with `to=alice`.
const NAME_A: &str = "alice";
const NAME_B: &str = "bob";

/// The original task that sets up the "working teammate" context before the seed — without it, whether a reply is natural cannot be decided.
const TASK_PROMPT_B: &str =
    "You are currently working on the auth module (login/session). When you're ready to start, reply in one line.";

/// ★Seed A→B (ADR-0092 — a natural teammate question, not a mechanical "use tool X")★: A asks B about progress, so answering A is the natural reaction. How to send is taught by the priming variant (C0/default = production A `prompts/agent-priming.md`, which teaches only eg_send — ADR-0126).
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

/// No existence check — the caller decides by attempting the read.
fn resolve_b_task_file_path(value: &str, repo_root: &std::path::Path) -> Option<PathBuf> {
    let rel = value.strip_prefix('@')?;
    let p = PathBuf::from(rel);
    Some(if p.is_absolute() {
        p
    } else {
        repo_root.join(p)
    })
}

/// `*_error` fields only record the fact of misuse; `run()` decides whether and when to bounce (fail fast with SETUP-FAIL before spawning a real claude).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    priming: Option<String>,
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
    let priming_selector = args.priming.clone();
    let resolved_priming = match resolve_priming_path(priming_selector.as_deref(), &repo_root) {
        Some(p) => p,
        None => {
            return setup_fail(&format!(
                "cannot resolve the priming selector ({priming_selector:?}) to an absolute path — experiment impossible"
            ));
        }
    };
    // `FilePrimingProvider` silently drops a nonexistent override and spawns UNPRIMED — so check it here, or the case label becomes false.
    if !resolved_priming.is_file() {
        return setup_fail(&format!(
            "priming file missing: {} (case={:?}) — a nonexistent override leads to an UNPRIMED spawn and makes the case label false",
            resolved_priming.display(),
            priming_selector
        ));
    }
    // ★Read the priming file only once and reuse it for the CLI-requirement guard below — do not revert★: touching it twice opened a TOCTOU window, and `read_to_string(...).unwrap_or(false)` swallowed every read failure as "not a CLI requirement", producing false normal negatives.
    let priming_content = match std::fs::read_to_string(&resolved_priming) {
        Ok(c) => c,
        Err(e) => {
            return setup_fail(&format!(
                "priming file read failed: {} (case={:?}): {e} — priming is required for the experiment, so it cannot proceed if unreadable",
                resolved_priming.display(),
                priming_selector
            ));
        }
    };
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
    // Set the three envs below before the provider · manager wiring so A and B get the same priming variant and grant set.
    std::env::set_var("ENGRAM_PRIMING_FILE", &resolved_priming);
    eprintln!(
        "[roundtrip] priming = {} (case={:?})",
        resolved_priming.display(),
        priming_selector
    );
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
            return 1;
        }
    };
    let url = handle.url.clone();
    let data_dir = std::env::temp_dir().join(format!("engram-roundtrip-{}", AgentId::new_v4()));
    let ws_a = std::env::temp_dir().join(format!("engram-roundtrip-ws-a-{}", AgentId::new_v4()));
    let ws_b = std::env::temp_dir().join(format!("engram-roundtrip-ws-b-{}", AgentId::new_v4()));
    let _ = std::fs::create_dir_all(&ws_a);
    let _ = std::fs::create_dir_all(&ws_b);

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
    let mentions_cli_surface = mentions_mail_cli_surface(&priming_content);
    if mentions_cli_surface {
        handle.shutdown().await;
        let dirs = [&data_dir, &ws_a, &ws_b];
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
        return setup_fail(&format!(
            "CLI-teaching priming on an MCP-capable spawn (case={:?}): the daemon refuses CLI mail requests from an MCP-capable spawn by credential (ADR-0133), so the command B learned does not work — a channel taught but unusable = a send freeze (ADR-0099 measured ~6/7 not sent) would be misattributed as a normal negative. There is no mode that measures the CLI entrance — the canonical mail path is MCP and the CLI is a mirror (ADR-0209 decision 3). ※The decision in this prohibition direction looks **broadly** (`mentions_mail_cli_surface`) — it is caught if **even one** of a mail CLI call form (`engram mail …`/`$ENGRAM_CLI_EXE mail …`) · the env variable name itself · a CLI-only flag (`--to`·`--body`·`--body-stdin`·`--request`·`--reply-by`·`--reply-to`) appears. Every arm compares at **word boundaries** after the same normalization (lowercase · whitespace/newline collapse · markdown emphasis/escape symbol removal · HTML comment removal · zero-width character removal), so disarray at the level of case · whitespace/newlines · markdown emphasis · HTML comments · zero-width characters is folded, and plain words like `engram mailbox`·`--token` are not caught. **Shell grammar in general (variable rebinding · aliases) and sentence negation are not decided** — the `teaches_mail_cli` comment is the source of truth for that limit. A negative sentence (\"do not use that command\") is still caught in this prohibition direction by the **bare mention** of the variable (an intended fail-closed: refusing is safer than misattributing — though it is not a 'taught' decision). To turn it into an MCP-only file, remove the mention itself",
            priming_selector
        ));
    }
    // ★This skip guards only the consistency of the harness's SETUP labels★, not "a CLI grant exists" — this mode's send grant is 0 whether or not the binary exists. Keeping or discarding the knob awaits a user decision.
    if args.disallow_mcp && send_exe.is_none() {
        handle.shutdown().await;
        let dirs = [&data_dir, &ws_a, &ws_b];
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
        return setup_skip(
            "--disallow-mcp: SETUP-SKIP because the control plane CLI sibling binary is missing. ※Note (ADR-0133) — this mode leaves the spawn MCP-capable, so if the binary is built the command runs but **the daemon refuses that request** (the CLI mail entrance of an MCP-capable credential is closed). Moreover, in the auto permission mode the grant is a NO-OP, so the agent sends through MCP anyway (measured 6/6). That is, building is only a procedure to get past this skip and does not create CLI routing — there is no mode that does that. To get past the skip: `cargo build -p engram-dashboard-cli`",
        );
    }
    let priming_provider: Arc<dyn PrimingProvider> = Arc::new(FilePrimingProvider::new(repo_root));
    let control: Arc<dyn ControlChannel> = Arc::new(DaemonControlChannel::new(
        registry.clone(),
        url,
        engram_dashboard_daemon::control::mcp_config::McpDir::new(data_dir.join("mcp-config")),
        send_exe,
        priming_provider,
    ));
    let sink: Arc<dyn StatusSink> = Arc::new(NoopStatus);
    let profile_dir =
        std::env::temp_dir().join(format!("engram-roundtrip-prof-{}", AgentId::new_v4()));
    let preset_dir =
        std::env::temp_dir().join(format!("engram-roundtrip-preset-{}", AgentId::new_v4()));
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
    // MessagingService assembly (3-way send branch · flush), wrapping the manager as a DeliveryPort. No flush sink is wired (NoopStatus), so parking scenarios take only the direct path; the seed goes to a live recipient (delivered).
    let messaging = Arc::new(messaging_for_manager(manager.clone(), registry.clone()));
    messaging_slot.set(messaging.clone());

    // ── A·B spawn (both real primed claude, stream-json, Fresh) ─────────────────────────
    let agent_a = match spawn_named(&manager, NAME_A, &args.model, &ws_a) {
        Some(a) => a,
        None => {
            let dirs = [&data_dir, &ws_a, &ws_b, &profile_dir, &preset_dir];
            cleanup(&manager, &[], &dirs).await;
            handle.shutdown().await;
            return skip_no_claude("A spawn/appearance failed");
        }
    };
    let agent_b = match spawn_named(&manager, NAME_B, &args.model, &ws_b) {
        Some(b) => b,
        None => {
            let dirs = [&data_dir, &ws_a, &ws_b, &profile_dir, &preset_dir];
            cleanup(&manager, &[agent_a.id], &dirs).await;
            handle.shutdown().await;
            return skip_no_claude("B spawn/appearance failed");
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
            let dirs = [&data_dir, &ws_a, &ws_b, &profile_dir, &preset_dir];
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
    let case_label = priming_selector.as_deref().unwrap_or("C0");
    println!("\n===== ROUNDTRIP CASE={case_label} B_SENT={b_sent} ENTRANCE={entrance_label} =====");
    println!("[model] {}", args.model);
    println!("[priming] {}", resolved_priming.display());
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
    let dirs = [&data_dir, &ws_a, &ws_b, &profile_dir, &preset_dir];
    cleanup(&manager, &[agent_a.id, agent_b.id], &dirs).await;
    handle.shutdown().await;
    0
}

/// Takes an `iter` to drop the std::env dependency (pure, so unit tests reach it).
/// A next token that is another flag (starts with `--`) is not swallowed as a value (FIX round-2 #7) — otherwise `--priming --model opus` would eat `--model` as the priming value.
fn parse_args(iter: impl Iterator<Item = String>) -> Args {
    let mut priming = None;
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
            "--priming" => {
                if let Some(v) = take_flag_value(&mut it) {
                    priming = Some(v);
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
        priming,
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

/// Always an absolute path — no existence check (`FilePrimingProvider` does the final existence · CLI-safety check).
fn resolve_priming_path(selector: Option<&str>, repo_root: &std::path::Path) -> Option<PathBuf> {
    let rel: &str = match selector {
        None | Some("C0") | Some("c0") => "prompts/agent-priming.md",
        Some(path) => {
            let p = PathBuf::from(path);
            let joined = if p.is_absolute() {
                p
            } else {
                repo_root.join(p)
            };
            return joined.is_absolute().then_some(joined);
        }
    };
    let joined = repo_root.join(rel);
    joined.is_absolute().then_some(joined)
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

fn spawn_named(
    manager: &Arc<AgentManager>,
    name: &str,
    model: &str,
    workspace: &std::path::Path,
) -> Option<AgentInfo> {
    // The two agents share the same workspace cwd, so cwd-derived names collide (alice/bob) and misroute. Plant the names in display_name to tell them apart deterministically (ADR-0101 WYSIWYA).
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
    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .ok()?
        .into_started()?;
    let deadline = Instant::now() + SPAWN_APPEAR_TIMEOUT;
    while Instant::now() < deadline {
        if manager.list_agents().iter().any(|a| a.id == info.id) {
            return Some(info);
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    None
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
        assert_eq!(a.priming, None);
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
        assert_eq!(a.priming, None);
    }

    #[test]
    fn parse_args_disallow_mcp_absent_is_false() {
        let a = parse_args(s(&["--priming", "some/priming.md", "--model", "haiku"]));
        assert!(!a.disallow_mcp);
    }

    #[test]
    fn parse_args_priming_and_model() {
        let a = parse_args(s(&["--priming", "some/priming.md", "--model", "opus"]));
        assert_eq!(a.priming.as_deref(), Some("some/priming.md"));
        assert_eq!(a.model, "opus");
    }

    #[test]
    fn parse_args_order_independent_and_ignores_unknown() {
        let a = parse_args(s(&[
            "--model",
            "haiku",
            "junk",
            "--priming",
            "other/priming.md",
        ]));
        assert_eq!(a.priming.as_deref(), Some("other/priming.md"));
        assert_eq!(a.model, "haiku");
    }

    #[test]
    fn parse_args_flag_without_value_is_ignored() {
        let a = parse_args(s(&["--priming"]));
        assert_eq!(a.priming, None);
        assert_eq!(a.model, "sonnet");
    }

    #[test]
    fn parse_args_flag_does_not_consume_next_flag_as_value() {
        let a = parse_args(s(&["--priming", "--model", "opus"]));
        assert_eq!(
            a.priming, None,
            "if the next token is a flag, it is not swallowed as the priming value"
        );
        assert_eq!(a.model, "opus", "--model must parse normally");
    }

    #[test]
    fn parse_args_trailing_flag_flags_both_ignored_cleanly() {
        let a = parse_args(s(&["--model", "--priming"]));
        assert_eq!(a.priming, None);
        assert_eq!(
            a.model, "sonnet",
            "a flag after --model means no value → the default stays"
        );
    }

    #[test]
    fn resolve_case_c0_maps_to_current_priming() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let got = resolve_priming_path(None, &root).expect("C0 path");
        assert!(got.is_absolute());
        assert!(
            got.ends_with("prompts/agent-priming.md") || got.ends_with("prompts\\agent-priming.md"),
            "C0 is the current priming: {got:?}"
        );
        let got2 = resolve_priming_path(Some("C0"), &root).expect("C0 path");
        assert_eq!(got, got2);
    }

    // ADR-0099: the explicit path override tests below cover the removed C1~C3 aliases' behavior.

    #[test]
    fn resolve_explicit_absolute_path_passthrough() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let abs = if cfg!(windows) {
            "C:\\custom\\my-priming.md"
        } else {
            "/custom/my-priming.md"
        };
        let got = resolve_priming_path(Some(abs), &root).expect("absolute override");
        assert_eq!(got, PathBuf::from(abs), "an absolute path passes as is");
    }

    #[test]
    fn resolve_explicit_relative_path_joined_under_root() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let got = resolve_priming_path(Some("sub/custom.md"), &root).expect("relative override");
        assert!(got.is_absolute());
        assert!(
            got.ends_with("sub/custom.md") || got.ends_with("sub\\custom.md"),
            "a relative path is joined from the repo root: {got:?}"
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
        let a = parse_args(s(&["--priming", "C0", "--model", "haiku"]));
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
