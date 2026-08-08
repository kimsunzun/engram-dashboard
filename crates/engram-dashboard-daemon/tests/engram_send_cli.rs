//! ADR-0086 스텝 2 · F7(b) — `engram-send` CLI **프로세스 레벨** 테스트.
//!
//! 단위 테스트는 순수 함수만 봤다 — 여기선 **wire → 파싱 → stdout JSON → exit code** 전 경로를 실측한다
//! (env 읽기·TCP·프로세스 종료코드 포함).
//!
//! ★claude 불요·결정적★: 스텁은 std 만 쓰고 고정 응답을 내므로 claude/데몬 없이 항상 같은 결과다.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::thread;

fn spawn_stub(response: &'static str) -> (String, u16, thread::JoinHandle<()>) {
    let (host, port, handle) = spawn_capturing_stub(response);
    let handle = thread::spawn(move || {
        let _ = handle.join();
    });
    (host, port, handle)
}

fn spawn_capturing_stub(response: &'static str) -> (String, u16, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
    let addr = listener.local_addr().expect("addr");
    let handle = thread::spawn(move || {
        let mut seen = String::new();
        if let Ok((mut stream, _)) = listener.accept() {
            // 요청을 조금 읽어 소켓을 소비(전부 안 읽어도 응답은 보낼 수 있다). non-blocking 회피 위해 짧게.
            let mut buf = [0u8; 4096];
            if let Ok(n) = stream.read(&mut buf) {
                seen = String::from_utf8_lossy(&buf[..n]).to_string();
            }
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            // Connection: close 응답이면 여기서 stream drop → 클라이언트가 EOF 를 본다.
        }
        seen
    });
    (addr.ip().to_string(), addr.port(), handle)
}

fn ok_response(body: &str) -> &'static str {
    let s = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    Box::leak(s.into_boxed_str())
}

fn run_cli_bytes(control_url: &str, args: &[&str], stdin: &[u8]) -> (String, i32) {
    use std::process::Stdio;
    let exe = env!("CARGO_BIN_EXE_engram-send");
    let mut child = Command::new(exe)
        .args(args)
        .env("ENGRAM_TOKEN", "test-token")
        .env("ENGRAM_CONTROL_URL", control_url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn engram-send");
    {
        let mut sink = child.stdin.take().expect("stdin piped");
        sink.write_all(stdin).expect("write stdin");
    }
    let out = child.wait_with_output().expect("wait engram-send");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

fn run_cli(control_url: &str, args: &[&str], stdin: Option<&str>) -> (String, i32) {
    use std::process::Stdio;
    let exe = env!("CARGO_BIN_EXE_engram-send");
    let mut child = Command::new(exe)
        .args(args)
        .env("ENGRAM_TOKEN", "test-token")
        .env("ENGRAM_CONTROL_URL", control_url)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn engram-send");
    if let Some(text) = stdin {
        let mut sink = child.stdin.take().expect("stdin piped");
        sink.write_all(text.as_bytes()).expect("write stdin");
        // drop → EOF: `--body-stdin` 의 read_to_end 가 여기서 끝난다.
    }
    let out = child.wait_with_output().expect("wait engram-send");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

fn run_send(control_url: &str, to: &str, body: &str) -> (String, i32) {
    let exe = env!("CARGO_BIN_EXE_engram-send");
    let out = Command::new(exe)
        .args(["--to", to, "--body", body])
        .env("ENGRAM_TOKEN", "test-token")
        .env("ENGRAM_CONTROL_URL", control_url)
        .output()
        .expect("spawn engram-send");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let code = out.status.code().unwrap_or(-1);
    (stdout, code)
}

#[test]
fn engram_send_delivered_prints_ack_and_exits_zero() {
    let body = r#"{"id":"m1","results":[{"to":"bob","status":"delivered"}]}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let response: &'static str = Box::leak(response.into_boxed_str());
    let (host, port, handle) = spawn_stub(response);
    let url = format!("http://{host}:{port}");

    let (stdout, code) = run_send(&url, "bob", "hi");
    let _ = handle.join();

    assert_eq!(code, 0, "성공 shape(results) → exit 0. stdout={stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "stdout 에 ACK JSON: {stdout}"
    );
    assert_eq!(v["results"][0]["to"], "bob");
}

#[test]
fn engram_send_corrective_error_prints_body_and_exits_one() {
    // chunk 크기는 16진 — "{\"status\":\"error\"," (0x12=18) + "\"code\":\"X\"}" (0xb=11) + 0.
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n\
                    12\r\n{\"status\":\"error\",\r\n\
                    b\r\n\"code\":\"X\"}\r\n\
                    0\r\n\r\n";
    let (host, port, handle) = spawn_stub(response);
    let url = format!("http://{host}:{port}");

    let (stdout, code) = run_send(&url, "ghost", "hi");
    let _ = handle.join();

    assert_eq!(code, 1, "교정 에러 → exit 1. stdout={stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(
        v["status"], "error",
        "stdout 에 에러 JSON(de-chunked): {stdout}"
    );
    assert_eq!(v["code"], "X");
}

#[test]
fn engram_send_non_2xx_exits_one() {
    let response = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    let (host, port, handle) = spawn_stub(response);
    let url = format!("http://{host}:{port}");

    let (_stdout, code) = run_send(&url, "bob", "hi");
    let _ = handle.join();

    assert_eq!(code, 1, "비-2xx → exit 1");
}

#[test]
fn engram_send_transport_error_exits_one_with_error_json() {
    // ★결정적 연결 실패(TOCTOU 없음)★: 포트 0 은 OS 가 예약한 포트로 어떤 프로세스도 리스닝할 수 없다.
    //   bind→drop 방식은 drop 과 connect 사이에 다른 프로세스가 그 포트를 재사용하는 TOCTOU 가 있지만,
    //   http://127.0.0.1:0 을 직접 목표로 하면 connect 가 즉시 실패하고 리스너 경합이 아예 없다.
    let url = "http://127.0.0.1:0";
    let (stdout, code) = run_send(url, "bob", "hi");

    assert_eq!(code, 1, "전송 실패 → exit 1. stdout={stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["status"], "error", "전송 실패는 에러 JSON: {stdout}");
    let code_str = v["code"].as_str().unwrap_or("");
    assert!(
        matches!(code_str, "CONNECT_FAILED" | "INCOMPLETE_RESPONSE"),
        "전송-계층 에러 코드 집합 중 하나여야(레이스 견고): got {code_str:?} — {stdout}"
    );
}

// ── D(spec §6): 서브커맨드 미러 — 라우트 선택·stdin 본문을 **프로세스 레벨**로 실측 ────────────────

#[test]
fn engram_send_pending_posts_to_the_messages_route_and_exits_zero() {
    let response = ok_response(r#"{"me":"alice","open":[]}"#);
    let (host, port, stub) = spawn_capturing_stub(response);
    let url = format!("http://{host}:{port}");

    let (stdout, code) = run_cli(&url, &["pending"], None);
    let request = stub.join().expect("stub join");

    assert_eq!(code, 0, "조회 성공 → exit 0. stdout={stdout}");
    assert!(
        request.starts_with("POST /control/messages HTTP/1.1"),
        "pending 은 messages 라우트로: {request}"
    );
    assert!(
        request.contains("Authorization: Bearer test-token"),
        "신원은 토큰으로만(요청 바디엔 신원 없음): {request}"
    );
    assert!(request.ends_with("{}"), "무인자 조회 바디: {request}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["me"], "alice");
}

#[test]
fn engram_send_query_subcommands_hit_their_routes_and_group_is_gone() {
    let response =
        ok_response(r#"{"id":"m-7f3k9q2d","from":"a","awaiting_reply":false,"rows":[]}"#);
    let (host, port, stub) = spawn_capturing_stub(response);
    let url = format!("http://{host}:{port}");
    let (_stdout, code) = run_cli(&url, &["status", "m-7f3k9q2d"], None);
    let request = stub.join().expect("stub join");
    assert_eq!(code, 0);
    assert!(
        request.starts_with("POST /control/messages HTTP/1.1"),
        "{request}"
    );
    assert!(request.contains(r#"{"id":"m-7f3k9q2d"}"#), "{request}");

    let response = ok_response(r#"{"me":"a","open":[]}"#);
    let (host, port, stub) = spawn_capturing_stub(response);
    let url = format!("http://{host}:{port}");
    let (_stdout, code) = run_cli(&url, &["pending"], None);
    let request = stub.join().expect("stub join");
    assert_eq!(code, 0);
    assert!(
        request.starts_with("POST /control/messages HTTP/1.1"),
        "{request}"
    );

    // 회귀 가드(ADR-0111 결정 4 · ADR-0112 결정 1) — group 동사가 부활하면 프라이밍이 없는 명령을 가르친다.
    // ★네트워크를 타지 않는다★: 인자 파싱 단계에서 끝나므로 스텁 서버가 필요 없다.
    let (stdout, code) = run_cli("http://127.0.0.1:1", &["group", "list"], None);
    assert_eq!(code, 1, "모르는 서브커맨드는 BAD_ARGS: {stdout}");
    assert!(
        stdout.contains("BAD_ARGS"),
        "인자 오류로 끝나야(라우트 조회 없음): {stdout}"
    );
}

#[test]
fn engram_send_body_stdin_sends_the_piped_text_verbatim() {
    // ★인용 지옥 회피의 요점★: 셸이 건드리기 쉬운 문자(따옴표·`$`·`&`·개행)가 **그대로** 본문에 실려야 한다.
    let response = ok_response(r#"{"id":"m1","results":[{"to":"bob","status":"delivered"}]}"#);
    let (host, port, stub) = spawn_capturing_stub(response);
    let url = format!("http://{host}:{port}");
    let piped = "line1\n\"quoted\" & $HOME\nline3\n";

    let (stdout, code) = run_cli(&url, &["--to", "bob", "--body-stdin"], Some(piped));
    let request = stub.join().expect("stub join");

    assert_eq!(code, 0, "발송 성공 → exit 0. stdout={stdout}");
    assert!(
        request.starts_with("POST /control/send HTTP/1.1"),
        "{request}"
    );
    let body_start = request.find("\r\n\r\n").expect("본문 경계") + 4;
    let v: serde_json::Value =
        serde_json::from_str(request[body_start..].trim()).expect("요청 바디 json");
    assert_eq!(v["to"], "bob");
    assert_eq!(v["body"], piped, "stdin 본문이 바이트 그대로: {request}");
}

#[test]
fn engram_send_body_stdin_lossily_replaces_invalid_utf8_instead_of_refusing_to_send() {
    let response = ok_response(r#"{"id":"m1","results":[{"to":"bob","status":"delivered"}]}"#);
    let (host, port, stub) = spawn_capturing_stub(response);
    let url = format!("http://{host}:{port}");
    // 0xB0 0xA1 = cp949 '가' — UTF-8 로는 무효 바이트열이다.
    let piped: &[u8] = b"before \xB0\xA1 after";

    let (stdout, code) = run_cli_bytes(&url, &["--to", "bob", "--body-stdin"], piped);
    let request = stub.join().expect("stub join");

    assert_eq!(code, 0, "비-UTF8 이어도 발송된다. stdout={stdout}");
    let body_start = request.find("\r\n\r\n").expect("본문 경계") + 4;
    let v: serde_json::Value =
        serde_json::from_str(request[body_start..].trim()).expect("요청 바디 json");
    let body = v["body"].as_str().expect("body 문자열");
    assert!(
        body.starts_with("before ") && body.ends_with(" after"),
        "유효한 부분은 그대로 살아야: {body:?}"
    );
    assert!(
        body.contains('\u{FFFD}'),
        "무효 바이트는 U+FFFD 로 치환돼야(거부가 아니라 열화): {body:?}"
    );
}

#[test]
fn engram_send_rejects_body_and_body_stdin_together_without_touching_the_network() {
    let (stdout, code) = run_cli(
        "http://127.0.0.1:0",
        &["--to", "bob", "--body", "hi", "--body-stdin"],
        None,
    );
    assert_eq!(code, 1, "형태 오류 → exit 1. stdout={stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["code"], "BAD_ARGS", "{stdout}");
    assert!(
        v["hint"].as_str().unwrap_or_default().contains("mutually"),
        "사유가 상호배타임을 알려야: {stdout}"
    );
}
