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
//!   2. 우리가 안 쓰는 인자(`-m` · MCP `-c mcp_servers.…` 오버라이드 문법) — 이 파일 주석에만 있고
//!      재는 곳이 없다.
//!   2-1. ★`codex resume <id>` 는 이제 **우리가 쓴다** — 그런데도 시험대가 실물로 재지 않는다★:
//!      [`production_spec`](../../../tests/backend_contract.rs) 이 `SpawnMode::Fresh` 로만 argv 를 뽑아
//!      이어받기 갈래가 그 레인에 애초에 안 실린다. 자동으로 재려면 그 파일에 살아 있는 스레드 id 를
//!      공급할 길이 먼저 필요하다.
//!      ★단 **동작 자체는 손으로 실측됐다(2026-09-19 · 0.155.0)** — 「기립하는 것까지는 아무도 안 본다」로
//!        적혀 있던 옛 문장은 더 이상 참이 아니다★: 그 argv 로 실제로 떴고, **세션 id 가 보존된다**. 새
//!        세션이 id S 를 `source:"startup"` 으로 신고하고, 이어받으면 **같은 S** 가 `source:"resume"` 으로
//!        다시 오며(반복 이어받기에도 안정) 기록은 S 의 원래 rollout 파일에 이어 붙는다. 그래서 이어받은
//!        세션의 훅 보고는 충돌이 아니라 「이미 같은 값」으로 끝나고, 낡은 id 로 흘러가는 갈래가 없다.
//!   3. 아래 `build_spec` 의 `%VAR%` 한계(그 자리 주석이 정본).
//!
//! capability 선언이 그 표와 어긋나면 시험대의 **비-`#[ignore]`** 항목이 빨개진다.
//!
//! tauri import 0.

pub(crate) mod decoder;
pub(crate) mod protocol;
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
// ADR-0185
fn thread_open(spec: &CommandSpec, resume_session_id: Option<Uuid>) -> ThreadOpen {
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

/// codex 설정을 명령줄에서 덮어쓰는 플래그. ★훅 등록에 파일을 하나도 쓰지 않게 하는 수단이 이것 하나다★
/// — 사용자 홈(`$CODEX_HOME/config.toml`)에도, 래퍼 스크립트에도 우리는 한 글자도 쓰지 않는다.
const CONFIG_OVERRIDE_FLAG: &str = "-c";

/// `SessionStart` 훅 표의 키. ★이 오버라이드는 사용자 자신의 config.toml 훅을 **대체하지 않고 공존한다**★
/// (실측 0.155.0) — 그래서 여기 우리 항목을 걸어도 사용자 훅이 사라지지 않는다.
const SESSION_START_HOOK_KEY: &str = "hooks.SessionStart";

/// 훅이 부를 우리 CLI 의 계열+동사. ★정본은 그 CLI 의 파서다★ —
/// `crates/engram-dashboard-daemon/src/bin/engram.rs` 의 `CLI_GROUP_HOOK` + `CLI_HOOK_VERB_SESSION_START`
/// 이고, 그쪽 `run_hook` 은 계열 뒤 argv 가 **정확히 한 낱말 `session-start`** 일 것을 요구한다.
/// ★어긋나도 아무 데도 안 남는다 — 그래서 손으로 맞춘다★: 그 CLI 는 반려 갈래에서도 exit 0 에 stdout
/// 봉인이라(ADR-0208 결정 3), 이 문자열에 오타 한 글자가 나면 증상은 「세션 id 가 영영 안 온다」 하나다.
const HOOK_REPORT_ARGV: &str = "hook session-start";

/// 상주 JSON 서버로 띄우는 하위 명령과 그 전송 선택(실측 0.154.0 — `--stdio` 는 `--listen stdio://` 와
/// 같고 그것이 기본값이다. 기본값에 기대지 않고 명시한다: 이 통로는 stdio 가 아니면 성립하지 않는데,
/// 기본값은 상류가 바꿀 수 있고 바뀌어도 우리 argv 는 조용히 그대로다).
const APP_SERVER_SUBCOMMAND: &str = "app-server";
const APP_SERVER_STDIO_FLAG: &str = "--stdio";

/// 터미널 모드 spawn 에 실을 `SessionStart` 훅 등록 오버라이드 값(`-c` 의 짝). `None` = 걸지 않는다 —
/// ★경고만 남기고 스폰은 그대로 간다★. 그 결과(이어받기가 안 선다)를 사람에게 말하는 자리는 여기가
/// 아니라 조립점이다([`crate::manager::AgentManager`] 의 `opens_a_new_conversation`) — 여기서 또 말하면
/// 같은 사실이 두 출처에서 갈려 적힌다.
///
/// ★파일을 하나도 만들지 않는다★ — 등록은 이 한 값 단독이고, 그래도 codex 의 TUI 신뢰 심사를 거쳐
///   정상 발화한다(실측 0.155.0 — `docs/research/codex-session-id-recovery-survey-2026-09-19.md` §10-1).
/// ★그 대가 = 설치 경로가 바뀌면 신뢰 심사 프롬프트가 한 번 다시 뜬다★: 신뢰 해시가 덮는 것은 훅
///   **정의 문자열**인데(같은 §10-1 결론 3) 그 문자열에 우리 exe 절대경로가 박혀 있기 때문이다.
///   ★그것을 「고치려고」 고정 경로 래퍼를 사용자 홈에 쓰지 말 것★ — 우리가 지우지 못하는 잔여물이
///   사용자 파일계에 남고, 이 함수가 파일을 0개 만드는 성질이 정확히 그것과 맞바꾼 것이다.
/// ★프로그램 경로에 공백이 있으면 codex 가 못 띄운다(실측 2026-09-19)★ — codex 는 `command` 의 **첫
///   공백까지**를 프로그램으로 잘라 셸 없이 직접 spawn 하고 나머지만 따옴표 인지 분할로 인자에 넣는다.
///   그래서 **인자는** 따옴표로 공백을 담을 수 있어도 **프로그램은 못 담고**(감싸도 실패했다), 증상은
///   TUI 의 `Hook failed` / `hook exited with code 1` 이다. 유일한 탈출구가 8.3 단축 경로다.
/// ★스키마에 `args` 배열이 없다★ — 인자는 이 한 문자열 안에 넣는 수밖에 없다(실측).
/// ★`%VAR%` 가 든 경로는 여기서도 새 위험이 아니다★ — 아래 `build_spec` 의 같은 이름 한계 주석이
///   정본이고(cmd 가 명령줄의 `%NAME%` 을 편다), 이 값도 그 cmd 를 지난다. 별도 가드를 두지 않는 것은
///   그 결정을 따르는 것이다.
/// ★이 값은 실 argv 경로를 **바이트 그대로** 건넌다(실측 2026-09-19 · 0.155.0)★ — `cmd.exe /c` 래핑과
///   PATH 의 `codex` `.cmd` shim 을 **둘 다** 지난 뒤 `hooks/list` 가 `command` 칸을 바뀌지 않은 채
///   돌려줬다. 즉 「작은따옴표·역슬래시·공백 없는 경로」 조합은 그 두 겹을 견딘다 — 「shim 을 지나며
///   망가질지 모른다」를 전제로 한 방어를 새로 세우지 말 것(`%VAR%` 한계는 위 문단이 정본이고 그것과
///   별개다).
fn session_start_hook_override(send_exe: Option<&std::path::Path>) -> Option<String> {
    let Some(exe) = send_exe else {
        tracing::warn!(
            "codex SessionStart 훅 미등록 — CLI 실행파일 경로가 없어 세션 id 를 되돌려 받을 창구를 못 건다"
        );
        return None;
    };
    let Some(raw) = exe.to_str() else {
        tracing::warn!("codex SessionStart 훅 미등록 — CLI 경로가 UTF-8 이 아니다: {exe:?}");
        return None;
    };
    // TOML 리터럴 문자열(작은따옴표)에는 이스케이프가 없다 — 그래서 Windows 역슬래시를 그대로 실을 수
    //   있는 대신 작은따옴표 자체는 담을 수 없다. 지어낸 이스케이프로 밀어 넣지 않고 건너뛴다.
    if raw.contains('\'') {
        tracing::warn!("codex SessionStart 훅 미등록 — CLI 경로에 작은따옴표가 있다: {raw}");
        return None;
    }
    // ★탈출구가 있는 공백문자는 **보통 공백 하나뿐**이다★ — 나머지는 8.3 변환으로도 안 없어진다. 탭은
    //   codex 가 프로그램 토큰을 자르는 자리를 옮겨 **다른 프로그램**을 띄우게 하고, 줄바꿈은 한 줄짜리
    //   TOML 리터럴 문자열 자체를 깨 값이 파싱에서 죽는다. 제어문자도 같은 부류다. 어느 쪽이든 증상은
    //   「훅이 안 돈다」 하나라 조용하므로, 실을 수 없는 것은 싣지 않고 여기서 끊는다.
    if let Some(bad) = raw
        .chars()
        .find(|c| c.is_control() || (c.is_whitespace() && *c != ' '))
    {
        tracing::warn!(
            "codex SessionStart 훅 미등록 — CLI 경로에 실을 수 없는 문자가 있다({bad:?}): {raw}"
        );
        return None;
    }
    let program = if raw.contains(' ') {
        match short_program_path(exe) {
            Some(short) => short,
            None => {
                tracing::warn!(
                    "codex SessionStart 훅 미등록 — CLI 경로에 공백이 있는데 8.3 단축 경로를 못 얻었다(걸면 `Hook failed` 로 매 세션 뜬다): {raw}"
                );
                return None;
            }
        }
    } else {
        raw.to_string()
    };
    Some(format!(
        "{SESSION_START_HOOK_KEY}=[{{hooks=[{{type='command',command='{program} {HOOK_REPORT_ARGV}'}}]}}]"
    ))
}

/// 호출자 패스스루가 **우리와 같은 설정 키**를 세우나. `true` = 우리 오버라이드가 그것을 덮는다(뒤에
/// 실리므로) — 사용자가 일부러 건 값을 말없이 지우지 않도록 그 자리에서 경고하게 한다.
///
/// ★잡는 모양은 하나뿐 = `-c` **다음 칸**이 이 키로 시작하는 형태다★(`-c hooks.SessionStart=…`).
/// ★못 잡는 것 — 알고 두는 구멍이다★: `-c` 와 값을 한 낱말로 붙인 형태 · `-c` 말고 긴 이름의 같은
///   플래그 · `hooks` 표 전체를 덮는 상위 키(`-c hooks=…`) · `hooks.SessionStart` 로 **시작만 하는** 다른
///   키. 이 목록을 키워 「확실히」 만들려 들지 말 것 — codex 오버라이드 문법의 재구현이 되고 그 재구현은
///   상류가 바뀔 때마다 조용히 낡는다. 놓쳐서 잃는 것은 **경고뿐이고 동작이 아니다** — 우리 것이 이기는
///   성질은 아래 `build_spec` 의 순서가 따로 보장한다.
fn passthrough_overrides_the_hook_key(extra_args: &[String]) -> bool {
    extra_args
        .windows(2)
        .any(|pair| pair[0] == CONFIG_OVERRIDE_FLAG && pair[1].starts_with(SESSION_START_HOOK_KEY))
}

/// 8.3 단축 경로. `None` = 못 얻었다 — 실물이 없거나, 볼륨이 단축 이름을 안 만들거나, 받은 값에 아직
/// 실을 수 없는 문자(공백문자·제어문자·작은따옴표)가 남아 있다.
///
/// ★두 번 부른다 — 두 호출의 반환이 서로 다른 것을 센다★: 버퍼 없이 부르면 **종단 NUL 을 포함한** 필요
///   길이를, 버퍼를 주고 부르면 **NUL 을 뺀** 기록 길이를 돌려준다(0 = 실패). 한 값으로 접으면 마지막
///   글자가 잘리거나 NUL 이 문자열에 섞인다.
/// ★성공했는데 공백이 남을 수 있다★ — 8.3 생성이 꺼진 볼륨에서 이 API 는 실패가 아니라 **원본을 그대로**
///   돌려준다. 그것을 실으면 우리가 막으려던 그 실패를 우리 손으로 만든다.
#[cfg(windows)]
fn short_program_path(exe: &std::path::Path) -> Option<String> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetShortPathNameW;

    let wide: Vec<u16> = exe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let needed = unsafe { GetShortPathNameW(PCWSTR(wide.as_ptr()), None) };
    if needed == 0 {
        return None;
    }
    let mut buf = vec![0u16; needed as usize];
    let written = unsafe { GetShortPathNameW(PCWSTR(wide.as_ptr()), Some(&mut buf)) };
    if written == 0 || written as usize > buf.len() {
        return None;
    }
    let short = std::ffi::OsString::from_wide(&buf[..written as usize])
        .into_string()
        .ok()?;
    if short
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '\'')
    {
        return None;
    }
    Some(short)
}

/// ★Windows 밖에는 8.3 이름이라는 것이 없다★ — 그래서 공백이 든 경로는 그 플랫폼에서 **언제나** 등록이
/// 건너뛰어진다. 공백 제약 자체는 플랫폼을 안 가린다(codex 가 셸 없이 첫 토큰을 프로그램으로 쓴다).
#[cfg(not(windows))]
fn short_program_path(_exe: &std::path::Path) -> Option<String> {
    None
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
    ///   모드로 뜬 codex 는 손잡이가 명부에 있어도 활성화 입구가 Fresh 로 띄워, 훅이 받아 적어 둔
    ///   ([`crate::profile::ProfileRegistry::adopt_session_id`]) 그 id 가 영영 안 쓰인다.
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
    ///   자리) endpoint 가 `None` 으로 오고, 아래 `build_spec` 의 주입이 한 줄도 돌지 않는다. 그러면
    ///   codex 가 띄우는 훅 프로세스도 자격증명 없이 뜬다.
    /// ★그 훅이 실제로 이 env 를 받는다(실측 2026-09-18, codex 0.155.0)★ — codex 는 `SessionStart` 훅
    ///   프로세스에 자기 env 를 **하나도 덧씌우지 않는다**(순수 상속). 즉 여기서 심은 토큰이 그 훅에
    ///   그대로 도착한다. 공용 주입이 「보조 프로세스의 자격증명이기도 하다」고 적은 조건을 이 백엔드가
    ///   만족한다는 실측이 이것이고, 그 조건 자체는 [`inject_cli_entrance`] doc 이 진다.
    /// ★이 칸이 **우편까지 열지는 않는다 — 그리고 그것이 공짜가 아니었다**★: 예전 데몬은 우편 가부를
    ///   `!accepts_mcp_config` 하나로 파생해서, 이 칸을 켜는 것만으로 이 백엔드에 **보내기 인가가 함께
    ///   열렸다**(받기는 [`AgentBackend::reads_messages`] 가 닫은 채로). 그 비대칭을 없애려고 보내기 축을
    ///   별도 선언으로 뽑았다 — 아래 [`AgentBackend::uses_mail`] 이 그것이고, 데몬은 두 축에서 한 값을
    ///   파생한다. ★그 칸을 지우거나 기본값으로 되돌리면 이 부수효과가 그대로 돌아온다★.
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

    /// ★"codex 가 MCP 를 못 쓴다" 가 아니다★ — 이 칸이 묻는 것은 **우리가 만든 mcp-config 파일을 먹일 수
    /// 있나**이고 그 답이 아니오다. codex 의 MCP 주입은 전역 TOML 오버라이드
    /// (`-c mcp_servers.<name>={…}`)라 claude 의 `--mcp-config <path>` 와 **기제가 다르다**(실측).
    /// 그 다른 기제를 배선하는 것은 이 단계의 범위가 아니다.
    // ADR-0099
    fn accepts_mcp_config(&self) -> bool {
        false
    }

    /// ★분류 사유가 shell 과 다르다★ — shell 이 false 인 것은 입력이 **명령으로 실행되기** 때문이고,
    /// codex 가 false 인 것은 **바쁜 때를 못 가리기** 때문이다. 바쁨 게이트는 fail-open 이라, 턴 신호가
    /// 없는 백엔드는 늘 한가한 것으로 읽혀 **생각하는 도중에 편지가 꽂힌다**.
    /// ★app-server 모드는 이제 그 신호를 낸다★ — 통로가 `turn/completed` 를 읽고(`turn/started` 는
    /// **일부러** 읽지 않는다 — 사유 정본은 그 통로의 `TURN_COMPLETED` doc), 번역기가 같은 알림을 턴
    /// 경계로 옮겨 아래 [`classify_turn`] 이 `Ended` 를 낸다. ★그런데도 이 칸이 false 인 것은 **터미널
    /// 모드 때문**이다★: 이 메서드는 `command` 를 받지 않아 두 모드를 가를 수 없는데, 그 모드는 decoder
    /// 가 없어 `TerminalBytes` 만 흐르고 그래서 신호가 하나도 없다. 여기서 true 를 돌려주면 그 모드까지
    /// 함께 열린다.
    /// ★여는 조건 = 이 축을 모드별로 가르는 것★(시그니처에 명령을 들이거나 자격을 세션 caps 로 옮기거나).
    /// shell 쪽 사유는 그때도 그대로 남으므로 둘을 같이 열지 말 것.
    fn reads_messages(&self) -> bool {
        false
    }

    /// ★받기와 **같이** 닫는다(사용자 결정 2026-09-18)★: 위 칸이 false 인 채로 이 칸만 열리면 「보내기만
    /// 되는」 비대칭이 생기고, 그 비대칭은 `supports_control_channel` 을 켠 부수효과로 **아무도 선언하지
    /// 않은 채** 한 번 생겼던 상태다. 그것을 되돌린 자리가 여기다.
    /// ★제어 동사는 그대로 쓴다★ — 이 칸이 닫는 것은 우편 입구(`/control/send`·`/control/messages`)
    ///   뿐이고, CLI 입구(토큰·주소)와 제어 라우트는 위 `supports_control_channel` 이 연다.
    /// ★여는 조건★: 받기 축을 먼저 열 것(그쪽 doc 의 「여는 조건」). 보내기만 먼저 열면 답장을 못 받는
    ///   발신자가 생기고, 그것은 우편 장부에 영원한 미결로 남는다.
    // ADR-0133
    // ADR-0209
    fn uses_mail(&self) -> bool {
        false
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

                // ★훅 등록은 패스스루 **뒤**다 — 앞에 두면 사용자 인자 한 줄에 우리 등록이 진다★.
                //   실측(codex-cli 0.155.0 · 2026-09-19): 같은 설정 키를 `-c` 로 두 번 넘기면 **마지막
                //   것이 이긴다** — 병합도 없고 중복 키 오류도 없다. 그래서 앞에 두면
                //   `-c hooks.SessionStart=[]` 한 줄이 우리 등록을 통째로 지우고, 그 결말은 `hooks/list`
                //   가 빈 배열 · 신뢰 심사 프롬프트 **없음** · 세션 id 영영 도착 안 함이다. 화면에도
                //   로그에도 아무 신호가 남지 않는다.
                //   ★그러니 「인자 조립을 정돈」한답시고 이 블록을 위 터미널 갈래로 되돌리지 말 것★.
                // ★터미널 모드에만 건다(ADR-0208/ADR-0210)★ — app-server 모드의 세션 id 는 `thread/start` 응답으로
                //   통로가 직접 받아 오고(그쪽 sink 가 언제나 먼저 같은 값을 채운다), 훅 보고는 늘 「이미
                //   같은 값」으로 끝난다. 이 모드에만 받을 창구가 없었다.
                // ★`build_spec` 이 이 값을 만드는 것이 ADR-0004 의 요점이다★ — 조립점은 훅도 `-c` 문법도
                //   모른다. 여기 쓰는 재료는 공용 endpoint 의 `send_exe` 하나이고, 그것은
                //   [`inject_cli_entrance`] 가 `ENGRAM_CLI_EXE`·PATH 에 쓰는 **같은 값**이다 — 실행파일을
                //   찾는 둘째 방법을 만들지 말 것.
                if matches!(output_format, AgentOutputFormat::Terminal) {
                    if let Some(value) = session_start_hook_override(
                        control.as_ref().and_then(|e| e.send_exe.as_deref()),
                    ) {
                        // ★거르지 않고 경고만 한다★ — 패스스루를 지우는 것은 사용자 인자를 우리가 검열하는
                        //   것이고, 우리 등록을 접으면 세션 id 회수가 통째로 죽는다. 둘 다 안 하고 이긴
                        //   사실만 남긴다.
                        if passthrough_overrides_the_hook_key(extra_args) {
                            tracing::warn!(
                                "codex 패스스루가 `{SESSION_START_HOOK_KEY}` 을 직접 세웠다 — 세션 id 회수를 위해 우리 등록을 뒤에 실어 그 값을 덮는다(마지막 `-c` 가 이긴다)"
                            );
                        }
                        args.push(CONFIG_OVERRIDE_FLAG.to_string());
                        args.push(value);
                    }
                }

                // ADR-0086 스텝 2(CLI 입구) — ★모드를 가르지 않는다★: 심는 것은 env 세 값뿐이고 그
                //   값을 읽는 것은 codex 가 아니라 **codex 가 띄우는 자식들**(셸 도구·훅 프로세스)이다.
                //   두 모드 다 자식을 띄우므로 갈릴 축이 없다.
                // ★`--mcp-config` 짝은 여기 없다 — 그것은 claude 의 플래그다★: 이 백엔드의
                //   [`AgentBackend::accepts_mcp_config`] 가 false 라 `endpoint.config_path` 는 애초에
                //   `None` 으로 온다(그 Option 의 뜻 = MCP 입구의 유무. CLI 배선의 유무가 아니다 —
                //   [`ControlEndpoint::config_path`] doc).
                // ★`priming_file`·`settings_file`·`grants` 도 쓰지 않는다★ — 전부 claude 의 플래그로만
                //   번역되는 칸이고, codex 에 그 짝이 있는지는 재 본 적이 없다. 없는 문법을 지어내
                //   붙이면 기동이 실패하므로 재기 전에는 싣지 않는다.
                // ADR-0086 / ADR-0133
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
    /// (app-server 는 `thread/start` 응답으로, 터미널 모드는 훅으로 — ADR-0208), 그 값으로 두 모드 다
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
    /// ★`sid_sink` 를 app-server 갈래에만 넘긴다★ — 이 **통로**로 받아 올 식별자가 터미널 갈래엔 없다.
    ///   그 포트로 나가는 값은 codex 가 `thread/start` 응답으로 발급한 thread id 이고, 통로가 그 세션으로
    ///   무엇을 보내기 전에 나간다([`AgentBackend::open_spawn`] 의 순서 계약).
    ///   ★「터미널 모드는 id 를 못 받는다」로 읽지 말 것 — 그쪽은 **다른 입구**로 받는다★: codex 가 띄우는
    ///   `SessionStart` 훅이 제어 평면으로 되돌려 보내고(ADR-0208), 그 값은 이 통로를 거치지 않는다.
    /// ★`thread/start` 냐 `thread/resume` 이냐를 고르는 자리도 여기다★ — 조립점이 넘긴
    ///   `resume_session_id` 하나로 갈린다([`thread_open`]). ★통로에게 다시 묻지 않는다★: 통로가 자기
    ///   상태를 보고 판정하면 가르는 자리가 둘이 된다.
    /// ★`resume_session_id` 를 터미널 갈래에서는 쓰지 않는다 — 그런데 사유는 「손잡이가 없어서」가
    ///   **아니다**★: 그 모드의 이어받기는 이미 argv 에 실려 나갔다([`AgentBackend::build_spec`] 의
    ///   터미널 갈래). 같은 값을 여기서 또 쓰면 한 spawn 이 두 수단으로 이어받으려 든다.
    // ADR-0185
    // ADR-0191
    fn open_spawn(
        &self,
        command: &AgentCommand,
        spec: &CommandSpec,
        cols: u16,
        rows: u16,
        sid_sink: Option<SessionIdSink>,
        resume_session_id: Option<Uuid>,
        link_sink: Option<LinkSink>,
    ) -> Result<SpawnParts, PtyError> {
        let (transport, child_pid): (Box<dyn AgentTransport>, Option<u32>) =
            if is_app_server(command) {
                let (t, pid) = CodexAppServerTransport::open(
                    spec,
                    true,
                    self.output_decoder(command),
                    thread_open(spec, resume_session_id),
                    sid_sink,
                    link_sink,
                )?;
                (Box::new(t), pid)
            } else {
                // ★터미널 모드에는 세울 연결이 없다★ — `declares_link()` 가 false 라 포트도 `None` 이다.
                let (t, pid) = PtyTransport::open(spec, cols, rows)?;
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
        TurnOutcome, CLI_EXE_ENV, MAIL_MARKER_ENV, MAIL_MARKER_OFF, MAIL_MARKER_ON,
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
                "on-request"
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

    #[test]
    fn extra_args_come_last() {
        let s = spec(&codex(vec!["-m", "gpt-5"]), "C:/workspace");
        let argv = codex_argv(&s);
        assert_eq!(
            &argv[argv.len() - 2..],
            &["-m".to_string(), "gpt-5".to_string()]
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
    /// 갈라놓는다. (훅 등록이 서는 스폰에서는 그것이 패스스루보다 더 뒤다 — 아래 훅 구획이 잰다.)
    #[test]
    fn resume_argv_still_ends_with_the_passthrough() {
        let s = spec_resuming(
            &codex(vec!["-m", "gpt-5"]),
            SpawnMode::Resume,
            Some(Uuid::new_v4()),
            "C:/workspace",
        );
        let argv = codex_argv(&s);
        assert_eq!(&argv[..1], &["resume".to_string()]);
        assert_eq!(
            &argv[argv.len() - 2..],
            &["-m".to_string(), "gpt-5".to_string()]
        );
    }

    // ── CLI 입구 주입(ADR-0086 스텝 2 · ADR-0133) ────────────────────────────────

    fn endpoint() -> ControlEndpoint {
        ControlEndpoint {
            url: "http://127.0.0.1:7777/mcp".to_string(),
            token: "deadbeef".to_string(),
            // ★None 이 이 백엔드의 정상값이다★ — `accepts_mcp_config()` 가 false 라 데몬이 mcp-config 를
            //   아예 쓰지 않는다. 그래도 아래 세 env 는 실려야 한다는 것이 이 구획이 재는 것이다.
            config_path: None,
            send_exe: Some(std::path::PathBuf::from("C:/engram/bin/engram.exe")),
            priming_file: Some(std::path::PathBuf::from("C:/engram/priming.md")),
            grants: vec![],
            settings_file: Some(std::path::PathBuf::from("C:/engram/session.json")),
            // ★운영이 이 백엔드에 싣는 값이 `false` 다★ — 이 폴더가 그렇게 선언하고(`uses_mail`) 데몬이
            //   그 선언에서 파생한다. 아래 형제 시험이 반대 값도 따라간다는 것을 따로 잰다.
            mail_allowed: false,
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
    /// 떠도 아무도 모른다. codex 가 띄우는 훅 프로세스는 이 env 를 상속으로만 받는다.
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

    // ── ADR-0208/ADR-0210: `SessionStart` 훅 등록(`-c` 오버라이드 단독) ─────────

    /// 실 codex 0.155.0 이 받아들인 값 그대로(실측 2026-09-19): `--strict-config app-server` 가 0 으로
    /// 끝났고, `hooks/list` 가 이 정의를 `untrusted` 로 되돌려 주며 `command` 칸이 바이트 단위로 같았다.
    ///
    /// ★작은따옴표(TOML 리터럴 문자열)가 load-bearing 이다★ — 큰따옴표면 Windows 역슬래시가 이스케이프로
    ///   먹히고, 그 값은 `cmd.exe /c codex …` 래핑을 지나며 한 번 더 망가진다. 리터럴 문자열은 둘 다 없다.
    #[test]
    fn the_terminal_spawn_registers_the_session_start_hook() {
        let s = spec_with_control(&codex(vec![]), Some(endpoint()));
        let argv = codex_argv(&s);
        assert_eq!(
            &argv[argv.len() - 2..],
            &[
                CONFIG_OVERRIDE_FLAG.to_string(),
                "hooks.SessionStart=[{hooks=[{type='command',command='C:/engram/bin/engram.exe hook session-start'}]}]"
                    .to_string(),
            ],
            "훅 등록이 실측된 모양 그대로 실려야 한다: {argv:?}"
        );
    }

    /// ★훅이 부르는 동사는 우리 CLI 파서와 **손으로** 맞춰져 있다★ — 정본 =
    /// `crates/engram-dashboard-daemon/src/bin/engram.rs` 의 `CLI_GROUP_HOOK` +
    /// `CLI_HOOK_VERB_SESSION_START`. 그쪽 `run_hook` 은 계열 뒤 argv 가 정확히 한 낱말일 것을 요구하고,
    /// 어긋나면 exit 0 · stdout 봉인으로 조용히 끝나 **어느 게이트도 못 잡는다**(ADR-0208 결정 3).
    /// 이 crate 는 데몬을 의존하지 않으므로(의존 방향) 여기서 잴 수 있는 것은 문자열 자체뿐이다.
    #[test]
    fn the_hook_command_carries_the_cli_verb_the_daemon_parses() {
        assert_eq!(HOOK_REPORT_ARGV, "hook session-start");
        let s = spec_with_control(&codex(vec![]), Some(endpoint()));
        assert!(
            codex_argv(&s)
                .iter()
                .any(|a| a.ends_with(" hook session-start'}]}]")),
            "훅 명령이 `<exe> hook session-start` 로 끝나야 한다: {:?}",
            s.args
        );
    }

    /// ★app-server 모드에는 걸지 않는다★ — 그 모드의 세션 id 는 `thread/start` 응답으로 통로가 직접
    /// 받아 오므로 훅이 **군더더기**다. ★거절 로그를 피하려는 것이 아니다★ — 두 경로가 나르는 값은
    /// 같아서 훅 보고는 충돌이 아니라 「이미 같은 값」으로 끝난다(위 `build_spec` 의 같은 자리 주석).
    #[test]
    fn the_app_server_spawn_registers_no_hook() {
        let s = spec_with_control(&codex_app_server(vec![]), Some(endpoint()));
        assert_eq!(codex_argv(&s), vec!["app-server", "--stdio"]);
    }

    /// ★훅 등록이 패스스루 **뒤**다★ — 이 순서가 뒤집히면 `-c hooks.SessionStart=…` 한 줄로 우리 등록이
    /// 조용히 진다(같은 키를 두 번 넘기면 마지막이 이긴다 — 실측 0.155.0 · 2026-09-19).
    #[test]
    fn the_hook_override_follows_the_passthrough() {
        let s = CodexBackend.build_spec(
            &codex(vec!["-m", "gpt-5"]),
            SpawnMode::Fresh,
            None,
            None,
            PathBuf::from("C:/workspace"),
            vec![],
            Some(endpoint()),
        );
        let argv = codex_argv(&s);
        let flag = argv
            .iter()
            .position(|a| a == CONFIG_OVERRIDE_FLAG)
            .expect("훅 등록이 실려야 한다");
        assert_eq!(
            flag,
            argv.len() - 2,
            "훅 등록이 맨 뒤가 아니다 — 뒤에 오는 쪽이 이긴다: {argv:?}"
        );
        assert_eq!(
            &argv[argv.len() - 4..argv.len() - 2],
            &["-m".to_string(), "gpt-5".to_string()],
            "패스스루가 훅 등록 앞에 와야 한다: {argv:?}"
        );
    }

    /// ★사용자가 같은 키를 직접 세워도 우리 것이 이긴다 — 그리고 그 승리를 **말없이** 하지 않는다★.
    /// 지면 `hooks/list` 가 빈 배열이 되고 신뢰 심사 프롬프트조차 안 떠서, 「세션 id 가 영영 안 온다」
    /// 말고는 아무 신호가 없다(실측 0.155.0). 거르지 않는 사유는 emission 자리 주석이 정본이다.
    #[test]
    fn a_conflicting_passthrough_loses_to_ours_and_trips_the_warning() {
        assert!(
            passthrough_overrides_the_hook_key(&[
                CONFIG_OVERRIDE_FLAG.to_string(),
                format!("{SESSION_START_HOOK_KEY}=[]"),
            ]),
            "`-c {SESSION_START_HOOK_KEY}=…` 를 못 알아봤다 — 경고 없이 사용자 설정을 덮는다"
        );
        let s = CodexBackend.build_spec(
            &codex(vec![CONFIG_OVERRIDE_FLAG, "hooks.SessionStart=[]"]),
            SpawnMode::Fresh,
            None,
            None,
            PathBuf::from("C:/workspace"),
            vec![],
            Some(endpoint()),
        );
        let argv = codex_argv(&s);
        assert_eq!(
            argv.iter().filter(|a| *a == CONFIG_OVERRIDE_FLAG).count(),
            2,
            "두 `-c` 가 다 실려야 한다 — 거르는 순간 사용자 인자를 우리가 검열하는 것이다: {argv:?}"
        );
        assert!(
            argv[argv.len() - 1].starts_with(SESSION_START_HOOK_KEY)
                && argv[argv.len() - 1].contains(HOOK_REPORT_ARGV),
            "마지막 `-c` 값이 우리 것이 아니다 — 마지막이 이기므로 이 자리를 뺏기면 훅이 죽는다: {argv:?}"
        );
    }

    /// 다른 키를 `-c` 로 넘기는 것은 충돌이 아니다 — 경고를 남발하면 아무도 안 읽는다.
    #[test]
    fn an_unrelated_config_passthrough_is_not_a_conflict() {
        assert!(!passthrough_overrides_the_hook_key(&[
            CONFIG_OVERRIDE_FLAG.to_string(),
            "model=gpt-5".to_string(),
        ]));
        assert!(!passthrough_overrides_the_hook_key(&[
            "--profile".to_string(),
            format!("{SESSION_START_HOOK_KEY}=[]"),
        ]));
    }

    /// 이어받기 갈래에서도 하위 명령은 맨 앞, 훅 등록은 맨 뒤 — 둘이 서로를 밀어내지 않는다.
    #[test]
    fn a_resuming_spawn_still_registers_the_hook() {
        let s = CodexBackend.build_spec(
            &codex(vec![]),
            SpawnMode::Resume,
            None,
            Some(Uuid::new_v4()),
            PathBuf::from("C:/workspace"),
            vec![],
            Some(endpoint()),
        );
        let argv = codex_argv(&s);
        assert_eq!(&argv[..1], &[RESUME_SUBCOMMAND.to_string()]);
        assert_eq!(argv[argv.len() - 2], CONFIG_OVERRIDE_FLAG);
    }

    /// ★CLI 실행파일을 모르면 걸지 않는다 — 없는 프로그램을 가리키는 훅은 매 세션 `Hook failed` 다★.
    #[test]
    fn without_a_cli_executable_no_hook_is_registered() {
        let mut ep = endpoint();
        ep.send_exe = None;
        let s = spec_with_control(&codex(vec![]), Some(ep));
        assert!(
            !codex_argv(&s).iter().any(|a| a == CONFIG_OVERRIDE_FLAG),
            "send_exe 부재인데 훅이 실렸다: {:?}",
            s.args
        );
    }

    /// endpoint 자체가 없는 스폰도 마찬가지다 — 이쪽은 `send_exe` 이전에 끊긴다.
    #[test]
    fn without_an_endpoint_no_hook_is_registered() {
        let s = spec_with_control(&codex(vec![]), None);
        assert!(
            !codex_argv(&s).iter().any(|a| a == CONFIG_OVERRIDE_FLAG),
            "endpoint 부재인데 훅이 실렸다: {:?}",
            s.args
        );
    }

    /// ★공백이 든 경로는 **실패할 것을 아는 명령을 거느니 건너뛴다**★ — codex 는 `command` 의 첫 공백
    /// 까지를 프로그램으로 잘라 셸 없이 띄우므로(실측), 그 경로는 따옴표로 감싸도 뜨지 않는다.
    ///
    /// ★이 경로가 **실재하지 않는 것**이 이 항목을 두 플랫폼에서 결정적으로 만든다★: Windows 에서
    ///   `GetShortPathNameW` 는 실물이 없으면 0 을 돌려주고, 그 밖의 플랫폼에는 8.3 이름이 아예 없다.
    ///   실재하는 공백 경로를 쓰면 이 항목은 볼륨의 8.3 설정에 따라 갈린다.
    #[test]
    fn a_spaced_executable_path_without_a_short_form_is_skipped() {
        let mut ep = endpoint();
        ep.send_exe = Some(std::path::PathBuf::from(
            "C:/Program Files/engram no such dir/engram.exe",
        ));
        let s = spec_with_control(&codex(vec![]), Some(ep));
        let argv = codex_argv(&s);
        assert!(
            !argv.iter().any(|a| a == CONFIG_OVERRIDE_FLAG),
            "단축 경로를 못 얻은 공백 경로가 그대로 실렸다: {argv:?}"
        );
        assert_eq!(
            argv,
            codex_argv(&spec(&codex(vec![]), "C:/workspace")),
            "훅을 건너뛴 argv 는 훅 없는 argv 와 바이트 단위로 같아야 한다"
        );
    }

    /// ★작은따옴표는 TOML 리터럴 문자열에 담을 수 없다 — 지어낸 이스케이프로 밀어 넣지 않는다★.
    #[test]
    fn an_executable_path_with_a_single_quote_is_skipped() {
        let mut ep = endpoint();
        ep.send_exe = Some(std::path::PathBuf::from("C:/o'brien/engram.exe"));
        let s = spec_with_control(&codex(vec![]), Some(ep));
        assert!(
            !codex_argv(&s).iter().any(|a| a == CONFIG_OVERRIDE_FLAG),
            "작은따옴표가 든 경로가 실렸다: {:?}",
            s.args
        );
    }

    /// ★탭·줄바꿈·제어문자는 8.3 으로도 못 구한다★ — 탭은 codex 가 프로그램 토큰을 자르는 자리를 옮기고,
    /// 줄바꿈은 한 줄짜리 TOML 리터럴 문자열 자체를 깨뜨린다. 그래서 보통 공백과 달리 단축 경로를
    /// 시도하지도 않고 끊는다.
    #[test]
    fn an_executable_path_with_a_tab_or_a_newline_is_skipped() {
        for raw in ["C:/engram\tbin/engram.exe", "C:/engram\nbin/engram.exe"] {
            let mut ep = endpoint();
            ep.send_exe = Some(std::path::PathBuf::from(raw));
            let s = spec_with_control(&codex(vec![]), Some(ep));
            assert!(
                !codex_argv(&s).iter().any(|a| a == CONFIG_OVERRIDE_FLAG),
                "공백 아닌 공백문자가 든 경로가 실렸다({raw:?}): {:?}",
                s.args
            );
        }
    }

    /// ★8.3 변환의 **성공** 갈래를 도는 유일한 항목이다★ — 형제 둘은 전부 건너뛰기 갈래라, 두 번 부르는
    /// 길이 규약(NUL 포함 ↔ 제외)이 한 번도 실행되지 않는다. 운영 경로로도 안 돈다 — 우리 실 exe 경로에
    /// 공백이 없기 때문이다.
    /// ★「이 볼륨엔 8.3 이름이 없다」는 실패가 아니라 정당한 다른 결말이다★ — 그 설정은 볼륨마다 다르고
    ///   테스트가 도는 볼륨을 우리가 고르지 않는다. 그래서 두 결말을 **둘 다** 받고, 대신 각 결말이
    ///   자기 짝(훅 값의 유무)과 어긋나지 않는 것을 잰다.
    #[cfg(windows)]
    #[test]
    fn a_real_spaced_path_exercises_the_short_name_success_branch() {
        let dir = std::env::temp_dir().join(format!("engram hook path {}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("임시 폴더 생성");
        let exe = dir.join("engram.exe");
        std::fs::write(&exe, b"").expect("임시 파일 생성");

        let short = short_program_path(&exe);
        let hook = session_start_hook_override(Some(exe.as_path()));
        let short_exists = short.as_deref().map(|s| std::path::Path::new(s).exists());
        std::fs::remove_dir_all(&dir).ok();

        match short {
            Some(short) => {
                assert!(
                    !short
                        .chars()
                        .any(|c| c.is_whitespace() || c.is_control() || c == '\''),
                    "단축 경로에 아직 실을 수 없는 문자가 남았다: {short:?}"
                );
                assert_eq!(
                    short_exists,
                    Some(true),
                    "단축 경로가 같은 실물을 안 가리킨다: {short:?}"
                );
                let hook = hook.expect("단축 경로를 얻었으면 훅 값도 서야 한다");
                assert!(
                    hook.contains(&short),
                    "훅 값이 단축 경로를 안 실었다: {hook}"
                );
            }
            None => assert!(
                hook.is_none(),
                "단축 경로가 없는데 훅이 섰다 — 공백이 든 경로가 그대로 실린다: {hook:?}"
            ),
        }
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

    #[test]
    fn reads_messages_is_false() {
        assert!(!CodexBackend.reads_messages());
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
