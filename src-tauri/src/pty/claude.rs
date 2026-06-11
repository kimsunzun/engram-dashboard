//! claude 전용 지식 격리 — 중립 프로필을 실제 실행 인자로 변환하는 **유일한 곳**.
//!
//! 세션 인자(`--session-id`/`--resume`) 조립 규칙이 claude에 종속되므로 여기 가둔다.
//! 추후 codex CLI 등이 붙으면 형제 모듈(`codex.rs`)을 두고 manager가 분기하면 된다 —
//! profile.rs·manager.rs의 중립성은 유지된다(미래 확장 seam, H-2).
//!
//! tauri import 0.

use uuid::Uuid;

use crate::pty::profile::{AgentCommand, SpawnMode};

/// claude 실행 파일명. PATH로 해석된다.
///
/// ※ Windows에서 `claude`가 `claude.cmd` shim(→cmd→node)일 수 있다. 그 경우 우리 child PID는
/// shim 프로세스라 `sessions/<pid>.json`이 child PID와 어긋난다 — session_tracker가 sid 스캔으로
/// 우회한다(설계상 흡수). 복원 정확성은 `--session-id`/`--resume`(우리 통제)에 있으므로 무관.
#[cfg(windows)]
const CLAUDE_PROGRAM: &str = "claude";
#[cfg(not(windows))]
const CLAUDE_PROGRAM: &str = "claude";

/// 이 명령이 claude 세션 추적 대상인가(= sid 발급·watcher 부착 대상).
pub fn needs_session(command: &AgentCommand) -> bool {
    matches!(command, AgentCommand::Claude { .. })
}

/// 프로필 + 모드 → `(program, args)`.
///
/// - Claude + Fresh  → `claude --session-id <sid> [extra…]`  (우리가 sid를 통제)
/// - Claude + Resume → `claude --resume <sid> [extra…]`       (기존 세션 무손실 이어받기)
/// - Shell           → 지정 program/args 그대로(세션 인자 없음)
pub fn build_command(
    command: &AgentCommand,
    mode: SpawnMode,
    session_id: Option<Uuid>,
) -> (String, Vec<String>) {
    match command {
        AgentCommand::Claude { extra_args } => {
            let mut args = Vec::with_capacity(2 + extra_args.len());
            if let Some(sid) = session_id {
                let flag = match mode {
                    SpawnMode::Fresh => "--session-id",
                    SpawnMode::Resume => "--resume",
                };
                args.push(flag.to_string());
                args.push(sid.to_string());
            }
            args.extend(extra_args.iter().cloned());
            (CLAUDE_PROGRAM.to_string(), args)
        }
        AgentCommand::Shell { program, args } => (program.clone(), args.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_fresh_uses_session_id_flag() {
        let sid = Uuid::new_v4();
        let (prog, args) = build_command(
            &AgentCommand::Claude {
                extra_args: vec!["--verbose".into()],
            },
            SpawnMode::Fresh,
            Some(sid),
        );
        assert_eq!(prog, CLAUDE_PROGRAM);
        assert_eq!(
            args,
            vec![
                "--session-id".to_string(),
                sid.to_string(),
                "--verbose".to_string()
            ]
        );
    }

    #[test]
    fn claude_resume_uses_resume_flag() {
        let sid = Uuid::new_v4();
        let (_, args) = build_command(
            &AgentCommand::Claude { extra_args: vec![] },
            SpawnMode::Resume,
            Some(sid),
        );
        assert_eq!(args, vec!["--resume".to_string(), sid.to_string()]);
    }

    #[test]
    fn shell_passes_through_without_session_args() {
        let (prog, args) = build_command(
            &AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec!["/c".into(), "echo hi".into()],
            },
            SpawnMode::Fresh,
            Some(Uuid::new_v4()), // shell은 sid 무시
        );
        assert_eq!(prog, "cmd.exe");
        assert_eq!(args, vec!["/c".to_string(), "echo hi".to_string()]);
    }

    #[test]
    fn needs_session_only_for_claude() {
        assert!(needs_session(&AgentCommand::Claude { extra_args: vec![] }));
        assert!(!needs_session(&AgentCommand::Shell {
            program: "x".into(),
            args: vec![]
        }));
    }
}
