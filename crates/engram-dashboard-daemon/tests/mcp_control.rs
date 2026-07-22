//! ADR-0086 스텝 1 통합 테스트 — 데몬 MCP 제어 채널 입구(토큰 auth + 세션 바인딩 + engram_ping).
//!
//! 실 claude 없이(in-process) 데몬 MCP 엔드포인트를 띄우고, HTTP/MCP 클라이언트로 검증한다:
//!   - 무/오/stale-epoch 토큰 → MCP 세션 생기기 전 401(reqwest raw 요청).
//!   - 유효 토큰 → initialize 200 + 세션 바인딩(registry 관측) + tools/list 에 engram_ping + tools/call
//!     이 바인딩된 신원을 되돌린다(rmcp 클라이언트로 handshake~call 전 과정).
//!
//! ★엔드포인트는 데몬이 생성★: mcp-config 포트를 daemon.json 에 싣지 않는다(ADR-0086 — 데몬이 자체
//!   생성). 여기선 start_mcp_server 가 돌려준 URL 을 그대로 클라이언트가 쓴다.

use std::sync::Arc;

use engram_dashboard_core::agent::types::AgentId;
use engram_dashboard_daemon::control::mcp_server::{start_mcp_server, ManagerSlot};
use engram_dashboard_daemon::control::registry::ControlRegistry;

/// 빈 manager 슬롯(스텝 1 테스트는 send 를 안 부르므로 relay 대상 불필요). 헬퍼로 반복 제거.
fn empty_slot() -> Arc<ManagerSlot> {
    Arc::new(ManagerSlot::new())
}

/// initialize JSON-RPC 바디(POST /mcp). rmcp 는 Accept: application/json+text/event-stream 를 요구하나,
/// 401 검증은 auth 미들웨어가 handshake **전에** 막으므로 Accept 헤더 유무와 무관하게 401 이 나야 한다.
fn initialize_body() -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "test-client", "version": "0.0.0" }
        }
    })
}

/// raw HTTP POST /mcp with 주어진 Authorization 헤더(없으면 미첨부) → 상태 코드.
async fn post_initialize(url: &str, bearer: Option<&str>) -> reqwest::StatusCode {
    let client = reqwest::Client::new();
    let mut req = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .json(&initialize_body());
    if let Some(b) = bearer {
        req = req.header("Authorization", format!("Bearer {b}"));
    }
    req.send().await.expect("http request").status()
}

/// raw HTTP GET /mcp(SSE 스트림 요청) with 주어진 Authorization → 상태 코드. auth 미들웨어가
/// handshake 전 401 을 내는지 검증용(토큰 없으면 세션 조회 전 차단).
async fn get_stream(url: &str, bearer: Option<&str>) -> reqwest::StatusCode {
    let client = reqwest::Client::new();
    let mut req = client.get(url).header("Accept", "text/event-stream");
    if let Some(b) = bearer {
        req = req.header("Authorization", format!("Bearer {b}"));
    }
    req.send().await.expect("http request").status()
}

/// raw HTTP DELETE /mcp with 주어진 Authorization + (선택) Mcp-Session-Id → 상태 코드.
async fn delete_session(
    url: &str,
    bearer: Option<&str>,
    session_id: Option<&str>,
) -> reqwest::StatusCode {
    let client = reqwest::Client::new();
    let mut req = client.delete(url);
    if let Some(b) = bearer {
        req = req.header("Authorization", format!("Bearer {b}"));
    }
    if let Some(s) = session_id {
        req = req.header("Mcp-Session-Id", s);
    }
    req.send().await.expect("http request").status()
}

/// POST initialize 로 세션을 열고 (상태코드, Mcp-Session-Id) 를 돌려준다. 유효 토큰이면 200 + 세션 id.
async fn open_session(url: &str, bearer: &str) -> (reqwest::StatusCode, Option<String>) {
    let client = reqwest::Client::new();
    let resp = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {bearer}"))
        .json(&initialize_body())
        .send()
        .await
        .expect("http request");
    let status = resp.status();
    let sid = resp
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    (status, sid)
}

/// tools/list JSON-RPC 를 주어진 세션 id + 토큰으로 POST → 상태 코드. cross-token/revoked-mid-session
/// 검증에 쓴다(세션을 실어 보내는 후속 요청 형태).
async fn post_tools_list(url: &str, bearer: &str, session_id: &str) -> reqwest::StatusCode {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    });
    client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {bearer}"))
        .header("Mcp-Session-Id", session_id)
        .json(&body)
        .send()
        .await
        .expect("http request")
        .status()
}

/// 주어진 크기(bytes)의 filler 를 담은 tools/list 요청을 세션 id + 토큰으로 POST → 결과.
/// body 상한(F4)을 넘기는 큰 요청을 만들 때 쓴다. filler 는 무해한 params 필드에 실어 JSON 유효성 유지.
///
/// ★반환이 Result 인 이유(Windows 특성)★: RequestBodyLimitLayer 는 Content-Length 가 상한을 넘으면
///   body 를 읽지 않고 즉시 413 을 응답하고 연결을 닫는다. 그런데 클라(reqwest)가 아직 큰 body 를
///   업로드하는 중이라 서버가 소켓을 먼저 닫으면 OS 가 연결을 reset 해(WinError 10053), reqwest 가 413
///   응답을 읽기 전에 ConnectionAborted 로 실패할 수 있다. 둘 다 "상한 초과 → 처리 거부"의 표현이므로
///   호출자가 (Ok(413) | Err(connection-abort)) 를 모두 거부로 받아들이게 Result 를 그대로 돌려준다.
async fn post_tools_list_with_padding(
    url: &str,
    bearer: &str,
    session_id: &str,
    padding_bytes: usize,
) -> Result<reqwest::StatusCode, reqwest::Error> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/list",
        "params": { "_pad": "x".repeat(padding_bytes) }
    });
    client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {bearer}"))
        .header("Mcp-Session-Id", session_id)
        .json(&body)
        .send()
        .await
        .map(|r| r.status())
}

#[tokio::test]
async fn missing_unknown_stale_tokens_are_rejected_before_session() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    // provision 시뮬레이션 — epoch 0 산 토큰 발급.
    registry.issue(id, 0, "valid-token-epoch0".to_string());
    // epoch 회전(재활성화) — epoch 0 토큰은 폐기되고 epoch 1 이 산 토큰.
    registry.issue(id, 1, "valid-token-epoch1".to_string());

    let handle = start_mcp_server(registry.clone(), empty_slot())
        .await
        .expect("start mcp server");
    let url = &handle.url;

    // 무토큰 → 401.
    assert_eq!(
        post_initialize(url, None).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "no token → 401 before handshake"
    );
    // 모르는 토큰 → 401.
    assert_eq!(
        post_initialize(url, Some("bogus")).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "unknown token → 401"
    );
    // stale-epoch(회전으로 폐기된 epoch 0) 토큰 → 401.
    assert_eq!(
        post_initialize(url, Some("valid-token-epoch0")).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "stale-epoch token → 401"
    );

    // 401 이 났으니 어떤 MCP 세션도 바인딩되지 않아야 한다(handshake 전 차단).
    assert_eq!(
        registry.bound_session_count(),
        0,
        "401 경로는 어떤 세션도 만들지 않아야 함"
    );

    handle.shutdown().await;
}

#[tokio::test]
async fn valid_token_initializes_binds_session_and_ping_returns_identity() {
    use rmcp::model::CallToolRequestParams;
    use rmcp::transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    };
    use rmcp::ServiceExt;

    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 7, "good-token".to_string());

    let handle = start_mcp_server(registry.clone(), empty_slot())
        .await
        .expect("start mcp server");

    // rmcp 클라이언트로 handshake(initialize + notifications/initialized) — auth_header 는 raw 토큰
    //   (클라가 reqwest .bearer_auth 로 "Bearer " 를 붙인다 → 서버 미들웨어의 strip_prefix 와 대칭).
    let config =
        StreamableHttpClientTransportConfig::with_uri(handle.url.clone()).auth_header("good-token");
    let transport = StreamableHttpClientTransport::from_config(config);
    let client = ().serve(transport).await.expect("MCP handshake with valid token");

    // 세션 바인딩 관측 — handshake 성공 시 데몬이 (AgentId, epoch) 세션을 붙잡았다.
    assert_eq!(
        registry.bound_session_count(),
        1,
        "유효 토큰 initialize 후 세션 1개 바인딩(acceptance)"
    );

    // tools/list 에 engram_ping 이 있어야 한다(= 에이전트 system:init 이 이 툴을 본다).
    let tools = client.list_all_tools().await.expect("list tools");
    assert!(
        tools.iter().any(|t| t.name == "engram_ping"),
        "system:init tools 에 engram_ping 존재: {:?}",
        tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>()
    );

    // tools/call engram_ping → 바인딩된 신원(agent=<id> epoch=7)을 되돌려야 한다(end-to-end 증명).
    //   CallToolRequestParams 는 #[non_exhaustive](타 크레이트) → 리터럴 불가, Default 후 필드 설정.
    let mut params = CallToolRequestParams::default();
    params.name = "engram_ping".into();
    params.arguments = Some(serde_json::Map::new());
    let result = client.call_tool(params).await.expect("call engram_ping");
    let text = result
        .content
        .iter()
        .find_map(|c| c.as_text().map(|t| t.text.clone()))
        .expect("engram_ping returns text content");
    assert!(
        text.contains(&id.to_string()) && text.contains("epoch=7"),
        "engram_ping 이 바인딩된 신원을 반환: {text}"
    );

    let _ = client.cancel().await;
    handle.shutdown().await;
}

// ── FIX 9: GET/DELETE 무토큰 401 ────────────────────────────────────────────────────
#[tokio::test]
async fn get_and_delete_without_token_are_rejected() {
    let registry = Arc::new(ControlRegistry::new());
    let handle = start_mcp_server(registry, empty_slot())
        .await
        .expect("start mcp server");
    let url = &handle.url;

    // GET(SSE) 무토큰 → 401(auth 미들웨어가 세션 조회 전 차단).
    assert_eq!(
        get_stream(url, None).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "no token GET → 401 before session lookup"
    );
    // DELETE 무토큰 → 401.
    assert_eq!(
        delete_session(url, None, Some("whatever")).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "no token DELETE → 401"
    );

    handle.shutdown().await;
}

// ── FIX 9/7: cross-token 세션 탈취 거부(403) ────────────────────────────────────────
#[tokio::test]
async fn cross_token_session_takeover_is_rejected() {
    let registry = Arc::new(ControlRegistry::new());
    let id_a = AgentId::new_v4();
    let id_b = AgentId::new_v4();
    registry.issue(id_a, 0, "token-a".to_string());
    registry.issue(id_b, 0, "token-b".to_string());

    let handle = start_mcp_server(registry.clone(), empty_slot())
        .await
        .expect("start mcp server");
    let url = &handle.url;

    // 토큰 A 로 세션 S 를 연다(initialize) → 세션 id 확보 + 신원 A 로 고정(bind_session_if_absent).
    let (status, sid) = open_session(url, "token-a").await;
    assert_eq!(status, reqwest::StatusCode::OK, "token A initialize 200");
    let sid = sid.expect("initialize 가 Mcp-Session-Id 를 돌려줘야");

    // 같은 세션 S 에 토큰 B 로 후속 요청 → 신원 불일치 → 403(탈취 거부).
    assert_eq!(
        post_tools_list(url, "token-b", &sid).await,
        reqwest::StatusCode::FORBIDDEN,
        "다른 토큰(B)으로 세션 S 접근 → 403(cross-token takeover 거부)"
    );

    // 대조군: 원래 토큰 A 로는 같은 세션에 정상 접근(403 이 무차별 거부가 아님을 확인).
    assert_eq!(
        post_tools_list(url, "token-a", &sid).await,
        reqwest::StatusCode::OK,
        "원 토큰 A 로는 세션 S 정상 접근(200)"
    );

    handle.shutdown().await;
}

// ── FIX 9: 세션 중간 revoke → 후속 요청 거부 ────────────────────────────────────────
#[tokio::test]
async fn revoked_mid_session_request_is_rejected() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "live-token".to_string());

    let handle = start_mcp_server(registry.clone(), empty_slot())
        .await
        .expect("start mcp server");
    let url = &handle.url;

    // 세션을 열고(200) 정상 동작 확인.
    let (status, sid) = open_session(url, "live-token").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let sid = sid.expect("session id");
    assert_eq!(
        post_tools_list(url, "live-token", &sid).await,
        reqwest::StatusCode::OK,
        "revoke 전 후속 요청은 200"
    );

    // 세션 도중 토큰 폐기(kill/terminal 모사) → validate None → 후속 요청 401.
    registry.revoke(id, 0);
    assert_eq!(
        post_tools_list(url, "live-token", &sid).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "revoke 후 같은 토큰 후속 요청 → 401(validate None)"
    );

    handle.shutdown().await;
}

// ── FIX 9: epoch 회전 → 옛 토큰 401 + 옛 config 삭제 + 새 config 존재 ──────────────────
// DaemonControlChannel.provision/revoke 를 직접 돌려 mcp-config 파일 생명주기(회전=구 폐기+새 발급)를
// 단언한다. registry 회전(issue)이 구 토큰을 evict 하는 건 registry unit 이 이미 커버하므로, 여기선
// **파일** 측(구 파일 삭제 + 새 파일 존재)을 본다.
#[tokio::test]
async fn epoch_rotation_revokes_old_token_and_config_file() {
    use engram_dashboard_core::agent::types::ControlChannel;
    use engram_dashboard_daemon::control::mcp_config;
    use engram_dashboard_daemon::control::priming::NoopPrimingProvider;
    use engram_dashboard_daemon::control::DaemonControlChannel;

    let registry = Arc::new(ControlRegistry::new());
    let handle = start_mcp_server(registry.clone(), empty_slot())
        .await
        .expect("start mcp server");

    let data_dir = std::env::temp_dir().join(format!("engram-mcp-rotate-{}", AgentId::new_v4()));
    let channel = DaemonControlChannel::new(
        registry.clone(),
        handle.url.clone(),
        data_dir.clone(),
        None,
        Arc::new(NoopPrimingProvider), // ADR-0092: epoch 회전 테스트 — 프라이밍 무관.
    );

    let id = AgentId::new_v4();
    // epoch 0 provision — 토큰 발급 + config 파일 기록. ADR-0099: MCP-capable(true)로 config 파일 기록.
    let ep0 = channel
        .provision(id, 0, true)
        .expect("provision ok")
        .expect("epoch0 endpoint");
    let old_token = ep0.token.clone();
    let old_path = mcp_config::config_path(&data_dir, id, 0);
    assert!(old_path.exists(), "epoch0 config 파일 생성");
    assert!(registry.validate(&old_token).is_some(), "epoch0 토큰 유효");

    // 재활성화(epoch 1) provision — registry.issue 가 구 토큰을 evict, 새 config 파일 기록.
    let ep1 = channel
        .provision(id, 1, true)
        .expect("provision ok")
        .expect("epoch1 endpoint");
    let new_path = mcp_config::config_path(&data_dir, id, 1);
    assert!(new_path.exists(), "epoch1 config 파일 생성");
    // 옛 epoch 토큰은 회전으로 폐기(401 대상) — 서버에 실제로 붙여 401 을 확인.
    assert_eq!(
        post_initialize(&handle.url, Some(&old_token)).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "회전된 구 epoch 토큰 → 401"
    );
    assert!(
        registry.validate(&ep1.token).is_some(),
        "새 epoch1 토큰 유효"
    );

    // 옛 epoch 을 revoke → 구 config 파일 삭제(idempotent). (issue 는 registry 만 evict, 파일은 revoke 가 지움.)
    channel.revoke(id, 0);
    assert!(!old_path.exists(), "revoke(epoch0) 후 구 config 파일 삭제");
    assert!(new_path.exists(), "새 epoch1 config 파일은 남아 있어야");

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── round-2 F1: orphaned-session attach 거부 ──────────────────────────────────────────────
// 에이전트 A 가 세션 S 를 열어 바인딩 → A revoke(kill) → 데몬 바인딩 prune. 그때 rmcp 측 세션 S 는
// 살아 있을 수 있는데, 다른 유효 토큰 B 가 S 를 제시하면 예전엔 미들웨어가 통과시켜 B 가 A 의 고아
// 세션에 attach 했다. 이제 바인딩 없는 세션-실은 요청은 전부 거부(404)한다.
#[tokio::test]
async fn orphaned_session_attach_is_rejected() {
    let registry = Arc::new(ControlRegistry::new());
    let id_a = AgentId::new_v4();
    let id_b = AgentId::new_v4();
    registry.issue(id_a, 0, "token-a".to_string());
    registry.issue(id_b, 0, "token-b".to_string());

    let handle = start_mcp_server(registry.clone(), empty_slot())
        .await
        .expect("start mcp server");
    let url = &handle.url;

    // A 가 세션 S 를 연다(initialize) → 세션 바인딩(신원 A).
    let (status, sid) = open_session(url, "token-a").await;
    assert_eq!(status, reqwest::StatusCode::OK, "token A initialize 200");
    let sid = sid.expect("initialize 가 Mcp-Session-Id 를 돌려줘야");
    assert_eq!(registry.bound_session_count(), 1, "A 세션 바인딩됨");

    // A 를 revoke(kill 모사) → 데몬 바인딩 prune(rmcp 측 세션 S 는 살아 있을 수 있음).
    registry.revoke(id_a, 0);
    assert_eq!(
        registry.bound_session_count(),
        0,
        "revoke 로 A 바인딩 prune"
    );

    // B(유효 토큰)가 고아 세션 S 를 제시 → 바인딩 없음 → 404(orphaned 거부, attach 차단).
    assert_eq!(
        post_tools_list(url, "token-b", &sid).await,
        reqwest::StatusCode::NOT_FOUND,
        "고아 세션 S 에 B 토큰 attach → 404(F1 orphaned-session 거부)"
    );

    handle.shutdown().await;
}

// ── round-2 F1: 아예 모르는(bound 된 적 없는) 세션 id 도 거부 ──────────────────────────────────
// 정상 클라이언트가 예전에 rmcp 404 를 받던 "truly-unknown id" 도 이제 미들웨어가 404 로 끊는다
// (happy-path 상태코드 불변 — 정상 클라 흐름 무영향). 유효 토큰 + 존재한 적 없는 세션 id 로 확인.
#[tokio::test]
async fn unknown_session_id_is_rejected_not_forwarded() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "valid".to_string());
    let handle = start_mcp_server(registry, empty_slot())
        .await
        .expect("start mcp server");
    let url = &handle.url;

    // 유효 토큰이지만 한 번도 바인딩된 적 없는 세션 id → 404(inner 로 forward 하지 않음).
    assert_eq!(
        post_tools_list(url, "valid", "never-bound-session-id").await,
        reqwest::StatusCode::NOT_FOUND,
        "미지 세션 id → 404(inner 미도달, F1)"
    );

    handle.shutdown().await;
}

// ── Codex LOW: malformed(비-UTF-8) Mcp-Session-Id 헤더 → 400(inner 미도달) ─────────────────────
// 헤더가 present-but-malformed 이면 예전엔 to_str() 실패로 None 이 돼 sessionless 로 오인, 바인딩 검사를
// 건너뛰고 inner(rmcp)로 통과했다(경계 우회). 이제 malformed 는 400 으로 끊는다(절대 200/forward 아님).
#[tokio::test]
async fn malformed_session_id_header_is_rejected_with_400() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "malformtok".to_string());
    let handle = start_mcp_server(registry, empty_slot())
        .await
        .expect("start mcp server");
    let url = &handle.url;

    // 비-UTF-8 바이트를 담은 Mcp-Session-Id 헤더(HeaderValue::from_bytes 로만 만들 수 있는 malformed 값).
    let bad_sid = reqwest::header::HeaderValue::from_bytes(&[0xff, 0xfe, 0x80, 0x81])
        .expect("bytes → HeaderValue(값 자체는 유효, to_str 만 실패)");
    let client = reqwest::Client::new();
    let status = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .header("Authorization", "Bearer malformtok")
        .header("Mcp-Session-Id", bad_sid)
        .json(&initialize_body())
        .send()
        .await
        .expect("http request")
        .status();
    assert_eq!(
        status,
        reqwest::StatusCode::BAD_REQUEST,
        "malformed(비-UTF-8) Mcp-Session-Id → 400(inner 미도달, sessionless 오인 아님). 200 이면 우회 = 실패"
    );

    handle.shutdown().await;
}

// ── security lens: 세션 operation(GET/DELETE) 무-세션id → 400(inner 미도달) ──────────────────────
// GET(SSE)·DELETE(teardown)은 기존 세션에 대한 조작이라 세션 id 가 반드시 있어야 한다. 세션 id 없는
// GET/DELETE 는 바인딩으로 resolve 될 수 없으므로 미들웨어에서 400 으로 끊는다(inner rmcp 4xx 에 의존 X).
// (POST 무-세션id 는 예외 = initialize — 아래 regression 테스트가 그 경로가 여전히 통함을 지킨다.)
#[tokio::test]
async fn session_ops_without_session_id_are_rejected_with_400() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "optok".to_string());
    let handle = start_mcp_server(registry, empty_slot())
        .await
        .expect("start mcp server");
    let url = &handle.url;

    // 유효 토큰 + GET(SSE) + 세션 id 없음 → 400(세션 operation 은 세션을 지목해야).
    assert_eq!(
        get_stream(url, Some("optok")).await,
        reqwest::StatusCode::BAD_REQUEST,
        "유효 토큰 GET(무-세션id) → 400(session op 는 바인딩으로 resolve 돼야, inner 미도달)"
    );
    // 유효 토큰 + DELETE + 세션 id 없음 → 400.
    assert_eq!(
        delete_session(url, Some("optok"), None).await,
        reqwest::StatusCode::BAD_REQUEST,
        "유효 토큰 DELETE(무-세션id) → 400(session op 는 바인딩으로 resolve 돼야)"
    );

    handle.shutdown().await;
}

// ── REGRESSION: POST initialize(무-세션id)는 여전히 inner 도달(초기화는 세션 id 가 아직 없는 게 정상) ──
// GET/DELETE 무-세션id 400 규칙이 POST initialize 경로를 깨면 안 된다. 유효 토큰 + POST + 세션 id 없음 →
// 200(정상 initialize, inner 도달) + 세션 바인딩 관측. (happy-path initialize→bind 는 위 별도 테스트가
// rmcp 클라로 end-to-end 커버 — 여기선 raw POST 로 "POST 무-세션id 예외가 유지됨"만 국소 확인.)
#[tokio::test]
async fn post_initialize_without_session_id_still_reaches_inner() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "inittok".to_string());
    let handle = start_mcp_server(registry.clone(), empty_slot())
        .await
        .expect("start mcp server");
    let url = &handle.url;

    // POST initialize + 세션 id 없음 → 200(초기화는 세션 id 가 아직 없는 게 정상 = 400 예외).
    let (status, sid) = open_session(url, "inittok").await;
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "POST initialize(무-세션id)는 여전히 inner 도달(200). 400 으로 막히면 initialize 파괴 = 실패"
    );
    assert!(
        sid.is_some(),
        "initialize 가 Mcp-Session-Id 를 돌려줘야(inner rmcp 가 세션 생성)"
    );
    assert_eq!(
        registry.bound_session_count(),
        1,
        "POST initialize 후 세션 바인딩 1개(POST 무-세션id 예외 경로 정상)"
    );

    handle.shutdown().await;
}

// ── round-2 F4: body 상한이 nested rmcp 서비스까지 도달(413) + 정상 요청 무영향 ────────────────
// axum DefaultBodyLimit 는 extractor 만 제한하나 rmcp 는 raw body 를 직접 소비한다. RequestBodyLimitLayer
// 로 교체해 하위 소비자 전부에 상한을 강제한다. >1MB 요청 → 413, 정상(<1MB) 요청은 통과함을 본다.
#[tokio::test]
async fn oversize_body_is_rejected_with_413() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "sizetok".to_string());
    let handle = start_mcp_server(registry.clone(), empty_slot())
        .await
        .expect("start mcp server");
    let url = &handle.url;

    // 세션을 열어(200) 정상 세션 id 확보 — body-limit 는 auth·세션 검사와 독립임을 보이려 유효 세션에 건다.
    let (status, sid) = open_session(url, "sizetok").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let sid = sid.expect("session id");

    // 정상 크기(작은 padding) 요청 → 통과(200). body-limit 가 정상 요청을 막지 않음을 대조.
    assert_eq!(
        post_tools_list_with_padding(url, "sizetok", &sid, 1024)
            .await
            .expect("정상 요청은 연결 성공"),
        reqwest::StatusCode::OK,
        "1KB 요청은 상한 이하 → 정상 처리(무영향)"
    );

    // >1MB body → RequestBodyLimitLayer 가 상한 초과로 거부한다(rmcp raw-body 소비 전에 상한 적용).
    //   깨끗한 413 응답이거나(Content-Length 로 즉시 거부), 큰 body 업로드 중 서버가 소켓을 먼저 닫아
    //   생기는 connection-abort(Windows 10053) 둘 다 "처리 거부"의 표현이다 — 둘 중 하나면 통과.
    //   ★핵심(F4)★: 2MB body 가 rmcp 로 전달돼 처리되면 안 된다(200 이면 상한이 안 먹힌 것 = 실패).
    match post_tools_list_with_padding(url, "sizetok", &sid, 2 * 1024 * 1024).await {
        Ok(status) => assert_eq!(
            status,
            reqwest::StatusCode::PAYLOAD_TOO_LARGE,
            ">1MB 요청 → 413(F4 — 상한이 nested rmcp 까지 도달). 200 이면 상한 미적용 = 실패"
        ),
        // 서버가 상한 초과로 소켓을 닫아 업로드 중 reset — 처리 거부로 간주(413 과 동치 의미).
        Err(e) => assert!(
            e.is_request() || e.is_connect(),
            "상한 초과 거부는 connection-abort 로도 나타날 수 있음(Windows). 예상 밖 에러: {e:?}"
        ),
    }

    handle.shutdown().await;
}
