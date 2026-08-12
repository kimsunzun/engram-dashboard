//! AgentBackend — 백엔드별 명령 명세 산출 trait + 자유 함수 dispatch.
//!
//! transport(PtyTransport)는 claude/codex를 모른다. 누가 어떤 프로그램인지 아는 곳은
//! 오직 backend/다.
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
use crate::agent::turn::TurnSignal;
use crate::agent::types::{BackendCaps, CommandSpec, ControlEndpoint, OutputEvent};

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

/// unit struct로 구현되어 &'static으로 사용된다 — 상태 없음.
pub trait AgentBackend: Send + Sync {
    /// true면 manager가 sid를 발급·watcher를 부착한다.
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

    /// 이 backend(프로그램)가 **MCP config 를 받아들일 수 있는가**(ADR-0099). claude=true(mcp-config 파일을
    /// `--mcp-config` 로 붙임), 그 외(shell·codex·gemini stub)=false. ★backend 지식(ADR-0004)★: "어느
    /// 프로그램이 MCP config 를 소비하나"는 backend-kind 지식이라 여기서 선언한다 — manager 가 `matches!`
    /// 로 직접 분기하지 않는다.
    ///
    /// 이 플래그 하나가 provision 의 MCP 입구·grant·프라이밍 변형·**우편 가부**를 전부 구동한다(정합
    /// 불변식 = 프라이밍이 가르치는 우편 채널 **=** 그 스폰이 쓸 수 있는 우편 채널. 못 쓰는 채널을 가르치면
    /// 발신 freeze 가 재발하고, 쓸 수 있는데 안 가르치면 통제 없는 우회 표면이 남는다). true 면
    /// `DaemonControlChannel::provision` 이 mcp-config 를 쓰고 MCP bits 를 endpoint 에 실으며 MCP-only 교육
    /// 프라이밍(`send_message` 만 — ADR-0126 결정 1)과 우편 불가 표식을, false 면 mcp-config 미기록 +
    /// CLI-only 프라이밍 + 우편 가능 표식을 고른다(ADR-0133). 제어 CLI 배선은 이 축과 무관하게 전원에게 간다.
    ///
    /// ★`supports_control_channel` 과의 관계★: 후자는 "provision 을 **부르나**"(제어 채널 자체를 소비하나),
    ///   이것은 "provision 이 붙일 채널 중 **MCP 를 낄 수 있나**"다 — 직교 축이다. 현재 claude 는 둘 다 true,
    ///   codex/gemini 는 둘 다 false 지만, 미래 "제어 채널은 CLI 로만 쓰는 백엔드"는 전자 true·후자 false 다.
    // ADR-0126
    // ADR-0133
    fn accepts_mcp_config(&self) -> bool;

    /// cwd·env는 manager가 정규화한 값을 전달한다.
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

    /// 이 backend 의 **턴 신호 분류자**(ADR-0113 사실 계층의 백엔드 지식 몫).
    ///
    /// ★왜 backend 인가(ADR-0004 · ADR-0110 결정 4 의 취지 승계)★: "어떤 출력 이벤트가 턴 진행이고
    ///   어떤 게 턴 종료인가" 는 프로그램별 지식이다. 특히 `OutputEvent::Structured` 는 **백엔드별
    ///   이벤트 탈출구**라 그 `kind` 의 의미가 백엔드마다 다르다 — 공용 층에서 해석하면 한 백엔드의
    ///   관례가 전원에게 강제된다(구조화 메타 라인을 내는 백엔드가 종료 신호 없이 영구 "턴 중" 이 된다).
    /// ★신호 어휘(`TurnSignal`)는 공용 하나뿐★: 백엔드마다 다른 건 **매핑**이지 어휘가 아니다.
    ///   백엔드별 신호 enum 을 만들면 소비자가 백엔드를 알아야 한다.
    /// ★기본값 = 신호 없음(fail-open)★: 매핑을 선언하지 않은 backend 는 관측 대상이 아니다. 미관측은
    ///   소비자 쪽에서 "즉시 배달" 로 흡수되지만(positive-knowledge-only), 근거 없는 진행 신호는
    ///   깨울 수 없는 "턴 중" 을 만든다 — 그래서 기본은 침묵이다.
    // ADR-0004
    // ADR-0113
    fn turn_classifier(&self) -> TurnClassifier {
        no_turn_signals
    }
}

/// 출력 이벤트 → 턴 신호 매핑 함수(ADR-0113). 백엔드가 자기 함수를 내주고 `OutputCore` 가 그 포인터를
/// 세션 수명 동안 들고 이벤트마다 부른다.
///
/// ★왜 함수 포인터인가★: emit 은 에이전트 출력마다 도는 hot path 다 — 매 이벤트에 `AgentCommand` 를
///   들고 dispatch 하면 세션이 자기 명령 사본을 들거나 매니저를 되짚어야 하고, `Box<dyn Fn>` 은 할당을
///   낳는다. backend 는 상태 없는 unit struct 라 매핑이 순수 함수로 떨어지므로, spawn 때 한 번 뽑아
///   포인터로 들고 있으면 할당·락·조회가 전부 0 이다.
pub type TurnClassifier = fn(&OutputEvent) -> Option<TurnSignal>;

/// 관측이 필요 없는 조립(테스트 하네스)도 이걸 꽂아 "신호 없음" 을 명시한다.
pub fn no_turn_signals(_event: &OutputEvent) -> Option<TurnSignal> {
    None
}

// ── 정적 싱글턴 ────────────────────────────────────────────────────────────────

static CLAUDE_BACKEND: ClaudeBackend = ClaudeBackend;
static SHELL_BACKEND: ShellBackend = ShellBackend;

// 새 variant 연결 시: tests::expected_channel_matrix(tripwire)가 의식적 capability 선언을 강제한다 — ADR-0099
fn backend_for(c: &AgentCommand) -> &'static dyn AgentBackend {
    match c {
        AgentCommand::Claude { .. } => &CLAUDE_BACKEND,
        AgentCommand::Shell { .. } => &SHELL_BACKEND,
    }
}

// ── 자유 함수 dispatch ─────────────────────────────────────────────────────────

pub fn needs_session(c: &AgentCommand) -> bool {
    backend_for(c).needs_session()
}

pub fn supports_control_channel(c: &AgentCommand) -> bool {
    backend_for(c).supports_control_channel()
}

pub fn accepts_mcp_config(c: &AgentCommand) -> bool {
    backend_for(c).accepts_mcp_config()
}

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

pub fn backend_caps(c: &AgentCommand) -> BackendCaps {
    backend_for(c).capabilities(c)
}

pub fn turn_classifier(c: &AgentCommand) -> TurnClassifier {
    backend_for(c).turn_classifier()
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
    /// `msg_uuid`: 이 유저 턴의 메시지 uuid(replay dedup 키). ClaudeStreamJson 은 stdin user 라인에
    ///   심어 claude 가 replay 시 그대로 되울리게 한다(uuid dedup 계약 — claude.rs wrap_user_turn).
    ///   같은 write_input 이 이 uuid 를 input_echo_event 에도 넘겨 합성 에코와 replay 를 uuid 로 합친다.
    ///   Raw(터미널·shell)는 uuid 를 쓰지 않는다(무시) — 바이트 동일 보장 유지.
    pub fn encode(&self, bytes: &[u8], msg_uuid: Uuid) -> Vec<u8> {
        match self {
            InputEncoder::Raw => bytes.to_vec(),
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
            InputEncoder::Raw => None,
            InputEncoder::ClaudeStreamJson => Some(crate::agent::types::OutputEvent::Structured {
                kind: "user".to_string(),
                json: claude::user_text_echo_json(&String::from_utf8_lossy(bytes), msg_uuid),
            }),
        }
    }
}

pub fn input_encoder(c: &AgentCommand) -> InputEncoder {
    if c.is_json_mode() {
        InputEncoder::ClaudeStreamJson
    } else {
        InputEncoder::Raw
    }
}

// ── 출력 정제(ADR-0044/0004/0045) — 입력 인코더의 대칭 짝 ──────────────────────────

/// pump→core 앞에 꽂히는 출력 정제 decoder. None = 바이트 직통(터미널·평문 불변).
///
/// ★대칭★: `input_encoder`(입력 방향)의 출력 방향 짝이다. 둘 다 "claude 스키마 지식"을
/// backend/claude.rs 에만 두는 격리(ADR-0004) — session 은 encoder 태그만, transport 는
/// `dyn OutputDecoder` 만 알고 claude 를 모른다. `Box<dyn OutputDecoder>` 반환이라 새 backend(codex 등)는
/// 자기 decoder 를 여기 분기에 추가하면 된다(교체성).
pub fn output_decoder(c: &AgentCommand) -> Option<Box<dyn OutputDecoder>> {
    if c.is_json_mode() {
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

    // ── ADR-0099 트립와이어: 새 AgentCommand variant 배선 시 capability 의식적 선언 강제 ──────
    //
    // ★ 와일드카드를 절대 추가하지 말 것 ★ — 새 AgentCommand variant가 생기면 이 match가
    // 컴파일 에러를 내서 아래 체크리스트를 강제로 방문하게 하는 장치다(목록 rot 방지 —
    // 갱신이 컴파일로 강제되는 목록은 rot하지 않는다).
    //
    // 새 variant 배선 시 체크리스트:
    //   ① 새 백엔드의 두 capability는 stub 복붙 금지 — CLI spike로 실측한 값으로 채울 것.
    //   ② supports_control_channel=false로 연결하면 메시징 없는 단독 에이전트가 된다(의도인지 확인).
    //   ③ 비-MCP(accepts_mcp_config=false)면 provision이 CLI판 프라이밍·[Cli] grant를 자동 선택한다
    //      — `roundtrip-smoke --cli-only`로 실측.
    //   ④ MCP-capable이면 기본 roundtrip으로 실측.
    //   참조: ADR-0099.
    fn expected_channel_matrix(c: &AgentCommand) -> (bool, bool) {
        // (supports_control_channel, accepts_mcp_config) — CLI spike 실측값
        match c {
            AgentCommand::Claude { .. } => (true, true),
            AgentCommand::Shell { .. } => (false, false),
        }
    }

    #[test]
    fn backend_channel_matrix_is_consciously_declared() {
        let variants: Vec<AgentCommand> = vec![
            AgentCommand::Claude {
                extra_args: vec![],
                output_format: ClaudeOutputFormat::Terminal,
            },
            AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
        ];

        for c in &variants {
            let (expected_control, expected_mcp) = expected_channel_matrix(c);
            let actual_control = supports_control_channel(c);
            let actual_mcp = accepts_mcp_config(c);
            assert_eq!(
                actual_control,
                expected_control,
                "variant {:?}: supports_control_channel 불일치 — backend mod.rs 상단 expected_channel_matrix 체크리스트를 따라 capability를 의식적으로 선언할 것(ADR-0099)",
                c
            );
            assert_eq!(
                actual_mcp,
                expected_mcp,
                "variant {:?}: accepts_mcp_config 불일치 — backend mod.rs 상단 expected_channel_matrix 체크리스트를 따라 capability를 의식적으로 선언할 것(ADR-0099)",
                c
            );
        }
    }

    // ── ADR-0113/0004: 턴 신호 분류자 dispatch(매핑은 백엔드 소유, 어휘는 공용) ──────────────

    #[test]
    fn turn_classifier_dispatch_maps_claude_events_and_silences_shell() {
        use crate::agent::types::OutputEvent;
        let json = AgentCommand::Claude {
            extra_args: vec![],
            output_format: ClaudeOutputFormat::StreamJson,
        };
        let shell = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let delta = OutputEvent::TextDelta {
            text: "x".into(),
            turn_id: None,
            message_id: None,
        };
        let done = OutputEvent::MessageDone {
            turn_id: None,
            message_id: None,
        };
        let claude_classify = turn_classifier(&json);
        assert_eq!(claude_classify(&delta), Some(TurnSignal::Progress));
        assert_eq!(claude_classify(&done), Some(TurnSignal::Ended));
        // ★`Structured` 해석이 백엔드별인 이유의 회귀★: claude 는 입력 시점 유저 에코를 여기 싣기에
        //   진행으로 세지만, 매핑을 선언하지 않은 backend 는 같은 이벤트에 침묵해야 한다 — 안 그러면
        //   턴과 무관한 구조화 메타 라인이 종료 신호 없는 영구 "턴 중" 을 만든다.
        let meta = OutputEvent::Structured {
            kind: "session_meta".into(),
            json: "{}".into(),
        };
        assert_eq!(claude_classify(&meta), Some(TurnSignal::Progress));
        let shell_classify = turn_classifier(&shell);
        for ev in [&delta, &done, &meta] {
            assert_eq!(
                shell_classify(ev),
                None,
                "매핑 미선언 backend 는 어떤 이벤트에도 신호를 내지 않는다(기본 = 침묵)"
            );
        }
    }

    #[test]
    fn input_encoder_dispatch_by_mode() {
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
        assert!(
            s.contains(&id.to_string()),
            "stdin user 라인에 msg_uuid 포함"
        );
    }

    // ── S15 B3: output_decoder dispatch(입력 encoder 의 대칭) ──────────────────────
    #[test]
    fn output_decoder_dispatch_by_mode() {
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
