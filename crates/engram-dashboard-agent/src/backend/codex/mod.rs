//! CodexBackend — codex CLI 전용 CommandSpec 산출.
//!
//! ★이 폴더가 세우는 규칙 = codex 지식은 여기 안에만 산다(ADR-0004)★. 근거·게이트·게이트가
//! 못 보는 것의 정본은 `backend/claude/mod.rs` 헤더이고 여기 되풀어 적지 않는다 — 이름만 바꿔
//! 읽는다. 밖으로 나가는 표면은 [`crate::backend::AgentBackend`] 구현 하나뿐이다.
//!
//! ★여기 적힌 codex 사실은 실측이다(codex-cli 0.153.4, 이 PC, 인증됨 — 2026-09-08 재확인)★.
//! ★그 뒤에 잰 것은 **자기 자리에 버전을 달고 있다**★ — 이 한 줄이 파일 전체를 한 버전으로 묶는다고
//! 읽지 말 것. 상류가 스스로 업데이트하므로 잰 시점이 항목마다 갈린다.
//! `tests/backend_contract.rs` 가 이 파일의 `build_spec` 이 낸 argv 를 **그대로 띄우므로**, 여기서 인자를
//! 바꾸면 그 레인이 바뀐 argv 로 실 codex 를 겪는다 — ★단 그 레인은 `#[ignore]` 라 부를 때만 돈다(CI 아님)★.
//!
//! ★시험대가 **다시 재지 않는 것** — 「전부 실측」이라 적던 옛 문장이 거짓이었다(리뷰 적출 2026-09-08)★:
//!   1. `workspace-write`·`on-request` 정책 **아래의 모델 동작** — 시험대는 그 argv 로 뜨는 것과 컴포저
//!      기립까지만 잰다(재려면 쓰기 권한을 가진 에이전트를 자동 레인에서 실제로 돌려야 한다).
//!   2. 우리가 안 쓰는 인자(`-m`) — 이 파일 주석에만 있고 재는 곳이 없다.
//!      ★MCP `-c mcp_servers.…` 오버라이드는 이제 **우리가 쓴다**★ — 그리고 시험대가 아니라 손으로
//!      쟀다(2026-09-19 · 0.155.0 · 버리는 `CODEX_HOME`): `--strict-config` 가 그 키를 받아들였고
//!      (가짜 키 둘은 `unknown configuration field` 로 죽었다), app-server 의 `mcpServerStatus/list` 가
//!      `engram` 을 그 서버의 발신·조회 툴 둘과 함께 돌려줬으며, 스텁 서버가
//!      `Authorization: Bearer <ENGRAM_TOKEN 값>` 을 받았다. `cmd.exe /c` 를 지난 갈래도 같았다.
//!      ★그날 화면에 찍힌 툴 이름은 `send_message`·`messages` 였다 — 오늘 그 이름은 없다★:
//!      `1b84045` 가 에이전트 내장 도구와의 충돌을 끊으려 `eg_send`·`eg_messages` 로 개명했다. 잰 것은
//!      그대로 유효하고(개명은 이름뿐이다) 바뀐 것은 부를 이름뿐이다 — 지금 다시 재면 그 목록에
//!      `eg_send`·`eg_messages` 가 온다.
//!   2-1. ★`codex resume <id>` 는 이제 **우리가 쓴다** — 그런데도 시험대가 실물로 재지 않는다★:
//!      [`production_spec`](../../../tests/backend_contract.rs) 이 `SpawnMode::Fresh` 로만 argv 를 뽑아
//!      이어받기 갈래가 그 레인에 애초에 안 실린다. 자동으로 재려면 그 파일에 살아 있는 스레드 id 를
//!      공급할 길이 먼저 필요하다.
//!      ★단 **동작 자체는 손으로 실측됐다(2026-09-19 · 0.155.0)** — 「기립하는 것까지는 아무도 안 본다」로
//!        적혀 있던 옛 문장은 더 이상 참이 아니다★: 그 argv 로 실제로 떴고, **세션 id 가 보존된다**. 새
//!        세션이 id S 를 `source:"startup"` 으로 신고하고, 이어받으면 **같은 S** 가 `source:"resume"` 으로
//!        다시 오며(반복 이어받기에도 안정) 기록은 S 의 원래 rollout 파일에 이어 붙는다. 그래서 이어받은
//!        세션의 회수는 덮어쓰기든 아니든 **같은 값에 착지하고**, 낡은 id 로 흘러가는 갈래가 없다.
//!        ★단 이것은 `source` 칸을 내던 **훅 채널**의 관측이고, 그 채널은 걷혔다(ADR-0216)★. 오늘 회수는
//!        TUI 상태줄을 읽고 그 자리엔 `source` 칸이 없다 — 그래서 **ADR-0216 은 재개 시 id 거동을 미측정
//!        으로 적어 두고 그 위에서 덮어쓰기를 골랐다.** 둘이 어긋나 보이면 이 문단이 옛 채널의 기록이다.
//!   3. 아래 `build_spec` 의 `%VAR%` 한계(그 자리 주석이 정본).
//!
//! capability 선언이 그 표와 어긋나면 시험대의 **비-`#[ignore]`** 항목이 빨개진다.
//!
//! tauri import 0.

pub(crate) mod decoder;
pub(crate) mod protocol;
mod thread_id;
pub(crate) mod transport;

use std::path::PathBuf;

use uuid::Uuid;

use self::decoder::CodexAppServerDecoder;
use self::protocol::{
    AskForApproval, SandboxMode, ThreadOpen, ThreadResumeParams, ThreadStartParams,
};
use self::transport::CodexAppServerTransport;
use crate::backend::{
    console_command, inject_cli_entrance, AgentBackend, InputEncoder, SessionIdSink, SpawnParts,
    TransportShape, TurnClassifier,
};
use crate::failure::AgentFailureKind;
use crate::profile::{AgentCommand, AgentOutputFormat, SpawnMode};
use crate::transport::pty::PtyTransport;
use crate::transport::{AgentTransport, LinkSink, OutputDecoder};
use crate::turn::TurnSignal;
use crate::types::{
    BackendCaps, CommandSpec, ControlEndpoint, ModelCaps, OutputEvent, PtyError, SessionCaps,
    MCP_SERVER_NAME, TOKEN_ENV,
};

/// codex 를 대화형 TUI 가 아니라 **상주 JSON 서버**로 띄우나 = 이 폴더 안의 네 축(통로 모양·통로 실물·
/// 입력 인코딩·출력 decoder)이 함께 갈리는 지점.
///
/// ★이 술어를 `profile` 로 올리지 말 것(ADR-0004)★: 거기 있으면 codex 의 출력 형식 축이 공용 층의 통로
///   선택을 직접 굴린다. 밖으로 나가는 것은 중립 축의 값뿐이다.
fn is_app_server(command: &AgentCommand) -> bool {
    matches!(
        command,
        AgentCommand::Codex {
            output_format: AgentOutputFormat::StreamJson,
            ..
        }
    )
}

/// 이 spawn 의 **핸드셰이크 둘째 요청**을 고른다 — `resume_session_id` 가 있으면 그 스레드를 이어받고,
/// 없으면 새 스레드를 연다. 부재 = 이어받을 것이 없다(저장된 값이 없거나 Fresh 로 띄운다).
///
/// ★정책 셋(작업 폴더·승인·샌드박스)을 두 갈래에 **똑같이** 싣는다★: 이어받기에서 빼면 codex 가 그
///   스레드를 만들 때 저장해 둔 값으로 돈다 — 프로필의 작업 폴더가 그 사이 바뀌었어도 이어받은 세션만
///   조용히 옛 폴더를 워크스페이스로 믿는다. 그 어긋남은 화면에 아무 표시도 남기지 않는다.
/// ★**미검인 것의 범위를 정확히 적는다 — 「다른 cwd 를 받아 주나」는 이미 실측됐다**★
///   (`docs/reference/backend-capabilities.md` §1 「cwd 가 다르면」): **이어진다.** 스레드 정체성은
///   워크스페이스가 아니라 `CODEX_HOME` 단위라 cwd 가 달라도 거절되지 않는다. 그러니 「거절하면 이어받기
///   실패로 화면에 오른다」는 있지도 않은 갈래를 대비한 문장이었다.
///   ★진짜 미검은 **우리가 보낸 cwd 가 기록된 값을 덮나**다★ — 같은 실측이 「응답의 `cwd` 는 **스레드에
///   기록된 원래 cwd**」라고 적는다. 즉 우리 값이 무시될 가능성이 있고, 그러면 바로 위 문단이 막으려던
///   그 어긋남(이어받은 세션만 옛 폴더를 믿는다)이 **이 인자를 실어 보내도 그대로 남는다.** 재려면 서로
///   다른 두 폴더로 같은 스레드를 이어받아 턴이 실제로 어느 쪽에서 도는지 봐야 한다 — 안 재 봤다.
///   ★그래도 인자는 계속 싣는다★: 무시되면 손해가 없고, 반영되면 의도한 값이 선다.
/// ★`excludeTurns` 를 켜는 것은 결정이다★ — 우리는 `thread.turns` 를 한 칸도 읽지 않는데(응답 타입에
///   그 칸이 없다), 채워 받으면 긴 대화 하나가 통로의 줄 상한(`MAX_LINE_BYTES`)을 넘겨 **그 줄만
///   버려지고** 핸드셰이크가 시한까지 오지 않을 답을 기다린다.
///   ★**그 결말이 거절과 구별되지 않는다는 것이 이 항목의 요점이다**★ — 상대는 살아 있고 stdout 으로
///   답을 보냈는데 우리가 그 줄을 버렸으므로, 우리 쪽에서 보이는 것은 「핸드셰이크가 시한으로 실패했고
///   자식은 멀쩡하다」 하나다. 그것은 `thread/resume` 이 JSON-RPC 오류로 거절당한 경우와 **같은 모양**
///   이고, 그 모양이 통로의 stdin 을 닫지 않으면 자식·리더·라이터가 통째로 붙들려 아무도 거두지 못하는
///   wedge 가 된다(그래서 그 닫기가 갈래를 가리지 않는다 — `transport.rs` 의 그 자리).
///   ★즉 상류가 `excludeTurns` 를 무시하기 시작하면 이 인자는 아무 것도 못 막고, 남는 방어는 그 닫기
///   하나뿐이다★ — 이 인자를 「막아 뒀다」로 읽지 말 것. 상류가 실제로 존중하는지는 미검이다.
/// ★`developer_instructions` 는 **여는 갈래에만** 실린다★ — [`ThreadResumeParams`] 에는 그 칸이 없고,
///   없는 채로 두는 것이 결정이다(그 타입의 doc 이 사유의 정본). 그래서 이어받은 app-server 스레드는
///   프라이밍을 못 받는다 — 터미널 모드는 argv 라 이어받기에도 그대로 실리므로 갭은 이 한 갈래뿐이다.
/// ★받은 값을 **손대지 않고** 싣는다★ — JSON 본문에는 명령줄 상한도 `%` 치환도 줄바꿈 절단도 없다.
///   터미널 갈래의 변환([`developer_instructions_override`])을 여기로 가져오면 아무 위험도 막지 못한
///   채 에이전트가 읽는 문서만 망가진다.
// ADR-0185
// ADR-0215
fn thread_open(
    spec: &CommandSpec,
    resume_session_id: Option<Uuid>,
    developer_instructions: Option<String>,
) -> ThreadOpen {
    let cwd = Some(spec.cwd.to_string_lossy().into_owned());
    let approval_policy = Some(AskForApproval::OnRequest);
    let sandbox = Some(SandboxMode::WorkspaceWrite);
    match resume_session_id {
        Some(thread_id) => ThreadOpen::Resume(ThreadResumeParams {
            thread_id: thread_id.to_string(),
            cwd,
            approval_policy,
            sandbox,
            exclude_turns: Some(true),
        }),
        None => ThreadOpen::Start(ThreadStartParams {
            cwd,
            approval_policy,
            sandbox,
            developer_instructions,
        }),
    }
}

/// PATH 로 해석되는 이름 그대로 띄운다(사용자 결정 2026-09-07 · TRD §6-H).
///
/// ★실 바이너리를 찾아 직접 띄우지 않는 이유★: Windows 에서 PATH 의 `codex` 는
/// `codex.cmd → node → codex.exe` 사슬이고 그 끝의 실 바이너리는 **버전이 박힌 `node_modules` 벤더
/// 경로** 아래 있다(실측). codex 는 스스로 자동 업데이트하므로 그 경로는 우리가 모르는 시점에 바뀐다 —
/// 하드코딩하면 업데이트 한 번에 죽고, 탐색·폴백을 짜면 그 사슬을 우리가 재구현하게 된다.
/// ★한 겹 더 깊은 shim 이 kill 인과를 바꾸지 않는다 — 단 그 보장의 범위를 정확히 적는다★:
/// Job 에 **편입된 뒤로는** breakaway 가 막혀 있어(`BREAKAWAY_OK`·`SILENT_BREAKAWAY_OK` 둘 다 안 켠다)
/// 그 아래 생기는 손자·증손자가 Job 을 벗어날 수 없고, `TerminateJobObject` 가 트리를 통째로 끝낸다
/// (ADR-0001 의 2 동사).
/// ★편입은 spawn **뒤**라 그 사이 창은 그 보장 밖이다★ — 그 창에서 태어난 자손은 Job 에 안 들어간다.
/// 이 저장소의 통로 셋이 전부 같은 모양이고(`CREATE_SUSPENDED` 는 한 줄도 없다) 기존 teardown 테스트는
/// 정착 상태만 재므로, 이 창은 **재 본 적이 없다**. 고치는 것은 세 통로를 함께 건드리는 별건이다.
/// 「그 스레드의 기록이 없다」를 뜻하는 상대 문구(소문자 비교). ★실측된 응답에서 그대로 딴다★ —
/// `-32600` + `no rollout found for thread id`(`docs/reference/backend-capabilities.md` §1).
/// ★코드가 아니라 이 문구가 판정 기준인 이유★: 같은 코드가 설정 오류·중복 `initialize`·하위 스레드
/// 이어받기에도 온다. 코드로 가르면 멀쩡한 손잡이가 무관한 실패에서 「이어받을 대화 없음」 도장을 받는다.
const NO_ROLLOUT_MARKER: &str = "no rollout found";

const CODEX_PROGRAM: &str = "codex";

/// codex 가 작업 폴더를 받는 플래그. ★프로세스 cwd 와 별개다★ — `CommandSpec.cwd` 는 우리가 프로세스를
/// 어디서 띄우나이고, 이 값은 codex 가 **어느 폴더를 워크스페이스로 신뢰·편집하나**다. 둘을 같은 값으로
/// 주지만 같은 칸이 아니다.
const CD_FLAG: &str = "--cd";

/// 샌드박스 모드 — 모델이 내는 명령을 **작업 폴더 안에서는 쓰기까지** 허용한다.
const SANDBOX_FLAG: &str = "-s";
const SANDBOX_WORKSPACE_WRITE: &str = "workspace-write";

/// 승인 모드 — 샌드박스 밖으로 나가는 동작만 사람에게 묻는다.
const APPROVAL_FLAG: &str = "-a";
const APPROVAL_ON_REQUEST: &str = "on-request";

/// 저장된 세션을 이어받는 하위 명령. ★플래그가 아니다 — `--resume`·`--session-id` 는 존재하지 않는다★
/// (실측). 받는 것은 **위치 인자**이고 그 자리는 `codex resume [OPTIONS] [SESSION_ID] [PROMPT]` 의 첫
/// 위치다(실측 0.155.0 `codex resume --help`).
///
/// ★정책 셋(`--cd`·`-s`·`-a`)을 이 하위 명령 **뒤**에 이어도 파싱된다★ — 그 셋은 `resume` 자신의
///   옵션으로도 선언돼 있다(같은 실측). 위치 인자와 옵션의 앞뒤 순서는 둘 다 통과하는 것을 봤다
///   (`codex resume <id> --cd . -s <잘못된값>` 과 옵션을 앞에 둔 짝 모두 **샌드박스 값 오류**로 죽었다
///   = 그 지점까지 파싱이 갔다는 뜻). 그래서 id 를 하위 명령 바로 뒤에 붙여 `codex resume <id>` 라는
///   한 덩어리가 읽히게 둔다.
/// ★둘째 위치 인자가 `[PROMPT]` 다★ — 그래서 아래 `extra_args` 에 **플래그가 아닌 맨 낱말**이 들어오면
///   그것이 첫 프롬프트로 먹힌다. Fresh 갈래(`codex [OPTIONS] [PROMPT]`)도 같은 성질이라 이어받기가
///   새로 들인 위험이 아니다 — 걸러 내지 않는 사유는 아래 패스스루 주석이 정본이다.
const RESUME_SUBCOMMAND: &str = "resume";

/// codex 설정을 명령줄에서 덮어쓰는 플래그. ★사용자 기기에 파일을 하나도 안 만들고 설정을 얻는 수단이 이것 하나다★
/// — 사용자 홈(`$CODEX_HOME/config.toml`)에도, 래퍼 스크립트에도 우리는 한 글자도 쓰지 않는다.
const CONFIG_OVERRIDE_FLAG: &str = "-c";

/// TUI 상태줄 항목 표의 키.
const STATUS_LINE_KEY: &str = "tui.status_line";

/// 터미널 모드 스폰에 싣는 상태줄 오버라이드 값 — ★codex 가 **전체 36 자 thread id** 를 화면에 찍게
/// 하는 것이 이 한 값의 전부다★(회수는 우리 PTY 가 한다 — [`thread_id`]).
///
/// ★항목 이름이 틀리면 오류가 아니라 조용한 무시다★ — `thread-name` 은 그럴듯하지만 **무효**이고,
///   유효한 것은 `thread-id`(`session-id` 는 레거시 별칭) · `thread` · `thread-title` 이다. 확인 수단은
///   `codex --strict-config -c '<이 값>' app-server --stdio`(모르는 루트 키면 `unknown configuration
///   field` 로 exit 1 — 실측 0.155.1 · 이 값은 exit 0).
/// ★작은따옴표(TOML 리터럴 문자열)가 load-bearing 이다 — 큰따옴표로 바꾸지 말 것★: 이 값은 argv 한
///   칸에 들어가 Windows 에서 `cmd.exe /c` 래핑을 한 겹 더 지난다. 큰따옴표를 넣으면 그 겹이 인자
///   경계를 다시 해석하며 값이 갈린다. 형제 오버라이드들이 같은 이유로 전부 작은따옴표다.
/// ★실측(codex-cli 0.155.1, 직전 세션 — 정본 = ADR-0216 「근거」)★: 스폰 약 0.3 초 뒤 · 첫 턴 전 ·
///   rollout 파일이 생기기도 전에 찍히고, 터미널 폭을 60 칸으로 좁혀도 36 자가 온전하다.
// ADR-0216
const STATUS_LINE_OVERRIDE: &str = "tui.status_line=['thread-id']";

/// 데몬 MCP 서버를 이 스폰에 붙이는 설정 오버라이드의 키 접두. 뒤에 서버 논리명이 붙어
/// `mcp_servers.engram` 한 키가 된다(정본 = [`MCP_SERVER_NAME`] — 이름을 여기 다시 타이핑하지 말 것).
///
/// ★claude 의 `--mcp-config <파일>` 과 **기제가 다르다**★ — codex 는 파일을 안 먹고 이 오버라이드만
///   먹는다(실측 0.155.0). 그래서 같은 endpoint 가 두 backend 에서 서로 다른 문법으로 번역된다
///   (ADR-0004 가 말하는 바로 그 지점).
const MCP_SERVER_OVERRIDE_PREFIX: &str = "mcp_servers.";

/// MCP 서버 설정에서 **bearer 토큰의 값이 아니라 그것이 든 env 변수 이름**을 받는 칸.
///
/// ★이 칸을 고른 것이 결정이다 — 값 인라인(`bearer_token='…'`)으로 되돌리지 말 것★: 그렇게 하면
///   토큰이 **명령줄에 박혀** 같은 사용자의 아무 프로세스나 argv 를 읽는 것만으로 새 나간다(Windows 의
///   프로세스 목록·WMI 가 argv 를 준다 — 이 저장소가 데몬 발견에 쓰는 그 표면이다). 이 칸을 쓰면 토큰은
///   이미 스폰 env 에 있는 그 한 벌뿐이고([`inject_cli_entrance`] 가 [`TOKEN_ENV`] 로 심는다) argv 에도
///   디스크에도 사본이 생기지 않는다.
/// ★codex 가 그 env 를 실제로 읽어 헤더로 싣는 것을 봤다(실측 2026-09-19 · 0.155.0)★ — 스텁 MCP 서버가
///   `initialize`·`tools/list` 전 요청에서 `Authorization: Bearer <그 값>` 을 받았다.
const MCP_BEARER_ENV_KEY: &str = "bearer_token_env_var";

/// 이 MCP 서버 항목 하나에 한해 도구 호출 승인 프롬프트를 없애는 칸.
///
/// 유효값은 `auto | prompt | writes | approve` 넷이다 — ★`auto` 는 프롬프트를 없애지 않는다.
///   없애는 것은 `approve` 뿐이다★(실측 2026-09-20). 이 둘을 헷갈려 한 번 잘못된 결론을 냈던
///   적이 있어 여기 이름을 둘 다 박는다. `approve`가 대화형 TUI · `codex exec`(이 칸 없이는
///   "MCP tool call requires approval, but approval policy is never"로 그냥 실패한다) ·
///   `codex app-server --stdio`(이 칸 없이는 게이트가 `mcpServer/elicitation/request`로 온다)
///   세 경로 전부에서 승인을 없애는 것을 확인했다.
/// ★이 서버 항목 하나에만 걸린다 — 승인을 전역으로 끈 것이 아니다★: `approvalPolicy: on-request`·
///   `sandbox: workspace-write`는 손대지 않으므로 일반 셸/실행 승인은 그대로 묻는다.
/// ★대화상자의 "Always allow"는 디스크에 남지 않는다★(빈 `CODEX_HOME`으로 전/후 스냅샷 대조
///   실측 — 새 프로세스는 다시 묻는다). 그래서 이 값은 한 번 설정해 두는 것으로 못 대체하고
///   스폰마다 실어야 한다.
const MCP_APPROVAL_MODE_KEY: &str = "default_tools_approval_mode";
const MCP_APPROVAL_MODE_VALUE: &str = "approve";

/// 프라이밍 지시서를 codex 기본 지시문 **뒤에 덧붙이는** 설정 키.
///
/// ★`model_instructions_file` 과 바꿔 쓰지 말 것 — 그쪽은 덧붙이기가 아니라 **대체**다(실측 0.155.0)★:
///   경로를 받는 지시문 키는 그것 하나뿐인데, 프라이밍을 거기 걸면 codex 자신의 운용 지시가 통째로
///   사라진다.
/// ★이 키에는 경로를 받는 `*_file` 짝이 **없다**★ — `developer_instructions_file` ·
///   `experimental_instructions_file` · `instructions_file` 은 전부 `unknown configuration field` 다
///   (같은 실측). 그래서 이 backend 는 claude 와 달리 경로가 아니라 **내용**을 싣고, 지시서 파일을
///   **여기서 읽는다**. 그 갈림이 ADR-0004 가 말하는 백엔드별 지식이다.
/// ★권위는 같은 층이다★ — OpenAI 문서상 `instructions` 파라미터와 `developer` 역할 메시지는 같은 등급
///   이라, 경로 대신 이 칸을 고른 대가가 「지시 강도」는 아니다.
// ADR-0004
// ADR-0215
const DEVELOPER_INSTRUCTIONS_KEY: &str = "developer_instructions";

/// 명령줄에 실을 때 줄바꿈을 대신하는 두 글자(역슬래시 + `n`).
///
/// ★실제 LF 가 한 글자라도 남으면 `cmd` 가 **그 자리에서 명령줄을 자르고 아무 오류도 내지 않는다**★
///   (실측) — 뒤따르던 인자가 통째로 사라진 채 codex 가 뜬다. 그 결말은 「프라이밍이 없다」가 아니라
///   「MCP 부착도 훅 등록도 없다」라, 조용한 절단이 이 값의 가장 비싼 실패 모드다.
/// ★두 글자로 바꾸는 것이 실측된 선택이다★ — 모델이 이 형태를 그대로 줄바꿈으로 읽었다(같은 실측).
///   TOML 리터럴 문자열에는 이스케이프가 없으므로 이 두 글자는 파서를 지나 **글자 그대로** 도착한다.
// ADR-0215
const NEWLINE_REPLACEMENT: &str = "\\n";

/// `cmd.exe` 가 받아 주는 명령줄 총 길이(UTF-16 코드 단위).
///
/// ★이 예산은 우리 값만의 것이 아니다★ — 정책 플래그·패스스루·오버라이드 둘이 같은 줄을 나눠 쓴다.
///   그래서 판정은 고정 상수 비교가 아니라 **이미 조립된 인자를 센 뒤의 잔액**으로 한다
///   ([`command_line_cost`]).
// ADR-0215
const CMD_LINE_LIMIT_UTF16: usize = 8191;

/// `console_command` 가 Windows 에서 덧대는 래핑(`cmd.exe /c codex`)의 길이 몫.
const CMD_WRAPPER_COST: usize = "cmd.exe /c ".len() + CODEX_PROGRAM.len();

/// 이미 조립된 인자가 쓴 명령줄 길이의 **보수적** 추정(UTF-16 코드 단위).
///
/// ★인용 규칙을 재현하지 않고 일부러 넉넉하게 센다★ — 인자마다 공백 하나와 감싸는 따옴표 둘을 무조건
///   얹는다. 정확히 세려면 portable-pty 의 인용 규칙과 `cmd` 의 재파싱을 **둘 다** 재구현해야 하고, 그
///   재구현은 상류가 바뀌면 조용히 낡는다. 넘게 세서 프라이밍을 건너뛰는 쪽이 모자라게 세서 명령줄이
///   잘리는 쪽보다 싸다 — 잘림은 오류 없이 뒤쪽 인자를 지운다.
/// ★래핑 몫을 플랫폼과 무관하게 더한다★ — 상한이 걸리는 곳은 Windows 뿐이지만, 판정을 플랫폼으로
///   가르면 같은 지시서가 어느 기기에서는 실리고 어느 기기에서는 안 실린다. 한 값으로 떨어뜨린다.
/// ★UTF-16 으로 세는 것이 의도다★ — Windows 명령줄은 UTF-16 이고 상한도 그 단위다. 바이트로 세면
///   비-ASCII 지시서에서 과대평가하고, `chars()` 로 세면 BMP 밖 문자에서 과소평가한다.
// ADR-0215
fn command_line_cost(args: &[String]) -> usize {
    CMD_WRAPPER_COST
        + args
            .iter()
            .map(|a| a.encode_utf16().count() + 3)
            .sum::<usize>()
}

/// 이 스폰에 실을 프라이밍 지시서의 **내용**. `None` = 실을 것이 없다(경로 부재·읽기 실패·빈 파일).
///
/// ★읽기 실패는 fail-open 이다★ — 경고만 남기고 `None` 을 돌려준다. 프라이밍은 있으면 좋은 것이고
///   스폰은 필수다(ADR-0216 이 일반 규칙으로 물려받은 「어떤 갈래로도 스폰을 실패시키지 않는다」).
/// ★내용을 **이 crate 가** 읽는 것이 claude 갈래와 갈리는 지점이다★ — 그쪽은 경로를 넘기고 claude 가
///   직접 읽는다(ADR-0092). codex 에는 경로를 받는 키가 없어서([`DEVELOPER_INSTRUCTIONS_KEY`]) 그
///   선택지가 없다. 이 읽기가 backend 폴더 안에 있는 것이 ADR-0004 의 요점이다 — 데몬의 프라이밍
///   provider 는 여전히 **경로만** 다룬다.
/// ★빈 파일도 `None` 이다★ — 빈 지시문을 싣는 것은 명령줄만 쓰고 아무것도 가르치지 않는다.
// ADR-0004
// ADR-0092
// ADR-0215
// ADR-0216
fn priming_text(control: Option<&ControlEndpoint>) -> Option<String> {
    let path = control?.priming_file.as_deref()?;
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => {
            tracing::warn!(
                "codex 프라이밍 미주입 — 지시서 파일이 비었다: {}",
                path.display()
            );
            None
        }
        Ok(text) => Some(text),
        Err(e) => {
            tracing::warn!(
                "codex 프라이밍 미주입 — 지시서 파일을 못 읽었다({}): {e}",
                path.display()
            );
            None
        }
    }
}

/// 터미널 모드 spawn 에 실을 `-c developer_instructions='…'` 값. `None` = 싣지 않는다 — ★경고만 남기고
/// 스폰은 그대로 간다★(ADR-0216 의 「어떤 갈래로도 스폰을 실패시키지 않는다」와 같은 규율).
///
/// `args_so_far` = 이 값 앞에 이미 조립된 인자들. 명령줄 예산 판정에만 쓴다.
///
/// ★TOML 리터럴 문자열(작은따옴표)로 감싼다★ — 이스케이프가 없어 값이 바이트 그대로 건너가고, 그 조합이
///   `cmd.exe /c` + `.cmd` shim 두 겹을 견디는 것이 실측돼 있다(ADR-0210). 대신 작은따옴표 자체는 담을
///   수 없다.
/// ★실을 수 없는 문자를 만나면 지어낸 이스케이프로 밀어 넣지 않고 건너뛴다★ — 네 글자가 각각 다른 층을
///   깬다: 작은따옴표는 TOML 리터럴을, 큰따옴표와 역슬래시는 그 바깥의 **명령줄 인용**(portable-pty 가
///   `\"` 로 이스케이프하는데 `cmd` 는 그 규칙을 모르고 따옴표 수만 센다)을, `%` 는 cmd 의 환경변수
///   치환을(**따옴표 안에서도 편다** — `build_spec` 의 같은 이름 한계 주석이 정본) 깬다.
/// ★이 가드가 오늘 한 번도 안 걸린다고 걷어내지 말 것★ — 지금 지시서에는 그 넷이 0 개지만 이 문서는
///   사람이 고치는 마크다운이고, 영어 축약형(`don't`) 한 번이면 작은따옴표가 들어온다.
/// ★탭을 포함한 제어문자도 끊는다★ — 줄바꿈만 위 [`NEWLINE_REPLACEMENT`] 로 바꾸고, 나머지는 `cmd` 의
///   토큰 분리와 인용 계층을 우리가 검증한 적이 없다. 증상이 전부 「조용히 어긋난 명령줄」이라 싣지
///   않는 쪽을 고른다.
/// ★가드는 **바꾸기 전 원문**을 본다★ — 뒤에 하면 우리가 넣은 역슬래시가 우리 가드에 걸린다.
// ADR-0215
// ADR-0216
fn developer_instructions_override(text: &str, args_so_far: &[String]) -> Option<String> {
    if let Some(bad) = text.chars().find(|c| {
        matches!(c, '\'' | '"' | '%' | '\\') || (c.is_control() && *c != '\n' && *c != '\r')
    }) {
        tracing::warn!(
            "codex 프라이밍 미주입 — 지시서에 명령줄로 실을 수 없는 문자가 있다({bad:?})"
        );
        return None;
    }
    let single_line = text
        .replace("\r\n", NEWLINE_REPLACEMENT)
        .replace('\n', NEWLINE_REPLACEMENT)
        .replace('\r', NEWLINE_REPLACEMENT);
    let value = format!("{DEVELOPER_INSTRUCTIONS_KEY}='{single_line}'");
    // `-c` 와 값 자신이 함께 드는 몫 — 위 [`command_line_cost`] 와 같은 셈법으로 센다.
    let cost = CONFIG_OVERRIDE_FLAG.encode_utf16().count() + 3 + value.encode_utf16().count() + 3;
    let used = command_line_cost(args_so_far);
    if used + cost > CMD_LINE_LIMIT_UTF16 {
        tracing::warn!(
            "codex 프라이밍 미주입 — 명령줄 예산을 넘는다(이미 {used}, 더 필요 {cost}, 상한 {CMD_LINE_LIMIT_UTF16})"
        );
        return None;
    }
    Some(value)
}

/// 이 스폰에 데몬 MCP 서버를 붙일지의 **단일 판정**. [`AgentBackend::build_spec`](실제 부착)과
/// [`AgentBackend::precheck_control_endpoint`](fail-closed 게이트)가 **같은 이 값**을 읽는다.
///
/// ★두 자리가 각자 판정하면 게이트가 공허해진다★ — 게이트가 「붙일 수 있다」고 통과시킨 스폰이
///   조립에서는 부착 없이 떠도 아무 신호가 없다. 그래서 판정은 [`mcp_attachment`] 하나뿐이고 이
///   enum 이 그 결과를 두 읽는 자리에 같은 모양으로 나른다.
enum McpAttachment {
    /// 붙인다 — `-c` 에 실을 값.
    Attach(String),
    /// 붙이지 않는다, 그리고 **그것이 정상 상태다**. 두 갈래가 여기 든다 — 제어 채널이 아예 없는
    /// 스폰(우편 평면 밖)과, 데몬이 이 스폰에 MCP 발신 입구를 **인가하지 않은** 스폰(운영자가 MCP
    /// 우편을 껐다 — [`ControlEndpoint::grants_mcp_send`]). 둘 다 스폰을 막지 않는다.
    NotWanted,
    /// 붙여야 하는데 **못 만든다**(사유). ★이 값만이 fail-closed 를 낳는다★ — 이 상태로 그냥 뜨면
    /// 에이전트는 수신 명단(`reads_messages`)에 오른 채 발신 입구가 0 이라(MCP 는 안 붙었고 CLI
    /// 미러는 `mail_allowed=false` 가 닫는다), 배달된 요청이 전부 아무도 답할 수 없는 계약이 된다.
    Unrepresentable(String),
}

/// 위 판정을 내리는 유일한 자리.
///
/// ★환경변수를 여기서 다시 읽지 않는다(ADR-0004)★: MCP 우편을 끄는 노브(`ENGRAM_DISALLOW_MCP_SEND` ·
///   하네스의 `ENGRAM_FORCE_CLI_ONLY_SEND`)의 주인은 **데몬**이고, 그 결정은 이미 씹혀서
///   [`ControlEndpoint::grants_mcp_send`] 로 실려 온다. backend 가 같은 변수를 다시 읽으면 권위가
///   둘이 되고, 갈리는 날 「claude 는 꺼졌는데 codex 만 켜져 있다」가 조용히 난다(그것이 이 갈래를
///   만든 적출이다).
///
/// ★이 갈래엔 평문 토큰 파일이 없다 — 그러나 그것을 보증하는 것은 이 함수가 아니다★: 여기서 토큰이
///
/// ★이 갈래엔 평문 토큰 파일이 없다 — 그러나 그것을 보증하는 것은 이 함수가 아니다★: 여기서 토큰이
///   argv 에 안 박힌다는 것은 위 [`MCP_BEARER_ENV_KEY`] 가 지키고, **디스크에도 안 쓰인다**는 것은
///   [`AgentBackend::writes_mcp_config_file`] 가 false 인 것이 지킨다(데몬 `control::provision` 이 그
///   칸 하나로 `mcp_config::write_config`/`write_settings` 를 가른다 — 그 파일의 내용·수명은
///   `control/mcp_config.rs`). ★한때 이 자리엔 「이 갈래는 그 파일이 애초에 없다」가 적혀 있었고 그것이
///   거짓이었다★ — 그때 데몬은 `accepts_mcp_config` 를 보고 codex 스폰마다 그 파일을 실제로 썼다.
///   이 함수가 파일을 안 만든다는 것과 그 스폰에 파일이 안 생긴다는 것은 다른 말이다.
/// ★TOML 리터럴 문자열(작은따옴표)을 쓴다★ — 이스케이프가 없어 값이 **바이트 그대로** 건너가고, 그
///   조합이 `cmd.exe /c` + `.cmd` shim 두 겹을 견디는 것이 이미 실측돼 있다(ADR-0210). 대신 작은따옴표
///   자체는 담을 수 없다.
/// ★실을 수 없는 문자를 만나면 지어낸 이스케이프로 밀어 넣지 않고 끊는다★ — 작은따옴표·공백문자·
///   제어문자는 값이나 TOML 줄 자체를 깨고, `%` 는 cmd 가 명령줄에서 **따옴표 안에서도** 환경변수로
///   펴서(아래 `build_spec` 의 같은 이름 한계 주석이 정본) 다른 주소를 가리키게 만든다. 셋 다 증상이
///   「우편이 조용히 안 된다」 하나라 여기서 소리를 낸다.
///   ★그 검사가 오늘 한 번도 안 걸리는 것이 정상이다★ — 이 url 은 데몬이 authoring 하는
///   `http://127.0.0.1:<port>/mcp` 이고 사용자 입력이 아니다. 검사는 그 형태가 바뀌는 날을 위한 것이다.
// ADR-0004
// ADR-0128
// ADR-0209
fn mcp_attachment(control: Option<&ControlEndpoint>) -> McpAttachment {
    let Some(endpoint) = control else {
        return McpAttachment::NotWanted;
    };
    // ★이 한 줄이 「운영자가 MCP 우편을 껐다」를 이 backend 에 닿게 하는 전부다★ — 예전에는 제어
    //   채널의 **존재**만 보고 무조건 붙여서, 노브를 켠 운영자가 claude 는 꺼지고 codex 는 켜진 채로
    //   남는 상태를 얻었다(`eg_send` 가 그대로 자동 승인으로 호출 가능).
    if !endpoint.grants_mcp_send() {
        return McpAttachment::NotWanted;
    }
    let url = endpoint.url.as_str();
    if let Some(bad) = url
        .chars()
        .find(|c| *c == '\'' || *c == '%' || c.is_control() || c.is_whitespace())
    {
        return McpAttachment::Unrepresentable(format!(
            "codex MCP 서버를 붙일 수 없다 — 제어 채널 주소에 실을 수 없는 문자가 있다({bad:?}): {url}"
        ));
    }
    McpAttachment::Attach(format!(
        "{MCP_SERVER_OVERRIDE_PREFIX}{MCP_SERVER_NAME}={{url='{url}',{MCP_BEARER_ENV_KEY}='{TOKEN_ENV}',{MCP_APPROVAL_MODE_KEY}='{MCP_APPROVAL_MODE_VALUE}'}}"
    ))
}

/// 상주 JSON 서버로 띄우는 하위 명령과 그 전송 선택(실측 0.154.0 — `--stdio` 는 `--listen stdio://` 와
/// 같고 그것이 기본값이다. 기본값에 기대지 않고 명시한다: 이 통로는 stdio 가 아니면 성립하지 않는데,
/// 기본값은 상류가 바꿀 수 있고 바뀌어도 우리 argv 는 조용히 그대로다).
const APP_SERVER_SUBCOMMAND: &str = "app-server";
const APP_SERVER_STDIO_FLAG: &str = "--stdio";

/// 호출자 패스스루가 **우리와 같은 설정 키**를 세우나. `true` = 우리 오버라이드가 그것을 덮는다(뒤에
/// 실리므로) — 사용자가 일부러 건 값을 말없이 지우지 않도록 그 자리에서 경고하게 한다.
///
/// ★잡는 모양은 하나뿐 = `-c` **다음 칸**이 `key` 로 시작하는 형태다★(`-c tui.status_line=…`).
/// ★못 잡는 것 — 알고 두는 구멍이다★: `-c` 와 값을 한 낱말로 붙인 형태 · `-c` 말고 긴 이름의 같은
///   플래그 · 표 전체를 덮는 상위 키(`-c tui=…` · `-c mcp_servers=…`) · 그 키로 **시작만 하는** 다른
///   키. 이 목록을 키워 「확실히」 만들려 들지 말 것 — codex 오버라이드 문법의 재구현이 되고 그 재구현은
///   상류가 바뀔 때마다 조용히 낡는다. 놓쳐서 잃는 것은 **경고뿐이고 동작이 아니다** — 우리 것이 이기는
///   성질은 아래 `build_spec` 의 순서가 따로 보장한다.
/// ★키를 인자로 받는 것이 의도다★ — 오버라이드가 여럿이라(MCP 서버 부착 · 상태줄 · 지시서) 판정을 키마다
///   복제하면 한쪽만 고쳐져 경고가 반쪽이 된다.
fn passthrough_overrides_key(extra_args: &[String], key: &str) -> bool {
    extra_args
        .windows(2)
        .any(|pair| pair[0] == CONFIG_OVERRIDE_FLAG && pair[1].starts_with(key))
}

pub struct CodexBackend;

impl AgentBackend for CodexBackend {
    /// ★호출자가 세션 id 를 정할 수 없다(실측)★ — codex 에는 `--session-id` 류 플래그가 없고 id 는
    /// codex 가 스스로 발급한다. true 로 두면 manager 가 우리 uuid 를 발급해 **프로필에 영속**하는데 그
    /// 값은 codex 가 한 번도 쓰지 않는다 — 그러고 나면 이어받기 판정이 그 가짜 값을 보고 서서, 실제로는
    /// 새 대화인 화신을 이어받았다고 믿는다.
    /// ★app-server 모드라고 켜지 말 것 — 두 모양 다 false 다★: 그 모드의 식별자(thread id)도 발급 주체는
    ///   codex 다. 그 id 로 **이어받을 수 있나**는 아래 별개 축이 답한다.
    // ADR-0185
    fn assigns_session_id(&self, _command: &AgentCommand) -> bool {
        false
    }

    /// ★두 모드 다 이어받는다 — 수단만 다르다★: app-server 는 저장된 thread id 로 `thread/resume` 을
    /// 내고([`AgentBackend::open_spawn`] 이 고른다), 터미널 모드는 같은 id 를 하위 명령 + 위치 인자로
    /// 실어 띄운다([`RESUME_SUBCOMMAND`] · [`AgentBackend::build_spec`] 의 터미널 갈래).
    /// ★그래서 이 칸은 [`is_app_server`] 를 보지 않는다 — 되돌리지 말 것★: 이 술어가 묻는 것은
    ///   **저장된 sid 로 이어받을 수 있나**이지 어느 통로로 이어받나가 아니다. 통로로 가르면 터미널
    ///   모드로 뜬 codex 는 손잡이가 명부에 있어도 활성화 입구가 Fresh 로 띄워, TUI 상태줄에서 읽어
    ///   적어 둔([`crate::profile::ProfileRegistry::observe_session_id`]) 그 id 가 영영 안 쓰인다
    ///   (ADR-0216).
    /// ★손잡이가 **없을 때**는 이 칸이 답하지 않는다★ — 그 판정은 저장된 sid 존재와 함께 보는
    ///   [`crate::backend::can_resume_profile`] 이 하고, 그래도 Resume 으로 들어온 spawn 은
    ///   [`AgentBackend::build_spec`] 이 새 대화 argv 로 떨어뜨린다(그 자리 doc).
    /// ★아래 [`AgentBackend::capabilities`] 의 `session.resume` 과 **같은 술어로 함께 켠다 — 한쪽만
    ///   건드리지 말 것**★: 어느 쪽이든 단독으로 켜면 이어받은 적 없는 새 스레드가 「이어받음」으로
    ///   보고되고, 단독으로 끄면 실제로 이어받는 스폰이 「새 대화」로 보고된다.
    // ADR-0185
    // ADR-0208/ADR-0210
    fn can_resume_stored_session(&self, _command: &AgentCommand) -> bool {
        true
    }

    /// ★true 인데 [`AgentBackend::accepts_mcp_config`] 는 false 다 — 두 축은 별개다(ADR-0133)★: 이 칸이
    /// 묻는 것은 **제어 채널을 소비하나**이고, 그 소비 수단은 MCP 만이 아니라 CLI 입구(크레덴셜 env)도
    /// 있다. 아래 칸이 묻는 것은 그중 **mcp-config 파일을 먹일 수 있나** 하나뿐이다.
    /// ★false 로 되돌리면 codex 스폰의 env 에서 `ENGRAM_TOKEN`·`ENGRAM_CONTROL_URL` 이 통째로 사라진다★
    ///   — 조립점이 이 칸을 보고 provision 자체를 건너뛰므로([`crate::manager::AgentManager`] 의 그
    ///   자리) endpoint 가 `None` 으로 오고, 아래 `build_spec` 의 주입이 한 줄도 돌지 않는다.
    /// ★한때 이 env 의 소비자에 **우리 훅 프로세스**가 있었다 — 오늘은 없다★(ADR-0216 이 등록을 걷었다).
    ///   그때 잰 것은 남긴다: codex 는 훅 프로세스에 자기 env 를 **하나도 덧씌우지 않았다**(순수 상속 —
    ///   실측 2026-09-18, 0.155.0). 살아 있는 소비자는 codex 가 띄우는 셸 도구와 MCP 우편이고, 공용
    ///   주입이 「보조 프로세스의 자격증명이기도 하다」고 적은 조건은 [`inject_cli_entrance`] doc 이 진다.
    /// ★이 칸이 **우편을 열지는 않는다 — 그리고 그것이 공짜가 아니었다**★: 예전 데몬은 우편 가부를
    ///   `!accepts_mcp_config` 하나로 파생해서, 이 칸을 켜는 것만으로 이 백엔드에 **보내기 인가가 함께
    ///   열렸다**(받기는 [`AgentBackend::reads_messages`] 가 닫은 채로). 그 비대칭을 없애려고 보내기 축을
    ///   별도 선언으로 뽑았다 — 아래 [`AgentBackend::uses_mail`] 이 그것이고, 데몬은 두 축에서 한 값을
    ///   파생한다. ★그 칸을 지우거나 기본값으로 되돌리면 이 부수효과가 그대로 돌아온다★.
    ///   (오늘 그 칸은 켜져 있다 — 그러나 **선언으로** 켜져 있고, 부수효과로가 아니다.)
    /// ★`engram` 실행파일이 없는 설치에서도 이 백엔드의 스폰은 **끊기지 않는다**★ — 그 fail-closed 는
    ///   「CLI 우편을 가르쳤는데 부를 실행파일이 없다」는 짝 위반을 지키는 것이고, 우편 평면 밖 스폰에는
    ///   그 짝이 없다(그 판정의 정본 = 데몬 `control::provision`). 제어 동사를 못 쓰게 되는 것은 남지만
    ///   그쪽은 fail-open + warn 이다.
    // ADR-0086
    // ADR-0132
    // ADR-0133
    // ADR-0208
    // ADR-0209
    fn supports_control_channel(&self) -> bool {
        true
    }

    /// ★이 칸의 **이름과 오늘의 뜻이 어긋나 있다 — 알고 켰다**★: 이름과 [`crate::types::ControlChannelNeeds`]
    /// doc 이 묻는 것은 「우리가 만든 mcp-config **파일**을 먹일 수 있나」이고 codex 의 답은 여전히
    /// **아니오**다(그 파일을 가리킬 플래그가 없다 — 실측 0.155.0). 그런데 데몬은 이 한 칸으로 **우편
    /// 채널 판정**까지 파생하므로(`mail_allowed = uses_mail && !accepts_mcp_config`), MCP 로 우편을 쓰는
    /// 이 백엔드가 false 를 유지하면 **CLI 미러가 열린 채로** 남는다 — ADR-0209 가 「MCP 가 정식, CLI 는
    /// 미러」라고 못 박은 것의 정반대다.
    /// ★한때 이 칸을 켜면 **파일까지 딸려 왔고, 그 대가는 갚았다**★: 데몬이 이 한 칸을 보고 mcp-config
    ///   JSON(평문 Bearer 토큰)과 세션 설정 조각을 실제로 썼고, codex 는 둘 다 안 읽으므로 스폰마다
    ///   아무도 안 여는 비밀 파일이 하나 생겼다 — 게다가 그 write 는 fail-closed 라 **안 읽는 파일 때문에
    ///   codex 스폰이 끊길 수 있었다**. 데몬의 파생 축을 둘로 쪼개 해소했다: 파일을 쓸지는 이제 아래
    ///   [`AgentBackend::writes_mcp_config_file`] 가 단독으로 가르고, 이 칸은 우편 채널 판정만 굴린다.
    ///   ★그 칸을 지우거나 여기 값으로 파생하면 그 대가가 그대로 돌아온다★.
    /// ★실제 부착 수단은 [`mcp_server_override`] 다★ — `-c mcp_servers.engram={…}` 한 값. 그 값이 실제로
    ///   서버를 세우는 것을 봤다(실측 2026-09-19 · 0.155.0: `mcpServerStatus/list` 가 `engram` 을 그
    ///   서버의 툴 둘과 함께 돌려줬고, 스텁 서버는 `Authorization: Bearer …` 를 받았다).
    ///   ★그날 찍힌 이름은 `send_message`·`messages` 이고, `1b84045` 가 `eg_send`·`eg_messages` 로
    ///   개명했다★ — 이 파일 헤더의 같은 실측 항목이 그 사유의 정본이다.
    // ADR-0099
    // ADR-0128
    // ADR-0209
    fn accepts_mcp_config(&self) -> bool {
        true
    }

    /// ★위 칸이 true 인데 이 칸은 false 다 — 그 갈림이 이 칸의 존재 이유다★: codex 에는 우리가 쓴 파일을
    /// 가리킬 플래그가 **없다**(실측 0.155.0 — `--mcp-config` 도 `--settings` 도, 설정 파일을 가리키는
    /// `--config <파일>` 도 없다). MCP 부착은 [`mcp_server_override`] 의 `-c mcp_servers.engram={…}`
    /// 한 값 단독이고, 토큰은 그 값이 이름으로 가리키는 env 로 간다([`MCP_BEARER_ENV_KEY`]) — 디스크에
    /// 사본을 만들 자리가 아예 없다.
    /// ★false 를 유지하는 것이 보안 결정이다 — 위 칸에서 파생하지 말 것★: true 로 되돌리면 데몬이
    ///   스폰마다 평문 Bearer 토큰 JSON 을 기본 ACL 로 데이터 디렉토리에 쓰고(아무도 안 연다), 그
    ///   write 실패가 **fail-closed** 라 codex 스폰이 그 파일 때문에 끊긴다. 그것이 이 칸이 갈라지기 전의
    ///   실제 상태였다.
    // ADR-0086
    // ADR-0099
    // ADR-0209
    fn writes_mcp_config_file(&self) -> bool {
        false
    }

    /// ★한때 false 였다 — 그 사유와 그것을 버린 이유를 **함께** 남긴다(지우지 말 것)★: 옛 값의 근거는
    /// 「이 메서드가 `command` 를 안 받아 두 모드를 가를 수 없다」였다. app-server 모드는 턴 신호를
    /// 낸다 — 통로가 `turn/completed` 를 읽고(`turn/started` 는 **일부러** 읽지 않는다. 사유 정본은 그
    /// 통로의 `TURN_COMPLETED` doc), 번역기가 그 알림을 턴 경계로 옮겨 아래 [`classify_turn`] 이
    /// `Ended` 를 낸다. 그런데 터미널 모드는 decoder 가 없어 `TerminalBytes` 만 흐르고 신호가 0 이라,
    /// 여기서 true 를 돌려주면 **그 모드까지 함께 열린다** — 그래서 닫아 뒀었다.
    /// ★그 사유는 막을 근거가 못 된다 — 터미널 claude 가 **정확히 같은 자리에 있고 이미 받는다**★:
    ///   구조화 출력이 없는 claude 도 턴 신호가 0 인데 수신자 명단에 있다. 정책 정본은 **ADR-0116
    ///   결정 7** 이고 그 문장이 「턴 신호 없음 → 게이트 없이 즉시 주입」이다(그 CLI 자신의 입력 큐가
    ///   게이트라서 우리가 idle 을 관측할 이유가 없다). 같은 ADR 의 거부한 대안이 「관측할 수 없으니
    ///   배달할 수 없다」를 **명시로** 죽였다 — 그 전제로 되돌리지 말 것.
    /// ★그래서 대가를 **알고** 받는다★: 터미널 모드에는 바쁨 게이트가 없으므로 봉투가 턴 한가운데
    ///   꽂힐 수 있고, TUI 가 모달 상태(승인 프롬프트·메뉴·플랜 확인)면 그 위젯이 봉투를 먹을 수 있다.
    ///   ADR-0116 결정 7 이 그 대가를 이미 명시 수용했다(문제가 실제로 관측되면 그때 좁힌다).
    /// ★shell 과 같이 열지 말 것★ — 그쪽 false 는 관측 축이 아니라 **입력이 명령으로 실행된다**는 축이라
    ///   이 변경과 무관하게 그대로 남는다(기본 구현 doc 의 그 문단).
    // ADR-0116
    // ADR-0209
    fn reads_messages(&self) -> bool {
        true
    }

    /// ★이 칸이 **무엇을 사는지** 정확히 적는다★: 데몬의 `build_grants` 가 이 값이 false 면 **한 줄도
    /// 내지 않고 단락**하고(발신 입구 grant 0), 프라이밍 변형 선택도 `needs.uses_mail.then_some(…)` 으로
    /// 끊긴다. 즉 이 칸을 닫은 채 위 `accepts_mcp_config` 만 켜면 에이전트는 MCP 툴을 **손에 쥔 채
    /// 아무도 그 존재를 말해 주지 않는** 상태가 된다.
    /// ★제어 동사와는 별개다★ — 이 칸이 여는 것은 우편이고, CLI 입구(토큰·주소)와 제어 라우트는 위
    ///   `supports_control_channel` 이 연다.
    /// ★한때 이 자리에 「미해소 — 보내기만 열렸다」가 적혀 있었다. 해소됐다★: 그 문단이 요구한 근거
    ///   둘 중 첫째(받기 축이 열렸다)가 섰다 — [`AgentBackend::reads_messages`] 가 true 라 이 백엔드는
    ///   데몬 `messaging_host` 의 배달 명단에 오르고, 이 에이전트가 보낸 `request` 의 답장이 돌아올
    ///   곳이 있다. 그래서 ADR-0209 결정 4 의 「받기 먼저」 순서를 이제 어기지 않는다.
    ///   ★되돌릴 때의 규칙은 그대로다★ — 받기 축을 닫으면서 이 칸만 열어 두면 그 비대칭이 되살아난다.
    // ADR-0133
    // ADR-0209
    fn uses_mail(&self) -> bool {
        true
    }

    /// ★app-server 모드에만 세울 연결이 있다★ — 핸드셰이크 왕복을 마쳐야 `turn/start` 가 허용된다.
    /// 터미널 모드는 PTY 라 프로세스가 뜬 순간부터 쓸 수 있어 이 축이 없다(claude·shell 과 같다).
    /// ★이 선언과 실제 배달이 **어긋나면 안 된다**★ — true 인데 안 부르면 감독자가 백스톱까지 기다리고,
    ///   false 인데 부르면 그 배달을 아무도 안 받는다. 그 짝은 시험대가 잰다.
    // ADR-0004
    fn declares_link(&self, command: &AgentCommand) -> bool {
        is_app_server(command)
    }

    /// 이어받기가 왜 실패했나 — ★이 backend 의 증거는 **통로의 연결 사유**로 온다★.
    ///
    /// ★그 사유가 두 꼬리 어디에도 없다는 것이 이 구현의 존재 이유다★: 이어받기 거절은 stdout 의
    ///   JSON-RPC 오류라, 콘솔 꼬리(`terminal_tail`)는 이 통로에 아예 없고 진단 꼬리
    ///   (`diagnostic_tail`)는 stderr 만 담는다. 그래서 판정은
    ///   [`crate::transport::LinkState::Down`] 의 `reason` 을 증거로 넘긴다 — 선언이 없으면 그 문자열이
    ///   와도 분류가 `None` 으로 떨어져 「이어받기 직후 조기 종료」라는 **틀린 맥락 기본값**이 찍힌다
    ///   (이 갈래에는 조기 종료가 없다. 프로세스는 멀쩡히 살아 있었다).
    ///
    /// ★**오류 코드로 가르지 않는다 — `-32600` 은 뜻이 하나가 아니다**★(실측 0.154.0): 모르는 스레드도,
    ///   두 번째 `initialize` 도, 설정 오류도, **하위 스레드를 직접 이어받으려 한 경우**도 전부 그 코드로
    ///   온다. 코드로 「모르는 스레드」를 유도하면 멀쩡한 손잡이가 무관한 실패에서 분류를 잘못 받는다.
    ///   그래서 보는 것은 **메시지**뿐이다.
    /// ★선언하는 문구는 **실측된 것 하나**다★ — `no rollout found for thread id`
    ///   (`docs/reference/backend-capabilities.md` §1 「모르는 id 를 주면」). 나머지 사유들은 실제 응답
    ///   문구를 우리가 갖고 있지 않으므로 **지어내지 않는다** — `None` 으로 떨어뜨려 호출자가 맥락
    ///   기본값을 쓰게 두고, 사람이 읽을 사유는 어차피 `reason` 문자열 그대로 결말에 실려 나간다.
    ///   ★특히 「하위 스레드는 부모를 먼저 이어받아라」 갈래는 문구를 모른다★ — 실물을 재기 전에
    ///   추측한 문자열을 넣으면 안 맞는 매칭이 조용히 죽은 코드로 남는다.
    /// ★살아 있는 세션에도 불린다(trait doc 의 조건)★ — 위 문구는 상대가 **요청을 거절할 때만** 내는
    ///   것이고 대화 본문에 섞일 수 있는 말이 아니라, 그 조건을 만족한다.
    // ADR-0004
    // ADR-0082
    // ADR-0172
    fn resume_failure_kind(&self, evidence: &str) -> Option<AgentFailureKind> {
        if evidence.to_lowercase().contains(NO_ROLLOUT_MARKER) {
            return Some(AgentFailureKind::NoConversationToResume);
        }
        None
    }

    /// ★claude 와 같은 자리에 fail-closed 를 세운다★ — 그쪽은 데몬의 `?`(mcp-config write 실패)가
    /// 끊어 주지만, 이 backend 는 파일을 안 쓰므로(`writes_mcp_config_file` = false) 데몬이 끊을 재료가
    /// 없다. 조립에서 부착 값을 못 만드는 것은 **여기서만** 보인다.
    ///
    /// ★끊는 조건은 [`McpAttachment::Unrepresentable`] 하나다 — 넓히지 말 것★: 그때의 상태가
    ///   「배달 명단에는 올라가는데(`reads_messages` = true) 발신 입구가 0」이다. MCP 는 안 붙었고,
    ///   CLI 미러는 데몬이 `mail_allowed=false` 로 닫아 뒀다(이 backend 는 `accepts_mcp_config` 가
    ///   true 라 그 파생이 언제나 그렇게 떨어진다) — 그러면 배달된 `request` 가 전부 **아무도 답할 수
    ///   없는 계약**이 되고, 증상은 오류가 아니라 영원한 무응답이다.
    /// ★열어 두는 갈래 둘(둘 다 정상 상태다)★: ① 제어 채널이 아예 없는 스폰 — 우편 없이 뜨는 codex 는
    ///   정당하다 ② 데몬이 MCP 발신 입구를 인가하지 않은 스폰 — 운영자가 MCP 우편을 껐다는 뜻이고,
    ///   그 스폰은 애초에 우편을 기대하지 않는다. 둘 다 끊으면 `engram` 없이 도는 설치와 노브를 켠
    ///   하네스에서 codex 스폰이 통째로 죽는다.
    // ADR-0004
    // ADR-0209
    fn precheck_control_endpoint(
        &self,
        _command: &AgentCommand,
        control: Option<&ControlEndpoint>,
    ) -> Result<(), String> {
        match mcp_attachment(control) {
            McpAttachment::Attach(_) | McpAttachment::NotWanted => Ok(()),
            McpAttachment::Unrepresentable(reason) => {
                tracing::warn!("codex spawn fail-closed — {reason}");
                Err(reason)
            }
        }
    }

    /// ★`session_id` 는 **여전히** 조립하지 않는다 — 되살리지 말 것★: `--session-id` 는 존재하지 않고
    /// ([`AgentBackend::assigns_session_id`] 가 false 라 이 칸은 언제나 `None` 이다), 그 값을 argv 에
    /// 실으면 codex 가 한 번도 쓰지 않을 uuid 를 명령줄에 밀어 넣는 것이 된다.
    /// ★이어받는 것은 그 칸이 아니라 `resume_session_id` 다 — 두 칸을 접지 말 것★: 이어받기는 플래그가
    ///   아니라 **하위 명령 + 위치 인자**라([`RESUME_SUBCOMMAND`]) 조립 모양부터 다르다. 접으면
    ///   `assigns_session_id() == false` 의 뜻이 거짓이 된다.
    /// ★터미널 모드만 조립한다★ — app-server 모드의 이어받기는 argv 가 아니라 핸드셰이크 둘째 요청
    ///   (`thread/resume`)이고, 그것을 고르는 자리는 [`AgentBackend::open_spawn`] 이다. 같은 값이 두
    ///   모드에서 서로 다른 수단으로 나가는 것이지 두 번 나가는 것이 아니다.
    /// ★Resume 인데 손잡이가 없으면 **새 대화 argv 로 떨어진다**★ — `codex resume` 를 id 없이 내면
    ///   **TUI 피커**가 뜨고(실측 0.155.0 `--help`: "picker by default"), 그 화면은 PTY 에 붙은 에이전트를
    ///   영원히 첫 화면에 묶어 둔다. 그래서 여기서는 하위 명령 자체를 빼 Fresh 와 **바이트 단위로 같은**
    ///   argv 를 낸다.
    ///   ★그 퇴행을 조용히 넘기지 않는다 — 그런데 신고하는 자리는 여기가 아니다★:
    ///   [`crate::manager::AgentManager`] 의 `resume_no_fallback` 이 spawn 전에 같은 조건
    ///   (`opens_a_new_conversation`)을 판정해 경고를 남기고 결말을 `Resumed` 가 아니라 `Started` 로
    ///   낸다. 여기서 또 신고하면 같은 사실이 두 출처에서 갈려 적힌다 — 이 자리는 인자만 만든다.
    // ADR-0004
    // ADR-0185
    // ADR-0208/ADR-0210
    fn build_spec(
        &self,
        command: &AgentCommand,
        mode: SpawnMode,
        _session_id: Option<Uuid>,
        resume_session_id: Option<Uuid>,
        cwd: PathBuf,
        mut env: Vec<(String, String)>,
        control: Option<ControlEndpoint>,
    ) -> CommandSpec {
        match command {
            AgentCommand::Codex {
                extra_args,
                output_format,
            } => {
                let mut args = Vec::with_capacity(8 + extra_args.len());
                match output_format {
                    AgentOutputFormat::Terminal => {
                        // ★하위 명령과 그 위치 인자가 **맨 앞**이어야 한다★ — 뒤로 밀리면 clap 이
                        //   `resume` 를 하위 명령이 아니라 루트의 `[PROMPT]` 로 읽는다(app-server 갈래의
                        //   같은 제약이 `app_server_extra_args_come_last` 로 못 박혀 있다).
                        // ★둘은 짝이다 — 하나만 내보내지 말 것★: id 없는 `codex resume` 는 TUI 피커라
                        //   에이전트가 첫 화면에서 멈춘다(위 doc 의 그 갈래).
                        if let (SpawnMode::Resume, Some(thread_id)) = (mode, resume_session_id) {
                            args.push(RESUME_SUBCOMMAND.to_string());
                            args.push(thread_id.to_string());
                        }
                        args.push(CD_FLAG.to_string());
                        args.push(cwd.to_string_lossy().into_owned());
                        args.push(SANDBOX_FLAG.to_string());
                        args.push(SANDBOX_WORKSPACE_WRITE.to_string());
                        args.push(APPROVAL_FLAG.to_string());
                        args.push(APPROVAL_ON_REQUEST.to_string());
                    }
                    // ★정책과 작업 폴더가 여기 안 실리는 것은 의도다★ — app-server 는 그 셋을
                    //   `thread/start` 파라미터(`cwd`·`sandbox`·`approvalPolicy`)로 받는다(실측 0.154.0).
                    //   그 값을 만드는 자리는 아래 [`AgentBackend::open_spawn`] 이고, 프로세스 cwd 는
                    //   `CommandSpec.cwd` 로 남는다 — 둘은 같은 값을 받지만 같은 칸이 아니다.
                    AgentOutputFormat::StreamJson => {
                        args.push(APP_SERVER_SUBCOMMAND.to_string());
                        args.push(APP_SERVER_STDIO_FLAG.to_string());
                    }
                }
                // 우리 인자를 먼저 소진하고 호출자 패스스루를 뒤에 잇는다 — 터미널 모드의 셋은 전부 값
                //   하나짜리라 뒤 인자를 흡수하지 않는다(claude 의 variadic `--allowedTools` 와 다른 점).
                // ★패스스루는 모드를 안 가린다 — 그리고 두 모드의 옵션 집합은 다르다(실측 0.154.0:
                //   `codex --help` 와 `codex app-server --help` 가 서로 없는 플래그를 갖는다)★. 즉 대화형
                //   CLI 를 보고 적은 인자는 app-server 에서 거절될 수 있다. 그래도 모드별로 거르지 않는다:
                //   무엇이 유효한지는 상류가 버전마다 바꾸므로 우리가 든 명단은 낡고, 낡은 명단은 멀쩡한
                //   인자를 조용히 지운다.
                args.extend(extra_args.iter().cloned());
                // ★알려진 한계 — `%VAR%` 가 든 경로는 shim 을 지나며 치환된다(2026-09-08, 고치지 않기로
                //   한 결정)★. 아래 `console_command` 가 Windows 에서 `cmd.exe /c` 로 감싸는데, cmd 는
                //   명령줄의 `%NAME%` 을 **따옴표 안에서도** 환경변수로 편다. 그래서 이름에 `%…%` 가
                //   들어간 실제 폴더(`C:\x\%USERNAME%\y`)를 받으면 codex 는 **다른 폴더**를 워크스페이스로
                //   본다. 터미널 모드에서는 위 `CD_FLAG` 값과 `extra_args` 가, app-server 모드에서는
                //   `extra_args` 만 그 경로를 탄다(그 모드의 작업 폴더는 명령줄이 아니라 `thread/start`
                //   파라미터로 가므로 cmd 를 지나지 않는다).
                //   ★왜 안 고치나★ — ① shim 은 걷을 수 없다(사용자 결정 · TRD §6-H: `codex` 를 이름
                //   그대로 띄운다. PATH 의 `codex` 는 `.cmd` 라 `CreateProcessW` 가 직접 못 띄운다)
                //   ② 알려진 해법이 **일반적인 이스케이프가 아니다**: rust std 가 `.bat` 을 띄울 때 쓰는
                //   수법은 `%` 마다 `%%cd:~,%`(no-op 치환)를 끼워 넣는 것이고 `cmd /e:ON` + `/c "한 문자열"`
                //   형태를 전제한다(RUSTSEC-2024-0006 대응). 우리 경로는 그 형태가 아니고(인자 배열을
                //   portable-pty 가 직접 조립해 `CreateProcessW` 로 넘긴다 — `%` 처리 0줄), 그 수법을 그
                //   전제 밖으로 옮기는 것은 검증되지 않은 이식이다 ③ 그 이식은 **claude 와 공용인**
                //   `console_command` 를 건드리므로, 틀리면 오늘 도는 스폰 전부가 죽는다.
                //   ★고칠 때 무엇이 필요한가★: 위 셋을 뒤집을 실측(그 형태로 감싼 `cmd` 아래에서 claude·
                //   codex 가 둘 다 뜨는가)이다. 그 전에는 이 한계를 아는 채로 둔다.
                // ADR-0004

                // ★MCP 서버 부착도 같은 자리·같은 규율이다(패스스루 **뒤**)★ — 아래 상태줄 블록의
                //   순서 주석이 그 사유의 정본이고 여기 되풀어 적지 않는다. ★그런데 **모드를 가르지
                //   않는다**★: 우편 입구는 두 모드 다 필요하다. 모드를 가르는 것은 상대가 같은 값을
                //   **다른 수단으로 이미 주는** 축(세션 id · 프라이밍)이고, 우편은 그 축에 안 든다.
                // ★이 한 값이 codex 우편의 **유일한 물리 배선**이다★ — claude 는 mcp-config 파일을 읽지만
                //   codex 는 이 오버라이드만 먹는다(실측 0.155.0). 빠지면 `eg_send` 툴이 아예 없고,
                //   데몬은 「MCP 로 우편을 쓴다」고 판정해(`mail_allowed=false`) CLI 미러까지 닫으므로
                //   **발신 입구가 0** 이 된다. 그 결말은 오류가 아니라 침묵이다.
                // ★우리 것이 뒤에 실리는 성질을 순서로 얻는다★ — 같은 키를 `-c` 로 두 번 넘기면 마지막이
                //   이긴다(실측 · ADR-0216 이 물려받은 「override 는 passthrough 뒤 · 덮으면 경고」).
                // ADR-0128
                // ADR-0209
                // ★「안 붙인다」의 두 갈래를 여기서 **가르지 않는다**★ — 정당한 부재
                //   ([`McpAttachment::NotWanted`])든 못 만든 것([`McpAttachment::Unrepresentable`])이든
                //   이 자리가 하는 일은 같다(인자를 안 싣는다). 못 만든 쪽을 스폰 중단으로 끊는 자리는
                //   [`AgentBackend::precheck_control_endpoint`] 이고, 이 함수는 인자만 만든다(바로 아래
                //   상태줄·지시서 블록이 신고 자리를 조립점으로 미루는 것과 같은 규율).
                match mcp_attachment(control.as_ref()) {
                    McpAttachment::Attach(value) => {
                        if passthrough_overrides_key(
                            extra_args,
                            &format!("{MCP_SERVER_OVERRIDE_PREFIX}{MCP_SERVER_NAME}"),
                        ) {
                            tracing::warn!(
                                "codex 패스스루가 `{MCP_SERVER_OVERRIDE_PREFIX}{MCP_SERVER_NAME}` 을 직접 세웠다 — 우편 입구를 위해 우리 부착을 뒤에 실어 그 값을 덮는다(마지막 `-c` 가 이긴다)"
                            );
                        }
                        args.push(CONFIG_OVERRIDE_FLAG.to_string());
                        args.push(value);
                    }
                    McpAttachment::NotWanted | McpAttachment::Unrepresentable(_) => {}
                }

                // ★★오버라이드는 전부 패스스루 **뒤**다 — 이 자리가 그 규율의 정본이다★★(형제 블록
                //   둘이 여기를 가리킨다). 실측(codex-cli 0.155.0 · 2026-09-19): 같은 설정 키를 `-c` 로
                //   두 번 넘기면 **마지막 것이 이긴다** — 병합도 없고 중복 키 오류도 없다. 그래서 앞에
                //   두면 사용자 인자 한 줄(`-c tui.status_line=[]`)이 우리 값을 통째로 지우고, 그 결말은
                //   화면에도 로그에도 아무 신호가 없다.
                //   ★그러니 「인자 조립을 정돈」한답시고 이 블록들을 위 터미널 갈래로 되돌리지 말 것★.
                // ★터미널 모드에만 얹는다★ — app-server 모드의 thread id 는 `thread/start` 응답으로
                //   구조화된 채 오므로 화면을 읽을 이유가 없고, 얹으면 한 spawn 이 같은 값을 두 경로로
                //   적으려 든다.
                // ★제어 채널 유무를 **안 본다** — 형제 오버라이드와 갈리는 지점이다★: MCP 부착은
                //   endpoint 에서 재료(url)를 받아야 서지만, 이 값은 상수라 받을 재료가 없다. 그래서
                //   제어 평면이 없는 스폰에서도 실린다.
                // ★★사용자 값을 덮으면 **말한다 — 조용한 덮어쓰기는 금지다**★★(ADR-0216 결정 1).
                //   ★그래도 거르지는 않는다★: 패스스루를 지우는 것은 사용자 인자를 우리가 검열하는
                //   것이고, 우리 값을 접으면 세션 id 회수가 통째로 죽는다. 둘 다 안 하고 이긴 사실만
                //   남긴다(이 자리도 형제 블록들의 정본이다).
                // ★`-c` 는 리스트를 병합하지 못한다★ — 그래서 사용자가 자기 상태줄 항목을 세워 뒀다면
                //   그 항목들은 이 스폰에서 사라진다(다음 스폰이 아니라 이 스폰에서만).
                // ADR-0216
                if matches!(output_format, AgentOutputFormat::Terminal) {
                    if passthrough_overrides_key(extra_args, STATUS_LINE_KEY) {
                        tracing::warn!(
                            "codex 패스스루가 `{STATUS_LINE_KEY}` 을 직접 세웠다 — 세션 id 회수를 위해 우리 값을 뒤에 실어 그 값을 덮는다(마지막 `-c` 가 이긴다)"
                        );
                    }
                    args.push(CONFIG_OVERRIDE_FLAG.to_string());
                    args.push(STATUS_LINE_OVERRIDE.to_string());
                }

                // ★`SessionStart` 훅을 여기 되살리지 말 것(ADR-0216)★ — 회수 경로가 위 상태줄이고
                //   훅 기계장치는 저장소에서 걷혔다. 훅이 **고장 나서**가 아니다(돌았다 — 실측
                //   0.155.0): 기각 사유는 codex 가 훅 **명령 문자열**을 해싱해 사람 승인을 요구하는데
                //   그 문자열에 우리 exe 절대경로가 박혀 있다는 것 하나다. 배포가 태그 이름 폴더로
                //   풀리므로 경로가 릴리스마다 바뀌고, 그래서 릴리스마다 승인 화면이 다시 뜨며
                //   **그 화면이 떠 있는 동안 터미널 codex 가 아예 안 뜬다.**

                // ★프라이밍 주입 — 터미널 모드만 여기서 조립한다★: app-server 모드는 같은 내용을 명령줄이
                //   아니라 핸드셰이크 JSON(`thread/start` 의 `developerInstructions`)으로 싣는다
                //   ([`AgentBackend::open_spawn`]). 같은 값이 두 모드에서 서로 다른 수단으로 나가는
                //   것이지 두 번 나가는 것이 아니다 — 이어받기(`resume`)가 argv 와 핸드셰이크로 갈리는
                //   것과 같은 모양이다.
                // ★왜 모드를 가르나 — 훅과 달리 「중복이라서」가 아니다★: 이 명령줄에는 8191 자 상한이
                //   있고 지시서가 그 예산을 통째로 먹을 수 있다. JSON 본문에는 그 상한이 없어(8 MB 까지
                //   받는 것을 실측) 값을 손대지 않고 그대로 보낼 수 있다. 두 모드에 다 명령줄로 실으면
                //   app-server 쪽이 이유 없이 그 상한과 `%`·줄바꿈 위험을 진다.
                // ★패스스루 **뒤**다★ — 같은 키를 `-c` 로 두 번 넘기면 마지막이 이긴다(위 두 블록의 같은
                //   규율). 사유 정본은 위 상태줄 블록의 순서 주석이고 여기 되풀어 적지 않는다.
                // ★어느 갈래로도 스폰을 실패시키지 않는다★ — 파일을 못 읽어도, 실을 수 없는 문자가 있어도,
                //   예산을 넘어도 경고만 남기고 인자를 안 싣는다(ADR-0216 이 물려받은 규율). 프라이밍은
                //   있으면 좋은 것이고 스폰은 필수다.
                // ADR-0004
                // ADR-0092
                // ADR-0215
                // ADR-0216
                if matches!(output_format, AgentOutputFormat::Terminal) {
                    if let Some(text) = priming_text(control.as_ref()) {
                        if let Some(value) = developer_instructions_override(&text, &args) {
                            if passthrough_overrides_key(extra_args, DEVELOPER_INSTRUCTIONS_KEY) {
                                tracing::warn!(
                                    "codex 패스스루가 `{DEVELOPER_INSTRUCTIONS_KEY}` 을 직접 세웠다 — 수신 계약 프라이밍을 위해 우리 값을 뒤에 실어 그 값을 덮는다(마지막 `-c` 가 이긴다)"
                                );
                            }
                            args.push(CONFIG_OVERRIDE_FLAG.to_string());
                            args.push(value);
                        }
                    }
                }

                // ADR-0086 스텝 2(CLI 입구) — ★모드를 가르지 않는다★: 심는 것은 env 세 값뿐이고 그
                //   값을 읽는 것은 codex 가 아니라 **codex 가 띄우는 자식들**(셸 도구 · 사용자 자신의 훅)이다.
                //   두 모드 다 자식을 띄우므로 갈릴 축이 없다.
                // ★`config_path`·`settings_file` 은 **안 온다 — 그리고 그것이 선언된 결과다**★: 둘 다
                //   claude 가 **경로로 읽는 파일**이고 codex 에는 그 플래그가 없어서(실측 0.155.0:
                //   `--mcp-config` 도 `--settings` 도, 설정 파일을 가리키는 `--config <파일>` 도 없다)
                //   이 폴더가 [`AgentBackend::writes_mcp_config_file`] 를 false 로 선언하고, 데몬이 그
                //   칸 하나로 write 를 가른다. 이 백엔드의 MCP 부착은 위 `-c` 한 값 단독이다.
                //   ★그 칸이 갈라지기 전에는 스폰마다 **아무도 안 읽는 평문 토큰 파일이 하나 생겼다**★ —
                //   데몬의 파생 축이 「MCP 로 우편을 쓰나」와 「우리 mcp-config 파일을 먹나」를 한 칸으로
                //   겸했기 때문이다. 그 겸직을 되살리면(= 그 칸을 지우거나 `accepts_mcp_config` 로
                //   파생하면) 낭비도 fail-closed 스폰 중단 위험도 그대로 돌아온다.
                //   ★그래도 아래 주입은 두 칸이 `Some` 으로 와도 무시한다 — 그 방어를 걷지 말 것★:
                //   제어 채널은 이 backend 하나만 쓰는 게 아니고, 「Some 이니 뭔가 쓰자」로 claude 의
                //   플래그를 베껴 붙이면 기동이 죽는다(그 단언 = `the_daemon_written_files_are_still_not_translated_into_argv`).
                // ★`grants` 도 안 쓴다★ — 그 목록은 권한 프롬프트의 **사전 승인**이지 툴 허용 목록이
                //   아니고(ADR-0094), codex 설정의 `enabled_tools` 로 번역하면 뜻이 「미리 승인」에서
                //   「이것만 존재」로 바뀐다. 다른 뜻의 값을 같은 값이라 부르지 않는다.
                // ★`priming_file` 은 **쓴다 — 단 경로가 아니라 내용을 쓴다**★: 경로를 받는 지시문 키는
                //   `model_instructions_file` 하나뿐인데 그것은 덧붙이기가 아니라 **대체**라 걸면 codex
                //   자신의 운용 지시가 사라지고, `developer_instructions_file` 은 키 자체가 없다(실측
                //   0.155.0). 그래서 이 backend 가 그 파일을 **읽어** 위 [`DEVELOPER_INSTRUCTIONS_KEY`]
                //   (터미널) 또는 `thread/start` 의 `developerInstructions`(app-server)로 싣는다.
                //   ★「내용을 읽는 것은 ADR-0092 에 어긋난다」로 되돌리지 말 것★ — 그 ADR 이 소비
                //   프로그램에 준 역할은 **경로 해석**이고, 그 역할은 데몬의 provider 가 여전히 단독으로
                //   진다(이 폴더는 실려 온 경로를 읽을 뿐 어느 파일인지 고르지 않는다). 읽는 자리가
                //   backend 폴더인 것은 ADR-0004 그대로다 — 「어느 키에 어떤 모양으로 싣나」가 여기 말고는
                //   갈 데가 없다.
                // ADR-0086 / ADR-0092 / ADR-0133 / ADR-0209
                if let Some(endpoint) = &control {
                    inject_cli_entrance(&mut env, endpoint);
                }
                let (program, args) = console_command(CODEX_PROGRAM, args);
                CommandSpec {
                    program,
                    args,
                    env,
                    cwd,
                }
            }
            AgentCommand::Claude { .. } | AgentCommand::Shell { .. } => {
                unreachable!("CodexBackend 는 Codex variant 만 처리한다. dispatch 버그.")
            }
        }
    }

    /// `session.resume` 이 켜진 근거는 ★발급 주체와 무관하다★ — 복원은 프로필에 저장된 backend sid
    /// **단독**에 의존하고 그 sid 를 누가 발급하는지는 백엔드가 정한다. codex 는 받아 쓰는 쪽이고
    /// (app-server 는 `thread/start` 응답으로, 터미널 모드는 TUI 상태줄을 읽어 — ADR-0216), 그 값으로 두 모드 다
    /// 이어받는다.
    /// ★그래서 이 칸도 모드를 안 가른다 — 위 [`AgentBackend::can_resume_stored_session`] 과 **같은
    ///   술어다. 한쪽만 건드리지 말 것**★: 그 축은 활성화 입구가 「Resume 으로 띄울까」를 묻는 자리이고
    ///   이 칸은 그 결과를 소비자에게 신고하는 자리라, 갈리면 새 스레드가 「이어받음」으로(또는 그
    ///   반대로) 보고된다.
    /// `model.select` 는 codex 에 `-m` 이 있는데도 false 다 — 이 칸은 **그 프로그램이 할 수 있는 것**이
    /// 아니라 **이 스폰이 쓰는 것**을 신고한다. 그 칸을 노출하지 않으므로 신고하지 않는다.
    // ADR-0185
    // ADR-0208/ADR-0210
    fn capabilities(&self, _command: &AgentCommand) -> BackendCaps {
        BackendCaps {
            session: SessionCaps {
                resume: true,
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

    /// app-server 모드는 봉투가 **양방향**으로 흐르는 파이프를 요구한다 — 상대가 우리에게도 묻는다.
    ///
    /// ★신고값일 뿐이고 실물은 아래 [`AgentBackend::open_spawn`] 이 만든다★ — 둘 다 [`is_app_server`] 를
    ///   보므로 한쪽만 고치면 신고와 실물이 어긋난다.
    fn transport_shape(&self, command: &AgentCommand) -> TransportShape {
        if is_app_server(command) {
            TransportShape::StdioBidiJson
        } else {
            TransportShape::Pty
        }
    }

    /// app-server 모드는 양방향 JSON 통로를, 터미널 모드는 PTY 를 만든다.
    ///
    /// ★판정은 [`is_app_server`] 단독 — `transport_shape` 신고값을 되읽어 match 하지 않는다(ADR-0191)★:
    ///   그렇게 하면 모양 값을 가르는 둘째 switch 가 생긴다.
    /// ★`structured: true` 와 `thread/start` 정책을 주입하는 자리가 여기다(ADR-0044/0030)★: 통로는 자기가
    ///   나르는 바이트가 무엇인지도, 어느 폴더를 어떤 샌드박스로 열어야 하는지도 모른다. 아는 쪽은 이
    ///   모드와 정책을 고른 이 backend 다.
    /// ★`sid_sink` 가 두 갈래로 **다 간다 — 나르는 수단만 다르다**★: app-server 는 `thread/start`
    ///   응답으로 **받아** 통로가 그대로 넘기고, 터미널은 우리가 켠 TUI 상태줄을 **읽어**
    ///   ([`thread_id::observer`]) 넘긴다. 어느 쪽이든 그 세션으로 무엇을 보내기 전에 나간다
    ///   ([`AgentBackend::open_spawn`] 의 순서 계약).
    ///   ★「터미널 갈래엔 이 통로로 받아 올 식별자가 없다」로 적혀 있던 옛 문장은 더 이상 참이 아니다★ —
    ///   그 시절의 입구는 codex 가 띄우는 `SessionStart` 훅이었고(ADR-0208/ADR-0210), ADR-0216 이 그
    ///   승인 게이트를 피해 회수를 이 통로로 옮겼다.
    /// ★관찰자는 **터미널 갈래에만** 꽂는다★ — app-server 모드는 PTY 로 뜨지도 않고, 그쪽 id 는 이미
    ///   구조화 응답으로 온다. 두 수단이 겹치면 한 spawn 이 같은 값을 두 경로로 적으려 든다.
    /// ★`thread/start` 냐 `thread/resume` 이냐를 고르는 자리도 여기다★ — 조립점이 넘긴
    ///   `resume_session_id` 하나로 갈린다([`thread_open`]). ★통로에게 다시 묻지 않는다★: 통로가 자기
    ///   상태를 보고 판정하면 가르는 자리가 둘이 된다.
    /// ★`resume_session_id` 를 터미널 갈래에서는 쓰지 않는다 — 그런데 사유는 「손잡이가 없어서」가
    ///   **아니다**★: 그 모드의 이어받기는 이미 argv 에 실려 나갔다([`AgentBackend::build_spec`] 의
    ///   터미널 갈래). 같은 값을 여기서 또 쓰면 한 spawn 이 두 수단으로 이어받으려 든다.
    /// ★`control` 도 **같은 규율로 app-server 갈래에서만 읽는다**★ — 여기서 꺼내는 것은 프라이밍 내용
    ///   하나이고([`priming_text`]), 터미널 갈래는 그것을 이미 `-c developer_instructions=…` 로 실어
    ///   보냈다. 그래서 PTY 쪽 생성자에는 이 칸이 닿지 않는다.
    // ADR-0185
    // ADR-0191
    // ADR-0215
    fn open_spawn(
        &self,
        command: &AgentCommand,
        spec: &CommandSpec,
        cols: u16,
        rows: u16,
        sid_sink: Option<SessionIdSink>,
        resume_session_id: Option<Uuid>,
        link_sink: Option<LinkSink>,
        control: Option<&ControlEndpoint>,
    ) -> Result<SpawnParts, PtyError> {
        let (transport, child_pid): (Box<dyn AgentTransport>, Option<u32>) =
            if is_app_server(command) {
                let (t, pid) = CodexAppServerTransport::open(
                    spec,
                    true,
                    self.output_decoder(command),
                    thread_open(spec, resume_session_id, priming_text(control)),
                    sid_sink,
                    link_sink,
                )?;
                (Box::new(t), pid)
            } else {
                // ★터미널 모드에는 세울 연결이 없다★ — `declares_link()` 가 false 라 포트도 `None` 이다.
                // ★기록 수단이 없으면 관찰자도 안 꽂는다★ — 건져 봐야 적을 곳이 없고, 그 갈래의 pump 는
                //   바이트 단위로 예전과 같은 길을 간다.
                // ADR-0216
                let (t, pid) = PtyTransport::open_with_observer(
                    spec,
                    cols,
                    rows,
                    sid_sink.map(thread_id::observer),
                )?;
                (Box::new(t), pid)
            };
        Ok(SpawnParts {
            transport,
            child_pid,
            backend_caps: self.capabilities(command),
            encoder: self.input_encoder(command),
            turn_classifier: self.turn_classifier(),
            reads_messages: self.reads_messages(),
        })
    }

    /// app-server 모드에서는 봉투를 **통로가** 만든다 — 인코더가 감쌀 것이 없다.
    fn input_encoder(&self, command: &AgentCommand) -> InputEncoder {
        if is_app_server(command) {
            InputEncoder::TransportFramed
        } else {
            InputEncoder::Raw
        }
    }

    fn turn_classifier(&self) -> TurnClassifier {
        classify_turn
    }

    // ★합성 입력 에코를 선언하지 않는다 — 되살리지 말 것★: 이 백엔드는 유저 메시지를 스스로
    //   `item/*` 의 `userMessage` item 으로 되울리고(실측), 번역기가 그것을 「우리가 보낸 것」으로
    //   표시해 흘린다. 여기에 합성 에코를 더하면 같은 질문이 화면에 두 벌 남는다 — 둘의 dedup 키가
    //   다르기 때문이다(합성 쪽은 우리 uuid, 되울린 쪽은 codex item id). 그 겹침이 ADR-0193 의
    //   「거부한 대안」이 실측을 근거로 기각한 바로 그것이다.
    // ADR-0193

    /// ★선언하지 않으면 번역기가 조립되지 않고 바이트가 그대로 흘러 화면이 깨진다 — 오류도 경고도 없다★.
    fn output_decoder(&self, command: &AgentCommand) -> Option<Box<dyn OutputDecoder>> {
        if is_app_server(command) {
            Some(Box::new(CodexAppServerDecoder::new()))
        } else {
            None
        }
    }
}

/// codex 출력 이벤트 → 턴 신호(ADR-0113).
///
/// ★이 백엔드의 종료 신호는 [`OutputEvent::TurnEnd`] 다★ — 번역기는 `MessageDone` 을 내지 않는다.
///   그래도 그 갈래를 `Ended` 로 적어 둔다: 뜻이 같은 두 어휘를 여기서 갈라 적으면, 어느 날 이 백엔드가
///   그것을 내게 됐을 때 종료가 조용히 사라진다.
/// ★`Structured` 를 진행으로 세는 것이 이 백엔드에서 load-bearing 이다★: 이 갈래로 들어오는 것은
///   되울린 유저 메시지(`userMessage` item 번역분) 하나뿐이고 — 모르는 알림은 버리므로(이 폴더
///   `decoder` 헤더) 그 밖엔 없다 — 그것이 「이 턴이 시작됐다」의 첫 관측이다.
/// ★그래도 「우리가 `turn/start` 를 쓴 순간부터 그 첫 알림이 도착하기까지」는 미관측이다★ — 그 창을
///   합성 에코로 메우지 않는다(ADR-0193 이 그 대안을 기각했다). 큐 해제의 「지금 보낼 수 있나」는 통로
///   구현체가 자기 상태로 답하고, 우편 게이트 쪽 미관측은 fail-open 으로 흡수된다(ADR-0104 의 선택).
/// ★`Usage`/`Error` 가 종료가 아닌 이유★: `Usage`(`thread/tokenUsage/updated`)는 턴 중간에 오고,
///   `Error`(`error` 알림)는 재시도 가능한 스트림 오류라 턴 경계가 아니다. 실패한 턴도 `turn/completed`
///   가 내는 `TurnEnd` 로 닫힌다 — 실패 사유는 그 이벤트 **안에** 실린다.
/// ★`turn/completed` 가 **아예 오지 않는** 포기 경로도 `TurnEnd` 로 닫힌다★(쓰기 실패·응답 해독 실패
///   ·오류 응답·시한 만료 — 통로의 `end_turn_if`). 그 자리에 `Error` 를 쓰던 시절에는 이 표가 종료를
///   못 세서 **한가한 에이전트가 턴 중으로 관측된 채 남아** 30 분 fail-open 밸브까지 우편이 막혔다
///   (ADR-0127). 그래서 이 매핑에서 `Error` 를 종료로 승격시키는 것이 아니라, 그쪽이 경계 어휘를 쓴다.
/// ★터미널 모드와 공유해도 되는 이유(모드별 분기 불필요)★: 그 모드는 decoder 가 없어 `TerminalBytes` 만
///   흐르므로 이 매핑을 그대로 써도 신호가 하나도 나오지 않는다.
/// ★`Ended` 앞에 `Progress` 가 없어도 안전하다(코드 근거)★: 표는 `Ended` 를 `in_turn = false` 로 적을
///   뿐이라 짝 없는 종료는 등록 직후 상태와 같은 값을 쓰고(`crate::turn::TurnObservations::observe_at`),
///   `in_turn_snapshot` 은 `in_turn` 인 것만 싣는다. 그래서 「시작 신호」를 지어내 채울 이유가 없다.
// ADR-0113
// ADR-0004
pub(crate) fn classify_turn(event: &OutputEvent) -> Option<TurnSignal> {
    match event {
        OutputEvent::TextDelta { .. }
        | OutputEvent::ToolCall { .. }
        | OutputEvent::Structured { .. } => Some(TurnSignal::Progress),
        OutputEvent::TurnEnd { .. } | OutputEvent::MessageDone { .. } => Some(TurnSignal::Ended),
        OutputEvent::Usage { .. } | OutputEvent::Error(_) | OutputEvent::TerminalBytes(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        ToolGrant, TurnOutcome, CLI_EXE_ENV, MAIL_MARKER_ENV, MAIL_MARKER_OFF, MAIL_MARKER_ON,
    };

    fn codex(extra_args: Vec<&str>) -> AgentCommand {
        AgentCommand::Codex {
            extra_args: extra_args.into_iter().map(String::from).collect(),
            output_format: AgentOutputFormat::Terminal,
        }
    }

    fn codex_app_server(extra_args: Vec<&str>) -> AgentCommand {
        AgentCommand::Codex {
            extra_args: extra_args.into_iter().map(String::from).collect(),
            output_format: AgentOutputFormat::StreamJson,
        }
    }

    fn spec(command: &AgentCommand, cwd: &str) -> CommandSpec {
        CodexBackend.build_spec(
            command,
            SpawnMode::Fresh,
            None,
            None,
            PathBuf::from(cwd),
            vec![],
            None,
        )
    }

    /// `console_command` 래핑을 걷어 낸 codex 자신의 argv. Windows 는 `cmd.exe /c codex …` 로 한 겹
    /// 감싸이므로(shim 해석) 두 플랫폼의 단언을 하나로 쓰려면 그 겹을 여기서 벗긴다.
    fn codex_argv(spec: &CommandSpec) -> Vec<String> {
        #[cfg(windows)]
        {
            assert_eq!(spec.program, "cmd.exe");
            assert_eq!(spec.args[0], "/c");
            assert_eq!(spec.args[1], CODEX_PROGRAM);
            spec.args[2..].to_vec()
        }
        #[cfg(not(windows))]
        {
            assert_eq!(spec.program, CODEX_PROGRAM);
            spec.args.clone()
        }
    }

    /// ★이 값은 **언제나 `None`** 이어야 한다 — 훅 등록도 그 빌더도 걷혔다(ADR-0216)★. 남은 쓸모는
    /// **되살아나지 않았다**를 재는 것 하나다.
    /// ★키를 리터럴로 적는 것이 의도다★ — 운영 코드에 이 문자열이 한 글자도 없는 것이 지금의 불변식이라,
    ///   상수를 되살려 참조하면 그 사실이 흐려진다.
    /// ★`-c` 의 **존재**로는 아무것도 세지 말 것★ — 오늘 그 플래그를 싣는 것이 둘이다(MCP 부착 ·
    ///   상태줄). 플래그만 세던 옛 형태는 둘째가 들어올 때마다 거짓 양성이 됐다.
    fn hook_override_value(argv: &[String]) -> Option<String> {
        argv.windows(2)
            .find(|p| p[0] == CONFIG_OVERRIDE_FLAG && p[1].starts_with("hooks.SessionStart"))
            .map(|p| p[1].clone())
    }

    /// ★상태줄 오버라이드가 여기 실리는 것은 **제어 평면과 무관하다**★ — 이 fixture 는 endpoint 가
    /// `None` 인데도 그 두 칸이 선다(형제 오버라이드 둘은 재료가 없어 빠진다).
    #[test]
    fn interactive_args_are_the_measured_ones() {
        let s = spec(&codex(vec![]), "C:/workspace");
        assert_eq!(
            codex_argv(&s),
            vec![
                "--cd",
                "C:/workspace",
                "-s",
                "workspace-write",
                "-a",
                "on-request",
                CONFIG_OVERRIDE_FLAG,
                STATUS_LINE_OVERRIDE,
            ]
        );
    }

    #[test]
    fn app_server_args_are_the_measured_ones() {
        let s = spec(&codex_app_server(vec![]), "C:/workspace");
        assert_eq!(codex_argv(&s), vec!["app-server", "--stdio"]);
    }

    /// ★정책과 작업 폴더는 argv 로 가지 않는다★ — app-server 는 그 셋을 `thread/start` 파라미터로 받는다.
    /// 여기 실리면 상대가 그 하위 명령에서 모르는 인자를 받아 기동이 실패한다.
    #[test]
    fn app_server_argv_carries_no_policy_and_no_workspace_path() {
        let s = spec(&codex_app_server(vec![]), "C:/workspace");
        let argv = codex_argv(&s);
        for forbidden in [CD_FLAG, SANDBOX_FLAG, APPROVAL_FLAG, "C:/workspace"] {
            assert!(
                !argv.iter().any(|a| a == forbidden),
                "`{forbidden}` 가 app-server argv 에 실렸다: {argv:?}"
            );
        }
    }

    /// 프로세스 cwd 는 두 모드가 같다 — 워크스페이스 폴더를 어디로 나르든 프로세스는 같은 곳에서 뜬다.
    #[test]
    fn process_cwd_is_the_same_in_both_modes() {
        let cwd = PathBuf::from("C:/workspace");
        assert_eq!(spec(&codex(vec![]), "C:/workspace").cwd, cwd);
        assert_eq!(spec(&codex_app_server(vec![]), "C:/workspace").cwd, cwd);
    }

    /// ★「맨 뒤」가 아니라 「우리 기본 인자 **뒤**」다 — 옛 이름(`extra_args_come_last`)은 우리
    /// 오버라이드가 생기기 전의 사실이었다★. 오버라이드가 패스스루보다 뒤에 서야 하는 것은 별개
    /// 축이고, 그쪽은 `…_follows_the_passthrough` 항목들이 잰다.
    #[test]
    fn extra_args_follow_our_base_args() {
        let s = spec(&codex(vec!["-m", "gpt-5"]), "C:/workspace");
        let argv = codex_argv(&s);
        let at = argv
            .iter()
            .position(|a| a == "-m")
            .expect("패스스루가 사라졌다");
        assert_eq!(&argv[at..at + 2], &["-m".to_string(), "gpt-5".to_string()]);
        assert!(
            argv[..at].iter().any(|a| a == APPROVAL_ON_REQUEST),
            "패스스루가 우리 기본 인자 앞으로 갔다: {argv:?}"
        );
    }

    /// ★세션 인자가 조립되면 안 된다★ — 존재하지 않는 문법이라 붙는 순간 codex 가 기동에 실패한다.
    #[test]
    fn session_id_never_reaches_the_command_line() {
        let sid = Uuid::new_v4();
        let s = CodexBackend.build_spec(
            &codex(vec![]),
            SpawnMode::Resume,
            Some(sid),
            None,
            PathBuf::from("."),
            vec![],
            None,
        );
        assert!(
            !s.args.iter().any(|a| a.contains(&sid.to_string())),
            "sid 가 argv 에 실렸다: {:?}",
            s.args
        );
        assert!(
            !s.args
                .iter()
                .any(|a| a == "--session-id" || a == "--resume" || a == "--session"),
            "존재하지 않는 세션 플래그가 실렸다: {:?}",
            s.args
        );
    }

    // ── ADR-0208: 터미널 모드 이어받기(`codex resume <id>`) ──────────────────────

    /// 이어받기 칸을 채워 뽑은 spec — 발급 칸(`session_id`)은 그대로 비워 둔다.
    ///
    /// ★두 칸을 **따로** 넘기는 것이 이 헬퍼의 요점이다★ — 하나로 접으면
    /// [`AgentBackend::assigns_session_id`] 가 false 인 백엔드의 argv 에 우리가 발급한 값이 실릴 수
    /// 있고, 아래 항목들은 그 접힘을 못 본다.
    fn spec_resuming(
        command: &AgentCommand,
        mode: SpawnMode,
        resume_session_id: Option<Uuid>,
        cwd: &str,
    ) -> CommandSpec {
        CodexBackend.build_spec(
            command,
            mode,
            None,
            resume_session_id,
            PathBuf::from(cwd),
            vec![],
            None,
        )
    }

    /// ★하위 명령과 위치 인자가 **맨 앞에 붙어** 나가야 한다★ — `resume` 가 뒤로 밀리면 clap 이 그것을
    /// 루트의 `[PROMPT]` 로 읽어 이어받기가 조용히 새 대화가 되고, id 가 하위 명령과 떨어지면
    /// `[SESSION_ID]` 자리를 놓친다.
    /// 정책 셋은 **그대로** 뒤에 실린다 — `resume` 도 그 셋을 자기 옵션으로 받는다(실측 0.155.0).
    #[test]
    fn resuming_with_a_stored_handle_emits_the_subcommand_and_the_positional_id() {
        let thread_id = Uuid::new_v4();
        let s = spec_resuming(
            &codex(vec![]),
            SpawnMode::Resume,
            Some(thread_id),
            "C:/workspace",
        );
        assert_eq!(
            codex_argv(&s),
            vec![
                "resume",
                &thread_id.to_string(),
                "--cd",
                "C:/workspace",
                "-s",
                "workspace-write",
                "-a",
                "on-request",
                CONFIG_OVERRIDE_FLAG,
                STATUS_LINE_OVERRIDE,
            ]
        );
    }

    /// ★손잡이가 없으면 **Fresh 와 바이트 단위로 같은** argv 여야 한다★ — id 없는 `codex resume` 는 TUI
    /// 피커라(실측 0.155.0 `--help`: "picker by default") 그 화면에 붙은 에이전트는 첫 화면에서 영영
    /// 멈춘다. 그래서 하위 명령만 내보내는 반쪽 조립을 금지한다.
    #[test]
    fn resuming_without_a_stored_handle_falls_back_to_the_fresh_argv() {
        let fresh = codex_argv(&spec(&codex(vec![]), "C:/workspace"));
        let fell_back = codex_argv(&spec_resuming(
            &codex(vec![]),
            SpawnMode::Resume,
            None,
            "C:/workspace",
        ));
        assert_eq!(
            fell_back, fresh,
            "손잡이 없는 Resume 이 Fresh 와 다른 argv 를 냈다 — 반쪽 조립이면 TUI 피커에 걸린다"
        );
        assert!(
            !fell_back.iter().any(|a| a == RESUME_SUBCOMMAND),
            "id 없이 하위 명령만 실렸다 — 피커가 떠 에이전트가 첫 화면에서 멈춘다: {fell_back:?}"
        );
    }

    /// ★그 퇴행의 **신고**는 이 파일이 하지 않는다 — 조립점이 한다★:
    /// [`crate::manager::AgentManager`] 의 `resume_no_fallback` 이 `opens_a_new_conversation` 을 세 항의
    /// 곱으로 판정해 경고를 남기고 결말을 `Resumed` 가 아니라 `Started` 로 낸다. 그중 **둘이 이 파일의
    /// 선언**이라 여기서 못 박는다.
    /// ★실행 단언으로는 이 회귀가 안 잡힌다★ — 어느 쪽이 뒤집혀도 argv 는 위 항목대로 새 대화로 멀쩡히
    ///   떨어지고, 거짓이 되는 것은 **보고뿐**이다(새 대화가 「이어받음」으로 나간다).
    // ADR-0208
    #[test]
    fn a_handleless_resume_still_trips_the_new_conversation_notice() {
        let terminal = codex(vec![]);
        assert!(
            !CodexBackend.assigns_session_id(&terminal),
            "발급 축이 켜지면 조립점이 `ensure_session_id` 로 손잡이를 만들어 줘, 「손잡이가 없다」는 \
             갈래 자체가 사라진다"
        );
        assert!(
            CodexBackend.can_resume_stored_session(&terminal),
            "이어받기 축이 꺼지면 그 곱이 언제나 거짓이라, 손잡이 없이 연 새 대화가 조용히 \
             「이어받음」으로 보고된다"
        );
    }

    /// ★Fresh 는 손잡이가 있어도 이어받지 않는다★ — 조립점이 Fresh 에서 이 칸을 비워 넘기는 것이 규율이지만
    /// (`AgentManager::spawn_agent`), 이 파일이 그 규율에 기대면 판정이 두 곳이 된다. 죽은 화신의 스레드로
    /// 새 대화를 열라는 요청이 바로 그 조합이다.
    #[test]
    fn a_fresh_spawn_ignores_a_stored_handle() {
        let s = spec_resuming(
            &codex(vec![]),
            SpawnMode::Fresh,
            Some(Uuid::new_v4()),
            "C:/workspace",
        );
        assert_eq!(
            codex_argv(&s),
            codex_argv(&spec(&codex(vec![]), "C:/workspace"))
        );
    }

    /// ★app-server 모드는 이어받기를 argv 로 내지 않는다★ — 그 모드의 수단은 핸드셰이크 둘째 요청
    /// (`thread/resume`)이고 [`AgentBackend::open_spawn`] 이 고른다. 여기 실리면 `app-server` 하위 명령이
    /// 모르는 인자를 받아 기동이 실패한다.
    #[test]
    fn the_app_server_argv_is_unchanged_by_a_stored_handle() {
        let s = spec_resuming(
            &codex_app_server(vec![]),
            SpawnMode::Resume,
            Some(Uuid::new_v4()),
            "C:/workspace",
        );
        assert_eq!(codex_argv(&s), vec!["app-server", "--stdio"]);
    }

    /// 패스스루는 이어받기 갈래에서도 하위 명령·정책 **뒤**다 — 앞으로 오면 하위 명령과 그 위치 인자를
    /// 갈라놓는다. (우리 오버라이드는 그 패스스루보다 더 뒤다 — `…_follows_the_passthrough` 항목들이
    /// 그 축을 잰다.)
    #[test]
    fn resume_argv_still_puts_the_passthrough_after_our_base_args() {
        let s = spec_resuming(
            &codex(vec!["-m", "gpt-5"]),
            SpawnMode::Resume,
            Some(Uuid::new_v4()),
            "C:/workspace",
        );
        let argv = codex_argv(&s);
        assert_eq!(&argv[..1], &["resume".to_string()]);
        let at = argv
            .iter()
            .position(|a| a == "-m")
            .expect("패스스루가 사라졌다");
        assert_eq!(&argv[at..at + 2], &["-m".to_string(), "gpt-5".to_string()]);
        assert!(
            argv[..at].iter().any(|a| a == APPROVAL_ON_REQUEST),
            "패스스루가 우리 기본 인자 앞으로 갔다: {argv:?}"
        );
    }

    // ── CLI 입구 주입(ADR-0086 스텝 2 · ADR-0133) ────────────────────────────────

    fn endpoint() -> ControlEndpoint {
        ControlEndpoint {
            url: "http://127.0.0.1:7777/mcp".to_string(),
            token: "deadbeef".to_string(),
            // ★운영에서 이 칸은 `None` 으로 온다 — 여기 `Some` 은 **일부러** 넣은 최악값이다★:
            //   `writes_mcp_config_file()` 가 false 라 데몬은 이 backend 에 파일을 안 쓰고 두 칸을 비워
            //   보낸다. 그래도 fixture 가 `Some` 을 싣는 이유는 아래 시험이 재는 것이 「경로가 오면
            //   argv 로 새는가」이기 때문이다 — `None` 을 넣으면 그 단언이 공허해지고, 데몬 쪽 게이트가
            //   무너지는 날 이 backend 가 그 경로를 그대로 받아 쓰는 회귀를 아무도 못 잡는다.
            config_path: Some(std::path::PathBuf::from("C:/engram/mcp-config.json")),
            send_exe: Some(std::path::PathBuf::from("C:/engram/bin/engram.exe")),
            // ★이 경로는 **없는 파일**이고 그것이 의도다★: 이 backend 는 이제 지시서의 **내용**을 읽어
            //   싣는데(ADR-0215), 실물을 가리키면 이 fixture 를 쓰는 모든 시험의 argv 에 프라이밍
            //   오버라이드가 한 칸 더 붙어 인접·순서를 재는 단언들이 흔들린다. 읽기 실패는 fail-open 이라
            //   여기서는 「안 실린다」로 착지한다. 주입을 재는 항목은 [`bake_priming`] 으로 실물을 굽는다.
            priming_file: Some(std::path::PathBuf::from("C:/engram/priming.md")),
            // ★이 칸이 「데몬이 이 스폰에 MCP 발신 입구를 인가했다」의 실물이다 — 비워 두지 말 것★:
            //   빈 목록은 운영자가 MCP 우편을 **끈** 상태이고(그쪽 fixture 는 아래
            //   [`endpoint_without_mcp_send`]), 그 값으로 기본 fixture 를 만들면 MCP 부착을 재는 시험이
            //   전부 「안 붙는 게 맞다」쪽을 재게 되어 통째로 공허해진다.
            //   ★툴 이름을 여기 박는 것은 재타이핑이 아니다★ — 판정은 **서버명만** 본다
            //   (`ControlEndpoint::grants_mcp_send`). 툴 이름의 정본은 데몬 쪽이고 이 crate 는 그것을
            //   데이터로만 나른다.
            grants: vec![ToolGrant::Mcp {
                server: MCP_SERVER_NAME.to_string(),
                tool: "eg_send".to_string(),
            }],
            settings_file: Some(std::path::PathBuf::from("C:/engram/session.json")),
            // ★운영이 이 백엔드에 싣는 값이 `false` 다★ — 이 폴더가 그렇게 선언하고(`uses_mail`) 데몬이
            //   그 선언에서 파생한다. 아래 형제 시험이 반대 값도 따라간다는 것을 따로 잰다.
            mail_allowed: false,
        }
    }

    /// 운영자가 MCP 우편을 **끈** 스폰의 endpoint. 데몬은 그때도 제어 채널 자체는 발급하고(제어 동사는
    /// 전원 개방이다 — ADR-0132 결정 5) 발신 입구 grant 만 뺀다 — 그 결과가 이 모양이다.
    ///
    /// ★`ENGRAM_DISALLOW_MCP_SEND` 와 `ENGRAM_FORCE_CLI_ONLY_SEND` 가 서로 다른 자리를 건드리는데도 이
    ///   한 fixture 로 둘 다 대표되는 이유★: 전자는 `build_grants` 에서 MCP grant 를 빼고, 후자는
    ///   `accepts_mcp_config` 를 뒤집어 grant 가 CLI 쪽으로 가게 한다 — **둘 다 「우리 서버의 MCP 발신
    ///   grant 가 없다」로 착지한다.** backend 가 보는 것은 그 착지점 하나다.
    fn endpoint_without_mcp_send() -> ControlEndpoint {
        ControlEndpoint {
            grants: vec![],
            ..endpoint()
        }
    }

    fn spec_with_control(command: &AgentCommand, control: Option<ControlEndpoint>) -> CommandSpec {
        CodexBackend.build_spec(
            command,
            SpawnMode::Fresh,
            None,
            None,
            PathBuf::from("C:/workspace"),
            vec![],
            control,
        )
    }

    fn env_value<'a>(spec: &'a CommandSpec, key: &str) -> Option<&'a str> {
        spec.env
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// ★터미널 모드·app-server 모드를 **각각** 잰다★ — 한 모드만 재면 다른 모드의 스폰이 자격증명 없이
    /// 떠도 아무도 모른다. codex 가 띄우는 자식들은 이 env 를 상속으로만 받는다.
    #[test]
    fn both_modes_carry_the_control_plane_entrance() {
        for (label, command) in [
            ("terminal", codex(vec![])),
            ("app-server", codex_app_server(vec![])),
        ] {
            let s = spec_with_control(&command, Some(endpoint()));
            assert_eq!(
                env_value(&s, "ENGRAM_TOKEN"),
                Some("deadbeef"),
                "{label}: ENGRAM_TOKEN"
            );
            assert_eq!(
                env_value(&s, "ENGRAM_CONTROL_URL"),
                Some("http://127.0.0.1:7777"),
                "{label}: ENGRAM_CONTROL_URL = MCP url 에서 /mcp 를 벗긴 base"
            );
            assert_eq!(
                env_value(&s, MAIL_MARKER_ENV),
                Some(MAIL_MARKER_OFF),
                "{label}: 우편 표식은 endpoint 가 실어 온 값 그대로(운영값 = off)"
            );
            assert_eq!(
                env_value(&s, CLI_EXE_ENV),
                Some("C:/engram/bin/engram.exe"),
                "{label}: CLI 절대경로"
            );
        }
    }

    /// 표식은 **endpoint 가 실어 온 값**이지 이 백엔드가 파생하는 값이 아니다(ADR-0133 결정 2).
    ///
    /// ★그래서 운영값과 **반대**를 실어 잰다★: 여기서 파생을 하면 데몬 판정과 갈리는데, 운영값으로만
    ///   재면 그 갈림이 안 보인다(두 값이 우연히 같아서 통과한다).
    #[test]
    fn the_mail_marker_follows_the_endpoint() {
        let mut ep = endpoint();
        ep.mail_allowed = true;
        let s = spec_with_control(&codex(vec![]), Some(ep));
        assert_eq!(env_value(&s, MAIL_MARKER_ENV), Some(MAIL_MARKER_ON));
    }

    /// ★`--mcp-config` 는 claude 의 플래그다 — 이쪽으로 새면 codex 가 모르는 인자로 기동에 실패한다★.
    #[test]
    fn no_claude_flag_rides_along_with_the_entrance() {
        for command in [codex(vec![]), codex_app_server(vec![])] {
            let s = spec_with_control(&command, Some(endpoint()));
            for forbidden in [
                "--mcp-config",
                "--append-system-prompt-file",
                "--settings",
                "--allowedTools",
                "--disallowedTools",
            ] {
                assert!(
                    !s.args.iter().any(|a| a == forbidden),
                    "`{forbidden}` 가 codex argv 에 실렸다: {:?}",
                    s.args
                );
            }
        }
    }

    /// endpoint 가 없는 스폰(= 제어 채널을 못 받은 경우)은 env 가 **한 줄도 늘지 않는다**.
    #[test]
    fn without_an_endpoint_nothing_is_injected() {
        let s = spec_with_control(&codex(vec![]), None);
        assert!(
            s.env.is_empty(),
            "endpoint 부재인데 env 가 실렸다: {:?}",
            s.env
        );
    }

    // ── ADR-0216: thread id 상태줄 켜기(`-c` 오버라이드 단독) ───────────────────

    fn status_line_override_value(argv: &[String]) -> Option<String> {
        argv.windows(2)
            .find(|p| p[0] == CONFIG_OVERRIDE_FLAG && p[1].starts_with(STATUS_LINE_KEY))
            .map(|p| p[1].clone())
    }

    /// ★값을 바이트 단위로 못 박는다★ — 항목 이름이 틀려도 codex 는 오류를 안 내고 **조용히 무시**하고,
    /// 그러면 증상은 「세션 id 가 영영 안 온다」 하나뿐이다(ADR-0216 「근거」).
    /// ★작은따옴표도 함께 못 박힌다★ — 큰따옴표는 `cmd.exe /c` 래핑을 지나며 인자 경계를 다시 해석시킨다.
    #[test]
    fn the_terminal_spawn_turns_on_the_thread_id_status_line() {
        let argv = codex_argv(&spec(&codex(vec![]), "C:/workspace"));
        assert_eq!(
            status_line_override_value(&argv).as_deref(),
            Some("tui.status_line=['thread-id']"),
            "{argv:?}"
        );
    }

    /// 충돌 판정이 보는 키와 우리가 싣는 값이 **같은 키**여야 한다 — 갈리면 경고가 영영 안 뜬다.
    #[test]
    fn the_status_line_value_and_the_conflict_key_agree() {
        assert!(STATUS_LINE_OVERRIDE.starts_with(&format!("{STATUS_LINE_KEY}=")));
    }

    /// ★app-server 모드에는 안 얹는다★ — 그 모드의 thread id 는 `thread/start` 응답으로 구조화된 채
    /// 오므로 화면을 읽을 이유가 없고, 얹으면 한 spawn 이 같은 값을 두 경로로 적으려 든다.
    #[test]
    fn the_app_server_spawn_leaves_the_status_line_alone() {
        let argv = codex_argv(&spec_with_control(
            &codex_app_server(vec![]),
            Some(endpoint()),
        ));
        assert_eq!(status_line_override_value(&argv), None, "{argv:?}");
    }

    /// ★형제 오버라이드 둘과 갈리는 축★ — MCP 부착·훅 등록은 endpoint 에서 재료를 받아야 서지만, 이
    /// 값은 상수라 받을 재료가 없다. 제어 평면이 없는 스폰에서도 회수가 서야 한다.
    #[test]
    fn the_status_line_override_needs_no_control_endpoint() {
        let argv = codex_argv(&spec_with_control(&codex(vec![]), None));
        assert!(status_line_override_value(&argv).is_some(), "{argv:?}");
    }

    /// ★패스스루 **뒤**다★ — 앞에 두면 `-c tui.status_line=[]` 한 줄에 우리 값이 지고, 그 결말은
    /// 화면에도 로그에도 신호가 없다(같은 키를 두 번 넘기면 마지막이 이긴다).
    #[test]
    fn the_status_line_override_follows_the_passthrough() {
        let argv = codex_argv(&spec(&codex(vec!["-m", "gpt-5"]), "C:/workspace"));
        let ours = argv
            .iter()
            .position(|a| a.starts_with(STATUS_LINE_KEY))
            .expect("상태줄 오버라이드가 실려야 한다");
        let passthrough = argv
            .iter()
            .position(|a| a == "gpt-5")
            .expect("패스스루가 사라졌다");
        assert!(
            passthrough < ours,
            "패스스루가 상태줄 오버라이드 앞에 와야 한다: {argv:?}"
        );
    }

    /// ★사용자가 같은 키를 직접 세워도 우리 것이 이긴다 — 그리고 그 승리를 **말없이** 하지 않는다★.
    /// 거르지 않는 사유는 emission 자리 주석이 정본이다(사용자 인자를 우리가 검열하지 않는다).
    #[test]
    fn a_conflicting_status_line_passthrough_loses_to_ours_and_trips_the_warning() {
        assert!(
            passthrough_overrides_key(
                &[
                    CONFIG_OVERRIDE_FLAG.to_string(),
                    format!("{STATUS_LINE_KEY}=[]"),
                ],
                STATUS_LINE_KEY,
            ),
            "`-c {STATUS_LINE_KEY}=…` 를 못 알아봤다 — 경고 없이 사용자 설정을 덮는다"
        );
        let argv = codex_argv(&spec(
            &codex(vec![CONFIG_OVERRIDE_FLAG, "tui.status_line=[]"]),
            "C:/workspace",
        ));
        assert_eq!(
            argv.iter()
                .filter(|a| a.starts_with(STATUS_LINE_KEY))
                .count(),
            2,
            "사용자 것과 우리 것이 다 실려야 한다 — 거르는 순간 사용자 인자를 우리가 검열하는 것이다: {argv:?}"
        );
        // ★「맨 뒤」가 아니라 「**그 사용자 값보다** 뒤」를 잰다★ — 실제 불변식은 패스스루 뒤에 선다는
        //   것뿐이고, argv 의 마지막 칸은 우리 형제 오버라이드(지시서)가 프라이밍 파일이 있을 때 가져간다.
        //   맨 뒤를 못 박으면 회귀가 아닌 변경에 이 항목이 깨진다.
        let theirs = argv
            .iter()
            .position(|a| a == "tui.status_line=[]")
            .expect("사용자 값이 사라졌다");
        let ours = argv
            .iter()
            .position(|a| a == STATUS_LINE_OVERRIDE)
            .expect("우리 값이 사라졌다");
        assert!(
            theirs < ours,
            "우리 값이 사용자 값보다 앞이다 — 마지막이 이기므로 이 순서를 뺏기면 회수가 죽는다: {argv:?}"
        );
    }

    // ── ADR-0216: 훅 등록은 걷혔다 ───────────────────────────────────────────────
    //
    // ★순서·충돌 경고 축은 상태줄 오버라이드 쪽으로 **옮겨 갔다**★ — 위 ADR-0216 구획의
    //   `the_status_line_override_follows_the_passthrough` 와
    //   `a_conflicting_status_line_passthrough_loses_to_ours_and_trips_the_warning` 이 그 자리다.
    //   여기 있던 훅판 둘은 잴 대상(argv 에 실린 훅 값)이 사라져 그쪽으로 승계됐다.

    /// ★터미널 스폰에 훅이 더는 실리지 않는다★ — 이 한 줄이 ADR-0216 의 회수 경로 교체를 못 박는다.
    /// 실리면 codex 의 신뢰 심사 화면이 되살아나 **승인 전까지 에이전트가 안 뜬다.**
    #[test]
    fn the_terminal_spawn_no_longer_registers_the_session_start_hook() {
        let argv = codex_argv(&spec_with_control(&codex(vec![]), Some(endpoint())));
        assert_eq!(hook_override_value(&argv), None, "{argv:?}");
        assert!(
            !argv.iter().any(|a| a.contains("hook session-start")),
            "훅 명령 문자열이 다른 모양으로 실렸다: {argv:?}"
        );
    }

    /// app-server 모드도 같다. ★한때 이 둘이 갈렸다 — 그 시절 훅은 터미널 전용이었다★(app-server 는
    /// `thread/start` 응답으로 id 를 직접 받아 왔다). 지금은 두 모드 다 안 건다.
    #[test]
    fn the_app_server_spawn_registers_no_hook() {
        let s = spec_with_control(&codex_app_server(vec![]), Some(endpoint()));
        let argv = codex_argv(&s);
        assert_eq!(hook_override_value(&argv), None, "{argv:?}");
        assert_eq!(
            &argv[..2],
            &["app-server".to_string(), APP_SERVER_STDIO_FLAG.to_string()],
            "하위 명령과 전송 선택은 여전히 맨 앞이어야: {argv:?}"
        );
    }

    /// endpoint 자체가 없는 스폰의 argv — ★`-c` 의 **부재**로 재던 옛 형태는 쓸 수 없다★: 상태줄
    /// 오버라이드는 endpoint 를 안 보므로 이 스폰에도 실린다(그 축을 재는 자리 =
    /// `the_status_line_override_needs_no_control_endpoint`). 그래서 여기서는 `-c` 가 **그것 하나뿐**임을
    /// 잰다 — MCP 부착도 훅도 안 실렸다는 뜻이다.
    #[test]
    fn without_an_endpoint_only_the_status_line_override_rides() {
        let argv = codex_argv(&spec_with_control(&codex(vec![]), None));
        assert_eq!(hook_override_value(&argv), None, "{argv:?}");
        assert_eq!(
            argv.iter().filter(|a| *a == CONFIG_OVERRIDE_FLAG).count(),
            1,
            "endpoint 부재인데 상태줄 말고 다른 `-c` 가 실렸다: {argv:?}"
        );
    }

    /// 다른 키를 `-c` 로 넘기는 것은 충돌이 아니다 — 경고를 남발하면 아무도 안 읽는다.
    ///
    /// ★키를 **살아 있는 쪽**(상태줄)으로 잰다★ — 걷어낸 훅 키로 재면 이 헬퍼가 실제로 굴리는 경고와
    ///   같은 축을 안 밟게 된다.
    #[test]
    fn an_unrelated_config_passthrough_is_not_a_conflict() {
        assert!(!passthrough_overrides_key(
            &[CONFIG_OVERRIDE_FLAG.to_string(), "model=gpt-5".to_string()],
            STATUS_LINE_KEY,
        ));
        assert!(!passthrough_overrides_key(
            &["--profile".to_string(), format!("{STATUS_LINE_KEY}=[]")],
            STATUS_LINE_KEY,
        ));
    }

    /// 이어받기 갈래에서도 하위 명령은 맨 앞, 우리 오버라이드는 패스스루 뒤 — 둘이 서로를 밀어내지 않는다.
    ///
    /// ★「맨 뒤」를 못 박지 않는다★ — 그 칸은 형제 오버라이드(지시서)가 프라이밍 파일이 있을 때 가져가고,
    ///   그건 회귀가 아니다. 실제 불변식은 **패스스루보다 뒤**뿐이다.
    #[test]
    fn a_resuming_spawn_still_carries_our_override_after_the_passthrough() {
        let s = CodexBackend.build_spec(
            &codex(vec!["-m", "gpt-5"]),
            SpawnMode::Resume,
            None,
            Some(Uuid::new_v4()),
            PathBuf::from("C:/workspace"),
            vec![],
            Some(endpoint()),
        );
        let argv = codex_argv(&s);
        assert_eq!(&argv[..1], &[RESUME_SUBCOMMAND.to_string()]);
        let passthrough = argv
            .iter()
            .position(|a| a == "gpt-5")
            .expect("패스스루가 사라졌다");
        let ours = argv
            .iter()
            .position(|a| a == STATUS_LINE_OVERRIDE)
            .expect("상태줄 오버라이드가 사라졌다");
        assert!(passthrough < ours, "{argv:?}");
    }

    #[test]
    fn app_server_extra_args_come_last() {
        let s = spec(&codex_app_server(vec!["-c", "model=x"]), "C:/workspace");
        let argv = codex_argv(&s);
        assert_eq!(
            argv,
            vec!["app-server", "--stdio", "-c", "model=x"],
            "하위 명령은 맨 앞이어야 한다 — 뒤로 밀리면 인자로 읽힌다"
        );
    }

    /// 두 모드가 **다른** 통로 모양을 신고하고, 실물도 그 신고를 따른다.
    #[test]
    fn the_two_modes_declare_different_transport_shapes() {
        assert_eq!(
            CodexBackend.transport_shape(&codex(vec![])),
            TransportShape::Pty
        );
        assert_eq!(
            CodexBackend.transport_shape(&codex_app_server(vec![])),
            TransportShape::StdioBidiJson
        );
    }

    /// ★`Raw` 였다면 세션이 본문 뒤에 CR 을 **별도 호출**로 한 번 더 내고, 봉투를 스스로 만드는 통로는
    /// 그것을 또 하나의 본문으로 읽어 빈 턴을 연다★.
    #[test]
    fn app_server_encoder_passes_bytes_through_and_asks_for_no_submit_byte() {
        let enc = CodexBackend.input_encoder(&codex_app_server(vec![]));
        assert_eq!(enc, InputEncoder::TransportFramed);
        assert_eq!(enc.submit_sequence(), None);
        let body = b"hello codex";
        assert_eq!(enc.encode(body, Uuid::new_v4()), body.to_vec());
    }

    /// ★되살리지 마라 — 합성 입력 에코는 기각된 대안이다(ADR-0193 「거부한 대안」)★: codex 가
    /// `userMessage` item 으로 되울리는 것과 겹쳐 같은 질문이 화면에 두 벌 남는다(dedup 키가 서로
    /// 다르다). 유저 발화를 화면에 올리는 것은 그 되울림의 **번역**이 진다(이 폴더 `decoder`).
    /// ★두 모드를 다 재는 것이 요점이다★ — 세션 층은 backend 가 아니라 [`InputEncoder`] 를 들고 dispatch
    /// 표를 지나므로, 선언만 보고 그 표를 안 보면 되살아난 에코가 초록인 채 지나간다.
    ///
    /// ★★이 부재에 **화면 복원의 순서**도 걸려 있다(ADR-0203) — 되살리려면 그 순서를 먼저 풀어야 한다★★:
    /// app-server 모드의 이력은 핸드셰이크 **뒤에** 상대에게 요청해서 받는데, 세션은 그 창에서도 입력을
    /// 받아 큐에 세운다(그것이 옳다 — 최대 10 초 동안 입력을 거절하는 쪽이 더 나쁘다). 에코가 있으면 그
    /// 입력이 **즉시** 링에 실리고, 뒤늦게 도착한 복원 이력이 그 아래 깔려 **사용자의 새 말이 복원된 옛
    /// 대화보다 위에** 그려진다. 오늘 그 일이 안 일어나는 이유는 순서를 지키는 장치가 아니라 **에코가
    /// 없다는 이 사실**이다.
    /// ★그 창에서 사용자가 보는 것★ = 자기 말풍선 대신 프론트의 대기 표시이고, 게이트가 열린 뒤 상대가
    /// 되울린 `userMessage` item 이 정상 순서로 말풍선을 세운다.
    #[test]
    fn neither_mode_makes_a_synthetic_input_echo() {
        for command in [codex_app_server(vec![]), codex(vec![])] {
            let enc = CodexBackend.input_encoder(&command);
            assert!(
                enc.input_echo_event("안녕 codex".as_bytes(), Uuid::new_v4())
                    .is_none(),
                "{enc:?}: 합성 에코가 되살아났다"
            );
        }
    }

    #[test]
    fn the_turn_end_event_is_the_ended_signal() {
        let classify = CodexBackend.turn_classifier();
        assert_eq!(
            classify(&OutputEvent::TurnEnd {
                turn_id: Some("u-1".into()),
                outcome: TurnOutcome::Completed
            }),
            Some(TurnSignal::Ended)
        );
        // 결말이 무엇이든 턴은 끝난 것이다 — 실패·중단·미상이 여기서 갈리면 그 결말의 대기 표시가 남는다.
        for outcome in [
            TurnOutcome::Failed {
                detail: Some("boom".into()),
            },
            TurnOutcome::Interrupted,
            TurnOutcome::Unknown,
        ] {
            assert_eq!(
                classify(&OutputEvent::TurnEnd {
                    turn_id: None,
                    outcome: outcome.clone()
                }),
                Some(TurnSignal::Ended),
                "{outcome:?}"
            );
        }
        // 되울린 유저 메시지 — 이 턴이 시작됐다는 첫 관측이다.
        assert_eq!(
            classify(&OutputEvent::Structured {
                kind: "user".into(),
                json: "{}".into()
            }),
            Some(TurnSignal::Progress)
        );
        assert_eq!(
            classify(&OutputEvent::TextDelta {
                text: "hi".into(),
                turn_id: None,
                message_id: None
            }),
            Some(TurnSignal::Progress)
        );
        // `error` 알림은 재시도 가능한 스트림 오류라 턴 경계가 아니다.
        assert_eq!(classify(&OutputEvent::Error("boom".into())), None);
        assert_eq!(
            classify(&OutputEvent::Usage {
                input_tokens: 1,
                output_tokens: 2,
                turn_id: None
            }),
            None
        );
    }

    /// 터미널 모드는 decoder 가 없어 `TerminalBytes` 만 흐른다 — 그래서 같은 분류자를 모드별 분기 없이
    /// 공유해도 그 모드에서는 신호가 하나도 나오지 않는다.
    #[test]
    fn terminal_bytes_carry_no_turn_signal() {
        assert_eq!(
            CodexBackend.turn_classifier()(&OutputEvent::TerminalBytes(b"[0m".to_vec())),
            None
        );
    }

    #[test]
    fn only_app_server_mode_gets_a_decoder() {
        assert!(CodexBackend.output_decoder(&codex(vec![])).is_none());
        assert!(CodexBackend
            .output_decoder(&codex_app_server(vec![]))
            .is_some());
    }

    /// 세 선언이 한 항목에 있는 이유 = **함께 봐야 하는 짝**이다. 발급 축은 두 모드 다 꺼져 있고
    /// (발급 주체가 codex 라 우리가 심을 값이 없다), 이어받기 두 칸은 두 모드 다 켜져 있다 —
    /// 갈리는 것은 **수단**뿐이고(argv ↔ `thread/resume`) 이 네 칸은 그 수단을 묻지 않는다.
    ///
    /// ★네 칸을 다 적는 것이 요점이다★ — 이어받기 축과 caps 신고 칸 중 **한쪽만** 갈리면 이어받은 적
    ///   없는 새 스레드가 「이어받음」으로 보고되거나 그 반대가 되는데, 모드별로 한 칸씩만 재면 그
    ///   어긋남이 이 파일에서 안 보인다.
    /// ★터미널 모드의 두 칸을 `false` 로 되돌리려면 argv 조립도 함께 걷어야 한다★ — 선언만 끄면
    ///   활성화 입구가 Fresh 로 띄우는데 [`AgentBackend::build_spec`] 은 여전히 이어받을 채비를 하고
    ///   있어, 그 배선이 영영 안 불리는 죽은 코드가 된다.
    #[test]
    fn both_modes_declare_resume_and_neither_claims_to_issue_the_id() {
        for c in [codex(vec![]), codex_app_server(vec![])] {
            assert!(
                !CodexBackend.assigns_session_id(&c),
                "{c:?}: 발급 주체는 codex 다 — 우리 uuid 를 심으면 그 값은 영영 안 쓰인다"
            );
            assert!(
                CodexBackend.can_resume_stored_session(&c),
                "{c:?}: 이어받기 축을 끄면 손잡이가 명부에 있어도 활성화 입구가 Fresh 로 띄운다"
            );
            assert!(
                CodexBackend.capabilities(&c).session.resume,
                "{c:?}: 축만 켜고 신고를 끄면 실제로 이어받는 스폰이 「새 대화」로 보고된다"
            );
        }
    }

    /// ★선언과 실물이 짝이어야 한다★ — `declares_link()` 가 조립점에서 배달 포트를 **깔지 말지**를
    /// 가르므로, true 인데 통로가 안 부르면 감독자가 백스톱까지 기다리고 false 인데 부르면 그 배달을
    /// 아무도 안 받는다.
    ///
    /// 여기서 재는 것은 선언 쪽이고, 실물 쪽(app-server 통로가 실제로 배달한다)은
    /// `a_rejected_resume_does_not_fall_back_and_ends_the_session` 이 실 통로로 잰다.
    #[test]
    fn only_app_server_declares_a_link() {
        assert!(
            CodexBackend.declares_link(&codex_app_server(vec![])),
            "app-server 는 핸드셰이크를 마쳐야 쓸 수 있다 — 축이 없다고 선언하면 그 결말이 판정에              참여하지 못하고, 거절이 다시 성공으로 보고된다"
        );
        assert!(
            !CodexBackend.declares_link(&codex(vec![])),
            "터미널 모드는 PTY 라 뜬 순간부터 쓸 수 있다 — 축이 있다고 선언하면 아무도 배달하지 않는              채널을 감독자가 백스톱까지 기다린다"
        );
    }

    /// ★이 값을 정하는 것은 턴 신호 유무가 아니라 ADR-0116 결정 7 이다★ — 터미널 모드에 신호가 없다는
    /// 사실은 여기서 false 를 낼 사유가 못 된다(터미널 claude 가 같은 자리에서 이미 받는다). 되돌리려면
    /// 그 결정부터 뒤집을 것.
    // ADR-0116
    #[test]
    fn reads_messages_is_true() {
        assert!(CodexBackend.reads_messages());
    }

    // ── ADR-0185: 받아 온 thread id 가 조립점의 기록 동사까지 실제로 간다 ──────────────────

    /// 가짜 app-server 스크립트를 지운다 — 지웠으면 `true`. ★한 번 시도하고 마는 모양은 `%TEMP%` 에
    /// 파일을 남겼다★: `shutdown()` 이 돌아온 뒤에도 자식이 그 파일 핸들을 잠깐 더 쥐고 있어 첫
    /// `remove_file` 이 실패한다.
    /// ★결과를 돌려주는 이유 = `eprintln!` 은 **통과한** 항목에서 libtest 가 삼킨다★ — 그 자리에 적으면
    /// 지우지 못한 사실이 아무 데도 안 남고 찌꺼기만 쌓인다. 호출자가 단언으로 올린다.
    #[cfg(windows)]
    #[must_use = "지우지 못한 사실을 버리면 찌꺼기가 조용히 쌓인다 — 단언으로 올릴 것"]
    fn remove_script(path: &std::path::Path) -> bool {
        for _ in 0..40 {
            if std::fs::remove_file(path).is_ok() || !path.exists() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        false
    }

    /// 가짜가 `thread/resume` 에 무엇으로 답하나.
    #[cfg(windows)]
    enum FakeResume {
        /// 받은 `threadId` 에 접두를 붙여 돌려준다 — 우리가 보낸 값과 **다른** 값이 되는 것이 요점이다.
        EchoesTheThreadId,
        /// `-32600` 으로 거절한다. ★이 코드가 「모르는 스레드」 전용이 아니라는 것이 ADR-0082 의 codex 쪽
        /// 보강 사유다★ — 모르는 메서드도 중복 `initialize` 도 설정 오류도 같은 코드로 오므로(실측
        /// 0.154.0 — 정본은 이 폴더 `protocol` 과 통로 시험대), 코드로 갈라 새 스레드로 폴백하면 아직
        /// 멀쩡한 손잡이를 무관한 실패에서 덮어쓴다.
        Rejects,
    }

    /// 가짜 app-server 를 임시 파일로 구워 그것을 띄울 [`CommandSpec`] 과 그 경로를 돌려준다.
    /// `start_thread_id` = 이 가짜가 `thread/start` 에 답할 id.
    ///
    /// ★경로를 함께 돌려주는 것은 계약이다★ — 항목마다 [`remove_script`] 로 지워야 하고, 못 지운 사실을
    /// 단언으로 올려야 한다.
    #[cfg(windows)]
    fn bake_fake_app_server(
        start_thread_id: &str,
        resume: FakeResume,
    ) -> (CommandSpec, std::path::PathBuf) {
        let resume_reply = match resume {
            FakeResume::EchoesTheThreadId => {
                r#"'"result":{"thread":{"id":"resumed-' + $tid + '","cliVersion":"0.0.0-fake"}}'"#
            }
            // ★문구를 실측된 것으로 쓴다★ — 이 가짜가 「모르는 스레드」를 흉내 내는 목적이 분류까지
            //   태우는 것인데, 지어낸 문구를 쓰면 `resume_failure_kind` 가 못 알아봐서 그 배선이 시험대를
            //   그냥 통과한다(`docs/reference/backend-capabilities.md` §1 「모르는 id 를 주면」).
            FakeResume::Rejects => {
                r#"'"error":{"code":-32600,"message":"no rollout found for thread id"}'"#
            }
        };
        let script = FAKE_APP_SERVER_PS1
            .replace("THREAD_ID_PLACEHOLDER", start_thread_id)
            .replace("RESUME_REPLY_PLACEHOLDER", resume_reply);
        let script_path =
            std::env::temp_dir().join(format!("engram-fake-app-server-{}.ps1", Uuid::new_v4()));
        std::fs::write(&script_path, script).expect("가짜 app-server 기록");

        let spec = CommandSpec {
            program: "powershell.exe".into(),
            args: vec![
                "-NoProfile".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                script_path.to_string_lossy().into_owned(),
            ],
            env: vec![],
            cwd: PathBuf::from("."),
        };
        (spec, script_path)
    }

    /// 핸드셰이크의 두 요청에만 답하는 최소 app-server. ★실 codex 가 아니다★ — 재는 것은 「상대가 준
    /// thread id 가 [`AgentBackend::open_spawn`] 에 건넨 동사까지 오나」 하나이고, 그 답에 실 CLI 는
    /// 무관하다(ADR-0012 격리).
    /// ★인자로 넘기지 않고 파일로 굽는다★ — 스크립트에 JSON 이 들어 있어 `"` 가 필수인데, 그 문자는
    /// Rust `Command` 의 인자 이스케이프와 powershell 의 명령줄 해석을 거치며 두 번 씹힌다.
    /// ★stdout 은 raw 바이트로 쓴다★ — `Write-Output` 은 인코딩·BOM 이 호스트 설정에 딸려 가고, BOM 한
    /// 바이트가 첫 줄을 JSON 이 아니게 만든다.
    ///
    /// ★`thread/resume` 갈래의 답은 [`FakeResume`] 이 채워 넣는다★ — 그 자리에 들어가는 것은 JSON 조각이
    /// 아니라 **powershell 식**이라, 받은 `threadId` 를 이어 붙이는 답도 쓸 수 있다.
    ///
    /// ★**이 가짜는 답한 뒤에도 죽지 않고 stdin 을 계속 읽는다 — 그 성질이 한 항목의 전제다**★.
    /// [`tests::a_recording_failure_ends_the_session`] 이 재는 것은 「기록이 실패하면 우리가 stdin 을 닫고
    /// 그 결과로 세션이 끝난다」인데, 이 가짜가 스스로 끝나 버리면 EOF 가 **닫기와 무관하게** 와서 그
    /// 항목이 배선을 하나도 안 재고 초록이 된다. 실제로 그렇지 않다는 것은 변이로 확인했다 — 닫기를
    /// 없애면 그 항목에 상태 전이가 **하나도** 관측되지 않는다. ★그러니 아래 while 루프를 「응답 뒤
    /// exit」 으로 바꾸면 그 항목이 조용히 무력해진다★.
    #[cfg(windows)]
    const FAKE_APP_SERVER_PS1: &str = r#"
$ErrorActionPreference = 'Stop'
$so = [Console]::OpenStandardOutput()
function Send([string]$s) {
  $b = [Text.Encoding]::ASCII.GetBytes($s + "`n")
  $so.Write($b, 0, $b.Length)
  $so.Flush()
}
while ($null -ne ($line = [Console]::In.ReadLine())) {
  if ($line -match '"id":(-?\d+)') {
    $rid = $Matches[1]
    if ($line -match '"method":"initialize"') {
      Send ('{"id":' + $rid + ',"result":{"codexHome":"h","platformFamily":"windows","platformOs":"windows","userAgent":"engram-fake/0"}}')
    } elseif ($line -match '"method":"thread/start"') {
      Send ('{"id":' + $rid + ',"result":{"thread":{"id":"THREAD_ID_PLACEHOLDER","cliVersion":"0.0.0-fake"}}}')
    } elseif ($line -match '"method":"thread/resume"') {
      $tid = 'MISSING'
      if ($line -match '"threadId":"([^"]+)"') { $tid = $Matches[1] }
      Send ('{"id":' + $rid + ',' + RESUME_REPLY_PLACEHOLDER + '}')
    }
  }
}
"#;

    /// ★터미널 갈래의 **운영 배선**을 도는 유일한 항목이다★ — [`AgentBackend::open_spawn`] 을 실제로
    /// 불러 PTY 를 띄우고, 자식이 찍은 id 가 기록 동사까지 닿는 것을 잰다.
    ///
    /// ★형제 시험들은 이것을 못 잡는다★: 스캐너 시험은 [`thread_id`] 안에서 스캐너만 돌리고, 통로 시험은
    ///   자기 클로저를 직접 꽂는다. 그래서 `open_spawn` 의 관찰자 인자를 `None` 으로 바꿔도 **그 전부가
    ///   초록이었다** — 기능을 지워도 통과하는 부류가 이 기능의 유일한 load-bearing seam 위에 앉아 있었다.
    ///   이 항목이 그 자리를 메운다.
    /// ★`cmd.exe` 로 찍는 것이 요점이다★ — 실 codex 가 아니라도 이 경로가 재는 것은 **우리 배선**이고,
    ///   실 codex 를 요구하면 CI 에서 못 돈다(그 레인은 `tests/backend_contract.rs` 의 `#[ignore]` 몫).
    /// ★값에 공백을 두지 않는다★ — 공백이 있으면 `CommandBuilder` 가 인자를 따옴표로 감싸고, 그 따옴표가
    ///   화면에 그대로 찍혀 단언이 무엇을 재는지 흐려진다.
    #[cfg(windows)]
    #[test]
    fn a_terminal_spawn_hands_the_status_line_id_to_the_sink() {
        use crate::output_core::{OutputCore, TurnWiring};
        use crate::types::{AgentInfo, AgentStatus, StatusSink};
        use std::sync::{Arc, Mutex};

        struct NoopStatus;
        impl StatusSink for NoopStatus {
            fn status_changed(&self, _id: Uuid, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        let thread_id = Uuid::new_v4().to_string();
        let spec = CommandSpec {
            program: "cmd.exe".to_string(),
            args: vec![
                "/c".to_string(),
                "echo".to_string(),
                format!("tid={thread_id}"),
            ],
            env: vec![],
            cwd: PathBuf::from("."),
        };

        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: SessionIdSink = {
            let seen = seen.clone();
            Arc::new(move |id: &str| seen.lock().unwrap().push(id.to_string()))
        };

        let parts =
            crate::backend::open_spawn(&codex(vec![]), &spec, 80, 24, Some(sink), None, None, None)
                .expect("open_spawn");

        parts.transport.start(Arc::new(OutputCore::new(
            Uuid::new_v4(),
            1,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        )));

        // 기록은 pump 가 아니라 배달 스레드가 한다 — pump 가 끝나도 그쪽이 아직 안 깼을 수 있다.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while seen.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let got = seen.lock().unwrap().clone();
        parts.transport.shutdown();

        assert_eq!(
            got,
            vec![thread_id],
            "터미널 스폰의 관찰자 배선이 끊겼다 — `open_spawn` 의 관찰자 인자를 확인할 것"
        );
    }

    /// ★조립점이 `None` 을 넘기던 시절에는 이 항목이 서지 않았다★ — 배선이 다시 끊기면 여기서 잡힌다.
    /// 재는 것은 **기록 동사가 불렸나**이고, 그 값이 프로필에 어떻게 앉나는 `manager.rs` 쪽 항목이 잰다.
    #[cfg(windows)]
    #[test]
    fn an_app_server_spawn_hands_the_thread_id_to_the_sink() {
        use crate::output_core::{OutputCore, TurnWiring};
        use crate::types::{AgentInfo, AgentStatus, StatusSink};
        use std::sync::{Arc, Mutex};

        struct NoopStatus;
        impl StatusSink for NoopStatus {
            fn status_changed(&self, _id: Uuid, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        let thread_id = Uuid::new_v4().to_string();
        let (spec, script_path) = bake_fake_app_server(&thread_id, FakeResume::EchoesTheThreadId);

        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: SessionIdSink = {
            let seen = seen.clone();
            Arc::new(move |id: &str| seen.lock().unwrap().push(id.to_string()))
        };

        let parts = crate::backend::open_spawn(
            &codex_app_server(vec![]),
            &spec,
            80,
            24,
            Some(sink),
            None,
            None,
            None,
        )
        .expect("open_spawn");

        parts.transport.start(Arc::new(OutputCore::new(
            Uuid::new_v4(),
            1,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        )));

        // 핸드셰이크는 powershell 기동 + 두 왕복이라 즉시 끝나지 않는다. 시한은 통로 자신의 요청 시한
        //   (30s)보다 짧게 둔다 — 넘기면 실패 사유가 「우리가 덜 기다렸다」로 흐려진다.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while seen.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let got = seen.lock().unwrap().clone();

        parts.transport.shutdown();

        // ★두 실패를 갈라 적는다★ — 「한 번도 안 불렸다」는 배선이 끊긴 것일 수도, 가짜 app-server 가
        //   시한 안에 안 뜬 것일 수도 있다(powershell 기동·실행 정책). 한 문장으로 적으면 환경 문제를
        //   배선 회귀로 읽는다. ★그래서 이 갈래에서는 스크립트를 지우지 않는다★ — 손으로 돌려 봐야 갈린다.
        assert!(
            !got.is_empty(),
            "시한(20초) 안에 기록 동사가 한 번도 불리지 않았다 — 배선이 끊겼거나, 가짜 app-server 가 그 안에 \
             핸드셰이크를 끝내지 못했다(powershell 기동 실패·실행 정책). 가르려면 {} 를 손으로 돌려 볼 것",
            script_path.display()
        );
        let removed = remove_script(&script_path);
        assert_eq!(
            got,
            vec![thread_id],
            "기록 동사가 app-server 가 준 thread id 와 다른 값을 받았다"
        );
        assert!(
            removed,
            "가짜 app-server 스크립트를 지우지 못했다: {}",
            script_path.display()
        );
    }

    /// ★조립점이 이어받을 값을 주면 둘째 요청이 `thread/resume` 으로 갈린다 — 그리고 기록되는 것은
    /// **상대가 답한 id** 다★.
    ///
    /// 단언 하나가 셋을 가른다(가짜의 답이 `resumed-<우리가 보낸 threadId>` 라서):
    ///   1. 나간 것이 `thread/start` 가 아니다 — 그쪽이었으면 가짜가 `start_only` 를 돌려준다.
    ///   2. `threadId` 칸이 우리가 넘긴 값을 그대로 실어 갔다(철자 camelCase 포함).
    ///   3. 기록 동사가 받는 것은 **상대가 준 값**이다. 우리가 보낸 값을 되쓰는 구현이었다면 접두 없는
    ///      원본이 올라와 여기서 갈린다 — 그 갈래가 중요한 이유는, 상대가 다른 id 를 주는 날 그 값이
    ///      이 화신이 실제로 말하는 스레드이기 때문이다.
    ///
    /// ★`send_input` 을 한 번도 하지 않는다★ — 재는 것은 핸드셰이크 구획뿐이고, 여기서 보내면 상한까지
    ///   큐가 차는 다른 갈래를 섞게 된다.
    #[cfg(windows)]
    #[test]
    fn a_resume_target_makes_the_handshake_issue_thread_resume() {
        use crate::output_core::{OutputCore, TurnWiring};
        use crate::types::{AgentInfo, AgentStatus, StatusSink};
        use std::sync::{Arc, Mutex};

        struct NoopStatus;
        impl StatusSink for NoopStatus {
            fn status_changed(&self, _id: Uuid, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        let resume_target = Uuid::new_v4();
        let (spec, script_path) = bake_fake_app_server("start_only", FakeResume::EchoesTheThreadId);

        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: SessionIdSink = {
            let seen = seen.clone();
            Arc::new(move |id: &str| seen.lock().unwrap().push(id.to_string()))
        };

        let parts = crate::backend::open_spawn(
            &codex_app_server(vec![]),
            &spec,
            80,
            24,
            Some(sink),
            Some(resume_target),
            None,
            None,
        )
        .expect("open_spawn");

        parts.transport.start(Arc::new(OutputCore::new(
            Uuid::new_v4(),
            1,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        )));

        // 시한 근거는 위 항목과 같다 — 통로 자신의 요청 시한(30s)보다 짧게 둔다.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while seen.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let got = seen.lock().unwrap().clone();

        parts.transport.shutdown();

        assert!(
            !got.is_empty(),
            "시한(20초) 안에 기록 동사가 한 번도 불리지 않았다 — 이어받기 요청이 안 나갔거나(가짜는              `thread/start` 에도 답하므로 그 경우에도 불려야 한다), 가짜 app-server 가 그 안에 뜨지              못했다. 가르려면 {} 를 손으로 돌려 볼 것",
            script_path.display()
        );
        let removed = remove_script(&script_path);
        assert_eq!(
            got,
            vec![format!("resumed-{resume_target}")],
            "이어받기 응답의 id 가 기록되지 않았다 — `start_only` 면 `thread/start` 가 나간 것이고,              접두 없는 원본이면 상대 답 대신 우리가 보낸 값을 되쓴 것이다"
        );
        // ★이 가짜가 주는 값은 **uuid 가 아니다** — 그 사실을 여기서 못 박는다★.
        //   조립점의 기록 동사(`manager::session_id_sink`)는 `Uuid::parse_str` 에 실패한 값을 **조용히
        //   버리므로**, 이 갈래에서 실제로 일어나는 일은 「기록됨」이 아니라 **갈림**이다: 통로는 이
        //   문자열을 `thread_id` 로 들고 그 뒤 모든 턴을 그것으로 내보내는데, 디스크의
        //   `backend_session_id` 는 **옛 값 그대로** 남는다. 그래서 다음 이어받기는 이 화신이 실제로
        //   말한 스레드가 아닌 곳을 연다.
        //   ★그 갈림을 재는 짝은 `manager.rs` 쪽 `a_non_uuid_thread_id_leaves_the_stored_one_diverged`
        //   이고, 여기서 재는 것은 「이 시험대가 그 조건을 실제로 만들어 낸다」 하나다★ — 이 단언이 없으면
        //   위 `assert_eq!` 가 「기록까지 됐다」로 잘못 읽힌다.
        assert!(
            Uuid::parse_str(&got[0]).is_err(),
            "가짜의 답이 uuid 로 읽힌다 — 이 항목의 전제(조립점이 이 값을 버린다)가 낡았다: {got:?}"
        );
        assert!(
            removed,
            "가짜 app-server 스크립트를 지우지 못했다: {}",
            script_path.display()
        );
    }

    /// ★거절당한 이어받기는 **새 스레드로 되돌아가지 않는다**(ADR-0082)★ — 그리고 프로필에 저장된
    /// 손잡이를 건드리지 않는다.
    ///
    /// 결정적 증거는 **기록 동사가 한 번도 안 불린다**는 것이다: 이 가짜는 `thread/start` 에도 답하므로
    /// (`start_only`), 통로가 거절을 보고 새 스레드를 열었다면 그 id 가 기록 동사로 올라온다. 그 부재가
    /// 곧 둘째 spawn 부재이고, 기록이 없으니 저장된 sid 도 그대로다.
    /// ★부재를 시한으로만 재지 않는다★ — 화면에 오르는 실패 경계를 **기다린 뒤에** 부재를 단언한다.
    ///   그래야 「아직 안 끝났을 뿐」과 「폴백이 없다」가 갈린다.
    ///
    /// ★그리고 **세션이 종점에 닿는 것까지 여기서 잰다**★ — 이것이 이 항목의 두 번째 축이고, 없으면
    ///   항목 전체가 **깨진 구현 위에서 초록**이다. 이 가짜는 거절을 답한 뒤에도 죽지 않고 stdin 을 계속
    ///   읽으므로(그 상수의 doc), 통로가 우리 쪽 stdin 을 안 놓으면 자식·리더·라이터가 그대로 살아
    ///   리더가 EOF 를 못 보고 [`crate::output_core::OutputCore::finish`] 가 영영 안 돈다. 그러면 reaper 로
    ///   가는 메시지가 아예 만들어지지 않고(그 단독 소비자 — ADR-0019), 매니저의 활성화 판정은 3초 내내
    ///   `Running` 만 보고 **「이어받기 성공」으로 도장을 찍으며 마지막 실패 기록까지 지운다**.
    ///   ★한때 이 자리에 「종점은 매니저·reaper 몫이고 그 회귀망은 `tests/activation.rs` 가 진다」고
    ///   적혀 있었다 — 그 그물은 **없다**(그 파일에 codex 항목은 프로필 배선 하나뿐이고 거절 갈래를 재지
    ///   않는다). 그래서 이 통로가 stdin 을 놓는 것을 재는 자리는 여기뿐이다.★
    /// ★`shutdown()` 전에 기다리는 것이 요점이다★ — 우리가 죽여 놓고 「끝났다」를 재면 이 항목이 재려던
    ///   인과를 우리가 대신 굴린 것이 된다(선례 = [`tests::a_recording_failure_ends_the_session`]).
    #[cfg(windows)]
    #[test]
    fn a_rejected_resume_does_not_fall_back_and_ends_the_session() {
        use crate::output_core::{OutputCore, TurnWiring};
        use crate::types::{
            AgentInfo, AgentStatus, OutputFrame, OutputPayload, OutputSink, SinkError, SinkId,
            StatusSink, TurnOutcome,
        };
        use std::sync::{Arc, Mutex};

        /// 종료 전이를 보는 눈 — 선례·사유는 [`tests::a_recording_failure_ends_the_session`] 과 같다.
        struct RecordingStatus(Arc<Mutex<Vec<AgentStatus>>>);
        impl StatusSink for RecordingStatus {
            fn status_changed(&self, _id: Uuid, s: AgentStatus, _e: u32) {
                self.0.lock().expect("status poisoned").push(s);
            }
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        struct EventSink {
            id: SinkId,
            seen: Arc<Mutex<Vec<OutputEvent>>>,
        }
        impl OutputSink for EventSink {
            fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
                if let OutputPayload::Event(e) = frame.payload {
                    self.seen.lock().unwrap().push(e.clone());
                }
                Ok(())
            }
            fn sink_id(&self) -> SinkId {
                self.id
            }
        }

        let resume_target = Uuid::new_v4();
        let (spec, script_path) = bake_fake_app_server("start_only", FakeResume::Rejects);

        let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: SessionIdSink = {
            let recorded = recorded.clone();
            Arc::new(move |id: &str| recorded.lock().unwrap().push(id.to_string()))
        };
        // ★배달을 그대로 받아 적는다★ — 이 항목이 재는 것은 「통로가 결말을 **내보내나**」이고, 그것이
        //   실 통로에서 확인되는 유일한 자리다(매니저 쪽 항목들은 대역으로 판정 로직만 잰다).
        let delivered: Arc<Mutex<Vec<crate::transport::LinkResolution>>> =
            Arc::new(Mutex::new(Vec::new()));
        let link_sink: crate::transport::LinkSink = {
            let delivered = delivered.clone();
            Arc::new(move |r| delivered.lock().unwrap().push(r))
        };

        let parts = crate::backend::open_spawn(
            &codex_app_server(vec![]),
            &spec,
            80,
            24,
            Some(sink),
            Some(resume_target),
            Some(link_sink),
            None,
        )
        .expect("open_spawn");

        let events: Arc<Mutex<Vec<OutputEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let statuses: Arc<Mutex<Vec<AgentStatus>>> = Arc::new(Mutex::new(Vec::new()));
        let core = Arc::new(OutputCore::new(
            Uuid::new_v4(),
            1,
            Arc::new(RecordingStatus(statuses.clone())),
            TurnWiring::detached(),
        ));
        core.subscribe(Arc::new(EventSink {
            id: SinkId::new_v4(),
            seen: events.clone(),
        }));
        parts.transport.start(core);

        let failed = |es: &[OutputEvent]| {
            es.iter().any(|e| {
                matches!(
                    e,
                    OutputEvent::TurnEnd {
                        outcome: TurnOutcome::Failed { .. },
                        ..
                    }
                )
            })
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !failed(&events.lock().unwrap()) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let seen = events.lock().unwrap().clone();
        let got = recorded.lock().unwrap().clone();
        let link_when_failed = match delivered.lock().unwrap().first() {
            Some(crate::transport::LinkResolution::Failed { reason }) => Some(reason.clone()),
            _ => None,
        };

        // 실패 경계가 오른 뒤, 통로가 우리 쪽 stdin 을 놓은 결과로 상대가 EOF 를 보고 끝나기를 기다린다.
        //   ★`shutdown()` 전이다★ — 위 doc 의 그 사유.
        let terminal = || {
            statuses
                .lock()
                .expect("status poisoned")
                .iter()
                .any(|s| !s.is_live())
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !terminal() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // ★통로가 실제로 「연결 못 섬 + 사유」를 신고하나 — 실물로 재는 자리는 여기뿐이다★:
        //   매니저 쪽 항목들은 대역 통로로 판정 로직만 재므로, 실 통로가 그 축을 안 채우면 그 배선이
        //   양쪽 다 초록인 채로 끊긴다. ★종점에 닿기 **전에** 잡는다★ — 그 뒤에는 이 값이 남아 있을
        //   이유가 없다.
        let link_when_failed = link_when_failed.expect(
            "실패 경계가 올랐는데 통로가 연결 결말을 **배달하지 않았다** — 활성화 판정이 받을 신호가 없다",
        );
        assert_eq!(
            delivered.lock().unwrap().len(),
            1,
            "연결 결말이 한 번이 아니다 — 이 포트의 계약은 **정확히 한 번**이다"
        );
        // 그리고 그 사유가 분류까지 가야 「마지막 실패」에 맥락 기본값(조기 종료)이 아닌 진짜 원인이 남는다.
        assert_eq!(
            CodexBackend.resume_failure_kind(&link_when_failed),
            Some(crate::failure::AgentFailureKind::NoConversationToResume),
            "연결 사유가 분류되지 않았다 — 이 갈래에는 조기 종료가 없으므로 맥락 기본값이 찍히면 거짓이다: {link_when_failed}"
        );

        let reached_terminal = terminal();
        let seen_statuses = statuses.lock().expect("status poisoned").clone();
        // ★`!is_live()` 로도 `Exited { .. }` 로도 **부족하다**★(리뷰 지적 — 옛 단언은 공허했다):
        //   리더가 EOF 를 보면 자식의 생사와 무관하게 `Exited { code: None }` 이 서므로, 종류만 보는
        //   단언은 위 `reached_terminal` 이 참일 때 **거짓이 될 수 없었다** — 즉 아무것도 안 재고 있었다.
        // ★코드가 실려 있는 것이 자식이 실제로 수거됐다는 유일한 증거다★ — `Some(_)` 은 `try_wait` 가
        //   성공했다는 뜻이고 그건 프로세스가 끝났을 때만 성공한다(그 값이 경합으로 유실되지 않게
        //   `reader_loop` 이 유계로 다시 본다 — 그 자리 주석이 정본).
        // ★어느 경로로 끝났는지는 여기서 가르지 않는다★ — 상대가 EOF 를 보고 스스로 끝났든, 유예를
        //   넘겨 통로가 직접 끝냈든 둘 다 정당한 결말이고, 가르려면 시간을 재야 해서 깜빡인다.
        //   이 항목이 막으려는 것은 **아무 결말도 없는 것**이다.
        let ended_with_a_reaped_child = seen_statuses
            .iter()
            .any(|s| matches!(s, AgentStatus::Exited { code: Some(_) }));

        parts.transport.shutdown();

        assert!(
            failed(&seen),
            "시한(20초) 안에 실패 경계가 화면에 오르지 않았다 — 거절이 삼켜졌거나 가짜 app-server 가 뜨지              못했다(스크립트는 남겨 둔다): {}",
            script_path.display()
        );
        let removed = remove_script(&script_path);
        assert!(
            got.is_empty(),
            "거절당한 이어받기 뒤에 기록 동사가 불렸다 — 새 스레드로 폴백했다는 뜻이고(값이              `start_only` 면 확정), 그 값이 프로필의 아직 멀쩡한 손잡이를 덮어쓴다: {got:?}"
        );
        assert!(
            reached_terminal,
            "이어받기가 거절됐는데 세션이 종료 상태에 닿지 않았다 — 통로가 우리 쪽 stdin 을 놓지 않아 \
             자식·리더·라이터가 그대로 붙들려 있고, pump 가 종결을 못 해 reaper 로 가는 메시지가 아예 \
             만들어지지 않는다. 그 상태를 매니저는 「이어받기 성공」으로 읽고 마지막 실패 기록까지 \
             지운다(ADR-0082 가 막으려던 결말). 관측된 상태: {seen_statuses:?}"
        );
        assert!(
            ended_with_a_reaped_child,
            "종점에 닿았는데 자식이 수거된 흔적이 없다 — `Failed` 와 `Exited{{code:None}}` 는 자식이 살아 \
             있어도 서므로, 그것만으로는 「세션만 끝나고 프로세스는 남았다」와 구별되지 않는다(관측: {seen_statuses:?})"
        );
        assert!(
            removed,
            "가짜 app-server 스크립트를 지우지 못했다: {}",
            script_path.display()
        );
    }

    /// ★기록이 패닉하면 세션이 **끝난다** — 조용히 멈추지도, 영원히 떠 있지도 않는다★.
    ///
    /// 이 항목이 재는 것은 셋이고 ★셋째가 핵심★이다:
    ///   1. 기록 동사가 실제로 불렸다(= 핸드셰이크가 섰다. 아니면 아래 둘이 공회전한다).
    ///   2. 입력이 거절된다 — 링크가 `Connecting` 에 멈춰 큐만 차던 모양이 아니다(ADR-0190).
    ///   3. ★세션이 **종료 상태에 닿는다**★. 2 만 재던 옛 모양은 **막힌 세션과 구별이 안 됐다** —
    ///      거절은 되는데 자식·리더·라이터가 그대로 남고 pump 가 영영 안 끝나 수거도 안 되는 상태가
    ///      2 를 그대로 통과한다. 그 갈래를 가르는 것은 종료 전이 하나뿐이다.
    #[cfg(windows)]
    #[test]
    fn a_recording_failure_ends_the_session() {
        use crate::output_core::{OutputCore, TurnWiring};
        use crate::types::{AgentInfo, AgentStatus, InputEvent, StatusSink};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};

        /// 종료 전이를 보는 눈 — `is_live()` 가 거짓인 상태가 곧 종료다(그 술어가 정본).
        struct RecordingStatus(Arc<Mutex<Vec<AgentStatus>>>);
        impl StatusSink for RecordingStatus {
            fn status_changed(&self, _id: Uuid, s: AgentStatus, _e: u32) {
                self.0.lock().expect("status poisoned").push(s);
            }
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        let (spec, script_path) =
            bake_fake_app_server(&Uuid::new_v4().to_string(), FakeResume::EchoesTheThreadId);

        // 불렸다는 사실만 남기고 터진다 — 「패닉이 났다」와 「아예 안 불렸다」를 갈라야 하기 때문.
        let reached = Arc::new(AtomicBool::new(false));
        let sink: SessionIdSink = {
            let reached = reached.clone();
            Arc::new(move |_: &str| {
                reached.store(true, Ordering::SeqCst);
                panic!("기록 포트가 터졌다");
            })
        };

        let parts = crate::backend::open_spawn(
            &codex_app_server(vec![]),
            &spec,
            80,
            24,
            Some(sink),
            None,
            None,
            None,
        )
        .expect("open_spawn");

        let statuses: Arc<Mutex<Vec<AgentStatus>>> = Arc::new(Mutex::new(Vec::new()));
        let terminal = {
            let statuses = statuses.clone();
            move || {
                statuses
                    .lock()
                    .expect("status poisoned")
                    .iter()
                    .any(|s| !s.is_live())
            }
        };

        // ★훅 교체를 맨손으로 하지 않는다★ — 전역이라 같은 바이너리의 다른 항목이 자기 패닉 출력을 잃고,
        //   중첩되면 조용한 훅이 영구히 남는다. 그 둘을 막는 헬퍼를 쓴다. 라이터 스레드가 이 구간 안에서
        //   터지므로 구간이 그 시점을 덮어야 한다.
        // ★조용한 훅 구간은 **패닉 순간까지만** 잡는다★ — 그 훅은 프로세스 전역이고 헬퍼가 static
        //   뮤텍스로 직렬화하므로, 종료 대기까지 감싸면 같은 바이너리의 다른 패닉 항목이 그만큼 줄을 서고
        //   그 창에 터진 **무관한** 항목이 자기 패닉 메시지와 위치를 잃는다. 종료 대기는 패닉과 무관하므로
        //   구간 밖으로 뺀다(최악 대기가 절반으로 준다).
        let (reached, refusal) = engram_dashboard_command::testing::with_quiet_panic_hook(|| {
            parts.transport.start(Arc::new(OutputCore::new(
                Uuid::new_v4(),
                1,
                Arc::new(RecordingStatus(statuses.clone())),
                TurnWiring::detached(),
            )));

            // ★먼저 기록 동사가 불릴 때까지 **아무것도 보내지 않고** 기다린다★ — 핸드셰이크 전의
            //   `send_input` 은 정상적으로 큐에 서므로(ADR-0190), 여기서 보내면 상한(32)을 채워
            //   **큐 가득참 오류**가 나고 이 항목이 엉뚱한 이유로 초록이 된다.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !reached.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }

            // 링크를 내리는 것은 그 다음 몇 마이크로초다. 시도는 상한보다 한참 적게 둔다 — 같은 이유.
            let mut refusal = None;
            for _ in 0..8 {
                match parts.transport.send_input(InputEvent::Raw(b"hi".to_vec())) {
                    Err(e) => {
                        refusal = Some(e);
                        break;
                    }
                    Ok(()) => std::thread::sleep(std::time::Duration::from_millis(50)),
                }
            }

            (reached.load(Ordering::SeqCst), refusal)
        });

        // stdin 을 놓은 뒤 상대가 스스로 끝나고(실측 41–51ms) 그 EOF 가 pump 를 끝내기까지 기다린다.
        //   ★`shutdown()` 을 부르지 않고 기다리는 것이 요점이다★ — 우리가 죽여 놓고 「끝났다」를 재면 이
        //   항목이 재려던 그 인과를 우리가 대신 굴린 것이 된다.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !terminal() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        let reached_terminal = terminal();
        let seen = statuses.lock().expect("status poisoned").clone();
        // ★`!is_live()` 로도 `Exited { .. }` 로도 **부족하다**★ — 리더가 EOF 를 보면 자식의 생사와 무관하게
        //   `Exited { code: None }` 이 서서, 종류만 보는 단언은 위 `reached_terminal` 이 참일 때 거짓이 될
        //   수 없었다(옛 모양은 공허했다 — 리뷰 지적. 그 복사본이 옆 항목에도 있었고 함께 고쳤다).
        // ★코드가 실려 있는 것이 자식이 실제로 수거됐다는 유일한 증거다★ — 사유의 정본은
        //   `a_rejected_resume_does_not_fall_back_and_ends_the_session` 의 같은 자리.
        let ended_with_a_reaped_child = seen
            .iter()
            .any(|s| matches!(s, AgentStatus::Exited { code: Some(_) }));
        parts.transport.shutdown();

        assert!(
            reached,
            "기록 동사가 한 번도 불리지 않았다 — 이 항목은 실패 처리를 재지 못했다. 가짜 app-server 가 안 \
             떴을 수 있다(스크립트는 남겨 둔다): {}",
            script_path.display()
        );
        let removed = remove_script(&script_path);

        let refusal = refusal.expect(
            "기록이 실패했는데 입력이 계속 받아들여진다 — 링크가 `Connecting` 에 멈춰 세션이 조용히 벙어리가 됐다",
        );
        let PtyError::WriteFailed(reason) = &refusal else {
            panic!("예상 밖 오류 종류: {refusal:?}");
        };
        // ★큐 가득참만 배제한다★ — 그것만이 「실패 경로가 안 돌았는데 거절됐다」를 뜻한다. 남은 두 사유
        //   (기록 실패로 내려간 링크 · 그 뒤 stdin 을 닫아 끝난 통로)는 **둘 다 이 경로가 돈 증거**이고,
        //   어느 쪽이 잡히나는 타이밍이라 하나로 못 박으면 그 자체가 깜빡이가 된다.
        assert!(
            !reason.contains("상한"),
            "거절 사유가 큐 가득참이다 — 실패 경로가 돈 것을 잰 것이 아니다: {reason}"
        );

        assert!(
            reached_terminal,
            "기록이 실패했는데 세션이 종료 상태에 닿지 않았다 — 자식·리더·라이터가 그대로 붙들려 있고 pump \
             도 reaper 도 움직이지 않는다(관측된 상태: {seen:?})"
        );
        assert!(
            ended_with_a_reaped_child,
            "종점에 닿았는데 자식이 수거된 흔적이 없다 — `Failed` 와 `Exited{{code:None}}` 는 자식이 살아 \
             있어도 서므로, 그것만으로는 「세션만 끝나고 프로세스는 남았다」와 구별되지 않는다(관측: {seen:?})"
        );
        assert!(
            removed,
            "가짜 app-server 스크립트를 지우지 못했다: {}",
            script_path.display()
        );
    }

    // ── ADR-0209: MCP 서버 부착(`-c mcp_servers.<name>=…`) — codex 우편의 유일한 물리 배선 ──────────

    /// `-c` **바로 뒤** 칸들만 모은다 — 순서·인접을 재는 단언이 그 형태에 기댄다.
    fn config_override_values(argv: &[String]) -> Vec<String> {
        argv.windows(2)
            .filter(|p| p[0] == CONFIG_OVERRIDE_FLAG)
            .map(|p| p[1].clone())
            .collect()
    }

    /// ★생성되는 값을 **글자 그대로** 못 박는다★ — 이 문자열은 codex 설정 파서가 읽는 wire 계약이고,
    /// 조립 중 한 글자만 어긋나도 증상은 오류가 아니라 「우편이 조용히 안 된다」다. 실 codex 로 이
    /// 모양을 확인했다(2026-09-19 · 0.155.0: `--strict-config` 통과 + `mcpServerStatus/list` 에 등록).
    #[test]
    fn the_mcp_override_value_is_pinned_byte_for_byte() {
        let s = spec_with_control(&codex(vec![]), Some(endpoint()));
        assert!(
            config_override_values(&codex_argv(&s)).contains(
                &"mcp_servers.engram={url='http://127.0.0.1:7777/mcp',bearer_token_env_var='ENGRAM_TOKEN',default_tools_approval_mode='approve'}"
                    .to_string()
            ),
            "생성된 `-c` 값들: {:?}",
            config_override_values(&codex_argv(&s))
        );
    }

    /// ★토큰이 argv 에 한 글자도 없어야 한다★ — 이 백엔드가 값 인라인 대신 env 변수 **이름**을 싣는
    /// 이유가 이것이다. 되돌리면 같은 사용자의 아무 프로세스나 argv 를 읽는 것만으로 토큰이 샌다.
    #[test]
    fn the_bearer_token_never_reaches_the_command_line() {
        let ep = endpoint();
        let token = ep.token.clone();
        for command in [codex(vec![]), codex_app_server(vec![])] {
            let s = spec_with_control(&command, Some(ep.clone()));
            assert!(
                !s.args.iter().any(|a| a.contains(&token)),
                "토큰이 argv 에 실렸다: {:?}",
                s.args
            );
            assert_eq!(
                env_value(&s, TOKEN_ENV),
                Some(token.as_str()),
                "토큰은 env 한 벌로만 가야 한다"
            );
        }
    }

    /// ★모드를 가르지 않는다★ — 우편 입구는 두 모드 다 필요하다. 훅 등록이 터미널에만 걸린 것과
    /// 혼동해 이 축까지 모드로 가르면 app-server 로 뜬 codex 는 발신 입구가 0 이 된다.
    #[test]
    fn the_mcp_override_rides_both_output_modes() {
        for command in [codex(vec![]), codex_app_server(vec![])] {
            let s = spec_with_control(&command, Some(endpoint()));
            let values = config_override_values(&codex_argv(&s));
            assert!(
                values
                    .iter()
                    .any(|v| v
                        .starts_with(&format!("{MCP_SERVER_OVERRIDE_PREFIX}{MCP_SERVER_NAME}="))),
                "{command:?} 에 MCP 부착이 빠졌다: {values:?}"
            );
        }
    }

    /// 제어 채널이 없는 스폰에는 붙일 것이 없다 — 빈 url 로 값을 지어내면 codex 가 못 뜨는 서버를 들고 뜬다.
    #[test]
    fn no_control_endpoint_means_no_mcp_override() {
        let s = spec(&codex(vec![]), "C:/workspace");
        assert!(
            !codex_argv(&s)
                .iter()
                .any(|a| a.starts_with(MCP_SERVER_OVERRIDE_PREFIX)),
            "endpoint 없는 스폰에 MCP 부착이 실렸다: {:?}",
            codex_argv(&s)
        );
    }

    /// ★운영자가 MCP 우편을 끄면 **codex 에서도** 꺼져야 한다(적출 2026-09-20)★: 옛 게이트는 제어
    /// 채널의 **존재**만 봤다 — 그래서 `ENGRAM_DISALLOW_MCP_SEND=1` 을 건 운영자는 claude 쪽 발신 입구만
    /// 잃고 codex 쪽은 그대로 열어 둔 상태를 얻었다. 게다가 그 부착은
    /// `default_tools_approval_mode='approve'` 를 싣고 있어 **자동 승인**된 채로 남는다.
    /// ★켠 행을 **함께** 재는 것이 요점이다★ — 끈 쪽만 재면 부착을 통째로 지워도 초록이고, 그 순간
    ///   codex 우편의 유일한 물리 배선이 사라진다(증상은 오류가 아니라 침묵이다).
    /// ★제어 평면 자체는 끊기지 않는다는 것도 함께 잰다★ — 우편을 끄는 것과 제어 동사를 끊는 것은
    ///   다른 일이고(ADR-0132 결정 5), 여기서 자격증명까지 사라지면 codex 가 띄우는 자식들이 그것을 잃는다.
    #[test]
    fn turning_mcp_send_off_takes_the_override_off_codex_too() {
        for command in [codex(vec![]), codex_app_server(vec![])] {
            let off = spec_with_control(&command, Some(endpoint_without_mcp_send()));
            assert!(
                !codex_argv(&off)
                    .iter()
                    .any(|a| a.starts_with(MCP_SERVER_OVERRIDE_PREFIX)),
                "{command:?}: MCP 발신 grant 가 없는데 부착이 실렸다 — 데몬의 차단이 이 backend 만 \
                 비껴간다: {:?}",
                codex_argv(&off)
            );
            assert_eq!(
                env_value(&off, TOKEN_ENV),
                Some("deadbeef"),
                "{command:?}: 우편을 껐다고 제어 평면 자격증명까지 사라지면 안 된다"
            );
            let on = spec_with_control(&command, Some(endpoint()));
            assert!(
                codex_argv(&on)
                    .iter()
                    .any(|a| a.starts_with(MCP_SERVER_OVERRIDE_PREFIX)),
                "{command:?}: 인가된 스폰에 부착이 빠졌다 — 발신 입구가 0 이 된다: {:?}",
                codex_argv(&on)
            );
        }
    }

    /// ★「배달 명단엔 오르는데 발신 입구가 0」인 스폰을 뜨기 전에 끊는다★: 이 backend 는
    /// `reads_messages` 가 true 라 배달 명단에 오르고, `accepts_mcp_config` 가 true 라 데몬이
    /// `mail_allowed=false` 를 실어 CLI 미러를 닫아 둔다 — 그 상태에서 MCP 부착까지 빠지면 도착한
    /// `request` 가 전부 **아무도 답할 수 없는 계약**이 된다. 그 결말은 오류가 아니라 영원한 무응답이라
    /// 아무 데도 안 남는다.
    /// ★claude 는 이 자리를 데몬의 `?`(mcp-config write 실패)가 지킨다 — 이 backend 는 파일을 안 써서
    ///   데몬에 끊을 재료가 없다★. 그 비대칭을 메우는 것이 이 게이트다.
    /// ★공개 dispatch 로 부른다★ — impl 에만 걸면 조립점이 부르는 경로가 안 잡힌다.
    #[test]
    fn an_unbuildable_attachment_fails_the_spawn_closed() {
        let bad = "http://127.0.0.1:1/it's/mcp";
        for command in [codex(vec![]), codex_app_server(vec![])] {
            let ep = ControlEndpoint {
                url: bad.to_string(),
                ..endpoint()
            };
            let err = crate::backend::precheck_control_endpoint(&command, Some(&ep))
                .expect_err("부착을 못 만드는데 스폰이 그대로 진행된다 — 발신 입구 0 으로 뜬다");
            assert!(
                err.contains(bad),
                "{command:?}: 사유에 원인이 있어야: {err}"
            );
            // 게이트가 있어도 조립은 **여전히** 인자를 지어내지 않는다 — 두 벽이 각각 선다.
            let s = spec_with_control(&command, Some(ep));
            assert!(
                !codex_argv(&s)
                    .iter()
                    .any(|a| a.starts_with(MCP_SERVER_OVERRIDE_PREFIX)),
                "{command:?}: 못 만드는 값을 억지로 실었다: {:?}",
                codex_argv(&s)
            );
        }
    }

    /// ★정당한 부재 셋을 **함께** 못 박는다 — 없으면 위 게이트가 멀쩡한 스폰까지 죽이는 쪽으로 자란다★.
    /// 특히 ②가 load-bearing 이다: 운영자가 MCP 우편을 끈 스폰은 붙일 의사가 애초에 없으므로, 주소가
    /// 어떻든 끊을 이유가 없다.
    #[test]
    fn the_legitimate_absences_still_spawn() {
        let bad = "http://127.0.0.1:1/it's/mcp";
        for command in [codex(vec![]), codex_app_server(vec![])] {
            // ① 제어 채널 자체가 없다 — 우편 없이 뜨는 codex 는 정상이다.
            assert!(
                crate::backend::precheck_control_endpoint(&command, None).is_ok(),
                "{command:?}: 제어 채널 없는 스폰을 끊었다"
            );
            // ② 데몬이 MCP 발신 입구를 인가하지 않았다(운영자가 껐다) — 주소를 못 실어도 무관하다.
            let off = ControlEndpoint {
                url: bad.to_string(),
                ..endpoint_without_mcp_send()
            };
            assert!(
                crate::backend::precheck_control_endpoint(&command, Some(&off)).is_ok(),
                "{command:?}: 붙일 의사가 없는 스폰을 부착 불가로 끊었다 — 노브를 켠 하네스에서 codex \
                 스폰이 통째로 죽는다"
            );
            // ③ 평범한 운영 스폰.
            assert!(
                crate::backend::precheck_control_endpoint(&command, Some(&endpoint())).is_ok(),
                "{command:?}: 운영 endpoint 를 끊었다"
            );
        }
    }

    /// ★TOML 리터럴 문자열에 담을 수 없는 주소는 **싣지 않고 끊는다**★ — 지어낸 이스케이프로 밀어 넣으면
    /// 값이 파싱에서 죽거나 다른 주소를 가리키고, 둘 다 증상이 「우편이 안 된다」 하나다.
    #[test]
    fn an_unrepresentable_control_url_drops_the_attachment_instead_of_mangling_it() {
        for bad in [
            "http://127.0.0.1:1/it's/mcp",
            "http://127.0.0.1:1/%USER%/mcp",
        ] {
            let mut ep = endpoint();
            ep.url = bad.to_string();
            let s = spec_with_control(&codex(vec![]), Some(ep));
            assert!(
                !codex_argv(&s)
                    .iter()
                    .any(|a| a.starts_with(MCP_SERVER_OVERRIDE_PREFIX)),
                "{bad} 로 부착 값을 만들었다: {:?}",
                codex_argv(&s)
            );
        }
    }

    /// ★패스스루가 같은 키를 세워도 우리 것이 **뒤에** 실린다★ — 마지막 `-c` 가 이기므로 이 순서가
    /// 곧 우편 입구의 생사다. 그리고 사용자 인자를 지우지 않는다(거르는 것은 검열이다).
    #[test]
    fn a_conflicting_mcp_passthrough_loses_to_ours_and_is_not_filtered() {
        let key = format!("{MCP_SERVER_OVERRIDE_PREFIX}{MCP_SERVER_NAME}");
        assert!(
            passthrough_overrides_key(
                &[
                    CONFIG_OVERRIDE_FLAG.to_string(),
                    format!("{key}={{url='http://evil/mcp'}}")
                ],
                &key,
            ),
            "같은 키의 패스스루를 못 알아봤다 — 경고 없이 사용자 설정을 덮는다"
        );
        let s = spec_with_control(
            &codex(vec![
                CONFIG_OVERRIDE_FLAG,
                "mcp_servers.engram={url='http://evil/mcp'}",
            ]),
            Some(endpoint()),
        );
        let values = config_override_values(&codex_argv(&s));
        let ours = values
            .iter()
            .position(|v| v.contains(MCP_BEARER_ENV_KEY))
            .expect("우리 부착 값이 없다");
        let theirs = values
            .iter()
            .position(|v| v.contains("evil"))
            .expect("패스스루가 걸러졌다 — 사용자 인자를 검열하면 안 된다");
        assert!(
            ours > theirs,
            "우리 값이 앞에 실렸다(지고 있다): {values:?}"
        );
    }

    /// ★파일 칸이 `Some` 으로 와도 argv 에 닿지 않는다★ — 운영에서는 이제 `None` 으로 오지만
    /// (`writes_mcp_config_file()` = false → 데몬이 안 쓴다), 이 시험은 그 게이트가 무너진 상태를
    /// **일부러** 먹여 두 번째 벽을 잰다. codex 에는 그 둘을 가리킬 플래그가 없어서 「Some 이니 뭔가
    /// 쓰자」로 claude 의 플래그를 베껴 붙이면 기동이 죽는다.
    /// ★셋째 칸(`priming_file`)이 여기 함께 있는 것은 이제 다른 뜻이다★ — 그 파일은 **읽어서 싣는다**
    ///   (ADR-0215). 그래도 **경로**가 argv 에 닿으면 안 되는 것은 그대로라 이 대조에 남긴다(같은 사실을
    ///   실물 파일로 재는 짝 = `the_priming_path_never_reaches_the_command_line`).
    #[test]
    fn the_daemon_written_files_are_still_not_translated_into_argv() {
        let ep = endpoint();
        let s = spec_with_control(&codex(vec![]), Some(ep.clone()));
        let argv = codex_argv(&s);
        for forbidden in ["--mcp-config", "--settings", "--append-system-prompt-file"] {
            assert!(
                !argv.iter().any(|a| a == forbidden),
                "claude 의 플래그 `{forbidden}` 가 codex argv 에 실렸다: {argv:?}"
            );
        }
        for path in [ep.config_path, ep.settings_file, ep.priming_file] {
            let path = path.expect("fixture 는 세 칸을 다 채운다");
            let path = path.to_string_lossy().into_owned();
            assert!(
                !argv.iter().any(|a| a.contains(&path)),
                "경로 `{path}` 가 argv 에 실렸다: {argv:?}"
            );
        }
    }

    // ── ADR-0215: 프라이밍 주입(터미널 = `-c developer_instructions=…` · app-server = 핸드셰이크 JSON) ──

    /// 지시서 파일을 임시 폴더에 구워 그것을 가리키는 endpoint 를 돌려준다.
    ///
    /// ★경로를 함께 돌려주는 것이 계약이다★ — 항목마다 지워야 한다. 기본 fixture 의
    /// `C:/engram/priming.md` 는 **없는 파일**이라 읽기 실패 갈래로 떨어진다(그래서 다른 시험의 argv 가
    /// 이 주입으로 늘지 않는다). 주입을 재려면 실물이 있어야 한다.
    fn bake_priming(text: &str) -> (ControlEndpoint, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("engram-priming-{}.md", Uuid::new_v4()));
        std::fs::write(&path, text).expect("프라이밍 파일 기록");
        (
            ControlEndpoint {
                priming_file: Some(path.clone()),
                ..endpoint()
            },
            path,
        )
    }

    /// 데몬이 프라이밍을 안 실어 준 스폰(비-MCP 갈래의 endpoint 모양).
    fn endpoint_without_priming() -> ControlEndpoint {
        ControlEndpoint {
            priming_file: None,
            ..endpoint()
        }
    }

    fn developer_instructions_value(argv: &[String]) -> Option<String> {
        let prefix = format!("{DEVELOPER_INSTRUCTIONS_KEY}=");
        config_override_values(argv)
            .into_iter()
            .find(|v| v.starts_with(&prefix))
    }

    /// 건너뛴 갈래가 **정말로 아무 것도 안 바꿨나** — 프라이밍 없는 스폰과 argv 가 바이트 단위로 같나.
    ///
    /// ★「주입이 없다」만 재면 부족하다★: 값을 못 만든 갈래가 인자를 반쯤 밀어 넣거나 순서를 흔들어도
    ///   그 단언은 통과한다. 이 대조가 「스폰은 그대로 간다」의 실물이다.
    fn assert_argv_matches_a_spawn_without_priming(argv: &[String]) {
        let bare = spec_with_control(&codex(vec![]), Some(endpoint_without_priming()));
        assert_eq!(
            argv,
            codex_argv(&bare),
            "프라이밍을 건너뛴 argv 가 프라이밍 없는 스폰과 다르다"
        );
    }

    /// ★값을 **글자 그대로** 못 박는다★ — codex 설정 파서가 읽는 wire 계약이고, 작은따옴표(TOML 리터럴
    /// 문자열)와 줄바꿈 두 글자 치환이 둘 다 load-bearing 이다.
    #[test]
    fn the_terminal_spawn_carries_the_priming_document_as_developer_instructions() {
        let (ep, path) = bake_priming("line one\nline two\n");
        let s = spec_with_control(&codex(vec![]), Some(ep));
        let value = developer_instructions_value(&codex_argv(&s)).expect("프라이밍이 안 실렸다");
        assert_eq!(value, "developer_instructions='line one\\nline two\\n'");
        std::fs::remove_file(&path).expect("임시 지시서 삭제");
    }

    /// ★실제 줄바꿈이 한 글자라도 남으면 cmd 가 그 자리에서 명령줄을 자른다(오류 없음)★ — 세 형태
    /// (`\r\n`·`\n`·`\r`)를 한 문서에 섞어, 어느 하나만 바꾸는 구현을 잡는다.
    #[test]
    fn every_line_break_shape_collapses_into_one_line() {
        let (ep, path) = bake_priming("a\r\nb\rc\nd");
        let s = spec_with_control(&codex(vec![]), Some(ep));
        let value = developer_instructions_value(&codex_argv(&s)).expect("프라이밍이 안 실렸다");
        assert_eq!(value, "developer_instructions='a\\nb\\nc\\nd'");
        assert!(
            !value.contains('\n') && !value.contains('\r'),
            "값에 실제 줄바꿈이 남았다: {value:?}"
        );
        std::fs::remove_file(&path).expect("임시 지시서 삭제");
    }

    /// ★작은따옴표 하나가 TOML 리터럴 문자열을 깬다 — 영어 축약형 한 번이면 들어온다★.
    #[test]
    fn a_quoted_priming_document_is_skipped_and_the_spawn_still_stands() {
        let (ep, path) = bake_priming("do not route around it — tell your principal you can't");
        let s = spec_with_control(&codex(vec![]), Some(ep));
        let argv = codex_argv(&s);
        assert!(
            developer_instructions_value(&argv).is_none(),
            "작은따옴표가 든 지시서가 그대로 실렸다: {argv:?}"
        );
        assert_argv_matches_a_spawn_without_priming(&argv);
        std::fs::remove_file(&path).expect("임시 지시서 삭제");
    }

    /// ★`%NAME%` 은 cmd 가 **따옴표 안에서도** 편다★ — 그대로 실으면 에이전트가 읽는 문서에 이 기기의
    /// 환경변수 값이 박히거나(치환 성공) 문장이 통째로 사라진다(미정의 변수).
    #[test]
    fn a_priming_document_with_a_percent_sign_is_skipped_and_the_spawn_still_stands() {
        let (ep, path) = bake_priming("progress is 50% done");
        let s = spec_with_control(&codex(vec![]), Some(ep));
        let argv = codex_argv(&s);
        assert!(
            developer_instructions_value(&argv).is_none(),
            "`%` 가 든 지시서가 그대로 실렸다: {argv:?}"
        );
        assert_argv_matches_a_spawn_without_priming(&argv);
        std::fs::remove_file(&path).expect("임시 지시서 삭제");
    }

    /// ★예산을 넘는 지시서는 **스폰을 죽이지 않고** 건너뛴다★ — 상한을 넘긴 명령줄은 오류가 아니라
    /// 절단으로 벌하므로, MCP 부착과 훅 등록까지 함께 사라지는 것이 이 갈래의 진짜 대가다.
    #[test]
    fn an_oversized_priming_document_is_skipped_and_the_spawn_still_stands() {
        let (ep, path) = bake_priming(&"x".repeat(CMD_LINE_LIMIT_UTF16));
        let s = spec_with_control(&codex(vec![]), Some(ep));
        let argv = codex_argv(&s);
        assert!(
            developer_instructions_value(&argv).is_none(),
            "예산을 넘는 지시서가 실렸다(길이 {})",
            argv.iter().map(|a| a.len()).sum::<usize>()
        );
        assert_argv_matches_a_spawn_without_priming(&argv);
        std::fs::remove_file(&path).expect("임시 지시서 삭제");
    }

    /// ★예산은 **잔액**으로 판정한다 — 우리 값만 재는 것이 아니다★: 같은 지시서가 패스스루가 긴
    /// 스폰에서는 건너뛰어진다. 고정 상수 비교로 되돌리면 이 항목이 무너진다.
    #[test]
    fn the_budget_counts_what_the_rest_of_the_command_line_already_spent() {
        let text = "y".repeat(CMD_LINE_LIMIT_UTF16 / 2);
        let (ep, path) = bake_priming(&text);
        let roomy = spec_with_control(&codex(vec![]), Some(ep.clone()));
        assert!(
            developer_instructions_value(&codex_argv(&roomy)).is_some(),
            "여유 있는 명령줄에서 건너뛰었다"
        );
        let filler = "z".repeat(CMD_LINE_LIMIT_UTF16 / 2);
        let crowded = spec_with_control(&codex(vec![filler.as_str()]), Some(ep));
        assert!(
            developer_instructions_value(&codex_argv(&crowded)).is_none(),
            "패스스루가 예산을 다 쓴 명령줄에 그대로 실었다"
        );
        std::fs::remove_file(&path).expect("임시 지시서 삭제");
    }

    /// ★패스스루 **뒤**여야 한다★ — 같은 키를 `-c` 로 두 번 넘기면 마지막이 이긴다. 앞에 두면 사용자
    /// 인자 한 줄이 수신 계약 프라이밍을 아무 신호 없이 지운다(훅·MCP 부착과 같은 규율).
    #[test]
    fn the_priming_override_follows_the_passthrough() {
        let (ep, path) = bake_priming("teach the reply contract");
        let s = spec_with_control(
            &codex(vec![
                CONFIG_OVERRIDE_FLAG,
                "developer_instructions='theirs'",
            ]),
            Some(ep),
        );
        let values = config_override_values(&codex_argv(&s));
        let ours = values
            .iter()
            .position(|v| v.contains("teach the reply contract"))
            .expect("우리 값이 없다");
        let theirs = values
            .iter()
            .position(|v| v.contains("theirs"))
            .expect("패스스루가 걸러졌다 — 사용자 인자를 검열하면 안 된다");
        assert!(
            ours > theirs,
            "우리 값이 앞에 실렸다(지고 있다): {values:?}"
        );
        std::fs::remove_file(&path).expect("임시 지시서 삭제");
    }

    /// ★app-server 갈래는 명령줄을 한 글자도 안 쓴다 — 그리고 값을 **손대지 않는다**★: 줄바꿈도,
    /// 터미널 갈래가 끊어 내는 `'`·`%` 도 JSON 본문에서는 아무 것도 못 깬다. 그쪽 변환을 이리 옮기면
    /// 아무 위험도 막지 못한 채 에이전트가 읽는 문서만 망가진다.
    #[test]
    fn the_app_server_handshake_carries_the_document_unmodified() {
        let text = "line one\nyou can't skip 50% of it\n";
        let (ep, path) = bake_priming(text);
        let s = spec_with_control(&codex_app_server(vec![]), Some(ep.clone()));
        assert!(
            developer_instructions_value(&codex_argv(&s)).is_none(),
            "app-server 갈래가 명령줄로도 실었다: {:?}",
            codex_argv(&s)
        );
        let params = match thread_open(&s, None, priming_text(Some(&ep))) {
            ThreadOpen::Start(p) => p,
            ThreadOpen::Resume(_) => panic!("이어받기가 아닌데 resume 이 골라졌다"),
        };
        let v = serde_json::to_value(&params).expect("직렬화");
        assert_eq!(
            v.get("developerInstructions").and_then(|v| v.as_str()),
            Some(text),
            "핸드셰이크 JSON 의 지시문이 원문과 다르다: {v}"
        );
        std::fs::remove_file(&path).expect("임시 지시서 삭제");
    }

    /// ★이어받은 app-server 스레드는 프라이밍을 못 받는다 — 회귀가 아니라 결정이다★
    /// (`ThreadResumeParams` 의 doc 이 사유의 정본). 그 칸이 실측으로 확인돼 생기는 날 이 항목이
    /// 빨개지면서 함께 고쳐진다.
    #[test]
    fn a_resuming_app_server_handshake_carries_no_instructions_field() {
        let (ep, path) = bake_priming("teach the reply contract");
        let s = spec_with_control(&codex_app_server(vec![]), Some(ep.clone()));
        let params = match thread_open(&s, Some(Uuid::new_v4()), priming_text(Some(&ep))) {
            ThreadOpen::Resume(p) => p,
            ThreadOpen::Start(_) => panic!("이어받기인데 start 가 골라졌다"),
        };
        let v = serde_json::to_value(&params).expect("직렬화");
        assert!(
            v.get("developerInstructions").is_none(),
            "재 본 적 없는 칸이 이어받기 요청에 실렸다: {v}"
        );
        std::fs::remove_file(&path).expect("임시 지시서 삭제");
    }

    /// 데몬이 프라이밍을 안 실어 준 스폰(`wants_priming` 이 false 인 갈래)은 argv 가 한 칸도 늘지 않는다.
    #[test]
    fn without_a_priming_file_nothing_is_injected() {
        for command in [codex(vec![]), codex_app_server(vec![])] {
            let s = spec_with_control(&command, Some(endpoint_without_priming()));
            assert!(
                developer_instructions_value(&codex_argv(&s)).is_none(),
                "{command:?}: 경로가 없는데 실렸다"
            );
        }
        assert!(priming_text(Some(&endpoint_without_priming())).is_none());
        assert!(priming_text(None).is_none());
    }

    /// ★읽기 실패는 fail-open 이다★ — 경고만 남기고 스폰은 그대로 간다. 오류로 올리면 지시서 파일 하나가
    /// 사라진 설치에서 codex 스폰이 통째로 죽는다.
    #[test]
    fn an_unreadable_priming_file_is_skipped_in_both_modes() {
        let missing =
            std::env::temp_dir().join(format!("engram-priming-absent-{}.md", Uuid::new_v4()));
        let ep = ControlEndpoint {
            priming_file: Some(missing),
            ..endpoint()
        };
        assert!(priming_text(Some(&ep)).is_none(), "없는 파일이 읽혔다");
        for command in [codex(vec![]), codex_app_server(vec![])] {
            let s = spec_with_control(&command, Some(ep.clone()));
            let argv = codex_argv(&s);
            assert!(
                developer_instructions_value(&argv).is_none(),
                "{command:?}: 없는 파일인데 실렸다"
            );
        }
        assert_argv_matches_a_spawn_without_priming(&codex_argv(&spec_with_control(
            &codex(vec![]),
            Some(ep),
        )));
    }

    /// 빈 지시서는 명령줄만 쓰고 아무 것도 안 가르친다 — 실을 이유가 없다.
    #[test]
    fn an_empty_priming_file_is_not_injected() {
        let (ep, path) = bake_priming("   \n\t\n");
        assert!(priming_text(Some(&ep)).is_none(), "빈 파일이 실렸다");
        std::fs::remove_file(&path).expect("임시 지시서 삭제");
    }

    /// ★경로는 여전히 argv 에 닿지 않는다★ — 실리는 것은 **내용**이다. 경로가 새면 codex 가 모르는
    /// 낱말을 첫 프롬프트로 먹는다(`[PROMPT]` 위치 인자).
    #[test]
    fn the_priming_path_never_reaches_the_command_line() {
        let (ep, path) = bake_priming("teach the reply contract");
        let s = spec_with_control(&codex(vec![]), Some(ep));
        let needle = path.to_string_lossy().into_owned();
        assert!(
            !s.args.iter().any(|a| a.contains(&needle)),
            "지시서 경로가 argv 에 실렸다: {:?}",
            s.args
        );
        std::fs::remove_file(&path).expect("임시 지시서 삭제");
    }

    #[test]
    fn cwd_and_env_are_forwarded() {
        let cwd = PathBuf::from("C:/workspace");
        let env = vec![("BAR".to_string(), "baz".to_string())];
        let s = CodexBackend.build_spec(
            &codex(vec![]),
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
