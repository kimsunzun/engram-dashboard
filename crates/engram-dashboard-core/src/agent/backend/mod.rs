//! AgentBackend — 백엔드별 명령 명세 산출 trait + 자유 함수 dispatch.
//!
//! transport(PtyTransport)는 claude/codex를 모른다. 누가 어떤 프로그램인지 아는 곳은
//! 오직 backend/다. manager(stage 6)가 이 dispatch 함수를 호출해 CommandSpec을 받아
//! PtyTransport에 주입한다.
//!
//! tauri import 0.

pub mod claude;
pub mod codex;
pub mod gemini;
pub mod shell;

pub use claude::ClaudeBackend;
pub use codex::CodexBackend;
pub use gemini::GeminiBackend;
pub use shell::ShellBackend;

use std::path::PathBuf;

use uuid::Uuid;

use crate::agent::profile::{AgentCommand, SpawnMode};
use crate::agent::transport::OutputDecoder;
use crate::agent::types::{BackendCaps, CommandSpec};

/// 콘솔 CLI(claude/codex/gemini 등 npm 설치형)를 플랫폼에서 실행 가능한 (program, args)로 변환.
///
/// **왜 필요한가:** Windows에서 `claude`는 확장자 없는 npm shim이라, ConPTY가 쓰는 CreateProcessW가
/// 직접 못 띄운다(error 193 — PATHEXT/셸 해석을 안 함). `cmd.exe /c <prog> …`로 감싸면 cmd가
/// `<prog>.cmd` shim을 해석해 실제 프로세스를 띄운다. `cmd /c`는 대상이 종료되면 함께 종료되므로
/// "PTY 자식 = 에이전트" 수명이 유지된다(JobObject가 트리 통째 kill). 비Windows는 그대로 직접 실행.
///
/// shim이 아닌 일반 실행파일(Shell의 cmd.exe 등)에는 적용하지 않는다 — CLI 백엔드 전용.
pub(crate) fn console_command(program: &str, args: Vec<String>) -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        let mut wrapped = Vec::with_capacity(args.len() + 2);
        wrapped.push("/c".to_string());
        wrapped.push(program.to_string());
        wrapped.extend(args);
        ("cmd.exe".to_string(), wrapped)
    }
    #[cfg(not(windows))]
    {
        (program.to_string(), args)
    }
}

/// 백엔드별 명령 명세 산출 인터페이스.
/// unit struct로 구현되어 &'static으로 사용된다 — 상태 없음.
pub trait AgentBackend: Send + Sync {
    /// 이 백엔드가 claude 세션 추적 대상인가.
    /// true면 manager(stage 6)가 sid를 발급·watcher를 부착한다.
    fn needs_session(&self) -> bool;

    /// 프로필 + 모드 → CommandSpec.
    /// cwd·env는 manager가 정규화한 값을 전달한다(stage 6에서 주입 예정).
    fn build_spec(
        &self,
        command: &AgentCommand,
        mode: SpawnMode,
        session_id: Option<Uuid>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
    ) -> CommandSpec;

    /// 이 backend(프로그램)가 결정하는 caps — session(resume)·model.
    /// transport(물리 채널)가 만드는 input/output/control 과 별개로, 최종 Capabilities 는
    /// `Capabilities::compose(transport_caps, backend_caps)` 로 합성된다.
    ///
    /// `command` 를 받는 이유(FIX 5): 같은 프로그램(claude)이라도 **모드에 따라 caps 가 다르다** —
    /// json(stream-json) 모드는 resume 미지원(ADR-0044 후속)이라 resume=false 를 신고해야 한다.
    /// backend 가 session caps 의 출처(ADR-0030)이고 mode 는 command 에 있으므로, 여기서 command 를
    /// 보고 정직하게 산출한다(type split 유지 — output/control 은 여전히 transport 소관).
    fn capabilities(&self, command: &AgentCommand) -> BackendCaps;
}

// ── 정적 싱글턴 ────────────────────────────────────────────────────────────────

static CLAUDE_BACKEND: ClaudeBackend = ClaudeBackend;
static SHELL_BACKEND: ShellBackend = ShellBackend;

fn backend_for(c: &AgentCommand) -> &'static dyn AgentBackend {
    match c {
        AgentCommand::Claude { .. } => &CLAUDE_BACKEND,
        AgentCommand::Shell { .. } => &SHELL_BACKEND,
    }
}

// ── 자유 함수 dispatch ─────────────────────────────────────────────────────────

/// 이 명령이 claude 세션 추적 대상인가.
pub fn needs_session(c: &AgentCommand) -> bool {
    backend_for(c).needs_session()
}

/// 프로필 → CommandSpec. manager가 stage 6에서 호출한다.
pub fn build_command_spec(
    c: &AgentCommand,
    mode: SpawnMode,
    session_id: Option<Uuid>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
) -> CommandSpec {
    backend_for(c).build_spec(c, mode, session_id, cwd, env)
}

/// 이 명령의 backend(프로그램)가 결정하는 caps(session/model). manager 가 spawn 시 산출해
/// AgentSession 에 주입하고, session 이 transport caps 와 합성한다(`Capabilities::compose`).
/// command 를 backend 에 넘겨 mode 별 caps(예: json 모드 resume=false, FIX 5)를 정직하게 산출한다.
pub fn backend_caps(c: &AgentCommand) -> BackendCaps {
    backend_for(c).capabilities(c)
}

// ── 입력 인코딩(ADR-0044/0004) ────────────────────────────────────────────────

/// 세션 입력(write_input)을 transport 로 보내기 **직전** 인코딩 방식. AgentSession 이 spawn 시
/// 받아 보관하고 write_input 마다 적용한다.
///
/// ★설계 의도★: transport 는 항상 raw 바이트만 쓴다(바보 파이프 — ADR-0044). "텍스트 턴을
/// claude JSON 라인으로 감싸는" 지식은 backend 소유다. session 은 이 enum(태그)만 들고, 실제
/// 스키마는 `claude::wrap_user_turn` 안에만 산다(ADR-0004 격리 — session/transport 는 형태 모름).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEncoder {
    /// 바이트 그대로 통과(PTY/터미널·shell). 기존 동작과 **바이트 동일**.
    Raw,
    /// claude stream-json: 텍스트 1턴을 user JSON 라인(`\n` 종단)으로 감싼다(스키마=claude.rs).
    ClaudeStreamJson,
}

impl InputEncoder {
    /// 입력 바이트 인코딩. Raw 는 무변환 복사(passthrough) — 터미널 경로 바이트 동일 보장.
    pub fn encode(&self, bytes: &[u8]) -> Vec<u8> {
        match self {
            InputEncoder::Raw => bytes.to_vec(),
            // json 모드 입력은 텍스트다 — UTF-8 로 해석(lossy)해 claude 유저 턴으로 감싼다.
            // escape/스키마는 claude.rs 단독(ADR-0004).
            // ※from_utf8_lossy(FIX 6b): 비-UTF8 입력은 U+FFFD 로 치환돼 손상될 수 있으나, json 모드
            //   입력은 텍스트 챗 메시지라 UTF-8 이 전제다(MVP=텍스트 챗, ADR-0044) → 허용.
            InputEncoder::ClaudeStreamJson => {
                claude::wrap_user_turn(&String::from_utf8_lossy(bytes))
            }
        }
    }
}

/// 이 명령의 입력 인코딩 방식. json 모드 claude 만 ClaudeStreamJson, 그 외 전부 Raw(터미널 불변).
pub fn input_encoder(c: &AgentCommand) -> InputEncoder {
    if c.is_json_mode() {
        InputEncoder::ClaudeStreamJson
    } else {
        InputEncoder::Raw
    }
}

// ── 출력 정제(ADR-0044/0004/0045) — 입력 인코더의 대칭 짝 ──────────────────────────

/// 이 명령의 출력 정제 decoder(pump→core 앞에 꽂힘). json 모드 claude 만 `ClaudeStreamDecoder`
/// (stream-json NDJSON → 구조화 OutputEvent), 그 외 전부 None(바이트 직통 = 터미널·평문 불변).
///
/// ★대칭★: `input_encoder`(입력 방향)의 출력 방향 짝이다. 둘 다 "claude 스키마 지식"을
/// backend/claude.rs 에만 두는 격리(ADR-0004) — session 은 encoder 태그만, transport 는
/// `dyn OutputDecoder` 만 알고 claude 를 모른다. manager.spawn 이 이걸 산출해 StdioTransport 에
/// 주입한다(json→decoder, 그 외→None). `Box<dyn OutputDecoder>` 반환이라 새 backend(codex 등)는
/// 자기 decoder 를 여기 분기에 추가하면 된다(교체성).
pub fn output_decoder(c: &AgentCommand) -> Option<Box<dyn OutputDecoder>> {
    if c.is_json_mode() {
        // claude stream-json 라이브 decoder. 스키마 지식은 claude.rs 단독(ADR-0004).
        Some(Box::new(claude::ClaudeStreamDecoder::new()))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::profile::ClaudeOutputFormat;

    #[test]
    fn input_encoder_dispatch_by_mode() {
        // 터미널 claude·shell → Raw. json claude → ClaudeStreamJson.
        let term = AgentCommand::Claude {
            extra_args: vec![],
            output_format: ClaudeOutputFormat::Terminal,
        };
        let json = AgentCommand::Claude {
            extra_args: vec![],
            output_format: ClaudeOutputFormat::StreamJson,
        };
        let shell = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        assert_eq!(input_encoder(&term), InputEncoder::Raw);
        assert_eq!(input_encoder(&shell), InputEncoder::Raw);
        assert_eq!(input_encoder(&json), InputEncoder::ClaudeStreamJson);
    }

    #[test]
    fn raw_encoder_is_byte_identical() {
        // 터미널 경로 회귀: Raw 는 입력 바이트를 그대로 돌려준다(변형 0).
        let input = b"echo hi\r\n\x1b[A\x03";
        assert_eq!(InputEncoder::Raw.encode(input), input.to_vec());
    }

    #[test]
    fn claude_stream_json_encoder_wraps_and_terminates() {
        let out = InputEncoder::ClaudeStreamJson.encode(b"hi");
        assert_eq!(*out.last().unwrap(), b'\n');
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\"type\":\"user\""));
        assert!(s.contains("\"text\":\"hi\""));
    }

    // ── S15 B3: output_decoder dispatch(입력 encoder 의 대칭) ──────────────────────
    #[test]
    fn output_decoder_dispatch_by_mode() {
        // json claude → Some(decoder), 터미널 claude·shell → None(바이트 직통).
        let term = AgentCommand::Claude {
            extra_args: vec![],
            output_format: ClaudeOutputFormat::Terminal,
        };
        let json = AgentCommand::Claude {
            extra_args: vec![],
            output_format: ClaudeOutputFormat::StreamJson,
        };
        let shell = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        assert!(
            output_decoder(&term).is_none(),
            "터미널 모드 → decoder 없음(직통)"
        );
        assert!(
            output_decoder(&shell).is_none(),
            "shell → decoder 없음(직통)"
        );
        assert!(
            output_decoder(&json).is_some(),
            "json 모드 → ClaudeStreamDecoder 주입"
        );
    }

    #[test]
    fn output_decoder_produces_structured_events_through_trait_object() {
        // 배선 증명: dispatch 가 준 trait object 로 stream-json 라인을 decode 하면 구조화 이벤트가
        //   나온다(impl OutputDecoder for ClaudeStreamDecoder 위임 확인 — decode/flush 트레이트 경로).
        use crate::agent::types::OutputEvent;
        let json = AgentCommand::Claude {
            extra_args: vec![],
            output_format: ClaudeOutputFormat::StreamJson,
        };
        let mut dec = output_decoder(&json).expect("json → decoder");
        let mut ev = dec.decode(b"{\"type\":\"result\",\"subtype\":\"success\"}\n");
        ev.extend(dec.flush());
        assert!(
            ev.iter()
                .any(|e| matches!(e, OutputEvent::MessageDone { .. })),
            "trait object decode 가 result 라인을 MessageDone 으로 정제해야 함: {ev:?}"
        );
    }
}
