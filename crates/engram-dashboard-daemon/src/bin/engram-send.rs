//! `engram-send` — CLI 입구(ADR-0086 스텝 2 · S18 D). 스폰된 claude 에이전트가 Bash 로 팀에 말을 걸고
//! 자기 미결을 확인하는 최소 클라이언트다. **MCP 툴 2종의 미러**(spec §6 듀얼 입구 — `group` 툴/서브커맨드는
//! ADR-0111 결정 4 로 제거됐다).
//!
//! ★동작★: 환경변수 `ENGRAM_TOKEN`(Bearer 토큰) + `ENGRAM_CONTROL_URL`(데몬 제어 base URL)을 읽어
//!   `<base>/control/<verb>` 로 JSON 을 POST 한다(Authorization: Bearer <token>). 응답 JSON 을 stdout 에
//!   **그대로** 찍는다.
//!
//! ```text
//! engram-send --to <수신자[,수신자…]> --body "…" [--request [--reply-by 10m]] [--reply-to m-xxxx]
//!     # 수신자 = 이름 | agent id | @here(나 빼고 지금 산 전원) | @all(나 빼고 명부 전원 — 잠든 것 포함,
//!     #          ADR-0121). **콤마로 여러 명**(ADR-0111 다중 수신자).
//! engram-send --to <수신자> --body-stdin <<'EOF' … EOF   # 인용 지옥 회피(D)
//! engram-send status <m-id>                              # 그 메시지의 배달 장부
//! engram-send pending                                    # 내 미결(보낸 것·기다리는 것·내가 답할 것)
//! ```
//!
//! ★exit code 3분법★: **0** = 접수/조회 성공 · **1** = 실패(반려 `{status:"error",code,hint}`·연결/env
//!   오류·비-2xx·비-JSON) · **2** = 2xx 인데 응답 shape 이 깨짐(데몬/프록시 결함 — 재시도 대상이 아니라
//!   보고 대상, stderr 에 사유 한 줄). 발송 판정 정본 = `exit_code_for_response`, 조회/관리 판정 정본 =
//!   `exit_code_for_query_response`(성공 shape 이 동사마다 달라 "에러가 아니면 성공" 규칙을 쓴다).
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

/// 연결/응답 타임아웃(로컬 데몬이라 짧게). 데몬이 죽었으면 빨리 실패해 에이전트가 재시도/보고하게 한다.
const TIMEOUT: Duration = Duration::from_secs(10);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = run(&args);
    std::process::exit(code);
}

fn run(args: &[String]) -> i32 {
    let parsed = match parse_command(args) {
        Ok(p) => p,
        Err(msg) => {
            print_error("BAD_ARGS", &msg);
            return 1;
        }
    };
    let command = match materialize_body(parsed, read_stdin_to_string) {
        Ok(c) => c,
        Err(msg) => {
            print_error("BAD_ARGS", &msg);
            return 1;
        }
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
    let is_send = matches!(command, Command::Send { .. });
    match post_json(&base, route, &token, &request_body) {
        Ok(resp) => {
            // 비-2xx 라도 body 를 찍는다 — 교정 JSON 이 실려 있어 발신 에이전트가 파싱한다.
            println!("{}", resp.body);
            if is_send {
                exit_code_for_response(resp.status, &resp.body)
            } else {
                exit_code_for_query_response(resp.status, &resp.body)
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

#[derive(Debug)]
enum ParsedCommand {
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
}

impl Command {
    /// ★경로 지식은 CLI 소유(ADR-0086)★: 데몬은 base URL 만 env 로 준다 — 라우트 조립은 여기가 정본이고,
    ///   데몬측 상수(`mcp_server.rs` CONTROL_*_PATH)와 **손으로 맞춰져** 있다(빌드가 강제 못 함).
    fn route(&self) -> &'static str {
        match self {
            Command::Send(_) => "/control/send",
            Command::Status { .. } | Command::Pending => "/control/messages",
        }
    }

    fn request_body(&self) -> String {
        match self {
            Command::Send(a) => build_request_body(a),
            Command::Status { id } => serde_json::json!({ "id": id }).to_string(),
            Command::Pending => serde_json::json!({}).to_string(),
        }
    }
}

/// ★기본 동사 = 발송(D)★: 인자가 아예 없거나 첫 인자가 플래그면 발송이다 — 옛 호출 형태
///   `engram-send --to … --body …` 와 100% 하위호환이라 기존 프라이밍·스크립트가 그대로 돈다.
/// ★kebab-case 플래그 ↔ snake_case wire(spec §1 표기 매핑)★: 셸 관례는 `--reply-by`, JSON 필드는
///   `reply_by` 다 — 변환은 `Command::request_body` 한 곳에서만 한다.
fn parse_command(args: &[String]) -> Result<ParsedCommand, String> {
    match args.first().map(|s| s.as_str()) {
        Some("status") => {
            let id = args
                .get(1)
                .ok_or("status requires a message id, e.g. engram-send status m-7f3k9q2d")?;
            if args.len() > 2 {
                return Err(format!(
                    "unexpected argument after status <id>: {}",
                    args[2]
                ));
            }
            Ok(ParsedCommand::Status { id: id.clone() })
        }
        Some("pending") => {
            if args.len() > 1 {
                return Err(format!("pending takes no arguments: {}", args[1]));
            }
            Ok(ParsedCommand::Pending)
        }
        _ => parse_send_args(args),
    }
}

/// ★플래그 설계(메인 재량, 보고)★: 명시 `--to`/`--body` 한 쌍 — 위치 인자는 body 에 공백/따옴표가 섞이면
///   셸 인용이 깨지기 쉬워(스파이크에서 관찰된 실패 모드) 명시 플래그로 고정한다.
fn parse_send_args(args: &[String]) -> Result<ParsedCommand, String> {
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
                to = Some(args.get(i).ok_or("--to requires a value")?.clone());
            }
            "--body" => {
                i += 1;
                body = Some(args.get(i).ok_or("--body requires a value")?.clone());
            }
            "--body-stdin" => body_stdin = true,
            "--request" => request = true,
            "--reply-by" => {
                i += 1;
                reply_by = Some(args.get(i).ok_or("--reply-by requires a value")?.clone());
            }
            "--reply-to" => {
                i += 1;
                reply_to = Some(args.get(i).ok_or("--reply-to requires a value")?.clone());
            }
            other => return Err(format!("unknown argument: {other}")),
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
    Ok(ParsedCommand::Send {
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
    parsed: ParsedCommand,
    read_stdin: impl FnOnce() -> Result<String, String>,
) -> Result<Command, String> {
    Ok(match parsed {
        ParsedCommand::Send {
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
        ParsedCommand::Status { id } => Command::Status { id },
        ParsedCommand::Pending => Command::Pending,
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
            "engram-send: malformed error response — 'status' is \"error\" but 'code' is missing or not a non-empty string, so this rejection cannot be acted on"
        );
        return EXIT_MALFORMED_SUCCESS;
    }
    let Some(results) = v.get("results") else {
        eprintln!(
            "engram-send: malformed success response — neither a success shape ('results') nor a valid error shape ('status':\"error\" + non-empty 'code')"
        );
        return EXIT_MALFORMED_SUCCESS;
    };
    let Some(items) = results.as_array() else {
        eprintln!("engram-send: malformed success response — 'results' is not an array");
        return EXIT_MALFORMED_SUCCESS;
    };
    let id_ok = v
        .get("id")
        .and_then(|i| i.as_str())
        .is_some_and(|s| !s.is_empty());
    if !id_ok {
        eprintln!("engram-send: malformed success response — missing or empty 'id'");
        return EXIT_MALFORMED_SUCCESS;
    }
    if items.is_empty() {
        eprintln!(
            "engram-send: malformed success response — 'results' is empty (nothing was accepted)"
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
                "engram-send: malformed success response — result entry needs a non-empty 'to' and a status of delivered|pending|failed"
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
            "engram-send: malformed error response — 'status' is \"error\" but 'code' is missing or not a non-empty string, so this rejection cannot be acted on"
        );
        return EXIT_MALFORMED_SUCCESS;
    }
    if !v.is_object() {
        eprintln!(
            "engram-send: malformed response — expected a JSON object from the daemon query route"
        );
        return EXIT_MALFORMED_SUCCESS;
    }
    0
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

    fn parse_args(args: &[String]) -> Result<CliArgs, String> {
        let parsed = parse_command(args)?;
        match materialize_body(parsed, || {
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

    // ── C3: 회신 계약 플래그(spec §6 CLI 미러) ────────────────────────────────────────────────

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

    // ── D: `--body-stdin`(인용 지옥 회피, spec §6) ──────────────────────────────────────
    #[test]
    fn body_stdin_reads_the_body_from_the_injected_reader() {
        let parsed = parse_command(&argv(&["--to", "bob", "--body-stdin"])).expect("parse");
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
        let err = parse_command(&argv(&["--to", "bob", "--body", "hi", "--body-stdin"]))
            .expect_err("상호배타 반려");
        assert!(err.contains("mutually exclusive"), "사유 문구: {err}");
    }

    #[test]
    fn body_stdin_with_no_input_is_rejected_not_sent_empty() {
        let parsed = parse_command(&argv(&["--to", "bob", "--body-stdin"])).expect("parse");
        let err = materialize_body(parsed, || Ok(String::new())).expect_err("빈 stdin 반려");
        assert!(err.contains("no input"), "사유 문구: {err}");
    }

    #[test]
    fn missing_body_mentions_both_ways_to_supply_it() {
        let err = parse_command(&argv(&["--to", "bob"])).expect_err("본문 누락");
        assert!(err.contains("--body-stdin"), "두 경로를 모두 안내: {err}");
    }

    // ── D: 서브커맨드 파싱(status / pending) ──────────────────────────────────────────

    fn wire(args: &[&str]) -> (&'static str, serde_json::Value) {
        let parsed = parse_command(&argv(args)).expect("parse");
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
            wire(&["status", "m-7f3k9q2d"]),
            (
                "/control/messages",
                serde_json::json!({ "id": "m-7f3k9q2d" })
            )
        );
        assert_eq!(
            wire(&["pending"]),
            ("/control/messages", serde_json::json!({}))
        );
    }

    #[test]
    fn subcommand_shape_errors_are_rejected_at_the_cli() {
        for args in [
            vec!["status"],
            vec!["status", "m-1", "extra"],
            vec!["pending", "extra"],
            vec!["group"],
            vec!["group", "wat"],
            vec!["group", "list", "extra"],
            vec!["group", "update"],
            vec!["group", "update", "--add", "a"],
            vec!["group", "update", "@t", "--add"],
            vec!["group", "update", "@t", "--nope", "x"],
            vec!["group", "delete"],
            vec!["group", "delete", "@t", "extra"],
        ] {
            assert!(
                parse_command(&argv(&args)).is_err(),
                "형태 오류여야: {args:?}"
            );
        }
    }

    #[test]
    fn a_flag_first_invocation_is_still_a_send_for_backward_compatibility() {
        let (route, body) = wire(&["--to", "bob", "--body", "hi"]);
        assert_eq!(route, "/control/send");
        assert_eq!(body, serde_json::json!({ "to": "bob", "body": "hi" }));
        let (route, body) = wire(&["--to", "@coders", "--body", "공지"]);
        assert_eq!(route, "/control/send");
        assert_eq!(body["to"], "@coders");
    }

    // ── D: 조회 응답 exit code 매핑 ─────────────────────────────────────────────────────
    #[test]
    fn query_exit_code_accepts_every_daemon_success_shape() {
        for body in [
            r#"{"id":"m-1","from":"a","awaiting_reply":false,"rows":[{"to":"b","status":"delivered","age_secs":1,"updated_secs_ago":1}]}"#,
            r#"{"me":"alice","open":[]}"#,
            r#"{"groups":["@all"]}"#,
            r#"{"group":"@coders","members":["alice"]}"#,
            r#"{"group":"@coders","deleted":true}"#,
        ] {
            assert_eq!(exit_code_for_query_response(200, body), 0, "성공: {body}");
        }
    }

    #[test]
    fn query_exit_code_maps_rejections_to_failure() {
        for body in [
            r#"{"status":"error","code":"MESSAGE_NOT_FOUND","hint":"h"}"#,
            r#"{"status":"error","code":"GROUP_NOT_FOUND","hint":"h"}"#,
            r#"{"status":"error","code":"INVALID_GROUP_ARGS","hint":"h"}"#,
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
                r#"{"status":"error","code":"GROUP_NOT_FOUND"}"#,
                EXIT_FAILED,
            ),
            (r#"{"status":"error"}"#, EXIT_MALFORMED_SUCCESS),
            (r#"{"status":"error","code":""}"#, EXIT_MALFORMED_SUCCESS),
            (r#"{"status":"error","code":7}"#, EXIT_MALFORMED_SUCCESS),
            (r#"{"status":"error","code":null}"#, EXIT_MALFORMED_SUCCESS),
            (
                r#"{"status":"error","groups":["@all"]}"#,
                EXIT_MALFORMED_SUCCESS,
            ),
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
            exit_code_for_query_response(200, r#"{"status":"ok","groups":["@all"]}"#),
            0
        );
    }
}
