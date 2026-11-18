//! ADR-0086 step 2 · F7(b) · ADR-0132 — **process-level** tests of the `engram` CLI.
//!
//! Unit tests look only at pure functions — here the whole path **wire → parse → stdout JSON → exit code** is measured (env
//!   reads·TCP·process exit codes).
//!
//! ★No claude needed·deterministic★: the stubs use only std and give fixed responses, with no claude/daemon.
//! ★help is measured in front of a stub too★: the screen is the daemon's (`/control/help`), so what is measured here is the CLI's part —
//!   request line · body characters · printing the received screen as is · failure paths;
//!   screen content is measured by the daemon tests (`control::help` · `tests/mail_gate.rs`).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::thread;

use engram_dashboard_agent::types::{CLI_EXE_NAME, CLI_GROUP_MAIL};

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
            // Reads a little of the request to consume the socket (a response can be sent without reading it all). Kept short to avoid
            //   non-blocking.
            let mut buf = [0u8; 4096];
            if let Ok(n) = stream.read(&mut buf) {
                seen = String::from_utf8_lossy(&buf[..n]).to_string();
            }
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            // With a Connection: close response, the stream drops here → the client sees EOF.
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

/// A stub that answers **several** requests in order — the discovery call path is two round trips (list → call).
///
/// ★A one-connection stub cannot measure that path★: the CLI opens a new socket per request (`Connection: close`), so the sibling
///   `spawn_capturing_stub` would fail the second round trip and the test would always see exit 1.
fn spawn_scripted_stub(
    responses: Vec<&'static str>,
) -> (String, u16, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
    listener.set_nonblocking(true).expect("nonblocking stub");
    let addr = listener.local_addr().expect("addr");
    let handle = thread::spawn(move || {
        // ★Why there is a deadline★: the CLI may legitimately call **fewer** times than the script (a local bounce skips the second round
        //   trip); a blocking accept would hang that test at join.
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

/// Reads one request **to the end** — up to the header boundary, then `Content-Length` worth.
///
/// ★Must not finish with a single `read`★: TCP can split header and body into different segments, and a body assertion would **panic, not
///   fail**; the stub must remove that split for reproducibility.
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
        // drop → EOF: the read_to_end of `--body-stdin` ends here.
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

    assert_eq!(code, 0, "success shape (results) → exit 0. stdout={stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "ACK JSON on stdout: {stdout}"
    );
    assert_eq!(v["results"][0]["to"], "bob");
}

#[test]
fn engram_mail_send_corrective_error_prints_body_and_exits_one() {
    // chunk sizes are hex — "{\"status\":\"error\"," (0x12=18) + "\"code\":\"X\"}" (0xb=11) + 0.
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n\
                    12\r\n{\"status\":\"error\",\r\n\
                    b\r\n\"code\":\"X\"}\r\n\
                    0\r\n\r\n";
    let (host, port, handle) = spawn_stub(response);
    let url = format!("http://{host}:{port}");

    let (stdout, code) = run_send(&url, "ghost", "hi");
    let _ = handle.join();

    assert_eq!(code, 1, "correctable error → exit 1. stdout={stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(
        v["status"], "error",
        "error JSON on stdout (de-chunked): {stdout}"
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

    assert_eq!(code, 1, "non-2xx → exit 1");
}

#[test]
fn engram_mail_send_transport_error_exits_one_with_error_json() {
    // ★Deterministic connection failure (no TOCTOU)★: port 0 is OS-reserved, so nothing can listen on it; bind→drop would let another
    //   process reuse the port before connect.
    let url = "http://127.0.0.1:0";
    let (stdout, code) = run_send(url, "bob", "hi");

    assert_eq!(code, 1, "transport failure → exit 1. stdout={stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(
        v["status"], "error",
        "a transport failure is error JSON: {stdout}"
    );
    let code_str = v["code"].as_str().unwrap_or("");
    assert!(
        matches!(code_str, "CONNECT_FAILED" | "INCOMPLETE_RESPONSE"),
        "must be one of the transport-layer error code set (race-robust): got {code_str:?} — {stdout}"
    );
}

// ── D: mail verb mirror — measures route selection·stdin body at the **process level** ────────────────

#[test]
fn engram_mail_pending_posts_to_the_messages_route_and_exits_zero() {
    let response = ok_response(r#"{"me":"alice","open":[]}"#);
    let (host, port, stub) = spawn_capturing_stub(response);
    let url = format!("http://{host}:{port}");

    let (stdout, code) = run_cli(&url, &["mail", "pending"], None);
    let request = stub.join().expect("stub join");

    assert_eq!(code, 0, "query success → exit 0. stdout={stdout}");
    assert!(
        request.starts_with("POST /control/messages HTTP/1.1"),
        "pending goes to the messages route: {request}"
    );
    assert!(
        request.contains("Authorization: Bearer test-token"),
        "identity only through the token (no identity in the request body): {request}"
    );
    assert!(request.ends_with("{}"), "no-argument query body: {request}");
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

    // Regression guard (ADR-0111 decision 4 · ADR-0112 decision 1) — if the group verb revives, it teaches a command that has no priming.
    // ★Does not touch the network★: it ends at the argument parsing stage, so no stub server is needed.
    let (stdout, code) = run_cli("http://127.0.0.1:1", &["mail", "group", "list"], None);
    assert_eq!(code, 1, "an unknown subcommand is BAD_ARGS: {stdout}");
    assert!(
        stdout.contains("BAD_ARGS"),
        "must end as an argument error (no route query): {stdout}"
    );
}

#[test]
fn engram_mail_send_body_stdin_sends_the_piped_text_verbatim() {
    // ★The point of avoiding quoting hell★: characters the shell easily touches (quotes·`$`·`&`·newline) must be carried in the body **as is**.
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

    assert_eq!(code, 0, "send success → exit 0. stdout={stdout}");
    assert!(
        request.starts_with("POST /control/send HTTP/1.1"),
        "{request}"
    );
    let body_start = request.find("\r\n\r\n").expect("body boundary") + 4;
    let v: serde_json::Value =
        serde_json::from_str(request[body_start..].trim()).expect("request body json");
    assert_eq!(v["to"], "bob");
    assert_eq!(v["body"], piped, "the stdin body byte for byte: {request}");
}

#[test]
fn engram_mail_send_body_stdin_lossily_replaces_invalid_utf8_instead_of_refusing_to_send() {
    let response = ok_response(r#"{"id":"m1","results":[{"to":"bob","status":"delivered"}]}"#);
    let (host, port, stub) = spawn_capturing_stub(response);
    let url = format!("http://{host}:{port}");
    // 0xB0 0xA1 = cp949 encoding of U+AC00 — an invalid byte sequence in UTF-8.
    let piped: &[u8] = b"before \xB0\xA1 after";

    let (stdout, code) = run_cli_bytes(
        &url,
        &["mail", "send", "--to", "bob", "--body-stdin"],
        piped,
    );
    let request = stub.join().expect("stub join");

    assert_eq!(code, 0, "sent even if non-UTF8. stdout={stdout}");
    let body_start = request.find("\r\n\r\n").expect("body boundary") + 4;
    let v: serde_json::Value =
        serde_json::from_str(request[body_start..].trim()).expect("request body json");
    let body = v["body"].as_str().expect("body string");
    assert!(
        body.starts_with("before ") && body.ends_with(" after"),
        "the valid part must survive as is: {body:?}"
    );
    assert!(
        body.contains('\u{FFFD}'),
        "invalid bytes must be replaced with U+FFFD (degradation, not refusal): {body:?}"
    );
}

// ── ADR-0132 piece ②: control family (`agent`) — measures route selection·body·exit code at the process level ──────

/// Extracts only the JSON body from the request the stub received.
fn request_body(request: &str) -> serde_json::Value {
    let start = request.find("\r\n\r\n").expect("body boundary") + 4;
    serde_json::from_str(request[start..].trim()).expect("request body json")
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

        assert_eq!(code, 0, "success response → exit 0 ({args:?}): {stdout}");
        assert!(
            request.starts_with("POST /control/agent HTTP/1.1"),
            "the control family goes to one route ({args:?}): {request}"
        );
        assert!(
            request.contains("Authorization: Bearer test-token"),
            "must carry the token ({args:?}): {request}"
        );
        assert_eq!(request_body(&request), want_body, "{args:?}");
        // stdout is the daemon response as is — the CLI does not rewrite it in prose.
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

    assert_eq!(code, 1, "bounce → exit 1: {stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["code"], "AGENT_NOT_FOUND", "{stdout}");
}

/// A refusal that carries the code's `args` is still a refusal (exit 1), and the CLI prints the arguments
/// as received — on the group verb and on the full-name surface alike.
#[test]
fn rejections_carrying_args_exit_one_and_print_the_args() {
    const BODY: &str = r#"{"status":"error","code":"CONFLICT","hint":"launch blocked","args":{"reasons":[{"code":"missing","item":"a/b"}]}}"#;
    let want: serde_json::Value = serde_json::from_str(BODY).expect("body json");

    let (host, port, stub) = spawn_capturing_stub(ok_response(BODY));
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["agent", "spawn", "w"], None);
    let _ = stub.join().expect("stub join");
    assert_eq!(code, 1, "refusal → exit 1: {stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v, want, "{stdout}");

    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog(), ok_response(BODY)]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["tab.create", "--title", "x"], None);
    let requests = stub.join().expect("stub join");
    assert_eq!(code, 1, "refusal → exit 1: {stdout}");
    assert_eq!(requests.len(), 2, "the call went out: {requests:?}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v, want, "{stdout}");
}

/// `engram agent spawn` on the launch answers (TRD S25 step 16): a blocked launch (`CONFLICT` + `args.reasons`) and
/// a launch refused after `--cwd` created the agent (`INTERNAL` + `args.agent_id`) exit 1; a success carrying
/// `notices` exits 0. Each body is printed as received — the notice `text` included.
#[test]
fn engram_agent_spawn_prints_launch_blocks_and_notices_as_received() {
    let cases: [(&[&str], &str, i32); 3] = [
        (
            &["agent", "spawn", "worker"],
            r#"{"status":"error","code":"CONFLICT","hint":"could not start agent 'worker': its launch is blocked — 1 reason(s) in args.reasons","args":{"reasons":[{"kind":"missing","item":"rules/a.md","via":["team.yaml"],"code":"not-found","args":{"path":"rules/a.md"},"line":3}]}}"#,
            1,
        ),
        (
            &["agent", "spawn", "--cwd", "C:/work", "--name", "w"],
            r#"{"status":"error","code":"INTERNAL","hint":"agent 'w' (i) was created but did not start: launch raced","args":{"agent_id":"i"}}"#,
            1,
        ),
        (
            &["agent", "spawn", "worker"],
            r#"{"agent_id":"i","name":"worker","state":"live","created":false,"notices":[{"code":"omitted","args":{"names":["a.md"]},"line":null,"text":"Notice (omitted): intended behavior. Target — a.md"}]}"#,
            0,
        ),
    ];
    for (args, body, want_code) in cases {
        let (host, port, stub) = spawn_capturing_stub(ok_response(body));
        let url = format!("http://{host}:{port}");
        let (stdout, code) = run_cli(&url, args, None);
        let _ = stub.join().expect("stub join");
        assert_eq!(code, want_code, "{args:?} ← {body}: {stdout}");
        let printed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
        let sent: serde_json::Value = serde_json::from_str(body).expect("body json");
        assert_eq!(printed, sent, "{args:?}: {stdout}");
    }
}

/// ★A 2xx without evidence of the change is not success★: with the mail query decider even `{}` was exit 0 and the caller believed
///   something was created. Pins at the process level that it is a report target (2), not a retry target (1).
#[test]
fn engram_agent_hollow_success_bodies_exit_two_not_zero() {
    for (args, body) in [
        (vec!["agent", "new", "--cwd", "C:/x"], r#"{}"#),
        (vec!["agent", "new", "--cwd", "C:/x"], r#"{"status":"ok"}"#),
        (vec!["agent", "list"], r#"{"agents":"not-an-array"}"#),
        // The identity is all carried but there is no outcome (`outcome`) — a body that does not answer whether the rename happened or not.
        (
            vec!["agent", "rename", "a", "b"],
            r#"{"agent_id":"i","name":"b"}"#,
        ),
        // Nesting (`{agent:{…}}`) is a shape no daemon emits — blocks the revival of that branch **at the process level** (a unit table
        //   alone would let this suite, which runs the real exe, stay green).
        (
            vec!["agent", "spawn", "w"],
            r#"{"agent":{"id":"i","name":"w","state":"live"},"created":false}"#,
        ),
    ] {
        let (host, port, stub) = spawn_capturing_stub(ok_response(body));
        let url = format!("http://{host}:{port}");
        let (stdout, code) = run_cli(&url, &args, None);
        let _ = stub.join().expect("stub join");
        assert_eq!(
            code, 2,
            "2xx without evidence → exit 2 ({args:?}): {stdout}"
        );
    }
}

/// ★The success shape (`{agent_id,name,state,…}`) is exit 0 at the process level★ — if it leaked here, `engram agent spawn` would
///   give exit 2 in front of a normal daemon.
///   ★The contrast stays★: the last case pins that axis too (a response with all fields but mismatched with the request).
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
    // The destination is port 0, so if even one POST is attempted, CONNECT_FAILED comes out and the assertion breaks.
    for args in [
        vec!["agent"],
        vec!["agent", "wat"],
        vec!["agent", "list", "--json"],
        // A call that asks for create and wake at once — there is no basis to pick which is meant.
        vec!["agent", "spawn", "qa-bravo", "--cwd", "C:/work"],
        vec!["agent", "spawn"],
        vec!["agent", "new"],
        vec!["agent", "rename", "only-one"],
        vec!["agent", "move", "qa-bravo"],
        vec!["agent", "new", "--cwd", "a", "--cwd", "b"],
        vec!["agent", "new", "--cwd", "--name"],
        vec!["agent", "spawn", "--help"],
        // The shape where an unset shell variable expands to an empty argument — folding it into "not given" would run a different command.
        vec!["agent", "move", "helper", "--parent", ""],
        vec!["agent", "new", "--cwd", ""],
    ] {
        let (stdout, code) = run_cli(UNREACHABLE_URL, &args, None);
        assert_eq!(code, 1, "argument error → exit 1 ({args:?}): {stdout}");
        let v: serde_json::Value =
            serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("stdout json: {e}"));
        assert_eq!(v["code"], "BAD_ARGS", "{args:?} → BAD_ARGS: {stdout}");
        let suggestions = backticked_help_commands(v["hint"].as_str().unwrap_or_default());
        assert!(
            !suggestions.is_empty(),
            "the hint has no runnable help command at all ({args:?}): {stdout}"
        );
        for suggested in suggestions {
            assert_suggested_help_runs(&suggested, &format!("{args:?}"));
        }
    }
}

#[test]
fn engram_agent_help_is_discoverable_the_same_way_as_the_mail_group() {
    for args in [
        vec!["help", "agent"],
        vec!["agent", "--help"],
        vec!["agent", "-h"],
    ] {
        let (out, code, requests) = run_help_against_stub(&args, "engram agent - stub screen");
        assert_eq!(code, 0, "family help exits successfully ({args:?}): {out}");
        assert_eq!(
            out, "engram agent - stub screen\n",
            "prints the received screen as is ({args:?})"
        );
        assert_help_request(&requests, r#"{"topic":"agent"}"#);
    }
}

// ── ADR-0132: family surface — help and argument errors ──────────────────────────────────────────────────

/// ★Destination = port 0, where nobody can listen★: argument-error cases must end before the network — one POST would produce
///   CONNECT_FAILED and break the assertion. Also used to deliberately produce a failure to reach the daemon.
const UNREACHABLE_URL: &str = "http://127.0.0.1:0";

/// The request line of the help route — ★written as characters★: with the shared constant (`CLI_HELP_ROUTE`) the assertion would move
///   along with a changed constant and catch nothing.
const HELP_REQUEST_LINE: &str = "POST /control/help HTTP/1.1\r\n";

/// The help route success envelope — carries one screen the daemon rendered.
fn screen_response(screen: &str) -> &'static str {
    ok_response(&serde_json::json!({ "screen": screen }).to_string())
}

/// Runs one help call in front of a screen stub — (stdout, exit code, the requests the stub received).
fn run_help_against_stub(args: &[&str], screen: &str) -> (String, i32, Vec<String>) {
    let (host, port, stub) = spawn_scripted_stub(vec![screen_response(screen)]);
    let (stdout, code) = run_cli(&format!("http://{host}:{port}"), args, None);
    let requests = stub.join().expect("stub join");
    (stdout, code, requests)
}

/// Was there exactly one request, and did it carry that body to the help route **character for character**.
fn assert_help_request(requests: &[String], body: &str) {
    assert_eq!(requests.len(), 1, "help is one round trip: {requests:?}");
    let request = &requests[0];
    assert!(
        request.starts_with(HELP_REQUEST_LINE),
        "must go to the help route: {request}"
    );
    assert_eq!(
        request.split_once("\r\n\r\n").map(|(_, b)| b),
        Some(body),
        "the request body character for character: {request}"
    );
}

/// ★Does the help command the hint suggested actually run★: with the right shape it goes out as one help request (the stub produces the screen)
///   and must be exit 0 — a caller that did as told must not hit the same wall again. Whether that word is a real screen is
///   the daemon's job (`control::help`).
fn assert_suggested_help_runs(suggested: &str, context: &str) {
    let argv: Vec<&str> = suggested.split_whitespace().skip(1).collect();
    let (out, code, requests) = run_help_against_stub(&argv, "stub screen");
    assert_eq!(
        code, 0,
        "the command the hint suggested must actually run ({context}): `{suggested}` → {out}"
    );
    assert_eq!(
        requests.len(),
        1,
        "`{suggested}` is one help request: {requests:?}"
    );
    assert!(
        requests[0].starts_with(HELP_REQUEST_LINE),
        "`{suggested}` must go to the help route: {}",
        requests[0]
    );
}

/// ★Shipped file name ↔ constant (`CLI_EXE_NAME`) match★ — no other test sees this direction: the priming pin compares priming **with the
///   constant** (passes if both move together), and the process tests find the binary **through the manifest**
///   (always pass). So only here does a rename of the constant·priming that misses `[[bin]]` turn red; at runtime the deployed executable
///   would differ from the name the agent learned and mail would stop silently (ADR-0094 alignment invariant).
/// ★Path source★: the `<name>` of `CARGO_BIN_EXE_<name>` is **the manifest's `[[bin]] name`** — renaming only `name` stops **compilation**
///   (`CARGO_BIN_EXE_engram not defined`; the explicit target occupies the path, so auto-discovery does not revive an `engram` target).
///   So a manifest-only rename is caught by compilation, a constant-only rename by the assertion below.
#[test]
fn the_built_binary_file_name_matches_the_shared_constant() {
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_engram"));
    let stem = exe.file_stem().and_then(|s| s.to_str()).unwrap_or_else(|| {
        panic!("could not read the file stem from the build output path: {exe:?}")
    });
    assert_eq!(
        stem, CLI_EXE_NAME,
        "the shipped executable name and CLI_EXE_NAME diverged — grant·priming·PATH resolution all follow the constant, so \
          the command the agent learned would not exist: {exe:?}"
    );
}

/// ★help carries one word to the daemon and prints the received screen as is★ — screen content is the daemon's and measured by its tests;
///   here: request line · body characters · screen printed untouched with one newline appended · shape errors cut without a round trip.
#[test]
fn engram_help_carries_one_word_and_prints_the_screen_it_gets() {
    // 0 arguments = help = table of contents (an agent sees the surface just by typing the name).
    let root = "engram - stub root\n\n  engram help mail      x\n";
    for args in [vec!["help"], vec![]] {
        let (stdout, code, requests) = run_help_against_stub(&args, root);
        assert_eq!(code, 0, "help exits successfully ({args:?}): {stdout}");
        assert_eq!(
            stdout,
            format!("{root}\n"),
            "the received screen + one newline ({args:?})"
        );
        assert_help_request(&requests, r#"{"topic":null}"#);
    }

    for group in ["mail", "agent", "window", "settings"] {
        let (screen, code, requests) = run_help_against_stub(&["help", group], "stub screen");
        assert_eq!(
            code, 0,
            "family help exits successfully ({group}): {screen}"
        );
        assert_eq!(screen, "stub screen\n", "{group}");
        assert_help_request(&requests, &format!(r#"{{"topic":"{group}"}}"#));
    }

    // There is no spot one level further below a family — it is cut as an argument error without a round trip and guided to the way to see
    //   the list.
    let (deeper, code) = run_cli(UNREACHABLE_URL, &["help", "mail", "send"], None);
    assert_eq!(
        code, 1,
        "the spot after the family is an argument error: {deeper}"
    );
    let v: serde_json::Value =
        serde_json::from_str(deeper.trim()).unwrap_or_else(|e| panic!("stdout json: {e}"));
    assert_eq!(v["code"], "BAD_ARGS", "{deeper}");
    assert!(
        v["hint"]
            .as_str()
            .unwrap_or_default()
            .contains(&format!("`{CLI_EXE_NAME} help`")),
        "the bounce must guide to the way to see the list: {deeper}"
    );
}

/// Runs **without** ENGRAM_TOKEN·ENGRAM_CONTROL_URL — erasing even inherited values to reproduce "outside a spawn".
fn run_cli_without_credentials(args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(args)
        .env_remove("ENGRAM_TOKEN")
        .env_remove("ENGRAM_CONTROL_URL")
        .output()
        .expect("spawn engram CLI");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// ★help fails the same way as other commands (ADR-0285)★ — without credentials it matches `mail pending`, and with the daemon unreachable
///   it matches `agent list`, with the same envelope · exit 1 **down to the bytes**; a help-only path
///   (fallback · copy · different code · channel) would make the same state look different per command.
/// ★The control group is taken from the same run★: hard-coded expected output would need fixing on every wording change and would not
///   catch help alone ending up with different wording.
// ADR-0285
#[test]
fn help_fails_the_same_way_as_every_other_command_without_credentials_or_a_daemon() {
    let (no_token, code) = run_cli_without_credentials(&["mail", "pending"]);
    assert_eq!(code, 1, "a mail verb without credentials fails: {no_token}");
    let v: serde_json::Value = serde_json::from_str(no_token.trim())
        .unwrap_or_else(|e| panic!("stdout json: {e} — {no_token}"));
    assert_eq!(v["code"], "NO_TOKEN", "{no_token}");

    let (unreached, code) = run_cli(UNREACHABLE_URL, &["agent", "list"], None);
    assert_eq!(code, 1, "an unreachable daemon is a failure: {unreached}");
    let v: serde_json::Value = serde_json::from_str(unreached.trim())
        .unwrap_or_else(|e| panic!("stdout json: {e} — {unreached}"));
    assert_eq!(v["code"], "CONNECT_FAILED", "{unreached}");

    for args in [
        vec!["help"],
        vec![],
        vec!["--help"],
        vec!["-h"],
        vec!["help", "mail"],
        vec!["mail", "--help"],
        vec!["mail", "-h"],
        vec!["help", "agent"],
        vec!["agent", "--help"],
        vec!["help", "window"],
        vec!["help", "settings"],
        vec!["help", "theme"],
    ] {
        let (stdout, code) = run_cli_without_credentials(&args);
        assert_eq!(
            (stdout.as_str(), code),
            (no_token.as_str(), 1),
            "help without credentials is the same envelope as other commands ({args:?})"
        );
        let (stdout, code) = run_cli(UNREACHABLE_URL, &args, None);
        assert_eq!(
            (stdout.as_str(), code),
            (unreached.as_str(), 1),
            "help that could not reach the daemon is the same envelope as other commands ({args:?})"
        );
    }
}

/// `engram mail --help` = `engram help mail` (the same request). If the two spellings sent different requests, the screen would differ
/// when the LLM typed what it learned.
#[test]
fn conventional_help_spellings_render_the_same_screens() {
    // ★Only the two the parser accepts as families have the `<family> --help` spelling★: `window`·`settings` have no daemon-side `<family>
    //   <verb>` entrance and are reached only through `engram help <word>`; the old word `theme` is carried as is (alias resolution is the
    //   daemon's).
    for (spellings, body) in [
        (
            vec![vec!["help"], vec!["--help"], vec!["-h"], vec![]],
            r#"{"topic":null}"#,
        ),
        (
            vec![
                vec!["help", "mail"],
                vec!["--help", "mail"],
                vec!["-h", "mail"],
                vec!["mail", "--help"],
                vec!["mail", "-h"],
            ],
            r#"{"topic":"mail"}"#,
        ),
        (
            vec![
                vec!["help", "agent"],
                vec!["--help", "agent"],
                vec!["agent", "--help"],
                vec!["agent", "-h"],
            ],
            r#"{"topic":"agent"}"#,
        ),
        (vec![vec!["help", "window"]], r#"{"topic":"window"}"#),
        (vec![vec!["help", "settings"]], r#"{"topic":"settings"}"#),
        (vec![vec!["help", "theme"]], r#"{"topic":"theme"}"#),
    ] {
        for args in spellings {
            let (out, code, requests) = run_help_against_stub(&args, "stub screen");
            assert_eq!(code, 0, "{args:?}: {out}");
            assert_eq!(out, "stub screen\n", "{args:?}");
            assert_help_request(&requests, body);
        }
    }

    // An unknown word is bounced by the daemon — that body is printed as is with exit 1 (silently going back to the top level would make a typo
    //   read as success).
    let refusal =
        r#"{"status":"error","code":"INVALID_ARGUMENT","hint":"unknown help topic: inbox"}"#;
    let (host, port, stub) = spawn_scripted_stub(vec![ok_response(refusal)]);
    let (out, code) = run_cli(&format!("http://{host}:{port}"), &["help", "inbox"], None);
    let requests = stub.join().expect("stub join");
    assert_eq!(code, 1, "an unknown family is a failure: {out}");
    assert_eq!(
        out,
        format!("{refusal}\n"),
        "the daemon's bounce must flow through as is"
    );
    assert_help_request(&requests, r#"{"topic":"inbox"}"#);
}

#[test]
fn engram_unknown_group_and_bare_flags_are_argument_errors_without_touching_the_network() {
    // Typing the old notation (flags·verbs without a family) as is ends as an argument error instead of flowing into a send.
    for args in [
        vec!["wat"],
        vec!["--to", "bob", "--body", "hi"],
        vec!["pending"],
        vec!["status", "m-1"],
        // A help token in a value position — `status <id> --help` used to **query** `--help` as a message id
        //   and make a real round trip. Being in this list is itself the claim "does not touch the network"
        //   (port 0). The spot **right after** a family is, conversely, a screen — the spelling test above measures that side.
        vec!["mail", "status", "m-7f3k9q2d", "--help"],
        vec!["mail", "status", "m-7f3k9q2d", "-h"],
        vec!["help", "mail", "inbox"],
        // help + leftover arguments — swallowing them with exit 0 would make the letter vanish with a success code.
        vec!["mail", "--help", "--to", "bob", "--body", "hi"],
        vec!["--help", "mail", "--to", "bob"],
        // Another help token after help — not a screen word (shape rule — no round trip).
        vec!["help", "--help"],
        vec!["--help", "help"],
        // The spot where the hint used to suggest itself back as an invalid command.
        vec!["help", "--to", "bob"],
        // A duplicated value flag — bounced instead of a silent loss.
        vec![
            "mail", "send", "--to", "a", "--body", "first", "--body", "second",
        ],
        // A value flag missing its value — it used to swallow the next flag as the body and discard the piped body.
        vec!["mail", "send", "--to", "bob", "--body", "--body-stdin"],
        vec!["mail", "send", "--to", "--body", "hi"],
    ] {
        let (stdout, code) = run_cli(UNREACHABLE_URL, &args, None);
        assert_eq!(code, 1, "argument error → exit 1 ({args:?}): {stdout}");
        let v: serde_json::Value =
            serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("stdout json: {e}"));
        assert_eq!(v["code"], "BAD_ARGS", "{args:?} → BAD_ARGS: {stdout}");
        assert!(
            v["hint"].as_str().unwrap_or_default().contains("help"),
            "the hint must guide to help ({args:?}): {stdout}"
        );
        // ★Checks that the suggested command actually runs★: `engram help --to bob` used to suggest "run `engram help --to`", itself
        //   another argument error, so a caller that did as told hit the same wall; looking only for the word help in the hint lets that pass.
        let suggestions = backticked_help_commands(v["hint"].as_str().unwrap_or_default());
        // ★If the set is empty, this test passes without asserting anything★ — to avoid making an empty test that reads as coverage,
        //   "at least one was picked" must be pinned first.
        assert!(
            !suggestions.is_empty(),
            "the hint has no runnable help command at all ({args:?}): {stdout}"
        );
        for suggested in suggestions {
            assert_suggested_help_runs(&suggested, &format!("{args:?}"));
        }
    }
}

/// Picks only the **help calls** among the backtick spans in the hint (example send commands are placeholders, not things to run).
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
    assert_eq!(code, 1, "shape error → exit 1. stdout={stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["code"], "BAD_ARGS", "{stdout}");
    assert!(
        v["hint"].as_str().unwrap_or_default().contains("mutually"),
        "the reason must say it is mutual exclusion: {stdout}"
    );
}

// ── ADR-0155/0156: discovery (`commands`) and full-name calls — process level ────────────────────────

/// A schema item (`spec_item_json`) of **the same shape** as what the daemon's own table emits — carried whole in the list's `help` field.
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

/// The argument schema of the `slot.assign` fixture — gathers the type branches into one command.
fn slot_args() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "index": { "type": "integer" },
            "sticky": { "type": "boolean" },
            "name": { "type": "string" },
            // ★A **boolean** argument named `help`★: this field is needed to reproduce the branch "asked for usage and Write ran"
            //   (review A). Taking it out of the fixture would hide that regression again.
            "help": { "type": "boolean" },
            "mode": { "anyOf": [{ "enum": ["wide", "tall"] }, { "type": "null" }] }
        },
        "required": ["index", "name"]
    })
}

/// List fixture — gathers the branches into one response: a schema we know · a help that is **not JSON** · `callable:false` ·
/// a nullable required field · a mail name (material to measure whether that family leaks onto the screen).
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
                // ★The return field name must not be a substring of the command name★: `slot` is already inside `slot.assign`,
                //   so asserting on that word stays green even if the whole return section vanishes.
                serde_json::json!({ "type": "object", "properties": { "placed_at": { "type": "integer" } }, "required": ["placed_at"] }),
            ),
            true,
        ),
        // Free text registered by a client — not JSON. This one line must not sink the whole list.
        catalog_row(
            "tab.create",
            "opens a tab in the dashboard\nsecond line nobody should see in the listing",
            false,
        ),
        // ★When a mail family name is carried in the table★: without this row, the leak test below passes on any implementation
        //   (the fixture does not have that word at all, so the assertion is always true).
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

    assert_eq!(code, 0, "list render success → exit 0: {stdout}");
    assert_eq!(
        requests.len(),
        1,
        "the list is one round trip: {requests:?}"
    );
    assert!(
        requests[0].starts_with("POST /control/commands HTTP/1.1"),
        "to the discovery route: {}",
        requests[0]
    );
    assert!(
        requests[0].contains("Authorization: Bearer test-token"),
        "must carry the token: {}",
        requests[0]
    );

    for (name, summary) in [
        ("agent.list", "every agent: name, state, folder, parent"),
        ("slot.assign", "put an agent in a slot"),
    ] {
        let row = stdout
            .lines()
            .find(|l| l.trim_start().starts_with(name))
            .unwrap_or_else(|| panic!("{name} is not in the list: {stdout}"));
        assert!(
            row.contains(summary),
            "the summary must come next to the name: {row}"
        );
    }
    // ★An unreadable blob does not sink the list★ — the name remains, and what can be kept (the first line) becomes the summary.
    let free_text = stdout
        .lines()
        .find(|l| l.trim_start().starts_with("tab.create"))
        .unwrap_or_else(|| panic!("an unparseable help removed the whole line: {stdout}"));
    assert!(
        free_text.contains("opens a tab"),
        "the first line of free text is the summary: {free_text}"
    );
    assert!(
        !free_text.contains("second line"),
        "one command = must be one line: {free_text}"
    );
    // ★This row scripts **a value no daemon emits today**★ (both merged sources are reachable, ADR-0160, so every real row is
    //   `callable:true`) — measured because the field is not a constant: the day it receives false, the CLI must show it.
    //   ★The wording must not promise `UNSUPPORTED`★: its producer is gone (`catalog::not_mine` deleted) and the refusal now is a
    //   different code the daemon decides.
    assert!(
        stdout.contains("cannot run"),
        "the fact that the name cannot be called must be visible on the screen: {stdout}"
    );
    assert!(
        !stdout.contains("UNSUPPORTED"),
        "it promises an error code that was removed: {stdout}"
    );
    // The discovery screen is rendered plain text — it does not leak the raw JSON as is.
    assert!(
        !stdout.trim_start().starts_with('{'),
        "the list is a plain-text screen: {stdout}"
    );
}

#[test]
fn engram_commands_with_a_name_shows_its_arguments_return_shape_and_errors() {
    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog()]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["commands", "slot.assign"], None);
    let requests = stub.join().expect("stub join");

    assert_eq!(code, 0, "detail render success → exit 0: {stdout}");
    assert_eq!(
        requests.len(),
        1,
        "the detail is one list round trip too: {requests:?}"
    );
    for token in [
        "slot.assign",
        "put an agent in a slot",
        "--index <integer>",
        "--name <string>",
        "--sticky <true|false>",
        // It is an optional field, so the null notation is not attached — attaching it, the screen would advertise a usage that does not
        //   work (M).
        "--mode <wide|tall>",
        "required",
        "optional",
        // ★The return field is measured with a word that is not a substring of the command name★: it used to be measured with `slot`, so
        //   even if the return section
        //   vanished entirely, `slot.assign` contained that word and it stayed green.
        "placed_at",
        "NOT_FOUND",
        "INVALID_ARGUMENT",
    ] {
        assert!(
            stdout.contains(token),
            "{token} on the detail screen: {stdout}"
        );
    }
    let returns = stdout
        .lines()
        .position(|l| l.starts_with("Returns —"))
        .unwrap_or_else(|| panic!("no return section: {stdout}"));
    assert!(
        stdout
            .lines()
            .skip(returns)
            .any(|l| l.trim_start().starts_with("placed_at")),
        "the return field must be inside that section: {stdout}"
    );
}

/// ★A. A usage request must not execute the command★: the `slot.assign` fixture declares a **boolean** argument named `help`
///   — the old parser read `--help` as that field and sent `{"help":true}` to `/control/call`.
///   Asked for usage and Write ran. All three spellings go to the detail screen, and the call route is never opened.
#[test]
fn asking_a_full_name_for_help_renders_its_detail_and_never_calls_it() {
    for token in ["--help", "-h", "help"] {
        let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog()]);
        let url = format!("http://{host}:{port}");
        let (stdout, code) = run_cli(&url, &["slot.assign", token], None);
        let requests = stub.join().expect("stub join");

        assert_eq!(
            code, 0,
            "a usage request exits successfully ({token}): {stdout}"
        );
        assert_eq!(
            requests.len(),
            1,
            "only one list round trip ({token}): {requests:?}"
        );
        assert!(
            !requests.iter().any(|r| r.contains("POST /control/call")),
            "a usage request executed the command ({token}): {requests:?}"
        );
        assert!(
            stdout.contains("--index <integer>"),
            "must be the detail screen ({token}): {stdout}"
        );
    }
    // ★L. A help shifted one position over is not an argument either★: it used to look only at the first position, so `--index 1 --help`
    //   was bound to that boolean
    //   field and `{"index":1,"help":true}` was actually POSTed. In any position the command does not run —
    //   and these cases have port 0 as the destination, so they **do not touch the network either**.
    for args in [
        vec!["slot.assign", "--index", "1", "--help"],
        vec!["slot.assign", "--index", "1", "-h"],
        vec!["slot.assign", "--sticky", "--help"],
        vec!["slot.assign", "--help", "--index", "1"],
    ] {
        let (stdout, code) = run_cli(UNREACHABLE_URL, &args, None);
        assert_eq!(
            code, 1,
            "a shifted help is an argument error ({args:?}): {stdout}"
        );
        let v: serde_json::Value = serde_json::from_str(stdout.trim())
            .unwrap_or_else(|e| panic!("envelope JSON ({args:?}): {e} — {stdout}"));
        assert_eq!(
            v["code"], "BAD_ARGS",
            "there must be neither a round trip nor execution ({args:?}): {stdout}"
        );
    }
    // It does not diverge from the rule of the lone call.
    let (stdout, code) = run_cli(
        UNREACHABLE_URL,
        &["slot.assign", "--help", "--index", "1"],
        None,
    );
    assert_eq!(
        code, 1,
        "help with leftover arguments is an argument error: {stdout}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["code"], "BAD_ARGS", "{stdout}");
}

/// ★B. The shape that detaches the parent must exist on this surface too★: `parent` is nullable and required, so without a way to carry null
///   the string `"none"` goes out and becomes NOT_FOUND, or it ends up under an agent that happens to have that name.
///   It measures **the same fact** as the old family's `detaching_sends_an_explicit_null_parent_not_an_absent_key` at the new entrance.
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

    assert_eq!(code, 0, "call success → exit 0: {stdout}");
    assert_eq!(requests.len(), 2, "{requests:?}");
    let sent = request_body(&requests[1]);
    assert_eq!(
        sent["args"]["parent"],
        serde_json::Value::Null,
        "a request to detach to root must be null: {sent}"
    );
    assert_eq!(sent["args"]["target"], serde_json::json!("qa"));
    // The attaching side is the string as is — two meanings overlap in one word, so the opposite direction is pinned too.
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

/// ★M. If that word consumed **even optional fields**, it would be a silent accident★: if `agent.spawn --cwd none` went out as `{"cwd":null}`,
///   it would be created in the daemon's default folder, and that response does not carry `cwd`, so the fact that a different folder was
///   used would be
///   visible nowhere. In an optional field "no value" means leaving out the flag, and the old families also send the same input as a string.
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
        "an optional field must go out as a string even if nullable: {sent}"
    );
}

/// ★N. Dropped rows must not be reported as "not in the list"★: one registration missing `callable` makes that row vanish,
///   and if absence is asserted in the envelope a machine reads, the caller permanently gives up on a command that exists.
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
        // The registration left out `callable` — only that row cannot be read.
        serde_json::json!({ "name": "slot.assign", "help": "x" }),
    ]);
    let (host, port, stub) = spawn_scripted_stub(vec![poisoned]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["slot.assign", "--index", "1"], None);
    let requests = stub.join().expect("stub join");

    assert_eq!(code, 1, "cannot be called → exit 1: {stdout}");
    assert_eq!(requests.len(), 1, "the call does not go out: {requests:?}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["code"], "UNKNOWN_COMMAND", "{stdout}");
    let hint = v["hint"].as_str().unwrap_or_default();
    assert!(
        !hint.contains("does not list"),
        "must not assert rows that were not read as absent: {hint}"
    );
    assert!(hint.contains("unreadable"), "must state the facts: {hint}");

    // Control group — without dropped rows it asserts absence as is (proof that the wording above does not always come out).
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

/// ★D. One unreadable row must not kill the whole surface★: when one registration gave an empty name, the list·detail·
///   **every call** used to be exit 2 until that client disconnected.
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
        // A registration with an empty name — the registry measures only length, so it can really come.
        serde_json::json!({ "name": "", "help": "h", "callable": true }),
    ]);
    let (host, port, stub) = spawn_scripted_stub(vec![poisoned]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["commands"], None);
    let _ = stub.join().expect("stub join");
    assert_eq!(
        code, 0,
        "if there are readable rows, the list stands: {stdout}"
    );
    assert!(stdout.contains("agent.list"), "{stdout}");

    // In the same state the call must stay alive too — this path used to be blocked entirely.
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
    assert_eq!(code, 0, "the call must be alive too: {stdout}");
    assert_eq!(requests.len(), 2, "{requests:?}");
}

#[test]
fn engram_commands_for_a_name_the_catalog_does_not_list_is_a_failure_not_a_crash() {
    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog()]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["commands", "nope.nope"], None);
    let _ = stub.join().expect("stub join");

    assert_eq!(code, 1, "name not in the table → exit 1: {stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["code"], "UNKNOWN_COMMAND", "{stdout}");
}

/// ★The **declared type** decides, not what the value looks like★: `--name 123` is a string field, so it must be carried as a string,
///   and `--index 3` is an integer field, so it must be carried as a number. Looking at only one side lets "all strings" or "all JSON
///   parsing" pass too.
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

    assert_eq!(code, 0, "call success → exit 0: {stdout}");
    assert_eq!(
        requests.len(),
        2,
        "schema query + call, two round trips: {requests:?}"
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
        "integer field: {sent}"
    );
    assert_eq!(
        sent["args"]["name"],
        serde_json::json!("123"),
        "a string field is a string even if it looks like a number: {sent}"
    );
    assert_eq!(
        sent["args"]["sticky"],
        serde_json::json!(true),
        "a boolean field stands without a value and does not swallow a value: {sent}"
    );
    assert_eq!(sent["args"]["mode"], serde_json::json!("wide"), "{sent}");
    // The call response is the daemon body as is — the CLI does not rewrite it.
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(stdout.trim()).expect("stdout json"),
        serde_json::json!({ "placed_at": 3 })
    );
}

/// ★F. The call response is also measured by **the declared return**★: with a lenient decider this gives exit 0, while for the same response
///   the old family gives exit 2. If one command got opposite decisions depending on the entrance, the caller would record something that
///   did not happen
///   as fact. The material (`ok.required`) already came in the first round trip.
#[test]
fn a_call_answered_without_the_declared_return_fields_exits_two_not_zero() {
    for body in [r#"{"status":"ok"}"#, r#"{}"#, r#"[]"#] {
        let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog(), ok_response(body)]);
        let url = format!("http://{host}:{port}");
        let (stdout, code) = run_cli(&url, &["slot.assign", "--index", "1", "--name", "qa"], None);
        let requests = stub.join().expect("stub join");
        assert_eq!(requests.len(), 2, "the call went out: {requests:?}");
        assert_eq!(
            code, 2,
            "2xx without evidence → exit 2 (a report target, not a retry): {body} → {stdout}"
        );
    }
    // Control group — a response carrying all declared fields is success as is (proof that the decision above is not a false alarm).
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

    assert_eq!(
        code, 1,
        "a value that cannot be converted → exit 1: {stdout}"
    );
    assert_eq!(
        requests.len(),
        1,
        "does not knock on the call route: {requests:?}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["code"], "BAD_ARGS", "{stdout}");
    assert!(
        v["hint"].as_str().unwrap_or_default().contains("integer"),
        "must state the declared type: {stdout}"
    );
}

/// ★An unknown flag ends **locally**★: the daemon makes the same decision too, but that round trip knocks on a side-effecting entrance one
///   more time
///   while giving the caller nothing more. The wording must carry the material to fix it (all declared fields).
#[test]
fn engram_invoking_with_an_unknown_flag_fails_locally_with_the_declared_list() {
    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog()]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["slot.assign", "--index", "1", "--nope", "x"], None);
    let requests = stub.join().expect("stub join");

    assert_eq!(code, 1, "unknown flag → exit 1: {stdout}");
    assert_eq!(
        requests.len(),
        1,
        "only up to the schema query — the call does not go out: {requests:?}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["code"], "BAD_ARGS", "{stdout}");
    let hint = v["hint"].as_str().unwrap_or_default();
    for declared in ["--index", "--name", "--sticky", "--mode"] {
        assert!(
            hint.contains(declared),
            "all declared fields must be carried ({declared}): {hint}"
        );
    }
}

/// ★E. It must say which request failed★: when the list query fails, stdout prints that route's body,
///   and it is byte-identical to the command's own bounce — the caller (LLM) starts fixing arguments that are fine.
#[test]
fn a_catalog_failure_during_a_call_says_which_request_failed() {
    let refusal = ok_response(r#"{"status":"error","code":"INTERNAL","hint":"roster is wedged"}"#);
    let (host, port, stub) = spawn_scripted_stub(vec![refusal]);
    let url = format!("http://{host}:{port}");
    let (stdout, stderr, code) = run_cli_streams(&url, &["agent.list"]);
    let requests = stub.join().expect("stub join");

    assert_eq!(code, 1, "bounce → exit 1: {stdout}");
    assert_eq!(requests.len(), 1, "the call did not go out: {requests:?}");
    assert!(
        stdout.contains("INTERNAL"),
        "a failed body must flow through as is: {stdout}"
    );
    assert!(
        stderr.contains("/control/commands") && stderr.contains("agent.list"),
        "must state which request failed: {stderr}"
    );
    assert!(
        stderr.contains("never called"),
        "must say that the command itself was not called: {stderr}"
    );

    // Control group — `commands`'s own failure does not get that line (then there is no room to ask which request).
    let refusal = ok_response(r#"{"status":"error","code":"INTERNAL","hint":"roster is wedged"}"#);
    let (host, port, stub) = spawn_scripted_stub(vec![refusal]);
    let url = format!("http://{host}:{port}");
    let (_stdout, stderr, code) = run_cli_streams(&url, &["commands"]);
    let _ = stub.join().expect("stub join");
    assert_eq!(code, 1);
    assert!(!stderr.contains("never called"), "{stderr}");
}

/// Receives stdout·stderr **separately** — diagnostics like which request failed go to stderr, and the existing helper discards it.
fn run_cli_streams(control_url: &str, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(args)
        .env("ENGRAM_TOKEN", "test-token")
        .env("ENGRAM_CONTROL_URL", control_url)
        .output()
        .expect("spawn engram CLI");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// ★The daemon is the source of truth for deciding it cannot be called★: even with `callable:false` the CLI does not cut it beforehand —
///   that would leave the refusal unobserved.
///
/// ★The refusal code must be **what the daemon actually emits now**★: this test used to script `UNSUPPORTED`, whose producer disappeared
///   with relaying (ADR-0160); a scripted stub returns anything, so it was **green on any implementation**.
/// Calling that name is now relayed, and if the owner does not answer by the deadline `TIMEOUT` comes back.
#[test]
fn engram_invoking_a_name_this_entrance_cannot_run_surfaces_the_daemons_refusal() {
    let refusal = ok_response(
        r#"{"status":"error","code":"TIMEOUT","hint":"'tab.create' was handed to its owner but no outcome came back before the deadline"}"#,
    );
    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog(), refusal]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["tab.create", "--title", "x"], None);
    let requests = stub.join().expect("stub join");

    assert_eq!(code, 1, "daemon bounce → exit 1: {stdout}");
    assert_eq!(
        requests.len(),
        2,
        "the call must actually go out (not a local block): {requests:?}"
    );
    assert!(
        requests[1].starts_with("POST /control/call HTTP/1.1"),
        "{}",
        requests[1]
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(
        v["code"], "TIMEOUT",
        "the refusal reason must flow through as is: {stdout}"
    );
    // ★A call carries its own number★ (ADR-0161 decision 2) — without it the daemon would issue a new one per request,
    //   and the re-execution prevention seat would structurally never be hit.
    assert!(
        requests[1].contains("request_id"),
        "the call body must carry the request number: {}",
        requests[1]
    );
}

/// A 2xx whose **envelope** cannot be read as a list is a report target (2), not a bounce (1) — the existing three-way split as is.
///
/// ★One broken row does not belong here★: that is a recoverable branch, so the list stands as is (see
///   `a_single_unreadable_catalog_row_does_not_take_the_surface_down` above), and only the dropped count goes to stderr.
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
        assert_eq!(code, 2, "unreadable envelope → exit 2 ({body}): {stdout}");
    }
    // An empty list is not a defect — it is the normal answer of a daemon whose table slot is empty.
    let (host, port, stub) = spawn_scripted_stub(vec![ok_response(r#"{"commands":[]}"#)]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["commands"], None);
    let _ = stub.join().expect("stub join");
    assert_eq!(code, 0, "an empty list is normal: {stdout}");

    // Dropped rows are not silent — if the name being looked for was among them, the next line is UNKNOWN_COMMAND, and this line is
    //   the only clue linking the two.
    let (host, port, stub) = spawn_scripted_stub(vec![ok_response(
        r#"{"commands":[{"name":"","help":"h","callable":true}]}"#,
    )]);
    let url = format!("http://{host}:{port}");
    let (_stdout, stderr, code) = run_cli_streams(&url, &["commands"]);
    let _ = stub.join().expect("stub join");
    assert_eq!(code, 0);
    assert!(
        stderr.contains("could not be read"),
        "must announce the dropped row count: {stderr}"
    );
}

#[test]
fn engram_commands_maps_rejections_and_transport_failures_to_one() {
    let rejected = ok_response(r#"{"status":"error","code":"INTERNAL","hint":"broken"}"#);
    let (host, port, stub) = spawn_scripted_stub(vec![rejected]);
    let url = format!("http://{host}:{port}");
    let (stdout, code) = run_cli(&url, &["commands"], None);
    let _ = stub.join().expect("stub join");
    assert_eq!(code, 1, "daemon bounce → exit 1: {stdout}");
    assert!(
        stdout.contains("INTERNAL"),
        "the body of a failed list must flow through as is: {stdout}"
    );

    let (stdout, code) = run_cli(UNREACHABLE_URL, &["commands"], None);
    assert_eq!(code, 1, "daemon unreachable → exit 1: {stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["status"], "error", "{stdout}");
}

/// Argument errors on the discovery·call surface follow the same discipline as the old families — **they do not touch the network**
///   (destination = port 0).
///
/// ★It asserts down to the reason★: asserting only exit 1 + BAD_ARGS was green on the old binary too (all these cases were bounced as
///   "unknown family" before this surface existed); pinning which rule each line hits turns it red when that rule disappears.
#[test]
fn engram_catalog_and_call_argument_errors_never_touch_the_network() {
    for (args, expect) in [
        (vec!["commands", "agent.list", "extra"], "at most one"),
        (vec!["commands", "--help"], "not a command name"),
        (vec!["commands", "-h"], "not a command name"),
        // The call shape does not take positional arguments — looking at the schema would not change the answer, so it is cut here.
        (vec!["agent.list", "oops"], "is not a flag"),
        // The flag check comes before the name check (if a dotted typo flag were read as a name, it would waste a round trip).
        (vec!["--nope.nope"], "not a flag"),
        // Anything that is not `<family>.<verb>` cannot be in the table — no wasting an authenticated POST.
        (vec!["."], "neither side may be empty"),
        (vec![".."], "neither side may be empty"),
        (vec![".foo"], "neither side may be empty"),
        (vec!["foo."], "neither side may be empty"),
        (vec!["README.md", "--x", "1"], "unknown command"),
    ] {
        let (stdout, code) = run_cli(UNREACHABLE_URL, &args, None);
        assert_eq!(code, 1, "argument error → exit 1 ({args:?}): {stdout}");
        let v: serde_json::Value =
            serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("stdout json: {e}"));
        let hint = v["hint"].as_str().unwrap_or_default();
        // `README.md` has a fine shape, so it can only be known by asking the list — only that line ends in a connection failure (one
        //   round trip).
        if expect == "unknown command" {
            assert_eq!(v["code"], "CONNECT_FAILED", "{args:?}: {stdout}");
            continue;
        }
        assert_eq!(v["code"], "BAD_ARGS", "{args:?} → BAD_ARGS: {stdout}");
        assert!(
            hint.contains(expect),
            "the reason must state this rule ({args:?}, {expect:?}): {hint}"
        );
    }
}

/// ★help does not receive the discovery list — the table-of-contents screen only points to `engram commands`★: otherwise one screen would
///   take two round trips and a list failure would look like a help failure;
///   the script has one screen, so a second round trip would fail the connection and break exit 0.
#[test]
fn the_static_help_points_at_the_catalog_without_fetching_it() {
    let (root, code, requests) = run_help_against_stub(&["help"], "stub root");
    assert_eq!(code, 0, "{root}");
    assert_help_request(&requests, r#"{"topic":null}"#);
    assert!(
        requests.iter().all(|r| !r.contains("/control/commands")),
        "help does not call the list route: {requests:?}"
    );
}

/// The discovery list emits only **rows the daemon sent** — the only place family names appear on the screen must be those rows.
///
/// ★The fixture carrying a mail name is the point★: without that word in the fixture the assertion was always true and passed on any
///   implementation; now `mail.send` is in the list, so any of our chrome (headers·section titles·guidance) adding it grows the line count
///   and turns red.
/// ★It does not erase names the table carried★: what is carried is daemon policy (parked separately); removing it would make discovery not
///   say "it exists" — this CLI is not the layer to fix that.
#[test]
fn the_catalog_surface_renders_only_the_rows_the_daemon_sent() {
    let (host, port, stub) = spawn_scripted_stub(vec![sample_catalog()]);
    let url = format!("http://{host}:{port}");
    let out = Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["commands"])
        .env("ENGRAM_TOKEN", "test-token")
        .env("ENGRAM_CONTROL_URL", &url)
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
        "the mail word must be only in rows the daemon sent: {stdout}"
    );
    assert!(
        leaking[0]
            .trim_start()
            .starts_with(&format!("{CLI_GROUP_MAIL}.send")),
        "that one line must be the row the table carried: {}",
        leaking[0]
    );
}

// ── ADR-0133: the daemon refuses mail — the CLI blocks neither the screen nor execution ─────────────────────

/// ★A mail screen request is the same whatever the environment carries★ — the marker (`ENGRAM_MAIL`) that picked the screen by spawn env
///   was removed; reviving it would let **a value the agent itself can erase** decide the screen again.
///   That the screen is the same regardless of the caller is measured by the daemon integration test (`tests/mail_gate.rs`).
///
/// ★Measuring both conditions side by side is the point★: an env without the marker and one with the old marker must send **the same**
///   request; one side alone stays green if an environment-bound branch revives.
// ADR-0133
#[test]
fn the_mail_screen_opens_whatever_the_environment_carries() {
    let screen = format!("{CLI_EXE_NAME} {CLI_GROUP_MAIL} - stub screen");
    for (label, marker) in [("no marker", None), ("old mail marker", Some("0"))] {
        for args in [
            vec!["help", CLI_GROUP_MAIL],
            vec![CLI_GROUP_MAIL, "--help"],
            vec![CLI_GROUP_MAIL, "-h"],
        ] {
            let (host, port, stub) = spawn_scripted_stub(vec![screen_response(&screen)]);
            let mut cli = Command::new(env!("CARGO_BIN_EXE_engram"));
            cli.args(&args)
                .env("ENGRAM_TOKEN", "test-token")
                .env("ENGRAM_CONTROL_URL", format!("http://{host}:{port}"));
            match marker {
                Some(value) => cli.env("ENGRAM_MAIL", value),
                None => cli.env_remove("ENGRAM_MAIL"),
            };
            let out = cli.output().expect("spawn engram CLI");
            let requests = stub.join().expect("stub join");
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            assert_eq!(
                out.status.code().unwrap_or(-1),
                0,
                "{label}: the mail screen exits successfully ({args:?}): {stdout}"
            );
            assert_eq!(stdout, format!("{screen}\n"), "{label}: {args:?}");
            assert_help_request(&requests, r#"{"topic":"mail"}"#);
        }
    }
}

/// ★Enforcement is the daemon's alone (ADR-0133 §Impact)★: a send typed with credentials that will be refused also **actually goes over the
///   network** — cutting it here beforehand would leave that refusal unobserved.
// ADR-0133
#[test]
fn a_mail_verb_the_daemon_refuses_still_posts_and_surfaces_the_refusal() {
    let rejection = r#"{"status":"error","code":"MAIL_NOT_ALLOWED","hint":"This credential is not allowed to use mail. Retrying will not change that."}"#;
    let (host, port, stub) = spawn_capturing_stub(ok_response(rejection));
    let out = Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["mail", "send", "--to", "bob", "--body", "hi"])
        .env("ENGRAM_TOKEN", "test-token")
        .env("ENGRAM_CONTROL_URL", format!("http://{host}:{port}"))
        .output()
        .expect("spawn engram CLI");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let request = stub.join().expect("stub thread");
    assert!(
        request.contains("POST /control/send"),
        "the request must go out — not a local block: {request}"
    );
    assert_eq!(
        out.status.code().unwrap_or(-1),
        1,
        "a daemon refusal is a failure (1) of the existing three-way split: {stdout}"
    );
    assert!(
        stdout.contains("MAIL_NOT_ALLOWED"),
        "the refusal reason must flow through as is: {stdout}"
    );
}

// ── ADR-0285: the common failure path of commands that call the daemon ──────────────────────────────────────────────

/// ★`--body-stdin` without credentials fails without waiting for stdin★ — reading stdin first would hang on an open stdin (agent shell ·
///   pipe), neither failing nor reaching the common failure path.
///   Launches with stdin open but neither written nor closed and checks `NO_TOKEN` within the deadline.
// ADR-0285
#[test]
fn a_body_stdin_send_without_credentials_fails_without_waiting_for_stdin() {
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let mut child = Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["mail", "send", "--to", "bob", "--body-stdin"])
        .env_remove("ENGRAM_TOKEN")
        .env_remove("ENGRAM_CONTROL_URL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn engram CLI");
    // The handle must be held to the end so the other side does not see EOF — that is what reproduces 「an stdin that stays open」.
    let held_stdin = child.stdin.take().expect("stdin piped");

    let deadline = Instant::now() + Duration::from_secs(5);
    let exited = loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break true,
            None if Instant::now() >= deadline => break false,
            None => thread::sleep(Duration::from_millis(20)),
        }
    };
    if !exited {
        let _ = child.kill();
        let _ = child.wait();
        panic!("--body-stdin without credentials did not end within 5 seconds — it is reading stdin before the credentials");
    }
    let out = child.wait_with_output().expect("collect engram CLI output");
    drop(held_stdin);

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(out.status.code().unwrap_or(-1), 1, "{stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout json: {e} — {stdout}"));
    assert_eq!(v["code"], "NO_TOKEN", "{stdout}");
}

/// ★If the daemon does not know that route (404 — a daemon of a different build), help and other commands give the same
///   `PROTOCOL_MISMATCH` envelope · exit 1★ — the 404 body is empty, so passing it through as an empty line would teach nothing.
// ADR-0285
#[test]
fn a_daemon_that_does_not_serve_the_route_is_reported_as_a_version_mismatch() {
    let not_found = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    for args in [vec!["help"], vec!["agent", "list"]] {
        let (host, port, stub) = spawn_scripted_stub(vec![not_found]);
        let (stdout, code) = run_cli(&format!("http://{host}:{port}"), &args, None);
        let requests = stub.join().expect("stub join");
        assert_eq!(requests.len(), 1, "one round trip ({args:?}): {requests:?}");
        assert_eq!(
            code, 1,
            "a version mismatch is a failure ({args:?}): {stdout}"
        );
        let v: serde_json::Value = serde_json::from_str(stdout.trim())
            .unwrap_or_else(|e| panic!("stdout json({args:?}): {e} — {stdout}"));
        assert_eq!(v["status"], "error", "{stdout}");
        assert_eq!(v["code"], "PROTOCOL_MISMATCH", "{args:?}: {stdout}");
        // It must state which route it does not know — the path actually typed is taken out of the request line and compared.
        let requested = requests[0].split_whitespace().nth(1).unwrap_or_default();
        assert!(
            requested.starts_with("/control/")
                && v["hint"].as_str().unwrap_or_default().contains(requested),
            "the hint must state the typed route ({requested}) ({args:?}): {stdout}"
        );
    }
}

// ── launch waits (ADR-0369) ─────────────────────────────────────────────────────────────────

/// A stub that answers requests in order, each after its own silence — for measuring the CLI's silence window.
fn spawn_slow_stub(
    script: Vec<(std::time::Duration, &'static str)>,
) -> (String, u16, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
    let addr = listener.local_addr().expect("addr");
    let handle = thread::spawn(move || {
        let mut seen = Vec::new();
        for (silence, response) in script {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            seen.push(read_request(&mut stream));
            thread::sleep(silence);
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
        seen
    });
    (addr.ip().to_string(), addr.port(), handle)
}

/// A catalog with one launch-class entry (`agent.spawnInto`, as the shell registers it) and one ordinary entry.
fn launch_catalog() -> &'static str {
    let args = serde_json::json!({
        "type": "object",
        "properties": { "cwd": { "type": "string" } },
        "required": ["cwd"]
    });
    let ok = serde_json::json!({ "type": "object", "properties": {}, "required": [] });
    let mut launch: serde_json::Value = serde_json::from_str(&schema_blob(
        "agent.spawnInto",
        "spawn and place",
        args.clone(),
        ok.clone(),
    ))
    .expect("blob");
    launch["deadline"] = serde_json::json!("Launch");
    catalog_response(vec![
        catalog_row("agent.spawnInto", &launch.to_string(), true),
        catalog_row(
            "tab.create",
            &schema_blob("tab.create", "open a tab", args, ok),
            true,
        ),
    ])
}

/// The CLI waits past its shared 10 s silence window for a launch — `engram agent spawn`, and a call whose catalog entry
/// states the launch class — while an ordinary call still gives up at 10 s. The three run side by side (the stub's
/// silence is real time: 11 s).
#[test]
fn launch_verbs_wait_past_the_shared_silence_window_and_ordinary_calls_do_not() {
    let silence = std::time::Duration::from_secs(11);
    let run = |script: Vec<(std::time::Duration, &'static str)>, args: &'static [&'static str]| {
        thread::spawn(move || {
            let (host, port, stub) = spawn_slow_stub(script);
            let url = format!("http://{host}:{port}");
            let started = std::time::Instant::now();
            let (stdout, code) = run_cli(&url, args, None);
            let took = started.elapsed();
            drop(stub);
            (stdout, code, took)
        })
    };
    let spawn = run(
        vec![(
            silence,
            ok_response(r#"{"agent_id":"i","name":"w","state":"live","created":false}"#),
        )],
        &["agent", "spawn", "w"],
    );
    let launch_call = run(
        vec![
            (std::time::Duration::ZERO, launch_catalog()),
            (silence, ok_response("{}")),
        ],
        &["agent.spawnInto", "--cwd", "C:/work"],
    );
    let ordinary_call = run(
        vec![
            (std::time::Duration::ZERO, launch_catalog()),
            (silence, ok_response("{}")),
        ],
        &["tab.create", "--cwd", "C:/work"],
    );

    let (stdout, code, took) = spawn.join().expect("agent spawn");
    assert_eq!(code, 0, "agent spawn waits past 10 s: {stdout}");
    assert!(took >= silence, "{took:?}");
    let (stdout, code, took) = launch_call.join().expect("launch call");
    assert_eq!(code, 0, "a launch-class call waits past 10 s: {stdout}");
    assert!(took >= silence, "{took:?}");
    // Exit 1 shows the CLI did not wait for the reply (a CLI that waited would get `{}` and exit 0) — not by itself that
    // the failure was the 10 s window; a wall-clock upper bound here would count process start-up too and flake.
    let (stdout, code, _took) = ordinary_call.join().expect("ordinary call");
    assert_eq!(code, 1, "an ordinary call gives up at its 10 s: {stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["status"], "error", "{stdout}");
}
