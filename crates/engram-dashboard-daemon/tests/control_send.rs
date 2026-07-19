//! ADR-0086 스텝 2 통합 테스트 — 듀얼 입구 A→B 메시지 전송(send_message MCP 툴 + /control/send HTTP 라우트).
//!
//! 실 DaemonControlChannel + AgentManager + MCP 서버를 배선하고 검증한다:
//!   - `/control/send`(CLI 입구): 무/오 토큰 → 401 · 유효 토큰 + 없는 수신자 → RECIPIENT_NOT_FOUND ·
//!     그룹(@) → GROUPS_NOT_SUPPORTED · 대용량 body → BODY_TOO_LARGE.
//!   - MCP `send_message` 툴: happy path(산 json 에이전트에 배달 + relay 가 래핑된 라인을 stdin 에 씀) +
//!     교정 에러(없는 수신자).
//!   - relay 관측: 산 json(stream-json) 에이전트에 보내면 write_input 이 동기 발행하는 입력-시점 유저
//!     에코(Structured{kind:"user"})에 래핑된 라인(`[message from … id:…] …`)이 담긴다(실 claude 스폰).
//!   - 발신자 생존은 배달 게이트가 아니다(사용자 결정 2026-07-19): 폐기 발신자여도 메시지는 **배달된다**
//!     (작성 시점 인증으로 유효 — is_identity_live 는 기록용 관측만). handle_send 직접 호출로 격리해
//!     배달 성공(enqueued ACK + 래핑 라인 주입)을 관측한다(claude-gated).
//!
//! ★relay 관측 방식(honest note)★: 별도 세션-레벨 테스트 더블이 없어(코어에 세션 주입 seam 없음), 산
//!   json 에이전트를 실제 스폰하고 write_input 이 send_input 성공 직후 **동기**로 내는 입력 에코를
//!   OutputSink 로 잡는다. 이 에코는 claude 왕복 이전에 발행되므로 claude 응답 지연·인증과 무관하게
//!   결정적이다(스폰 자체는 실 바이너리 필요 — 없으면 그 테스트는 무의미하나, 이 머신엔 claude 2.1.170 존재).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::preset::PresetRegistry;
use engram_dashboard_core::agent::profile::{
    AgentCommand, AgentProfile, ClaudeOutputFormat, ProfileRegistry, SpawnMode,
};
use engram_dashboard_core::agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, OutputEvent, OutputFrame, OutputPayload,
    OutputSink, SinkError, SinkId, StatusSink,
};
use engram_dashboard_core::persistence::{FilePresetStore, FileProfileStore};

use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server, ManagerSlot, McpServerHandle,
};
use engram_dashboard_daemon::control::registry::ControlRegistry;
use engram_dashboard_daemon::control::DaemonControlChannel;

struct NoopSink;
impl StatusSink for NoopSink {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

/// core 로 emit 된 구조화 이벤트의 json 을 수집하는 OutputSink(relay 관측용).
struct EventCapture {
    id: SinkId,
    seen: Arc<Mutex<Vec<String>>>,
}
impl OutputSink for EventCapture {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        if let OutputPayload::Event(OutputEvent::Structured { json, .. }) = frame.payload {
            self.seen.lock().unwrap().push(json.clone());
        }
        Ok(())
    }
    fn sink_id(&self) -> SinkId {
        self.id
    }
}

fn wait_until<F: Fn() -> bool>(timeout: Duration, cond: F) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    cond()
}

/// ★loud skip(F7a)★: claude 스폰이 안 되는 머신에서 relay-관측 테스트를 **구조적으로 눈에 띄게** 건너뛴다.
/// CI 를 깨지 않되(테스트는 Ok 로 끝난다) SKIPPED 라벨을 stdout+stderr 둘 다에 남겨 "조용히 통과"로
/// 오인되지 않게 한다. 이 경로에 도달했다 = claude 부재/인증 실패로 relay 단언을 못 했다는 뜻.
///
/// ★CI 강제 knob(M2)★: cargo 는 test 의 stdout 을 기본 캡처해 삼키므로, loud print 를 해도 통과 요약엔
///   "ok" 만 남아 skip 이 조용히 새어 나간다. env `ENGRAM_TEST_REQUIRE_CLAUDE=1` 이 설정돼 있으면(=
///   claude 가 반드시 있어야 하는 CI 레인) skip 을 **panic 으로 승격**해 테스트를 실제로 실패시킨다 —
///   "silent skip 금지" 강제. 미설정(로컬 개발 기본)이면 기존대로 loud print 후 조용히 Ok 로 넘어간다.
fn skip_no_claude(test: &str) {
    let line = format!(
        "SKIPPED [{test}]: claude(stream-json) 에이전트 스폰 실패 — relay 실측 불가(claude 부재/인증). \
         registry/ingress 단위 테스트가 로직을 커버하나 end-to-end relay 는 이 머신에서 미검증."
    );
    // stdout(`cargo test -- --nocapture` 에서 보임) + stderr(항상 보임) 둘 다.
    println!("{line}");
    eprintln!("{line}");
    // CI knob: skip 금지 레인이면 여기서 panic → 테스트 실패로 skip 이 요약에 드러난다.
    if std::env::var("ENGRAM_TEST_REQUIRE_CLAUDE").as_deref() == Ok("1") {
        panic!(
            "ENGRAM_TEST_REQUIRE_CLAUDE=1 인데 [{test}] 가 claude 부재로 skip 됨 — \
             이 레인은 silent skip 을 금지한다(claude(stream-json) 스폰이 반드시 성공해야 함)."
        );
    }
}

/// 실 DaemonControlChannel + MCP 서버 + AgentManager(슬롯 주입 완료) 배선. run() 조립 순서 미러:
/// registry → slot → start_mcp_server(registry, slot) → DaemonControlChannel(url) → manager → slot.set.
async fn wire(
    tag: &str,
) -> (
    Arc<AgentManager>,
    Arc<ControlRegistry>,
    String,
    std::path::PathBuf,
    McpServerHandle,
) {
    let registry = Arc::new(ControlRegistry::new());
    let slot = Arc::new(ManagerSlot::new());
    let handle = start_mcp_server(registry.clone(), slot.clone())
        .await
        .expect("start mcp server");
    let url = handle.url.clone();
    let data_dir = std::env::temp_dir().join(format!("engram-send-{tag}-{}", AgentId::new_v4()));

    let control: Arc<dyn ControlChannel> = Arc::new(DaemonControlChannel::new(
        registry.clone(),
        url.clone(),
        data_dir.clone(),
        None, // send_exe: relay 테스트는 CLI 경로 불요(직접 HTTP/MCP 호출).
    ));

    let sink: Arc<dyn StatusSink> = Arc::new(NoopSink);
    let profiles = Arc::new(ProfileRegistry::new(Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-send-prof-{tag}-{}", AgentId::new_v4())),
    ))));
    let presets = Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(
        std::env::temp_dir().join(format!("engram-send-preset-{tag}-{}", AgentId::new_v4())),
    ))));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            sessions_dir: None,
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = Arc::new(AgentManager::new_with_control(
        sink, profiles, presets, tracker, control,
    ));
    slot.set(manager.clone());

    // base URL(/mcp 벗김) — /control/send 요청에 쓴다.
    let base = url.strip_suffix("/mcp").unwrap_or(&url).to_string();
    (manager, registry, base, data_dir, handle)
}

/// /control/send 로 POST → (상태코드, body 텍스트). bearer None 이면 헤더 미첨부.
async fn post_send(
    base: &str,
    bearer: Option<&str>,
    to: &str,
    body: &str,
) -> (reqwest::StatusCode, String) {
    let client = reqwest::Client::new();
    let mut req = client
        .post(format!("{base}/control/send"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "to": to, "body": body }));
    if let Some(b) = bearer {
        req = req.header("Authorization", format!("Bearer {b}"));
    }
    let resp = req.send().await.expect("http request");
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

/// 산 json(stream-json) claude 에이전트를 스폰하고 (info, control 토큰)을 돌려준다. provision 이 발급한
/// 토큰을 registry 에서 뽑아(그 에이전트 신원의 Bearer) 발신자로 쓴다.
fn spawn_json_agent(
    manager: &Arc<AgentManager>,
    registry: &Arc<ControlRegistry>,
    name: &str,
) -> Option<(AgentInfo, String)> {
    let profile = AgentProfile::new(
        name.to_string(),
        AgentCommand::Claude {
            extra_args: vec![],
            output_format: ClaudeOutputFormat::StreamJson,
        },
        std::path::PathBuf::from("."),
        vec![],
        false,
    );
    let info = manager.spawn_agent(&profile, SpawnMode::Fresh).ok()?;
    if !wait_until(Duration::from_secs(5), || {
        manager.list_agents().iter().any(|a| a.id == info.id)
    }) {
        return None;
    }
    // provision 이 이 (id, epoch) 에 발급한 토큰을 찾는다(registry 내부 조회 API 가 없어 재검증으로 확인
    //   불가하므로, 발신자용 토큰은 별도로 issue 해 심는다 — 발신자 신원만 맞으면 relay 는 동일).
    let token = format!("sender-tok-{}", info.id);
    registry.issue(info.id, info.epoch, token.clone());
    Some((info, token))
}

// ── /control/send: 인증 ────────────────────────────────────────────────────────────
#[tokio::test]
async fn control_send_missing_token_is_401() {
    let (_m, _r, base, data_dir, handle) = wire("auth-missing").await;
    let (status, _body) = post_send(&base, None, "bob", "hi").await;
    assert_eq!(
        status,
        reqwest::StatusCode::UNAUTHORIZED,
        "무토큰 /control/send → 401"
    );
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

#[tokio::test]
async fn control_send_wrong_token_is_401() {
    let (_m, _r, base, data_dir, handle) = wire("auth-wrong").await;
    let (status, _body) = post_send(&base, Some("bogus-token"), "bob", "hi").await;
    assert_eq!(
        status,
        reqwest::StatusCode::UNAUTHORIZED,
        "모르는 토큰 /control/send → 401"
    );
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── /control/send: 교정 에러(수신자 없음·그룹·대용량) — 유효 토큰 필요 ────────────────────────
#[tokio::test]
async fn control_send_corrective_errors() {
    let (_m, registry, base, data_dir, handle) = wire("corrective").await;
    // 유효 토큰(발신자 신원) — 아무 (id, epoch) 로 issue. 수신자 없음이라 relay 엔 안 간다.
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "valid-sender".to_string());

    // 없는 수신자 → RECIPIENT_NOT_FOUND(200 + 에러 JSON).
    let (status, body) = post_send(&base, Some("valid-sender"), "nobody", "hi").await;
    assert_eq!(status, reqwest::StatusCode::OK, "교정 에러도 200 + JSON");
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["status"], "error");
    assert_eq!(v["code"], "RECIPIENT_NOT_FOUND", "없는 수신자: {body}");

    // 그룹 주소(@) → GROUPS_NOT_SUPPORTED.
    let (_s, body) = post_send(&base, Some("valid-sender"), "@team", "hi").await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["code"], "GROUPS_NOT_SUPPORTED", "@ 주소: {body}");

    // 대용량 body(>64KiB) → BODY_TOO_LARGE.
    let big = "x".repeat(64 * 1024 + 1);
    let (_s, body) = post_send(&base, Some("valid-sender"), "nobody", &big).await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["code"], "BODY_TOO_LARGE", "대용량 body: {body}");

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── /control/send: shell 수신자는 RECIPIENT_NOT_REACHABLE(제어 채널 TUI 제외) ──────────────────
#[tokio::test]
async fn control_send_shell_recipient_not_reachable() {
    let (manager, registry, base, data_dir, handle) = wire("not-reachable").await;
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "valid-sender".to_string());

    // shell 에이전트(structured=false = 도달 불가) 스폰.
    let profile = AgentProfile::new(
        "sheller".to_string(),
        AgentCommand::Shell {
            program: engram_dashboard_core::agent::manager::default_shell().to_string(),
            args: vec![],
        },
        std::path::PathBuf::from("."),
        vec![],
        false,
    );
    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("shell spawn");
    assert!(wait_until(Duration::from_secs(3), || manager
        .list_agents()
        .iter()
        .any(|a| a.id == info.id)));

    let (_s, body) = post_send(&base, Some("valid-sender"), "sheller", "hi").await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["code"], "RECIPIENT_NOT_REACHABLE",
        "shell(TUI/비-structured) 수신자는 도달 불가: {body}"
    );

    manager.kill_agent(info.id).ok();
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── relay happy path: json 에이전트에 보내면 래핑된 라인이 stdin 입력 에코로 관측된다 ────────────────
// 실 claude(stream-json) 스폰 + write_input 동기 입력 에코 관측(claude 왕복 이전이라 결정적).
#[tokio::test]
async fn control_send_relays_wrapped_line_to_json_agent() {
    let (manager, registry, base, data_dir, handle) = wire("relay").await;

    // 산 json 에이전트 B 스폰. 스폰 실패(claude 부재 등)면 이 테스트는 무의미 — 건너뛴다(loud skip).
    let Some((b_info, _b_tok)) = spawn_json_agent(&manager, &registry, "bee") else {
        skip_no_claude("control_send_relays_wrapped_line_to_json_agent");
        let _ = std::fs::remove_dir_all(&data_dir);
        handle.shutdown().await;
        return;
    };

    // B 출력에 관측 sink 부착 — write_input 이 내는 입력-시점 유저 에코(Structured{kind:"user"})를 잡는다.
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = Arc::new(EventCapture {
        id: SinkId::new_v4(),
        seen: seen.clone(),
    });
    manager.subscribe(b_info.id, sink).expect("subscribe B");

    // 발신자 토큰(유효) — /control/send 는 이 토큰의 신원을 from 으로 쓴다.
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "relay-sender".to_string());

    let (status, body) = post_send(&base, Some("relay-sender"), "bee", "ping-body-XYZ").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).expect("json ACK");
    assert_eq!(v["status"], "enqueued", "배달 성공 ACK: {body}");
    assert_eq!(v["to"], "bee", "해석된 수신자 이름 동봉");
    assert!(v["id"].is_string(), "msg-id 동봉");

    // 래핑된 라인이 B 의 입력 에코로 관측돼야 한다(`[message from … id:…] ping-body-XYZ`).
    let observed = wait_until(Duration::from_secs(3), || {
        seen.lock()
            .unwrap()
            .iter()
            .any(|j| j.contains("ping-body-XYZ") && j.contains("message from"))
    });
    assert!(
        observed,
        "relay 가 래핑된 라인을 B stdin 에 주입(입력 에코 관측): {:?}",
        seen.lock().unwrap()
    );

    manager.kill_agent(b_info.id).ok();
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// (구 `control_send_revalidation_runs_after_reachability_f3` 제거 — 사용자 결정 2026-07-19로 발신자
//  생존이 게이트가 아니게 되면서 "재검증이 도달성 뒤" 라는 순서 고정 의미가 사라졌다. 남는 단언(유효
//  발신자 + shell 수신자 → RECIPIENT_NOT_REACHABLE)은 위 `control_send_shell_recipient_not_reachable`
//  과 완전히 동일한 경로라 중복 → 그 테스트로 병합/흡수한다.)

// ── 폐기된 발신자여도 메시지는 배달된다(생존은 게이트 아님·기록용 관측만) — handle_send 직접 호출로 격리 ──
// ★사용자 결정 2026-07-19★: 메시지 유효성은 **작성 시점 인증**(입구 auth)으로 이미 성립한다. 발신자가
//   그 뒤 죽거나 회전돼도(토큰 revoke) 메시지는 무효가 되지 않는다 — "결과 보내고 종료"(유언 패턴)는
//   멀티에이전트 핵심 패턴이고 미래 메일박스 커밋 시맨틱과도 정합한다. is_identity_live 는 배달을 막지
//   않고 forensic 로그만 남긴다. 이 테스트는 폐기 발신자여도 **배달됨**(enqueued ACK + 래핑 라인 stdin
//   주입)을 관측한다(구 SENDER_REVOKED 거부 단언의 반전).
// ★왜 HTTP 가 아니라 handle_send 직접인가★: HTTP 경로는 미들웨어(bearer_auth)가 토큰을 먼저 validate 하므로
//   revoke 하면 401 로 먼저 막혀 commit-point 에 못 닿는다(revoke 와 send 사이 mid-flight 주입은 단일
//   동기 요청에서 결정적으로 못 만든다). 그래서 공통 핸들러를 직접 부른다: 발신자 신원을 산 상태로
//   만들었다가 **relay 직전에 revoke** 한 뒤 handle_send 호출 → 배달됨 관측. 도달 가능 수신자가 필요하므로
//   json claude 스폰에 의존(loud skip).
#[tokio::test]
async fn control_send_revoked_sender_still_delivers_observation() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, Entrance};
    use engram_dashboard_daemon::control::registry::BoundIdentity;

    let (manager, registry, _base, data_dir, handle) = wire("revoked-delivers").await;

    // 도달 가능한 수신자 B(json claude). 없으면 relay 를 못 관측해 스킵.
    let Some((b_info, _b_tok)) = spawn_json_agent(&manager, &registry, "target-b") else {
        skip_no_claude("control_send_revoked_sender_still_delivers_observation");
        let _ = std::fs::remove_dir_all(&data_dir);
        handle.shutdown().await;
        return;
    };

    // B 출력 관측 sink — 폐기 발신자여도 래핑 라인이 **주입되어야**(배달됨) 함을 확인.
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = Arc::new(EventCapture {
        id: SinkId::new_v4(),
        seen: seen.clone(),
    });
    manager.subscribe(b_info.id, sink).expect("subscribe B");

    // 발신자 신원 발급 → 산 상태. 그 다음 relay 직전 revoke → is_identity_live(from) == false(관측용만).
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "sender-tok".to_string());
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };
    registry.revoke(sender, 0); // ★relay 직전 발신자 폐기 모사 — 그래도 배달돼야★.

    let cmd = ControlCommand {
        from,
        to: "target-b".to_string(),
        body: "revoked-but-DELIVERED".to_string(),
    };
    let result = handle_send(&manager, &registry, Entrance::Cli, cmd);
    let v = result.to_json();
    assert_eq!(
        v["status"], "enqueued",
        "폐기 발신자여도 배달됨(생존은 게이트 아님, 사용자 결정): {v}"
    );
    assert_eq!(v["to"], "target-b", "해석된 수신자 이름 동봉");
    assert!(v["id"].is_string(), "msg-id 동봉");

    // 배달됨 — 래핑 라인이 B 입력 에코로 관측돼야 한다(폐기 발신자여도 relay 진행).
    let delivered = wait_until(Duration::from_secs(3), || {
        seen.lock()
            .unwrap()
            .iter()
            .any(|j| j.contains("revoked-but-DELIVERED") && j.contains("message from"))
    });
    assert!(
        delivered,
        "폐기 발신자여도 래핑 라인이 B stdin 에 주입돼야(배달됨): {:?}",
        seen.lock().unwrap()
    );

    manager.kill_agent(b_info.id).ok();
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── MCP send_message 툴: happy path + 교정 에러(rmcp 클라이언트) ─────────────────────────────
#[tokio::test]
async fn mcp_send_message_tool_happy_and_error() {
    use rmcp::model::CallToolRequestParams;
    use rmcp::transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    };
    use rmcp::ServiceExt;

    let (manager, registry, _base, data_dir, handle) = wire("mcp-tool").await;

    // 수신자 B(산 json 에이전트) 스폰. 실패 시 스킵.
    let Some((b_info, _b_tok)) = spawn_json_agent(&manager, &registry, "recv") else {
        skip_no_claude("mcp_send_message_tool_happy_and_error");
        let _ = std::fs::remove_dir_all(&data_dir);
        handle.shutdown().await;
        return;
    };

    // 발신자 A 토큰(유효) — MCP 클라이언트가 이 토큰으로 handshake(신원=A).
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "mcp-sender-tok".to_string());

    let config = StreamableHttpClientTransportConfig::with_uri(handle.url.clone())
        .auth_header("mcp-sender-tok");
    let transport = StreamableHttpClientTransport::from_config(config);
    let client = ().serve(transport).await.expect("MCP handshake");

    // tools/list 에 send_message 존재.
    let tools = client.list_all_tools().await.expect("list tools");
    assert!(
        tools.iter().any(|t| t.name == "send_message"),
        "tools 에 send_message: {:?}",
        tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>()
    );

    // happy path — B 로 전송 → enqueued ACK(text content = JSON).
    let mut params = CallToolRequestParams::default();
    params.name = "send_message".into();
    params.arguments = Some(
        serde_json::json!({ "to": "recv", "body": "mcp-hello" })
            .as_object()
            .unwrap()
            .clone(),
    );
    let result = client.call_tool(params).await.expect("call send_message");
    let text = result
        .content
        .iter()
        .find_map(|c| c.as_text().map(|t| t.text.clone()))
        .expect("text content");
    let v: serde_json::Value = serde_json::from_str(&text).expect("ACK json");
    assert_eq!(v["status"], "enqueued", "MCP send happy path: {text}");
    assert_eq!(v["to"], "recv");

    // 교정 에러 — 없는 수신자.
    let mut params = CallToolRequestParams::default();
    params.name = "send_message".into();
    params.arguments = Some(
        serde_json::json!({ "to": "ghost", "body": "x" })
            .as_object()
            .unwrap()
            .clone(),
    );
    let result = client
        .call_tool(params)
        .await
        .expect("call send_message err");
    let text = result
        .content
        .iter()
        .find_map(|c| c.as_text().map(|t| t.text.clone()))
        .expect("text content");
    let v: serde_json::Value = serde_json::from_str(&text).expect("err json");
    assert_eq!(v["status"], "error");
    assert_eq!(v["code"], "RECIPIENT_NOT_FOUND", "MCP 없는 수신자: {text}");

    let _ = client.cancel().await;
    manager.kill_agent(b_info.id).ok();
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}
