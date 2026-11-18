//! argv golden of Noop launch materials — pins today's `backend::build_command_spec` result (program · args · env · cwd) per backend ·
//! mode · resume · control-channel shape. A spawn without launch materials must produce argv byte-identical to this
//! (TRD S25 §1-5). Regenerate with `regenerate_launch_argv_golden` only for an intended change.
//!
//! ★Measures argv only (calls `build_command_spec` directly)★ — whether the manager's spawn paths (activation · wake-up · on-the-spot
//! creation) produce the same argv is checked against this golden by the manager-level Noop regression (`src/manager/prelaunch.rs` tests) (TRD S25 §5-2 4).
//!
//! The two things the OS decides are written OS-neutral in the golden — console wrapping (the `console` field · re-wrapped with platform
//! `console_command` when comparing) and the PATH separator (written as `;`). Fixed paths use only `/`, the same text on both OSes. So
//! regeneration works only on the OS where wrapping is visible (Windows).
// ADR-0334

use std::path::PathBuf;

use engram_dashboard_agent::backend;
use engram_dashboard_agent::commands::NEW_AGENT_OUTPUT_FORMAT;
use engram_dashboard_agent::launch::LaunchArtifacts;
use engram_dashboard_agent::profile::{AgentCommand, AgentOutputFormat, SpawnMode};
use engram_dashboard_agent::types::{CommandSpec, ControlEndpoint, ToolGrant};
use engram_dashboard_platform::shell::console_command;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const GOLDEN: &str = include_str!("fixtures/launch_argv_golden.json");
const GOLDEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/launch_argv_golden.json"
);

const FRESH_SID: Uuid = Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1111);
const STORED_SID: Uuid = Uuid::from_u128(0x2222_2222_2222_4222_8222_2222_2222_2222);
const AGENT_ID: Uuid = Uuid::from_u128(0x3333_3333_3333_4333_8333_3333_3333_3333);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct GoldenCase {
    case: String,
    console: bool,
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    cwd: String,
}

/// (case name, command, per control-channel shape (name suffix, shape)) — the shape without a control channel (`no-endpoint`) is taken by every command separately.
type Command = (
    &'static str,
    AgentCommand,
    Vec<(&'static str, ControlEndpoint)>,
);

fn commands() -> Vec<Command> {
    let strings = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let claude_endpoints = || {
        vec![
            ("endpoint", full_endpoint()),
            ("endpoint-mcp-grant-only", claude_mcp_grant_only_endpoint()),
        ]
    };
    vec![
        (
            "claude-terminal",
            AgentCommand::Claude {
                extra_args: strings(&["--model", "golden-model"]),
                output_format: AgentOutputFormat::Terminal,
            },
            claude_endpoints(),
        ),
        (
            "claude-json",
            AgentCommand::Claude {
                extra_args: strings(&["--model", "golden-model"]),
                output_format: AgentOutputFormat::StreamJson,
            },
            claude_endpoints(),
        ),
        (
            "codex-terminal",
            AgentCommand::Codex {
                extra_args: strings(&["-c", "model=golden-model"]),
                output_format: AgentOutputFormat::Terminal,
            },
            vec![("endpoint", codex_endpoint())],
        ),
        (
            "codex-json",
            AgentCommand::Codex {
                extra_args: strings(&["-c", "model=golden-model"]),
                output_format: AgentOutputFormat::StreamJson,
            },
            vec![("endpoint", codex_endpoint())],
        ),
        // shell gets no control channel in production (`supports_control_channel` = false, so the daemon does not provision one).
        //   This `endpoint` shape is not a production shape but a marker guarding 「shell ignores the control channel」.
        (
            "shell",
            AgentCommand::Shell {
                program: "golden-shell".to_string(),
                args: strings(&["--login", "-i"]),
            },
            vec![("endpoint", full_endpoint())],
        ),
    ]
}

/// The command shape of the on-the-spot creation paths (wire `SpawnByCwd` — `commands::create_and_start` → `AgentManager::create_and_activate`
/// Fresh · smoke bin — a profile not in the roster spawned Fresh with `spawn_agent`). The env is
/// empty, so only the `no-endpoint` control-channel shape is generated — when a control channel puts the CLI entry on PATH it reads this
/// process's PATH, which is not deterministic.
// TRD S25 §5-2 4 「put those two paths in the golden」
fn spawn_path_commands() -> Vec<(&'static str, AgentCommand)> {
    vec![
        // The default command of `commands::create_and_start` — the received backend · the common default mode · no args.
        (
            "spawn-by-cwd/claude",
            AgentCommand::Claude {
                extra_args: Vec::new(),
                output_format: NEW_AGENT_OUTPUT_FORMAT,
            },
        ),
        (
            "spawn-by-cwd/codex",
            AgentCommand::Codex {
                extra_args: Vec::new(),
                output_format: NEW_AGENT_OUTPUT_FORMAT,
            },
        ),
        // Harness bins (`saturation_pilot` etc.) — claude JSON carrying `--model <m>`.
        (
            "smoke-bin/claude-json",
            AgentCommand::Claude {
                extra_args: vec!["--model".to_string(), "golden-model".to_string()],
                output_format: AgentOutputFormat::StreamJson,
            },
        ),
    ]
}

fn mcp_send_grant() -> ToolGrant {
    ToolGrant::Mcp {
        server: "engram".to_string(),
        tool: "eg_send".to_string(),
    }
}

/// Every field filled. The CLI grant is not in production's claude branch (a backend that takes the MCP config gets no
/// CLI grant — the daemon's `build_grants`).
fn full_endpoint() -> ControlEndpoint {
    ControlEndpoint {
        agent_id: AGENT_ID,
        epoch: 7,
        url: "http://127.0.0.1:47000/mcp".to_string(),
        token: "golden-token".to_string(),
        config_path: Some(PathBuf::from("/golden/mcp/agent.mcp.json")),
        send_exe: Some(PathBuf::from("/golden/bin/engram.exe")),
        mail_allowed: false,
        grants: vec![
            mcp_send_grant(),
            ToolGrant::Cli {
                exe: "engram".to_string(),
            },
        ],
    }
}

/// The shape the daemon gives claude — the MCP send grant only (the daemon's `control/mod.rs` provision).
fn claude_mcp_grant_only_endpoint() -> ControlEndpoint {
    ControlEndpoint {
        grants: vec![mcp_send_grant()],
        ..full_endpoint()
    }
}

/// The shape the daemon gives codex — a backend that takes no file (`writes_mcp_config_file` = false), so the config
/// path is empty.
fn codex_endpoint() -> ControlEndpoint {
    ControlEndpoint {
        config_path: None,
        grants: vec![mcp_send_grant()],
        ..full_endpoint()
    }
}

// PATH is set for determinism — without it the CLI entry injection reads this process's PATH.
fn profile_env() -> Vec<(String, String)> {
    let path = std::env::join_paths(["/golden/sys", "/golden/tools"])
        .expect("fixed PATH")
        .into_string()
        .expect("UTF-8");
    vec![
        ("PATH".to_string(), path),
        ("GOLDEN_FLAG".to_string(), "1".to_string()),
    ]
}

/// The shape the manager passes per case as is (the two session id fields are split by backend-axis dispatch).
fn actual_cases() -> Vec<(String, CommandSpec)> {
    let mut out = Vec::new();
    for (name, command, endpoints) in commands() {
        for (mode_name, mode) in [("fresh", SpawnMode::Fresh), ("resume", SpawnMode::Resume)] {
            let controls = std::iter::once(("no-endpoint", None))
                .chain(endpoints.iter().map(|(n, ep)| (*n, Some(ep.clone()))));
            for (endpoint_name, control) in controls {
                let session_id = backend::assigns_session_id(&command).then_some(match mode {
                    SpawnMode::Fresh => FRESH_SID,
                    SpawnMode::Resume => STORED_SID,
                });
                let resume_session_id = (mode == SpawnMode::Resume
                    && backend::can_resume_stored_session(&command))
                .then_some(STORED_SID);
                let spec = backend::build_command_spec(
                    &command,
                    mode,
                    session_id,
                    resume_session_id,
                    PathBuf::from("/golden/work"),
                    profile_env(),
                    control,
                    &LaunchArtifacts::default(),
                );
                out.push((format!("{name}/{mode_name}/{endpoint_name}"), spec));
            }
        }
    }
    for (name, command) in spawn_path_commands() {
        let spec = backend::build_command_spec(
            &command,
            SpawnMode::Fresh,
            backend::assigns_session_id(&command).then_some(FRESH_SID),
            None,
            PathBuf::from("/golden/work"),
            Vec::new(),
            None,
            &LaunchArtifacts::default(),
        );
        out.push((format!("{name}/fresh/no-endpoint"), spec));
    }
    out
}

/// The console wrapping prefix — `Some((wrapping program, leading args))` only on an OS where `console_command` wraps.
fn console_wrapper() -> Option<(String, Vec<String>)> {
    const PROBE: &str = "launch-golden-probe";
    let (program, mut args) = console_command(PROBE, Vec::new());
    if program == PROBE {
        return None;
    }
    assert_eq!(
        args.pop().as_deref(),
        Some(PROBE),
        "the end of the wrapped form = the wrapped program"
    );
    Some((program, args))
}

fn unwrap_console(program: &str, args: &[String]) -> (bool, String, Vec<String>) {
    if let Some((wrapper, head)) = console_wrapper() {
        if program == wrapper && args.len() > head.len() && args[..head.len()] == head[..] {
            return (
                true,
                args[head.len()].clone(),
                args[head.len() + 1..].to_vec(),
            );
        }
    }
    (false, program.to_string(), args.to_vec())
}

fn neutral_env(env: &[(String, String)]) -> Vec<(String, String)> {
    env.iter()
        .map(|(k, v)| {
            if k.eq_ignore_ascii_case("PATH") {
                let parts: Vec<String> = std::env::split_paths(v)
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                (k.clone(), parts.join(";"))
            } else {
                (k.clone(), v.clone())
            }
        })
        .collect()
}

fn record(case: &str, spec: &CommandSpec) -> GoldenCase {
    let (console, program, args) = unwrap_console(&spec.program, &spec.args);
    GoldenCase {
        case: case.to_string(),
        console,
        program,
        args,
        env: neutral_env(&spec.env),
        cwd: spec.cwd.to_string_lossy().into_owned(),
    }
}

fn matches(golden: &GoldenCase, spec: &CommandSpec) -> bool {
    let (program, args) = if golden.console {
        console_command(&golden.program, golden.args.clone())
    } else {
        (golden.program.clone(), golden.args.clone())
    };
    program == spec.program
        && args == spec.args
        && golden.env == neutral_env(&spec.env)
        && golden.cwd == spec.cwd.to_string_lossy()
}

fn render() -> String {
    let cases: Vec<GoldenCase> = actual_cases()
        .iter()
        .map(|(name, spec)| record(name, spec))
        .collect();
    serde_json::to_string_pretty(&cases).expect("serialize") + "\n"
}

#[test]
fn noop_argv_matches_the_golden() {
    let golden: Vec<GoldenCase> = serde_json::from_str(GOLDEN).expect("the golden is JSON");
    let actual = actual_cases();
    assert_eq!(
        golden.iter().map(|g| g.case.as_str()).collect::<Vec<_>>(),
        actual.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        "the golden's case list"
    );
    let drift: Vec<String> = golden
        .iter()
        .zip(&actual)
        .filter(|(g, (_, spec))| !matches(g, spec))
        .map(|(_, (name, spec))| serde_json::to_string_pretty(&record(name, spec)).unwrap())
        .collect();
    assert!(
        drift.is_empty(),
        "Noop argv differs from the golden — the launch-material port must not change a Noop spawn by a single byte. If the change is \
         intended, regenerate with `regenerate_launch_argv_golden`. Current value:\n{}",
        drift.join("\n")
    );
}

#[test]
#[ignore = "regenerates the golden — run only when Noop argv was changed on purpose"]
fn regenerate_launch_argv_golden() {
    assert!(
        console_wrapper().is_some(),
        "regenerate on the OS where console wrapping is visible (Windows) — otherwise the `console` field cannot be generated"
    );
    let text = render();
    assert_eq!(text, render(), "two generations give the same bytes");
    std::fs::write(GOLDEN_PATH, text).expect("write the golden");
}
