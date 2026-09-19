//! GeminiBackend — Gemini CLI 전용 CommandSpec 산출 stub.
//!
//! ★이 폴더가 세우는 규칙 = gemini 지식은 여기 안에만 산다(ADR-0004)★. 근거·게이트·게이트가
//! 못 보는 것의 정본은 `backend/claude/mod.rs` 헤더이고 여기 되풀어 적지 않는다 — 이름만 바꿔
//! 읽는다. 밖으로 나가는 표면은 [`crate::backend::AgentBackend`] 구현 하나뿐이다.
//!
//! AgentCommand에 Gemini variant가 없으므로 backend_for dispatch에서 이 backend로 라우팅되지
//! 않는다. 이 파일은 구조 확보 목적의 stub이며, AgentCommand::Gemini variant 추가와
//! backend_for 매칭은 CLI spike 완료 후 별도 작업에서 확정한다.
//!
//! tauri import 0.

use std::path::PathBuf;

use uuid::Uuid;

use crate::backend::AgentBackend;
use crate::profile::{AgentCommand, SpawnMode};
use crate::types::{BackendCaps, CommandSpec, ControlEndpoint, ModelCaps, SessionCaps};

/// Gemini 실행 파일명. PATH로 해석된다.
///
/// ※ best-guess: Google Gemini CLI의 실제 바이너리명이 "gemini"인지 확인 필요.
/// CLI spike에서 `which gemini` / `gemini --help` 로 확정할 것.
/// Google AI Studio CLI 또는 `gemini-cli` 패키지명일 가능성 있음.
const GEMINI_PROGRAM: &str = "gemini";

pub struct GeminiBackend;

impl AgentBackend for GeminiBackend {
    /// ★아래 두 축은 실측된 적이 없고, 이 백엔드는 오늘 **도달 불가**다★ — `AgentCommand` 에 gemini
    /// variant 가 없어 `backend_for` 가 이 구현으로 오는 길이 없다(그래서 `tests/backend_contract.rs` 의
    /// 선언 표에도 행이 없다). 값은 CLI spike 전의 best-guess 이고, variant 를 들이는 작업이 실측으로
    /// 교체한다.
    /// ★**이 두 메서드에 한해** 단위 테스트를 두지 않는 것은 의도다★: 부르려면 남의 백엔드 명령을 먹여야
    ///   하는데(`backend_for` 가 만들 수 없는 짝) **이 둘은 그 명령을 읽지도 않아서**(`_command`) 그렇게 쓴
    ///   단언은 상수 하나를 되읽을 뿐이다. ★같은 파일의 `build_spec` 은 다르다★ — 그쪽은 명령을 실제로
    ///   match 해 인자를 옮기므로 합성 표본으로도 재는 것이 있고, 그래서 테스트를 둔다(그 한계는 아래
    ///   `tests::spec` 주석).
    fn assigns_session_id(&self, _command: &AgentCommand) -> bool {
        // best-guess: Gemini CLI도 대화 세션 개념이 있다고 가정해 true.
        // 세션리스 CLI라면 false로 변경.
        true
    }

    fn can_resume_stored_session(&self, _command: &AgentCommand) -> bool {
        // best-guess: 아래 `--resume <sid>` 조립과 같은 가정.
        true
    }

    fn supports_control_channel(&self) -> bool {
        // 보수적 stub(ADR-0086 F3) — Gemini CLI 의 MCP 지원 여부는 CLI spike 전이라 미상이다.
        //   capabilities stub 가 전부 false 인 것과 같은 정신으로 false(미측정 backend 는 제어 채널을
        //   소비한다고 주장하지 않는다). spike 후 실측값으로 교체.
        false
    }

    fn accepts_mcp_config(&self) -> bool {
        // 보수적 stub(ADR-0099) — 미측정 backend 는 MCP-capable 을 주장하지 않는다 → false(비-MCP 스폰).
        //   Gemini 의 실제 MCP config 지원은 CLI spike 후 실측값으로 교체(ADR-0004 backend 지식).
        // ADR-0099
        false
    }

    fn build_spec(
        &self,
        command: &AgentCommand,
        mode: SpawnMode,
        session_id: Option<Uuid>,
        // ADR-0185 stub — 이 백엔드는 dispatch 에 배선되지 않았고, 명령줄 이어받기 문법도 미측정이다.
        _resume_session_id: Option<Uuid>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
        // ADR-0086: stub — 제어 채널 주입은 CLI spike 후 variant 확정 시 구현(현재 무시).
        // TODO(ADR-0094): translate ControlEndpoint.grants to gemini permission flags
        //   (claude 는 --allowedTools mcp__{s}__{t} / Bash({e}:*)+PowerShell({e}:*); gemini 방언은 CLI spike 후 확정).
        _control: Option<ControlEndpoint>,
    ) -> CommandSpec {
        let mut args: Vec<String> = Vec::new();

        if let Some(sid) = session_id {
            // Gemini CLI의 세션 재개 플래그가 --session / --resume / --conversation 등인지 미확인.
            // Claude와 동일한 패턴을 best-guess로 선택(대부분의 AI CLI가 유사한 UX를 따른다고 가정).
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
                    program: GEMINI_PROGRAM.to_string(),
                    args: {
                        let _ = program;
                        shell_args.clone()
                    },
                    env,
                    cwd,
                };
            }
            // 이 backend 는 dispatch 에 배선되지 않았고, 배선된 형제(codex)의 인자를 흉내 내면
            //   그 형제의 지식이 여기로 샌다(ADR-0004).
            AgentCommand::Codex { .. } => {
                unreachable!("GeminiBackend 는 Codex variant 를 처리하지 않음. dispatch 버그.")
            }
        }

        CommandSpec {
            program: GEMINI_PROGRAM.to_string(),
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

    /// ★표본이 합성이다 — `backend_for` 는 이 백엔드에 claude 명령을 줄 수 없다★(gemini variant 자체가
    /// 없어 **어떤** 명령도 여기 닿지 않는다). 그래도 이 helper 를 쓰는 항목들이 재는 것이 있다:
    /// `build_spec` 은 명령을 실제로 match 해 `extra_args` 를 옮기고 세션 플래그를 조립하므로, 아래
    /// 단언들은 **그 조립**을 잰다.
    /// ★재지 **않는** 것 = 그 조립이 실 gemini 와 맞나★ — 프로그램 이름도 `--session`/`--resume` 도 전부
    ///   best-guess 이고 실 CLI 를 띄운 적이 없다(파일 헤더의 stub 선언 그대로). variant 를 들이는 작업이
    ///   표본을 진짜 명령으로 바꾸고 값을 실측으로 교체한다.
    fn spec(mode: SpawnMode, sid: Option<Uuid>) -> CommandSpec {
        GeminiBackend.build_spec(
            &AgentCommand::Claude {
                extra_args: vec![],
                output_format: crate::profile::AgentOutputFormat::Terminal,
            },
            mode,
            sid,
            None,
            PathBuf::from("."),
            vec![],
            None,
        )
    }

    #[test]
    fn gemini_program_name_is_correct() {
        let s = spec(SpawnMode::Fresh, None);
        assert_eq!(s.program, GEMINI_PROGRAM);
        assert_eq!(s.program, "gemini");
    }

    #[test]
    fn gemini_fresh_uses_session_flag_best_guess() {
        let sid = Uuid::new_v4();
        let s = spec(SpawnMode::Fresh, Some(sid));
        assert_eq!(s.program, GEMINI_PROGRAM);
        assert_eq!(s.args, vec!["--session".to_string(), sid.to_string()]);
    }

    #[test]
    fn gemini_resume_uses_resume_flag_best_guess() {
        let sid = Uuid::new_v4();
        let s = spec(SpawnMode::Resume, Some(sid));
        assert_eq!(s.args, vec!["--resume".to_string(), sid.to_string()]);
    }

    /// 표본이 합성인 사정과 그 한계는 위 [`spec`] 주석이 정본이다 — 이 항목만 `spec` 을 안 쓰는 것은
    /// cwd·env 를 직접 넘겨야 해서다.
    #[test]
    fn cwd_and_env_are_forwarded() {
        let cwd = PathBuf::from("C:/workspace");
        let env = vec![("GEMINI_KEY".to_string(), "dummy".to_string())];
        let s = GeminiBackend.build_spec(
            &AgentCommand::Claude {
                extra_args: vec![],
                output_format: crate::profile::AgentOutputFormat::Terminal,
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
