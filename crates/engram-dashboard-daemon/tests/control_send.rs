//! ADR-0086 스텝 2 통합 테스트 — 듀얼 입구 A→B 메시지 전송(send_message MCP 툴 + /control/send HTTP 라우트).
//!
//! 실 DaemonControlChannel + AgentManager + MCP 서버를 배선하고 검증한다:
//!   - `/control/send`(CLI 입구): 무/오 토큰 → 401 · 유효 토큰 + 없는 수신자 → RECIPIENT_NOT_FOUND ·
//!     미등록 그룹(@) → GROUP_NOT_FOUND · 대용량 body → BODY_TOO_LARGE.
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
    start_mcp_server, ManagerSlot, McpServerHandle, MessagingSlot,
};
use engram_dashboard_daemon::control::registry::ControlRegistry;
use engram_dashboard_daemon::control::DaemonControlChannel;
use engram_dashboard_messaging::service::MessagingService;

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
    Arc<MessagingService>,
    // C2: idle 게이트 — c2 테스트가 수신자 core 에 턴 이벤트를 먹여 busy/idle 을 구동한다.
    Arc<engram_dashboard_messaging::busy::BusyPolicy>,
) {
    let registry = Arc::new(ControlRegistry::new());
    let slot = Arc::new(ManagerSlot::new());
    // C1: MessagingService 늦은 주입 슬롯 — MCP/CLI 입구·flush sink 가 공유(manager 조립 후 채운다).
    let messaging_slot = Arc::new(MessagingSlot::new());
    let handle = start_mcp_server(registry.clone(), slot.clone(), messaging_slot.clone())
        .await
        .expect("start mcp server");
    let url = handle.url.clone();
    let data_dir = std::env::temp_dir().join(format!("engram-send-{tag}-{}", AgentId::new_v4()));

    let control: Arc<dyn ControlChannel> = Arc::new(DaemonControlChannel::new(
        registry.clone(),
        url.clone(),
        data_dir.clone(),
        None, // send_exe: relay 테스트는 CLI 경로 불요(직접 HTTP/MCP 호출).
        // ADR-0092: 기존 relay 테스트는 프라이밍 무관 — Noop 으로 오늘 동작과 byte-identical.
        Arc::new(engram_dashboard_daemon::control::priming::NoopPrimingProvider),
    ));

    // ★C1: MessagingFlushSink 로 감싼 status sink★ — 로스터 등장/epoch bump 를 데몬측 diff 해 파킹 flush
    //   를 건다(파킹→스폰→자동배달 acceptance 의 트리거). 감싼 NoopSink 는 프론트 broadcast 를 안 하지만
    //   flush 로직엔 무관(로스터 diff 는 agent_list_updated 인자만 본다). messaging_slot 은 아래에서 set.
    // finding 5: sink 는 flush 대상만 채널로 push, 실제 flush 는 flush worker 가 수행한다(status 콜백 blocking
    //   분리). 테스트도 운영과 동일 배선 — worker 를 detached 로 띄운다(manager 가 sink 로 flush_tx 를 살려
    //   두므로 worker 는 manager 수명 동안 산다; 테스트 종료 시 프로세스와 함께 정리).
    let (flush_tx, flush_rx) =
        tokio::sync::mpsc::unbounded_channel::<engram_dashboard_daemon::ws::FlushMsg>();
    // C2 리뷰 fix 10: 운영 run() 과 동일하게 Idle coalescer 를 sink(턴 종료 push 중계)와 flush 레인이 공유한다.
    let idle_coalescer = Arc::new(engram_dashboard_daemon::ws::IdleCoalescer::new());
    let sink: Arc<dyn StatusSink> =
        Arc::new(engram_dashboard_daemon::ws::MessagingFlushSink::new_test(
            Box::new(NoopSink),
            flush_tx.clone(),
            idle_coalescer.clone(),
        ));
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
    // C2: idle 게이트(운영 run() 미러) — manager 조립 후에 만든다(코어 턴 관측 표를 든다).
    let idle_notifier = Arc::new(engram_dashboard_daemon::ws::ChannelIdleNotifier::new(
        flush_tx,
        idle_coalescer.clone(),
    ));
    let busy = Arc::new(
        engram_dashboard_daemon::messaging_host::busy_gate_for_manager(
            manager.clone(),
            idle_notifier.clone(),
        ),
    );
    // C1: MessagingService 조립 후 슬롯 주입(manager 를 감싼다) — 이제 send/flush 가 서비스에 닿는다.
    //   C2: idle 게이트를 함께 주입(busy 수신자 → 파킹) + flush 도어벨(fix 11 — 자가치유 배치 write 를
    //   발신 스레드에서 떼어 flush 레인으로 넘긴다). 운영 배선과 동일하게 유지한다.
    let messaging = Arc::new(
        engram_dashboard_daemon::messaging_host::messaging_for_manager_gated(
            manager.clone(),
            registry.clone(),
            busy.clone(),
        )
        .with_flush_trigger(idle_notifier),
    );
    messaging_slot.set(messaging.clone());
    // finding 5 + fix 3: 2-레인 flush worker(운영과 동일 배선) — 수신은 main lane, 배달은 flush 레인.
    //   round-3 finding 1: 조립은 `spawn_flush_worker` 단일 지점(두 레인 핸들 묶음). 이 하네스는 핸들을
    //   detach 한다(테스트 종료 = 프로세스 종료로 회수 — 운영 종료 경로는 lib.rs 가 belt 로 내린다).
    drop(engram_dashboard_daemon::ws::spawn_flush_worker(
        flush_rx,
        engram_dashboard_daemon::ws::FlushWiring {
            messaging: messaging_slot.clone(),
            idle: idle_coalescer,
        },
    ));

    // base URL(/mcp 벗김) — /control/send 요청에 쓴다.
    let base = url.strip_suffix("/mcp").unwrap_or(&url).to_string();
    (manager, registry, base, data_dir, handle, messaging, busy)
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
    let mut profile = AgentProfile::new(
        name.to_string(),
        AgentCommand::Claude {
            extra_args: vec![],
            output_format: ClaudeOutputFormat::StreamJson,
        },
        std::path::PathBuf::from("."),
        vec![],
        false,
    );
    // ADR-0101 (WYSIWYA): 라우팅/로스터가 쓰는 canonical name = display_name ?? cwd basename 이다
    //   (profile.name 은 더 이상 주소축 아님). 테스트가 이 `name` 으로 지목하므로 display_name 에 심어
    //   "보이는 이름 = 주소" 를 성립시킨다(cwd="." 의 basename 은 "." 이라 name 으로 매치 불가).
    profile.display_name = Some(name.to_string());
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
    let (_m, _r, base, data_dir, handle, _messaging, _busy) = wire("auth-missing").await;
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
    let (_m, _r, base, data_dir, handle, _messaging, _busy) = wire("auth-wrong").await;
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
    let (_m, registry, base, data_dir, handle, _messaging, _busy) = wire("corrective").await;
    // 유효 토큰(발신자 신원) — 아무 (id, epoch) 로 issue. 수신자 없음이라 relay 엔 안 간다.
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "valid-sender".to_string());

    // ★없는 수신자 → **수신자별 실패 행**(ADR-0111 결정 1 — `RECIPIENT_NOT_FOUND` 부활)★. 파킹하지 않고
    //   장부에 종점 행만 남긴다. **발송 단위 반려는 아니다** — 응답은 성공 shape(`{id, results}`)이고 그
    //   행 하나가 `failed` 다(부분 진행의 극단 = 전원 실패도 shape 유지, spec §5).
    let (status, body) = post_send(&base, Some("valid-sender"), "nobody", "hi").await;
    assert_eq!(status, reqwest::StatusCode::OK, "접수도 200 + JSON");
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert!(
        v.get("status").is_none(),
        "성공 응답엔 최상위 status 없음(spec §6): {body}"
    );
    assert_eq!(
        v["results"][0]["status"], "failed",
        "★ADR-0111 결정 1★ 없는 수신자는 **입구 반려 = 수신자별 실패 행**(옛 부재 파킹 폐지): {body}"
    );
    assert!(
        v["results"][0]["hint"].is_string(),
        "실패 행엔 code + hint 필수(spec §6)"
    );
    assert_eq!(v["results"][0]["code"], "RECIPIENT_NOT_FOUND", "{body}");

    // ★`@`주소 오류 = **주소 공간 오류 → 발송 단위 전체 반려**(ADR-0114 결정 3)★. 사용자 정의 그룹이
    //   제거됐으므로(ADR-0111 결정 4) `@all` 외의 `@이름`은 전부 `GROUP_NOT_FOUND` 다 — 이름 부재(행 실패)와
    //   **다른 층위**다: 발신자가 주소를 고쳐 다시 보내야 한다.
    let (_s, body) = post_send(&base, Some("valid-sender"), "@team", "hi").await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["code"], "GROUP_NOT_FOUND", "미등록 @ 주소: {body}");

    // 대용량 body(>64KiB) → BODY_TOO_LARGE.
    let big = "x".repeat(64 * 1024 + 1);
    let (_s, body) = post_send(&base, Some("valid-sender"), "nobody", &big).await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["code"], "BODY_TOO_LARGE", "대용량 body: {body}");

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── /control/send: shell(턴 신호 없음) 산 세션이 **로스터에 들어 배달된다** — ADR-0116 결정 1 ────────────
// ★이 테스트의 실제 범위 = 멤버십 한 축뿐★: 실제 spawn 한 shell(structured=false) 세션이 실물 어댑터 술어
//   (messaging_host `is_live`)를 통과해 배달까지 간다는 것.
// ★게이트 생략은 여기서 검증되지 않는다(뮤테이션 실측 — 착각 금지)★: 두 판정 지점에 게이트를 되살려도 이
//   테스트는 초록이다. shell 은 턴 신호가 없어 busy 가 항상 false 고 보관함도 비어, 게이트가 있어도 파킹으로
//   분기할 일이 없기 때문이다. 게이트 생략은 **반쪽 둘**이고 방어선은 전부 커널에 있다(`messaging/src/service.rs`):
//   busy 반쪽 = `a_live_agent_without_a_turn_signal_is_injected_with_no_gate` · 큐 백로그 반쪽 =
//   `inject_failure_parks_pending_without_a_turn_signal`. 어느 쪽을 지우든 이 통합 테스트가 대신 잡아주지 **않는다**.
#[tokio::test]
async fn control_send_shell_recipient_is_in_the_roster_and_delivered() {
    let (manager, registry, base, data_dir, handle, messaging, _busy) =
        wire("no-turn-signal").await;
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "valid-sender".to_string());

    // shell 에이전트(structured=false = **턴 신호 없음**) 스폰.
    let mut profile = AgentProfile::new(
        "sheller".to_string(),
        AgentCommand::Shell {
            program: engram_dashboard_core::agent::manager::default_shell().to_string(),
            args: vec![],
        },
        std::path::PathBuf::from("."),
        vec![],
        false,
    );
    // ADR-0101 (WYSIWYA): "sheller" 로 지목하므로 canonical name(display_name)에 심는다(cwd 는 ".").
    profile.display_name = Some("sheller".to_string());
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
        v["results"][0]["status"], "delivered",
        "★ADR-0116 결정 1★ 턴 신호가 없어도 산 세션은 로스터에 들어 배달된다: {body}"
    );
    assert!(
        v["results"][0]["code"].is_null(),
        "배달 행에는 실패 코드가 없어야: {body}"
    );
    assert_eq!(
        messaging.parked_len("sheller"),
        0,
        "배달됐으니 파킹 잔여도 없어야(게이트 생략 자체의 증거는 아니다 — 위 헤더): {body}"
    );

    manager.kill_agent(info.id).ok();
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── 로스터 술어 봉인: terminal 상태 세션은 **맵에 남아 있어도** 로스터에서 빠진다 — ADR-0116 결정 1 ──────
// ★왜 필요한가(뮤테이션 실측 — 리뷰 fix D9-a)★: `messaging_host::is_live` 의 상태 조건을 지워도 데몬 412
//   테스트가 전부 초록이었다. 4차 개정으로 그 조건이 **유일한 멤버십 게이트**가 됐으므로(capability 는 타이밍
//   축으로 내려갔다) 실물 어댑터 레벨에서 봉인한다. "list_agents 에 있음 ≠ 로스터" 가 이 테스트의 주제다.
#[tokio::test]
async fn roster_excludes_a_terminal_session_still_in_the_map() {
    use engram_dashboard_messaging::service::DeliveryPort;

    let (manager, _registry, _base, data_dir, handle, messaging, _busy) = wire("dead-roster").await;
    let dead = obs_seam::insert_terminal_seam_recipient(&manager, "corpse");
    let port = engram_dashboard_daemon::messaging_host::ManagerDeliveryPort::new(manager.clone());

    // 전제: 맵엔 **남아 있다**(reaper 가 수거하지 않는 주입 세션) — 그런데 상태는 terminal 이다.
    assert!(
        manager.list_agents().iter().any(|a| a.id == dead),
        "주입 세션은 맵에 남아 있어야(이 테스트의 전제)"
    );
    assert!(
        !matches!(
            manager
                .list_agents()
                .into_iter()
                .find(|a| a.id == dead)
                .expect("맵에 있다")
                .status,
            AgentStatus::Running | AgentStatus::Exiting
        ),
        "상태는 terminal 이어야(이 테스트의 전제)"
    );

    assert!(
        !port.live_agents().iter().any(|a| a.id == dead),
        "★D9-a★ terminal 세션이 로스터에 섞였다(상태 술어가 지워졌다)"
    );
    let sources = port.addressing_sources();
    assert!(
        !sources.roster.iter().any(|a| a.id == dead),
        "입구 판정 소스도 같은 술어여야: {sources:?}"
    );
    assert!(
        !port.is_agent_live(dead),
        "삭제 정리 게이트도 같은 술어여야(시체를 산 것으로 보면 정리가 영원히 안 돈다)"
    );
    // 그 이름으로 보내면 배달 시도가 아니라 입구 반려다(프로필도 없으므로 `RECIPIENT_NOT_FOUND`).
    let rows = messaging
        .handle_send(
            "m-dead",
            engram_dashboard_messaging::SenderIdentity {
                peer_id: AgentId::new_v4(),
                epoch: 0,
            },
            "outsider",
            &["corpse".to_string()],
            "hi",
            engram_dashboard_messaging::envelope::Entrance::Cli,
            &engram_dashboard_messaging::service::SendMeta::default(),
        )
        .expect("행 응답");
    assert_eq!(
        rows[0].code,
        Some(engram_dashboard_messaging::service::FailCode::RecipientNotFound),
        "시체는 수신자가 아니다: {rows:?}"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── relay happy path: json 에이전트에 보내면 래핑된 라인이 stdin 입력 에코로 관측된다 ────────────────
// 실 claude(stream-json) 스폰 + write_input 동기 입력 에코 관측(claude 왕복 이전이라 결정적).
#[tokio::test]
async fn control_send_relays_wrapped_line_to_json_agent() {
    let (manager, registry, base, data_dir, handle, _messaging, _busy) = wire("relay").await;

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
    // spec §6: 성공 = `{ id, results: [{to, status:"delivered"}] }`(산 수신자 = 즉시 배달).
    assert_eq!(v["results"][0]["status"], "delivered", "배달 성공: {body}");
    assert_eq!(v["results"][0]["to"], "bee", "해석된 수신자 이름 동봉");
    assert!(v["id"].is_string(), "msg-id 동봉");

    // 래핑된 라인이 B 의 입력 에코로 관측돼야 한다. ADR-0103: 운영 기본 봉투 = **xml**
    //   (`<message from="{sender}">{body}</message>`). 발신자는 profile 부재라 표시이름 = sender id 앞
    //   8자(sender_display_name fallback)로 결정적이다.
    // ★정확 일치 단언(느슨한 substring 금지 — 리뷰 지적)★: 관측 sink 가 잡는 `j` 는 유저 에코 이벤트의
    //   **전체 JSON**(`{"type":"text","text":"<봉투>","uuid":"X"}`)이라 봉투는 그 안 `text` 필드 값으로
    //   박혀 있다. 예전엔 raw 라인에 substring `contains` 를 썼는데, 그러면 `</message>` 뒤에 잘림·오염이
    //   덧붙어도 통과했다(트레일링 corruption 미탐). 그래서 `j` 를 JSON 파싱해 `text` 필드를 뽑아 기대 봉투
    //   문자열과 **정확(==) 비교**한다 — 프레이밍(uuid 등)은 필드 밖이라 무관하고 봉투 자체의 온전성만 본다.
    let sender_display = &sender.to_string()[..8];
    let expected_envelope = obs_seam::expected_default_envelope(sender_display, "ping-body-XYZ");
    let observed = wait_until(Duration::from_secs(3), || {
        seen.lock().unwrap().iter().any(|j| {
            serde_json::from_str::<serde_json::Value>(j)
                .ok()
                .and_then(|v| v["text"].as_str().map(|t| t == expected_envelope))
                .unwrap_or(false)
        })
    });
    assert!(
        observed,
        "relay 가 래핑된 라인을 B stdin 에 주입(입력 에코의 text 필드 = 기대 봉투 정확 일치): expect={:?} seen={:?}",
        expected_envelope,
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
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("revoked-delivers").await;

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
        to: vec!["target-b".to_string()],
        body: "revoked-but-DELIVERED".to_string(),
        contract: Default::default(),
    };
    let result = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "폐기 발신자여도 배달됨(생존은 게이트 아님, 사용자 결정): {v}"
    );
    assert_eq!(v["results"][0]["to"], "target-b", "해석된 수신자 이름 동봉");
    assert!(v["id"].is_string(), "msg-id 동봉");

    // 배달됨 — 래핑 라인이 B 입력 에코로 관측돼야 한다(폐기 발신자여도 relay 진행).
    //   ADR-0103: 운영 기본 봉투 = **xml**(`<message from="{sender}">{body}</message>`). 발신자는 profile
    //   부재라 표시이름 = sender id 앞 8자로 결정적이다.
    // ★앵커 단언(느슨한 substring 금지)★: `j` = 유저 에코 전체 JSON(`{"type":"text","text":"<봉투>",…}`)
    //   이라 봉투 안 `"` 는 JSON 인코딩으로 `\"` 다 — `"text":"<message from=\"발신자\">` 로 봉투 **시작에
    //   발신자를 핀**한다(발신자를 덧댄 잘못된 렌더는 이 앵커를 통과 못 한다).
    let sender_display = &sender.to_string()[..8];
    let anchored_envelope =
        format!(r#""text":"<message from=\"{sender_display}\">revoked-but-DELIVERED</message>"#);
    let delivered = wait_until(Duration::from_secs(3), || {
        seen.lock()
            .unwrap()
            .iter()
            .any(|j| j.contains(&anchored_envelope))
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
    seen: Arc<Mutex<Vec<engram_dashboard_messaging::envelope::DeliveryObservation>>>,
}
impl engram_dashboard_messaging::envelope::DeliveryObserver for DeliveryCapture {
    fn observe(&self, obs: engram_dashboard_messaging::envelope::DeliveryObservation) {
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
    use engram_dashboard_core::agent::output_core::{OutputCore, TurnWiring};
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
        // 기본은 "이름 = id 앞 8자"(fallback_name) — 대부분의 테스트가 유일 이름을 전제한다.
        let id = AgentId::new_v4();
        let name = id.to_string()[..8].to_string();
        insert_seam_recipient_named(manager, fail, id, &name)
    }

    /// ★terminal 상태인데 **맵에 남아 있는** 세션을 주입한다(리뷰 fix D9-a)★ — 로스터 술어의 상태 조건을
    /// 결정적으로 봉인하기 위한 seam.
    ///
    /// ★왜 실 종료로는 못 만드나★: 실제 종료는 reaper 가 세션을 맵에서 **곧바로 제거**한다(시체 보존은
    ///   프로필 축이다 — reaper.rs). 그래서 "list_agents 엔 있는데 상태는 terminal" 이라는 상태를 실 세션으로
    ///   재현할 수 없다. 주입 세션은 pump 가 없어 ReapMsg 가 나가지 않으므로 그 상태로 남는다
    ///   (`insert_test_session` doc 의 안전 불변식 (a)).
    /// ★왜 이 술어가 중요한가★: 상태 조건을 지우면 시체가 로스터에 섞여 ① 그 이름 앞 발송이 배달 시도로 가고
    ///   ② 프로필이 남은 이름이 잠듦 파킹으로 내려가지 못한다(입구 3분기가 통째로 흔들린다).
    pub fn insert_terminal_seam_recipient(manager: &Arc<AgentManager>, name: &str) -> AgentId {
        let id = AgentId::new_v4();
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        // ★종점 전이(pump 단독 소유 — ADR-0005)를 직접 부른다★: 이 세션엔 pump 가 없어 finalize 경쟁자가
        //   없다. 결과 = 맵에 남은 채 상태만 terminal(Killed).
        core.finish(engram_dashboard_core::agent::types::TerminalReason::Killed);
        let session = Arc::new(AgentSession::new(
            id,
            std::path::PathBuf::from(format!("seam-root/{name}")),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            backend_caps(),
            InputEncoder::ClaudeStreamJson,
            core,
            Box::new(SeamTransport {
                fail: false,
                captured: Arc::new(Mutex::new(Vec::new())),
            }),
        ));
        manager.insert_test_session(session);
        id
    }

    /// ★동명 다수 시나리오용(D 리뷰 B1)★ — AgentId 와 **보이는 이름을 따로** 지정한다. 두 세션에 같은
    /// 이름을 주면 로스터에 동명 두 명이 뜨고, 그때 발신자는 exact AgentId 로만 한쪽을 지목할 수 있다.
    pub fn insert_seam_recipient_named(
        manager: &Arc<AgentManager>,
        fail: bool,
        id: AgentId,
        name: &str,
    ) -> (AgentId, Arc<Mutex<Vec<Vec<u8>>>>) {
        // 관측 배선 없는 core — 대부분의 테스트는 idle 게이트를 타지 않아야 한다(주입이 만드는 유저 에코가
        //   그 수신자를 turn-중으로 만들면 다음 발송이 파킹돼 그 테스트들의 전제가 바뀐다).
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        let (agent, captured, _core) = insert_seam_with_core(manager, fail, id, name, core);
        (agent, captured)
    }

    /// ★관측 배선된 seam 수신자(ADR-0113)★ — core 를 매니저의 **통지 경로·턴 관측 표**에 이어 꽂고 그
    ///   core 를 함께 돌려준다. 호출자가 `core.emit(...)` 으로 턴 이벤트를 먹이면 운영과 **같은 경로**
    ///   (분류 → 표 → 턴 종료 push → 도어벨)를 탄다.
    pub fn insert_observed_seam_recipient(
        manager: &Arc<AgentManager>,
        fail: bool,
    ) -> (AgentId, Arc<Mutex<Vec<Vec<u8>>>>, Arc<OutputCore>) {
        let id = AgentId::new_v4();
        let name = id.to_string()[..8].to_string();
        // 운영 spawn 과 같은 dispatch 로 분류자를 뽑는다 — seam 세션은 json 모드 claude 캐리어를 흉내낸다.
        let core = manager.wired_test_core(
            id,
            0,
            engram_dashboard_core::agent::backend::turn_classifier(
                &engram_dashboard_core::agent::profile::AgentCommand::Claude {
                    extra_args: vec![],
                    output_format:
                        engram_dashboard_core::agent::profile::ClaudeOutputFormat::StreamJson,
                },
            ),
        );
        insert_seam_with_core(manager, fail, id, &name, core)
    }

    fn insert_seam_with_core(
        manager: &Arc<AgentManager>,
        fail: bool,
        id: AgentId,
        name: &str,
        core: Arc<OutputCore>,
    ) -> (AgentId, Arc<Mutex<Vec<Vec<u8>>>>, Arc<OutputCore>) {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let core_out = core.clone();
        // ADR-0101 (WYSIWYA): canonical name = display_name ?? basename(session.cwd) 이고 seam 세션엔
        //   프로필(=display_name)이 없다. 그래서 cwd 의 basename 이 곧 이 세션의 addressable name 이 된다.
        //   테스트는 fallback_name(id)=id[:8] 로 지목하므로, cwd basename 을 id[:8] 로 맞춰 "보이는 이름
        //   = 주소" 를 성립시킨다(옛 cwd="." 는 basename="." 이라 지목 불가·동명 충돌).
        let cwd = std::path::PathBuf::from(format!("seam-root/{name}"));
        // ClaudeStreamJson encoder — json 모드 캐리어를 흉내(래핑된 봉투가 stream-json 라인으로 감싸짐).
        //   요청 바이트(WriteOutcome.bytes_requested)는 감싸기 **전** 논리 메시지 = wrap_message 봉투 그대로다.
        let session = Arc::new(AgentSession::new(
            id,
            cwd,
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
        (id, captured, core_out)
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

    /// ★기대 봉투 재구성(ADR-0103 — 운영 기본 = xml)★: 데몬이 기본 포맷으로 감싸는 봉투를 테스트가
    ///   바이트-정확 회계하려고 재구성한다. 기본 = plain `<message from="{sender}">{body}</message>`
    ///   (속성 없음 — 현 스코프 increment A). ★주의★: 이 헬퍼는 XML 이스케이프를 하지 않으므로 sender/body
    ///   에 `<`/`>`/`&`/`"` 가 없는 테스트 픽스처에서만 데몬 렌더와 바이트 일치한다(현 호출부 전부 해당).
    pub fn expected_default_envelope(sender: &str, body: &str) -> String {
        format!("<message from=\"{sender}\">{body}</message>")
    }
}

// ── ADR-0088(FIX-4): 배달 관측 core 단언을 claude 없이 — seam 수신자에 성공 relay ──────────────
// 위 e2e 테스트가 claude 부재 시 skip 되는 것과 달리, 이 테스트는 seam 으로 structured 수신자를 꽂아
//   **항상** 관측 레코드(요청/실제 바이트·msg_id↔msg_uuid 상관·is_delivered)를 단언한다(green-when-skipped 제거).
#[tokio::test]
async fn control_send_delivery_observation_via_seam_no_claude() {
    use engram_dashboard_core::agent::backend::InputEncoder;
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("obs-seam-ok").await;

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
        to: vec![to_name.clone()],
        body: body.to_string(),
        contract: Default::default(),
    };
    let result = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "seam 성공 배달 ACK: {v}"
    );
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
    //   기대치를 정확히 계산한다(발신자 표시이름 = sender id 앞8자 fallback).
    // ADR-0103: 운영 기본 봉투 = xml(`<message from="{sender}">{body}</message>`) — msg_id 는 봉투에
    //   심기지 않는다(봉투에서 uuid 제거, ADR-0095 거부 대안). 그래서 재구성도 xml plain 이다.
    let sender_name = obs_seam::fallback_name(sender);
    let expected_wrapped = obs_seam::expected_default_envelope(&sender_name, body);
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
    assert_eq!(obs.from, from.into(), "레코드 발신자 신원(토큰 파생)");

    // ★계층 관통(exact bytes)★: 세션이 실제 받은 write 바이트 = encoder(봉투, msg_uuid). XML 봉투의 `"` 는
    //   stream-json JSON 인코딩에서 `\"` 로 이스케이프되므로 raw 봉투 문자열 substring 비교는 성립하지
    //   않는다 — 그래서 encoder 로 기대 라인을 재구성해 바이트-정확 일치를 단언한다(멀티바이트 본체 온전성
    //   + handoff 잘림/오염 탐지). msg_uuid 는 성공 레코드라 항상 Some.
    let written = obs_seam::last_written(&captured);
    let msg_uuid = obs.msg_uuid.expect("성공 배달 msg_uuid");
    let expected_line =
        InputEncoder::ClaudeStreamJson.encode(expected_wrapped.as_bytes(), msg_uuid);
    assert_eq!(
        written,
        expected_line,
        "세션이 받은 stream-json 라인이 기대 encoded 봉투와 바이트-정확 일치해야: {}",
        String::from_utf8_lossy(&written)
    );

    manager.kill_agent(b_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── ADR-0088 확장(리뷰 F1/F2): DeliveryObservation.in_reply_to — 구조화 메타에서만 파생 ───────────
// ★고쳐진 결함(F1)★: 옛 구현은 렌더된 봉투 문자열을 `in-reply-to="` 로 substring 탐색해 이 필드를
//   파생했는데, 본문 이스케이프(`escape_xml_text`)가 따옴표를 이스케이프하지 않아 발신자가 본문에
//   그 속성 문자열을 흉내 내 넣으면 관측이 위조됐다(재현됨). 고친 구현은 `SendMeta.reply_to`(ingress
//   `validate_contract` 가 이미 검증한 발신 인자)를 `observe_success`/`observe_failure` 에 파라미터로
//   그대로 넘긴다 — 봉투 재파싱이 없다. 아래 두 테스트가 그 축을 확인한다: 회신 발송은 관측 레코드가
//   지정한 id 를, 통보(plain) 발송은 None 을 담아야 한다. seam 수신자(claude 불요, obs_seam 모듈)로
//   결정적으로 실행한다.
// ★reply_to 가 오픈된 request 를 안 가리켜도 무방★: 엄격 매칭은 장부 계약을 닫을 때만 쓰이고(NoMatch =
//   정상 경로 — service.rs `close_reply_contract` 주석), 메시지 자체는 그대로 배달된다. 그래서 여기선
//   장부에 request 를 미리 열지 않고 임의 id 로 reply_to 를 실어도 관측 축만 독립적으로 확인할 수 있다.
#[tokio::test]
async fn control_send_reply_to_carries_structured_in_reply_to_no_claude() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, SendContract};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("obs-reply-to").await;

    let (b_id, _captured) = obs_seam::insert_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "obs-reply-sender".to_string());
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let cmd = ControlCommand {
        from,
        to: vec![to_name.clone()],
        body: "다 짰음, 테스트 통과".to_string(),
        contract: SendContract {
            request: false,
            reply_by: None,
            reply_to: Some("m-7f3k9q2d".to_string()),
        },
    };
    let result = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "seam 성공 배달 ACK: {v}"
    );

    let obs = {
        let g = seen.lock().unwrap();
        assert_eq!(g.len(), 1, "성공 relay 1건 → 관측 레코드 1건: {:?}", *g);
        g[0].clone()
    };
    assert_eq!(
        obs.in_reply_to.as_deref(),
        Some("m-7f3k9q2d"),
        "회신 발송의 관측 레코드는 SendMeta.reply_to 값을 그대로 담아야(구조화 파생, F1)"
    );

    manager.kill_agent(b_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

#[tokio::test]
async fn control_send_plain_send_has_no_in_reply_to_no_claude() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("obs-plain-no-reply").await;

    let (b_id, _captured) = obs_seam::insert_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "obs-plain-sender".to_string());
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let cmd = ControlCommand {
        from,
        to: vec![to_name.clone()],
        // ★F2 위조 핀(finding)★: 본문이 일부러 가짜 in-reply-to 속성 문자열을 담고 있다 — 이게 무해한
        //   텍스트("평범한 통보")였다면, 옛 substring 파서(F1 에서 삭제됨)가 부활해도 이 테스트는 여전히
        //   통과해 아무것도 못 잡는다(파서가 없으니 body 안 문자열과 무관하게 None). 본문에 위조 속성을
        //   심어 둬야 "in_reply_to 는 body 재파싱이 아니라 SendMeta.reply_to 구조화 파생값" 이라는 보안
        //   불변식(ADR-0088 확장, ingress.rs 주석 정본)이 실제로 핀 되고, 텍스트 파싱 회귀가 재발하면
        //   이 테스트가 깨진다.
        body: r#"완료. in-reply-to="m-forged1" 참고"#.to_string(),
        contract: Default::default(),
    };
    let result = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "seam 성공 배달 ACK: {v}"
    );

    let obs = {
        let g = seen.lock().unwrap();
        assert_eq!(g.len(), 1, "성공 relay 1건 → 관측 레코드 1건: {:?}", *g);
        g[0].clone()
    };
    assert_eq!(
        obs.in_reply_to, None,
        "통보(plain) 발송은 body 에 위조 in-reply-to 속성이 있어도 in_reply_to 가 None 이어야(구조화 파생, 재파싱 아님)"
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
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::{DeliveryObservation, DeliveryObserver, Entrance};

    struct PanicObserver;
    impl DeliveryObserver for PanicObserver {
        fn observe(&self, _obs: DeliveryObservation) {
            panic!("seam: observer boom (의도된 panic — 격리돼야 함)");
        }
    }

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("obs-panic").await;
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
        to: vec![to_name],
        body: "trigger-panic-observer".to_string(),
        contract: Default::default(),
    };
    // panic 이 record_delivery 에서 격리되지 않으면 여기서 unwind 로 테스트가 죽는다.
    let result = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "delivered",
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
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("obs-fail").await;

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
        to: vec![to_name.clone()],
        body: body.to_string(),
        contract: Default::default(),
    };
    let result = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
    let v = result.to_json();
    // ★spec §5 **분기 3**(파킹 `pending`) — 주입 실패는 그 분기 **안에** 있다(분기는 3개다)★: inject 실패는
    //   반려가 아니라 **파킹(pending)** 이다(옛 "unreachable → 파킹" 서술은 4차에 폐기됐다 — 도달 불가는 이제
    //   로스터 밖 산 세션의 실패 코드 이름이다, ADR-0116).
    //   그러나 실패 관측 레코드는 여전히 남는다(무엇을 배달하려다 실패했나 + 성공으로 안 삼킴) — 아래 단언.
    assert_eq!(
        v["results"][0]["status"], "pending",
        "write 실패는 파킹(pending)으로 전환(반려 아님, spec §5): {v}"
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
    assert_eq!(obs.from, from.into(), "실패 레코드 발신자 신원");

    manager.kill_agent(b_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

#[tokio::test]
async fn control_send_delivery_observation_records_bytes_and_correlated_ids() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("delivery-obs").await;

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
        to: vec!["obs-target".to_string()],
        body: body.to_string(),
        contract: Default::default(),
    };
    let result = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
    let v = result.to_json();
    assert_eq!(v["results"][0]["status"], "delivered", "배달 성공 ACK: {v}");
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
    //     profile 부재라 sender id 앞8자 fallback.
    //     ADR-0103: 운영 기본 봉투 = xml(`<message from="{sender}">{body}</message>`) — 봉투에 msg_id 미포함.
    //     bytes_written 은 by-construction 복사(short-write 탐지 아님 — 완결성은 error None 으로 본다).
    let sender_name = sender.to_string()[..8].to_string();
    let expected_wrapped = obs_seam::expected_default_envelope(&sender_name, body);
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
    assert_eq!(obs.from, from.into(), "레코드 발신자 신원(토큰 파생)");

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
/// ★증명하지 않는다(커버리지 공백)★: **물리 OS-pipe 바이트 무인터리브**. 그 직렬화는
///   운영 StdioTransport 의 `stdin.lock()`(write_all+flush 내내 보유, stdio.rs ~322)이 담당하는데
///   이 seam 은 그 계층을 **우회**한다(SeamTransport 는 완결 Vec 을 원자 push). 그 응용계층 직렬화를
///   지우는 회귀는 여기서 **안 잡힌다**.
///   ▶ follow-up 존재: 실 StdioTransport+실 pipe(느린 reader/backpressure) 하네스가 이 물리 계층을
///     커버한다 — core 크레이트 `tests/stdio_physical_pipe.rs` ::
///     `physical_pipe_concurrent_sends_no_interleave`(런렝스로 무인터리브 단언). 단 그 테스트가 잡는
///     회귀 형태는 **"한 논리 메시지를 배타 락 없이 여러 OS write 로 쓰는 것"(응용계층 직렬화 회귀)**
///     로 한정된다 — 락 없는 단일-WriteFile 구현은 NPFS 커널 직렬화(문서화 안 됨) 가능성 때문에
///     잡는다고 주장하지 않는다(그 테스트 docstring 의 "증명하지 않는다" 참조).
#[tokio::test]
async fn stage1_concurrent_sends_exact_once_distinct_bodies_intact_at_seam() {
    use engram_dashboard_core::agent::backend::InputEncoder;
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;
    use std::sync::Barrier;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("stage1-concurrency").await;

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
        let messaging = messaging.clone();
        let to = to_name.clone();
        let body = marker.clone();
        let barrier = barrier.clone();
        // handle_send 는 sync(&Arc<..>) — OS 스레드로 near-simultaneous 발화(tokio task 아님, 병렬성 확보).
        handles.push(std::thread::spawn(move || {
            barrier.wait(); // ★입구 정렬 — 모든 스레드가 여기 모인 뒤 near-simultaneous 하게 handle_send 로 돌진(실행 겹침 강제는 아님)★.
            let cmd = ControlCommand {
                from,
                to: vec![to],
                body,
                contract: Default::default(),
            };
            let result = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
            let v = result.to_json();
            // 각 발화는 접수 성공 + 고유 msg_id 를 받아야(중복/유실 없음의 발신측 증거). 행 상태는 보지
            //   않는다 — 겹친 드레인에서 물러난 쪽은 `pending` 이 정상이다(아래 주석).
            assert!(
                v.get("results").is_some(),
                "동시 발화도 각기 접수(results): {v}"
            );
            v["id"].as_str().expect("msg-id").to_string()
        }));
    }
    let ack_ids: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    // ★동시 버스트의 결말 = `delivered`/`pending` **혼합**이다(7차 · ADR-0125)★: 모든 발송이 큐 꼬리에
    //   적재된 뒤 **자기 호출 안에서** 그 수신자 큐를 드레인한다. 여러 발신이 겹치면 먼저 배치를 든 쪽이
    //   영수증(in-flight)을 쥐고, 뒤따른 드레인은 **중복 진입 가드**에 걸려 물러나며 유예 표식만 남긴다
    //   (진행 중 배치를 앞지르면 배달 순서가 적재 순서에서 풀리므로). 물러난 쪽은 자기 편지의 주입 여부를
    //   모르니 응답을 `pending` 으로 답하지만(spec §6 ㉯ — "안 갔다" 가 아니다) 그 편지는 **이긴 쪽 배치에
    //   실려 나가거나**, 영수증 보유자가 정산하며 되울린 도어벨 → flush 레인이 집어 간다.
    //   ★그래서 여기 단언 대상은 차수와 무관하게 그대로다★: 유실·중복 없음(고유 msg_id N개) · 봉투 바이트
    //   무결(multiset 일치). 아래에서 기다리는 것은 그 파이프라인의 정지(quiescence)뿐이고, 운영 경로
    //   (동기 드레인 → 물러남 → 되울린 도어벨 → 레인 → `flush_for_agent`)를 그대로 태우므로 물러난 몫이
    //   실제로 배달되는지도 함께 실증된다. ★위 "산 수신자라 delivered" 주석을 근거로 전원 `delivered` 를
    //   단언으로 승격시키지 말 것★ — 겹친 드레인에서 물러난 쪽은 정상적으로 `pending` 이다.
    for _ in 0..600 {
        if seen.lock().unwrap().len() >= N && messaging.parked_len(&to_name) == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

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
        &engram_dashboard_messaging::envelope::DeliveryObservation,
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
        //   ADR-0103: 운영 기본 봉투 = xml(`<message from="{sender}">{body}</message>`) — 봉투에 msg_id 미포함.
        //   (obs 는 위 msg_uuid→레코드 매칭·유령 write 검증에 여전히 쓰인다 — msg_id 만 봉투에서 빠짐.)
        //   body 마커는 XML 특수문자를 안 쓰므로(BODY-NNNN) 이스케이프 없이 데몬 렌더와 바이트 일치.
        let wrapped = obs_seam::expected_default_envelope(&sender_name, body);
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
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    const MAX: usize = 64 * 1024; // = MAX_BODY_BYTES(ingress 상수 — 여기 미러; 값 드리프트 시 아래가 잡는다).

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("stage1-boundary").await;

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
        messaging: &Arc<MessagingService>,
        from: BoundIdentity,
        body: String,
    ) -> (
        serde_json::Value,
        Option<engram_dashboard_messaging::envelope::DeliveryObservation>,
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
            to: vec![to_name.clone()],
            body: body.clone(),
            contract: Default::default(),
        };
        let result = handle_send(manager, registry, messaging, Entrance::Cli, cmd);
        let v = result.to_json();
        let obs = seen.lock().unwrap().first().cloned();
        let written = obs_seam::last_written(&captured);
        // ADR-0103: 운영 기본 봉투 = xml(`<message from="{sender}">{body}</message>`) — 봉투에 msg_id 미포함.
        //   기대 봉투 바이트 = xml 봉투의 UTF-8 len. body 는 XML 특수문자 없는 픽스처라 이스케이프 무영향.
        //   ★MAX 게이트는 body 기준★: 64 KiB 는 body 길이라 게이트를 통과하고, 봉투 wrapper 는 그 위에 얹힌다.
        let sender_name = obs_seam::fallback_name(from.agent_id);
        let expected_env_bytes = obs_seam::expected_default_envelope(&sender_name, &body).len();
        // ★기대 encoded 라인 재구성★: 성공 시 봉투를 관측 레코드의 msg_uuid 로 재-encode 한다(= session 이
        //   실제 send_input 에 넘긴 바이트). msg_uuid 가 있어야 encode 하므로 성공 레코드에서만 만든다.
        let expected_line = match obs.as_ref().and_then(|o| o.msg_uuid) {
            Some(uuid) => {
                let wrapped = obs_seam::expected_default_envelope(&sender_name, &body);
                InputEncoder::ClaudeStreamJson.encode(wrapped.as_bytes(), uuid)
            }
            None => Vec::new(),
        };
        (v, obs, written, expected_env_bytes, expected_line, b_id)
    }

    // ── (1) 정확히 64 KiB(경계 포함) → 배달됨 ──────────────────────────────────────────────
    let body_eq = "x".repeat(MAX);
    assert_eq!(body_eq.len(), MAX, "테스트 전제: 정확히 64 KiB");
    let (v, obs, written, env_bytes, expected_line, b_id) =
        send_once(&manager, &registry, &messaging, from, body_eq.clone()).await;
    assert_eq!(
        v["results"][0]["status"], "delivered",
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
        send_once(&manager, &registry, &messaging, from, body_lt).await;
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "64 KiB−1 은 배달돼야: {v}"
    );
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
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![to_name],
            body: body_gt,
            contract: Default::default(),
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
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![to_name],
            body: body_mb_over,
            contract: Default::default(),
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
        send_once(&manager, &registry, &messaging, from, body_mb_ok).await;
    assert_eq!(
        v["results"][0]["status"], "delivered",
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

/// ── ADR-0088 Stage 1-오라클 3(a) → ADR-0111 갱신: 수신자 부재 → **실패 행** + 배달 관측 없음 ──────────
/// ★ADR-0111 결정 1★: 해석 시점 수신자 부재는 **수신자별 실패 행**(`failed` + `RECIPIENT_NOT_FOUND`)이다 —
///   파킹하지 않는다("없는 이름 파킹" = 스폰 전 선지시는 v1 비지원). 배달 관측 레코드는 **0** 이다(주입이
///   없으므로 유령 배달도 없다) — 관측은 실제 inject 에서만 생긴다. 이 두 성질을 함께 못 박는다.
#[tokio::test]
async fn stage1_lifecycle_recipient_absent_is_a_failed_row_with_no_observation() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("stage1-absent").await;

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
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec!["no-such-agent".to_string()],
            body: "hi".to_string(),
            contract: Default::default(),
        },
    );
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "failed",
        "★ADR-0111 결정 1★ 부재 수신자는 실패 행: {v}"
    );
    assert!(
        seen.lock().unwrap().is_empty(),
        "파킹은 주입 안 하므로 배달 관측 레코드 없음(유령 배달 없음)"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── S18 메시징 v1 수용 시나리오(spec §7): **busy 파킹 → 턴 종료 → 자동 배달**(실 claude) ──────────────
// ★acceptance(spec §7)★: 실 에이전트가 **턴 진행 중**일 때 보내면 `pending`(파킹) → 그 턴이 끝나면 idle
//   트리거가 flush 를 걸어 파킹분이 **자동 배달**된다. 배달을 DeliveryObserver 로 관측한다(from=발신자·
//   to=그 이름). claude(stream-json) 스폰 필요 — 없으면 loud skip.
// ★★이 테스트는 더 이상 로스터 등장 diff 를 태우지 않는다(ADR-0111 결정 1 · 본문 안 주석이 정본)★★:
//   옛 판은 "부재 파킹 → 스폰" 이었고 부재 파킹이 폐지돼 그 진입로가 없다. 지금 남은 축은 "실 claude 를
//   상대로도 idle→flush 배선이 돈다" 이고, **등장 diff(MessagingFlushSink) 축은 현재 미커버**다(백로그).
// ★multi_thread 런타임(이 테스트 고유 사유 — round-4 finding 1)★: flush 는 별도 flush worker(tokio task)가
//   수행한다(콜백 blocking 분리). worker 는 이제 inject 를 spawn_blocking 으로 던져 runtime worker 를 굶기지
//   않으므로 executor 굶주림은 해소됐다 — 그래도 이 테스트는 multi_thread 가 필요하다. 이유는 worker 가 아니라
//   **이 테스트 본문**이다: 아래 `wait_until` 은 std::thread::sleep 로 블록하는 **동기** 폴링이라, current-thread
//   런타임에선 이 test task 가 그 스레드를 붙잡고 sleep 을 도는 동안 worker task(recv().await·JoinHandle await)
//   가 폴링될 틈이 없어 flush 가 진행되지 않는다. multi_thread 로 worker 를 다른 런타임 스레드에서 돌려
//   test 본문의 동기 대기와 병렬로 진행시킨다. (wait_until 을 async tokio::time 폴링으로 바꾸면 default 런타임도
//   가능하나, 이 헬퍼는 다수 동기 테스트가 공유하므로 여기선 런타임 flavor 로만 격리 — 최소 변경.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c1_park_then_spawn_auto_delivers() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, busy) = wire("c1-park-spawn").await;

    // 배달 관측 싱크 — flush 자동 배달을 여기로 회수(로그 스크레이핑 없이).
    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    // 발신자 신원(유효 토큰).
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c1-sender".to_string());
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    // ── 1) 실 에이전트를 먼저 띄운다 ────────────────────────────────────────────────────
    // ★ADR-0111 결정 1 로 setup 이 바뀌었다★: 옛 판은 "아직 안 뜬 이름으로 보내 파킹 → 나중에 스폰" 이었다.
    //   부재는 이제 **입구 반려(실패 행)** 라 그 진입로가 없어졌다. 이 테스트가 지키는 건 부재가 아니라
    //   **"파킹된 메일이 flush 계기에 자동으로 배달된다"** 는 배선이므로, 파킹 사유를 spec §5 가 남긴 유일한
    //   경로(busy = 턴 진행 중)로 바꾼다 — 관측 대상(등장/idle diff → flush 워커 → 주입 → 관측)은 그대로다.
    // ★미커버 축의 정직한 표기(리뷰 C1)★: 이 테스트의 setup 이 busy→idle 로 바뀌면서, **로스터 등장
    //   diff → flush**(MessagingFlushSink 가 agent_list_updated 를 보고 그 이름 큐를 여는 배선) 축은 지금
    //   **어떤 테스트도 덮지 않는다**(커널 테스트는 `flush_for` 를 직접 부른다). 옛 판은 "부재 파킹 → 스폰"
    //   으로 그 축을 태웠지만 부재 파킹이 폐지돼 그 진입로가 없어졌다. 되찾으려면 "산 수신자에게 파킹 →
    //   그 에이전트의 epoch 교체(재활성화)" 로 등장 diff 를 만드는 데몬 레벨 테스트가 필요하다(백로그).
    let target_name = "late-recv";
    let Some((info, _tok)) = spawn_json_agent(&manager, &registry, target_name) else {
        skip_no_claude("c1_park_then_spawn_auto_delivers");
        let _ = std::fs::remove_dir_all(&data_dir);
        handle.shutdown().await;
        return;
    };

    // ── 2) 그 에이전트가 **턴 진행 중**(프라이밍 응답)일 때를 잡는다 ──────────────────────────
    //    실 claude 라 타이밍이 우리 손에 없다 — busy 창을 못 잡으면(이미 idle) 이 축은 검증 불가이므로
    //    조용한 초록 대신 명시적 스킵으로 남긴다(하네스 한계를 감추지 않는다).
    let saw_busy = wait_until(Duration::from_secs(20), || {
        busy.is_busy(info.id, info.epoch)
    });
    if !saw_busy {
        eprintln!(
            "SKIP c1_park_then_spawn_auto_delivers: 스폰 직후 busy 창을 관측하지 못함(실 claude 타이밍) — \
             busy 파킹→flush 축은 c2_busy_recipient_parks_then_batch_flushes_on_turn_end 가 결정적으로 커버"
        );
        manager.kill_agent(info.id).ok();
        let _ = std::fs::remove_dir_all(&data_dir);
        handle.shutdown().await;
        return;
    }

    // ── 3) 턴 중 발송 → 파킹(pending). 주입이 없으므로 배달 관측도 없다 ─────────────────────
    let result = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![target_name.to_string()],
            body: "parked-until-idle".to_string(),
            contract: Default::default(),
        },
    );
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "pending",
        "턴 진행 중 도착은 파킹(spec §5 분기 3): {v}"
    );
    assert!(
        seen.lock().unwrap().is_empty(),
        "파킹 시점엔 배달 관측 없음(주입 안 함)"
    );

    // ── 4) 턴이 끝나면 idle 트리거 → flush 레인 → 자동 배달 ─────────────────────────────────
    let delivered = wait_until(Duration::from_secs(30), || {
        seen.lock().unwrap().iter().any(
            |o: &engram_dashboard_messaging::envelope::DeliveryObservation| {
                o.to_name == target_name && o.from.peer_id == sender && o.is_delivered()
            },
        )
    });
    assert!(
        delivered,
        "턴 종료(idle 전이) 시 파킹분이 자동 배달돼야(flush → inject → 관측): {:?}",
        seen.lock().unwrap()
    );

    manager.kill_agent(info.id).ok();
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── S18 메시징 v1 C2 수용 시나리오(spec §7 "배치 검증 강화"): busy 중 도착 → 미주입 → 턴 종료 시 일괄 flush ──
// ★무엇을 증명하나★: idle 게이트 배선 **전 구간**이 실제로 이어져 있음 — core.emit(분류) → 턴 관측 표 →
//   MessagingService 게이트(파킹) → IdleNotifier → flush 채널 → flush worker → flush_for_agent → 주입.
//   단언은 ① busy 중 도착분이 **주입되지 않고** pending ② 턴 종료(MessageDone) 후 **한 배치로 오래된 순**
//   주입 ③ 장부가 pending→delivered 로만 전이(유령 delivered 없음).
// ★왜 claude 없이 결정적인가★: 수신자는 obs_seam 의 structured 세션(실 PTY·claude 불요, write 캡처 가능)이고,
//   턴 이벤트는 그 세션 core 에 **직접** emit 한다 — 실 claude 턴의 타이밍(응답 지연·인증)에 의존하지
//   않는다. 실경로 e2e 축은 아래 `c2_live_*` 가 담당한다(claude-gated). 두 축이 상보적이다.
// ★multi_thread 런타임★: flush 는 flush worker(tokio task)가 수행하고 이 본문의 `wait_until` 은 동기
//   블로킹 폴링이다 — c1 테스트와 같은 사유(현재 스레드를 붙잡으면 worker 가 폴링될 틈이 없다).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_busy_recipient_parks_then_batch_flushes_on_turn_end() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;
    use engram_dashboard_messaging::ledger::DeliveryStatus;

    let (manager, registry, _base, data_dir, handle, messaging, busy) = wire("c2-idle-gate").await;

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c2-sender".to_string());
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    // 도달 가능(structured) 수신자 — write 성공 + 바이트 캡처 + 턴 관측 배선(ADR-0113).
    let (b_id, captured, core) = obs_seam::insert_observed_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    // ★턴 이벤트를 그 수신자의 core 에 직접 emit 한다★: 운영에서 pump 가 하는 일과 같은 진입점이라
    //   분류(이벤트→턴 신호) → 코어 표 → 턴 종료 push → 도어벨까지 **운영 경로 그대로** 탄다.
    let feed = |ev: OutputEvent| core.emit(ev);

    // ── 1) 턴 시작 관측(어시스턴트 델타) → busy ────────────────────────────────────────
    feed(OutputEvent::TextDelta {
        text: "thinking...".to_string(),
        turn_id: None,
        message_id: None,
    });
    assert!(
        busy.is_busy(b_id, 0),
        "턴 이벤트 관측 후 게이트가 busy 로 보여야(emit → 코어 표 → 게이트 배선)"
    );

    // ── 2) 턴 중 2건 발송 → 둘 다 pending(주입 0) ───────────────────────────────────────
    let mut ids = Vec::new();
    for body in ["first", "second"] {
        let v = handle_send(
            &manager,
            &registry,
            &messaging,
            Entrance::Cli,
            ControlCommand {
                from,
                to: vec![to_name.clone()],
                body: body.to_string(),
                contract: Default::default(),
            },
        )
        .to_json();
        assert_eq!(
            v["results"][0]["status"], "pending",
            "턴 진행 중 도착은 파킹(pending) — CLI stdin 선주입 금지(ADR-0104): {v}"
        );
        ids.push(v["id"].as_str().expect("msg id").to_string());
    }
    assert!(
        obs_seam::all_written(&captured).is_empty(),
        "턴 중에는 수신자 stdin 에 아무것도 쓰지 않는다"
    );
    assert!(
        seen.lock().unwrap().is_empty(),
        "주입이 없으므로 배달 관측도 없다(유령 delivered 금지)"
    );
    assert_eq!(messaging.parked_len(&to_name), 2, "2건이 메일박스에 대기");
    for id in &ids {
        assert_eq!(messaging.ledger_statuses(id), vec![DeliveryStatus::Pending]);
    }

    // ── 3) 턴 종료(MessageDone) → idle 트리거 → 일괄 flush ─────────────────────────────
    feed(OutputEvent::MessageDone {
        turn_id: None,
        message_id: None,
    });
    assert!(!busy.is_busy(b_id, 0), "MessageDone → idle");
    let flushed = wait_until(Duration::from_secs(10), || {
        obs_seam::all_written(&captured).len() == 2
    });
    let written = obs_seam::all_written(&captured);
    assert!(
        flushed,
        "턴 종료 시 파킹분이 **한 배치로** 주입돼야(idle 게이트 flush): 실제 {}건",
        written.len()
    );
    // 오래된 순 — 각 write 는 개별 봉투(stream-json 라인)로 분리돼 있다.
    let first = String::from_utf8_lossy(&written[0]).to_string();
    let second = String::from_utf8_lossy(&written[1]).to_string();
    assert!(
        first.contains("first") && !first.contains("second"),
        "배치 첫 주입 = 가장 오래된 메시지(개별 봉투): {first}"
    );
    assert!(
        second.contains("second"),
        "배치 둘째 주입 = 그 다음 메시지: {second}"
    );
    // 장부는 실제 주입 시점에만 delivered(ADR-0104) + 배달 관측 2건.
    for id in &ids {
        assert_eq!(
            messaging.ledger_statuses(id),
            vec![DeliveryStatus::Delivered],
            "flush 주입 후 pending→delivered 전이"
        );
    }
    assert_eq!(messaging.parked_len(&to_name), 0, "flush 후 큐 비움");
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|o| o.to_name == to_name && o.is_delivered())
            .count(),
        2,
        "배달 관측 2건(배치 각 항목)"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── C2 리뷰 fix 1: 재개 transcript 로 busy 를 부트스트랩하지 않는다 ──────────────────────────────
// ★무엇을 막는 회귀인가(치명)★: resume 스폰은 transcript 를 링에 seed 하는데(core ADR-0079
//   seed-before-publish), 그 transcript 가 **턴 중간에 끊긴** 것이면 진행 신호로 끝나고 종료 신호가 없다.
//   그걸 관측으로 먹이면 (id, epoch) 가 "턴 중" 으로 찍히는데 그 턴의 종료는 **영원히 오지 않는다** → 그
//   수신자 앞 모든 발송이 TTL 까지 파킹된다(깨울 수 없는 false-busy = 배달 정지).
// ★어떻게 결정적으로 재현하나(claude 불요)★: 관측 배선된 seam 수신자의 core 에 `seed` 로 "종료 신호 없이
//   끝난 과거" 를 정확히 쌓고, ① 그게 관측으로 새지 않는지 ② 그래도 라이브 emit 은 관측되는지 ③ 결과적으로
//   발송이 즉시 배달되는지를 본다.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_a_resumed_transcript_never_bootstraps_busy() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, busy) = wire("c2-seed").await;

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c2-seed-sender".to_string());
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let (b_id, captured, core) = obs_seam::insert_observed_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    // 1) 링에 "턴 중간에서 끝난 과거"(진행 신호만, 종료 신호 없음)를 seed = 죽은 incarnation 의 transcript.
    core.seed(vec![OutputEvent::TextDelta {
        text: "past mid-turn".to_string(),
        turn_id: None,
        message_id: None,
    }]);

    // 2) ★핵심 단언★: seed 는 관측이 아니므로 busy 가 아니다.
    assert!(
        !busy.is_busy(b_id, 0),
        "재개 transcript 로 busy 를 부트스트랩하면 그 busy 는 깨울 수 없다(TTL 까지 배달 정지)"
    );

    // 3) 사용자 관점 결과: 발송이 파킹되지 않고 즉시 배달된다(false-busy 가 없다는 증거).
    let written_before = obs_seam::all_written(&captured).len();
    let v = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![to_name.clone()],
            body: "hello".to_string(),
            contract: Default::default(),
        },
    )
    .to_json();
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "부트스트랩이 없으니 즉시 배달: {v}"
    );
    assert_eq!(messaging.parked_len(&to_name), 0, "파킹 0");

    // 4) 그러나 관측은 살아 있다 — 방금 그 주입이 만든 **라이브** 유저 에코가 표에 들어간다.
    //    (주입은 write_stdin_observed 경로라 반환 시점에 이미 emit 됐다 = 결정적.)
    assert!(
        obs_seam::all_written(&captured).len() > written_before,
        "주입이 실제로 일어났어야(전제)"
    );
    assert!(
        busy.is_busy(b_id, 0),
        "라이브 emit 은 관측된다 — seed 배제가 '아무것도 안 본다' 가 아님을 못 박는다"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── C2: 턴 중에 죽은 수신자가 유령 busy 를 남기지 않는다(데몬 전 구간, claude 불요) ─────────────────
// ★무엇을 증명하나★: 코어의 종료 청소(`OutputCore::finish` → 턴 관측 제거)와 종료 후 지각 emit 가드가
//   **실 데몬 조립**(매니저 표 → ManagerTurnFacts → BusyPolicy 게이트) 위에서 성립함. 단위 테스트는
//   코어 안에서만 보므로 어댑터·게이트가 같은 표를 보는지는 여기서만 잡힌다.
// ★막는 회귀★: 턴 중에 죽은 화신의 "턴 중" 이 남으면 ① 그 이름 앞 파킹이 영영 안 풀리고 ② 상한 sweep 이
//   죽은 에이전트에게 60초마다 도어벨을 울린다(프로세스 수명 내내).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_a_recipient_that_dies_mid_turn_leaves_no_busy_ghost() {
    use engram_dashboard_core::agent::types::TerminalReason;

    let (manager, _registry, _base, data_dir, handle, _messaging, busy) = wire("c2-ghost").await;
    let (b_id, _captured, core) = obs_seam::insert_observed_seam_recipient(&manager, false);

    core.emit(OutputEvent::TextDelta {
        text: "thinking...".to_string(),
        turn_id: None,
        message_id: None,
    });
    assert!(busy.is_busy(b_id, 0), "전제: 게이트가 턴 중으로 본다");

    // 턴 종료 신호 없이 죽는다(비정상 종료 — 상한 sweep 이 원래 다루던 그 모양).
    core.finish(TerminalReason::Killed);
    assert!(
        !busy.is_busy(b_id, 0),
        "종료한 화신은 게이트에서 즉시 사라져야(상한 30분을 기다리지 않는다)"
    );

    // 막혀 있던 주입 스레드가 뒤늦게 입력 에코를 낸다 = 유령 되살리기 시도(같은 epoch).
    core.emit(OutputEvent::Structured {
        kind: "user".to_string(),
        json: "{}".to_string(),
    });
    assert!(
        !busy.is_busy(b_id, 0),
        "종료 후 지각 에코가 유령 busy 를 되살리면 안 된다"
    );
    assert!(
        manager
            .turns()
            .in_turn_snapshot()
            .iter()
            .all(|(id, _, _)| *id != b_id),
        "상한 sweep 이 훑는 목록에도 남지 않아야(죽은 에이전트 도어벨 반복 방지)"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── C2 실경로 축(claude-gated): 실 claude 턴 중 발송이 파킹되는지 ────────────────────────────────
// ★위 결정적 테스트와의 차이★: 여기선 busy 관측이 **실 claude decoder** 가 만든 이벤트로 일어난다(합성
//   emit 아님). 즉 "capability 프록시(structured=turn 이벤트 있음)" 가 실제로 성립하는지의 실측이다.
// ★어디까지 결정적인가(정직 범위)★: 첫 주입 직후의 busy 관측은 결정적이다 — `write_input_observed` 가
//   send_input 성공 직후 **동기로** 입력-시점 유저 에코(`Structured{kind:"user"}`)를 core.emit 하므로,
//   handle_send 가 반환한 시점에 표는 이미 그 이벤트를 반영했다(claude 응답·인증과 무관). 그래서 "턴 중
//   발송 → pending" 까지는 hard assert 한다.
//   반면 **턴 종료 후 배달**은 claude 가 실제로 응답해 `result` 라인을 내야 한다(인증·네트워크·모델 지연에
//   의존) — 그건 관측되면 단언하고, 시간 내 안 오면 loud 경고 후 넘어간다(`ENGRAM_TEST_REQUIRE_CLAUDE=1`
//   레인에선 panic 으로 승격 — silent skip 금지 정책 정합). 그 축의 결정적 커버리지는 위 테스트가 갖는다.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_live_mid_turn_send_parks_and_delivers_after_turn_end() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, busy) = wire("c2-live").await;

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c2-live-sender".to_string());
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let target = "c2-live-recv";
    let Some((info, _tok)) = spawn_json_agent(&manager, &registry, target) else {
        skip_no_claude("c2_live_mid_turn_send_parks_and_delivers_after_turn_end");
        let _ = std::fs::remove_dir_all(&data_dir);
        handle.shutdown().await;
        return;
    };

    // 1) 입력 전 claude 는 조용하다(system/init 은 decoder 가 흘린다) → 시작 상태 = idle 확인.
    //    ★관측 배선이 실제로 살아 있다는 증거는 아래 2)의 hard assert 가 진다★ — 그게 없으면 이 테스트가
    //    "게이트가 아예 없어서 통과" 하는 위약이 된다.
    assert!(
        wait_until(Duration::from_secs(5), || !busy
            .is_busy(info.id, info.epoch)),
        "입력 전 수신자는 idle 로 관측돼야(턴 이벤트 없음)"
    );

    // 2) 첫 발송 → idle 이라 즉시 배달. 이 write 가 동기 유저 에코를 내므로 반환 시점에 표는 turn-중.
    let v1 = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![target.to_string()],
            body: "say OK".to_string(),
            contract: Default::default(),
        },
    )
    .to_json();
    assert_eq!(
        v1["results"][0]["status"], "delivered",
        "idle 수신자에게는 즉시 주입: {v1}"
    );
    assert!(
        busy.is_busy(info.id, info.epoch),
        "주입 = 턴 시작 — 입력-시점 유저 에코(Structured)가 실 emit 경로로 표에 들어가야"
    );

    // 3) 턴 진행 중 두 번째 발송 → 파킹(pending). 실 claude 턴 중 주입 금지의 e2e 증거.
    let v2 = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![target.to_string()],
            body: "and then say DONE".to_string(),
            contract: Default::default(),
        },
    )
    .to_json();
    assert_eq!(
        v2["results"][0]["status"], "pending",
        "실 claude 턴 진행 중 도착은 파킹: {v2}"
    );
    let msg2 = v2["id"].as_str().expect("msg id").to_string();
    assert_eq!(messaging.parked_len(target), 1, "턴 중 도착분이 대기");

    // 4) 턴 종료(claude 의 result 라인)를 기다린다 → idle 트리거 → 파킹분 배달.
    let delivered = wait_until(Duration::from_secs(90), || {
        seen.lock().unwrap().iter().any(
            |o: &engram_dashboard_messaging::envelope::DeliveryObservation| {
                o.msg_id == msg2 && o.is_delivered()
            },
        )
    });
    if delivered {
        assert_eq!(messaging.parked_len(target), 0, "턴 종료 flush 로 큐 비움");
    } else {
        // 실 claude 왕복(인증·네트워크·모델)에 달린 축 — 결정적 커버리지는 위 c2_busy_* 가 갖는다.
        eprintln!(
            "[c2_live] 턴 종료(result)가 90s 내 관측되지 않아 turn-end 배달 축은 미확인 \
             (parked={}) — 실 claude 응답 의존 축, 결정적 단언은 c2_busy_recipient_parks_* 가 담당",
            messaging.parked_len(target)
        );
        if std::env::var("ENGRAM_TEST_REQUIRE_CLAUDE").as_deref() == Ok("1") {
            panic!(
                "ENGRAM_TEST_REQUIRE_CLAUDE=1 레인에서 실 claude 턴 종료 후 파킹 배달이 관측되지 않음 \
                 (턴 이벤트 관측/idle flush 회귀 의심)"
            );
        }
    }

    manager.kill_agent(info.id).ok();
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// ── ADR-0088 Stage 1-오라클 4(write 실패): 실패 **관측 형태** — 단일 실패 레코드(부분/중복 관측 없음) ──
/// 수신자가 도달 가능(structured)하나 relay write(send_input)가 Err.
/// ★증명한다★: 실패의 **관측 형태(observation shape)** — send_input 이 Err 를 낼 때 레코드가 정확히
///   1건 + error=Some + bytes_written=None + msg_uuid=None + **to_epoch=None** + !is_delivered(성공 필드가
///   하나도 새지 않음). 봉투 바이트(bytes_requested)는 실려도(무엇을 배달하려다 실패했나의 forensic) 성공 신호는 안 샌다.
/// ★증명하지 않는다(커버리지 공백)★: **실제 OS write 가 prefix 를 쓴 뒤 Err 를 내는
///   부분 배달/truncation 부재**. 이 seam 은 send_input 이 push **전에** 통째로 Err 를 반환하므로(원자
///   all-or-nothing 모사) "prefix 만 쓰이고 실패" 상황 자체가 발생하지 않는다.
///   ▶ follow-up 존재: 실 pipe(자식이 K 바이트만 읽고 종료 → prefix 를 물리적으로 받아들인 뒤 끊김)
///     하네스가 이 축을 커버한다 — core 크레이트 `tests/stdio_physical_pipe.rs` ::
///     `physical_pipe_partial_write_then_err_surfaces_as_err`(prefix 쓴 뒤에도 WriteFailed 로 표면화).
#[tokio::test]
async fn stage1_lifecycle_write_error_single_failure_no_partial_dup() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("stage1-write-err").await;

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
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![to_name.clone()],
            body: "will-fail-once".to_string(),
            contract: Default::default(),
        },
    );
    let v = result.to_json();
    // ★C1(spec §5)★: inject 실패 → 파킹(pending). 실패 관측 shape 는 그대로(성공 필드 누출 없음, 아래).
    assert_eq!(
        v["results"][0]["status"], "pending",
        "write 실패는 파킹(pending): {v}"
    );

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
    // ★None 계약 고정(ADR-0088)★: 실패 분기가 to_epoch 를 Some(epoch)으로 뒤집으면(완결 write 없이 착지
    //   incarnation 을 참칭) 성공 필드 누출이다 — 이 단언이 그 회귀를 잡는다.
    assert_eq!(
        obs.to_epoch, None,
        "실패 = to_epoch None(완결 write 없음 → attest 할 착지 incarnation 없음, 성공 필드 누출 없음)"
    );
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
/// ★진짜 mid-flight epoch race 는 별도 오라클이 결정적으로 커버(follow-up 닫힘)★: resolve 가 epoch 0 을
///   보고 그 직후 재시작으로 epoch 1 이 current 가 된 뒤 write 가 epoch 1 로 착지하는 resolve↔write
///   **사이**의 경쟁은 이 순차 교체 테스트가 다루는 범위가 아니다. 그 race 는 이제
///   `stage1_lifecycle_mid_flight_epoch_race_lands_on_new_incarnation_deterministic` 가 프로덕션
///   yield-seam(handle_send 의 write 직전 test hook — ControlRegistry::set_mid_send_hook, ADR-0088)으로
///   **결정적으로** 재현·단언한다. ★ADR-0086 §F5 는 이 race 를 design-accepted 로 표시★(메일은 논리
///   에이전트를 향하므로 새 incarnation 착지가 올바른 동작) — seam 은 그 동작을 **관측**할 뿐 epoch 를
///   pin 하지 않는다.
///
/// ★관측 한계 닫힘(follow-up)★: DeliveryObservation 이 이제 착지 incarnation 의 epoch(`to_epoch`)을
///   담는다(ADR-0088 — core WriteOutcome.epoch 를 실은 값 = write 를 집행한 세션의 epoch). 그래서 이
///   테스트는 "현재 incarnation(epoch 1)이 받았다" 를 (i) 배달 성사·(ii) 새 버퍼에만 바이트 착지 라는
///   간접 증거에 더해 **레코드의 `to_epoch == Some(1)` 로 직접** 단언한다(record-self-sufficient).
#[tokio::test]
async fn stage1_lifecycle_epoch_rotation_delivers_to_current_incarnation() {
    use engram_dashboard_core::agent::backend::InputEncoder;
    use engram_dashboard_core::agent::output_core::{OutputCore, TurnWiring};
    use engram_dashboard_core::agent::session::AgentSession;
    use engram_dashboard_core::agent::transport::AgentTransport;
    use engram_dashboard_core::agent::types::{
        AgentId as CoreAgentId, AgentStatus, BackendCaps, ControlCaps, InputCaps, InputEvent,
        ModelCaps, OutputCaps, PtyError, SessionCaps, StatusSink, TransportCaps,
    };
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;
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
        let core = Arc::new(OutputCore::new(
            id,
            epoch,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        // ADR-0101 (WYSIWYA): 프로필 없는 seam 세션의 canonical name = basename(session.cwd) 이므로,
        //   테스트가 fallback_name(id)=id[:8] 로 지목하려면 cwd basename 을 id[:8] 로 맞춰야 한다
        //   (옛 cwd="." 는 basename="." 이라 id[:8] 지목이 RECIPIENT_NOT_FOUND 로 튄다).
        let cwd = std::path::PathBuf::from(format!("seam-root/{}", &id.to_string()[..8]));
        let session = Arc::new(AgentSession::new(
            id,
            cwd,
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

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("stage1-epoch").await;

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
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![to_name],
            body: "to-current-incarnation".to_string(),
            contract: Default::default(),
        },
    );
    let v = result.to_json();
    // 배달은 성사돼야(안정 주소로 향함) — 유실이면 여기서 error 가 뜬다.
    assert_eq!(
        v["results"][0]["status"], "delivered",
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
    // ADR-0088: 착지 incarnation epoch 을 레코드가 직접 담는다 — 교체된 현재 incarnation(epoch 1)에 배달됐음을
    //   레코드만으로 단언(record-self-sufficient, 옛 docstring 의 "간접 확인" 한계 제거).
    assert_eq!(
        g[0].to_epoch,
        Some(1),
        "레코드 to_epoch = 착지한 현재 incarnation(epoch 1) — 직접 단언"
    );
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

/// ── ADR-0088 Stage 1-오라클(mid-flight epoch race): **결정적** resolve↔write 경쟁 재현 ──────────────
/// ★증명한다★: handle_send 가 수신자를 해석(list_agents 스냅샷)한 **직후**, write_stdin_observed **직전**에
///   같은 AgentId 가 새 epoch incarnation 으로 교체되면(재시작=epoch bump 모사), write 는 resolve 가 본
///   구 incarnation(epoch 0)이 아니라 **write 해석 시점의** incarnation(epoch 1)에 착지한다. (엄밀히는
///   get_session 이후 한 번 더 동시 교체가 끼면 그 사이 "직전"이 된 incarnation 에 바이트가 갈 수도 있다 —
///   그래도 to_epoch 는 실제 착지 incarnation 을 정확히 담으므로 record-self-sufficiency 는 유지된다.) 이 race 는 순차 교체
///   (오라클 5)가 아니라 resolve↔write **사이**의 진짜 경쟁이다 — 프로덕션 yield-seam
///   (ControlRegistry::set_mid_send_hook, feature=test-harness)이 write 직전에 hook 을 발화해 그 갭에
///   결정적으로 개입한다(스케줄러 타이밍 의존 없음). ★ADR-0086 §F5 = design-accepted★: 메일은 논리
///   에이전트(안정 주소)를 향하므로 새 incarnation 착지가 **올바른** 동작이다 — 유실 없음, 현재
///   incarnation 배달. 그 F5 설계 의도를 레코드의 `to_epoch == Some(1)` 로 **직접 입증**한다
///   (record-self-sufficient — 오라클 5 옛 docstring 이 "불가능"이라 했던 바로 그 단언).
///
/// ★증명하지 않는다(정직 범위)★: 실 StdioTransport/실 claude 는 개입하지 않는다(seam 레벨 — EpochSeam 이
///   write 를 캡처만). 여기서 주입한 그 한 지점(write 직전) 외의 다른 스케줄링 race(예: reachability↔write
///   사이, 물리 OS-pipe 인터리브)는 다루지 않는다. 이 테스트가 잡는 것은 **resolve 스냅샷과 실제 write
///   착지 incarnation 이 어긋날 수 있고, 그때 write 가 current 로 가며 레코드가 그 사실을 자기충족적으로
///   담는다**는 계약이다.
#[tokio::test]
async fn stage1_lifecycle_mid_flight_epoch_race_lands_on_new_incarnation_deterministic() {
    use engram_dashboard_core::agent::backend::InputEncoder;
    use engram_dashboard_core::agent::output_core::{OutputCore, TurnWiring};
    use engram_dashboard_core::agent::session::AgentSession;
    use engram_dashboard_core::agent::transport::AgentTransport;
    use engram_dashboard_core::agent::types::{
        AgentId as CoreAgentId, AgentStatus, BackendCaps, ControlCaps, InputCaps, InputEvent,
        ModelCaps, OutputCaps, PtyError, SessionCaps, StatusSink, TransportCaps,
    };
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

    // epoch 별 다른 캡처 버퍼를 심어 incarnation 을 구분한다(오라클 5 의 EpochSeam 과 동형, 인라인).
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
        let core = Arc::new(OutputCore::new(
            id,
            epoch,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        // ADR-0101 (WYSIWYA): 프로필 없는 seam 세션의 canonical name = basename(session.cwd) 이므로,
        //   테스트가 fallback_name(id)=id[:8] 로 지목하려면 cwd basename 을 id[:8] 로 맞춰야 한다
        //   (옛 cwd="." 는 basename="." 이라 id[:8] 지목이 RECIPIENT_NOT_FOUND 로 튄다).
        let cwd = std::path::PathBuf::from(format!("seam-root/{}", &id.to_string()[..8]));
        let session = Arc::new(AgentSession::new(
            id,
            cwd,
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
        // insert_test_session 은 같은 id 를 교체하므로 hook 안 재주입이 곧 incarnation 교체.
        manager.insert_test_session(session);
        captured
    }

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("stage1-midflight").await;

    let id = CoreAgentId::new_v4();
    let to_name = obs_seam::fallback_name(id);

    // incarnation A(epoch 0) 주입 → resolve 가 이걸 본다. old_buf = 그 캡처 버퍼.
    let old_buf = insert_epoch(&manager, id, 0);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    // ★mid-send hook: resolve↔write 갭에서 incarnation 을 epoch 1 로 교체 주입★. hook 은 write 직전 1회
    //   발화한다 — AtomicBool 가드로 정확히 한 번만 교체하고(방어적: 재발화해도 이중 rotate 없음), 교체 후
    //   생성한 새 버퍼를 공유 슬롯(new_buf_slot)에 실어 본체가 회수한다. self-clearing: 가드가 이미 켜지면
    //   이후 발화는 no-op이라 이 send 한 번에만 실효(별도 clear 불필요).
    let new_buf_slot: Arc<Mutex<Option<Arc<Mutex<Vec<Vec<u8>>>>>>> = Arc::new(Mutex::new(None));
    let rotated = Arc::new(AtomicBool::new(false));
    // ★Arc 순환 차단★: registry 는 hook(클로저)을 저장하고, registry 는 manager-side wiring 이 전이 소유한다.
    //   여기서 hook 이 manager 를 Arc 로 강하게 잡으면 manager↔hook 참조 순환이 생겨, cleanup
    //   (set_mid_send_hook(None)) 전에 단언이 panic 하면 manager(와 reaper 스레드)가 프로세스 수명 내내
    //   누수된다. 그래서 Weak 로 잡고 발화 때 upgrade 한다(실패 시 comment 후 조기 반환).
    let mgr_weak = Arc::downgrade(&manager);
    let slot_for_hook = new_buf_slot.clone();
    let rotated_for_hook = rotated.clone();
    registry.set_mid_send_hook(Some(Arc::new(move || {
        // compare_exchange: 딱 한 번만 rotate(재발화 방어). 이미 rotate 됐으면 즉시 반환.
        if rotated_for_hook
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        // Weak → Arc 승격. handle_send 진행 중이면 manager 는 살아 있어 항상 성공하나, 이미 drop 됐다면
        //   rotate 없이 빠진다(순환 차단의 대가 — 발화 시점엔 산 상태라 사실상 발생 안 함).
        let Some(mgr_for_hook) = mgr_weak.upgrade() else {
            return;
        };
        // 같은 AgentId 를 epoch 1 로 교체 주입 → 맵엔 이제 B(epoch 1)만 남는다(A 는 빠진다).
        let new_buf = insert_epoch(&mgr_for_hook, id, 1);
        *slot_for_hook.lock().unwrap() = Some(new_buf);
    })));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "stage1-midflight-sender".to_string());
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    // handle_send: resolve 는 epoch 0 을 보고, write 직전 hook 이 epoch 1 로 rotate, write 는 epoch 1 착지.
    let result = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![to_name],
            body: "mid-flight-race-body".to_string(),
            contract: Default::default(),
        },
    );
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "mid-flight 교체 후에도 현재 incarnation 으로 배달돼야(유실 없음, ADR-0086 §F5): {v}"
    );
    let ack_id = v["id"].as_str().expect("msg-id 동봉").to_string();

    // 관측 레코드 1건 — wrong-epoch 이중배달 없음.
    let obs = {
        let g = seen.lock().unwrap();
        assert_eq!(
            g.len(),
            1,
            "논리 메시지 1건 → 관측 레코드 1건(mid-flight 이중배달 없음): {:?}",
            *g
        );
        g[0].clone()
    };
    assert!(obs.is_delivered(), "is_delivered() = true(전량 수용)");
    assert_eq!(obs.to_id, id, "레코드 수신자 = 안정 AgentId");
    // ★핵심(record-self-sufficient)★: write 가 실제 착지한 incarnation 의 epoch = 1(resolve 가 본 0 아님).
    //   오라클 5 옛 docstring 이 "레코드만으로는 불가능"이라 한 바로 그 직접 단언.
    assert_eq!(
        obs.to_epoch,
        Some(1),
        "write 는 교체된 현재 incarnation(epoch 1)에 착지 — resolve 시점(epoch 0)이 아님. to_epoch={:?}",
        obs.to_epoch
    );
    // 상관 축: 레코드 msg_id = ACK id · msg_uuid 존재.
    assert_eq!(obs.msg_id, ack_id, "레코드 msg_id = ACK id(상관 축 1)");
    assert!(
        obs.msg_uuid.is_some(),
        "성공 배달은 msg_uuid 를 담아야(상관 축 2)"
    );

    // 바이트는 **epoch 1** 버퍼에만 — resolve 가 본 epoch 0 버퍼엔 안 꽂힘.
    let new_buf = new_buf_slot
        .lock()
        .unwrap()
        .clone()
        .expect("hook 이 epoch 1 incarnation 을 교체 주입했어야");
    assert_eq!(
        new_buf.lock().unwrap().len(),
        1,
        "현재 incarnation(epoch 1) 이 바이트를 받아야"
    );
    assert!(
        old_buf.lock().unwrap().is_empty(),
        "resolve 가 본 구 incarnation(epoch 0) 은 바이트를 받지 않아야(mid-flight 착지는 current)"
    );
    assert!(
        String::from_utf8_lossy(&new_buf.lock().unwrap()[0]).contains("mid-flight-race-body"),
        "현재 incarnation 버퍼에 봉투 본체가 온전히 담겨야"
    );

    // hook 해제(다른 테스트 격리 — 이 registry 는 이 테스트 전용이지만 명시적으로 clear).
    registry.set_mid_send_hook(None);
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

    let (manager, registry, _base, data_dir, handle, _messaging, _busy) = wire("mcp-tool").await;

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
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "MCP send happy path: {text}"
    );
    assert_eq!(v["results"][0]["to"], "recv");

    // ★C1: 없는 수신자 → 파킹(pending)★(RECIPIENT_NOT_FOUND 소멸, spec §5). 반려 아니라 접수 성공.
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
    let v: serde_json::Value = serde_json::from_str(&text).expect("ack json");
    assert_eq!(
        v["results"][0]["status"], "failed",
        "★ADR-0111 결정 1★ MCP 입구에서도 없는 수신자는 실패 행: {text}"
    );

    let _ = client.cancel().await;
    manager.kill_agent(b_info.id).ok();
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── C3(ADR-0103): 회신 계약 — request→reply→replied 전이 + 기한 초과 notice(seam, claude 불요) ────────
//
// ★왜 seam 인가★: 계약 전이·notice 주입은 **배관** 검증이라 실 claude 왕복이 필요 없다(모델 응답에
//   의존하면 플레이키해진다). structured 캐리어 seam 수신자를 꽂아 주입 바이트를 직접 회계한다.
// ★결정적 시계★: sweep 은 `now` 를 인자로 받으므로(순수성 불변식) 실제 대기 없이 기한을 넘긴 시각을 손으로
//   밀어 넣는다 — sleep 기반 타임아웃 테스트의 플레이키를 원천 제거한다.

/// C3 인자를 실은 /control/send POST(임의 JSON 바디). `post_send` 는 통보 전용이라 별도로 둔다.
async fn post_send_json(
    base: &str,
    bearer: Option<&str>,
    body: serde_json::Value,
) -> (reqwest::StatusCode, String) {
    post_control(base, "/control/send", bearer, body).await
}

/// 임의의 제어 라우트로 JSON POST(D — 조회/그룹 미러가 send 와 같은 서버·auth·프레이밍을 쓴다).
async fn post_control(
    base: &str,
    route: &str,
    bearer: Option<&str>,
    body: serde_json::Value,
) -> (reqwest::StatusCode, String) {
    let client = reqwest::Client::new();
    let mut req = client
        .post(format!("{base}{route}"))
        .header("Content-Type", "application/json")
        .json(&body);
    if let Some(b) = bearer {
        req = req.header("Authorization", format!("Bearer {b}"));
    }
    let resp = req.send().await.expect("http request");
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

#[tokio::test]
async fn c3_request_reply_roundtrip_transitions_ledger_to_replied() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, SendContract};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;
    use engram_dashboard_messaging::ledger::DeliveryStatus;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("c3-roundtrip").await;

    // A = 요청자(발신자이자 회신 수신자), B = 회신자. 둘 다 structured seam(도달 가능).
    let (a_id, a_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let (b_id, b_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let a_name = obs_seam::fallback_name(a_id);
    let b_name = obs_seam::fallback_name(b_id);
    registry.issue(a_id, 0, "c3-a".to_string());
    registry.issue(b_id, 0, "c3-b".to_string());
    let from_a = BoundIdentity {
        agent_id: a_id,
        epoch: 0,
    };
    let from_b = BoundIdentity {
        agent_id: b_id,
        epoch: 0,
    };

    // 1) A → B: request(+ 기한). 봉투에 id/type/reply-by 가 실려야 B 가 회신할 id 를 안다.
    let ack = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Mcp,
        ControlCommand {
            from: from_a,
            to: vec![b_name.clone()],
            body: "코드 짜고 회신해".to_string(),
            contract: SendContract {
                request: true,
                reply_by: Some("10m".to_string()),
                reply_to: None,
            },
        },
    )
    .to_json();
    assert_eq!(
        ack["results"][0]["status"], "delivered",
        "request ACK: {ack}"
    );
    let req_id = ack["id"].as_str().expect("msg id").to_string();
    assert!(
        req_id.starts_with("m-") && req_id.len() == 10,
        "wire id 는 짧은 base36 계약(spec §1): {req_id}"
    );
    assert_eq!(
        messaging.open_request_count(),
        1,
        "계약이 장부에 열려야(spec §3 단계 2)"
    );

    // B 가 실제로 받은 바이트에 request 속성이 들어 있어야 한다(stream-json 인코딩 안이라 substring 검사).
    let b_line = String::from_utf8_lossy(&obs_seam::last_written(&b_captured)).to_string();
    for needle in ["type=", "request", &req_id, "reply-by", "10m"] {
        assert!(
            b_line.contains(needle),
            "B 가 받은 봉투에 '{needle}' 가 있어야: {b_line}"
        );
    }

    // 2) B → A: 그 id 로 회신. 엄격 매칭이 계약을 닫고 이력을 Replied 로 전이한다.
    let ack = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from: from_b,
            to: vec![a_name.clone()],
            body: "다 짰음, 테스트 통과".to_string(),
            contract: SendContract {
                request: false,
                reply_by: None,
                reply_to: Some(req_id.clone()),
            },
        },
    )
    .to_json();
    assert_eq!(ack["results"][0]["status"], "delivered", "회신 ACK: {ack}");
    assert!(
        ack.get("status").is_none(),
        "회신 응답 shape 은 통보와 동일해야(새 필드 없음): {ack}"
    );

    assert_eq!(
        messaging.ledger_statuses(&req_id),
        vec![DeliveryStatus::Replied],
        "request 레코드가 replied 로 전이(spec §3 단계 3)"
    );
    assert_eq!(messaging.open_request_count(), 0, "계약이 닫혀야");

    // A 가 받은 봉투엔 in-reply-to 가 실린다(발신 인자 reply_to → 수신 속성 in-reply-to, spec §1).
    let a_line = String::from_utf8_lossy(&obs_seam::last_written(&a_captured)).to_string();
    assert!(
        a_line.contains("in-reply-to") && a_line.contains(&req_id),
        "A 가 받은 회신 봉투에 in-reply-to: {a_line}"
    );

    // 3) 기한이 지나도 notice 는 없다(회신된 계약은 due 대상이 아님).
    messaging.sweep(Instant::now() + Duration::from_secs(3600));
    let a_line = String::from_utf8_lossy(&obs_seam::last_written(&a_captured)).to_string();
    assert!(
        !a_line.contains("<notice>"),
        "회신된 계약엔 타임아웃 통지가 없어야: {a_line}"
    );

    manager.kill_agent(a_id).ok();
    manager.kill_agent(b_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ★multi_thread 필수★: 이 테스트는 **flush 레인(tokio task)** 이 실제로 진행해야 한다 — 아래 wait_until 은
//   블로킹 폴링이라 current_thread 런타임에선 그 레인을 굶겨 영영 배달이 안 된다(false-red). 기존 C1/C2
//   flush 관측 테스트들과 같은 flavor 를 쓴다.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c3_reply_by_timeout_injects_notice_to_the_sender() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, SendContract};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("c3-timeout").await;

    // A = 요청자(산·도달 — notice 를 받을 대상). B = **수용된** 수신자(산·도달)지만 **답하지 않는다**.
    // ★ADR-0111 결정 1 로 setup 이 바뀌었다★: 옛 판은 부재 이름 앞으로 request 를 걸었는데, 부재는 이제
    //   실패 행이고 **계약도 열리지 않는다**(추적 없는 request 금지). 기한 초과 통지의 전제는 "수용된
    //   수신자에게 계약이 열렸다" 이므로 산 수신자를 쓰고, 회신이 없다는 사실만 그대로 둔다.
    let (a_id, a_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let (b_id, _b_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let silent_worker = obs_seam::fallback_name(b_id);
    registry.issue(a_id, 0, "c3-timeout-a".to_string());
    let from_a = BoundIdentity {
        agent_id: a_id,
        epoch: 0,
    };

    let ack = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Mcp,
        ControlCommand {
            from: from_a,
            to: vec![silent_worker.clone()],
            body: "해줘".to_string(),
            contract: SendContract {
                request: true,
                reply_by: Some("1m".to_string()),
                reply_to: None,
            },
        },
    )
    .to_json();
    assert_eq!(
        ack["results"][0]["status"], "delivered",
        "수용된 수신자에게 배달 — 그 수신자의 계약이 열린다(spec §3): {ack}"
    );
    let req_id = ack["id"].as_str().expect("msg id").to_string();
    assert_eq!(messaging.open_request_count(), 1, "계약 1건(수신자 1명)");
    assert!(
        obs_seam::all_written(&a_captured).is_empty(),
        "아직 A(요청자)에게는 아무 것도 주입되지 않았다"
    );
    let _ = b_id;

    // 기한을 넘긴 시각으로 sweep(주입 시계 조작 — 대기 없음).
    messaging.sweep(Instant::now() + Duration::from_secs(61));

    // ★비동기 배달(운영 배선 미러)★: sweep 은 notice 를 **파킹 + 도어벨**만 하고 즉시 반환한다(자식 stdin
    //   blocking write 를 sweep task 에서 떼어내는 규율 — service.rs deliver_notice). 이 하네스는 운영과
    //   동일하게 flush 워커를 띄우므로 실제 주입은 그 레인에서 일어난다 → 폴링으로 기다린다.
    assert!(
        wait_until(Duration::from_secs(5), || !obs_seam::all_written(
            &a_captured
        )
        .is_empty()),
        "flush 레인이 notice 를 A 에게 주입해야"
    );
    let a_line = String::from_utf8_lossy(&obs_seam::last_written(&a_captured)).to_string();
    assert!(
        a_line.contains("<notice>"),
        "기한 초과 통지는 <notice> 태그(from 없음 = 회신 대상 아님): {a_line}"
    );
    // `[engram]` = 시스템 발신 표시(사용자 요청 2026-07-26 — 프라이밍 없이도 출처가 읽히도록). 가독용
    //   라벨이지 파싱 계약이 아니다(기계 판정은 태그 모양 · from 부재).
    for needle in [req_id.as_str(), "1m", silent_worker.as_str(), "[engram]"] {
        assert!(
            a_line.contains(needle),
            "notice 문구에 '{needle}' 가 있어야(spec §1 템플릿): {a_line}"
        );
    }
    assert!(
        !a_line.contains("<message"),
        "notice 는 <message> 로 새면 안 된다(회신 가능성 오인): {a_line}"
    );

    // 이중 통지 금지 — 다시 sweep 해도 A 에게 주입이 늘지 않는다(장부 notified 플래그).
    //   비동기 레인이라 "안 늘어남" 은 잠깐 기다린 뒤 확인해야 의미가 있다(즉시 확인하면 아직 안 온 걸
    //   안 온 것으로 오판할 수 있다 — false-green 방지).
    let before = obs_seam::all_written(&a_captured).len();
    messaging.sweep(Instant::now() + Duration::from_secs(120));
    assert!(
        !wait_until(Duration::from_millis(500), || obs_seam::all_written(
            &a_captured
        )
        .len()
            > before),
        "notice 는 정확히 1회(두 번째 sweep 은 아무 것도 안 보낸다)"
    );

    manager.kill_agent(a_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

#[tokio::test]
async fn c3_invalid_contract_args_are_rejected_identically_at_the_cli_entrance() {
    // 두 입구의 반려 코드/shape 가 같아야 한다(entrance-agnostic — ADR-0086). 여기선 HTTP 입구로 확인.
    let (_m, registry, base, data_dir, handle, _messaging, _busy) = wire("c3-args").await;
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c3-args-sender".to_string());
    let tok = Some("c3-args-sender");

    // 상호배타.
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": "nobody", "body": "x", "request": true, "reply_to": "m-1" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["code"], "INVALID_SEND_ARGS", "request+reply_to: {body}");
    assert!(v["hint"].is_string(), "교정 hint 동봉");

    // ★`reply_to` 는 수신자 정확히 1명 — **표기 단계에서** 막는다(spec §3 항목 7-① · ADR-0111)★.
    //   ① 다중 수신자 + reply_to.
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": ["a", "b"], "body": "x", "reply_to": "m-1" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["code"], "INVALID_SEND_ARGS",
        "reply_to+다중 수신자: {body}"
    );
    //   ② `@`토큰 동반 — **펼침 결과가 1명이어도** 반려한다(로스터 상태로 답이 갈리면 규칙을 못 배운다).
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": ["@all"], "body": "x", "reply_to": "m-1" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["code"], "INVALID_SEND_ARGS",
        "reply_to+@주소는 표기 단계에서 반려(펼침 전): {body}"
    );
    //   ③ 대조군 — 수신자 1명 + reply_to 는 통과한다(부재라 행만 실패).
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": ["nobody"], "body": "x", "reply_to": "m-1" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert!(
        v.get("code").is_none(),
        "단일 수신자 회신은 인자 반려 아님: {body}"
    );
    assert_eq!(v["results"][0]["status"], "failed", "{body}");

    // ★빈 수신자 목록 = INVALID_SEND_ARGS(spec §6)★ — 빈 배열·빈 문자열 둘 다.
    for empty in [serde_json::json!([]), serde_json::json!("")] {
        let (_s, body) =
            post_send_json(&base, tok, serde_json::json!({ "to": empty, "body": "x" })).await;
        let v: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(v["code"], "INVALID_SEND_ARGS", "빈 수신자 목록: {body}");
    }

    // reply_by 단독.
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": "nobody", "body": "x", "reply_by": "10m" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["code"], "INVALID_SEND_ARGS", "reply_by 단독: {body}");

    // 표기 오류.
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": "nobody", "body": "x", "request": true, "reply_by": "ten min" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["code"], "INVALID_SEND_ARGS", "기간 표기 오류: {body}");

    // ★1분 미만 기한(리뷰 fix 7)★ — 판정 해상도가 sweep 주기(60s)라 지킬 수 없는 약속은 받지 않는다.
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": "nobody", "body": "x", "request": true, "reply_by": "30s" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["code"], "INVALID_SEND_ARGS", "1분 미만 기한: {body}");
    assert!(
        v["hint"].as_str().unwrap_or_default().contains("1-minute"),
        "hint 가 하한을 알려야: {body}"
    );
    // 대조군: 정확히 1분(초 표기)은 수용 — 하한은 값에 걸리지 표기에 걸리지 않는다.
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": "nobody", "body": "x", "request": true, "reply_by": "60s" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["results"][0]["status"], "failed",
        "60s 는 유효한 표기다(수신자 부재로 행은 실패지만 INVALID_SEND_ARGS 가 아니다): {body}"
    );

    // ★`@`주소 request 는 이제 **허용**된다(ADR-0111 결정 5 — 옛 "v1 영구 금지" 폐기)★ — 아래 반려는
    //   "request 라서" 가 아니라 "`@coders` 라는 주소가 없어서" 다(주소 공간 오류 = 전체 반려, ADR-0114 결정 3).
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": "@coders", "body": "x", "request": true }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    // ★`GROUP_REQUEST_UNSUPPORTED` 는 폐지됐다(ADR-0111 결정 5 — 다중 수신자 request 허용)★. 이제 남은
    //   반려는 **주소 공간 오류**뿐이다: 사용자 정의 그룹이 사라져 `@coders` 라는 주소가 존재하지 않는다.
    assert_eq!(v["code"], "GROUP_NOT_FOUND", "없는 @주소: {body}");

    // 같은 그룹 주소라도 통보면 **request 금지에 걸리지 않는다** — fan-out 갈래로 내려가 해석 결과에 따라
    //   답한다(여기선 미등록이라 GROUP_NOT_FOUND). 두 코드가 갈리는 게 요점: request 금지는 이름과 무관한
    //   영구 계약이고, NOT_FOUND 는 명단 상태에 따른 답이다.
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": "@coders", "body": "x" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["code"], "GROUP_NOT_FOUND", "그룹 통보(미등록): {body}");

    // 옛 `{to, body}` 바디는 그대로 동작해야(통보 wire 호환).
    let (status, body) = post_send(&base, tok, "nobody", "hi").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["results"][0]["status"], "failed",
        "통보 회귀(수신자 부재로 행만 실패 — 인자 반려가 아님): {body}"
    );
    let id = v["id"].as_str().expect("id");
    assert!(id.starts_with("m-") && id.len() == 10, "id 포맷: {id}");

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ★C3 리뷰 fix 1★: 계약 필드(request/reply_to)는 XML 봉투 전용 — 콜론 포맷에선 반려된다.
//   ★왜 통합 테스트인가★: 판정 로직 자체는 ingress 단위 테스트가 두 갈래(포맷·템플릿 env) 모두 덮는다.
//   여기서 확인할 건 **실제 입구가 그 판정을 실제로 태우는지**(런타임 포맷 전환이 발송 반려로 이어지는지)다.
#[tokio::test]
async fn c3_contract_fields_are_rejected_while_the_colon_envelope_is_active() {
    use engram_dashboard_messaging::envelope::EnvelopeFormat;

    let (_m, registry, base, data_dir, handle, _messaging, _busy) = wire("c3-colon").await;
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c3-colon-sender".to_string());
    let tok = Some("c3-colon-sender");

    // 런타임 스위치를 콜론으로 — 이 포맷의 렌더는 id/type/reply-by/in-reply-to 를 통째로 버린다.
    registry.set_envelope_format(EnvelopeFormat::Colon);

    for payload in [
        serde_json::json!({ "to": "nobody", "body": "x", "request": true, "reply_by": "10m" }),
        serde_json::json!({ "to": "nobody", "body": "x", "reply_to": "m-7f3k" }),
    ] {
        let (_s, body) = post_send_json(&base, tok, payload.clone()).await;
        let v: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(
            v["code"], "INVALID_SEND_ARGS",
            "콜론 봉투에선 계약 필드 반려({payload}): {body}"
        );
        assert!(
            v["hint"]
                .as_str()
                .unwrap_or_default()
                .contains("XML envelope"),
            "교정 hint 가 사유를 말해야: {body}"
        );
    }

    // 대조군 ①: 같은 콜론 포맷에서도 **통보**는 정상 접수된다(기존 동작 불변).
    let (status, body) = post_send(&base, tok, "nobody", "hi").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["results"][0]["status"], "failed",
        "통보는 포맷 반려에 걸리지 않는다(수신자 부재로 행만 실패): {body}"
    );

    // 대조군 ②: xml 로 되돌리면 계약 발송이 다시 접수된다(반려는 포맷에만 걸린 것).
    registry.set_envelope_format(EnvelopeFormat::Xml);
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": "nobody", "body": "x", "request": true, "reply_by": "10m" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["results"][0]["status"], "failed",
        "xml 로 되돌리면 계약 발송이 접수된다(수신자 부재로 행만 실패 — 포맷 반려가 아님): {body}"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// stream-json 캐리어가 stdin 에 쓴 한 줄에서 **논리 봉투 텍스트**만 꺼낸다(`message.content[0].text`).
///
/// ★왜 문자열 contains 가 아니라 파싱인가★: 캐리어가 봉투를 JSON 문자열로 감싸며 `"` 를 이스케이프하므로
///   (`to=\"@all\"`) 원바이트에 대고 `to="@all"` 을 찾으면 **실제로는 맞는데 틀렸다고 나온다**. 봉투 golden
///   단언은 캐리어 인코딩이 아니라 봉투 자체를 봐야 하므로 한 겹 벗기고 비교한다.
fn stream_json_text(written: &[u8]) -> String {
    let line = String::from_utf8_lossy(written);
    let v: serde_json::Value = serde_json::from_str(&line)
        .unwrap_or_else(|e| panic!("stream-json 라인 파싱 실패({e}): {line}"));
    v["message"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("stream-json 라인에 봉투 텍스트가 없다: {line}"))
        .to_string()
}

// ── S18 메시징 v1 C4 수용 시나리오(spec §4·§6): 그룹 fan-out 이 **실 입구(HTTP /control/send)** 를 탄다 ──
// ★무엇을 증명하나(단위 테스트와 갈리는 지점)★: service.rs 단위 테스트는 fan-out 로직 자체를 덮는다.
//   여기서 볼 건 **입구 배선**이다 — `@` 주소가 HTTP 입구에서 fan-out 갈래로 내려가고, 멤버별 결과가
//   spec §6 의 `results[]`(멤버당 한 줄) JSON 으로 나오며, 실제 수신자 stdin 에 `to="@…"` 봉투가 쓰이는지.
// ★claude 불요·결정적★: 수신자는 obs_seam 의 structured 세션(실 PTY·claude 없이 write 캡처 가능)이라
//   로스터·도달성·주입을 전부 손으로 통제한다.

/// `@all` = 발송 순간 산 수신자 전원 − **발신자 자신**(spec §4 + 자기 메아리 금지 정책).
#[tokio::test]
async fn c4_all_group_fans_out_to_live_agents_and_excludes_the_sender() {
    let (manager, registry, base, data_dir, handle, _messaging, _busy) = wire("c4-all").await;

    // 발신자도 **산 structured 에이전트**여야 "자기 제외" 를 실증할 수 있다(로스터에 있어야 뺄 게 생긴다).
    let (sender_id, sender_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let (a_id, a_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let (b_id, b_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let sender_name = obs_seam::fallback_name(sender_id);
    let a_name = obs_seam::fallback_name(a_id);
    let b_name = obs_seam::fallback_name(b_id);
    registry.issue(sender_id, 0, "c4-all-token".to_string());

    let (status, body) = post_send(&base, Some("c4-all-token"), "@all", "전원 리베이스 대기").await;
    assert_eq!(status, reqwest::StatusCode::OK, "방송 접수도 200: {body}");
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert!(
        v.get("status").is_none(),
        "성공 응답엔 최상위 status 없음(spec §6): {body}"
    );

    // 멤버당 한 줄 — 발신자 줄은 **없어야** 한다.
    let results = v["results"].as_array().expect("results 배열").clone();
    // ★순서까지 단언한다(C4 리뷰 fix H)★: `got` 을 정렬하지 않는다 — 운영 `ManagerDeliveryPort` 가 로스터를
    //   (이름, id) 오름차순으로 내므로 `@all` 의 결과 순서 = **이름 정렬 순서**여야 한다. 정렬하면 이 결정성이
    //   테스트에서 사라지고(HashMap 순회 순서 그대로여도 초록) 실행마다 다른 주입 순서를 못 잡는다.
    let got: Vec<(String, String)> = results
        .iter()
        .map(|r| {
            (
                r["to"].as_str().unwrap_or_default().to_string(),
                r["status"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let mut want = vec![
        (a_name.clone(), "delivered".to_string()),
        (b_name.clone(), "delivered".to_string()),
    ];
    want.sort(); // 기대값만 정렬 — "결과가 이름 순으로 나온다" 가 단언 대상이다.
    assert_eq!(
        got, want,
        "@all = 산 전원 − 발신자, 각자 delivered, **이름 오름차순**(결정적 로스터 — fix H): {body}"
    );
    assert!(
        !results
            .iter()
            .any(|r| r["to"].as_str() == Some(sender_name.as_str())),
        "발신자 자신은 @all 명단에 없다(자기 방송 메아리 금지): {body}"
    );

    // 실제 주입 — 두 수신자 stdin 에만 쓰였고, 봉투에 방송 표시(`to="@all"`)가 붙는다(spec §1 노출 원칙).
    for (captured, who) in [(&a_captured, &a_name), (&b_captured, &b_name)] {
        let written = obs_seam::all_written(captured);
        assert_eq!(written.len(), 1, "{who} 에게 정확히 1건 주입");
        assert_eq!(
            stream_json_text(&written[0]),
            format!(r#"<message from="{sender_name}" to="@all">전원 리베이스 대기</message>"#),
            "방송 봉투 golden — to 속성이 실려야({who})"
        );
    }
    assert!(
        obs_seam::all_written(&sender_captured).is_empty(),
        "발신자 자신의 stdin 엔 아무것도 쓰이지 않는다"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// ★수신자 지목의 앞뒤 공백은 **그룹 축에만** 걷어낸다(C4 리뷰 fix G → round-3 fix 4 에서 범위 축소)★.
///
/// ★왜 실입구 테스트인가★: 이건 **판정들 사이의 불일치** 버그였다 — 그룹 갈래는 raw `to` 를
///   `starts_with('@')` 로 보고, 그룹 이름 정규화는 trim 한 값을 본다. 그래서 `" @all"` 은 단일 발송으로
///   흘러 "그런 이름의 에이전트 없음 → **부재 파킹**" 이 됐다: 발신자에겐 `pending` 성공으로 보이는데 실제로는
///   아무도 못 받고 TTL 에 소멸한다(공백 한 칸 뒤에 숨은 조용한 유실). 두 판정이 같은 문자열을 보는지는
///   입구를 실제로 태워야 증명된다.
/// ★단일 수신자는 **바이트 그대로**다(round-3 fix 4)★: C4 는 `cmd.to` 자체를 덮어써 단일 발송 주소까지
///   정규화했는데, 그건 과교정이다 — 이름 네임스페이스는 바이트 정확(WYSIWYA — ADR-0101)이라 무조건 trim 은
///   발신자가 쓰지 않은 이름으로 **재지목**하고 파킹 키·장부 키·응답 `to` 까지 바꾼다. 그래서 단일 갈래는
///   C4 이전 동작(= 이름 매치 실패 → 원문 이름으로 부재 파킹)으로 되돌린다. 아래 ②가 그걸 고정한다.
#[tokio::test]
async fn c4_leading_whitespace_in_the_destination_does_not_change_routing() {
    let (manager, registry, base, data_dir, handle, messaging, _busy) = wire("c4-trim").await;

    let (a_id, a_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let a_name = obs_seam::fallback_name(a_id);
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c4-trim-token".to_string());
    let tok = Some("c4-trim-token");

    // ① `" @all"` — 공백이 있어도 **그룹 갈래**로 간다(단일 발송 부재 파킹이 아니라 멤버별 회계).
    let (status, body) = post_send(&base, tok, " @all", "공백 방송").await;
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["results"][0]["to"], a_name.as_str(),
        "results 는 **멤버 이름** — 그룹으로 라우팅됐다는 증거(단일 발송이면 ' @all' 이 그대로 실린다): {body}"
    );
    assert_eq!(v["results"][0]["status"], "delivered", "{body}");
    assert_eq!(
        messaging.parked_len(" @all"),
        0,
        "공백 이름 앞으로 부재 파킹되지 않는다(조용한 유실 방지)"
    );
    // ★봉투 `to` 속성은 **수용 판정된 수신자가 2인 이상일 때만** 실린다(spec §1 — ADR-0111 로 노출 기준이
    //   "그룹이면" 에서 "수신자 2인 이상이면" 으로 바뀌었다)★. 여기선 산 수신자가 A 하나뿐이라 생략된다.
    assert!(
        !stream_json_text(&obs_seam::last_written(&a_captured)).contains("to="),
        "수용 수신자 1명이면 to 속성 없음(혼자 받은 편지): {:?}",
        stream_json_text(&obs_seam::last_written(&a_captured))
    );

    // ② `" <이름>"` — ★해석 순서 ①의 트림이 **모든 원소**에 적용된다(spec §5 · ADR-0111)★. 옛 판은 단일
    //    발송 주소를 바이트 그대로 두어(round-3 fix 4) 공백 붙은 지목이 부재 파킹으로 끝났는데, 부재 파킹이
    //    폐지된 지금 그 결말은 **조용한 유실이 아니라 실패 행**이고 발신자에겐 아무 이득이 없다. spec 이
    //    트림을 해석 순서에 명시하므로 데몬 단일점에서 다듬고, 그 결과 공백 지목은 **그 에이전트에게 배달**된다.
    //    응답 `to` 는 발신자가 쓴 표기(트림된 토큰)를 그대로 돌려준다(WYSIWYA — ADR-0101).
    let padded = format!(" {a_name}");
    let (status, body) = post_send(&base, tok, &padded, "공백 1:1").await;
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "앞뒤 공백은 해석 순서 ①에서 다듬어져 그 에이전트에게 배달된다(spec §5): {body}"
    );
    assert_eq!(
        v["results"][0]["to"],
        a_name.as_str(),
        "결과 `to` 는 트림된 토큰(발신자 표기) 그대로: {body}"
    );
    assert_eq!(
        messaging.parked_len(&padded),
        0,
        "공백 이름 앞으로 유령 큐가 생기지 않는다"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── D(spec §6): 조회·관리 입구 — `messages` / `group` 두 입구 동일 JSON ──────────────────────
//
// ★왜 통합 테스트인가★: 두 입구(MCP 툴 · CLI HTTP 라우트)가 **같은 공통 핸들러**를 부른다는 게 이 증분의
//   핵심 계약(ADR-0086 entrance-agnostic)인데, 그건 배선을 실제로 태워야만 증명된다 — 단위 테스트는
//   핸들러 하나만 본다. 그래서 실 데몬(MCP 서버 + auth 미들웨어 + MessagingService)을 띄우고 두 경로의
//   응답 JSON 이 **동일한지** 직접 비교한다.
// ★claude 불요★: 조회·그룹 관리는 자식 프로세스 stdin 을 건드리지 않는다(읽기/명단 조작뿐).

#[tokio::test]
async fn d_messages_reports_delivery_state_by_id_and_the_callers_open_items() {
    let (manager, registry, base, data_dir, handle, _messaging, _busy) = wire("d-messages").await;

    // 수신자(구조화 seam) + 발신자(순수 신원).
    let (recv_id, _recv_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let recv_name = obs_seam::fallback_name(recv_id);
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "d-msg-tok".to_string());
    let tok = Some("d-msg-tok");
    let sender_name = obs_seam::fallback_name(sender);

    // ① 통보 발송 → 그 id 로 status 조회.
    let (_s, body) = post_send(&base, tok, &recv_name, "hello").await;
    let sent: serde_json::Value = serde_json::from_str(&body).expect("json");
    let msg_id = sent["id"].as_str().expect("id").to_string();

    let (status, body) = post_control(
        &base,
        "/control/messages",
        tok,
        serde_json::json!({ "id": msg_id }),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["id"], msg_id.as_str());
    assert_eq!(v["from"], sender_name.as_str(), "발신자 라벨: {body}");
    assert_eq!(v["awaiting_reply"], false, "통보는 회신 대기 아님");
    assert_eq!(v["rows"].as_array().map(|a| a.len()), Some(1));
    assert_eq!(v["rows"][0]["to"], recv_name.as_str());
    assert_eq!(v["rows"][0]["status"], "delivered");
    assert!(v["rows"][0]["age_secs"].is_number(), "경과 초 동봉: {body}");

    // ② 없는 id → MESSAGE_NOT_FOUND(반려 shape 은 발송과 동일).
    let (_s, body) = post_control(
        &base,
        "/control/messages",
        tok,
        serde_json::json!({ "id": "m-nope1234" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["status"], "error");
    assert_eq!(v["code"], "MESSAGE_NOT_FOUND", "{body}");

    // ③ 무인자 = 내 미결. **파킹 1건**이 outbound_pending 으로 잡힌다.
    // ★ADR-0111 결정 1 로 setup 이 바뀌었다★: 부재 파킹이 폐지돼(실패 행) 미결을 만들 수 있는 경로는
    //   spec §5 분기 3 뿐이다 — 여기선 **주입(write) 실패** 파킹을 쓴다(산·도달 수신자인데 stdin write 가
    //   실패하는 seam). 관측 대상(미결 목록의 방향 태그·필드)은 그대로다.
    let (stuck_id, _stuck_captured) = obs_seam::insert_seam_recipient(&manager, true);
    let stuck_name = obs_seam::fallback_name(stuck_id);
    let (_s, _b) = post_send(&base, tok, &stuck_name, "parked").await;
    let (_s, body) = post_control(&base, "/control/messages", tok, serde_json::json!({})).await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["me"], sender_name.as_str(), "'나' 는 토큰 신원: {body}");
    let open = v["open"].as_array().expect("open 배열");
    assert_eq!(open.len(), 1, "미결 1건(배달된 통보는 미결 아님): {body}");
    assert_eq!(open[0]["direction"], "outbound_pending");
    assert_eq!(open[0]["to"], stuck_name.as_str());
    assert!(open[0].get("reply_by").is_none(), "통보 줄엔 계약 축 없음");

    // ④ 다른 신원으로 물으면 자기 것만 본다(남의 미결이 새지 않는다 — 신원은 토큰 파생).
    let other = AgentId::new_v4();
    registry.issue(other, 0, "d-msg-other".to_string());
    let (_s, body) = post_control(
        &base,
        "/control/messages",
        Some("d-msg-other"),
        serde_json::json!({}),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["open"],
        serde_json::json!([]),
        "남의 미결은 안 보인다: {body}"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

#[tokio::test]
async fn d_messages_shows_the_reply_debt_on_both_sides_of_a_request() {
    let (manager, registry, base, data_dir, handle, _messaging, _busy) = wire("d-req-debt").await;

    let (recv_id, _c) = obs_seam::insert_seam_recipient(&manager, false);
    let recv_name = obs_seam::fallback_name(recv_id);
    // 수신자 신원 토큰 — 그쪽 관점의 미결(내가 답할 것)을 물어본다.
    registry.issue(recv_id, 0, "d-recv-tok".to_string());
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "d-send-tok".to_string());

    let (_s, body) = post_send_json(
        &base,
        Some("d-send-tok"),
        serde_json::json!({ "to": recv_name, "body": "해줘", "request": true, "reply_by": "10m" }),
    )
    .await;
    let sent: serde_json::Value = serde_json::from_str(&body).expect("json");
    let req_id = sent["id"].as_str().expect("id").to_string();

    // 발신자 관점: 회신 대기(awaiting_their_reply) + 기한 표기 원본.
    let (_s, body) = post_control(
        &base,
        "/control/messages",
        Some("d-send-tok"),
        serde_json::json!({}),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    let open = v["open"].as_array().expect("배열");
    assert_eq!(open.len(), 1, "{body}");
    assert_eq!(open[0]["direction"], "awaiting_their_reply");
    assert_eq!(open[0]["id"], req_id.as_str());
    assert_eq!(open[0]["reply_by"], "10m", "기한 표기 원본 그대로: {body}");
    assert_eq!(open[0]["timed_out"], false);

    // 수신자 관점: **내가 답할 차례**(reply_owed_by_me) — 같은 계약이 정반대 태그로 보인다.
    let (_s, body) = post_control(
        &base,
        "/control/messages",
        Some("d-recv-tok"),
        serde_json::json!({}),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    let open = v["open"].as_array().expect("배열");
    assert_eq!(open.len(), 1, "{body}");
    assert_eq!(open[0]["direction"], "reply_owed_by_me", "{body}");
    assert_eq!(open[0]["id"], req_id.as_str());

    // id 조회는 awaiting_reply 로 같은 사실을 알린다.
    let (_s, body) = post_control(
        &base,
        "/control/messages",
        Some("d-send-tok"),
        serde_json::json!({ "id": req_id }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["awaiting_reply"], true, "{body}");

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// MCP 툴 호출 → text content JSON(D parity 테스트 전용 헬퍼).
async fn call_mcp_tool(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    use rmcp::model::CallToolRequestParams;
    let mut params = CallToolRequestParams::default();
    params.name = name.to_string().into();
    params.arguments = Some(args.as_object().expect("객체").clone());
    let result = client.call_tool(params).await.expect("call tool");
    let text = result
        .content
        .iter()
        .find_map(|c| c.as_text().map(|t| t.text.clone()))
        .expect("text content");
    serde_json::from_str(&text).expect("tool json")
}

/// ★두 입구 동일 JSON(spec §6)★ — MCP 툴과 CLI 라우트의 응답이 **바이트 동형**인지 직접 비교한다.
/// 어긋나면 한쪽 입구의 LLM 만 다른 계약을 배우게 되므로(발신 freeze 와 같은 계열의 사고) 회귀 그물이다.
#[tokio::test]
async fn d_mcp_and_cli_entrances_return_identical_json_for_messages_and_group() {
    use rmcp::transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    };
    use rmcp::ServiceExt;

    let (manager, registry, base, data_dir, handle, _messaging, _busy) = wire("d-parity").await;
    let caller = AgentId::new_v4();
    registry.issue(caller, 0, "d-parity-tok".to_string());
    let tok = Some("d-parity-tok");

    let config = StreamableHttpClientTransportConfig::with_uri(handle.url.clone())
        .auth_header("d-parity-tok");
    let transport = StreamableHttpClientTransport::from_config(config);
    let client = ().serve(transport).await.expect("MCP handshake");

    // tools/list 에 조회·관리 툴이 노출된다(프라이밍이 가르치는 이름과 같은 값).
    let tools = client.list_all_tools().await.expect("list tools");
    let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    for want in ["send_message", "messages"] {
        assert!(
            names.contains(&want.to_string()),
            "tools 에 {want}: {names:?}"
        );
    }
    // ★`group` 툴 제거(ADR-0111 결정 4 · ADR-0112 결정 1)★ — 남아 있으면 프라이밍이 없는 툴을 가르친다.
    assert!(
        !names.contains(&"group".to_string()),
        "group 툴은 제거돼야: {names:?}"
    );

    // ① messages 무인자 — 같은 토큰이므로 "나" 도 같고 결과도 같다(빈 미결).
    let via_mcp = call_mcp_tool(&client, "messages", serde_json::json!({})).await;
    let (_s, body) = post_control(&base, "/control/messages", tok, serde_json::json!({})).await;
    let via_cli: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        via_mcp, via_cli,
        "미결 조회가 두 입구에서 **바이트 동형**이어야"
    );
    assert_eq!(via_mcp["open"], serde_json::json!([]));

    // ② 조회 반려도 동일 shape·동일 code.
    let via_mcp = call_mcp_tool(
        &client,
        "messages",
        serde_json::json!({ "id": "m-nope1234" }),
    )
    .await;
    let (_s, body) = post_control(
        &base,
        "/control/messages",
        tok,
        serde_json::json!({ "id": "m-nope1234" }),
    )
    .await;
    let via_cli: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(via_mcp, via_cli, "반려 JSON 도 두 입구 동일");
    assert_eq!(via_mcp["code"], "MESSAGE_NOT_FOUND");

    // ③ ★발송 응답 parity(ADR-0111 다중 수신자)★ — MCP 는 **배열**, CLI 는 **콤마 문자열**로 같은 두
    //    수신자를 지목한다. 표기는 입구마다 다르지만(콤마 분해는 CLI 전용) **정규화 이후는 데몬 단일점**
    //    이라 `results[]` 가 바이트 동형이어야 한다(`id` 만 발송마다 다르므로 제외하고 비교).
    let (x_id, _x) = obs_seam::insert_seam_recipient(&manager, false);
    let (y_id, _y) = obs_seam::insert_seam_recipient(&manager, false);
    let (x, y) = (obs_seam::fallback_name(x_id), obs_seam::fallback_name(y_id));
    let via_mcp = call_mcp_tool(
        &client,
        "send_message",
        serde_json::json!({ "to": [x.clone(), y.clone()], "body": "parity" }),
    )
    .await;
    let (_s, body) = post_send(&base, tok, &format!("{x},{y}"), "parity").await;
    let via_cli: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        via_mcp["results"], via_cli["results"],
        "다중 수신자 발송 응답이 두 입구에서 동일해야: mcp={via_mcp} cli={via_cli}"
    );
    assert_eq!(
        via_mcp["results"].as_array().map(|a| a.len()),
        Some(2),
        "수신자 1명당 1행: {via_mcp}"
    );

    // ④ ★MCP 배열 원소는 **콤마로 쪼개지 않는다**(spec §6)★ — `"a,b"` 는 그런 **이름 하나**다. 같은
    //    문자열이 CLI 로 오면 두 수신자로 쪼개지므로, 여기서 두 입구의 결과는 **일부러 달라야** 한다.
    let split_free = call_mcp_tool(
        &client,
        "send_message",
        serde_json::json!({ "to": [format!("{x},{y}")], "body": "no-split" }),
    )
    .await;
    let rows = split_free["results"].as_array().expect("results");
    assert_eq!(rows.len(), 1, "배열 원소는 이름 하나: {split_free}");
    assert_eq!(rows[0]["status"], "failed");
    assert_eq!(
        rows[0]["code"], "RECIPIENT_NOT_FOUND",
        "'x,y' 라는 이름의 에이전트는 없다(이중 분해 금지의 증거): {split_free}"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// ★D 리뷰 B1 — 실 입구 판 회귀 그물★: 같은 이름의 산 에이전트가 둘일 때, exact AgentId 로 건 request 의
/// 회신 의무가 **쌍둥이에게 잘못 붙지 않는다**. 신원은 두 토큰(각 AgentId)에서 파생되므로 이 축은 실제
/// 인증 경로를 태워야만 증명된다(단위 테스트는 서비스 함수 인자만 본다).
#[tokio::test]
async fn d_messages_does_not_misassign_a_reply_obligation_to_a_same_named_twin() {
    let (manager, registry, base, data_dir, handle, _messaging, _busy) = wire("d-twin").await;

    // ★같은 **보이는 이름**의 산 에이전트 둘★ — 이 상태에서만 이름-only 귀속의 오귀속이 드러난다.
    //   이름이 겹치므로 발신자는 exact AgentId 로만 한쪽을 지목할 수 있다(이름 지목은 AMBIGUOUS 반려).
    let twin_a = AgentId::new_v4();
    let twin_b = AgentId::new_v4();
    let (_a, _cap_a) = obs_seam::insert_seam_recipient_named(&manager, false, twin_a, "worker");
    let (_b, _cap_b) = obs_seam::insert_seam_recipient_named(&manager, false, twin_b, "worker");
    registry.issue(twin_a, 0, "d-twin-a".to_string());
    registry.issue(twin_b, 0, "d-twin-b".to_string());
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "d-twin-send".to_string());

    // exact AgentId 로 A 에게만 request.
    let (_s, body) = post_send_json(
        &base,
        Some("d-twin-send"),
        serde_json::json!({ "to": twin_a.to_string(), "body": "해줘", "request": true, "reply_by": "10m" }),
    )
    .await;
    let sent: serde_json::Value = serde_json::from_str(&body).expect("json");
    let req_id = sent["id"].as_str().expect("id").to_string();

    // A 관점: 내가 답할 차례.
    let (_s, body) = post_control(
        &base,
        "/control/messages",
        Some("d-twin-a"),
        serde_json::json!({}),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    let a_open = v["open"].as_array().expect("배열");
    assert_eq!(a_open.len(), 1, "지목된 A 는 의무를 진다: {body}");
    assert_eq!(a_open[0]["direction"], "reply_owed_by_me");
    assert_eq!(a_open[0]["id"], req_id.as_str());

    // B 관점: 받은 적이 없으므로 **아무 것도 없다**(옛 구현은 여기서 A 의 의무를 돌려줬다).
    let (_s, body) = post_control(
        &base,
        "/control/messages",
        Some("d-twin-b"),
        serde_json::json!({}),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["open"],
        serde_json::json!([]),
        "받은 적 없는 쌍둥이에게 의무가 붙으면 안 된다(B1): {body}"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// ★D 리뷰 B2 — 조회가 완전성을 단언하는지 여부를 응답에 싣는다★.
#[tokio::test]
async fn d_messages_declares_whether_the_row_list_is_complete() {
    let (manager, registry, base, data_dir, handle, _messaging, _busy) = wire("d-trunc").await;
    let (recv_id, _c) = obs_seam::insert_seam_recipient(&manager, false);
    let recv_name = obs_seam::fallback_name(recv_id);
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "d-trunc-tok".to_string());
    let tok = Some("d-trunc-tok");

    let (_s, body) = post_send(&base, tok, &recv_name, "hello").await;
    let sent: serde_json::Value = serde_json::from_str(&body).expect("json");
    let msg_id = sent["id"].as_str().expect("id").to_string();

    let (_s, body) = post_control(
        &base,
        "/control/messages",
        tok,
        serde_json::json!({ "id": msg_id }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    // 링이 아직 안 밀린 상태 = **확실히 전부**. 필드는 항상 실린다(부재를 완전성으로 오독하지 않게).
    assert_eq!(
        v["may_be_truncated"], false,
        "완전성은 적극적으로 단언한다: {body}"
    );
    assert!(
        v.get("hint").is_none(),
        "완전할 때는 경고 문장을 붙이지 않는다: {body}"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}
