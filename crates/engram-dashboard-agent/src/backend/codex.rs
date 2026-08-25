//! CodexBackend — Codex CLI 전용 CommandSpec 산출 stub.
//!
//! AgentCommand에 Codex variant가 없으므로 backend_for dispatch에서 이 backend로 라우팅되지
//! 않는다. 이 파일은 구조 확보 목적의 stub이며, AgentCommand::Codex variant 추가와
//! backend_for 매칭은 CLI spike 완료 후 별도 작업에서 확정한다.
//!
//! tauri import 0.

use std::path::PathBuf;

use uuid::Uuid;

use crate::backend::AgentBackend;
use crate::profile::{AgentCommand, SpawnMode};
use crate::types::{BackendCaps, CommandSpec, ControlEndpoint, ModelCaps, SessionCaps};

/// Codex 실행 파일명. PATH로 해석된다.
///
/// ※ best-guess: Codex CLI의 실제 바이너리명이 "codex"인지 확인 필요.
/// CLI spike에서 `which codex` / `codex --help` 로 확정할 것.
const CODEX_PROGRAM: &str = "codex";

pub struct CodexBackend;

impl AgentBackend for CodexBackend {
    fn needs_session(&self) -> bool {
        // best-guess: Codex도 세션 개념이 있다고 가정해 true.
        // CLI spike에서 실측 후 확정. 세션 없는 CLI라면 false로 변경.
        true
    }

    fn supports_control_channel(&self) -> bool {
        // 보수적 stub(ADR-0086 F3) — Codex 의 MCP 지원 여부는 CLI spike 전이라 미상이다. capabilities
        //   stub 가 전부 false 인 것과 같은 정신으로 false(미측정 backend 는 제어 채널을 소비한다고
        //   주장하지 않는다). spike 후 실측값으로 교체.
        false
    }

    fn accepts_mcp_config(&self) -> bool {
        // 보수적 stub(ADR-0099) — 미측정 backend 는 MCP-capable 을 주장하지 않는다 → false(비-MCP 스폰:
        //   mcp-config 미기록 + CLI-only 프라이밍 + [Cli] grant). Codex 의 실제 MCP config 지원은 CLI
        //   spike 후 실측값으로 교체(ADR-0004 backend 지식).
        // ADR-0099
        false
    }

    fn build_spec(
        &self,
        command: &AgentCommand,
        mode: SpawnMode,
        session_id: Option<Uuid>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
        // ADR-0086: stub — 제어 채널 주입은 CLI spike 후 variant 확정 시 구현(현재 무시).
        // TODO(ADR-0094): translate ControlEndpoint.grants to codex permission flags
        //   (claude 는 --allowedTools mcp__{s}__{t} / Bash({e}:*)+PowerShell({e}:*); codex 방언은 CLI spike 후 확정).
        _control: Option<ControlEndpoint>,
    ) -> CommandSpec {
        let mut args: Vec<String> = Vec::new();

        if let Some(sid) = session_id {
            // codex CLI의 세션 재개 플래그가 --session / --resume / --continue 등인지 미확인.
            let flag = match mode {
                SpawnMode::Fresh => "--session",
                SpawnMode::Resume => "--resume",
            };
            args.push(flag.to_string());
            args.push(sid.to_string());
        }

        match command {
            AgentCommand::Claude { extra_args, .. } => {
                args.extend(extra_args.iter().cloned());
            }
            AgentCommand::Shell {
                program,
                args: shell_args,
            } => {
                return CommandSpec {
                    program: CODEX_PROGRAM.to_string(),
                    args: {
                        let _ = program;
                        shell_args.clone()
                    },
                    env,
                    cwd,
                };
            }
        }

        CommandSpec {
            program: CODEX_PROGRAM.to_string(),
            args,
            env,
            cwd,
        }
    }

    /// 보수적 stub — CLI spike 전이라 실제 resume/model 능력 미상 → 전부 false.
    /// spike 후 실측값으로 교체.
    fn capabilities(&self, _command: &AgentCommand) -> BackendCaps {
        BackendCaps {
            session: SessionCaps {
                resume: false,
                snapshot: false,
                cwd_env: true,
            },
            model: ModelCaps {
                select: false,
                temperature: false,
                max_tokens: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(mode: SpawnMode, sid: Option<Uuid>) -> CommandSpec {
        CodexBackend.build_spec(
            &AgentCommand::Claude {
                extra_args: vec![],
                output_format: crate::profile::ClaudeOutputFormat::Terminal,
            },
            mode,
            sid,
            PathBuf::from("."),
            vec![],
            None,
        )
    }

    #[test]
    fn codex_program_name_is_correct() {
        let s = spec(SpawnMode::Fresh, None);
        assert_eq!(s.program, CODEX_PROGRAM);
        assert_eq!(s.program, "codex");
    }

    #[test]
    fn codex_fresh_uses_session_flag_best_guess() {
        let sid = Uuid::new_v4();
        let s = spec(SpawnMode::Fresh, Some(sid));
        assert_eq!(s.program, CODEX_PROGRAM);
        assert_eq!(s.args, vec!["--session".to_string(), sid.to_string()]);
    }

    #[test]
    fn codex_resume_uses_resume_flag_best_guess() {
        let sid = Uuid::new_v4();
        let s = spec(SpawnMode::Resume, Some(sid));
        assert_eq!(s.args, vec!["--resume".to_string(), sid.to_string()]);
    }

    #[test]
    fn needs_session_is_true() {
        assert!(CodexBackend.needs_session());
    }

    #[test]
    fn cwd_and_env_are_forwarded() {
        let cwd = PathBuf::from("C:/workspace");
        let env = vec![("BAR".to_string(), "baz".to_string())];
        let s = CodexBackend.build_spec(
            &AgentCommand::Claude {
                extra_args: vec![],
                output_format: crate::profile::ClaudeOutputFormat::Terminal,
            },
            SpawnMode::Fresh,
            None,
            cwd.clone(),
            env.clone(),
            None,
        );
        assert_eq!(s.cwd, cwd);
        assert_eq!(s.env, env);
    }
}
