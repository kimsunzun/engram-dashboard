//! `engram` — 제어 평면 CLI(ADR-0086 스텝 2 · S18 D · ADR-0132). 스폰된 에이전트가 셸로 팀에 말을 걸고,
//! 자기 미결을 확인하고, 팀의 구성을 바꾸는 최소 클라이언트다. 계열은 둘이다: **`mail`**(우편 — MCP 툴
//! 2종의 미러. `group` 툴/서브커맨드는 ADR-0111 결정 4 로 제거됐다)과 **`agent`**(에이전트 제어 —
//! 짝이 되는 MCP 툴이 **없다**. 제어를 CLI 로만 내는 것이 ADR-0132 의 결정이다).
//!
//! ★인자 표면의 정본은 이 파일이다★ — S18 spec §6 의 CLI 블록은 그룹 구조 이전의 표기라 인자 계약으로
//!   읽으면 안 된다(그 절이 자기 supersede 를 적고 있다). 그 spec 이 계속 정본인 것은 **응답 shape·상태
//!   어휘** 쪽이고, 아래 exit code 판정자들이 §6 을 인용하는 것도 그 축이다. 표면 결정 = ADR-0132.
//!
//! ★표면 = `engram <계열> <동사>`★: 계열·동사 없이 플래그만 친 호출은 인자 오류다(기본 동사 없음).
//!   실행파일 이름은 core 의 `CLI_EXE_NAME` 이 정본이고 help 문구도 거기서 만들어진다 — grant·프라이밍·
//!   PATH 해석이 그 한 이름에 정렬돼야 하기 때문이다(ADR-0094).
//!
//! ★발견은 help 로만★(ADR-0132 결정 4 — MCP 가 툴 스키마를 스스로 드러내는 것의 CLI 대응): 인자 없는
//!   호출과 `engram help`(`--help`·`-h` 동일)가 계열 목록을, `engram help <계열>`(= `engram <계열> --help`)
//!   이 그 계열 사용법을 낸다. help 는 stdout **평문**이고 exit 0 이다 — 읽는 쪽이 LLM 이라 파싱이 아니라
//!   독해 대상이다. 모르는 계열·동사는 다른 인자 오류와 같은 반려 JSON(exit 1)으로 끝난다.
//! ★help 는 크레덴셜·데몬 없이 답한다★: env 검사보다 **먼저** 처리한다 — 표면을 배우는 자리가 "이미
//!   스폰돼 있어야" 하면 발견이 아니다.
//!
//! ★우편 계열은 보이기도 하고 안 보이기도 한다 — 그러나 막지는 않는다(ADR-0133)★: 스폰이 심은 표식
//!   (`MAIL_MARKER_ENV`)이 off 면 우편 계열을 **사용법에서만** 감춘다(감춰진 계열의 help 요청은 오타와
//!   같은 반려를 받는다). 우편 **동사 자체는 그대로 데몬으로 나간다** — 거절은 데몬이 자격증명으로 하는
//!   일이고, 여기서 막으면 그 거절이 관측되지 않는다. 표식은 조작 가능하므로 여기에 강제를 기대면 안 된다.
//!
//! ```text
//! engram mail send --to <수신자[,수신자…]> --body "…" [--request [--reply-by 10m]] [--reply-to m-xxxx]
//!     # 수신자 = 이름 | agent id | @here(나 빼고 지금 산 전원) | @all(나 빼고 명부 전원 — 잠든 것 포함,
//!     #          ADR-0121). **콤마로 여러 명**(ADR-0111 다중 수신자).
//! engram mail send --to <수신자> --body-stdin <<'EOF' … EOF   # 인용 지옥 회피(D)
//! engram mail status <m-id>                                   # 그 메시지의 배달 장부
//! engram mail pending                                         # 내 미결(보낸 것·기다리는 것·내가 답할 것)
//!
//! engram agent list                                           # 명부(산 것·잠든 것)
//! engram agent spawn <이름>                                    # 있는 에이전트 깨우기
//! engram agent spawn --cwd <경로> [--name <이름>]               # 새로 만들어 바로 띄우기
//! engram agent new --cwd <경로> [--name <이름>]                 # 만들기만(잠든 채)
//! engram agent rename <이름> <새-이름>
//! engram agent move <이름> --parent <이름|none>                 # none = 루트로 떼기
//! ```
//!
//! ★제어 동사의 **의미 검증도 데몬 단독**이다★(우편과 같은 규율): 대상 실재·이름 유일성·계층 규칙은 전부
//!   데몬이 판정한다. CLI 는 형태(값 누락·모르는 플래그·`spawn` 두 형태의 양립 불가)만 본다.
//!
//! ★동작★: 환경변수 `ENGRAM_TOKEN`(Bearer 토큰) + `ENGRAM_CONTROL_URL`(데몬 제어 base URL)을 읽어
//!   `<base>/control/<route>` 로 JSON 을 POST 한다(Authorization: Bearer <token>). 응답 body 를 stdout 에
//!   **그대로** 찍는다.
//! ★stdout 이 JSON 이라는 보장은 없다(파서를 이 가정 위에 쓰지 말 것)★: 비-2xx 응답도 body 를 그대로
//!   흘린다 — 401 은 빈 줄, 프록시가 끼면 HTML 이 나올 수 있다. **일부러** 그렇게 둔다: 반려 body 엔
//!   발신 에이전트가 파싱해 자기교정할 교정 JSON 이 실려 있을 수 있어, 우리가 형태로 걸러 버리면 그
//!   정보가 사라진다. 이 CLI 가 **스스로** 내는 반려(BAD_ARGS·NO_TOKEN·전송 실패)만 항상 봉투 JSON 이고,
//!   help 는 평문이다. 기계 판정은 stdout 형태가 아니라 **exit code** 로 한다.
//!
//! ★exit code 3분법★: **0** = 접수/조회 성공 · **1** = 실패(반려 `{status:"error",code,hint}`·연결/env
//!   오류·비-2xx·비-JSON) · **2** = 2xx 인데 응답 shape 이 깨짐(데몬/프록시 결함 — 재시도 대상이 아니라
//!   보고 대상, stderr 에 사유 한 줄). 발송 판정 정본 = `exit_code_for_response`, 조회(`status`·`pending`) 판정
//!   정본 = `exit_code_for_query_response`(성공 shape 이 동사마다 달라 "에러가 아니면 성공" 규칙을 쓴다).
//!
//! ★신원(from·"나")은 payload 아님 — `--as` 같은 플래그를 두지 않는다(D 결정)★: 발신자도, `pending` 의
//!   "나" 도 **토큰에서만** 파생된다(데몬이 토큰→신원 조회). 이 프로세스는 자기 신원을 주장하지 않는다
//!   (사칭 차단, ADR-0086 불변식). 그래서 조회 명령에도 신원 인자가 없고, ENGRAM_TOKEN 이 없으면 애초에
//!   `NO_TOKEN` 으로 끝난다 — "누구로 조회할지" 를 CLI 가 고를 여지 자체를 만들지 않는다.
//!
//! ★의미 검증은 데몬(ingress) 단독★: 상호배타(`--request` + `--reply-to`)·기간 표기·수신자 해석은 전부
//!   데몬이 판정한다 — MCP 입구와 반려 코드/문구가 같아야 하기 때문이다(entrance-agnostic). CLI 는 **형태**
//!   (값 누락·모르는 플래그)와 **CLI 고유 배관**(`--body` ↔ `--body-stdin` 상호배타)만 본다.
//!
//! ★의존성 최소화★: 블로킹 HTTP 클라이언트로 std `TcpStream` 위에 최소 HTTP/1.1 POST 를 손조립한다.
//!   reqwest(blocking) 를 정식 의존으로 넣으면 tokio 런타임·TLS 스택까지 딸려 오는데, 이 CLI 는 로컬
//!   평문 HTTP 로 작은 JSON 하나만 보내므로 과하다. wire 조립·매핑은 순수 함수로 분리해 단위 테스트한다.
//!
//! tauri import 0.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use engram_dashboard_core::agent::types::{
    AGENT_STATE_LIVE, AGENT_STATE_SLEEPING, CLI_AGENT_FLAGS, CLI_AGENT_VERBS, CLI_EXE_NAME,
    CLI_GROUP_AGENT, CLI_GROUP_MAIL, CLI_MAIL_FLAGS, CLI_MAIL_VERBS, MAIL_MARKER_ENV,
    MAIL_MARKER_OFF, RENAME_OUTCOME_RENAMED, RENAME_OUTCOME_UNCHANGED,
};

/// 연결/응답 타임아웃(로컬 데몬이라 짧게). 데몬이 죽었으면 빨리 실패해 에이전트가 재시도/보고하게 한다.
const TIMEOUT: Duration = Duration::from_secs(10);

/// help 동사. `--help`/`-h` 도 같은 자리로 받는다 — `is_help_token`.
const HELP_VERB: &str = "help";

/// help 템플릿의 실행파일 이름 자리. 출력 직전 `CLI_EXE_NAME` 으로 치환한다 — help 는 에이전트가 표면을
/// 배우는 유일한 자리라, 여기 적힌 이름이 실제 실행파일과 갈리면 배운 대로 쳐도 명령을 못 찾는다.
const HELP_TOOL_SLOT: &str = "{tool}";

/// 이 프로세스가 **사용법에 보여 줄** 표면(ADR-0133 결정 1·2). 스폰이 심은 표식에서 읽는다.
///
/// ★강제가 아니라 교육이다★: `Hidden` 은 우편 계열을 사용법과 예시에서 빼기만 한다 — 우편 동사는 그대로
///   데몬으로 나가고 거기서 거절당한다. 여기서 실행까지 막으면 ① 강제가 두 곳(에이전트 프로세스 · 데몬)에
///   생기고 ② 표식을 뗀 프로세스는 그 두 곳 중 하나를 잃어 **우편이 열린 것처럼 보인다**.
/// ★기본이 `Shown` 인 것은 의도다★: 표식은 스폰이 심는 것이라 사람이 셸에서 직접 부르면 없다 — 그때
///   사용법이 반쪽으로 나오면 안 된다. 모르는 값도 같은 이유로 `Shown` 이다(모르는 값에 fail-closed 해도
///   막히는 건 사용법뿐이라 얻는 게 없다).
// ADR-0133
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum MailSurface {
    #[default]
    Shown,
    Hidden,
}

impl MailSurface {
    fn from_env() -> Self {
        match std::env::var(MAIL_MARKER_ENV) {
            Ok(v) if v == MAIL_MARKER_OFF => Self::Hidden,
            _ => Self::Shown,
        }
    }

    fn shows_mail(self) -> bool {
        self == Self::Shown
    }
}

/// 계열 목록의 머리·꼬리와 **계열당 한 줄**. 줄 단위로 쪼갠 이유는 필터가 필터로 남게 하기 위해서다 —
/// 완성된 두 번째 목록 문자열을 따로 두면 한쪽만 고쳐진 채 갈린다(ADR-0133 결정 1).
const HELP_ROOT_HEAD: &str = "\
{tool} — CLI for the Engram broker daemon. Usage: {tool} <group> <verb> [flags]

Groups:
";
const HELP_ROOT_GROUP_MAIL: &str =
    "  mail     message your teammates and check your own outstanding items\n";
const HELP_ROOT_GROUP_AGENT: &str =
    "  agent    list, create, start, rename and re-parent the agents on this team\n";
const HELP_ROOT_TAIL: &str = "
Run `{tool} help <group>` for that group's verbs (`{tool} <group> --help` works too).
";

/// `help mail` — 우편 동사 전량과 그 플래그. 읽는 쪽이 LLM 이라 한 동사당 한 줄 + 플래그 목록으로 짧게
/// 유지한다(산문 금지).
const HELP_MAIL: &str = "\
{tool} mail — messages between the agents on this team.

  {tool} mail send --to <name[,name...]> (--body <text> | --body-stdin) [--request] [--reply-by <dur>] [--reply-to <m-id>]
      Send a message. Prints one result row per recipient.
      --to <name[,name...]>  teammate name or agent id; comma-separated for several;
                             @here = everyone live except you, @all = every agent in the tree except you
      --body <text>          the body, on the command line
      --body-stdin           read the body from stdin instead (heredoc-friendly); exactly one of --body / --body-stdin
      --request              an answer is owed; you get notified if none arrives
      --reply-by <dur>       deadline for that answer, e.g. 5m / 10m / 1h (1 minute minimum)
      --reply-to <m-id>      this message answers that request; mutually exclusive with --request
  {tool} mail status <m-id>
      Delivery state of one message you sent, one row per recipient.
  {tool} mail pending
      Your open items: answers you owe, answers you are waiting for, sends not confirmed as delivered yet.

Your identity is taken from the token the broker injected, never from an argument — there is no flag to send or query as somebody else.

Exit codes: 0 = accepted or read | 1 = rejected, or the daemon could not be reached — stdout carries the daemon's own reply when there was one, and {\"status\":\"error\",\"code\":...,\"hint\":...} when this CLI rejected the call itself | 2 = the daemon answered 2xx in a shape this CLI cannot read; report it, retrying will not help. Judge the outcome by the exit code, not by the shape of stdout.
";

/// `help agent` — 제어 동사 전량. `mail` 화면과 같은 규율(동사당 한 줄 + 플래그, 산문 금지).
///
/// ★없는 동사를 여기 적지 않는다★: 죽이기·지우기는 표면에 없다(`CLI_AGENT_VERBS` 주석이 사유의 정본).
///   "지금은 안 된다" 는 안내도 넣지 않는다 — 읽는 쪽이 LLM 이라 목록에 있는 낱말은 시도 대상이 된다.
const HELP_AGENT_HEAD: &str = "\
{tool} agent — the agents on this team: who exists, and starting or re-arranging them.

  {tool} agent list
      Every agent, running or asleep. One JSON object: agents[] with id, name, state (live|sleeping), cwd, parent.
";

/// 우편이 보이는 프로세스에서만 붙는 상호참조 한 줄(ADR-0133) — 감춘 계열을 다른 계열의 help 가 가르치면
/// 필터가 무의미해진다.
const HELP_AGENT_MAIL_XREF: &str =
    "      Names are how you address teammates in `{tool} mail send --to <name>`.\n";

const HELP_AGENT_TAIL: &str = "\
  {tool} agent spawn <name>
      Start an agent that already exists (it keeps its own past session when it has one).
  {tool} agent spawn --cwd <path> [--name <name>]
      Create a new agent in that folder and start it right away.
  {tool} agent new --cwd <path> [--name <name>]
      Create a new agent without starting it. It shows up as sleeping.
      --cwd <path>           the folder the agent works in (required)
      --name <name>          what to call it; without this the folder name is used
  {tool} agent rename <name> <new-name>
      Rename an agent. `outcome` says what happened: renamed (with the name it actually got — a
      number is appended when that name is taken) or unchanged (it already held that name).
  {tool} agent move <name> --parent <name|none>
      Put an agent under another one, or `--parent none` to move it back to the top level.
      --parent <name|none>   the new parent, or the word none to detach (required)

Agents are named exactly: no case-folding, no prefixes. If two agents share a name the command is
refused rather than guessing — pass the id from `{tool} agent list` instead. An agent literally
called `none` can only be used as a parent by its id.

Exit codes: 0 = done | 1 = refused, or the daemon could not be reached — stdout carries the daemon's own reply when there was one, and {\"status\":\"error\",\"code\":...,\"hint\":...} when this CLI refused the call itself | 2 = the daemon answered 2xx in a shape this CLI cannot read; report it, retrying will not help. Judge the outcome by the exit code, not by the shape of stdout.
";

/// 화면 하나 = 조각들의 이어붙임 + 실행파일 이름 치환. 조각 선택이 곧 표면 필터다(ADR-0133).
fn render_help(topic: HelpTopic, mail: MailSurface) -> String {
    let mut out = String::new();
    match topic {
        HelpTopic::Root => {
            out.push_str(HELP_ROOT_HEAD);
            if mail.shows_mail() {
                out.push_str(HELP_ROOT_GROUP_MAIL);
            }
            out.push_str(HELP_ROOT_GROUP_AGENT);
            out.push_str(HELP_ROOT_TAIL);
        }
        HelpTopic::Mail => out.push_str(HELP_MAIL),
        HelpTopic::Agent => {
            out.push_str(HELP_AGENT_HEAD);
            if mail.shows_mail() {
                out.push_str(HELP_AGENT_MAIL_XREF);
            }
            out.push_str(HELP_AGENT_TAIL);
        }
    }
    out.replace(HELP_TOOL_SLOT, CLI_EXE_NAME)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = run(&args);
    std::process::exit(code);
}

fn run(args: &[String]) -> i32 {
    // ★표식은 파싱보다 먼저, 크레덴셜보다 먼저 읽는다★: help 가 크레덴셜 검사 앞에서 답한다는 성질을
    //   유지해야 하는데(ADR-0132 조각 ①) 그 화면이 표식에 따라 갈리기 때문이다.
    let mail = MailSurface::from_env();
    let parsed = match parse_command(args, mail) {
        Ok(p) => p,
        Err(msg) => {
            print_error("BAD_ARGS", &msg);
            return 1;
        }
    };
    // help 는 크레덴셜도 데몬도 없이 답한다 — 표면을 배우는 자리가 "먼저 스폰돼 있어야" 하면 발견이 아니다.
    let command = match parsed {
        ParsedCommand::Help(topic) => {
            println!("{}", render_help(topic, mail));
            return 0;
        }
        // 제어 동사는 stdin 을 읽지 않으므로 materialize 단계가 없다(본문이라는 개념이 없다).
        ParsedCommand::Agent(a) => Command::Agent(a),
        // ★이 단계의 반려도 같은 접기를 탄다★: 파싱이 성공한 **뒤**에 나는 인자 오류(빈 stdin 등)라
        //   `parse_command` 의 접기를 지나쳐 온다 — 그 문구가 계열의 다른 플래그를 되돌려 주면 감춘 계열이
        //   여기로 새어 나간다(실제로 `--body-stdin` 빈 입력 반려가 `--body` 를 안내했다).
        // ★진짜 stdin I/O 실패(EIO·EISDIR 등)까지 함께 접히는 것은 **감수한 대가**다★: 접기가 발동하는
        //   프로세스는 우편이 감춰진 에이전트뿐이고, 그 요청은 어차피 데몬이 거절하므로 구체적 진단이
        //   그에게 줄 값이 거의 없다. 반대로 사유를 그대로 흘리면 그 문구가 **계열의 존재와 인자 표기를
        //   드러내** 감춘 것이 무의미해진다 — 진단 가능성보다 은닉을 택한 자리다. 이걸 "실패 원인이 안
        //   보인다" 로 읽고 원문을 되살리지 말 것.
        // ADR-0133
        ParsedCommand::Mail(m) => match materialize_body(m, read_stdin_to_string) {
            Ok(c) => c,
            Err(msg) => {
                print_error("BAD_ARGS", &hide_mail_reason(mail, msg));
                return 1;
            }
        },
    };

    let token = match std::env::var("ENGRAM_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => {
            print_error(
                "NO_TOKEN",
                "ENGRAM_TOKEN is not set; this command must run inside an engram-spawned agent.",
            );
            return 1;
        }
    };
    let base = match std::env::var("ENGRAM_CONTROL_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => {
            print_error(
                "NO_CONTROL_URL",
                "ENGRAM_CONTROL_URL is not set; this command must run inside an engram-spawned agent.",
            );
            return 1;
        }
    };

    let route = command.route();
    let request_body = command.request_body();
    match post_json(&base, route, &token, &request_body) {
        Ok(resp) => {
            // 비-2xx 라도 body 를 찍는다 — 교정 JSON 이 실려 있어 발신 에이전트가 파싱한다.
            println!("{}", resp.body);
            // ★판정기는 계열·동사가 고른다★: 발송은 고정 shape, 우편 조회는 "에러가 아니면 성공", 제어는
            //   동사별 성공 shape 를 직접 본다(각 함수 doc 이 그 차이의 근거).
            match &command {
                Command::Send(_) => exit_code_for_response(resp.status, &resp.body),
                Command::Status { .. } | Command::Pending => {
                    exit_code_for_query_response(resp.status, &resp.body)
                }
                Command::Agent(a) => exit_code_for_agent_response(a, resp.status, &resp.body),
            }
        }
        Err(e) => {
            print_error(e.code(), &e.to_string());
            1
        }
    }
}

/// 전송 계층 실패 분류(M1) — exit code 는 항상 1 이지만 **에러 코드**는 원인별로 갈라 stdout JSON 에 싣는다.
#[derive(Debug)]
enum SendError {
    /// 연결·쓰기·읽기 IO 실패 또는 응답 프레이밍 파싱 실패(base/URL 문제 포함).
    Connect(String),
    /// Content-Length 가 있는데 수신 body 가 그보다 짧음(절단). received/expected 바이트 수 동봉.
    Incomplete { received: usize, expected: usize },
}

impl SendError {
    fn code(&self) -> &'static str {
        match self {
            SendError::Connect(_) => "CONNECT_FAILED",
            SendError::Incomplete { .. } => "INCOMPLETE_RESPONSE",
        }
    }
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::Connect(msg) => write!(f, "{msg}"),
            SendError::Incomplete { received, expected } => write!(
                f,
                "response body truncated: received {received} bytes but Content-Length declared {expected}"
            ),
        }
    }
}

#[derive(Debug)]
struct CliArgs {
    to: String,
    body: String,
    request: bool,
    reply_by: Option<String>,
    reply_to: Option<String>,
}

/// ★왜 파싱 단계에서 stdin 을 읽지 않나(load-bearing — 테스트 가능성)★: 파서를 순수하게 유지해야 인자
///   조합 전수를 단위 테스트할 수 있다(stdin 은 프로세스 전역 자원이라 테스트 병렬 실행에서 공유된다).
///   그래서 파서는 "stdin 에서 읽어라" 는 **의도만** 값으로 남기고, 실제 읽기는 `materialize_body` 가
///   주입받은 리더로 수행한다(운영 = 진짜 stdin, 테스트 = 가짜 클로저).
#[derive(Debug, Clone, PartialEq, Eq)]
enum BodySource {
    Literal(String),
    Stdin,
}

/// 어느 help 화면인가. 완성된 문자열이 아니라 **주제**를 나르는 이유는 화면 조립이 표면 필터
/// (`MailSurface`)에 달려 있어서다 — 파서가 문자열을 골라 버리면 필터가 파서로 새어 든다.
// ADR-0133
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelpTopic {
    Root,
    Mail,
    Agent,
}

/// 파싱 결과의 최상위 갈래. help 를 우편 동사와 **타입으로** 갈라 두면 네트워크·크레덴셜 경로가 help 를
/// 영영 만나지 않는다(그 경로에 "help 면 건너뛴다" 분기를 둘 필요가 없다).
#[derive(Debug)]
enum ParsedCommand {
    Help(HelpTopic),
    Mail(ParsedMail),
    Agent(ParsedAgent),
}

/// 제어 계열의 파싱 결과(ADR-0132 결정 6). 데몬 검증과 **겹치지 않는 것만** 여기서 본다 — 형태(값 누락·
/// 모르는 플래그·양립 불가 조합)뿐이고, 대상이 실재하는지·이름이 유일한지는 데몬이 판정한다.
#[derive(Debug, PartialEq, Eq)]
enum ParsedAgent {
    List,
    /// `target` 과 `cwd` 는 **정확히 하나만** 채워진다(파서가 그 전에 반려한다 — `parse_agent_spawn`).
    Spawn {
        target: Option<String>,
        cwd: Option<String>,
        name: Option<String>,
    },
    New {
        cwd: String,
        name: Option<String>,
    },
    Rename {
        target: String,
        name: String,
    },
    Move {
        target: String,
        /// `None` = 루트로 떼기(`--parent none`).
        parent: Option<String>,
    },
}

#[derive(Debug)]
enum ParsedMail {
    Send {
        to: String,
        body: BodySource,
        request: bool,
        reply_by: Option<String>,
        reply_to: Option<String>,
    },
    Status {
        id: String,
    },
    Pending,
}

#[derive(Debug)]
enum Command {
    Send(CliArgs),
    Status { id: String },
    Pending,
    Agent(ParsedAgent),
}

impl Command {
    /// ★경로 지식은 CLI 소유(ADR-0086)★: 데몬은 base URL 만 env 로 준다 — 라우트 조립은 여기가 정본이고,
    ///   데몬측 상수(`mcp_server.rs` CONTROL_*_PATH)와 **손으로 맞춰져** 있다(빌드가 강제 못 함).
    fn route(&self) -> &'static str {
        match self {
            Command::Send(_) => "/control/send",
            Command::Status { .. } | Command::Pending => "/control/messages",
            Command::Agent(_) => "/control/agent",
        }
    }

    fn request_body(&self) -> String {
        match self {
            Command::Send(a) => build_request_body(a),
            Command::Status { id } => serde_json::json!({ "id": id }).to_string(),
            Command::Pending => serde_json::json!({}).to_string(),
            Command::Agent(a) => build_agent_body(a).to_string(),
        }
    }
}

/// 제어 동사의 요청 JSON. **동사는 바디의 `verb` 로 간다**(경로가 아니라) — 라우트 상수 하나로 계열 전체를
/// 태우기 위해서다(데몬측 `CONTROL_AGENT_PATH` 주석).
///
/// ★미지정 인자는 키 자체를 안 싣는다★ — `mail` 쪽과 같은 규율. 단 `move` 의 `parent` 는 **null 을 명시**
///   한다: 그 자리에서 부재는 "안 줬다" 가 아니라 "루트로 떼라" 는 **적극적 의도**라, 키를 빼면 두 뜻이
///   구별되지 않는다.
fn build_agent_body(agent: &ParsedAgent) -> serde_json::Value {
    match agent {
        ParsedAgent::List => serde_json::json!({ "verb": "list" }),
        ParsedAgent::Spawn { target, cwd, name } => {
            let mut v = serde_json::json!({ "verb": "spawn" });
            if let Some(t) = target {
                v["target"] = serde_json::Value::String(t.clone());
            }
            if let Some(c) = cwd {
                v["cwd"] = serde_json::Value::String(c.clone());
            }
            if let Some(n) = name {
                v["name"] = serde_json::Value::String(n.clone());
            }
            v
        }
        ParsedAgent::New { cwd, name } => {
            let mut v = serde_json::json!({ "verb": "new", "cwd": cwd });
            if let Some(n) = name {
                v["name"] = serde_json::Value::String(n.clone());
            }
            v
        }
        ParsedAgent::Rename { target, name } => {
            serde_json::json!({ "verb": "rename", "target": target, "name": name })
        }
        ParsedAgent::Move { target, parent } => {
            serde_json::json!({ "verb": "move", "target": target, "parent": parent })
        }
    }
}

/// ★기본 동사 없음★: 계열·동사를 생략한 호출은 발송으로 흘리지 않는다 — 인자 0 은 help 고, 플래그로
///   시작하면 인자 오류다. 그래야 계열이 늘어도 "어느 동사인지" 를 첫 토큰만 보고 안다.
/// ★kebab-case 플래그 ↔ snake_case wire(spec §1 표기 매핑)★: 셸 관례는 `--reply-by`, JSON 필드는
///   `reply_by` 다 — 변환은 `Command::request_body` 한 곳에서만 한다.
/// ★표식이 가리는 것은 **렌더링**뿐이다(ADR-0133 결정 1)★: 계열이 감춰지면 그 계열의 사용법 요청과
///   **인자 오류**가 전부 "모르는 계열" 로 렌더된다 — 반려 문구도 목록·상호참조와 같은 교육 표면이고,
///   특히 동사 목록을 되돌려 주는 문구(`mail needs a verb (send|status|pending)`)는 감춘 화면보다 **더 많이**
///   가르친다. 반면 **말이 되는 호출은 그대로 통과**해 데몬에 닿는다: 강제는 데몬 한 곳에만 남아야 그
///   거절이 실제로 관측된다. 인자 오류는 애초에 네트워크를 타지 않으므로(이 파일의 규율) 문구를 바꾼다고
///   강제가 클라이언트로 옮겨 오지 않는다.
fn parse_command(args: &[String], mail: MailSurface) -> Result<ParsedCommand, String> {
    let Some(first) = args.first().map(|s| s.as_str()) else {
        return Ok(ParsedCommand::Help(HelpTopic::Root));
    };
    match first {
        t if is_help_token(t) => parse_help_topic(&args[1..], mail),
        CLI_GROUP_MAIL => {
            let rest = &args[1..];
            if rest.first().is_some_and(|a| is_help_token(a)) {
                if !mail.shows_mail() {
                    // 감춰진 계열의 사용법 요청 = 모르는 계열을 친 것과 같은 답(구분되면 감춘 의미가 없다).
                    return Err(unknown_group(first));
                }
                reject_help_with_extra_args(rest)?;
                return Ok(ParsedCommand::Help(HelpTopic::Mail));
            }
            // ★반려를 여기 한 곳에서 갈아끼운다★: 계열 안쪽 문구를 자리마다 고치면 새 동사·새 플래그가
            //   생길 때마다 한 자리가 빠지고, 그 한 자리가 감춘 계열을 되돌려 준다. 성공 갈래는 손대지
            //   않으므로 실행 경로는 그대로다.
            parse_mail(rest)
                .map(ParsedCommand::Mail)
                .map_err(|e| hide_mail_reason(mail, e))
        }
        CLI_GROUP_AGENT => {
            let rest = &args[1..];
            if rest.first().is_some_and(|a| is_help_token(a)) {
                reject_help_with_extra_args(rest)?;
                return Ok(ParsedCommand::Help(HelpTopic::Agent));
            }
            parse_agent(rest).map(ParsedCommand::Agent)
        }
        other if other.starts_with('-') => Err(format!(
            "the first argument must be a group, not a flag ({other}) — e.g. `{}`; run `{CLI_EXE_NAME} help` to list groups",
            example_invocation(mail)
        )),
        other => Err(unknown_group(other)),
    }
}

fn unknown_group(name: &str) -> String {
    format!("unknown group: {name} — run `{CLI_EXE_NAME} help` to list groups")
}

/// 우편 계열의 반려 사유를 **표면에 맞게 렌더한다**(ADR-0133) — 감춰져 있으면 "모르는 계열" 한 문구로 접고,
/// 보이면 원래 사유를 그대로 낸다.
///
/// ★우편 계열이 자기 사유를 내는 자리는 전부 여기를 지나야 한다★: 반려 문구는 목록·상호참조와 같은 교육
///   표면이고, 특히 계열의 다른 동사·플래그를 되돌려 주는 문구는 감춘 화면보다 더 많이 가르친다. 자리가
///   둘 이상이면 하나가 빠지고, 실제로 파싱 **이후** 단계(`materialize_body`)가 그렇게 빠져 있었다.
// ADR-0133
fn hide_mail_reason(mail: MailSurface, reason: String) -> String {
    if mail.shows_mail() {
        reason
    } else {
        unknown_group(CLI_GROUP_MAIL)
    }
}

/// 반려 문구가 드는 예시·계열 이름은 **보이는 표면에서만** 고른다 — 감춘 계열을 에러 메시지가 가르치면
/// 필터가 무의미해진다(ADR-0133).
fn example_invocation(mail: MailSurface) -> String {
    if mail.shows_mail() {
        format!("{CLI_EXE_NAME} {CLI_GROUP_MAIL} send --to <name> --body <text>")
    } else {
        format!("{CLI_EXE_NAME} {CLI_GROUP_AGENT} list")
    }
}

fn example_help_topic(mail: MailSurface) -> &'static str {
    if mail.shows_mail() {
        CLI_GROUP_MAIL
    } else {
        CLI_GROUP_AGENT
    }
}

/// ★help 토큰의 유일한 규칙 — **키워드 자리에서만** help 다★: 계열 자리(`engram --help`)와 동사 자리
///   (`engram mail --help`)에서만 발견 요청으로 읽는다. **값이 와야 하는 자리**에서는 절대 help 가 아니다:
///   명시 플래그의 값(`--body -h`)은 **그대로 값으로** 쓰이고(임의 텍스트라 가로채면 편지가 사라진다),
///   동사의 위치 인자(`status --help`)는 **인자 오류**로 끊는다(그 자리에 올 수 있는 값이 아니고, 그대로
///   보내면 무의미한 조회가 네트워크를 탄다).
fn is_help_token(arg: &str) -> bool {
    matches!(arg, HELP_VERB | "--help" | "-h")
}

/// ★help 호출에 인자가 더 붙으면 성공(0)으로 삼키지 않는다★: `engram mail --help --to bob --body hi` 를
///   help 로 처리하면 보내려던 편지가 사라지고 **exit 0** 이라 호출자는 성공으로 읽는다 — 값 자리에서 막은
///   것과 같은 조용한 유실이 계열 자리에서 나는 것뿐이다. 그래서 help 는 **단독 호출**일 때만 help 다.
fn reject_help_with_extra_args(args: &[String]) -> Result<(), String> {
    if args.len() > 1 {
        return Err(format!(
            "help takes no further arguments: {} — run `{CLI_EXE_NAME} help` on its own, or run the command you meant",
            args[1]
        ));
    }
    Ok(())
}

fn parse_help_topic(rest: &[String], mail: MailSurface) -> Result<ParsedCommand, String> {
    let Some(topic) = rest.first().map(|s| s.as_str()) else {
        return Ok(ParsedCommand::Help(HelpTopic::Root));
    };
    reject_help_with_extra_args(rest)?;
    match topic {
        CLI_GROUP_MAIL if mail.shows_mail() => Ok(ParsedCommand::Help(HelpTopic::Mail)),
        CLI_GROUP_AGENT => Ok(ParsedCommand::Help(HelpTopic::Agent)),
        // help 뒤에 또 help 토큰이 오는 것은 계열 이름이 아니다 — 규칙("help 는 단독 호출") 그대로 반려한다.
        // 감춰진 계열도 여기로 떨어진다(ADR-0133) — 오타와 같은 답이라야 감춘 의미가 있다.
        other => Err(format!(
            "unknown help topic: {other} — run `{CLI_EXE_NAME} help` to list groups, or `{CLI_EXE_NAME} help {}`",
            example_help_topic(mail)
        )),
    }
}

fn parse_mail(rest: &[String]) -> Result<ParsedMail, String> {
    match rest.first().map(|s| s.as_str()) {
        Some("send") => parse_send_args(&rest[1..]),
        Some("status") => {
            let id = rest.get(1).ok_or_else(|| {
                format!(
                    "status requires a message id, e.g. `{CLI_EXE_NAME} {CLI_GROUP_MAIL} status m-7f3k9q2d`"
                )
            })?;
            // ★위치 인자에 플래그 표기가 오면 값이 아니라 오타다★: 브로커가 발급하는 id 는 `-` 로 시작하지
            //   않는다. 그대로 실어 보내면 `--help` 가 메시지 id 로 조회돼 무의미한 왕복 + MESSAGE_NOT_FOUND 로
            //   끝난다(인자 오류가 네트워크를 타면 안 된다는 이 파일의 규율에도 어긋난다).
            if id.starts_with('-') {
                return Err(format!(
                    "`{id}` is not a message id (ids look like m-7f3k9q2d) — run `{CLI_EXE_NAME} help {CLI_GROUP_MAIL}`"
                ));
            }
            if rest.len() > 2 {
                return Err(format!(
                    "unexpected argument after status <id>: {} — run `{CLI_EXE_NAME} help {CLI_GROUP_MAIL}`",
                    rest[2]
                ));
            }
            Ok(ParsedMail::Status { id: id.clone() })
        }
        Some("pending") => {
            if rest.len() > 1 {
                return Err(format!(
                    "pending takes no arguments: {} — run `{CLI_EXE_NAME} help {CLI_GROUP_MAIL}`",
                    rest[1]
                ));
            }
            Ok(ParsedMail::Pending)
        }
        Some(other) => Err(format!(
            "unknown {CLI_GROUP_MAIL} verb: {other} — run `{CLI_EXE_NAME} help {CLI_GROUP_MAIL}`"
        )),
        None => Err(format!(
            "{CLI_GROUP_MAIL} needs a verb ({}) — run `{CLI_EXE_NAME} help {CLI_GROUP_MAIL}`",
            CLI_MAIL_VERBS.join("|")
        )),
    }
}

/// `--parent` 가 "부모 없음(루트)" 을 뜻하는 낱말.
///
/// ★플래그를 생략하는 형태로 두지 않은 이유★: 생략은 "안 줬다" 와 구분되지 않아 오타 하나가 조용히 루트로
///   떼는 동작이 된다. 명시 낱말이면 의도가 argv 에 남는다.
/// ★대가(문서화된 한계)★: **`none` 이라는 이름의 에이전트는 이 플래그로 부모 지정이 안 된다** — 그 경우는
///   id 로 지목한다(help 에 적혀 있다). 이름이 유일해도 낱말과 이름은 같은 공간을 쓰므로 어느 쪽이든 한
///   자리는 내줘야 하고, 데몬이 그 이름을 실제로 갖는지 물어보는 왕복은 파싱을 네트워크에 매단다.
const AGENT_PARENT_NONE: &str = "none";

fn parse_agent(rest: &[String]) -> Result<ParsedAgent, String> {
    match rest.first().map(|s| s.as_str()) {
        Some("list") => {
            if rest.len() > 1 {
                return Err(format!(
                    "list takes no arguments: {} — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`",
                    rest[1]
                ));
            }
            Ok(ParsedAgent::List)
        }
        Some("spawn") => parse_agent_spawn(&rest[1..]),
        Some("new") => parse_agent_new(&rest[1..]),
        Some("rename") => parse_agent_rename(&rest[1..]),
        Some("move") => parse_agent_move(&rest[1..]),
        Some(other) => Err(format!(
            "unknown {CLI_GROUP_AGENT} verb: {other} — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`"
        )),
        None => Err(format!(
            "{CLI_GROUP_AGENT} needs a verb ({}) — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`",
            CLI_AGENT_VERBS.join("|")
        )),
    }
}

/// 위치 인자 1개를 **한 번만** 받는다 — 값 플래그의 `take_once` 와 같은 규율(두 번째 값이 첫 번째를 조용히
/// 덮지 않는다).
///
/// ★`-` 로 시작하면 이름이 아니라 오타다★: 에이전트 이름은 cwd basename 또는 사용자가 준 표시명이라 대시로
///   시작하지 않는다. 그대로 실어 보내면 `--help` 가 에이전트 이름으로 조회돼 무의미한 왕복 + NOT_FOUND 로
///   끝난다(우편의 `status <id>` 자리와 같은 판단).
/// 제어 계열 값 플래그 — `take_once` 에 **빈 값 반려**를 더한다.
///
/// ★왜 계열 전용인가★: 빈 값이 인자 오류인 것은 이 계열의 성질이다(이름·경로·부모는 전부 무언가를 가리키는
///   토큰이다). 우편 본문은 반대로 빈 문자열이 유효할 수 있어 공용 `take_once` 에 이 규칙을 얹지 않는다 —
///   그쪽 동작을 바꾸지 않는다.
/// ★막는 사고★: 셸에서 미설정 변수는 **빈 인자**로 펼쳐진다(`--parent "$UNSET"`). 그걸 값으로 실어 보내면
///   데몬이 반려하긴 하지만(같은 규율), 인자 오류가 네트워크를 타지 않는다는 이 파일의 규율에 어긋난다.
fn take_agent_value(
    slot: &mut Option<String>,
    flag: &str,
    value: Option<&String>,
) -> Result<(), String> {
    take_once(slot, flag, value, CLI_GROUP_AGENT, &CLI_AGENT_FLAGS)?;
    if slot.as_deref().is_some_and(|s| s.trim().is_empty()) {
        return Err(format!(
            "{flag} was given an empty value — that is usually an unset shell variable; pass a value or drop the flag; run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`"
        ));
    }
    Ok(())
}

fn take_positional(
    slot: &mut Option<String>,
    what: &str,
    value: &str,
    verb: &str,
) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!(
            "{verb} was given an empty {what} — that is usually an unset shell variable; run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`"
        ));
    }
    if value.starts_with('-') {
        return Err(format!(
            "`{value}` is not {what} — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}` for this group's flags"
        ));
    }
    if slot.is_some() {
        return Err(format!(
            "{verb} takes one {what}, but got a second one ({value}) — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`"
        ));
    }
    *slot = Some(value.to_string());
    Ok(())
}

fn unknown_agent_argument(arg: &str) -> String {
    format!(
        "unknown argument: {arg} — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}` for this group's flags"
    )
}

/// `spawn <name>`(깨우기) 와 `spawn --cwd <path>`(만들어서 띄우기)가 한 동사를 나눠 쓴다.
///
/// ★두 형태를 여기서 갈라 반려한다 — 네트워크 전에★: 둘 다 주거나 둘 다 안 준 호출은 데몬도 반려하지만,
///   왕복 없이 끝내는 편이 낫다(인자 오류가 네트워크를 타지 않는다는 이 파일의 규율). 데몬 쪽 검사는
///   지우지 말 것 — 이 CLI 가 그 라우트의 유일한 호출자라는 보장은 없다.
fn parse_agent_spawn(args: &[String]) -> Result<ParsedAgent, String> {
    let mut target: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut name: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--cwd" => {
                i += 1;
                take_agent_value(&mut cwd, "--cwd", args.get(i))?;
            }
            "--name" => {
                i += 1;
                take_agent_value(&mut name, "--name", args.get(i))?;
            }
            other if other.starts_with('-') => return Err(unknown_agent_argument(other)),
            other => take_positional(&mut target, "an agent name", other, "spawn")?,
        }
        i += 1;
    }
    match (&target, &cwd) {
        (Some(_), Some(_)) => Err(format!(
            "spawn takes either the name of an agent to wake or --cwd for a new one, not both — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`"
        )),
        (None, None) => Err(format!(
            "spawn needs either the name of an agent to wake or --cwd <path> to create a new one — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`"
        )),
        // ★조용히 무시하지 않는다★: 깨우기는 이름을 바꾸지 않으므로 `--name` 이 아무 일도 못 한다. 삼키면
        //   호출자는 이름이 바뀐 줄 안다(개명은 `rename` 이다).
        (Some(t), None) if name.is_some() => Err(format!(
            "--name does not apply when waking an existing agent ({t}); use `{CLI_EXE_NAME} {CLI_GROUP_AGENT} rename` to change a name, or drop --name"
        )),
        _ => Ok(ParsedAgent::Spawn {
            target: target.clone(),
            cwd: cwd.clone(),
            name,
        }),
    }
}

fn parse_agent_new(args: &[String]) -> Result<ParsedAgent, String> {
    let mut cwd: Option<String> = None;
    let mut name: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--cwd" => {
                i += 1;
                take_agent_value(&mut cwd, "--cwd", args.get(i))?;
            }
            "--name" => {
                i += 1;
                take_agent_value(&mut name, "--name", args.get(i))?;
            }
            other if other.starts_with('-') => return Err(unknown_agent_argument(other)),
            other => {
                return Err(format!(
                    "new takes no positional arguments ({other}); the folder goes in --cwd <path> — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`"
                ))
            }
        }
        i += 1;
    }
    let cwd = cwd.ok_or_else(|| {
        format!("new requires --cwd <path> — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`")
    })?;
    Ok(ParsedAgent::New { cwd, name })
}

fn parse_agent_rename(args: &[String]) -> Result<ParsedAgent, String> {
    let mut target: Option<String> = None;
    let mut name: Option<String> = None;
    for arg in args {
        if arg.starts_with('-') {
            return Err(unknown_agent_argument(arg));
        }
        // 첫 위치 인자가 대상, 둘째가 새 이름. 셋째부터는 `take_positional` 이 반려한다.
        if target.is_none() {
            take_positional(&mut target, "an agent name", arg, "rename")?;
        } else {
            take_positional(&mut name, "a new name", arg, "rename")?;
        }
    }
    let target = target.ok_or_else(|| {
        format!(
            "rename needs the agent to rename and its new name, e.g. `{CLI_EXE_NAME} {CLI_GROUP_AGENT} rename qa-bravo qa-lead` — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`"
        )
    })?;
    let name = name.ok_or_else(|| {
        format!(
            "rename needs the new name, e.g. `{CLI_EXE_NAME} {CLI_GROUP_AGENT} rename {target} qa-lead` — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`"
        )
    })?;
    Ok(ParsedAgent::Rename { target, name })
}

fn parse_agent_move(args: &[String]) -> Result<ParsedAgent, String> {
    let mut target: Option<String> = None;
    let mut parent: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--parent" => {
                i += 1;
                take_agent_value(&mut parent, "--parent", args.get(i))?;
            }
            other if other.starts_with('-') => return Err(unknown_agent_argument(other)),
            other => take_positional(&mut target, "an agent name", other, "move")?,
        }
        i += 1;
    }
    let target = target.ok_or_else(|| {
        format!(
            "move needs the agent to move, e.g. `{CLI_EXE_NAME} {CLI_GROUP_AGENT} move qa-bravo --parent lead` — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`"
        )
    })?;
    let parent = parent.ok_or_else(|| {
        format!(
            "move requires --parent <name|{AGENT_PARENT_NONE}> ({AGENT_PARENT_NONE} moves it back to the top level) — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`"
        )
    })?;
    Ok(ParsedAgent::Move {
        target,
        parent: (parent != AGENT_PARENT_NONE).then_some(parent),
    })
}

/// 값 플래그 1개를 **한 번만** 받는다.
///
/// ★두 번째 값이 첫 번째를 조용히 덮게 두지 않는다★: `--body first --body second` 는 예전엔 `second` 만
///   보내고 `first` 를 말없이 버렸다. argv 를 이어 붙여 명령을 고쳐 쓰는 호출자(에이전트가 흔히 그렇게
///   한다)는 그 유실을 볼 방법이 없다 — `--body` ↔ `--body-stdin` 상호배타를 반려하는 이유와 **같은**
///   이유라, 같은 자리에서 같게 끊는다.
/// ★`group`·`known_flags` 를 받는 이유★: 두 계열이 각자의 플래그 집합과 각자의 help 화면을 갖는다. 한
///   계열의 목록으로 다른 계열의 값 자리를 방어하면 `--parent` 를 본문으로 삼키는 부류의 사고가 되살아난다.
fn take_once(
    slot: &mut Option<String>,
    flag: &str,
    value: Option<&String>,
    group: &str,
    known_flags: &[&str],
) -> Result<(), String> {
    let value = value.ok_or_else(|| {
        format!(
            "{flag} requires a value — run `{CLI_EXE_NAME} help {group}` for this group's flags"
        )
    })?;
    // ★값 자리에 **우리가 아는 플래그**가 오면 값이 아니라 빠뜨린 값이다★: `--body --body-stdin` 은 예전엔
    //   `--body-stdin` 을 본문 문자열로 삼켰다 — 그러면 상호배타 검사가 발화하지 않고, 파이프로 들어온
    //   진짜 본문이 버려진 채 리터럴 `--body-stdin` 이 팀에 배달된다(exit 0 으로).
    // ★판정 기준은 `-` 로 시작하는지가 **아니라** 이 CLI 가 아는 플래그인지다★: 본문·수신자는 임의
    //   텍스트라 `--body -h` 나 `--body --anything-we-do-not-know` 는 그대로 값으로 실려야 한다.
    if known_flags.contains(&value.as_str()) {
        return Err(format!(
            "{flag} has no value — the next argument is another flag ({value}); give {flag} its value, or drop it; run `{CLI_EXE_NAME} help {group}` for this group's flags"
        ));
    }
    if slot.is_some() {
        return Err(format!(
            "{flag} was given more than once — pass it once (the later value would silently replace the earlier one); run `{CLI_EXE_NAME} help {group}` for this group's flags"
        ));
    }
    *slot = Some(value.clone());
    Ok(())
}

/// ★플래그 설계(메인 재량, 보고)★: 명시 `--to`/`--body` 한 쌍 — 위치 인자는 body 에 공백/따옴표가 섞이면
///   셸 인용이 깨지기 쉬워(스파이크에서 관찰된 실패 모드) 명시 플래그로 고정한다.
fn parse_send_args(args: &[String]) -> Result<ParsedMail, String> {
    let mut to: Option<String> = None;
    let mut body: Option<String> = None;
    let mut body_stdin = false;
    let mut request = false;
    let mut reply_by: Option<String> = None;
    let mut reply_to: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--to" => {
                i += 1;
                take_once(&mut to, "--to", args.get(i), CLI_GROUP_MAIL, &CLI_MAIL_FLAGS)?;
            }
            "--body" => {
                i += 1;
                take_once(&mut body, "--body", args.get(i), CLI_GROUP_MAIL, &CLI_MAIL_FLAGS)?;
            }
            // 불리언 플래그는 반복돼도 버려지는 값이 없다(같은 뜻의 반복) — 그래서 반려하지 않는다.
            "--body-stdin" => body_stdin = true,
            "--request" => request = true,
            "--reply-by" => {
                i += 1;
                take_once(
                    &mut reply_by,
                    "--reply-by",
                    args.get(i),
                    CLI_GROUP_MAIL,
                    &CLI_MAIL_FLAGS,
                )?;
            }
            "--reply-to" => {
                i += 1;
                take_once(
                    &mut reply_to,
                    "--reply-to",
                    args.get(i),
                    CLI_GROUP_MAIL,
                    &CLI_MAIL_FLAGS,
                )?;
            }
            other => {
                return Err(format!(
                    "unknown argument: {other} — run `{CLI_EXE_NAME} help {CLI_GROUP_MAIL}` for this group's flags"
                ))
            }
        }
        i += 1;
    }
    let to = to.ok_or("missing required --to <agent-name>")?;
    // ★상호배타(CLI 고유 배관)★: 두 출처가 동시에 오면 어느 쪽이 본문인지 **데몬은 영영 알 수 없다**
    //   (wire 엔 body 하나뿐). 조용히 한쪽을 고르면 heredoc 으로 넣은 긴 본문이 짧은 `--body` 에 먹혀
    //   사라지는 사고가 나므로, 여기서 반려한다.
    let body = match (body, body_stdin) {
        (Some(_), true) => {
            return Err(
                "--body and --body-stdin are mutually exclusive; pass the text one way only."
                    .to_string(),
            )
        }
        (Some(text), false) => BodySource::Literal(text),
        (None, true) => BodySource::Stdin,
        (None, false) => {
            return Err(
                "missing required --body <text> (or --body-stdin to read it from stdin)"
                    .to_string(),
            )
        }
    };
    Ok(ParsedMail::Send {
        to,
        body,
        request,
        reply_by,
        reply_to,
    })
}

/// ★왜 lossy 인가(D 리뷰 A2 — 구현으로 정렬)★: 이전 구현은 `read_to_string` 이라 잘못된 UTF-8 을 만나면
///   `InvalidData` 로 **발송 자체를 거부**했다. 이 CLI 가 도는 자리는 Windows 셸이고, cp949 로 인코딩된
///   파이프 입력이 현실적으로 들어온다 — 그때 "메시지를 아예 못 보낸다" 보다 "몇 글자가 U+FFFD 로 깨진
///   채라도 팀에 전달된다" 가 낫다. 본문은 사람이 읽는 텍스트지 바이트 정확성이 계약인 데이터가
///   아니고(봉투 XML 이스케이프는 데몬이 별도 처리),
///   조용한 유실보다 가시적 열화를 택하는 이 프로젝트의 기조와도 같은 방향이다.
/// ★상한 방어는 데몬이 한다★: 64KiB 초과는 `BODY_TOO_LARGE` 로 반려된다(ingress).
fn read_stdin_to_string() -> Result<String, String> {
    let mut buf = Vec::new();
    std::io::stdin()
        .read_to_end(&mut buf)
        .map_err(|e| format!("--body-stdin: failed to read stdin: {e}"))?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// ★본문 trim 하지 않음★: 앞뒤 공백·개행은 발신자가 의도한 내용일 수 있다(코드 블록·heredoc 마지막 개행).
///   데몬도 body 를 trim 하지 않는다(ingress 규율) — 두 층이 같은 규칙을 지킨다.
/// ★빈 stdin 은 반려★: heredoc 을 붙이지 않고 `--body-stdin` 만 친 경우(흔한 실수)를 빈 본문 발송으로
///   흘려보내면 수신자에게 빈 봉투가 도착한다 — 형태 오류로 잡는 게 낫다.
fn materialize_body(
    parsed: ParsedMail,
    read_stdin: impl FnOnce() -> Result<String, String>,
) -> Result<Command, String> {
    Ok(match parsed {
        ParsedMail::Send {
            to,
            body,
            request,
            reply_by,
            reply_to,
        } => {
            let body = match body {
                BodySource::Literal(t) => t,
                BodySource::Stdin => {
                    let t = read_stdin()?;
                    if t.is_empty() {
                        return Err(
                            "--body-stdin got no input; pipe the text in (e.g. with a heredoc) or use --body."
                                .to_string(),
                        );
                    }
                    t
                }
            };
            Command::Send(CliArgs {
                to,
                body,
                request,
                reply_by,
                reply_to,
            })
        }
        ParsedMail::Status { id } => Command::Status { id },
        ParsedMail::Pending => Command::Pending,
    })
}

/// `{to, body, request?, reply_by?, reply_to?}` 요청 JSON 문자열. escape 는 serde_json 이 처리(손조립
/// 금지).
///
/// ★미지정 인자는 **키 자체를 안 싣는다**★: `request:false`/`reply_by:null` 을 실어도 데몬은 같게 읽지만,
///   옛 `{to, body}` 바디와 바이트 동형을 유지해 통보 경로의 wire 회귀 위험을 0 으로 둔다.
fn build_request_body(args: &CliArgs) -> String {
    let mut v = serde_json::json!({ "to": args.to, "body": args.body });
    if args.request {
        v["request"] = serde_json::Value::Bool(true);
    }
    if let Some(rb) = &args.reply_by {
        v["reply_by"] = serde_json::Value::String(rb.clone());
    }
    if let Some(rt) = &args.reply_to {
        v["reply_to"] = serde_json::Value::String(rt.clone());
    }
    v.to_string()
}

const EXIT_FAILED: i32 = 1;

/// ★성공 shape 자체가 깨졌을 때의 exit code(리뷰 fix 13)★ — 2xx 인데 body 가 spec §6 성공 shape 을 만족하지
///   않는 경우. `EXIT_FAILED`(반려)와 **구분**하는 이유: 반려는 발신자가 인자를 고쳐 재시도할 일이지만,
///   이건 데몬/프록시/버전 불일치 쪽 결함이라 재시도가 아니라 보고 대상이다. 두 값을 뭉개면 발신 에이전트가
///   "내 인자가 틀렸나" 를 무한히 자기교정하게 된다.
const EXIT_MALFORMED_SUCCESS: i32 = 2;

/// 성공 `results[]` 항목이 가질 수 있는 상태 어휘(spec §5·§6). 이 밖의 값은 shape 위반으로 본다.
///   ★`failed` 행이 있어도 exit 0(ADR-0111 결정 3 부분 진행)★ — 그 수신자만의 실패이고 발송 자체는
///   접수됐다. 누구에게 안 갔는지는 발신자가 `results[]` 를 읽고 판단한다.
const VALID_RESULT_STATUSES: [&str; 3] = ["delivered", "pending", "failed"];

/// 반려 shape(`{ status:"error", code, hint }` — spec §6)을 **검증**한다: `status` 가 정확히 `"error"` 이고
/// `code` 가 비어 있지 않은 문자열일 때만 true.
///
/// ★왜 검증하나(리뷰 fix 14 · load-bearing)★: 예전엔 `results` 키가 없기만 하면 무조건 실패(1)로 뭉갰다 —
///   `{}` 나 `{"id":"m-x"}`(성공 응답이 절반만 온 경우)도 "반려" 로 보고했다는 뜻이다. 반려는 `code` 로
///   분기할 수 있을 때만 반려다 — 그 외 2xx JSON 은 전부 shape 결함(2)으로 갈라 "보고 대상" 임을 알린다.
fn is_validated_error_shape(v: &serde_json::Value) -> bool {
    let status_is_error = v.get("status").and_then(|s| s.as_str()) == Some("error");
    let code_ok = v
        .get("code")
        .and_then(|c| c.as_str())
        .is_some_and(|s| !s.is_empty());
    status_is_error && code_ok
}

/// (HTTP status, body) → exit code. 성공(0) 조건 = **HTTP 2xx** 이고 body 가 spec §6 성공 shape 을 **완전히**
/// 만족할 때. **검증된** 반려 shape(`is_validated_error_shape`)·프레이밍 오류(비-JSON)·비-2xx 는
/// `EXIT_FAILED`(1). 2xx JSON 인데 성공 shape 도 반려 shape 도 아니면 `EXIT_MALFORMED_SUCCESS`(2) —
/// 성공/반려 어느 쪽으로도 읽을 수 없는 body(`{}`·`{"id":"m-x"}`)를 반려로 뭉개지 않는다(fix 14).
///
/// ★왜 "results 가 배열" 만으론 부족한가(리뷰 fix 13 · load-bearing)★: 예전 판정은 `results` 가 배열이기만
///   하면 exit 0 이었다 — `{"results":[]}`(아무에게도 안 갔다)나 `{"results":[{"to":null,"status":"exploded"}]}`
///   같은 body 도 **성공**으로 통과했다. 이 CLI 의 exit code 는 발신 에이전트가 "메시지가 접수됐나" 를
///   판단하는 유일한 기계적 신호라, 여기서 새는 가짜 성공은 곧 "보냈다고 믿고 넘어가는" 조용한 유실이 된다.
///   그래서 성공은 **전 조건**을 만족할 때만 0 이다:
///     ① 최상위 `id` 가 비어 있지 않은 문자열(장부·회신 상관 키 — spec §6)
///     ② `results` 가 **비어 있지 않은** 배열(수신자 0명 = 접수된 게 없다)
///     ③ 각 항목의 `to` 가 비어 있지 않은 문자열이고, `status` 가 어휘(delivered|pending|failed) 안
///   하나라도 어긋나면 우리가 아는 성공이 아니므로 `EXIT_MALFORMED_SUCCESS` + stderr 한 줄로 갈라 낸다.
/// ★비-2xx 는 항상 1★: status 를 무시하면 프레이밍 오류를 성공으로 오인할 위험이 있어 게이트를 둔다.
///   body 파싱 실패(비-JSON)도 1 — 2xx + 비-JSON 은 "성공 shape 위반" 이 아니라 프레이밍 실패로 본다
///   (절단·프록시 오류가 이 모양이라, 이미 있는 실패 축에 붙이는 게 정직하다).
fn exit_code_for_response(status: u16, resp_body: &str) -> i32 {
    if !(200..300).contains(&status) {
        return EXIT_FAILED;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(resp_body) else {
        return EXIT_FAILED;
    };
    // ★`status:"error"` 를 **가장 먼저** 본다(round-2 리뷰 F4 · load-bearing)★: 예전엔 이 검사가 "results 가
    //   없을 때" 안에만 있어서, **혼종** 응답(`{"status":"error", …, "results":[…]}`)이 error 축을 건너뛰고
    //   성공 shape 검사로 흘러 **exit 0** 이 났다 — 반려당한 발송을 성공으로 읽는 조용한 실패다.
    //   status 가 error 인 2xx 객체는 어떤 형태든 0 이 될 수 없다.
    if v.get("status").and_then(|s| s.as_str()) == Some("error") {
        if is_validated_error_shape(&v) {
            return EXIT_FAILED;
        }
        eprintln!(
            "{CLI_EXE_NAME}: malformed error response — 'status' is \"error\" but 'code' is missing or not a non-empty string, so this rejection cannot be acted on"
        );
        return EXIT_MALFORMED_SUCCESS;
    }
    let Some(results) = v.get("results") else {
        eprintln!(
            "{CLI_EXE_NAME}: malformed success response — neither a success shape ('results') nor a valid error shape ('status':\"error\" + non-empty 'code')"
        );
        return EXIT_MALFORMED_SUCCESS;
    };
    let Some(items) = results.as_array() else {
        eprintln!("{CLI_EXE_NAME}: malformed success response — 'results' is not an array");
        return EXIT_MALFORMED_SUCCESS;
    };
    let id_ok = v
        .get("id")
        .and_then(|i| i.as_str())
        .is_some_and(|s| !s.is_empty());
    if !id_ok {
        eprintln!("{CLI_EXE_NAME}: malformed success response — missing or empty 'id'");
        return EXIT_MALFORMED_SUCCESS;
    }
    if items.is_empty() {
        eprintln!(
            "{CLI_EXE_NAME}: malformed success response — 'results' is empty (nothing was accepted)"
        );
        return EXIT_MALFORMED_SUCCESS;
    }
    for item in items {
        let to_ok = item
            .get("to")
            .and_then(|t| t.as_str())
            .is_some_and(|s| !s.is_empty());
        let status_ok = item
            .get("status")
            .and_then(|s| s.as_str())
            .is_some_and(|s| VALID_RESULT_STATUSES.contains(&s));
        if !to_ok || !status_ok {
            eprintln!(
                "{CLI_EXE_NAME}: malformed success response — result entry needs a non-empty 'to' and a status of delivered|pending|failed"
            );
            return EXIT_MALFORMED_SUCCESS;
        }
    }
    0
}

/// ★조회 응답의 exit code 매핑(D)★ — `status`/`pending` 전용.
///
/// ★왜 발송과 다른 판정기인가(load-bearing)★: 발송 성공은 shape 이 **하나로 고정**(`{id, results[]}`)이라
///   전 조건을 검사할 수 있다. 조회 성공은 동사마다 다르다(`{id,from,awaiting_reply,rows}` /
///   `{me,open}`). 그 shape 들을 CLI 가 다시
///   기술하면 데몬 응답이 늘 때마다 **두 곳**을 고쳐야 하고, 한쪽만 고치면 정상 응답이 exit 2 로 튄다
///   (거짓 경보). 그래서 조회는 반대 방향으로 판정한다: **검증된 에러 shape 이면 실패(1), 그 외 2xx JSON
///   객체면 성공(0)**. 에러 어휘(`status:"error"` + 비지 않은 `code`)는 발송과 공유하는 안정 계약이라
///   이 규칙이 성립한다.
/// ★그래도 2 가 남는 이유★: 2xx 인데 JSON **객체가 아니면**(배열·스칼라·null) 우리 계약 밖 응답이다 —
///   프록시가 끼어들었거나 버전이 어긋난 것이라 재시도가 아니라 보고 대상이다.
/// ★`status:"error"` 는 **절대 0 이 될 수 없다**(D 리뷰 B5 · load-bearing)★: 반전 검증만 두면
///   `{"status":"error"}`(code 없음)나 `{"status":"error","code":""}` 같은 **반쯤 깨진 반려**가 "에러 shape 도
///   아니고 객체이긴 하다" 로 통과해 **exit 0** 이 났다 — 호출자는 반려당한 걸 성공으로 읽는다(조용한 실패의
///   교과서적 형태). 그래서 `status` 필드가 `"error"` 인 객체는 먼저 걸러 두 갈래로만 보낸다.
fn exit_code_for_query_response(status: u16, resp_body: &str) -> i32 {
    if !(200..300).contains(&status) {
        return EXIT_FAILED;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(resp_body) else {
        return EXIT_FAILED;
    };
    if is_validated_error_shape(&v) {
        return EXIT_FAILED;
    }
    if v.get("status").and_then(|s| s.as_str()) == Some("error") {
        eprintln!(
            "{CLI_EXE_NAME}: malformed error response — 'status' is \"error\" but 'code' is missing or not a non-empty string, so this rejection cannot be acted on"
        );
        return EXIT_MALFORMED_SUCCESS;
    }
    if !v.is_object() {
        eprintln!(
            "{CLI_EXE_NAME}: malformed response — expected a JSON object from the daemon query route"
        );
        return EXIT_MALFORMED_SUCCESS;
    }
    0
}

/// ★제어 동사 응답의 exit code 매핑(ADR-0132 조각 ②)★ — `agent` 계열 전용.
///
/// ★왜 우편 조회 판정기를 쓰지 않는가(load-bearing)★: 그쪽은 **"검증된 에러가 아니면 성공"** 이라, 2xx 로
///   온 `{}` 나 `{"status":"ok"}` 도 exit 0 이다. 우편에서는 그 관대함이 감수된 선재 결함이고(T-19) 이번에
///   건드리지 않는다 — 응답 shape 이 동사마다 다른데 CLI 가 그걸 다시 기술하면 두 곳을 함께 고쳐야 하기
///   때문이다. **제어는 사정이 다르다**: 이 계열은 양쪽 끝(데몬 라우트·이 CLI)을 우리가 같은 조각에서
///   만들었으므로 동사별 성공 shape 이 계약이고, 그걸 검사할 수 있다.
/// ★검사하지 않으면 나는 증상★: `engram agent new --cwd …` 가 아무것도 만들지 않은 응답에 대해 exit 0 을
///   내고, 호출자(LLM)는 만들어졌다고 믿고 그 이름으로 편지를 쓴다. 그래서 **변경의 증거를 싣지 않은 2xx 는
///   성공이 아니라 "읽을 수 없는 응답"(2)** 이다 — 재시도가 아니라 보고 대상이라는 뜻이고, 반려(1)와도 갈린다.
fn exit_code_for_agent_response(agent: &ParsedAgent, status: u16, resp_body: &str) -> i32 {
    if !(200..300).contains(&status) {
        return EXIT_FAILED;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(resp_body) else {
        return EXIT_FAILED;
    };
    if is_validated_error_shape(&v) {
        return EXIT_FAILED;
    }
    if v.get("status").and_then(|s| s.as_str()) == Some("error") {
        eprintln!(
            "{CLI_EXE_NAME}: malformed error response — 'status' is \"error\" but 'code' is missing or not a non-empty string, so this rejection cannot be acted on"
        );
        return EXIT_MALFORMED_SUCCESS;
    }
    // ★증거는 **있기만** 해선 안 되고 **요청한 동작과 맞아야** 한다★: 깨우기가 "만들었다" 고 답하거나
    //   루트로 떼라는 요청이 부모를 달고 오면, 필드가 다 있어도 그건 우리가 시킨 일의 결과가 아니다.
    //   그런 응답을 성공으로 읽으면 호출자는 일어나지 않은 일을 사실로 기록한다.
    let ok = match agent {
        // 빈 명부는 정상적인 답이다 — 배열이라는 것과 각 행의 형태만 본다. 행의 형태는 `help agent` 가
        //   약속하는 다섯 필드 그대로다 — 약속과 검사가 갈리면 둘 중 하나가 거짓말이 된다.
        ParsedAgent::List => v
            .get("agents")
            .and_then(|a| a.as_array())
            .is_some_and(|rows| rows.iter().all(list_row_ok)),
        // 띄우기는 **무엇이 떴는지** + **깨운 것인지 만든 것인지**가 증거다. 후자는 우리가 무엇을 요청했는지로
        //   이미 정해져 있으므로(`cwd` 를 준 호출만 새로 만든다) 값이 그 요청과 맞는지까지 본다.
        ParsedAgent::Spawn { cwd, .. } => {
            agent_object_ok(&v, true) && v.get("created") == Some(&serde_json::json!(cwd.is_some()))
        }
        // `new` 는 프로세스를 띄우지 않는다 — 살아 있다는 답은 우리가 시킨 일의 결과일 수 없다.
        ParsedAgent::New { .. } => {
            agent_object_ok(&v, true)
                && read_agent_evidence(&v).and_then(|e| e.state) == Some(AGENT_STATE_SLEEPING)
        }
        // 개명은 **확정된 이름**과 결말이 증거다(확정 이름은 요청 이름과 다를 수 있어 값 자체는 대조하지 않는다).
        ParsedAgent::Rename { .. } => {
            agent_object_ok(&v, false)
                && v.get("outcome")
                    .and_then(|o| o.as_str())
                    .is_some_and(|o| o == RENAME_OUTCOME_RENAMED || o == RENAME_OUTCOME_UNCHANGED)
        }
        // 이동은 **결과 부모**가 증거다. 데몬은 id 로 답하고 우리는 이름으로 물었으니 값은 대조할 수 없지만,
        //   **붙였나 뗐나**는 우리가 고른 축이라 그건 맞아야 한다.
        ParsedAgent::Move { parent, .. } => {
            agent_object_ok(&v, false)
                && match parent {
                    None => v.get("parent").is_some_and(|p| p.is_null()),
                    Some(_) => v
                        .get("parent")
                        .is_some_and(|p| p.as_str().is_some_and(|s| !s.is_empty())),
                }
        }
    };
    if ok {
        return 0;
    }
    eprintln!(
        "{CLI_EXE_NAME}: malformed success response — the daemon answered 2xx without the evidence this verb must carry (see `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`)"
    );
    // ★한 줄 더 내는 이유(load-bearing)★: 이 exit 2 가 dev 에서 가장 자주 나는 경로는 데몬 결함이 아니라
    //   **빌드 세대 차**다 — `cargo build` 는 이 CLI 만 갈아 끼우고 이미 떠 있는 데몬은 relink 하지 않는데,
    //   기존 에이전트는 다음 호출부터 새 CLI 를 집는다. 그 자리에서 옛 shape 을 받으면 성공한 조작이
    //   exit 2 로 보고된다. 단정하지 않고 후보로만 적는다 — CLI 는 상대 데몬의 빌드 세대를 알 수 없다.
    eprintln!(
        "{CLI_EXE_NAME}: one possible cause is a daemon from an older build — rebuilding this CLI does not relink a daemon that is already running, so restart the daemon and retry before reporting it (`cargo build` alone is not enough)"
    );
    EXIT_MALFORMED_SUCCESS
}

/// `list` 행 하나 — **`engram help agent` 가 약속하는 다섯 필드**가 정본이다.
///
/// ★`cwd`·`parent` 는 값 대신 **형태**만 본다★: 경로는 비어 있을 수 있고(저장된 cwd 가 그럴 수 있다) 부모는
///   최상위면 null 이다. 요구하는 것은 "그 자리가 답해졌다" 이지 특정 값이 아니다.
fn list_row_ok(row: &serde_json::Value) -> bool {
    nonempty_str(row, "id")
        && nonempty_str(row, "name")
        && has_agent_state(row)
        && row.get("cwd").is_some_and(|c| c.is_string())
        && row
            .get("parent")
            .is_some_and(|p| p.is_null() || p.as_str().is_some_and(|s| !s.is_empty()))
}

fn nonempty_str(v: &serde_json::Value, key: &str) -> bool {
    v.get(key)
        .and_then(|x| x.as_str())
        .is_some_and(|s| !s.is_empty())
}

fn has_agent_state(v: &serde_json::Value) -> bool {
    v.get("state")
        .and_then(|s| s.as_str())
        .is_some_and(is_agent_state)
}

fn is_agent_state(s: &str) -> bool {
    s == AGENT_STATE_LIVE || s == AGENT_STATE_SLEEPING
}

/// 변경 동사 응답이 나르는 "어느 에이전트인가". 값 검사는 하지 않는다 — 비지 않았나·어휘 안인가는 호출부 몫.
struct AgentEvidence<'a> {
    id: &'a str,
    name: &'a str,
    /// 생사를 싣지 않는 동사(개명·이동)의 응답에선 `None` — 값이 문자열이 아닐 때도 같게 접힌다.
    /// 이 자리를 요구할지는 `agent_object_ok` 의 `with_state` 가 정한다.
    state: Option<&'a str>,
}

/// 성공 body 최상위에서 신원을 집는다 — 신원 두 자리(`agent_id`·`name`) 중 하나라도 없거나 문자열이 아니면
/// 증거 자체가 없는 것으로 접는다(신원을 반만 읽고 통과시키면 그 반쪽이 어느 에이전트인지 말해주지 못한다).
/// `state` 는 접는 축이 아니다 — 부재·비문자열이면 `None` 이고, 요구 여부는 호출부가 `with_state` 로 정한다.
///
/// ★`{agent:{…}}` 갈래를 되살리지 말 것★: 데몬은 평평하게만 답한다(사용자 결정 2026-08-13, 라우트 착지
///   732c9b8). 그 갈래는 **어떤 데몬도 내지 않는** shape 을 받는 죽은 코드라, 되살리면 계약 밖 body 에
///   exit 0 을 내주는 통로가 된다.
fn read_agent_evidence(v: &serde_json::Value) -> Option<AgentEvidence<'_>> {
    Some(AgentEvidence {
        id: v.get("agent_id")?.as_str()?,
        name: v.get("name")?.as_str()?,
        state: v.get("state").and_then(|s| s.as_str()),
    })
}

/// 변경 동사 공통 증거 — 어느 에이전트인지(id·이름). `with_state` 는 생사까지 실리는 동사에만 켠다
/// (개명·이동은 생사를 바꾸지 않으므로 그 필드를 요구하지 않는다).
fn agent_object_ok(v: &serde_json::Value, with_state: bool) -> bool {
    read_agent_evidence(v).is_some_and(|e| {
        !e.id.is_empty()
            && !e.name.is_empty()
            && (!with_state || e.state.is_some_and(is_agent_state))
    })
}

/// (Debug = 단위 테스트에서 expect_err 시 Ok 쪽 표시용.)
#[derive(Debug)]
struct HttpResponse {
    status: u16,
    body: String,
}

/// 로컬 평문 HTTP 전용(TLS 미지원 — 데몬은 127.0.0.1 평문).
///
/// ★라우트 인자화(D)★: 발송·조회가 같은 서버·같은 auth·같은 프레이밍을 쓰므로 경로만 갈아 끼운다
///   (동사마다 HTTP 클라이언트를 복제하면 절단 처리·헤더 규약이 세 벌이 된다).
fn post_json(
    base: &str,
    route: &str,
    token: &str,
    request_body: &str,
) -> Result<HttpResponse, SendError> {
    let (host, port) = parse_host_port(base).map_err(SendError::Connect)?;
    let path = format!("{}{route}", base_path(base));

    let mut stream = TcpStream::connect((host.as_str(), port))
        .map_err(|e| SendError::Connect(format!("connect {host}:{port} failed: {e}")))?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|_| stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|e| SendError::Connect(format!("set timeout failed: {e}")))?;

    // Content-Length 필수(서버가 body 경계를 알게), Connection: close(응답 후 종료 → 서버가 응답 뒤 소켓을
    //   닫아 read_to_end 가 결정적으로 EOF 를 본다).
    let req = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: {host}:{port}\r\n\
         Authorization: Bearer {token}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\
         \r\n\
         {request_body}",
        len = request_body.len(),
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| SendError::Connect(format!("write failed: {e}")))?;
    stream.flush().ok();

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| SendError::Connect(format!("read failed: {e}")))?;
    parse_response(&raw)
}

/// raw HTTP 응답 바이트 → (status, body). 최소하지만 **정확한** HTTP/1.1 응답 파서(F4):
///   - status line(`HTTP/1.1 <code> ...`)에서 코드 파싱.
///   - 헤더는 헤더/본문 경계(`\r\n\r\n`)까지, key 는 **대소문자 무시** 비교.
///   - `Transfer-Encoding: chunked` 면 청크를 de-frame(길이-접두 청크 이어붙임, 0-청크에서 종료).
///   - 아니고 `Content-Length` 가 있으면 정확히 그 바이트만큼 취한다(초과분=파이프라인 잔재 무시). ★수신
///     body 가 선언된 길이보다 **짧으면**(mid-body 절단) INCOMPLETE_RESPONSE 에러 — 절단 버퍼가 우연히
///     JSON 으로 파싱돼 가짜 성공(exit 0)으로 새는 걸 원천 차단한다(M1).
///   - 둘 다 없으면 나머지 전부를 body 로(Connection: close read-to-EOF fallback).
fn parse_response(raw: &[u8]) -> Result<HttpResponse, SendError> {
    let sep = find_subslice(raw, b"\r\n\r\n").ok_or_else(|| {
        SendError::Connect("malformed HTTP response (no header/body separator)".to_string())
    })?;
    let head = &raw[..sep];
    let body_bytes = &raw[sep + 4..];
    let head_text = String::from_utf8_lossy(head);
    let mut lines = head_text.split("\r\n");

    let status_line = lines.next().ok_or_else(|| {
        SendError::Connect("malformed HTTP response (no status line)".to_string())
    })?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| SendError::Connect(format!("malformed HTTP status line: {status_line}")))?;

    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim();
            match key.as_str() {
                "content-length" => content_length = val.parse().ok(),
                "transfer-encoding" => {
                    // 값에 chunked 가 포함되면(단일/코딩 목록) chunked 로 취급.
                    if val.to_ascii_lowercase().contains("chunked") {
                        chunked = true;
                    }
                }
                _ => {}
            }
        }
    }

    let body = if chunked {
        dechunk(body_bytes)?
    } else if let Some(len) = content_length {
        if body_bytes.len() < len {
            return Err(SendError::Incomplete {
                received: body_bytes.len(),
                expected: len,
            });
        }
        String::from_utf8_lossy(&body_bytes[..len]).to_string()
    } else {
        String::from_utf8_lossy(body_bytes).to_string()
    };
    Ok(HttpResponse {
        status,
        body: body.trim().to_string(),
    })
}

/// `Transfer-Encoding: chunked` de-framing. 각 청크 = `<hex-len>\r\n<data>\r\n`, 0-길이 청크에서 종료.
/// chunk extension(`;` 뒤)·trailer 는 무시한다(로컬 데몬 응답엔 안 나오나 방어적으로 스킵).
/// 프레이밍/절단 실패는 SendError::Connect(프레이밍 파싱 오류로 취급 — M1 의 Content-Length short-read 와
/// 달리 chunked 는 응답이 스스로 종료(0-청크)를 선언하는 프로토콜이라 파서 관점의 malformed 다).
fn dechunk(mut bytes: &[u8]) -> Result<String, SendError> {
    let mut out: Vec<u8> = Vec::new();
    loop {
        let line_end = find_subslice(bytes, b"\r\n").ok_or_else(|| {
            SendError::Connect("malformed chunked body (no size line)".to_string())
        })?;
        let size_line = String::from_utf8_lossy(&bytes[..line_end]);
        let hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(hex, 16)
            .map_err(|e| SendError::Connect(format!("malformed chunk size '{hex}': {e}")))?;
        bytes = &bytes[line_end + 2..];
        if size == 0 {
            break;
        }
        if bytes.len() < size {
            return Err(SendError::Connect("truncated chunked body".to_string()));
        }
        out.extend_from_slice(&bytes[..size]);
        bytes = &bytes[size..];
        if bytes.starts_with(b"\r\n") {
            bytes = &bytes[2..];
        }
    }
    Ok(String::from_utf8_lossy(&out).to_string())
}

/// std 만 — memchr 미의존.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// `http://127.0.0.1:PORT` 형태 전제(스킴은 http 만).
fn parse_host_port(base: &str) -> Result<(String, u16), String> {
    let rest = base
        .strip_prefix("http://")
        .ok_or_else(|| format!("unsupported control url (expected http://): {base}"))?;
    let authority = rest.split('/').next().unwrap_or(rest);
    let (host, port) = authority
        .rsplit_once(':')
        .ok_or_else(|| format!("control url missing port: {base}"))?;
    let port: u16 = port
        .parse()
        .map_err(|e| format!("invalid port in control url: {e}"))?;
    Ok((host.to_string(), port))
}

fn base_path(base: &str) -> String {
    let rest = base.strip_prefix("http://").unwrap_or(base);
    match rest.find('/') {
        Some(idx) => rest[idx..].trim_end_matches('/').to_string(),
        None => String::new(),
    }
}

/// 데몬 ACK/에러와 **같은 shape**(status/code/hint)으로 낸다 — 발신 에이전트가 파싱해 자기교정한다.
fn print_error(code: &str, hint: &str) {
    let v = serde_json::json!({ "status": "error", "code": code, "hint": hint });
    println!("{v}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_core::agent::types::MAIL_MARKER_ON;

    /// 표식 없는 프로세스(= 전부 보이는 기본 표면)로 파싱한다. **표면 축을 보는 테스트만**
    /// `super::parse_command` 를 직접 불러 `MailSurface` 를 명시한다 — 나머지는 그 축과 무관하다.
    fn parse_command(args: &[String]) -> Result<ParsedCommand, String> {
        super::parse_command(args, MailSurface::default())
    }

    fn parse_mail_command(args: &[String]) -> Result<ParsedMail, String> {
        match parse_command(args)? {
            ParsedCommand::Mail(m) => Ok(m),
            other => Err(format!("우편 커맨드가 아니다: {other:?}")),
        }
    }

    /// 플래그 조합 전수 테스트가 계열·동사를 매번 다시 적지 않게 여기서 한 번 붙인다. 표기 자체
    /// (`mail send` 여야 한다)는 아래 「계열·동사 표기」 구획이 본다.
    fn parse_args(flags: &[String]) -> Result<CliArgs, String> {
        let mut args = argv(&[CLI_GROUP_MAIL, "send"]);
        args.extend_from_slice(flags);
        match materialize_body(parse_mail_command(&args)?, || {
            panic!("리터럴 본문 경로가 stdin 을 읽으면 안 된다")
        })? {
            Command::Send(a) => Ok(a),
            _ => Err("발송이 아닌 커맨드".to_string()),
        }
    }

    fn argv(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // ── 인자 파싱 ────────────────────────────────────────────────────────────────
    #[test]
    fn parse_args_both_flags() {
        let a = vec![
            "--to".into(),
            "bob".into(),
            "--body".into(),
            "hello world".into(),
        ];
        let p = parse_args(&a).expect("parse");
        assert_eq!(p.to, "bob");
        assert_eq!(p.body, "hello world");
    }

    #[test]
    fn parse_args_order_independent() {
        let a = vec!["--body".into(), "hi".into(), "--to".into(), "alice".into()];
        let p = parse_args(&a).expect("parse");
        assert_eq!(p.to, "alice");
        assert_eq!(p.body, "hi");
    }

    #[test]
    fn parse_args_missing_to_errs() {
        let a = vec!["--body".into(), "hi".into()];
        assert!(parse_args(&a).is_err(), "--to 누락은 에러");
    }

    #[test]
    fn parse_args_missing_value_errs() {
        let a = vec!["--to".into()];
        assert!(parse_args(&a).is_err(), "값 없는 --to 는 에러");
    }

    #[test]
    fn parse_args_unknown_flag_errs() {
        let a = vec!["--nope".into(), "x".into()];
        assert!(parse_args(&a).is_err(), "알 수 없는 플래그는 에러");
    }

    // ── C3: 회신 계약 플래그(MCP 툴 미러 — 표면 정본 = ADR-0132) ─────────────────────────────

    #[test]
    fn parse_args_request_flag_takes_no_value() {
        let a = vec![
            "--request".into(),
            "--to".into(),
            "bob".into(),
            "--body".into(),
            "해줘".into(),
        ];
        let p = parse_args(&a).expect("parse");
        assert!(p.request);
        assert_eq!(p.to, "bob");
        assert_eq!(p.body, "해줘");
    }

    #[test]
    fn parse_args_reply_by_and_reply_to_take_values() {
        let a = vec![
            "--to".into(),
            "bob".into(),
            "--body".into(),
            "해줘".into(),
            "--request".into(),
            "--reply-by".into(),
            "10m".into(),
        ];
        let p = parse_args(&a).expect("parse");
        assert!(p.request);
        assert_eq!(p.reply_by.as_deref(), Some("10m"));
        assert_eq!(p.reply_to, None);

        let a = vec![
            "--to".into(),
            "alice".into(),
            "--body".into(),
            "했음".into(),
            "--reply-to".into(),
            "m-7f3k".into(),
        ];
        let p = parse_args(&a).expect("parse");
        assert!(!p.request);
        assert_eq!(p.reply_to.as_deref(), Some("m-7f3k"));
    }

    #[test]
    fn parse_args_missing_values_for_c3_flags_err() {
        assert!(parse_args(&["--reply-by".to_string()]).is_err());
        assert!(parse_args(&["--reply-to".to_string()]).is_err());
    }

    #[test]
    fn parse_args_does_not_validate_semantics_daemon_does() {
        let a = vec![
            "--to".into(),
            "bob".into(),
            "--body".into(),
            "x".into(),
            "--request".into(),
            "--reply-to".into(),
            "m-1".into(),
        ];
        let p = parse_args(&a).expect("CLI 는 형태만 본다");
        assert!(p.request && p.reply_to.is_some());
        let v: serde_json::Value =
            serde_json::from_str(&build_request_body(&p)).expect("valid json");
        assert_eq!(v["request"], true);
        assert_eq!(v["reply_to"], "m-1");
    }

    #[test]
    fn build_request_body_omits_unset_contract_fields() {
        let v: serde_json::Value =
            serde_json::from_str(&build_request_body(&plain("bob", "hi"))).expect("valid json");
        assert!(v.get("request").is_none(), "미지정 request 키 없음");
        assert!(v.get("reply_by").is_none(), "미지정 reply_by 키 없음");
        assert!(v.get("reply_to").is_none(), "미지정 reply_to 키 없음");
        assert_eq!(
            build_request_body(&plain("bob", "hi")),
            serde_json::json!({ "to": "bob", "body": "hi" }).to_string(),
            "통보 바디는 옛 shape 그대로"
        );
    }

    #[test]
    fn build_request_body_maps_kebab_flags_to_snake_case_wire() {
        let args = CliArgs {
            to: "bob".to_string(),
            body: "해줘".to_string(),
            request: true,
            reply_by: Some("10m".to_string()),
            reply_to: None,
        };
        let v: serde_json::Value =
            serde_json::from_str(&build_request_body(&args)).expect("valid json");
        assert_eq!(v["request"], true);
        assert_eq!(v["reply_by"], "10m");
        assert!(v.get("reply-by").is_none(), "wire 는 snake_case 뿐");
    }

    fn plain(to: &str, body: &str) -> CliArgs {
        CliArgs {
            to: to.to_string(),
            body: body.to_string(),
            request: false,
            reply_by: None,
            reply_to: None,
        }
    }

    // ── 요청 본문 조립(escape) ─────────────────────────────────────────────────────
    #[test]
    fn build_request_body_escapes() {
        let b = build_request_body(&plain("bob", "line1\n\"quoted\""));
        let v: serde_json::Value = serde_json::from_str(&b).expect("valid json");
        assert_eq!(v["to"], "bob");
        assert_eq!(v["body"], "line1\n\"quoted\"");
        assert!(v.get("from").is_none(), "요청에 from 필드가 없어야");
    }

    // ── exit code 매핑 ─────────────────────────────────────────────────────────────
    #[test]
    fn exit_code_delivered_2xx_is_zero() {
        assert_eq!(
            exit_code_for_response(
                200,
                r#"{"id":"x","results":[{"to":"bob","status":"delivered"}]}"#
            ),
            0
        );
    }

    #[test]
    fn exit_code_pending_2xx_is_zero() {
        assert_eq!(
            exit_code_for_response(
                200,
                r#"{"id":"x","results":[{"to":"ghost","status":"pending","hint":"parked"}]}"#
            ),
            0
        );
    }

    #[test]
    fn exit_code_error_is_one() {
        assert_eq!(
            exit_code_for_response(
                200,
                r#"{"status":"error","code":"MAILBOX_FULL","hint":"h"}"#
            ),
            1
        );
    }

    #[test]
    fn exit_code_malformed_is_one() {
        assert_eq!(exit_code_for_response(200, "not json"), 1);
    }

    // ── 성공 shape 완전 검증(리뷰 fix 13) ────────────────────────────────────────────
    #[test]
    fn exit_code_empty_results_is_malformed_not_success() {
        assert_eq!(
            exit_code_for_response(200, r#"{"id":"m-1","results":[]}"#),
            EXIT_MALFORMED_SUCCESS
        );
    }

    #[test]
    fn exit_code_missing_or_empty_id_is_malformed() {
        assert_eq!(
            exit_code_for_response(200, r#"{"results":[{"to":"bob","status":"delivered"}]}"#),
            EXIT_MALFORMED_SUCCESS,
            "id 없음"
        );
        assert_eq!(
            exit_code_for_response(
                200,
                r#"{"id":"","results":[{"to":"bob","status":"delivered"}]}"#
            ),
            EXIT_MALFORMED_SUCCESS,
            "id 빈 문자열"
        );
        assert_eq!(
            exit_code_for_response(
                200,
                r#"{"id":7,"results":[{"to":"bob","status":"delivered"}]}"#
            ),
            EXIT_MALFORMED_SUCCESS,
            "id 가 문자열이 아님"
        );
    }

    #[test]
    fn exit_code_bad_result_entry_is_malformed() {
        for body in [
            r#"{"id":"m-1","results":[{"to":"bob","status":"exploded"}]}"#,
            r#"{"id":"m-1","results":[{"to":"","status":"delivered"}]}"#,
            r#"{"id":"m-1","results":[{"status":"delivered"}]}"#,
            r#"{"id":"m-1","results":[{"to":"bob"}]}"#,
            r#"{"id":"m-1","results":[{"to":null,"status":"delivered"}]}"#,
            r#"{"id":"m-1","results":["bob"]}"#,
        ] {
            assert_eq!(
                exit_code_for_response(200, body),
                EXIT_MALFORMED_SUCCESS,
                "성공 shape 위반은 2: {body}"
            );
        }
        assert_eq!(
            exit_code_for_response(
                200,
                r#"{"id":"m-1","results":[{"to":"bob","status":"delivered"},{"to":"amy","status":"?"}]}"#
            ),
            EXIT_MALFORMED_SUCCESS
        );
    }

    #[test]
    fn exit_code_results_not_array_is_malformed() {
        assert_eq!(
            exit_code_for_response(200, r#"{"id":"m-1","results":{"to":"bob"}}"#),
            EXIT_MALFORMED_SUCCESS
        );
    }

    #[test]
    fn exit_code_failed_row_is_still_an_accepted_send() {
        // ★부분 진행(ADR-0111 결정 3)★
        assert_eq!(
            exit_code_for_response(
                200,
                r#"{"id":"m-1","results":[{"to":"bob","status":"delivered"},{"to":"dead","status":"failed","code":"RECIPIENT_NOT_FOUND"}]}"#
            ),
            0
        );
        assert_eq!(
            exit_code_for_response(
                200,
                r#"{"id":"m-1","results":[{"to":"dead","status":"failed","code":"RECIPIENT_NOT_FOUND"}]}"#
            ),
            0
        );
        // 폐지된 어휘(`skipped`)는 이제 shape 위반이다 — 조용히 통과시키면 옛 데몬과의 불일치를 못 본다.
        assert_eq!(
            exit_code_for_response(
                200,
                r#"{"id":"m-1","results":[{"to":"dead","status":"skipped"}]}"#
            ),
            2
        );
    }

    #[test]
    fn exit_code_error_shape_stays_plain_failure_not_malformed() {
        assert_eq!(
            exit_code_for_response(
                200,
                r#"{"status":"error","code":"REQUEST_CAPACITY","hint":"h"}"#
            ),
            EXIT_FAILED
        );
        assert_eq!(
            exit_code_for_response(200, r#"{"status":"error","code":"MAILBOX_FULL"}"#),
            EXIT_FAILED
        );
    }

    #[test]
    fn send_exit_code_never_reports_success_for_an_error_status_even_with_results() {
        let cases: [(&str, i32); 7] = [
            (
                r#"{"status":"error","code":"MAILBOX_FULL","hint":"h","id":"m-1","results":[{"to":"bob","status":"delivered"}]}"#,
                EXIT_FAILED,
            ),
            // 옛 코드는 이 혼종에 2 를 냈다(반려를 성공 shape 결함으로 오분류).
            (
                r#"{"status":"error","code":"MAILBOX_FULL","hint":"h","results":[{"to":"bob","status":"delivered"}]}"#,
                EXIT_FAILED,
            ),
            (
                r#"{"status":"error","results":[{"to":"bob","status":"delivered"}]}"#,
                EXIT_MALFORMED_SUCCESS,
            ),
            (
                r#"{"status":"error","code":"","results":[{"to":"bob","status":"delivered"}]}"#,
                EXIT_MALFORMED_SUCCESS,
            ),
            (
                r#"{"status":"error","code":"BODY_TOO_LARGE","hint":"h"}"#,
                EXIT_FAILED,
            ),
            (r#"{"status":"error"}"#, EXIT_MALFORMED_SUCCESS),
            (
                r#"{"id":"m-1","results":[{"to":"bob","status":"delivered"}]}"#,
                0,
            ),
        ];
        for (body, want) in cases {
            let got = exit_code_for_response(200, body);
            assert_eq!(got, want, "body={body}");
            if body.contains(r#""status":"error""#) {
                assert_ne!(
                    got, 0,
                    "status:\"error\" 는 어떤 형태든 성공일 수 없다: {body}"
                );
            }
        }
    }

    #[test]
    fn exit_code_2xx_that_is_neither_success_nor_valid_error_is_malformed() {
        for body in [
            r#"{}"#,
            r#"{"id":"m-x"}"#,
            r#"{"status":"error"}"#,
            r#"{"status":"error","code":""}"#,
            r#"{"status":"error","code":7}"#,
            r#"{"code":"MAILBOX_FULL"}"#,
            r#"{"status":"ok","code":"MAILBOX_FULL"}"#,
            r#"{"error":"boom"}"#,
        ] {
            assert_eq!(
                exit_code_for_response(200, body),
                EXIT_MALFORMED_SUCCESS,
                "성공도 반려도 아닌 2xx body 는 2: {body}"
            );
        }
    }

    #[test]
    fn exit_code_malformed_classification_does_not_leak_into_non_2xx() {
        for body in [r#"{}"#, r#"{"id":"m-x"}"#, r#"{"status":"error"}"#, ""] {
            assert_eq!(
                exit_code_for_response(503, body),
                EXIT_FAILED,
                "비-2xx 는 항상 1: {body}"
            );
        }
    }

    #[test]
    fn exit_code_non_2xx_is_one_even_if_body_looks_ok() {
        assert_eq!(
            exit_code_for_response(
                500,
                r#"{"id":"x","results":[{"to":"bob","status":"delivered"}]}"#
            ),
            1
        );
        assert_eq!(exit_code_for_response(401, ""), 1);
    }

    // ── URL 파싱 ──────────────────────────────────────────────────────────────────
    #[test]
    fn parse_host_port_ok() {
        let (h, p) = parse_host_port("http://127.0.0.1:54321").expect("parse");
        assert_eq!(h, "127.0.0.1");
        assert_eq!(p, 54321);
    }

    #[test]
    fn parse_host_port_strips_path() {
        let (h, p) = parse_host_port("http://127.0.0.1:8080/extra").expect("parse");
        assert_eq!(h, "127.0.0.1");
        assert_eq!(p, 8080);
    }

    #[test]
    fn parse_host_port_rejects_non_http() {
        assert!(parse_host_port("https://127.0.0.1:1").is_err());
        assert!(parse_host_port("127.0.0.1:1").is_err());
    }

    #[test]
    fn base_path_empty_for_bare_authority() {
        assert_eq!(base_path("http://127.0.0.1:1"), "");
        assert_eq!(base_path("http://127.0.0.1:1/sub"), "/sub");
    }

    // ── HTTP 응답 파싱(F4: status·헤더·프레이밍) ─────────────────────────────────────
    #[test]
    fn parse_response_content_length_body() {
        let body = "{\"id\":\"m1\",\"results\":[]}";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}EXTRA-GARBAGE",
            body.len(),
            body
        );
        let r = parse_response(resp.as_bytes()).expect("parse");
        assert_eq!(r.status, 200);
        assert_eq!(r.body, body, "Content-Length 만큼만 취해 잔재 제외");
    }

    #[test]
    fn parse_response_short_body_is_incomplete() {
        let partial = r#"{"id":"m1","results":[]}"#;
        assert!(partial.len() < 100, "테스트 전제: partial < 100");
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{partial}"
        );
        let err = parse_response(resp.as_bytes()).expect_err("절단 body 는 에러여야");
        assert_eq!(err.code(), "INCOMPLETE_RESPONSE", "short read 코드");
        match err {
            SendError::Incomplete { received, expected } => {
                assert_eq!(received, partial.len(), "수신 바이트 수");
                assert_eq!(expected, 100, "선언된 Content-Length");
            }
            other => panic!("Incomplete 여야: {other:?}"),
        }
    }

    #[test]
    fn parse_response_case_insensitive_headers() {
        let body = "{\"id\":\"m1\",\"results\":[]}";
        let resp = format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let r = parse_response(resp.as_bytes()).expect("parse");
        assert_eq!(r.body, body);
    }

    #[test]
    fn parse_response_chunked_body() {
        let resp = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                    c\r\n{\"id\":\"m1\",\"\r\n\
                    c\r\nresults\":[]}\r\n\
                    0\r\n\r\n";
        let r = parse_response(resp.as_bytes()).expect("parse chunked");
        assert_eq!(r.status, 200);
        assert_eq!(r.body, "{\"id\":\"m1\",\"results\":[]}", "chunked de-frame");
    }

    #[test]
    fn parse_response_read_to_eof_fallback() {
        let resp =
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"id\":\"m1\",\"results\":[]}";
        let r = parse_response(resp.as_bytes()).expect("parse");
        assert_eq!(r.status, 200);
        assert_eq!(r.body, "{\"id\":\"m1\",\"results\":[]}");
    }

    #[test]
    fn parse_response_non_2xx_status() {
        let resp = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n";
        let r = parse_response(resp.as_bytes()).expect("parse");
        assert_eq!(r.status, 401);
        assert_eq!(r.body, "");
    }

    #[test]
    fn parse_response_no_separator_errs() {
        assert!(
            parse_response(b"HTTP/1.1 200 OK").is_err(),
            "경계 없으면 에러"
        );
    }

    #[test]
    fn dechunk_handles_extension_and_multiple_chunks() {
        let raw = "3;ext=1\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        assert_eq!(dechunk(raw.as_bytes()).expect("dechunk"), "abcde");
    }

    // ── D: `--body-stdin`(인용 지옥 회피 — 표면 정본 = ADR-0132) ──────────────────────────
    #[test]
    fn body_stdin_reads_the_body_from_the_injected_reader() {
        let parsed = parse_mail_command(&argv(&["mail", "send", "--to", "bob", "--body-stdin"]))
            .expect("parse");
        let cmd = materialize_body(parsed, || Ok("line1\n\"quoted\" & $stuff\n".to_string()))
            .expect("materialize");
        let Command::Send(a) = cmd else {
            panic!("발송이어야")
        };
        assert_eq!(a.to, "bob");
        assert_eq!(
            a.body, "line1\n\"quoted\" & $stuff\n",
            "본문은 trim 하지 않는다(heredoc 마지막 개행 포함)"
        );
        let v: serde_json::Value = serde_json::from_str(&build_request_body(&a)).expect("json");
        assert_eq!(v["body"], "line1\n\"quoted\" & $stuff\n");
        assert!(
            v.get("body_stdin").is_none(),
            "wire 에 CLI 전용 플래그 없음"
        );
    }

    #[test]
    fn body_and_body_stdin_are_mutually_exclusive() {
        let err = parse_command(&argv(&[
            "mail",
            "send",
            "--to",
            "bob",
            "--body",
            "hi",
            "--body-stdin",
        ]))
        .expect_err("상호배타 반려");
        assert!(err.contains("mutually exclusive"), "사유 문구: {err}");
    }

    #[test]
    fn body_stdin_with_no_input_is_rejected_not_sent_empty() {
        let parsed = parse_mail_command(&argv(&["mail", "send", "--to", "bob", "--body-stdin"]))
            .expect("parse");
        let err = materialize_body(parsed, || Ok(String::new())).expect_err("빈 stdin 반려");
        assert!(err.contains("no input"), "사유 문구: {err}");
    }

    #[test]
    fn missing_body_mentions_both_ways_to_supply_it() {
        let err = parse_command(&argv(&["mail", "send", "--to", "bob"])).expect_err("본문 누락");
        assert!(err.contains("--body-stdin"), "두 경로를 모두 안내: {err}");
    }

    // ── D: 우편 동사 파싱(send / status / pending) ─────────────────────────────────────

    fn wire(args: &[&str]) -> (&'static str, serde_json::Value) {
        let parsed = parse_mail_command(&argv(args)).expect("parse");
        let cmd = materialize_body(parsed, || panic!("이 경로는 stdin 을 읽지 않는다"))
            .expect("materialize");
        let route = cmd.route();
        let body: serde_json::Value =
            serde_json::from_str(&cmd.request_body()).expect("valid json");
        (route, body)
    }

    #[test]
    fn status_and_pending_hit_the_messages_route() {
        assert_eq!(
            wire(&["mail", "status", "m-7f3k9q2d"]),
            (
                "/control/messages",
                serde_json::json!({ "id": "m-7f3k9q2d" })
            )
        );
        assert_eq!(
            wire(&["mail", "pending"]),
            ("/control/messages", serde_json::json!({}))
        );
    }

    #[test]
    fn send_hits_the_send_route() {
        let (route, body) = wire(&["mail", "send", "--to", "bob", "--body", "hi"]);
        assert_eq!(route, "/control/send");
        assert_eq!(body, serde_json::json!({ "to": "bob", "body": "hi" }));
        let (route, body) = wire(&["mail", "send", "--to", "@coders", "--body", "공지"]);
        assert_eq!(route, "/control/send");
        assert_eq!(body["to"], "@coders");
    }

    #[test]
    fn subcommand_shape_errors_are_rejected_at_the_cli() {
        for args in [
            vec!["mail"],
            vec!["mail", "status"],
            vec!["mail", "status", "m-1", "extra"],
            vec!["mail", "pending", "extra"],
            vec!["mail", "send"],
            // 회귀 가드(ADR-0111 결정 4 · ADR-0112 결정 1) — group 동사가 부활하면 프라이밍이 없는
            //   명령을 가르치게 된다. 계열 자리(옛 표기)와 우편 동사 자리 양쪽에서 막는다.
            vec!["group", "list"],
            vec!["group", "update", "@t", "--add", "a"],
            vec!["group", "delete", "@t"],
            vec!["mail", "group"],
            vec!["mail", "group", "list"],
        ] {
            assert!(
                parse_command(&argv(&args)).is_err(),
                "형태 오류여야: {args:?}"
            );
        }
    }

    // ── 계열·동사 표기 ────────────────────────────────────────────────────────────────

    #[test]
    fn a_flag_first_invocation_is_an_argument_error() {
        // 기본 동사가 없으므로 계열 없는 플래그 호출은 발송으로 흐르지 않는다.
        for args in [
            vec!["--to", "bob", "--body", "hi"],
            vec!["--body-stdin"],
            vec!["send", "--to", "bob", "--body", "hi"],
            vec!["status", "m-1"],
            vec!["pending"],
        ] {
            let err = match parse_command(&argv(&args)) {
                Err(e) => e,
                Ok(cmd) => panic!("계열 없는 호출은 인자 오류여야: {args:?} → {cmd:?}"),
            };
            assert!(
                err.contains(CLI_EXE_NAME) && err.contains("help"),
                "반려 사유가 help 로 안내해야: {err}"
            );
        }
    }

    #[test]
    fn repeated_value_flags_are_rejected_instead_of_silently_replacing() {
        // 예전엔 두 번째 값이 첫 번째를 말없이 덮었다 — argv 를 이어 붙이는 호출자에겐 무신호 유실이다.
        for args in [
            vec![
                "mail", "send", "--to", "alice", "--body", "first", "--body", "second",
            ],
            vec![
                "mail", "send", "--to", "alice", "--to", "bob", "--body", "hi",
            ],
            vec![
                "mail",
                "send",
                "--to",
                "a",
                "--body",
                "hi",
                "--reply-by",
                "5m",
                "--reply-by",
                "10m",
            ],
            vec![
                "mail",
                "send",
                "--to",
                "a",
                "--body",
                "hi",
                "--reply-to",
                "m-1",
                "--reply-to",
                "m-2",
            ],
        ] {
            let err = match parse_command(&argv(&args)) {
                Err(e) => e,
                Ok(cmd) => panic!("중복 값 플래그는 반려여야({args:?}): {cmd:?}"),
            };
            assert!(err.contains("more than once"), "사유 문구({args:?}): {err}");
        }
        // 불리언 반복은 버려지는 값이 없으므로 그대로 통과한다.
        let (_, body) = wire(&[
            "mail",
            "send",
            "--to",
            "a",
            "--body",
            "hi",
            "--request",
            "--request",
        ]);
        assert_eq!(body["request"], true);
    }

    /// ★값 자리에 아는 플래그가 오면 "값을 빠뜨린 것"★ — 예전엔 그걸 본문 문자열로 삼켜, 파이프로 들어온
    ///   진짜 본문이 사라지고 리터럴 `--body-stdin` 이 배달됐다(그리고 exit 0).
    #[test]
    fn a_value_flag_followed_by_another_known_flag_is_rejected_not_swallowed() {
        for args in [
            vec!["mail", "send", "--to", "bob", "--body", "--body-stdin"],
            vec!["mail", "send", "--to", "--body", "hi"],
            vec![
                "mail",
                "send",
                "--to",
                "bob",
                "--body",
                "hi",
                "--reply-by",
                "--request",
            ],
            vec![
                "mail",
                "send",
                "--to",
                "bob",
                "--body",
                "hi",
                "--reply-to",
                "--to",
            ],
        ] {
            let err = match parse_command(&argv(&args)) {
                Err(e) => e,
                Ok(cmd) => panic!("빠뜨린 값은 반려여야({args:?}): {cmd:?}"),
            };
            assert!(err.contains("has no value"), "사유 문구({args:?}): {err}");
        }
    }

    /// 반대 방향 — 값은 임의 텍스트다. `-` 로 시작한다는 이유로 가로채면 정당한 본문이 사라진다.
    #[test]
    fn values_that_merely_look_like_flags_are_still_sent_verbatim() {
        for value in ["-h", "--help", "--not-a-flag-we-know", "-", "--"] {
            let (_, body) = wire(&["mail", "send", "--to", "bob", "--body", value]);
            assert_eq!(body["body"], value, "본문은 그대로 실린다: {value:?}");
        }
        let (_, body) = wire(&["mail", "send", "--to", "--not-a-flag", "--body", "hi"]);
        assert_eq!(body["to"], "--not-a-flag");
    }

    #[test]
    fn every_value_flag_reports_a_missing_value_at_end_of_args() {
        for flag in ["--to", "--body", "--reply-by", "--reply-to"] {
            let err = match parse_command(&argv(&["mail", "send", flag])) {
                Err(e) => e,
                Ok(cmd) => panic!("값 없는 {flag} 는 반려여야: {cmd:?}"),
            };
            assert!(err.contains("requires a value"), "{flag}: {err}");
        }
    }

    /// ★공용 어휘 ↔ 파서 드리프트 가드★: core 의 목록에 플래그·동사를 더하고 파서 arm 을 안 고치면 값 자리
    ///   방어와 프라이밍 판정이 그 새 표기를 못 본다 — 여기서 잡는다.
    #[test]
    fn the_parser_recognises_every_flag_and_verb_in_the_shared_vocabulary() {
        for flag in CLI_MAIL_FLAGS {
            let err = parse_command(&argv(&["mail", "send", flag])).err();
            assert!(
                !err.as_deref()
                    .unwrap_or_default()
                    .contains("unknown argument"),
                "파서가 모르는 플래그가 공용 목록에 있다: {flag}"
            );
        }
        for verb in CLI_MAIL_VERBS {
            let err = parse_command(&argv(&["mail", verb]))
                .err()
                .unwrap_or_default();
            assert!(
                !err.contains("unknown"),
                "파서가 모르는 동사가 공용 목록에 있다: {verb} ({err})"
            );
        }
        for verb in CLI_AGENT_VERBS {
            let err = parse_command(&argv(&[CLI_GROUP_AGENT, verb]))
                .err()
                .unwrap_or_default();
            assert!(
                !err.contains("unknown"),
                "파서가 모르는 동사가 공용 목록에 있다: {verb} ({err})"
            );
        }
        // 제어 플래그는 동사마다 쓰이는 자리가 달라(`--parent` 는 move 전용) 계열의 **어느 동사에선가**
        //   인식되면 통과로 본다 — 어디서도 안 알려지는 플래그만 잡는다.
        for flag in CLI_AGENT_FLAGS {
            let recognised = CLI_AGENT_VERBS.iter().any(|verb| {
                !parse_command(&argv(&[CLI_GROUP_AGENT, verb, flag]))
                    .err()
                    .unwrap_or_default()
                    .contains("unknown argument")
            });
            assert!(
                recognised,
                "파서가 어느 동사에서도 모르는 플래그가 공용 목록에 있다: {flag}"
            );
        }
    }

    // ── ADR-0132 조각 ②: 제어 계열(`agent`) ──────────────────────────────────────────

    fn parse_agent_command(args: &[&str]) -> Result<ParsedAgent, String> {
        match parse_command(&argv(args))? {
            ParsedCommand::Agent(a) => Ok(a),
            other => Err(format!("제어 커맨드가 아니다: {other:?}")),
        }
    }

    /// 동사별 wire(라우트 + 바디) — 계열 전체가 라우트 하나를 탄다.
    fn agent_wire(args: &[&str]) -> (&'static str, serde_json::Value) {
        let cmd = Command::Agent(parse_agent_command(args).expect("parse"));
        let body: serde_json::Value =
            serde_json::from_str(&cmd.request_body()).expect("valid json");
        (cmd.route(), body)
    }

    #[test]
    fn every_agent_verb_posts_to_the_one_control_route_with_the_verb_in_the_body() {
        let cases: [(&[&str], serde_json::Value); 6] = [
            (&["agent", "list"], serde_json::json!({ "verb": "list" })),
            (
                &["agent", "spawn", "qa-bravo"],
                serde_json::json!({ "verb": "spawn", "target": "qa-bravo" }),
            ),
            (
                &["agent", "spawn", "--cwd", "C:/work", "--name", "qa"],
                serde_json::json!({ "verb": "spawn", "cwd": "C:/work", "name": "qa" }),
            ),
            (
                &["agent", "new", "--cwd", "C:/work"],
                serde_json::json!({ "verb": "new", "cwd": "C:/work" }),
            ),
            (
                &["agent", "rename", "qa-bravo", "qa-lead"],
                serde_json::json!({ "verb": "rename", "target": "qa-bravo", "name": "qa-lead" }),
            ),
            (
                &["agent", "move", "qa-bravo", "--parent", "lead"],
                serde_json::json!({ "verb": "move", "target": "qa-bravo", "parent": "lead" }),
            ),
        ];
        for (args, want) in cases {
            let (route, body) = agent_wire(args);
            assert_eq!(route, "/control/agent", "{args:?}");
            assert_eq!(body, want, "{args:?}");
        }
    }

    #[test]
    fn detaching_sends_an_explicit_null_parent_not_an_absent_key() {
        // 부재로 표현하면 "부모를 안 줬다" 와 "루트로 떼라" 가 wire 에서 같은 모양이 된다.
        let (_, body) = agent_wire(&["agent", "move", "qa-bravo", "--parent", "none"]);
        assert!(body.get("parent").is_some(), "키 자체는 실린다: {body}");
        assert!(body["parent"].is_null(), "값은 null: {body}");
    }

    #[test]
    fn spawn_rejects_both_forms_and_neither_before_touching_the_network() {
        for args in [
            vec!["agent", "spawn", "qa-bravo", "--cwd", "C:/work"],
            vec!["agent", "spawn", "--cwd", "C:/work", "qa-bravo"],
        ] {
            let err = parse_agent_command(&args).expect_err("양립 불가");
            assert!(err.contains("not both"), "사유 문구({args:?}): {err}");
            assert!(err.contains("help"), "복구 경로({args:?}): {err}");
        }
        let err = parse_agent_command(&["agent", "spawn"]).expect_err("둘 다 없음");
        assert!(err.contains("either"), "사유 문구: {err}");
    }

    #[test]
    fn waking_an_agent_rejects_a_name_flag_instead_of_ignoring_it() {
        let err = parse_agent_command(&["agent", "spawn", "qa-bravo", "--name", "qa-lead"])
            .expect_err("깨우기엔 --name 이 없다");
        assert!(err.contains("rename"), "개명 동사로 안내: {err}");
    }

    #[test]
    fn agent_shape_errors_are_rejected_at_the_cli() {
        for args in [
            vec!["agent"],
            vec!["agent", "wat"],
            vec!["agent", "list", "extra"],
            vec!["agent", "new"],
            vec!["agent", "new", "C:/work"],
            vec!["agent", "rename"],
            vec!["agent", "rename", "only-one"],
            vec!["agent", "rename", "a", "b", "c"],
            vec!["agent", "move", "qa-bravo"],
            vec!["agent", "move", "--parent", "lead"],
            vec!["agent", "spawn", "a", "b"],
            // 값 자리에 아는 플래그 — 우편 쪽과 같은 사고(값을 빠뜨린 것을 값으로 삼킴).
            vec!["agent", "new", "--cwd", "--name"],
            vec!["agent", "move", "a", "--parent", "--cwd"],
            // 중복 값 플래그.
            vec!["agent", "new", "--cwd", "a", "--cwd", "b"],
            vec!["agent", "move", "a", "--parent", "x", "--parent", "y"],
            // 위치 인자 자리의 help 토큰 — 네트워크를 타면 안 된다.
            vec!["agent", "spawn", "--help"],
            vec!["agent", "rename", "-h", "x"],
            vec!["agent", "move", "--help", "--parent", "none"],
            // 모르는 플래그.
            vec!["agent", "list", "--json"],
            vec!["agent", "new", "--cwd", "a", "--to", "bob"],
        ] {
            let err = match parse_command(&argv(&args)) {
                Err(e) => e,
                Ok(cmd) => panic!("형태 오류여야({args:?}): {cmd:?}"),
            };
            assert!(
                err.contains("help"),
                "복구 경로를 안내해야({args:?}): {err}"
            );
        }
    }

    #[test]
    fn empty_values_are_argument_errors_not_silently_absent_fields() {
        // 셸의 미설정 변수는 빈 인자로 펼쳐진다 — 그걸 "안 준 것" 으로 접으면 `move --parent "$UNSET"` 가
        //   계층 해제로 실행된다(그게 --parent 를 필수로 둔 이유 자체를 무력화한다).
        for args in [
            vec!["agent", "move", "helper", "--parent", ""],
            vec!["agent", "move", "helper", "--parent", "   "],
            vec!["agent", "new", "--cwd", ""],
            vec!["agent", "new", "--cwd", "C:/x", "--name", ""],
            vec!["agent", "spawn", ""],
            vec!["agent", "rename", "a", ""],
            vec!["agent", "rename", "", "b"],
        ] {
            let err = match parse_command(&argv(&args)) {
                Err(e) => e,
                Ok(cmd) => panic!("빈 값은 인자 오류여야({args:?}): {cmd:?}"),
            };
            assert!(err.contains("empty"), "사유 문구({args:?}): {err}");
        }
        // 대조군 — 값을 준 같은 명령은 통과한다(위 반려가 "이 형태 자체가 막힌 것" 이 아님을 못박는다).
        let (_, body) = agent_wire(&["agent", "move", "helper", "--parent", "lead"]);
        assert_eq!(body["parent"], "lead");
    }

    // ── 제어 응답 exit code 매핑 ──────────────────────────────────────────────────────

    fn agent_of(args: &[&str]) -> ParsedAgent {
        parse_agent_command(args).expect("parse")
    }

    #[test]
    fn agent_exit_code_accepts_only_payloads_that_carry_the_evidence() {
        let cases: [(&[&str], &str, i32); 16] = [
            (&["agent", "list"], r#"{"agents":[]}"#, 0),
            (
                &["agent", "list"],
                r#"{"agents":[{"id":"i","name":"n","state":"live","cwd":"c","parent":null}]}"#,
                0,
            ),
            // 상태 어휘 밖 값 = 우리가 아는 응답이 아니다.
            (
                &["agent", "list"],
                r#"{"agents":[{"id":"i","name":"n","state":"zombie"}]}"#,
                EXIT_MALFORMED_SUCCESS,
            ),
            (&["agent", "list"], r#"{}"#, EXIT_MALFORMED_SUCCESS),
            (
                &["agent", "spawn", "w"],
                r#"{"agent_id":"i","name":"w","state":"live","created":false}"#,
                0,
            ),
            // created 누락 = 깨운 건지 만든 건지 모른다.
            (
                &["agent", "spawn", "w"],
                r#"{"agent_id":"i","name":"w","state":"live"}"#,
                EXIT_MALFORMED_SUCCESS,
            ),
            (
                &["agent", "new", "--cwd", "c"],
                r#"{"agent_id":"i","name":"n","state":"sleeping"}"#,
                0,
            ),
            // 신원 자리가 반만 답해졌다.
            (
                &["agent", "new", "--cwd", "c"],
                r#"{"agent_id":"i","state":"sleeping"}"#,
                EXIT_MALFORMED_SUCCESS,
            ),
            // 만들었다면서 무엇을 만들었는지가 없다 — 예전 판정기는 이걸 exit 0 으로 흘렸다.
            (
                &["agent", "new", "--cwd", "c"],
                r#"{"status":"ok"}"#,
                EXIT_MALFORMED_SUCCESS,
            ),
            (
                &["agent", "rename", "a", "b"],
                r#"{"agent_id":"i","name":"b","outcome":"renamed"}"#,
                0,
            ),
            (
                &["agent", "rename", "a", "b"],
                r#"{"agent_id":"i","name":"b","outcome":"maybe"}"#,
                EXIT_MALFORMED_SUCCESS,
            ),
            (
                &["agent", "move", "a", "--parent", "none"],
                r#"{"agent_id":"i","name":"a","parent":null}"#,
                0,
            ),
            // parent 키 자체가 없으면 어디로 갔는지 모른다.
            (
                &["agent", "move", "a", "--parent", "none"],
                r#"{"agent_id":"i","name":"a"}"#,
                EXIT_MALFORMED_SUCCESS,
            ),
            (
                &["agent", "move", "a", "--parent", "none"],
                r#"{"agent_id":"","name":"a","parent":null}"#,
                EXIT_MALFORMED_SUCCESS,
            ),
            // 신원을 `agent` 밑에 넣은 body 는 최상위가 비어 있는 것과 같다 — 이 줄이 중첩 갈래의 부활을 막는다.
            (
                &["agent", "spawn", "w"],
                r#"{"agent":{"id":"i","name":"w","state":"live"},"created":false}"#,
                EXIT_MALFORMED_SUCCESS,
            ),
            // 혼종 — 신원을 두 자리에 나눠 실은 body. 위 줄과 잡는 실수가 다르다: 위는 중첩 갈래의 부활을,
            //   이 줄은 id 는 중첩에서 이름은 최상위에서 집는 **병합** reader 를 막는다.
            (
                &["agent", "spawn", "w"],
                r#"{"agent":{"id":"i"},"name":"w","state":"live","created":false}"#,
                EXIT_MALFORMED_SUCCESS,
            ),
        ];
        for (args, body, want) in cases {
            assert_eq!(
                exit_code_for_agent_response(&agent_of(args), 200, body),
                want,
                "{args:?} ← {body}"
            );
        }
    }

    /// ★필드가 다 있어도 **요청한 동작과 어긋나면** 성공이 아니다★ — 호출자가 일어나지 않은 일을 사실로
    ///   기록하는 것을 막는 축이다.
    #[test]
    fn agent_exit_code_rejects_evidence_that_contradicts_the_request() {
        let cases: [(&[&str], &str, i32); 6] = [
            // 깨우기인데 "만들었다" 고 답한다.
            (
                &["agent", "spawn", "worker"],
                r#"{"agent_id":"i","name":"worker","state":"live","created":true}"#,
                EXIT_MALFORMED_SUCCESS,
            ),
            // 만들어 띄우라 했는데 "깨웠다" 고 답한다.
            (
                &["agent", "spawn", "--cwd", "C:/x"],
                r#"{"agent_id":"i","name":"x","state":"live","created":false}"#,
                EXIT_MALFORMED_SUCCESS,
            ),
            // 루트로 떼라 했는데 부모를 달고 온다.
            (
                &["agent", "move", "a", "--parent", "none"],
                r#"{"agent_id":"i","name":"a","parent":"p-id"}"#,
                EXIT_MALFORMED_SUCCESS,
            ),
            // 부모 밑으로 넣으라 했는데 루트라고 답한다.
            (
                &["agent", "move", "a", "--parent", "lead"],
                r#"{"agent_id":"i","name":"a","parent":null}"#,
                EXIT_MALFORMED_SUCCESS,
            ),
            // `new` 는 아무것도 띄우지 않는다 — 살아 있다는 답은 그 동사의 결과일 수 없다.
            (
                &["agent", "new", "--cwd", "C:/x"],
                r#"{"agent_id":"i","name":"x","state":"live"}"#,
                EXIT_MALFORMED_SUCCESS,
            ),
            // 대조군 — 요청과 맞는 응답은 그대로 통과한다.
            (
                &["agent", "move", "a", "--parent", "lead"],
                r#"{"agent_id":"i","name":"a","parent":"p-id"}"#,
                0,
            ),
        ];
        for (args, body, want) in cases {
            assert_eq!(
                exit_code_for_agent_response(&agent_of(args), 200, body),
                want,
                "{args:?} ← {body}"
            );
        }
    }

    /// ★reader 가 "필드가 다 있으면 통과" 로 무너지지 않았나★ — 신원(`agent_id`·`name`·`state`)만 완전한
    ///   응답은 어느 변경 동사에서도 성공이 아니다. 동사마다 요구하는 대조 필드가 따로 있다.
    #[test]
    fn a_payload_without_the_cross_check_still_exits_two() {
        let identity_only = r#"{"agent_id":"i","name":"n","state":"live"}"#;
        for args in [
            &["agent", "spawn", "n"][..],
            &["agent", "new", "--cwd", "c"][..],
            &["agent", "rename", "a", "n"][..],
            &["agent", "move", "a", "--parent", "lead"][..],
        ] {
            assert_eq!(
                exit_code_for_agent_response(&agent_of(args), 200, identity_only),
                EXIT_MALFORMED_SUCCESS,
                "신원만으론 부족하다({args:?}): {identity_only}"
            );
        }
    }

    /// `list` 행은 help 가 약속하는 다섯 필드를 다 싣는다 — 약속과 검사가 갈리면 둘 중 하나가 거짓말이다.
    #[test]
    fn agent_exit_code_requires_every_list_field_the_help_screen_promises() {
        let full =
            r#"{"agents":[{"id":"i","name":"n","state":"live","cwd":"C:/x","parent":null}]}"#;
        assert_eq!(
            exit_code_for_agent_response(&agent_of(&["agent", "list"]), 200, full),
            0
        );
        for missing in [
            r#"{"agents":[{"name":"n","state":"live","cwd":"C:/x","parent":null}]}"#,
            r#"{"agents":[{"id":"i","state":"live","cwd":"C:/x","parent":null}]}"#,
            r#"{"agents":[{"id":"i","name":"n","cwd":"C:/x","parent":null}]}"#,
            r#"{"agents":[{"id":"i","name":"n","state":"live","parent":null}]}"#,
            r#"{"agents":[{"id":"i","name":"n","state":"live","cwd":"C:/x"}]}"#,
        ] {
            assert_eq!(
                exit_code_for_agent_response(&agent_of(&["agent", "list"]), 200, missing),
                EXIT_MALFORMED_SUCCESS,
                "help 가 약속한 필드가 빠졌다: {missing}"
            );
        }
        // help 화면이 실제로 그 다섯을 약속하는지까지 여기서 묶는다(한쪽만 바뀌는 것을 막는다).
        let help = render_help(HelpTopic::Agent, MailSurface::Shown);
        for promised in ["id", "name", "state", "cwd", "parent"] {
            assert!(help.contains(promised), "{promised} 가 help 에: {help}");
        }
    }

    #[test]
    fn agent_exit_code_maps_rejections_and_transport_failures_to_one() {
        let list = agent_of(&["agent", "list"]);
        for body in [
            r#"{"status":"error","code":"AGENT_NOT_FOUND","hint":"h"}"#,
            r#"{"status":"error","code":"ROSTER_FULL","hint":"h"}"#,
        ] {
            assert_eq!(exit_code_for_agent_response(&list, 200, body), EXIT_FAILED);
        }
        assert_eq!(
            exit_code_for_agent_response(&list, 200, "not json"),
            EXIT_FAILED
        );
        assert_eq!(
            exit_code_for_agent_response(&list, 503, r#"{"agents":[]}"#),
            EXIT_FAILED,
            "비-2xx 는 body 가 멀쩡해도 1"
        );
    }

    #[test]
    fn agent_exit_code_never_reports_success_for_an_error_status() {
        let list = agent_of(&["agent", "list"]);
        for body in [
            r#"{"status":"error"}"#,
            r#"{"status":"error","code":""}"#,
            r#"{"status":"error","code":7}"#,
            r#"{"status":"error","agents":[]}"#,
        ] {
            let got = exit_code_for_agent_response(&list, 200, body);
            assert_eq!(got, EXIT_MALFORMED_SUCCESS, "{body}");
            assert_ne!(
                got, 0,
                "status:\"error\" 는 어떤 형태든 성공일 수 없다: {body}"
            );
        }
    }

    /// ★우편 판정기는 이 조각에서 바뀌지 않는다(T-19 는 그대로)★ — 제어를 엄격하게 만들면서 우편까지 함께
    ///   조인 것으로 오독되지 않게 못박는다.
    #[test]
    fn the_mail_query_judge_keeps_its_documented_permissiveness() {
        assert_eq!(exit_code_for_query_response(200, r#"{}"#), 0);
        assert_eq!(exit_code_for_query_response(200, r#"{"status":"ok"}"#), 0);
        // 같은 body 를 제어 판정기는 통과시키지 않는다 — 두 판정기가 실제로 다르다는 것이 요점이다.
        let new = agent_of(&["agent", "new", "--cwd", "c"]);
        assert_eq!(
            exit_code_for_agent_response(&new, 200, r#"{}"#),
            EXIT_MALFORMED_SUCCESS
        );
    }

    /// 우편 쪽 방어와 같은 반대 방향 — 값은 임의 텍스트다. 계열이 다른 플래그(`--to`)는 이 계열의 값 자리를
    /// 가로채지 않는다(경로·이름에 그런 문자열이 올 수 있다).
    #[test]
    fn agent_values_are_only_guarded_against_this_groups_flags() {
        let (_, body) = agent_wire(&["agent", "new", "--cwd", "--to"]);
        assert_eq!(body["cwd"], "--to", "다른 계열의 플래그는 그냥 값이다");
        let (_, body) = agent_wire(&["agent", "new", "--cwd", "C:/x", "--name", "-weird"]);
        assert_eq!(body["name"], "-weird");
    }

    #[test]
    fn a_help_token_is_not_a_help_topic() {
        for args in [
            vec!["help", "--help"],
            vec!["--help", "help"],
            vec!["help", "-h"],
            vec!["mail", "--help", "-h"],
        ] {
            assert!(
                parse_command(&argv(&args)).is_err(),
                "help 는 단독 호출일 때만 help 다: {args:?}"
            );
        }
    }

    #[test]
    fn unknown_group_and_unknown_verb_point_at_help() {
        for args in [vec!["wat"], vec!["mail", "wat"], vec!["help", "wat"]] {
            let err = parse_command(&argv(&args)).expect_err("모르는 이름은 인자 오류");
            assert!(err.contains("help"), "반려 사유가 help 로 안내해야: {err}");
        }
    }

    #[test]
    fn help_lists_groups_and_each_group_documents_its_verbs() {
        for args in [vec![], vec!["help"]] {
            match parse_command(&argv(&args)).expect("help 는 성공") {
                ParsedCommand::Help(t) => {
                    assert_eq!(t, HelpTopic::Root, "인자 없음·help = 계열 목록")
                }
                other => panic!("help 여야: {other:?}"),
            }
        }
        match parse_command(&argv(&["help", "mail"])).expect("help mail") {
            ParsedCommand::Help(t) => assert_eq!(t, HelpTopic::Mail),
            other => panic!("help 여야: {other:?}"),
        }
        assert!(parse_command(&argv(&["help", "mail", "extra"])).is_err());

        // 관례 철자 — 발견 입구가 가장 흔한 표기에서 실패하면 안 된다.
        for (args, want) in [
            (vec!["--help"], HelpTopic::Root),
            (vec!["-h"], HelpTopic::Root),
            (vec!["--help", "mail"], HelpTopic::Mail),
            (vec!["mail", "--help"], HelpTopic::Mail),
            (vec!["mail", "-h"], HelpTopic::Mail),
        ] {
            match parse_command(&argv(&args)).unwrap_or_else(|e| panic!("{args:?}: {e}")) {
                ParsedCommand::Help(t) => assert_eq!(t, want, "{args:?}"),
                other => panic!("help 여야({args:?}): {other:?}"),
            }
        }
        // ★값 자리의 help 토큰은 값이다★ — 발송이 조용히 help 로 새면 편지가 사라진다.
        let (route, body) = wire(&["mail", "send", "--to", "bob", "--body", "-h"]);
        assert_eq!(route, "/control/send");
        assert_eq!(body["body"], "-h", "본문 `-h` 는 그대로 실린다");
        let (_, body) = wire(&["mail", "send", "--to", "-h", "--body", "hi"]);
        assert_eq!(body["to"], "-h", "수신자 값도 가로채지 않는다");

        // ★위치 인자 자리의 help 토큰은 인자 오류다 — 네트워크를 타면 안 된다★: 그대로 실어 보내면
        //   `--help` 가 메시지 id 로 조회된다(실제로 왕복해 MESSAGE_NOT_FOUND 로 끝났다).
        for args in [
            vec!["mail", "status", "--help"],
            vec!["mail", "status", "-h"],
            vec!["mail", "pending", "--help"],
            vec!["mail", "send", "--help"],
        ] {
            let err = match parse_command(&argv(&args)) {
                Err(e) => e,
                Ok(cmd) => panic!("인자 오류여야({args:?}): {cmd:?}"),
            };
            assert!(
                err.contains("help"),
                "복구 경로를 안내해야({args:?}): {err}"
            );
        }

        // ★help + 추가 인자 = 인자 오류★: exit 0 으로 삼키면 보내려던 편지가 성공 코드와 함께 사라진다.
        for args in [
            vec!["mail", "--help", "--to", "bob", "--body", "hi"],
            vec!["mail", "-h", "send"],
            vec!["--help", "mail", "--to", "bob"],
            vec!["help", "mail", "--to", "bob"],
        ] {
            assert!(
                parse_command(&argv(&args)).is_err(),
                "help 에 붙은 잔여 인자는 오류여야: {args:?}"
            );
        }

        // 제어 계열도 같은 발견 규칙을 따른다(계열이 늘 때 이 규칙이 계열마다 갈리면 안 된다).
        for args in [
            vec!["help", CLI_GROUP_AGENT],
            vec![CLI_GROUP_AGENT, "--help"],
            vec![CLI_GROUP_AGENT, "-h"],
            vec!["--help", CLI_GROUP_AGENT],
        ] {
            match parse_command(&argv(&args)).unwrap_or_else(|e| panic!("{args:?}: {e}")) {
                ParsedCommand::Help(t) => assert_eq!(t, HelpTopic::Agent, "{args:?}"),
                other => panic!("help 여야({args:?}): {other:?}"),
            }
        }

        let root = render_help(HelpTopic::Root, MailSurface::Shown);
        let mail = render_help(HelpTopic::Mail, MailSurface::Shown);
        let agent = render_help(HelpTopic::Agent, MailSurface::Shown);
        assert!(root.contains(CLI_GROUP_AGENT), "계열 목록에 agent: {root}");
        for verb in CLI_AGENT_VERBS {
            assert!(agent.contains(verb), "{verb} 동사가 help 에: {agent}");
        }
        for flag in CLI_AGENT_FLAGS {
            assert!(agent.contains(flag), "{flag} 가 help 에: {agent}");
        }
        // 표면에 없는 동사를 help 가 가르치면 LLM 이 없는 명령을 시도한다(ADR-0122 미해소분).
        for absent in ["kill", " rm ", "delete"] {
            assert!(
                !agent.contains(absent),
                "표면에 없는 동사가 help 에 있다({absent}): {agent}"
            );
        }
        for text in [&root, &mail, &agent] {
            assert!(
                !text.contains(HELP_TOOL_SLOT),
                "치환 안 된 자리가 남으면 안 된다: {text}"
            );
            assert!(text.contains(CLI_EXE_NAME), "실행파일 이름이 상수에서 와야");
        }
        assert!(root.contains(CLI_GROUP_MAIL), "계열 목록에 mail: {root}");
        for verb in ["send", "status", "pending"] {
            assert!(mail.contains(verb), "{verb} 동사가 help 에: {mail}");
        }
        for flag in [
            "--to",
            "--body",
            "--body-stdin",
            "--request",
            "--reply-by",
            "--reply-to",
        ] {
            assert!(mail.contains(flag), "{flag} 가 help 에: {mail}");
        }
    }

    // ── ADR-0133: 우편 표식 — 목록만 가리고 실행은 가리지 않는다 ─────────────────────────

    /// 표식 값 → 표면. **부재·모르는 값이 전부 보이는 쪽**인 것이 계약이다(사람이 셸에서 여는 자리).
    ///
    /// ★운영 함수(`MailSurface::from_env`)를 직접 부른다 — 매핑을 여기서 다시 적지 말 것★: 다시 적으면
    ///   테스트가 자기 사본을 검사하게 되어 진짜 파서의 회귀를 못 잡는다.
    /// ★env 를 만지므로 직렬화한다★: 프로세스 전역 자원이라 병렬 테스트와 겹치면 플레이키해진다.
    #[test]
    fn only_the_explicit_off_marker_hides_mail() {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var(MAIL_MARKER_ENV).ok();
        for (value, want) in [
            (Some(MAIL_MARKER_OFF), MailSurface::Hidden),
            (Some(MAIL_MARKER_ON), MailSurface::Shown),
            (Some("OFF"), MailSurface::Shown),
            (Some("off "), MailSurface::Shown),
            (Some("no"), MailSurface::Shown),
            (Some(""), MailSurface::Shown),
            (None, MailSurface::Shown),
        ] {
            match value {
                Some(v) => std::env::set_var(MAIL_MARKER_ENV, v),
                None => std::env::remove_var(MAIL_MARKER_ENV),
            }
            assert_eq!(MailSurface::from_env(), want, "표식 {value:?}");
        }
        match previous {
            Some(v) => std::env::set_var(MAIL_MARKER_ENV, v),
            None => std::env::remove_var(MAIL_MARKER_ENV),
        }
        assert_eq!(
            MailSurface::default(),
            MailSurface::Shown,
            "기본(표식 부재) = 전부 보인다"
        );
    }

    #[test]
    fn the_marker_filters_the_group_list_and_the_cross_reference_only() {
        let shown = render_help(HelpTopic::Root, MailSurface::Shown);
        let hidden = render_help(HelpTopic::Root, MailSurface::Hidden);
        assert!(shown.contains(CLI_GROUP_MAIL), "표식 on: 우편 계열 노출");
        assert!(
            !hidden.contains(CLI_GROUP_MAIL),
            "표식 off: 계열 목록에 우편이 없어야: {hidden}"
        );
        for text in [&shown, &hidden] {
            assert!(text.contains(CLI_GROUP_AGENT), "제어 계열은 늘 보인다");
            assert!(
                !text.contains(HELP_TOOL_SLOT),
                "치환 안 된 자리가 남으면 안 된다: {text}"
            );
        }
        // 다른 계열의 화면이 감춘 계열을 가르치면 필터가 무의미해진다.
        let agent_hidden = render_help(HelpTopic::Agent, MailSurface::Hidden);
        assert!(
            !agent_hidden.contains(CLI_GROUP_MAIL),
            "표식 off: 제어 help 도 우편을 가르치지 않아야: {agent_hidden}"
        );
        let agent_shown = render_help(HelpTopic::Agent, MailSurface::Shown);
        assert!(
            agent_shown.contains(&format!("{CLI_EXE_NAME} {CLI_GROUP_MAIL} send")),
            "표식 on: 상호참조 한 줄은 그대로: {agent_shown}"
        );
        for verb in CLI_AGENT_VERBS {
            assert!(
                agent_hidden.contains(verb),
                "제어 동사는 표식과 무관하게 전부({verb}): {agent_hidden}"
            );
        }
    }

    /// 감춘 계열의 **사용법 요청**은 오타와 구별되지 않아야 한다 — 구별되면 목록에 없는 계열의 존재가
    /// 반려 문구로 새어 나간다.
    #[test]
    fn asking_for_a_hidden_groups_usage_reads_as_a_typo() {
        let hidden = MailSurface::Hidden;
        let typo_topic = super::parse_command(&argv(&["help", "nosuch"]), hidden)
            .expect_err("모르는 주제는 반려");
        let mail_topic =
            super::parse_command(&argv(&["help", CLI_GROUP_MAIL]), hidden).expect_err("감춘 주제");
        assert_eq!(
            typo_topic.replace("nosuch", CLI_GROUP_MAIL),
            mail_topic,
            "같은 문형이어야(주제 이름만 다름)"
        );
        let typo_group =
            super::parse_command(&argv(&["nosuch", "--help"]), hidden).expect_err("모르는 계열");
        let mail_group = super::parse_command(&argv(&[CLI_GROUP_MAIL, "--help"]), hidden)
            .expect_err("감춘 계열의 --help");
        assert_eq!(typo_group.replace("nosuch", CLI_GROUP_MAIL), mail_group);
        // 반려 문구가 감춘 계열을 예시로 들면 그 자체가 교육이다.
        for err in [&mail_topic, &mail_group] {
            assert!(
                !err.contains(&format!("help {CLI_GROUP_MAIL}")),
                "반려 문구가 감춘 계열을 안내하면 안 된다: {err}"
            );
        }
        let flag_first = super::parse_command(&argv(&["--to"]), hidden).expect_err("플래그 먼저");
        assert!(
            !flag_first.contains(CLI_GROUP_MAIL),
            "예시도 보이는 계열에서 골라야: {flag_first}"
        );
    }

    /// ★반려 문구도 교육 표면이다★: 감춘 계열의 인자 오류가 자기 사유를 그대로 돌려주면 두 연속 명령이
    ///   서로 모순되고(`mail --help` 는 "모르는 계열", `mail` 은 동사 목록), 특히 동사 없는 호출의 반려는
    ///   **감춘 화면보다 더 많이** 가르친다(계열 전체 동사 목록). 그래서 감춰진 계열의 모든 인자 오류는
    ///   한 문구로 접힌다.
    #[test]
    fn a_hidden_groups_argument_errors_never_hand_back_its_inventory() {
        let folded = unknown_group(CLI_GROUP_MAIL);
        let malformed = [
            vec!["mail"],
            vec!["mail", "wat"],
            vec!["mail", "send"],
            vec!["mail", "send", "--to", "bob"],
            vec!["mail", "send", "--nope"],
            vec!["mail", "send", "--help"],
            vec!["mail", "status"],
            vec!["mail", "status", "--help"],
            vec!["mail", "pending", "extra"],
        ];
        for args in &malformed {
            let err = super::parse_command(&argv(args), MailSurface::Hidden)
                .expect_err("형태가 틀린 호출은 반려");
            assert_eq!(
                err, folded,
                "감춘 계열의 인자 오류는 한 문구로 접힌다: {args:?}"
            );
            for verb in CLI_MAIL_VERBS {
                assert!(
                    !err.contains(verb),
                    "동사 목록이 새면 안 된다({verb}): {err}"
                );
            }
            for flag in CLI_MAIL_FLAGS {
                assert!(
                    !err.contains(flag),
                    "플래그 표기가 새면 안 된다({flag}): {err}"
                );
            }
        }
        // 대조군 — 보이는 표면에선 각자의 구체적 사유가 그대로 나온다(접기가 표식에 달렸다는 증명).
        for args in &malformed {
            let err = super::parse_command(&argv(args), MailSurface::Shown)
                .expect_err("형태가 틀린 호출은 반려");
            assert_ne!(err, folded, "표식 on 에선 구체적 사유를 준다: {args:?}");
        }
    }

    /// ★파싱 **이후** 단계의 반려도 같은 접기를 탄다★: 위 스위트는 파서 반려만 쓸어 담으므로 이 갈래를
    ///   덮지 못한다 — 실제로 `--body-stdin` 빈 입력 반려가 `--body` 를 되돌려 주며 새고 있었다.
    #[test]
    fn a_post_parse_rejection_is_folded_too_when_the_family_is_hidden() {
        let args = argv(&["mail", "send", "--to", "bob", "--body-stdin"]);
        for surface in [MailSurface::Hidden, MailSurface::Shown] {
            let parsed = super::parse_command(&args, surface).expect("형태는 유효하다(파싱 통과)");
            let ParsedCommand::Mail(m) = parsed else {
                panic!("우편 커맨드여야")
            };
            // 빈 stdin = 파싱 뒤에야 드러나는 인자 오류.
            let reason = materialize_body(m, || Ok(String::new())).expect_err("빈 본문은 반려");
            let rendered = hide_mail_reason(surface, reason);
            match surface {
                MailSurface::Hidden => {
                    assert_eq!(rendered, unknown_group(CLI_GROUP_MAIL));
                    for flag in CLI_MAIL_FLAGS {
                        assert!(
                            !rendered.contains(flag),
                            "치지도 않은 플래그를 되돌려 주면 안 된다({flag}): {rendered}"
                        );
                    }
                }
                MailSurface::Shown => assert!(
                    rendered.contains("--body"),
                    "보이는 표면에선 구체적 복구 안내를 그대로: {rendered}"
                ),
            }
        }
    }

    /// ★강제는 데몬 하나뿐이다(ADR-0133 §영향)★: 표식이 off 여도 우편 동사는 **그대로 조립돼 나간다** —
    ///   여기서 막으면 데몬 거절이 관측되지 않고, 표식을 뗀 프로세스에선 아무도 막지 않게 된다.
    #[test]
    fn a_hidden_mail_verb_still_goes_to_the_daemon() {
        let parsed = super::parse_command(
            &argv(&["mail", "send", "--to", "bob", "--body", "hi"]),
            MailSurface::Hidden,
        )
        .expect("표식 off 여도 발송은 로컬에서 막지 않는다");
        let cmd = match parsed {
            ParsedCommand::Mail(m) => materialize_body(m, || panic!("stdin 미사용")).expect("본문"),
            other => panic!("우편 커맨드여야: {other:?}"),
        };
        assert_eq!(cmd.route(), "/control/send");
        for args in [vec!["mail", "pending"], vec!["mail", "status", "m-1"]] {
            assert!(
                super::parse_command(&argv(&args), MailSurface::Hidden).is_ok(),
                "조회도 로컬에서 막지 않는다: {args:?}"
            );
        }
    }

    // ── D: 조회 응답 exit code 매핑 ─────────────────────────────────────────────────────
    #[test]
    fn query_exit_code_accepts_every_daemon_success_shape() {
        for body in [
            r#"{"id":"m-1","from":"a","awaiting_reply":false,"rows":[{"to":"b","status":"delivered","age_secs":1,"updated_secs_ago":1}]}"#,
            r#"{"me":"alice","open":[]}"#,
            r#"{"me":"alice","open":[{"id":"m-1","direction":"reply_owed_by_me"}]}"#,
            r#"{"id":"m-1","from":"a","awaiting_reply":true,"rows":[]}"#,
        ] {
            assert_eq!(exit_code_for_query_response(200, body), 0, "성공: {body}");
        }
    }

    #[test]
    fn query_exit_code_maps_rejections_to_failure() {
        for body in [
            r#"{"status":"error","code":"MESSAGE_NOT_FOUND","hint":"h"}"#,
            r#"{"status":"error","code":"BAD_ARGS","hint":"h"}"#,
        ] {
            assert_eq!(
                exit_code_for_query_response(200, body),
                EXIT_FAILED,
                "반려: {body}"
            );
        }
        assert_eq!(exit_code_for_query_response(503, "{}"), EXIT_FAILED);
        assert_eq!(exit_code_for_query_response(200, "not json"), EXIT_FAILED);
    }

    #[test]
    fn query_exit_code_flags_non_object_payloads_as_malformed() {
        for body in ["[]", "null", "42", "\"ok\""] {
            assert_eq!(
                exit_code_for_query_response(200, body),
                EXIT_MALFORMED_SUCCESS,
                "객체가 아님: {body}"
            );
        }
    }

    #[test]
    fn query_exit_code_never_reports_success_for_an_error_status() {
        let cases: [(&str, i32); 7] = [
            (
                r#"{"status":"error","code":"MESSAGE_NOT_FOUND","hint":"h"}"#,
                EXIT_FAILED,
            ),
            (
                r#"{"status":"error","code":"MESSAGE_NOT_FOUND"}"#,
                EXIT_FAILED,
            ),
            (r#"{"status":"error"}"#, EXIT_MALFORMED_SUCCESS),
            (r#"{"status":"error","code":""}"#, EXIT_MALFORMED_SUCCESS),
            (r#"{"status":"error","code":7}"#, EXIT_MALFORMED_SUCCESS),
            (r#"{"status":"error","code":null}"#, EXIT_MALFORMED_SUCCESS),
            (r#"{"status":"error","rows":[]}"#, EXIT_MALFORMED_SUCCESS),
        ];
        for (body, want) in cases {
            let got = exit_code_for_query_response(200, body);
            assert_eq!(got, want, "body={body}");
            assert_ne!(
                got, 0,
                "status:\"error\" 는 어떤 형태든 성공일 수 없다: {body}"
            );
        }
        assert_eq!(
            exit_code_for_query_response(200, r#"{"status":"ok","rows":[]}"#),
            0
        );
    }
}
