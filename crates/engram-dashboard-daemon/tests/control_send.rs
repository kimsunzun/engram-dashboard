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

    /// ★ADR-0088 Stage 1★: 캡처된 모든 write 를 **순서대로** 스냅샷한다(디코딩 없이 원바이트). 동시성
    ///   오라클 검증용 — 각 원소는 send_input 1회가 받은 **이미 완결된 봉투 봉인**(stream-json 라인)이다.
    ///   ★정직 범위(seam 이 무엇을 잡고 무엇을 못 잡나)★: SeamTransport 는 `push(bytes)` 로 캡처하는데
    ///   push 는 원자라 두 스레드의 바이트가 한 Vec 안에서 섞이지 않는다. 이는 **session 조립 계약**
    ///   (session.write_input_observed 가 encoder 로 완결 봉투 1개를 만들어 send_input 에 통째로 넘김)을
    ///   확인할 뿐이다 — 각 write 가 온전한 봉투면 "session 이 봉투를 쪼개거나 합치지 않았다"의 증거다.
    ///   ★이것은 물리 OS-pipe 무인터리브의 증거가 아니다★: 진짜 pipe 경계 직렬화는 운영 StdioTransport 의
    ///   `stdin.lock()`(write_all+flush 내내 보유, stdio.rs ~322)이 담당하는데 이 seam 은 그 계층을
    ///   **우회**한다(이미 완결된 Vec 을 받는다). 그 lock 을 지우는 회귀는 이 스냅샷으로 **안 잡힌다**
    ///   (오라클 1 docstring 의 커버리지 공백 참조).
    pub fn all_written(captured: &Arc<Mutex<Vec<Vec<u8>>>>) -> Vec<Vec<u8>> {
        captured.lock().unwrap().clone()
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

// ═══════════════════════════════════════════════════════════════════════════════════════════
// ADR-0088 Stage 1 — 배달 정확성 오라클 (결정적·seam 기반, 실 claude 불요)
// ═══════════════════════════════════════════════════════════════════════════════════════════
// ★프레이밍(정직 범위 — 무엇을 증명하고 무엇을 못 하나)★: 아래는 green-chasing 이 아니라 **정확성**
//   테스트다. 다만 seam 기반이라 증명 범위가 seam 관측면에 갇힌다 — 이 한계를 각 오라클 docstring 이
//   정확히 밝힌다(과대 주장 금지, 리뷰 FIX). seam 은 handle_send → registry → write_stdin_observed
//   → session.write_input_observed(봉투 조립·encoder) → SeamTransport.send_input **까지**를 관측한다.
//   그 아래 물리 계층(운영 StdioTransport 의 `stdin.lock()` + `write_all`/`flush`, stdio.rs ~322)은
//   이 seam 이 **우회**한다 — SeamTransport 는 이미 완결된 Vec 을 받아 `push` 로 원자 캡처하므로.
//   ▶ 이 하네스가 **확립**하는 것: 경계(본체 크기·바이트-vs-char) + 순차 수명(부재/실패/epoch 교체)
//     + 동시 **입구(entry)** exact-once(handle_send/registry/observed-write 레벨의 무유실·무중복) +
//     각 봉투가 transport 에 **완결된 정확-바이트 버퍼 1개**로 넘어감(session 조립 계약).
//   ▶ 이 하네스가 **커버하지 않는 것**(coverage gap / follow-up): (1) 물리 OS-pipe 바이트 무인터리브
//     — `stdin.lock()` 이 담당, 이 seam 아래라 lock 을 지워도 여기선 안 걸린다; (2) 부분 write 후 Err
//     (prefix 만 쓰고 실패) — seam 은 push 전에 실패하므로 truncation 관측 불가; (3) 진짜 mid-flight
//     epoch race(resolve 가 epoch0 을 보고 write 가 epoch1 로 간 뒤 도착) — resolve↔write 사이 yield
//     seam 이 프로덕션에 없어 결정적 재현 불가.
//   운영 동작이 (확립 범위 안에서) 오라클을 위반하면 테스트를 약화하지 않고 실패로 남겨 FINDING 으로
//   보고한다(마스킹 금지). 커버 안 되는 축은 아래 각 오라클의 "커버리지 공백" 및 반환 follow-up 목록.

/// ── ADR-0088 Stage 1-오라클 1: 동시 **입구** exact-once + N 개 distinct 본체 무결 배달(seam handoff) ──
/// N 개 OS 스레드가 `Barrier` 로 **입구를 정렬 후 near-simultaneous** handle_send 발화 → 하나의 seam
///   수신자에게 각기 고유 본체를 보낸다(barrier 로 **진입(entry)** 을 near-simultaneous 하게 정렬 — 초반
///   스레드가 후반 시작 전에 끝나 race window 가 안 열리는 문제 제거. 단 barrier 는 진입 정렬만 보장할 뿐
///   handle_send **내부의 실행 겹침**까지는 강제하지 못한다 — 단일코어/스케줄러가 여전히 직렬화할 수 있다).
///
/// ★증명한다(seam 관측면)★:
///   (i) **exact-once (N distinct 본체)**: handle_send/registry/observed-write 레벨에서 각 메시지 정확히
///       1회 — 관측 레코드 N건 + msg_id 전부 distinct + ACK id 전부 distinct + **수신된 본체 다중집합 ==
///       발신된 N 개 distinct 본체 집합(각 정확히 1회 — 무유실·무중복·무치환)**. 이 본체 다중집합 등식이
///       "치환 버그(모든 메시지 → 같은 본체)" 를 차단한다(각 write 자기일관 검사만으론 안 잡힘).
///   (ii) **session→transport handoff 무결**: 각 봉투가 transport 에 **완결된 정확-바이트 버퍼 1개**로
///       넘어감 — 캡처된 write 다중집합이 (각 관측의 msg_uuid 로 재구성한) 기대 encoded 라인 다중집합과
///       **정확히 일치**(exact bytes). 즉 session 이 encoder 출력을 잘라내거나 두 메시지 바이트를 한
///       write 로 합치는 등 **handoff 를 오염시키면** 여기서 깨진다. (encoder **내부** 정확성은 이 검사가
///       증명하지 않는다 — actual·expected 가 같은 encoder 를 쓰므로. FIX-2 참조: encoder 정확성은
///       claude.rs 의 golden unit test `wrap_user_turn_exact_line_and_newline_terminated` 가 커버.)
/// ★증명하지 않는다(커버리지 공백 — follow-up)★: **물리 OS-pipe 바이트 무인터리브**. 그 직렬화는
///   운영 StdioTransport 의 `stdin.lock()`(write_all+flush 내내 보유, stdio.rs ~322)이 담당하는데
///   이 seam 은 그 계층을 **우회**한다(SeamTransport 는 완결 Vec 을 원자 push). 그 lock 을 지우는 회귀는
///   여기서 **안 잡힌다** → 진짜 물리 인터리브 검증은 실 StdioTransport+실 pipe(강제 부분 write/느린
///   reader) 하네스가 필요(반환 follow-up).
#[tokio::test]
async fn stage1_concurrent_sends_exact_once_distinct_bodies_intact_at_seam() {
    use engram_dashboard_core::agent::backend::InputEncoder;
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, Entrance};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use std::sync::Barrier;

    let (manager, registry, _base, data_dir, handle) = wire("stage1-concurrency").await;

    // 하나의 seam 수신자(성공 경로). captured 는 순서 있는 다중 write 를 그대로 담는다.
    let (b_id, captured) = obs_seam::insert_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    // 배달 관측 싱크 — N건이 전부 성공 레코드로 남는지 본다(exact-once 의 관측 축). 관측 레코드는
    //   봉투 재구성에 필요한 (msg_id, msg_uuid) 쌍도 담아 아래 exact-bytes 다중집합 검사의 기대치를 만든다.
    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    // 발신자 신원(유효) — 모든 스레드가 같은 신원으로 보낸다(수신자 1개에 몰아치는 게 요점).
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "stage1-conc-sender".to_string());
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    const N: usize = 100;
    // 각 스레드의 고유 본체 = 안정 마커(BODY-<zero-padded idx>). 특수문자 없음(JSON escape 회피 → 봉투
    //   문자열이 캡처 라인에 부분열로 그대로 들어감). idx 를 zero-pad 해 부분열 오검(1 ⊂ 10)도 방지.
    let markers: Vec<String> = (0..N).map(|i| format!("BODY-{i:04}")).collect();

    // ★Barrier(입구 정렬)★: N 스레드가 handle_send **진입 직전**에 전부 모여 near-simultaneous 하게
    //   풀린다 — 초반 스레드가 후반 스레드 spawn 전에 끝나 race window 가 안 열리는 문제를 제거한다.
    //   ★한계★: barrier 는 진입(entry) 을 near-simultaneous 하게 정렬할 뿐 handle_send **내부의 실행
    //   겹침**까지 강제하지 못한다(단일코어/스케줄러가 여전히 직렬화 가능). 그래도 진입 정렬만으로 초반-
    //   스레드-먼저-끝남 문제는 사라져 registry/observed-write 경로의 exact-once 를 near-simultaneous
    //   진입 하에서 실측한다.
    let barrier = Arc::new(Barrier::new(N));

    let mut handles = Vec::with_capacity(N);
    for marker in &markers {
        let manager = manager.clone();
        let registry = registry.clone();
        let to = to_name.clone();
        let body = marker.clone();
        let barrier = barrier.clone();
        // handle_send 는 sync(&Arc<..>) — OS 스레드로 near-simultaneous 발화(tokio task 아님, 병렬성 확보).
        handles.push(std::thread::spawn(move || {
            barrier.wait(); // ★입구 정렬 — 모든 스레드가 여기 모인 뒤 near-simultaneous 하게 handle_send 로 돌진(실행 겹침 강제는 아님)★.
            let cmd = ControlCommand { from, to, body };
            let result = handle_send(&manager, &registry, Entrance::Cli, cmd);
            let v = result.to_json();
            // 각 발화는 성공 ACK + 고유 msg_id 를 받아야(중복/유실 없음의 발신측 증거).
            assert_eq!(v["status"], "enqueued", "동시 발화도 각기 enqueued: {v}");
            v["id"].as_str().expect("msg-id").to_string()
        }));
    }
    let ack_ids: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    // (i) exact-once — 관측 레코드 N건, msg_id 전부 distinct + ACK id 전부 distinct.
    let obs_records = { seen.lock().unwrap().clone() };
    assert_eq!(
        obs_records.len(),
        N,
        "동시 N 발화 → 관측 레코드 정확히 N건(동시 입구 유실/중복 없음)"
    );
    let distinct_obs: std::collections::HashSet<&String> =
        obs_records.iter().map(|o| &o.msg_id).collect();
    assert_eq!(
        distinct_obs.len(),
        N,
        "관측 msg_id 전부 distinct(중복 배달 없음)"
    );
    let distinct_ack: std::collections::HashSet<&String> = ack_ids.iter().collect();
    assert_eq!(distinct_ack.len(), N, "ACK id 전부 distinct");

    // (ii) ★봉투 조립 정확-바이트 다중집합 등식★: 각 캡처 write 에 대해 그 봉투가 session 에 넘어갔을
    //   **정확한 encoded 바이트**를 재구성해 exact-eq 비교하고, N 개 write 가 N 개 관측 레코드에 1:1 로
    //   매칭됨(다중집합 등식)을 단언한다. 재구성 경로(관측 레코드엔 body 가 없으므로 캡처 write 에서 결합):
    //     ① 캡처 라인(stream-json)의 top-level "uuid" = 그 봉투를 만든 msg_uuid(wrap_user_turn 이 심음).
    //     ② 그 msg_uuid 로 관측 레코드를 찾아 msg_id 를 얻는다(봉투 prefix `id:<msg_id>` 확정).
    //     ③ 캡처 write 안의 유일 마커 = body(각 스레드 고유).
    //     ④ wrapped = "[message from <sender8> id:<msg_id>] <body>" →
    //        expected = InputEncoder::ClaudeStreamJson.encode(wrapped, msg_uuid)
    //   ④ 는 session.write_input_observed 가 실제로 send_input 에 넘긴 바로 그 바이트다(같은 encoder·
    //   같은 msg_uuid). ★이 검사가 증명하는 것★: session→transport **handoff 무결** — session 이
    //   encoder 출력을 잘라내거나(truncate) 오염시키거나 두 봉투를 한 write 로 합치면 캡처 write ≠
    //   expected 라 깨진다(그리고 `bytes_requested` 만으론 이 handoff 오염을 못 잡는다). ★증명하지
    //   않는 것★: encoder **내부** 정확성 — expected 도 같은 encoder 로 만들므로 encoder 자체 결함
    //   (예: wrap_user_turn 이 개행을 빠뜨림)은 양쪽을 똑같이 오염시켜 여기선 안 걸린다. encoder
    //   정확성은 claude.rs 의 golden unit test `wrap_user_turn_exact_line_and_newline_terminated` 소관.
    let sender_name = obs_seam::fallback_name(sender);
    let writes = obs_seam::all_written(&captured);
    assert_eq!(
        writes.len(),
        N,
        "캡처된 write 수 == N(각 send_input 이 완결 봉투 1개 — 잘림/합병 없음)"
    );
    // msg_uuid → 관측 레코드(정확 봉투 재구성용). 성공 레코드는 msg_uuid Some.
    let by_uuid: std::collections::HashMap<
        uuid::Uuid,
        &engram_dashboard_daemon::control::ingress::DeliveryObservation,
    > = obs_records
        .iter()
        .filter_map(|o| o.msg_uuid.map(|u| (u, o)))
        .collect();
    assert_eq!(
        by_uuid.len(),
        N,
        "성공 레코드마다 고유 msg_uuid(상관 키 충돌 없음)"
    );

    // 각 캡처 write 를 정확 기대 바이트와 대조한다. write 안의 유일 마커로 body 를, 그 write 의 encoded
    //   라인을 파싱해 담긴 msg_uuid 로 관측 레코드를 찾아 msg_id 를 얻어 봉투를 완성 → 재-encode 해 exact-eq.
    let mut matched_uuids: std::collections::HashSet<uuid::Uuid> = std::collections::HashSet::new();
    // ★수신 본체 다중집합★(FIX-1): 각 write 에서 실제로 배달된 body 마커를 모은다. 아래 exact-bytes
    //   재구성은 body 를 그 write 자신에서 뽑아(self-consistent) 검사하므로 "모든 메시지 → 같은 body"
    //   치환 버그를 자기일관적으로 통과시킨다. 그걸 막으려면 수신된 본체 다중집합이 발신된 N 개 distinct
    //   마커 집합과 정확히 같은지(각 1회) 별도로 대조해야 한다.
    let mut received_bodies: Vec<String> = Vec::with_capacity(N);
    for (i, w) in writes.iter().enumerate() {
        // 온전한 UTF-8 라인이어야(물리 인터리브면 여기서 U+FFFD 로 깨질 수 있으나 — 그 검증은 seam 밖·follow-up).
        let s = std::str::from_utf8(w)
            .unwrap_or_else(|e| panic!("write[{i}] 가 온전한 UTF-8 이 아님: {e}"));
        // 캡처 라인(stream-json)에서 이 봉투의 msg_uuid 를 파싱한다(top-level "uuid" 필드 = wrap_user_turn).
        let line_json: serde_json::Value = serde_json::from_str(s.trim_end()).unwrap_or_else(|e| {
            panic!("write[{i}] 가 온전한 stream-json 라인이 아님(합병/잘림 의심): {e} in {s:?}")
        });
        let line_uuid: uuid::Uuid = line_json["uuid"]
            .as_str()
            .and_then(|u| u.parse().ok())
            .unwrap_or_else(|| panic!("write[{i}] 에 top-level uuid 없음: {s:?}"));
        let obs = by_uuid.get(&line_uuid).unwrap_or_else(|| {
            panic!("write[{i}] 의 msg_uuid={line_uuid} 에 대응하는 관측 레코드 없음(유령 write)")
        });
        assert!(
            matched_uuids.insert(line_uuid),
            "write[{i}] 의 msg_uuid={line_uuid} 가 두 번 캡처됨(중복 write)"
        );
        // 이 write 안의 유일 마커 = body(각 스레드 고유). 정확히 1개여야(두 메시지 바이트가 한 write 에
        //   섞이면 2개가 보인다 — seam 레벨 합병 탐지).
        let hits: Vec<&String> = markers.iter().filter(|m| s.contains(m.as_str())).collect();
        assert_eq!(
            hits.len(),
            1,
            "write[{i}] 는 봉투 마커 정확히 1개만 담아야(seam 레벨 무합병) — 관측: {hits:?}"
        );
        let body = hits[0];
        // ★FIX-1: 수신 본체 다중집합에 적재(치환 버그 차단용 — 루프 뒤 발신 마커 집합과 대조)★.
        received_bodies.push(body.clone());
        // ★정확-바이트 재구성★: 이 봉투가 session 에 넘어갔을 바로 그 바이트 = encoder(봉투, 그 msg_uuid).
        let wrapped = format!("[message from {sender_name} id:{}] {body}", obs.msg_id);
        let expected_line = InputEncoder::ClaudeStreamJson.encode(wrapped.as_bytes(), line_uuid);
        assert_eq!(
            w, &expected_line,
            "write[{i}] 가 기대 encoded 봉투와 바이트-정확 일치해야(session→transport handoff 잘림/오염/합병 탐지 — encoder 내부 정확성 아님): body={body} msg_id={}",
            obs.msg_id
        );
    }
    // 모든 성공 레코드의 msg_uuid 가 정확히 한 write 로 매칭됐는지(집합 등식 = 유실/중복 없음).
    assert_eq!(
        matched_uuids.len(),
        N,
        "N 개 msg_uuid 전부 정확히 1 write 로 배달(exact-once, 다중집합 등식)"
    );

    // ★FIX-1: 수신 본체 다중집합 == 발신된 N 개 distinct 본체(각 정확히 1회)★.
    //   위 exact-bytes 재구성은 body 를 그 write 자신에서 뽑아 검사하므로 "모든 메시지 → 같은 body"
    //   치환 버그를 자기일관적으로 통과시킨다(각 write 가 BODY-0000 을 담고 BODY-0000 으로 재구성 → 통과).
    //   여기서 수신 본체 다중집합을 발신 마커 집합과 직접 대조해 그 구멍을 막는다: sorted 두 벡터가
    //   같아야(발신 마커는 전부 distinct 이므로 이 등식 = "N 개 distinct 본체가 각 정확히 1회 배달,
    //   무유실·무중복·무치환"). 발신 마커는 이미 distinct 이나 방어적으로 확인한다.
    {
        let mut sent_sorted: Vec<String> = markers.clone();
        sent_sorted.sort();
        let distinct_sent: std::collections::HashSet<&String> = sent_sorted.iter().collect();
        assert_eq!(
            distinct_sent.len(),
            N,
            "테스트 전제: 발신 마커는 전부 distinct"
        );
        let mut received_sorted = received_bodies.clone();
        received_sorted.sort();
        assert_eq!(
            received_sorted, sent_sorted,
            "수신 본체 다중집합이 발신된 N 개 distinct 본체와 정확히 일치해야(각 1회 — 무유실·무중복·무치환). \
             치환 버그(모든 메시지 → 같은 body)면 이 등식이 깨진다(한 body 가 N 번, 나머지 0번). \
             sent={sent_sorted:?} received={received_sorted:?}"
        );
    }

    manager.kill_agent(b_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// ── ADR-0088 Stage 1-오라클 2: 본체 크기 경계(MAX_BODY_BYTES = 64 KiB), 바이트 vs char ──────────
/// 상한 근처 본체: 정확히 64 KiB, 64 KiB−1, 64 KiB+1, 그리고 바이트 길이가 경계를 straddle 하는
///   멀티바이트(UTF-8) 본체. 오라클:
///   - >64 KiB → BODY_TOO_LARGE 교정(write 시도 없음 = 캡처 0),
///   - ≤64 KiB → 배달됨 + seam 캡처 write 가 **기대 encoded 봉투와 바이트-정확 일치**(msg_uuid 로 재구성)
///     + DeliveryObservation.bytes_requested == 봉투의 정확 바이트 길이,
///   - 상한은 char 수가 아니라 **바이트** 로 잰다(멀티바이트 본체의 char 수는 64Ki 미만인데 바이트는 초과).
/// ★정직 범위(FIX-2)★: 수용 케이스의 캡처 write 대조가 증명하는 것은 **session→transport handoff
///   무결** — session 이 encoder 출력을 잘라내거나(truncate) 오염시키지 않고 그대로 transport 에
///   넘겼는가다. `bytes_requested` 는 encoding **이전** 봉투 복사값이라 handoff 에서의 truncation 을
///   못 잡으므로 캡처 바이트를 직접 대조한다. ★증명하지 않는 것★: encoder **내부** 정확성 — expected
///   도 같은 `InputEncoder::ClaudeStreamJson.encode` 로 만들므로 encoder 자체 결함(예: wrap_user_turn
///   이 개행/본체를 빠뜨림)은 actual·expected 를 똑같이 오염시켜 여기선 안 걸린다. encoder 정확성은
///   claude.rs 의 golden unit test `wrap_user_turn_exact_line_and_newline_terminated` 소관.
///   이 아래 물리 write(부분 write/OS-pipe)는 seam 밖(follow-up).
#[tokio::test]
async fn stage1_body_size_boundary_bytes_not_chars() {
    use engram_dashboard_core::agent::backend::InputEncoder;
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, Entrance};
    use engram_dashboard_daemon::control::registry::BoundIdentity;

    const MAX: usize = 64 * 1024; // = MAX_BODY_BYTES(ingress 상수 — 여기 미러; 값 드리프트 시 아래가 잡는다).

    let (manager, registry, _base, data_dir, handle) = wire("stage1-boundary").await;

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "stage1-boundary-sender".to_string());
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    // 한 요청을 보내고 (ControlResult, 관측 레코드 Option, 마지막 캡처 write, 기대 봉투 바이트 길이,
    //   ★기대 encoded 라인★, 수신자 id) 를 돌려주는 로컬 헬퍼. 매 케이스마다 fresh seam 수신자 + fresh
    //   observer 를 심어 상태 누적을 피한다. 기대 encoded 라인 = 성공 시 관측 레코드의 msg_uuid 로
    //   `InputEncoder::ClaudeStreamJson.encode(봉투, msg_uuid)` 재구성(= session 이 send_input 에 넘긴
    //   바로 그 바이트) — 수용 케이스의 캡처 write 와 exact-eq 비교용. 실패/거부면 빈 Vec.
    async fn send_once(
        manager: &Arc<AgentManager>,
        registry: &Arc<ControlRegistry>,
        from: BoundIdentity,
        body: String,
    ) -> (
        serde_json::Value,
        Option<engram_dashboard_daemon::control::ingress::DeliveryObservation>,
        Vec<u8>,
        usize,   // 봉투(wrap_message) 의 기대 바이트 길이
        Vec<u8>, // 기대 encoded stream-json 라인(성공 시 재구성, 실패 시 빈 Vec)
        AgentId,
    ) {
        let (b_id, captured) = obs_seam::insert_seam_recipient(manager, false);
        let to_name = obs_seam::fallback_name(b_id);
        let seen = Arc::new(Mutex::new(Vec::new()));
        registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

        let cmd = ControlCommand {
            from,
            to: to_name.clone(),
            body: body.clone(),
        };
        let result = handle_send(manager, registry, Entrance::Cli, cmd);
        let v = result.to_json();
        let obs = seen.lock().unwrap().first().cloned();
        let written = obs_seam::last_written(&captured);
        // 기대 봉투 바이트 = "[message from <sender8> id:<msg_id>] <body>" 의 UTF-8 len.
        //   msg_id 는 ACK 로 나온 것(성공 시)만 알 수 있으므로 성공 케이스에서만 정확 계산에 쓴다.
        let sender_name = obs_seam::fallback_name(from.agent_id);
        let expected_env_bytes = v["id"]
            .as_str()
            .map(|mid| format!("[message from {sender_name} id:{mid}] {body}").len())
            .unwrap_or(0);
        // ★기대 encoded 라인 재구성★: 성공 시 봉투를 관측 레코드의 msg_uuid 로 재-encode 한다(= session 이
        //   실제 send_input 에 넘긴 바이트). msg_id·msg_uuid 둘 다 있어야 하므로 성공 레코드에서만 만든다.
        let expected_line = match (v["id"].as_str(), obs.as_ref().and_then(|o| o.msg_uuid)) {
            (Some(mid), Some(uuid)) => {
                let wrapped = format!("[message from {sender_name} id:{mid}] {body}");
                InputEncoder::ClaudeStreamJson.encode(wrapped.as_bytes(), uuid)
            }
            _ => Vec::new(),
        };
        (v, obs, written, expected_env_bytes, expected_line, b_id)
    }

    // ── (1) 정확히 64 KiB(경계 포함) → 배달됨 ──────────────────────────────────────────────
    let body_eq = "x".repeat(MAX);
    assert_eq!(body_eq.len(), MAX, "테스트 전제: 정확히 64 KiB");
    let (v, obs, written, env_bytes, expected_line, b_id) =
        send_once(&manager, &registry, from, body_eq.clone()).await;
    assert_eq!(
        v["status"], "enqueued",
        "정확히 64 KiB 는 배달돼야(≤ 상한): {v}"
    );
    let obs = obs.expect("성공 배달은 관측 레코드");
    assert_eq!(
        obs.bytes_requested, env_bytes,
        "요청 바이트 = 봉투의 정확 바이트 길이"
    );
    assert!(obs.is_delivered(), "정확히 64 KiB 는 is_delivered()");
    // ★캡처 write 가 기대 encoded 봉투와 바이트-정확 일치★ — session 이 64 KiB 봉투를 handoff 에서
    //   잘라내거나 오염시키면 여기서 잡힌다(bytes_requested 는 encoding 이전 복사라 handoff truncation
    //   못 잡음 — 캡처 바이트를 직접 대조). encoder 내부 정확성 아님(expected 도 같은 encoder).
    assert_eq!(
        written, expected_line,
        "seam 캡처가 64 KiB 봉투의 정확 encoded 바이트여야(session→transport handoff 잘림/오염 탐지)"
    );
    manager.kill_agent(b_id).ok();

    // ── (2) 64 KiB − 1 → 배달됨 ───────────────────────────────────────────────────────────
    let body_lt = "x".repeat(MAX - 1);
    let (v, obs, written, env_bytes, expected_line, b_id) =
        send_once(&manager, &registry, from, body_lt).await;
    assert_eq!(v["status"], "enqueued", "64 KiB−1 은 배달돼야: {v}");
    assert_eq!(
        obs.expect("관측").bytes_requested,
        env_bytes,
        "64 KiB−1: 요청 바이트 = 봉투 정확 길이"
    );
    assert_eq!(
        written, expected_line,
        "64 KiB−1: 캡처 write 가 기대 encoded 봉투와 바이트-정확 일치(잘림 탐지)"
    );
    manager.kill_agent(b_id).ok();

    // ── (3) 64 KiB + 1 → BODY_TOO_LARGE, write 시도 없음(캡처 0) ────────────────────────────
    let body_gt = "x".repeat(MAX + 1);
    let (b_id, captured) = obs_seam::insert_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);
    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));
    let result = handle_send(
        &manager,
        &registry,
        Entrance::Cli,
        ControlCommand {
            from,
            to: to_name,
            body: body_gt,
        },
    );
    let v = result.to_json();
    assert_eq!(v["status"], "error", "64 KiB+1 은 거부: {v}");
    assert_eq!(v["code"], "BODY_TOO_LARGE", "초과는 BODY_TOO_LARGE: {v}");
    assert!(
        obs_seam::all_written(&captured).is_empty(),
        "상한 초과는 write 시도 자체가 없어야(캡처 0 — 바이트가 수신자에 안 닿음)"
    );
    assert!(
        seen.lock().unwrap().is_empty(),
        "상한 초과는 배달 관측 레코드도 없어야(relay 미진입)"
    );
    manager.kill_agent(b_id).ok();

    // ── (4) 멀티바이트: char 수 < 64Ki 이나 바이트 > 64Ki → BODY_TOO_LARGE(상한=바이트 증명) ──────
    // '가'(U+AC00) = UTF-8 3바이트. (MAX/3 + 1) char → char 수는 ~21846(≪ 64Ki 문자)인데 바이트는 > MAX.
    let char_count = MAX / 3 + 1;
    let body_mb_over = "가".repeat(char_count);
    assert!(
        body_mb_over.chars().count() < MAX,
        "멀티바이트 전제: char 수({}) 는 64Ki 미만",
        body_mb_over.chars().count()
    );
    assert!(
        body_mb_over.len() > MAX,
        "멀티바이트 전제: 바이트 수({}) 는 64Ki 초과",
        body_mb_over.len()
    );
    let (b_id, captured) = obs_seam::insert_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);
    let result = handle_send(
        &manager,
        &registry,
        Entrance::Cli,
        ControlCommand {
            from,
            to: to_name,
            body: body_mb_over,
        },
    );
    let v = result.to_json();
    assert_eq!(
        v["code"], "BODY_TOO_LARGE",
        "멀티바이트 본체도 상한은 char 가 아니라 **바이트** 로 잰다(char<64Ki 인데 거부돼야): {v}"
    );
    assert!(
        obs_seam::all_written(&captured).is_empty(),
        "멀티바이트 초과도 write 시도 없음"
    );
    manager.kill_agent(b_id).ok();

    // ── (5) 멀티바이트 straddle: char 수는 그대로인데 바이트가 경계 바로 아래 → 배달됨(경계의 바이트성 확인) ──
    // (MAX/3) char → 정확히 MAX 바이트(MAX 가 3 의 배수는 아니므로 MAX - (MAX%3) 바이트). ≤ MAX 라 배달돼야.
    let char_count_ok = MAX / 3; // floor → 바이트 = char_count_ok*3 ≤ MAX
    let body_mb_ok = "가".repeat(char_count_ok);
    assert!(
        body_mb_ok.len() <= MAX,
        "straddle 전제: 바이트({}) ≤ 64Ki",
        body_mb_ok.len()
    );
    let (v, obs, written, env_bytes, expected_line, b_id) =
        send_once(&manager, &registry, from, body_mb_ok).await;
    assert_eq!(
        v["status"], "enqueued",
        "바이트 ≤ 64Ki 인 멀티바이트 본체는 배달돼야(경계는 바이트로 판정): {v}"
    );
    assert_eq!(
        obs.expect("관측").bytes_requested,
        env_bytes,
        "멀티바이트 straddle: 요청 바이트 = 봉투 정확 UTF-8 길이(char 수 아님)"
    );
    // 멀티바이트 수용 케이스도 캡처 write 가 기대 encoded 봉투와 바이트-정확 일치(session 이 멀티바이트
    //   봉투를 handoff 에서 잘못 자르거나 오염시키는 회귀를 잡는다 — encoder 내부 정확성 아님, expected
    //   도 같은 encoder). encoder 자체 결함은 claude.rs golden test 소관.
    assert_eq!(
        written, expected_line,
        "멀티바이트 straddle: 캡처 write 가 기대 encoded 봉투와 바이트-정확 일치(handoff 무결)"
    );
    manager.kill_agent(b_id).ok();

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// ── ADR-0088 Stage 1-오라클 3(a): 수신자 부재 → RECIPIENT_NOT_FOUND, 배달 관측 없음 ──────────────
/// 해석 시점에 수신자가 아예 없으면 교정 에러 + relay 미진입(관측 레코드 0). registry 단위 테스트가
///   resolve 로직을 커버하나, 여기선 **배달 경계 관측이 안 남는다**(부분/유령 레코드 없음)까지 못 박는다.
#[tokio::test]
async fn stage1_lifecycle_recipient_absent_not_found_no_observation() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, Entrance};
    use engram_dashboard_daemon::control::registry::BoundIdentity;

    let (manager, registry, _base, data_dir, handle) = wire("stage1-absent").await;

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "stage1-absent-sender".to_string());
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let result = handle_send(
        &manager,
        &registry,
        Entrance::Cli,
        ControlCommand {
            from,
            to: "no-such-agent".to_string(),
            body: "hi".to_string(),
        },
    );
    let v = result.to_json();
    assert_eq!(v["code"], "RECIPIENT_NOT_FOUND", "부재 수신자: {v}");
    assert!(
        seen.lock().unwrap().is_empty(),
        "부재 수신자는 배달 관측 레코드를 남기지 않아야(유령 배달 없음)"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// ── ADR-0088 Stage 1-오라클 4(write 실패): 실패 **관측 형태** — 단일 실패 레코드(부분/중복 관측 없음) ──
/// 수신자가 도달 가능(structured)하나 relay write(send_input)가 Err.
/// ★증명한다★: 실패의 **관측 형태(observation shape)** — send_input 이 Err 를 낼 때 레코드가 정확히
///   1건 + error=Some + bytes_written=None + msg_uuid=None + !is_delivered(성공 필드가 하나도 새지
///   않음). 봉투 바이트(bytes_requested)는 실려도(무엇을 배달하려다 실패했나의 forensic) 성공 신호는 안 샌다.
/// ★증명하지 않는다(커버리지 공백 — follow-up)★: **실제 OS write 가 prefix 를 쓴 뒤 Err 를 내는
///   부분 배달/truncation 부재**. 이 seam 은 send_input 이 push **전에** 통째로 Err 를 반환하므로(원자
///   all-or-nothing 모사) "prefix 만 쓰이고 실패" 상황 자체가 발생하지 않는다 — 그 축은 실 pipe(접두를
///   받아들인 뒤 끊기는)를 쓰는 하네스가 필요(반환 follow-up).
#[tokio::test]
async fn stage1_lifecycle_write_error_single_failure_no_partial_dup() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, Entrance};
    use engram_dashboard_daemon::control::registry::BoundIdentity;

    let (manager, registry, _base, data_dir, handle) = wire("stage1-write-err").await;

    // fail=true → 도달성(structured) 통과하되 send_input 이 Err.
    let (b_id, captured) = obs_seam::insert_seam_recipient(&manager, true);
    let to_name = obs_seam::fallback_name(b_id);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "stage1-write-err-sender".to_string());
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let result = handle_send(
        &manager,
        &registry,
        Entrance::Cli,
        ControlCommand {
            from,
            to: to_name.clone(),
            body: "will-fail-once".to_string(),
        },
    );
    let v = result.to_json();
    assert_eq!(v["status"], "error", "write 실패는 error: {v}");
    assert_eq!(v["code"], "RECIPIENT_NOT_REACHABLE", "write 실패 교정: {v}");

    // 정확히 1건의 실패 레코드 — 부분/중복 없음.
    let g = seen.lock().unwrap();
    assert_eq!(
        g.len(),
        1,
        "실패도 관측 레코드 정확히 1건(부분/중복 없음): {:?}",
        *g
    );
    let obs = &g[0];
    assert!(obs.error.is_some(), "실패 = error Some");
    assert_eq!(
        obs.bytes_written, None,
        "실패 = bytes_written None(성공 필드 누출 없음)"
    );
    assert_eq!(obs.msg_uuid, None, "실패 = msg_uuid None");
    assert!(!obs.is_delivered(), "실패 = !is_delivered()");
    // fail seam 은 send_input 에서 Err 를 내기 전 push 하지 않으므로 캡처는 비어야(바이트가 안 꽂혔다).
    assert!(
        obs_seam::all_written(&captured).is_empty(),
        "write 실패면 수신자에 바이트가 꽂히지 않아야(캡처 0)"
    );

    drop(g);
    manager.kill_agent(b_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// ── ADR-0088 Stage 1-오라클 5(epoch): **순차** incarnation 교체 시맨틱 — 현재 incarnation 에 배달 ──────
/// ★증명한다(순차 교체)★: ADR-0086 §F5 는 epoch pinning 을 **하지 않는다**(메일은 논리 에이전트=안정
///   주소를 향함). 이 테스트는 그 **순차** 시맨틱을 결정적으로 확인한다: seam 수신자를 같은 AgentId 로
///   **교체 주입**(=incarnation 교체가 이미 끝난 맵 상태)한 뒤 그 이름으로 보내면, 메시지는 **현재 맵에
///   있는 그 AgentId 의 incarnation** 으로 배달되고(유실 없음), 교체된 맵 상태에서 wrong-epoch 로 이중
///   배달되지 않는다(레코드 1건). 배달 실패 시 조용히 유실되지 않고 도달 에러로 표면화돼야 한다.
///
/// ★증명하지 않는다(커버리지 공백 — follow-up)★: **진짜 mid-flight epoch race** — resolve 가 epoch 0 을
///   보고, 그 직후 재시작으로 epoch 1 이 current 가 된 뒤 write 가 epoch 1 로 도착하는 시나리오. 이건
///   순차 교체가 아니라 resolve↔write **사이**의 경쟁인데, handle_send 안에서 list_agents(해석)와
///   write_stdin_observed(write) 사이에 외부가 끼어들 **yield seam 이 프로덕션에 없어**(둘 다 동기, 그
///   사이 yield point 없음) 결정적으로 재현할 수 없다. ★ADR-0086 §F5 는 이 race 를 **design-accepted**
///   로 표시한다 — 이 테스트는 그 race 가 안전하다고 **주장하지 않는다**(주장할 근거를 만들지 못함).
///   결정적 커버리지는 프로덕션에 test-hookable yield-seam(설계 결정)이 필요(반환 follow-up).
///
/// ★관측 한계(follow-up)★: 현재 DeliveryObservation 은 수신자의 **epoch 을 담지 않는다**
///   (to_id·to_name·msg_id·msg_uuid 만). 그래서 "정확히 어느 epoch 의 incarnation 이 받았나" 를 레코드
///   **만으로는** 단정할 수 없다 — 여기선 (i) 배달 성사(레코드 1건·is_delivered) (ii) 교체된 새 incarnation
///   의 캡처 버퍼에 바이트가 꽂혔고 구 버퍼엔 안 꽂힘 으로 **간접** 확인한다(레코드에 수신자 epoch 필드가
///   있으면 직접 단언 가능 — 관측-레코드 스키마 변경 = follow-up).
#[tokio::test]
async fn stage1_lifecycle_epoch_rotation_delivers_to_current_incarnation() {
    use engram_dashboard_core::agent::backend::InputEncoder;
    use engram_dashboard_core::agent::output_core::OutputCore;
    use engram_dashboard_core::agent::session::AgentSession;
    use engram_dashboard_core::agent::transport::AgentTransport;
    use engram_dashboard_core::agent::types::{
        AgentId as CoreAgentId, AgentStatus, BackendCaps, ControlCaps, InputCaps, InputEvent,
        ModelCaps, OutputCaps, PtyError, SessionCaps, StatusSink, TransportCaps,
    };
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, Entrance};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use std::sync::atomic::AtomicU8;

    // 로컬 seam transport — obs_seam 의 것과 동형이나 여기선 epoch 별로 **다른 캡처 버퍼**를 심어야
    //   incarnation 을 구분하므로 인라인으로 둔다(같은 AgentId, 다른 버퍼).
    struct NoopStatus;
    impl StatusSink for NoopStatus {
        fn status_changed(&self, _id: CoreAgentId, _s: AgentStatus, _e: u32) {}
        fn agent_list_updated(&self, _a: Vec<engram_dashboard_core::agent::types::AgentInfo>) {}
    }
    struct EpochSeam {
        captured: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl AgentTransport for EpochSeam {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
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
    fn insert_epoch(
        manager: &Arc<AgentManager>,
        id: CoreAgentId,
        epoch: u32,
    ) -> Arc<Mutex<Vec<Vec<u8>>>> {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let core = Arc::new(OutputCore::new(id, epoch, Arc::new(NoopStatus)));
        let session = Arc::new(AgentSession::new(
            id,
            std::path::PathBuf::from("."),
            epoch,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            backend_caps(),
            InputEncoder::ClaudeStreamJson,
            core,
            Box::new(EpochSeam {
                captured: captured.clone(),
            }),
        ));
        manager.insert_test_session(session);
        captured
    }

    let (manager, registry, _base, data_dir, handle) = wire("stage1-epoch").await;

    let id = CoreAgentId::new_v4();
    let to_name = obs_seam::fallback_name(id);

    // incarnation A(epoch 0) 주입 → 그 버퍼 old_buf.
    let old_buf = insert_epoch(&manager, id, 0);
    // incarnation B(epoch 1) 를 같은 AgentId 로 교체 주입(재시작=epoch bump 모사). insert_test_session 은
    //   같은 id 를 교체하므로 맵엔 이제 B 만 남는다(A 는 맵에서 빠진다).
    let new_buf = insert_epoch(&manager, id, 1);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "stage1-epoch-sender".to_string());
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let result = handle_send(
        &manager,
        &registry,
        Entrance::Cli,
        ControlCommand {
            from,
            to: to_name,
            body: "to-current-incarnation".to_string(),
        },
    );
    let v = result.to_json();
    // 배달은 성사돼야(안정 주소로 향함) — 유실이면 여기서 error 가 뜬다.
    assert_eq!(
        v["status"], "enqueued",
        "교체된 현재 incarnation 으로 배달돼야(유실 없음, ADR-0086 §F5): {v}"
    );

    // 레코드 1건 — wrong-epoch 이중배달 없음(같은 논리 메시지가 2건으로 안 남는다).
    let g = seen.lock().unwrap();
    assert_eq!(
        g.len(),
        1,
        "논리 메시지 1건 → 관측 레코드 1건(wrong-epoch 이중배달 없음): {:?}",
        *g
    );
    assert_eq!(g[0].to_id, id, "레코드 수신자 = 그 안정 AgentId");
    drop(g);

    // 바이트는 **현재(B, epoch 1)** incarnation 버퍼에만 꽂혀야 — 구(A) 버퍼엔 안 꽂힘.
    assert_eq!(
        new_buf.lock().unwrap().len(),
        1,
        "현재 incarnation(epoch 1) 이 바이트를 받아야"
    );
    assert!(
        old_buf.lock().unwrap().is_empty(),
        "교체된 구 incarnation(epoch 0) 은 바이트를 받지 않아야(wrong-epoch 배달 없음)"
    );
    assert!(
        String::from_utf8_lossy(&new_buf.lock().unwrap()[0]).contains("to-current-incarnation"),
        "현재 incarnation 버퍼에 봉투 본체가 온전히 담겨야"
    );

    manager.kill_agent(id).ok();
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
