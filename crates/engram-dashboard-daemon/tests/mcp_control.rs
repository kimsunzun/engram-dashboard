//! ADR-0086 스텝 1 통합 테스트 — 데몬 MCP 제어 채널 입구(토큰 auth + 세션 바인딩 + engram_ping).
//!
//! 실 claude 없이(in-process) 데몬 MCP 엔드포인트를 띄우고, HTTP/MCP 클라이언트로 검증한다.

use std::sync::Arc;

use engram_dashboard_agent::types::AgentId;
use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server, CommandTableSlot, ManagerSlot, MessagingSlot,
};
use engram_dashboard_daemon::control::registry::ControlRegistry;

/// 이 파일의 테스트는 send/flush 를 부르지 않으므로 슬롯은 빈 채로 둔다.
fn empty_slot() -> Arc<ManagerSlot> {
    Arc::new(ManagerSlot::new())
}

fn empty_messaging_slot() -> Arc<MessagingSlot> {
    Arc::new(MessagingSlot::new())
}

fn empty_commands_slot() -> Arc<CommandTableSlot> {
    Arc::new(CommandTableSlot::new())
}

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

async fn get_stream(url: &str, bearer: Option<&str>) -> reqwest::StatusCode {
    let client = reqwest::Client::new();
    let mut req = client.get(url).header("Accept", "text/event-stream");
    if let Some(b) = bearer {
        req = req.header("Authorization", format!("Bearer {b}"));
    }
    req.send().await.expect("http request").status()
}

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
    registry.issue(id, 0, "valid-token-epoch0".to_string(), true);
    registry.issue(id, 1, "valid-token-epoch1".to_string(), true);

    // ★수거기를 들고 있어야 한다★ — 떨어뜨리면 그 자리에서 자리 표가 닫혀 그 뒤 왕복이 전부 반려된다.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server(
        registry.clone(),
        empty_slot(),
        empty_messaging_slot(),
        empty_commands_slot(),
        relay_bus,
    )
    .await
    .expect("start mcp server");
    let url = &handle.url;

    assert_eq!(
        post_initialize(url, None).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "no token → 401 before handshake"
    );
    assert_eq!(
        post_initialize(url, Some("bogus")).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "unknown token → 401"
    );
    assert_eq!(
        post_initialize(url, Some("valid-token-epoch0")).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "stale-epoch token → 401"
    );

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
    registry.issue(id, 7, "good-token".to_string(), true);

    // ★수거기를 들고 있어야 한다★ — 떨어뜨리면 그 자리에서 자리 표가 닫혀 그 뒤 왕복이 전부 반려된다.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server(
        registry.clone(),
        empty_slot(),
        empty_messaging_slot(),
        empty_commands_slot(),
        relay_bus,
    )
    .await
    .expect("start mcp server");

    // auth_header 는 raw 토큰 — rmcp 클라가 reqwest .bearer_auth 로 "Bearer " 를 붙인다.
    let config =
        StreamableHttpClientTransportConfig::with_uri(handle.url.clone()).auth_header("good-token");
    let transport = StreamableHttpClientTransport::from_config(config);
    let client = ().serve(transport).await.expect("MCP handshake with valid token");

    assert_eq!(
        registry.bound_session_count(),
        1,
        "유효 토큰 initialize 후 세션 1개 바인딩(acceptance)"
    );

    let tools = client.list_all_tools().await.expect("list tools");
    assert!(
        tools.iter().any(|t| t.name == "engram_ping"),
        "system:init tools 에 engram_ping 존재: {:?}",
        tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>()
    );

    // CallToolRequestParams 는 #[non_exhaustive](타 크레이트) → 리터럴 불가, Default 후 필드 설정.
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

// ── FIX 9: GET/DELETE 무토큰 ────────────────────────────────────────────────────────
#[tokio::test]
async fn get_and_delete_without_token_are_rejected() {
    let registry = Arc::new(ControlRegistry::new());
    // ★수거기를 들고 있어야 한다★ — 떨어뜨리면 그 자리에서 자리 표가 닫혀 그 뒤 왕복이 전부 반려된다.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server(
        registry,
        empty_slot(),
        empty_messaging_slot(),
        empty_commands_slot(),
        relay_bus,
    )
    .await
    .expect("start mcp server");
    let url = &handle.url;

    assert_eq!(
        get_stream(url, None).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "no token GET → 401 before session lookup"
    );
    assert_eq!(
        delete_session(url, None, Some("whatever")).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "no token DELETE → 401"
    );

    handle.shutdown().await;
}

// ── FIX 9/7: cross-token 세션 탈취 ──────────────────────────────────────────────────
#[tokio::test]
async fn cross_token_session_takeover_is_rejected() {
    let registry = Arc::new(ControlRegistry::new());
    let id_a = AgentId::new_v4();
    let id_b = AgentId::new_v4();
    registry.issue(id_a, 0, "token-a".to_string(), true);
    registry.issue(id_b, 0, "token-b".to_string(), true);

    // ★수거기를 들고 있어야 한다★ — 떨어뜨리면 그 자리에서 자리 표가 닫혀 그 뒤 왕복이 전부 반려된다.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server(
        registry.clone(),
        empty_slot(),
        empty_messaging_slot(),
        empty_commands_slot(),
        relay_bus,
    )
    .await
    .expect("start mcp server");
    let url = &handle.url;

    let (status, sid) = open_session(url, "token-a").await;
    assert_eq!(status, reqwest::StatusCode::OK, "token A initialize 200");
    let sid = sid.expect("initialize 가 Mcp-Session-Id 를 돌려줘야");

    assert_eq!(
        post_tools_list(url, "token-b", &sid).await,
        reqwest::StatusCode::FORBIDDEN,
        "다른 토큰(B)으로 세션 S 접근 → 403(cross-token takeover 거부)"
    );

    assert_eq!(
        post_tools_list(url, "token-a", &sid).await,
        reqwest::StatusCode::OK,
        "원 토큰 A 로는 세션 S 정상 접근(200)"
    );

    handle.shutdown().await;
}

// ── FIX 9: 세션 중간 revoke ─────────────────────────────────────────────────────────
#[tokio::test]
async fn revoked_mid_session_request_is_rejected() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "live-token".to_string(), true);

    // ★수거기를 들고 있어야 한다★ — 떨어뜨리면 그 자리에서 자리 표가 닫혀 그 뒤 왕복이 전부 반려된다.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server(
        registry.clone(),
        empty_slot(),
        empty_messaging_slot(),
        empty_commands_slot(),
        relay_bus,
    )
    .await
    .expect("start mcp server");
    let url = &handle.url;

    let (status, sid) = open_session(url, "live-token").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let sid = sid.expect("session id");
    assert_eq!(
        post_tools_list(url, "live-token", &sid).await,
        reqwest::StatusCode::OK,
        "revoke 전 후속 요청은 200"
    );

    registry.revoke(id, 0);
    assert_eq!(
        post_tools_list(url, "live-token", &sid).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "revoke 후 같은 토큰 후속 요청 → 401(validate None)"
    );

    handle.shutdown().await;
}

// ── FIX 9: epoch 회전 ───────────────────────────────────────────────────────────────
// registry 회전(issue)의 구 토큰 evict 는 registry unit 이 커버한다 — 여기선 config 파일 수명까지 본다.
#[tokio::test]
async fn epoch_rotation_revokes_old_token_and_config_file() {
    use engram_dashboard_agent::types::ControlChannel;
    use engram_dashboard_daemon::control::mcp_config;
    use engram_dashboard_daemon::control::priming::NoopPrimingProvider;
    use engram_dashboard_daemon::control::DaemonControlChannel;

    let registry = Arc::new(ControlRegistry::new());
    // ★수거기를 들고 있어야 한다★ — 떨어뜨리면 그 자리에서 자리 표가 닫혀 그 뒤 왕복이 전부 반려된다.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server(
        registry.clone(),
        empty_slot(),
        empty_messaging_slot(),
        empty_commands_slot(),
        relay_bus,
    )
    .await
    .expect("start mcp server");

    let data_dir = std::env::temp_dir().join(format!("engram-mcp-rotate-{}", AgentId::new_v4()));
    let channel = DaemonControlChannel::new(
        registry.clone(),
        handle.url.clone(),
        data_dir.clone(),
        None,
        Arc::new(NoopPrimingProvider),
    );

    let id = AgentId::new_v4();
    let ep0 = channel
        .provision(id, 0, true)
        .expect("provision ok")
        .expect("epoch0 endpoint");
    let old_token = ep0.token.clone();
    let old_path = mcp_config::config_path(&data_dir, id, 0);
    assert!(old_path.exists(), "epoch0 config 파일 생성");
    assert!(registry.validate(&old_token).is_some(), "epoch0 토큰 유효");

    let ep1 = channel
        .provision(id, 1, true)
        .expect("provision ok")
        .expect("epoch1 endpoint");
    let new_path = mcp_config::config_path(&data_dir, id, 1);
    assert!(new_path.exists(), "epoch1 config 파일 생성");
    assert_eq!(
        post_initialize(&handle.url, Some(&old_token)).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "회전된 구 epoch 토큰 → 401"
    );
    assert!(
        registry.validate(&ep1.token).is_some(),
        "새 epoch1 토큰 유효"
    );

    channel.revoke(id, 0);
    assert!(!old_path.exists(), "revoke(epoch0) 후 구 config 파일 삭제");
    assert!(new_path.exists(), "새 epoch1 config 파일은 남아 있어야");

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── round-2 F1: orphaned-session attach ─────────────────────────────────────────────
#[tokio::test]
async fn orphaned_session_attach_is_rejected() {
    let registry = Arc::new(ControlRegistry::new());
    let id_a = AgentId::new_v4();
    let id_b = AgentId::new_v4();
    registry.issue(id_a, 0, "token-a".to_string(), true);
    registry.issue(id_b, 0, "token-b".to_string(), true);

    // ★수거기를 들고 있어야 한다★ — 떨어뜨리면 그 자리에서 자리 표가 닫혀 그 뒤 왕복이 전부 반려된다.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server(
        registry.clone(),
        empty_slot(),
        empty_messaging_slot(),
        empty_commands_slot(),
        relay_bus,
    )
    .await
    .expect("start mcp server");
    let url = &handle.url;

    let (status, sid) = open_session(url, "token-a").await;
    assert_eq!(status, reqwest::StatusCode::OK, "token A initialize 200");
    let sid = sid.expect("initialize 가 Mcp-Session-Id 를 돌려줘야");
    assert_eq!(registry.bound_session_count(), 1, "A 세션 바인딩됨");

    registry.revoke(id_a, 0);
    assert_eq!(
        registry.bound_session_count(),
        0,
        "revoke 로 A 바인딩 prune"
    );

    assert_eq!(
        post_tools_list(url, "token-b", &sid).await,
        reqwest::StatusCode::NOT_FOUND,
        "고아 세션 S 에 B 토큰 attach → 404(F1 orphaned-session 거부)"
    );

    handle.shutdown().await;
}

// ── round-2 F1: 바인딩된 적 없는 세션 id ─────────────────────────────────────────────
#[tokio::test]
async fn unknown_session_id_is_rejected_not_forwarded() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "valid".to_string(), true);
    // ★수거기를 들고 있어야 한다★ — 떨어뜨리면 그 자리에서 자리 표가 닫혀 그 뒤 왕복이 전부 반려된다.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server(
        registry,
        empty_slot(),
        empty_messaging_slot(),
        empty_commands_slot(),
        relay_bus,
    )
    .await
    .expect("start mcp server");
    let url = &handle.url;

    assert_eq!(
        post_tools_list(url, "valid", "never-bound-session-id").await,
        reqwest::StatusCode::NOT_FOUND,
        "미지 세션 id → 404(inner 미도달, F1)"
    );

    handle.shutdown().await;
}

// ── Codex LOW: malformed(비-UTF-8) Mcp-Session-Id 헤더 ───────────────────────────────
#[tokio::test]
async fn malformed_session_id_header_is_rejected_with_400() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "malformtok".to_string(), true);
    // ★수거기를 들고 있어야 한다★ — 떨어뜨리면 그 자리에서 자리 표가 닫혀 그 뒤 왕복이 전부 반려된다.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server(
        registry,
        empty_slot(),
        empty_messaging_slot(),
        empty_commands_slot(),
        relay_bus,
    )
    .await
    .expect("start mcp server");
    let url = &handle.url;

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

// ── security lens: 세션 operation(GET/DELETE) 무-세션id ──────────────────────────────
#[tokio::test]
async fn session_ops_without_session_id_are_rejected_with_400() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "optok".to_string(), true);
    // ★수거기를 들고 있어야 한다★ — 떨어뜨리면 그 자리에서 자리 표가 닫혀 그 뒤 왕복이 전부 반려된다.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server(
        registry,
        empty_slot(),
        empty_messaging_slot(),
        empty_commands_slot(),
        relay_bus,
    )
    .await
    .expect("start mcp server");
    let url = &handle.url;

    assert_eq!(
        get_stream(url, Some("optok")).await,
        reqwest::StatusCode::BAD_REQUEST,
        "유효 토큰 GET(무-세션id) → 400(session op 는 바인딩으로 resolve 돼야, inner 미도달)"
    );
    assert_eq!(
        delete_session(url, Some("optok"), None).await,
        reqwest::StatusCode::BAD_REQUEST,
        "유효 토큰 DELETE(무-세션id) → 400(session op 는 바인딩으로 resolve 돼야)"
    );

    handle.shutdown().await;
}

// ── REGRESSION: POST initialize(무-세션id) ───────────────────────────────────────────
#[tokio::test]
async fn post_initialize_without_session_id_still_reaches_inner() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "inittok".to_string(), true);
    // ★수거기를 들고 있어야 한다★ — 떨어뜨리면 그 자리에서 자리 표가 닫혀 그 뒤 왕복이 전부 반려된다.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server(
        registry.clone(),
        empty_slot(),
        empty_messaging_slot(),
        empty_commands_slot(),
        relay_bus,
    )
    .await
    .expect("start mcp server");
    let url = &handle.url;

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

// ── round-2 F4: body 상한 + 정상 요청 무영향 ─────────────────────────────────────────
#[tokio::test]
async fn oversize_body_is_rejected_with_413() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "sizetok".to_string(), true);
    // ★수거기를 들고 있어야 한다★ — 떨어뜨리면 그 자리에서 자리 표가 닫혀 그 뒤 왕복이 전부 반려된다.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server(
        registry.clone(),
        empty_slot(),
        empty_messaging_slot(),
        empty_commands_slot(),
        relay_bus,
    )
    .await
    .expect("start mcp server");
    let url = &handle.url;

    // body-limit 가 auth·세션 검사와 독립임을 보이려 **유효** 세션에 건다.
    let (status, sid) = open_session(url, "sizetok").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let sid = sid.expect("session id");

    assert_eq!(
        post_tools_list_with_padding(url, "sizetok", &sid, 1024)
            .await
            .expect("정상 요청은 연결 성공"),
        reqwest::StatusCode::OK,
        "1KB 요청은 상한 이하 → 정상 처리(무영향)"
    );

    match post_tools_list_with_padding(url, "sizetok", &sid, 2 * 1024 * 1024).await {
        Ok(status) => assert_eq!(
            status,
            reqwest::StatusCode::PAYLOAD_TOO_LARGE,
            ">1MB 요청 → 413(F4 — 상한이 nested rmcp 까지 도달). 200 이면 상한 미적용 = 실패"
        ),
        Err(e) => assert!(
            e.is_request() || e.is_connect(),
            "상한 초과 거부는 connection-abort 로도 나타날 수 있음(Windows). 예상 밖 에러: {e:?}"
        ),
    }

    handle.shutdown().await;
}
