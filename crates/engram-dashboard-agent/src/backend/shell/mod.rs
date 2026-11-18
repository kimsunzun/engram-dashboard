//! ShellBackend — produces the CommandSpec for arbitrary shell programs only.
//!
//! ★The rule this folder sets = shell knowledge lives only in here (ADR-0004)★. The canonical grounds, gates and what the gates
//! cannot see are in the `backend/claude/mod.rs` header and are not repeated here — read it with the names swapped.
//! The only surface going out is the one [`crate::backend::AgentBackend`] implementation.
//!
//! tauri import 0.

use std::path::PathBuf;

use uuid::Uuid;

use crate::backend::AgentBackend;
use crate::profile::{AgentCommand, SpawnMode};
use crate::types::{BackendCaps, CommandSpec, ControlEndpoint, ModelCaps, SessionCaps};

pub struct ShellBackend;

impl AgentBackend for ShellBackend {
    /// A shell has no conversation-session concept — both axes are false.
    fn assigns_session_id(&self, _command: &AgentCommand) -> bool {
        false
    }

    fn can_resume_stored_session(&self, _command: &AgentCommand) -> bool {
        false
    }

    fn supports_control_channel(&self) -> bool {
        false
    }

    fn accepts_mcp_config(&self) -> bool {
        // provision is never called in the first place (false above), so this value is not actually consumed, but for contract completeness
        //   it is honestly false.
        // ADR-0099
        false
    }

    /// ★A shell is not a reader of mail (user decision 2026-08-17)★ — an envelope that arrives at a shell is not read but
    /// **executed as a command**. The body is LLM free text, so if `&`·`|`·`;` are mixed in, what follows is parsed as a separate
    /// command. So it is left out of the recipient list (including `@all`) entirely — not a mitigation such as dropping the submit bytes.
    fn reads_messages(&self) -> bool {
        false
    }

    fn build_spec(
        &self,
        command: &AgentCommand,
        _mode: SpawnMode,
        _session_id: Option<Uuid>,
        _resume_session_id: Option<Uuid>,
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
            AgentCommand::Claude { .. } | AgentCommand::Codex { .. } => {
                unreachable!("ShellBackend handles only the Shell variant. dispatch bug.")
            }
        }
    }

    /// A general-purpose shell has no session-resume concept like `--resume` → resume=false. Previously the transport hardcoded resume=true
    /// regardless of backend, so it was inaccurate for shell. cwd_env=true — a shell also runs in the cwd.
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
            None,
            cwd.clone(),
            env.clone(),
            None,
        );
        assert_eq!(s.cwd, cwd);
        assert_eq!(s.env, env);
    }
}
