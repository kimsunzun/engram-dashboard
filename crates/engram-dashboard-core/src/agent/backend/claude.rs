//! ClaudeBackend — claude CLI 전용 CommandSpec 산출.
//!
//! 세션 인자(`--session-id`/`--resume`) 조립 규칙이 claude에 종속되므로 여기에만 둔다.
//! 현 `pty/claude.rs`의 build_command 로직과 완전히 동치이며,
//! 차이점은 (program, args) 대신 CommandSpec(cwd·env 포함)을 반환한다는 것뿐이다.
//!
//! tauri import 0.

use std::path::PathBuf;

use uuid::Uuid;

use crate::agent::backend::{console_command, AgentBackend};
use crate::agent::profile::{AgentCommand, ClaudeOutputFormat, SpawnMode};
use crate::agent::types::{BackendCaps, CommandSpec, ModelCaps, SessionCaps};

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
            AgentCommand::Claude {
                extra_args,
                output_format,
            } => {
                let mut args = Vec::with_capacity(6 + extra_args.len());
                match output_format {
                    // ── 터미널(PTY 대화형) — 기존 경로, 바이트/인자 완전 불변(회귀 금지) ──
                    ClaudeOutputFormat::Terminal => {
                        if let Some(sid) = session_id {
                            // Fresh → --session-id(우리가 sid를 통제), Resume → --resume(무손실 이어받기, ADR-0008).
                            let flag = match mode {
                                SpawnMode::Fresh => "--session-id",
                                SpawnMode::Resume => "--resume",
                            };
                            args.push(flag.to_string());
                            args.push(sid.to_string());
                        }
                    }
                    // ── JSON(헤드리스 stream-json) — ADR-0044 ──
                    // stream-json 입출력은 claude `-p` 전용(실측: --help "only works with --print").
                    // --replay-user-messages: 유저 턴을 출력 스트림에 되울림 → 프론트가 출력 단일 출처로 렌더.
                    ClaudeOutputFormat::StreamJson => {
                        args.push("-p".to_string());
                        args.push("--input-format".to_string());
                        args.push("stream-json".to_string());
                        args.push("--output-format".to_string());
                        args.push("stream-json".to_string());
                        args.push("--replay-user-messages".to_string());
                        // ★--verbose 필수(M2 QA 실측 확정, 2026-07-02)★: claude 2.1.170 은 --help 엔
                        //   문구가 없지만 런타임이 "When using --print, --output-format=stream-json
                        //   requires --verbose" 로 즉사시킨다(스폰 직후 에이전트 소멸로 발현). 빼면 안 됨.
                        args.push("--verbose".to_string());
                        if let Some(sid) = session_id {
                            // ★json 모드 resume 은 MVP 밖(ADR-0044 후속)★: mode 와 무관하게 항상
                            //   --session-id(fresh 경로)로 고정한다. 터미널 resume(ADR-0008)은 위
                            //   Terminal 분기에서 그대로 동작 — 여기서 건드리지 않는다.
                            // ★sid 재사용 충돌 위험(FIX 5)★: 여기서 매번 fresh sid 를 강제하므로
                            //   caps.session.resume 도 반드시 false 로 신고해야 한다(capabilities 참조)
                            //   — true 로 두면 M2 가 같은 sid 로 재사용/이어받기를 시도해 claude 가
                            //   "session already in use" 로 거부한다.
                            args.push("--session-id".to_string());
                            args.push(sid.to_string());
                        }
                    }
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

    /// 터미널 claude 는 `--resume` 으로 세션을 무손실 재개하므로 resume=true(이 backend 의 결정).
    /// cwd_env=true(작업 디렉토리에서 실행). snapshot·model 옵션은 미지원(콘솔 CLI).
    ///
    /// ★json 모드 resume 정직 신고(FIX 5 / ADR-0044 후속)★: json(stream-json) 경로는 build_spec 이
    ///   SpawnMode 와 무관하게 **항상 --session-id(fresh)** 로 고정한다(위 StreamJson 분기) — 매 spawn
    ///   새 sid. 그런데 caps 를 resume=true 로 신고하면 M2 가 "이 세션은 이어받기 가능"으로 오인해
    ///   같은 sid 로 --resume/재사용을 시도하고, claude 가 sid 충돌("session already in use")로 거부하는
    ///   지뢰가 된다. 통제-sid resume(ADR-0008)은 **터미널 경로 전용** 인프라이므로 json 모드는
    ///   resume=false 로 내린다. 터미널 모드는 그대로 true. mode 는 command 에서 읽는다(backend 가
    ///   session caps 의 출처 — ADR-0030, type split 유지).
    fn capabilities(&self, command: &AgentCommand) -> BackendCaps {
        // json 모드면 resume 불가(위 사유). 그 외(터미널 claude, 방어적 Shell)는 resume 가능.
        let resume = !command.is_json_mode();
        BackendCaps {
            session: SessionCaps {
                resume,
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

/// claude stream-json stdin 의 유저 턴 1줄(라인 종단 `\n`)을 만든다(ADR-0044 §4).
///
/// ★1 호출 = 완결된 유저 턴 1개(FIX 6a)★: `text` 를 유저 턴 1줄로 통째 감싼다 — 부분/한 글자
///   텍스트를 넘기면 그 조각이 그대로 한 턴이 돼 대화가 깨진다. 호출자(AgentSession.write_input →
///   RichSlot·M2)가 **완성된 메시지 전체**를 넘길 책임이다(계약 정본 = session.write_input 주석).
/// ★ADR-0004/0044 불변식★: 이 JSON 스키마(`{"type":"user","message":{...}}`)는 **이 함수 안에만**
///   존재한다. 스키마가 backend/claude.rs 밖으로 새면 ADR-0004(claude 지식 격리) 위반이다.
///   transport(StdioTransport)·session·통로는 최종 바이트만 알고 이 형태를 모른다.
/// ★정확한 escape★: 따옴표·개행·유니코드는 serde_json 이 처리한다 — 문자열 포맷팅으로 손조립 금지
///   (`"` 미escape 시 stdin JSON 파서가 깨진다).
/// claude 는 라인 단위로 stdin 을 파싱하므로 반드시 `\n` 으로 종단한다.
///
/// ★키 순서★: `serde_json::json!`(Value=BTreeMap)는 키를 알파벳순으로 재배열한다. claude 는 임의
///   순서를 받지만, 스키마를 사양(`{"type":"user","message":{"role":"user","content":[…]}}`) 그대로
///   드러내려고 **typed struct**로 직렬화한다 — serde 는 struct 필드를 선언 순서대로 쓰므로 순서가
///   결정적이고 사양과 일치한다. escape 는 serde_json 이 처리(손조립 금지).
pub(crate) fn wrap_user_turn(text: &str) -> Vec<u8> {
    // stream-json 유저 턴 스키마(선언 순서 = 직렬화 순서). `type` 은 Rust 예약어라 rename.
    #[derive(serde::Serialize)]
    struct UserTurn<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        message: UserMessage<'a>,
    }
    #[derive(serde::Serialize)]
    struct UserMessage<'a> {
        role: &'static str,
        content: [ContentBlock<'a>; 1],
    }
    #[derive(serde::Serialize)]
    struct ContentBlock<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        text: &'a str,
    }

    let turn = UserTurn {
        kind: "user",
        message: UserMessage {
            role: "user",
            content: [ContentBlock { kind: "text", text }],
        },
    };
    // to_string 은 이 형태에선 실패하지 않는다 — 방어적으로 unwrap_or_default.
    let mut line = serde_json::to_string(&turn).unwrap_or_default();
    line.push('\n');
    line.into_bytes()
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

    /// 터미널 모드 claude 명령(기존 경로 회귀 테스트용).
    fn terminal(extra: Vec<&str>) -> AgentCommand {
        AgentCommand::Claude {
            extra_args: extra.into_iter().map(String::from).collect(),
            output_format: ClaudeOutputFormat::Terminal,
        }
    }

    #[test]
    fn claude_fresh_uses_session_id_flag() {
        let sid = Uuid::new_v4();
        let s = spec(&terminal(vec!["--verbose"]), SpawnMode::Fresh, Some(sid));
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
        let s = spec(&terminal(vec![]), SpawnMode::Resume, Some(sid));
        let (_p, a) = console_command(
            CLAUDE_PROGRAM,
            vec!["--resume".to_string(), sid.to_string()],
        );
        assert_eq!(s.args, a);
    }

    #[test]
    fn claude_no_session_id_produces_no_flags() {
        let s = spec(&terminal(vec!["--debug"]), SpawnMode::Fresh, None);
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
    fn capabilities_terminal_resume_is_true() {
        // 터미널 claude 는 --resume 지원 → backend 가 resume=true 를 결정.
        assert!(ClaudeBackend.capabilities(&terminal(vec![])).session.resume);
    }

    #[test]
    fn capabilities_json_mode_resume_is_false() {
        // ★FIX 5★: json(stream-json) 모드는 매 spawn fresh --session-id 강제(build_spec) →
        //   resume 을 true 로 신고하면 sid 재사용 충돌 지뢰. backend 가 mode 를 보고 resume=false.
        assert!(
            !ClaudeBackend.capabilities(&json(vec![])).session.resume,
            "json 모드 claude 는 resume=false(sid fresh 강제)"
        );
    }

    #[test]
    fn cwd_and_env_are_forwarded() {
        let cwd = PathBuf::from("C:/workspace");
        let env = vec![("FOO".to_string(), "bar".to_string())];
        let s = ClaudeBackend.build_spec(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            cwd.clone(),
            env.clone(),
        );
        assert_eq!(s.cwd, cwd);
        assert_eq!(s.env, env);
    }

    // ── ADR-0044: json(stream-json) 모드 build_spec 골든 ─────────────────────────
    fn json(extra: Vec<&str>) -> AgentCommand {
        AgentCommand::Claude {
            extra_args: extra.into_iter().map(String::from).collect(),
            output_format: ClaudeOutputFormat::StreamJson,
        }
    }

    #[test]
    fn json_mode_build_spec_uses_headless_stream_json_args() {
        let sid = Uuid::new_v4();
        let s = spec(
            &json(vec!["--model", "sonnet"]),
            SpawnMode::Fresh,
            Some(sid),
        );
        // 기대 인자(console_command 래핑 전) — -p + stream-json 입출력 + replay + verbose + session-id + extra.
        // ★--verbose 필수(실측 확정 2026-07-02)★: 없으면 claude 가 "requires --verbose" 로 즉사(build_spec 주석).
        let (p, a) = console_command(
            CLAUDE_PROGRAM,
            vec![
                "-p".to_string(),
                "--input-format".to_string(),
                "stream-json".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--replay-user-messages".to_string(),
                "--verbose".to_string(),
                "--session-id".to_string(),
                sid.to_string(),
                "--model".to_string(),
                "sonnet".to_string(),
            ],
        );
        assert_eq!(s.program, p);
        assert_eq!(s.args, a, "json 모드 인자 골든 불일치");
        // --verbose 필수 포함(실측: 없으면 스폰 즉사).
        assert!(s.args.iter().any(|x| x == "--verbose"));
    }

    #[test]
    fn json_mode_resume_falls_back_to_session_id_not_resume() {
        // ★ADR-0044 후속★: json 모드는 resume 미지원 → SpawnMode::Resume 이어도 --session-id(fresh).
        let sid = Uuid::new_v4();
        let s = spec(&json(vec![]), SpawnMode::Resume, Some(sid));
        assert!(
            s.args.iter().any(|x| x == "--session-id"),
            "json resume 은 --session-id(fresh) 로 가야 함"
        );
        assert!(
            !s.args.iter().any(|x| x == "--resume"),
            "json 모드에서 --resume 을 쓰면 안 됨(MVP 밖)"
        );
    }

    #[test]
    fn terminal_mode_spec_unchanged_regression() {
        // 회귀: 터미널 모드는 -p/stream-json 인자가 전혀 없어야 함(기존 동작 불변).
        let sid = Uuid::new_v4();
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, Some(sid));
        for forbidden in ["-p", "--input-format", "--output-format", "stream-json"] {
            assert!(
                !s.args.iter().any(|x| x == forbidden),
                "터미널 모드에 json 인자 누출: {forbidden}"
            );
        }
    }

    // ── ADR-0044/0004: 입력 wrapping(stdin 유저 턴 JSON) 골든 ─────────────────────
    #[test]
    fn wrap_user_turn_exact_line_and_newline_terminated() {
        let bytes = wrap_user_turn("hello");
        // 정확한 라인 + \n 종단.
        assert_eq!(
            bytes,
            b"{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"hello\"}]}}\n".to_vec()
        );
        assert_eq!(*bytes.last().unwrap(), b'\n', "라인 종단 \\n 필수");
    }

    #[test]
    fn wrap_user_turn_escapes_quotes_newlines_unicode() {
        // 따옴표·개행·유니코드(한글)·백슬래시가 serde_json 으로 정확히 escape 돼야 stdin 파서가 안 깨진다.
        let bytes = wrap_user_turn("a\"b\nc\\d 한글 😀");
        let line = String::from_utf8(bytes).unwrap();
        // 한 줄(마지막 \n 외 내부 개행 없음) — 개행은 \\n 으로 escape.
        assert_eq!(
            line.matches('\n').count(),
            1,
            "내부 개행이 raw 로 새면 안 됨"
        );
        assert!(line.contains("\\\""), "따옴표 escape");
        assert!(line.contains("\\n"), "개행 escape");
        assert!(line.contains("\\\\d"), "백슬래시 escape");
        // 되파싱해 원문 복원 확인(round-trip) — text 필드가 정확히 보존.
        let v: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(v["message"]["content"][0]["text"], "a\"b\nc\\d 한글 😀");
        assert_eq!(v["type"], "user");
    }
}
