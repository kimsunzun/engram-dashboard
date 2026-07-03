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
use crate::agent::types::{BackendCaps, CommandSpec, ModelCaps, OutputEvent, SessionCaps};

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

// ── S15 B2: claude stream-json(NDJSON) → OutputEvent decoder (ADR-0044/0045) ────────
//
// ★층 소속(ADR-0004)★: claude stream-json 스키마 지식(assistant/user/result 라인, content[]
//   ContentBlock 4종)은 **이 파일 안에만** 존재한다. transport(StdioTransport)는 바보 파이프라
//   바이트만 알고(ADR-0044), core(OutputCore)는 wire/직렬화 형식을 모른다(ADR-0003). 그래서
//   "바이트 → OutputEvent" 재조립·파싱을 backend 인 여기가 소유한다.
//
// ★core 도메인 타입만 생성(ADR-0003)★: decoder 는 core 도메인 타입 `OutputEvent` 값만 만든다
//   (Serialize 미부착). core↔wire 변환은 daemon adapter 몫이다.
//
// ★스코프★: 이 유닛은 standalone decoder 다 — pump/session/manager 배선은 별도 모듈(B3/B4)이며
//   여기서 하지 않는다. 정본 스키마·매핑 근거 = 프론트 파서(src/lab/richslot/parse.ts,
//   streamParse.ts) + 실측 fixture(backend/fixtures/claude_{text,tool}.jsonl).

/// ★미종결 라인 버퍼 상한★: 개행이 영영 오지 않는 malformed/폭주 출력이면 버퍼가 무한 증가해
///   OOM 을 낸다. 통로는 바보 파이프(ADR-0044 무정제 불변)라 상류가 라인을 보장하지 않으므로
///   소비자(decoder)가 방어한다 — 4MB 넘으면 부분 라인을 버리고 다음 개행부터 복구한다. NDJSON
///   한 라인이 4MB 를 넘는 정상 케이스는 없다(thinking/text 블록도 그보다 훨씬 작다) → 상한 초과
///   = 비정상으로 간주. 프론트 streamParse.ts 의 MAX_BUFFER_CHARS(4MB) 이식(단 여기선 바이트 단위).
const MAX_BUFFER_BYTES: usize = 4 * 1024 * 1024;

/// claude stream-json 라이브 decoder. `decode`로 임의 크기 바이트 청크를 밀어 넣으면 완성된
/// NDJSON 라인마다 파싱해 `Vec<OutputEvent>`를 돌려준다. EOF 시 `flush`로 개행 없는 잔여 라인을
/// 처리한다.
///
/// ★유일한 상태 = 부분 라인 바이트 버퍼★: 메시지 병합(같은 message.id 블록 concat)은 decoder
///   책임이 아니다(프론트 RichSlot 이 함) — decoder 는 라인만 재조립하고 라인별로 파싱해 뱉는다.
///   그래서 상태는 "마지막 개행 뒤 미완성 라인 바이트"뿐이다.
#[derive(Debug, Default)]
pub struct ClaudeStreamDecoder {
    /// 마지막 `\n` 뒤 미완성 라인 바이트(라인-레벨 분할 재조립용).
    ///
    /// ★불변식(load-bearing)★: **완성 라인(개행까지)이 확정되기 전에는 절대 UTF-8 디코딩하지
    ///   않는다.** pump 는 NDJSON 라인 경계·문자 경계를 무시하고 임의 바이트 청크(최대 4096B)로
    ///   던지므로, 멀티바이트 UTF-8 문자(한글·이모지)가 청크 경계에서 잘릴 수 있다. 바이트로만
    ///   이어붙였다가 `\n` 이 온 완성 라인만 디코딩하면 경계 잘림이 자연 흡수된다(개행 `0x0A` 는
    ///   UTF-8 연속 바이트로 등장할 수 없어 라인 경계 탐색이 바이트 레벨에서 안전하다).
    buffer: Vec<u8>,

    /// ★오버플로 resync 상태(FIX-A)★: 오버플로한 오염 라인의 **잔여 꼬리를 다음 `\n` 까지 통째
    ///   폐기**하는 중인가. true 인 동안 들어오는 바이트는 개행이 나올 때까지 버린다 — 개행을 만나면
    ///   false 로 풀고 그 뒤부터 정상 라인 처리를 재개한다.
    ///
    /// ★왜 필요한가★: 단순 `buffer.clear()` 만으로는 오염 라인의 꼬리(아직 도착 안 한 나머지
    ///   바이트, 그리고 clear 후 이어 붙는 바이트)가 다음 `\n` 까지 "새 라인"으로 파싱돼 **가짜
    ///   이벤트**를 낼 수 있다(꼬리에 우연히 valid JSON 조각이 있으면 특히). 오염 라인은 1개만
    ///   손실하고, **그 라인이 끝나는 `\n` 이후부터** 온전히 복구하려면 "다음 개행까지 버리는"
    ///   상태가 있어야 한다. (프론트 streamParse.ts 는 clear 만 하지만 — 아래 decode 주석 참조.)
    discarding: bool,
}

impl ClaudeStreamDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 바이트 청크를 밀어 넣고, 이번 청크로 **완성된 라인**들만 파싱해 이벤트를 돌려준다.
    /// 꼬리(개행 없는 미완성 라인)는 버퍼에 남겨 다음 청크와 합친다.
    pub fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent> {
        let mut events = Vec::new();

        // ★resync(FIX-A)★: 이전 오버플로로 오염 라인의 꼬리를 버리는 중이면, 이번 청크에서 먼저
        //   다음 `\n` 을 찾아 그 앞(오염 라인의 잔여)을 통째 버린다. 개행을 못 찾으면 청크 전체가
        //   아직 오염 라인의 일부이므로 전부 버리고(버퍼에 안 쌓음) 종료 — 다음 청크에서 계속 찾는다.
        let chunk = if self.discarding {
            match chunk.iter().position(|&b| b == b'\n') {
                // 개행 발견 → 오염 라인 종료. 그 개행 다음 바이트부터 정상 처리 재개.
                Some(nl) => {
                    self.discarding = false;
                    &chunk[nl + 1..]
                }
                // 개행 없음 → 아직 오염 라인 진행 중. 전부 버리고 discarding 유지.
                None => return events,
            }
        } else {
            chunk
        };

        self.buffer.extend_from_slice(chunk);

        // `\n` 기준으로 완성 라인만 잘라 소비. 최초 개행부터 라인 단위로 반복해 drain 하고, 마지막
        // 개행 뒤 잔여는 tail 로 buffer 에 남겨 다음 청크와 합친다(FIX-D: 주석을 실제 코드와 일치).
        while let Some(nl) = self.buffer.iter().position(|&b| b == b'\n') {
            // 라인 = buffer[..nl] (개행 제외). drain 으로 라인+개행을 버퍼에서 제거한다.
            let line: Vec<u8> = self.buffer.drain(..=nl).collect();
            // 개행 1바이트를 뺀 라인 바이트. (CRLF 대비 \r 도 뒤에서 trim 처리)
            Self::consume_line(&line[..line.len() - 1], &mut events);
        }

        // 완성 라인을 모두 소비한 뒤 남은 미종결 tail 이 상한을 넘으면 오염 라인으로 간주하고 버린다.
        // ★단순 clear 가 아니라 resync 진입(FIX-A)★: 여기까지 온 tail 은 개행이 없는 초장문 라인의
        //   앞부분이다. buffer 를 비우는 것만으로 끝내면, 이 오염 라인의 **나머지 꼬리**(아직 도착
        //   안 한 바이트 + 이후 청크)가 다음 `\n` 까지 새 라인으로 파싱돼 가짜 이벤트를 낼 수 있다.
        //   그래서 discarding=true 로 들어가 그 오염 라인의 꼬리를 다음 개행까지 통째 버린다 — 오염
        //   라인 1개만 손실하고 그 다음 정상 라인부터 온전히 복구한다.
        if self.buffer.len() > MAX_BUFFER_BYTES {
            let dropped = self.buffer.len();
            self.buffer.clear();
            self.discarding = true;
            events.push(OutputEvent::Error(format!(
                "claude stream-json decoder: partial-line buffer overflow — dropping {dropped} bytes (no line terminator); resyncing to next newline"
            )));
        }
        events
    }

    /// EOF(스트림 종료) 시 호출 — 개행으로 종단되지 않은 마지막 라인을 처리한다.
    /// 정상 종료면 버퍼가 비어 있어 이벤트 0개다(마지막 라인도 `\n` 종단이 관례).
    // ★불변식★: discarding=true 일 땐 buffer 가 항상 비어 있다(overflow 시 clear + discarding 중
    //   미적재). 따라서 그 상태의 flush 는 이벤트 0개다 — flush 에서 discarding 잔여를 처리하려
    //   들지 말 것(처리할 잔여가 없다). 로직 추가는 이 불변식을 깨는 회귀다.
    pub fn flush(&mut self) -> Vec<OutputEvent> {
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
            Self::consume_line(&line, &mut events);
        }
        events
    }

    /// 완성 라인 1개(개행 제외 바이트) → 0개 이상의 OutputEvent 를 events 에 append.
    ///
    /// 파싱 규칙 — 실패·메타는 조용히 skip(panic 금지):
    /// - 비-UTF8 / 비-JSON(예: stderr "Warning: no stdin…") → skip.
    /// - `assistant`/`user` 라인 → message.content[] 의 각 블록을 순서대로 이벤트로.
    ///   (블록 타입→이벤트 매핑의 정본은 프론트 parse.ts — content[] 스키마 해석만 공유.)
    /// - `result` 라인 → MessageDone(+ result.usage 있으면 Usage 추가 emit;
    ///   is_error/subtype 이 error 계열이면 MessageDone **앞에** Error 도 emit — FIX-C).
    ///   ※ result 의 is_error/subtype 오류 표면화는 **백엔드 신규 정책**이다 — parse.ts 는
    ///   result 의 subtype/is_error 를 전혀 검사하지 않는다(`return {kind:'result'}` 뿐).
    /// - `system`/`rate_limit_event`/그 외 unknown type → skip(0개).
    fn consume_line(line: &[u8], events: &mut Vec<OutputEvent>) {
        // ★여기서 처음 UTF-8 디코딩★(위 buffer 불변식). 라인 하나가 완성됐으므로 문자 경계 잘림
        //   위험이 없다. 그래도 방어적으로 lossy 를 쓰지 않고 엄격 검증 후 실패 시 skip 한다 —
        //   비-UTF8 라인은 claude 정상 출력이 아니므로 조용히 버린다(터미널 경로가 아니다).
        let text = match std::str::from_utf8(line) {
            Ok(t) => t.trim(), // 앞뒤 공백·CR(\r, CRLF 대비) 제거
            Err(_) => return,  // 비-UTF8 → skip
        };
        if text.is_empty() {
            return; // 빈 줄·개행만 있는 청크
        }

        let value: serde_json::Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(_) => return, // 비-JSON(stderr 경고 등) → skip
        };

        match value.get("type").and_then(|t| t.as_str()) {
            // assistant/user 는 message.content[] 배열의 각 블록을 이벤트로. message.id 는 병합
            // 키로 프론트가 쓰지만, decoder 는 message_id 필드에 실어 그대로 전달만 한다(병합 X).
            Some(role @ ("assistant" | "user")) => {
                let msg = match value.get("message") {
                    Some(m) => m,
                    None => return,
                };
                let message_id = msg.get("id").and_then(|v| v.as_str()).map(String::from);
                let blocks = match msg.get("content").and_then(|c| c.as_array()) {
                    Some(arr) => arr,
                    None => return, // content 가 배열이 아니면(스키마 이탈) skip
                };
                for block in blocks {
                    Self::consume_block(role, block, message_id.as_deref(), events);
                }
            }
            // result = 턴 종료. usage 가 있으면 토큰을 추가 emit(선택적).
            Some("result") => {
                // ★Usage 를 MessageDone 보다 먼저 emit★: 소비자가 "턴 종료" 신호를 보기 전에
                //   그 턴의 최종 토큰 집계를 받게 순서를 고정한다(MessageDone 뒤 Usage 면 종료 후
                //   지연 도착처럼 보인다). result.usage.{input_tokens,output_tokens} — 실측 fixture
                //   확인(text.jsonl 라인5: input=17095, output=4).
                // ★0/0 usage 스킵은 의도된 노이즈 방지★: input/output 둘 다 0이면 유의미한 usage 가
                //   아니므로(빈 집계) Usage 를 만들지 않는다.
                if let Some(usage) = value.get("usage") {
                    let input_tokens = usage
                        .get("input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let output_tokens = usage
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    // 0/0 은 유의미 usage 아님 → 스킵(의미 없는 0 토큰 노이즈 방지).
                    if input_tokens != 0 || output_tokens != 0 {
                        events.push(OutputEvent::Usage {
                            input_tokens,
                            output_tokens,
                            // stream-json 라인엔 우리 도메인의 turn 개념이 없다(session_id 는 별개) → None.
                            turn_id: None,
                        });
                    }
                }
                // ★실패 턴 표면화(FIX-C)★: result 라인이 is_error 든 아니든 늘 MessageDone 만 내면
                //   API 오류·max-turns·거부로 실패한 턴이 "정상 완료"로 위장된다. is_error==true(또는
                //   subtype 이 error 계열)면 MessageDone **에 더해** Error 를 emit 해 소비자가 실패를
                //   인지하게 한다. is_error:true payload 는 미캡처(실측 fixture 없음)라 방어적으로:
                //   존재하는 필드만 문자열화해 메시지에 담는다.
                // ★순서(Error 먼저)★: 소비자가 종료 신호(MessageDone)를 보기 전에 오류를 알도록
                //   Error 를 MessageDone 보다 먼저 push 한다.
                let is_error = value
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let subtype = value.get("subtype").and_then(|v| v.as_str());
                // ★error allowlist(denylist 아님)★: 오류로 잡는 건 subtype 이 error 계열일 때만이다
                //   (실측 error_max_turns·error_during_execution → s.starts_with("error") 로 커버).
                //   과거엔 `s != "success"`(여집합=denylist)였으나, 유저가 Esc 로 정상 중단한 턴의
                //   subtype:"interrupted" 마저 오류로 오분류했다 — interrupt 는 이 프로젝트 1급 정상
                //   경로(TerminalReason::Interrupted 별도)라 실패 턴으로 위장하면 안 된다. 또 denylist 는
                //   미래에 추가될 non-error subtype 을 자동으로 오류化한다. 그래서 방향을 뒤집어, 알려진
                //   error 접두사만 오류로 잡고 나머지(success·interrupted·미지 non-error)는 오류 아님.
                let subtype_is_error = subtype.map(|s| s.starts_with("error")).unwrap_or(false);
                if is_error || subtype_is_error {
                    // 가용 정보만 담아 진단 메시지 조립: subtype + result 텍스트(있으면).
                    let mut detail = String::from("claude stream-json result reported failure");
                    if let Some(s) = subtype {
                        detail.push_str(&format!(" (subtype={s})"));
                    }
                    if let Some(r) = value.get("result").and_then(|v| v.as_str()) {
                        detail.push_str(&format!(": {r}"));
                    }
                    events.push(OutputEvent::Error(detail));
                }
                events.push(OutputEvent::MessageDone {
                    turn_id: None,
                    message_id: None,
                });
            }
            // system/init·rate_limit_event·thinking_tokens 등 메타 라인, unknown type → skip.
            _ => {}
        }
    }

    /// content[] 한 블록 → OutputEvent. 매핑 근거는 각 arm 주석(정본 = 과업 매핑표 + parse.ts).
    fn consume_block(
        role: &str,
        block: &serde_json::Value,
        message_id: Option<&str>,
        events: &mut Vec<OutputEvent>,
    ) {
        // ★user 라인 블록은 통째로 Structured{kind:"user"} 로 보존★: OutputEvent 에 role 개념이
        //   없어(assistant 전용 필드만) user replay(--replay-user-messages) 턴을 정형 variant 로
        //   표현할 수 없다 → 원본 블록을 그대로 직렬화해 탈출구로 넘긴다. (블록 type 별로 쪼개지
        //   않는다 — user 턴은 렌더층이 통째로 해석.)
        if role == "user" {
            events.push(Self::structured("user", block));
            return;
        }

        // assistant 라인: 블록 type 별 매핑.
        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                // text 블록 → TextDelta. (통짜 모드라 실은 델타가 아닌 완결 텍스트지만, OutputEvent
                //   에 "완결 텍스트" variant 가 없고 TextDelta 가 텍스트 증분의 정형 표현이다. 병합은
                //   프론트 몫 — decoder 는 라인별로 그대로 흘린다.)
                // ★malformed 계약(FIX-B)★: 문자열 `text` 가 없으면(스키마 이탈) 빈 TextDelta 를
                //   방출하지 않고 skip 한다 — 빈 델타는 다운스트림에 무의미한 노이즈이고, "정상 text
                //   블록인데 내용이 빈 문자열"과 구분도 안 된다. (Structured 보존 대신 skip 선택:
                //   text 결손은 tool_use name 결손과 달리 매칭 정보 유실이 없어 조용히 버려도 안전.)
                let Some(text) = block.get("text").and_then(|v| v.as_str()) else {
                    return;
                };
                events.push(OutputEvent::TextDelta {
                    text: text.to_string(),
                    turn_id: None,
                    message_id: message_id.map(String::from),
                });
            }
            Some("tool_use") => {
                // tool_use → ToolCall. input(임의 JSON 객체)을 그대로 문자열화해 args_json 에 싣는다
                //   (backend 별 스키마 그대로 — OutputEvent 주석 계약). id 는 tool_use.id(결과 매칭용).
                // ★malformed 계약(FIX-B)★: 문자열 `name` 이 없으면(스키마 이탈) 빈 name 의 가짜
                //   ToolCall 을 만들지 않는다 — 빈 name 호출은 다운스트림에 "이름 없는 도구 실행"으로
                //   위장돼 위험하다. 대신 원본 블록을 Structured{kind:"tool_use"} 로 통째 보존한다
                //   (정보 유실·가짜 호출 둘 다 회피 — 렌더층이 원본을 보고 판단).
                let Some(name) = block.get("name").and_then(|v| v.as_str()) else {
                    events.push(Self::structured("tool_use", block));
                    return;
                };
                let id = block.get("id").and_then(|v| v.as_str()).map(String::from);
                let args_json = block
                    .get("input")
                    .map(|v| v.to_string())
                    // input 이 없으면 빈 객체로(스키마 이탈 방어) — args_json 은 항상 유효 JSON.
                    .unwrap_or_else(|| "{}".to_string());
                events.push(OutputEvent::ToolCall {
                    name: name.to_string(),
                    args_json,
                    id,
                    turn_id: None,
                    message_id: message_id.map(String::from),
                });
            }
            // thinking·tool_result 는 정형 variant 가 없다 → Structured 탈출구로 원본 블록 보존.
            // (thinking = 추론 블록, tool_result 는 통상 user 라인에 실려 위 user 분기가 먹지만,
            //  방어적으로 assistant 라인에 와도 탈출구로 흡수. kind 는 블록 type 그대로.)
            Some(kind @ ("thinking" | "tool_result")) => {
                events.push(Self::structured(kind, block));
            }
            // unknown 블록 type → 정형화 못 하니 탈출구로 보존(forward-compat: 새 블록 종류 유실 방지).
            Some(other) => {
                events.push(Self::structured(other, block));
            }
            None => {} // type 없는 블록(스키마 이탈) → skip.
        }
    }

    /// Structured 탈출구 헬퍼 — 블록/라인을 원본 그대로 직렬화해 kind 태그와 함께 보존.
    fn structured(kind: &str, value: &serde_json::Value) -> OutputEvent {
        OutputEvent::Structured {
            kind: kind.to_string(),
            json: value.to_string(),
        }
    }
}

// ── S15 B3: pump→core 배선 seam (ADR-0004/0044) ──────────────────────────────────
//
// ★claude 지식은 계속 여기만★: transport(StdioTransport)는 `dyn OutputDecoder` 만 알고 claude 를
//   모른다(ADR-0004). manager 가 json 모드 세션에 `Box::new(ClaudeStreamDecoder::new())` 를 만들어
//   StdioTransport 에 주입하면, pump 가 이 트레이트 메서드로 바이트를 정제해 core 로 흘린다.
//   inherent decode/flush(위 impl)를 그대로 위임 — 파싱 로직은 한 벌만 존재한다.
impl crate::agent::transport::OutputDecoder for ClaudeStreamDecoder {
    fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent> {
        ClaudeStreamDecoder::decode(self, chunk)
    }
    fn flush(&mut self) -> Vec<OutputEvent> {
        ClaudeStreamDecoder::flush(self)
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

    // ── S15 B2: ClaudeStreamDecoder(stream-json → OutputEvent) ────────────────────
    //
    // 정본 = 실측 fixture(backend/fixtures/claude_{text,tool}.jsonl, claude 2.1.170 캡처).
    // include_str! 는 이 소스 파일 기준 상대경로라 경로가 안정적이다(cwd 무관).

    const TEXT_JSONL: &str = include_str!("fixtures/claude_text.jsonl");
    const TOOL_JSONL: &str = include_str!("fixtures/claude_tool.jsonl");

    /// 이벤트를 사람이 읽기 쉬운 태그 문자열로 요약(시퀀스 단언용 — 매핑을 그대로 검증).
    fn tags(events: &[OutputEvent]) -> Vec<String> {
        events
            .iter()
            .map(|e| match e {
                OutputEvent::TerminalBytes(_) => "terminal".to_string(),
                OutputEvent::TextDelta { .. } => "text".to_string(),
                OutputEvent::ToolCall { name, .. } => format!("tool:{name}"),
                OutputEvent::Usage { .. } => "usage".to_string(),
                OutputEvent::MessageDone { .. } => "done".to_string(),
                OutputEvent::Error(_) => "error".to_string(),
                OutputEvent::Structured { kind, .. } => format!("structured:{kind}"),
            })
            .collect()
    }

    /// 통짜(한 방에) 디코드 헬퍼 — 바이트 전체를 한 번에 decode + flush.
    fn decode_all(bytes: &[u8]) -> Vec<OutputEvent> {
        let mut d = ClaudeStreamDecoder::new();
        let mut out = d.decode(bytes);
        out.extend(d.flush());
        out
    }

    #[test]
    fn text_fixture_maps_to_text_then_usage_done() {
        // text.jsonl: Warning(비-JSON)·system·rate_limit → skip / assistant[text "hello"] / result(usage).
        let events = decode_all(TEXT_JSONL.as_bytes());
        assert_eq!(tags(&events), vec!["text", "usage", "done"]);

        // text 내용·message_id 정확성.
        match &events[0] {
            OutputEvent::TextDelta {
                text, message_id, ..
            } => {
                assert_eq!(text, "hello");
                assert_eq!(message_id.as_deref(), Some("msg_01QDurZCCdyuXSWuV5NwWr6c"));
            }
            other => panic!("expected TextDelta, got {other:?}"),
        }
        // result.usage 추출(실측: input=17095, output=4).
        match &events[1] {
            OutputEvent::Usage {
                input_tokens,
                output_tokens,
                ..
            } => {
                assert_eq!(*input_tokens, 17095);
                assert_eq!(*output_tokens, 4);
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn tool_fixture_maps_thinking_tooluse_toolresult_text() {
        // tool.jsonl 실측 시퀀스:
        //  9  assistant[thinking]           → structured:thinking
        //  10 assistant[tool_use Read]      → tool:Read  (같은 msg id, disjoint 배치 — decoder 는 병합 안 함)
        //  11 user[tool_result]             → structured:user  (user 라인은 통째 보존)
        //  16 assistant[thinking]           → structured:thinking
        //  17 assistant[text]               → text
        //  18 result(usage)                 → usage, done
        // (system/status·init·rate_limit·thinking_tokens 메타 라인은 전부 skip)
        let events = decode_all(TOOL_JSONL.as_bytes());
        assert_eq!(
            tags(&events),
            vec![
                "structured:thinking",
                "tool:Read",
                "structured:user",
                "structured:thinking",
                "text",
                "usage",
                "done",
            ]
        );

        // tool_use 매핑 세부: id·args_json(input 직렬화) 보존.
        let tool = events
            .iter()
            .find(|e| matches!(e, OutputEvent::ToolCall { .. }))
            .unwrap();
        match tool {
            OutputEvent::ToolCall {
                name,
                args_json,
                id,
                message_id,
                ..
            } => {
                assert_eq!(name, "Read");
                assert_eq!(id.as_deref(), Some("toolu_01LDdR9FU6CFjgEKeLPF1x1D"));
                assert_eq!(message_id.as_deref(), Some("msg_01DXXosoarwv9i1cBXa8wVXJ"));
                // args_json 은 input 객체를 그대로 직렬화한 유효 JSON — file_path 필드 보존.
                let v: serde_json::Value = serde_json::from_str(args_json).unwrap();
                assert_eq!(
                    v["file_path"],
                    "I:\\Engram\\apps\\engram-dashboard\\package.json"
                );
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn chunk_boundary_invariance_arbitrary_offsets() {
        // 청크 경계 불변: fixture 전체 바이트를 여러 오프셋 크기로 쪼개 순차 투입해도, 통짜로 넣은
        // 것과 동일한 이벤트 시퀀스가 나와야 한다(라인 재조립이 청크 분할에 견고).
        for fixture in [TEXT_JSONL, TOOL_JSONL] {
            let whole = tags(&decode_all(fixture.as_bytes()));
            for chunk_size in [1usize, 3, 7, 64, 4096] {
                let mut d = ClaudeStreamDecoder::new();
                let mut ev = Vec::new();
                for c in fixture.as_bytes().chunks(chunk_size) {
                    ev.extend(d.decode(c));
                }
                ev.extend(d.flush());
                assert_eq!(
                    tags(&ev),
                    whole,
                    "chunk_size={chunk_size} 에서 시퀀스 불일치"
                );
            }
        }
    }

    #[test]
    fn utf8_multibyte_split_across_chunks_is_recovered() {
        // 한글·이모지가 든 라인을 멀티바이트 문자 중간 바이트에서 쪼개도 깨짐 없이 복원돼야 한다
        // (완성 라인 전엔 UTF-8 디코딩 안 함 불변식 검증).
        let line = r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"안녕 😀 world"}]}}"#;
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\n');

        // 통짜 기준값.
        let whole = decode_all(&bytes);
        let whole_text = match &whole[0] {
            OutputEvent::TextDelta { text, .. } => text.clone(),
            other => panic!("expected TextDelta, got {other:?}"),
        };
        assert_eq!(whole_text, "안녕 😀 world");

        // 1바이트씩 쪼개 투입(멀티바이트 경계가 반드시 갈림) → 동일 복원.
        let mut d = ClaudeStreamDecoder::new();
        let mut ev = Vec::new();
        for b in &bytes {
            ev.extend(d.decode(std::slice::from_ref(b)));
        }
        ev.extend(d.flush());
        match &ev[0] {
            OutputEvent::TextDelta { text, .. } => assert_eq!(text, "안녕 😀 world"),
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn non_json_and_meta_lines_are_skipped_without_panic() {
        // 비-JSON(stderr 경고), 메타 라인(system/rate_limit_event), 빈 줄 → 이벤트 0개, panic 없음.
        let input = concat!(
            "Warning: no stdin data received in 3s, proceeding without it.\n",
            "{\"type\":\"system\",\"subtype\":\"init\"}\n",
            "{\"type\":\"rate_limit_event\",\"rate_limit_info\":{}}\n",
            "\n",
            "not json at all {{{\n",
            "{\"type\":\"unknown_future_line\"}\n",
        );
        let events = decode_all(input.as_bytes());
        assert!(
            events.is_empty(),
            "메타·비-JSON 라인은 모두 skip: {events:?}"
        );
    }

    #[test]
    fn empty_and_newline_only_chunks() {
        let mut d = ClaudeStreamDecoder::new();
        assert!(d.decode(b"").is_empty());
        assert!(d.decode(b"\n").is_empty());
        assert!(d.decode(b"\n\n\n").is_empty());
        assert!(d.flush().is_empty());
    }

    #[test]
    fn flush_processes_trailing_line_without_newline() {
        // EOF 시 개행 없이 끝난 마지막 라인도 flush 로 처리된다.
        let mut d = ClaudeStreamDecoder::new();
        let line = br#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"tail"}]}}"#;
        assert!(d.decode(line).is_empty(), "개행 전엔 아무것도 안 나온다");
        let ev = d.flush();
        assert_eq!(tags(&ev), vec!["text"]);
    }

    #[test]
    fn result_without_usage_emits_only_done() {
        // usage 없는 result 라인 → MessageDone 만(0 토큰 Usage 노이즈 없음).
        let ev = decode_all(b"{\"type\":\"result\",\"subtype\":\"success\"}\n");
        assert_eq!(tags(&ev), vec!["done"]);
    }

    #[test]
    fn buffer_overflow_resets_and_emits_error() {
        // FIX-A (1): 개행 없는 거대 입력(>4MB)이 오면 버퍼를 버리고 Error 이벤트 1개를 낸다(OOM 방지).
        let mut d = ClaudeStreamDecoder::new();
        let huge = vec![b'x'; MAX_BUFFER_BYTES + 1];
        let ev = d.decode(&huge);
        assert_eq!(tags(&ev), vec!["error"], "오버플로 → Error 1개 + 버퍼 리셋");
        assert!(d.buffer.is_empty(), "오버플로 후 버퍼 리셋");

        // 오버플로 라인은 아직 끝나지 않았다(개행 안 옴) → resync 상태에서 꼬리를 마저 버린다.
        // 오염 라인의 나머지 꼬리 + 그 라인을 끝내는 개행까지 통째 폐기하고, 개행 이후부터 복구.
        let tail_then_newline = b"garbage-tail-continues{\"type\":\"assistant\"}\n";
        let ev_tail = d.decode(tail_then_newline);
        assert!(
            ev_tail.is_empty(),
            "오염 라인 꼬리는 개행까지 통째 폐기 — 이벤트 0개: {ev_tail:?}"
        );

        // 복구: 오염 라인 종료(개행) 이후 정상 assistant 라인은 다시 이벤트를 낸다.
        let line = b"{\"type\":\"assistant\",\"message\":{\"id\":\"m1\",\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}}\n";
        let ev2 = d.decode(line);
        assert_eq!(tags(&ev2), vec!["text"], "오버플로 후 정상 라인 복구");
    }

    #[test]
    fn buffer_overflow_tail_with_valid_json_fragment_does_not_forge_events() {
        // FIX-A (2): 오버플로 라인의 꼬리에 우연히 valid JSON 조각(가짜 이벤트가 될 수 있는)이 섞여
        //   있어도, 그 꼬리는 오염 라인의 일부라 다음 개행까지 통째 폐기돼야 한다(가짜 이벤트 금지).
        //   그리고 그 개행 뒤의 **진짜** 정상 라인부터 복구된다.
        let mut d = ClaudeStreamDecoder::new();

        // 오버플로 유발: 개행 없는 4MB 초과 바이트(오염 라인 시작).
        let huge = vec![b'x'; MAX_BUFFER_BYTES + 1];
        let ev = d.decode(&huge);
        assert_eq!(tags(&ev), vec!["error"], "오버플로 → Error");

        // 꼬리에 valid JSON 라인 조각이 붙는다: `...xxx{"type":"result"}\n{정상 text}\n`.
        // 첫 번째 `{"type":"result"}` 는 오염 라인의 꼬리에 이어붙은 것 → resync 로 버려야 한다
        //   (버려지지 않으면 여기서 가짜 done 이벤트가 새어 나온다).
        let tail = concat!(
            "still-part-of-poisoned-line{\"type\":\"result\",\"subtype\":\"success\"}\n",
            "{\"type\":\"assistant\",\"message\":{\"id\":\"m2\",\"content\":[{\"type\":\"text\",\"text\":\"recovered\"}]}}\n",
        );
        let ev2 = d.decode(tail.as_bytes());
        // 첫 라인(오염 꼬리)의 result 조각은 안 나오고, 다음 정상 라인의 text 만 나와야 한다.
        assert_eq!(
            tags(&ev2),
            vec!["text"],
            "오염 꼬리의 valid JSON 조각은 가짜 이벤트로 새면 안 됨 — 다음 정상 라인만 복구"
        );
        match &ev2[0] {
            OutputEvent::TextDelta { text, .. } => assert_eq!(text, "recovered"),
            other => panic!("expected recovered TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn buffer_overflow_resync_spans_multiple_chunks_without_newline() {
        // FIX-A: 오염 라인 꼬리가 여러 청크에 걸쳐(개행 없이) 와도 resync 상태를 유지하며 전부 버린다.
        let mut d = ClaudeStreamDecoder::new();
        let huge = vec![b'x'; MAX_BUFFER_BYTES + 1];
        assert_eq!(tags(&d.decode(&huge)), vec!["error"]);

        // 개행 없는 꼬리 청크 여러 개 → 전부 폐기, 이벤트 0개.
        assert!(d.decode(b"tail-part-1").is_empty());
        assert!(d.decode(b"tail-part-2{\"type\":\"result\"}").is_empty());
        // 마침내 개행 도착 → 그 뒤부터 복구.
        let ev = d.decode(b"final-tail\n{\"type\":\"result\",\"subtype\":\"success\"}\n");
        assert_eq!(tags(&ev), vec!["done"], "resync 종료 후 정상 result 복구");
    }

    #[test]
    fn multiple_blocks_in_one_line_expand_in_order() {
        // 한 라인에 여러 블록 → 블록 순서대로 여러 이벤트(text, tool_use).
        let line = concat!(
            r#"{"type":"assistant","message":{"id":"m1","content":["#,
            r#"{"type":"text","text":"first"},"#,
            r#"{"type":"tool_use","id":"t1","name":"Bash","input":{"cmd":"ls"}}"#,
            "]}}\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(tags(&ev), vec!["text", "tool:Bash"]);
    }

    // ── FIX-B: malformed 블록 계약(가짜 정형 이벤트 금지) ──────────────────────────

    #[test]
    fn tool_use_without_name_preserved_as_structured_not_empty_toolcall() {
        // FIX-B: 문자열 name 이 없는 tool_use 는 빈 name ToolCall 을 만들지 않고 Structured 로 보존.
        let line = concat!(
            r#"{"type":"assistant","message":{"id":"m1","content":["#,
            r#"{"type":"tool_use","id":"t1","input":{"cmd":"ls"}}"#, // name 없음
            "]}}\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["structured:tool_use"],
            "name 없는 tool_use → 빈 ToolCall 금지, Structured 보존"
        );
        // 원본 블록이 통째 보존됐는지(input 등 정보 유실 없음).
        match &ev[0] {
            OutputEvent::Structured { kind, json } => {
                assert_eq!(kind, "tool_use");
                let v: serde_json::Value = serde_json::from_str(json).unwrap();
                assert_eq!(v["id"], "t1");
                assert_eq!(v["input"]["cmd"], "ls");
                assert!(v.get("name").is_none(), "원본에 name 없음 보존");
            }
            other => panic!("expected Structured, got {other:?}"),
        }
    }

    #[test]
    fn tool_use_with_non_string_name_preserved_as_structured() {
        // FIX-B: name 이 문자열이 아닌 경우(스키마 이탈)도 as_str() 실패 → Structured 보존.
        let line = concat!(
            r#"{"type":"assistant","message":{"id":"m1","content":["#,
            r#"{"type":"tool_use","id":"t1","name":123,"input":{}}"#, // name 이 숫자
            "]}}\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(tags(&ev), vec!["structured:tool_use"]);
    }

    #[test]
    fn text_block_without_text_field_is_skipped() {
        // FIX-B: 문자열 text 가 없는 text 블록은 빈 TextDelta 대신 skip(정보 유실 없음 → 조용히 버림).
        let line = concat!(
            r#"{"type":"assistant","message":{"id":"m1","content":["#,
            r#"{"type":"text"},"#,              // text 필드 없음 → skip
            r#"{"type":"text","text":"kept"}"#, // 정상 → 유지
            "]}}\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["text"],
            "text 없는 블록은 skip, 정상 text 블록만 유지"
        );
        match &ev[0] {
            OutputEvent::TextDelta { text, .. } => assert_eq!(text, "kept"),
            other => panic!("expected TextDelta 'kept', got {other:?}"),
        }
    }

    // ── FIX-C: result.is_error → Error + MessageDone(Error 먼저) ───────────────────

    #[test]
    fn result_is_error_emits_error_before_done() {
        // FIX-C: is_error:true 를 담은 result 라인 → Error 를 MessageDone 보다 먼저 emit.
        //   subtype·result 텍스트 등 가용 정보를 Error 메시지에 담는다.
        let line = concat!(
            r#"{"type":"result","subtype":"error_during_execution",""#,
            r#"is_error":true,"result":"API rate limit exceeded"}"#,
            "\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["error", "done"],
            "is_error → Error 먼저, MessageDone 나중"
        );
        match &ev[0] {
            OutputEvent::Error(msg) => {
                assert!(
                    msg.contains("error_during_execution"),
                    "subtype 담김: {msg}"
                );
                assert!(
                    msg.contains("API rate limit exceeded"),
                    "result 텍스트 담김: {msg}"
                );
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn result_error_subtype_without_is_error_flag_still_emits_error() {
        // FIX-C: is_error 플래그가 없어도 subtype 이 error 계열(starts_with "error")이면 Error 를 낸다.
        let line = r#"{"type":"result","subtype":"error_max_turns"}"#.to_string() + "\n";
        let ev = decode_all(line.as_bytes());
        assert_eq!(tags(&ev), vec!["error", "done"]);
    }

    #[test]
    fn result_interrupted_subtype_emits_only_done_no_error() {
        // FIX-E 회귀: 유저 Esc 정상 중단 턴(subtype:"interrupted", is_error 없음)은 error allowlist
        //   (starts_with "error")에 안 걸린다 → Error 없이 MessageDone 만. interrupt 는 1급 정상 경로라
        //   실패 턴으로 오분류하면 안 된다. (과거 denylist `!= "success"` 는 이걸 오류로 잡았다.)
        let line = r#"{"type":"result","subtype":"interrupted"}"#.to_string() + "\n";
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["done"],
            "interrupted 는 오류 아님 → Error 없이 done 만"
        );
    }

    #[test]
    fn result_interrupted_subtype_with_is_error_false_emits_only_done() {
        // FIX-E 회귀: is_error:false 가 명시된 interrupted 도 Error 없이 done 만.
        let line =
            r#"{"type":"result","subtype":"interrupted","is_error":false}"#.to_string() + "\n";
        let ev = decode_all(line.as_bytes());
        assert_eq!(tags(&ev), vec!["done"]);
    }

    #[test]
    fn result_success_subtype_emits_only_done_no_error() {
        // FIX-C 회귀: 정상 result(subtype=success, is_error 없음)는 Error 를 내지 않는다.
        let line = r#"{"type":"result","subtype":"success"}"#.to_string() + "\n";
        let ev = decode_all(line.as_bytes());
        assert_eq!(tags(&ev), vec!["done"], "정상 result 는 Error 없이 done 만");
    }

    #[test]
    fn result_error_with_usage_orders_usage_error_done() {
        // FIX-C + Usage 순서: usage 가 있고 is_error 면 Usage → Error → MessageDone 순.
        //   (Usage 는 종료 전 토큰 집계, Error 는 종료 전 실패 통지, 둘 다 MessageDone 앞.)
        let line = concat!(
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"#,
            r#""usage":{"input_tokens":10,"output_tokens":2}}"#,
            "\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(tags(&ev), vec!["usage", "error", "done"]);
    }
}
