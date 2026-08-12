//! ClaudeBackend — claude CLI 전용 CommandSpec 산출.
//!
//! ★claude 지식 격리(ADR-0004)★: claude 플래그·env 규약 · stream-json 스키마 · `.jsonl` transcript
//! 파일 배치 지식은 **이 파일 안에만** 둔다. generic 층(manager·backend dispatch)·transport·core 는
//! 추상 descriptor 와 바이트만 나른다.
//!
//! tauri import 0.

use std::path::PathBuf;

use uuid::Uuid;

use crate::agent::backend::{console_command, AgentBackend, TurnClassifier};
use crate::agent::profile::{AgentCommand, ClaudeOutputFormat, SpawnMode};
use crate::agent::turn::TurnSignal;
use crate::agent::types::{
    BackendCaps, CommandSpec, ControlEndpoint, ModelCaps, OutputEvent, SessionCaps, ToolGrant,
    CLI_EXE_ENV, CLI_EXE_NAME, MAIL_MARKER_ENV, MAIL_MARKER_OFF, MAIL_MARKER_ON,
};

const CLAUDE_PROGRAM: &str = "claude";

pub struct ClaudeBackend;

impl AgentBackend for ClaudeBackend {
    fn needs_session(&self) -> bool {
        true
    }

    fn supports_control_channel(&self) -> bool {
        true
    }

    fn accepts_mcp_config(&self) -> bool {
        // claude 는 mcp-config 를 `--mcp-config` 로 부착한다 → MCP-capable.
        //
        // ★이 값이 가르는 것 — 배선이 아니라 교육과 강제다(ADR-0133)★:
        //   - **배선은 전원에게 간다.** `engram` 실행파일·크레덴셜·PATH 는 MCP 가능 스폰에도 깔린다 —
        //     제어 동사가 전원 개방이고(ADR-0132 결정 5) 실행파일이 하나뿐이라 계열 단위로 갈라 깔 수 없다.
        //   - **교육만 갈린다.** 프라이밍 변형(MCP-only ↔ CLI-only)과 우편 표식(`MAIL_MARKER_ENV`)이
        //     이 값으로 갈려, MCP 가능 스폰의 `engram help` 에는 우편 계열이 나오지 않는다.
        //   - **강제는 데몬 거절 하나뿐이다.** MCP 가능 스폰의 자격증명으로 온 우편 요청은 데몬이 거절한다.
        // ★표식 필터를 강제로 세지 말 것★: 표식은 에이전트 자신의 env 라 떼면 목록에 우편이 보인다 —
        //   그때도 막는 것은 데몬 거절뿐이므로, 거절을 지우고 표식만 남기면 우편이 열린다.
        // ★못 쓰는 채널을 가르치면 ADR-0099 가 실측한 발신 freeze 가 재현된다★ — 프라이밍 변형과 우편
        //   가부는 반드시 같은 값에서 갈려야 한다.
        // ★이 플래그가 구현하는 살아 있는 불변식은 여전히 채널 단일화다(ADR-0128 결정 1)★: 우편 채널은
        //   백엔드 capability 로만 갈리고 런타임 스위칭·폴백이 없다. ADR-0133 이 바꾼 것은 그 단일화를
        //   **어떻게 강제하느냐**뿐이다.
        // ADR-0099
        // ADR-0126
        // ADR-0128
        // ADR-0133
        true
    }

    fn build_spec(
        &self,
        command: &AgentCommand,
        mode: SpawnMode,
        session_id: Option<Uuid>,
        cwd: PathBuf,
        mut env: Vec<(String, String)>,
        control: Option<ControlEndpoint>,
    ) -> CommandSpec {
        match command {
            AgentCommand::Claude {
                extra_args,
                output_format,
            } => {
                let mut args = Vec::with_capacity(8 + extra_args.len());
                // ★AUTO 권한 모드 — `--permission-mode bypassPermissions`(사용자 결정 2026-07-22)★:
                //   스폰되는 claude 는 헤드리스 워커라 승인자가 없다 — 기본 거부 + per-tool grant 배선은
                //   승인 프롬프트를 띄울 사람이 없어 성립하지 않는다(CLI 실측 0/38 전량 permission-block).
                //   **임시 체제**이며 정식 대체는 전 LLM 공용 제약 레이어다(그때 이 base 플래그를 걷는다).
                //   ★맨 앞 배치★: `--permission-mode`+`bypassPermissions` 는 NON-variadic 2요소라 맨 앞에
                //   둬도 뒤 그룹의 흡수 규칙(아래 extra_args 주석)에 걸리지 않는다. control endpoint 유무·
                //   spawn 모드와 무관하게 **무조건** 주입.
                // 사용자 결정 2026-07-22
                // ADR-0097
                args.push("--permission-mode".to_string());
                args.push("bypassPermissions".to_string());
                match output_format {
                    // ── 터미널(PTY 대화형) — 바이트/인자 동결(회귀 금지) ──
                    ClaudeOutputFormat::Terminal => {
                        if let Some(sid) = session_id {
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
                        // ADR-0044 후속 완료 / ADR-0008 재사용: json(stream-json) resume 활성화.
                        if let Some(sid) = session_id {
                            // ★실측(2026-07-13, claude 2.1.170)★: stream-json 헤드리스도 `--resume <sid>`
                            //   를 지원한다 — `-p`/`--input-format stream-json`/`--output-format stream-json`
                            //   과 공존하고 "session already in use" 없이 과거 대화를 무손실 재개한다. 그래서
                            //   터미널 분기와 같은 mode 갈림을 쓴다(통제-sid 인프라 ADR-0008 재사용).
                            let flag = match mode {
                                SpawnMode::Fresh => "--session-id",
                                SpawnMode::Resume => "--resume",
                            };
                            args.push(flag.to_string());
                            args.push(sid.to_string());
                        }
                        // ADR-0049: 실측(2026-07-06, claude 2.1.170) — 헤드리스 stream-json 은 env
                        //   MAX_THINKING_TOKENS 가 있어야 `"type":"thinking"` 블록을 낸다(CLI 기본은 꺼짐).
                        //   그래서 json 모드에 한해 8000 을 주입한다. 터미널 경로는 CLI parity 유지 — 미주입.
                        // ★프로필 우선(explicit-skip)★: 프로필이 같은 키를 이미 넣었으면 건너뛴다. env 는
                        //   (k,v) Vec 이고 transport 가 순서대로 cmd.env 해서 뒤가 이기지만, 병합 순서에
                        //   기대지 않고 결정적으로 프로필이 이기게 한다.
                        //   ★대소문자 무시★: Windows 환경변수는 대소문자를 구분하지 않으므로 프로필의
                        //   `max_thinking_tokens` 도 같은 키로 인식해 중복 주입을 막는다.
                        //   ★프로필 내 중복 키 정규화는 범위 밖★: 표준 last-wins env 의미론을 그대로 따른다
                        //   (한 키만 특별 처리하면 일관성이 깨진다).
                        const MAX_THINKING_TOKENS_KEY: &str = "MAX_THINKING_TOKENS";
                        if !env
                            .iter()
                            .any(|(k, _)| k.eq_ignore_ascii_case(MAX_THINKING_TOKENS_KEY))
                        {
                            env.push((MAX_THINKING_TOKENS_KEY.to_string(), "8000".to_string()));
                        }
                    }
                }
                // ADR-0086: 제어 채널 주입 — endpoint 를 claude 방식으로 번역한다. CLI 입구(크레덴셜 env·
                //   PATH·우편 표식)는 **control endpoint 가 있는 모든 스폰**에 깔고, `--mcp-config` 는
                //   config_path 가 있을 때 **더한다**(둘은 배타가 아니다 — ADR-0133).
                //   ★mode 무관 동일 주입★: 터미널·json 둘 다 연결 대상이다(claude 2.1.170 실측 — headers
                //   Authorization 을 initialize/tools/list/tools/call 전 요청에 실전송).
                //   ★두 주입을 배타로 되돌리면 제어 평면이 반쪽이 된다★: MCP 가능 스폰이 `engram` 을 못 부르면
                //   제어 동사(ADR-0132 결정 5 = 전원 개방)에 닿을 수 없다. 우편 격리는 이 갈림이 아니라
                //   데몬의 자격증명 거절이 한다.
                // ADR-0086 / ADR-0099 / ADR-0133
                if let Some(endpoint) = &control {
                    inject_cli_entrance(&mut env, endpoint);
                    if let Some(config_path) = &endpoint.config_path {
                        args.push("--mcp-config".to_string());
                        args.push(config_path.to_string_lossy().into_owned());
                    }
                }
                // ADR-0092: 프라이밍(수신 계약) 주입 — `--append-system-prompt-file <abs-path>`.
                //   claude CLI(2.1.170)가 그 파일을 **직접 읽어** 기본 시스템 프롬프트 **뒤에** 덧붙인다.
                //   터미널·json 둘 다 시스템 프롬프트를 받으므로 mode 무관 동일 주입.
                // ADR-0092
                if let Some(endpoint) = &control {
                    if let Some(priming) = &endpoint.priming_file {
                        args.push("--append-system-prompt-file".to_string());
                        args.push(priming.to_string_lossy().into_owned());
                    }
                }
                // S18 D(spec §6): 세션 한정 설정 조각 주입 — `--settings <abs-path>`. (ADR-0109)
                //
                // ★인라인 JSON 이 아니라 **파일 경로**를 쓰는 이유(load-bearing — Windows 인용 지옥)★:
                //   claude 의 `--settings` 는 JSON 문자열도 받지만, Windows 스폰은 `console_command` 가
                //   cmd.exe 를 한 겹 더 끼운다. 그 argv 에 `{"allowedMcpServers":[…]}` 같은 따옴표·중괄호
                //   덩어리를 실으면 cmd 의 인용/이스케이프 규칙(따옴표 소실·`&`/`^` 특수문자)에 조용히
                //   깨지기 쉽고, 깨진 결과는 "설정이 무시된 채 정상 기동"(= MCP 툴 부재 재발)이라 발현이
                //   늦다. 파일 경로는 공백만 다루면 되고, 같은 경로 전달 방식(`--mcp-config`/
                //   `--append-system-prompt-file`)이 이미 실측으로 검증돼 있다(M3 심의 기록) — 그래서
                //   경로를 택한다. 대가는 파일 하나의 수명 관리인데, 그건 mcp-config 파일과 **같은 폴더·
                //   같은 생성/삭제/부팅스윕 경로**를 그대로 재사용해 흡수한다(mcp_config.rs).
                if let Some(endpoint) = &control {
                    if let Some(settings) = &endpoint.settings_file {
                        args.push("--settings".to_string());
                        args.push(settings.to_string_lossy().into_owned());
                    }
                }
                // ★순서 불변(load-bearing, ADR-0094 최소권한)★: `extra_args`(호출자 패스스루)를
                //   **`--allowedTools` 주입보다 먼저** 잇는다. 이유는 `--allowedTools <tools...>` 가
                //   claude(2.1.170 실측)에서 **variadic** 이라서다 — 이 플래그 뒤에 오는 positional
                //   argv 값들을 다음 `--flag` 가 나올 때까지 전부 "허용 툴"로 흡수한다. 만약 grant 그룹
                //   뒤에 extra_args 가 오고 그 첫 요소가 bare positional(예: `Bash`)이면, 그 값이
                //   **추가 허용 툴**로 빨려 들어가 blanket Bash 권한을 부여한다(ADR-0094 최소권한 위반).
                //   그래서 extra_args 를 먼저 소진하고, `--allowedTools` 그룹을 args 벡터의 **맨 끝**에
                //   둔다. 두 spawn 모드 모두 프롬프트를 stdin(write_input)으로 먹여 argv 에 **trailing
                //   positional 이 없으므로**(AgentCommand::Claude 에 프롬프트 필드 없음) 그룹 뒤 흡수가
                //   구조적으로 불가능하다.
                args.extend(extra_args.iter().cloned());
                // ADR-0094 / ADR-0106: 내장 SendMessage 차단 — `--disallowedTools SendMessage`,
                //   **control endpoint 있을 때만** 주입.
                //   ★왜★: harness 내장 툴 `SendMessage`(PascalCase)와 우리 MCP 툴 `send_message`
                //   (server: engram, snake_case)가 이름이 충돌한다 — 스폰된 claude 가 프라이밍을 오독해
                //   내장 SendMessage 를 호출하면 engram 에이전트 이름을 몰라 "No agent named 'X' is
                //   reachable" 로 실패한다(실측 2026-07-26 roundtrip 진단 — 스폰마다 재현). 프라이밍 문구
                //   교정만으론 재발하므로 툴 자체를 막아 결정적으로 끊는다.
                //   ★스코프 = control 있을 때만(ADR-0106, 리뷰 지적 2026-07-26)★: 충돌이 문제 되는 건
                //   메시징 프라이밍을 받은 에이전트뿐이고 그건 control endpoint 가 있는 스폰뿐이다. 비메시징
                //   스폰까지 막으면 (a) 이유 없이 내장 기능을 잃고 (b) claude 가 미등록 툴을 deny 목록에서
                //   만나 매 호출마다 경고 비용을 문다.
                //   ★순서★: `--disallowedTools` 도 variadic 이라 extra_args **뒤**에 둔다. 뒤이은
                //   `--allowedTools` 가 새 `--flag` 로서 이 variadic 을 종료시키므로 allowedTools 그룹의
                //   맨-끝 자리는 유지된다.
                // ADR-0094 / ADR-0106
                if control.is_some() {
                    args.push("--disallowedTools".to_string());
                    args.push("SendMessage".to_string());
                }
                // ADR-0094: 발신 입구 pre-authorization — `--allowedTools <pattern>...`.
                //   ★grant 유지 이유(bypass 가 스폰 기본값이 된 뒤에도)★: 실질 인가는 위 base 플래그의
                //   bypass 가 판다. 그럼에도 이 최소권한 grant 주입은 **유지**한다: (a) bypass 아래선
                //   무해한 중복 인가고, (b) 미래 공용 제약 레이어가 bypass 를 걷을 때 재사용할 결정적
                //   정책 표면이며, (c) "이 에이전트가 어떤 발신 입구를 갖는가"를 드러내는 표면이다.
                //   지금은 게이트가 아니라 정책 선언이다 — 지우지 말 것.
                //   ★arg shape★: `--allowedTools` 뒤에 각 패턴을 **개별 args 요소**로 잇는다 — 공백 포함
                //   패턴(`Bash(C:\Program Files\…:*)`)이 한 argv 요소로 유지돼야 claude 가 공백을 값
                //   구분자로 쪼개지 않는다(comma-join 단일 값으로 합치지 않는 이유). 권한 게이트는 mode 무관.
                //   ★맨 끝 배치★ 회귀 가드 = 테스트 claude_allowed_tools_group_is_last_and_*.
                // ADR-0094
                if let Some(endpoint) = &control {
                    let patterns = grants_to_allowed_tools(&endpoint.grants);
                    if !patterns.is_empty() {
                        args.push("--allowedTools".to_string());
                        args.extend(patterns);
                    }
                }
                let (program, args) = console_command(CLAUDE_PROGRAM, args);
                CommandSpec {
                    program,
                    args,
                    env,
                    cwd,
                }
            }
            // dispatch 가 ClaudeBackend 에는 Claude variant 만 보내지만, 방어적으로 Shell 도 처리한다.
            AgentCommand::Shell { program, args } => CommandSpec {
                program: program.clone(),
                args: args.clone(),
                env,
                cwd,
            },
        }
    }

    /// 터미널·json(stream-json) **둘 다** `--resume <sid>` 로 무손실 재개하므로 resume=true — 그래서
    /// `command`(모드)를 보지 않는다. snapshot·model 옵션은 콘솔 CLI 라 미지원.
    /// 시그니처는 backend 가 session caps 의 출처라는 계약(ADR-0030 type split)을 유지한다.
    fn capabilities(&self, _command: &AgentCommand) -> BackendCaps {
        let resume = true;
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

    fn turn_classifier(&self) -> TurnClassifier {
        classify_turn
    }
}

/// ADR-0086 스텝 2(CLI 입구): 스폰 env 에 CLI 크레덴셜 + 제어 평면 CLI(`CLI_EXE_NAME`) 형제 디렉토리
/// PATH 프리펜드 + 우편 가부 표식.
///
/// ★호출 조건 = control endpoint 가 있는 스폰 전부★: 제어 동사는 전원에게 열려 있고(ADR-0132 결정 5)
///   실행파일이 하나뿐이라 계열 단위로 갈라 깔 수 없다. 우편은 여기서 가리지 않는다 — 표식이 사용법을
///   가리고(교육), 데몬이 자격증명으로 거절한다(강제).
/// ★표식은 강제가 아니다★: 에이전트가 자기 env 를 지울 수 있으므로 표식을 뗀 프로세스는 우편 사용법을
///   **본다**. 그때 막는 것은 데몬 거절뿐이라, 거절 없이 이 표식만으로 통제하려 들면 우편이 열린다.
/// ★왜 env 인가★: 에이전트가 shell 로 그 명령을 부를 때 이 값을 읽어 데몬 제어 라우트에 Bearer
///   토큰으로 POST 한다. portable-pty CommandBuilder 가 부모 env 를 시드하므로 **모든 자식 프로세스
///   (Bash·그 손자)까지 상속**된다.
/// ★보안★: 토큰이 env 로 노출된다 — 같은 OS 유저의 자식에만 상속되고 로그엔 안 찍지만 하드 격리는
///   원래 불가다(ADR-0086 §불변식).
// ADR-0086 / ADR-0133
fn inject_cli_entrance(env: &mut Vec<(String, String)>, endpoint: &ControlEndpoint) {
    // ★ENGRAM_CONTROL_URL = base(스킴+호스트+포트)★: endpoint.url 은 MCP 라우트
    //   (`http://127.0.0.1:<port>/mcp`)라 CLI 가 붙을 base 로 쓰려면 라우트 suffix(`/mcp`)를 벗겨 base 만
    //   남긴다 — CLI 가 `<base>/control/send` 를 조립한다(라우트 경로 지식은 CLI 소유). suffix 가 없으면
    //   (형태 변주) url 을 그대로 base 로 쓴다(방어적).
    //   ★keep-in-sync(M5)★: 아래 strip_suffix 의 리터럴 "/mcp" 는 데몬측 MCP_PATH 상수와 **손으로 맞춰진**
    //   값이다 — 정본 = `crates/engram-dashboard-daemon/src/control/mcp_server.rs`(const MCP_PATH). 그쪽
    //   경로를 바꾸면 여기 리터럴도 함께 고쳐야 한다(빌드가 강제 못 함 → 어긋나면 base 파생이 틀어져 CLI 가
    //   조용히 404). 두 곳 상호 앵커.
    let base = endpoint
        .url
        .strip_suffix("/mcp")
        .unwrap_or(&endpoint.url)
        .to_string();
    env.push(("ENGRAM_TOKEN".to_string(), endpoint.token.clone()));
    env.push(("ENGRAM_CONTROL_URL".to_string(), base));
    // ★두 값 다 명시로 싣는다(부재를 off 로 쓰지 않는다)★: 부재는 "스폰 밖" 을 뜻해 CLI 가 전부 보여
    //   준다(`MAIL_MARKER_ENV`). 켜짐을 생략하면 두 뜻이 겹쳐, 표식을 못 실은 배선 사고가 정상 스폰과
    //   구별되지 않는다.
    // ADR-0133
    env.push((
        MAIL_MARKER_ENV.to_string(),
        if endpoint.mail_allowed {
            MAIL_MARKER_ON
        } else {
            MAIL_MARKER_OFF
        }
        .to_string(),
    ));
    // ★ENGRAM_CLI_EXE = CLI 바이너리 절대경로(F1)★: 프라이밍과 grant 는 bare 실행파일 이름
    //   (`CLI_EXE_NAME` — 아래 PATH 주입으로 해석)을 가르치지만, 이 절대경로 env 도 함께 싣는다 —
    //   진단·수동 조작용이다(ADR-0094 의 이름 정렬 자체는 PATH 로 이룬다).
    //   ★이 값을 가르치는 프라이밍은 없다 — 그러니 아래 loud skip 갈래(PATH 조합 실패·비-UTF8)의 복구
    //     수단으로 세지 말 것★: 그 갈래에서 에이전트는 bare 이름만 배운 채 PATH 로 해석하지 못하므로
    //     실질적으로 발신 불가이고, 신호는 그 warn 로그 하나뿐이다.
    //   None 갈래는 발신 입구가 하나도 안 남는 조합이라 데몬이 provision 에서 이미 fail-closed 로 끊는다
    //   — 여기 도달하지 않는 방어 경로다(도달해도 크레덴셜만 있고 부를 CLI 가 없는 무해한 상태).
    if let Some(send_exe) = &endpoint.send_exe {
        env.push((
            CLI_EXE_ENV.to_string(),
            send_exe.to_string_lossy().into_owned(),
        ));
        // ★PATH 주입(ADR-0094 bare 이름 해석)★: grant(`Bash(<CLI_EXE_NAME>:*)`)와 프라이밍이 모두 bare
        //   실행파일 이름을 가르치므로 스폰된 에이전트의 shell(및 그 자식 Bash 도구)이 그 이름을 실제로
        //   **찾을** 수 있어야 한다. send_exe 의 **부모 디렉토리**를 PATH **맨 앞**에 붙인다.
        //
        // ★base = env 벡터에 이미 있는 PATH(프로필 우선, FIX-1)★: 프로필 env 는 이 지점보다 **먼저**
        //   벡터에 들어와 있다. 데몬 프로세스 PATH(std::env::var_os) 로 리빌드하면 프로필이 실은 커스텀
        //   PATH 가 통째로 증발하므로, 벡터에 PATH 가 없을 때만 데몬 PATH 로 폴백한다.
        //   ★키 대소문자(Windows)★: 프로필이 "Path"·"PATH" 어느 표기로 넣어도 같은 변수다.
        //   ★last-match-wins + dedupe(load-bearing)★: transport(portable-pty)는 env 를 **순서대로**
        //   cmd.env(k,v) 하므로 같은 변수의 중복 항목이 있으면 자식엔 **마지막** 값이 산다(예: Windows
        //   에서 `[("PATH", 데몬), ("Path", 프로필)]`). 그래서 마지막 case-equivalent PATH 를 base 이자
        //   승리 항목으로 삼아 그 키 표기 그대로 제자리 교체하고, **나머지 PATH 항목은 전부 제거**한다 —
        //   중복을 남기면 앞쪽만 고친 뒤 뒤쪽 미수정 항목이 last-wins 로 이겨 주입이 **조용히 무력화**된다
        //   (adversarial 리뷰 must-fix). 구성: `send_exe_parent + separator + base` — 형제 디렉토리가
        //   **맨 앞**(shadowing 방어), 프로필/데몬 PATH 는 **tail 로 생존**.
        if let Some(parent) = send_exe.parent() {
            let is_path_key = |k: &str| {
                if cfg!(windows) {
                    k.eq_ignore_ascii_case("PATH")
                } else {
                    k == "PATH"
                }
            };
            let winner_idx = env.iter().rposition(|(k, _)| is_path_key(k));
            let base_os = winner_idx
                .map(|i| std::ffi::OsString::from(env[i].1.clone()))
                .or_else(|| std::env::var_os("PATH"));
            let mut dirs = vec![parent.to_path_buf()];
            if let Some(base) = &base_os {
                dirs.extend(std::env::split_paths(base));
            }
            match std::env::join_paths(dirs)
                .ok()
                .and_then(|j| j.into_string().ok())
            {
                Some(joined) => match winner_idx {
                    Some(i) => {
                        env[i].1 = joined;
                        let mut seen = 0usize;
                        env.retain(|(k, _)| {
                            if is_path_key(k) {
                                let keep = seen == i;
                                seen += 1;
                                keep
                            } else {
                                seen += 1;
                                true
                            }
                        });
                    }
                    None => env.push(("PATH".to_string(), joined)),
                },
                // ★loud skip(FIX-2/3)★: join 실패·비-UTF8 이면 주입을 **통째 건너뛴다** — lossy 변환한
                //   PATH 를 절대 push 하지 않는다(비-Unicode PATH 항목을 조용히 손상시키면 skip 보다
                //   나쁘다). skip 시 env 벡터는 **원래 그대로** 둬서 상속 PATH 가 안전 폴백이 된다.
                None => {
                    tracing::warn!(
                        "CLI PATH 주입 건너뜀(PATH 조합 실패 또는 비-UTF8) — grant/프라이밍은 bare `{}` 를 약속하나 이 설치에선 자식이 이름을 해석하지 못할 수 있음; 상속 PATH 유지",
                        CLI_EXE_NAME
                    );
                }
            }
        }
    }
}

/// claude stream-json 이벤트 → 턴 신호(ADR-0113).
///
/// ★`Structured` 를 진행으로 세는 게 load-bearing — 그리고 그 판정은 **claude 한정**이다★: claude 는
///   **입력 시점 유저 에코**를 이 variant 로 낸다(`user_text_echo_json` · decoder 의 user 라인). 그래서
///   대시보드 사용자가 터미널에 직접 입력해 시작한 턴도, 우편 주입이 시작한 턴도 이 갈래로 잡힌다 —
///   빼면 그 두 경로의 턴 시작이 통째로 관측 밖으로 나간다.
/// ★`kind` 를 보지 않는 이유(현 범위의 정직한 표기)★: claude decoder 가 내는 `Structured` 는 전부 턴
///   안에서 발생하는 라인이라 지금은 kind 구분이 불필요하다. claude 가 턴 밖 구조화 라인을 내기
///   시작하면 여기서 kind 를 걸러야 한다.
/// ★`Usage`/`Error` 가 종료가 아닌 이유★: `Usage` 는 턴 중간에도 오고, `Error` 는 스트림 내부 오류지
///   턴 경계가 아니다(실패 턴도 `MessageDone` 으로 닫힌다 — decoder FIX-C). `TerminalBytes` 는 턴 경계
///   정보가 없는 콘솔 바이트다.
/// ★상관 키가 없다(알려진 범위)★: claude 의 `MessageDone` 은 `turn_id`/`message_id` 가 모두 None 이라
///   "어느 턴의 종료인가" 를 맞출 키가 없다. 그래서 턴 카운팅·펜싱을 하지 않고 **마지막 관측이
///   결정한다**. 중첩 Task 서브에이전트의 종료 라인이 부모 턴 종료로 새는지는 미검증이고, 새면 증상은
///   "부모가 아직 턴 중인데 idle 로 오판 → 조기 주입"(유실 없이 타이밍만 어긋남)이다.
/// ★터미널 모드와 공유해도 되는 이유(모드별 분기 불필요)★: 터미널 모드는 decoder 가 없어
///   `TerminalBytes` 만 흐르므로 이 매핑을 그대로 써도 신호가 하나도 나오지 않는다.
// ADR-0113
// ADR-0004
pub(crate) fn classify_turn(event: &OutputEvent) -> Option<TurnSignal> {
    match event {
        OutputEvent::TextDelta { .. }
        | OutputEvent::ToolCall { .. }
        | OutputEvent::Structured { .. } => Some(TurnSignal::Progress),
        OutputEvent::MessageDone { .. } => Some(TurnSignal::Ended),
        OutputEvent::Usage { .. } | OutputEvent::Error(_) | OutputEvent::TerminalBytes(_) => None,
    }
}

/// ADR-0094: 추상 `ToolGrant` 목록 → claude `--allowedTools` 패턴 문자열 목록.
///
/// ★MCP 패턴★: `mcp__<server>__<tool>` 은 claude MCP 툴 네이밍 규약이다.
/// ★CLI 패턴 문법★: `Bash(<X>:*)` 의 **colon-star** 는 Claude Code 권한 시스템의 문서화된 **prefix
///   와일드카드** 문법이다 — 그 문자열로 **시작하는** 명령만 허용한다(전체 Bash 아님). exe 는 bare 명령
///   이름이고, 프라이밍이 가르치는 명령·이 grant 패턴·실제 invocation 이 모두 그 bare 이름으로 정렬된다.
///   ★이 prefix 가 지금 덮는 범위(사용자 수용, 좁히지 않기로 결정)★: 실행파일 이름이 `engram` 이라
///     `Bash(engram:*)` 는 그 이름으로 **시작하는** 형제들 — 릴리즈 폴더에 동거하는 `engram-dashboard`·
///     `engram-dashboard-daemon`(ADR-0100 co-location) — 도 함께 덮는다. 그 형제들은 아래 PATH 프리펜드
///     때문에 bare 이름으로도 닿는다. **매처가 토큰 경계를 보는지는 미검증**이라("prefix" 가 문자열 접두인지
///     인자 경계까지 보는지) 좁히려면 그 동작부터 실측해야 한다.
///   ★그래도 지금 위험이 아닌 이유 — 그리고 언제 다시 볼지★: 스폰은 `--permission-mode bypassPermissions`
///     아래 무조건 돌아서(ADR-0097) grant 는 런타임 게이트가 **아니라 정책 표면**이다(= 지금 이 목록의
///     넓이는 아무 것도 열지 않는다). 그 auto 모드는 임시 체제이고 후계가 지명돼 있으므로(`ToolGrant`
///     주석의 "전 LLM 공용 제약 레이어"), **그 플래그가 걷히는 순간 이 prefix 범위를 먼저 재검토해야
///     한다** — 그때는 넓이가 곧 권한이 된다.
///   ★그 "무조건" 에 붙은 미확인 하나(ADR-0132 §영향)★: 프로필 `extra_args` 는 우리 플래그 **뒤에** 붙는다.
///     사용자가 거기 `--permission-mode` 를 또 넣었을 때 **뒤 값이 이기는지는 미검증**이다 — 이긴다면 그
///     스폰만 auto 가 아니게 되고, 위 "지금은 inert" 전제가 그 스폰에서만 깨져 넓은 prefix 가 **실제 권한**이
///     된다. 즉 이 절의 수용 판단은 그 미확인에 조건부다.
///   ★Windows PowerShell 도구 커버(FIX-4)★: Windows 의 Claude Code 는 **PowerShell 도구**를 별도로
///     노출하고 에이전트가 거기서 명령을 실행하기도 한다(실측: 에이전트가 "PowerShell 로 보내겠다").
///     그래서 `Cli{exe}` 하나가 두 패턴을 낸다 — 같은 발신 입구의 두 shell 모양일 뿐 새 명령은 없다.
///     PowerShell 도구가 없는 환경에선 무해한 no-op 다.
///   ★옛 `Bash(<abs> *)`(space-star + 절대경로) 폐기★: 라이브 측정에서 0/38 로 전부 permission-blocked
///     됐다(미매칭 문법 + 배포 비친화적 절대 좌표). colon-star + bare 이름 + 주입 PATH 로 교체했다.
/// ★패턴 순서(Cli)★: Bash 먼저, PowerShell 다음 — build_spec 이 이 순서로 args 에 잇는다(테스트 앵커).
// ADR-0098
pub(crate) fn grants_to_allowed_tools(grants: &[ToolGrant]) -> Vec<String> {
    let mut out = Vec::with_capacity(grants.len());
    for g in grants {
        match g {
            ToolGrant::Mcp { server, tool } => out.push(format!("mcp__{server}__{tool}")),
            ToolGrant::Cli { exe } => {
                out.push(format!("Bash({exe}:*)"));
                out.push(format!("PowerShell({exe}:*)"));
            }
        }
    }
    out
}

/// claude stream-json stdin 의 유저 턴 1줄(라인 종단 `\n`)을 만든다(ADR-0044 §4).
///
/// ★1 호출 = 완결된 유저 턴 1개(FIX 6a)★: `text` 를 유저 턴 1줄로 통째 감싼다 — 부분/한 글자
///   텍스트를 넘기면 그 조각이 그대로 한 턴이 돼 대화가 깨진다. 호출자가 **완성된 메시지 전체**를
///   넘길 책임이다.
/// ★uuid dedup 계약(공식 VS Code 확장 방식)★: 우리가 심은 `uuid` 를 top-level 에 실으면 claude 가
///   `--replay-user-messages` 로 되울린 user 라인이 **이 uuid 를 그대로 보존**하고 `"isReplay":true` 를
///   단다(실측 확정, 2026-07-06). 그래서 여기 uuid 는 같은 write_input 이 합성 에코
///   (user_text_echo_json)에 쓴 값과 **반드시 같아야** 한다 — 호출자가 한 번 생성해 양쪽에 넘긴다.
/// ★정확한 escape★: 따옴표·개행·유니코드는 serde_json 이 처리한다 — 문자열 포맷팅으로 손조립 금지
///   (`"` 미escape 시 stdin JSON 파서가 깨진다).
/// claude 는 라인 단위로 stdin 을 파싱하므로 반드시 `\n` 으로 종단한다.
///
/// ★키 순서★: `serde_json::json!`(Value=BTreeMap)는 키를 알파벳순으로 재배열한다. claude 는 임의
///   순서를 받지만, 스키마를 사양(`{"type":"user","message":{"role":"user","content":[…]},"uuid":…}`)
///   그대로 드러내려고 **typed struct**로 직렬화한다 — serde 는 struct 필드를 선언 순서대로 쓴다.
pub(crate) fn wrap_user_turn(text: &str, uuid: Uuid) -> Vec<u8> {
    #[derive(serde::Serialize)]
    struct UserTurn<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        message: UserMessage<'a>,
        uuid: String,
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
        uuid: uuid.to_string(),
    };
    // to_string 은 이 형태에선 실패하지 않는다 — 방어적으로 unwrap_or_default.
    let mut line = serde_json::to_string(&turn).unwrap_or_default();
    line.push('\n');
    line.into_bytes()
}

/// 입력-시점 유저 에코의 `Structured{kind:"user"}` json 페이로드를 만든다(ADR-0044/0045).
///
/// ★왜 입력 시점에 만드나★: json(stream-json) 모드는 PTY 처럼 입력이 즉시 로컬 에코되지 않는다 —
///   claude 가 `--replay-user-messages` 로 되울릴 때까지(왕복 지연) 화면에 안 뜬다. 그래서 write_input
///   성공 직후 세션 층이 **합성 유저 이벤트**를 emit 해 터미널의 즉시 에코를 흉내낸다.
///
/// ★uuid dedup shape 계약(load-bearing)★: 이 json 은 decoder 가 replay 된 유저 text 블록에 대해 만드는
///   것과 **동일한 shape**(`{"type":"text","text":<raw>,"uuid":"X"}`)여야 한다 — 프론트 accumulator 가
///   user item 을 `uuid` 로 dedup 하므로, shape 나 uuid 가 어긋나면 합성 에코와 replay 에코가 두 개로
///   남는다.
pub(crate) fn user_text_echo_json(text: &str, uuid: Uuid) -> String {
    #[derive(serde::Serialize)]
    struct TextBlock<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        text: &'a str,
        uuid: String,
    }
    let block = TextBlock {
        kind: "text",
        text,
        uuid: uuid.to_string(),
    };
    // to_string 은 이 형태에선 실패하지 않는다 — 방어적으로 unwrap_or_default.
    serde_json::to_string(&block).unwrap_or_default()
}

// ── S15 B2: claude stream-json(NDJSON) → OutputEvent decoder (ADR-0044/0045) ────────
//
// 스키마 근거 = 실측 fixture `backend/fixtures/claude_{text,tool}.jsonl`.

/// ★미종결 라인 버퍼 상한★: 개행이 영영 오지 않는 malformed/폭주 출력이면 버퍼가 무한 증가해
///   OOM 을 낸다. 통로는 바보 파이프(ADR-0044 무정제 불변)라 상류가 라인을 보장하지 않으므로
///   소비자(decoder)가 방어한다 — 4MB 넘으면 부분 라인을 버리고 다음 개행부터 복구한다. NDJSON
///   한 라인이 4MB 를 넘는 정상 케이스는 없다(thinking/text 블록도 그보다 훨씬 작다) → 상한 초과
///   = 비정상으로 간주.
const MAX_BUFFER_BYTES: usize = 4 * 1024 * 1024;

/// claude stream-json 라이브 decoder.
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
    ///   상태가 있어야 한다.
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

        // ★resync(FIX-A)★: 오염 라인의 꼬리를 버리는 중이면 다음 `\n` 앞을 통째 버린다. 개행이
        //   없으면 청크 전체가 아직 그 라인의 일부다 — 버퍼에 쌓지 않고 종료한다.
        let chunk = if self.discarding {
            match chunk.iter().position(|&b| b == b'\n') {
                Some(nl) => {
                    self.discarding = false;
                    &chunk[nl + 1..]
                }
                None => return events,
            }
        } else {
            chunk
        };

        self.buffer.extend_from_slice(chunk);

        // 마지막 개행 뒤 잔여는 tail 로 buffer 에 남겨 다음 청크와 합친다(FIX-D: 주석을 실제 코드와 일치).
        while let Some(nl) = self.buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=nl).collect();
            Self::consume_line(&line[..line.len() - 1], &mut events);
        }

        // ★단순 clear 가 아니라 resync 진입(FIX-A)★: buffer 만 비우면 이 오염 라인의 나머지 꼬리가
        //   다음 `\n` 까지 새 라인으로 파싱돼 가짜 이벤트를 낸다. 오염 라인 1개만 잃고 복구한다.
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
    /// - `result` 라인 → MessageDone(+ result.usage 있으면 Usage 추가 emit;
    ///   is_error/subtype 이 error 계열이면 MessageDone **앞에** Error 도 emit — FIX-C).
    ///   ※ result 의 오류 표면화는 **백엔드 신규 정책**이다(프론트 파서엔 없던 판정).
    /// - `system`/`rate_limit_event`/그 외 unknown type → skip(0개).
    fn consume_line(line: &[u8], events: &mut Vec<OutputEvent>) {
        // ★여기서 처음 UTF-8 디코딩★(위 buffer 불변식). lossy 가 아니라 엄격 검증 후 실패 시 skip —
        //   비-UTF8 라인은 claude 정상 출력이 아니다(터미널 경로가 아니다).
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
            Some(role @ ("assistant" | "user")) => {
                let msg = match value.get("message") {
                    Some(m) => m,
                    None => return,
                };
                let message_id = msg.get("id").and_then(|v| v.as_str()).map(String::from);
                // ★user replay dedup 키★: line-level 이라 블록 루프 밖에서 1회 추출한다. assistant
                //   라인엔 이 개념이 없어 None 이 된다(consume_block 의 assistant arm 은 안 쓴다).
                let line_uuid = value.get("uuid").and_then(|v| v.as_str());
                let blocks = match msg.get("content").and_then(|c| c.as_array()) {
                    Some(arr) => arr,
                    None => return, // content 가 배열이 아니면(스키마 이탈) skip
                };
                for block in blocks {
                    Self::consume_block(role, block, message_id.as_deref(), line_uuid, events);
                }
            }
            Some("result") => {
                // ★Usage 를 MessageDone 보다 먼저 emit★: 소비자가 "턴 종료" 신호를 보기 전에 그 턴의
                //   최종 토큰 집계를 받게 순서를 고정한다(뒤에 오면 종료 후 지연 도착처럼 보인다).
                //   result.usage.{input_tokens,output_tokens} — 실측 fixture 확인(text.jsonl 라인5:
                //   input=17095, output=4).
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
                // ★실패 턴 표면화(FIX-C)★: 늘 MessageDone 만 내면 API 오류·max-turns·거부로 실패한
                //   턴이 "정상 완료"로 위장된다. is_error:true payload 는 미캡처(실측 fixture 없음)라
                //   존재하는 필드만 문자열화해 담는다. 순서는 Error → MessageDone(소비자가 종료 신호를
                //   보기 전에 오류를 알도록).
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

    /// content[] 한 블록 → OutputEvent.
    ///
    /// `line_uuid`: user 라인의 line-level `uuid`(replay dedup 키). user-role 블록에만 쓴다.
    fn consume_block(
        role: &str,
        block: &serde_json::Value,
        message_id: Option<&str>,
        line_uuid: Option<&str>,
        events: &mut Vec<OutputEvent>,
    ) {
        // ★user 라인 블록은 통째로 Structured{kind:"user"} 로 보존★: OutputEvent 에 role 개념이 없어
        //   (assistant 전용 필드만) user replay 턴을 정형 variant 로 표현할 수 없다 → 원본 블록을
        //   그대로 탈출구로 넘긴다.
        if role == "user" {
            // ★억제 금지(되살리지 말 것)★: 예전엔 user-role text 블록을 무조건 억제했다 — 입력-시점
            //   합성 에코와 중복이라는 이유였지만, resume 을 켜면 과거 user text 가 전부 사라지는
            //   버그였다(합성 에코를 만든 적 없는 라인까지 삭제). 대신 uuid 를 실어 통과시키고 dedup 은
            //   프론트에 맡긴다.
            //   ★tool_result 안전(HIGH FIX)★: 한 user 라인에 텍스트 에코와 tool_result 가 함께 올 수
            //     있다. 모든 블록에 같은 line-level uuid 를 실어도 프론트 dedup 은 `type==="text"` 에만
            //     걸리므로(extractUserUuid 가 비-text 에 null 반환) tool_result 는 항상 보존된다.
            //   uuid 가 없으면(과거 라인·비-replay) 원본 그대로 — 그런 item 은 dedup 되지 않는다.
            let json = match line_uuid {
                Some(u) => Self::user_block_with_uuid(block, u),
                None => block.to_string(),
            };
            events.push(OutputEvent::Structured {
                kind: "user".to_string(),
                json,
            });
            return;
        }

        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                // 통짜 모드라 실은 델타가 아닌 완결 텍스트지만, OutputEvent 에 "완결 텍스트" variant 가
                //   없고 TextDelta 가 텍스트 증분의 정형 표현이다.
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
                // id 는 tool_use.id — 뒤따르는 tool_result 와 짝짓는 키다.
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
            None => {}
        }
    }

    fn structured(kind: &str, value: &serde_json::Value) -> OutputEvent {
        OutputEvent::Structured {
            kind: kind.to_string(),
            json: value.to_string(),
        }
    }

    /// user replay 블록에 line-level `uuid` 를 얹어 직렬화(dedup 키 부착).
    ///
    /// ★shape 계약★: user_text_echo_json 이 만드는 합성 에코(`{"type":…,…,"uuid":"X"}`)와 동형이
    ///   되도록 원본 블록 객체에 top-level `uuid` 키를 추가한다. 원본이 JSON object 가 아니면(스키마
    ///   이탈) uuid 를 얹을 자리가 없어 원본 그대로 직렬화한다(방어적 — 정상 replay 블록은 항상 object).
    ///   블록이 이미 `uuid` 를 갖고 있어도 line-level(우리 통제값)로 덮는다 — dedup 키의 단일 출처.
    fn user_block_with_uuid(block: &serde_json::Value, uuid: &str) -> String {
        match block.as_object() {
            Some(map) => {
                let mut owned = map.clone();
                owned.insert(
                    "uuid".to_string(),
                    serde_json::Value::String(uuid.to_string()),
                );
                serde_json::Value::Object(owned).to_string()
            }
            None => block.to_string(),
        }
    }
}

// ── ADR-0079: resume 시 `.jsonl` transcript → OutputEvent seed ──────
//
// ★매핑 재사용(디코더 한 벌)★: transcript 의 `assistant`/`user` 라인은 라이브 stream-json 과 **동일한**
//   봉투(top-level `type` + `message.content[]` 블록)를 갖는다(실측 2026-07-13, claude 2.1.170). 그래서
//   과거 턴 매핑을 새로 짜지 않고 라이브 디코더의 `consume_line` 을 그대로 재사용한다. transcript 는
//   봉투에 추가 top-level 키(`parentUuid`/`uuid`/`timestamp`/`sessionId`/`isSidechain` …)와 라이브에
//   없는 라인 타입(`summary`/`file-history-snapshot`/`queue-operation`/`attachment`/`ai-title`)을 더 싣지만,
//   `consume_line` 의 catch-all(`_ => {}`)이 모르는 타입을 이미 무해히 스킵하므로 그 라인들은 자연 배제된다.
//   유일한 추가 필터는 `isSidechain:true`(sub-agent 턴) — 이건 `type` 이 여전히 user/assistant 라
//   consume_line 이 안 걸러내므로 여기서 라인 레벨로 스킵한다.

/// transcript seed 시 파싱할 최대 바이트(파일 끝에서부터). Ring 상한(2MB/4096 events)에 맞춰
/// **파일 끝(tail)에서 이만큼만** 읽어 파싱한다 — 거대한 `.jsonl`(수십 MB)을 통째 파싱한 뒤 Ring 이
/// 어차피 버릴 오래된 것까지 훑으면 spawn 지연이 폭발한다. 상한 초과분(오래된 과거)은 잘려도 수용
/// (ADR-0079 — Ring 도 어차피 오래된 것부터 evict). 2MB + 여유로 4MB(seek 지점의 잘린 첫 부분 라인은
/// read_transcript_events 가 첫 `\n` 까지 명시 폐기하므로 안전).
const TRANSCRIPT_TAIL_BYTES: u64 = 4 * 1024 * 1024;

/// cwd 를 claude 프로젝트 디렉토리 슬러그로 인코딩한다.
///
/// ★인코딩 규칙(실측 확정 2026-07-13)★: cwd 의 **모든 비-영숫자 문자**(`:` `\` `/` `.` `_` 공백 등)를
///   `-` 로 치환한다. 예: `C:\Users\X\AppData\Local\Temp\engram-resume-test` →
///   `C--Users-X-AppData-Local-Temp-engram-resume-test`(콜론+백슬래시 = `--`), `I:\Engram_Workspace\a`
///   → `I--Engram-Workspace-a`. claude 가 `~/.claude/projects/<slug>/` 디렉토리를 이 규칙으로 만든다
///   (로컬 45개 프로젝트 디렉토리 대조로 100% 일치 확인). 손실적(다른 cwd 가 같은 슬러그가 될 수 있음)이나
///   claude 실제 동작과 일치시키는 것이 목적이라 그대로 따른다.
fn project_slug(cwd: &std::path::Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// `~/.claude/projects/<slug>/<sid>.jsonl` 경로. `CLAUDE_CONFIG_DIR` 이 설정돼 있으면 우선한다
/// (session_tracker 의 default_sessions_dir 과 동일 규약). home 을 못 찾으면 None.
fn transcript_path(cwd: &std::path::Path, sid: Uuid) -> Option<PathBuf> {
    let base = if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        if dir.is_empty() {
            claude_home()?.join(".claude")
        } else {
            PathBuf::from(dir)
        }
    } else {
        claude_home()?.join(".claude")
    };
    Some(
        base.join("projects")
            .join(project_slug(cwd))
            .join(format!("{sid}.jsonl")),
    )
}

#[cfg(windows)]
fn claude_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}
#[cfg(not(windows))]
fn claude_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// ADR-0079: transcript 원문(라인 NDJSON) → 과거 `OutputEvent` 목록. **순수 함수(외부 의존 0)** — 실
/// 픽스처로 단위 테스트한다. 파일 I/O 는 `read_transcript_events` 가 담당하고 이 함수는 이미 읽은
/// 문자열만 받는다(ADR-0012 seam 격리).
///
/// - `isSidechain:true`(sub-agent 턴) 라인은 스킵한다 — 원본 대화만 복원한다.
/// - result 라인은 라이브와 동일하게 MessageDone(+usage) 로 매핑돼 턴 경계 구분선이 생긴다.
pub(crate) fn parse_transcript_events(transcript: &str) -> Vec<OutputEvent> {
    let mut events = Vec::new();
    for line in transcript.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // top-level 필드만 훑어도 되지만 이미 라인 하나라 파싱 비용이 작고 정확도가 높아 serde 로
        //   판정한다(비-JSON 라인은 어차피 consume_line 이 스킵).
        if is_sidechain_line(trimmed) {
            continue;
        }
        ClaudeStreamDecoder::consume_line(trimmed.as_bytes(), &mut events);
    }
    events
}

/// 라인이 `isSidechain:true` 인가(sub-agent 턴). 비-JSON/필드 부재는 false(=원본 턴 취급, 보존).
fn is_sidechain_line(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|v| v.get("isSidechain").and_then(|s| s.as_bool()))
        .unwrap_or(false)
}

/// ADR-0079: resume 스폰 시 데몬이 부르는 진입점. 파일이 없거나(신규 세션) 읽기 실패면 빈
/// Vec(seed 안 함 = fresh 와 동일). **여기가 유일한 파일 I/O 지점**이고 파싱은 순수 함수에 위임한다.
///
/// ★tail 읽기(spawn 지연 방지)★: 파일이 상한보다 크면 끝에서 TRANSCRIPT_TAIL_BYTES 만 읽는다. 그 경우
///   seek 지점의 첫 (부분) 라인은 오브젝트 중간에서 잘렸으므로 **명시적으로 폐기**한다 — 첫 `\n` 까지의
///   바이트를 버린 뒤 파싱한다(cross-family review 2026-07-13). "부분 라인은 어차피 비-JSON"이라는 암묵
///   가정에 의존하지 않는다: 잘린 조각이 우연히 유효한 JSON suffix 로 끝나면 가짜 이벤트가 합성될 수 있어서다.
///   seek offset 이 0(파일 ≤ 상한)이면 첫 라인은 온전하므로 폐기하지 않는다.
///
/// ★bounded read(동시 성장 방어)★: `read_to_end` 는 파일이 읽는 도중 커지면 무한정 읽는다. `take` 로
///   최대 TRANSCRIPT_TAIL_BYTES 만 읽어 상한을 하드하게 건다(seek 지점부터 딱 그만큼). seek 실패는
///   조용히 임의 위치에서 읽지 않고 명시적으로 빈 Vec 을 돌린다(오프셋 오염 방지).
pub(crate) fn read_transcript_events(cwd: &std::path::Path, sid: Uuid) -> Vec<OutputEvent> {
    use std::io::{Read, Seek, SeekFrom};

    let Some(path) = transcript_path(cwd, sid) else {
        return Vec::new();
    };
    let Ok(mut file) = std::fs::File::open(&path) else {
        return Vec::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let seeked = len > TRANSCRIPT_TAIL_BYTES;
    if seeked {
        // seek 실패면 파일 포인터 위치가 불명 → 임의 위치 읽기 대신 포기(빈 Vec).
        if file
            .seek(SeekFrom::Start(len - TRANSCRIPT_TAIL_BYTES))
            .is_err()
        {
            return Vec::new();
        }
    }
    let mut buf = Vec::new();
    if file
        .take(TRANSCRIPT_TAIL_BYTES)
        .read_to_end(&mut buf)
        .is_err()
    {
        return Vec::new();
    }
    // 바이트→문자열은 lossy — 잘린 첫 라인의 부분 멀티바이트 문자를 흡수한다(그 라인은 아래서 폐기).
    let text = String::from_utf8_lossy(&buf);
    // `\n` 이 없으면(= 상한 안에 개행 하나도 없는 초장문 단일 라인) 온전한 라인이 없다.
    let to_parse: &str = if seeked {
        match text.find('\n') {
            Some(idx) => &text[idx + 1..],
            None => "",
        }
    } else {
        &text
    };
    parse_transcript_events(to_parse)
}

// ── S15 B3: pump→core 배선 seam (ADR-0004/0044) ──────────────────────────────────
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

    fn spec(command: &AgentCommand, mode: SpawnMode, sid: Option<Uuid>) -> CommandSpec {
        ClaudeBackend.build_spec(command, mode, sid, PathBuf::from("."), vec![], None)
    }

    fn spec_with_control(
        command: &AgentCommand,
        mode: SpawnMode,
        sid: Option<Uuid>,
        control: Option<ControlEndpoint>,
    ) -> CommandSpec {
        ClaudeBackend.build_spec(command, mode, sid, PathBuf::from("."), vec![], control)
    }

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
        let (p, a) = console_command(
            CLAUDE_PROGRAM,
            vec![
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
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
            vec![
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
                "--resume".to_string(),
                sid.to_string(),
            ],
        );
        assert_eq!(s.args, a);
    }

    // ── ADR-0086: `--mcp-config` 주입(제어 채널 입구) ─────────────────────────────────
    fn ep() -> ControlEndpoint {
        ControlEndpoint {
            url: "http://127.0.0.1:54321/mcp".to_string(),
            token: "deadbeef".to_string(),
            // ADR-0099: config_path 는 Option — MCP-capable(claude) 케이스라 Some.
            config_path: Some(PathBuf::from("C:/data/mcp/agent-x.json")),
            // 기본 헬퍼는 send_exe 를 담아 ENGRAM_CLI_EXE 주입 경로를 검증할 수 있게 한다.
            send_exe: Some(PathBuf::from("C:/app/engram.exe")),
            // 기본 헬퍼는 프라이밍 파일을 담지 않는다(ADR-0092) — 프라이밍 주입 테스트가 명시로 채운다.
            priming_file: None,
            // ADR-0094: 기본 헬퍼는 grants 를 비워둔다 — grant 주입 테스트가 명시로 채운다(회귀 격리:
            //   기존 mcp-config/priming 테스트가 --allowedTools 영향을 안 받게).
            grants: vec![],
            // S18 D: 기본 헬퍼는 설정 조각을 담지 않는다 — `--settings` 주입 테스트가 명시로 채운다
            //   (기존 arg-golden 테스트가 새 플래그의 영향을 안 받게 — priming_file 과 같은 규율).
            settings_file: None,
            // ADR-0133: MCP 가능 갈래(config_path=Some)의 데몬 산출값과 같은 짝 — 우편 불가.
            mail_allowed: false,
        }
    }

    /// 세션 한정 설정 조각 경로를 담은 변주(S18 D · spec §6) — `--settings` 주입 검증용.
    fn ep_with_settings() -> ControlEndpoint {
        ControlEndpoint {
            settings_file: Some(PathBuf::from("C:/data/mcp/agent-x.settings.json")),
            ..ep()
        }
    }

    /// 발신 입구 grant(MCP + CLI)를 담은 변주(ADR-0094) — `--allowedTools` 주입 검증용.
    fn ep_with_grants() -> ControlEndpoint {
        ControlEndpoint {
            grants: vec![
                ToolGrant::Mcp {
                    server: "engram".to_string(),
                    tool: "send_message".to_string(),
                },
                ToolGrant::Cli {
                    exe: CLI_EXE_NAME.to_string(),
                },
            ],
            ..ep()
        }
    }

    /// 프라이밍 파일 경로를 담은 변주(ADR-0092) — `--append-system-prompt-file` 주입 검증용.
    fn ep_with_priming() -> ControlEndpoint {
        ControlEndpoint {
            priming_file: Some(PathBuf::from("C:/repo/prompts/agent-priming.md")),
            ..ep()
        }
    }

    /// 비-MCP 백엔드 스폰이 받는 endpoint — `config_path=None`(MCP 입구 부재) + 우편 허용. CLI 배선 자체는
    ///   두 변주가 똑같이 받으므로(ADR-0133), 이 변주가 홀로 지키는 것은 **mcp-config 부재**와 **우편 표식
    ///   on** 두 축이다.
    fn ep_cli_only() -> ControlEndpoint {
        ControlEndpoint {
            config_path: None,
            mail_allowed: true,
            ..ep()
        }
    }

    /// 비-MCP + send_exe=None(형제 바이너리 부재 모사).
    fn ep_cli_only_no_send() -> ControlEndpoint {
        ControlEndpoint {
            send_exe: None,
            ..ep_cli_only()
        }
    }

    #[test]
    fn claude_control_endpoint_injects_mcp_config_flag() {
        let sid = Uuid::new_v4();
        let s = spec_with_control(&terminal(vec![]), SpawnMode::Fresh, Some(sid), Some(ep()));
        let (_p, a) = console_command(
            CLAUDE_PROGRAM,
            vec![
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
                "--session-id".to_string(),
                sid.to_string(),
                "--mcp-config".to_string(),
                "C:/data/mcp/agent-x.json".to_string(),
                "--disallowedTools".to_string(),
                "SendMessage".to_string(),
            ],
        );
        assert_eq!(s.args, a, "터미널 모드 claude 에 --mcp-config 주입");
    }

    #[test]
    fn claude_control_endpoint_token_never_in_args() {
        // ★보안 회귀 가드★: 토큰은 args 에 절대 실리지 않는다(config_path 파일 안에만). 오직 파일
        //   경로만 args 에 온다 → args/프로세스 목록/로그에 토큰 평문 노출 없음.
        let s = spec_with_control(&terminal(vec![]), SpawnMode::Fresh, None, Some(ep()));
        assert!(
            !s.args.iter().any(|a| a.contains("deadbeef")),
            "토큰이 args 에 새면 안 됨: {:?}",
            s.args
        );
        assert!(
            s.args.iter().any(|a| a == "--mcp-config"),
            "--mcp-config 플래그는 있어야 함"
        );
    }

    #[test]
    fn claude_no_control_endpoint_no_mcp_config() {
        let s = spec(&terminal(vec!["--debug"]), SpawnMode::Fresh, None);
        assert!(
            !s.args.iter().any(|a| a == "--mcp-config"),
            "control 없으면 --mcp-config 없음: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_control_endpoint_none_config_path_no_mcp_config_flag() {
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_cli_only()),
        );
        assert!(
            !s.args.iter().any(|a| a == "--mcp-config"),
            "config_path=None → --mcp-config 미주입: {:?}",
            s.args
        );
        assert!(
            s.env.iter().any(|(k, _)| k == "ENGRAM_TOKEN"),
            "config_path=None(CLI 전용) → ENGRAM_TOKEN 주입"
        );
    }

    // ── ADR-0086 스텝 2: CLI 크레덴셜 env 주입(ENGRAM_TOKEN / ENGRAM_CONTROL_URL) ──────────────
    #[test]
    fn claude_cli_only_endpoint_injects_cli_env() {
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_cli_only()),
        );
        let token = s
            .env
            .iter()
            .find(|(k, _)| k == "ENGRAM_TOKEN")
            .map(|(_, v)| v.as_str());
        let url = s
            .env
            .iter()
            .find(|(k, _)| k == "ENGRAM_CONTROL_URL")
            .map(|(_, v)| v.as_str());
        assert_eq!(token, Some("deadbeef"), "ENGRAM_TOKEN = endpoint 토큰");
        assert_eq!(
            url,
            Some("http://127.0.0.1:54321"),
            "ENGRAM_CONTROL_URL = MCP url 에서 /mcp 를 벗긴 base"
        );
        let send_exe = s
            .env
            .iter()
            .find(|(k, _)| k == CLI_EXE_ENV)
            .map(|(_, v)| v.as_str());
        assert_eq!(
            send_exe,
            Some("C:/app/engram.exe"),
            "ENGRAM_CLI_EXE = endpoint.send_exe 절대경로"
        );
    }

    #[test]
    fn claude_cli_only_without_send_exe_omits_send_env() {
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_cli_only_no_send()),
        );
        assert!(
            s.env.iter().any(|(k, _)| k == "ENGRAM_TOKEN")
                && s.env.iter().any(|(k, _)| k == "ENGRAM_CONTROL_URL"),
            "send_exe 없어도 token/url 은 주입: {:?}",
            s.env
        );
        assert!(
            !s.env.iter().any(|(k, _)| k == CLI_EXE_ENV),
            "send_exe=None 이면 ENGRAM_CLI_EXE 는 생략: {:?}",
            s.env
        );
    }

    #[test]
    fn claude_no_control_endpoint_no_cli_env() {
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, None);
        for key in [
            "ENGRAM_TOKEN",
            "ENGRAM_CONTROL_URL",
            CLI_EXE_ENV,
            MAIL_MARKER_ENV,
        ] {
            assert!(
                !s.env.iter().any(|(k, _)| k == key),
                "control 없으면 CLI env 없음({key}): {:?}",
                s.env
            );
        }
    }

    // ── ADR-0094: PATH 주입(bare 실행파일 이름 해석 — send_exe 부모 디렉토리 prepend) ──────────
    #[test]
    fn claude_cli_only_endpoint_injects_path_with_send_exe_dir_prepended() {
        // ep_cli_only() 의 send_exe = C:/app/engram.exe → 부모 = C:/app.
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_cli_only()),
        );
        let path = s
            .env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.as_str())
            .expect("send_exe 있으면 PATH env 주입");
        // 문자열 prefix 비교가 아니라 split_paths 분해로 단언한다 — 구분자·표기 차를 흡수.
        let first = std::env::split_paths(path)
            .next()
            .expect("PATH 에 최소 한 요소");
        assert_eq!(
            first,
            std::path::PathBuf::from("C:/app"),
            "PATH 첫 요소 = send_exe 부모 디렉토리: {path}"
        );
        if let Some(orig) = std::env::var_os("PATH") {
            if let Some(orig_first) = std::env::split_paths(&orig).next() {
                let injected: Vec<std::path::PathBuf> = std::env::split_paths(path).collect();
                assert!(
                    injected.iter().skip(1).any(|c| *c == orig_first),
                    "원래 PATH 의 첫 컴포넌트가 주입값의 부모 뒤에 보존돼야 함: injected={injected:?} orig_first={orig_first:?}"
                );
            }
        }
    }

    #[test]
    fn claude_cli_only_without_send_exe_omits_path_env() {
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_cli_only_no_send()),
        );
        assert!(
            !s.env.iter().any(|(k, _)| k == "PATH"),
            "send_exe=None 이면 PATH env 미주입: {:?}",
            s.env
        );
    }

    #[test]
    fn claude_no_control_endpoint_no_path_env() {
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, None);
        assert!(
            !s.env.iter().any(|(k, _)| k == "PATH"),
            "control 없으면 PATH env 없음: {:?}",
            s.env
        );
    }

    /// 프로필 env(초기 env 벡터)를 주입하는 build_spec 헬퍼 — 프로필 PATH 우선(FIX-1) 검증용.
    ///   spec_with_control 은 항상 빈 env 로 시작하므로, 프로필이 미리 심은 env 를 재현하려면
    ///   build_spec 을 직접 호출한다(spawn 경로가 profile.env 를 먼저 push 하는 상황 모사).
    fn spec_with_env(
        command: &AgentCommand,
        control: Option<ControlEndpoint>,
        profile_env: Vec<(String, String)>,
    ) -> CommandSpec {
        ClaudeBackend.build_spec(
            command,
            SpawnMode::Fresh,
            None,
            PathBuf::from("."),
            profile_env,
            control,
        )
    }

    fn path_entries(s: &CommandSpec) -> Vec<&str> {
        s.env
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("PATH"))
            .map(|(_, v)| v.as_str())
            .collect()
    }

    #[test]
    fn claude_profile_path_survives_as_tail_uppercase_key() {
        let profile_env = vec![("PATH".to_string(), "C:\\custom;C:\\other".to_string())];
        let s = spec_with_env(&terminal(vec![]), Some(ep_cli_only()), profile_env);
        let entries = path_entries(&s);
        assert_eq!(
            entries.len(),
            1,
            "승리 PATH 는 정확히 하나(기존 프로필 항목 제자리 교체): {:?}",
            s.env
        );
        let components: Vec<std::path::PathBuf> = std::env::split_paths(entries[0]).collect();
        assert_eq!(
            components.first(),
            Some(&std::path::PathBuf::from("C:/app")),
            "PATH 첫 컴포넌트 = send_exe 부모(C:/app): {:?}",
            components
        );
        assert!(
            components.contains(&std::path::PathBuf::from("C:\\custom"))
                && components.contains(&std::path::PathBuf::from("C:\\other")),
            "프로필 PATH(C:\\custom;C:\\other)가 tail 로 생존해야 함: {:?}",
            components
        );
    }

    #[cfg(windows)]
    #[test]
    fn claude_profile_path_survives_with_windows_path_key_casing() {
        let profile_env = vec![("Path".to_string(), "C:\\custom;C:\\other".to_string())];
        let s = spec_with_env(&terminal(vec![]), Some(ep_cli_only()), profile_env);
        let entries = path_entries(&s);
        assert_eq!(
            entries.len(),
            1,
            "Windows: 'Path' 키를 같은 변수로 인식 → 승리 PATH 정확히 하나: {:?}",
            s.env
        );
        let components: Vec<std::path::PathBuf> = std::env::split_paths(entries[0]).collect();
        assert_eq!(
            components.first(),
            Some(&std::path::PathBuf::from("C:/app")),
            "PATH 첫 컴포넌트 = send_exe 부모: {:?}",
            components
        );
        assert!(
            components.contains(&std::path::PathBuf::from("C:\\custom"))
                && components.contains(&std::path::PathBuf::from("C:\\other")),
            "프로필 'Path' 값이 tail 로 생존: {:?}",
            components
        );
        assert!(
            s.env.iter().any(|(k, _)| k == "Path"),
            "제자리 교체라 프로필 키 표기('Path')를 유지: {:?}",
            s.env
        );
    }

    #[cfg(windows)]
    #[test]
    fn claude_duplicate_case_variant_path_uses_last_value_and_dedupes() {
        let profile_env = vec![
            ("PATH".to_string(), "C:\\daemon".to_string()),
            ("Path".to_string(), "C:\\profile".to_string()),
        ];
        let s = spec_with_env(&terminal(vec![]), Some(ep_cli_only()), profile_env);
        let entries = path_entries(&s);
        assert_eq!(
            entries.len(),
            1,
            "case-equivalent PATH 는 정확히 하나(중복 제거): {:?}",
            s.env
        );
        assert!(
            s.env.iter().any(|(k, _)| k == "Path") && !s.env.iter().any(|(k, _)| k == "PATH"),
            "승리 키 표기 = 마지막 항목('Path'), 'PATH' 는 제거됨: {:?}",
            s.env
        );
        let components: Vec<std::path::PathBuf> = std::env::split_paths(entries[0]).collect();
        assert_eq!(
            components.first(),
            Some(&std::path::PathBuf::from("C:/app")),
            "PATH 첫 컴포넌트 = send_exe 부모(C:/app): {:?}",
            components
        );
        assert!(
            components.contains(&std::path::PathBuf::from("C:\\profile")),
            "tail = 마지막 PATH 값(C:\\profile) 보존: {:?}",
            components
        );
        assert!(
            !components.contains(&std::path::PathBuf::from("C:\\daemon")),
            "첫 PATH 값(C:\\daemon)은 base 로 쓰이지 않아야 함(마지막이 이김): {:?}",
            components
        );
    }

    #[test]
    fn claude_duplicate_same_key_path_uses_last_value_and_dedupes() {
        // 정확히 같은 키 중복 — case-insensitive 매칭에 의존하지 않아 모든 OS 에서 돈다
        //   (cfg(windows) 형제 테스트와의 구분).
        let profile_env = vec![
            ("PATH".to_string(), "C:\\first".to_string()),
            ("PATH".to_string(), "C:\\second".to_string()),
        ];
        let s = spec_with_env(&terminal(vec![]), Some(ep_cli_only()), profile_env);
        let entries = path_entries(&s);
        assert_eq!(
            entries.len(),
            1,
            "PATH 는 정확히 하나(중복 제거): {:?}",
            s.env
        );
        let components: Vec<std::path::PathBuf> = std::env::split_paths(entries[0]).collect();
        assert_eq!(
            components.first(),
            Some(&std::path::PathBuf::from("C:/app")),
            "PATH 첫 컴포넌트 = send_exe 부모: {:?}",
            components
        );
        assert!(
            components.contains(&std::path::PathBuf::from("C:\\second")),
            "tail = 마지막 PATH 값(C:\\second) 보존: {:?}",
            components
        );
        assert!(
            !components.contains(&std::path::PathBuf::from("C:\\first")),
            "첫 PATH 값(C:\\first)은 base 로 쓰이지 않아야 함(마지막이 이김): {:?}",
            components
        );
    }

    // ★비-UTF8/join-failure skip 은 이식성 있게 재현 불가★: join_paths 실패(경로에 세퍼레이터 문자
    //   포함)나 비-UTF8 PATH 를 CommandSpec.env(=Vec<(String,String)>) 경계에서 만들려면 이미 유효
    //   UTF-8 String 이어야 해 모순이고, OsString 비-UTF8 주입 경로가 없다. 그래서 이 스킵 분기는
    //   단위 테스트로 강제하지 않는다(FIX-2/3 주석·loud warn 으로 관측성 확보). 실패 시 오늘 동작
    //   (상속 PATH 유지)이라 안전 폴백 — 회귀 위험 낮음.

    // ── ADR-0133: 배선은 전원에게, 우편만 표식으로 갈린다(두 갈래 양방향 단언) ──────────────────
    fn env_value<'a>(s: &'a CommandSpec, key: &str) -> Option<&'a str> {
        s.env
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// ★프로필 PATH 를 반드시 실어 둔다(빈 env 면 PATH 단언이 공허해진다)★: 항목이 0 개면 tail 생존을
    ///   견줄 대상이 없다.
    #[test]
    fn claude_mcp_capable_spawn_gets_cli_wiring_with_mail_marked_off() {
        let profile_env = vec![("PATH".to_string(), "C:\\custom".to_string())];
        let s = spec_with_env(&terminal(vec![]), Some(ep()), profile_env);
        for key in ["ENGRAM_TOKEN", "ENGRAM_CONTROL_URL", CLI_EXE_ENV] {
            assert!(
                s.env.iter().any(|(k, _)| k == key),
                "MCP 가능 스폰도 제어 CLI 입구를 받아야({key}) — 제어는 전원 개방: {:?}",
                s.env
            );
        }
        let entries = path_entries(&s);
        assert_eq!(entries.len(), 1, "PATH 는 정확히 하나: {:?}", s.env);
        let components: Vec<PathBuf> = std::env::split_paths(entries[0]).collect();
        assert_eq!(
            components.first(),
            Some(&PathBuf::from("C:/app")),
            "PATH 맨 앞 = send_exe 형제 디렉토리: {components:?}"
        );
        assert!(
            components.contains(&PathBuf::from("C:\\custom")),
            "프로필 PATH 가 tail 로 생존: {components:?}"
        );
        assert_eq!(
            env_value(&s, MAIL_MARKER_ENV),
            Some(MAIL_MARKER_OFF),
            "MCP 가능 스폰은 우편 표식이 off — 사용법에서 우편 계열을 감춘다: {:?}",
            s.env
        );
        assert!(
            s.args.iter().any(|a| a == "--mcp-config"),
            "MCP 갈래는 mcp-config 도 함께 받아야(배선과 배타가 아니다): {:?}",
            s.args
        );
    }

    #[test]
    fn claude_non_mcp_spawn_gets_the_same_wiring_with_mail_marked_on() {
        let profile_env = vec![("PATH".to_string(), "C:\\custom".to_string())];
        let s = spec_with_env(&terminal(vec![]), Some(ep_cli_only()), profile_env);
        for key in ["ENGRAM_TOKEN", "ENGRAM_CONTROL_URL", CLI_EXE_ENV] {
            assert!(
                s.env.iter().any(|(k, _)| k == key),
                "비-MCP 스폰은 {key} 를 받아야: {:?}",
                s.env
            );
        }
        let entries = path_entries(&s);
        assert_eq!(entries.len(), 1, "PATH 는 정확히 하나: {:?}", s.env);
        let components: Vec<PathBuf> = std::env::split_paths(entries[0]).collect();
        assert_eq!(
            components.first(),
            Some(&PathBuf::from("C:/app")),
            "PATH 맨 앞 = send_exe 형제 디렉토리: {components:?}"
        );
        assert!(
            components.contains(&PathBuf::from("C:\\custom")),
            "프로필 PATH 가 tail 로 생존: {components:?}"
        );
        assert_eq!(
            env_value(&s, MAIL_MARKER_ENV),
            Some(MAIL_MARKER_ON),
            "비-MCP 스폰은 우편 표식이 on: {:?}",
            s.env
        );
        assert!(
            !s.args.iter().any(|a| a == "--mcp-config"),
            "비-MCP 갈래엔 mcp-config 없음: {:?}",
            s.args
        );
    }

    /// ★표식은 endpoint 가 실어 준 사실을 그대로 옮긴다 — backend 가 다른 필드에서 유도하지 않는다★:
    ///   `config_path` 로 정책을 재파생하면 데몬 판정과 backend 판정 두 곳이 생겨 갈릴 수 있다.
    #[test]
    fn claude_mail_marker_follows_the_endpoint_not_the_mcp_config_field() {
        let mcp_with_mail = ControlEndpoint {
            mail_allowed: true,
            ..ep()
        };
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(mcp_with_mail),
        );
        assert_eq!(env_value(&s, MAIL_MARKER_ENV), Some(MAIL_MARKER_ON));
        assert!(s.args.iter().any(|a| a == "--mcp-config"));
    }

    // ── ADR-0092: 프라이밍 주입(`--append-system-prompt-file`) ──────────────────────────
    #[test]
    fn claude_priming_file_injects_append_system_prompt_flag_terminal() {
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_with_priming()),
        );
        let pos = s
            .args
            .iter()
            .position(|a| a == "--append-system-prompt-file")
            .expect("프라이밍 파일 있으면 --append-system-prompt-file 주입");
        assert_eq!(
            s.args.get(pos + 1).map(|s| s.as_str()),
            Some("C:/repo/prompts/agent-priming.md"),
            "플래그 다음 인자 = 프라이밍 MD 절대경로: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_priming_file_injects_flag_json_mode() {
        let s = spec_with_control(
            &json(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_with_priming()),
        );
        assert!(
            s.args.iter().any(|a| a == "--append-system-prompt-file"),
            "json 모드에도 프라이밍 주입: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_no_priming_file_no_append_flag() {
        let s = spec_with_control(&terminal(vec![]), SpawnMode::Fresh, None, Some(ep()));
        assert!(
            !s.args.iter().any(|a| a == "--append-system-prompt-file"),
            "프라이밍 없으면 append-system-prompt-file 없음: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_no_control_endpoint_no_priming_flag() {
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, None);
        assert!(
            !s.args.iter().any(|a| a == "--append-system-prompt-file"),
            "control 없으면 프라이밍 플래그 없음: {:?}",
            s.args
        );
    }

    // ── S18 D(spec §6): 세션 한정 설정 조각 주입(`--settings`) ────────────────────────────
    #[test]
    fn claude_settings_file_injects_settings_flag_terminal() {
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_with_settings()),
        );
        let pos = s
            .args
            .iter()
            .position(|a| a == "--settings")
            .expect("settings_file 있으면 --settings 주입");
        assert_eq!(
            s.args.get(pos + 1).map(|s| s.as_str()),
            Some("C:/data/mcp/agent-x.settings.json"),
            "플래그 다음 인자 = 설정 조각 절대경로: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_settings_file_injects_flag_json_mode() {
        // json(stream-json) 헤드리스가 실제 스폰 경로다 — 여기 안 붙으면 대책 자체가 무의미.
        let s = spec_with_control(
            &json(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_with_settings()),
        );
        assert!(
            s.args.iter().any(|a| a == "--settings"),
            "json 모드에도 설정 조각 주입: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_no_settings_file_no_settings_flag() {
        let s = spec_with_control(&terminal(vec![]), SpawnMode::Fresh, None, Some(ep()));
        assert!(
            !s.args.iter().any(|a| a == "--settings"),
            "설정 조각 없으면 --settings 없음: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_no_control_endpoint_no_settings_flag() {
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, None);
        assert!(
            !s.args.iter().any(|a| a == "--settings"),
            "control 없으면 --settings 없음: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_settings_flag_does_not_disturb_the_allowed_tools_tail() {
        let ep = ControlEndpoint {
            settings_file: Some(PathBuf::from("C:/data/mcp/a.settings.json")),
            ..ep_with_grants()
        };
        let s = spec_with_control(&json(vec![]), SpawnMode::Fresh, None, Some(ep));
        let settings_pos = s.args.iter().position(|a| a == "--settings").expect("주입");
        let allowed_pos = s
            .args
            .iter()
            .position(|a| a == "--allowedTools")
            .expect("grant 그룹 주입");
        assert!(
            settings_pos < allowed_pos,
            "--settings 는 --allowedTools 그룹보다 앞: {:?}",
            s.args
        );
        assert_eq!(
            s.args.last().map(|s| s.as_str()),
            Some(format!("PowerShell({CLI_EXE_NAME}:*)").as_str()),
            "grant 그룹이 여전히 맨 끝: {:?}",
            s.args
        );
    }

    // ── ADR-0094: 발신 권한 pre-authorization(`--allowedTools`) ─────────────────────────
    #[test]
    fn grants_to_allowed_tools_mcp_pattern() {
        let out = grants_to_allowed_tools(&[ToolGrant::Mcp {
            server: "engram".to_string(),
            tool: "send_message".to_string(),
        }]);
        assert_eq!(out, vec!["mcp__engram__send_message".to_string()]);
    }

    #[test]
    fn grants_to_allowed_tools_cli_pattern() {
        let out = grants_to_allowed_tools(&[ToolGrant::Cli {
            exe: CLI_EXE_NAME.to_string(),
        }]);
        assert_eq!(
            out,
            vec![
                format!("Bash({CLI_EXE_NAME}:*)"),
                format!("PowerShell({CLI_EXE_NAME}:*)"),
            ]
        );
    }

    #[test]
    fn grants_to_allowed_tools_both_preserve_order() {
        let out = grants_to_allowed_tools(&[
            ToolGrant::Mcp {
                server: "engram".to_string(),
                tool: "send_message".to_string(),
            },
            ToolGrant::Cli {
                exe: CLI_EXE_NAME.to_string(),
            },
        ]);
        assert_eq!(
            out,
            vec![
                "mcp__engram__send_message".to_string(),
                format!("Bash({CLI_EXE_NAME}:*)"),
                format!("PowerShell({CLI_EXE_NAME}:*)"),
            ]
        );
    }

    #[test]
    fn grants_to_allowed_tools_empty_is_empty() {
        assert!(grants_to_allowed_tools(&[]).is_empty());
    }

    #[test]
    fn claude_grants_inject_allowed_tools_flag_terminal() {
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_with_grants()),
        );
        let pos = s
            .args
            .iter()
            .position(|a| a == "--allowedTools")
            .expect("grants 있으면 --allowedTools 주입");
        assert_eq!(
            s.args.get(pos + 1).map(|s| s.as_str()),
            Some("mcp__engram__send_message"),
            "첫 패턴 = MCP 발신 입구(1차 확실 경로): {:?}",
            s.args
        );
        assert_eq!(
            s.args.get(pos + 2).map(|s| s.as_str()),
            Some(format!("Bash({CLI_EXE_NAME}:*)").as_str()),
            "둘째 패턴 = CLI 발신 입구 Bash 모양(bare 이름 colon-star): {:?}",
            s.args
        );
        assert_eq!(
            s.args.get(pos + 3).map(|s| s.as_str()),
            Some(format!("PowerShell({CLI_EXE_NAME}:*)").as_str()),
            "셋째 패턴 = CLI 발신 입구 PowerShell 모양(FIX-4 — 같은 입구의 두 shell 도구): {:?}",
            s.args
        );
    }

    #[test]
    fn claude_grants_inject_allowed_tools_flag_json_mode() {
        let s = spec_with_control(
            &json(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_with_grants()),
        );
        assert!(
            s.args.iter().any(|a| a == "--allowedTools"),
            "json 모드에도 --allowedTools 주입: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_empty_grants_no_allowed_tools_flag() {
        let s = spec_with_control(&terminal(vec![]), SpawnMode::Fresh, None, Some(ep()));
        assert!(
            !s.args.iter().any(|a| a == "--allowedTools"),
            "grants 비면 --allowedTools 없음: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_no_control_endpoint_no_allowed_tools_flag() {
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, None);
        assert!(
            !s.args.iter().any(|a| a == "--allowedTools"),
            "control 없으면 --allowedTools 없음: {:?}",
            s.args
        );
    }

    // ── ADR-0094/0106: 내장 SendMessage 차단(`--disallowedTools SendMessage`, control-scoped) ──
    #[test]
    fn claude_no_control_endpoint_no_disallowed_tools_flag() {
        // ADR-0106
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, None);
        assert!(
            !s.args.iter().any(|a| a == "--disallowedTools"),
            "control 없으면 --disallowedTools 미주입(일반 스폰은 내장 SendMessage 유지): {:?}",
            s.args
        );
    }

    #[test]
    fn claude_disallowed_tools_precedes_allowed_tools_group() {
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_with_grants()),
        );
        let disallowed = s
            .args
            .iter()
            .position(|a| a == "--disallowedTools")
            .expect("control(grants 포함) 있으면 --disallowedTools 주입");
        let allowed = s
            .args
            .iter()
            .position(|a| a == "--allowedTools")
            .expect("grants 있으면 --allowedTools 존재");
        assert_eq!(
            s.args.get(disallowed + 1).map(|s| s.as_str()),
            Some("SendMessage"),
            "disallowedTools 값 = SendMessage 하나뿐: {:?}",
            s.args
        );
        assert!(
            disallowed + 2 == allowed,
            "disallowedTools 그룹(플래그+값 2요소) 바로 뒤에 --allowedTools 가 와야 함(사이 흡수 없음): disallowed={disallowed} allowedTools={allowed} args={:?}",
            s.args
        );
    }

    #[test]
    fn claude_disallowed_tools_present_in_json_mode_too_with_control() {
        let s = spec_with_control(&json(vec![]), SpawnMode::Fresh, None, Some(ep()));
        assert!(
            s.args.iter().any(|a| a == "--disallowedTools"),
            "control 있으면 json 모드에도 --disallowedTools 주입: {:?}",
            s.args
        );
    }

    fn permission_mode_pair_index(args: &[String]) -> Option<usize> {
        args.windows(2)
            .position(|w| w[0] == "--permission-mode" && w[1] == "bypassPermissions")
    }

    #[test]
    fn claude_terminal_injects_auto_permission_mode_pair() {
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, None);
        // ★첫 `--` 플래그 = pair 핀★: 절대 인덱스는 Windows console_command 래퍼(`cmd.exe /c claude`)가
        //   앞에 토큰을 넣어 플랫폼마다 다르다. 대신 "args 의 첫 번째 플래그가 이 pair" 를 단언 —
        //   extra_args 패스스루가 우연히 pair 를 만들어도 base 주입 회귀를 못 가린다(base 가 항상 먼저).
        let first_flag = s.args.iter().position(|a| a.starts_with("--"));
        assert!(
            first_flag.is_some()
                && permission_mode_pair_index(&s.args) == first_flag,
            "터미널 spawn 의 첫 플래그는 pair `--permission-mode bypassPermissions` 여야 함(control 없어도): {:?}",
            s.args
        );
    }

    #[test]
    fn claude_json_injects_auto_permission_mode_pair() {
        let s = spec(&json(vec![]), SpawnMode::Fresh, None);
        let first_flag = s.args.iter().position(|a| a.starts_with("--"));
        assert!(
            first_flag.is_some()
                && permission_mode_pair_index(&s.args) == first_flag,
            "json spawn 의 첫 플래그는 pair `--permission-mode bypassPermissions` 여야 함(control 없어도): {:?}",
            s.args
        );
    }

    #[test]
    fn claude_auto_permission_mode_precedes_allowed_tools_terminal() {
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_with_grants()),
        );
        let pair = permission_mode_pair_index(&s.args).expect("auto 권한 pair 존재");
        let allowed = s
            .args
            .iter()
            .position(|a| a == "--allowedTools")
            .expect("grants 있으면 --allowedTools 존재");
        assert!(
            pair + 1 < allowed,
            "auto 권한 pair 는 --allowedTools 그룹보다 앞이어야 함: pair={pair} allowedTools={allowed} args={:?}",
            s.args
        );
    }

    #[test]
    fn claude_auto_permission_mode_precedes_allowed_tools_json() {
        let s = spec_with_control(
            &json(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_with_grants()),
        );
        let pair = permission_mode_pair_index(&s.args).expect("auto 권한 pair 존재");
        let allowed = s
            .args
            .iter()
            .position(|a| a == "--allowedTools")
            .expect("grants 있으면 --allowedTools 존재");
        assert!(
            pair + 1 < allowed,
            "json 모드: auto 권한 pair 는 --allowedTools 그룹보다 앞이어야 함: pair={pair} allowedTools={allowed} args={:?}",
            s.args
        );
    }

    fn terminal_extra(extra: Vec<&str>) -> AgentCommand {
        terminal(extra)
    }

    fn expected_grant_patterns() -> Vec<String> {
        grants_to_allowed_tools(&ep_with_grants().grants)
    }

    #[test]
    fn claude_allowed_tools_group_is_last_and_exact_terminal() {
        // ★FIX #1 회귀(variadic 흡수 방지)★: extra_args 에 bare positional("Bash")을 넣어 그게
        //   grant 값 run 에 인접-후행하지 않음을 단언.
        let s = spec_with_control(
            &terminal_extra(vec!["Bash", "--debug"]),
            SpawnMode::Fresh,
            None,
            Some(ep_with_grants()),
        );
        let pos = s
            .args
            .iter()
            .position(|a| a == "--allowedTools")
            .expect("grants 있으면 --allowedTools 주입");
        let patterns = expected_grant_patterns();
        assert_eq!(
            &s.args[pos + 1..],
            &patterns[..],
            "allowedTools 뒤 토큰 run 은 정확히 grant 패턴들이고 그 뒤엔 아무것도 없어야 함(변주 흡수 방지): {:?}",
            s.args
        );
        let bash_pos = s
            .args
            .iter()
            .position(|a| a == "Bash")
            .expect("extra_args 의 bare positional 'Bash' 는 args 에 존재");
        assert!(
            bash_pos < pos,
            "bare 'Bash' 는 --allowedTools 앞에 있어야 함(뒤면 variadic 에 흡수돼 blanket Bash grant): bash={bash_pos} allowedTools={pos} args={:?}",
            s.args
        );
    }

    #[test]
    fn claude_allowed_tools_group_is_last_and_exact_json_mode() {
        let s = spec_with_control(
            &AgentCommand::Claude {
                extra_args: vec!["Bash".to_string()],
                output_format: ClaudeOutputFormat::StreamJson,
            },
            SpawnMode::Fresh,
            None,
            Some(ep_with_grants()),
        );
        let pos = s
            .args
            .iter()
            .position(|a| a == "--allowedTools")
            .expect("json 모드 grants 주입");
        let patterns = expected_grant_patterns();
        assert_eq!(
            &s.args[pos + 1..],
            &patterns[..],
            "json 모드: allowedTools 뒤 run 은 정확히 grant 패턴들이고 그 뒤엔 없음: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_space_containing_bash_pattern_stays_single_argv_element() {
        // 운영 grant 는 bare 실행파일 이름이지만 argv 무결성 회귀는 공백 케이스로 검증한다
        //   (grants_to_allowed_tools 는 이름-무관이라 공백 포함 값도 그대로 colon-star 로 감싼다).
        let ep = ControlEndpoint {
            grants: vec![ToolGrant::Cli {
                exe: "C:\\Program Files\\eng\\engram.exe".to_string(),
            }],
            ..ep()
        };
        let s = spec_with_control(&terminal(vec![]), SpawnMode::Fresh, None, Some(ep));
        let pos = s.args.iter().position(|a| a == "--allowedTools").unwrap();
        assert_eq!(
            s.args.get(pos + 1).map(|s| s.as_str()),
            Some("Bash(C:\\Program Files\\eng\\engram.exe:*)"),
            "공백 포함 Bash 패턴은 한 argv 요소로 유지(쪼개지지 않음): {:?}",
            s.args
        );
        assert_eq!(
            s.args.get(pos + 2).map(|s| s.as_str()),
            Some("PowerShell(C:\\Program Files\\eng\\engram.exe:*)"),
            "공백 포함 PowerShell 패턴도 한 argv 요소로 유지: {:?}",
            s.args
        );
        // CLI grant 1개 = 패턴 2개(그래서 pos+3 이 끝).
        assert_eq!(pos + 3, s.args.len(), "패턴이 args 의 마지막: {:?}", s.args);
    }

    #[test]
    fn claude_json_mode_control_endpoint_injects_mcp_config() {
        let s = spec_with_control(&json(vec![]), SpawnMode::Fresh, None, Some(ep()));
        assert!(
            s.args.iter().any(|a| a == "--mcp-config"),
            "json 모드에도 --mcp-config 주입: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_no_session_id_produces_no_flags() {
        let s = spec(&terminal(vec!["--debug"]), SpawnMode::Fresh, None);
        let (p, a) = console_command(
            CLAUDE_PROGRAM,
            vec![
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
                "--debug".to_string(),
            ],
        );
        assert_eq!(s.program, p);
        assert_eq!(s.args, a);
    }

    #[test]
    fn shell_passthrough_via_claude_backend() {
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
        assert!(ClaudeBackend.capabilities(&terminal(vec![])).session.resume);
    }

    #[test]
    fn capabilities_json_mode_resume_is_true() {
        assert!(
            ClaudeBackend.capabilities(&json(vec![])).session.resume,
            "json 모드 claude 도 resume=true(--resume 지원, spike-verified)"
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
            None,
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
        let (p, a) = console_command(
            CLAUDE_PROGRAM,
            vec![
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
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
                // ADR-0106: control=None(비메시징 스폰) → --disallowedTools 미주입.
            ],
        );
        assert_eq!(s.program, p);
        assert_eq!(s.args, a, "json 모드 인자 골든 불일치");
        assert!(s.args.iter().any(|x| x == "--verbose"));
    }

    #[test]
    fn json_mode_resume_uses_resume_flag() {
        let sid = Uuid::new_v4();
        let s = spec(&json(vec![]), SpawnMode::Resume, Some(sid));
        assert!(
            s.args.iter().any(|x| x == "--resume"),
            "json resume 은 --resume 로 가야 함(spike-verified)"
        );
        assert!(
            !s.args.iter().any(|x| x == "--session-id"),
            "json Resume 모드에서 --session-id(fresh) 를 쓰면 안 됨"
        );
        assert!(
            s.args.iter().any(|x| x == &sid.to_string()),
            "resume sid 가 인자에 실려야 함"
        );
    }

    #[test]
    fn terminal_mode_spec_unchanged_regression() {
        let sid = Uuid::new_v4();
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, Some(sid));
        for forbidden in ["-p", "--input-format", "--output-format", "stream-json"] {
            assert!(
                !s.args.iter().any(|x| x == forbidden),
                "터미널 모드에 json 인자 누출: {forbidden}"
            );
        }
    }

    // ── ADR-0049: json 모드 MAX_THINKING_TOKENS 기본 주입(extended thinking 활성화) ──────
    #[test]
    fn json_mode_injects_default_max_thinking_tokens() {
        let s = spec(&json(vec![]), SpawnMode::Fresh, None);
        let vals: Vec<&str> = s
            .env
            .iter()
            .filter(|(k, _)| k == "MAX_THINKING_TOKENS")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(
            vals,
            vec!["8000"],
            "json 모드는 기본 MAX_THINKING_TOKENS=8000 주입"
        );
    }

    #[test]
    fn json_mode_profile_max_thinking_tokens_wins() {
        let env = vec![("MAX_THINKING_TOKENS".to_string(), "1234".to_string())];
        let s = ClaudeBackend.build_spec(
            &json(vec![]),
            SpawnMode::Fresh,
            None,
            PathBuf::from("."),
            env,
            None,
        );
        let vals: Vec<&str> = s
            .env
            .iter()
            .filter(|(k, _)| k == "MAX_THINKING_TOKENS")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(
            vals,
            vec!["1234"],
            "프로필이 준 값이 유일하게 남아야 함(기본 미주입 — 정확히 1개)"
        );
    }

    #[test]
    fn json_mode_profile_lowercase_key_skips_injection() {
        let env = vec![("max_thinking_tokens".to_string(), "1234".to_string())];
        let s = ClaudeBackend.build_spec(
            &json(vec![]),
            SpawnMode::Fresh,
            None,
            PathBuf::from("."),
            env,
            None,
        );
        let vals: Vec<&str> = s
            .env
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("MAX_THINKING_TOKENS"))
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(
            vals,
            vec!["1234"],
            "소문자 프로필 키도 중복 주입 방지 — 정확히 1개(값은 프로필 원본 1234)"
        );
    }

    #[test]
    fn terminal_mode_does_not_inject_max_thinking_tokens() {
        let sid = Uuid::new_v4();
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, Some(sid));
        assert!(
            !s.env.iter().any(|(k, _)| k == "MAX_THINKING_TOKENS"),
            "터미널 모드에 MAX_THINKING_TOKENS 주입 금지"
        );
    }

    // ── ADR-0044/0004: 입력 wrapping(stdin 유저 턴 JSON) 골든 ─────────────────────
    #[test]
    fn wrap_user_turn_exact_line_and_newline_terminated() {
        let id = Uuid::new_v4();
        let bytes = wrap_user_turn("hello", id);
        let expected = format!(
            "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"hello\"}}]}},\"uuid\":\"{id}\"}}\n"
        );
        assert_eq!(bytes, expected.into_bytes());
        assert_eq!(*bytes.last().unwrap(), b'\n', "라인 종단 \\n 필수");
        let v: serde_json::Value =
            serde_json::from_str(String::from_utf8(bytes.clone()).unwrap().trim_end()).unwrap();
        assert_eq!(v["uuid"], id.to_string());
    }

    // ── ADR-0044/0045: 입력-시점 유저 에코 json 헬퍼(user_text_echo_json) ─────────────
    #[test]
    fn user_text_echo_json_matches_decoder_uuid_block_shape() {
        let id = Uuid::new_v4();
        let json = user_text_echo_json("hello", id);
        let expected = format!(r#"{{"type":"text","text":"hello","uuid":"{id}"}}"#);
        assert_eq!(json, expected);
        let json2 = user_text_echo_json("a\"b\nc 한글", id);
        let v: serde_json::Value = serde_json::from_str(&json2).unwrap();
        assert_eq!(v["type"], "text");
        assert_eq!(v["text"], "a\"b\nc 한글");
        assert_eq!(v["uuid"], id.to_string());
    }

    // ── ADR-0044/0045: user-role replay dedup ─────────────────────────────────────────
    #[test]
    fn user_role_text_block_passes_through_with_line_uuid() {
        let line = concat!(
            r#"{"type":"user","message":{"role":"user","content":["#,
            r#"{"type":"text","text":"내가 친 메시지"}"#,
            r#"]},"uuid":"11111111-1111-1111-1111-111111111111","isReplay":true}"#,
            "\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["structured:user"],
            "replay user text 는 억제 아님 — uuid 실어 통과: {ev:?}"
        );
        match &ev[0] {
            OutputEvent::Structured { kind, json } => {
                assert_eq!(kind, "user");
                let v: serde_json::Value = serde_json::from_str(json).unwrap();
                assert_eq!(v["type"], "text");
                assert_eq!(v["text"], "내가 친 메시지");
                assert_eq!(
                    v["uuid"], "11111111-1111-1111-1111-111111111111",
                    "line-level uuid 가 블록 json 에 실려야 함(dedup 키)"
                );
            }
            other => panic!("expected Structured user, got {other:?}"),
        }
    }

    #[test]
    fn user_role_past_text_block_without_uuid_is_preserved_vanish_guard() {
        let line = concat!(
            r#"{"type":"user","message":{"role":"user","content":["#,
            r#"{"type":"text","text":"과거에 친 메시지"}"#,
            "]}}\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["structured:user"],
            "uuid 없는 과거 user text 는 보존돼야 함(vanish 회귀 금지): {ev:?}"
        );
        match &ev[0] {
            OutputEvent::Structured { json, .. } => {
                let v: serde_json::Value = serde_json::from_str(json).unwrap();
                assert_eq!(v["text"], "과거에 친 메시지");
                assert!(
                    v.get("uuid").is_none(),
                    "uuid 없으면 붙이지 않음(원본 보존)"
                );
            }
            other => panic!("expected Structured user, got {other:?}"),
        }
    }

    #[test]
    fn user_role_tool_result_block_still_emitted_regression_guard() {
        let line = concat!(
            r#"{"type":"user","message":{"role":"user","content":["#,
            r#"{"tool_use_id":"toolu_1","type":"tool_result","content":"파일 내용"}"#,
            "]}}\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["structured:user"],
            "user-role tool_result 는 억제 대상 아님 — 보존돼야 함"
        );
        match &ev[0] {
            OutputEvent::Structured { kind, json } => {
                assert_eq!(kind, "user");
                let v: serde_json::Value = serde_json::from_str(json).unwrap();
                assert_eq!(v["type"], "tool_result");
                assert_eq!(v["tool_use_id"], "toolu_1");
                assert_eq!(v["content"], "파일 내용");
            }
            other => panic!("expected Structured, got {other:?}"),
        }
    }

    #[test]
    fn user_role_mixed_blocks_all_preserved_with_line_uuid() {
        let line = concat!(
            r#"{"type":"user","message":{"role":"user","content":["#,
            r#"{"type":"text","text":"echo"},"#,
            r#"{"type":"tool_result","tool_use_id":"t1","content":"r"}"#,
            r#"]},"uuid":"22222222-2222-2222-2222-222222222222","isReplay":true}"#,
            "\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["structured:user", "structured:user"],
            "text·tool_result 둘 다 보존(각각 structured:user)"
        );
        for e in &ev {
            if let OutputEvent::Structured { json, .. } = e {
                let v: serde_json::Value = serde_json::from_str(json).unwrap();
                assert_eq!(v["uuid"], "22222222-2222-2222-2222-222222222222");
            }
        }
    }

    #[test]
    fn tool_fixture_user_tool_result_survives_suppression() {
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
            ],
            "tool_result(user-role) 는 억제 후에도 structured:user 로 보존돼야 함"
        );
    }

    #[test]
    fn wrap_user_turn_escapes_quotes_newlines_unicode() {
        let bytes = wrap_user_turn("a\"b\nc\\d 한글 😀", Uuid::new_v4());
        let line = String::from_utf8(bytes).unwrap();
        assert_eq!(
            line.matches('\n').count(),
            1,
            "내부 개행이 raw 로 새면 안 됨"
        );
        assert!(line.contains("\\\""), "따옴표 escape");
        assert!(line.contains("\\n"), "개행 escape");
        assert!(line.contains("\\\\d"), "백슬래시 escape");
        let v: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(v["message"]["content"][0]["text"], "a\"b\nc\\d 한글 😀");
        assert_eq!(v["type"], "user");
    }

    // ── S15 B2: ClaudeStreamDecoder(stream-json → OutputEvent) ────────────────────
    //
    // 정본 = 실측 fixture(claude 2.1.170 캡처).

    const TEXT_JSONL: &str = include_str!("fixtures/claude_text.jsonl");
    const TOOL_JSONL: &str = include_str!("fixtures/claude_tool.jsonl");
    // ADR-0079: resume seed 용 transcript 픽스처 — 실측 봉투 재현, sidechain 턴 1개 포함.
    const TRANSCRIPT_JSONL: &str = include_str!("fixtures/claude_transcript.jsonl");

    /// ★env 직렬화 락★: `CLAUDE_CONFIG_DIR` 은 프로세스 전역이라 cargo 기본 병렬 실행에서 이 키를
    ///   만지는 테스트끼리 경합한다(한 테스트가 set 한 값을 다른 테스트가 읽음). 그래서 이 키를 건드리는
    ///   테스트는 모두 이 락을 잡고 set→호출→remove 를 원자 구간으로 만든다(poison 무시 = 다른 테스트
    ///   panic 이 이 테스트를 오염시키지 않게).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 반환한 임시 base 디렉토리(`CLAUDE_CONFIG_DIR` 로 넘길 값)는 호출자가 drop 시 정리한다.
    ///   ENV_LOCK 을 잡은 상태에서만 호출할 것(경로 파생이 env 에 의존).
    fn write_temp_transcript(cwd: &std::path::Path, sid: Uuid, content: &[u8]) -> PathBuf {
        // 유니크 base(pid+sid) — 병렬이라도 파일 경로가 겹치지 않게(락은 env 파생 구간만 보호).
        let base = std::env::temp_dir().join(format!(
            "engram-transcript-test-{}-{sid}",
            std::process::id()
        ));
        std::env::set_var("CLAUDE_CONFIG_DIR", &base);
        let path = transcript_path(cwd, sid).expect("temp transcript path");
        std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir temp transcript dir");
        std::fs::write(&path, content).expect("write temp transcript");
        base
    }

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

        match &events[0] {
            OutputEvent::TextDelta {
                text, message_id, ..
            } => {
                assert_eq!(text, "hello");
                assert_eq!(message_id.as_deref(), Some("msg_01QDurZCCdyuXSWuV5NwWr6c"));
            }
            other => panic!("expected TextDelta, got {other:?}"),
        }
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
        //  10 assistant[tool_use Read]      → tool:Read  (9 와 같은 msg id)
        //  11 user[tool_result]             → structured:user
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
        let line = r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"안녕 😀 world"}]}}"#;
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\n');

        let whole = decode_all(&bytes);
        let whole_text = match &whole[0] {
            OutputEvent::TextDelta { text, .. } => text.clone(),
            other => panic!("expected TextDelta, got {other:?}"),
        };
        assert_eq!(whole_text, "안녕 😀 world");

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
        let mut d = ClaudeStreamDecoder::new();
        let line = br#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"tail"}]}}"#;
        assert!(d.decode(line).is_empty(), "개행 전엔 아무것도 안 나온다");
        let ev = d.flush();
        assert_eq!(tags(&ev), vec!["text"]);
    }

    #[test]
    fn result_without_usage_emits_only_done() {
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

        let tail_then_newline = b"garbage-tail-continues{\"type\":\"assistant\"}\n";
        let ev_tail = d.decode(tail_then_newline);
        assert!(
            ev_tail.is_empty(),
            "오염 라인 꼬리는 개행까지 통째 폐기 — 이벤트 0개: {ev_tail:?}"
        );

        let line = b"{\"type\":\"assistant\",\"message\":{\"id\":\"m1\",\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}}\n";
        let ev2 = d.decode(line);
        assert_eq!(tags(&ev2), vec!["text"], "오버플로 후 정상 라인 복구");
    }

    #[test]
    fn buffer_overflow_tail_with_valid_json_fragment_does_not_forge_events() {
        // FIX-A (2): 오버플로 라인 꼬리에 섞인 valid JSON 조각도 다음 개행까지 통째 폐기(가짜 이벤트 금지).
        let mut d = ClaudeStreamDecoder::new();

        let huge = vec![b'x'; MAX_BUFFER_BYTES + 1];
        let ev = d.decode(&huge);
        assert_eq!(tags(&ev), vec!["error"], "오버플로 → Error");

        let tail = concat!(
            "still-part-of-poisoned-line{\"type\":\"result\",\"subtype\":\"success\"}\n",
            "{\"type\":\"assistant\",\"message\":{\"id\":\"m2\",\"content\":[{\"type\":\"text\",\"text\":\"recovered\"}]}}\n",
        );
        let ev2 = d.decode(tail.as_bytes());
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

        assert!(d.decode(b"tail-part-1").is_empty());
        assert!(d.decode(b"tail-part-2{\"type\":\"result\"}").is_empty());
        let ev = d.decode(b"final-tail\n{\"type\":\"result\",\"subtype\":\"success\"}\n");
        assert_eq!(tags(&ev), vec!["done"], "resync 종료 후 정상 result 복구");
    }

    #[test]
    fn multiple_blocks_in_one_line_expand_in_order() {
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
            r#"{"type":"tool_use","id":"t1","input":{"cmd":"ls"}}"#,
            "]}}\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["structured:tool_use"],
            "name 없는 tool_use → 빈 ToolCall 금지, Structured 보존"
        );
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
            r#"{"type":"tool_use","id":"t1","name":123,"input":{}}"#,
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
            r#"{"type":"text"},"#,
            r#"{"type":"text","text":"kept"}"#,
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
        // FIX-E 회귀: 유저 Esc 정상 중단 턴(subtype:"interrupted").
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
        let line = concat!(
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"#,
            r#""usage":{"input_tokens":10,"output_tokens":2}}"#,
            "\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(tags(&ev), vec!["usage", "error", "done"]);
    }

    // ── ADR-0079: `.jsonl` transcript → 과거 이벤트(순수 함수 parse_transcript_events) ────────

    #[test]
    fn transcript_maps_conversation_turns_and_skips_meta_and_sidechain() {
        // 픽스처 라인 ↔ 기대 시퀀스:
        //     user "첫 질문"                   → structured:user
        //     assistant thinking               → structured:thinking
        //     assistant tool_use Read          → tool:Read
        //     user tool_result                 → structured:user
        //     assistant [text + tool_use Write]→ text, tool:Write
        //     (sidechain assistant text        → 제외)
        //     assistant text "최종 답변"        → text
        //     result(usage) success            → usage, done
        let events = parse_transcript_events(TRANSCRIPT_JSONL);
        assert_eq!(
            tags(&events),
            vec![
                "structured:user",
                "structured:thinking",
                "tool:Read",
                "structured:user",
                "text",
                "tool:Write",
                "text",
                "usage",
                "done",
            ],
            "대화 턴만 매핑 + 다중블록 턴 둘 다 매핑 + 메타·sidechain 배제: {events:?}"
        );

        let mb_text_idx = events
            .iter()
            .position(
                |e| matches!(e, OutputEvent::TextDelta { text, .. } if text == "확인했습니다"),
            )
            .expect("다중블록 턴의 text 블록이 매핑돼야 함");
        match &events[mb_text_idx + 1] {
            OutputEvent::ToolCall { name, id, .. } => {
                assert_eq!(
                    name, "Write",
                    "같은 메시지의 2번째 블록이 tool_use Write 로 이어져야 함"
                );
                assert_eq!(id.as_deref(), Some("toolu_2"));
            }
            other => panic!("다중블록 턴의 2번째 블록이 ToolCall 이어야 함, got {other:?}"),
        }

        let tool = events
            .iter()
            .find(|e| matches!(e, OutputEvent::ToolCall { .. }))
            .unwrap();
        match tool {
            OutputEvent::ToolCall {
                name,
                id,
                args_json,
                ..
            } => {
                assert_eq!(name, "Read");
                assert_eq!(id.as_deref(), Some("toolu_1"));
                let v: serde_json::Value = serde_json::from_str(args_json).unwrap();
                assert_eq!(v["file_path"], "C:\\Users\\X\\proj\\a.txt");
            }
            _ => unreachable!(),
        }

        match &events[0] {
            OutputEvent::Structured { kind, json } => {
                assert_eq!(kind, "user");
                let v: serde_json::Value = serde_json::from_str(json).unwrap();
                assert_eq!(v["text"], "첫 질문");
            }
            other => panic!("expected Structured user, got {other:?}"),
        }
    }

    #[test]
    fn transcript_empty_or_meta_only_yields_no_events() {
        assert!(parse_transcript_events("").is_empty());
        let meta_only = concat!(
            "{\"type\":\"summary\",\"summary\":\"x\"}\n",
            "{\"type\":\"file-history-snapshot\",\"messageId\":\"m\",\"snapshot\":{}}\n",
            "\n",
        );
        assert!(
            parse_transcript_events(meta_only).is_empty(),
            "메타 전용 transcript 는 이벤트 0개"
        );
    }

    #[test]
    fn transcript_sidechain_line_is_filtered_even_with_valid_content() {
        let line = concat!(
            r#"{"isSidechain":true,"type":"assistant","message":{"id":"m","role":"assistant",""#,
            r#"content":[{"type":"text","text":"sub"}]},"uuid":"x"}"#,
            "\n",
        );
        assert!(
            parse_transcript_events(line).is_empty(),
            "sidechain 턴은 제외(sub-agent 출력 복원 안 함)"
        );
    }

    #[test]
    fn project_slug_matches_claude_encoding() {
        use std::path::Path;
        assert_eq!(
            project_slug(Path::new(
                r"C:\Users\X\AppData\Local\Temp\engram-resume-test"
            )),
            "C--Users-X-AppData-Local-Temp-engram-resume-test"
        );
        assert_eq!(project_slug(Path::new(r"C:\")), "C--");
        assert_eq!(
            project_slug(Path::new(r"I:\Engram_Workspace\a")),
            "I--Engram-Workspace-a"
        );
    }

    #[test]
    fn transcript_path_uses_projects_slug_and_sid() {
        let sid = Uuid::parse_str("d75b7f40-a13a-4cf3-b872-e4d5ba2cec55").unwrap();
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("CLAUDE_CONFIG_DIR", r"C:\claude-cfg");
        let p = transcript_path(std::path::Path::new(r"C:\proj\a"), sid).unwrap();
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let s = p.to_string_lossy().replace('/', "\\");
        assert!(s.contains("projects"), "projects 디렉토리 경유: {s}");
        assert!(s.contains("C--proj-a"), "cwd 슬러그: {s}");
        assert!(
            s.ends_with(&format!("{sid}.jsonl")),
            "파일명 = <sid>.jsonl: {s}"
        );
    }

    #[test]
    fn read_transcript_events_missing_file_is_empty() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("CLAUDE_CONFIG_DIR", r"C:\nonexistent-claude-cfg-xyz");
        let ev =
            read_transcript_events(std::path::Path::new(r"C:\no-such-proj-xyz"), Uuid::new_v4());
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        assert!(ev.is_empty(), "파일 없으면 빈 Vec(seed 안 함)");
    }

    #[test]
    fn read_transcript_events_small_file_reads_whole() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cwd = std::path::Path::new(r"C:\proj\small");
        let sid = Uuid::new_v4();
        let base = write_temp_transcript(cwd, sid, TRANSCRIPT_JSONL.as_bytes());
        let ev = read_transcript_events(cwd, sid);
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(
            tags(&ev),
            tags(&parse_transcript_events(TRANSCRIPT_JSONL)),
            "작은 파일은 전량 읽어 순수 파싱과 동일(부분 라인 폐기 없음)"
        );
    }

    #[test]
    fn read_transcript_events_empty_file_is_empty() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cwd = std::path::Path::new(r"C:\proj\empty");
        let sid = Uuid::new_v4();
        let base = write_temp_transcript(cwd, sid, b"");
        let ev = read_transcript_events(cwd, sid);
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&base);
        assert!(ev.is_empty(), "빈 파일 → 빈 Vec");
    }

    /// ★테스트가 실제로 폐기를 증명하는 구성(cross-family review 2026-07-13 FIX B)★: seek 지점(= len-4MB)이
    ///   **함정 라인 안의 내부 유효 JSON 오브젝트 시작 바이트**에 정확히 떨어지게 맞춘다. 그래서 seek 후 첫 `\n`
    ///   전까지의 앞조각(= drop 대상)은 **그 자체로 온전히 파싱되는 유효 대화 라인**(phantom "text" 이벤트를
    ///   합성함)이다. 이렇게 하면 폐기가 없으면 phantom 이 나타나고, 폐기가 있으면 사라진다 — drop 을 빼면
    ///   테스트가 실패한다(옛 구성은 앞조각이 비-JSON 이라 drop 유무와 무관하게 스킵돼 아무것도 증명 못 했음).
    ///   ★paired 반증★: 아래에서 drop 없이 파싱(`parse_transcript_events(leading)`)하면 phantom 이 실제로
    ///   생긴다는 것을 함께 단언해, "앞조각이 유효 라인"이라는 전제가 참임을 고정한다.
    #[test]
    fn read_transcript_events_tail_drops_partial_leading_line_no_phantom() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cwd = std::path::Path::new(r"C:\proj\big");
        let sid = Uuid::new_v4();

        // ★함정 라인★ = [접두 쓰레기][내부 유효 JSON 라인]. 접두 쓰레기는 seek 지점 앞이라 read buffer 에
        //   들어오지 않는다.
        let junk_prefix =
            br#"{"type":"summary","summary":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}garbage"#;
        let inner_valid = br#"{"type":"assistant","message":{"id":"phantom","content":[{"type":"text","text":"phantom"}]}}"#;
        let real = r#"{"isSidechain":false,"type":"assistant","message":{"id":"real","content":[{"type":"text","text":"진짜 답변"}]}}"#.as_bytes();

        // seek 지점은 EOF 로부터 고정 거리(4MB)다. 그래서 "inner_valid 시작 ~ EOF" 바이트 수를 정확히 4MB 로
        //   맞추면 seek 지점 = inner_valid 시작 바이트가 된다. inner_valid 시작~EOF = inner + \n + real + \n + trail.
        let inner_start_to_eof = TRANSCRIPT_TAIL_BYTES as usize;
        let trail_len = inner_start_to_eof - (inner_valid.len() + 1 + real.len() + 1);
        let mut content: Vec<u8> =
            Vec::with_capacity(inner_start_to_eof + junk_prefix.len() + 4096);
        content.extend_from_slice(junk_prefix);
        content.extend_from_slice(inner_valid);
        content.push(b'\n');
        content.extend_from_slice(real);
        content.push(b'\n');
        let pad_line =
            b"{\"type\":\"file-history-snapshot\",\"messageId\":\"pad\",\"snapshot\":{}}\n";
        let trail_start = content.len();
        while content.len() - trail_start < trail_len {
            content.extend_from_slice(pad_line);
        }
        content.truncate(trail_start + trail_len);

        let leading = String::from_utf8(inner_valid.to_vec()).unwrap();
        assert_eq!(
            tags(&parse_transcript_events(&leading)),
            vec!["text"],
            "전제: 앞조각(drop 대상)은 그 자체로 phantom text 를 합성하는 유효 라인이어야 한다"
        );

        let base = write_temp_transcript(cwd, sid, &content);
        let ev = read_transcript_events(cwd, sid);
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(
            tags(&ev),
            vec!["text"],
            "부분 첫 라인 폐기 → phantom 없이 온전 라인만: {ev:?}"
        );
        match &ev[0] {
            OutputEvent::TextDelta { text, .. } => assert_eq!(text, "진짜 답변"),
            other => panic!("expected TextDelta 진짜 답변, got {other:?}"),
        }
    }

    #[test]
    fn read_transcript_events_tail_utf8_split_is_safe() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cwd = std::path::Path::new(r"C:\proj\utf8");
        let sid = Uuid::new_v4();

        // 패딩을 한글(3바이트/문자) 라인으로 채워 seek 지점(len-4MB)이 멀티바이트 문자 경계를 비껴가게 한다.
        let pad_line = "{\"type\":\"summary\",\"summary\":\"가나다라마\"}\n".as_bytes();
        let mut content: Vec<u8> = Vec::with_capacity((TRANSCRIPT_TAIL_BYTES as usize) + 4096);
        while (content.len() as u64) < TRANSCRIPT_TAIL_BYTES + 1 {
            content.extend_from_slice(pad_line);
        }
        content.push(b'x'); // 경계를 1바이트 밀어 문자 중간 절단 유도.
        content.extend_from_slice(pad_line); // 잘린 첫 라인이 될 조각.
        content.extend_from_slice(
            r#"{"isSidechain":false,"type":"assistant","message":{"id":"real","content":[{"type":"text","text":"안녕"}]}}"#.as_bytes(),
        );
        content.push(b'\n');

        let base = write_temp_transcript(cwd, sid, &content);
        let ev = read_transcript_events(cwd, sid);
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(
            tags(&ev),
            vec!["text"],
            "UTF-8 절단 경계에서도 온전 라인만: {ev:?}"
        );
    }

    #[test]
    fn read_transcript_events_exact_boundary_no_seek() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cwd = std::path::Path::new(r"C:\proj\boundary");
        let sid = Uuid::new_v4();

        let first = r#"{"isSidechain":false,"type":"assistant","message":{"id":"first","content":[{"type":"text","text":"경계"}]}}"#;
        let mut content: Vec<u8> = Vec::with_capacity(TRANSCRIPT_TAIL_BYTES as usize);
        content.extend_from_slice(first.as_bytes());
        content.push(b'\n');
        let pad = b"{\"type\":\"summary\",\"summary\":\"p\"}\n";
        while (content.len() as u64) < TRANSCRIPT_TAIL_BYTES {
            content.extend_from_slice(pad);
        }
        content.truncate(TRANSCRIPT_TAIL_BYTES as usize);
        assert_eq!(content.len() as u64, TRANSCRIPT_TAIL_BYTES);

        let base = write_temp_transcript(cwd, sid, &content);
        let ev = read_transcript_events(cwd, sid);
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(
            ev.first()
                .map(|e| matches!(e, OutputEvent::TextDelta { text, .. } if text == "경계")),
            Some(true),
            "정확히 상한이면 seek 없이 첫 라인부터 파싱: {ev:?}"
        );
    }

    // ── ADR-0084: 재활성화(Resume) 시 `--resume <sid>` 조립 실증(dispatch 레벨) ──────────────────
    //
    // ADR-0083 회귀 ③의 약한 Shell 테스트 보강: 재활성화가 통제 sid 를 `--resume` 인자로 실제 흘리는지는
    // **공개 dispatch `build_command_spec`** 를 통해 backend 계약으로 직접 단언한다(실 claude 프로세스
    // 없음). manager.spawn_agent(Resume) 이 이 dispatch 를 부르므로, 여기서 --resume<sid> 를 검증하면
    // 재활성화 respawn 이 이어받기 sid 를 무손실 전달함이 증명된다(ADR-0008).
    #[test]
    fn build_command_spec_resume_emits_resume_flag_with_sid() {
        use crate::agent::backend::build_command_spec;

        let sid = Uuid::new_v4();
        let spec = build_command_spec(
            &terminal(vec![]),
            SpawnMode::Resume,
            Some(sid),
            PathBuf::from("."),
            vec![],
            None,
        );

        let pos = spec
            .args
            .iter()
            .position(|x| x == "--resume")
            .expect("Resume 모드 dispatch 는 --resume 를 조립해야 함");
        assert_eq!(
            spec.args.get(pos + 1).map(String::as_str),
            Some(sid.to_string().as_str()),
            "--resume 바로 뒤에 통제 sid(uuid)가 실려야 함(ADR-0008 무손실 이어받기)"
        );
        // Resume 모드는 fresh 전용 --session-id 를 쓰면 안 된다(sid 충돌 회피 — ADR-0076).
        assert!(
            !spec.args.iter().any(|x| x == "--session-id"),
            "Resume 모드에서 --session-id(fresh)가 누출되면 안 됨"
        );
    }
}
