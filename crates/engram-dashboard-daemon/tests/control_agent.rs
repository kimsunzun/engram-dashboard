//! ADR-0132 조각 ② 통합 테스트 — `/control/agent` 라우트(에이전트 제어 동사).
//!
//! ★claude 불요★: 실 프로세스가 필요한 곳은 **셸 백엔드**로 띄운다(어느 러너에도 있다). 유일한 예외가
//!   `spawn --cwd`(만들어서 띄우기)인데, 그 동사는 claude 에이전트를 만들므로 **시동 성패가 머신에 달려
//!   있다** — 그 테스트는 기계에 따라 갈리지 않는 것만 단언한다(해당 테스트 주석).

use std::sync::{Arc, Mutex};

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::manager::MAX_ROSTER_SIZE;
use engram_dashboard_core::agent::preset::{Preset, PresetRegistry, PresetStore};
use engram_dashboard_core::agent::profile::{
    AgentCommand, AgentProfile, ProfileRegistry, ProfileStore,
};
use engram_dashboard_core::agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, NoopControlChannel, StatusSink,
};
use engram_dashboard_daemon::control::agent::RosterBroadcast;
use engram_dashboard_daemon::control::commands::make_daemon_table;
use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server, CommandTableSlot, ManagerSlot, McpServerHandle, MessagingSlot,
    RosterBroadcastSlot,
};
use engram_dashboard_daemon::control::registry::ControlRegistry;

// ── 하네스 ────────────────────────────────────────────────────────────────────────

struct NoopSink;
impl StatusSink for NoopSink {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

#[derive(Default)]
struct MemProfileStore {
    saved: Mutex<Vec<AgentProfile>>,
}
impl ProfileStore for MemProfileStore {
    fn save(&self, profiles: &[AgentProfile]) {
        *self.saved.lock().expect("poisoned") = profiles.to_vec();
    }
    fn load(&self) -> Vec<AgentProfile> {
        self.saved.lock().expect("poisoned").clone()
    }
}

#[derive(Default)]
struct MemPresetStore;
impl PresetStore for MemPresetStore {
    fn save(&self, _presets: &[Preset]) {}
    fn load(&self) -> Vec<Preset> {
        vec![]
    }
}

/// 명부 통지 관측자 — 어떤 동사가 화면 갱신을 일으키는지 세는 것이 목적이라 횟수만 센다.
#[derive(Default)]
struct CountingBroadcast {
    calls: Mutex<usize>,
}
impl CountingBroadcast {
    fn count(&self) -> usize {
        *self.calls.lock().expect("poisoned")
    }
}
impl RosterBroadcast for CountingBroadcast {
    fn roster_changed(&self) {
        *self.calls.lock().expect("poisoned") += 1;
    }
}

struct Fixture {
    manager: Arc<AgentManager>,
    base: String,
    token: String,
    broadcast: Arc<CountingBroadcast>,
    _handle: McpServerHandle,
}

impl Fixture {
    async fn post(&self, body: serde_json::Value) -> (reqwest::StatusCode, serde_json::Value) {
        self.post_with(Some(self.token.clone()), body).await
    }

    async fn post_with(
        &self,
        bearer: Option<String>,
        body: serde_json::Value,
    ) -> (reqwest::StatusCode, serde_json::Value) {
        let mut req = reqwest::Client::new()
            .post(format!("{}/control/agent", self.base))
            .header("Content-Type", "application/json")
            .json(&body);
        if let Some(b) = bearer {
            req = req.header("Authorization", format!("Bearer {b}"));
        }
        let resp = req.send().await.expect("http request");
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let json = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    /// 좀비 PTY 를 남기지 않는다 — 실 프로세스를 띄운 테스트는 끝에서 반드시 부른다.
    fn shutdown_agents(&self) {
        self.manager.shutdown_all();
    }
}

async fn fixture(tag: &str) -> Fixture {
    fixture_with_table(tag, true).await
}

/// `with_table = false` 는 **표가 아직 안 꽂힌 조립**을 재현한다 — 그 상태의 계약(503)을 보는 테스트가
/// 쓴다. 그 외엔 언제나 표를 꽂는다(그게 운영 조립이다).
async fn fixture_with_table(tag: &str, with_table: bool) -> Fixture {
    let registry = Arc::new(ControlRegistry::new());
    let manager_slot = Arc::new(ManagerSlot::new());
    let broadcast = Arc::new(CountingBroadcast::default());
    let broadcast_slot = Arc::new(RosterBroadcastSlot::new());
    broadcast_slot.set(broadcast.clone());
    let command_slot = Arc::new(CommandTableSlot::new());
    let handle = start_mcp_server(
        registry.clone(),
        manager_slot.clone(),
        Arc::new(MessagingSlot::new()),
        command_slot.clone(),
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
                sessions_dir: None,
                enabled: false,
                poll_interval: std::time::Duration::from_secs(1),
            },
            Arc::new(|_, _| {}),
        )),
        control,
    ));
    manager_slot.set(manager.clone());
    if with_table {
        // 라우트가 실제로 태우는 것이 이 표다(ADR-0140) — 운영 조립(`lib.rs`)과 같은 조립 함수를 쓴다.
        command_slot.set(Arc::new(make_daemon_table(
            manager.clone(),
            broadcast_slot.clone(),
        )));
    }

    // 호출자 신원 — 제어 동사는 신원을 인가에만 쓴다(발신자 파생이 없다). 그래서 아무 에이전트 신원이나
    //   유효하고, 여기선 명부에 없는 id 로 발급해 "제어는 자기 존재를 전제하지 않는다" 도 함께 고정한다.
    let token = format!("test-token-{tag}");
    registry.issue(AgentId::new_v4(), 0, token.clone(), true);

    let base = handle
        .url
        .strip_suffix("/mcp")
        .expect("mcp url suffix")
        .to_string();
    Fixture {
        manager,
        base,
        token,
        broadcast,
        _handle: handle,
    }
}

/// 셸 백엔드 에이전트를 명부에 등록한다 — 실제로 띄워도 claude 가 필요 없다.
fn seed_shell_agent(manager: &Arc<AgentManager>, name: &str) -> AgentId {
    let cwd = std::env::temp_dir();
    let mut profile = AgentProfile::new(
        name.to_string(),
        AgentCommand::Shell {
            program: engram_dashboard_core::agent::manager::default_shell().to_string(),
            args: vec![],
        },
        cwd,
        vec![],
        false,
    );
    profile.display_name = Some(name.to_string());
    manager.create_agent(profile).expect("등록 성공").id
}

fn agents_in(list: &serde_json::Value) -> Vec<(String, String)> {
    list["agents"]
        .as_array()
        .expect("agents 배열")
        .iter()
        .map(|a| {
            (
                a["name"].as_str().unwrap_or_default().to_string(),
                a["state"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

// ── 인증 ──────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_agent_route_refuses_calls_without_a_valid_token() {
    let f = fixture("auth").await;
    let list = serde_json::json!({ "verb": "list" });
    for bearer in [None, Some("not-a-real-token".to_string())] {
        let (status, _) = f.post_with(bearer.clone(), list.clone()).await;
        assert_eq!(
            status,
            reqwest::StatusCode::UNAUTHORIZED,
            "토큰 없음/무효는 401({bearer:?})"
        );
    }
    let (status, body) = f.post(list).await;
    assert_eq!(status, reqwest::StatusCode::OK, "유효 토큰은 통과: {body}");
}

// ── 동사별 happy path ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_reports_both_live_and_sleeping_agents() {
    let f = fixture("list").await;
    seed_shell_agent(&f.manager, "sleepy");
    let live = seed_shell_agent(&f.manager, "awake");
    let profile = f.manager.agent_snapshot(live).expect("스냅샷");
    f.manager
        .activate_profile(
            &profile,
            engram_dashboard_core::agent::profile::SpawnMode::Fresh,
        )
        .expect("셸 스폰");

    let (status, body) = f.post(serde_json::json!({ "verb": "list" })).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let mut rows = agents_in(&body);
    rows.sort();
    assert_eq!(
        rows,
        vec![
            ("awake".to_string(), "live".to_string()),
            ("sleepy".to_string(), "sleeping".to_string()),
        ],
        "명부는 산 것과 잠든 것을 함께 낸다: {body}"
    );
    // 조회는 명부를 바꾸지 않으므로 화면 갱신도 없다.
    assert_eq!(f.broadcast.count(), 0, "list 는 통지하지 않는다");
    f.shutdown_agents();
}

#[tokio::test]
async fn new_registers_a_sleeping_agent_and_refreshes_the_clients() {
    let f = fixture("new").await;
    let cwd = std::env::temp_dir().to_string_lossy().to_string();
    let (status, body) = f
        .post(serde_json::json!({ "verb": "new", "cwd": cwd, "name": "fresh-one" }))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    // 성공 본문은 평평하다 — 반환을 명령마다 선언하므로 한 겹 더 감싸지 않는다(ADR-0140).
    assert_eq!(body["name"], "fresh-one", "{body}");
    assert_eq!(body["state"], "sleeping", "{body}");
    assert!(
        body["agent_id"].as_str().is_some_and(|s| !s.is_empty()),
        "id 를 돌려줘야 이름이 겹칠 때 지목할 수 있다: {body}"
    );
    assert_eq!(
        f.broadcast.count(),
        1,
        "명부가 바뀌면 클라이언트를 갱신한다"
    );

    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    assert_eq!(
        agents_in(&list),
        vec![("fresh-one".to_string(), "sleeping".to_string())]
    );
}

#[tokio::test]
async fn new_falls_back_to_the_folder_name_and_appends_a_number_when_it_is_taken() {
    let f = fixture("new-suffix").await;
    let dir = std::env::temp_dir().join(format!("engram-agent-route-{}", AgentId::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let base = dir
        .file_name()
        .and_then(|s| s.to_str())
        .expect("basename")
        .to_string();
    let cwd = dir.to_string_lossy().to_string();

    let (_, first) = f
        .post(serde_json::json!({ "verb": "new", "cwd": cwd }))
        .await;
    assert_eq!(first["name"], base, "이름을 안 주면 폴더 이름: {first}");
    let (_, second) = f
        .post(serde_json::json!({ "verb": "new", "cwd": cwd }))
        .await;
    // ★요청 이름이 아니라 **확정된 이름**을 돌려준다★(ADR-0120/0123) — 아니면 화면·주소와 어긋난다.
    assert_eq!(
        second["name"],
        format!("{base}(1)"),
        "겹치면 번호가 붙고 응답이 그 값을 싣는다: {second}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn spawn_wakes_an_agent_that_already_exists() {
    let f = fixture("spawn-wake").await;
    seed_shell_agent(&f.manager, "worker");

    let (status, body) = f
        .post(serde_json::json!({ "verb": "spawn", "target": "worker" }))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["name"], "worker", "{body}");
    assert_eq!(body["state"], "live", "{body}");
    assert_eq!(body["created"], false, "있는 것을 깨운 것이다: {body}");

    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    assert_eq!(
        agents_in(&list),
        vec![("worker".to_string(), "live".to_string())]
    );
    // ★깨우기는 명부의 **구성**을 안 바꾼다★ — 생사 전이는 매니저가 이미 흘리므로(`agent_list_updated`)
    //   여기서 통지를 겹쳐 보내면 트리가 같은 변화를 두 번 받는다. 표로 옮기면서 이 분담이 유지되는지를
    //   라우트 레벨에서 못박는다(코어 단위 테스트와 같은 축, 다른 층).
    assert_eq!(f.broadcast.count(), 0, "깨우기는 통지하지 않는다");
    f.shutdown_agents();
}

/// ★변경 동사가 낸 상태와 `list` 가 낸 상태는 갈릴 수 없다★ — 전엔 변경 동사가 `"live"` 를 박아서, 깨우자마자
///   죽은 에이전트를 `spawn` 은 살아 있다고 `list` 는 잠들었다고 보고할 수 있었다. 셸 백엔드는 계속 살아
///   있으므로 이 대조가 결정적이다.
#[tokio::test]
async fn the_state_a_mutation_reports_is_the_state_the_roster_reports() {
    let f = fixture("state-honesty").await;
    seed_shell_agent(&f.manager, "worker");

    let (_, spawned) = f
        .post(serde_json::json!({ "verb": "spawn", "target": "worker" }))
        .await;
    let claimed = spawned["state"].as_str().unwrap_or_default().to_string();
    assert_eq!(claimed, "live", "산 셸 에이전트: {spawned}");

    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    let listed = agents_in(&list)
        .into_iter()
        .find(|(n, _)| n == "worker")
        .expect("worker 행")
        .1;
    assert_eq!(
        claimed, listed,
        "같은 라우트의 두 동사가 같은 에이전트를 두고 다른 답을 내면 안 된다: {spawned} vs {list}"
    );
    f.shutdown_agents();
}

/// 만들기+띄우기에서 **기계와 무관하게 확정적인 부분만** 엄격히 본다: 등록 · 통지 **정확히 1회** · 명부 등장 ·
/// 응답이 상태 어휘 안의 값만 싣는 것. 시동 성패 자체는 claude 유무에 달려 있어(claude 에이전트를 만든다)
/// 여기서 단언하지 않는다 — 시동의 회귀는 위 셸 백엔드 테스트가 결정적으로 본다.
#[tokio::test]
async fn spawn_with_cwd_registers_exactly_once_and_never_invents_a_state() {
    let f = fixture("spawn-create").await;
    let cwd = std::env::temp_dir().to_string_lossy().to_string();
    let (status, body) = f
        .post(serde_json::json!({ "verb": "spawn", "cwd": cwd, "name": "newborn" }))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    assert_eq!(
        f.broadcast.count(),
        1,
        "등록 1건 = 통지 1건(시동 성패와 무관): {body}"
    );

    match body["status"].as_str() {
        Some("error") => {
            // 시동 실패는 인자 문제도 대상 부재도 아니다 — 고칠 인자가 없는 실패라 INTERNAL 이다(TRD §4-⑦).
            assert_eq!(body["code"], "INTERNAL", "{body}");
            let hint = body["hint"].as_str().unwrap_or_default();
            assert!(
                hint.contains("newborn") && hint.contains("created"),
                "만들어졌으나 못 떴다는 사실을 알려야: {body}"
            );
        }
        _ => {
            assert_eq!(body["name"], "newborn", "{body}");
            assert_eq!(body["created"], true, "{body}");
            let state = body["state"].as_str().unwrap_or_default();
            assert!(
                state == "live" || state == "sleeping",
                "상태 어휘 밖 값을 지어내면 안 된다: {body}"
            );
        }
    }

    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    let names: Vec<String> = agents_in(&list).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, vec!["newborn".to_string()], "등록은 남는다: {list}");
    f.shutdown_agents();
}

// ── 지목 규칙 ─────────────────────────────────────────────────────────────────────

/// ★대상이 **실재하는 동안** 빗나감을 확인한다★: 예전 판본은 대상을 먼저 개명해 놓고 빗나감을 단언해서,
///   대소문자 무시·접두 매칭 해석기를 넣어도 그대로 통과했다(빈 명부에서는 무엇으로도 안 맞는다).
#[tokio::test]
async fn near_miss_tokens_do_not_resolve_while_the_real_agent_is_still_there() {
    let f = fixture("targeting-exact").await;
    seed_shell_agent(&f.manager, "worker");

    for miss in ["work", "WORKER", "Worker", "worker ", " worker", "worker2"] {
        let (_, body) = f
            .post(serde_json::json!({ "verb": "rename", "target": miss, "name": "hijacked" }))
            .await;
        // 지목이 아무도 안 가리키면 NOT_FOUND — 코드 어휘는 계열마다 갈리지 않는다(TRD §4-⑦).
        assert_eq!(
            body["code"], "NOT_FOUND",
            "'{miss}' 는 'worker' 로 해석되면 안 된다: {body}"
        );
    }
    // ★대조군 — 정확한 토큰은 통한다★. 이게 없으면 위 단언들은 "명부가 비어서" 통과한 것과 구분되지 않는다.
    let (_, body) = f
        .post(
            serde_json::json!({ "verb": "rename", "target": "worker", "name": "renamed-exactly" }),
        )
        .await;
    assert_eq!(body["outcome"], "renamed", "{body}");
    assert_eq!(body["name"], "renamed-exactly", "{body}");
    // 빗나간 호출들이 아무것도 바꾸지 않았다(성공 1회분만 통지됐다).
    assert_eq!(f.broadcast.count(), 1);
}

#[tokio::test]
async fn an_id_beats_a_name_that_looks_like_that_id() {
    let f = fixture("targeting-id").await;
    let real = seed_shell_agent(&f.manager, "worker");
    // 남의 **id 문자열을 이름으로** 가진 에이전트가 명부에 함께 있다.
    let impostor = seed_shell_agent(&f.manager, &real.to_string());

    let (_, body) = f
        .post(serde_json::json!({ "verb": "rename", "target": real.to_string(), "name": "renamed-by-id" }))
        .await;
    assert_eq!(body["outcome"], "renamed", "{body}");
    assert_eq!(
        body["agent_id"],
        real.to_string(),
        "id 지목은 그 id 의 에이전트에게 간다: {body}"
    );

    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    let impostor_row = list["agents"]
        .as_array()
        .expect("배열")
        .iter()
        .find(|a| a["id"] == impostor.to_string())
        .expect("사칭 행")
        .clone();
    assert_eq!(
        impostor_row["name"],
        real.to_string(),
        "사칭 에이전트는 건드려지지 않았다: {list}"
    );
}

#[tokio::test]
async fn move_reparents_and_none_detaches_back_to_the_top_level() {
    let f = fixture("move").await;
    seed_shell_agent(&f.manager, "lead");
    let child = seed_shell_agent(&f.manager, "helper");

    let (status, body) = f
        .post(serde_json::json!({ "verb": "move", "target": "helper", "parent": "lead" }))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["name"], "helper", "{body}");
    assert!(body["parent"].as_str().is_some(), "새 부모 id: {body}");
    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    let row = list["agents"]
        .as_array()
        .expect("배열")
        .iter()
        .find(|a| a["name"] == "helper")
        .expect("helper 행")
        .clone();
    assert_eq!(row["parent"], child_parent(&list, "lead"), "{list}");

    let (_, detached) = f
        .post(serde_json::json!({ "verb": "move", "target": "helper", "parent": null }))
        .await;
    assert!(detached["parent"].is_null(), "루트로 떼기: {detached}");
    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    let row = list["agents"]
        .as_array()
        .expect("배열")
        .iter()
        .find(|a| a["name"] == "helper")
        .expect("helper 행")
        .clone();
    assert!(row["parent"].is_null(), "{list}");
    assert_eq!(f.broadcast.count(), 2, "계층 변경 2회 = 화면 갱신 2회");
    let _ = child;
}

fn child_parent(list: &serde_json::Value, parent_name: &str) -> serde_json::Value {
    list["agents"]
        .as_array()
        .expect("배열")
        .iter()
        .find(|a| a["name"] == parent_name)
        .expect("부모 행")["id"]
        .clone()
}

#[tokio::test]
async fn move_refuses_an_impossible_parent_instead_of_silently_doing_nothing() {
    let f = fixture("move-reject").await;
    seed_shell_agent(&f.manager, "solo");
    let (status, body) = f
        .post(serde_json::json!({ "verb": "move", "target": "solo", "parent": "solo" }))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    // 트리 구조가 거부한 것은 인자 오류가 아니라 상태 충돌이다 — CONFLICT(TRD §4-⑦).
    assert_eq!(body["code"], "CONFLICT", "{body}");
    assert_eq!(f.broadcast.count(), 0, "거부는 화면 갱신을 부르지 않는다");
}

// ── 개명의 네 결말 ────────────────────────────────────────────────────────────────

/// ★네 결말이 서로 구분돼야 한다★: 확정 개명 · 무변경 · 대상 부재 · 이름 공간 소진.
///
/// ★소진(`RenameOutcome::Exhausted`)은 여기서 만들 수 없다★ — 매니저는 그 계열의 1..=u32::MAX 가 **전부**
///   점유됐을 때만 그 값을 내므로(명부에 42억 항목) 실행 가능한 재현이 없다. 그 갈래가 wire 로 어떻게 나가는지는
///   코드가 유일한 진술이고(`CONFLICT`), 여기서는 나머지 셋을 고정한다.
#[tokio::test]
async fn rename_surfaces_each_of_its_four_outcomes_distinctly() {
    let f = fixture("rename").await;
    seed_shell_agent(&f.manager, "alpha");
    seed_shell_agent(&f.manager, "beta");

    // ① 확정 개명 — 요청한 이름 그대로.
    let (_, body) = f
        .post(serde_json::json!({ "verb": "rename", "target": "alpha", "name": "gamma" }))
        .await;
    assert_eq!(body["outcome"], "renamed", "{body}");
    assert_eq!(body["name"], "gamma", "{body}");

    // ② 확정 개명 — 이름이 겹쳐 번호가 붙었다. **요청 이름을 되돌려주면 안 된다**(화면·주소와 어긋난다).
    let (_, body) = f
        .post(serde_json::json!({ "verb": "rename", "target": "gamma", "name": "beta" }))
        .await;
    assert_eq!(body["outcome"], "renamed", "{body}");
    assert_eq!(body["name"], "beta(1)", "확정된 이름: {body}");

    // ③ 무변경 — 이미 그 계열의 이름을 쥐고 있다. 성공이지만 ①·②와 **다른 사실**이다.
    let (_, body) = f
        .post(serde_json::json!({ "verb": "rename", "target": "beta(1)", "name": "beta" }))
        .await;
    assert_eq!(body["outcome"], "unchanged", "{body}");
    assert_eq!(body["name"], "beta(1)", "{body}");

    // ④ 대상 부재.
    let (_, body) = f
        .post(serde_json::json!({ "verb": "rename", "target": "ghost", "name": "whatever" }))
        .await;
    // 대상이 없다 — NOT_FOUND(TRD §4-⑦).
    assert_eq!(body["code"], "NOT_FOUND", "{body}");
    assert!(
        body["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("exactly"),
        "정확 일치 규칙을 알려야: {body}"
    );
}

// ── 지목 규칙 ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_duplicate_name_is_refused_rather_than_guessed() {
    let f = fixture("ambiguous").await;
    // 유일성(ADR-0120)을 우회해 정상 경로로는 못 만드는 상태(동명 2건)를 재현한다.
    for _ in 0..2 {
        let mut p = AgentProfile::new(
            "twin".to_string(),
            AgentCommand::Shell {
                program: engram_dashboard_core::agent::manager::default_shell().to_string(),
                args: vec![],
            },
            std::env::temp_dir(),
            vec![],
            false,
        );
        p.display_name = Some("twin".to_string());
        f.manager.seed_agent_bypassing_uniqueness(p);
    }
    let (status, body) = f
        .post(serde_json::json!({ "verb": "rename", "target": "twin", "name": "x" }))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    // 동명 둘은 **부재와 다른 사실**이다 — 부재는 NOT_FOUND, 이건 CONFLICT 로 갈린다(TRD §4-⑦).
    assert_eq!(body["code"], "CONFLICT", "{body}");
    assert!(
        body["hint"].as_str().unwrap_or_default().contains("id"),
        "탈출구(id 지목)를 알려야: {body}"
    );
    assert_eq!(f.broadcast.count(), 0, "모호하면 아무것도 바꾸지 않는다");
}

// ── 인자 반려(데몬 측) ─────────────────────────────────────────────────────────────

/// ★바디가 JSON 계약을 못 지켜도 **빈 400 이 아니라 봉투**로 답한다★: 호출자가 LLM 이라 사유가 실려야
///   자기교정이 된다. `Json<…>` 추출기를 그대로 썼다면 이 네 경우가 전부 빈 400 이었다.
#[tokio::test]
async fn a_body_that_is_not_a_command_object_still_gets_a_reason() {
    let f = fixture("bad-body").await;
    let client = reqwest::Client::new();
    for raw in [
        r#"{"verb": 5}"#,                   // 타입 불일치
        r#"{"verb":"list","verb":"move"}"#, // 동사 중복
        // ★인자 칸 중복도 같은 자리에서 걸린다★: 뒤 값을 택하면 `parent:"lead"` 로 붙이려던 요청이
        //   **루트로 떼기**로 조용히 바뀐다 — 어느 값을 고를 근거가 없으므로 고르지 않는다(ADR-0142).
        r#"{"verb":"move","target":"helper","parent":"lead","parent":null}"#,
        r#"["list"]"#,       // 객체가 아님
        r#"{"verb":"list""#, // 깨진 JSON
        "",                  // 빈 바디
    ] {
        let resp = client
            .post(format!("{}/control/agent", f.base))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", f.token))
            .body(raw)
            .send()
            .await
            .expect("http request");
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        assert_eq!(
            status,
            reqwest::StatusCode::OK,
            "봉투로 답한다: {raw} → {text}"
        );
        let v: serde_json::Value = serde_json::from_str(&text).expect("응답 json");
        // 바디가 명령 객체가 아니면 인자 오류다 — 이 입구의 코드는 한 어휘로만 나간다(TRD §4-⑦).
        assert_eq!(v["code"], "INVALID_ARGUMENT", "{raw} → {text}");
        assert!(
            !v["hint"].as_str().unwrap_or_default().is_empty(),
            "사유가 비면 자기교정이 안 된다: {raw} → {text}"
        );
    }
    assert_eq!(f.broadcast.count(), 0);
}

#[tokio::test]
async fn the_daemon_refuses_malformed_verbs_on_its_own() {
    // ★CLI 가 이미 막는 것도 데몬이 다시 본다★: 이 라우트의 호출자가 우리 CLI 뿐이라는 보장이 없다.
    let f = fixture("bad-args").await;
    // ★두 어휘가 갈린다(TRD §4-②)★: 이름 자체를 모르는 것과 이름은 아는데 인자가 어긋난 것은 호출자가
    //   할 일이 다르다 — 전자는 동사를 다시 찾아야 하고, 후자는 같은 동사로 인자만 고치면 된다.
    let cases: [(serde_json::Value, &str); 6] = [
        (serde_json::json!({}), "UNKNOWN_COMMAND"),
        (serde_json::json!({ "verb": "explode" }), "UNKNOWN_COMMAND"),
        (
            serde_json::json!({ "verb": "spawn", "target": "a", "cwd": "C:/x" }),
            "INVALID_ARGUMENT",
        ),
        (serde_json::json!({ "verb": "spawn" }), "INVALID_ARGUMENT"),
        (serde_json::json!({ "verb": "new" }), "INVALID_ARGUMENT"),
        (
            serde_json::json!({ "verb": "rename", "target": "a" }),
            "INVALID_ARGUMENT",
        ),
    ];
    for (body, want) in cases {
        let (status, resp) = f.post(body.clone()).await;
        assert_eq!(status, reqwest::StatusCode::OK, "반려도 200 + JSON: {body}");
        assert_eq!(resp["code"], want, "{body} → {resp}");
        assert!(
            resp["hint"].as_str().unwrap_or_default().contains("help"),
            "자기교정 경로를 알려야: {resp}"
        );
    }
    assert_eq!(f.broadcast.count(), 0);
}

/// ★부재 · 공백 · 명시 null 은 서로 다른 요청이다★ — 이 라우트의 호출자가 우리 CLI 뿐이라는 보장이 없으므로
///   **raw 바디**로 직접 친다(CLI 를 거치면 CLI 의 방어가 이 판정을 가린다).
#[tokio::test]
async fn move_distinguishes_an_absent_parent_a_blank_parent_and_an_explicit_detach() {
    let f = fixture("move-tristate").await;
    seed_shell_agent(&f.manager, "lead");
    seed_shell_agent(&f.manager, "helper");
    f.post(serde_json::json!({ "verb": "move", "target": "helper", "parent": "lead" }))
        .await;
    let under_lead = |list: &serde_json::Value| -> bool {
        list["agents"]
            .as_array()
            .expect("배열")
            .iter()
            .find(|a| a["name"] == "helper")
            .expect("helper 행")["parent"]
            .is_string()
    };
    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    assert!(under_lead(&list), "전제: helper 가 lead 밑에 있다: {list}");
    let after_setup = f.broadcast.count();

    // ① 필드 부재 — "부모를 안 줬으니 떼자" 로 읽으면 오타 한 번이 계층 해제가 된다.
    let (_, absent) = f
        .post(serde_json::json!({ "verb": "move", "target": "helper" }))
        .await;
    assert_eq!(absent["code"], "INVALID_ARGUMENT", "{absent}");
    // ② 공백 값 — 셸의 미설정 변수(`--parent "$UNSET"`)가 이 모양으로 도착한다.
    for blank in ["", "   "] {
        let (_, body) = f
            .post(serde_json::json!({ "verb": "move", "target": "helper", "parent": blank }))
            .await;
        assert_eq!(body["code"], "INVALID_ARGUMENT", "공백 '{blank}': {body}");
    }
    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    assert!(
        under_lead(&list),
        "반려된 요청은 계층을 건드리지 않았어야: {list}"
    );
    assert_eq!(f.broadcast.count(), after_setup, "반려는 통지하지 않는다");

    // ③ 명시 null — 이것만이 "루트로 떼기" 다.
    let (_, detached) = f
        .post(serde_json::json!({ "verb": "move", "target": "helper", "parent": null }))
        .await;
    assert!(detached["parent"].is_null(), "{detached}");
    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    assert!(!under_lead(&list), "이제 루트다: {list}");
}

/// 오타 하나가 **다른 동작**이 되지 않게 한다 — 모르는 필드도, 그 동사가 쓰지 않는 필드도 반려한다.
#[tokio::test]
async fn unknown_and_irrelevant_fields_are_refused_instead_of_being_dropped() {
    let f = fixture("stray-fields").await;
    seed_shell_agent(&f.manager, "helper");
    seed_shell_agent(&f.manager, "lead");
    f.post(serde_json::json!({ "verb": "move", "target": "helper", "parent": "lead" }))
        .await;
    let before = f.broadcast.count();

    let cases: [(serde_json::Value, &str); 5] = [
        // `parnet` 오타 — 흘려보내면 "부모 없음" 으로 읽혀 계층이 해제된다.
        (
            serde_json::json!({ "verb": "move", "target": "helper", "parnet": "lead" }),
            "parnet",
        ),
        (serde_json::json!({ "verb": "list", "wat": 1 }), "wat"),
        // 동사가 쓰지 않는 필드 — 무시하면 인자가 사라진 것을 호출자가 못 본다.
        (
            serde_json::json!({ "verb": "spawn", "target": "helper", "name": "x" }),
            "name",
        ),
        (
            serde_json::json!({ "verb": "list", "target": "helper" }),
            "target",
        ),
        (
            serde_json::json!({ "verb": "rename", "target": "helper", "name": "x", "cwd": "C:/x" }),
            "cwd",
        ),
    ];
    for (body, culprit) in cases {
        let (status, resp) = f.post(body.clone()).await;
        assert_eq!(status, reqwest::StatusCode::OK, "반려도 200 + JSON: {body}");
        assert_eq!(resp["code"], "INVALID_ARGUMENT", "{body} → {resp}");
        assert!(
            resp["hint"].as_str().unwrap_or_default().contains(culprit),
            "어느 필드가 문제인지 지목해야({culprit}): {resp}"
        );
    }
    assert_eq!(
        f.broadcast.count(),
        before,
        "반려된 요청은 아무것도 바꾸지 않는다"
    );
    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    let helper = list["agents"]
        .as_array()
        .expect("배열")
        .iter()
        .find(|a| a["name"] == "helper")
        .expect("helper 행")
        .clone();
    assert!(
        helper["parent"].is_string(),
        "오타 요청이 계층을 해제하지 않았어야: {list}"
    );
}

/// ★반려 목록이 **선언에서 파생됐다**는 것까지 본다(ADR-0142)★: 문구가 틀린 칸만 짚고 끝나면 손으로 적은
///   허용 목록으로도 통과한다 — 그 사본은 동사가 늘 때 조용히 뒤처져 모르는 칸을 통과시킨다. 선언된 칸
///   **전량**이 함께 실리는지를 보면 판정 재료가 선언이라는 것이 드러나고, 호출자도 스스로 고칠 수 있다.
#[tokio::test]
async fn an_unknown_field_is_refused_with_the_declared_argument_list() {
    let f = fixture("declared-args").await;
    seed_shell_agent(&f.manager, "helper");

    let (status, resp) = f
        .post(serde_json::json!({ "verb": "rename", "target": "helper", "nmae": "x" }))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK, "반려도 200 + JSON: {resp}");
    assert_eq!(resp["code"], "INVALID_ARGUMENT", "{resp}");
    let hint = resp["hint"].as_str().unwrap_or_default();
    assert!(hint.contains("nmae"), "틀린 칸을 짚어야: {resp}");
    for declared in ["target", "name"] {
        assert!(
            hint.contains(declared),
            "선언된 칸 전량이 실려야({declared}): {resp}"
        );
    }
    assert_eq!(f.broadcast.count(), 0, "반려는 아무것도 바꾸지 않는다");
}

/// 모르는 동사는 **봉투로** 답한다 — 라우트가 없는 것도(404) 서버가 터진 것도(500) 아니다.
///
/// ★이름을 모르는 것과 인자가 어긋난 것을 코드로 가른다(TRD §4-②)★: 앞은 동사를 다시 찾아야 하고 뒤는
///   같은 동사로 인자만 고치면 된다.
#[tokio::test]
async fn an_unknown_verb_answers_with_the_error_envelope_not_a_404() {
    let f = fixture("unknown-verb").await;
    let (status, resp) = f.post(serde_json::json!({ "verb": "explode" })).await;

    assert_eq!(status, reqwest::StatusCode::OK, "봉투로 답한다: {resp}");
    assert_eq!(resp["status"], "error", "{resp}");
    assert_eq!(resp["code"], "UNKNOWN_COMMAND", "{resp}");
    let hint = resp["hint"].as_str().unwrap_or_default();
    assert!(hint.contains("explode"), "어느 동사인지 짚어야: {resp}");
    assert!(hint.contains("help"), "자기교정 경로를 알려야: {resp}");
}

/// ★표가 안 꽂힌 조립은 503 이다★ — 요청 형식도 인증도 문제가 아니라 **배선 순서** 문제이므로 4xx 로
///   내리면 호출자가 자기 인자를 고치러 간다(고칠 것이 없다). 그 갈림이 상태코드에 실린다.
#[tokio::test]
async fn the_route_reports_a_missing_command_table_as_unavailable() {
    let f = fixture_with_table("no-table", false).await;
    let (status, body) = f.post(serde_json::json!({ "verb": "list" })).await;
    assert_eq!(
        status,
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "표 미설정은 503: {body}"
    );
}

/// ★거부 사유 목록은 실제 거부 조건과 대응해야 한다★: 2단 트리에서 실제로 걸리는 "옮기려는 쪽이 이미 부모"
///   가 문구에 없으면, 거부당한 호출자는 다음에 무엇을 할지 알 수 없다.
#[tokio::test]
async fn a_refused_move_names_the_rule_that_actually_fired() {
    let f = fixture("move-rules").await;
    seed_shell_agent(&f.manager, "a");
    seed_shell_agent(&f.manager, "b");
    seed_shell_agent(&f.manager, "c");
    // b 를 a 밑으로 — 이제 a 는 자식을 가진 부모다.
    let (_, ok) = f
        .post(serde_json::json!({ "verb": "move", "target": "b", "parent": "a" }))
        .await;
    assert!(ok["parent"].is_string(), "{ok}");

    // 자식을 가진 a 를 c 밑으로 = 3단이 되므로 거부된다(트리는 1단 중첩까지).
    let (_, refused) = f
        .post(serde_json::json!({ "verb": "move", "target": "a", "parent": "c" }))
        .await;
    assert_eq!(refused["code"], "CONFLICT", "{refused}");
    let hint = refused["hint"].as_str().unwrap_or_default();
    assert!(
        hint.contains("children"),
        "실제로 발화한 조건(옮기려는 쪽이 이미 부모)이 문구에 있어야: {hint}"
    );
    // 부모가 이미 남의 자식인 경우도 같은 문구가 답해야 한다.
    let (_, refused) = f
        .post(serde_json::json!({ "verb": "move", "target": "c", "parent": "b" }))
        .await;
    assert_eq!(refused["code"], "CONFLICT", "{refused}");
    assert!(
        refused["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("top-level"),
        "부모 자격 조건도 문구에 있어야: {refused}"
    );
}

/// ★폭주 백스톱(사용자 결정)★ — 개수 검사 하나. 경계에서 정확히 막히고, 막힌 뒤에도 조회·개명 같은
///   비-증가 동사는 계속 된다(상한이 명부를 얼려 버리면 복구 자체가 불가능해진다).
///
/// ★상한 자체는 코어가 강제한다★(`AgentManager` — 등록 커밋 자리, 이름 배정 게이트 안). 여기서 보는 것은
///   **이 라우트가 그 거부를 어떻게 번역하는가**뿐이다: 이름 공간 소진과 구분되는 코드, 그리고 성격을
///   밝히는 힌트. 상한이 모든 입구에 걸린다는 것과 동시 등록에서도 새지 않는다는 것은 코어 단위 테스트가 본다.
#[tokio::test]
async fn creating_agents_stops_at_the_runaway_ceiling() {
    let f = fixture("ceiling").await;
    let cwd = std::env::temp_dir().to_string_lossy().to_string();
    // 상한 직전까지 채운다(유일성 우회 seam — 정상 경로로 채우면 이름 배정이 O(n²)로 돈다).
    for i in 0..(MAX_ROSTER_SIZE - 1) {
        let mut p = AgentProfile::new(
            format!("filler-{i}"),
            AgentCommand::Shell {
                program: engram_dashboard_core::agent::manager::default_shell().to_string(),
                args: vec![],
            },
            std::env::temp_dir(),
            vec![],
            false,
        );
        p.display_name = Some(format!("filler-{i}"));
        f.manager.seed_agent_bypassing_uniqueness(p);
    }
    assert_eq!(f.manager.roster().len(), MAX_ROSTER_SIZE - 1, "채움 전제");

    // 마지막 한 자리는 통과한다 — 경계가 "미만" 이 아니라 "이상" 에서 막힌다는 것까지 본다.
    let (_, last) = f
        .post(serde_json::json!({ "verb": "new", "cwd": cwd, "name": "last-one" }))
        .await;
    assert_eq!(last["name"], "last-one", "{last}");
    assert_eq!(f.manager.roster().len(), MAX_ROSTER_SIZE);

    for body in [
        serde_json::json!({ "verb": "new", "cwd": cwd, "name": "one-too-many" }),
        serde_json::json!({ "verb": "spawn", "cwd": cwd, "name": "one-too-many" }),
    ] {
        let (status, resp) = f.post(body.clone()).await;
        assert_eq!(status, reqwest::StatusCode::OK);
        // 상한은 "자리를 비워라" 는 뜻이지 인자를 고치라는 뜻이 아니다 — CONFLICT(TRD §4-⑦).
        assert_eq!(resp["code"], "CONFLICT", "{body} → {resp}");
        let hint = resp["hint"].as_str().unwrap_or_default();
        assert!(
            hint.contains("safety ceiling") && hint.contains("not a product limit"),
            "상한의 성격을 밝혀야(튜닝 유혹 차단): {hint}"
        );
    }
    assert_eq!(
        f.manager.roster().len(),
        MAX_ROSTER_SIZE,
        "막힌 뒤엔 늘지 않는다"
    );

    // 상한에 닿아도 비-증가 동사는 계속 된다(복구 경로가 살아 있어야 한다).
    let (_, renamed) = f
        .post(serde_json::json!({ "verb": "rename", "target": "last-one", "name": "still-works" }))
        .await;
    assert_eq!(renamed["outcome"], "renamed", "{renamed}");
}

// ── 지목 규칙 ↔ 우편 입구 교차 대조 ─────────────────────────────────────────────────

/// ★두 입구의 지목 해석이 같은지를 **우편 입구를 실제로 태워** 확인한다★.
///
/// 비교 대상은 우편 커널의 `resolve_live`(private)이므로 직접 부르지 않고, 그것이 유일한 해석기인 공개
/// 경로(`MessagingService::handle_send`)에 같은 로스터를 물려 결말을 읽는다 — 배달됐으면 그 수신자로
/// 해석된 것이고, `RECIPIENT_NOT_FOUND`/`RECIPIENT_AMBIGUOUS` 면 각각 부재·모호로 해석된 것이다.
///
/// ★제어 쪽은 **입구가 실제로 태우는 해석기**를 부른다★: `agent.*` 표의 동사들이 지목을 푸는 함수가
/// `core::agent::commands::resolve_in` 하나이고(그 표를 `/control/agent` 가 부른다), 이 대조가 그 함수를
/// 직접 태운다. 사본을 재면 사본만 초록이고 실입구는 아무 보장도 못 받는다.
///
/// ★이 대조가 덮는 축과 안 덮는 축★: 덮는 것은 **매칭 규칙**(정확 일치 · id 우선 · 동명 거부)이다. 두
/// 해석기의 **정의역**은 원래 다르다 — 우편은 산 로스터만 보고, 제어는 잠든 에이전트까지 본다. 그래서
/// 픽스처를 전부 산 것으로 두고 규칙만 맞댄다(정의역까지 같게 만드는 것은 이 슬라이스의 결정이 아니다).
mod resolver_alignment {
    use super::*;
    use engram_dashboard_command::ErrorCode;
    use engram_dashboard_core::agent::commands::{resolve_in, AgentRosterRow};
    use engram_dashboard_messaging::envelope::{DeliveryObservation, Entrance, EnvelopeFormat};
    use engram_dashboard_messaging::service::{
        AddressingSources, ControlPlanePort, DeliveryPort, FailCode, InjectReceipt, LiveAgent,
        MessagingService, SendMeta, SendStatus,
    };
    use engram_dashboard_messaging::{PeerId, SenderIdentity};

    /// 픽스처 한 줄 = (id, 이름).
    type Fixture = Vec<(PeerId, String)>;

    struct FixturePort {
        roster: Fixture,
        injected: Mutex<Vec<PeerId>>,
    }

    impl DeliveryPort for FixturePort {
        fn inject(&self, to_id: PeerId, bytes: &[u8]) -> Result<InjectReceipt, String> {
            self.injected.lock().expect("poisoned").push(to_id);
            Ok(InjectReceipt {
                bytes_requested: bytes.len(),
                bytes_written: bytes.len(),
                msg_uuid: uuid::Uuid::new_v4(),
                epoch: 0,
            })
        }
        fn live_agents(&self) -> Vec<LiveAgent> {
            self.roster
                .iter()
                .map(|(id, name)| LiveAgent {
                    id: *id,
                    name: name.clone(),
                    epoch: 0,
                    turn_signal: false,
                })
                .collect()
        }
        fn addressing_sources(&self) -> AddressingSources {
            AddressingSources {
                roster: self.live_agents(),
                // 잠든 층을 비워 두 해석기의 정의역을 맞춘다(위 모듈 주석).
                dormant_names: vec![],
            }
        }
        fn is_agent_live(&self, id: PeerId) -> bool {
            self.roster.iter().any(|(rid, _)| *rid == id)
        }
        fn live_id_for_name(&self, name: &str) -> Option<PeerId> {
            self.roster
                .iter()
                .find(|(_, n)| n == name)
                .map(|(id, _)| *id)
        }
        fn canonical_name(&self, id: PeerId) -> Option<String> {
            self.roster
                .iter()
                .find(|(rid, _)| *rid == id)
                .map(|(_, n)| n.clone())
        }
    }

    struct FixtureControlPlane;
    impl ControlPlanePort for FixtureControlPlane {
        fn envelope_format(&self) -> EnvelopeFormat {
            EnvelopeFormat::default()
        }
        fn record_delivery(&self, _obs: DeliveryObservation) {}
    }

    /// 두 입구의 결말을 맞대기 위한 **대조 어휘**. 어느 crate 의 계약도 아니고 이 파일 안에서만 산다 —
    /// 두 해석기의 반환 타입이 서로 다르므로(한쪽은 배달 행, 한쪽은 타입드 오류) 공통 축이 필요하다.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Resolution {
        Found(PeerId),
        NotFound,
        Ambiguous,
    }

    /// 우편 입구가 그 토큰을 어떻게 해석했나 — 위 대조 어휘로 환산한다.
    fn mail_resolution(fixture: &Fixture, token: &str) -> Resolution {
        let port = Arc::new(FixturePort {
            roster: fixture.clone(),
            injected: Mutex::new(vec![]),
        });
        let service = MessagingService::new(port.clone(), Arc::new(FixtureControlPlane));
        // 발신자는 로스터 밖 신원 — 자기 지목이 결과에 끼어들지 않게 한다.
        let from = SenderIdentity {
            peer_id: uuid::Uuid::new_v4(),
            epoch: 0,
        };
        let rows = service
            .handle_send(
                &format!("m-{}", uuid::Uuid::new_v4().simple()),
                from,
                "cross-check-sender",
                &[token.to_string()],
                "body",
                Entrance::Cli,
                &SendMeta::default(),
            )
            .expect("발송 단위 반려가 아닌 경우만 픽스처로 쓴다");
        let row = rows.first().expect("수신자 1명당 1행");
        match (row.status, row.code) {
            (SendStatus::Delivered, _) => {
                let injected = port.injected.lock().expect("poisoned");
                Resolution::Found(*injected.first().expect("배달된 수신자 id"))
            }
            (SendStatus::Failed, Some(FailCode::RecipientAmbiguous)) => Resolution::Ambiguous,
            (SendStatus::Failed, Some(FailCode::RecipientNotFound)) => Resolution::NotFound,
            other => panic!("이 픽스처가 낼 수 없는 결말: {other:?} ({token})"),
        }
    }

    /// 제어 입구가 그 토큰을 어떻게 해석했나 — **표의 동사들이 부르는 그 함수**를 태운다.
    ///
    /// 결말은 타입드 코드로 읽는다: 부재 = `NOT_FOUND` · 동명 둘 이상 = `CONFLICT`(그 함수가 내는 둘).
    fn control_resolution(fixture: &Fixture, token: &str) -> Resolution {
        let roster: Vec<AgentRosterRow> = fixture
            .iter()
            .map(|(id, name)| AgentRosterRow {
                id: *id,
                canonical_name: name.clone(),
                live: None,
                cwd: String::new(),
                parent: None,
            })
            .collect();
        match resolve_in(&roster, token) {
            Ok(found) => Resolution::Found(found.id),
            Err(e) if e.code() == ErrorCode::NotFound => Resolution::NotFound,
            Err(e) if e.code() == ErrorCode::Conflict => Resolution::Ambiguous,
            Err(e) => panic!("이 해석기가 낼 수 없는 결말: {e:?} ({token})"),
        }
    }

    #[test]
    fn control_and_mail_resolve_the_same_tokens_the_same_way() {
        let alice = uuid::Uuid::new_v4();
        let bob = uuid::Uuid::new_v4();
        let twin_a = uuid::Uuid::new_v4();
        let twin_b = uuid::Uuid::new_v4();
        // ★UUID 처럼 생긴 **이름**★: bob 의 id 를 이름으로 달고 있다 — id 지목을 가로채면 안 된다.
        let impostor = uuid::Uuid::new_v4();
        let fixture: Fixture = vec![
            (alice, "alice".to_string()),
            (bob, "bob".to_string()),
            (twin_a, "twin".to_string()),
            (twin_b, "twin".to_string()),
            (impostor, bob.to_string()),
        ];

        let tokens = vec![
            "alice".to_string(),  // 이름 정확 일치
            bob.to_string(),      // id 정확 일치(= 남의 이름이기도 하다)
            alice.to_string(),    // id 정확 일치(이름 충돌 없음)
            "twin".to_string(),   // 동명 2건
            "nobody".to_string(), // 부재
            "ALICE".to_string(),  // 대소문자 — 관대한 해석기라면 여기서 갈린다
            "ali".to_string(),    // 접두 — 유일 접두 매칭이 있으면 여기서 갈린다
        ];

        let mut agreed = 0;
        for token in &tokens {
            let control = control_resolution(&fixture, token);
            let mail = mail_resolution(&fixture, token);
            assert_eq!(
                control, mail,
                "두 입구의 지목 해석이 갈렸다(token={token}): control={control:?} mail={mail:?}"
            );
            agreed += 1;
        }
        assert_eq!(agreed, tokens.len(), "모든 토큰이 대조됐다");

        // ★알고 남긴 단 하나의 불일치 — 토큰 전처리 축★: 우편 입구는 대조 전에 토큰을 trim 한다(CLI 가
        //   `--to a, b` 를 콤마로 쪼갠 뒤 남는 공백을 구제하는 load-bearing 처리). 이 라우트는 쪼갬이 없어
        //   trim 하지 않는다. 두 방향 모두 실수로 뒤집히지 않게 **양쪽을 다 단언**한다 — 여기가 초록인 채로
        //   `resolve_in` 에 trim 을 넣거나 우편의 trim 을 지우면 이 테스트가 빨개진다.
        let padded = " alice ";
        assert_eq!(
            control_resolution(&fixture, padded),
            Resolution::NotFound,
            "제어 라우트는 정확 일치를 약속한다 — 패딩을 보정하지 않는다"
        );
        assert_eq!(
            mail_resolution(&fixture, padded),
            Resolution::Found(alice),
            "우편 입구는 콤마 분해 잔여 공백을 구제한다(그 trim 을 지우지 말 것)"
        );

        // 대조가 **무의미하게** 통과하지 않았음을 못박는다 — 세 결말이 실제로 다 나왔어야 한다.
        assert_eq!(
            control_resolution(&fixture, "alice"),
            Resolution::Found(alice)
        );
        assert_eq!(
            control_resolution(&fixture, &bob.to_string()),
            Resolution::Found(bob),
            "id 가 UUID 를 닮은 이름을 이긴다"
        );
        assert_eq!(control_resolution(&fixture, "twin"), Resolution::Ambiguous);
        assert_eq!(control_resolution(&fixture, "nobody"), Resolution::NotFound);
    }
}
