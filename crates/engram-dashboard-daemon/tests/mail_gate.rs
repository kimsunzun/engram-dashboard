//! ADR-0133 결정 3 통합 테스트 — **자격증명으로 우편을 거절하는 게이트**.
//!
//! ★여기가 유일한 강제 지점이다★: 에이전트 쪽 표식(`ENGRAM_MAIL`)은 사용법을 가릴 뿐이고 조작 가능하다.
//!   그래서 "표식을 뗀 프로세스" 를 흉내 낼 필요조차 없다 — 이 테스트는 HTTP 로 직접 때리므로 애초에
//!   표식이 없는 호출자다. 그런데도 거절돼야 한다는 것이 이 파일의 요점이다.
//!
//! ★claude 불요·결정적★: 실 에이전트를 띄우지 않는다. 우편 라우트는 messaging 슬롯을 비워 둬 **핸들러가
//!   503 을 내게** 한다 — 그 503 이 곧 "미들웨어를 통과했다" 는 증거다(거절은 200 + 반려 봉투라 구별된다).

use std::sync::{Arc, Mutex};

use engram_dashboard_core::agent::backend::accepts_mcp_config;
use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::preset::{Preset, PresetRegistry, PresetStore};
use engram_dashboard_core::agent::profile::{
    AgentCommand, AgentProfile, ClaudeOutputFormat, ProfileRegistry, ProfileStore,
};
use engram_dashboard_core::agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, NoopControlChannel, StatusSink,
};
use engram_dashboard_daemon::control::commands::make_daemon_table;
use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server, CommandTableSlot, ManagerSlot, McpServerHandle, MessagingSlot,
    RosterBroadcastSlot,
};
use engram_dashboard_daemon::control::priming::NoopPrimingProvider;
use engram_dashboard_daemon::control::registry::ControlRegistry;
use engram_dashboard_daemon::control::DaemonControlChannel;

/// 데몬이 우편을 거절할 때 싣는 코드. 데몬 소스의 상수는 crate-private 이라(미들웨어 내부 구현) 여기선
/// wire 값으로 적는다 — 그래서 이 문자열이 바뀌면 이 테스트가 빨개진다(그게 의도다: 코드 문자열은 CLI 가
/// 읽는 **wire 계약**이다).
const MAIL_NOT_ALLOWED: &str = "MAIL_NOT_ALLOWED";

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

struct MemPresetStore;
impl PresetStore for MemPresetStore {
    fn save(&self, _presets: &[Preset]) {}
    fn load(&self) -> Vec<Preset> {
        vec![]
    }
}

struct Fixture {
    base: String,
    /// MCP 가능 스폰이 받는 자격증명(우편 거절 대상).
    mcp_token: String,
    /// 비-MCP 스폰이 받는 자격증명(우편 허용).
    cli_token: String,
    _handle: McpServerHandle,
}

impl Fixture {
    async fn post(&self, token: &str, route: &str, body: serde_json::Value) -> (u16, String) {
        let resp = reqwest::Client::new()
            .post(format!("{}{route}", self.base))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {token}"))
            .json(&body)
            .send()
            .await
            .expect("http request");
        let status = resp.status().as_u16();
        (status, resp.text().await.unwrap_or_default())
    }
}

/// 우편 거절인가 — 응답이 200 + 검증된 반려 봉투 + 그 코드일 때만.
fn is_mail_rejection(status: u16, body: &str) -> bool {
    if status != 200 {
        return false;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    v.get("status").and_then(|s| s.as_str()) == Some("error")
        && v.get("code").and_then(|c| c.as_str()) == Some(MAIL_NOT_ALLOWED)
}

async fn fixture(tag: &str) -> Fixture {
    let registry = Arc::new(ControlRegistry::new());
    let manager_slot = Arc::new(ManagerSlot::new());
    let broadcast_slot = Arc::new(RosterBroadcastSlot::new());
    let command_slot = Arc::new(CommandTableSlot::new());
    let handle = start_mcp_server(
        registry.clone(),
        manager_slot.clone(),
        // ★비워 둔다★: 우편 핸들러가 503 을 내야 "게이트를 통과했다" 가 거절과 구별된다.
        Arc::new(MessagingSlot::new()),
        command_slot.clone(),
        engram_dashboard_daemon::command_roster::CommandRoster::new(),
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
    // ★제어 라우트는 표를 태운다(ADR-0155)★: 여기를 비워 두면 그 라우트가 503 을 내, 아래 「제어는 전원
    //   개방」 단언이 게이트가 아니라 배선 부재를 재게 된다.
    command_slot.set(Arc::new(make_daemon_table(manager, broadcast_slot)));

    // ★판정을 손으로 심는다 — 그래서 이 픽스처만으로는 부족하다★: `provision` 이 판정을 잘못 파생해도
    //   여기 심은 값은 그대로라 전부 초록으로 남는다. 그 축은 아래
    //   `a_credential_minted_by_the_real_provision_path_is_refused_end_to_end` 가 운영 경로로 덮는다.
    //   여기서 심는 이유는 라우트별 게이트 동작을 **판정 파생과 무관하게** 격리해 보기 위해서다.
    let mcp_token = format!("mcp-cred-{tag}");
    let cli_token = format!("cli-cred-{tag}");
    registry.issue(AgentId::new_v4(), 0, mcp_token.clone(), false);
    registry.issue(AgentId::new_v4(), 0, cli_token.clone(), true);

    let base = handle
        .url
        .strip_suffix("/mcp")
        .expect("mcp url suffix")
        .to_string();
    Fixture {
        base,
        mcp_token,
        cli_token,
        _handle: handle,
    }
}

/// 우편 라우트 전량 — 발송과 조회가 같은 축으로 갈린다(한쪽만 막으면 다른 쪽으로 장부를 읽는다).
const MAIL_ROUTES: [(&str, &str); 3] = [
    ("/control/send", r#"{"to":"bob","body":"hi"}"#),
    ("/control/messages", "{}"),
    ("/control/messages", r#"{"id":"m-1"}"#),
];

#[tokio::test]
async fn an_mcp_capable_credential_is_refused_on_every_mail_route() {
    let f = fixture("refuse").await;
    for (route, body) in MAIL_ROUTES {
        let payload: serde_json::Value = serde_json::from_str(body).expect("fixture body");
        let (status, text) = f.post(&f.mcp_token, route, payload).await;
        assert!(
            is_mail_rejection(status, &text),
            "{route} 는 이 자격증명으로 거절돼야: {status} {text}"
        );
    }
}

#[tokio::test]
async fn a_non_mcp_credential_still_reaches_the_mail_handlers() {
    let f = fixture("allow").await;
    for (route, body) in MAIL_ROUTES {
        let payload: serde_json::Value = serde_json::from_str(body).expect("fixture body");
        let (status, text) = f.post(&f.cli_token, route, payload).await;
        assert!(
            !is_mail_rejection(status, &text),
            "{route} 는 이 자격증명으로 거절되면 안 된다: {status} {text}"
        );
        assert_eq!(
            status, 503,
            "핸들러까지 갔다는 증거 — messaging 슬롯이 비어 503({route}): {text}"
        );
    }
}

/// ★제어는 전원 개방이다(ADR-0132 결정 5)★ — 우편이 막힌 자격증명도 제어 라우트에서는 정상 응답을 받는다.
///   이 단언이 없으면 게이트를 라우터 전체로 넓히는 회귀가 조용히 통과한다.
#[tokio::test]
async fn the_same_credential_passes_on_the_agent_control_route() {
    let f = fixture("control").await;
    for token in [&f.mcp_token, &f.cli_token] {
        let (status, text) = f
            .post(token, "/control/agent", serde_json::json!({"verb":"list"}))
            .await;
        assert_eq!(status, 200, "제어 라우트는 통과: {text}");
        let v: serde_json::Value = serde_json::from_str(&text).expect("JSON 응답");
        assert!(v.get("agents").is_some(), "명부 응답이어야: {text}");
        assert!(!is_mail_rejection(status, &text));
    }
}

/// ★발견과 전체 이름 호출도 제어 평면이다★ — 편지를 못 쓰는 자격증명이 **무엇을 부를 수 있는지조차** 못
///   배우면, 그 백엔드로 스폰된 에이전트는 자기 도구를 영영 모른다. 그것이 두 라우트를 「우편 아님」으로
///   분류한 이유 전부이므로, 분류 함수의 단위 시험 말고 **실제 미들웨어를 태운** 단언이 있어야 한다.
///
/// ★"거절이 아니다" 만 보지 않는다★: 그것만 보면 라우트가 통째로 사라져도 초록이다 — 응답이 그 라우트의
///   실제 계약(발견은 `commands` 배열, 호출은 명부 payload)인지까지 본다.
#[tokio::test]
async fn the_same_credential_passes_on_the_catalog_routes() {
    let f = fixture("catalog").await;
    for token in [&f.mcp_token, &f.cli_token] {
        let (status, text) = f
            .post(token, "/control/commands", serde_json::json!({}))
            .await;
        assert_eq!(status, 200, "발견은 통과: {text}");
        assert!(!is_mail_rejection(status, &text));
        let v: serde_json::Value = serde_json::from_str(&text).expect("JSON 응답");
        assert!(
            v.get("commands")
                .and_then(|c| c.as_array())
                .is_some_and(|c| !c.is_empty()),
            "발견 목록이어야: {text}"
        );

        let (status, text) = f
            .post(
                token,
                "/control/call",
                serde_json::json!({"name":"agent.list"}),
            )
            .await;
        assert_eq!(status, 200, "전체 이름 호출은 통과: {text}");
        assert!(!is_mail_rejection(status, &text));
        let v: serde_json::Value = serde_json::from_str(&text).expect("JSON 응답");
        assert!(v.get("agents").is_some(), "명부 응답이어야: {text}");
    }
}

/// MCP 라우트는 이 게이트의 대상이 아니다 — 우편을 MCP 로 쓰는 것이 바로 그 자격증명의 채널이기 때문이다.
///
/// ★"거절이 아니다" 만 보면 라우트가 통째로 사라져도 초록이다★: 그래서 **게이트를 통과했다** 를 라우트가
///   실제로 붙어 있다는 사실과 함께 본다. rmcp 가 handshake 없는 요청에 내는 구체적 상태코드는 판올림에
///   흔들리므로 거기 묶지 않고, ① 우리 거절 봉투가 아니고 ② 라우트 미존재(404)도 아니며 ③ 인증 실패(401)도
///   아니라는 세 가지로 좁힌다 — 그 셋이 아니면 요청은 auth·게이트를 지나 rmcp 에 도달한 것이다.
#[tokio::test]
async fn the_mcp_route_is_not_touched_by_the_mail_gate() {
    let f = fixture("mcp").await;
    let (status, text) = f
        .post(
            &f.mcp_token,
            "/mcp",
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        )
        .await;
    assert!(
        !is_mail_rejection(status, &text),
        "MCP 라우트가 우편 게이트에 걸리면 안 된다: {status} {text}"
    );
    assert_ne!(status, 404, "MCP 라우트가 붙어 있지 않다: {status} {text}");
    assert_ne!(
        status, 401,
        "유효 토큰인데 인증에서 끊겼다: {status} {text}"
    );
    // 대조군 — 같은 자격증명·같은 서버에서 우편 라우트는 거절된다(둘을 가르는 것이 경로뿐임을 고정).
    let (mail_status, mail_text) = f
        .post(
            &f.mcp_token,
            "/control/send",
            serde_json::json!({"to":"bob","body":"hi"}),
        )
        .await;
    assert!(
        is_mail_rejection(mail_status, &mail_text),
        "대조군: 같은 자격증명의 우편 라우트는 거절: {mail_status} {mail_text}"
    );
}

/// ★명단 밖 `/control/...` 는 두 방향으로 못박는다★: ① 우편이 막힌 자격증명에겐 **거절**(분류 누락이
///   조용히 입구를 열지 않는다 — fail-closed) ② 우편이 열린 자격증명에겐 **404**(그 경로가 게이트에
///   가려진 게 아니라 애초에 라우팅되지 않는다는 증명). ②가 없으면 나중에 누가 실제로 라우트를 붙여도
///   ①만 보고 초록으로 남는다.
#[tokio::test]
async fn an_unlisted_control_path_is_refused_when_blocked_and_unrouted_otherwise() {
    let f = fixture("unlisted").await;
    for path in ["/control/whatever", "/control/send/extra", "/control"] {
        let (status, text) = f.post(&f.mcp_token, path, serde_json::json!({})).await;
        assert!(
            is_mail_rejection(status, &text),
            "명단 밖 제어 경로는 막힌 자격증명에게 거절돼야({path}): {status} {text}"
        );
        let (status, text) = f.post(&f.cli_token, path, serde_json::json!({})).await;
        assert_eq!(
            status, 404,
            "그 경로는 실제로 라우팅되지 않아야({path}): {text}"
        );
    }
    // 제어 평면 밖은 이 접기의 대상이 아니다 — 막힌 자격증명에게도 404 그대로다. `/controlfoo` 류는
    //   이름만 비슷할 뿐 이 네임스페이스가 아니므로(세그먼트 경계) 여기 함께 둔다.
    for path in ["/nope", "/controlfoo", "/control-x"] {
        let (status, text) = f.post(&f.mcp_token, path, serde_json::json!({})).await;
        assert_eq!(
            status, 404,
            "제어 네임스페이스 밖까지 게이트를 넓히지 않는다({path}): {text}"
        );
    }
}

/// ★손으로 심은 판정이 하나도 없는 end-to-end★: 위 픽스처들은 `issue` 에 판정을 직접 넣으므로,
///   `provision` 이 모든 claude 자격증명을 우편 허용으로 발급하도록 회귀해도 전부 초록으로 남는다.
///   이 테스트만은 **운영 경로 그대로** 간다 — 실 `DaemonControlChannel::provision` 이 판정을 파생해
///   레지스트리에 박고, 그 토큰이 실제 HTTP 우편 요청에서 거절되는지 본다.
#[tokio::test]
async fn a_credential_minted_by_the_real_provision_path_is_refused_end_to_end() {
    let registry = Arc::new(ControlRegistry::new());
    let manager_slot = Arc::new(ManagerSlot::new());
    let handle = start_mcp_server(
        registry.clone(),
        manager_slot.clone(),
        Arc::new(MessagingSlot::new()),
        Arc::new(CommandTableSlot::new()),
        engram_dashboard_daemon::command_roster::CommandRoster::new(),
    )
    .await
    .expect("start mcp server");
    let base = handle
        .url
        .strip_suffix("/mcp")
        .expect("mcp url suffix")
        .to_string();

    let data_dir = std::env::temp_dir().join(format!("engram-mail-gate-e2e-{}", AgentId::new_v4()));
    let channel = DaemonControlChannel::new(
        registry.clone(),
        handle.url.clone(),
        data_dir.clone(),
        Some(std::path::PathBuf::from("C:/app/engram.exe")),
        Arc::new(NoopPrimingProvider),
    );

    // claude 백엔드의 실제 capability 를 그대로 넘긴다 — 여기 리터럴 true 를 적으면 그 판정이 다시
    //   테스트 사본이 된다.
    let accepts_mcp = accepts_mcp_config(&AgentCommand::Claude {
        extra_args: vec![],
        output_format: ClaudeOutputFormat::StreamJson,
    });
    assert!(accepts_mcp, "claude 는 MCP-capable 이어야(전제)");
    let ep = channel
        .provision(AgentId::new_v4(), 0, accepts_mcp)
        .expect("provision ok")
        .expect("endpoint");

    let resp = reqwest::Client::new()
        .post(format!("{base}/control/send"))
        .header("Authorization", format!("Bearer {}", ep.token))
        .json(&serde_json::json!({"to":"bob","body":"hi"}))
        .send()
        .await
        .expect("http request");
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    assert!(
        is_mail_rejection(status, &text),
        "실 provision 이 발급한 claude 자격증명은 HTTP 우편에서 거절돼야: {status} {text}"
    );
    // 표식도 같은 판정에서 나왔는지 함께 본다(교육과 강제가 한 값에서 갈린다 — ADR-0133 결정 2).
    assert!(!ep.mail_allowed, "endpoint 표식도 off 여야");

    let _ = std::fs::remove_dir_all(&data_dir);
}
