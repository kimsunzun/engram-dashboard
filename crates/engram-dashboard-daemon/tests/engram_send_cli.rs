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
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
    let addr = listener.local_addr().expect("addr");
    let handle = thread::spawn(move || {
        // 첫 연결 1건만 처리(CLI 는 요청 1회).
        if let Ok((mut stream, _)) = listener.accept() {
            // 요청을 조금 읽어 소켓을 소비(전부 안 읽어도 응답은 보낼 수 있다). non-blocking 회피 위해 짧게.
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            // Connection: close 응답이면 여기서 stream drop → 클라이언트가 EOF 를 본다.
        }
    });
    (addr.ip().to_string(), addr.port(), handle)
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
fn engram_send_enqueued_prints_ack_and_exits_zero() {
    // 200 + enqueued JSON(Content-Length) → stdout ACK + exit 0.
    let body = r#"{"status":"enqueued","id":"m1","to":"bob"}"#;
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

    assert_eq!(code, 0, "enqueued → exit 0. stdout={stdout}");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout json");
    assert_eq!(v["status"], "enqueued", "stdout 에 ACK JSON: {stdout}");
    assert_eq!(v["to"], "bob");
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
