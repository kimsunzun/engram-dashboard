//! `engram-send` — CLI 입구(ADR-0086 스텝 2). 스폰된 claude 에이전트가 Bash 로 다른 에이전트에게
//! 텍스트 메시지를 보내는 최소 클라이언트다.
//!
//! ★동작★: 환경변수 `ENGRAM_TOKEN`(Bearer 토큰) + `ENGRAM_CONTROL_URL`(데몬 제어 base URL)을 읽어,
//!   `<base>/control/send` 로 `{to, body}` JSON 을 POST 한다(Authorization: Bearer <token>). 응답 JSON 을
//!   stdout 에 **그대로** 찍고, `"status":"enqueued"` 면 exit 0, 아니면 1(연결/env 오류도 1 + 에러 JSON).
//!
//! ★from 은 payload 아님★: 발신자 신원은 토큰에서만 파생된다(데몬이 토큰→신원 조회). CLI 는 to/body 만
//!   보낸다 — 이 프로세스가 자기 신원을 주장하지 않는다(사칭 차단, ADR-0086).
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

/// 진입 로직(main 이 부름) — exit code 반환. env 읽기·인자 파싱·요청·출력 매핑을 순서대로 한다.
/// 실패는 전부 stdout 에 에러 JSON 을 찍고 1 을 돌려준다(발신 에이전트가 파싱해 자기교정).
fn run(args: &[String]) -> i32 {
    // 1) 인자 파싱.
    let parsed = match parse_args(args) {
        Ok(p) => p,
        Err(msg) => {
            print_error("BAD_ARGS", &msg);
            return 1;
        }
    };

    // 2) env 크레덴셜.
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

    // 3) 요청 조립 + 전송.
    let request_body = build_request_body(&parsed.to, &parsed.body);
    match post_send(&base, &token, &request_body) {
        Ok(resp) => {
            // 응답 body 를 그대로 찍는다(verbatim). exit code 는 HTTP status(2xx?) + body status 필드 둘 다 반영.
            //   비-2xx 라도 body 가 있으면 찍는다(교정 JSON 일 수 있어 발신 에이전트가 파싱). status·body 로 매핑.
            println!("{}", resp.body);
            exit_code_for_response(resp.status, &resp.body)
        }
        // ★에러 코드 분기(M1)★: 전송 계층 실패는 그 종류에 맞는 코드로 stdout 에 찍는다. 특히
        //   INCOMPLETE_RESPONSE(Content-Length 미달 = mid-body 절단)는 CONNECT_FAILED 와 구분해야 한다 —
        //   절단된 버퍼가 우연히 JSON 으로 파싱돼 "가짜 성공(exit 0)"으로 새는 걸 막고(그래서 애초에 에러로
        //   승격), 발신 에이전트가 "연결 자체 실패"와 "응답 절단"을 구별해 재시도/보고하게 한다.
        Err(e) => {
            print_error(e.code(), &e.to_string());
            1
        }
    }
}

/// 전송 계층 실패 분류(M1) — exit code 는 항상 1 이지만 **에러 코드**는 원인별로 갈라 stdout JSON 에 싣는다.
/// CONNECT_FAILED = 연결/쓰기/읽기 IO 실패(전송 못 함) · INCOMPLETE_RESPONSE = 응답이 선언된
/// Content-Length 보다 짧게 도착(mid-body 절단 — 우연 파싱으로 가짜 성공 나는 걸 원천 차단).
#[derive(Debug)]
enum SendError {
    /// 연결·쓰기·읽기 IO 실패 또는 응답 프레이밍 파싱 실패(base/URL 문제 포함).
    Connect(String),
    /// Content-Length 가 있는데 수신 body 가 그보다 짧음(절단). received/expected 바이트 수 동봉.
    Incomplete { received: usize, expected: usize },
}

impl SendError {
    /// stdout 에러 JSON 의 `code` 필드.
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

/// 파싱된 CLI 인자.
struct CliArgs {
    to: String,
    body: String,
}

/// `--to <name>` + `--body <text>` 파싱(순서 무관). 둘 다 필수. 알 수 없는 플래그·값 누락은 Err.
/// ★플래그 설계(메인 재량, 보고)★: 명시 `--to`/`--body` 한 쌍 — 위치 인자는 body 에 공백/따옴표가 섞이면
///   셸 인용이 깨지기 쉬워(스파이크에서 관찰된 실패 모드) 명시 플래그로 고정한다.
fn parse_args(args: &[String]) -> Result<CliArgs, String> {
    let mut to: Option<String> = None;
    let mut body: Option<String> = None;
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
            other => return Err(format!("unknown argument: {other}")),
        }
        i += 1;
    }
    let to = to.ok_or("missing required --to <agent-name>")?;
    let body = body.ok_or("missing required --body <text>")?;
    Ok(CliArgs { to, body })
}

/// `{to, body}` 요청 JSON 문자열. escape 는 serde_json 이 처리(손조립 금지). from 필드 없음(신원=토큰).
fn build_request_body(to: &str, body: &str) -> String {
    serde_json::json!({ "to": to, "body": body }).to_string()
}

/// (HTTP status, body) → exit code. 성공 조건 = **HTTP 2xx** 이고 body `"status":"enqueued"` 일 때만 0.
/// ★비-2xx 는 항상 1★: 서버가 200 이 아닌데 body 만 enqueued 로 보일 수는 없으나(공통 핸들러가 200 을
/// 씀), status 를 무시하면 프레이밍 오류를 성공으로 오인할 위험이 있어 status 게이트를 둔다. body 파싱
/// 실패(비-JSON)도 1.
fn exit_code_for_response(status: u16, resp_body: &str) -> i32 {
    if !(200..300).contains(&status) {
        return 1;
    }
    match serde_json::from_str::<serde_json::Value>(resp_body) {
        Ok(v) if v.get("status").and_then(|s| s.as_str()) == Some("enqueued") => 0,
        _ => 1,
    }
}

/// 파싱된 HTTP 응답 — status code + body 텍스트. run() 이 둘 다 봐서 exit code 를 정한다.
/// (Debug = 단위 테스트에서 expect_err 시 Ok 쪽 표시용.)
#[derive(Debug)]
struct HttpResponse {
    status: u16,
    body: String,
}

/// base URL(`http://host:port`) + `/control/send` 로 최소 HTTP/1.1 POST → 파싱된 응답(status+body).
/// 로컬 평문 HTTP 전용(TLS 미지원 — 데몬은 127.0.0.1 평문). 실패는 SendError(연결/절단 구분, M1).
fn post_send(base: &str, token: &str, request_body: &str) -> Result<HttpResponse, SendError> {
    let (host, port) = parse_host_port(base).map_err(SendError::Connect)?;
    let path = format!("{}/control/send", base_path(base));

    let mut stream = TcpStream::connect((host.as_str(), port))
        .map_err(|e| SendError::Connect(format!("connect {host}:{port} failed: {e}")))?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|_| stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|e| SendError::Connect(format!("set timeout failed: {e}")))?;

    // HTTP/1.1 POST 손조립. Content-Length 필수(서버가 body 경계를 알게), Connection: close(응답 후 종료 →
    //   서버가 응답 뒤 소켓을 닫아 read_to_end 가 결정적으로 EOF 를 본다).
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
/// body 는 UTF-8 lossy 로 문자열화(로컬 데몬은 JSON UTF-8). 파싱 불가·절단이면 SendError.
fn parse_response(raw: &[u8]) -> Result<HttpResponse, SendError> {
    // 헤더/본문 경계 = 최초 CRLF CRLF.
    let sep = find_subslice(raw, b"\r\n\r\n").ok_or_else(|| {
        SendError::Connect("malformed HTTP response (no header/body separator)".to_string())
    })?;
    let head = &raw[..sep];
    let body_bytes = &raw[sep + 4..];
    let head_text = String::from_utf8_lossy(head);
    let mut lines = head_text.split("\r\n");

    // status line — `HTTP/1.1 200 OK`. 두 번째 토큰이 status code.
    let status_line = lines.next().ok_or_else(|| {
        SendError::Connect("malformed HTTP response (no status line)".to_string())
    })?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| SendError::Connect(format!("malformed HTTP status line: {status_line}")))?;

    // 헤더 파싱(대소문자 무시 key). Content-Length·Transfer-Encoding 만 관심.
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
        // ★short read = 에러(M1)★: 수신 body 가 선언된 Content-Length 보다 짧으면 연결이 body 도중 끊긴
        //   것이다. 예전엔 min() 으로 있는 만큼만 취했는데, 절단된 조각이 우연히 유효 JSON 이면
        //   exit_code_for_response 가 "enqueued" 로 오인해 가짜 성공(exit 0)이 날 수 있다. 그래서 절단은
        //   조용히 받지 않고 INCOMPLETE_RESPONSE 로 승격한다. (초과분은 파이프라인 잔재라 무시 — len 만큼만.)
        if body_bytes.len() < len {
            return Err(SendError::Incomplete {
                received: body_bytes.len(),
                expected: len,
            });
        }
        String::from_utf8_lossy(&body_bytes[..len]).to_string()
    } else {
        // read-to-EOF fallback(Connection: close). 남은 전부가 body.
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
        // 청크 크기 라인 = 다음 CRLF 까지.
        let line_end = find_subslice(bytes, b"\r\n").ok_or_else(|| {
            SendError::Connect("malformed chunked body (no size line)".to_string())
        })?;
        let size_line = String::from_utf8_lossy(&bytes[..line_end]);
        // chunk extension 제거(`;` 앞만) + hex 파싱.
        let hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(hex, 16)
            .map_err(|e| SendError::Connect(format!("malformed chunk size '{hex}': {e}")))?;
        bytes = &bytes[line_end + 2..]; // 크기 라인 CRLF 소비.
        if size == 0 {
            break; // 마지막 청크(trailer 무시).
        }
        if bytes.len() < size {
            return Err(SendError::Connect("truncated chunked body".to_string()));
        }
        out.extend_from_slice(&bytes[..size]);
        bytes = &bytes[size..];
        // 청크 데이터 뒤 CRLF 소비(있으면).
        if bytes.starts_with(b"\r\n") {
            bytes = &bytes[2..];
        }
    }
    Ok(String::from_utf8_lossy(&out).to_string())
}

/// haystack 안에서 needle 의 첫 시작 인덱스(std 만 — memchr 미의존).
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// base URL 의 host·port 추출. `http://127.0.0.1:PORT` 형태 전제(스킴은 http 만). 실패는 Err.
fn parse_host_port(base: &str) -> Result<(String, u16), String> {
    let rest = base
        .strip_prefix("http://")
        .ok_or_else(|| format!("unsupported control url (expected http://): {base}"))?;
    // path 가 붙어 있으면 잘라낸다(authority 만).
    let authority = rest.split('/').next().unwrap_or(rest);
    let (host, port) = authority
        .rsplit_once(':')
        .ok_or_else(|| format!("control url missing port: {base}"))?;
    let port: u16 = port
        .parse()
        .map_err(|e| format!("invalid port in control url: {e}"))?;
    Ok((host.to_string(), port))
}

/// base URL 에서 path prefix 추출(authority 뒤). 대개 빈 문자열(base=host:port). 있으면 그대로 붙인다.
fn base_path(base: &str) -> String {
    let rest = base.strip_prefix("http://").unwrap_or(base);
    match rest.find('/') {
        Some(idx) => rest[idx..].trim_end_matches('/').to_string(),
        None => String::new(),
    }
}

/// 에러 JSON 을 stdout 에 찍는다(ACK/에러와 같은 shape — status/code/hint). 발신 에이전트가 파싱해 자기교정.
fn print_error(code: &str, hint: &str) {
    let v = serde_json::json!({ "status": "error", "code": code, "hint": hint });
    println!("{v}");
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── 요청 본문 조립(escape) ─────────────────────────────────────────────────────
    #[test]
    fn build_request_body_escapes() {
        let b = build_request_body("bob", "line1\n\"quoted\"");
        let v: serde_json::Value = serde_json::from_str(&b).expect("valid json");
        assert_eq!(v["to"], "bob");
        assert_eq!(v["body"], "line1\n\"quoted\"");
        // ★from 필드 없음★ — 신원은 토큰에서만(payload from 금지).
        assert!(v.get("from").is_none(), "요청에 from 필드가 없어야");
    }

    // ── exit code 매핑 ─────────────────────────────────────────────────────────────
    #[test]
    fn exit_code_enqueued_2xx_is_zero() {
        assert_eq!(
            exit_code_for_response(200, r#"{"status":"enqueued","id":"x","to":"bob"}"#),
            0
        );
    }

    #[test]
    fn exit_code_error_is_one() {
        assert_eq!(
            exit_code_for_response(
                200,
                r#"{"status":"error","code":"RECIPIENT_NOT_FOUND","hint":"h"}"#
            ),
            1
        );
    }

    #[test]
    fn exit_code_malformed_is_one() {
        assert_eq!(exit_code_for_response(200, "not json"), 1);
    }

    #[test]
    fn exit_code_non_2xx_is_one_even_if_body_looks_enqueued() {
        // ★F4★: 비-2xx 는 body 가 enqueued 처럼 보여도 실패(프레이밍 오류를 성공으로 오인 방지).
        assert_eq!(
            exit_code_for_response(500, r#"{"status":"enqueued","id":"x","to":"bob"}"#),
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
        // Content-Length 만큼만 body 로 취한다(초과 잔재 무시).
        let body = "{\"status\":\"enqueued\"}";
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
        // ★M1★: Content-Length 가 100 인데 실제 body 는 그보다 짧게 도착(mid-body 절단) → INCOMPLETE_RESPONSE.
        //   절단 조각이 우연히 유효 JSON 이어도 성공으로 오인하지 않는다(가짜 exit 0 차단).
        let partial = r#"{"status":"enqueued"}"#; // 21바이트 — 100 미달.
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
        // 헤더 key 는 대소문자 무시로 인식한다(content-length 소문자).
        let body = "{\"status\":\"enqueued\"}";
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
        // Transfer-Encoding: chunked → 청크 이어붙임(0-청크 종료). "{\"status\":" + "\"enqueued\"}".
        let resp = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                    a\r\n{\"status\":\r\n\
                    b\r\n\"enqueued\"}\r\n\
                    0\r\n\r\n";
        let r = parse_response(resp.as_bytes()).expect("parse chunked");
        assert_eq!(r.status, 200);
        assert_eq!(r.body, "{\"status\":\"enqueued\"}", "chunked de-frame");
    }

    #[test]
    fn parse_response_read_to_eof_fallback() {
        // Content-Length·Transfer-Encoding 둘 다 없으면 나머지 전부가 body(Connection: close).
        let resp =
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"status\":\"enqueued\"}";
        let r = parse_response(resp.as_bytes()).expect("parse");
        assert_eq!(r.status, 200);
        assert_eq!(r.body, "{\"status\":\"enqueued\"}");
    }

    #[test]
    fn parse_response_non_2xx_status() {
        // 비-2xx status line 도 파싱해 status 를 정확히 돌려준다(run 이 exit 1 로 매핑).
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
        // chunk extension(`;`) + 여러 청크 + 0 종료.
        let raw = "3;ext=1\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        assert_eq!(dechunk(raw.as_bytes()).expect("dechunk"), "abcde");
    }
}
