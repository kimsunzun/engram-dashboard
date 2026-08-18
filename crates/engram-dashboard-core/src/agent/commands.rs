//! `agent.*` 명령 — **선언과 본문이 한 세트**로 일하는 코드(`AgentManager`) 옆에 산다(ADR-0155).
//!
//! ★이 표가 CLI 제어 동사의 실입구다★: 데몬의 `/control/agent` 라우트가 `{verb}` 를 `agent.<verb>` 로
//!   찾아 여기 핸들러를 부른다(daemon `control/agent.rs`). 즉 이 파일의 반환 모양·오류 코드·통지 횟수가
//!   그대로 `engram agent …` 의 응답이다.
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

// ★성공 응답은 평평하다(사용자 결정 2026-08-13)★: 명령마다 반환을 선언하므로 `{"agent":{…}}` 한 겹을
//   더 감쌀 이유가 없다.
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
// ADR-0155
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
///   때까지 옛 명부를 보여 준다**(조용한 stale). 명부의 **구성**을 바꾼 동사는 반드시 이 포트를 부른다 —
///   그 판정은 동사마다 다르고(깨우기는 구성을 안 바꾼다) 핸들러가 진다.
/// ★왜 표의 의존이 아니라 포트인가★: 실제 통지는 전-연결 팬아웃이라 데몬 소유다. 소비자인 여기가 좁은
///   trait 만 갖고 실물 어댑터는 조립부가 준다(daemon `control::RosterBroadcast` 와 같은 모양).
/// 논블록이어야 한다 — 팬아웃은 연결별 큐에 try_send 만 한다.
// ADR-0155
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
/// ★값의 집은 여기 하나다★ — 만들기 동사를 여는 입구가 늘어도 자기 상수를 두지 않고 이것을 참조한다.
pub const NEW_AGENT_OUTPUT_FORMAT: ClaudeOutputFormat = ClaudeOutputFormat::StreamJson;

/// `agent.*` 표를 조립한다 — ★핸들러 실물이 들어오는 유일한 자리★(규칙 T-1).
///
/// ★blocking 계약★: 핸들러 본문은 프로필 락을 쥔 채 디스크를 쓰고 resume 조기 종료를 폴링한다
///   (`AgentManager::activate_profile`). async 런타임 스레드에서 폴링하면 그 스레드를 막으므로 조립부가
///   `spawn_blocking` 뒤에서 불러야 한다 — 데몬 `/control/agent` 어댑터가 이미 그렇게 한다.
/// ★`notify` 를 인자로 받는 이유★: 명부를 바꾼 동사는 반드시 통지해야 하는데(포트 doc 참조) 그 실물은
///   데몬 소유다. 조립부가 넘기게 하면 **빠뜨릴 수 없다** — trait 기본 구현으로 두면 조용히 안 부른다.
/// ★명령이 늘어도 조립부(실행 파일)는 안 바뀐다★ — 늘어나는 것은 선언 블록과 이 함수의 한 줄이다.
// ADR-0155
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
    let (target, cwd, name) = (
        args.target.as_deref(),
        args.cwd.as_deref(),
        args.name.as_deref(),
    );
    // ★이 동사의 제약은 칸 하나가 아니라 **조합**이다★: 선언상 셋 다 `Option` 이지만 호출에는 `target` 이나
    //   `cwd` 중 하나가 반드시 있어야 한다. 그래서 「빼도 된다」로 안내하면 그대로 뺀 재시도가 「둘 중 하나는
    //   줘야 한다」로 **다시 반려**된다 — 칸 하나만 보는 문구로는 이 조건을 말할 수 없어 여기서 직접 쓴다.
    let blanks = blank_fields(&[("target", target), ("cwd", cwd), ("name", name)]);
    if !blanks.is_empty() {
        return Err(CommandError::invalid_argument(format!(
            "blank value(s) for: {} — an empty argument is usually an unset shell variable. spawn needs exactly one of target (an existing agent to wake) or cwd (a folder to create one in); name only applies when creating",
            blanks.join(", ")
        )));
    }

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
        // ★문구에 명령 식별자를 적지 않는다★: `agent.rename` 은 카탈로그 이름이지 칠 수 있는 명령이
        //   아니라, 그대로 적으면 LLM 이 그것을 셸에 친다. 표면별 표기는 어댑터가 붙인다.
        (Some(token), None) if name.is_some() => Err(CommandError::invalid_argument(format!(
            "name does not apply when waking an existing agent ({token}) — changing a name is a separate verb, so drop this field"
        ))),
        (Some(token), None) => wake_existing(host, token),
        (None, Some(cwd)) => create_and_start(host, notify, cwd, name.map(str::to_string)),
    }
}

/// ★깨우기는 명부 통지를 겹쳐 보내지 않는다★ — 항목 수·이름·계층이 그대로이고, 생사 전이는 매니저가
/// 이미 흘린다(`spawn_agent` 가 `agent_list_updated` 를 낸다).
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
            // ★회복 경로를 문구가 나른다★: 만들어진 에이전트는 명부에 남아 있으므로 호출자가 할 일은
            //   「다시 만들기」가 아니라 **그 이름으로 다시 띄우기**다. 안 적으면 같은 cwd 로 또 만들어
            //   이름이 하나씩 늘어난다.
            CommandError::internal(format!(
                "agent '{}' ({}) was created but did not start: {e} — it is registered and asleep, so start it again by that name instead of creating another",
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
    let (cwd, name) = (args.cwd.as_str(), args.name.as_deref());
    reject_blanks(&[
        ("cwd", Some(cwd), Blank::NeedsValue),
        ("name", name, Blank::OrOmit),
    ])?;
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
        // ★상한의 **성격**을 문구가 밝힌다★: 이 줄이 없으면 상한에 부딪힌 호출자(사람·LLM)의 첫 반응이
        //   "숫자를 올린다" 가 된다 — 그건 폭주 생성 루프를 막으려고 둔 백스톱을 스스로 걷는 것이다.
        PtyError::RosterFull { current, limit } => CommandError::of(
            ErrorCode::Conflict,
            format!(
                "the team already has {current} agents, which is the safety ceiling ({limit}) that stops a runaway create loop — it is not a product limit and raising it is not the fix. Remove agents you no longer need, then try again"
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
    let (token, name) = (args.target.as_str(), args.name.as_str());
    reject_blanks(&[
        ("target", Some(token), Blank::NeedsValue),
        ("name", Some(name), Blank::NeedsValue),
    ])?;
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
    let token = args.target.as_str();
    // ★부재는 「부모를 안 줬으니 떼자」가 아니다★ — 루트로 떼는 지시는 `null` 이다. 부재를 접으면 오타
    //   필드 하나가 계층 해제로 실행된다.
    // ★wire 로 들어온 부재는 여기까지 못 온다★ — 선언 매크로가 이 칸에 `#[serde(default)]` 를 안 달아
    //   역직렬화가 `missing field` 로 반려한다. 이 갈래는 코드가 인자를 직접 지어 부르는 경로 몫이다.
    let Some(parent_token) = args.parent.as_ref() else {
        return Err(CommandError::invalid_argument(
            "move needs parent: a name/id to move under, or null to move it back to the top level",
        ));
    };
    let parent_token = parent_token.as_deref();
    // ★`parent` 의 회복 경로는 다른 칸과 다르다★ — 빈 값을 「값을 채워라」로만 반려하면 위 부재 문구가 알려
    //   주는 `null` 갈래(최상위로 떼기)가 이 갈래에서만 감춰진다. 그렇다고 이 칸만 먼저 반려하면 **형제 칸이
    //   숨는다**: 둘 다 빈 값인 요청이 `parent` 만 지적받고, 고쳐 보낸 다음 트립에서 `target` 을 처음 듣는다.
    //   두 요구는 같은 검사 안에서 선다 — 칸마다 다른 안내를, 한 번에.
    reject_blanks(&[
        ("target", Some(token), Blank::NeedsValue),
        (
            "parent",
            parent_token,
            Blank::OrElse("a name/id to move under, or null to move it back to the top level"),
        ),
    ])?;
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
    // ★이 응답은 한 시점의 스냅샷이 **아니다**(알고 남긴 범위)★: 이름은 적용 뒤 명부에서 읽고 `parent` 는
    //   **이 호출이 지시한 값**이다. 그래서 뒤이어 다른 호출이 같은 에이전트를 또 옮기면, 응답은 「지금
    //   이름 + 이 호출이 붙인 부모」가 되어 그 조합이 어느 순간에도 동시에 참이 아닐 수 있다. 그래도 이
    //   보고는 **거짓이 아니다** — 이 호출이 요청한 이동은 실제로 적용됐고, 그것이 이 답이 진술하는 전부다.
    //   원자적 스냅샷을 원하면 매니저가 「적용 후 상태」를 함께 돌려주는 API 가 필요하고, 그건 여기 결정이
    //   아니다. 같은 이유로 **적용 뒤 사라진 대상도 `Ok`** 다(요청한 이동은 일어났다).
    Ok(AgentMoveOk {
        agent_id: child.id.to_string(),
        // ★이름은 **적용 뒤** 명부에서 다시 읽는다★: 해석 시점 값을 그대로 실으면, 그 사이 커밋된 개명
        //   때문에 응답이 「옛 이름 + 새 부모」라는 **실재한 적 없는 조합**이 된다(호출자는 그 이름으로
        //   다음 명령을 친다). 대가는 성공 경로의 명부 조회 한 번이다.
        // ★못 읽으면 해석 시점 이름으로 되돌린다 — 빈 문자열로 접지 않는다★: 그 사이 사라진 경우에
        //   이름 없는 행을 성공 응답으로 내면 호출자의 판정기가 그것을 "읽을 수 없는 응답" 으로 읽는다.
        name: current_name(host, child.id).unwrap_or(child.name),
        parent: parent.map(|p| p.to_string()),
    })
}

fn current_name(host: &dyn AgentCommandHost, id: AgentId) -> Option<String> {
    host.roster()
        .into_iter()
        .find(|row| row.id == id)
        .map(|row| row.canonical_name)
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
/// ★관대한 해석기를 들이지 말 것★: 사람과 LLM 은 우편의 수신자 토큰과 제어의 지목 토큰이 같은 X 를
/// 가리킨다고 읽는다. 두 입구의 해석이 갈리면 편지를 받은 에이전트와 이름이 바뀐 에이전트가 달라진다 —
/// 그 일치는 데몬 crate 의 교차 대조 테스트가 [`resolve_in`] 을 태워 지킨다.
// ADR-0132
fn resolve(host: &dyn AgentCommandHost, token: &str) -> Result<ResolvedAgent, CommandError> {
    resolve_in(&host.roster(), token)
}

/// [`resolve`] 의 순수한 알맹이 — **제어 입구가 실제로 쓰는 해석 규칙 그 자체**다.
///
/// ★`pub` 인 이유(이것만이 근거다)★: 데몬 crate 의 교차 대조 테스트가 우편 입구와 **같은 규칙**인지를
/// 재려면 실입구가 쓰는 해석기를 태워야 한다. 명부를 인자로 받는 형태라 그 테스트가 `AgentCommandHost`
/// 전체를 흉내 내지 않아도 되고, 사본을 따로 두지 않으므로 재는 것과 도는 것이 갈릴 수 없다.
/// ★결말은 코드로 읽는다★: 부재 = `NOT_FOUND` · 동명 둘 이상 = `CONFLICT`(이 함수가 내는 두 코드다).
// ADR-0132
// ADR-0155
pub fn resolve_in(roster: &[AgentRosterRow], token: &str) -> Result<ResolvedAgent, CommandError> {
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
                "more than one agent is called '{token}', so this command would have to guess — use the agent id instead; the roster listing shows both"
            ),
        )),
        _ => Err(not_found(token)),
    }
}

/// 지목이 가리킨 에이전트 — id 와 **그때 명부가 말한 이름**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAgent {
    pub id: AgentId,
    pub name: String,
}

fn not_found(token: &str) -> CommandError {
    CommandError::not_found(format!(
        "no agent called '{token}' — names are matched exactly, with no case-folding, prefixes or trimming; the roster listing shows the names in use"
    ))
}

/// 공백만 있는 값은 **부재로 접지 않고** 반려한다.
///
/// ★셸에서 미설정 변수가 빈 인자로 펼쳐지는 형태(`--parent "$UNSET"`)가 현실적으로 들어온다★ — 그걸
/// "안 준 것" 으로 접으면 오타 한 번이 다른 동작(계층 해제 등)으로 조용히 바뀐다.
/// ★한 칸씩 끊지 않고 **전부 모아** 낸다★: 첫 칸에서 끊으면 셋을 빈 값으로 보낸 호출자가 반려·수정을
/// 세 번 반복한다(같은 이유로 입구의 모르는 칸 검문도 전부 센다 — `CommandTable::check_args`).
/// ★안내는 **칸마다** 다르다(load-bearing)★: 시키는 대로 했더니 또 반려당하면 호출자(특히 LLM)는 같은
/// 자리를 맴돈다. 필수 칸에 "빼도 된다" 고 말하면 뺀 재시도가 「빠졌다」로 반려되고, `null` 이 적극적 지시인
/// 칸에 "값을 채워라" 만 말하면 그 갈래가 통째로 감춰진다. 그래서 칸마다 **그 칸에서 실제로 통하는 길**을
/// 적는다([`Blank`]).
fn reject_blanks(fields: &[(&str, Option<&str>, Blank)]) -> Result<(), CommandError> {
    let advice: Vec<String> = fields
        .iter()
        .filter(|(_, given, _)| is_blank(*given))
        .map(|(field, _, what)| match what {
            Blank::NeedsValue => format!("{field} needs a real value"),
            Blank::OrOmit => format!("{field} must either carry a value or be left out entirely"),
            Blank::OrElse(alternative) => format!("{field} needs {alternative}"),
        })
        .collect();
    if advice.is_empty() {
        return Ok(());
    }
    Err(CommandError::invalid_argument(format!(
        "blank value(s) — an empty argument is usually an unset shell variable: {}",
        advice.join("; ")
    )))
}

/// 빈 값으로 온 칸에서 **무엇이 실제로 통하는가**.
enum Blank<'a> {
    /// 값이 반드시 있어야 한다(빼면 「빠졌다」로 반려된다).
    NeedsValue,
    /// 값을 넣거나 칸을 통째로 빼면 된다.
    OrOmit,
    /// 값 말고 다른 갈래가 있다 — 그 갈래를 문구에 그대로 싣는다(예: `null` 이 적극적 지시인 칸).
    OrElse(&'a str),
}

fn is_blank(given: Option<&str>) -> bool {
    given.is_some_and(|v| v.trim().is_empty())
}

/// 빈 값으로 온 칸 이름 전부 — 반려 문구를 **동사가 직접** 쓸 때 쓴다(제약이 칸 하나가 아니라 조합인 자리).
fn blank_fields<'a>(fields: &[(&'a str, Option<&str>)]) -> Vec<&'a str> {
    fields
        .iter()
        .filter(|(_, given)| is_blank(*given))
        .map(|(field, _)| *field)
        .collect()
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
        /// reparent 호출 **안에서** 명부에서 지울 대상 — 「그 사이 사라졌다」를 재현한다(자식이든 부모든).
        /// ★적용 성패와 무관하게 지운다★: 실패 뒤 사라짐은 사유 분기를, 성공 뒤 사라짐은 응답 이름의
        /// fallback 을 태운다 — 한쪽에만 걸면 다른 쪽 코드에 어떤 테스트도 못 닿는다.
        vanish_on_reparent: Mutex<Option<AgentId>>,
        /// `activate_profile` 이 돌려줄 상태. ★terminal 을 넣을 수 있어야 한다★ — 띄우자마자 죽는 경우
        ///   (ADR-0082 resume 조기 종료)가 응답 상태를 지어내는지 재는 유일한 방법이고, 여기가 `Running`
        ///   으로 굳어 있으면 `started_payload` 에 `"live"` 를 박아도 전 스위트가 초록이다.
        started_status: Mutex<Option<AgentStatus>>,
        /// reparent 성공 직후 자식이 얻는 새 이름 — 「적용 뒤 개명이 끼어들었다」를 재현한다.
        rename_on_reparent: Mutex<Option<String>>,
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
            self.with_status(name, live.then_some(AgentStatus::Running), resumable)
        }

        /// 명부 행의 상태를 **그대로** 심는다 — terminal 상태의 행이 `list` 에서 어떤 낱말로 나오는지를
        /// 재려면 `Running` 말고도 넣을 수 있어야 한다.
        fn with_status(
            self: &Arc<Self>,
            name: &str,
            live: Option<AgentStatus>,
            resumable: bool,
        ) -> AgentId {
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
                live,
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
            let status = self
                .started_status
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(AgentStatus::Running);
            Ok(StartedAgent {
                id: profile.id,
                name: profile.canonical_name_when_live(),
                status,
            })
        }

        fn rename_agent(&self, _id: AgentId, display_name: Option<String>) -> RenameOutcome {
            self.rename_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| RenameOutcome::Renamed(display_name.unwrap_or_default()))
        }

        fn reparent_agent(&self, child: AgentId, _parent: Option<AgentId>) -> bool {
            let ok = *self.reparent_ok.lock().unwrap();
            // ★적용 성패와 무관하게 사라진다★: 성공 **뒤** 사라지는 창이 응답 이름의 fallback 을 태우는
            //   유일한 경로다. 실패 쪽에만 걸면 그 fallback 은 어떤 테스트로도 못 닿는다.
            if let Some(vanished) = *self.vanish_on_reparent.lock().unwrap() {
                self.rows.lock().unwrap().retain(|row| row.id != vanished);
            }
            // 적용과 응답 사이에 개명이 커밋되는 창 — 실제로는 다른 호출자가 낸다.
            if let (true, Some(renamed)) = (ok, self.rename_on_reparent.lock().unwrap().take()) {
                for row in self.rows.lock().unwrap().iter_mut() {
                    if row.id == child {
                        row.canonical_name = renamed.clone();
                    }
                }
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
        // ★같은 칸이면 어느 실수를 했든 같은 갈래를 알려 준다★: 부재 문구가 알려 주는 `null`(최상위로
        //   떼기)을 빈 값 문구가 감추면, 호출자는 그 칸에 값을 넣는 길밖에 없다고 읽는다.
        assert!(
            err.message().contains("null"),
            "빈 값에도 null 갈래를 알려야: {}",
            err.message()
        );
    }

    /// ★반려 문구가 시키는 대로 하면 통해야 한다★: 필수 칸을 빈 값으로 보낸 호출자에게 "빼도 된다" 고
    /// 말하면, 그대로 뺀 재시도가 **다시 반려**된다(이번엔 「빠졌다」로) — LLM 은 문구를 그대로 따르므로
    /// 같은 자리를 맴돈다. 필수 칸에는 값을 채우라고만 말한다.
    #[test]
    fn the_advice_for_a_blank_required_field_does_not_tell_the_caller_to_drop_it() {
        let host = FakeHost::new();
        let (table, notify) = wiring(&host);

        let required = call(&table, "agent.new", json!({ "cwd": "   " }))
            .expect_err("필수 칸이 공백")
            .message()
            .to_string();
        assert!(
            required.contains("cwd needs a real value"),
            "값을 채우라고 말해야(수 일치 포함): {required}"
        );
        assert!(
            !required.contains("left out"),
            "필수 칸을 빼라고 하면 재시도가 다시 반려된다: {required}"
        );
        // ★반려는 아무것도 만들지 않는다★ — 검사가 등록 뒤로 밀리면 빈 인자 호출이 에이전트를 만들어 놓고
        //   반려를 답한다(호출자는 실패로 읽고 다시 부른다 — 이름만 하나씩 늘어난다).
        assert!(
            host.rows.lock().unwrap().is_empty(),
            "반려는 등록하지 않는다"
        );
        assert_eq!(*notify.calls.lock().unwrap(), 0, "반려는 통지하지 않는다");

        // 선택 칸은 반대다 — 빼는 것이 실제로 통하는 길이라 그 길을 함께 알려 준다.
        let optional = call(&table, "agent.new", json!({ "cwd": "C:/x", "name": " " }))
            .expect_err("선택 칸이 공백")
            .message()
            .to_string();
        assert!(
            optional.contains("name") && optional.contains("left out"),
            "선택 칸은 빼도 된다고 말해도 참이다: {optional}"
        );
        assert!(
            host.rows.lock().unwrap().is_empty(),
            "반려는 등록하지 않는다"
        );
        assert_eq!(*notify.calls.lock().unwrap(), 0, "반려는 통지하지 않는다");

        // 시킨 대로 뺀 재시도가 실제로 통한다.
        call(&table, "agent.new", json!({ "cwd": "C:/x" })).expect("빼라는 안내대로 하면 통한다");
    }

    /// ★`rename` 의 두 칸은 **둘 다 필수**다★ — 어느 쪽에든 「빼도 된다」고 말하면 그대로 뺀 재시도가 입구
    /// 검문에서 「requires …」로 다시 반려되어 호출자가 같은 자리를 맴돈다.
    ///
    /// ★칸 하나하나가 아니라 **호출 지점마다** 봐야 하는 성질이다★: 빈 값 안내의 옳고 그름은 그 칸이 그
    /// 동사에서 무엇을 요구받는지에 달렸으므로, 공용 함수를 한 번 검사하는 것으로는 어느 동사도 안 지켜진다.
    #[test]
    fn a_blank_rename_argument_is_told_to_fill_it_not_to_drop_it() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, notify) = wiring(&host);

        for (args, blank) in [
            (json!({ "target": "alpha", "name": "  " }), "name"),
            (json!({ "target": " ", "name": "beta" }), "target"),
        ] {
            let refusal = call(&table, "agent.rename", args.clone())
                .expect_err("공백 값")
                .message()
                .to_string();

            assert!(
                refusal.contains(&format!("{blank} needs a real value")),
                "{blank} 에 값을 채우라고 말해야: {refusal}"
            );
            assert!(
                !refusal.contains("left out"),
                "{blank} 을 빼라고 하면 그 재시도가 「requires」로 다시 반려된다: {refusal}"
            );
            assert_eq!(*notify.calls.lock().unwrap(), 0, "반려는 통지하지 않는다");
        }

        // 문구대로 두 칸을 채운 재시도가 통한다.
        let out = call(
            &table,
            "agent.rename",
            json!({ "target": "alpha", "name": "beta" }),
        )
        .expect("안내대로 채우면 통한다");
        assert_eq!(out["name"], "beta", "{out}");
    }

    /// ★`spawn` 의 제약은 칸이 아니라 **조합**이다★: `cwd` 는 선언상 `Option` 이라 「빼도 된다」가 선언
    /// 기준으로는 참이지만, 그대로 뺀 재시도는 「target 이나 cwd 중 하나는 줘야 한다」로 다시 반려된다.
    /// 문구가 조합을 말해야 한 번에 빠져나온다.
    #[test]
    fn a_blank_spawn_argument_is_told_what_actually_satisfies_the_verb() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, notify) = wiring(&host);
        let before = host.rows.lock().unwrap().len();

        let refusal = call(&table, "agent.spawn", json!({ "cwd": "  " }))
            .expect_err("공백 cwd")
            .message()
            .to_string();

        assert!(refusal.contains("cwd"), "어느 칸인지 짚어야: {refusal}");
        assert!(
            !refusal.contains("left out"),
            "빼라고 하면 그 재시도가 다시 반려된다: {refusal}"
        );
        assert!(
            refusal.contains("target") && refusal.contains("cwd"),
            "무엇을 주면 통하는지(둘 중 하나)를 말해야: {refusal}"
        );
        // ★이 동사에서 반려는 **되돌릴 수 없는 일을 하기 전에** 나야 한다★: 빈 값 검사가 조합 분기 뒤로
        //   밀리면 `cwd` 가 공백인 호출이 에이전트를 **만들고 통지까지 한 다음** 반려를 답한다. 호출자는
        //   실패로 읽고 다시 부르므로 이름만 하나씩 늘고, 만들어진 것은 지울 경로가 없다(ADR-0122).
        assert_eq!(
            host.rows.lock().unwrap().len(),
            before,
            "반려가 에이전트를 만들었다"
        );
        assert_eq!(*notify.calls.lock().unwrap(), 0, "반려는 통지하지 않는다");

        // 문구가 가리키는 두 길이 **둘 다** 실제로 통한다.
        call(&table, "agent.spawn", json!({ "target": "alpha" })).expect("깨우기 길");
        call(&table, "agent.spawn", json!({ "cwd": "C:/work/new" })).expect("만들기 길");
    }

    /// ★`move` 는 빈 칸 둘을 **한 번에** 짚는다★ — `parent` 만 먼저 반려하면 고쳐 보낸 다음 트립에서야
    /// `target` 을 처음 듣는다(한 번 잘못 친 호출에 왕복 두 번). 그러면서도 `parent` 의 `null` 갈래는 남는다.
    #[test]
    fn a_move_with_two_blank_fields_names_both_in_one_trip() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        let refusal = call(
            &table,
            "agent.move",
            json!({ "target": " ", "parent": "  " }),
        )
        .expect_err("둘 다 공백")
        .message()
        .to_string();

        assert!(refusal.contains("target"), "형제 칸이 숨었다: {refusal}");
        assert!(refusal.contains("parent"), "{refusal}");
        assert!(
            refusal.contains("null"),
            "한 번에 짚으면서도 null 갈래를 잃지 않는다: {refusal}"
        );

        // 문구대로 고친 재시도가 통한다.
        call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": null }),
        )
        .expect("안내대로 고치면 통한다");
    }

    /// ★빈 칸이 여럿이면 여럿 다 나온다★ — 하나씩 짚으면 셸의 미설정 변수 셋을 보낸 호출자가 반려·수정을
    /// 세 번 반복한다(입구의 모르는 칸 검문과 같은 규율).
    #[test]
    fn every_blank_field_is_named_not_just_the_first() {
        let (table, _notify) = wiring(&FakeHost::new());

        let err = call(
            &table,
            "agent.spawn",
            json!({ "target": " ", "cwd": "  ", "name": "\t" }),
        )
        .expect_err("전부 공백");

        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        for field in ["target", "cwd", "name"] {
            assert!(
                err.message().contains(field),
                "{field} 이 빠졌다: {}",
                err.message()
            );
        }
    }

    // ── 상태 정직성 ──────────────────────────────────────────────────────────────

    /// ★변경 동사와 `list` 가 같은 함수를 지나야 두 답이 갈릴 수 없다★ — 전 상태를 여기서 못박는다.
    ///
    /// ★알려진 사각(고치지 않았다)★: 아래 목록은 **손으로 적은 것**이고 `state_of` 는 포괄 갈래(`_ =>`)로
    /// 잠듦을 낸다. 그래서 새 변형이 생겨도 이 테스트는 초록이고, 그 변형이 「살아 있는 성질」이면 조용히
    /// 잠듦으로 나간다. 목록을 자동으로 채우려면 `AgentStatus` 쪽 형태를 바꿔야 해 여기서 하지 않는다.
    #[test]
    fn every_status_maps_to_one_of_the_two_agent_state_words() {
        assert_eq!(
            state_of(None),
            AGENT_STATE_SLEEPING,
            "세션이 없으면 잠든 것"
        );
        assert_eq!(state_of(Some(&AgentStatus::Running)), AGENT_STATE_LIVE);
        assert_eq!(
            state_of(Some(&AgentStatus::Exiting)),
            AGENT_STATE_LIVE,
            "내려가는 중은 아직 산 것이다"
        );
        for dead in [
            AgentStatus::Exited { code: Some(0) },
            AgentStatus::Exited { code: None },
            AgentStatus::Killed,
            AgentStatus::Failed {
                message: "boom".to_string(),
            },
        ] {
            assert_eq!(
                state_of(Some(&dead)),
                AGENT_STATE_SLEEPING,
                "시체는 산 것이 아니다: {dead:?}"
            );
        }
        // 메시지 결말 어휘와 섞이지 않는다(두 축을 섞어 결정이 꼬인 적이 있다 — ADR-0116).
        for message_word in ["delivered", "pending", "failed"] {
            assert_ne!(AGENT_STATE_LIVE, message_word);
            assert_ne!(AGENT_STATE_SLEEPING, message_word);
        }
    }

    /// ★응답의 상태는 **파생**이지 리터럴이 아니다★: 띄우자마자 죽은 에이전트(ADR-0082 resume 조기 종료 —
    /// fresh fallback 이 없다)를 `spawn` 이 살아 있다고 답하면, 직후의 `list` 는 잠들었다고 답한다. 호출자는
    /// 그 사이에 시체에게 편지를 쓴다. `started_payload` 에 `"live"` 를 박으면 여기서 죽는다.
    #[test]
    fn a_spawn_whose_activation_lands_terminal_reports_sleeping() {
        for terminal in [
            AgentStatus::Exited { code: Some(1) },
            AgentStatus::Killed,
            AgentStatus::Failed {
                message: "resume died".to_string(),
            },
        ] {
            let host = FakeHost::new();
            host.with_agent("alpha", false, true);
            *host.started_status.lock().unwrap() = Some(terminal.clone());
            let (table, _notify) = wiring(&host);

            let woken = call(&table, "agent.spawn", json!({ "target": "alpha" })).expect("깨우기");
            assert_eq!(
                woken["state"], AGENT_STATE_SLEEPING,
                "깨우자마자 죽은 것을 살아 있다고 보고하면 안 된다: {terminal:?}"
            );
            assert_eq!(woken["created"], false);

            let born = call(&table, "agent.spawn", json!({ "cwd": "C:/work/beta" }))
                .expect("만들어서 띄우기");
            assert_eq!(
                born["state"], AGENT_STATE_SLEEPING,
                "만들기 경로도 같은 파생을 쓴다: {terminal:?}"
            );
            assert_eq!(born["created"], true);
        }
    }

    /// 명부가 든 terminal 상태도 같은 낱말로 나온다 — 변경 동사와 조회가 한 함수를 지난다는 축의 나머지 반.
    #[test]
    fn the_roster_reports_a_terminal_status_as_sleeping() {
        let host = FakeHost::new();
        host.with_status(
            "zombie",
            Some(AgentStatus::Failed {
                message: "died".to_string(),
            }),
            false,
        );
        host.with_status("alive", Some(AgentStatus::Running), false);
        let (table, _notify) = wiring(&host);

        let out = call(&table, "agent.list", json!({})).expect("조회");
        let agents = out["agents"].as_array().expect("배열");
        assert_eq!(agents[0]["state"], AGENT_STATE_SLEEPING, "{out}");
        assert_eq!(agents[1]["state"], AGENT_STATE_LIVE, "{out}");
    }

    /// ★이동 응답의 이름은 **적용 뒤** 값이다★ — 해석 시점 값을 실으면 그 사이 커밋된 개명 때문에 응답이
    /// 「옛 이름 + 새 부모」라는 실재한 적 없는 조합이 되고, 호출자는 그 옛 이름으로 다음 명령을 친다.
    #[test]
    fn move_reports_the_name_the_agent_has_after_the_move() {
        let host = FakeHost::new();
        let helper = host.with_agent("helper", false, false);
        let lead = host.with_agent("lead", false, false);
        // ★옛 부모를 심어 둔다★: 응답의 `parent` 가 **이 호출이 지시한 부모**인지 명부에 남아 있던 옛 값인지를
        //   가르는 유일한 방법이다(둘 다 없으면 어느 쪽을 실어도 같은 값이라 구분이 안 된다).
        //   ★가짜 매니저는 부모 칸을 안 고친다★ — 그래서 명부에는 이 옛 값이 그대로 남는다.
        let stale = AgentId::new_v4();
        for row in host.rows.lock().unwrap().iter_mut() {
            if row.id == helper {
                row.parent = Some(stale);
            }
        }
        *host.rename_on_reparent.lock().unwrap() = Some("renamed".to_string());
        let (table, _notify) = wiring(&host);

        let out = call(
            &table,
            "agent.move",
            json!({ "target": "helper", "parent": "lead" }),
        )
        .expect("이동");
        assert_eq!(out["name"], "renamed", "{out}");
        assert_eq!(
            out["parent"],
            lead.to_string(),
            "이 호출이 붙인 부모를 싣는다(명부에 남은 옛 부모가 아니다): {out}"
        );
    }

    /// ★적용은 됐는데 그 사이 사라진 대상★ — 응답은 **이름 없는 성공 행**이 되면 안 된다. 빈 이름을 실으면
    /// 호출자의 판정기가 성공 응답을 "읽을 수 없는 응답" 으로 읽고, 실제로 일어난 이동이 실패로 기록된다.
    ///
    /// ★해석은 성공해야 이 갈래에 닿는다★: 명부를 미리 비우면 `resolve` 가 먼저 `NOT_FOUND` 를 내 이 코드는
    /// 돌지도 않는다 — 그래서 사라짐을 **적용 시점에** 일으킨다(`vanish_on_reparent`).
    #[test]
    fn move_falls_back_to_the_resolved_name_when_the_agent_vanishes_after_the_move() {
        let host = FakeHost::new();
        let helper = host.with_agent("helper", false, false);
        // 적용은 성공하되(reparent_ok = true) 그 호출 안에서 명부에서 사라진다.
        *host.vanish_on_reparent.lock().unwrap() = Some(helper);
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.move",
            json!({ "target": "helper", "parent": null }),
        )
        .expect("적용 자체는 성공했다");

        // ★이 단언이 없으면 테스트가 fallback 을 안 태운다★: 행이 남아 있으면 `current_name` 이 값을 내
        //   아래 이름 단언이 그대로 통과한다 — 즉 seam 이 안 발화해도 초록이다.
        assert!(
            host.roster().iter().all(|row| row.id != helper),
            "전제: 적용 도중 명부에서 사라졌어야 fallback 이 돈다"
        );
        assert_eq!(
            out["name"], "helper",
            "적용 뒤 못 읽으면 해석 시점 이름으로 되돌린다(빈 이름 금지): {out}"
        );
        assert_eq!(out["agent_id"], helper.to_string(), "{out}");
        assert!(
            out["name"].as_str().is_some_and(|n| !n.is_empty()),
            "성공 응답에 이름 없는 행이 실리면 안 된다: {out}"
        );
        // 대상이 사라져도 **부모 축은 이 호출이 지시한 값**을 싣는다 — 떼라고 시킨 요청이 성공했는데
        //   `parent` 칸이 비거나 사라지면 호출자는 무엇이 적용됐는지 못 읽는다.
        //   (옛 부모를 싣는 결함은 명부가 남아 있어야 갈리므로 아래 이웃 테스트가 본다.)
        assert!(
            out["parent"].is_null(),
            "떼라고 지시했으니 null 이다: {out}"
        );
        assert_eq!(*notify.calls.lock().unwrap(), 1, "적용됐으므로 알린다");
    }

    // ── 지목 해석(교차 대조가 태우는 그 함수) ────────────────────────────────────────

    /// ★데몬의 교차 대조 테스트가 태우는 진입점이 이것이다★ — 여기가 표의 동사들이 쓰는 해석기와 같은
    /// 함수라는 것이 그 대조의 전제다(`resolve` 가 이 함수로 위임한다).
    #[test]
    fn resolve_in_matches_ids_first_then_exact_names_and_refuses_duplicates() {
        let host = FakeHost::new();
        let alpha = host.with_agent("alpha", false, false);
        let twin_a = host.with_agent("twin", false, false);
        let _twin_b = host.with_agent("twin", false, false);
        let roster = host.roster();

        assert_eq!(resolve_in(&roster, "alpha").expect("이름 일치").id, alpha);
        assert_eq!(
            resolve_in(&roster, &twin_a.to_string())
                .expect("id 일치")
                .id,
            twin_a,
            "동명이어도 id 지목은 통한다"
        );
        assert_eq!(
            resolve_in(&roster, "twin").expect_err("동명 둘").code(),
            ErrorCode::Conflict
        );
        for miss in ["ALPHA", "alph", " alpha", "alpha "] {
            assert_eq!(
                resolve_in(&roster, miss).expect_err("정확 일치만").code(),
                ErrorCode::NotFound,
                "{miss}"
            );
        }
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
