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
use crate::agent::types::{BackendCaps, CommandSpec, ControlEndpoint};

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

    /// 이 백엔드가 데몬 제어 채널(MCP 입구)을 **소비**하는가(ADR-0086 F3).
    /// true 면 manager 가 spawn 전에 provision 을 부르고(토큰+mcp-config 발급), 그 endpoint 를
    /// build_spec 에 넘긴다(claude=`--mcp-config`). false 면 manager 가 provision 을 **아예 건드리지
    /// 않는다** — shell 처럼 제어 채널을 안 쓰는 backend 는 registry 에 손대지 않아, config-write 실패가
    /// MCP 가 필요 없던 스폰을 중단시키는 회귀(round-2 F3)가 생기지 않는다.
    ///
    /// ★fail-closed 는 provision 을 **부르는** backend 에만★: true 인 backend 는 provision 이 Err 면
    ///   스폰이 중단된다(제어 채널 없이 몰래 도는 에이전트 금지). false 인 backend 는 그 계약과 무관하다.
    fn supports_control_channel(&self) -> bool;

    /// 프로필 + 모드 → CommandSpec.
    /// cwd·env는 manager가 정규화한 값을 전달한다(stage 6에서 주입 예정).
    ///
    /// `control`(ADR-0086): 데몬이 발급한 제어 채널 엔드포인트(추상 descriptor). 있으면 backend 가
    ///   자기 프로그램 방식으로 명령줄에 주입한다(claude=`--mcp-config <path>` — 그 지식은 claude.rs
    ///   단독, ADR-0004). None 이거나 제어 채널을 안 쓰는 backend(shell)면 무시한다.
    fn build_spec(
        &self,
        command: &AgentCommand,
        mode: SpawnMode,
        session_id: Option<Uuid>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
        control: Option<ControlEndpoint>,
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

/// 이 명령의 backend 가 데몬 제어 채널(MCP)을 소비하는가(ADR-0086 F3). manager 가 provision 호출 여부를
/// 이 dispatch 로 판정한다 — true(claude)면 provision, false(shell)면 registry 미접촉. ★backend dispatch
/// 로 판정(ADR-0004)★: manager 가 `matches!(command, ...)` 로 직접 분기하지 않고 backend 지식에 위임한다.
pub fn supports_control_channel(c: &AgentCommand) -> bool {
    backend_for(c).supports_control_channel()
}

/// 프로필 → CommandSpec. manager가 stage 6에서 호출한다.
/// `control`(ADR-0086): 데몬 제어 채널 엔드포인트 — backend 가 자기 방식으로 주입(claude=`--mcp-config`).
pub fn build_command_spec(
    c: &AgentCommand,
    mode: SpawnMode,
    session_id: Option<Uuid>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
    control: Option<ControlEndpoint>,
) -> CommandSpec {
    backend_for(c).build_spec(c, mode, session_id, cwd, env, control)
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
    ///
    /// `msg_uuid`: 이 유저 턴의 메시지 uuid(replay dedup 키). ClaudeStreamJson 은 stdin user 라인에
    ///   심어 claude 가 replay 시 그대로 되울리게 한다(uuid dedup 계약 — claude.rs wrap_user_turn).
    ///   같은 write_input 이 이 uuid 를 input_echo_event 에도 넘겨 합성 에코와 replay 를 uuid 로 합친다.
    ///   Raw(터미널·shell)는 uuid 를 쓰지 않는다(무시) — 바이트 동일 보장 유지.
    pub fn encode(&self, bytes: &[u8], msg_uuid: Uuid) -> Vec<u8> {
        match self {
            InputEncoder::Raw => bytes.to_vec(),
            // json 모드 입력은 텍스트다 — UTF-8 로 해석(lossy)해 claude 유저 턴으로 감싼다.
            // escape/스키마·uuid 부착은 claude.rs 단독(ADR-0004).
            // ※from_utf8_lossy(FIX 6b): 비-UTF8 입력은 U+FFFD 로 치환돼 손상될 수 있으나, json 모드
            //   입력은 텍스트 챗 메시지라 UTF-8 이 전제다(MVP=텍스트 챗, ADR-0044) → 허용.
            InputEncoder::ClaudeStreamJson => {
                claude::wrap_user_turn(&String::from_utf8_lossy(bytes), msg_uuid)
            }
        }
    }

    /// 입력 성공 직후 세션 층이 core.emit 할 **입력-시점 유저 에코 이벤트**를 만든다(ADR-0044/0045).
    ///
    /// ★왜 여기(backend) 인가★: 터미널(Raw)은 PTY 가 입력을 즉시 로컬 에코하지만, json 모드는 claude
    ///   가 되울릴 때까지 화면에 안 뜬다. 그 왕복 지연을 없애려 write_input 직후 합성 유저 이벤트를
    ///   emit 한다. 어떤 encoder 가 이 에코가 필요한지·이벤트의 json 스키마가 뭔지는 backend 지식이라
    ///   session 이 아니라 여기서 판정한다(ADR-0004 — session 은 encoder 태그만 들고 형태를 모른다).
    ///   Raw(터미널)는 None 을 돌려줘 세션이 아무 것도 emit 하지 않는다(PTY 가 이미 에코 — 중복 방지).
    ///
    /// ★decoder uuid dedup 과 짝(blunt-suppress → uuid dedup 교체)★: 이 이벤트는 decoder 가 replay 된
    ///   user-role 블록에 대해 만드는 것과 동일 shape(`Structured{kind:"user", json:{"type":"text",
    ///   "text":<raw>,"uuid":"X"}}`)이다. `msg_uuid` 가 stdin(encode)에 심은 값과 같아, 이후 claude 가
    ///   되울린 replay(같은 uuid)를 프론트 accumulator 가 uuid 로 dedup 해 한 개로 합친다. 예전엔 decoder 가
    ///   user text 블록을 blunt 억제해 이 합성 에코가 "자리 대체"했으나, resume 시 과거 대화가 사라지는
    ///   버그라 uuid dedup 으로 바꿨다(과거/비매칭 uuid user text 는 전부 보존).
    pub fn input_echo_event(
        &self,
        bytes: &[u8],
        msg_uuid: Uuid,
    ) -> Option<crate::agent::types::OutputEvent> {
        match self {
            // 터미널·shell: PTY 로컬 에코가 이미 있음 → 합성 에코 불필요(중복 방지).
            InputEncoder::Raw => None,
            // json 모드 claude: 유저 텍스트를 즉시 구조화 유저 이벤트로 에코. json 스키마·uuid 부착은
            // claude.rs 단독(ADR-0004). from_utf8_lossy 근거는 encode 와 동일(텍스트 챗 전제).
            InputEncoder::ClaudeStreamJson => Some(crate::agent::types::OutputEvent::Structured {
                kind: "user".to_string(),
                json: claude::user_text_echo_json(&String::from_utf8_lossy(bytes), msg_uuid),
            }),
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

// ── ADR-0079: resume 시 `.jsonl` transcript → 과거 이벤트 seed (backend dispatch) ──────

/// ADR-0079: resume 스폰 시 이 명령의 과거 대화를 복원한 `OutputEvent` 목록. json 모드 claude 만
/// `.jsonl` transcript 를 읽어 seed 한다(터미널 claude 는 TUI 가 PTY repaint 로 복원하므로 불필요,
/// shell 은 대화 개념 없음). 그 외 전부 빈 Vec(seed 안 함 = 기존 fresh 버퍼 동작 불변).
///
/// ★claude 지식 격리(ADR-0004)★: transcript 경로(cwd→슬러그)·파싱은 claude.rs 단독. manager 는 이
///   dispatch 만 부르고 파일 포맷·경로 규칙을 모른다. `output_decoder`(라이브 정제)의 resume 방향 짝.
pub fn resume_transcript_events(
    c: &AgentCommand,
    cwd: &std::path::Path,
    session_id: Uuid,
) -> Vec<crate::agent::types::OutputEvent> {
    // json 모드 claude 만 해당(터미널·shell 은 seed 불필요/불가). command 로 판정해 backend 격리 유지.
    match c {
        AgentCommand::Claude { .. } if c.is_json_mode() => {
            claude::read_transcript_events(cwd, session_id)
        }
        _ => Vec::new(),
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
        // 터미널 경로 회귀: Raw 는 입력 바이트를 그대로 돌려준다(변형 0). msg_uuid 는 무시된다.
        let input = b"echo hi\r\n\x1b[A\x03";
        assert_eq!(
            InputEncoder::Raw.encode(input, Uuid::new_v4()),
            input.to_vec()
        );
    }

    // ── ADR-0044/0045: 입력-시점 유저 에코 이벤트 dispatch(input_echo_event) — uuid dedup ──────
    #[test]
    fn input_echo_event_json_mode_emits_structured_user_with_uuid() {
        use crate::agent::types::OutputEvent;
        // json 모드 → Some(Structured{kind:"user", json:{"type":"text","text":<raw>,"uuid":"X"}}).
        //   uuid 는 write_input 이 encode 에 넘긴 것과 같은 값(dedup 키) — 여기선 부착 여부만 검증.
        let id = Uuid::new_v4();
        let ev = InputEncoder::ClaudeStreamJson
            .input_echo_event(b"hi there", id)
            .expect("json 모드 → 합성 유저 에코 이벤트");
        match ev {
            OutputEvent::Structured { kind, json } => {
                assert_eq!(kind, "user");
                let v: serde_json::Value = serde_json::from_str(&json).unwrap();
                assert_eq!(v["type"], "text");
                assert_eq!(v["text"], "hi there");
                assert_eq!(
                    v["uuid"],
                    id.to_string(),
                    "합성 에코에 msg_uuid 부착(dedup 키)"
                );
            }
            other => panic!("expected Structured user, got {other:?}"),
        }
    }

    #[test]
    fn input_echo_event_raw_is_none() {
        // 터미널·shell(Raw) → None. PTY 로컬 에코가 이미 있어 합성 에코를 추가하면 중복이 된다.
        assert!(
            InputEncoder::Raw
                .input_echo_event(b"echo hi\r\n", Uuid::new_v4())
                .is_none(),
            "Raw 는 합성 유저 에코를 만들지 않아야 함(PTY 에코 중복 방지)"
        );
    }

    #[test]
    fn claude_stream_json_encoder_wraps_and_terminates() {
        let id = Uuid::new_v4();
        let out = InputEncoder::ClaudeStreamJson.encode(b"hi", id);
        assert_eq!(*out.last().unwrap(), b'\n');
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\"type\":\"user\""));
        assert!(s.contains("\"text\":\"hi\""));
        // stdin user 라인에 msg_uuid 가 실려야 replay 가 그대로 되울린다(dedup 계약).
        assert!(
            s.contains(&id.to_string()),
            "stdin user 라인에 msg_uuid 포함"
        );
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
