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

// ── ADR-0088 Stage 0: 배달-경계 관측 레코드 — in-proc 싱크로 회수(detached 로그 스크레이핑 없이) ──
// ★왜 in-proc observer 인가★: 운영 데몬은 detached 로 돌아 로그 스크레이핑이 do-not(ADR-0088 HARD
//   CONSTRAINT). registry 에 DeliveryObserver 를 설치하고 handle_send 를 직접 부르면(공통 핸들러 격리)
//   레코드를 로그 없이 직접 단언할 수 있다.
// ★커버리지 구조(FIX-4)★: 관측 레코드의 core 단언은 위 seam 테스트(`..._via_seam_no_claude`)가
//   claude 없이 **항상** 실행해 green-when-skipped 를 없앤다. 아래 claude-gated 테스트는 그에 더해 산
//   json 수신자로 end-to-end(실 encoder/transport) 경로까지 관측이 성립함을 확인한다(있으면 실행, 없으면
//   loud skip). 두 축이 상보적이다 — seam=바이너리 독립 core, gated=실경로 e2e.
struct DeliveryCapture {
    seen: Arc<Mutex<Vec<engram_dashboard_daemon::control::ingress::DeliveryObservation>>>,
}
impl engram_dashboard_daemon::control::ingress::DeliveryObserver for DeliveryCapture {
    fn observe(&self, obs: engram_dashboard_daemon::control::ingress::DeliveryObservation) {
        self.seen.lock().unwrap().push(obs);
    }
}

// ── ADR-0088(FIX-3/FIX-4): claude 바이너리 없이 배달-경계 관측을 구동하는 세션 seam ──────────────
// ★왜 seam 인가★: 위 e2e 테스트는 산 claude 스폰이 필요해(claude 부재 머신에선 skip) 배달 관측의
//   core 단언이 바이너리 유무에 매인다(FIX-4). 여기 helper 는 `AgentManager::insert_test_session` 으로
//   **structured=true(도달 가능) 캐리어를 흉내 내되 write 성공/실패를 우리가 정하는** 세션을 맵에 직접
//   꽂는다 — claude 없이 handle_send 의 성공/실패 두 갈래를 모두 실측한다. 운영 경로는 이 seam 을 절대
//   부르지 않는다(spawn_session 만 정규 등록점, insert_test_session doc 참조).
mod obs_seam {
    use std::sync::atomic::AtomicU8;
    use std::sync::{Arc, Mutex};

    use engram_dashboard_core::agent::backend::InputEncoder;
    use engram_dashboard_core::agent::manager::AgentManager;
    use engram_dashboard_core::agent::output_core::OutputCore;
    use engram_dashboard_core::agent::session::AgentSession;
    use engram_dashboard_core::agent::transport::AgentTransport;
    use engram_dashboard_core::agent::types::{
        AgentId, AgentStatus, BackendCaps, ControlCaps, InputCaps, InputEvent, ModelCaps,
        OutputCaps, PtyError, SessionCaps, StatusSink, TransportCaps,
    };

    struct NoopStatus;
    impl StatusSink for NoopStatus {
        fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
        fn agent_list_updated(&self, _a: Vec<engram_dashboard_core::agent::types::AgentInfo>) {}
    }

    /// 테스트 transport — structured=true 로 신고(도달 가능)하되 send_input 은 `fail` 플래그에 따라
    /// Ok 또는 WriteFailed(Err). 실제 자식·파이프 없음(pump 미기동). captured 로 성공 write 바이트 확인 가능.
    struct SeamTransport {
        fail: bool,
        captured: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl AgentTransport for SeamTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            if self.fail {
                // ★FIX-3★: relay write 실패를 강제 — handle_send 의 Err 갈래를 탄다.
                return Err(PtyError::WriteFailed("seam: recipient stdin closed".into()));
            }
            let InputEvent::Raw(bytes) = input;
            self.captured.lock().unwrap().push(bytes);
            Ok(())
        }
        fn resize(&self, _c: u16, _r: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            TransportCaps {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                // ★도달성 게이트(handle_send step 4)★: structured=true 라야 reachable 로 통과한다.
                output: OutputCaps {
                    terminal_bytes: false,
                    structured: true,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: false,
                    interrupt: false,
                    cancel: false,
                    graceful_shutdown: false,
                },
            }
        }
    }

    fn backend_caps() -> BackendCaps {
        BackendCaps {
            session: SessionCaps {
                resume: true,
                snapshot: false,
                cwd_env: true,
            },
            model: ModelCaps {
                select: false,
                temperature: false,
                max_tokens: false,
            },
        }
    }

    /// structured 캐리어 세션을 조립해 매니저 맵에 꽂고 그 AgentId 를 돌려준다. `fail=true` 면 write 실패.
    /// captured 로 성공 경로의 write 바이트를 검사할 수 있다(멀티바이트 회귀 등).
    pub fn insert_seam_recipient(
        manager: &Arc<AgentManager>,
        fail: bool,
    ) -> (AgentId, Arc<Mutex<Vec<Vec<u8>>>>) {
        let id = AgentId::new_v4();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let core = Arc::new(OutputCore::new(id, 0, Arc::new(NoopStatus)));
        // ClaudeStreamJson encoder — json 모드 캐리어를 흉내(래핑된 봉투가 stream-json 라인으로 감싸짐).
        //   요청 바이트(WriteOutcome.bytes_requested)는 감싸기 **전** 논리 메시지 = wrap_message 봉투 그대로다.
        let session = Arc::new(AgentSession::new(
            id,
            std::path::PathBuf::from("."),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            backend_caps(),
            InputEncoder::ClaudeStreamJson,
            core,
            Box::new(SeamTransport {
                fail,
                captured: captured.clone(),
            }),
        ));
        manager.insert_test_session(session);
        (id, captured)
    }

    /// 성공 write 로 캡처된 마지막 바이트(래핑된 stream-json 라인 전체)를 돌려준다(디코딩 없이 바이트 검사용).
    pub fn last_written(captured: &Arc<Mutex<Vec<Vec<u8>>>>) -> Vec<u8> {
        captured.lock().unwrap().last().cloned().unwrap_or_default()
    }

    /// insert_test_session 은 profiles 에 이름을 안 넣으므로, agent 이름 = id 앞 8자(agent_info fallback).
    pub fn fallback_name(id: AgentId) -> String {
        id.to_string()[..8].to_string()
    }
}

// ── ADR-0088(FIX-4): 배달 관측 core 단언을 claude 없이 — seam 수신자에 성공 relay ──────────────
// 위 e2e 테스트가 claude 부재 시 skip 되는 것과 달리, 이 테스트는 seam 으로 structured 수신자를 꽂아
//   **항상** 관측 레코드(요청/실제 바이트·msg_id↔msg_uuid 상관·is_delivered)를 단언한다(green-when-skipped 제거).
#[tokio::test]
async fn control_send_delivery_observation_via_seam_no_claude() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, Entrance};
    use engram_dashboard_daemon::control::registry::BoundIdentity;

    let (manager, registry, _base, data_dir, handle) = wire("obs-seam-ok").await;

    let (b_id, captured) = obs_seam::insert_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "seam-ok-sender".to_string());
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    // ★FIX-5: 멀티바이트 본체★ — 요청 바이트가 char 수가 아니라 바이트 수임을 세션→관측 계층까지 관통 검증.
    let body = "안녕-msg-α"; // 한글 2자(6B) + "-msg-"(5B) + α(2B) = 13B.
    let cmd = ControlCommand {
        from,
        to: to_name.clone(),
        body: body.to_string(),
    };
    let result = handle_send(&manager, &registry, Entrance::Cli, cmd);
    let v = result.to_json();
    assert_eq!(v["status"], "enqueued", "seam 성공 배달 ACK: {v}");
    let ack_id = v["id"].as_str().expect("msg-id 동봉").to_string();

    let obs = {
        let g = seen.lock().unwrap();
        assert_eq!(g.len(), 1, "성공 relay 1건 → 관측 레코드 1건: {:?}", *g);
        g[0].clone()
    };

    // 상관 축.
    assert_eq!(obs.msg_id, ack_id, "레코드 msg_id = ACK id(상관 축 1)");
    assert!(
        obs.msg_uuid.is_some(),
        "성공 배달은 msg_uuid 를 담아야(상관 축 2)"
    );

    // ★FIX-5: exact 바이트 회계★ — 요청 = wrap_message 봉투의 정확한 바이트 수. 봉투 문자열을 재구성해
    //   기대치를 정확히 계산한다(발신자 표시이름 = sender id 앞8자 fallback, ack_id 는 봉투에 심긴 msg_id).
    let sender_name = obs_seam::fallback_name(sender);
    let expected_wrapped = format!("[message from {sender_name} id:{ack_id}] {body}");
    let expected_bytes = expected_wrapped.len(); // String::len = UTF-8 바이트 수(char 수 아님).
    assert_eq!(
        obs.bytes_requested, expected_bytes,
        "요청 바이트 = 봉투의 정확한 UTF-8 바이트 수(멀티바이트 관통): got={} expect={} wrapped={:?}",
        obs.bytes_requested, expected_bytes, expected_wrapped
    );
    assert_eq!(
        obs.bytes_written,
        Some(obs.bytes_requested),
        "by-construction 복사(bytes_written = 요청) — short-write 탐지 아님"
    );
    assert!(obs.error.is_none(), "성공 배달은 error None");
    assert!(obs.is_delivered(), "is_delivered() = true");
    assert_eq!(obs.to_id, b_id, "레코드 수신자 AgentId");
    assert_eq!(obs.to_name, to_name, "레코드 수신자 이름(fallback)");
    assert_eq!(obs.from, from, "레코드 발신자 신원(토큰 파생)");

    // ★계층 관통★: 세션이 실제 받은 write 바이트(래핑된 stream-json 라인)에 멀티바이트 본체가 온전히
    //   담겼는지 — 라인은 봉투 텍스트를 감싼 것이라 그 안에 봉투 문자열이 부분열로 들어있다.
    let written = obs_seam::last_written(&captured);
    let written_str = String::from_utf8_lossy(&written);
    assert!(
        written_str.contains(&expected_wrapped),
        "세션이 받은 stream-json 라인이 래핑된 봉투를 포함해야: {written_str}"
    );

    manager.kill_agent(b_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── ADR-0088(FIX-2): 관측 싱크 panic 격리 — 배달/ACK 는 영향 없음(즉시 push 불변식) ───────────────
// panic 하는 observer 를 설치하고 seam 수신자에 성공 배달을 돌려도 handle_send 는 여전히 Enqueued 를
//   돌려줘야 한다(관측을 켰다는 이유로 ACK 유실 → 발신자 재시도 → 중복 배달, 이 회귀를 막는다).
#[tokio::test]
async fn control_send_observer_panic_does_not_break_delivery_or_ack() {
    use engram_dashboard_daemon::control::ingress::{
        handle_send, ControlCommand, DeliveryObservation, DeliveryObserver, Entrance,
    };
    use engram_dashboard_daemon::control::registry::BoundIdentity;

    struct PanicObserver;
    impl DeliveryObserver for PanicObserver {
        fn observe(&self, _obs: DeliveryObservation) {
            panic!("seam: observer boom (의도된 panic — 격리돼야 함)");
        }
    }

    let (manager, registry, _base, data_dir, handle) = wire("obs-panic").await;
    let (b_id, _captured) = obs_seam::insert_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    registry.set_delivery_observer(Arc::new(PanicObserver));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "panic-sender".to_string());
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let cmd = ControlCommand {
        from,
        to: to_name,
        body: "trigger-panic-observer".to_string(),
    };
    // panic 이 record_delivery 에서 격리되지 않으면 여기서 unwind 로 테스트가 죽는다.
    let result = handle_send(&manager, &registry, Entrance::Cli, cmd);
    let v = result.to_json();
    assert_eq!(
        v["status"], "enqueued",
        "observer panic 이 있어도 배달/ACK 는 정상(Enqueued)이어야: {v}"
    );

    manager.kill_agent(b_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── ADR-0088(FIX-3): relay write 실패 → 관측 레코드가 실패를 성공으로 삼키지 않는다 ────────────────
// seam 수신자의 send_input 을 강제 실패시켜 handle_send 의 Err 갈래를 탄다. 관측 레코드는 error=Some,
//   bytes_written=None, msg_uuid=None, is_delivered()==false — "don't swallow failure as success" 증거.
#[tokio::test]
async fn control_send_delivery_failure_observation_records_error_not_success() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, Entrance};
    use engram_dashboard_daemon::control::registry::BoundIdentity;

    let (manager, registry, _base, data_dir, handle) = wire("obs-fail").await;

    // ★fail=true★: 도달성(structured)은 통과하지만 relay write 가 Err — handle_send Err 갈래를 강제.
    let (b_id, _captured) = obs_seam::insert_seam_recipient(&manager, true);
    let to_name = obs_seam::fallback_name(b_id);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "fail-sender".to_string());
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let body = "this-delivery-will-fail";
    let cmd = ControlCommand {
        from,
        to: to_name.clone(),
        body: body.to_string(),
    };
    let result = handle_send(&manager, &registry, Entrance::Cli, cmd);
    let v = result.to_json();
    // 실패는 교정 에러(RECIPIENT_NOT_REACHABLE)로 나가야 한다(성공 ACK 아님).
    assert_eq!(v["status"], "error", "write 실패는 error 로 나가야: {v}");
    assert_eq!(
        v["code"], "RECIPIENT_NOT_REACHABLE",
        "write 실패 교정 코드: {v}"
    );

    let obs = {
        let g = seen.lock().unwrap();
        assert_eq!(g.len(), 1, "실패 relay 1건 → 관측 레코드 1건: {:?}", *g);
        g[0].clone()
    };

    // ★실패의 명시 증거(성공으로 삼키지 않음)★.
    assert!(
        obs.error.is_some(),
        "실패 배달은 error=Some 이어야(성공으로 삼키지 않음): {obs:?}"
    );
    assert_eq!(obs.bytes_written, None, "실패면 bytes_written=None");
    assert_eq!(obs.msg_uuid, None, "실패면 msg_uuid=None(write 안 됨)");
    assert!(!obs.is_delivered(), "실패는 is_delivered()==false");
    // 요청 바이트는 여전히 실려야(넘기려던 봉투 크기 — 무엇을 배달하려다 실패했나의 forensic).
    assert!(
        obs.bytes_requested > body.len(),
        "실패 레코드도 요청 바이트(봉투 크기)는 실려야: req={} body={}",
        obs.bytes_requested,
        body.len()
    );
    // 상관 축(수신자·발신자)은 실패 레코드에도 실린다.
    assert_eq!(obs.to_id, b_id, "실패 레코드 수신자 AgentId");
    assert_eq!(obs.from, from, "실패 레코드 발신자 신원");

    manager.kill_agent(b_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

#[tokio::test]
async fn control_send_delivery_observation_records_bytes_and_correlated_ids() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, Entrance};
    use engram_dashboard_daemon::control::registry::BoundIdentity;

    let (manager, registry, _base, data_dir, handle) = wire("delivery-obs").await;

    // 산 json(stream-json) 수신자 B. 없으면 relay 실측 불가 → loud skip.
    let Some((b_info, _b_tok)) = spawn_json_agent(&manager, &registry, "obs-target") else {
        skip_no_claude("control_send_delivery_observation_records_bytes_and_correlated_ids");
        let _ = std::fs::remove_dir_all(&data_dir);
        handle.shutdown().await;
        return;
    };

    // 배달 관측 싱크 설치(ADR-0088) — handle_send 가 relay 마다 여기로 레코드를 흘린다.
    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    // 발신자 신원(유효).
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "obs-sender-tok".to_string());
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    // ★FIX-5: 멀티바이트 본체★ — 바이트 vs char 회귀를 e2e 경로에서도 잡는다.
    let body = "observe-me-안녕-α"; // ASCII 11자 + 한글2자(6B) + '-'(1B) + α(2B).
    let cmd = ControlCommand {
        from,
        to: "obs-target".to_string(),
        body: body.to_string(),
    };
    let result = handle_send(&manager, &registry, Entrance::Cli, cmd);
    let v = result.to_json();
    assert_eq!(v["status"], "enqueued", "배달 성공 ACK: {v}");
    let ack_id = v["id"].as_str().expect("msg-id 동봉").to_string();

    // 관측 레코드 1건이 나와야 한다.
    let obs = {
        let g = seen.lock().unwrap();
        assert_eq!(g.len(), 1, "성공 relay 1건 → 관측 레코드 1건: {:?}", *g);
        g[0].clone()
    };

    // (a) msg_id ↔ ACK id 상관: 레코드 msg_id 가 ACK 로 나간 논리 메시지 id 와 같아야 한다.
    assert_eq!(
        obs.msg_id, ack_id,
        "레코드 msg_id 는 ACK id 와 같아야(상관 축 1)"
    );
    // (b) msg_uuid 상관 축: write_input 이 만든 session-level replay-dedup 키가 실려야 한다.
    assert!(
        obs.msg_uuid.is_some(),
        "성공 배달은 correlated msg_uuid 를 담아야(상관 축 2)"
    );
    // (c) ★FIX-5: exact 바이트 회계★ — 요청 = wrap_message 봉투의 정확한 UTF-8 바이트 수. 발신자 표시이름은
    //     profile 부재라 sender id 앞8자 fallback, msg_id 는 ack_id 라 봉투를 정확히 재구성할 수 있다.
    //     bytes_written 은 by-construction 복사(short-write 탐지 아님 — 완결성은 error None 으로 본다).
    let sender_name = sender.to_string()[..8].to_string();
    let expected_wrapped = format!("[message from {sender_name} id:{ack_id}] {body}");
    assert_eq!(
        obs.bytes_requested,
        expected_wrapped.len(), // String::len = UTF-8 바이트 수(char 수 아님).
        "요청 바이트 = 봉투의 정확 UTF-8 바이트 수(멀티바이트 관통): got={} wrapped={:?}",
        obs.bytes_requested,
        expected_wrapped
    );
    assert_eq!(
        obs.bytes_written,
        Some(obs.bytes_requested),
        "by-construction 복사(bytes_written = 요청) — short-write 탐지 아님"
    );
    // (d) 성공은 error None + is_delivered().
    assert!(obs.error.is_none(), "성공 배달은 error None");
    assert!(obs.is_delivered(), "is_delivered() = true(전송 완결)");
    // 수신자 신원/이름도 실렸는지.
    assert_eq!(obs.to_id, b_info.id, "레코드 수신자 AgentId");
    assert_eq!(obs.to_name, "obs-target", "레코드 수신자 이름");
    assert_eq!(obs.from, from, "레코드 발신자 신원(토큰 파생)");

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
