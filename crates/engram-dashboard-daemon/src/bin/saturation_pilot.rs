//! saturation-pilot — ADR-0090 Stage 2 context saturation measurement driver (experiment-only bin).
//!
//! Drives one real claude spawn in an isolated workspace; delivery runs through the real control path (handle_send → wrap_message → write_stdin_observed) — experiment path = production path is the premise of interpreting results (ADR-0090 d2). Pure logic lives in `experiment::{cli,filler,probe,record,transcript}`; this file is wiring + the turn loop.
//!
//! ## Invariants
//! - Summary always written — whatever the path (normal/timeout/abort/panic), a summary record is written last.
//! - Isolated workspace — a fresh temp dir is the cwd, secrets are not written, removed on exit (`--keep-workspace` excepted).
// ADR-0090

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::persistence::FileProfileStore;
use engram_dashboard_agent::profile::{
    AgentCommand, AgentOutputFormat, AgentProfile, ProfileRegistry, SpawnMode,
};
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, InputOrigin, OutputEvent, OutputFrame,
    OutputPayload, OutputSink, SinkError, SinkId, StatusSink,
};

use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server_without_help, CommandTableSlot, ManagerSlot, McpServerHandle,
    RosterBroadcastSlot,
};
use engram_dashboard_daemon::control::registry::{BoundIdentity, ControlRegistry};
use engram_dashboard_daemon::control::DaemonControlChannel;
use engram_dashboard_daemon::experiment::cli::{self, ParseError, PilotConfig};
use engram_dashboard_daemon::experiment::filler::{doc_title, filler_doc};
use engram_dashboard_daemon::experiment::probe::{
    detect_suspected_compaction, score_probe, select_detection_series, UsageSample,
};
use engram_dashboard_daemon::experiment::record::{
    cap_response, sha256_hex, CompactSignalRecord, HeaderRecord, HistogramRecord, InjectionRecord,
    ProbeRecord, Record, StallRecord, SummaryRecord, SuspectedCompactionRecord, TurnRecord,
    UsageSnapshot,
};
use engram_dashboard_daemon::experiment::transcript::{self, TranscriptSummary};
use engram_dashboard_messaging::envelope::{DeliveryObservation, DeliveryObserver, Entrance};

// ── hard caps (ADR-0090 invariants) ────────────────────────────────────────────────────────
const MAX_SPAWNS_PER_INVOCATION: u32 = 6;
const MAX_TURNS_PER_RUN: u32 = 120;
const MAX_WALLCLOCK_PER_RUN: Duration = Duration::from_secs(45 * 60);
const TURN_WAIT_CAP: Duration = Duration::from_secs(240);
const SPAWN_APPEAR_TIMEOUT: Duration = Duration::from_secs(10);

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match cli::parse_args(&argv) {
        Ok(c) => c,
        Err(ParseError::Help) => {
            print!("{}", cli::usage());
            std::process::exit(0);
        }
        Err(ParseError::Unknown(f)) => {
            eprintln!("unknown flag: {f}\n");
            print!("{}", cli::usage());
            std::process::exit(2);
        }
        Err(ParseError::Invalid(msg)) => {
            eprintln!("invalid argument: {msg}\n");
            print!("{}", cli::usage());
            std::process::exit(2);
        }
    };

    // The MCP server is async, so a runtime is needed — the driver body is blocking, so it runs inside block_on.
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("tokio runtime creation failed: {e}");
            std::process::exit(1);
        }
    };

    let exit_code = rt.block_on(async { run_all(cfg).await });
    std::process::exit(exit_code);
}

async fn run_all(cfg: PilotConfig) -> i32 {
    let claude_version = capture_claude_version();
    let git_commit = capture_git_commit();
    if claude_version.is_none() {
        // Without claude the experiment itself does not hold — fail instead of skipping.
        eprintln!(
            "FATAL [saturation-pilot]: cannot find the claude CLI (`claude --version` failed). \
             cannot spawn stream-json — the experiment does not hold. Check the claude install/auth."
        );
        return 3;
    }

    let out_dir = cfg
        .out
        .clone()
        .unwrap_or_else(|| default_out_dir(&utc_stamp_compact()));
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!(
            "output directory creation failed ({}): {e}",
            out_dir.display()
        );
        return 1;
    }
    eprintln!("[pilot] out dir = {}", out_dir.display());

    let runs = cfg.runs.min(MAX_SPAWNS_PER_INVOCATION);
    if runs < cfg.runs {
        eprintln!(
            "[pilot] runs {} → clamped to {} (MAX_SPAWNS_PER_INVOCATION)",
            cfg.runs, runs
        );
    }

    let mut worst = 0;
    for run_idx in 0..runs {
        let out_file = out_dir.join(format!("run-{run_idx}.jsonl"));
        eprintln!(
            "[pilot] === run {}/{} → {} ===",
            run_idx + 1,
            runs,
            out_file.display()
        );
        let code = run_one(
            &cfg,
            run_idx,
            &out_file,
            claude_version.clone(),
            git_commit.clone(),
        )
        .await;
        if code != 0 {
            worst = code;
        }
    }
    worst
}

async fn run_one(
    cfg: &PilotConfig,
    run_idx: u32,
    out_file: &std::path::Path,
    claude_version: Option<String>,
    git_commit: Option<String>,
) -> i32 {
    let run_started = Instant::now();
    let run_id = AgentId::new_v4().to_string();

    // The workspace is made before the result file — reversed, a failed workspace creation would leave an empty run-N.jsonl.
    let workspace = std::env::temp_dir().join(format!("engram-pilot-ws-{run_id}"));
    if let Err(e) = std::fs::create_dir_all(&workspace) {
        eprintln!("workspace creation failed: {e}");
        return 1;
    }

    let mut writer = match JsonlWriter::create(out_file) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("JSONL file creation failed ({}): {e}", out_file.display());
            let _ = std::fs::remove_dir_all(&workspace);
            return 1;
        }
    };

    let config_json = config_to_json(cfg);

    let Wiring {
        manager,
        registry,
        messaging,
        mcp_handle,
        // ★Do not fold this name into `..` or remove it★ — binding it keeps the reaper alive to the end of this scope (see that field of `Wiring`); otherwise it drops at once and the seat table closes.
        _relay_sweeper,
        data_dir,
        profile_dir,
    } = match wire(&run_id).await {
        Ok(w) => w,
        Err(e) => {
            eprintln!("wiring failed: {e}");
            // Accepted residual: a wiring failure is a setup abort before the header. An empty run file would confuse post-hoc parsing, so one minimal marker line states the cause (not the formal record schema).
            writer.write_raw_line(r#"{"aborted":"wiring_failed"}"#);
            let _ = std::fs::remove_dir_all(&workspace);
            return 1;
        }
    };

    let delivery_seen: Arc<Mutex<Vec<DeliveryObservation>>> = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture {
        seen: delivery_seen.clone(),
    }));

    // The header is written after the spawn — the transcript must be found by session id first to fill resolved_model/transcript_available.
    let agent = match spawn_pilot_agent(&manager, &workspace, &cfg.model) {
        Some(a) => a,
        None => {
            eprintln!(
                "FATAL [saturation-pilot]: claude (stream-json) spawn failed — the experiment does not hold (missing/auth)."
            );
            writer.write(&Record::Header(HeaderRecord {
                claude_version,
                daemon_git_commit: git_commit,
                model_pin: cfg.model.clone(),
                resolved_model: None,
                resolved_model_note: Some("claude spawn failed — no session".to_string()),
                transcript_available: false,
                transcript_path: None,
                timestamp_utc: utc_stamp_rfc3339(),
                run_index: run_idx,
                run_id: run_id.clone(),
                config: config_json,
            }));
            writer.write(&Record::Summary(SummaryRecord {
                max_context_tokens: 0,
                total_turns: 0,
                duration_ms: run_started.elapsed().as_millis() as u64,
                abort_reason: Some("claude spawn failed".to_string()),
                resolved_model: None,
                transcript_available: false,
                transcript_path: None,
            }));
            cleanup(
                &manager,
                None,
                mcp_handle,
                &CleanupPaths {
                    data_dir: &data_dir,
                    workspace: &workspace,
                    profile_dir: &profile_dir,
                },
                cfg,
            )
            .await;
            return 3;
        }
    };

    // ★The session id and transcript tap are found after the first send (ADR-0226)★: the session id is not persisted before the first submit, so the roster right after spawn is empty. See `RunState::learn_session_id` and `refresh_real_context`; until then it is a character estimate.
    eprintln!("[pilot] the transcript tap is found after the first send — character estimate until then (best-effort)");

    let obs = Arc::new(TurnObserver::new());
    let sink_id = match manager.subscribe(agent.id, obs.clone()) {
        Ok(id) => Some(id),
        Err(e) => {
            eprintln!("subscribe failed: {e}");
            None
        }
    };

    // ── run state ──
    let mut state = RunState::new(None, None);

    // ★Header-first contract★: HeaderRecord is the first line of the file; a write after the first task turn would put a turn ahead of it. So the transcript tap is always absent at header time.
    writer.write(&Record::Header(HeaderRecord {
        claude_version,
        daemon_git_commit: git_commit,
        model_pin: cfg.model.clone(),
        resolved_model: None,
        resolved_model_note: Some(
            "transcript missing (at spawn time — found after the first send) — the real usage and model id are settled by the end-of-run reparse; progress falls back to character estimates"
                .to_string(),
        ),
        transcript_available: false,
        transcript_path: None,
        timestamp_utc: utc_stamp_rfc3339(),
        run_index: run_idx,
        run_id: run_id.clone(),
        config: config_json,
    }));

    // ★Cleanup guaranteed even on the panic path★: unwrapped, a mid-run panic would skip the cleanup below, leaving claude alive and the temp dir behind. Cleanup resources (manager/mcp_handle/paths) stay in this scope; the body moves only state/writer.
    let run_ctx = RunDriveCtx {
        manager: &manager,
        registry: &registry,
        messaging: &messaging,
        agent: &agent,
        obs: &obs,
        delivery_seen: &delivery_seen,
        run_id: &run_id,
        run_started,
        cfg,
    };
    let drive_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        drive_run(&run_ctx, &mut state, &mut writer)
    }));
    let (abort_reason, total_turns, max_ctx) = match drive_result {
        Ok(outcome) => outcome,
        Err(_) => {
            eprintln!("[pilot] run {run_idx} PANICKED — forcing cleanup (finding 8)");
            let panic_ts = state
                .transcript_path
                .as_deref()
                .and_then(transcript::parse_transcript);
            writer.write(&Record::Summary(SummaryRecord {
                max_context_tokens: state.max_context_tokens,
                total_turns: state.turn_idx,
                duration_ms: run_started.elapsed().as_millis() as u64,
                abort_reason: Some("run body panicked".to_string()),
                resolved_model: panic_ts.as_ref().and_then(|ts| ts.resolved_model.clone()),
                transcript_available: state.transcript_path.is_some(),
                transcript_path: state
                    .transcript_path
                    .as_deref()
                    .map(|p| p.display().to_string()),
            }));
            (
                Some("run body panicked".to_string()),
                state.turn_idx,
                state.max_context_tokens,
            )
        }
    };

    if let Some(sid) = sink_id {
        let _ = manager.unsubscribe(agent.id, sid);
    }
    cleanup(
        &manager,
        Some(agent.id),
        mcp_handle,
        &CleanupPaths {
            data_dir: &data_dir,
            workspace: &workspace,
            profile_dir: &profile_dir,
        },
        cfg,
    )
    .await;

    if abort_reason.is_some() {
        eprintln!("[pilot] run {run_idx} aborted: {abort_reason:?}");
        // abort is graceful — reports nonzero upward, but the file is intact.
        return 4;
    }
    eprintln!("[pilot] run {run_idx} done: turns={total_turns} max_ctx={max_ctx}");
    0
}

// ── drive_run — the synchronous run body (turn loop + finalize) ──

struct RunDriveCtx<'a> {
    manager: &'a Arc<AgentManager>,
    registry: &'a Arc<ControlRegistry>,
    messaging: &'a Arc<engram_dashboard_messaging::service::MessagingService>,
    agent: &'a AgentInfo,
    obs: &'a Arc<TurnObserver>,
    delivery_seen: &'a Arc<Mutex<Vec<DeliveryObservation>>>,
    run_id: &'a str,
    run_started: Instant,
    cfg: &'a PilotConfig,
}

/// Every turn (fill · inject · probe · FINAL) must pass this gate right before it — the post-processing phase outside the loop is no exception.
fn cap_gate(ctx: &RunDriveCtx, state: &RunState) -> Result<(), String> {
    if ctx.run_started.elapsed() >= MAX_WALLCLOCK_PER_RUN {
        return Err("MAX_WALLCLOCK_PER_RUN reached".to_string());
    }
    if state.turn_idx >= MAX_TURNS_PER_RUN {
        return Err("MAX_TURNS_PER_RUN reached".to_string());
    }
    Ok(())
}

fn drive_run(
    ctx: &RunDriveCtx,
    state: &mut RunState,
    writer: &mut JsonlWriter,
) -> (Option<String>, u32, u64) {
    let RunDriveCtx {
        manager,
        registry,
        messaging,
        agent,
        obs,
        delivery_seen,
        run_id,
        cfg,
        ..
    } = *ctx;
    let mut abort_reason: Option<String> = None;

    if let Err(r) = cap_gate(ctx, state) {
        abort_reason = Some(r);
    } else {
        let task_prompt = original_task_prompt();
        match drive_turn(
            manager,
            agent.id,
            obs,
            &task_prompt,
            "task",
            0,
            state,
            writer,
        ) {
            TurnResult::Ok => {}
            TurnResult::Stalled => abort_reason = Some("turn 1 (task) stalled".to_string()),
            TurnResult::Terminal => {
                abort_reason = Some("agent terminated during task turn".to_string())
            }
            TurnResult::Error(e) => abort_reason = Some(format!("turn 1 (task) error: {e}")),
        }
    }

    let inject_thresholds: Vec<(u32, f64, u64)> = cfg
        .inject_at
        .iter()
        .enumerate()
        .map(|(k, &frac)| {
            (
                k as u32,
                frac,
                (cfg.fill_target_tokens as f64 * frac) as u64,
            )
        })
        .collect();
    let mut next_inject = 0usize;
    let mut pending_probes: Vec<PendingProbe> = Vec::new();

    // ── Fill + Inject + Probe loop ──
    if abort_reason.is_none() {
        loop {
            if let Err(r) = cap_gate(ctx, state) {
                abort_reason = Some(r);
                break;
            }

            if next_inject < inject_thresholds.len() {
                let (k, frac, threshold) = inject_thresholds[next_inject];
                if state.max_context_tokens >= threshold {
                    match do_injection(
                        manager,
                        registry,
                        messaging,
                        obs,
                        agent.id,
                        agent.epoch,
                        run_id,
                        k,
                        frac,
                        cfg.seed,
                        delivery_seen,
                        state,
                        writer,
                    ) {
                        InjectOutcome::Ok(inj) => {
                            pending_probes.push(PendingProbe {
                                k,
                                remaining_gap: cfg.probe_gap_turns,
                                sender_name: inj.sender_name,
                                msg_id: inj.msg_id,
                                codeword: inj.codeword,
                            });
                            next_inject += 1;
                            // ★An inject turn does not consume the probe gap★: decrementing a waiting probe's gap here would let the just-injected turn and other injections eat it, contradicting `--probe-gap-turns` ("fill turns after injection"). The gap decrement happens only in the fill turn handling below.
                            continue;
                        }
                        InjectOutcome::Abort(r) => {
                            abort_reason = Some(r);
                            break;
                        }
                    }
                }
            }

            if let Some(pos) = pending_probes.iter().position(|p| p.remaining_gap == 0) {
                let probe = pending_probes.remove(pos);
                if let Err(r) = run_probe(manager, agent.id, obs, &probe, state, writer) {
                    abort_reason = Some(r);
                    break;
                }
                continue;
            }

            let fill_target_reached = state.max_context_tokens >= cfg.fill_target_tokens;
            if fill_target_reached
                && next_inject >= inject_thresholds.len()
                && pending_probes.is_empty()
            {
                break;
            }

            state.doc_counter += 1;
            let doc_n = state.doc_counter;
            let body = filler_doc(cfg.seed, doc_n, cfg.doc_chars);
            let prompt = format!("{body}\nreceived {doc_n}?");
            match drive_turn(
                manager, agent.id, obs, &prompt, "fill", doc_n, state, writer,
            ) {
                TurnResult::Ok => {}
                TurnResult::Stalled => {
                    abort_reason = Some(format!("fill turn (doc {doc_n}) stalled"));
                    break;
                }
                TurnResult::Terminal => {
                    abort_reason = Some("agent terminated during fill".to_string());
                    break;
                }
                TurnResult::Error(e) => {
                    abort_reason = Some(format!("fill turn (doc {doc_n}) error: {e}"));
                    break;
                }
            }
            for p in pending_probes.iter_mut() {
                if p.remaining_gap > 0 {
                    p.remaining_gap -= 1;
                }
            }
        }
    }

    // Remaining probes are used up regardless of the gap — recall at the end of the run is measured too.
    if abort_reason.is_none() {
        let leftover: Vec<PendingProbe> = std::mem::take(&mut pending_probes);
        for probe in leftover {
            if let Err(r) = cap_gate(ctx, state) {
                abort_reason = Some(r);
                break;
            }
            if let Err(r) = run_probe(manager, agent.id, obs, &probe, state, writer) {
                abort_reason = Some(r);
                break;
            }
        }
    }

    // ★Forced /compact phase removed — do not revive★: stream-json headless has no interactive slash interception, so a literal `/compact` sent as user text cannot trigger native compaction and gives a misleading observation. Compaction is observed only by capturing the transcript compact-marker of organic native compaction (transcript.compact_marker_lines); the end-of-run authoritative parse already records it as CompactSignal.

    if abort_reason.is_none() {
        match cap_gate(ctx, state) {
            Ok(()) => {
                let final_probe = PendingProbe {
                    k: u32::MAX, // marker: FINAL (no injection).
                    remaining_gap: 0,
                    sender_name: String::new(),
                    msg_id: String::new(),
                    codeword: String::new(),
                };
                if let Err(r) = run_final_report(
                    manager,
                    agent.id,
                    obs,
                    &final_probe,
                    cfg,
                    state.doc_counter,
                    state,
                    writer,
                ) {
                    abort_reason = Some(r);
                }
            }
            Err(r) => abort_reason = Some(r),
        }
    }

    // ★End-of-run authoritative parse (ADR-0090 Fix 1)★: the final truth over the per-turn best-effort tap.
    let final_transcript: Option<TranscriptSummary> = state
        .transcript_path
        .as_deref()
        .and_then(transcript::parse_transcript);

    match &final_transcript {
        Some(ts) => writer.write(&Record::Histogram(HistogramRecord {
            counts: ts.event_histogram.clone(),
            source: "transcript_raw_stream_json_types (session JSONL tap — ADR-0090 Fix 1)"
                .to_string(),
        })),
        None => writer.write(&Record::Histogram(HistogramRecord {
            counts: obs.histogram_snapshot(),
            source: "decoded_output_event_variants (transcript tap absent — fallback)".to_string(),
        })),
    }

    let real_footprints: Option<Vec<u64>> = final_transcript.as_ref().and_then(|ts| {
        if ts.real_usage_series.is_empty() {
            None
        } else {
            Some(
                ts.real_usage_series
                    .iter()
                    .map(|u| u.context_footprint())
                    .collect(),
            )
        }
    });
    let detection_series =
        select_detection_series(real_footprints.as_deref(), &state.estimate_samples);
    let flags = detect_suspected_compaction(&detection_series);
    if !flags.is_empty() {
        writer.write(&Record::SuspectedCompaction(SuspectedCompactionRecord {
            flagged_turn_idxs: flags,
        }));
    }

    if let Some(ts) = &final_transcript {
        for line in &ts.compact_marker_lines {
            writer.write(&Record::CompactSignal(CompactSignalRecord {
                verbatim: cap_response(line),
                source: "transcript_compact_marker".to_string(),
            }));
        }
    }
    for sig in obs.drain_compact_signals() {
        writer.write(&Record::CompactSignal(sig));
    }

    let max_real = final_transcript.as_ref().and_then(|ts| {
        ts.real_usage_series
            .iter()
            .map(|u| u.context_footprint())
            .max()
    });
    let max_ctx = max_real.unwrap_or(state.max_context_tokens);
    // Reflect writer io_errors in abort_reason (makes silent record loss visible).
    if writer.io_errors > 0 && abort_reason.is_none() {
        abort_reason = Some(format!("{} JSONL write/flush errors", writer.io_errors));
    }
    // ★The header's resolved_model is right after spawn (transcript not yet written), so usually None — the model id · tap existence · path settled by the end-of-run parse must also be carried in the summary so the reproducibility pin (ADR-0088 d5a) is not lost.
    let summary_resolved_model = final_transcript
        .as_ref()
        .and_then(|ts| ts.resolved_model.clone());
    let summary_transcript_path = state
        .transcript_path
        .as_deref()
        .map(|p| p.display().to_string());
    writer.write(&Record::Summary(SummaryRecord {
        max_context_tokens: max_ctx,
        total_turns: state.turn_idx,
        duration_ms: ctx.run_started.elapsed().as_millis() as u64,
        abort_reason: abort_reason.clone(),
        resolved_model: summary_resolved_model,
        transcript_available: final_transcript.is_some(),
        transcript_path: summary_transcript_path,
    }));

    (abort_reason, state.turn_idx, max_ctx)
}

// ── wiring ──

struct Wiring {
    manager: Arc<AgentManager>,
    registry: Arc<ControlRegistry>,
    /// 3-way send branch (handed to handle_send).
    messaging: Arc<engram_dashboard_messaging::service::MessagingService>,
    mcp_handle: McpServerHandle,
    /// ★This struct must hold the reaper★ — as a local of `wire()`, the seat table closes the moment the function returns (`CommandDeliveries::drain`) and every later relay of this server is bounced as 「shutting down」. The roster is empty so no relays happen today; that is the trap for whoever runs relays through this pilot (the same shape was already stepped on in three fixtures).
    _relay_sweeper: engram_dashboard_daemon::command_delivery::BusSweeper,
    data_dir: PathBuf,
    /// Per-run profile temp dir — cleanup must delete it too so temp does not leak.
    profile_dir: PathBuf,
}

/// Mirrors the wire() order of control_send.rs.
async fn wire(tag: &str) -> Result<Wiring, String> {
    let registry = Arc::new(ControlRegistry::new());
    let slot = Arc::new(ManagerSlot::new());
    let messaging_slot =
        Arc::new(engram_dashboard_daemon::control::mcp_server::MessagingSlot::new());
    // ★No client to attach★ — the roster notification fanout slot is handed to the table empty (skipping notifications is normal here, ADR-0132). The table itself is filled — left empty, `/control/agent` would only give 503.
    let broadcast_slot = Arc::new(RosterBroadcastSlot::new());
    let command_slot = Arc::new(CommandTableSlot::new());
    // ★The return value holds the reaper★ — bind it here with `_` and the table closes when the function returns (see that field of `Wiring`).
    // ★Attach the stage-1 table through the slot★ — same shape as production `lib.rs`; with `without_commands()` the decider's and the bus's tables differ and `/control/call` would not recognize `agent.*`.
    let (relay_bus, relay_sweeper) = engram_dashboard_daemon::command_delivery::CommandBus::new(
        engram_dashboard_daemon::command_roster::CommandRoster::new(),
        engram_dashboard_daemon::command_delivery::CommandDeliveries::new(),
        Arc::new(
            engram_dashboard_daemon::control::commands::DaemonLocalCommands::new(
                command_slot.clone(),
            ),
        ),
    );
    let handle = start_mcp_server_without_help(
        registry.clone(),
        slot.clone(),
        messaging_slot.clone(),
        command_slot.clone(),
        relay_bus,
    )
    .await
    .map_err(|e| format!("start mcp server: {e}"))?;
    let url = handle.url.clone();
    let data_dir = std::env::temp_dir().join(format!("engram-pilot-{tag}"));

    let control: Arc<dyn ControlChannel> = Arc::new(DaemonControlChannel::new(
        registry.clone(),
        url,
        engram_dashboard_daemon::control::mcp_config::McpDir::new(data_dir.join("mcp-config")),
        None, // send_exe: the pilot calls handle_send directly, so the CLI path is not needed.
    ));

    let sink: Arc<dyn StatusSink> = Arc::new(NoopStatus);
    let profile_dir = std::env::temp_dir().join(format!("engram-pilot-prof-{tag}"));
    let profiles = Arc::new(ProfileRegistry::new(Arc::new(FileProfileStore::new(
        profile_dir.clone(),
    ))));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = Arc::new(AgentManager::new_with_control(
        sink, profiles, tracker, control,
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
    let messaging = Arc::new(
        engram_dashboard_daemon::messaging_host::messaging_for_manager(
            manager.clone(),
            registry.clone(),
        ),
    );
    messaging_slot.set(messaging.clone());

    Ok(Wiring {
        manager,
        registry,
        messaging,
        mcp_handle: handle,
        _relay_sweeper: relay_sweeper,
        data_dir,
        profile_dir,
    })
}

/// Mirror of control_send.rs's spawn_json_agent.
fn spawn_pilot_agent(
    manager: &Arc<AgentManager>,
    workspace: &std::path::Path,
    model: &str,
) -> Option<AgentInfo> {
    let profile = AgentProfile::new(
        format!("pilot-{}", &AgentId::new_v4().to_string()[..8]),
        AgentCommand::Claude {
            // ★Model pin★: `--model` through extra_args (no backend code change — ADR-0090 d3).
            extra_args: vec!["--model".to_string(), model.to_string()],
            output_format: AgentOutputFormat::StreamJson,
        },
        workspace.to_path_buf(),
        vec![],
        false,
    );
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

// ── output observation sink ──

struct NoopStatus;
impl StatusSink for NoopStatus {
    fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
    fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
}

struct DeliveryCapture {
    seen: Arc<Mutex<Vec<DeliveryObservation>>>,
}
impl DeliveryObserver for DeliveryCapture {
    fn observe(&self, obs: DeliveryObservation) {
        self.seen.lock().unwrap().push(obs);
    }
}

struct TurnObserver {
    id: SinkId,
    inner: Mutex<ObserverInner>,
    /// MessageDone count (turn end signal) — the Condvar wait condition.
    done_count: AtomicU64,
    cv: Condvar,
    /// Terminal state reached (agent death) — not the status sink; unused here.
    _reserved: (),
}

#[derive(Default)]
struct ObserverInner {
    response_buf: String,
    /// Latest usage (input/output).
    last_usage: Option<(u64, u64)>,
    /// Histogram of decoded event variants.
    histogram: BTreeMap<String, u64>,
    /// Approximate compact signal ("compact" in Structured/Error text).
    compact_signals: Vec<CompactSignalRecord>,
    /// The non-compaction API error observed in the current turn (if any). Even if MessageDone follows, the turn counts as failed — else the failure hides as abort_reason:null.
    turn_error: Option<String>,
}

impl TurnObserver {
    fn new() -> Self {
        Self {
            id: SinkId::new_v4(),
            inner: Mutex::new(ObserverInner::default()),
            done_count: AtomicU64::new(0),
            cv: Condvar::new(),
            _reserved: (),
        }
    }

    /// ★Load-bearing: the caller MUST wait_turn_end before the next stdin write★ — begin_turn reset, write stdin, fence with that turn's wait_turn_end. Without the fence, the previous turn's late MessageDone releases the next wait early and the reply is attributed to the wrong turn.
    fn begin_turn(&self) {
        let mut g = self.inner.lock().unwrap();
        g.response_buf.clear();
        g.last_usage = None;
        g.turn_error = None;
    }

    fn turn_error(&self) -> Option<String> {
        self.inner.lock().unwrap().turn_error.clone()
    }

    fn done_snapshot(&self) -> u64 {
        self.done_count.load(Ordering::Acquire)
    }

    /// Waits until done_count exceeds `baseline` (= this turn's MessageDone). Returns false on timeout.
    fn wait_turn_end(&self, baseline: u64, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let guard = self.inner.lock().unwrap();
        // done_count is atomic and changes outside the lock, but the Condvar wait sleeps with this lock released, so loop on wait_timeout to not miss a notify (spurious wakes).
        let mut g = guard;
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
        self.inner.lock().unwrap().response_buf.clone()
    }

    fn last_usage(&self) -> Option<(u64, u64)> {
        self.inner.lock().unwrap().last_usage
    }

    fn histogram_snapshot(&self) -> BTreeMap<String, u64> {
        self.inner.lock().unwrap().histogram.clone()
    }

    fn drain_compact_signals(&self) -> Vec<CompactSignalRecord> {
        std::mem::take(&mut self.inner.lock().unwrap().compact_signals)
    }
}

impl OutputSink for TurnObserver {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        let OutputPayload::Event(ev) = frame.payload else {
            // A json agent gets only Events (not TerminalBytes); Bytes is ignored defensively.
            return Ok(());
        };
        let mut g = self.inner.lock().unwrap();
        let key = decoded_variant_key(ev);
        *g.histogram.entry(key).or_insert(0) += 1;

        match ev {
            OutputEvent::TextDelta { text, .. } => {
                g.response_buf.push_str(text);
            }
            OutputEvent::Usage {
                input_tokens,
                output_tokens,
                ..
            } => {
                g.last_usage = Some((*input_tokens, *output_tokens));
            }
            OutputEvent::MessageDone { .. } => {
                // Wake waiters after the atomic increment outside the lock; the lock is held only briefly.
                self.done_count.fetch_add(1, Ordering::Release);
                self.cv.notify_all();
            }
            OutputEvent::Structured { json, .. } => {
                if json.to_ascii_lowercase().contains("compact") {
                    g.compact_signals.push(CompactSignalRecord {
                        verbatim: cap_response(json),
                        source: "decoded_structured".to_string(),
                    });
                }
            }
            OutputEvent::Error(msg) => {
                // ★OutputEvent::Error is always a real error even if its text contains "compact"★ — a substring must not demote an error to a harmless compact signal (real API errors like "compaction failed" were swallowed as success, abort_reason:null).
                if g.turn_error.is_none() {
                    g.turn_error = Some(cap_response(msg));
                }
                if msg.to_ascii_lowercase().contains("compact") {
                    g.compact_signals.push(CompactSignalRecord {
                        verbatim: cap_response(msg),
                        source: "decoded_error (real error; mentions compact — logged for diagnosis, not downgraded)".to_string(),
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn sink_id(&self) -> SinkId {
        self.id
    }
}

/// Decoded OutputEvent → histogram key (variant name), not the raw stream-json type.
fn decoded_variant_key(ev: &OutputEvent) -> String {
    match ev {
        OutputEvent::TerminalBytes(_) => "TerminalBytes",
        OutputEvent::TextDelta { .. } => "TextDelta",
        OutputEvent::ToolCall { .. } => "ToolCall",
        OutputEvent::ToolResult { .. } => "ToolResult",
        OutputEvent::Usage { .. } => "Usage",
        OutputEvent::MessageDone { .. } => "MessageDone",
        OutputEvent::TurnEnd { .. } => "TurnEnd",
        OutputEvent::Error(_) => "Error",
        OutputEvent::Structured { kind, .. } => return format!("Structured/{kind}"),
        OutputEvent::QueuedInput(_) => "QueuedInput",
    }
    .to_string()
}

// ── turn driver ──

enum TurnResult {
    Ok,
    Stalled,
    Terminal,
    /// The turn ended with MessageDone, but a non-compaction API error was observed in between — failure.
    Error(String),
}

/// Character→token ratio (about 4 char/token for English) — a heuristic for fill progress estimates only.
const CHARS_PER_TOKEN_EST: u64 = 4;

struct RunState {
    turn_idx: u32,
    /// Basis for fill progress/injection threshold decisions (= token estimate of the cumulative characters sent).
    max_context_tokens: u64,
    doc_counter: u32,
    estimate_samples: Vec<UsageSample>,
    /// ★Measured★: on the spawn path the decoded `Usage.input_tokens` reports only that turn's incremental input (≈3), so thresholds use the token estimate of cumulative characters sent (controlled · deterministic), not usage.
    cumulative_chars_sent: u64,
    /// Transcript tap path (if any). The file is reparsed each turn for the latest real usage (best-effort). ADR-0008 boundary: only a measurement tap — absent, the harness falls back to the character estimate.
    transcript_path: Option<PathBuf>,
    session_id: Option<String>,
    latest_real_context: Option<u64>,
}
impl RunState {
    fn new(transcript_path: Option<PathBuf>, session_id: Option<String>) -> Self {
        Self {
            turn_idx: 0,
            max_context_tokens: 0,
            doc_counter: 0,
            estimate_samples: Vec::new(),
            cumulative_chars_sent: 0,
            transcript_path,
            session_id,
            latest_real_context: None,
        }
    }

    fn account_context(&mut self, prompt_len: usize) {
        self.cumulative_chars_sent += prompt_len as u64;
        let est = self.cumulative_chars_sent / CHARS_PER_TOKEN_EST;
        if est > self.max_context_tokens {
            self.max_context_tokens = est;
        }
    }

    fn context_estimate(&self) -> u64 {
        (self.cumulative_chars_sent / CHARS_PER_TOKEN_EST).max(self.max_context_tokens)
    }

    /// Called after a send succeeds — the session id is persisted by the first submit (ADR-0226), so it is not in the roster before that.
    fn learn_session_id(&mut self, manager: &AgentManager, agent_id: AgentId) {
        if self.session_id.is_none() {
            self.session_id = manager
                .agent_backend_session_id(agent_id)
                .map(|s| s.to_string());
        }
    }

    fn refresh_real_context(&mut self) -> Option<u64> {
        if self.transcript_path.is_none() {
            if let Some(sid) = &self.session_id {
                self.transcript_path = transcript::locate_transcript(sid);
            }
        }
        let path = self.transcript_path.as_deref()?;
        if let Some(summary) = transcript::parse_transcript(path) {
            if let Some(last) = summary.real_usage_series.last() {
                self.latest_real_context = Some(last.context_footprint());
            }
        }
        self.latest_real_context
    }

    fn usage_snapshot(&self, decoded: Option<(u64, u64)>) -> UsageSnapshot {
        let (input, output) = decoded.unwrap_or((0, 0));
        UsageSnapshot {
            input_tokens: input,
            output_tokens: output,
            context_tokens_real: self.latest_real_context,
            context_tokens_estimate: self.context_estimate(),
        }
    }

    fn push_estimate_sample(&mut self, harness_reset: bool) {
        self.estimate_samples.push(UsageSample {
            turn_idx: self.turn_idx,
            context_tokens: self.context_estimate(),
            harness_reset,
        });
    }
}

struct PendingProbe {
    k: u32,
    remaining_gap: u32,
    sender_name: String,
    msg_id: String,
    codeword: String,
}

#[allow(clippy::too_many_arguments)]
fn drive_turn(
    manager: &Arc<AgentManager>,
    agent_id: AgentId,
    obs: &Arc<TurnObserver>,
    prompt: &str,
    kind: &str,
    doc_n: u32,
    state: &mut RunState,
    writer: &mut JsonlWriter,
) -> TurnResult {
    obs.begin_turn();
    let baseline = obs.done_snapshot();
    let t0 = Instant::now();

    // User turn send = write_stdin (the session wraps it with wrap_user_turn). ADR-0231: the pilot is not a person — `Mail`.
    if manager
        .write_stdin(agent_id, prompt.as_bytes(), InputOrigin::Mail)
        .is_err()
    {
        return TurnResult::Terminal;
    }
    state.learn_session_id(manager, agent_id);

    let ended = obs.wait_turn_end(baseline, TURN_WAIT_CAP);
    let wallclock_ms = t0.elapsed().as_millis() as u64;

    if !ended {
        let alive = manager.list_agents().iter().any(|a| a.id == agent_id);
        writer.write(&Record::Stall(StallRecord {
            turn_idx: state.turn_idx,
            reason: if alive {
                "turn wait cap exceeded".to_string()
            } else {
                "agent terminated before turn end".to_string()
            },
            waited_ms: wallclock_ms,
        }));
        return if alive {
            TurnResult::Stalled
        } else {
            TurnResult::Terminal
        };
    }

    state.account_context(prompt.len());
    state.refresh_real_context();

    // Always write 1 record so turns with only measured values and no decoded usage stay in the series.
    let usage = Some(state.usage_snapshot(obs.last_usage()));
    state.push_estimate_sample(false);

    writer.write(&Record::Turn(TurnRecord {
        idx: state.turn_idx,
        kind: kind.to_string(),
        chars_sent: prompt.len(),
        body_sha256: sha256_hex(prompt.as_bytes()),
        usage,
        wallclock_ms,
    }));
    let _ = doc_n; // the doc number can already be checked through the sha — not written separately to the record (the no-original-text invariant).
    state.turn_idx += 1;

    // turn_idx was already raised, so this turn counts as consumed — caps and series stay consistent.
    if let Some(e) = obs.turn_error() {
        return TurnResult::Error(e);
    }
    TurnResult::Ok
}

struct InjectionMeta {
    sender_name: String,
    msg_id: String,
    codeword: String,
}

/// An injection is a first-class turn too, so a stall/failure must abort the run (the next probe must not be misattributed with partial state).
enum InjectOutcome {
    Ok(InjectionMeta),
    Abort(String),
}

#[allow(clippy::too_many_arguments)]
fn do_injection(
    manager: &Arc<AgentManager>,
    registry: &Arc<ControlRegistry>,
    messaging: &Arc<engram_dashboard_messaging::service::MessagingService>,
    obs: &Arc<TurnObserver>,
    agent_id: AgentId,
    agent_epoch: u32,
    run_id: &str,
    k: u32,
    at_fraction: f64,
    seed: u64,
    delivery_seen: &Arc<Mutex<Vec<DeliveryObservation>>>,
    state: &mut RunState,
    writer: &mut JsonlWriter,
) -> InjectOutcome {
    // Issue the sender identity (the control_send.rs pattern — a registered sender token).
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, format!("pilot-sender-{k}-{run_id}"), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };
    let sender_name = sender.to_string()[..8].to_string(); // no profile → fallback to the first 8 chars of the id.

    // ★Framing★: an imperative "report ..." wording was mistaken for an injection and refused — so it holds the recall target (codeword) but is framed as a friendly memo (no command). Recall is requested by the later probe question.
    let codeword = codeword_for(seed, k);
    let body = format!(
        "Just a friendly note to keep in mind for later: our shared codeword for this session is {codeword} (reference {run_id}-{k}). No action needed right now — thanks!"
    );
    let body_len = body.len();
    let body_sha256 = sha256_hex(body.as_bytes());

    // Recipient target = the exact AgentId string (robust regardless of the spawn name).
    let to = agent_id.to_string();

    obs.begin_turn();
    let baseline = obs.done_snapshot();
    let t0 = Instant::now();

    let before = delivery_seen.lock().unwrap().len();
    let result = handle_send(
        manager,
        registry,
        messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![to],
            body,
            contract: Default::default(),
        },
    );
    let v = result.to_json();
    let msg_id = v["id"].as_str().unwrap_or("").to_string();

    // Delivery observation (1 new record on success).
    let delivery = {
        let g = delivery_seen.lock().unwrap();
        g.get(before).cloned()
    };
    let (delivered, bytes_requested, bytes_written, to_epoch, error) = match &delivery {
        Some(o) => (
            o.is_delivered(),
            o.bytes_requested,
            o.bytes_written,
            o.to_epoch,
            o.error.clone(),
        ),
        // With no observation record, decide by the response `results[]`.
        //   ★Look at the row status★: `results` holds failed rows too (ADR-0111 partial progress), so deciding by existence would record a send that reached nobody as delivered and pollute the pilot data.
        None => (
            v.get("results")
                .and_then(|r| r.as_array())
                .is_some_and(|rows| {
                    rows.iter().any(|r| {
                        matches!(r.get("status").and_then(|s| s.as_str()), Some("delivered"))
                    })
                }),
            0,
            None,
            None,
            None,
        ),
    };

    writer.write(&Record::Injection(InjectionRecord {
        k,
        at_fraction,
        msg_id: msg_id.clone(),
        codeword: codeword.clone(),
        sender_name: sender_name.clone(),
        delivered,
        bytes_requested,
        bytes_written,
        to_epoch,
        error: error.clone(),
    }));
    let _ = agent_epoch; // no epoch pin (ADR-0086 F5/ADR-0089) — to_epoch is observation only.

    // ★On a delivery failure no real user turn went into stdin★ — waiting for wait_turn_end would time out in vain, so skip the fencing.
    if !delivered {
        // ★Turn-index continuity contract★: every path that consumes turn_idx leaves a TurnRecord, failure paths included — else the index sequence gets holes and the index↔record mapping breaks.
        writer.write(&Record::Turn(TurnRecord {
            idx: state.turn_idx,
            kind: "inject".to_string(),
            chars_sent: body_len,
            body_sha256: body_sha256.clone(),
            usage: None,
            wallclock_ms: t0.elapsed().as_millis() as u64,
        }));
        state.turn_idx += 1;
        return InjectOutcome::Abort(format!(
            "injection k={k} not delivered: {}",
            error.unwrap_or_else(|| v["code"].as_str().unwrap_or("unknown").to_string())
        ));
    }

    let ended = obs.wait_turn_end(baseline, TURN_WAIT_CAP);
    let wallclock_ms = t0.elapsed().as_millis() as u64;
    if !ended {
        let alive = manager.list_agents().iter().any(|a| a.id == agent_id);
        writer.write(&Record::Stall(StallRecord {
            turn_idx: state.turn_idx,
            reason: if alive {
                "injection turn wait cap exceeded".to_string()
            } else {
                "agent terminated during injection turn".to_string()
            },
            waited_ms: wallclock_ms,
        }));
        writer.write(&Record::Turn(TurnRecord {
            idx: state.turn_idx,
            kind: "inject".to_string(),
            chars_sent: body_len,
            body_sha256: body_sha256.clone(),
            usage: None,
            wallclock_ms,
        }));
        state.turn_idx += 1;
        return InjectOutcome::Abort(format!("injection k={k} turn did not complete"));
    }

    state.account_context(bytes_requested);
    state.refresh_real_context();
    state.push_estimate_sample(false);

    writer.write(&Record::Turn(TurnRecord {
        idx: state.turn_idx,
        kind: "inject".to_string(),
        chars_sent: body_len,
        body_sha256: body_sha256.clone(),
        usage: Some(state.usage_snapshot(obs.last_usage())),
        wallclock_ms,
    }));
    state.turn_idx += 1;

    if let Some(e) = obs.turn_error() {
        return InjectOutcome::Abort(format!("injection k={k} turn error: {e}"));
    }

    InjectOutcome::Ok(InjectionMeta {
        sender_name,
        msg_id,
        codeword,
    })
}

fn run_probe(
    manager: &Arc<AgentManager>,
    agent_id: AgentId,
    obs: &Arc<TurnObserver>,
    probe: &PendingProbe,
    state: &mut RunState,
    writer: &mut JsonlWriter,
) -> Result<(), String> {
    // ★Pattern break (ADR-0090)★: the probe got stuck in the fill pattern and returned only `received N` (measuring pattern lock, not recall). So the probe wording explicitly breaks the pattern ("stop the document exercise for this one answer only · do not answer with `received N`") and asks for sender/message id/codeword in complete sentences.
    let question =
        "Let's PAUSE the document exercise for just this one reply — please do NOT answer \
         `received N` this time. Earlier in this same session, a friendly note from another \
         participant was passed along to you, and it carried a shared codeword. In one or two \
         complete sentences, could you tell me: (a) who the note was from (their name or short id), \
         (b) the reference id that came with it, and (c) the exact codeword it contained? \
         Afterwards we'll resume the document exercise as normal. Thanks!"
            .to_string();
    let out = send_and_collect(manager, agent_id, obs, &question, state, "probe", writer);

    // body is the probe question (experiment meta, not the original task filler, so recording sha256+len is OK).
    writer.write(&Record::Turn(TurnRecord {
        idx: out.turn_idx,
        kind: "probe".to_string(),
        chars_sent: question.len(),
        body_sha256: sha256_hex(question.as_bytes()),
        usage: out.usage,
        wallclock_ms: out.wallclock_ms,
    }));

    let scores = score_probe(
        &out.response,
        &probe.sender_name,
        &probe.msg_id,
        &probe.codeword,
        false,
        0,
        "",
    );
    writer.write(&Record::Probe(ProbeRecord {
        for_injection_k: Some(probe.k),
        turn_idx: out.turn_idx,
        usage: out.usage,
        question,
        response: cap_response(&out.response),
        final_report: false,
        scores,
    }));
    match out.abort {
        Some(r) => Err(r),
        None => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_final_report(
    manager: &Arc<AgentManager>,
    agent_id: AgentId,
    obs: &Arc<TurnObserver>,
    _probe: &PendingProbe,
    cfg: &PilotConfig,
    doc_count: u32,
    state: &mut RunState,
    writer: &mut JsonlWriter,
) -> Result<(), String> {
    let question =
        "That's the end of the document exercise — thank you! Now please PAUSE the `received N` \
         pattern for this final answer. In one full sentence, could you give me the FINAL REPORT: \
         the total number of documents you received during this session, and the exact title of \
         DOC-1 (the first document)?"
            .to_string();
    let out = send_and_collect(manager, agent_id, obs, &question, state, "final", writer);

    writer.write(&Record::Turn(TurnRecord {
        idx: out.turn_idx,
        kind: "final".to_string(),
        chars_sent: question.len(),
        body_sha256: sha256_hex(question.as_bytes()),
        usage: out.usage,
        wallclock_ms: out.wallclock_ms,
    }));

    let expected_title = doc_title(cfg.seed, 1);
    let scores = score_probe(&out.response, "", "", "", true, doc_count, &expected_title);
    writer.write(&Record::Probe(ProbeRecord {
        for_injection_k: None,
        turn_idx: out.turn_idx,
        usage: out.usage,
        question,
        response: cap_response(&out.response),
        final_report: true,
        scores,
    }));
    match out.abort {
        Some(r) => Err(r),
        None => Ok(()),
    }
}

/// Returns more than a String so a timeout or write failure no longer completes silently as partial text (turn_idx not incremented · stall not recorded · abort not gated).
struct CollectOutcome {
    response: String,
    /// If Some, the caller must abort the run with this reason (so a late result cannot silently complete the next wait).
    abort: Option<String>,
    /// The value before send_and_collect raises turn_idx = this turn's index.
    turn_idx: u32,
    /// This turn's usage snapshot. None on stall/write-fail.
    usage: Option<UsageSnapshot>,
    /// This turn's real wall-clock ms (no fabricated 0); on stall/write-fail, the ms waited.
    wallclock_ms: u64,
}

/// Does not write the turn record (the caller wraps it) but updates the usage sample/turn_idx.
fn send_and_collect(
    manager: &Arc<AgentManager>,
    agent_id: AgentId,
    obs: &Arc<TurnObserver>,
    prompt: &str,
    state: &mut RunState,
    kind: &str,
    writer: &mut JsonlWriter,
) -> CollectOutcome {
    obs.begin_turn();
    let baseline = obs.done_snapshot();
    let t0 = Instant::now();

    if manager
        .write_stdin(agent_id, prompt.as_bytes(), InputOrigin::Mail)
        .is_err()
    {
        let elapsed_ms = t0.elapsed().as_millis() as u64;
        let this_idx = state.turn_idx;
        writer.write(&Record::Stall(StallRecord {
            turn_idx: state.turn_idx,
            reason: "write_stdin failed (agent gone)".to_string(),
            waited_ms: elapsed_ms,
        }));
        state.turn_idx += 1;
        return CollectOutcome {
            response: String::new(),
            abort: Some(format!("{kind} turn write failed")),
            turn_idx: this_idx,
            usage: None,
            wallclock_ms: elapsed_ms,
        };
    }
    state.learn_session_id(manager, agent_id);

    let ended = obs.wait_turn_end(baseline, TURN_WAIT_CAP);
    let waited_ms = t0.elapsed().as_millis() as u64;
    if !ended {
        let this_idx = state.turn_idx;
        let alive = manager.list_agents().iter().any(|a| a.id == agent_id);
        writer.write(&Record::Stall(StallRecord {
            turn_idx: state.turn_idx,
            reason: if alive {
                format!("{kind} turn wait cap exceeded")
            } else {
                format!("{kind} agent terminated before turn end")
            },
            waited_ms,
        }));
        let response = obs.response_text(); // returns with the partial response too (for diagnosis).
        state.turn_idx += 1;
        return CollectOutcome {
            response,
            abort: Some(format!("{kind} turn stalled")),
            turn_idx: this_idx,
            usage: None,
            wallclock_ms: waited_ms,
        };
    }

    state.account_context(prompt.len());
    state.refresh_real_context();
    state.push_estimate_sample(false);
    let this_idx = state.turn_idx;
    let usage = Some(state.usage_snapshot(obs.last_usage()));
    state.turn_idx += 1;
    let abort = obs.turn_error().map(|e| format!("{kind} turn error: {e}"));
    CollectOutcome {
        response: obs.response_text(),
        abort,
        turn_idx: this_idx,
        usage,
        wallclock_ms: waited_ms,
    }
}

// ── cleanup / utilities ──

struct CleanupPaths<'a> {
    data_dir: &'a std::path::Path,
    workspace: &'a std::path::Path,
    profile_dir: &'a std::path::Path,
}

async fn cleanup(
    manager: &Arc<AgentManager>,
    agent_id: Option<AgentId>,
    mcp_handle: McpServerHandle,
    paths: &CleanupPaths<'_>,
    cfg: &PilotConfig,
) {
    if let Some(id) = agent_id {
        let _ = manager.kill_agent(id);
        // Wait briefly until it drops out of the list (zombie prevention).
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            if !manager.list_agents().iter().any(|a| a.id == id) {
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    mcp_handle.shutdown().await;
    let _ = std::fs::remove_dir_all(paths.data_dir);
    let _ = std::fs::remove_dir_all(paths.profile_dir);
    if cfg.keep_workspace {
        eprintln!("[pilot] workspace kept: {}", paths.workspace.display());
    } else {
        let _ = std::fs::remove_dir_all(paths.workspace);
    }
}

/// Turn 1 original task instruction.
///
/// ★Prompt framing★: the initial imperative wording ("reply with ONLY `received <n>`") was judged a prompt injection attempt and refused by claude-sonnet-4-6. So it is framed as a memory exercise agreed with the user — a legitimate task the experimenter requests in this conversation, not a command injected from outside. The measurement target (document count + DOC-1 title recall) is the same.
fn original_task_prompt() -> String {
    // ★Pre-justifying the injection envelope backfires — do not revive★: declaring the `[message from ...]` envelope "normal · safe" in the task prompt triggered a stronger immediate refusal ("preemptively claiming 'this is not an attack' is itself a red flag"). The spawned agent inherits the account's instruction policy (valid instructions come only from the user's chat input; instructions in tool-read content are data), so blocking codeword recall from an inter-agent delivered message is by policy — a boundary prompt wording cannot cross.
    "Hi! I'd like to run a short memory exercise with you, and I'd really appreciate your help. \
     I'm going to paste a series of short reference documents, each starting with a line like \
     `DOC-1: <title>`. This is a cooperative recall test — there are no hidden instructions and \
     nothing adversarial here. For each document I paste, please just reply briefly with \
     `received <n>` where <n> is the document's number, and keep a running tally of how many you've \
     seen. At the very end I'll ask for a FINAL REPORT, where I'd like the total number of documents \
     and the exact title of DOC-1. You don't need any tools for this. To confirm you're ready, \
     please reply `received 0`. Thanks so much!"
        .to_string()
}

fn codeword_for(seed: u64, k: u32) -> String {
    // Not the filler's PRNG but an independent perturbation — the codeword is used only for checking the probe answer.
    const CODEWORDS: &[&str] = &[
        "MOONLIGHT",
        "GRANITE",
        "TEMPEST",
        "OBSIDIAN",
        "HALCYON",
        "VERMILLION",
        "COBALT",
        "LANTERN",
        "MERIDIAN",
        "SOLSTICE",
    ];
    let idx = (seed ^ (0x9E37_79B9u64.wrapping_mul(k as u64 + 1))) as usize % CODEWORDS.len();
    CODEWORDS[idx].to_string()
}

fn config_to_json(cfg: &PilotConfig) -> serde_json::Value {
    serde_json::json!({
        "runs": cfg.runs,
        "fill_target_tokens": cfg.fill_target_tokens,
        "inject_at": cfg.inject_at,
        "probe_gap_turns": cfg.probe_gap_turns,
        "doc_chars": cfg.doc_chars,
        "model": cfg.model,
        "seed": cfg.seed,
        "keep_workspace": cfg.keep_workspace,
    })
}

/// Captures the `claude --version` string (best-effort). On Windows it goes through cmd /c (shim resolution).
fn capture_claude_version() -> Option<String> {
    let output = if cfg!(windows) {
        std::process::Command::new("cmd")
            .args(["/c", "claude", "--version"])
            .output()
    } else {
        std::process::Command::new("claude")
            .arg("--version")
            .output()
    };
    match output {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
        _ => None,
    }
}

fn capture_git_commit() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if output.status.success() {
        let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    }
}

fn default_out_dir(stamp: &str) -> PathBuf {
    PathBuf::from("target")
        .join("experiments")
        .join(format!("pilot-{stamp}"))
}

fn utc_stamp_compact() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

/// Approximate RFC3339 UTC string (second precision) — epoch seconds + Z notation, since an exact calendar conversion needs chrono etc. (no new deps). Post-hoc analysis converts the seconds.
fn utc_stamp_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("epoch:{secs}Z")
}

/// One line per record + flush — the flush keeps the partial file intact even on abort.
///
/// The file is opened with truncate (create+write), not append: appending mixed two runs' records in run-0.jsonl when `--out` was reused. The header-first contract is kept by the caller writing the HeaderRecord first.
/// Write/flush errors are counted cumulatively, not ignored — the count before the summary diagnoses lost records (e.g. full disk).
struct JsonlWriter {
    file: std::fs::File,
    io_errors: u64,
}
impl JsonlWriter {
    fn create(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        Ok(Self { file, io_errors: 0 })
    }
    fn write(&mut self, rec: &Record) {
        let line = rec.to_jsonl_line();
        self.write_raw_line(&line);
    }

    /// Writes an arbitrary raw line + flush. Only for the setup-abort marker before the header (wiring_failed) — a diagnostic line outside the schema, not a formal record (accepted residual).
    fn write_raw_line(&mut self, line: &str) {
        if writeln!(self.file, "{line}").is_err() {
            self.io_errors += 1;
            return;
        }
        if self.file.flush().is_err() {
            self.io_errors += 1;
        }
    }
}
