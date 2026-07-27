//! ADR-0086 스텝 2 · F7(b) — `engram-send` CLI **프로세스 레벨** 테스트.
//!
//! 실제 빌드된 바이너리(`CARGO_BIN_EXE_engram-send`)를 스폰하고, 테스트가 띄운 tiny std TcpListener
//! 스텁이 canned HTTP 응답을 돌려주게 해 **wire → 파싱 → stdout JSON → exit code** 전 경로를 검증한다.
//! (단위 테스트는 순수 함수만 봤다 — 이건 env 읽기·TCP·프로세스 종료코드까지 실측.)
//!
//! ★claude 불요·결정적★: 스텁은 std 만 쓰고 고정 응답을 내므로 claude/데몬 없이 항상 같은 결과다.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::thread;

/// 스텁 리스너를 127.0.0.1:0 에 띄우고, 첫 연결 1건에 canned 응답을 돌려준다. (host, port, join) 반환.
/// 요청 바디는 무시(핸드셰이크만 소비) — 이 테스트는 CLI 의 응답 파싱·exit code 매핑을 본다.
fn spawn_stub(response: &'static str) -> (String, u16, thread::JoinHandle<()>) {
    let (host, port, handle) = spawn_capturing_stub(response);
    // 캡처를 안 쓰는 기존 테스트용 어댑터 — join 핸들의 반환값(요청 텍스트)을 버린다.
    let handle = thread::spawn(move || {
        let _ = handle.join();
    });
    (host, port, handle)
}

/// 요청을 **캡처하는** 스텁(D) — join 하면 CLI 가 보낸 raw 요청 텍스트를 돌려준다. 라우트 선택(경로)과
/// 바디가 실제 바이너리에서 어떻게 나가는지를 프로세스 레벨로 단언하는 데 쓴다.
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

/// 200 + Content-Length 응답 문자열을 만든다(스텁 canned 응답 조립 — 반복 제거).
fn ok_response(body: &str) -> &'static str {
    let s = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    Box::leak(s.into_boxed_str())
}

/// 임의 인자로 바이너리를 스폰하되 stdin 에 **raw 바이트**를 먹인다(비-UTF8 경로 검증용 — A2).
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

/// 임의 인자로 바이너리를 스폰(선택적으로 stdin 을 먹인다). (stdout, exit code) 반환.
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
        // drop → EOF: `--body-stdin` 의 read_to_string 이 여기서 끝난다.
    }
    let out = child.wait_with_output().expect("wait engram-send");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// 빌드된 engram-send 바이너리를 env(ENGRAM_TOKEN/ENGRAM_CONTROL_URL) 붙여 스폰. (stdout, exit code) 반환.
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
    // ★C1(spec §6)★: 200 + 성공 shape `{ id, results:[{to,status:"delivered"}] }`(Content-Length)
    //   → stdout ACK + exit 0. 옛 `{"status":"enqueued"}` 는 S18 메시징 v1 이 이 shape 로 교체(ADR-0103).
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
    // 200 + error JSON(chunked) → stdout 에러 body + exit 1(교정 에러도 CLI 는 1 로 매핑).
    // chunked: "{\"status\":\"error\"," (0x12=18) + "\"code\":\"X\"}" (0xb=11) + 0.
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
    // 401 + 빈 body → exit 1(비-2xx).
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
    //   바이너리가 전송 실패에 내는 코드는 CONNECT_FAILED(연결/쓰기/읽기 IO 실패·프레이밍 파싱 실패)와
    //   INCOMPLETE_RESPONSE(Content-Length 미달 절단) 둘뿐이다 — 두 코드만 허용한다(교정 에러
    //   RECIPIENT_NOT_FOUND 등은 서버가 200 으로 응답해야 나오므로 이 경로에선 불가).
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
//
// ★단위 테스트와 다른 축★: 단위 테스트는 `Command::route()`/`request_body()` 값을 본다. 여기선 실제
//   바이너리가 그 값으로 **정말 그 경로에 POST 하는지**를 wire 에서 확인한다(라우트 조립이 base URL 의
//   path prefix 처리와 엮여 있어, 값이 맞아도 조립이 틀릴 수 있다).

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
    // 바디는 무인자 조회 = 빈 객체. `--as` 같은 신원 인자가 새어 나가면 안 된다.
    assert!(request.ends_with("{}"), "무인자 조회 바디: {request}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["me"], "alice");
}

#[test]
fn engram_send_status_and_group_subcommands_hit_their_routes() {
    // status <id> → /control/messages + {"id": …}
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

    // group update @g --add a,b → /control/group + 콤마 분해된 배열
    let response = ok_response(r#"{"group":"@coders","members":["alice","bob"]}"#);
    let (host, port, stub) = spawn_capturing_stub(response);
    let url = format!("http://{host}:{port}");
    let (_stdout, code) = run_cli(
        &url,
        &["group", "update", "@coders", "--add", "alice,bob"],
        None,
    );
    let request = stub.join().expect("stub join");
    assert_eq!(code, 0);
    assert!(
        request.starts_with("POST /control/group HTTP/1.1"),
        "{request}"
    );
    // ★값은 가공 없이 그대로 실린다(D 리뷰 A1)★ — 콤마 분해·trim 은 데몬(ingress)의 일이다. CLI 가
    //   미리 다듬으면 같은 표기가 MCP 로 왔을 때와 최종 상태가 갈린다(유령 멤버 결함의 원인).
    assert!(
        request.contains(r#"{"add":["alice,bob"],"group":"@coders"}"#),
        "add 값은 argv 그대로 전달: {request}"
    );

    // group delete @g → delete:true
    let response = ok_response(r#"{"group":"@coders","deleted":true}"#);
    let (host, port, stub) = spawn_capturing_stub(response);
    let url = format!("http://{host}:{port}");
    let (_stdout, code) = run_cli(&url, &["group", "delete", "@coders"], None);
    let request = stub.join().expect("stub join");
    assert_eq!(code, 0);
    assert!(
        request.starts_with("POST /control/group HTTP/1.1"),
        "{request}"
    );
    assert!(request.contains(r#""delete":true"#), "{request}");
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
    // ★D 리뷰 A2 — 문서와 코드를 **구현 쪽으로** 정렬★: 이전엔 `read_to_string` 이라 비-UTF8 stdin 이
    //   `InvalidData` 로 발송 자체를 막았다(주석은 lossy 라고 적혀 있었다). Windows 셸에서 cp949 파이프는
    //   현실적으로 들어오는데, 그때 "아예 못 보낸다" 보다 "몇 글자 깨진 채라도 전달된다" 가 낫다.
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
    // 형태 오류는 연결 전에 끝난다 — 스텁 없이(연결 불가 주소로) 돌려도 BAD_ARGS 가 나와야 한다.
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
