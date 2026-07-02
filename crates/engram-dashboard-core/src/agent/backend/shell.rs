//! ShellBackend — 임의 셸 프로그램 전용 CommandSpec 산출.
//!
//! 세션 추적 불필요. program/args를 그대로 CommandSpec에 실어 반환한다.
//!
//! tauri import 0.

use std::path::PathBuf;

use uuid::Uuid;

use crate::agent::backend::AgentBackend;
use crate::agent::profile::{AgentCommand, SpawnMode};
use crate::agent::types::{BackendCaps, CommandSpec, ModelCaps, SessionCaps};

/// 셸 백엔드 unit struct. &'static으로 사용, 상태 없음.
pub struct ShellBackend;

impl AgentBackend for ShellBackend {
    fn needs_session(&self) -> bool {
        // 셸은 claude 세션 추적 불필요.
        false
    }

    fn build_spec(
        &self,
        command: &AgentCommand,
        _mode: SpawnMode,
        _session_id: Option<Uuid>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
    ) -> CommandSpec {
        match command {
            AgentCommand::Shell { program, args } => CommandSpec {
                program: program.clone(),
                args: args.clone(),
                env,
                cwd,
            },
            // dispatch가 ShellBackend에는 Shell variant만 보내지만, Claude가 들어오면 방어적으로 처리.
            // Claude 전용 세션 인자를 모르므로 program만 채우고 args는 비운다.
            AgentCommand::Claude { .. } => {
                // ShellBackend는 Claude 세션 인자 조립 방법을 모른다 — dispatch 오류 방어.
                unreachable!("ShellBackend는 Claude variant를 처리하지 않음. dispatch 버그.")
            }
        }
    }

    /// ★이번 정확화의 핵심★: 범용 셸은 `--resume` 같은 세션 재개 개념이 없다 → resume=false.
    /// 예전엔 transport 가 backend 무관하게 resume=true 를 하드코딩해 shell 이 부정확했다.
    /// cwd_env=true(셸도 cwd 에서 실행). model 옵션 없음. (셸은 mode 개념이 없어 command 미사용.)
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

    fn spec(command: &AgentCommand) -> CommandSpec {
        ShellBackend.build_spec(command, SpawnMode::Fresh, None, PathBuf::from("."), vec![])
    }

    #[test]
    fn shell_passthrough() {
        let s = spec(&AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo hi".into()],
        });
        assert_eq!(s.program, "cmd.exe");
        assert_eq!(s.args, vec!["/c".to_string(), "echo hi".to_string()]);
    }

    #[test]
    fn needs_session_is_false() {
        assert!(!ShellBackend.needs_session());
    }

    #[test]
    fn capabilities_resume_is_false() {
        // 핵심 회귀: 범용 셸은 --resume 없음 → resume=false 여야 한다(이전 부정확 = transport
        // 가 backend 무관하게 resume=true 하드코딩했던 것을 backend 소관으로 바로잡음).
        let cmd = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        assert!(!ShellBackend.capabilities(&cmd).session.resume);
    }

    #[test]
    fn cwd_and_env_are_forwarded() {
        let cwd = PathBuf::from("C:/workspace");
        let env = vec![("BAR".to_string(), "baz".to_string())];
        let s = ShellBackend.build_spec(
            &AgentCommand::Shell {
                program: "bash".into(),
                args: vec![],
            },
            SpawnMode::Fresh,
            None,
            cwd.clone(),
            env.clone(),
        );
        assert_eq!(s.cwd, cwd);
        assert_eq!(s.env, env);
    }
}
