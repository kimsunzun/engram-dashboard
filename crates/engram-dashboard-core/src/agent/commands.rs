//! `agent.*` 명령 — **선언과 본문이 한 세트**로 일하는 코드(`AgentManager`) 옆에 산다(ADR-0134).
//!
//! ★배선 0★: 지금 이 표로 배달하는 곳은 아무 데도 없다. 에이전트 제어의 실제 입구는 여전히 데몬의
//!   `/control/agent` 동사 match 이고, 그 자리를 이 표 조회로 바꾸는 것이 S20 Step 2 다. 그래서 이
//!   모듈이 바뀌어도 지금 동작은 한 톨도 변하지 않는다.
//! ★호스팅은 데몬, 선언은 코어★ — 이 둘이 갈리는 유일한 자리다(ADR-0029 + TRD §2-3).
//!
//! 진입점: [`make_table`](fn.make_table.html)(조립) · [`AgentCommandHost`](trait.AgentCommandHost.html)(주입 seam).

use std::path::PathBuf;
use std::sync::Arc;

use engram_dashboard_command::{
    blocking_handler, declare_commands, CommandError, CommandTable, ErrorCode,
};

use crate::agent::manager::{AgentManager, RenameOutcome};
use crate::agent::profile::{AgentCommand, AgentProfile, ClaudeOutputFormat, SpawnMode};
use crate::agent::types::{
    AgentId, AgentStatus, PtyError, AGENT_STATE_LIVE, AGENT_STATE_SLEEPING, RENAME_OUTCOME_RENAMED,
    RENAME_OUTCOME_UNCHANGED,
};

// ★성공 응답은 평평하다(사용자 결정 2026-08-13)★: 현행 데몬 응답의 `{"agent":{…}}` 한 겹은 옛 입구의
//   흔적이고, 새 표는 명령마다 반환을 선언하므로 감쌀 이유가 없다.
// TODO(S20 Step 2 — ADR-0134): CLI 응답 파싱을 이 모양에 맞춘다 — `daemon/src/bin/engram.rs` 의
//   `v["agent"]["state"]` 계열이 지금은 중첩을 읽는다. 지금 바꾸지 않는 것은 배선 0 때문이다.
declare_commands! {
    catalog_version: 1;

    /// 명부의 한 행.
    struct AgentRow {
        id: String,
        name: String,
        state: String,
        cwd: String,
        parent: Option<String>,
    }

    // errors 에는 **이 명령 고유의** 코드만 적는다 — 인자 반려(INVALID_ARGUMENT)와 내부 실패(INTERNAL)는
    //   표가 내는 것이라 `CommandSpec::advertised_errors` 가 자동으로 얹는다.

    /// 명부 전량 — 이름·생사·작업 폴더·부모.
    #[effect(Read)]
    #[since(1)]
    "agent.list" => args AgentListArgs {}
                 -> ok   AgentListOk { agents: Vec<AgentRow> }
                 errors [];

    /// 에이전트를 띄운다(잠든 것 깨우기 포함).
    #[effect(Write)]
    #[since(1)]
    "agent.spawn" => args AgentSpawnArgs {
        /// 깨울 대상(이름 또는 id). cwd 와 상호배타.
        target: Option<String>,
        /// 새로 만들어 띄울 작업 폴더. target 과 상호배타.
        cwd: Option<String>,
        /// 새로 만들 때만 쓴다 — 깨우기에는 적용되지 않는다.
        name: Option<String>,
    } -> ok AgentSpawnOk {
        agent_id: String,
        name: String,
        state: String,
        created: bool,
    } errors [NOT_FOUND, CONFLICT];

    /// 새 에이전트를 명부에 등록한다(띄우지는 않는다 — 잠든 상태로 남는다).
    #[effect(Write)]
    #[since(1)]
    "agent.new" => args AgentNewArgs {
        cwd: String,
        name: Option<String>,
    } -> ok AgentNewOk {
        agent_id: String,
        name: String,
        state: String,
    } errors [CONFLICT];

    /// 표시 이름을 바꾼다.
    #[effect(Write)]
    #[since(1)]
    "agent.rename" => args AgentRenameArgs {
        target: String,
        name: String,
    } -> ok AgentRenameOk {
        agent_id: String,
        /// 확정된 이름 — 요청한 이름과 다를 수 있다(명부 유일성 접미사).
        name: String,
        outcome: String,
    } errors [NOT_FOUND, CONFLICT];

    /// 트리에서 부모를 바꾼다.
    #[effect(Write)]
    #[since(1)]
    "agent.move" => args AgentMoveArgs {
        target: String,
        /// 새 부모(이름 또는 id). ★`null` = 최상위로 뗀다 · **필드 부재는 반려**★ — 오타 필드 하나가
        /// 조용히 계층 해제로 실행되면 안 되므로 부재와 `null` 을 가른다(바깥 Option = 실렸나).
        parent: Option<Option<String>>,
    } -> ok AgentMoveOk {
        agent_id: String,
        name: String,
        parent: Option<String>,
    } errors [NOT_FOUND, CONFLICT];
}

/// 명부 한 행 — 이 표가 매니저에게서 보는 것만.
///
/// `live` = `Some(status)` 이면 세션이 붙어 있다(생사 판정은 [`AgentStatus::is_live`] 하나, ADR-0116).
pub struct AgentRosterRow {
    pub id: AgentId,
    pub canonical_name: String,
    pub cwd: String,
    pub parent: Option<AgentId>,
    pub live: Option<AgentStatus>,
}

/// 띄우기 결과 — 이 표가 보는 세 칸.
pub struct StartedAgent {
    pub id: AgentId,
    pub name: String,
    pub status: AgentStatus,
}

/// `agent.*` 핸들러가 매니저에게 시키는 일 **전부**.
///
/// ★왜 trait 인가★: `AgentManager` 를 그대로 받으면 핸들러 하나를 단언하려고 실제 PTY 프로세스가
///   딸려온다 — ADR-0012 「외부 의존을 seam 으로 끊는다」의 직접 위반이고, 규칙 T-1 이 금지하는 형태다.
///   이 seam 이 있어야 가짜 매니저로 표 전체가 프로세스 없이 검증된다.
/// ★구현체는 번역만 한다★: 동사 규칙(지목 해석 · 모드 유도 · 결말 번역)은 **핸들러가** 갖는다.
///   구현체에 규칙이 들어가면 진짜와 가짜가 서로 다른 규칙을 돌게 되어 하네스가 아무것도 못 지킨다.
// ADR-0134
// ADR-0012
pub trait AgentCommandHost: Send + Sync {
    fn roster(&self) -> Vec<AgentRosterRow>;
    fn agent_snapshot(&self, id: AgentId) -> Option<AgentProfile>;
    fn create_agent(&self, profile: AgentProfile) -> Result<AgentProfile, PtyError>;
    fn activate_profile(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<StartedAgent, PtyError>;
    fn rename_agent(&self, id: AgentId, display_name: Option<String>) -> RenameOutcome;
    /// ★`false` 는 사유를 말해 주지 않는다★ — 매니저가 bool 하나만 준다(`ProfileRegistry::reparent`).
    /// 부재와 구조 충돌은 핸들러가 **명부 재조회로** 가른다(`verb_move`).
    fn reparent_agent(&self, child: AgentId, parent: Option<AgentId>) -> bool;
}

/// 명부가 바뀌었음을 붙어 있는 클라이언트에게 알리는 출구(포트).
///
/// ★이게 빠지면 나는 증상★: 에이전트를 만들거나 이름을 바꿔도 대시보드 트리가 **무관한 이벤트가 올
///   때까지 옛 명부를 보여 준다**(조용한 stale). 데몬 `/control/agent` 는 변경 4동사마다 같은 통지를
///   부르고 있고, 그 자리를 이 표가 넘겨받을 때 통지가 함께 오지 않으면 그 증상이 재발한다.
/// ★왜 표의 의존이 아니라 포트인가★: 실제 통지는 전-연결 팬아웃이라 데몬 소유다. 소비자인 여기가 좁은
///   trait 만 갖고 실물 어댑터는 조립부가 준다(daemon `control::RosterBroadcast` 와 같은 모양).
/// 논블록이어야 한다 — 팬아웃은 연결별 큐에 try_send 만 한다.
// ADR-0134
pub trait RosterChanged: Send + Sync {
    fn roster_changed(&self);
}

impl AgentCommandHost for AgentManager {
    fn roster(&self) -> Vec<AgentRosterRow> {
        AgentManager::roster(self)
            .into_iter()
            .map(|entry| AgentRosterRow {
                id: entry.id,
                canonical_name: entry.canonical_name,
                cwd: entry.cwd,
                parent: entry.parent,
                live: entry.live.map(|info| info.status),
            })
            .collect()
    }

    fn agent_snapshot(&self, id: AgentId) -> Option<AgentProfile> {
        AgentManager::agent_snapshot(self, id)
    }

    fn create_agent(&self, profile: AgentProfile) -> Result<AgentProfile, PtyError> {
        AgentManager::create_agent(self, profile)
    }

    fn activate_profile(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<StartedAgent, PtyError> {
        AgentManager::activate_profile(self, profile, mode).map(|info| StartedAgent {
            id: info.id,
            name: info.name,
            status: info.status,
        })
    }

    fn rename_agent(&self, id: AgentId, display_name: Option<String>) -> RenameOutcome {
        AgentManager::rename_agent(self, id, display_name)
    }

    fn reparent_agent(&self, child: AgentId, parent: Option<AgentId>) -> bool {
        AgentManager::reparent_agent(self, child, parent)
    }
}

/// 새로 만드는 에이전트의 백엔드 출력 형식.
///
/// ★왜 StreamJson 인가★: 이 계열을 부르는 주체가 LLM 이고, 프론트에서 LLM 이 부르는 생성 커맨드
///   (`agentlist.createAgent`)의 기본값이 이미 StreamJson 이다(ADR-0078). 두 입구가 같은 동사에 다른
///   기본을 주면 "만들었는데 화면이 다르다" 가 된다.
/// ★값의 집은 여기 하나다★ — 데몬 `control/agent.rs` 가 이것을 참조한다(자기 상수를 두지 않는다).
pub const NEW_AGENT_OUTPUT_FORMAT: ClaudeOutputFormat = ClaudeOutputFormat::StreamJson;

/// `agent.*` 표를 조립한다 — ★핸들러 실물이 들어오는 유일한 자리★(규칙 T-1).
///
/// ★blocking 계약★: 핸들러 본문은 프로필 락을 쥔 채 디스크를 쓰고 resume 조기 종료를 폴링한다
///   (`AgentManager::activate_profile`). async 런타임 스레드에서 폴링하면 그 스레드를 막으므로 조립부가
///   `spawn_blocking` 뒤에서 불러야 한다 — 데몬 `/control/agent` 어댑터가 이미 그렇게 한다.
/// ★`notify` 를 인자로 받는 이유★: 명부를 바꾼 동사는 반드시 통지해야 하는데(포트 doc 참조) 그 실물은
///   데몬 소유다. 조립부가 넘기게 하면 **빠뜨릴 수 없다** — trait 기본 구현으로 두면 조용히 안 부른다.
/// ★명령이 늘어도 조립부(실행 파일)는 안 바뀐다★ — 늘어나는 것은 선언 블록과 이 함수의 한 줄이다.
// ADR-0134
pub fn make_table(host: Arc<dyn AgentCommandHost>, notify: Arc<dyn RosterChanged>) -> CommandTable {
    let mut table = CommandTable::new(COMMAND_SPECS);

    let list = Arc::clone(&host);
    let spawn = (Arc::clone(&host), Arc::clone(&notify));
    let new = (Arc::clone(&host), Arc::clone(&notify));
    let rename = (Arc::clone(&host), Arc::clone(&notify));
    let move_ = (Arc::clone(&host), Arc::clone(&notify));

    // ★조립 때 터뜨린다★: insert 가 반려하는 셋(선언 집합에 없는 이름 · 중복 삽입 · 선언 스키마 텍스트가
    //   JSON 이 아님) 전부 **빌드가 정하는 값**이라 런타임에 달라지지 않는다. 어느 것인지는 패닉에 함께
    //   실리는 `TableError` 가 말하므로 아래 메시지는 사유를 단정하지 않는다.
    table
        .insert(
            "agent.list",
            blocking_handler(move |_: AgentListArgs| verb_list(list.as_ref())),
        )
        .expect("agent.list 를 표에 꽂지 못했다");
    table
        .insert(
            "agent.spawn",
            blocking_handler(move |args: AgentSpawnArgs| {
                verb_spawn(spawn.0.as_ref(), spawn.1.as_ref(), args)
            }),
        )
        .expect("agent.spawn 을 표에 꽂지 못했다");
    table
        .insert(
            "agent.new",
            blocking_handler(move |args: AgentNewArgs| {
                verb_new(new.0.as_ref(), new.1.as_ref(), args)
            }),
        )
        .expect("agent.new 를 표에 꽂지 못했다");
    table
        .insert(
            "agent.rename",
            blocking_handler(move |args: AgentRenameArgs| {
                verb_rename(rename.0.as_ref(), rename.1.as_ref(), args)
            }),
        )
        .expect("agent.rename 을 표에 꽂지 못했다");
    table
        .insert(
            "agent.move",
            blocking_handler(move |args: AgentMoveArgs| {
                verb_move(move_.0.as_ref(), move_.1.as_ref(), args)
            }),
        )
        .expect("agent.move 를 표에 꽂지 못했다");

    table
}

fn verb_list(host: &dyn AgentCommandHost) -> Result<AgentListOk, CommandError> {
    // ★한 번의 명부 조회로 전부 만든다★: 이름·생사·cwd·계층이 같은 스냅샷에서 나와야 한 응답 안의
    //   행끼리 정합한다.
    let agents = host
        .roster()
        .into_iter()
        .map(|row| AgentRow {
            id: row.id.to_string(),
            name: row.canonical_name,
            state: state_of(row.live.as_ref()).to_string(),
            cwd: row.cwd,
            parent: row.parent.map(|p| p.to_string()),
        })
        .collect();
    Ok(AgentListOk { agents })
}

fn verb_spawn(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    args: AgentSpawnArgs,
) -> Result<AgentSpawnOk, CommandError> {
    let target = value("target", &args.target)?;
    let cwd = value("cwd", &args.cwd)?;
    let name = value("name", &args.name)?;

    // ★두 동사가 한 이름을 쓴다(깨우기 / 만들어서 띄우기)★: 어느 쪽인지는 인자가 가른다. 둘 다 주면
    //   고를 근거가 없고, 조용히 한쪽을 고르면 만들려던 에이전트 대신 남을 깨운다.
    match (target, cwd) {
        (Some(_), Some(_)) => Err(CommandError::invalid_argument(
            "spawn takes either an existing agent (target) or cwd for a new one, not both",
        )),
        (None, None) => Err(CommandError::invalid_argument(
            "spawn needs either an existing agent to wake (target) or cwd to create a new one",
        )),
        // 깨우기는 이름을 바꾸지 않으므로 name 이 아무 일도 못 한다 — 조용히 무시하면 호출자는 이름이
        //   바뀐 줄 안다.
        (Some(token), None) if name.is_some() => Err(CommandError::invalid_argument(format!(
            "name does not apply when waking an existing agent ({token}); use agent.rename"
        ))),
        (Some(token), None) => wake_existing(host, token),
        (None, Some(cwd)) => create_and_start(host, notify, cwd, name.map(str::to_string)),
    }
}

/// ★깨우기는 명부 통지를 겹쳐 보내지 않는다★ — 항목 수·이름·계층이 그대로이고, 생사 전이는 매니저가
/// 이미 흘린다(`spawn_agent` 가 `agent_list_updated` 를 낸다). 데몬 `/control/agent` 도 같다.
fn wake_existing(host: &dyn AgentCommandHost, token: &str) -> Result<AgentSpawnOk, CommandError> {
    let id = resolve(host, token)?.id;
    let Some(profile) = host.agent_snapshot(id) else {
        return Err(not_found(token));
    };
    // ★모드 유도 규칙은 WS 경로와 같은 것을 쓴다(ADR-0076)★: 저장된 세션이 있으면 이어받기, 없으면
    //   새로. 여기서 다른 규칙을 쓰면 같은 에이전트가 어느 입구로 깨우느냐에 따라 대화 이력을 잃는다.
    let mode = if profile.claude_session_id.is_some() {
        SpawnMode::Resume
    } else {
        SpawnMode::Fresh
    };
    let started = host
        .activate_profile(&profile, mode)
        .map_err(|e| CommandError::internal(format!("could not start agent '{token}': {e}")))?;
    Ok(started_payload(started, false))
}

fn create_and_start(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    cwd: &str,
    name: Option<String>,
) -> Result<AgentSpawnOk, CommandError> {
    let stored = register(host, cwd, name)?;
    notify.roster_changed();
    let started = host
        .activate_profile(&stored, SpawnMode::Fresh)
        .map_err(|e| {
            // ★등록을 되돌리지 않는다★: 만들어진 에이전트는 잠든 상태로 명부에 남는다. 되감기는 두 번째
            //   삭제 경로를 만드는데 삭제의 semantics 자체가 미결이다(ADR-0122).
            CommandError::internal(format!(
                "agent '{}' ({}) was created but did not start: {e} — it is registered and asleep",
                stored.canonical_name_when_live(),
                stored.id
            ))
        })?;
    Ok(started_payload(started, true))
}

fn verb_new(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    args: AgentNewArgs,
) -> Result<AgentNewOk, CommandError> {
    let cwd = required("cwd", &args.cwd)?;
    let name = value("name", &args.name)?;
    let stored = register(host, cwd, name.map(str::to_string))?;
    notify.roster_changed();
    Ok(AgentNewOk {
        agent_id: stored.id.to_string(),
        name: stored.canonical_name_when_live(),
        state: AGENT_STATE_SLEEPING.to_string(),
    })
}

/// 명부 등록 공통부 — `agent.new` 와 `agent.spawn --cwd` 가 같은 자리를 쓴다.
///
/// ★확정 이름은 요청 이름과 다를 수 있다★: 명부 전역 유일성 때문에 접미사가 붙는다(ADR-0120/0123).
/// 그래서 응답은 **등록된 값**에서 이름을 읽는다 — 요청 이름을 되돌려주면 화면·주소와 어긋난다.
fn register(
    host: &dyn AgentCommandHost,
    cwd: &str,
    name: Option<String>,
) -> Result<AgentProfile, CommandError> {
    let mut profile = AgentProfile::new(
        cwd.to_string(),
        AgentCommand::Claude {
            extra_args: vec![],
            output_format: NEW_AGENT_OUTPUT_FORMAT,
        },
        PathBuf::from(cwd),
        vec![],
        false,
    );
    profile.display_name = name;
    // ★실패 사유마다 호출자가 할 일이 다르다★ — 자리를 비워라(CONFLICT) · 인자를 고쳐라
    //   (INVALID_ARGUMENT) · 다시 시도해도 소용없다(INTERNAL). 한 코드로 뭉개면 그 갈림이 사라진다.
    host.create_agent(profile).map_err(|e| match e {
        PtyError::RosterFull { current, limit } => CommandError::of(
            ErrorCode::Conflict,
            format!(
                "the team already has {current} agents, which is the safety ceiling ({limit}) that stops a runaway create loop — remove agents you no longer need, then try again"
            ),
        ),
        PtyError::CwdDenied => CommandError::invalid_argument(format!(
            "cwd is outside the allowed workspace: {cwd}"
        )),
        PtyError::Unsupported(reason) => CommandError::of(
            ErrorCode::Conflict,
            format!("could not register a new agent: {reason}"),
        ),
        other => CommandError::internal(format!("could not register a new agent: {other}")),
    })
}

fn verb_rename(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    args: AgentRenameArgs,
) -> Result<AgentRenameOk, CommandError> {
    let token = required("target", &args.target)?;
    let name = required("name", &args.name)?;
    let id = resolve(host, token)?.id;
    // ★네 결말을 뭉개지 않는다★: 확정된 이름 · 이미 그 계열을 쥐어 무변경 · 부재 · 이름 공간 소진.
    //   앞 둘은 성공이지만 다른 사실이라 outcome 으로 갈라 싣는다.
    match host.rename_agent(id, Some(name.to_string())) {
        RenameOutcome::Renamed(committed) => {
            notify.roster_changed();
            Ok(AgentRenameOk {
                agent_id: id.to_string(),
                name: committed,
                outcome: RENAME_OUTCOME_RENAMED.to_string(),
            })
        }
        RenameOutcome::Unchanged(kept) => {
            notify.roster_changed();
            Ok(AgentRenameOk {
                agent_id: id.to_string(),
                name: kept,
                outcome: RENAME_OUTCOME_UNCHANGED.to_string(),
            })
        }
        RenameOutcome::NotFound => Err(not_found(token)),
        RenameOutcome::Exhausted => Err(CommandError::of(
            ErrorCode::Conflict,
            format!("every numbered variant of that name is taken, so '{token}' was left as it is"),
        )),
    }
}

fn verb_move(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    args: AgentMoveArgs,
) -> Result<AgentMoveOk, CommandError> {
    let token = required("target", &args.target)?;
    // ★부재는 「부모를 안 줬으니 떼자」가 아니다★ — 루트로 떼는 지시는 `null` 이다. 부재를 접으면 오타
    //   필드 하나가 계층 해제로 실행된다.
    // ★wire 로 들어온 부재는 여기까지 못 온다★ — 선언 매크로가 이 칸에 `#[serde(default)]` 를 안 달아
    //   역직렬화가 `missing field` 로 반려한다. 이 갈래는 코드가 인자를 직접 지어 부르는 경로 몫이다.
    let Some(parent_token) = args.parent.as_ref() else {
        return Err(CommandError::invalid_argument(
            "move needs parent: a name/id to move under, or null to move it back to the top level",
        ));
    };
    let parent_token = value("parent", parent_token)?;
    let child = resolve(host, token)?;
    let parent = match parent_token {
        None => None,
        Some(p) => Some(resolve(host, p)?.id),
    };
    if !host.reparent_agent(child.id, parent) {
        // ★`false` 하나로는 사유를 모른다★: 방금 해석한 대상이 그 사이 사라졌을 수도 있고(NOT_FOUND),
        //   트리 구조가 거부했을 수도 있다(CONFLICT). 둘은 호출자가 할 일이 다르므로 명부를 다시 보고
        //   가른다 — 사유 목록은 `ProfileRegistry::reparent` 의 거부 조건과 한 줄씩 대응한다.
        // ★사라진 쪽의 이름을 댄다★: 둘 중 어느 쪽이 없어졌든 자식 이름을 대면, 부모만 사라진 경우에
        //   호출자는 **멀쩡히 있는** 에이전트를 없다고 듣고 엉뚱한 데를 뒤진다.
        let roster = host.roster();
        let gone = |id: AgentId| !roster.iter().any(|row| row.id == id);
        if gone(child.id) {
            return Err(not_found(token));
        }
        // parent 가 Some 이면 parent_token 도 Some 이다(위 match 가 그 둘을 함께 만든다).
        if let Some((parent_token, parent_id)) = parent_token.zip(parent) {
            if gone(parent_id) {
                return Err(not_found(parent_token));
            }
        }
        return Err(CommandError::of(
            ErrorCode::Conflict,
            format!(
                "'{token}' cannot go there. The tree is one level deep, so: an agent cannot be its own parent; the new parent must itself be top-level; and the agent being moved must not already have children."
            ),
        ));
    }
    notify.roster_changed();
    Ok(AgentMoveOk {
        agent_id: child.id.to_string(),
        // ★이름은 해석 시점의 명부에서 온다★ — 여기서 다시 조회해 빈 문자열로 접으면(그 사이 사라진
        //   경우) 성공 응답에 이름 없는 행이 실린다.
        name: child.name,
        parent: parent.map(|p| p.to_string()),
    })
}

fn started_payload(started: StartedAgent, created: bool) -> AgentSpawnOk {
    AgentSpawnOk {
        agent_id: started.id.to_string(),
        name: started.name,
        state: state_of(Some(&started.status)).to_string(),
        created,
    }
}

/// 상태 → wire 어휘의 **단일 매핑**. 술어는 [`AgentStatus::is_live`] 하나다(ADR-0116).
///
/// ★리터럴을 박지 않는 이유(실제로 갈렸던 자리)★: 변경 동사가 `"live"` 를 박으면 깨우자마자 죽은
/// 에이전트(ADR-0082 resume 조기 종료)를 `spawn` 은 살아 있다고, 직후의 `list` 는 잠들었다고 보고한다 —
/// 그러면 호출자는 시체에게 편지를 쓴다.
fn state_of(status: Option<&AgentStatus>) -> &'static str {
    match status {
        Some(s) if s.is_live() => AGENT_STATE_LIVE,
        _ => AGENT_STATE_SLEEPING,
    }
}

/// 지목 토큰 → 에이전트 하나.
///
/// ★규칙(우편 입구와 **같아야** 한다)★: ① agent id 문자열 정확 일치를 먼저 본다 — UUID 처럼 생긴
/// *이름* 이 id 지목을 가로채지 못하게 ② 그다음 이름 **정확** 일치(대소문자 구분 · 접두 매칭 없음 ·
/// 공백 보정 없음) ③ 같은 이름이 둘 이상이면 아무도 고르지 않고 거부한다.
/// ★관대한 해석기를 들이지 말 것★: 사람과 LLM 은 `mail send --to X` 와 `agent.rename X` 가 같은 X 를
/// 가리킨다고 읽는다. 두 입구의 해석이 갈리면 편지를 받은 에이전트와 이름이 바뀐 에이전트가 달라진다.
// ADR-0132
// TODO(S20 Step 2 — ADR-0134): 데몬 `control/agent.rs::resolve_target` 이 같은 규칙의 사본이다. 그 파일의
//   동사 match 가 이 표 조회로 바뀔 때 사본을 지운다(지금 옮기면 배선 0 이 깨진다).
fn resolve(host: &dyn AgentCommandHost, token: &str) -> Result<ResolvedAgent, CommandError> {
    let roster = host.roster();
    let found = |row: &AgentRosterRow| ResolvedAgent {
        id: row.id,
        name: row.canonical_name.clone(),
    };
    if let Some(row) = roster.iter().find(|r| r.id.to_string() == token) {
        return Ok(found(row));
    }
    let mut by_name = roster.iter().filter(|r| r.canonical_name == token);
    match (by_name.next(), by_name.next()) {
        (Some(row), None) => Ok(found(row)),
        (Some(_), Some(_)) => Err(CommandError::of(
            ErrorCode::Conflict,
            format!(
                "more than one agent is called '{token}', so this command would have to guess — use the agent id instead"
            ),
        )),
        _ => Err(not_found(token)),
    }
}

/// 지목이 가리킨 에이전트 — id 와 **그때 명부가 말한 이름**.
struct ResolvedAgent {
    id: AgentId,
    name: String,
}

fn not_found(token: &str) -> CommandError {
    CommandError::not_found(format!(
        "no agent called '{token}' — names are matched exactly, with no case-folding, prefixes or trimming"
    ))
}

/// 공백만 있는 값은 **부재로 접지 않고** 반려한다.
///
/// ★셸에서 미설정 변수가 빈 인자로 펼쳐지는 형태(`--parent "$UNSET"`)가 현실적으로 들어온다★ — 그걸
/// "안 준 것" 으로 접으면 오타 한 번이 다른 동작(계층 해제 등)으로 조용히 바뀐다.
fn value<'a>(field: &str, given: &'a Option<String>) -> Result<Option<&'a str>, CommandError> {
    match given.as_deref() {
        Some(v) if v.trim().is_empty() => Err(CommandError::invalid_argument(format!(
            "blank value for '{field}' — an empty argument is usually an unset variable; pass a value or drop the field"
        ))),
        other => Ok(other),
    }
}

fn required<'a>(field: &str, given: &'a str) -> Result<&'a str, CommandError> {
    if given.trim().is_empty() {
        return Err(CommandError::invalid_argument(format!(
            "blank value for '{field}' — an empty argument is usually an unset variable; pass a value"
        )));
    }
    Ok(given)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use engram_dashboard_command::testing::block_on;
    use serde_json::json;

    use super::*;

    /// 가짜 매니저 — ★프로세스도 PTY 도 없다★. 이 하네스가 서지 않으면 T-1 이 깨진 것이다.
    #[derive(Default)]
    struct FakeHost {
        rows: Mutex<Vec<AgentRosterRow>>,
        profiles: Mutex<HashMap<AgentId, AgentProfile>>,
        started: Mutex<Vec<(AgentId, bool)>>,
        rename_result: Mutex<Option<RenameOutcome>>,
        reparent_ok: Mutex<bool>,
        start_fails: Mutex<bool>,
        create_fails: Mutex<Option<PtyError>>,
        /// reparent 실패 직후 명부에서 지울 대상 — 「그 사이 사라졌다」를 재현한다(자식이든 부모든).
        vanish_on_reparent: Mutex<Option<AgentId>>,
    }

    /// 명부 통지 계수기 — 「이름을 바꿨는데 트리가 옛 명부를 보여준다」의 감시자.
    #[derive(Default)]
    struct FakeNotify {
        calls: Mutex<usize>,
    }

    impl RosterChanged for FakeNotify {
        fn roster_changed(&self) {
            *self.calls.lock().expect("notify poisoned") += 1;
        }
    }

    fn wiring(host: &Arc<FakeHost>) -> (CommandTable, Arc<FakeNotify>) {
        let notify = Arc::new(FakeNotify::default());
        let table = make_table(
            Arc::clone(host) as Arc<dyn AgentCommandHost>,
            Arc::clone(&notify) as Arc<dyn RosterChanged>,
        );
        (table, notify)
    }

    impl FakeHost {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                reparent_ok: Mutex::new(true),
                ..Self::default()
            })
        }

        fn with_agent(self: &Arc<Self>, name: &str, live: bool, resumable: bool) -> AgentId {
            let profile = AgentProfile::new(
                format!("C:/work/{name}"),
                AgentCommand::Claude {
                    extra_args: vec![],
                    output_format: NEW_AGENT_OUTPUT_FORMAT,
                },
                PathBuf::from(format!("C:/work/{name}")),
                vec![],
                false,
            );
            let id = profile.id;
            let mut profile = profile;
            profile.display_name = Some(name.to_string());
            if resumable {
                profile.claude_session_id = Some(uuid::Uuid::new_v4());
            }
            self.profiles.lock().unwrap().insert(id, profile);
            self.rows.lock().unwrap().push(AgentRosterRow {
                id,
                canonical_name: name.to_string(),
                cwd: format!("C:/work/{name}"),
                parent: None,
                live: live.then_some(AgentStatus::Running),
            });
            id
        }
    }

    impl AgentCommandHost for FakeHost {
        fn roster(&self) -> Vec<AgentRosterRow> {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .map(|r| AgentRosterRow {
                    id: r.id,
                    canonical_name: r.canonical_name.clone(),
                    cwd: r.cwd.clone(),
                    parent: r.parent,
                    live: r.live.clone(),
                })
                .collect()
        }

        fn agent_snapshot(&self, id: AgentId) -> Option<AgentProfile> {
            self.profiles.lock().unwrap().get(&id).cloned()
        }

        fn create_agent(&self, profile: AgentProfile) -> Result<AgentProfile, PtyError> {
            if let Some(failure) = self.create_fails.lock().unwrap().take() {
                return Err(failure);
            }
            let id = profile.id;
            let name = profile.canonical_name_when_live();
            self.profiles.lock().unwrap().insert(id, profile.clone());
            self.rows.lock().unwrap().push(AgentRosterRow {
                id,
                canonical_name: name,
                cwd: profile.cwd.to_string_lossy().to_string(),
                parent: None,
                live: None,
            });
            Ok(profile)
        }

        fn activate_profile(
            &self,
            profile: &AgentProfile,
            mode: SpawnMode,
        ) -> Result<StartedAgent, PtyError> {
            if *self.start_fails.lock().unwrap() {
                return Err(PtyError::SpawnFailed("fake".to_string()));
            }
            let resumed = matches!(mode, SpawnMode::Resume);
            self.started.lock().unwrap().push((profile.id, resumed));
            Ok(StartedAgent {
                id: profile.id,
                name: profile.canonical_name_when_live(),
                status: AgentStatus::Running,
            })
        }

        fn rename_agent(&self, _id: AgentId, display_name: Option<String>) -> RenameOutcome {
            self.rename_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| RenameOutcome::Renamed(display_name.unwrap_or_default()))
        }

        fn reparent_agent(&self, _child: AgentId, _parent: Option<AgentId>) -> bool {
            let ok = *self.reparent_ok.lock().unwrap();
            if let (false, Some(vanished)) = (ok, *self.vanish_on_reparent.lock().unwrap()) {
                self.rows.lock().unwrap().retain(|row| row.id != vanished);
            }
            ok
        }
    }

    fn call(
        table: &CommandTable,
        name: &str,
        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, CommandError> {
        // 표를 거쳐 부른다 — 인자를 선언 스키마에 맞추는 자리가 거기 하나뿐이다(`CommandTable::call`).
        let future = table.call(name, &mut args).expect("표에 있는 이름");
        block_on(future)
    }

    #[test]
    fn table_holds_exactly_the_declared_verbs() {
        let (table, _notify) = wiring(&FakeHost::new());
        let names: Vec<&str> = table.specs().map(|s| s.name).collect();
        assert_eq!(
            names,
            vec![
                "agent.list",
                "agent.move",
                "agent.new",
                "agent.rename",
                "agent.spawn"
            ]
        );
        assert_eq!(names.len(), COMMAND_SPECS.len());
    }

    #[test]
    fn decls_carry_a_non_empty_json_shape_for_every_verb() {
        let (table, _notify) = wiring(&FakeHost::new());
        for decl in table.decls() {
            let item: serde_json::Value =
                serde_json::from_str(&decl.help).expect("help 는 JSON 항목 하나");
            assert_eq!(item["name"], decl.name.as_str());
            assert!(item["args"]["type"] == "object");
            assert!(item["ok"]["type"] == "object");
            assert!(item["errors"].as_array().is_some_and(|e| !e.is_empty()));
        }
    }

    #[test]
    fn list_reports_live_and_sleeping_without_a_process() {
        let host = FakeHost::new();
        host.with_agent("alpha", true, false);
        host.with_agent("beta", false, false);
        let (table, _notify) = wiring(&host);

        let out = call(&table, "agent.list", json!({})).expect("조회 성공");
        let agents = out["agents"].as_array().expect("배열");
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0]["name"], "alpha");
        assert_eq!(agents[0]["state"], "live");
        assert_eq!(agents[1]["state"], "sleeping");
        assert_eq!(agents[0]["parent"], serde_json::Value::Null);
    }

    #[test]
    fn spawn_refuses_both_target_and_cwd() {
        let (table, _notify) = wiring(&FakeHost::new());
        let err = call(
            &table,
            "agent.spawn",
            json!({ "target": "alpha", "cwd": "C:/x" }),
        )
        .expect_err("둘 다 주면 반려");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn spawn_refuses_neither_target_nor_cwd() {
        let (table, _notify) = wiring(&FakeHost::new());
        let err = call(&table, "agent.spawn", json!({})).expect_err("아무것도 안 주면 반려");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn spawn_refuses_name_when_waking() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.spawn",
            json!({ "target": "alpha", "name": "beta" }),
        )
        .expect_err("깨우기에 name 은 아무 일도 못 한다");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn waking_a_stored_session_resumes_it() {
        let host = FakeHost::new();
        let id = host.with_agent("alpha", false, true);
        let (table, notify) = wiring(&host);

        let out = call(&table, "agent.spawn", json!({ "target": "alpha" })).expect("깨우기");
        assert_eq!(out["created"], false);
        assert_eq!(out["state"], "live");
        assert_eq!(host.started.lock().unwrap().as_slice(), &[(id, true)]);
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "깨우기는 명부 구성을 안 바꾼다 — 생사 전이는 매니저가 흘린다"
        );
    }

    #[test]
    fn waking_without_a_stored_session_starts_fresh() {
        let host = FakeHost::new();
        let id = host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        call(&table, "agent.spawn", json!({ "target": "alpha" })).expect("깨우기");
        assert_eq!(host.started.lock().unwrap().as_slice(), &[(id, false)]);
    }

    #[test]
    fn spawn_by_cwd_creates_then_starts() {
        let host = FakeHost::new();
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.spawn",
            json!({ "cwd": "C:/work/gamma", "name": "gamma" }),
        )
        .expect("생성 + 띄우기");
        assert_eq!(out["created"], true);
        assert_eq!(out["name"], "gamma");
        assert_eq!(host.started.lock().unwrap().len(), 1);
        assert_eq!(
            *notify.calls.lock().unwrap(),
            1,
            "새 항목이 생겼으므로 알린다"
        );
    }

    #[test]
    fn a_created_agent_that_fails_to_start_is_not_rolled_back() {
        let host = FakeHost::new();
        *host.start_fails.lock().unwrap() = true;
        let (table, _notify) = wiring(&host);

        let err =
            call(&table, "agent.spawn", json!({ "cwd": "C:/work/gamma" })).expect_err("기동 실패");
        assert_eq!(err.code(), ErrorCode::Internal);
        assert_eq!(host.rows.lock().unwrap().len(), 1, "명부에는 남는다");
    }

    #[test]
    fn new_registers_a_sleeping_agent() {
        let host = FakeHost::new();
        let (table, notify) = wiring(&host);

        let out = call(&table, "agent.new", json!({ "cwd": "C:/work/delta" })).expect("등록");
        assert_eq!(out["state"], "sleeping");
        assert!(host.started.lock().unwrap().is_empty(), "띄우지 않는다");
        assert_eq!(*notify.calls.lock().unwrap(), 1, "명부 변경을 알린다");
    }

    /// 등록 실패는 사유마다 다른 코드로 나간다 — 호출자가 할 일이 갈리기 때문이다.
    #[test]
    fn registration_failures_map_to_distinct_codes() {
        let cases = [
            (
                PtyError::RosterFull {
                    current: 40,
                    limit: 40,
                },
                ErrorCode::Conflict,
            ),
            (PtyError::CwdDenied, ErrorCode::InvalidArgument),
            (
                PtyError::SpawnFailed("disk on fire".to_string()),
                ErrorCode::Internal,
            ),
        ];

        for (failure, expected) in cases {
            let host = FakeHost::new();
            *host.create_fails.lock().unwrap() = Some(failure);
            let (table, notify) = wiring(&host);

            let err = call(&table, "agent.new", json!({ "cwd": "C:/work/delta" }))
                .expect_err("등록 실패");
            assert_eq!(err.code(), expected);
            assert_eq!(
                *notify.calls.lock().unwrap(),
                0,
                "실패했으면 명부는 안 바뀌었다"
            );
        }
    }

    #[test]
    fn rename_splits_renamed_from_unchanged() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        *host.rename_result.lock().unwrap() = Some(RenameOutcome::Unchanged("alpha".to_string()));
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.rename",
            json!({ "target": "alpha", "name": "alpha" }),
        )
        .expect("개명");
        assert_eq!(out["outcome"], "unchanged");

        let out = call(
            &table,
            "agent.rename",
            json!({ "target": "alpha", "name": "beta" }),
        )
        .expect("개명");
        assert_eq!(out["outcome"], "renamed");
        assert_eq!(out["name"], "beta");
        assert_eq!(
            *notify.calls.lock().unwrap(),
            2,
            "두 결말 모두 명부 표시가 바뀌므로 알린다"
        );
    }

    #[test]
    fn rename_exhaustion_is_a_conflict() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        *host.rename_result.lock().unwrap() = Some(RenameOutcome::Exhausted);
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.rename",
            json!({ "target": "alpha", "name": "beta" }),
        )
        .expect_err("이름 공간 소진");
        assert_eq!(err.code(), ErrorCode::Conflict);
    }

    /// ★`null` 은 「루트로 떼라」는 적극적 지시이고, **부재는 반려**다★ — 접으면 오타 필드 하나가
    /// 조용히 계층 해제를 실행한다.
    #[test]
    fn move_splits_an_absent_parent_from_an_explicit_null() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": null }),
        )
        .expect("null = 최상위로");
        assert_eq!(out["parent"], serde_json::Value::Null);
        assert_eq!(out["name"], "alpha", "성공 응답에 이름이 실린다");
        assert_eq!(*notify.calls.lock().unwrap(), 1, "명부 변경을 알린다");

        let err =
            call(&table, "agent.move", json!({ "target": "alpha" })).expect_err("부재는 반려");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("parent"),
            "어느 칸이 빠졌는지 말한다: {}",
            err.message()
        );
        assert_eq!(*notify.calls.lock().unwrap(), 1, "반려는 통지하지 않는다");
    }

    #[test]
    fn a_structural_rejection_is_conflict() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        host.with_agent("beta", false, false);
        *host.reparent_ok.lock().unwrap() = false;
        let (table, notify) = wiring(&host);

        let err = call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": "beta" }),
        )
        .expect_err("트리 구조가 거부");
        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(*notify.calls.lock().unwrap(), 0);
    }

    /// ★`false` 하나를 사유로 읽지 않는다★ — 그 사이 사라진 경우와 구조 충돌은 호출자가 할 일이 다르다.
    #[test]
    fn a_target_that_vanished_mid_move_is_not_found() {
        let host = FakeHost::new();
        let alpha = host.with_agent("alpha", false, false);
        *host.reparent_ok.lock().unwrap() = false;
        *host.vanish_on_reparent.lock().unwrap() = Some(alpha);
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": null }),
        )
        .expect_err("대상이 사라졌다");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(err.message().contains("alpha"), "{}", err.message());
    }

    /// ★사라진 쪽의 이름을 댄다★ — 부모만 없어졌는데 자식 이름을 대면, 호출자는 멀쩡히 있는 에이전트를
    /// 없다고 듣고 엉뚱한 데를 뒤진다.
    #[test]
    fn a_parent_that_vanished_mid_move_names_the_parent_not_the_child() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let beta = host.with_agent("beta", false, false);
        *host.reparent_ok.lock().unwrap() = false;
        *host.vanish_on_reparent.lock().unwrap() = Some(beta);
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": "beta" }),
        )
        .expect_err("새 부모가 사라졌다");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(
            err.message().contains("beta") && !err.message().contains("alpha"),
            "사라진 쪽을 지목해야 한다: {}",
            err.message()
        );
    }

    #[test]
    fn unknown_target_is_not_found_and_ambiguous_target_is_conflict() {
        let host = FakeHost::new();
        host.with_agent("twin", false, false);
        host.with_agent("twin", false, false);
        let (table, _notify) = wiring(&host);

        let missing =
            call(&table, "agent.spawn", json!({ "target": "nope" })).expect_err("없는 이름");
        assert_eq!(missing.code(), ErrorCode::NotFound);

        let ambiguous =
            call(&table, "agent.spawn", json!({ "target": "twin" })).expect_err("동명 둘");
        assert_eq!(ambiguous.code(), ErrorCode::Conflict);
    }

    #[test]
    fn an_id_is_matched_before_a_name() {
        let host = FakeHost::new();
        let id = host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        call(&table, "agent.spawn", json!({ "target": id.to_string() })).expect("id 지목");
        assert_eq!(host.started.lock().unwrap()[0].0, id);
    }

    #[test]
    fn a_blank_value_is_refused_not_treated_as_absent() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": "   " }),
        )
        .expect_err("공백만 있는 값");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn a_wrong_argument_type_is_invalid_argument() {
        let (table, _notify) = wiring(&FakeHost::new());
        let err = call(&table, "agent.rename", json!({ "target": 5 })).expect_err("타입 어긋남");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn the_list_schema_inlines_the_nested_row_type() {
        let spec = AgentListArgs::SPEC;
        let ok: serde_json::Value = serde_json::from_str(spec.ok_schema).expect("ok 스키마는 JSON");
        assert_eq!(ok["properties"]["agents"]["type"], "array");
        assert_eq!(
            ok["properties"]["agents"]["items"]["properties"]["state"]["type"], "string",
            "블록 안 선언 struct 가 인라인으로 펼쳐진다"
        );
    }
}
