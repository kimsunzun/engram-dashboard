//! ADR-0086 스텝 2 통합 테스트 — 듀얼 입구 A→B 메시지 전송(send_message MCP 툴 + /control/send HTTP 라우트).
//!
//! ★relay 관측 방식(honest note)★: 산 json 에이전트를 실제 스폰하고 write_input 이 send_input 성공 직후
//!   **동기**로 내는 입력 에코를 OutputSink 로 잡는다. 이 에코는 claude 왕복 이전에 발행되므로 claude
//!   응답 지연·인증과 무관하게 결정적이다.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::persistence::{FilePresetStore, FileProfileStore};
use engram_dashboard_agent::preset::PresetRegistry;
use engram_dashboard_agent::profile::{
    AgentCommand, AgentProfile, ClaudeOutputFormat, ProfileRegistry, SpawnMode,
};
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, OutputEvent, OutputFrame, OutputPayload,
    OutputSink, SinkError, SinkId, StatusSink,
};

use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server, CommandTableSlot, ManagerSlot, McpServerHandle, MessagingSlot,
};
use engram_dashboard_daemon::control::registry::ControlRegistry;
use engram_dashboard_daemon::control::DaemonControlChannel;
use engram_dashboard_messaging::service::MessagingService;

struct NoopSink;
impl StatusSink for NoopSink {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

/// ★중계 수거기를 **프로세스 수명에** 묶는 자리★
///
/// 떨어뜨리면 수거기가 나가는 길에 자리 표를 비우고 **닫아**(`CommandDeliveries::drain`) 그 서버의 그 뒤
/// 중계가 전부 「종료 중」으로 반려된다 — 그런데 `wire()` 안에서 지역 변수로 묶으면 함수를 빠져나오는
/// 순간 그 일이 일어난다(이 파일이 오랫동안 그 상태였고, 여기 테스트가 자리 표를 안 거치는 것만 불러
/// 초록이었다).
/// ★반환 튜플에 축을 하나 더 얹지 않는 이유★: 이 하네스는 호출부가 20곳이 넘고, 그 축은 어느 테스트도
/// 읽지 않는다. 그리고 이 하네스가 세우는 것들은 이미 **테스트 종료 = 프로세스 종료로 회수**하는 규율을
/// 쓴다(아래 flush worker 핸들 detach 와 같은 자리). 그래서 여기 모아 두고 프로세스 끝까지 산다.
static RELAY_SWEEPERS: Mutex<Vec<engram_dashboard_daemon::command_delivery::BusSweeper>> =
    Mutex::new(Vec::new());

fn hold_for_the_process(sweeper: engram_dashboard_daemon::command_delivery::BusSweeper) {
    RELAY_SWEEPERS
        .lock()
        .expect("relay sweepers poisoned")
        .push(sweeper);
}

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

/// ★CI 강제 knob(M2)★: cargo 는 test 의 stdout 을 기본 캡처해 삼키므로, loud print 를 해도 통과 요약엔
///   "ok" 만 남아 skip 이 조용히 새어 나간다. env `ENGRAM_TEST_REQUIRE_CLAUDE=1` 이 설정돼 있으면(=
///   claude 가 반드시 있어야 하는 CI 레인) skip 을 **panic 으로 승격**해 테스트를 실제로 실패시킨다 —
///   "silent skip 금지" 강제. 미설정(로컬 개발 기본)이면 기존대로 loud print 후 조용히 Ok 로 넘어간다.
fn skip_no_claude(test: &str) {
    let line = format!(
        "SKIPPED [{test}]: claude(stream-json) 에이전트 스폰 실패 — relay 실측 불가(claude 부재/인증). \
         registry/ingress 단위 테스트가 로직을 커버하나 end-to-end relay 는 이 머신에서 미검증."
    );
    println!("{line}");
    eprintln!("{line}");
    if std::env::var("ENGRAM_TEST_REQUIRE_CLAUDE").as_deref() == Ok("1") {
        panic!(
            "ENGRAM_TEST_REQUIRE_CLAUDE=1 인데 [{test}] 가 claude 부재로 skip 됨 — \
             이 레인은 silent skip 을 금지한다(claude(stream-json) 스폰이 반드시 성공해야 함)."
        );
    }
}

/// 운영 `run()` 의 조립 순서를 미러한 배선.
async fn wire(
    tag: &str,
) -> (
    Arc<AgentManager>,
    Arc<ControlRegistry>,
    String,
    std::path::PathBuf,
    McpServerHandle,
    Arc<MessagingService>,
    Arc<engram_dashboard_messaging::busy::BusyPolicy>,
) {
    let registry = Arc::new(ControlRegistry::new());
    let slot = Arc::new(ManagerSlot::new());
    let messaging_slot = Arc::new(MessagingSlot::new());
    // ★수거기를 들고 있어야 한다★ — 지역 변수로 묶으면 이 함수를 나가는 순간 자리 표가 닫힌다
    //   ([`hold_for_the_process`] 가 그 사유와 이 형태를 고른 근거를 적는다).
    let (relay_bus, relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    hold_for_the_process(relay_sweeper);
    let handle = start_mcp_server(
        registry.clone(),
        slot.clone(),
        messaging_slot.clone(),
        // 이 파일은 제어 동사를 부르지 않는다 — 명령 표를 비우면 그 라우트만 503 이 된다.
        Arc::new(CommandTableSlot::new()),
        relay_bus,
    )
    .await
    .expect("start mcp server");
    let url = handle.url.clone();
    let data_dir = std::env::temp_dir().join(format!("engram-cli-{tag}-{}", AgentId::new_v4()));

    let control: Arc<dyn ControlChannel> = Arc::new(DaemonControlChannel::new(
        registry.clone(),
        url.clone(),
        data_dir.clone(),
        None, // send_exe: relay 테스트는 CLI 경로 불요(직접 HTTP/MCP 호출).
        // ADR-0092: 기존 relay 테스트는 프라이밍 무관 — Noop 으로 오늘 동작과 byte-identical.
        Arc::new(engram_dashboard_daemon::control::priming::NoopPrimingProvider),
    ));

    let (flush_tx, flush_rx) =
        tokio::sync::mpsc::unbounded_channel::<engram_dashboard_daemon::messaging_host::FlushMsg>();
    let idle_coalescer = Arc::new(engram_dashboard_daemon::messaging_host::IdleCoalescer::new());
    let sink: Arc<dyn StatusSink> = Arc::new(
        engram_dashboard_daemon::messaging_host::MessagingFlushSink::new_test(
            Box::new(NoopSink),
            flush_tx.clone(),
            idle_coalescer.clone(),
        ),
    );
    let profiles = Arc::new(ProfileRegistry::new(Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-cli-prof-{tag}-{}", AgentId::new_v4())),
    ))));
    let presets = Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(
        std::env::temp_dir().join(format!("engram-cli-preset-{tag}-{}", AgentId::new_v4())),
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
    let idle_notifier = Arc::new(
        engram_dashboard_daemon::messaging_host::ChannelIdleNotifier::new(
            flush_tx,
            idle_coalescer.clone(),
        ),
    );
    let busy = Arc::new(
        engram_dashboard_daemon::messaging_host::busy_gate_for_manager(
            manager.clone(),
            idle_notifier.clone(),
        ),
    );
    let messaging = Arc::new(
        engram_dashboard_daemon::messaging_host::messaging_for_manager_gated(
            manager.clone(),
            registry.clone(),
            busy.clone(),
        )
        .with_flush_trigger(idle_notifier),
    );
    messaging_slot.set(messaging.clone());
    // 이 하네스는 worker 핸들을 detach 한다(테스트 종료 = 프로세스 종료로 회수 — 운영 종료 경로는
    //   lib.rs 가 belt 로 내린다).
    drop(engram_dashboard_daemon::messaging_host::spawn_flush_worker(
        flush_rx,
        engram_dashboard_daemon::messaging_host::FlushWiring {
            messaging: messaging_slot.clone(),
            idle: idle_coalescer,
        },
    ));

    let base = url.strip_suffix("/mcp").unwrap_or(&url).to_string();
    (manager, registry, base, data_dir, handle, messaging, busy)
}

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
    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .ok()?
        .into_started()?;
    if !wait_until(Duration::from_secs(5), || {
        manager.list_agents().iter().any(|a| a.id == info.id)
    }) {
        return None;
    }
    // registry 에 발급 토큰 조회 API 가 없어 provision 이 이 (id, epoch) 에 발급한 토큰을 못 꺼낸다 —
    //   발신자용 토큰을 따로 issue 해 심는다(발신자 신원만 맞으면 relay 는 동일).
    let token = format!("sender-tok-{}", info.id);
    registry.issue(info.id, info.epoch, token.clone(), true);
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
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "valid-sender".to_string(), true);

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

    let big = "x".repeat(64 * 1024 + 1);
    let (_s, body) = post_send(&base, Some("valid-sender"), "nobody", &big).await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["code"], "BODY_TOO_LARGE", "대용량 body: {body}");

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── /control/send: shell 산 세션은 **수신자가 아니다**(사용자 결정 2026-08-17) ─────────────────────────
// ★왜★: 셸에 도착한 봉투는 읽히는 게 아니라 **명령으로 실행된다**. 본문은 LLM 자유 텍스트라 `&`·`|`·`;` 가
//   섞이면 그 뒤가 별도 명령으로 파싱된다. 그래서 제출 바이트를 빼는 완화가 아니라 명단에서 통째로 뺀다.
// ★ADR-0116 결정 1 로의 회귀가 아니다★: 그건 "턴 신호가 없으니 배달할 수 없다" 를 기각한 것이고(터미널
//   claude 는 지금도 그대로 받는다), 이건 **입력이 무엇으로 해석되는가** 라는 다른 축이다. 그 결정이 지키던
//   축(capability ≠ 멤버십)의 봉인은 `roster_includes_a_terminal_agent_without_a_turn_signal_no_claude`.
// ★게이트 생략은 여기서 검증되지 않는다(뮤테이션 실측 — 착각 금지)★: 방어선은 전부 커널에 있다
//   (`messaging/src/service.rs`): busy 반쪽 = `a_live_agent_without_a_turn_signal_is_injected_with_no_gate` ·
//   큐 백로그 반쪽 = `inject_failure_parks_pending_without_a_turn_signal`.
#[tokio::test]
async fn control_send_shell_recipient_is_not_a_mail_recipient() {
    let (manager, registry, base, data_dir, handle, messaging, _busy) =
        wire("no-turn-signal").await;
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "valid-sender".to_string(), true);

    let mut profile = AgentProfile::new(
        "sheller".to_string(),
        AgentCommand::Shell {
            program: engram_dashboard_agent::manager::default_shell().to_string(),
            args: vec![],
        },
        std::path::PathBuf::from("."),
        vec![],
        false,
    );
    profile.display_name = Some("sheller".to_string());
    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("shell spawn")
        .into_started()
        .expect("이 호출은 실제로 띄운다(중복 요청 아님)");
    assert!(wait_until(Duration::from_secs(3), || manager
        .list_agents()
        .iter()
        .any(|a| a.id == info.id)));

    let (_s, body) = post_send(&base, Some("valid-sender"), "sheller", "hi").await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["results"][0]["status"], "failed",
        "산 셸이어도 편지의 수신자는 아니다: {body}"
    );
    assert_eq!(
        v["results"][0]["code"], "RECIPIENT_NOT_FOUND",
        "명단에 없는 이름과 같은 결말 — 발신자는 주소를 고쳐 다시 보내면 된다: {body}"
    );
    assert_eq!(
        messaging.parked_len("sheller"),
        0,
        "파킹도 되면 안 된다(깨어날 일 없는 이름 앞에 편지가 쌓인다): {body}"
    );

    manager.kill_agent(info.id).ok();
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── 로스터 술어 봉인: terminal 상태 세션은 **맵에 남아 있어도** 로스터에서 빠진다 — ADR-0116 결정 1 ──────
// ★왜 필요한가(뮤테이션 실측 — 리뷰 fix D9-a)★: `messaging_host::is_live` 의 상태 조건을 지워도 데몬 412
//   테스트가 전부 초록이었다. 4차 개정으로 그 조건이 **유일한 멤버십 게이트**가 됐으므로(capability 는 타이밍
//   축으로 내려갔다) 실물 어댑터 레벨에서 봉인한다.
#[tokio::test]
async fn roster_excludes_a_terminal_session_still_in_the_map() {
    use engram_dashboard_messaging::service::DeliveryPort;

    let (manager, _registry, _base, data_dir, handle, messaging, _busy) = wire("dead-roster").await;
    let dead = obs_seam::insert_terminal_seam_recipient(&manager, "corpse");
    let port = engram_dashboard_daemon::messaging_host::ManagerDeliveryPort::new(manager.clone());

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
#[tokio::test]
async fn control_send_relays_wrapped_line_to_json_agent() {
    let (manager, registry, base, data_dir, handle, _messaging, _busy) = wire("relay").await;

    let Some((b_info, _b_tok)) = spawn_json_agent(&manager, &registry, "bee") else {
        skip_no_claude("control_send_relays_wrapped_line_to_json_agent");
        let _ = std::fs::remove_dir_all(&data_dir);
        handle.shutdown().await;
        return;
    };

    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = Arc::new(EventCapture {
        id: SinkId::new_v4(),
        seen: seen.clone(),
    });
    manager.subscribe(b_info.id, sink).expect("subscribe B");

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "relay-sender".to_string(), true);

    let (status, body) = post_send(&base, Some("relay-sender"), "bee", "ping-body-XYZ").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).expect("json ACK");
    assert_eq!(v["results"][0]["status"], "delivered", "배달 성공: {body}");
    assert_eq!(v["results"][0]["to"], "bee", "해석된 수신자 이름 동봉");
    assert!(v["id"].is_string(), "msg-id 동봉");

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

// ── 폐기된 발신자여도 메시지는 배달된다(생존은 게이트 아님·기록용 관측만) — handle_send 직접 호출로 격리 ──
// ★사용자 결정 2026-07-19★: 메시지 유효성은 **작성 시점 인증**(입구 auth)으로 이미 성립한다. 발신자가
//   그 뒤 죽거나 회전돼도(토큰 revoke) 메시지는 무효가 되지 않는다 — "결과 보내고 종료"(유언 패턴)는
//   멀티에이전트 핵심 패턴이고 미래 메일박스 커밋 시맨틱과도 정합한다. is_identity_live 는 배달을 막지
//   않고 forensic 로그만 남긴다.
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

    let Some((b_info, _b_tok)) = spawn_json_agent(&manager, &registry, "target-b") else {
        skip_no_claude("control_send_revoked_sender_still_delivers_observation");
        let _ = std::fs::remove_dir_all(&data_dir);
        handle.shutdown().await;
        return;
    };

    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = Arc::new(EventCapture {
        id: SinkId::new_v4(),
        seen: seen.clone(),
    });
    manager.subscribe(b_info.id, sink).expect("subscribe B");

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "sender-tok".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };
    registry.revoke(sender, 0);

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
//   **structured=true 캐리어를 흉내 내되 write 성공/실패를 우리가 정하는** 세션을 맵에 직접 꽂는다 —
//   claude 없이 handle_send 의 성공/실패 두 갈래를 모두 실측한다.
mod obs_seam {
    use std::sync::atomic::AtomicU8;
    use std::sync::{Arc, Mutex};

    use engram_dashboard_agent::backend::InputEncoder;
    use engram_dashboard_agent::manager::AgentManager;
    use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
    use engram_dashboard_agent::session::AgentSession;
    use engram_dashboard_agent::transport::AgentTransport;
    use engram_dashboard_agent::types::{
        AgentId, AgentStatus, BackendCaps, ControlCaps, InputCaps, InputEvent, ModelCaps,
        OutputCaps, PtyError, SessionCaps, StatusSink, TransportCaps,
    };

    struct NoopStatus;
    impl StatusSink for NoopStatus {
        fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
        fn agent_list_updated(&self, _a: Vec<engram_dashboard_agent::types::AgentInfo>) {}
    }

    /// `structured`: 이 캐리어가 구조화 출력(= 턴 신호)을 내는가. **기본은 true(json claude 대역)**이고,
    ///   false 는 터미널 claude 대역 — 그 부류가 명단에 남는지가 ADR-0116 결정 1·7 의 회귀 축이다.
    struct SeamTransport {
        fail: bool,
        structured: bool,
        captured: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl AgentTransport for SeamTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            if self.fail {
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
                output: OutputCaps {
                    terminal_bytes: false,
                    structured: self.structured,
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

    pub fn insert_seam_recipient(
        manager: &Arc<AgentManager>,
        fail: bool,
    ) -> (AgentId, Arc<Mutex<Vec<Vec<u8>>>>) {
        let id = AgentId::new_v4();
        let name = id.to_string()[..8].to_string();
        insert_seam_recipient_named(manager, fail, id, &name)
    }

    /// ★왜 실 종료로는 못 만드나★: 실제 종료는 reaper 가 세션을 맵에서 **곧바로 제거**한다. 그래서
    ///   "list_agents 엔 있는데 상태는 terminal" 이라는 상태를 실 세션으로 재현할 수 없다.
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
        core.finish(engram_dashboard_agent::types::TerminalReason::Killed);
        let session = Arc::new(AgentSession::new(
            id,
            std::path::PathBuf::from(format!("seam-root/{name}")),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            backend_caps(),
            InputEncoder::ClaudeStreamJson,
            true,
            core,
            Box::new(SeamTransport {
                fail: false,
                structured: true,
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
        let (agent, captured, _core) = insert_seam_with_core(
            manager,
            fail,
            id,
            name,
            core,
            InputEncoder::ClaudeStreamJson,
            true,
        );
        (agent, captured)
    }

    /// 수신자의 **캐리어 종류를 지정**하는 판 — 터미널 claude 대역(`Raw` + 턴 신호 없음)과 json claude
    /// 대역(`ClaudeStreamJson` + 턴 신호 있음)을 한 테스트에서 대조하려고 둔다(다른 헬퍼는 json 고정).
    ///
    /// ★두 축을 encoder 하나로 묶는 이유★: 실물에서도 같이 움직인다 — 터미널 claude 는 Raw 입력에
    ///   구조화 출력이 없고, json claude 는 그 반대다. 따로 받으면 실재하지 않는 조합을 만들 수 있다.
    pub fn insert_seam_recipient_with_encoder(
        manager: &Arc<AgentManager>,
        encoder: InputEncoder,
    ) -> (AgentId, Arc<Mutex<Vec<Vec<u8>>>>) {
        let id = AgentId::new_v4();
        let name = id.to_string()[..8].to_string();
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        let structured = encoder == InputEncoder::ClaudeStreamJson;
        let (agent, captured, _core) =
            insert_seam_with_core(manager, false, id, &name, core, encoder, structured);
        (agent, captured)
    }

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
            engram_dashboard_agent::backend::turn_classifier(
                &engram_dashboard_agent::profile::AgentCommand::Claude {
                    extra_args: vec![],
                    output_format: engram_dashboard_agent::profile::ClaudeOutputFormat::StreamJson,
                },
            ),
        );
        insert_seam_with_core(
            manager,
            fail,
            id,
            &name,
            core,
            InputEncoder::ClaudeStreamJson,
            true,
        )
    }

    fn insert_seam_with_core(
        manager: &Arc<AgentManager>,
        fail: bool,
        id: AgentId,
        name: &str,
        core: Arc<OutputCore>,
        encoder: InputEncoder,
        structured: bool,
    ) -> (AgentId, Arc<Mutex<Vec<Vec<u8>>>>, Arc<OutputCore>) {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let core_out = core.clone();
        // ADR-0101 (WYSIWYA): canonical name = display_name ?? basename(session.cwd) 이고 seam 세션엔
        //   프로필(=display_name)이 없다. 그래서 cwd 의 basename 이 곧 이 세션의 addressable name 이 된다.
        //   테스트는 fallback_name(id)=id[:8] 로 지목하므로, cwd basename 을 id[:8] 로 맞춰 "보이는 이름
        //   = 주소" 를 성립시킨다(옛 cwd="." 는 basename="." 이라 지목 불가·동명 충돌).
        let cwd = std::path::PathBuf::from(format!("seam-root/{name}"));
        let session = Arc::new(AgentSession::new(
            id,
            cwd,
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            backend_caps(),
            encoder,
            // seam 수신자는 claude 대역이라 우편 자격 true — 셸 축은 실 spawn 테스트가 덮는다
            //   (`messaging_host::tests::a_live_shell_is_excluded_from_the_mail_roster_but_still_counts_as_live`).
            true,
            core,
            Box::new(SeamTransport {
                fail,
                structured,
                captured: captured.clone(),
            }),
        ));
        manager.insert_test_session(session);
        (id, captured, core_out)
    }

    pub fn last_written(captured: &Arc<Mutex<Vec<Vec<u8>>>>) -> Vec<u8> {
        captured.lock().unwrap().last().cloned().unwrap_or_default()
    }

    pub fn all_written(captured: &Arc<Mutex<Vec<Vec<u8>>>>) -> Vec<Vec<u8>> {
        captured.lock().unwrap().clone()
    }

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
#[tokio::test]
async fn control_send_delivery_observation_via_seam_no_claude() {
    use engram_dashboard_agent::backend::InputEncoder;
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("obs-seam-ok").await;

    let (b_id, captured) = obs_seam::insert_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "seam-ok-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

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

    assert_eq!(obs.msg_id, ack_id, "레코드 msg_id = ACK id(상관 축 1)");
    assert!(
        obs.msg_uuid.is_some(),
        "성공 배달은 msg_uuid 를 담아야(상관 축 2)"
    );

    let sender_name = obs_seam::fallback_name(sender);
    let expected_wrapped = obs_seam::expected_default_envelope(&sender_name, body);
    let expected_bytes = expected_wrapped.len();
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
    //   + handoff 잘림/오염 탐지).
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

// ── ADR-0116 결정 1·7 봉인: 턴 신호 없는 터미널 에이전트도 **명단에 남는다** ─────────────────────────
// ★막는 회귀★: 어댑터 산출 함수에 `.filter(|a| a.capabilities.output.structured)` 를 되살리면 **터미널
//   claude 전원이 조용히 편지를 못 받는다**(파킹 계기조차 없어 24h TTL 로 만료). 그 부류는 실 claude
//   바이너리 없이는 spawn 할 수 없어, 세션 주입 seam 으로 같은 모양(Raw 입력 + 구조화 출력 없음)을 만든다.
// ★셸 제외(`reads_messages`)와 다른 축이다★: 이건 capability 축, 그건 "입력이 무엇으로 해석되는가" 축.
//   한 함수에 두 필터가 있으므로 **둘 다** 회귀 커버가 필요하다(셸 축 = messaging_host 실 spawn 테스트).
#[tokio::test]
async fn roster_includes_a_terminal_agent_without_a_turn_signal_no_claude() {
    use engram_dashboard_agent::backend::InputEncoder;
    use engram_dashboard_daemon::messaging_host::ManagerDeliveryPort;
    use engram_dashboard_messaging::service::DeliveryPort;

    let (manager, _registry, _base, data_dir, handle, _messaging, _busy) =
        wire("no-signal-roster").await;
    let port = ManagerDeliveryPort::new(manager.clone());

    let (id, _captured) = obs_seam::insert_seam_recipient_with_encoder(&manager, InputEncoder::Raw);

    let entry = port
        .live_agents()
        .into_iter()
        .find(|a| a.id == id)
        .expect("턴 신호가 없어도 배달 명단에 있어야(capability 는 멤버십 축이 아니다)");
    assert!(
        !entry.turn_signal,
        "그 사실은 turn_signal=false 로만 나타난다(= 게이트 없이 즉시 주입 대상): {entry:?}"
    );
    let sources = port.addressing_sources();
    assert!(
        sources.roster.iter().any(|a| a.id == id && !a.turn_signal),
        "입구 판정 소스도 같은 술어여야(한쪽만 걸러도 그 부류가 통째로 막힌다): {sources:?}"
    );

    manager.kill_agent(id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── 터미널(TUI) 수신자에게는 제출(CR)이 **별도 write** 로 한 번 더 나간다 ───────────────────────────
// ★결함 재발 방지 축 = write 경계★: 봉투 바이트는 예전에도 PTY 에 도착했지만 claude TUI 가 그걸 입력창에
//   담아 둔 채 턴을 시작하지 않았다(실측 2026-08-17 — `본문+CR` 한 write 는 제출 안 됨, 두 write 는 제출됨).
//   그래서 "바이트가 갔나" 가 아니라 **"write 가 두 번 갈렸나"** 를 단언한다.
// ★수신자를 하나씩 따로 보내는 이유★: 수신자가 둘이면 봉투에 `to` 속성이 붙어 기대 문자열을 재구성하기
//   어렵다. 하나씩이면 양쪽 다 **바이트 정확 일치**로 못 박을 수 있고, 그래야 "무엇이든 추가되면 걸리는"
//   단언이 된다(약한 포함 검사와 달리).
#[tokio::test]
async fn control_send_adds_a_separate_submit_write_only_for_terminal_recipients_no_claude() {
    use engram_dashboard_agent::backend::InputEncoder;
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("submit-split").await;

    let (term_id, term_captured) =
        obs_seam::insert_seam_recipient_with_encoder(&manager, InputEncoder::Raw);
    let (json_id, json_captured) =
        obs_seam::insert_seam_recipient_with_encoder(&manager, InputEncoder::ClaudeStreamJson);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "submit-split-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };
    let body = "hi";
    let envelope = obs_seam::expected_default_envelope(&obs_seam::fallback_name(sender), body);

    let send_to = |id| {
        let v = handle_send(
            &manager,
            &registry,
            &messaging,
            Entrance::Cli,
            ControlCommand {
                from,
                to: vec![obs_seam::fallback_name(id)],
                body: body.to_string(),
                contract: Default::default(),
            },
        )
        .to_json();
        assert_eq!(v["results"][0]["status"], "delivered", "{v}");
    };
    send_to(term_id);
    send_to(json_id);

    assert_eq!(
        obs_seam::all_written(&term_captured),
        vec![envelope.as_bytes().to_vec(), b"\r".to_vec()],
        "터미널 수신자 = 봉투 그대로 1회 + 제출(CR) 단독 1회, **분리된 두 write**"
    );

    // json 쪽은 그 배달의 msg_uuid 로 기대 라인을 재구성해 바이트 정확 일치를 본다 — 제출 write 가 새거나
    // 봉투에 문자가 덧붙으면 여기서 깨진다(종전 동작 봉인).
    let json_uuid = seen
        .lock()
        .unwrap()
        .iter()
        .find(|o| o.to_id == json_id)
        .and_then(|o| o.msg_uuid)
        .expect("json 배달의 관측 레코드 + msg_uuid");
    assert_eq!(
        obs_seam::all_written(&json_captured),
        vec![InputEncoder::ClaudeStreamJson.encode(envelope.as_bytes(), json_uuid)],
        "json 수신자는 종전 그대로 인코더 산출물 1회뿐이어야(종단 \\n 이 이미 제출)"
    );

    manager.kill_agent(term_id).ok();
    manager.kill_agent(json_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── ADR-0088 확장(리뷰 F1/F2): DeliveryObservation.in_reply_to — 구조화 메타에서만 파생 ───────────
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
    registry.issue(sender, 0, "obs-reply-sender".to_string(), true);
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
    registry.issue(sender, 0, "obs-plain-sender".to_string(), true);
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
        //   불변식이 실제로 핀 되고, 텍스트 파싱 회귀가 재발하면
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
// 관측을 켰다는 이유로 ACK 가 유실되면 발신자 재시도 → 중복 배달 — 그 회귀를 막는다.
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
    registry.issue(sender, 0, "panic-sender".to_string(), true);
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
#[tokio::test]
async fn control_send_delivery_failure_observation_records_error_not_success() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("obs-fail").await;

    // ★fail=true★: relay write 를 Err 로 만들어 handle_send 의 Err 갈래를 강제한다.
    let (b_id, _captured) = obs_seam::insert_seam_recipient(&manager, true);
    let to_name = obs_seam::fallback_name(b_id);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "fail-sender".to_string(), true);
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
    assert_eq!(
        v["results"][0]["status"], "pending",
        "write 실패는 파킹(pending)으로 전환(반려 아님, spec §5): {v}"
    );

    let obs = {
        let g = seen.lock().unwrap();
        assert_eq!(g.len(), 1, "실패 relay 1건 → 관측 레코드 1건: {:?}", *g);
        g[0].clone()
    };

    assert!(
        obs.error.is_some(),
        "실패 배달은 error=Some 이어야(성공으로 삼키지 않음): {obs:?}"
    );
    assert_eq!(obs.bytes_written, None, "실패면 bytes_written=None");
    assert_eq!(obs.msg_uuid, None, "실패면 msg_uuid=None(write 안 됨)");
    assert!(!obs.is_delivered(), "실패는 is_delivered()==false");
    assert!(
        obs.bytes_requested > body.len(),
        "실패 레코드도 요청 바이트(봉투 크기)는 실려야: req={} body={}",
        obs.bytes_requested,
        body.len()
    );
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

    let Some((b_info, _b_tok)) = spawn_json_agent(&manager, &registry, "obs-target") else {
        skip_no_claude("control_send_delivery_observation_records_bytes_and_correlated_ids");
        let _ = std::fs::remove_dir_all(&data_dir);
        handle.shutdown().await;
        return;
    };

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "obs-sender-tok".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

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

    let obs = {
        let g = seen.lock().unwrap();
        assert_eq!(g.len(), 1, "성공 relay 1건 → 관측 레코드 1건: {:?}", *g);
        g[0].clone()
    };

    assert_eq!(
        obs.msg_id, ack_id,
        "레코드 msg_id 는 ACK id 와 같아야(상관 축 1)"
    );
    assert!(
        obs.msg_uuid.is_some(),
        "성공 배달은 correlated msg_uuid 를 담아야(상관 축 2)"
    );
    let sender_name = sender.to_string()[..8].to_string();
    let expected_wrapped = obs_seam::expected_default_envelope(&sender_name, body);
    assert_eq!(
        obs.bytes_requested,
        expected_wrapped.len(),
        "요청 바이트 = 봉투의 정확 UTF-8 바이트 수(멀티바이트 관통): got={} wrapped={:?}",
        obs.bytes_requested,
        expected_wrapped
    );
    assert_eq!(
        obs.bytes_written,
        Some(obs.bytes_requested),
        "by-construction 복사(bytes_written = 요청) — short-write 탐지 아님"
    );
    assert!(obs.error.is_none(), "성공 배달은 error None");
    assert!(obs.is_delivered(), "is_delivered() = true(전송 완결)");
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
// seam 관측 범위 = handle_send → registry → submit_stdin_observed → session.submit_input_observed
//   (봉투 조립·encoder) → SeamTransport.send_input **까지**. 그 아래 물리 계층(운영
//   `StdioTransport::send_input` 의 `stdin.lock()` + `write_all`/`flush`)은 이 seam 이 **우회**한다 —
//   SeamTransport 는 이미 완결된 Vec 을 받아 `push` 로 원자 캡처하므로.
// 운영 동작이 오라클을 위반하면 테스트를 약화하지 않고 실패로 남겨 FINDING 으로 보고한다(마스킹 금지).

/// ── ADR-0088 Stage 1-오라클 1: 동시 **입구** exact-once + N 개 distinct 본체 무결 배달(seam handoff) ──
/// ★증명하지 않는다(커버리지 공백)★:
///   (1) **물리 OS-pipe 바이트 무인터리브** — 그 직렬화는 이 seam 이 우회하는 `stdin.lock()` 계층
///       소관이라, 그 응용계층 직렬화를 지우는 회귀는 여기서 **안 잡힌다**. ▶ core 크레이트
///       `tests/stdio_physical_pipe.rs` :: `physical_pipe_concurrent_sends_no_interleave` 가 커버한다.
///   (2) **encoder 내부 정확성** — actual·expected 가 같은 encoder 를 쓰므로 encoder 자체 결함(예:
///       wrap_user_turn 이 개행을 빠뜨림)은 양쪽을 똑같이 오염시켜 여기선 안 걸린다. ▶ claude.rs 의
///       golden unit test `wrap_user_turn_exact_line_and_newline_terminated` 가 커버한다.
#[tokio::test]
async fn stage1_concurrent_sends_exact_once_distinct_bodies_intact_at_seam() {
    use engram_dashboard_agent::backend::InputEncoder;
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;
    use std::sync::Barrier;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("stage1-concurrency").await;

    // 하나의 seam 수신자(fail=false → 성공 경로).
    let (b_id, captured) = obs_seam::insert_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    // 모든 스레드가 같은 신원으로 보낸다 — 수신자 1개에 몰아치는 게 요점.
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "stage1-conc-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    const N: usize = 100;
    // 마커에 특수문자가 없어야 JSON escape 를 피해 봉투 문자열이 캡처 라인에 부분열로 그대로 들어간다.
    //   idx zero-pad 는 부분열 오검(1 ⊂ 10) 방지.
    let markers: Vec<String> = (0..N).map(|i| format!("BODY-{i:04}")).collect();

    // ★Barrier(입구 정렬)★: N 스레드를 handle_send **진입 직전**에 모아 "초반 스레드가 후반 스레드 spawn
    //   전에 끝나 race window 가 안 열리는" 문제를 없앤다. ★한계★: 진입 정렬만 보장할 뿐 handle_send
    //   **내부의 실행 겹침**까지는 강제하지 못한다(단일코어/스케줄러가 여전히 직렬화 가능).
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
            barrier.wait();
            let cmd = ControlCommand {
                from,
                to: vec![to],
                body,
                contract: Default::default(),
            };
            let result = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
            let v = result.to_json();
            assert!(
                v.get("results").is_some(),
                "동시 발화도 각기 접수(results): {v}"
            );
            v["id"].as_str().expect("msg-id").to_string()
        }));
    }
    let ack_ids: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    // ★전원 `delivered` 를 단언으로 승격시키지 말 것★: 겹친 드레인에서 물러난 발신은 `pending` 으로
    //   답하고(ADR-0125), 그 편지는 이긴 쪽 배치나 되울린 도어벨로 뒤늦게 나간다. 그래서 단언 축은
    //   차수와 무관하게 유실·중복 없음 + 봉투 바이트 무결이다.
    for _ in 0..600 {
        if seen.lock().unwrap().len() >= N && messaging.parked_len(&to_name) == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

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

    let sender_name = obs_seam::fallback_name(sender);
    let writes = obs_seam::all_written(&captured);
    assert_eq!(
        writes.len(),
        N,
        "캡처된 write 수 == N(각 send_input 이 완결 봉투 1개 — 잘림/합병 없음)"
    );
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

    let mut matched_uuids: std::collections::HashSet<uuid::Uuid> = std::collections::HashSet::new();
    let mut received_bodies: Vec<String> = Vec::with_capacity(N);
    for (i, w) in writes.iter().enumerate() {
        let s = std::str::from_utf8(w)
            .unwrap_or_else(|e| panic!("write[{i}] 가 온전한 UTF-8 이 아님: {e}"));
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
        let hits: Vec<&String> = markers.iter().filter(|m| s.contains(m.as_str())).collect();
        assert_eq!(
            hits.len(),
            1,
            "write[{i}] 는 봉투 마커 정확히 1개만 담아야(seam 레벨 무합병) — 관측: {hits:?}"
        );
        let body = hits[0];
        received_bodies.push(body.clone());
        // ★정확-바이트 재구성★: 같은 encoder·같은 msg_uuid 로 만든 이 라인이 곧 session 이
        //   send_input 에 넘긴 바이트다 — 그래서 exact-eq 가 handoff 오라클로 성립한다.
        let wrapped = obs_seam::expected_default_envelope(&sender_name, body);
        let expected_line = InputEncoder::ClaudeStreamJson.encode(wrapped.as_bytes(), line_uuid);
        assert_eq!(
            w, &expected_line,
            "write[{i}] 가 기대 encoded 봉투와 바이트-정확 일치해야(session→transport handoff 잘림/오염/합병 탐지 — encoder 내부 정확성 아님): body={body} msg_id={}",
            obs.msg_id
        );
    }
    assert_eq!(
        matched_uuids.len(),
        N,
        "N 개 msg_uuid 전부 정확히 1 write 로 배달(exact-once, 다중집합 등식)"
    );

    // ★치환 버그 차단(FIX-1)★: 위 exact-bytes 재구성은 body 를 그 write 자신에서 뽑아 검사하므로
    //   "모든 메시지 → 같은 body" 치환 버그를 자기일관적으로 통과시킨다(각 write 가 BODY-0000 을 담고
    //   BODY-0000 으로 재구성 → 통과). 발신 마커 집합과의 직접 대조가 그 구멍을 막는다.
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
/// ★캡처 write 를 직접 대조하는 이유(FIX-2)★: `bytes_requested` 는 encoding **이전** 봉투 복사값이라
///   session→transport handoff 에서의 truncation·오염을 못 잡는다.
#[tokio::test]
async fn stage1_body_size_boundary_bytes_not_chars() {
    use engram_dashboard_agent::backend::InputEncoder;
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    const MAX: usize = 64 * 1024; // = MAX_BODY_BYTES(ingress 상수 — 여기 미러; 값 드리프트 시 아래가 잡는다).

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("stage1-boundary").await;

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "stage1-boundary-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    // 매 케이스마다 fresh seam 수신자 + fresh observer 를 심어 상태 누적을 피한다.
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
        // ★MAX 게이트는 body 기준★: 64 KiB 는 body 길이라 게이트를 통과하고, 봉투 wrapper 는 그 위에 얹힌다.
        let sender_name = obs_seam::fallback_name(from.agent_id);
        let expected_env_bytes = obs_seam::expected_default_envelope(&sender_name, &body).len();
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
    assert_eq!(
        written, expected_line,
        "멀티바이트 straddle: 캡처 write 가 기대 encoded 봉투와 바이트-정확 일치(handoff 무결)"
    );
    manager.kill_agent(b_id).ok();

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// ── ADR-0088 Stage 1-오라클 3(a) → ADR-0111 갱신: 수신자 부재 → **실패 행** + 배달 관측 없음 ──────────
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
    registry.issue(sender, 0, "stage1-absent-sender".to_string(), true);
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
// ★multi_thread 런타임이 필요한 이유★: 아래 `wait_until` 은 std::thread::sleep 로 블록하는 **동기**
//   폴링이라, current-thread 런타임에선 이 test task 가 스레드를 붙잡고 도는 동안 flush worker task 가
//   폴링될 틈이 없어 flush 가 진행되지 않는다. (`wait_until` 을 async 폴링으로 바꾸면 default 런타임도
//   가능하나 이 헬퍼는 다수 동기 테스트가 공유하므로 런타임 flavor 로만 격리한다.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c1_park_then_spawn_auto_delivers() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, busy) = wire("c1-park-spawn").await;

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c1-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    // ── 1) 실 에이전트를 먼저 띄운다 ────────────────────────────────────────────────────
    // ★ADR-0111 결정 1 로 setup 이 바뀌었다★: 옛 판은 "아직 안 뜬 이름으로 보내 파킹 → 나중에 스폰" 이었고,
    //   부재가 **입구 반려(실패 행)** 가 되며 그 진입로가 없어졌다. 이 테스트가 지키는 건 부재가 아니라
    //   **"파킹된 메일이 flush 계기에 자동으로 배달된다"** 는 배선이라, 파킹 사유만 남은 유일한 경로
    //   (busy = 턴 진행 중)로 바꿨다 — 관측 대상(idle diff → flush 워커 → 주입 → 관측)은 그대로다.
    // ★미커버 축의 정직한 표기(리뷰 C1)★: 그 교체로 **로스터 등장 diff → flush**(MessagingFlushSink 가
    //   agent_list_updated 를 보고 그 이름 큐를 여는 배선) 축은 **통합 경로로는** 아무 테스트도 안 덮는다 —
    //   양 끝만 단위로 덮인다(데몬 `flush_sink_appears_*` · 커널 `flush_on_appearance_*` 는 `flush_for` 를
    //   직접 부른다). 되찾으려면 "산 수신자에게 파킹 → 그 에이전트의 epoch 교체(재활성화)" 로 등장 diff 를
    //   만드는 데몬 레벨 테스트가 필요하다(백로그).
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
// ★왜 claude 없이 결정적인가★: 턴 이벤트를 수신자 core 에 **직접** emit 하므로 실 claude 턴의 타이밍
//   (응답 지연·인증)에 의존하지 않는다.
// ★multi_thread 런타임★: c1 테스트와 같은 사유(동기 폴링이 현재 스레드를 붙잡으면 worker 가 못 돈다).
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
    registry.issue(sender, 0, "c2-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let (b_id, captured, core) = obs_seam::insert_observed_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    // core.emit 은 운영에서 pump 가 쓰는 것과 **같은 진입점**이다 — 그래서 위 전 구간이 그대로 탄다.
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
    // 장부는 실제 주입 시점에만 delivered(ADR-0104).
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
//   seed-before-pump), 그 transcript 가 **턴 중간에 끊긴** 것이면 진행 신호로 끝나고 종료 신호가 없다.
//   그걸 관측으로 먹이면 (id, epoch) 가 "턴 중" 으로 찍히는데 그 턴의 종료는 **영원히 오지 않는다** → 그
//   수신자 앞 모든 발송이 TTL 까지 파킹된다(깨울 수 없는 false-busy = 배달 정지).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_a_resumed_transcript_never_bootstraps_busy() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, busy) = wire("c2-seed").await;

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c2-seed-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let (b_id, captured, core) = obs_seam::insert_observed_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    // seed 픽스처 = 죽은 incarnation 의 transcript("턴 중간에서 끝난 과거" — 진행 신호만, 종료 신호 없음).
    core.seed(vec![OutputEvent::TextDelta {
        text: "past mid-turn".to_string(),
        turn_id: None,
        message_id: None,
    }]);

    assert!(
        !busy.is_busy(b_id, 0),
        "재개 transcript 로 busy 를 부트스트랩하면 그 busy 는 깨울 수 없다(TTL 까지 배달 정지)"
    );

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
// ★왜 데몬 레벨인가★: 단위 테스트는 코어 안에서만 보므로, 어댑터·게이트(매니저 표 → ManagerTurnFacts
//   → BusyPolicy)가 코어와 **같은 표**를 보는지는 여기서만 잡힌다.
// ★막는 회귀★: 턴 중에 죽은 화신의 "턴 중" 이 남으면 ① 그 이름 앞 파킹이 영영 안 풀리고 ② 상한 sweep 이
//   죽은 에이전트에게 60초마다 도어벨을 울린다(프로세스 수명 내내).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_a_recipient_that_dies_mid_turn_leaves_no_busy_ghost() {
    use engram_dashboard_agent::types::TerminalReason;

    let (manager, _registry, _base, data_dir, handle, _messaging, busy) = wire("c2-ghost").await;
    let (b_id, _captured, core) = obs_seam::insert_observed_seam_recipient(&manager, false);

    core.emit(OutputEvent::TextDelta {
        text: "thinking...".to_string(),
        turn_id: None,
        message_id: None,
    });
    assert!(busy.is_busy(b_id, 0), "전제: 게이트가 턴 중으로 본다");

    // 턴 종료 신호 없는 비정상 종료 = 상한 sweep 이 원래 다루던 그 모양.
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
// ★이 축이 더하는 것★: busy 관측이 **실 claude decoder** 가 만든 이벤트로 일어난다(합성 emit 아님) —
//   "capability 프록시(structured = 턴 이벤트 있음)" 가 실제로 성립하는지의 실측이다.
// ★단언이 비대칭인 이유★: "턴 중 발송 → pending" 은 hard assert 다. 반면 **턴 종료 후 배달**은 claude 가
//   실제로 `result` 라인을 내야 하므로(인증·네트워크·모델 지연) 관측되면 단언하고 시간 내 안 오면 loud
//   경고로 넘어간다 — `ENGRAM_TEST_REQUIRE_CLAUDE=1` 레인에선 panic 으로 승격한다.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_live_mid_turn_send_parks_and_delivers_after_turn_end() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, busy) = wire("c2-live").await;

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c2-live-sender".to_string(), true);
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

    // 입력 전 claude 는 조용하다 — system/init 은 decoder 가 흘린다.
    //   ★이 단언 하나로는 위약★: "게이트가 아예 없어서 통과" 와 구별되지 않는다. 관측 배선이 살아
    //   있다는 증거는 첫 주입 뒤의 `is_busy` hard assert 가 진다.
    assert!(
        wait_until(Duration::from_secs(5), || !busy
            .is_busy(info.id, info.epoch)),
        "입력 전 수신자는 idle 로 관측돼야(턴 이벤트 없음)"
    );

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
/// ★증명하지 않는다(커버리지 공백)★: **실제 OS write 가 prefix 를 쓴 뒤 Err 를 내는 부분 배달** — 이 seam
///   은 send_input 이 push **전에** 통째로 Err 를 반환하므로(원자 all-or-nothing 모사) "prefix 만 쓰이고
///   실패" 상황 자체가 발생하지 않는다. ▶ core 크레이트 `tests/stdio_physical_pipe.rs` ::
///   `physical_pipe_partial_write_then_err_surfaces_as_err` 가 커버한다.
#[tokio::test]
async fn stage1_lifecycle_write_error_single_failure_no_partial_dup() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("stage1-write-err").await;

    // fail=true → 로스터에 든 산 수신자이나 send_input 이 Err.
    let (b_id, captured) = obs_seam::insert_seam_recipient(&manager, true);
    let to_name = obs_seam::fallback_name(b_id);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "stage1-write-err-sender".to_string(), true);
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
    assert_eq!(
        v["results"][0]["status"], "pending",
        "write 실패는 파킹(pending): {v}"
    );

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
    assert_eq!(
        obs.to_epoch, None,
        "실패 = to_epoch None(완결 write 없음 → attest 할 착지 incarnation 없음, 성공 필드 누출 없음)"
    );
    assert!(!obs.is_delivered(), "실패 = !is_delivered()");
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
/// ★설계 의도★: 메일은 논리 에이전트(안정 주소)를 향하므로 epoch pinning 을 **하지 않는다**(ADR-0086 §F5).
#[tokio::test]
async fn stage1_lifecycle_epoch_rotation_delivers_to_current_incarnation() {
    use engram_dashboard_agent::backend::InputEncoder;
    use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
    use engram_dashboard_agent::session::AgentSession;
    use engram_dashboard_agent::transport::AgentTransport;
    use engram_dashboard_agent::types::{
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
        fn agent_list_updated(&self, _a: Vec<engram_dashboard_agent::types::AgentInfo>) {}
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
            true,
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

    let old_buf = insert_epoch(&manager, id, 0);
    // 같은 AgentId 로 교체 주입 = 재시작(새 화신 표식) 모사 — `insert_test_session` 은 같은 id 를 덮으므로
    //   맵엔 이제 B(표식 1)만 남는다. 두 값의 대소에는 뜻이 없다(표식은 난수 — `AgentProfile::epoch`).
    let new_buf = insert_epoch(&manager, id, 1);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "stage1-epoch-sender".to_string(), true);
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
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "교체된 현재 incarnation 으로 배달돼야(유실 없음, ADR-0086 §F5): {v}"
    );

    let g = seen.lock().unwrap();
    assert_eq!(
        g.len(),
        1,
        "논리 메시지 1건 → 관측 레코드 1건(wrong-epoch 이중배달 없음): {:?}",
        *g
    );
    assert_eq!(g[0].to_id, id, "레코드 수신자 = 그 안정 AgentId");
    assert_eq!(
        g[0].to_epoch,
        Some(1),
        "레코드 to_epoch = 착지한 현재 incarnation(epoch 1) — 직접 단언"
    );
    drop(g);

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
/// ★증명한다★: handle_send 가 수신자를 해석(list_agents 스냅샷)한 **직후**, 수신자 주입 **직전**에
///   같은 AgentId 가 새 표식의 incarnation 으로 교체되면(재시작 모사), write 는 resolve 가 본
///   구 incarnation(표식 0)이 아니라 **write 해석 시점의** incarnation(표식 1)에 착지한다. (엄밀히는
///   get_session 이후 한 번 더 동시 교체가 끼면 그 사이 "직전"이 된 incarnation 에 바이트가 갈 수도 있다 —
///   그래도 to_epoch 는 실제 착지 incarnation 을 정확히 담으므로 record-self-sufficiency 는 유지된다.) 이 race 는 순차 교체
///   (오라클 5)가 아니라 resolve↔write **사이**의 진짜 경쟁이다 — 프로덕션 yield-seam
///   (ControlRegistry::set_mid_send_hook, feature=test-harness)이 write 직전에 hook 을 발화해 그 갭에
///   결정적으로 개입한다(스케줄러 타이밍 의존 없음). ★ADR-0086 §F5 = design-accepted★: 메일은 논리
///   에이전트(안정 주소)를 향하므로 새 incarnation 착지가 **올바른** 동작이다 — 유실 없음, 현재
///   incarnation 배달. 그 F5 설계 의도를 레코드의 `to_epoch == Some(1)` 로 **직접 입증**한다
///   (record-self-sufficient).
///
/// ★증명하지 않는다(정직 범위)★: 실 StdioTransport/실 claude 는 개입하지 않는다(seam 레벨 — EpochSeam 이
///   write 를 캡처만). 여기서 주입한 그 한 지점(write 직전) 외의 다른 스케줄링 race(예: reachability↔write
///   사이, 물리 OS-pipe 인터리브)는 다루지 않는다. 이 테스트가 잡는 것은 **resolve 스냅샷과 실제 write
///   착지 incarnation 이 어긋날 수 있고, 그때 write 가 current 로 가며 레코드가 그 사실을 자기충족적으로
///   담는다**는 계약이다.
#[tokio::test]
async fn stage1_lifecycle_mid_flight_epoch_race_lands_on_new_incarnation_deterministic() {
    use engram_dashboard_agent::backend::InputEncoder;
    use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
    use engram_dashboard_agent::session::AgentSession;
    use engram_dashboard_agent::transport::AgentTransport;
    use engram_dashboard_agent::types::{
        AgentId as CoreAgentId, AgentStatus, BackendCaps, ControlCaps, InputCaps, InputEvent,
        ModelCaps, OutputCaps, PtyError, SessionCaps, StatusSink, TransportCaps,
    };
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

    struct NoopStatus;
    impl StatusSink for NoopStatus {
        fn status_changed(&self, _id: CoreAgentId, _s: AgentStatus, _e: u32) {}
        fn agent_list_updated(&self, _a: Vec<engram_dashboard_agent::types::AgentInfo>) {}
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
            true,
            core,
            Box::new(EpochSeam {
                captured: captured.clone(),
            }),
        ));
        manager.insert_test_session(session);
        captured
    }

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("stage1-midflight").await;

    let id = CoreAgentId::new_v4();
    let to_name = obs_seam::fallback_name(id);

    let old_buf = insert_epoch(&manager, id, 0);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let new_buf_slot: Arc<Mutex<Option<Arc<Mutex<Vec<Vec<u8>>>>>>> = Arc::new(Mutex::new(None));
    let rotated = Arc::new(AtomicBool::new(false));
    // ★Arc 순환 차단★: registry 는 hook(클로저)을 저장하고, registry 는 manager-side wiring 이 전이 소유한다.
    //   여기서 hook 이 manager 를 Arc 로 강하게 잡으면 manager↔hook 참조 순환이 생겨, cleanup
    //   (set_mid_send_hook(None)) 전에 단언이 panic 하면 manager(와 reaper 스레드)가 프로세스 수명 내내
    //   누수된다. 그래서 Weak 로 잡고 발화 때 upgrade 한다.
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
        let Some(mgr_for_hook) = mgr_weak.upgrade() else {
            return;
        };
        let new_buf = insert_epoch(&mgr_for_hook, id, 1);
        *slot_for_hook.lock().unwrap() = Some(new_buf);
    })));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "stage1-midflight-sender".to_string(), true);
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
    assert_eq!(
        obs.to_epoch,
        Some(1),
        "write 는 교체된 현재 incarnation(epoch 1)에 착지 — resolve 시점(epoch 0)이 아님. to_epoch={:?}",
        obs.to_epoch
    );
    assert_eq!(obs.msg_id, ack_id, "레코드 msg_id = ACK id(상관 축 1)");
    assert!(
        obs.msg_uuid.is_some(),
        "성공 배달은 msg_uuid 를 담아야(상관 축 2)"
    );

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

    let Some((b_info, _b_tok)) = spawn_json_agent(&manager, &registry, "recv") else {
        skip_no_claude("mcp_send_message_tool_happy_and_error");
        let _ = std::fs::remove_dir_all(&data_dir);
        handle.shutdown().await;
        return;
    };

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "mcp-sender-tok".to_string(), true);

    let config = StreamableHttpClientTransportConfig::with_uri(handle.url.clone())
        .auth_header("mcp-sender-tok");
    let transport = StreamableHttpClientTransport::from_config(config);
    let client = ().serve(transport).await.expect("MCP handshake");

    let tools = client.list_all_tools().await.expect("list tools");
    assert!(
        tools.iter().any(|t| t.name == "send_message"),
        "tools 에 send_message: {:?}",
        tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>()
    );

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

async fn post_send_json(
    base: &str,
    bearer: Option<&str>,
    body: serde_json::Value,
) -> (reqwest::StatusCode, String) {
    post_control(base, "/control/send", bearer, body).await
}

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
    registry.issue(a_id, 0, "c3-a".to_string(), true);
    registry.issue(b_id, 0, "c3-b".to_string(), true);
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
    registry.issue(a_id, 0, "c3-timeout-a".to_string(), true);
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

    messaging.sweep(Instant::now() + Duration::from_secs(61));

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

    // 비동기 레인이라 "안 늘어남" 은 잠깐 기다린 뒤 확인해야 의미가 있다(즉시 확인하면 아직 안 온 걸
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
    registry.issue(sender, 0, "c3-args-sender".to_string(), true);
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

    // ★1분 미만 기한(리뷰 fix 7)★
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
    // 대조군: 정확히 1분(초 표기)은 수용.
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
    registry.issue(sender, 0, "c3-colon-sender".to_string(), true);
    let tok = Some("c3-colon-sender");

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
    registry.issue(sender_id, 0, "c4-all-token".to_string(), true);

    let (status, body) = post_send(&base, Some("c4-all-token"), "@all", "전원 리베이스 대기").await;
    assert_eq!(status, reqwest::StatusCode::OK, "방송 접수도 200: {body}");
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert!(
        v.get("status").is_none(),
        "성공 응답엔 최상위 status 없음(spec §6): {body}"
    );

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
    want.sort();
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

/// ★왜 실입구 테스트인가★: 이건 **판정들 사이의 불일치** 버그였다 — 그룹 갈래는 raw `to` 를
///   `starts_with('@')` 로 보고, 그룹 이름 정규화는 trim 한 값을 본다. 그래서 `" @all"` 은 단일 발송으로
///   흘러 "그런 이름의 에이전트 없음 → **부재 파킹**" 이 됐다: 발신자에겐 `pending` 성공으로 보이는데 실제로는
///   아무도 못 받고 TTL 에 소멸한다(공백 한 칸 뒤에 숨은 조용한 유실). 두 판정이 같은 문자열을 보는지는
///   입구를 실제로 태워야 증명된다.
#[tokio::test]
async fn c4_leading_whitespace_in_the_destination_does_not_change_routing() {
    let (manager, registry, base, data_dir, handle, messaging, _busy) = wire("c4-trim").await;

    let (a_id, a_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let a_name = obs_seam::fallback_name(a_id);
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c4-trim-token".to_string(), true);
    let tok = Some("c4-trim-token");

    // ① `" @all"` — 공백이 있어도 **그룹 갈래**로 간다(단일 발송이 아니라 멤버별 회계).
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

// ── D(spec §6): 조회 입구 — `messages` 가 MCP·CLI 두 입구에서 동일 JSON ──────────────────────
//
// ★왜 통합 테스트인가★: 두 입구(MCP 툴 · CLI HTTP 라우트)가 **같은 공통 핸들러**를 부른다는 게 이 증분의
//   핵심 계약(ADR-0086 entrance-agnostic)인데, 그건 배선을 실제로 태워야만 증명된다 — 단위 테스트는
//   핸들러 하나만 본다. 그래서 실 데몬(MCP 서버 + auth 미들웨어 + MessagingService)을 띄우고 두 경로의
//   응답 JSON 이 **동일한지** 직접 비교한다.
// ★claude 불요★: 조회는 자식 프로세스 stdin 을 건드리지 않는다(읽기뿐).

#[tokio::test]
async fn d_messages_reports_delivery_state_by_id_and_the_callers_open_items() {
    let (manager, registry, base, data_dir, handle, _messaging, _busy) = wire("d-messages").await;

    let (recv_id, _recv_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let recv_name = obs_seam::fallback_name(recv_id);
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "d-msg-tok".to_string(), true);
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
    registry.issue(other, 0, "d-msg-other".to_string(), true);
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
    registry.issue(recv_id, 0, "d-recv-tok".to_string(), true);
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "d-send-tok".to_string(), true);

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
    registry.issue(caller, 0, "d-parity-tok".to_string(), true);
    let tok = Some("d-parity-tok");

    let config = StreamableHttpClientTransportConfig::with_uri(handle.url.clone())
        .auth_header("d-parity-tok");
    let transport = StreamableHttpClientTransport::from_config(config);
    let client = ().serve(transport).await.expect("MCP handshake");

    // tools/list 노출 이름 = 프라이밍이 가르치는 이름과 같은 값.
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
    let twin_a = AgentId::new_v4();
    let twin_b = AgentId::new_v4();
    let (_a, _cap_a) = obs_seam::insert_seam_recipient_named(&manager, false, twin_a, "worker");
    let (_b, _cap_b) = obs_seam::insert_seam_recipient_named(&manager, false, twin_b, "worker");
    registry.issue(twin_a, 0, "d-twin-a".to_string(), true);
    registry.issue(twin_b, 0, "d-twin-b".to_string(), true);
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "d-twin-send".to_string(), true);

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
    registry.issue(sender, 0, "d-trunc-tok".to_string(), true);
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
