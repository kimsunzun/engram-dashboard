//! ShellBackend — 임의 셸 프로그램 전용 CommandSpec 산출.
//!
//! tauri import 0.

use std::path::PathBuf;

use uuid::Uuid;

use crate::agent::backend::AgentBackend;
use crate::agent::profile::{AgentCommand, SpawnMode};
use crate::agent::types::{BackendCaps, CommandSpec, ControlEndpoint, ModelCaps, SessionCaps};

pub struct ShellBackend;

impl AgentBackend for ShellBackend {
    fn needs_session(&self) -> bool {
        false
    }

    fn supports_control_channel(&self) -> bool {
        false
    }

    fn accepts_mcp_config(&self) -> bool {
        // provision 을 애초에 안 부르므로(위 false) 이 값은 실제 소비되지 않지만, 계약 완결성 위해
        //   정직하게 false.
        // ADR-0099
        false
    }

    fn build_spec(
        &self,
        command: &AgentCommand,
        _mode: SpawnMode,
        _session_id: Option<Uuid>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
        _control: Option<ControlEndpoint>,
    ) -> CommandSpec {
        match command {
            AgentCommand::Shell { program, args } => CommandSpec {
                program: program.clone(),
                args: args.clone(),
                env,
                cwd,
            },
            AgentCommand::Claude { .. } => {
                unreachable!("ShellBackend는 Claude variant를 처리하지 않음. dispatch 버그.")
            }
        }
    }

    /// 범용 셸은 `--resume` 같은 세션 재개 개념이 없다 → resume=false. 예전엔 transport 가 backend
    /// 무관하게 resume=true 를 하드코딩해 shell 이 부정확했다. cwd_env=true — 셸도 cwd 에서 실행한다.
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
        ShellBackend.build_spec(
            command,
            SpawnMode::Fresh,
            None,
            PathBuf::from("."),
            vec![],
            None,
        )
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
            None,
        );
        assert_eq!(s.cwd, cwd);
        assert_eq!(s.env, env);
    }
}
