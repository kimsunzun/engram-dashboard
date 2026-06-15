//! ShellBackend — 임의 셸 프로그램 전용 CommandSpec 산출.
//!
//! 세션 추적 불필요. program/args를 그대로 CommandSpec에 실어 반환한다.
//!
//! tauri import 0.

use std::path::PathBuf;

use uuid::Uuid;

use crate::agent::backend::AgentBackend;
use crate::agent::profile::{AgentCommand, SpawnMode};
use crate::agent::types::CommandSpec;

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
