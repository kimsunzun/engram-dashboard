//! ADR-0086 스텝 2 · F7(b) · ADR-0132 — `engram` CLI **프로세스 레벨** 테스트.
//!
//! 단위 테스트는 순수 함수만 봤다 — 여기선 **wire → 파싱 → stdout JSON → exit code** 전 경로를 실측한다
//! (env 읽기·TCP·프로세스 종료코드 포함).
//!
//! ★claude 불요·결정적★: 스텁은 std 만 쓰고 고정 응답을 내므로 claude/데몬 없이 항상 같은 결과다.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::thread;

use engram_dashboard_core::agent::types::{
    CLI_EXE_NAME, CLI_GROUP_MAIL, MAIL_MARKER_ENV, MAIL_MARKER_OFF, MAIL_MARKER_ON,
};

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

/// 요청 **여러 번**에 순서대로 답하는 스텁 — 발견 표면의 호출 경로가 두 왕복(목록 → 호출)이라 필요하다.
///
/// ★한 연결짜리 스텁으로는 그 경로를 못 잰다★: CLI 는 `Connection: close` 로 매번 새 소켓을 연다. 형제
///   스텁(`spawn_capturing_stub`)을 그대로 쓰면 두 번째 왕복이 연결 실패로 끝나 테스트가 늘 exit 1 을 본다.
fn spawn_scripted_stub(
    responses: Vec<&'static str>,
) -> (String, u16, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
    listener.set_nonblocking(true).expect("nonblocking stub");
    let addr = listener.local_addr().expect("addr");
    let handle = thread::spawn(move || {
        // ★기한을 두는 이유★: CLI 가 대본보다 **적게** 부르는 것이 정상인 케이스가 있다(로컬 반려는 두
        //   번째 왕복을 안 한다). blocking accept 로 두면 그 테스트가 join 에서 영원히 멈춘다.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut seen = Vec::new();
        for response in responses {
            let stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break Some(stream),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            break None;
                        }
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(_) => break None,
                }
            };
            let Some(mut stream) = stream else { break };
            let _ = stream.set_nonblocking(false);
            seen.push(read_request(&mut stream));
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
        seen
    });
    (addr.ip().to_string(), addr.port(), handle)
}

/// 요청 하나를 **끝까지** 읽는다 — 헤더 경계까지, 그 다음 `Content-Length` 만큼.
///
/// ★한 번의 `read` 로 끝내면 안 된다★: TCP 는 헤더와 본문을 다른 세그먼트로 줄 수 있고, 그러면 바디를 보는
///   단언이 **실패가 아니라 패닉**으로 끝난다(있어야 할 것이 아예 안 들어온다). 스텁이 그 갈림을 없애야
///   테스트가 재현 가능해진다.
fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut raw: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let head_end = raw.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4);
        if let Some(head_end) = head_end {
            let head = String::from_utf8_lossy(&raw[..head_end]).to_ascii_lowercase();
            let want: usize = head
                .split("\r\n")
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            if raw.len() >= head_end + want {
                break;
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => raw.extend_from_slice(&chunk[..n]),
        }
    }
    String::from_utf8_lossy(&raw).to_string()
}

fn run_cli_bytes(control_url: &str, args: &[&str], stdin: &[u8]) -> (String, i32) {
    use std::process::Stdio;
    let exe = env!("CARGO_BIN_EXE_engram");
    let mut child = Command::new(exe)
        .args(args)
        .env("ENGRAM_TOKEN", "test-token")
        .env("ENGRAM_CONTROL_URL", control_url)
        // 표식 부재 = 전부 보이는 표면(ADR-0133). 상속된 값이 화면을 갈라 놓지 않게 지운다.
        .env_remove(MAIL_MARKER_ENV)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn engram CLI");
    {
        let mut sink = child.stdin.take().expect("stdin piped");
        sink.write_all(stdin).expect("write stdin");
    }
    let out = child.wait_with_output().expect("wait engram CLI");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

fn run_cli(control_url: &str, args: &[&str], stdin: Option<&str>) -> (String, i32) {
    use std::process::Stdio;
    let exe = env!("CARGO_BIN_EXE_engram");
    let mut child = Command::new(exe)
        .args(args)
        .env("ENGRAM_TOKEN", "test-token")
        .env("ENGRAM_CONTROL_URL", control_url)
        .env_remove(MAIL_MARKER_ENV)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn engram CLI");
    if let Some(text) = stdin {
        let mut sink = child.stdin.take().expect("stdin piped");
        sink.write_all(text.as_bytes()).expect("write stdin");
        // drop → EOF: `--body-stdin` 의 read_to_end 가 여기서 끝난다.
    }
    let out = child.wait_with_output().expect("wait engram CLI");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

fn run_send(control_url: &str, to: &str, body: &str) -> (String, i32) {
    let exe = env!("CARGO_BIN_EXE_engram");
    let out = Command::new(exe)
        .args(["mail", "send", "--to", to, "--body", body])
        .env("ENGRAM_TOKEN", "test-token")
        .env("ENGRAM_CONTROL_URL", control_url)
        .env_remove(MAIL_MARKER_ENV)
        .output()
        .expect("spawn engram CLI");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let code = out.status.code().unwrap_or(-1);
    (stdout, code)
}

#[test]
fn engram_mail_send_delivered_prints_ack_and_exits_zero() {
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
fn engram_mail_send_corrective_error_prints_body_and_exits_one() {
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
fn engram_mail_send_non_2xx_exits_one() {
    let response = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    let (host, port, handle) = spawn_stub(response);
    let url = format!("http://{host}:{port}");

    let (_stdout, code) = run_send(&url, "bob", "hi");
    let _ = handle.join();

    assert_eq!(code, 1, "비-2xx → exit 1");
}

#[test]
fn engram_mail_send_transport_error_exits_one_with_error_json() {
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

// ── D: 우편 동사 미러 — 라우트 선택·stdin 본문을 **프로세스 레벨**로 실측 ────────────────

#[test]
fn engram_mail_pending_posts_to_the_messages_route_and_exits_zero() {
    let response = ok_response(r#"{"me":"alice","open":[]}"#);
    let (host, port, stub) = spawn_capturing_stub(response);
    let url = format!("http://{host}:{port}");

    let (stdout, code) = run_cli(&url, &["mail", "pending"], None);
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
fn engram_mail_query_verbs_hit_their_routes_and_group_is_gone() {
    let response =
        ok_response(r#"{"id":"m-7f3k9q2d","from":"a","awaiting_reply":false,"rows":[]}"#);
    let (host, port, stub) = spawn_capturing_stub(response);
    let url = format!("http://{host}:{port}");
    let (_stdout, code) = run_cli(&url, &["mail", "status", "m-7f3k9q2d"], None);
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
    let (_stdout, code) = run_cli(&url, &["mail", "pending"], None);
    let request = stub.join().expect("stub join");
    assert_eq!(code, 0);
    assert!(
        request.starts_with("POST /control/messages HTTP/1.1"),
        "{request}"
    );

    // 회귀 가드(ADR-0111 결정 4 · ADR-0112 결정 1) — group 동사가 부활하면 프라이밍이 없는 명령을 가르친다.
    // ★네트워크를 타지 않는다★: 인자 파싱 단계에서 끝나므로 스텁 서버가 필요 없다.
    let (stdout, code) = run_cli("http://127.0.0.1:1", &["mail", "group", "list"], None);
    assert_eq!(code, 1, "모르는 서브커맨드는 BAD_ARGS: {stdout}");
    assert!(
        stdout.contains("BAD_ARGS"),
        "인자 오류로 끝나야(라우트 조회 없음): {stdout}"
    );
}

#[test]
fn engram_mail_send_body_stdin_sends_the_piped_text_verbatim() {
    // ★인용 지옥 회피의 요점★: 셸이 건드리기 쉬운 문자(따옴표·`$`·`&`·개행)가 **그대로** 본문에 실려야 한다.
    let response = ok_response(r#"{"id":"m1","results":[{"to":"bob","status":"delivered"}]}"#);
    let (host, port, stub) = spawn_capturing_stub(response);
    let url = format!("http://{host}:{port}");
    let piped = "line1\n\"quoted\" & $HOME\nline3\n";

    let (stdout, code) = run_cli(
        &url,
        &["mail", "send", "--to", "bob", "--body-stdin"],
        Some(piped),
    );
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
fn engram_mail_send_body_stdin_lossily_replaces_invalid_utf8_instead_of_refusing_to_send() {
    let response = ok_response(r#"{"id":"m1","results":[{"to":"bob","status":"delivered"}]}"#);
    let (host, port, stub) = spawn_capturing_stub(response);
    let url = format!("http://{host}:{port}");
    // 0xB0 0xA1 = cp949 '가' — UTF-8 로는 무효 바이트열이다.
    let piped: &[u8] = b"before \xB0\xA1 after";

    let (stdout, code) = run_cli_bytes(
        &url,
        &["mail", "send", "--to", "bob", "--body-stdin"],
        piped,
    );
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

// ── ADR-0132 조각 ②: 제어 계열(`agent`) — 라우트 선택·바디·exit code 를 프로세스 레벨로 실측 ──────

/// 스텁이 받은 요청에서 JSON 바디만 꺼낸다.
fn request_body(request: &str) -> serde_json::Value {
    let start = request.find("\r\n\r\n").expect("본문 경계") + 4;
    serde_json::from_str(request[start..].trim()).expect("요청 바디 json")
}

#[test]
fn engram_agent_verbs_post_to_the_control_agent_route_with_the_verb_in_the_body() {
    let cases: [(&[&str], serde_json::Value, &'static str); 5] = [
        (
            &["agent", "list"],
            serde_json::json!({ "verb": "list" }),
            r#"{"agents":[]}"#,
        ),
        (
            &["agent", "spawn", "qa-bravo"],
            serde_json::json!({ "verb": "spawn", "target": "qa-bravo" }),
            r#"{"agent_id":"i","name":"qa-bravo","state":"live","created":false}"#,
        ),
        (
            &["agent", "new", "--cwd", "C:/work", "--name", "qa"],
            serde_json::json!({ "verb": "new", "cwd": "C:/work", "name": "qa" }),
            r#"{"agent_id":"i","name":"qa","state":"sleeping"}"#,
        ),
        (
            &["agent", "rename", "qa", "qa-lead"],
            serde_json::json!({ "verb": "rename", "target": "qa", "name": "qa-lead" }),
            r#"{"agent_id":"i","name":"qa-lead","outcome":"renamed"}"#,
        ),
        (
            &["agent", "move", "qa-lead", "--parent", "none"],
            serde_json::json!({ "verb": "move", "target": "qa-lead", "parent": null }),
            r#"{"agent_id":"i","name":"qa-lead","parent":null}"#,
        ),
    ];
    for (args, want_body, response) in cases {
        let (host, port, stub) = spawn_capturing_stub(ok_response(response));
        let url = format!("http://{host}:{port}");
        let (stdout, code) = run_cli(&url, args, None);
        let request = stub.join().expect("stub join");

        assert_eq!(code, 0, "성공 응답 → exit 0({args:?}): {stdout}");
        assert!(
            request.starts_with("POST /control/agent HTTP/1.1"),
            "제어 계열은 한 라우트로({args:?}): {request}"
        );
        assert!(
            request.contains("Authorization: Bearer test-token"),
            "토큰을 실어야({args:?}): {request}"
        );
        assert_eq!(request_body(&request), want_body, "{args:?}");
        // stdout 은 데몬 응답 그대로 — CLI 가 산문으로 다시 쓰지 않는다.
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(stdout.trim()).expect("stdout json"),
            serde_json::from_str::<serde_json::Value>(response).expect("response json"),
            "{args:?}"
        );
    }
}

#[test]
fn engram_agent_rejections_from_the_daemon_exit_one_with_the_body_intact() {
    let response = ok_response(r#"{"status":"error","code":"AGENT_NOT_FOUND","hint":"no agent"}"#);
    let (host, port, stub) = spawn_capturing_stub(response);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["agent", "spawn", "ghost"], None);
    let _ = stub.join().expect("stub join");

    assert_eq!(code, 1, "반려 → exit 1: {stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["code"], "AGENT_NOT_FOUND", "{stdout}");
}

/// ★변경의 증거가 없는 2xx 는 성공이 아니다★: 우편 조회 판정기를 그대로 썼을 땐 `{}` 도 exit 0 이라,
///   아무것도 만들지 않은 응답을 받고도 호출자가 만들어졌다고 믿었다. 재시도 대상(1)이 아니라 보고
///   대상(2)이라는 것까지 프로세스 레벨로 못박는다.
#[test]
fn engram_agent_hollow_success_bodies_exit_two_not_zero() {
    for (args, body) in [
        (vec!["agent", "new", "--cwd", "C:/x"], r#"{}"#),
        (vec!["agent", "new", "--cwd", "C:/x"], r#"{"status":"ok"}"#),
        (vec!["agent", "list"], r#"{"agents":"not-an-array"}"#),
        // 신원은 다 실렸는데 결말(`outcome`)이 없다 — 개명이 일어났는지 그대로였는지를 답하지 않은 body 다.
        (
            vec!["agent", "rename", "a", "b"],
            r#"{"agent_id":"i","name":"b"}"#,
        ),
        // 중첩(`{agent:{…}}`)은 어떤 데몬도 내지 않는 shape 이다 — 이 줄이 **프로세스 레벨**에서 그 갈래의
        //   부활을 막는다. 단위 표에만 두면 실제 exe 를 돌리는 이 스위트는 부활해도 전부 초록으로 남는다.
        (
            vec!["agent", "spawn", "w"],
            r#"{"agent":{"id":"i","name":"w","state":"live"},"created":false}"#,
        ),
    ] {
        let (host, port, stub) = spawn_capturing_stub(ok_response(body));
        let url = format!("http://{host}:{port}");
        let (stdout, code) = run_cli(&url, &args, None);
        let _ = stub.join().expect("stub join");
        assert_eq!(code, 2, "증거 없는 2xx → exit 2({args:?}): {stdout}");
    }
}

/// ★성공 shape(`{agent_id,name,state,…}`)이 프로세스 레벨에서 exit 0★ — 여기서 새면 정상 데몬 앞에서
///   `engram agent spawn` 이 exit 2 를 낸다.
///   ★대조는 그대로★: 마지막 케이스가 그 축을 같이 못박는다(필드만 다 있고 요청과 어긋난 응답).
#[test]
fn engram_agent_accepts_the_success_shape_with_the_same_cross_checks() {
    let cases: [(&[&str], &str, i32); 5] = [
        (
            &["agent", "spawn", "qa-bravo"],
            r#"{"agent_id":"i","name":"qa-bravo","state":"live","created":false}"#,
            0,
        ),
        (
            &["agent", "new", "--cwd", "C:/work", "--name", "qa"],
            r#"{"agent_id":"i","name":"qa","state":"sleeping"}"#,
            0,
        ),
        (
            &["agent", "rename", "qa", "qa-lead"],
            r#"{"agent_id":"i","name":"qa-lead","outcome":"renamed"}"#,
            0,
        ),
        (
            &["agent", "move", "qa-lead", "--parent", "none"],
            r#"{"agent_id":"i","name":"qa-lead","parent":null}"#,
            0,
        ),
        (
            &["agent", "spawn", "qa-bravo"],
            r#"{"agent_id":"i","name":"qa-bravo","state":"live","created":true}"#,
            2,
        ),
    ];
    for (args, response, want) in cases {
        let (host, port, stub) = spawn_capturing_stub(ok_response(response));
        let url = format!("http://{host}:{port}");
        let (stdout, code) = run_cli(&url, args, None);
        let _ = stub.join().expect("stub join");
        assert_eq!(code, want, "{args:?} ← {response}: {stdout}");
    }
}

#[test]
fn engram_agent_argument_errors_never_touch_the_network() {
    // 목적지가 포트 0 이므로 하나라도 POST 를 시도하면 CONNECT_FAILED 가 나와 단언이 깨진다.
    for args in [
        vec!["agent"],
        vec!["agent", "wat"],
        vec!["agent", "list", "--json"],
        // 만들기와 깨우기를 동시에 요구한 호출 — 어느 뜻인지 고를 근거가 없다.
        vec!["agent", "spawn", "qa-bravo", "--cwd", "C:/work"],
        vec!["agent", "spawn"],
        vec!["agent", "new"],
        vec!["agent", "rename", "only-one"],
        vec!["agent", "move", "qa-bravo"],
        vec!["agent", "new", "--cwd", "a", "--cwd", "b"],
        vec!["agent", "new", "--cwd", "--name"],
        vec!["agent", "spawn", "--help"],
        // 셸의 미설정 변수가 빈 인자로 펼쳐지는 형태 — "안 준 것" 으로 접으면 다른 명령이 실행된다.
        vec!["agent", "move", "helper", "--parent", ""],
        vec!["agent", "new", "--cwd", ""],
    ] {
        let (stdout, code) = run_cli(UNREACHABLE_URL, &args, None);
        assert_eq!(code, 1, "인자 오류 → exit 1({args:?}): {stdout}");
        let v: serde_json::Value =
            serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("stdout json: {e}"));
        assert_eq!(v["code"], "BAD_ARGS", "{args:?} → BAD_ARGS: {stdout}");
        let suggestions = backticked_help_commands(v["hint"].as_str().unwrap_or_default());
        assert!(
            !suggestions.is_empty(),
            "hint 에 실행 가능한 help 명령이 하나도 없다({args:?}): {stdout}"
        );
        for suggested in suggestions {
            let argv: Vec<&str> = suggested.split_whitespace().skip(1).collect();
            let (out, code) = run_cli_without_credentials(&argv);
            assert_eq!(
                code, 0,
                "hint 가 제안한 명령이 실제로 돌아야({args:?}): `{suggested}` → {out}"
            );
        }
    }
}

#[test]
fn engram_agent_help_is_discoverable_the_same_way_as_the_mail_group() {
    let (root, code) = run_cli_without_credentials(&["help"]);
    assert_eq!(code, 0);
    assert!(root.contains("agent"), "계열 목록에 agent: {root}");

    let (canonical, code) = run_cli_without_credentials(&["help", "agent"]);
    assert_eq!(code, 0, "계열 help 는 성공 종료: {canonical}");
    for token in [
        "list", "spawn", "new", "rename", "move", "--cwd", "--name", "--parent",
    ] {
        assert!(
            canonical.contains(token),
            "{token} 이 계열 help 에: {canonical}"
        );
    }
    for alias in [vec!["agent", "--help"], vec!["agent", "-h"]] {
        let (out, code) = run_cli_without_credentials(&alias);
        assert_eq!(code, 0);
        assert_eq!(out, canonical, "계열 help 화면이 같아야: {alias:?}");
    }
}

// ── ADR-0132: 계열 표면 — help 와 인자 오류(둘 다 네트워크를 타지 않는다) ──────────────────────

/// ★목적지 = 아무도 리스닝할 수 없는 포트 0★: 이 구획의 케이스가 전부 인자 파싱 단계에서 끝난다는
///   주장을 URL 로 못박는다. 하나라도 POST 를 시도하면 연결 실패 JSON(CONNECT_FAILED)이 나와 단언이 깨진다.
const UNREACHABLE_URL: &str = "http://127.0.0.1:0";

/// ★배송되는 파일 이름 ↔ 상수(`CLI_EXE_NAME`) 대조★ — 이 방향은 다른 어떤 테스트도 못 본다: 프라이밍
///   pin 은 프라이밍을 **상수와** 대조하고(둘이 함께 움직이면 통과), 아래 프로세스 테스트들은 바이너리를
///   **매니페스트를 통해** 찾는다(매니페스트가 정의한 이름으로 찾으니 언제나 통과). 그래서 상수·프라이밍만
///   개명하고 `[[bin]]` 을 놓친 조합은 여기서만 빨개진다. 그 조합의 런타임 증상은 배포된 실행파일이 에이전트가
///   배운 이름과 달라 우편이 조용히 멈추는 것이다(ADR-0094 정렬 불변식).
/// ★경로 출처(실측 2026-08-11)★: `CARGO_BIN_EXE_<name>` 의 `<name>` 은 **매니페스트의 `[[bin]] name`** 이다.
///   레포 사본에서 `name` 만 `engram2` 로 바꿔(`path` 는 `src/bin/engram.rs` 그대로) 빌드하니
///   `error: environment variable CARGO_BIN_EXE_engram not defined at compile time` 로 **컴파일이 멈췄고**,
///   `cargo metadata` 의 bin 타깃도 `engram2` 하나뿐이었다 — 명시 타깃이 그 경로를 이미 점유하므로
///   `src/bin/*.rs` 자동 발견이 같은 파일로 `engram` 타깃을 되살리지 않는다. 즉 매니페스트만 개명한 경우는
///   컴파일이, 상수만 개명한 경우는 아래 단언이 잡는다.
#[test]
fn the_built_binary_file_name_matches_the_shared_constant() {
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_engram"));
    let stem = exe
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_else(|| panic!("빌드 산출 경로에서 파일 stem 을 못 읽음: {exe:?}"));
    assert_eq!(
        stem, CLI_EXE_NAME,
        "배송되는 실행파일 이름과 CLI_EXE_NAME 이 갈렸다 — grant·프라이밍·PATH 해석이 전부 상수를 따르므로 \
         에이전트가 배운 명령이 존재하지 않게 된다: {exe:?}"
    );
}

#[test]
fn engram_help_lists_groups_and_group_help_documents_its_verbs() {
    let (stdout, code) = run_cli(UNREACHABLE_URL, &["help"], None);
    assert_eq!(code, 0, "help 는 성공 종료: {stdout}");
    assert!(stdout.contains("mail"), "계열 목록에 mail: {stdout}");
    assert!(
        stdout.contains("help"),
        "계열 help 로 안내하는 줄이 있어야: {stdout}"
    );

    // 인자 0 = help 와 같은 화면(에이전트가 이름만 쳐도 표면을 본다).
    let (bare, bare_code) = run_cli(UNREACHABLE_URL, &[], None);
    assert_eq!(bare_code, 0, "인자 없는 호출도 성공 종료: {bare}");
    assert_eq!(bare, stdout, "인자 없음 = help 와 같은 출력");

    let (mail, code) = run_cli(UNREACHABLE_URL, &["help", "mail"], None);
    assert_eq!(code, 0, "계열 help 는 성공 종료: {mail}");
    for token in [
        "send",
        "status",
        "pending",
        "--to",
        "--body",
        "--body-stdin",
        "--request",
        "--reply-by",
        "--reply-to",
    ] {
        assert!(mail.contains(token), "{token} 이 계열 help 에: {mail}");
    }
}

/// ENGRAM_TOKEN·ENGRAM_CONTROL_URL 을 **주지 않고** 돌린다 — 상속된 값까지 지워 "스폰 밖" 을 재현한다.
fn run_cli_without_credentials(args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(args)
        .env_remove("ENGRAM_TOKEN")
        .env_remove("ENGRAM_CONTROL_URL")
        .env_remove(MAIL_MARKER_ENV)
        .output()
        .expect("spawn engram CLI");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// ★help 의 설계 속성 pin: 크레덴셜 검사보다 **먼저** 답한다★ — 이게 깨지면 아직 스폰되지 않은(또는 env 가
///   비어 있는) 에이전트가 표면을 배울 방법이 사라져 "발견" 이 성립하지 않는다. 같은 실행에서 우편 동사는
///   `NO_TOKEN` 으로 끝나는 것까지 함께 본다 — 그래야 "env 검사가 실제로 있고, help 만 그 앞에 있다" 가
///   증명된다(둘 다 0 이면 검사 자체가 사라진 것이고, 이 테스트만으론 못 가른다).
/// ★평문 pin★: exit 0 + JSON 봉투(`{"status":…}`)로 바꿔도 substring 단언은 통과한다 — 형태를 직접 못박는다.
#[test]
fn engram_help_answers_before_any_credential_check_and_prints_plain_text() {
    for args in [
        vec!["help"],
        vec![],
        vec!["--help"],
        vec!["-h"],
        vec!["help", "mail"],
        vec!["mail", "--help"],
        vec!["mail", "-h"],
    ] {
        let (stdout, code) = run_cli_without_credentials(&args);
        assert_eq!(
            code, 0,
            "크레덴셜 없이도 help 는 성공해야({args:?}): {stdout}"
        );
        assert!(!stdout.trim().is_empty(), "help 본문이 비었다({args:?})");
        assert!(
            !stdout.trim_start().starts_with('{'),
            "help 는 JSON 봉투가 아니라 평문이어야({args:?}): {stdout}"
        );
        assert!(
            serde_json::from_str::<serde_json::Value>(stdout.trim()).is_err(),
            "help stdout 이 JSON 으로 파싱되면 안 된다({args:?}): {stdout}"
        );
        assert!(
            stdout.contains(CLI_EXE_NAME),
            "사용법이 실행파일 이름을 그대로 보여야({args:?}): {stdout}"
        );
    }

    // 대조군 — 같은 조건에서 우편 동사는 env 검사에 걸린다(= 검사가 실재하고 help 만 그 앞에 있다).
    let (stdout, code) = run_cli_without_credentials(&["mail", "pending"]);
    assert_eq!(code, 1, "크레덴셜 없는 우편 동사는 실패: {stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout json: {e} — {stdout}"));
    assert_eq!(v["code"], "NO_TOKEN", "{stdout}");
}

/// `engram mail --help` = `engram help mail`(같은 화면). 두 철자가 갈리면 LLM 이 배운 대로 쳤을 때
/// 화면이 달라진다.
#[test]
fn conventional_help_spellings_render_the_same_screens() {
    let (canonical_root, _) = run_cli_without_credentials(&["help"]);
    for alias in [vec!["--help"], vec!["-h"], vec![]] {
        let (out, code) = run_cli_without_credentials(&alias);
        assert_eq!(code, 0);
        assert_eq!(out, canonical_root, "계열 목록 화면이 같아야: {alias:?}");
    }
    let (canonical_mail, _) = run_cli_without_credentials(&["help", "mail"]);
    for alias in [vec!["mail", "--help"], vec!["mail", "-h"]] {
        let (out, code) = run_cli_without_credentials(&alias);
        assert_eq!(code, 0);
        assert_eq!(out, canonical_mail, "계열 help 화면이 같아야: {alias:?}");
    }
    assert_ne!(canonical_root, canonical_mail, "두 화면은 서로 달라야");
}

#[test]
fn engram_unknown_group_and_bare_flags_are_argument_errors_without_touching_the_network() {
    // 옛 표기(계열 없는 플래그·동사)를 그대로 치면 발송으로 흐르지 않고 인자 오류로 끝난다.
    for args in [
        vec!["wat"],
        vec!["help", "wat"],
        vec!["--to", "bob", "--body", "hi"],
        vec!["pending"],
        vec!["status", "m-1"],
        // help 토큰이 값 자리에 온 경우 — `status --help` 는 예전에 `--help` 를 메시지 id 로 **조회**해
        //   실제 왕복을 했다. 이 목록에 있다는 것 자체가 "네트워크를 안 탄다" 는 주장이다(포트 0).
        vec!["mail", "status", "--help"],
        vec!["mail", "status", "-h"],
        vec!["mail", "pending", "--help"],
        vec!["mail", "send", "--help"],
        // help + 잔여 인자 — exit 0 으로 삼키면 편지가 성공 코드와 함께 사라진다.
        vec!["mail", "--help", "--to", "bob", "--body", "hi"],
        vec!["--help", "mail", "--to", "bob"],
        // help 뒤에 또 help 토큰 — 예전엔 exit 0 으로 root help 를 냈다(규칙 위반).
        vec!["help", "--help"],
        vec!["--help", "help"],
        // hint 가 자기 자신을 무효한 명령으로 되받던 자리.
        vec!["help", "--to", "bob"],
        // 중복 값 플래그 — 조용한 유실 대신 반려.
        vec![
            "mail", "send", "--to", "a", "--body", "first", "--body", "second",
        ],
        // 값을 빠뜨린 값 플래그 — 예전엔 다음 플래그를 본문으로 삼켜 파이프 본문을 버렸다.
        vec!["mail", "send", "--to", "bob", "--body", "--body-stdin"],
        vec!["mail", "send", "--to", "--body", "hi"],
    ] {
        let (stdout, code) = run_cli(UNREACHABLE_URL, &args, None);
        assert_eq!(code, 1, "인자 오류 → exit 1({args:?}): {stdout}");
        let v: serde_json::Value =
            serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("stdout json: {e}"));
        assert_eq!(v["code"], "BAD_ARGS", "{args:?} → BAD_ARGS: {stdout}");
        assert!(
            v["hint"].as_str().unwrap_or_default().contains("help"),
            "hint 가 help 로 안내해야({args:?}): {stdout}"
        );
        // ★안내한 명령이 실제로 도는지까지 본다★: 예전엔 `engram help --to bob` 이 "run `engram help --to`"
        //   를 제안했다 — 그 자체가 또 인자 오류라, 시키는 대로 한 호출자가 같은 벽에 다시 부딪힌다.
        //   "hint 에 help 라는 낱말이 있다" 만 보면 그런 제안도 통과한다.
        let suggestions = backticked_help_commands(v["hint"].as_str().unwrap_or_default());
        // ★공집합이면 이 테스트는 아무것도 단언하지 않은 채 통과한다★ — 커버리지처럼 읽히는 빈 테스트를
        //   만들지 않으려면 "적어도 하나는 뽑혔다" 를 먼저 못박아야 한다.
        assert!(
            !suggestions.is_empty(),
            "hint 에 실행 가능한 help 명령이 하나도 없다({args:?}): {stdout}"
        );
        for suggested in suggestions {
            let argv: Vec<&str> = suggested.split_whitespace().skip(1).collect();
            let (out, code) = run_cli_without_credentials(&argv);
            assert_eq!(
                code, 0,
                "hint 가 제안한 명령이 실제로 돌아야({args:?}): `{suggested}` → {out}"
            );
        }
    }
}

/// hint 안의 백틱 구간 중 **help 호출**만 골라 낸다(예시 발송 명령은 자리표시자라 실행 대상이 아니다).
fn backticked_help_commands(hint: &str) -> Vec<String> {
    hint.split('`')
        .skip(1)
        .step_by(2)
        .filter(|s| s.starts_with(&format!("{CLI_EXE_NAME} help")))
        .map(|s| s.to_string())
        .collect()
}

#[test]
fn engram_mail_send_rejects_body_and_body_stdin_together_without_touching_the_network() {
    let (stdout, code) = run_cli(
        "http://127.0.0.1:0",
        &[
            "mail",
            "send",
            "--to",
            "bob",
            "--body",
            "hi",
            "--body-stdin",
        ],
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

// ── ADR-0155/0156: 발견(`commands`)과 전체 이름 호출 — 프로세스 레벨 ────────────────────────

/// 데몬 자기 표가 내는 것과 **같은 모양**의 스키마 항목(`spec_item_json`) — 목록의 `help` 칸에 통째로 실린다.
fn schema_blob(
    name: &str,
    summary: &str,
    args: serde_json::Value,
    ok: serde_json::Value,
) -> String {
    serde_json::json!({
        "name": name,
        "effect": "Write",
        "since": 1,
        "summary": summary,
        "args": args,
        "ok": ok,
        "errors": ["NOT_FOUND", "INVALID_ARGUMENT", "INTERNAL"],
    })
    .to_string()
}

fn catalog_row(name: &str, help: &str, callable: bool) -> serde_json::Value {
    serde_json::json!({ "name": name, "help": help, "callable": callable })
}

fn catalog_response(rows: Vec<serde_json::Value>) -> &'static str {
    let body = serde_json::json!({ "commands": rows }).to_string();
    let s = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    Box::leak(s.into_boxed_str())
}

/// `slot.assign` 픽스처의 인자 스키마 — 타입 갈래를 한 명령에 모아 둔다.
fn slot_args() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "index": { "type": "integer" },
            "sticky": { "type": "boolean" },
            "name": { "type": "string" },
            // ★`help` 라는 이름의 **불리언** 인자★: 이 칸이 있어야 "사용법을 물었더니 Write 가 돌았다" 는
            //   갈래를 재현할 수 있다(리뷰 A). 픽스처에서 빼면 그 회귀가 다시 보이지 않는다.
            "help": { "type": "boolean" },
            "mode": { "anyOf": [{ "enum": ["wide", "tall"] }, { "type": "null" }] }
        },
        "required": ["index", "name"]
    })
}

/// 목록 픽스처 — 한 응답에 갈래를 모은다: 우리가 아는 스키마 · **JSON 이 아닌** help · `callable:false` ·
/// nullable 필수 칸 · 우편 이름(그 계열이 화면에 새는지 재는 재료).
fn sample_catalog() -> &'static str {
    catalog_response(vec![
        catalog_row(
            "agent.list",
            &schema_blob(
                "agent.list",
                "every agent: name, state, folder, parent",
                serde_json::json!({ "type": "object", "properties": {}, "required": [] }),
                serde_json::json!({ "type": "object", "properties": { "agents": { "type": "array" } }, "required": ["agents"] }),
            ),
            true,
        ),
        catalog_row(
            "agent.move",
            &schema_blob(
                "agent.move",
                "re-parent an agent",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "target": { "type": "string" },
                        "parent": { "anyOf": [{ "type": "string" }, { "type": "null" }] }
                    },
                    "required": ["target", "parent"]
                }),
                serde_json::json!({ "type": "object", "properties": { "agent_id": { "type": "string" } }, "required": ["agent_id"] }),
            ),
            true,
        ),
        catalog_row(
            "slot.assign",
            &schema_blob(
                "slot.assign",
                "put an agent in a slot",
                slot_args(),
                // ★반환 칸 이름이 명령 이름의 부분문자열이면 안 된다★: `slot` 은 `slot.assign` 안에 이미
                //   있어서, 그 낱말로 단언하면 반환 구획이 통째로 사라져도 초록이다.
                serde_json::json!({ "type": "object", "properties": { "placed_at": { "type": "integer" } }, "required": ["placed_at"] }),
            ),
            true,
        ),
        // 클라이언트가 등록한 자유 텍스트 — JSON 이 아니다. 이 한 줄이 목록 전체를 가라앉히면 안 된다.
        catalog_row(
            "tab.create",
            "opens a tab in the dashboard\nsecond line nobody should see in the listing",
            false,
        ),
        // ★우편 계열 이름이 표에 실려 온 경우★: 이 행이 없으면 아래 누출 테스트는 어떤 구현에서도 통과한다
        //   (픽스처에 그 낱말이 아예 없으니 단언이 늘 참이다).
        catalog_row(
            &format!("{CLI_GROUP_MAIL}.send"),
            &schema_blob(
                &format!("{CLI_GROUP_MAIL}.send"),
                "post a note to a teammate",
                serde_json::json!({ "type": "object", "properties": { "to": { "type": "string" } }, "required": ["to"] }),
                serde_json::json!({ "type": "object", "properties": { "id": { "type": "string" } }, "required": ["id"] }),
            ),
            true,
        ),
    ])
}

#[test]
fn engram_commands_lists_every_name_with_its_summary_and_marks_what_it_cannot_run() {
    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog()]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["commands"], None);
    let requests = stub.join().expect("stub join");

    assert_eq!(code, 0, "목록 렌더 성공 → exit 0: {stdout}");
    assert_eq!(requests.len(), 1, "목록은 한 왕복: {requests:?}");
    assert!(
        requests[0].starts_with("POST /control/commands HTTP/1.1"),
        "발견 라우트로: {}",
        requests[0]
    );
    assert!(
        requests[0].contains("Authorization: Bearer test-token"),
        "토큰을 실어야: {}",
        requests[0]
    );

    for (name, summary) in [
        ("agent.list", "every agent: name, state, folder, parent"),
        ("slot.assign", "put an agent in a slot"),
    ] {
        let row = stdout
            .lines()
            .find(|l| l.trim_start().starts_with(name))
            .unwrap_or_else(|| panic!("{name} 이 목록에 없다: {stdout}"));
        assert!(row.contains(summary), "이름 옆에 요약이 와야: {row}");
    }
    // ★못 읽는 블롭이 목록을 가라앉히지 않는다★ — 이름은 남고, 남길 수 있는 만큼(첫 줄)이 요약이 된다.
    let free_text = stdout
        .lines()
        .find(|l| l.trim_start().starts_with("tab.create"))
        .unwrap_or_else(|| panic!("파싱 안 되는 help 가 줄을 통째로 없앴다: {stdout}"));
    assert!(
        free_text.contains("opens a tab"),
        "자유 텍스트의 첫 줄이 요약: {free_text}"
    );
    assert!(
        !free_text.contains("second line"),
        "한 명령 = 한 줄이어야: {free_text}"
    );
    // ★이 행은 **오늘 데몬이 내지 않는 값**을 스크립트한 것이다★ — 목록을 합치는 두 출처가 둘 다
    //   도달 가능해졌으므로(ADR-0160) 진짜 데몬은 지금 모든 행을 `callable:true` 로 낸다. 그래도
    //   이 갈래를 재는 이유는 그 칸이 상수가 아니기 때문이다 — 거짓을 받는 날 CLI 가 그걸 화면에
    //   보여 줘야 한다. ★단 문구가 `UNSUPPORTED` 를 약속하면 안 된다★: 그 코드를 내던 생산자는
    //   사라졌고(`catalog::not_mine` 삭제), 지금 그 자리에 오는 거절은 데몬이 정하는 다른 코드다.
    assert!(
        stdout.contains("cannot run"),
        "부를 수 없는 이름이라는 사실이 화면에 보여야: {stdout}"
    );
    assert!(
        !stdout.contains("UNSUPPORTED"),
        "지워진 오류 코드를 약속하고 있다: {stdout}"
    );
    // 발견 화면은 렌더된 평문이다 — 원문 JSON 을 그대로 흘리지 않는다.
    assert!(
        !stdout.trim_start().starts_with('{'),
        "목록은 평문 화면: {stdout}"
    );
}

#[test]
fn engram_commands_with_a_name_shows_its_arguments_return_shape_and_errors() {
    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog()]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["commands", "slot.assign"], None);
    let requests = stub.join().expect("stub join");

    assert_eq!(code, 0, "상세 렌더 성공 → exit 0: {stdout}");
    assert_eq!(requests.len(), 1, "상세도 목록 한 왕복: {requests:?}");
    for token in [
        "slot.assign",
        "put an agent in a slot",
        "--index <integer>",
        "--name <string>",
        "--sticky <true|false>",
        // 옵션 칸이라 null 표기가 붙지 않는다 — 붙이면 화면이 안 되는 사용법을 광고한다(M).
        "--mode <wide|tall>",
        "required",
        "optional",
        // ★반환 칸은 명령 이름의 부분문자열이 아닌 낱말로 잰다★: 예전엔 `slot` 으로 재서, 반환 구획이
        //   통째로 사라져도 `slot.assign` 이 그 낱말을 담고 있어 초록이었다.
        "placed_at",
        "NOT_FOUND",
        "INVALID_ARGUMENT",
    ] {
        assert!(stdout.contains(token), "{token} 이 상세 화면에: {stdout}");
    }
    let returns = stdout
        .lines()
        .position(|l| l.starts_with("Returns —"))
        .unwrap_or_else(|| panic!("반환 구획이 없다: {stdout}"));
    assert!(
        stdout
            .lines()
            .skip(returns)
            .any(|l| l.trim_start().starts_with("placed_at")),
        "반환 칸이 그 구획 안에 있어야: {stdout}"
    );
}

/// ★A. 사용법 요청이 명령을 실행하면 안 된다★: `slot.assign` 픽스처는 `help` 라는 이름의 **불리언** 인자를
///   선언한다 — 예전 파서는 `--help` 를 그 칸으로 읽어 `{"help":true}` 를 `/control/call` 로 보냈다.
///   사용법을 물었더니 Write 가 돈 것이다. 세 철자 모두 상세 화면으로 가고, 호출 라우트는 한 번도 안 열린다.
#[test]
fn asking_a_full_name_for_help_renders_its_detail_and_never_calls_it() {
    for token in ["--help", "-h", "help"] {
        let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog()]);
        let url = format!("http://{host}:{port}");
        let (stdout, code) = run_cli(&url, &["slot.assign", token], None);
        let requests = stub.join().expect("stub join");

        assert_eq!(code, 0, "사용법 요청은 성공 종료({token}): {stdout}");
        assert_eq!(requests.len(), 1, "목록 한 왕복뿐({token}): {requests:?}");
        assert!(
            !requests.iter().any(|r| r.contains("POST /control/call")),
            "사용법 요청이 명령을 실행했다({token}): {requests:?}"
        );
        assert!(
            stdout.contains("--index <integer>"),
            "상세 화면이어야({token}): {stdout}"
        );
    }
    // ★L. 한 칸 옆으로 밀린 help 도 인자가 아니다★: 예전엔 첫 자리만 봐서 `--index 1 --help` 가 그 불리언
    //   칸으로 바인딩돼 `{"index":1,"help":true}` 가 실제로 POST 됐다. 어느 자리든 명령은 돌지 않는다 —
    //   그리고 이 케이스들은 목적지가 포트 0 이라 **네트워크를 타지도 않는다**.
    for args in [
        vec!["slot.assign", "--index", "1", "--help"],
        vec!["slot.assign", "--index", "1", "-h"],
        vec!["slot.assign", "--sticky", "--help"],
        vec!["slot.assign", "--help", "--index", "1"],
    ] {
        let (stdout, code) = run_cli(UNREACHABLE_URL, &args, None);
        assert_eq!(code, 1, "밀린 help 는 인자 오류({args:?}): {stdout}");
        let v: serde_json::Value = serde_json::from_str(stdout.trim())
            .unwrap_or_else(|e| panic!("봉투 JSON({args:?}): {e} — {stdout}"));
        assert_eq!(
            v["code"], "BAD_ARGS",
            "왕복도 실행도 없어야({args:?}): {stdout}"
        );
    }
    // 단독 호출과 그 규칙이 갈리지 않는다.
    let (stdout, code) = run_cli(
        UNREACHABLE_URL,
        &["slot.assign", "--help", "--index", "1"],
        None,
    );
    assert_eq!(code, 1, "잔여 인자가 붙은 help 는 인자 오류: {stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["code"], "BAD_ARGS", "{stdout}");
}

/// ★B. 부모를 떼는 형태가 이 표면에도 있어야 한다★: `parent` 는 nullable 이면서 필수라, null 을 실을 방법이
///   없으면 문자열 `"none"` 이 나가 NOT_FOUND 가 되거나 하필 그 이름의 에이전트 밑으로 들어간다.
///   옛 계열의 `detaching_sends_an_explicit_null_parent_not_an_absent_key` 와 **같은 사실**을 새 입구에서 잰다.
#[test]
fn detaching_over_the_full_name_surface_sends_a_real_json_null() {
    let ok = ok_response(r#"{"agent_id":"a-1"}"#);
    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog(), ok]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(
        &url,
        &["agent.move", "--target", "qa", "--parent", "none"],
        None,
    );
    let requests = stub.join().expect("stub join");

    assert_eq!(code, 0, "호출 성공 → exit 0: {stdout}");
    assert_eq!(requests.len(), 2, "{requests:?}");
    let sent = request_body(&requests[1]);
    assert_eq!(
        sent["args"]["parent"],
        serde_json::Value::Null,
        "루트로 떼는 요청은 null 이어야: {sent}"
    );
    assert_eq!(sent["args"]["target"], serde_json::json!("qa"));
    // 붙이는 쪽은 문자열 그대로 — 두 뜻이 한 낱말에 겹쳐 있으므로 반대 방향도 함께 못박는다.
    let ok = ok_response(r#"{"agent_id":"a-1"}"#);
    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog(), ok]);
    let url = format!("http://{host}:{port}");
    let (_stdout, code) = run_cli(
        &url,
        &["agent.move", "--target", "qa", "--parent", "lead"],
        None,
    );
    let requests = stub.join().expect("stub join");
    assert_eq!(code, 0);
    assert_eq!(
        request_body(&requests[1])["args"]["parent"],
        serde_json::json!("lead")
    );
}

/// ★M. 그 낱말이 **옵션 칸까지** 먹으면 조용한 사고가 된다★: `agent.spawn --cwd none` 이 `{"cwd":null}` 로
///   나가면 데몬 기본 폴더에 만들어지는데 그 응답은 `cwd` 를 안 실어, 다른 폴더가 쓰였다는 사실이 어디에도
///   보이지 않는다. 옵션 칸에서 "값 없음" 은 플래그를 빼는 것이고, 옛 계열도 같은 입력을 문자열로 보낸다.
#[test]
fn the_detach_word_reaches_the_daemon_as_a_string_in_an_optional_slot() {
    let ok = ok_response(r#"{"placed_at":1}"#);
    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog(), ok]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(
        &url,
        &[
            "slot.assign",
            "--index",
            "1",
            "--name",
            "qa",
            "--mode",
            "none",
        ],
        None,
    );
    let requests = stub.join().expect("stub join");

    assert_eq!(code, 0, "{stdout}");
    let sent = request_body(&requests[1]);
    assert_eq!(
        sent["args"]["mode"],
        serde_json::json!("none"),
        "옵션 칸은 nullable 이어도 문자열로 나가야: {sent}"
    );
}

/// ★N. 버린 행을 "목록에 없다" 로 보고하면 안 된다★: `callable` 이 빠진 등록 하나로 그 행이 사라지는데,
///   기계가 읽는 봉투에 부재가 단정되면 호출자는 실재하는 명령을 영구히 포기한다.
#[test]
fn a_call_for_a_name_whose_row_was_dropped_does_not_claim_the_name_is_absent() {
    let poisoned = catalog_response(vec![
        catalog_row(
            "agent.list",
            &schema_blob(
                "agent.list",
                "every agent",
                serde_json::json!({ "type": "object", "properties": {}, "required": [] }),
                serde_json::json!({ "type": "object", "properties": { "agents": { "type": "array" } }, "required": ["agents"] }),
            ),
            true,
        ),
        // 등록이 `callable` 을 빼먹었다 — 그 행만 읽히지 않는다.
        serde_json::json!({ "name": "slot.assign", "help": "x" }),
    ]);
    let (host, port, stub) = spawn_scripted_stub(vec![poisoned]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["slot.assign", "--index", "1"], None);
    let requests = stub.join().expect("stub join");

    assert_eq!(code, 1, "부를 수는 없다 → exit 1: {stdout}");
    assert_eq!(requests.len(), 1, "호출은 나가지 않는다: {requests:?}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["code"], "UNKNOWN_COMMAND", "{stdout}");
    let hint = v["hint"].as_str().unwrap_or_default();
    assert!(
        !hint.contains("does not list"),
        "읽히지 않은 행을 부재로 단정하면 안 된다: {hint}"
    );
    assert!(hint.contains("unreadable"), "사실을 말해야: {hint}");

    // 대조군 — 버린 행이 없으면 부재를 그대로 단정한다(위 문구가 늘 나오는 게 아니라는 증명).
    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog()]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["nope.nope", "--x", "1"], None);
    let _ = stub.join().expect("stub join");
    assert_eq!(code, 1);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert!(
        v["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("does not list"),
        "{stdout}"
    );
}

/// ★D. 읽을 수 없는 행 하나가 표면 전체를 죽이면 안 된다★: 등록 하나가 빈 이름을 내면 예전에는 목록·상세·
///   **모든 호출**이 그 클라이언트가 끊길 때까지 exit 2 였다.
#[test]
fn a_single_unreadable_catalog_row_does_not_take_the_surface_down() {
    let poisoned = catalog_response(vec![
        catalog_row(
            "agent.list",
            &schema_blob(
                "agent.list",
                "every agent",
                serde_json::json!({ "type": "object", "properties": {}, "required": [] }),
                serde_json::json!({ "type": "object", "properties": { "agents": { "type": "array" } }, "required": ["agents"] }),
            ),
            true,
        ),
        // 이름이 빈 등록 — 명부는 길이만 재므로 실제로 올 수 있다.
        serde_json::json!({ "name": "", "help": "h", "callable": true }),
    ]);
    let (host, port, stub) = spawn_scripted_stub(vec![poisoned]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["commands"], None);
    let _ = stub.join().expect("stub join");
    assert_eq!(code, 0, "읽을 수 있는 행이 있으면 목록은 선다: {stdout}");
    assert!(stdout.contains("agent.list"), "{stdout}");

    // 같은 상태에서 호출도 살아 있어야 한다 — 예전엔 이 경로가 통째로 막혔다.
    let poisoned = catalog_response(vec![
        catalog_row(
            "agent.list",
            &schema_blob(
                "agent.list",
                "every agent",
                serde_json::json!({ "type": "object", "properties": {}, "required": [] }),
                serde_json::json!({ "type": "object", "properties": { "agents": { "type": "array" } }, "required": ["agents"] }),
            ),
            true,
        ),
        serde_json::json!({ "name": "", "help": "h", "callable": true }),
    ]);
    let (host, port, stub) = spawn_scripted_stub(vec![poisoned, ok_response(r#"{"agents":[]}"#)]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["agent.list"], None);
    let requests = stub.join().expect("stub join");
    assert_eq!(code, 0, "호출도 살아 있어야: {stdout}");
    assert_eq!(requests.len(), 2, "{requests:?}");
}

#[test]
fn engram_commands_for_a_name_the_catalog_does_not_list_is_a_failure_not_a_crash() {
    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog()]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["commands", "nope.nope"], None);
    let _ = stub.join().expect("stub join");

    assert_eq!(code, 1, "표에 없는 이름 → exit 1: {stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["code"], "UNKNOWN_COMMAND", "{stdout}");
}

/// ★값의 생김새가 아니라 **선언된 타입**이 정한다★: `--name 123` 은 문자열 칸이므로 문자열로 실려야 하고,
///   `--index 3` 은 정수 칸이므로 수로 실려야 한다. 한쪽만 보면 "전부 문자열" 이나 "전부 JSON 파싱" 도 통과한다.
#[test]
fn engram_invoking_a_full_name_coerces_each_value_to_its_declared_type() {
    let ok = ok_response(r#"{"placed_at":3}"#);
    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog(), ok]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(
        &url,
        &[
            "slot.assign",
            "--index",
            "3",
            "--name",
            "123",
            "--sticky",
            "--mode",
            "wide",
        ],
        None,
    );
    let requests = stub.join().expect("stub join");

    assert_eq!(code, 0, "호출 성공 → exit 0: {stdout}");
    assert_eq!(
        requests.len(),
        2,
        "스키마 조회 + 호출 두 왕복: {requests:?}"
    );
    assert!(
        requests[0].starts_with("POST /control/commands HTTP/1.1"),
        "{}",
        requests[0]
    );
    assert!(
        requests[1].starts_with("POST /control/call HTTP/1.1"),
        "{}",
        requests[1]
    );
    let sent = request_body(&requests[1]);
    assert_eq!(sent["name"], "slot.assign");
    assert_eq!(
        sent["args"]["index"],
        serde_json::json!(3),
        "정수 칸: {sent}"
    );
    assert_eq!(
        sent["args"]["name"],
        serde_json::json!("123"),
        "문자열 칸은 숫자처럼 생겨도 문자열: {sent}"
    );
    assert_eq!(
        sent["args"]["sticky"],
        serde_json::json!(true),
        "불리언 칸은 값 없이 서고 값을 안 삼킨다: {sent}"
    );
    assert_eq!(sent["args"]["mode"], serde_json::json!("wide"), "{sent}");
    // 호출 응답은 데몬 body 그대로 — CLI 가 다시 쓰지 않는다.
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(stdout.trim()).expect("stdout json"),
        serde_json::json!({ "placed_at": 3 })
    );
}

/// ★F. 호출 응답도 **선언된 반환**으로 잰다★: 관대한 판정기를 쓰면 여기서 exit 0 이 나는데, 같은 응답에
///   옛 계열은 exit 2 를 낸다. 한 명령이 입구에 따라 반대 판정을 받으면 호출자는 일어나지 않은 일을 사실로
///   기록한다. 재료(`ok.required`)는 첫 왕복에 이미 왔다.
#[test]
fn a_call_answered_without_the_declared_return_fields_exits_two_not_zero() {
    for body in [r#"{"status":"ok"}"#, r#"{}"#, r#"[]"#] {
        let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog(), ok_response(body)]);
        let url = format!("http://{host}:{port}");
        let (stdout, code) = run_cli(&url, &["slot.assign", "--index", "1", "--name", "qa"], None);
        let requests = stub.join().expect("stub join");
        assert_eq!(requests.len(), 2, "호출은 나갔다: {requests:?}");
        assert_eq!(
            code, 2,
            "증거 없는 2xx → exit 2(재시도 아니라 보고 대상): {body} → {stdout}"
        );
    }
    // 대조군 — 선언된 칸이 다 실린 응답은 그대로 성공이다(위 판정이 거짓 경보가 아니라는 증명).
    let (host, port, stub) =
        spawn_scripted_stub(vec![sample_catalog(), ok_response(r#"{"placed_at":1}"#)]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["slot.assign", "--index", "1", "--name", "qa"], None);
    let _ = stub.join().expect("stub join");
    assert_eq!(code, 0, "{stdout}");
}

#[test]
fn engram_invoking_with_an_unparsable_value_never_reaches_the_call_route() {
    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog()]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["slot.assign", "--index", "three"], None);
    let requests = stub.join().expect("stub join");

    assert_eq!(code, 1, "옮길 수 없는 값 → exit 1: {stdout}");
    assert_eq!(requests.len(), 1, "호출 라우트는 안 두드린다: {requests:?}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["code"], "BAD_ARGS", "{stdout}");
    assert!(
        v["hint"].as_str().unwrap_or_default().contains("integer"),
        "선언된 타입을 알려야: {stdout}"
    );
}

/// ★모르는 플래그는 **로컬에서** 끝난다★: 데몬도 같은 판정을 하지만 그 왕복은 부작용 있는 입구를 한 번 더
///   두드리면서 호출자에게 아무것도 더 주지 않는다. 문구엔 고칠 재료(선언된 칸 전량)가 실려야 한다.
#[test]
fn engram_invoking_with_an_unknown_flag_fails_locally_with_the_declared_list() {
    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog()]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["slot.assign", "--index", "1", "--nope", "x"], None);
    let requests = stub.join().expect("stub join");

    assert_eq!(code, 1, "모르는 플래그 → exit 1: {stdout}");
    assert_eq!(
        requests.len(),
        1,
        "스키마 조회까지만 — 호출은 안 나간다: {requests:?}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["code"], "BAD_ARGS", "{stdout}");
    let hint = v["hint"].as_str().unwrap_or_default();
    for declared in ["--index", "--name", "--sticky", "--mode"] {
        assert!(
            hint.contains(declared),
            "선언된 칸 전량이 실려야({declared}): {hint}"
        );
    }
}

/// ★E. 어느 요청이 실패했는지 말해야 한다★: 목록 조회가 실패하면 stdout 에는 그 라우트의 body 가 찍히는데,
///   그것이 명령 자신의 반려와 바이트 단위로 같다 — 호출자(LLM)는 멀쩡한 인자를 고치기 시작한다.
#[test]
fn a_catalog_failure_during_a_call_says_which_request_failed() {
    let refusal = ok_response(r#"{"status":"error","code":"INTERNAL","hint":"roster is wedged"}"#);
    let (host, port, stub) = spawn_scripted_stub(vec![refusal]);
    let url = format!("http://{host}:{port}");
    let (stdout, stderr, code) = run_cli_streams(&url, &["agent.list"]);
    let requests = stub.join().expect("stub join");

    assert_eq!(code, 1, "반려 → exit 1: {stdout}");
    assert_eq!(requests.len(), 1, "호출은 나가지 않았다: {requests:?}");
    assert!(
        stdout.contains("INTERNAL"),
        "실패한 body 는 그대로 흘러야: {stdout}"
    );
    assert!(
        stderr.contains("/control/commands") && stderr.contains("agent.list"),
        "어느 요청이 실패했는지 밝혀야: {stderr}"
    );
    assert!(
        stderr.contains("never called"),
        "명령 자신은 안 불렸다는 사실을 말해야: {stderr}"
    );

    // 대조군 — `commands` 자신의 실패에는 그 줄이 붙지 않는다(그때는 어느 요청인지 물을 여지가 없다).
    let refusal = ok_response(r#"{"status":"error","code":"INTERNAL","hint":"roster is wedged"}"#);
    let (host, port, stub) = spawn_scripted_stub(vec![refusal]);
    let url = format!("http://{host}:{port}");
    let (_stdout, stderr, code) = run_cli_streams(&url, &["commands"]);
    let _ = stub.join().expect("stub join");
    assert_eq!(code, 1);
    assert!(!stderr.contains("never called"), "{stderr}");
}

/// stdout·stderr 를 **따로** 받는다 — 어느 요청이 실패했나 같은 진단은 stderr 로 나가고, 기존 헬퍼는 그것을 버린다.
fn run_cli_streams(control_url: &str, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(args)
        .env("ENGRAM_TOKEN", "test-token")
        .env("ENGRAM_CONTROL_URL", control_url)
        .env_remove(MAIL_MARKER_ENV)
        .output()
        .expect("spawn engram CLI");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// ★부를 수 없다는 판정의 정본은 데몬이다★: 목록이 `callable:false` 라 해도 CLI 가 미리 끊지 않는다 —
///   끊으면 그 거절이 관측되지 않는다.
///
/// ★거절 코드는 **지금 데몬이 실제로 내는 것**이어야 한다★: 예전 이 시험은 `UNSUPPORTED` 를
///   스크립트했는데, 그 코드를 내던 생산자가 중계 도입으로 사라졌다(ADR-0160). 스크립트된 스텀은
///   무엇이든 되돌려 주므로 그 시험은 **어떤 구현에서도 초록**이었다. 지금 그 이름을 부르면 중계되고,
///   주인이 마감까지 안 답하면 `TIMEOUT` 이 돌아온다.
#[test]
fn engram_invoking_a_name_this_entrance_cannot_run_surfaces_the_daemons_refusal() {
    let refusal = ok_response(
        r#"{"status":"error","code":"TIMEOUT","hint":"'tab.create' was handed to its owner but no outcome came back before the deadline"}"#,
    );
    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog(), refusal]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["tab.create", "--title", "x"], None);
    let requests = stub.join().expect("stub join");

    assert_eq!(code, 1, "데몬 반려 → exit 1: {stdout}");
    assert_eq!(
        requests.len(),
        2,
        "호출은 실제로 나가야(로컬 차단이 아니다): {requests:?}"
    );
    assert!(
        requests[1].starts_with("POST /control/call HTTP/1.1"),
        "{}",
        requests[1]
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["code"], "TIMEOUT", "거절 사유가 그대로 흘러야: {stdout}");
    // ★호출은 자기 번호를 실어 보낸다★(ADR-0161 결정 2) — 안 실으면 데몬이 요청마다 새로 발급해
    //   재실행 방지 좌석이 구조적으로 한 번도 안 걸린다.
    assert!(
        requests[1].contains("request_id"),
        "호출 바디에 요청 번호가 실려야: {}",
        requests[1]
    );
}

/// 2xx 인데 **봉투**를 목록으로 읽을 수 없으면 반려(1)가 아니라 보고 대상(2)이다 — 기존 3분법 그대로다.
///
/// ★행 하나가 깨진 것은 여기 들지 않는다★: 그건 살릴 수 있는 갈래라 목록이 그대로 서고(위
///   `a_single_unreadable_catalog_row_does_not_take_the_surface_down`), 버린 수만 stderr 로 나간다.
#[test]
fn engram_commands_reports_an_unreadable_catalog_envelope_as_two_not_one() {
    for body in [
        r#"{}"#,
        r#"{"commands":"not-an-array"}"#,
        r#"{"commands":7}"#,
    ] {
        let (host, port, stub) = spawn_scripted_stub(vec![ok_response(body)]);
        let url = format!("http://{host}:{port}");
        let (stdout, code) = run_cli(&url, &["commands"], None);
        let _ = stub.join().expect("stub join");
        assert_eq!(code, 2, "읽을 수 없는 봉투 → exit 2 ({body}): {stdout}");
    }
    // 빈 목록은 결함이 아니다 — 표 슬롯이 비어 있는 데몬이 내는 정상적인 답이다.
    let (host, port, stub) = spawn_scripted_stub(vec![ok_response(r#"{"commands":[]}"#)]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["commands"], None);
    let _ = stub.join().expect("stub join");
    assert_eq!(code, 0, "빈 목록은 정상: {stdout}");

    // 버린 행은 침묵하지 않는다 — 찾던 이름이 그 안에 있었으면 다음 줄이 UNKNOWN_COMMAND 인데, 그 둘을
    //   잇는 실마리가 이 줄밖에 없다.
    let (host, port, stub) = spawn_scripted_stub(vec![ok_response(
        r#"{"commands":[{"name":"","help":"h","callable":true}]}"#,
    )]);
    let url = format!("http://{host}:{port}");
    let (_stdout, stderr, code) = run_cli_streams(&url, &["commands"]);
    let _ = stub.join().expect("stub join");
    assert_eq!(code, 0);
    assert!(
        stderr.contains("could not be read"),
        "버린 행 수를 알려야: {stderr}"
    );
}

#[test]
fn engram_commands_maps_rejections_and_transport_failures_to_one() {
    let rejected = ok_response(r#"{"status":"error","code":"INTERNAL","hint":"broken"}"#);
    let (host, port, stub) = spawn_scripted_stub(vec![rejected]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["commands"], None);
    let _ = stub.join().expect("stub join");
    assert_eq!(code, 1, "데몬 반려 → exit 1: {stdout}");
    assert!(
        stdout.contains("INTERNAL"),
        "실패한 목록의 body 는 그대로 흘러야: {stdout}"
    );

    let (stdout, code) = run_cli(UNREACHABLE_URL, &["commands"], None);
    assert_eq!(code, 1, "데몬 불통 → exit 1: {stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["status"], "error", "{stdout}");
}

/// 발견·호출 표면의 인자 오류도 옛 계열과 같은 규율 — **네트워크를 타지 않는다**(목적지 = 포트 0).
///
/// ★사유까지 단언한다★: 예전 판은 exit 1 + BAD_ARGS 만 봤는데, 이 케이스들은 **이 표면이 생기기 전에도**
///   전부 "모르는 계열" 로 반려됐다 — 즉 옛 바이너리에서도 초록이라 새 규칙을 하나도 지키지 않았다.
///   각 줄이 어떤 규칙에 걸리는지를 문구로 못박아야 그 규칙이 사라졌을 때 빨개진다.
#[test]
fn engram_catalog_and_call_argument_errors_never_touch_the_network() {
    for (args, expect) in [
        (vec!["commands", "agent.list", "extra"], "at most one"),
        (vec!["commands", "--help"], "not a command name"),
        (vec!["commands", "-h"], "not a command name"),
        // 호출 형태는 위치 인자를 받지 않는다 — 스키마를 봐도 답이 달라지지 않으므로 여기서 끊는다.
        (vec!["agent.list", "oops"], "is not a flag"),
        // 플래그 검사가 이름 검사보다 앞이다(점 달린 오타 플래그가 이름으로 읽히면 왕복을 낭비한다).
        (vec!["--nope.nope"], "not a flag"),
        // `<계열>.<동사>` 가 아닌 것은 표에 있을 수 없다 — 인증된 POST 를 낭비하지 않는다.
        (vec!["."], "neither side may be empty"),
        (vec![".."], "neither side may be empty"),
        (vec![".foo"], "neither side may be empty"),
        (vec!["foo."], "neither side may be empty"),
        (vec!["README.md", "--x", "1"], "unknown command"),
    ] {
        let (stdout, code) = run_cli(UNREACHABLE_URL, &args, None);
        assert_eq!(code, 1, "인자 오류 → exit 1({args:?}): {stdout}");
        let v: serde_json::Value =
            serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("stdout json: {e}"));
        let hint = v["hint"].as_str().unwrap_or_default();
        // `README.md` 는 형태가 멀쩡해 목록을 물어야 알 수 있다 — 그 줄만 연결 실패로 끝난다(왕복은 한 번).
        if expect == "unknown command" {
            assert_eq!(v["code"], "CONNECT_FAILED", "{args:?}: {stdout}");
            continue;
        }
        assert_eq!(v["code"], "BAD_ARGS", "{args:?} → BAD_ARGS: {stdout}");
        assert!(
            hint.contains(expect),
            "사유가 이 규칙을 말해야({args:?}, {expect:?}): {hint}"
        );
    }
}

/// ★static help 는 데몬 없이 답한다 — 그 성질은 새 표면이 생겨도 그대로다★: 새로 생긴 것은 **가리키는 한
///   줄**뿐이고, 그 화면이 데몬을 부르기 시작하면 크레덴셜 없는 프로세스는 표면을 배울 방법을 잃는다.
#[test]
fn the_static_help_points_at_the_catalog_without_fetching_it() {
    let (root, code) = run_cli_without_credentials(&["help"]);
    assert_eq!(code, 0, "크레덴셜 없이도 성공: {root}");
    assert!(
        root.contains(&format!("{CLI_EXE_NAME} commands")),
        "root help 가 발견 표면을 가리켜야: {root}"
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(root.trim()).is_err(),
        "help 는 평문이어야: {root}"
    );
    // 대조군 — 같은 조건에서 발견 동사 자체는 크레덴셜 검사에 걸린다(= help 만 그 앞에 있다).
    let (out, code) = run_cli_without_credentials(&["commands"]);
    assert_eq!(code, 1, "크레덴셜 없는 발견은 실패: {out}");
    let v: serde_json::Value = serde_json::from_str(out.trim()).expect("stdout json");
    assert_eq!(v["code"], "NO_TOKEN", "{out}");
}

/// ★새 표면의 **우리 문구**가 감춘 계열을 가르치면 안 된다★.
///
/// ★픽스처에 우편 이름이 실려 있는 것이 이 테스트의 요점이다★: 예전 판은 픽스처에 그 낱말이 아예 없어서
///   **어떤 구현에서도** 통과했다(단언이 늘 참). 이제 목록에 `mail.send` 가 오므로, 우편이라는 낱말이 화면에
///   나오는 자리는 **데몬이 보낸 그 행 하나뿐**이어야 한다 — 우리 chrome(머리글·구획 제목·안내)이 그 낱말을
///   더하면 줄 수가 늘어 빨개진다.
/// ★표가 실어 온 이름을 지우지는 않는다★: 무엇이 실리느냐는 데몬 정책이고(그 질문은 별도로 파킹돼 있다),
///   목록에서 빼면 발견이 "있다" 를 말하지 않게 된다 — 이 CLI 가 고칠 층이 아니다.
#[test]
fn the_catalog_surface_never_teaches_the_hidden_mail_group_in_its_own_words() {
    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog()]);
    let url = format!("http://{host}:{port}");
    let out = Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["commands"])
        .env("ENGRAM_TOKEN", "test-token")
        .env("ENGRAM_CONTROL_URL", &url)
        .env(MAIL_MARKER_ENV, MAIL_MARKER_OFF)
        .output()
        .expect("spawn engram CLI");
    let _ = stub.join().expect("stub join");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(out.status.code().unwrap_or(-1), 0, "{stdout}");
    let leaking: Vec<&str> = stdout
        .lines()
        .filter(|l| l.contains(CLI_GROUP_MAIL))
        .collect();
    assert_eq!(
        leaking.len(),
        1,
        "우편 낱말은 데몬이 보낸 행에만 있어야: {stdout}"
    );
    assert!(
        leaking[0]
            .trim_start()
            .starts_with(&format!("{CLI_GROUP_MAIL}.send")),
        "그 한 줄이 표가 실어 온 행이어야: {}",
        leaking[0]
    );

    let (hidden_root, code) = run_cli_with_marker(Some(MAIL_MARKER_OFF), &["help"]);
    assert_eq!(code, 0);
    assert!(
        hidden_root.contains("commands"),
        "발견 안내 줄은 표식과 무관하게 남는다: {hidden_root}"
    );
    assert!(
        !hidden_root.contains(CLI_GROUP_MAIL),
        "정적 화면은 여전히 감춘다: {hidden_root}"
    );
}

// ── ADR-0133: 우편 표식 — 목록만 가리고 실행은 가리지 않는다(프로세스 레벨) ─────────────────────

/// 표식 값을 명시해 돌린다. 크레덴셜은 주지 않는다 — 여기 케이스는 전부 파싱·렌더 단계에서 끝난다.
fn run_cli_with_marker(marker: Option<&str>, args: &[&str]) -> (String, i32) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_engram"));
    cmd.args(args)
        .env_remove("ENGRAM_TOKEN")
        .env_remove("ENGRAM_CONTROL_URL");
    match marker {
        Some(v) => cmd.env(MAIL_MARKER_ENV, v),
        None => cmd.env_remove(MAIL_MARKER_ENV),
    };
    let out = cmd.output().expect("spawn engram CLI");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// ★부재·모르는 값은 전부 보인다★: 사람이 스폰 밖 셸에서 여는 자리라 반쪽 사용법이 나오면 안 된다.
#[test]
fn only_the_explicit_off_marker_hides_the_mail_group_from_help() {
    for marker in [None, Some(MAIL_MARKER_ON), Some("nonsense"), Some("")] {
        let (out, code) = run_cli_with_marker(marker, &["help"]);
        assert_eq!(code, 0, "help 는 성공 종료({marker:?}): {out}");
        assert!(
            out.contains(CLI_GROUP_MAIL),
            "표식 {marker:?} 에서는 우편 계열이 보여야: {out}"
        );
    }
    let (hidden, code) = run_cli_with_marker(Some(MAIL_MARKER_OFF), &["help"]);
    assert_eq!(code, 0, "표식 off 여도 help 자체는 성공: {hidden}");
    assert!(
        !hidden.contains(CLI_GROUP_MAIL),
        "표식 off: 계열 목록에 우편이 없어야: {hidden}"
    );
    // 제어 계열은 표식과 무관하다(전원 개방 — ADR-0132 결정 5).
    assert!(hidden.contains("agent"), "제어 계열은 그대로: {hidden}");
    let (agent_help, code) = run_cli_with_marker(Some(MAIL_MARKER_OFF), &["help", "agent"]);
    assert_eq!(code, 0);
    assert!(
        !agent_help.contains(CLI_GROUP_MAIL),
        "표식 off: 제어 계열 help 도 우편을 가르치지 않아야: {agent_help}"
    );
}

/// 감춘 계열의 사용법 요청·**인자 오류**는 오타와 같은 반려(exit 1 + 봉투)로 끝난다. 하나라도 자기 사유를
/// 돌려주면 두 연속 명령이 서로 모순되고, 동사 없는 호출의 반려는 감춘 화면보다 더 많이 가르친다.
#[test]
fn asking_for_the_hidden_mail_usage_is_refused_like_a_typo() {
    for args in [
        vec!["help", "mail"],
        vec!["mail", "--help"],
        vec!["mail", "-h"],
        vec!["mail"],
        vec!["mail", "wat"],
        vec!["mail", "send"],
        vec!["mail", "status"],
    ] {
        let (out, code) = run_cli_with_marker(Some(MAIL_MARKER_OFF), &args);
        assert_eq!(code, 1, "감춘 계열의 잘못된 호출은 반려({args:?}): {out}");
        let v: serde_json::Value = serde_json::from_str(out.trim())
            .unwrap_or_else(|e| panic!("반려는 봉투 JSON 이어야({args:?}): {e} — {out}"));
        assert_eq!(v["code"], "BAD_ARGS", "{out}");
        let hint = v["hint"].as_str().unwrap_or_default();
        for leak in ["send", "status", "pending", "--to", "--body"] {
            assert!(
                !hint.contains(leak),
                "반려 문구가 감춘 계열의 내용물을 돌려주면 안 된다({leak}, {args:?}): {hint}"
            );
        }
    }
    // 대조군 — 같은 인자가 표식 없이는 화면·구체적 사유를 낸다(= 접기가 표식 때문이라는 증명).
    let (shown, code) = run_cli_with_marker(None, &["help", "mail"]);
    assert_eq!(code, 0, "표식 없으면 화면: {shown}");
    assert!(shown.contains("--to"), "{shown}");
    let (verbless, code) = run_cli_with_marker(None, &["mail"]);
    assert_eq!(code, 1);
    assert!(
        verbless.contains("send"),
        "표식 없으면 동사 목록을 그대로 안내한다: {verbless}"
    );
}

/// ★강제는 데몬 하나뿐이다(ADR-0133 §영향)★: 표식이 off 여도 발송은 **실제로 네트워크를 탄다** —
///   여기서 막으면 데몬 거절이 관측되지 않고, 표식을 뗀 프로세스에선 아무도 막지 않게 된다.
#[test]
fn a_hidden_mail_verb_still_posts_to_the_daemon() {
    let rejection = r#"{"status":"error","code":"MAIL_NOT_ALLOWED","hint":"This credential is not allowed to use mail. Retrying will not change that."}"#;
    let (host, port, stub) = spawn_capturing_stub(ok_response(rejection));
    let out = Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["mail", "send", "--to", "bob", "--body", "hi"])
        .env("ENGRAM_TOKEN", "test-token")
        .env("ENGRAM_CONTROL_URL", format!("http://{host}:{port}"))
        .env(MAIL_MARKER_ENV, MAIL_MARKER_OFF)
        .output()
        .expect("spawn engram CLI");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let request = stub.join().expect("stub thread");
    assert!(
        request.contains("POST /control/send"),
        "표식 off 여도 요청은 나가야 — 로컬 차단이 아니다: {request}"
    );
    assert_eq!(
        out.status.code().unwrap_or(-1),
        1,
        "데몬 거절은 기존 3분법의 실패(1): {stdout}"
    );
    assert!(
        stdout.contains("MAIL_NOT_ALLOWED"),
        "거절 사유가 그대로 흘러야: {stdout}"
    );
}

/// ★파싱 뒤에 나는 반려도 접힌다(프로세스 레벨)★: 파서 반려만 보는 스위트가 못 잡던 갈래다 — 빈 stdin
/// 반려가 `--body` 를 되돌려 주면 치지도 않은 플래그로 감춘 계열이 새어 나간다.
#[test]
fn a_hidden_post_parse_rejection_does_not_hand_back_a_sibling_flag() {
    use std::process::Stdio;
    let run = |marker: Option<&str>| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_engram"));
        cmd.args(["mail", "send", "--to", "bob", "--body-stdin"])
            .env_remove("ENGRAM_TOKEN")
            .env_remove("ENGRAM_CONTROL_URL")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match marker {
            Some(v) => cmd.env(MAIL_MARKER_ENV, v),
            None => cmd.env_remove(MAIL_MARKER_ENV),
        };
        let mut child = cmd.spawn().expect("spawn engram CLI");
        // stdin 을 열자마자 닫는다 = 빈 입력(리뷰어 repro 의 `</dev/null`).
        drop(child.stdin.take().expect("stdin piped"));
        let out = child.wait_with_output().expect("wait engram CLI");
        (
            String::from_utf8_lossy(&out.stdout).to_string(),
            out.status.code().unwrap_or(-1),
        )
    };

    let (hidden, code) = run(Some(MAIL_MARKER_OFF));
    assert_eq!(code, 1, "빈 본문은 반려: {hidden}");
    let v: serde_json::Value =
        serde_json::from_str(hidden.trim()).unwrap_or_else(|e| panic!("봉투 JSON: {e} — {hidden}"));
    assert_eq!(v["code"], "BAD_ARGS");
    let hint = v["hint"].as_str().unwrap_or_default();
    for leak in ["--body", "--to", "send", "status", "pending"] {
        assert!(
            !hint.contains(leak),
            "감춘 계열의 내용물이 파싱-후 반려로 새면 안 된다({leak}): {hint}"
        );
    }

    // 대조군 — 표식이 없거나 on 이면 구체적 복구 안내를 그대로 준다.
    for marker in [None, Some(MAIL_MARKER_ON)] {
        let (shown, code) = run(marker);
        assert_eq!(code, 1, "{marker:?}: {shown}");
        assert!(
            shown.contains("--body"),
            "{marker:?}: 보이는 표면에선 복구 안내가 그대로여야: {shown}"
        );
    }
}
