//! GeminiBackend — Gemini CLI 전용 CommandSpec 산출 stub.
//!
//! AgentCommand에 Gemini variant가 없으므로 backend_for dispatch에서 이 backend로 라우팅되지
//! 않는다. 이 파일은 구조 확보 목적의 stub이며, AgentCommand::Gemini variant 추가와
//! backend_for 매칭은 CLI spike 완료 후 별도 작업에서 확정한다.
//!
//! tauri import 0.

use std::path::PathBuf;

use uuid::Uuid;

use crate::pty::backend::AgentBackend;
use crate::pty::profile::{AgentCommand, SpawnMode};
use crate::pty::types::CommandSpec;

/// Gemini 실행 파일명. PATH로 해석된다.
///
/// ※ best-guess: Google Gemini CLI의 실제 바이너리명이 "gemini"인지 확인 필요.
/// CLI spike에서 `which gemini` / `gemini --help` 로 확정할 것.
/// Google AI Studio CLI 또는 `gemini-cli` 패키지명일 가능성 있음.
const GEMINI_PROGRAM: &str = "gemini";

/// Gemini 백엔드 unit struct. &'static으로 사용, 상태 없음.
pub struct GeminiBackend;

impl AgentBackend for GeminiBackend {
    fn needs_session(&self) -> bool {
        // best-guess: Gemini CLI도 대화 세션 개념이 있다고 가정해 true.
        // CLI spike에서 실측 후 확정. 세션리스 CLI라면 false로 변경.
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
        // AgentCommand에 Gemini variant가 없으므로 Claude/Shell만 들어올 수 있다.
        // dispatch(backend_for)에서 이 backend로 오는 경로가 현재 없음 — 라우팅 미연결.
        // build_spec은 단위 테스트에서만 직접 호출해 구조를 검증한다.
        //
        // TODO: CLI spike 완료 후 AgentCommand::Gemini variant 추가 + 전용 분기 구현.
        let mut args: Vec<String> = Vec::new();

        if let Some(sid) = session_id {
            // TODO: CLI spike로 플래그 확정. 현재 best-guess.
            // Gemini CLI의 세션 재개 플래그가 --session / --resume / --conversation 등인지 미확인.
            // spike 전까지 --session (Fresh) / --resume (Resume) 를 best-guess로 사용.
            // Claude와 동일한 패턴을 best-guess로 선택(대부분의 AI CLI가 유사한 UX를 따른다고 가정).
            let flag = match mode {
                SpawnMode::Fresh => "--session",
                SpawnMode::Resume => "--resume",
            };
            args.push(flag.to_string());
            args.push(sid.to_string());
        }

        // AgentCommand 분기: Gemini variant 없으므로 Shell의 extra args를 패스스루하거나,
        // Claude의 extra_args를 그대로 붙인다. 실제 Gemini 라우팅 시에는 전용 분기로 교체.
        match command {
            AgentCommand::Claude { extra_args } => {
                args.extend(extra_args.iter().cloned());
            }
            AgentCommand::Shell {
                program,
                args: shell_args,
            } => {
                // Shell 명령을 받은 경우 program만 GEMINI_PROGRAM으로 교체, args는 패스스루.
                // 이 경로는 dispatch에서 오지 않으므로 테스트 외 호출 없음.
                return CommandSpec {
                    program: GEMINI_PROGRAM.to_string(),
                    args: {
                        let _ = program; // 사용하지 않음 — 명시적 억제
                        shell_args.clone()
                    },
                    env,
                    cwd,
                };
            }
        }

        CommandSpec {
            program: GEMINI_PROGRAM.to_string(),
            args,
            env,
            cwd,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(mode: SpawnMode, sid: Option<Uuid>) -> CommandSpec {
        GeminiBackend.build_spec(
            &AgentCommand::Claude { extra_args: vec![] },
            mode,
            sid,
            PathBuf::from("."),
            vec![],
        )
    }

    #[test]
    fn gemini_program_name_is_correct() {
        // best-guess program명 "gemini" 검증 — spike 후 실제값으로 교체 예정.
        let s = spec(SpawnMode::Fresh, None);
        assert_eq!(s.program, GEMINI_PROGRAM);
        assert_eq!(s.program, "gemini");
    }

    #[test]
    fn gemini_fresh_uses_session_flag_best_guess() {
        // best-guess: Fresh → --session <sid>.
        // TODO: CLI spike로 플래그 확정. 현재 best-guess.
        let sid = Uuid::new_v4();
        let s = spec(SpawnMode::Fresh, Some(sid));
        assert_eq!(s.program, GEMINI_PROGRAM);
        assert_eq!(s.args, vec!["--session".to_string(), sid.to_string()]);
    }

    #[test]
    fn gemini_resume_uses_resume_flag_best_guess() {
        // best-guess: Resume → --resume <sid>.
        // TODO: CLI spike로 플래그 확정. 현재 best-guess.
        let sid = Uuid::new_v4();
        let s = spec(SpawnMode::Resume, Some(sid));
        assert_eq!(s.args, vec!["--resume".to_string(), sid.to_string()]);
    }

    #[test]
    fn needs_session_is_true() {
        // best-guess: Gemini CLI도 세션 개념 있다고 가정. spike 후 확인.
        assert!(GeminiBackend.needs_session());
    }

    #[test]
    fn cwd_and_env_are_forwarded() {
        let cwd = PathBuf::from("C:/workspace");
        let env = vec![("GEMINI_KEY".to_string(), "dummy".to_string())];
        let s = GeminiBackend.build_spec(
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
