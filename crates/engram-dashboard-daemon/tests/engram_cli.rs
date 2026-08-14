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
            r#"{"agent":{"id":"i","name":"qa-bravo","state":"live"},"created":false}"#,
        ),
        (
            &["agent", "new", "--cwd", "C:/work", "--name", "qa"],
            serde_json::json!({ "verb": "new", "cwd": "C:/work", "name": "qa" }),
            r#"{"agent":{"id":"i","name":"qa","state":"sleeping"}}"#,
        ),
        (
            &["agent", "rename", "qa", "qa-lead"],
            serde_json::json!({ "verb": "rename", "target": "qa", "name": "qa-lead" }),
            r#"{"agent":{"id":"i","name":"qa-lead"},"outcome":"renamed"}"#,
        ),
        (
            &["agent", "move", "qa-lead", "--parent", "none"],
            serde_json::json!({ "verb": "move", "target": "qa-lead", "parent": null }),
            r#"{"agent":{"id":"i","name":"qa-lead"},"parent":null}"#,
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
        (
            vec!["agent", "rename", "a", "b"],
            r#"{"agent":{"id":"i","name":"b"}}"#,
        ),
    ] {
        let (host, port, stub) = spawn_capturing_stub(ok_response(body));
        let url = format!("http://{host}:{port}");
        let (stdout, code) = run_cli(&url, &args, None);
        let _ = stub.join().expect("stub join");
        assert_eq!(code, 2, "증거 없는 2xx → exit 2({args:?}): {stdout}");
    }
}

/// ★평면 성공 shape(`{agent_id,name,state,…}`)도 프로세스 레벨에서 exit 0★ — 데몬 라우트와 이 CLI 는 서로
///   다른 조각에서 손보므로, 여기서 새면 정상 데몬 앞에서 `engram agent spawn` 이 exit 2 를 낸다.
///   ★평면이어도 대조는 그대로★: 마지막 케이스가 그 축을 같이 못박는다(필드만 다 있고 요청과 어긋난 응답).
#[test]
fn engram_agent_accepts_the_flat_success_shape_with_the_same_cross_checks() {
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
