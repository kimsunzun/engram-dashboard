//! `/control/agent` 입구(ADR-0132 결정 6) — 에이전트 명부·수명 제어 동사의 공통 파이프라인.
//!
//! ★역할★: CLI(`engram agent …`)가 POST 한 `{verb, …}` 를 받아 **WS 디스패처가 부르는 것과 같은
//!   `AgentManager` 메서드**를 부른다. 매니저 로직을 복제하지 않는다 — 이 모듈이 더하는 것은 (a) 이름/id
//!   지목의 해석 (b) 결말의 wire 번역 (c) 명부 변경의 클라이언트 통지, 셋뿐이다.
//!
//! ★지금 이 라우트에 닿을 수 있는 자는 둘뿐이다(ADR-0132 결정 5 는 아직 도달 불가)★: **비-MCP 스폰
//!   에이전트**와 **데몬 크레덴셜을 쥔 사람**이다. 결정 5("제어는 전원 개방")를 여기서 완성하려면 MCP 가능
//!   스폰에도 CLI 를 깔아야 하는데, 그 배선은 **우편 CLI 도 함께 되살린다**(`backend/claude.rs` 의 env·PATH
//!   주입이 계열 단위가 아니라 실행파일 단위라 갈라 깔 수 없다). ADR-0128 의 우편 단일화를 지키는 것이
//!   지금은 그 "안 깔기" 분기 하나뿐이므로(ADR-0132 §영향의 ★표), **우편 거절 게이트(결정 3)가 서기 전에는
//!   스폰 배선을 건드리지 않는다**. 즉 여기 도달 범위가 좁은 것은 결함이 아니라 순서다.
//!
//! ★`kill`·`rm` 이 없는 것도 미구현이 아니다★ — `CLI_AGENT_VERBS` 주석이 정본(ADR-0122 미해소).
//!
//! ★입력 규율(세 가지 — 전부 조용한 오작동을 막으려는 것)★
//!   - **모르는 필드는 거부한다.** `parnet` 오타를 흘려보내면 `move … --parent lead` 가 **루트로 떼기**로
//!     조용히 바뀐다. 동사가 쓰지 않는 필드도 같은 이유로 거부한다(무시하면 인자가 사라진 걸 호출자가 못 본다).
//!   - **공백만 있는 값은 거부한다.** 셸에서 미설정 변수가 빈 인자로 펼쳐지는 형태(`--parent "$UNSET"`)가
//!     현실적으로 들어오는데, 그걸 "안 준 것" 으로 접으면 같은 사고가 난다.
//!   - **지목 토큰을 다듬지 않는다(trim 없음).** 이 라우트는 **정확 일치**를 약속하므로 `"worker "` 는
//!     `worker` 가 아니다 — 다듬으면 약속과 코드가 갈리고, 패딩된 이름이 도착했다는 호출자 버그가 묻힌다.
//!     (저장 측 표시명 정규화는 매니저 소관이라 그대로 둔다 — 여기서 흉내 내지 않는다.)
//!
//! ★어휘★: wire 필드·힌트 문구는 전부 **에이전트 어휘**다(ADR-0119 결정 5 — "프로필" 은 매니저 경계 밖으로
//!   나가지 않는다). 매니저 호출에 `AgentProfile` 이 필요한 것은 그 API 형태 때문이고, 그 타입은 이 파일
//!   밖으로도 wire 로도 나가지 않는다.
//!
//! tauri import 0(daemon crate).

use std::sync::Arc;

use engram_dashboard_core::agent::manager::{AgentManager, RenameOutcome, RosterEntry};
use engram_dashboard_core::agent::profile::{
    AgentCommand, AgentProfile, ClaudeOutputFormat, SpawnMode,
};
use engram_dashboard_core::agent::types::{
    AgentId, AgentStatus, PtyError, AGENT_STATE_LIVE, AGENT_STATE_SLEEPING, CLI_EXE_NAME,
    CLI_GROUP_AGENT, RENAME_OUTCOME_RENAMED, RENAME_OUTCOME_UNCHANGED,
};

use super::ingress::ControlQueryResult;

/// 명부가 바뀌었음을 붙어 있는 클라이언트 전원에게 알리는 출구(포트).
///
/// ★왜 포트인가★: 실제 통지는 전-연결 팬아웃으로 `ProfileListUpdated` 를 미는 것인데, 그 조립은
///   `connection_core` 소유다. `control/` 이 그쪽을 직접 부르면 데몬 층 결정(ADR-0130)이 재론 대상이 되므로
///   (이 디렉토리의 나가는 간선 0 이 추적 중인 성질이다) 소비자인 여기가 좁은 trait 만 소유하고 실물
///   어댑터는 조립부가 준다 — 메시징 커널의 포트 규율(ADR-0110)과 같은 모양이다.
/// ★이게 없으면 나는 증상★: 에이전트가 이름을 바꾸거나 형제를 띄워도 대시보드 트리는 **무관한 이벤트가
///   올 때까지 옛 명부를 보여 준다**(조용한 stale).
// ADR-0132
// ADR-0130
pub trait RosterBroadcast: Send + Sync {
    /// 논블록이어야 한다 — 팬아웃은 연결별 큐에 try_send 만 한다.
    fn roster_changed(&self);
}

/// 상태 → wire 어휘의 **단일 매핑**.
///
/// ★한 함수여야 하는 이유(실제로 갈렸던 자리)★: 전엔 변경 동사가 `"live"` 를 박고 `list` 는 명부 항목에서
///   파생해, **같은 라우트의 두 동사가 같은 에이전트를 두고 서로 다른 답을 낼 수 있었다** — 깨우자마자 죽은
///   에이전트(ADR-0082 resume 조기 종료)를 `spawn` 은 살아 있다고, 직후의 `list` 는 잠들었다고 보고한다.
///   그러면 호출자는 시체에게 편지를 쓴다.
/// ★술어는 `AgentStatus::is_live` 하나★ — 명부(`roster`)가 시체를 거르는 데 쓰는 그 술어다(ADR-0116).
fn state_of(status: &AgentStatus) -> &'static str {
    if status.is_live() {
        AGENT_STATE_LIVE
    } else {
        AGENT_STATE_SLEEPING
    }
}

fn agent_payload(id: AgentId, name: &str, state: &str) -> serde_json::Value {
    serde_json::json!({ "id": id.to_string(), "name": name, "state": state })
}

/// 띄우기 응답 본문 — `spawn` 두 형태가 공유한다.
///
/// ★별도 함수인 이유는 **테스트 가능성** 하나다★: 상태를 지어내지 않고 `state_of` 로 파생한다는 성질은 이
///   슬라이스를 시작하게 만든 결함(하드코딩된 `"live"`)의 재발 방지선인데, 라우트 통합 테스트로는 그걸 못
///   고정한다 — 그 자리에서 결정적으로 **살아 있지 않은** 에이전트를 만들 방법이 없어(셸은 계속 살아 있고,
///   죽는 백엔드는 머신에 달렸다) 어떤 응답이든 `"live"` 로도 맞아떨어진다. 순수 함수로 떼어 내면 terminal
///   상태의 `AgentInfo` 를 손으로 만들어 넣을 수 있고, 그때 리터럴은 즉시 틀린다.
fn spawn_payload(
    id: AgentId,
    name: &str,
    status: &AgentStatus,
    created: bool,
) -> serde_json::Value {
    serde_json::json!({
        "agent": agent_payload(id, name, state_of(status)),
        "created": created,
    })
}

/// wire 필드 이름 — 반려 문구와 동사별 허용 목록이 같은 문자열을 봐야 "모르는 필드" 판정이 실제 표면과
/// 어긋나지 않는다.
const FIELD_TARGET: &str = "target";
const FIELD_CWD: &str = "cwd";
const FIELD_NAME: &str = "name";
const FIELD_PARENT: &str = "parent";

/// 문자열 필드 역직렬화 — **부재 · null · 값** 셋을 가른다(바깥 `Option` = 실렸나, 안쪽 = 값이 있나).
///
/// ★왜 모든 필드에 셋이 필요한가(load-bearing)★: 두 가지가 여기 걸려 있다.
///   ① `move` 에서 `null` 은 "안 줬다" 가 아니라 **"루트로 떼라"** 는 적극적 지시다. 평범한
///      `Option<String>` 은 둘을 같은 값으로 접어, 필드를 빠뜨린 요청이 조용히 계층 해제로 실행된다.
///   ② 동사별 허용 검사가 **실린 것**을 봐야 한다. 접어 버리면 `{"verb":"new","target":null}` 이
///      "target 을 안 보냈다" 로 보여 검사를 통과하고, 호출자는 자기가 보낸 필드가 무시된 줄 모른다.
fn transmitted<'de, D>(de: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(de).map(Some)
}

/// 실렸든 아니든 **값**만 꺼낸다(동사 본문이 보는 축).
fn value(field: &Option<Option<String>>) -> Option<&str> {
    field.as_ref().and_then(|v| v.as_deref())
}

/// `/control/agent` 요청 바디. `verb` 만 필수이고 나머지는 동사별로 쓰인다.
///
/// ★평평한 struct 인 이유(태그드 enum 아님)★: 태그드 enum 은 동사마다 형태가 갈려 **역직렬화가 실패**하는
///   경우가 훨씬 넓고, 실패는 호출자에게 "무엇을 고쳐야 하는지" 를 말해 주지 못한다. 평평하게 받아 동사별로
///   검증하면 그 대부분이 코드·힌트가 붙은 반려로 나간다.
/// ★그래도 역직렬화가 실패하는 부류는 남는다(정직한 범위)★: 타입이 어긋난 값(`{"verb": 5}`)·중복 키·객체가
///   아닌 바디·깨진 JSON 바이트는 여기까지 오지 못한다. 그 부류도 빈 400 이 되지 않도록 **어댑터가 직접
///   역직렬화해 serde 의 사유 문구를 봉투에 실어 보낸다**(`mcp_server::control_agent_handler`) — 즉 이 라우트가
///   빈 body 를 내는 경우는 인증 실패와 요청 크기 초과뿐이다.
#[derive(Debug, Default, serde::Deserialize)]
pub struct AgentRequest {
    /// ★필수인데 `default` 인 이유★: 없으면 역직렬화가 실패해 **빈 400** 으로 끝나는데, 그 응답은 무엇을
    ///   고쳐야 하는지 말해 주지 않는다. 빈 문자열로 받아 아래 dispatch 가 "모르는 동사" 반려를 내면
    ///   호출자가 help 로 갈 수 있다.
    #[serde(default)]
    pub verb: String,
    /// 대상 지목 — 이름 또는 정확한 agent id(`resolve_target`).
    #[serde(default, deserialize_with = "transmitted")]
    pub target: Option<Option<String>>,
    #[serde(default, deserialize_with = "transmitted")]
    pub cwd: Option<Option<String>>,
    #[serde(default, deserialize_with = "transmitted")]
    pub name: Option<Option<String>>,
    /// `move` 의 새 부모. `None` = 필드 부재(반려) · `Some(None)` = 루트로 떼기 · `Some(Some(n))` = 그 부모로.
    #[serde(default, deserialize_with = "transmitted")]
    pub parent: Option<Option<String>>,
    /// ★모르는 필드를 여기로 모은다(`deny_unknown_fields` 대신)★: 그 attribute 는 역직렬화를 실패시켜 빈
    ///   400 이 되는데, 그러면 호출자는 **어느 키가 문제인지** 알 수 없다. 모아 두면 반려 문구가 그 키를
    ///   지목할 수 있다.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// 지목 해석 결과. **모호(동명 2개 이상)를 부재와 갈라 놓는 것이 요점**이다 — 뭉치면 "그런 에이전트 없음"
/// 이라고 답하면서 실제로는 둘이 있는 상태가 된다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetResolution {
    Found(AgentId),
    NotFound,
    Ambiguous,
}

/// 지목 토큰 → 에이전트 하나.
///
/// ★규칙(우편 입구와 **같아야** 한다)★: ① agent id 문자열 정확 일치를 **먼저** 본다 — 그래야 UUID 처럼
///   생긴 *이름* 이 id 지목을 가로채지 못한다 ② 그다음 이름 **정확** 일치(대소문자 구분 · 접두 매칭 없음 ·
///   공백 보정 없음) ③ 같은 이름이 둘 이상이면 아무도 고르지 않고 거부한다.
/// ★왜 "같아야" 가 계약인가★: 사람과 LLM 은 `engram mail send --to X` 와 `engram agent rename X …` 가 같은
///   X 를 가리킨다고 읽는다. 두 입구의 해석이 갈리면 편지를 받은 그 에이전트와 이름이 바뀐 에이전트가
///   달라진다. 이 저장소엔 **관대한 해석기가 따로 있었다**(옛 앱 CLI 의 대소문자 무시 + 유일 접두 매칭) —
///   그 규칙을 여기로 들이지 말 것.
/// ★같은 것은 **매칭 규칙**이고, 토큰 전처리는 의도적으로 다르다(정확히 이 축만)★: 우편 입구는 수신자
///   토큰을 대조 **전에** trim 한다(`service.rs` — CLI 가 `--to a, b` 를 콤마로 쪼갠 뒤 남는 공백을 구제하는
///   load-bearing 처리라 지우면 안 된다). 이 라우트는 토큰이 argv/JSON 값 하나로 오므로 쪼갬이 없고, 그래서
///   trim 하지 않는다 — 즉 `"worker "` 는 우편에선 닿고 여기선 안 닿는다. **그 차이는 알고 남긴 것**이며
///   `tests/control_agent.rs` 의 교차 대조가 (a) 다듬을 것 없는 토큰에서의 규칙 일치와 (b) 이 한 축의
///   불일치를 **둘 다** 단언한다(우편 입구를 실제로 태워서).
// ADR-0132
pub fn resolve_target(roster: &[RosterEntry], token: &str) -> TargetResolution {
    if let Some(e) = roster.iter().find(|e| e.id.to_string() == token) {
        return TargetResolution::Found(e.id);
    }
    let mut by_name = roster.iter().filter(|e| e.canonical_name == token);
    match (by_name.next(), by_name.next()) {
        (Some(e), None) => TargetResolution::Found(e.id),
        (Some(_), Some(_)) => TargetResolution::Ambiguous,
        _ => TargetResolution::NotFound,
    }
}

/// 새로 만드는 에이전트의 백엔드(내부 결정 — 사용자 보고 대상).
///
/// ★왜 StreamJson 인가★: 이 계열을 부르는 주체가 LLM 이고, 프론트에서 **LLM 이 부르는** 생성 커맨드
///   (`agentlist.createAgent`)의 기본값이 이미 StreamJson 이다(ADR-0078). 두 입구가 같은 동사에 다른 기본을
///   주면 "만들었는데 화면이 다르다" 가 된다. 형식을 고르는 플래그는 두지 않았다 — 지금 표면에 필요한 축이
///   아니고, 필요해지면 플래그 하나를 더하는 무파괴 확장이다.
const NEW_AGENT_OUTPUT_FORMAT: ClaudeOutputFormat = ClaudeOutputFormat::StreamJson;

/// 제어 동사 공통 핸들러 — HTTP 어댑터가 유일하게 부르는 진입점.
///
/// ★blocking 함수다(호출자 계약)★: `activate_profile` 은 resume 모드에서 조기 종료를 **약 3초 폴링**하고
///   (manager doc), 이름 변경·계층 이동은 프로필 락을 쥔 채 디스크에 저장한다. 그래서 async 런타임 스레드가
///   아니라 blocking 풀에서 불러야 한다 — WS 디스패처가 같은 메서드를 async 컨텍스트에서 인라인으로 부르는
///   것은 따라 할 패턴이 아니라 남아 있는 흠이다. 어댑터(`mcp_server::control_agent_handler`)가
///   `spawn_blocking` 으로 감싼다.
/// ★`broadcast` 가 `None` 인 조립★: 클라이언트가 붙을 수 없는 하네스·스모크 bin(팬아웃 자체가 없다). 운영
///   데몬에서 None 이면 명부 변경이 화면에 반영되지 않는다.
pub fn handle_agent(
    manager: &Arc<AgentManager>,
    broadcast: Option<&Arc<dyn RosterBroadcast>>,
    req: AgentRequest,
) -> ControlQueryResult {
    // 형태 검사는 동사 해석보다 먼저다 — 오타 하나가 **다른 동작**이 되는 것을 막는 게 목적이라, 어느
    //   동사인지와 무관하게 걸려야 한다.
    if let Some(rejection) = reject_unknown_fields(&req) {
        return rejection;
    }
    if let Some(rejection) = reject_blank_fields(&req) {
        return rejection;
    }

    match req.verb.as_str() {
        "list" => guard(&req, "list", &[], || verb_list(manager)),
        "spawn" => guard(&req, "spawn", &[FIELD_TARGET, FIELD_CWD, FIELD_NAME], || {
            verb_spawn(manager, broadcast, &req)
        }),
        "new" => guard(&req, "new", &[FIELD_CWD, FIELD_NAME], || {
            verb_new(manager, broadcast, &req)
        }),
        "rename" => guard(&req, "rename", &[FIELD_TARGET, FIELD_NAME], || {
            verb_rename(manager, broadcast, &req)
        }),
        "move" => guard(&req, "move", &[FIELD_TARGET, FIELD_PARENT], || {
            verb_move(manager, broadcast, &req)
        }),
        other => bad_args(format!(
            "unknown verb '{other}' — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}` for this group's verbs."
        )),
    }
}

/// 동사가 쓰지 않는 필드가 실려 있으면 **무시하지 않고 반려**한다.
///
/// ★무시가 위험한 이유★: `spawn <이름> --name x` 를 흘려보내면 호출자는 이름이 바뀐 줄 알지만 아무 일도
///   일어나지 않았다(개명은 `rename` 이다). 인자가 조용히 사라지는 형태의 사고는 이 CLI 가 계속 막아 온 부류다.
fn guard(
    req: &AgentRequest,
    verb: &str,
    allowed: &[&str],
    run: impl FnOnce() -> ControlQueryResult,
) -> ControlQueryResult {
    let stray: Vec<&str> = present_fields(req)
        .into_iter()
        .filter(|f| !allowed.contains(f))
        .collect();
    if stray.is_empty() {
        return run();
    }
    let takes = if allowed.is_empty() {
        "no other fields".to_string()
    } else {
        allowed.join(", ")
    };
    bad_args(format!(
        "{verb} does not use: {} (it takes {takes}) — dropping them silently would hide an argument that never took effect; run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`.",
        stray.join(", ")
    ))
}

fn present_fields(req: &AgentRequest) -> Vec<&'static str> {
    let mut present = Vec::new();
    if req.target.is_some() {
        present.push(FIELD_TARGET);
    }
    if req.cwd.is_some() {
        present.push(FIELD_CWD);
    }
    if req.name.is_some() {
        present.push(FIELD_NAME);
    }
    if req.parent.is_some() {
        present.push(FIELD_PARENT);
    }
    present
}

fn reject_unknown_fields(req: &AgentRequest) -> Option<ControlQueryResult> {
    if req.extra.is_empty() {
        return None;
    }
    let keys: Vec<&str> = req.extra.keys().map(|k| k.as_str()).collect();
    Some(bad_args(format!(
        "unknown field(s): {} — a typo here would silently become a different operation, so the call is refused; run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`.",
        keys.join(", ")
    )))
}

/// 값이 공백만인 필드는 **부재로 접지 않고** 반려한다(모듈 헤더 입력 규율).
fn reject_blank_fields(req: &AgentRequest) -> Option<ControlQueryResult> {
    let blanks: Vec<&str> = [
        (FIELD_TARGET, value(&req.target)),
        (FIELD_CWD, value(&req.cwd)),
        (FIELD_NAME, value(&req.name)),
        (FIELD_PARENT, value(&req.parent)),
    ]
    .into_iter()
    .filter(|(_, v)| v.is_some_and(|s| s.trim().is_empty()))
    .map(|(field, _)| field)
    .collect();
    if blanks.is_empty() {
        return None;
    }
    Some(bad_args(format!(
        "blank value(s) for: {} — an empty argument is usually an unset shell variable, and treating it as 'not given' would run a different command than you typed; pass a value or drop the field.",
        blanks.join(", ")
    )))
}

/// 바디 자체가 JSON 계약을 못 지킨 경우의 반려 — 어댑터가 부른다(`control_agent_handler`).
///
/// ★빈 body 400 을 쓰지 않는 이유★: 이 라우트의 호출자는 자기교정하는 LLM 이라 **사유가 실려야** 한다.
///   serde 의 문구(어느 필드가 어떤 타입이어야 하는지·중복 키·JSON 구문 위치)를 그대로 싣는다.
/// ★바디 원문은 싣지 않는다★ — 잘린 앞부분만 형태 파악용으로 붙인다(요청 바디에 뭐가 들었든 응답으로
///   되돌려 주는 것은 입구가 할 일이 아니다).
pub fn malformed_body(reason: &str, raw: &str) -> ControlQueryResult {
    const PREVIEW: usize = 80;
    let head: String = raw.chars().take(PREVIEW).collect();
    bad_args(format!(
        "the request body is not a valid {CLI_GROUP_AGENT} command object: {reason} — it must be a JSON object like {{\"verb\":\"list\"}}; got: {head}"
    ))
}

fn bad_args(hint: String) -> ControlQueryResult {
    ControlQueryResult::Error {
        code: "INVALID_AGENT_ARGS",
        hint,
    }
}

fn verb_list(manager: &Arc<AgentManager>) -> ControlQueryResult {
    // ★한 번의 명부 조회로 전부 만든다★: 이름·생사·cwd·계층이 **같은 스냅샷**에서 나와야 한 응답 안의
    //   행끼리 정합한다. 예전엔 저장 스냅샷을 따로 떠 합쳤고, 그 사이에 생긴 에이전트가 cwd 없는 행으로,
    //   그 사이의 계층 이동이 옛 부모로 나왔다(ADR-0119 가 금지한 소비자측 합성이기도 하다).
    let agents: Vec<serde_json::Value> = manager
        .roster()
        .into_iter()
        .map(|entry| {
            let state = entry
                .live
                .as_ref()
                .map(|info| state_of(&info.status))
                .unwrap_or(AGENT_STATE_SLEEPING);
            let mut row = agent_payload(entry.id, &entry.canonical_name, state);
            row["cwd"] = serde_json::Value::String(entry.cwd);
            row["parent"] = match entry.parent {
                Some(p) => serde_json::Value::String(p.to_string()),
                None => serde_json::Value::Null,
            };
            row
        })
        .collect();
    ControlQueryResult::Ok(serde_json::json!({ "agents": agents }))
}

fn verb_spawn(
    manager: &Arc<AgentManager>,
    broadcast: Option<&Arc<dyn RosterBroadcast>>,
    req: &AgentRequest,
) -> ControlQueryResult {
    // ★두 동사가 한 이름을 쓴다(깨우기 / 만들어서 띄우기)★: 어느 쪽인지는 **인자가** 가른다. 둘 다 주면
    //   어느 뜻인지 고를 근거가 없고, 조용히 한쪽을 고르면 만들려던 에이전트 대신 남을 깨운다.
    match (value(&req.target), value(&req.cwd)) {
        (Some(_), Some(_)) => bad_args(format!(
            "spawn takes either an existing agent (target) or cwd for a new one, not both — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`."
        )),
        (None, None) => bad_args(format!(
            "spawn needs either an existing agent to wake (target) or cwd to create a new one — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`."
        )),
        // ★조용히 무시하지 않는다★: 깨우기는 이름을 바꾸지 않으므로 `name` 이 아무 일도 못 한다.
        (Some(token), None) if value(&req.name).is_some() => bad_args(format!(
            "name does not apply when waking an existing agent ({token}); use `{CLI_EXE_NAME} {CLI_GROUP_AGENT} rename` to change a name, or drop it."
        )),
        (Some(token), None) => wake_existing(manager, token),
        (None, Some(cwd)) => {
            create_and_start(manager, broadcast, cwd, value(&req.name).map(str::to_string))
        }
    }
}

fn wake_existing(manager: &Arc<AgentManager>, token: &str) -> ControlQueryResult {
    let id = match resolve_or_reject(manager, token) {
        Ok(id) => id,
        Err(e) => return e,
    };
    let Some(agent) = manager.agent_snapshot(id) else {
        return not_found(token);
    };
    // ★모드 유도 규칙은 WS 경로와 같은 것을 쓴다(ADR-0076)★: 저장된 세션이 있으면 이어받기, 없으면 새로.
    //   여기서 다른 규칙을 쓰면 같은 에이전트가 어느 입구로 깨우느냐에 따라 대화 이력을 잃는다.
    let mode = if agent.claude_session_id.is_some() {
        SpawnMode::Resume
    } else {
        SpawnMode::Fresh
    };
    match manager.activate_profile(&agent, mode) {
        // 깨우기는 명부의 **구성**을 바꾸지 않는다(항목 수·이름·계층 불변). 생사 전이는 매니저가 이미
        //   흘린다 — `spawn_agent` 가 성공 직전 `status_sink.agent_list_updated` 를 내고 데몬 sink 이 그걸
        //   전 연결에 팬아웃한다. 그래서 여기서 명부 통지를 겹쳐 보내지 않는다(WS 의 깨우기 경로도 같다).
        Ok(info) => ControlQueryResult::Ok(spawn_payload(info.id, &info.name, &info.status, false)),
        Err(e) => ControlQueryResult::Error {
            code: "SPAWN_FAILED",
            hint: format!("could not start agent '{token}': {e}"),
        },
    }
}

fn create_and_start(
    manager: &Arc<AgentManager>,
    broadcast: Option<&Arc<dyn RosterBroadcast>>,
    cwd: &str,
    name: Option<String>,
) -> ControlQueryResult {
    let stored = match register(manager, cwd, name) {
        Ok(p) => p,
        Err(e) => return e,
    };
    notify(broadcast);
    match manager.activate_profile(&stored, SpawnMode::Fresh) {
        Ok(info) => ControlQueryResult::Ok(spawn_payload(info.id, &info.name, &info.status, true)),
        // ★등록을 되돌리지 않는다★: 만들어진 에이전트는 명부에 남아 잠든 상태로 보인다. 되감기는 두 번째
        //   삭제 경로를 만드는데 삭제의 semantics 자체가 미결이고(ADR-0122), 조용히 지우면 호출자는 이름을
        //   점유한 채 사라진 에이전트를 못 본다. 그래서 사실대로 알린다.
        Err(e) => ControlQueryResult::Error {
            code: "SPAWN_FAILED",
            hint: format!(
                "agent '{}' ({}) was created but did not start: {e} — it is registered and asleep; try `{CLI_EXE_NAME} {CLI_GROUP_AGENT} spawn {}`.",
                stored.canonical_name_when_live(),
                stored.id,
                stored.canonical_name_when_live()
            ),
        },
    }
}

fn verb_new(
    manager: &Arc<AgentManager>,
    broadcast: Option<&Arc<dyn RosterBroadcast>>,
    req: &AgentRequest,
) -> ControlQueryResult {
    let Some(cwd) = value(&req.cwd) else {
        return bad_args(format!(
            "new needs cwd (the folder the agent works in) — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`."
        ));
    };
    let stored = match register(manager, cwd, value(&req.name).map(str::to_string)) {
        Ok(p) => p,
        Err(e) => return e,
    };
    notify(broadcast);
    ControlQueryResult::Ok(serde_json::json!({
        "agent": agent_payload(
            stored.id,
            &stored.canonical_name_when_live(),
            AGENT_STATE_SLEEPING,
        ),
    }))
}

/// 명부 등록 공통부 — `new` 와 `spawn --cwd` 가 같은 자리를 쓴다.
///
/// ★확정 이름은 요청 이름과 다를 수 있다★: 명부 전역 유일성 때문에 접미사가 붙는다(ADR-0120/0123). 그래서
///   응답은 **등록된 값**에서 이름을 읽는다 — 요청 이름을 되돌려주면 화면·주소와 어긋난다.
fn register(
    manager: &Arc<AgentManager>,
    cwd: &str,
    name: Option<String>,
) -> Result<AgentProfile, ControlQueryResult> {
    let mut agent = AgentProfile::new(
        cwd.to_string(),
        AgentCommand::Claude {
            extra_args: vec![],
            output_format: NEW_AGENT_OUTPUT_FORMAT,
        },
        std::path::PathBuf::from(cwd),
        vec![],
        false,
    );
    agent.display_name = name;
    // ★상한 판정은 코어가 한다(입구가 아니라)★: 여기서 미리 세면 입구마다 사본이 생기고, 그 사본이 없는
    //   입구(데스크톱 CreateProfile · ad-hoc spawn)는 그냥 통과한다 — 실제로 그렇게 새어 있었다. 코어는
    //   등록을 커밋하는 게이트 안에서 세므로 원자적이고 입구 수와 무관하다. 여기 남는 일은 **번역**뿐이다.
    manager.create_agent(agent).map_err(|e| match e {
        PtyError::RosterFull { current, limit } => ControlQueryResult::Error {
            code: "ROSTER_FULL",
            hint: format!(
                "the team already has {current} agents, which is the safety ceiling ({limit}) that stops a runaway create loop — it is not a product limit and raising it is not the fix. Remove agents you no longer need from the tree in the dashboard, then try again."
            ),
        },
        other => ControlQueryResult::Error {
            code: "NAME_SPACE_EXHAUSTED",
            hint: format!("could not register a new agent: {other}"),
        },
    })
}

fn verb_rename(
    manager: &Arc<AgentManager>,
    broadcast: Option<&Arc<dyn RosterBroadcast>>,
    req: &AgentRequest,
) -> ControlQueryResult {
    let Some(token) = value(&req.target) else {
        return bad_args(format!(
            "rename needs the agent to rename (target) — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`."
        ));
    };
    let Some(name) = value(&req.name).map(str::to_string) else {
        return bad_args(format!(
            "rename needs the new name — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`."
        ));
    };
    let id = match resolve_or_reject(manager, token) {
        Ok(id) => id,
        Err(e) => return e,
    };
    // ★네 결말을 뭉개지 않는다★: 확정된 이름(접미사가 붙었을 수 있다) · 이미 그 계열을 쥐어 무변경 · 대상
    //   부재 · 이름 공간 소진. 앞 둘은 성공이지만 **다른 사실**이라 outcome 으로 갈라 싣는다 — 뭉치면
    //   호출자는 자기가 요청한 이름이 실제로 붙었는지 알 수 없다.
    match manager.rename_agent(id, Some(name)) {
        RenameOutcome::Renamed(committed) => {
            notify(broadcast);
            ControlQueryResult::Ok(serde_json::json!({
                "agent": { "id": id.to_string(), "name": committed },
                "outcome": RENAME_OUTCOME_RENAMED,
            }))
        }
        RenameOutcome::Unchanged(kept) => {
            notify(broadcast);
            ControlQueryResult::Ok(serde_json::json!({
                "agent": { "id": id.to_string(), "name": kept },
                "outcome": RENAME_OUTCOME_UNCHANGED,
            }))
        }
        RenameOutcome::NotFound => not_found(token),
        RenameOutcome::Exhausted => ControlQueryResult::Error {
            code: "NAME_SPACE_EXHAUSTED",
            hint: format!(
                "every numbered variant of that name is taken, so '{token}' was left as it is."
            ),
        },
    }
}

fn verb_move(
    manager: &Arc<AgentManager>,
    broadcast: Option<&Arc<dyn RosterBroadcast>>,
    req: &AgentRequest,
) -> ControlQueryResult {
    let Some(token) = value(&req.target) else {
        return bad_args(format!(
            "move needs the agent to move (target) — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`."
        ));
    };
    // 부재는 반려다 — "부모를 안 줬으니 떼자" 로 읽으면 오타 한 번이 계층 해제가 된다(null 이 그 지시다).
    let Some(parent_request) = req.parent.as_ref() else {
        return bad_args(format!(
            "move needs parent: a name/id to move under, or null to move it back to the top level — run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`."
        ));
    };
    let child = match resolve_or_reject(manager, token) {
        Ok(id) => id,
        Err(e) => return e,
    };
    let parent_id = match parent_request.as_deref() {
        None => None,
        Some(p) => match resolve_or_reject(manager, p) {
            Ok(id) => Some(id),
            Err(e) => return e,
        },
    };
    if manager.reparent_agent(child, parent_id) {
        notify(broadcast);
        ControlQueryResult::Ok(serde_json::json!({
            "agent": { "id": child.to_string(), "name": name_of(manager, child) },
            "parent": parent_id.map(|p| p.to_string()),
        }))
    } else {
        // ★사유 목록은 `ProfileRegistry::reparent` 의 거부 조건과 **한 줄씩 대응**한다★: 목록에 없는 사유로
        //   거부당한 호출자는 다음에 무엇을 할지 알 수 없다. 특히 **"옮기려는 쪽이 이미 부모"** 는 2단 트리에서
        //   실제로 자주 걸리는 조건인데 예전 문구엔 빠져 있었다.
        ControlQueryResult::Error {
            code: "MOVE_REJECTED",
            hint: format!(
                "'{token}' cannot go there. The tree is one level deep, so: an agent cannot be its own parent; the new parent must itself be top-level (an agent that already has a parent cannot take children); the agent being moved must not already have children of its own; and both agents must still exist."
            ),
        }
    }
}

fn name_of(manager: &Arc<AgentManager>, id: AgentId) -> String {
    manager
        .roster()
        .into_iter()
        .find(|e| e.id == id)
        .map(|e| e.canonical_name)
        .unwrap_or_default()
}

fn resolve_or_reject(
    manager: &Arc<AgentManager>,
    token: &str,
) -> Result<AgentId, ControlQueryResult> {
    match resolve_target(&manager.roster(), token) {
        TargetResolution::Found(id) => Ok(id),
        TargetResolution::NotFound => Err(not_found(token)),
        TargetResolution::Ambiguous => Err(ControlQueryResult::Error {
            code: "AGENT_AMBIGUOUS",
            hint: format!(
                "more than one agent is called '{token}', so this command would have to guess — use the agent id instead (`{CLI_EXE_NAME} {CLI_GROUP_AGENT} list` shows them)."
            ),
        }),
    }
}

fn not_found(token: &str) -> ControlQueryResult {
    ControlQueryResult::Error {
        code: "AGENT_NOT_FOUND",
        hint: format!(
            "no agent called '{token}' — names are matched exactly, with no case-folding, prefixes or trimming (`{CLI_EXE_NAME} {CLI_GROUP_AGENT} list` shows the roster)."
        ),
    }
}

fn notify(broadcast: Option<&Arc<dyn RosterBroadcast>>) {
    match broadcast {
        Some(b) => b.roster_changed(),
        None => tracing::debug!(
            "명부 변경 통지 생략 — 팬아웃 포트 미설정(클라이언트가 붙을 수 없는 조립)"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: AgentId, name: &str) -> RosterEntry {
        RosterEntry {
            id,
            canonical_name: name.to_string(),
            live: None,
            cwd: String::new(),
            parent: None,
        }
    }

    fn parse(body: serde_json::Value) -> AgentRequest {
        serde_json::from_value(body).expect("요청 역직렬화")
    }

    #[test]
    fn resolve_matches_names_exactly_and_never_by_prefix_or_case() {
        let a = AgentId::new_v4();
        let roster = vec![entry(a, "alpha")];
        assert_eq!(resolve_target(&roster, "alpha"), TargetResolution::Found(a));
        for miss in ["alph", "alpha2", "ALPHA", "Alpha", " alpha", "alpha "] {
            assert_eq!(
                resolve_target(&roster, miss),
                TargetResolution::NotFound,
                "정확 일치만: {miss}"
            );
        }
    }

    #[test]
    fn an_id_looking_name_cannot_hijack_id_targeting() {
        let real = AgentId::new_v4();
        let impostor = AgentId::new_v4();
        // 남의 id 를 **이름으로** 달고 있는 에이전트가 먼저 나와도 id 지목은 진짜에게 간다.
        let roster = vec![entry(impostor, &real.to_string()), entry(real, "real")];
        assert_eq!(
            resolve_target(&roster, &real.to_string()),
            TargetResolution::Found(real)
        );
        assert_eq!(
            resolve_target(&roster, "real"),
            TargetResolution::Found(real)
        );
    }

    #[test]
    fn duplicate_names_refuse_instead_of_picking_one() {
        let a = AgentId::new_v4();
        let b = AgentId::new_v4();
        let roster = vec![entry(a, "twin"), entry(b, "twin")];
        assert_eq!(resolve_target(&roster, "twin"), TargetResolution::Ambiguous);
        // 모호해도 id 지목은 여전히 통한다(그게 탈출구다).
        assert_eq!(
            resolve_target(&roster, &b.to_string()),
            TargetResolution::Found(b)
        );
    }

    #[test]
    fn an_empty_roster_finds_nothing() {
        assert_eq!(resolve_target(&[], "anyone"), TargetResolution::NotFound);
    }

    // ── 상태 어휘 ────────────────────────────────────────────────────────────────

    /// ★변경 동사와 `list` 가 같은 함수를 지나야 두 답이 갈릴 수 없다★ — 전 상태를 여기서 못박는다.
    #[test]
    fn every_status_maps_to_one_of_the_two_agent_state_words() {
        assert_eq!(state_of(&AgentStatus::Running), AGENT_STATE_LIVE);
        assert_eq!(state_of(&AgentStatus::Exiting), AGENT_STATE_LIVE);
        for dead in [
            AgentStatus::Exited { code: Some(0) },
            AgentStatus::Exited { code: None },
            AgentStatus::Killed,
            AgentStatus::Failed {
                message: "boom".to_string(),
            },
        ] {
            assert_eq!(
                state_of(&dead),
                AGENT_STATE_SLEEPING,
                "시체는 산 것이 아니다: {dead:?}"
            );
        }
        // 메시지 결말 어휘와 섞이지 않는다(CLAUDE.md 고정 용어 — 두 축을 섞어 결정이 꼬인 적이 있다).
        for message_word in ["delivered", "pending", "failed"] {
            assert_ne!(AGENT_STATE_LIVE, message_word);
            assert_ne!(AGENT_STATE_SLEEPING, message_word);
        }
    }

    // ── 입력 규율 ────────────────────────────────────────────────────────────────

    #[test]
    fn parent_distinguishes_absent_from_explicit_null() {
        let absent = parse(serde_json::json!({ "verb": "move", "target": "a" }));
        assert_eq!(absent.parent, None, "필드 부재");
        let detach = parse(serde_json::json!({ "verb": "move", "target": "a", "parent": null }));
        assert_eq!(detach.parent, Some(None), "명시 null = 루트로 떼기");
        let under = parse(serde_json::json!({ "verb": "move", "target": "a", "parent": "lead" }));
        assert_eq!(under.parent, Some(Some("lead".to_string())));
    }

    #[test]
    fn unknown_fields_are_collected_and_refused() {
        let req = parse(serde_json::json!({ "verb": "move", "target": "a", "parnet": "lead" }));
        assert!(req.extra.contains_key("parnet"), "모르는 키를 모은다");
        let rejection = reject_unknown_fields(&req).expect("반려");
        match rejection {
            ControlQueryResult::Error { code, hint } => {
                assert_eq!(code, "INVALID_AGENT_ARGS");
                assert!(hint.contains("parnet"), "어느 키인지 지목해야: {hint}");
            }
            other => panic!("반려여야: {other:?}"),
        }
    }

    /// 명시 null 은 **모르는 필드가 아니다** — 아는 필드를 null 로 준 요청까지 "오타" 로 반려하면 정상
    /// 호출이 막힌다. 대신 그 필드는 **실린 것**으로 세어져 동사별 허용 검사를 받는다(아래 테스트).
    #[test]
    fn an_explicit_null_on_a_known_field_is_not_an_unknown_field() {
        let req = parse(serde_json::json!({
            "verb": "spawn", "cwd": "C:/x", "name": null
        }));
        assert!(req.extra.is_empty(), "아는 필드는 extra 로 새지 않는다");
        assert!(reject_unknown_fields(&req).is_none());
        assert_eq!(value(&req.name), None, "null 은 값이 없는 것이다");
    }

    /// ★null 로 보낸 필드도 "보냈다" 로 센다★: 접어 버리면 `{"verb":"new","target":null}` 이 검사를 통과해
    ///   에이전트를 만들고, 호출자는 자기가 보낸 `target` 이 아무 일도 못 했다는 것을 못 본다.
    #[test]
    fn a_field_sent_as_null_still_counts_as_sent_for_the_per_verb_guard() {
        let req = parse(serde_json::json!({ "verb": "new", "cwd": "C:/x", "target": null }));
        assert_eq!(present_fields(&req), vec![FIELD_TARGET, FIELD_CWD]);
        let out = guard(&req, "new", &[FIELD_CWD, FIELD_NAME], || {
            panic!("허용 밖 필드가 실렸으면 본문이 돌면 안 된다")
        });
        match out {
            ControlQueryResult::Error { code, hint } => {
                assert_eq!(code, "INVALID_AGENT_ARGS");
                assert!(hint.contains(FIELD_TARGET), "{hint}");
            }
            other => panic!("반려여야: {other:?}"),
        }
    }

    // ── 상태 정직성(라운드 1 HIGH 의 회귀망) ──────────────────────────────────────────

    /// ★이 테스트가 막는 것 = 응답에 상태를 **박는** 것★. terminal 상태의 에이전트를 넣으므로 리터럴
    ///   `"live"` 로는 절대 통과할 수 없다 — 라우트 레벨에서는 그 자리에 결정적으로 죽은 에이전트를 만들
    ///   방법이 없어(셸은 계속 살아 있다) 이 축을 여기서 못박는다.
    #[test]
    fn the_spawn_payload_derives_state_and_cannot_be_a_literal() {
        let id = AgentId::new_v4();
        let live = spawn_payload(id, "worker", &AgentStatus::Running, false);
        assert_eq!(live["agent"]["state"], AGENT_STATE_LIVE);
        assert_eq!(live["created"], false);

        for dead in [
            AgentStatus::Exited { code: Some(1) },
            AgentStatus::Killed,
            AgentStatus::Failed {
                message: "resume died".to_string(),
            },
        ] {
            let payload = spawn_payload(id, "worker", &dead, true);
            assert_eq!(
                payload["agent"]["state"], AGENT_STATE_SLEEPING,
                "깨우자마자 죽은 에이전트를 살아 있다고 보고하면 호출자가 시체에게 편지를 쓴다: {dead:?}"
            );
            assert_ne!(
                payload["agent"]["state"], AGENT_STATE_LIVE,
                "상태를 박아 두면 여기서 죽는다: {dead:?}"
            );
            assert_eq!(payload["created"], true);
        }
    }

    #[test]
    fn blank_values_are_refused_not_folded_into_absent() {
        for body in [
            serde_json::json!({ "verb": "move", "target": "a", "parent": "" }),
            serde_json::json!({ "verb": "move", "target": "a", "parent": "   " }),
            serde_json::json!({ "verb": "rename", "target": "", "name": "x" }),
            serde_json::json!({ "verb": "new", "cwd": " " }),
            serde_json::json!({ "verb": "new", "cwd": "C:/x", "name": "" }),
        ] {
            let req = parse(body.clone());
            assert!(
                reject_blank_fields(&req).is_some(),
                "공백 값은 반려여야: {body}"
            );
        }
        let ok = parse(serde_json::json!({ "verb": "move", "target": "a", "parent": null }));
        assert!(
            reject_blank_fields(&ok).is_none(),
            "명시 null 은 공백이 아니다"
        );
    }

    #[test]
    fn fields_a_verb_does_not_use_are_refused() {
        let req = parse(serde_json::json!({ "verb": "list", "target": "a" }));
        let out = guard(&req, "list", &[], || {
            panic!("허용 밖 필드가 있으면 본문이 돌면 안 된다")
        });
        match out {
            ControlQueryResult::Error { code, hint } => {
                assert_eq!(code, "INVALID_AGENT_ARGS");
                assert!(hint.contains(FIELD_TARGET), "{hint}");
            }
            other => panic!("반려여야: {other:?}"),
        }
        let clean = parse(serde_json::json!({ "verb": "list" }));
        assert!(matches!(
            guard(&clean, "list", &[], || ControlQueryResult::Ok(
                serde_json::json!({})
            )),
            ControlQueryResult::Ok(_)
        ));
    }
}
