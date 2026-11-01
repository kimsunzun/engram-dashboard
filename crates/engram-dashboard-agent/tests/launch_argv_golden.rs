//! Noop 실행 재료의 argv 골든 — 오늘의 `backend::build_command_spec` 결과(프로그램 · 인자 · env · cwd)를 백엔드 ·
//! 모드 · 이어받기 · 제어 채널 모양마다 못 박는다. 실행 재료가 없는 스폰의 argv 는 이것과 바이트 같아야 한다
//! (TRD S25 §1-5). 일부러 바꿀 때만 `regenerate_launch_argv_golden` 으로 다시 뜬다.
//!
//! ★argv 만 잰다(`build_command_spec` 을 직접 부른다)★ — manager 의 스폰 길(활성화 · 깨우기 · 즉석 생성)이 같은
//! argv 를 내는지는 manager 수준 Noop 회귀(`src/manager/prelaunch.rs` 시험)가 이 골든에 견준다(TRD S25 §5-2 4).
//!
//! OS 가 가르는 둘은 골든에 OS 중립으로 적는다 — 콘솔 감싸기(`console` 칸 · 견줄 때 platform `console_command` 로
//! 다시 감싼다)와 PATH 구분자(`;` 로 적는다). 고정 경로는 `/` 로만 적어 두 OS 에서 같은 글자다. 그래서 다시 뜨기는
//! 감싸기가 보이는 OS(Windows)에서만 된다.
// ADR-0334

use std::path::{Path, PathBuf};

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
// codex 터미널은 프라이밍 파일의 **내용**을 argv 에 싣는다 — 그래서 그 경우만 실제 파일을 준다(경로는 argv 에 안 실린다).
const PRIMING_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/launch_golden_priming.md"
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

/// (경우 이름, 명령, 제어 채널 꼴마다 (이름 꼬리, 모양)) — 제어 채널 없는 꼴(`no-endpoint`)은 모든 명령이 따로 받는다.
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
            ("endpoint-no-priming-no-settings", claude_bare_endpoint()),
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
        // shell 은 운영에서 제어 채널을 받지 않는다(`supports_control_channel` = false 라 데몬이 provision 하지 않는다).
        //   이 `endpoint` 꼴은 운영 모양이 아니라 「shell 은 제어 채널을 무시한다」 를 지키는 표지다.
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

/// 즉석 생성 길(wire `SpawnByCwd` · 스모크 bin — 명부에 없는 프로필을 `spawn_agent` 로 Fresh)의 명령 모양. env 가
/// 비어 있어 제어 채널 꼴은 `no-endpoint` 하나만 뜬다 — 제어 채널이 CLI 입구를 PATH 에 넣을 때 이 프로세스의 PATH 를
/// 읽어 결정적이지 않다.
// TRD S25 §5-2 4 「골든에 그 두 길을 넣는다」
fn spawn_path_commands() -> Vec<(&'static str, AgentCommand)> {
    vec![
        // 데몬 `spawn_command_by_cwd` — 받은 백엔드 · 공통 기본 모드 · 인자 없음.
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
        // 스모크 bin(`priming_smoke` 등) — `--model <m>` 을 싣는 claude JSON.
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

/// 칸을 전부 채운 꼴 — 경로를 argv 에 그대로 싣는 백엔드 몫이라 프라이밍은 읽히지 않는 고정 경로다. CLI grant 는
/// 운영의 claude 갈래에 없다(MCP 설정을 받는 백엔드는 CLI grant 를 받지 않는다 — 데몬 `build_grants`).
fn full_endpoint() -> ControlEndpoint {
    ControlEndpoint {
        agent_id: AGENT_ID,
        epoch: 7,
        url: "http://127.0.0.1:47000/mcp".to_string(),
        token: "golden-token".to_string(),
        config_path: Some(PathBuf::from("/golden/mcp/agent.mcp.json")),
        send_exe: Some(PathBuf::from("/golden/bin/engram.exe")),
        mail_allowed: false,
        priming_file: Some(PathBuf::from("/golden/priming.md")),
        grants: vec![
            mcp_send_grant(),
            ToolGrant::Cli {
                exe: "engram".to_string(),
            },
        ],
        settings_file: Some(PathBuf::from("/golden/mcp/agent.settings.json")),
    }
}

/// 데몬이 claude 에 내는 꼴 중 프라이밍 파일이 없고 설정 조각 쓰기가 실패한 것(데몬 `control/mod.rs` 의 provision).
fn claude_bare_endpoint() -> ControlEndpoint {
    ControlEndpoint {
        priming_file: None,
        settings_file: None,
        grants: vec![mcp_send_grant()],
        ..full_endpoint()
    }
}

/// 데몬이 codex 에 내는 꼴 — 파일을 쓰지 않는 백엔드라(`writes_mcp_config_file` = false) 경로 두 칸이 빈다.
fn codex_endpoint() -> ControlEndpoint {
    ControlEndpoint {
        config_path: None,
        settings_file: None,
        priming_file: Some(PathBuf::from(PRIMING_FIXTURE)),
        grants: vec![mcp_send_grant()],
        ..full_endpoint()
    }
}

// PATH 를 넣는 것은 결정성 때문이다 — 없으면 CLI 입구 주입이 이 프로세스의 PATH 를 읽는다.
fn profile_env() -> Vec<(String, String)> {
    let path = std::env::join_paths(["/golden/sys", "/golden/tools"])
        .expect("고정 PATH")
        .into_string()
        .expect("UTF-8");
    vec![
        ("PATH".to_string(), path),
        ("GOLDEN_FLAG".to_string(), "1".to_string()),
    ]
}

/// 경우마다 manager 가 넘기는 꼴 그대로(세션 id 두 칸은 백엔드 축 dispatch 로 가른다).
fn actual_cases() -> Vec<(String, CommandSpec)> {
    assert!(
        Path::new(PRIMING_FIXTURE).is_file(),
        "프라이밍 고정물이 없다: {PRIMING_FIXTURE}"
    );
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

/// 콘솔 감싸기의 앞머리 — `console_command` 가 감싸는 OS 에서만 `Some((감싼 프로그램, 앞 인자))`.
fn console_wrapper() -> Option<(String, Vec<String>)> {
    const PROBE: &str = "launch-golden-probe";
    let (program, mut args) = console_command(PROBE, Vec::new());
    if program == PROBE {
        return None;
    }
    assert_eq!(
        args.pop().as_deref(),
        Some(PROBE),
        "감싼 꼴의 끝 = 감싼 프로그램"
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
    serde_json::to_string_pretty(&cases).expect("직렬화") + "\n"
}

#[test]
fn noop_argv_matches_the_golden() {
    let golden: Vec<GoldenCase> = serde_json::from_str(GOLDEN).expect("골든은 JSON");
    let actual = actual_cases();
    assert_eq!(
        golden.iter().map(|g| g.case.as_str()).collect::<Vec<_>>(),
        actual.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        "골든의 경우 목록"
    );
    let drift: Vec<String> = golden
        .iter()
        .zip(&actual)
        .filter(|(g, (_, spec))| !matches(g, spec))
        .map(|(_, (name, spec))| serde_json::to_string_pretty(&record(name, spec)).unwrap())
        .collect();
    assert!(
        drift.is_empty(),
        "Noop argv 가 골든과 다르다 — 실행 재료 포트는 Noop 스폰을 바이트 하나 바꾸지 않아야 한다. 일부러 바꾼 것이면 \
         `regenerate_launch_argv_golden` 으로 다시 뜬다. 지금 값:\n{}",
        drift.join("\n")
    );
}

#[test]
#[ignore = "골든을 다시 뜬다 — Noop argv 를 일부러 바꿨을 때만 돌린다"]
fn regenerate_launch_argv_golden() {
    assert!(
        console_wrapper().is_some(),
        "콘솔 감싸기가 보이는 OS(Windows)에서 다시 뜬다 — 아니면 `console` 칸을 뜰 수 없다"
    );
    let text = render();
    assert_eq!(text, render(), "두 번 뜨면 같은 바이트");
    std::fs::write(GOLDEN_PATH, text).expect("골든 쓰기");
}
