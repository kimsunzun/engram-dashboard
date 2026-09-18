//! `/control/hook` 통합 테스트 — 훅이 보낸 봉투가 **실 프로필에 앉는 것까지** 한 번에 잰다.
//!
//! ★실 프로세스가 하나도 없다★: 에이전트는 셸 백엔드 프로필로 명부에만 등록하고 띄우지 않는다. 훅도
//!   실 codex 도 뜨지 않는다 — 대신 CLI 가 만드는 것과 **같은 봉투**를 실 HTTP 로 민다.
//!
//! ★왕복이 두 조각으로 갈려 있고 그 이음매가 [`CLI_HOOK_SESSION_ID_FIELD`] 다★:
//!   - 「상대의 stdin JSON → 우리 봉투」는 CLI 쪽 단위 시험이 실측 페이로드로 잰다
//!     (`bin/engram.rs` 의 `the_measured_payload_becomes_a_one_field_envelope`).
//!   - 「우리 봉투 → 프로필」은 이 파일이 잰다.
//!   두 조각이 같은 상수로 칸 이름을 적으므로, 한쪽만 바뀌면 다른 쪽이 빨개진다. ★bin 은 라이브러리가
//!   아니라 이 파일에서 그 함수를 못 부른다 — 그래서 이음매를 상수로 둔 것이다.★

use std::sync::Arc;

use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::preset::{Preset, PresetRegistry, PresetStore};
use engram_dashboard_agent::profile::{
    AgentCommand, AgentOutputFormat, AgentProfile, ProfileRegistry, ProfileStore,
};
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, NoopControlChannel, StatusSink,
    CLI_HOOK_SESSION_ID_FIELD, CONTROL_HOOK_ROUTE,
};
use engram_dashboard_daemon::command_delivery::{BusSweeper, CommandBus, CommandDeliveries};
use engram_dashboard_daemon::command_roster::CommandRoster;
use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server, CommandTableSlot, ManagerSlot, McpServerHandle, MessagingSlot,
};
use engram_dashboard_daemon::control::registry::ControlRegistry;
use uuid::Uuid;

// ── 하네스 ────────────────────────────────────────────────────────────────────────

struct NoopSink;
impl StatusSink for NoopSink {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

#[derive(Default)]
struct MemProfileStore {
    saved: std::sync::Mutex<Vec<AgentProfile>>,
}
impl ProfileStore for MemProfileStore {
    fn save(&self, profiles: &[AgentProfile]) {
        *self.saved.lock().unwrap() = profiles.to_vec();
    }
    fn load(&self) -> Vec<AgentProfile> {
        self.saved.lock().unwrap().clone()
    }
}

struct MemPresetStore;
impl PresetStore for MemPresetStore {
    fn save(&self, _presets: &[Preset]) {}
    fn load(&self) -> Vec<Preset> {
        Vec::new()
    }
}

struct Fixture {
    manager: Arc<AgentManager>,
    registry: Arc<ControlRegistry>,
    base: String,
    _sweeper: BusSweeper,
    _handle: McpServerHandle,
}

impl Fixture {
    /// 훅이 미는 것과 같은 모양 — 칸 하나, 이름은 공유 상수.
    async fn report(&self, token: &str, sid: &str) -> (reqwest::StatusCode, serde_json::Value) {
        self.post(
            Some(token.to_string()),
            Some(serde_json::json!({ CLI_HOOK_SESSION_ID_FIELD: sid })),
        )
        .await
    }

    async fn post(
        &self,
        bearer: Option<String>,
        body: Option<serde_json::Value>,
    ) -> (reqwest::StatusCode, serde_json::Value) {
        let mut req = reqwest::Client::new()
            .post(format!("{}{CONTROL_HOOK_ROUTE}", self.base))
            .header("Content-Type", "application/json");
        if let Some(b) = body {
            req = req.json(&b);
        }
        if let Some(b) = bearer {
            req = req.header("Authorization", format!("Bearer {b}"));
        }
        let resp = req.send().await.expect("http request");
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let json = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
        (status, json)
    }
}

async fn fixture(tag: &str) -> Fixture {
    let registry = Arc::new(ControlRegistry::new());
    let manager_slot = Arc::new(ManagerSlot::new());
    let command_slot = Arc::new(CommandTableSlot::new());
    let (bus, sweeper) = CommandBus::new(
        CommandRoster::new(),
        CommandDeliveries::new(),
        Arc::new(
            engram_dashboard_daemon::control::commands::DaemonLocalCommands::new(
                command_slot.clone(),
            ),
        ),
    );
    let handle = start_mcp_server(
        registry.clone(),
        manager_slot.clone(),
        Arc::new(MessagingSlot::new()),
        command_slot.clone(),
        bus.clone(),
    )
    .await
    .unwrap_or_else(|e| panic!("start mcp server({tag}): {e}"));

    let control: Arc<dyn ControlChannel> = Arc::new(NoopControlChannel);
    let manager = Arc::new(AgentManager::new_with_control(
        Arc::new(NoopSink),
        Arc::new(ProfileRegistry::new(Arc::new(MemProfileStore::default()))),
        Arc::new(PresetRegistry::new(Arc::new(MemPresetStore))),
        Arc::new(SessionTracker::new(
            TrackerConfig {
                enabled: false,
                poll_interval: std::time::Duration::from_secs(1),
            },
            Arc::new(|_, _| {}),
        )),
        control,
    ));
    manager_slot.set(manager.clone());

    let base = handle
        .url
        .strip_suffix("/mcp")
        .expect("mcp url suffix")
        .to_string();
    Fixture {
        manager,
        registry,
        base,
        _sweeper: sweeper,
        _handle: handle,
    }
}

/// 띄우지 않는 프로필 하나 — 이 라우트는 세션이 아니라 **명부**를 건드린다.
///
/// ★백엔드를 인자로 받는 것이 이 하네스의 요점이다★: 이 입구의 자격 축은 **우리가 세션 id 를 발급하는
///   쪽인가**이고, 그 답은 백엔드마다 다르다. 리터럴로 박으면 자격 시험이 자기가 정한 답을 다시 읽는다.
fn seed_agent(manager: &Arc<AgentManager>, name: &str, command: AgentCommand) -> AgentId {
    let mut profile = AgentProfile::new(
        name.to_string(),
        command,
        std::env::temp_dir(),
        vec![],
        false,
    );
    profile.display_name = Some(name.to_string());
    manager.create_agent(profile).expect("등록 성공").id
}

/// 세션 id 를 **스스로 발급하는** 백엔드(= 우리가 안 정하는 쪽) — 이 입구가 받아 주는 쪽이다.
fn reports_its_own_session_id() -> AgentCommand {
    let c = AgentCommand::Codex {
        extra_args: vec![],
        output_format: AgentOutputFormat::Terminal,
    };
    assert!(
        !engram_dashboard_agent::backend::assigns_session_id(&c),
        "전제: 이 백엔드의 세션 id 는 우리가 발급하지 않는다"
    );
    c
}

/// ★우리가 세션 id 를 **발급하는** 백엔드★ — 이 입구가 거절해야 하는 쪽이다.
fn we_issue_its_session_id() -> AgentCommand {
    let c = AgentCommand::Claude {
        extra_args: vec![],
        output_format: AgentOutputFormat::Terminal,
    };
    assert!(
        engram_dashboard_agent::backend::assigns_session_id(&c),
        "전제: 이 백엔드의 세션 id 는 우리가 발급한다"
    );
    c
}

/// 등록 직후 프로필의 화신 표식 — 토큰을 그 값으로 발급해야 관측이 통과한다.
fn epoch_of(manager: &Arc<AgentManager>, id: AgentId) -> u32 {
    manager.agent_snapshot(id).expect("프로필").epoch
}

// ── 왕복 ──────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_reported_session_id_lands_in_the_profile() {
    let f = fixture("roundtrip").await;
    let id = seed_agent(&f.manager, "worker", reports_its_own_session_id());
    let epoch = epoch_of(&f.manager, id);
    f.registry.issue(id, epoch, "tok-roundtrip".into(), true);
    let sid = Uuid::new_v4();

    let (status, body) = f.report("tok-roundtrip", &sid.to_string()).await;

    assert_eq!(status, reqwest::StatusCode::OK, "body={body}");
    assert_eq!(body["outcome"], "recorded", "body={body}");
    assert_eq!(
        f.manager.agent_backend_session_id(id),
        Some(sid),
        "훅이 보고한 세션 id 가 프로필에 앉아야 한다"
    );
}

/// ★신원은 토큰에서만 나온다★ — 봉투에 "누구" 칸이 없고, 토큰이 없으면 아무 일도 안 일어난다.
#[tokio::test]
async fn the_route_refuses_calls_without_a_valid_token() {
    let f = fixture("auth").await;
    let id = seed_agent(&f.manager, "worker", reports_its_own_session_id());
    f.registry
        .issue(id, epoch_of(&f.manager, id), "tok-auth".into(), true);

    for bearer in [None, Some("not-a-real-token".to_string())] {
        let (status, _) = f
            .post(
                bearer.clone(),
                Some(serde_json::json!({ CLI_HOOK_SESSION_ID_FIELD: Uuid::new_v4().to_string() })),
            )
            .await;
        assert_eq!(
            status,
            reqwest::StatusCode::UNAUTHORIZED,
            "토큰 없음/무효는 401({bearer:?})"
        );
    }
    assert_eq!(f.manager.agent_backend_session_id(id), None);
}

// ── 세 갈래(없음 / 같음 / 다름) ───────────────────────────────────────────────────

/// ★이 시험이 재는 사고 둘이 같은 모양으로 온다★: ① 에이전트가 하위 세션을 띄우면 그 훅이 **같은
/// 토큰**(env 상속)으로 자기 세션 id 를 보고한다 ② 통로가 이미 적어 둔 진짜 손잡이를 훅이 다른 값으로
/// 덮으려 한다. 어느 쪽이든 **저장된 값이 그대로 남아야** 한다 — ②를 덮으면 없는 대화를 여는 재개가 된다.
#[tokio::test]
async fn a_different_session_id_is_refused_and_the_stored_one_survives() {
    let f = fixture("conflict").await;
    let id = seed_agent(&f.manager, "worker", reports_its_own_session_id());
    let epoch = epoch_of(&f.manager, id);
    f.registry.issue(id, epoch, "tok-conflict".into(), true);
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();

    let (_, ok) = f.report("tok-conflict", &first.to_string()).await;
    assert_eq!(ok["outcome"], "recorded", "{ok}");

    let (status, body) = f.report("tok-conflict", &second.to_string()).await;

    assert_eq!(status, reqwest::StatusCode::OK, "반려도 200 + JSON 이다");
    assert_eq!(body["status"], "error", "body={body}");
    assert_eq!(body["code"], "CONFLICTING_SESSION", "body={body}");
    assert_eq!(
        f.manager.agent_backend_session_id(id),
        Some(first),
        "거절했는데 저장된 값이 바뀌었다"
    );
}

/// ★**칸이 찬 뒤로는** 순서가 판정을 바꾸지 않는다★ — 재는 것은 「첫 값이 끝까지 남는다」다.
/// 그 앞의 빈 칸 갈래는 first-wins 이고, 그 한계의 정본은 `control::hook` 헤더다.
#[tokio::test]
async fn the_first_value_to_land_is_the_one_that_stays() {
    let f = fixture("order").await;
    let id = seed_agent(&f.manager, "worker", reports_its_own_session_id());
    let epoch = epoch_of(&f.manager, id);
    f.registry.issue(id, epoch, "tok-order".into(), true);
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();

    f.report("tok-order", &a.to_string()).await;
    f.report("tok-order", &b.to_string()).await;
    f.report("tok-order", &a.to_string()).await;

    assert_eq!(f.manager.agent_backend_session_id(id), Some(a));
}

/// 같은 값을 다시 보고하는 것(훅 재발화 · 통로가 적은 값을 훅이 뒤따라 보고)은 사고가 아니다.
#[tokio::test]
async fn reporting_the_same_id_twice_is_accepted() {
    let f = fixture("repeat").await;
    let id = seed_agent(&f.manager, "worker", reports_its_own_session_id());
    let epoch = epoch_of(&f.manager, id);
    f.registry.issue(id, epoch, "tok-repeat".into(), true);
    let sid = Uuid::new_v4();

    f.report("tok-repeat", &sid.to_string()).await;
    let (status, body) = f.report("tok-repeat", &sid.to_string()).await;

    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["outcome"], "already_known", "body={body}");
    assert_eq!(f.manager.agent_backend_session_id(id), Some(sid));
}

// ── 자격 ──────────────────────────────────────────────────────────────────────────

/// ★우리가 세션 id 를 발급하는 에이전트의 보고는 **값을 보기도 전에** 끝난다★ — 그 손잡이는 우리가 정한
/// 값이라 밖에서 건드릴 이유가 없다. 이 한 줄이 「그 에이전트가 자기 이어받기를 스스로 깬다」를
/// 구조적으로 닫는다(자격증명은 유효하다 — 막는 것은 인증이 아니라 자격이다).
#[tokio::test]
async fn an_agent_whose_session_id_we_issue_is_refused() {
    let f = fixture("eligibility").await;
    let id = seed_agent(&f.manager, "issued", we_issue_its_session_id());
    let epoch = epoch_of(&f.manager, id);
    f.registry.issue(id, epoch, "tok-issued".into(), true);

    let (status, body) = f.report("tok-issued", &Uuid::new_v4().to_string()).await;

    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "인증은 통과한다(자격이 막는다)"
    );
    assert_eq!(body["status"], "error", "body={body}");
    assert_eq!(body["code"], "NOT_ELIGIBLE", "body={body}");
    assert_eq!(
        f.manager.agent_backend_session_id(id),
        None,
        "자격 없는 보고가 빈 칸에도 앉으면 안 된다"
    );
}

// ── 화신 ──────────────────────────────────────────────────────────────────────────

/// 화신이 갈린 뒤 도착한 보고는 프로필을 못 바꾼다 — 자격증명은 아직 살아 있는 창을 가정한 시험이다.
#[tokio::test]
async fn a_report_from_a_dead_incarnation_writes_nothing() {
    let f = fixture("stale").await;
    let id = seed_agent(&f.manager, "worker", reports_its_own_session_id());
    let live = epoch_of(&f.manager, id);
    // ★프로필의 표식과 **다른** 값으로 발급한다★ — 산 화신이 갈린 뒤 옛 훅이 늦게 도착한 모양이다.
    f.registry
        .issue(id, live.wrapping_add(1), "tok-stale".into(), true);

    let (status, body) = f.report("tok-stale", &Uuid::new_v4().to_string()).await;

    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["outcome"], "not_applied", "body={body}");
    assert_eq!(f.manager.agent_backend_session_id(id), None);
}

// ── 읽을 수 없는 입력 ─────────────────────────────────────────────────────────────

/// ★빈 400 을 내지 않는 것이 이 라우트의 계약이다★ — 훅은 응답을 안 읽지만, 사람이 배선을 확인하러
/// 직접 칠 때 사유가 보여야 한다.
#[tokio::test]
async fn a_malformed_body_is_refused_with_a_reason() {
    let f = fixture("malformed").await;
    let id = seed_agent(&f.manager, "worker", reports_its_own_session_id());
    f.registry
        .issue(id, epoch_of(&f.manager, id), "tok-bad".into(), true);

    for body in [
        serde_json::json!({}),
        serde_json::json!({ CLI_HOOK_SESSION_ID_FIELD: "not-a-uuid" }),
        serde_json::json!([1, 2, 3]),
    ] {
        let (status, resp) = f
            .post(Some("tok-bad".to_string()), Some(body.clone()))
            .await;
        assert_eq!(status, reqwest::StatusCode::OK, "req={body}");
        assert_eq!(resp["status"], "error", "req={body} resp={resp}");
        assert!(
            resp["hint"].as_str().is_some_and(|h| !h.is_empty()),
            "사유가 비었다: req={body} resp={resp}"
        );
    }
    assert_eq!(f.manager.agent_backend_session_id(id), None);
}
