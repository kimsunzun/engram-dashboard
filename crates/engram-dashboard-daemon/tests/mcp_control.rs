//! ADR-0086 step 1 integration test — the daemon MCP control channel entrance (token auth + session binding + engram_ping).
//!
//! Without a real claude (in-process), brings up the daemon MCP endpoint and verifies with an HTTP/MCP client.

use std::sync::Arc;

use engram_dashboard_agent::types::{AgentId, ControlChannelNeeds};

/// The three axes an MCP-capable spawn passes — this file measures only the channel physics (token issue); where production reads those values from is measured by `mail_gate.rs`.
///
/// ★Claude-shaped★ — even the file axis is on. The row 「uses MCP but does not read our file」 (codex) is measured by that test in the daemon `control/mod.rs` (ADR-0209).
fn mcp_needs() -> ControlChannelNeeds {
    ControlChannelNeeds {
        accepts_mcp_config: true,
        writes_mcp_config_file: true,
        uses_mail: true,
    }
}
use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server_without_help, CommandTableSlot, ManagerSlot, MessagingSlot,
};
use engram_dashboard_daemon::control::registry::ControlRegistry;

/// These tests do not call send/flush, so the slot is left empty.
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

/// ★Why the return is a Result (Windows-specific)★: if Content-Length exceeds the cap, RequestBodyLimitLayer responds 413 at once without reading the body and closes the connection. The client (reqwest) is still uploading, so the OS resets the connection (WinError 10053) and reqwest can fail with ConnectionAborted before reading the 413. Both mean "cap exceeded → refused", so the Result is returned and the caller accepts Ok(413) | Err(connection-abort).
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

    // ★Hold the collector★ — dropped, the seat ticket closes at once and every later round trip is bounced.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server_without_help(
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
        "the 401 path must create no session"
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

    // ★Hold the collector★ — dropped, the seat ticket closes at once and every later round trip is bounced.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server_without_help(
        registry.clone(),
        empty_slot(),
        empty_messaging_slot(),
        empty_commands_slot(),
        relay_bus,
    )
    .await
    .expect("start mcp server");

    // auth_header is the raw token — the rmcp client prepends "Bearer " via reqwest .bearer_auth.
    let config =
        StreamableHttpClientTransportConfig::with_uri(handle.url.clone()).auth_header("good-token");
    let transport = StreamableHttpClientTransport::from_config(config);
    let client = ().serve(transport).await.expect("MCP handshake with valid token");

    assert_eq!(
        registry.bound_session_count(),
        1,
        "after a valid-token initialize, one session is bound (acceptance)"
    );

    let tools = client.list_all_tools().await.expect("list tools");
    assert!(
        tools.iter().any(|t| t.name == "engram_ping"),
        "engram_ping is present in system:init tools: {:?}",
        tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>()
    );

    // CallToolRequestParams is #[non_exhaustive] (another crate) → no literal, set fields after Default.
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
        "engram_ping returns the bound identity: {text}"
    );

    let _ = client.cancel().await;
    handle.shutdown().await;
}

// ── FIX 9: GET/DELETE without token ────────────────────────────────────────────────────────
#[tokio::test]
async fn get_and_delete_without_token_are_rejected() {
    let registry = Arc::new(ControlRegistry::new());
    // ★Hold the collector★ — dropped, the seat ticket closes at once and every later round trip is bounced.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server_without_help(
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

// ── FIX 9/7: cross-token session takeover ──────────────────────────────────────────────────
#[tokio::test]
async fn cross_token_session_takeover_is_rejected() {
    let registry = Arc::new(ControlRegistry::new());
    let id_a = AgentId::new_v4();
    let id_b = AgentId::new_v4();
    registry.issue(id_a, 0, "token-a".to_string(), true);
    registry.issue(id_b, 0, "token-b".to_string(), true);

    // ★Hold the collector★ — dropped, the seat ticket closes at once and every later round trip is bounced.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server_without_help(
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
    let sid = sid.expect("initialize must return Mcp-Session-Id");

    assert_eq!(
        post_tools_list(url, "token-b", &sid).await,
        reqwest::StatusCode::FORBIDDEN,
        "accessing session S with another token (B) → 403 (cross-token takeover refused)"
    );

    assert_eq!(
        post_tools_list(url, "token-a", &sid).await,
        reqwest::StatusCode::OK,
        "original token A accesses session S normally (200)"
    );

    handle.shutdown().await;
}

// ── FIX 9: mid-session revoke ─────────────────────────────────────────────────────────
#[tokio::test]
async fn revoked_mid_session_request_is_rejected() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "live-token".to_string(), true);

    // ★Hold the collector★ — dropped, the seat ticket closes at once and every later round trip is bounced.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server_without_help(
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
        "follow-up request before revoke is 200"
    );

    registry.revoke(id, 0);
    assert_eq!(
        post_tools_list(url, "live-token", &sid).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "follow-up request with the same token after revoke → 401 (validate None)"
    );

    handle.shutdown().await;
}

// ── FIX 9: epoch rotation ──
// The old-token evict of registry rotation (issue) is covered by the registry unit — here the config file lifetime is checked too.
#[tokio::test]
async fn epoch_rotation_revokes_old_token_and_config_file() {
    use engram_dashboard_agent::types::ControlChannel;
    use engram_dashboard_daemon::control::mcp_config;
    use engram_dashboard_daemon::control::priming::NoopPrimingProvider;
    use engram_dashboard_daemon::control::DaemonControlChannel;

    let registry = Arc::new(ControlRegistry::new());
    // ★Hold the collector★ — dropped, the seat ticket closes at once and every later round trip is bounced.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server_without_help(
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
        engram_dashboard_daemon::control::mcp_config::McpDir::new(data_dir.join("mcp-config")),
        None,
        Arc::new(NoopPrimingProvider),
    );

    let id = AgentId::new_v4();
    let ep0 = channel
        .provision(id, 0, mcp_needs())
        .expect("provision ok")
        .expect("epoch0 endpoint");
    let old_token = ep0.token.clone();
    let old_path =
        mcp_config::config_path(&mcp_config::McpDir::new(data_dir.join("mcp-config")), id, 0);
    assert!(old_path.exists(), "epoch0 config file created");
    assert!(
        registry.validate(&old_token).is_some(),
        "epoch0 token valid"
    );

    let ep1 = channel
        .provision(id, 1, mcp_needs())
        .expect("provision ok")
        .expect("epoch1 endpoint");
    let new_path =
        mcp_config::config_path(&mcp_config::McpDir::new(data_dir.join("mcp-config")), id, 1);
    assert!(new_path.exists(), "epoch1 config file created");
    assert_eq!(
        post_initialize(&handle.url, Some(&old_token)).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "rotated old epoch token → 401"
    );
    assert!(
        registry.validate(&ep1.token).is_some(),
        "new epoch1 token valid"
    );

    channel.revoke(id, 0);
    assert!(
        !old_path.exists(),
        "old config file deleted after revoke(epoch0)"
    );
    assert!(new_path.exists(), "the new epoch1 config file must remain");

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

    // ★Hold the collector★ — dropped, the seat ticket closes at once and every later round trip is bounced.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server_without_help(
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
    let sid = sid.expect("initialize must return Mcp-Session-Id");
    assert_eq!(registry.bound_session_count(), 1, "session A bound");

    registry.revoke(id_a, 0);
    assert_eq!(
        registry.bound_session_count(),
        0,
        "revoke prunes the A binding"
    );

    assert_eq!(
        post_tools_list(url, "token-b", &sid).await,
        reqwest::StatusCode::NOT_FOUND,
        "attaching token B to orphan session S → 404 (F1 orphaned-session refused)"
    );

    handle.shutdown().await;
}

// ── round-2 F1: a session id that was never bound ─────────────────────────────────────────────
#[tokio::test]
async fn unknown_session_id_is_rejected_not_forwarded() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "valid".to_string(), true);
    // ★Hold the collector★ — dropped, the seat ticket closes at once and every later round trip is bounced.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server_without_help(
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
        "unknown session id → 404 (inner not reached, F1)"
    );

    handle.shutdown().await;
}

// ── Codex LOW: malformed (non-UTF-8) Mcp-Session-Id header ───────────────────────────────
#[tokio::test]
async fn malformed_session_id_header_is_rejected_with_400() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "malformtok".to_string(), true);
    // ★Hold the collector★ — dropped, the seat ticket closes at once and every later round trip is bounced.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server_without_help(
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
        .expect("bytes → HeaderValue (the value itself is valid, only to_str fails)");
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
        "malformed (non-UTF-8) Mcp-Session-Id → 400 (inner not reached, not mistaken for sessionless). 200 means bypass = failure"
    );

    handle.shutdown().await;
}

// ── security lens: session operation (GET/DELETE) without session id ──────────────────────────────
#[tokio::test]
async fn session_ops_without_session_id_are_rejected_with_400() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "optok".to_string(), true);
    // ★Hold the collector★ — dropped, the seat ticket closes at once and every later round trip is bounced.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server_without_help(
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
        "valid-token GET (no session id) → 400 (session ops must resolve through the binding, inner not reached)"
    );
    assert_eq!(
        delete_session(url, Some("optok"), None).await,
        reqwest::StatusCode::BAD_REQUEST,
        "valid-token DELETE (no session id) → 400 (session ops must resolve through the binding)"
    );

    handle.shutdown().await;
}

// ── REGRESSION: POST initialize (without session id) ───────────────────────────────────────────
#[tokio::test]
async fn post_initialize_without_session_id_still_reaches_inner() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "inittok".to_string(), true);
    // ★Hold the collector★ — dropped, the seat ticket closes at once and every later round trip is bounced.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server_without_help(
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
        "POST initialize (no session id) still reaches inner (200). Blocked with 400 means initialize is broken = failure"
    );
    assert!(
        sid.is_some(),
        "initialize must return Mcp-Session-Id (inner rmcp creates the session)"
    );
    assert_eq!(
        registry.bound_session_count(),
        1,
        "one session bound after POST initialize (the POST no-session-id exception path is normal)"
    );

    handle.shutdown().await;
}

// ── round-2 F4: body cap + normal requests unaffected ─────────────────────────────────────────
#[tokio::test]
async fn oversize_body_is_rejected_with_413() {
    let registry = Arc::new(ControlRegistry::new());
    let id = AgentId::new_v4();
    registry.issue(id, 0, "sizetok".to_string(), true);
    // ★Hold the collector★ — dropped, the seat ticket closes at once and every later round trip is bounced.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server_without_help(
        registry.clone(),
        empty_slot(),
        empty_messaging_slot(),
        empty_commands_slot(),
        relay_bus,
    )
    .await
    .expect("start mcp server");
    let url = &handle.url;

    // To show the body-limit is independent of auth and session checks, apply it to a valid session.
    let (status, sid) = open_session(url, "sizetok").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let sid = sid.expect("session id");

    assert_eq!(
        post_tools_list_with_padding(url, "sizetok", &sid, 1024)
            .await
            .expect("a normal request connects"),
        reqwest::StatusCode::OK,
        "a 1KB request is under the cap → handled normally (no effect)"
    );

    match post_tools_list_with_padding(url, "sizetok", &sid, 2 * 1024 * 1024).await {
        Ok(status) => assert_eq!(
            status,
            reqwest::StatusCode::PAYLOAD_TOO_LARGE,
            ">1MB request → 413 (F4 — the cap reaches the nested rmcp). 200 means the cap is not applied = failure"
        ),
        Err(e) => assert!(
            e.is_request() || e.is_connect(),
            "refusal over the cap may also show up as connection-abort (Windows). Unexpected error: {e:?}"
        ),
    }

    handle.shutdown().await;
}
