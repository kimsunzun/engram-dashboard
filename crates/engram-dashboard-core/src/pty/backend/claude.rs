//! ClaudeBackend — claude CLI 전용 CommandSpec 산출.
//!
//! 세션 인자(`--session-id`/`--resume`) 조립 규칙이 claude에 종속되므로 여기에만 둔다.
//! 현 `pty/claude.rs`의 build_command 로직과 완전히 동치이며,
//! 차이점은 (program, args) 대신 CommandSpec(cwd·env 포함)을 반환한다는 것뿐이다.
//!
//! tauri import 0.

use std::path::PathBuf;

use uuid::Uuid;

use crate::pty::backend::{console_command, AgentBackend};
use crate::pty::profile::{AgentCommand, SpawnMode};
use crate::pty::types::CommandSpec;

/// claude 실행 파일명(논리값). 실제 spawn 시 Windows에선 `console_command`가 `cmd.exe /c claude`로
/// 감싼다(npm shim 해석, error 193 회피 — backend/mod.rs 참조).
///
/// ※ Windows shim 경유 시 우리 child PID는 cmd/shim 프로세스라 `sessions/<pid>.json`이 child PID와
/// 어긋난다 — session_tracker가 sid 스캔으로 우회한다(설계상 흡수). 복원 정확성은
/// `--session-id`/`--resume`(우리 통제)에 있으므로 무관.
const CLAUDE_PROGRAM: &str = "claude";

/// claude 백엔드 unit struct. &'static으로 사용, 상태 없음.
pub struct ClaudeBackend;

impl AgentBackend for ClaudeBackend {
    fn needs_session(&self) -> bool {
        // claude는 항상 세션 추적 대상 — sid 발급·watcher 부착 필요.
        true
    }

    fn build_spec(
        &self,
        command: &AgentCommand,
        mode: SpawnMode,
        session_id: Option<Uuid>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
    ) -> CommandSpec {
        match command {
            AgentCommand::Claude { extra_args } => {
                let mut args = Vec::with_capacity(2 + extra_args.len());
                if let Some(sid) = session_id {
                    // Fresh → --session-id(우리가 sid를 통제), Resume → --resume(기존 세션 무손실 이어받기).
                    let flag = match mode {
                        SpawnMode::Fresh => "--session-id",
                        SpawnMode::Resume => "--resume",
                    };
                    args.push(flag.to_string());
                    args.push(sid.to_string());
                }
                args.extend(extra_args.iter().cloned());
                // Windows shim 회피: cmd /c claude … 로 감싼다(비Windows는 그대로).
                let (program, args) = console_command(CLAUDE_PROGRAM, args);
                CommandSpec {
                    program,
                    args,
                    env,
                    cwd,
                }
            }
            // dispatch가 ClaudeBackend에는 Claude variant만 보내지만, 방어적으로 Shell도 처리한다.
            // 현 claude.rs build_command와 동일하게 program/args 패스스루.
            AgentCommand::Shell { program, args } => CommandSpec {
                program: program.clone(),
                args: args.clone(),
                env,
                cwd,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── backend/claude.rs 단위 테스트 ─────────────────────────────────────────
    // 현 pty/claude.rs tests의 build_command 검증을 build_spec 시그니처로 이식.
    // 기존 claude.rs 테스트는 그대로 두고, stage 6에서 claude.rs 제거 시 이쪽만 남는다.

    fn spec(command: &AgentCommand, mode: SpawnMode, sid: Option<Uuid>) -> CommandSpec {
        ClaudeBackend.build_spec(command, mode, sid, PathBuf::from("."), vec![])
    }

    #[test]
    fn claude_fresh_uses_session_id_flag() {
        let sid = Uuid::new_v4();
        let s = spec(
            &AgentCommand::Claude {
                extra_args: vec!["--verbose".into()],
            },
            SpawnMode::Fresh,
            Some(sid),
        );
        // Windows면 cmd /c claude … 로 래핑되므로 기대값도 console_command로 계산.
        let (p, a) = console_command(
            CLAUDE_PROGRAM,
            vec![
                "--session-id".to_string(),
                sid.to_string(),
                "--verbose".to_string(),
            ],
        );
        assert_eq!(s.program, p);
        assert_eq!(s.args, a);
    }

    #[test]
    fn claude_resume_uses_resume_flag() {
        let sid = Uuid::new_v4();
        let s = spec(
            &AgentCommand::Claude { extra_args: vec![] },
            SpawnMode::Resume,
            Some(sid),
        );
        let (_p, a) = console_command(
            CLAUDE_PROGRAM,
            vec!["--resume".to_string(), sid.to_string()],
        );
        assert_eq!(s.args, a);
    }

    #[test]
    fn claude_no_session_id_produces_no_flags() {
        let s = spec(
            &AgentCommand::Claude {
                extra_args: vec!["--debug".into()],
            },
            SpawnMode::Fresh,
            None,
        );
        // sid 없으면 세션 플래그 없이 extra_args만(Windows면 cmd /c 래핑).
        let (p, a) = console_command(CLAUDE_PROGRAM, vec!["--debug".to_string()]);
        assert_eq!(s.program, p);
        assert_eq!(s.args, a);
    }

    #[test]
    fn shell_passthrough_via_claude_backend() {
        // dispatch가 보내지 않는 경로지만 방어 코드 검증.
        let s = spec(
            &AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec!["/c".into(), "echo hi".into()],
            },
            SpawnMode::Fresh,
            Some(Uuid::new_v4()),
        );
        assert_eq!(s.program, "cmd.exe");
        assert_eq!(s.args, vec!["/c".to_string(), "echo hi".to_string()]);
    }

    #[test]
    fn needs_session_is_true() {
        assert!(ClaudeBackend.needs_session());
    }

    #[test]
    fn cwd_and_env_are_forwarded() {
        let cwd = PathBuf::from("C:/workspace");
        let env = vec![("FOO".to_string(), "bar".to_string())];
        let s = ClaudeBackend.build_spec(
            &AgentCommand::Claude { extra_args: vec![] },
            SpawnMode::Fresh,
            None,
            cwd.clone(),
            env.clone(),
        );
        assert_eq!(s.cwd, cwd);
        assert_eq!(s.env, env);
    }
}
