//! 데몬이 **직접 실행할 수 있는** 명령의 표 — 실물 의존이 들어오는 유일한 자리(규칙 T-1).
//!
//! ★이 표를 부르는 표면은 **셋**이고 검문은 **하나**다★: 계열 라우트(`/control/agent` — 이웃 `agent.rs`
//!   가 `{verb}` 를 `agent.<verb>` 로 찾는다) · 전체 이름 라우트(`/control/call` — 이웃 `catalog.rs`) ·
//!   명령 버스 배달의 1단계(`command_delivery`). 셋 다 [`call_daemon_command`] 하나로 들어오므로 입구
//!   검문(ADR-0157)을 건너뛰는 표면이 없다. 표가 슬롯에 안 꽂혀 있으면 두 라우트는 503 이고 배달은
//!   1단계 미스다.
//! ★선언은 여기 없다★ — `agent.*` 의 계약은 agent 가 소유하고(ADR-0155 결정 1: 선언이 사는 곳이 곧
//!   주인이다) 이 파일은 그 선언에 데몬의 실물(매니저 · 명부 통지 팬아웃)을 꽂기만 한다. 데몬 자기
//!   명령(`mail.*`)이 생기면 그때 선언 블록이 이 crate 로 들어온다.
//!
//! 진입점: [`make_daemon_table`](조립) · [`call_daemon_command`](세 표면의 공통 입구) ·
//! [`DaemonLocalCommands`](배달 1단계 어댑터) · [`InputLease`](입력 임대 포트).
//!
//! tauri import 0(daemon crate).
// ADR-0155

use std::sync::Arc;

use engram_dashboard_agent::commands::{
    make_table, resolve_in, AgentCommandHost, RosterChanged, INPUT_AFFECTING,
};
use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::types::AgentId;
use engram_dashboard_command::{
    CommandDecl, CommandError, CommandFuture, CommandSpec, CommandTable, Effect, ErrorCode,
};
use engram_dashboard_net::frame_port::ConnId;
use futures_util::FutureExt as _;

use crate::command_delivery::LocalCommands;
use crate::connection_core::INPUT_LOCKED_REFUSAL;

use super::mcp_server::{CommandTableSlot, RosterBroadcastSlot};

/// `agent.*` 표에 데몬 실물을 꽂는다.
///
/// ★blocking 계약이 그대로 딸려 온다★: 핸들러 본문은 프로필 락을 쥔 채 디스크를 쓰고 resume 조기
///   종료를 폴링한다(agent `make_table` doc). 그래서 이 표는 [`call_daemon_command`] 로만 부르고, 그
///   호출은 blocking 풀 위에 있어야 한다.
/// ★`broadcast` 가 값이 아니라 **슬롯**인 이유(load-bearing)★: 명부 통지 팬아웃은 연결 레지스트리에서
///   파생돼 이 표보다 **뒤에** 조립된다. 조립 시점의 값을 받으면 그때 비어 있던 슬롯이 프로세스 수명
///   내내 "통지 없음" 으로 굳고, 증상은 에러도 로그도 없이 **명부를 바꿔도 트리가 옛 명부를 보여
///   주는 것**이다(agent `RosterChanged` 포트 doc). 슬롯을 받으면 읽는 시점이 호출 때로 밀려 **표 조립
///   순서**는 그 증상을 만들 수 없다 — 인자 형태가 순서 규율을 산문 대신 타입으로 지고 있다.
///   ★단 이게 닫는 건 조립 순서뿐이다★: 서버가 뜨고 나서 이 슬롯이 채워지기 전에 도착한 변경 요청은
///   여전히 통지를 건너뛴 채 성공(ok)으로 응답한다 — 그 창은 슬롯을 늦게 채우는 조립이 있는 한 남는다.
/// ★`lease` 는 WS 연결이 쥐는 **그 한 부**여야 한다★(운영 = 수락 루프에 넘기는 `MultiViewState`). 그
///   값의 `clone()` 은 같은 공유 상태를 가리키는 손잡이라 괜찮다. ★새 `MultiViewState::new()` 를 넘기면
///   틀린다★ — 빈 표라 버스·CLI 의 입력 영향 명령이 임대를 조용히 무시한다(에러도 로그도 없다).
// ADR-0155
// ADR-0231
pub fn make_daemon_table(
    manager: Arc<AgentManager>,
    broadcast: Arc<RosterBroadcastSlot>,
    lease: Arc<dyn InputLease>,
) -> DaemonTable {
    DaemonTable {
        table: make_table(manager.clone(), Arc::new(LateRosterBroadcast(broadcast))),
        roster: manager,
        lease,
    }
}

/// 데몬 명령 표 — `agent.*` 표와, 그 표의 입력 영향 명령([`INPUT_AFFECTING`])을 검문할 재료(명부 · 임대
/// 포트)를 **한 값으로** 든다.
///
/// ★한 값인 이유★: 재료가 따로 꽂히면 표는 들어왔는데 임대 포트는 빈 조립이 표현 가능해지고, 그 상태는
///   아무 신호 없이 임대를 건너뛴다. 표가 슬롯에 들어가는 순간(매니저 조립 뒤) 셋이 함께 들어간다.
/// ★안의 [`CommandTable`] 을 꺼내 주는 문을 두지 않는다★ — 꺼낼 수 있으면 [`call_daemon_command`] 를
///   건너뛰는 두 번째 입구가 생기고, 두 검문(ADR-0157 인자 · ADR-0231 임대)이 함께 빠진다.
// ADR-0231
pub struct DaemonTable {
    table: CommandTable,
    /// 지목 토큰을 푸는 명부 — 동사 본문이 쓰는 그 호스트다(해석기가 같아야 검사한 에이전트 = 실행하는 에이전트).
    roster: Arc<dyn AgentCommandHost>,
    lease: Arc<dyn InputLease>,
}

impl DaemonTable {
    pub fn specs(&self) -> impl Iterator<Item = &'static CommandSpec> + '_ {
        self.table.specs()
    }

    pub fn decls(&self) -> Vec<CommandDecl> {
        self.table.decls()
    }
}

/// 입력 임대 포트 — 산 에이전트의 입력을 움직이는 명령이 그 에이전트에 닿아도 되는가.
///
/// `caller` = 호출한 연결. `None` = 연결 없는 호출자(CLI 두 라우트)이고 **임대 보유자가 아니다** — 아무도 안
/// 쥐었으면 통과, 누가 쥐었으면 거절. WS 의 `WriteStdin`·`Interrupt`·`CancelQueuedInput` 과 **같은 판정**이어야
/// 한다(운영 구현 = `MultiViewState` — 판정을 그 한 곳에 둔다). 판정 중 잡는 잠금은 이 호출 안에서 끝난다.
// ADR-0231
pub trait InputLease: Send + Sync {
    fn permits(&self, agent_id: AgentId, caller: Option<ConnId>) -> bool;
}

/// WS 연결을 받지 않는 조립(스모크 bin · 제어 라우트 하네스)의 임대 포트 — 임대를 쥘 표면이 없어 늘 통과다.
///
/// ★WS 수락 루프가 도는 조립에 꽂지 말 것★ — 그러면 버스·CLI 의 입력 영향 명령이 뷰어의 임대를 무시한다.
// ADR-0231
pub struct NoInputLeases;

impl InputLease for NoInputLeases {
    fn permits(&self, _agent_id: AgentId, _caller: Option<ConnId>) -> bool {
        true
    }
}

/// 이 표를 부르는 **유일한 자리** — 세 표면(제어 라우트 둘 · 명령 버스 배달)이 여기로 든다.
///
/// 반환 `None` = **이 표의 이름이 아니다**. 그 갈래를 어떻게 대접할지는 부르는 쪽이 안다: 계열 라우트는
/// 「모르는 동사」로 답하고, 전체 이름 라우트는 명부에 다시 물어 「모르는 이름」과 「이 입구가 못 닿는 남의
/// 이름」을 가르며(`catalog::handle_call`), 배달은 3단계의 다음 단계로 넘어간다.
///
/// ★입구 검문이 여기 하나다(ADR-0157)★: 두 표면 다 **사람·LLM 이 방금 친 것**이 오는 자리라 선언에 없는
///   칸·빠진 필수 칸을 이름 지어 반려한다(`parnet` 오타를 흘리면 `move … --parent lead` 가 조용히 루트로
///   떼기가 된다). ★[`CommandTable::call`] 을 직접 부르는 두 번째 경로를 만들지 말 것★ — 그 표면만 검문
///   없이 돌게 되고, 그것이 ADR-0157 이 막으려던 실패 그대로다.
/// ★남의 이름은 여기서 검문받지 않는다(그게 옳다)★: 이 표에 없는 이름은 대조할 선언이 없어
///   `check_args` 가 통과시키고, 그 관용이 홉 간 additive 진화를 살린다(TRD §4-③ · 그 함수 doc).
///
/// ★blocking 함수다(호출자 계약)★: 이 표의 핸들러는 전부 blocking 이다 — resume 모드의 조기 종료를 약
///   3초 폴링하고, 이름 변경·계층 이동은 프로필 락을 쥔 채 디스크에 저장한다(agent `make_table` doc).
///   그래서 async 런타임 스레드가 아니라 blocking 풀에서 불러야 한다 — 제어 라우트는
///   `mcp_server::control_agent_handler` 의 `spawn_blocking`, 배달은 `command_delivery::run_locally` 가
///   그 자리다.
/// ★`entrance` 는 로그 라벨이다★ — 아래 계약 위반(async 핸들러 반입)은 오류 답장 하나로만 보이므로,
///   어느 표면에서 났는지가 로그에 없으면 원인을 못 찾는다.
/// ★입력 임대 검문도 여기 하나다(ADR-0231)★: [`INPUT_AFFECTING`] 명령은 인자 검문 다음에 `admit_input` 을
///   지난다. 세 표면이 다 이 함수를 지나므로 여기 말고 다른 자리(`DaemonLocalCommands::run` 등)에 두면
///   `/control/agent` 가 빠진다. `caller` = 호출한 연결(버스 = `Some(origin)` · CLI 두 라우트 = `None`).
// ADR-0155
// ADR-0157
// ADR-0231
pub fn call_daemon_command(
    table: &DaemonTable,
    name: &str,
    args: &mut serde_json::Value,
    entrance: &'static str,
    caller: Option<ConnId>,
) -> Option<Result<serde_json::Value, CommandError>> {
    // 표에 없는 이름은 **검문보다 먼저** 갈라낸다: `check_args` 는 모르는 이름을 통과시키므로(대조할
    //   선언이 이 표에 없다) 순서를 뒤집으면 남의 이름이 "인자 이상 없음" 을 지나 아래까지 온다.
    if !table.table.contains(name) {
        return None;
    }
    if let Err(rejection) = table.table.check_args(name, args) {
        return Some(Err(rejection));
    }
    if let Err(refusal) = admit_input(table, name, args, caller) {
        return Some(Err(refusal));
    }
    // 바로 위 `contains` 를 통과했고 표는 이 호출 동안 불변이라 `None` 갈래에 닿지 않는다.
    let future = table.table.call(name, args)?;
    Some(drive_to_completion(future, name, entrance))
}

/// 입력 영향 명령의 임대 검문 — 통과하면 `args.target` 을 **푼 id** 로 바꿔 둔다.
///
/// 순서: `target` 을 **한 번** 푼다(동사 본문과 같은 [`resolve_in`] · 같은 명부) → 그 id 로 임대를 본다 → 그
/// id 를 본문에 싣는다. ★마지막 걸음을 빼지 말 것★: 본문이 이름을 다시 풀면 그 사이의 개명 하나로 **검사한
/// 에이전트와 실행하는 에이전트가 달라진다**. id 는 해석기가 이름보다 먼저 정확 일치로 보므로 본문의 재해석은
/// 같은 에이전트이거나 부재다.
/// 해석 실패(부재 `NOT_FOUND` · 동명 `CONFLICT`)는 본문이 낼 것과 같은 답이라 여기서 그대로 돌려준다 —
/// 본문으로 흘리면 그 사이 생긴 같은 이름의 에이전트가 검문 없이 실행된다.
/// 거절 = `CONFLICT` + WS 와 **같은 글자**([`INPUT_LOCKED_REFUSAL`]) — 비보유자는 어느 입구로 와도 같은 답을 받는다.
/// 연결 없는 입구(`caller == None`)만 그 뒤에 [`CONNECTIONLESS_LEASE_TAIL`] 을 단다 — 머리는 그대로다.
/// ★`target` 이 문자열이 아니거나 공백뿐이면 손대지 않고 넘긴다★: 본문이 값으로 반려하고(타입 불일치 ·
///   빈 값) 아무것도 실행하지 않으며, 그 반려가 고칠 칸을 짚는다. 여기서 풀면 공백 이름이 「그런 에이전트
///   없음」으로 뭉개진다.
/// ★check-then-act 잔여★: 검사와 본문 사이에 다른 연결이 임대를 쥘 수 있다 — WS 쪽(`check_input` 뒤
///   manager 호출)과 같은 창이다.
// ADR-0231
fn admit_input(
    table: &DaemonTable,
    name: &str,
    args: &mut serde_json::Value,
    caller: Option<ConnId>,
) -> Result<(), CommandError> {
    if !INPUT_AFFECTING.contains(&name) {
        return Ok(());
    }
    let Some(token) = args
        .get("target")
        .and_then(serde_json::Value::as_str)
        .filter(|token| !token.trim().is_empty())
    else {
        return Ok(());
    };
    let agent = resolve_in(&table.roster.roster(), token)?;
    if !table.lease.permits(agent.id, caller) {
        // ★임대 전용 오류 코드를 새로 만들면 `command_delivery::retains_the_id` 의 「놓는 실패」에도 더할 것★ —
        //   안 그러면 임대가 풀린 뒤 같은 번호의 재시도가 `ALREADY_APPLIED` 로 삼켜진다(지금은 `CONFLICT` 라 놓인다).
        let refusal = match caller {
            Some(_) => INPUT_LOCKED_REFUSAL.to_string(),
            None => format!("{INPUT_LOCKED_REFUSAL}{CONNECTIONLESS_LEASE_TAIL}"),
        };
        return Err(CommandError::of(ErrorCode::Conflict, refusal));
    }
    args["target"] = serde_json::Value::String(agent.id.to_string());
    Ok(())
}

/// 연결 없는 입구(CLI 두 라우트)의 임대 거절 꼬리 — WS 문구의 「acquire first」는 연결이 있어야 칠 수 있는
/// 걸음이라, 이 입구의 호출자에게는 누가 쥐고 있고 무엇을 기다릴지를 말한다.
// ADR-0231
const CONNECTIONLESS_LEASE_TAIL: &str =
    " — a connected viewer holds this agent's input; retry after it releases";

/// 표가 준 future 를 **이 스레드에서** 끝낸다.
///
/// ★실행기를 두지 않는 근거★: 이 표의 핸들러는 전부 `blocking_handler` 라 본문이 **첫 poll 에서 끝까지
///   돈다**(도구 crate 가 그것을 계약으로 적었다). 부르는 쪽이 이미 blocking 풀 위에 있으므로 여기서 async
///   런타임을 다시 부르면 blocking 경계가 두 겹이 된다.
/// ★계약이 깨지면 조용히 성공하지 않는다★: 진짜 async 핸들러가 표에 들어오면 첫 poll 이 `Pending` 이고,
///   그때 이 자리는 답을 지어내는 대신 `OUTCOME_UNKNOWN` 으로 드러낸다.
/// ★`INTERNAL` 이 아니다★: 그 코드는 `retry: never` = **이 홉에서 확실히 실패했다**는 뜻인데, 첫 poll 이
///   이미 일의 일부를 적용했을 수 있다(그리고 폐기되는 future 는 나머지를 안 돌린다). 확실성은 「불명」이라
///   같은 request_id 로만 다시 묻게 해야 한다(TRD §4-④ · 도구 crate 의 전달 패닉이 같은 코드를 쓴다).
/// ★타입이 강제하지 않는 계약이라 계측한다★: 이 갈래는 표에 async 핸들러가 들어오는 순간에만 나고, 그때
///   증상은 오류 답장 하나뿐이라 로그가 없으면 원인을 못 찾는다.
fn drive_to_completion(
    future: CommandFuture,
    name: &str,
    entrance: &'static str,
) -> Result<serde_json::Value, CommandError> {
    future.now_or_never().unwrap_or_else(|| {
        tracing::error!(
            entrance,
            command = name,
            "명령이 첫 poll 에서 끝나지 않았다 — 이 입구는 blocking 핸들러만 몬다(표에 async 핸들러가 들어왔다)"
        );
        Err(CommandError::of(
            ErrorCode::OutcomeUnknown,
            format!(
                "'{name}' did not finish on its first poll — this entrance only drives blocking handlers, so part of it may already have been applied"
            ),
        ))
    })
}

/// 배달 1단계의 데몬 어댑터 — 표 슬롯을 **부를 때** 읽는다(포트가 요구하는 성질 = [`LocalCommands`] doc).
///
/// ★슬롯이 빈 것은 실패가 아니라 「내 명령 없음」이다★ — 표가 아직 안 꽂힌 조립(스모크 bin · 배선 순서상
/// 이른 시점)에서는 1단계가 미스가 되고, 그 이름은 다음 단계에서 명부를 탄다.
pub struct DaemonLocalCommands(Arc<CommandTableSlot>);

impl DaemonLocalCommands {
    pub fn new(table: Arc<CommandTableSlot>) -> Self {
        Self(table)
    }
}

impl LocalCommands for DaemonLocalCommands {
    /// ★`claim` 과 `run` 이 **같은 불변의 표**를 본다★: 슬롯은 `OnceLock` 이라 한 번 채워지면 안 바뀌므로
    /// 「내 것이라 해 놓고 빈손」이 이 어댑터에서는 날 수 없다. 포트가 그 상태를 계약 위반으로 규정한
    /// 근거가 이것이다([`LocalCommands::claim`]) — 다른 구현을 꽂을 때 이 성질을 함께 가져와야 한다.
    ///
    /// ★여기서 흘려보내는 `effect` 는 **광고용 칸이 아니라 정확성 재료**다★: 배달이 그 값으로 「이 번호를
    /// 붙들어 재실행을 막을까」를 가른다(`command_delivery` 의 `retains_the_id`). 그래서 **선언을 `Read` 로
    /// 적어 놓고 본문이 상태를 바꾸는 동사**는 이중 적용 보호를 조용히 잃는다 — 에러도 로그도 없다.
    /// ★그 어긋남은 여기서 못 잡는다★: 선언은 의도이고 본문은 행위라, 선언만 보고 본문을 검증할 방법이
    /// 없다. 동사를 더하는 사람이 `#[effect(...)]` 를 **광고 문구가 아니라 계약으로** 적는 수밖에 없다.
    fn claim(&self, name: &str) -> Option<Effect> {
        self.0
            .get()?
            .specs()
            .find(|spec| spec.name == name)
            .map(|spec| spec.effect)
    }

    /// ★입구 라벨과 호출자를 여기서 지어내지 않고 **받아서 넘긴다**★ — 이 어댑터를 부르는 입구는 둘이다:
    /// 소켓 디스패치(`bus` · 호출한 연결)와 전체 이름 라우트 `/control/call`(`cli` · 연결 없음 —
    /// `catalog::handle_call`). 상수를 박으면 두 입구의 사고가 한 이름표를 달고 적히고, 호출자를 박으면
    /// 임대 판정이 입구와 무관해진다. 아래 시험(`the_entrance_label_is_forwarded_not_hardcoded`)이 라벨
    /// forwarding 을 잰다.
    fn run(
        &self,
        name: &str,
        args: &mut serde_json::Value,
        entrance: &'static str,
        caller: Option<ConnId>,
    ) -> Option<Result<serde_json::Value, CommandError>> {
        call_daemon_command(self.0.get()?, name, args, entrance, caller)
    }

    fn decls(&self) -> Vec<CommandDecl> {
        self.0.get().map(|table| table.decls()).unwrap_or_default()
    }
}

/// 명부 통지 포트의 데몬 어댑터 — 슬롯을 **부를 때** 읽는다(사유는 [`make_daemon_table`] doc).
///
/// 슬롯이 빈 것은 실패가 아니라 통지 생략이다 — 붙을 클라이언트가 없는 조립이 실재한다
/// ([`RosterBroadcastSlot`]).
struct LateRosterBroadcast(Arc<RosterBroadcastSlot>);

impl RosterChanged for LateRosterBroadcast {
    fn roster_changed(&self) {
        match self.0.get() {
            Some(broadcast) => broadcast.roster_changed(),
            None => tracing::debug!(
                "명부 변경 통지 생략 — 팬아웃 포트 미설정(클라이언트가 붙을 수 없는 조립)"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use engram_dashboard_agent::preset::{Preset, PresetRegistry, PresetStore};
    use engram_dashboard_agent::profile::{AgentProfile, ProfileRegistry, ProfileStore};
    use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
    use engram_dashboard_agent::types::{
        AgentId, AgentInfo, AgentStatus, StatusSink, CLI_AGENT_VERBS,
    };
    use engram_dashboard_command::CommandError;
    use serde_json::json;

    use super::super::agent::RosterBroadcast;
    use super::*;

    #[derive(Default)]
    struct MemProfileStore {
        saved: Mutex<Vec<AgentProfile>>,
    }
    impl ProfileStore for MemProfileStore {
        fn save(&self, profiles: &[AgentProfile]) {
            *self.saved.lock().expect("store poisoned") = profiles.to_vec();
        }
        fn load(&self) -> Vec<AgentProfile> {
            self.saved.lock().expect("store poisoned").clone()
        }
    }

    struct NoPresets;
    impl PresetStore for NoPresets {
        fn save(&self, _presets: &[Preset]) {}
        fn load(&self) -> Vec<Preset> {
            Vec::new()
        }
    }

    struct NoopSink;
    impl StatusSink for NoopSink {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
        fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
    }

    #[derive(Default)]
    struct CountingBroadcast {
        calls: Mutex<usize>,
    }
    impl CountingBroadcast {
        fn calls(&self) -> usize {
            *self.calls.lock().expect("counter poisoned")
        }
    }
    impl RosterBroadcast for CountingBroadcast {
        fn roster_changed(&self) {
            *self.calls.lock().expect("counter poisoned") += 1;
        }
    }

    /// ★디스크도 PTY 도 없는 매니저★ — 표 하네스가 실물 프로세스를 딸고 오면 T-1 이 깨진 것이다.
    fn manager() -> Arc<AgentManager> {
        Arc::new(AgentManager::new(
            Arc::new(NoopSink),
            Arc::new(ProfileRegistry::new(Arc::new(MemProfileStore::default()))),
            Arc::new(PresetRegistry::new(Arc::new(NoPresets))),
            Arc::new(SessionTracker::new(
                TrackerConfig {
                    enabled: false,
                    poll_interval: Duration::from_secs(1),
                },
                Arc::new(|_, _| {}),
            )),
        ))
    }

    fn call(
        table: &DaemonTable,
        name: &str,
        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, CommandError> {
        call_daemon_command(table, name, &mut args, "test", None).expect("표에 있는 이름")
    }

    /// 명부에 항목을 하나 더하는 동사 — 통지가 걸리는 가장 싼 자리다(띄우지 않으므로 백엔드가 없다).
    fn register_one(table: &DaemonTable) {
        call(
            table,
            "agent.new",
            json!({ "cwd": "C:/work/probe", "backend": "Claude" }),
        )
        .expect("등록 성공");
    }

    /// ★CLI 동사 명단에서 기대값을 **파생**한다★ — 손으로 적으면 agent 에 동사가 늘어도 이 단언이 옛
    ///   명단을 그대로 통과시킨다. 선언 없이 늘어난 동사는 CLI 가 부를 수 없는 채로 남는다.
    /// ★CLI 동사가 없는 이름은 아래 목록에 적은 것뿐이다★ — 대기 목록 두 동사는 `engram agent <동사>` 에
    ///   올리지 않고(`CLI_AGENT_VERBS` 밖) 버스·`engram call` 로만 부른다. 목록에 없는 이름이 표에 늘면
    ///   CLI 에 올릴지를 정하지 않은 채 늘어난 것이라 여기서 멈춘다.
    // ADR-0231
    #[test]
    fn the_daemon_table_holds_every_cli_agent_verb() {
        const BUS_ONLY: [&str; 2] = ["agent.cancelQueuedInput", "agent.listQueuedInputs"];
        let table = make_daemon_table(
            manager(),
            Arc::new(RosterBroadcastSlot::new()),
            Arc::new(NoInputLeases),
        );

        let mut held: Vec<&str> = table.specs().map(|spec| spec.name).collect();
        held.sort_unstable();
        let mut wanted: Vec<String> = CLI_AGENT_VERBS
            .iter()
            .map(|verb| format!("agent.{verb}"))
            .chain(BUS_ONLY.iter().map(|name| name.to_string()))
            .collect();
        wanted.sort();
        assert_eq!(
            held, wanted,
            "표의 이름 = CLI 동사 + 버스 전용 이름(목록에 적은 것)이어야 한다"
        );
    }

    /// 붙을 클라이언트가 없는 조립(스모크 bin·격리 하네스)이 실재한다 — 그 조립에서 명부를 바꾸는
    /// 동사가 패닉하면 데몬 자체가 못 뜬다.
    #[test]
    fn a_missing_broadcast_slot_does_not_panic_the_table() {
        let table = make_daemon_table(
            manager(),
            Arc::new(RosterBroadcastSlot::new()),
            Arc::new(NoInputLeases),
        );

        register_one(&table);

        let out = call(&table, "agent.list", json!({})).expect("조회 성공");
        assert_eq!(
            out["agents"].as_array().map(Vec::len),
            Some(1),
            "통지가 없어도 등록 자체는 끝난다: {out}"
        );
    }

    // ── 두 표면의 공통 입구 ──────────────────────────────────────────────────────

    fn table_slot() -> Arc<CommandTableSlot> {
        let slot = Arc::new(CommandTableSlot::new());
        slot.set(Arc::new(make_daemon_table(
            manager(),
            Arc::new(RosterBroadcastSlot::new()),
            Arc::new(NoInputLeases),
        )));
        slot
    }

    /// ★두 표면이 **같은 검문**을 받는다★ — 한쪽만 검문하면 그쪽에서 막히는 오타가 다른 쪽으로는 통과해
    /// 조용히 다른 동작이 된다(ADR-0157). 같은 인자를 두 입구에 넣어 **같은 반려**가 나오는지 본다.
    #[test]
    fn the_control_route_and_the_bus_share_one_argument_check() {
        let slot = table_slot();
        let bus = DaemonLocalCommands::new(slot.clone());
        let table = slot.get().expect("표").clone();
        let typo = json!({ "cwdd": "C:/work/x" });

        let direct = call_daemon_command(&table, "agent.new", &mut typo.clone(), "cli", None)
            .expect("이 표의 이름")
            .expect_err("모르는 칸은 반려");
        let over_bus = bus
            .run("agent.new", &mut typo.clone(), "bus", Some(2))
            .expect("이 표의 이름")
            .expect_err("모르는 칸은 반려");

        assert_eq!(direct.code(), ErrorCode::InvalidArgument);
        assert_eq!(over_bus.code(), direct.code());
        assert_eq!(over_bus.message(), direct.message(), "같은 검문, 같은 문구");
        assert!(direct.message().contains("cwdd"), "{}", direct.message());
    }

    /// ★남의 이름은 **손대지 않고** 돌려준다★ — 미스에서 인자를 건드리면 그 봉투를 받을 진짜 주인이
    /// 반쪽짜리 인자를 받는다(홉 간 배선은 모르는 칸을 그대로 나른다 — TRD §4-③).
    #[test]
    fn a_name_this_daemon_does_not_hold_is_a_miss_that_leaves_the_arguments_alone() {
        let bus = DaemonLocalCommands::new(table_slot());
        let mut args = json!({ "window": "main", "unknown": 1 });

        assert!(bus.claim("tab.create").is_none());
        assert!(bus.run("tab.create", &mut args, "bus", Some(2)).is_none());
        assert_eq!(args, json!({ "window": "main", "unknown": 1 }));
    }

    /// 표가 아직 안 꽂힌 조립(스모크 bin · 배선 순서상 이른 시점)에서는 1단계가 통째로 미스다 —
    /// 패닉도 빈 답장도 아니다.
    #[test]
    fn an_unfilled_table_slot_is_a_stage_one_miss() {
        let bus = DaemonLocalCommands::new(Arc::new(CommandTableSlot::new()));

        assert!(bus.claim("agent.list").is_none());
        assert!(bus
            .run("agent.list", &mut json!({}), "bus", Some(2))
            .is_none());
        assert!(bus.decls().is_empty());
    }

    /// 발견 목록이 싣는 것은 **표에 실제로 꽂힌 것**이다 — 선언만 있고 안 꽂힌 이름을 광고하면 그 이름의
    /// 호출은 배달 1단계를 지나쳐 「모르는 명령」으로 되돌아온다.
    #[test]
    fn the_advertised_names_are_the_ones_the_table_actually_holds() {
        let slot = table_slot();
        let bus = DaemonLocalCommands::new(slot.clone());

        let mut advertised: Vec<String> = bus.decls().into_iter().map(|d| d.name).collect();
        advertised.sort();
        let mut held: Vec<String> = slot
            .get()
            .expect("표")
            .specs()
            .map(|spec| spec.name.to_string())
            .collect();
        held.sort();
        assert_eq!(advertised, held);
        assert!(advertised.iter().all(|name| bus.claim(name).is_some()));
    }

    // ── 입구 라벨 forwarding ─────────────────────────────────────────────────────

    /// 첫 poll 이 끝나지 않는 핸들러 — ★이 표의 blocking 계약을 **일부러** 어긴다★.
    ///
    /// 그 갈래만이 `entrance` 를 관측 가능한 곳으로 흘린다([`drive_to_completion`] 의 `error!`). 정상
    /// 핸들러로는 그 값이 어디에도 안 나와, 라벨을 상수로 박는 회귀가 아무 신호를 내지 않는다.
    struct NeverFinishes;

    impl engram_dashboard_command::CommandHandler for NeverFinishes {
        fn call(&self, _args: serde_json::Value) -> engram_dashboard_command::CommandFuture {
            Box::pin(std::future::pending())
        }
    }

    /// 시험 전용 선언 — `CommandTable::insert` 가 「이 crate 가 선언한 이름」만 받으므로 하나 세운다.
    static PENDING_SPEC: engram_dashboard_command::CommandSpec =
        engram_dashboard_command::CommandSpec {
            name: "fixture.pending",
            effect: Effect::Write,
            since: 1,
            summary: "첫 poll 에서 끝나지 않는 시험용 핸들러",
            args_schema: "{}",
            ok_schema: "{}",
            errors: &[],
            args_type: "FixturePendingArgs",
            ok_type: "FixturePendingOk",
        };

    static PENDING_DECLARED: &[&engram_dashboard_command::CommandSpec] = &[&PENDING_SPEC];

    fn pending_table_slot() -> Arc<CommandTableSlot> {
        let mut table = CommandTable::new(PENDING_DECLARED);
        table
            .insert(PENDING_SPEC.name, Arc::new(NeverFinishes))
            .expect("시험용 핸들러 삽입");
        let slot = Arc::new(CommandTableSlot::new());
        slot.set(Arc::new(DaemonTable {
            table,
            roster: manager(),
            lease: Arc::new(NoInputLeases),
        }));
        slot
    }

    /// ★어댑터는 받은 입구 라벨을 **그대로 넘긴다 — 상수를 박지 않는다**★
    ///
    /// ★이 시험이 필요한 이유★: 형제 시험들은 라벨을 인자로 넣고 결과만 보므로, `run` 이 인자를 버리고
    /// `"bus"` 를 박아도 전부 초록이다. 그래서 여기서는 **여러 값**을 넣고 그 값이 로그 필드에 그대로
    /// 도착하는지 본다 — 오늘 운영 값이 하나뿐이라(그 사실의 정본 = `LocalCommands::run` 구현 doc) 이
    /// forwarding 을 지켜 줄 다른 관측 표면이 없다.
    /// ★계약 위반 갈래를 태우는 것이 의도다★ — 그 갈래가 라벨을 밖으로 내는 유일한 자리이고, 그때
    /// 오류 코드가 `OUTCOME_UNKNOWN` 인 것도 함께 못박는다(`INTERNAL` 로 바뀌면 재질의 규약이 갈린다).
    #[test]
    fn the_entrance_label_is_forwarded_not_hardcoded() {
        for entrance in ["bus", "control", "a-third-door"] {
            let (code, lines) = crate::log_capture::capture_loud(|| {
                let adapter = DaemonLocalCommands::new(pending_table_slot());
                adapter
                    .run(PENDING_SPEC.name, &mut json!({}), entrance, None)
                    .expect("이 표의 이름")
                    .expect_err("첫 poll 이 안 끝나면 결말을 지어내지 않는다")
                    .code()
            });

            assert_eq!(code, ErrorCode::OutcomeUnknown);
            let wanted = format!("entrance=\"{entrance}\"");
            assert!(
                lines.iter().any(|line| line.contains(&wanted)),
                "받은 라벨이 그대로 적혀야 한다({entrance}): {lines:?}"
            );
        }
    }

    /// ★조립 순서가 통지를 죽이지 못한다는 것을 못박는다★: 슬롯이 **빈 채로** 표를 만들고 나중에
    ///   채운다 — 어댑터가 조립 시점의 값을 잡아 두는 형태로 바뀌면 여기서 0 이 나온다. 그 회귀는
    ///   런타임에 무신호라(에러도 로그도 없이 트리만 옛 명부를 보여 준다) 이 테스트 말고는 잡을 것이
    ///   없다.
    #[test]
    fn the_notifier_reads_the_slot_when_called_not_when_the_table_is_built() {
        let slot = Arc::new(RosterBroadcastSlot::new());
        let table = make_daemon_table(manager(), slot.clone(), Arc::new(NoInputLeases));

        let broadcast = Arc::new(CountingBroadcast::default());
        slot.set(broadcast.clone());
        register_one(&table);

        assert_eq!(
            broadcast.calls(),
            1,
            "표 조립 뒤에 채운 팬아웃도 통지를 받아야 한다"
        );
    }

    // ── 입력 임대 검문(ADR-0231) ─────────────────────────────────────────────────

    const CANCEL: &str = "agent.cancelQueuedInput";

    /// 한 연결이 한 에이전트의 임대를 쥔 것처럼 답하는 포트 — 물어 온 (에이전트, 호출자) 를 적는다.
    struct HeldBy {
        agent: AgentId,
        holder: ConnId,
        asked: Mutex<Vec<(AgentId, Option<ConnId>)>>,
    }

    impl HeldBy {
        fn new(agent: AgentId, holder: ConnId) -> Arc<Self> {
            Arc::new(Self {
                agent,
                holder,
                asked: Mutex::new(Vec::new()),
            })
        }

        fn asked(&self) -> Vec<(AgentId, Option<ConnId>)> {
            self.asked.lock().expect("lease poisoned").clone()
        }
    }

    impl InputLease for HeldBy {
        fn permits(&self, agent_id: AgentId, caller: Option<ConnId>) -> bool {
            self.asked
                .lock()
                .expect("lease poisoned")
                .push((agent_id, caller));
            agent_id != self.agent || caller == Some(self.holder)
        }
    }

    /// 취소 동사의 본문 자리에 서서 **받은 인자**를 적는다 — 검문이 본문에 무엇을 넘기는지 본다.
    struct RecordsArgs(Arc<Mutex<Vec<serde_json::Value>>>);

    impl engram_dashboard_command::CommandHandler for RecordsArgs {
        fn call(&self, args: serde_json::Value) -> CommandFuture {
            self.0.lock().expect("recorder poisoned").push(args);
            Box::pin(std::future::ready(Ok(json!({ "outcome": "requested" }))))
        }
    }

    /// `names` 를 명부에 등록한 매니저와 그 id 들 — 띄우지 않는다(검문은 명부만 본다).
    fn manager_with(names: &[&str]) -> (Arc<AgentManager>, Vec<AgentId>) {
        let manager = manager();
        let real = make_daemon_table(
            manager.clone(),
            Arc::new(RosterBroadcastSlot::new()),
            Arc::new(NoInputLeases),
        );
        let ids = names
            .iter()
            .map(|name| {
                let ok = call(
                    &real,
                    "agent.new",
                    json!({ "cwd": format!("C:/work/{name}"), "name": name, "backend": "Claude" }),
                )
                .expect("등록 성공");
                ok["agent_id"]
                    .as_str()
                    .and_then(|id| id.parse().ok())
                    .expect("id")
            })
            .collect();
        (manager, ids)
    }

    /// 취소 동사 자리에 기록 핸들러를 꽂은 표 — 선언은 agent 의 실물을 그대로 써서 인자 검문이 실물
    /// 스키마를 탄다. 명부는 실물 매니저다(해석기가 운영과 같은 명부를 본다).
    fn gated_slot(
        manager: Arc<AgentManager>,
        lease: Arc<dyn InputLease>,
    ) -> (Arc<CommandTableSlot>, Arc<Mutex<Vec<serde_json::Value>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut table = CommandTable::new(engram_dashboard_agent::commands::COMMAND_SPECS);
        table
            .insert(CANCEL, Arc::new(RecordsArgs(seen.clone())))
            .expect("선언된 이름");
        let slot = Arc::new(CommandTableSlot::new());
        slot.set(Arc::new(DaemonTable {
            table,
            roster: manager,
            lease,
        }));
        (slot, seen)
    }

    fn cancel_args(target: &str) -> serde_json::Value {
        json!({ "target": target, "input_id": "q1" })
    }

    fn command_args(value: serde_json::Value) -> super::super::agent::CommandArgs {
        serde_json::from_value(value).expect("인자 객체")
    }

    /// 제어 라우트의 봉투 → (코드, 문구). 성공이면 `("OK", payload)`.
    fn flatten(result: super::super::ingress::ControlQueryResult) -> (String, String) {
        match result {
            super::super::ingress::ControlQueryResult::Ok(payload) => {
                ("OK".to_string(), payload.to_string())
            }
            super::super::ingress::ControlQueryResult::Error { code, hint } => {
                (code.to_string(), hint)
            }
        }
    }

    /// 같은 취소를 **세 입구**로 낸다 — (버스 · `/control/call` · `/control/agent`) 순의 (코드, 문구).
    fn cancel_through_three_entrances(
        slot: &Arc<CommandTableSlot>,
        target: &str,
        bus_caller: ConnId,
    ) -> Vec<(String, String)> {
        let bus = DaemonLocalCommands::new(slot.clone());
        let over_bus = match bus
            .run(CANCEL, &mut cancel_args(target), "bus", Some(bus_caller))
            .expect("이 표의 이름")
        {
            Ok(payload) => ("OK".to_string(), payload.to_string()),
            Err(e) => (e.code().as_str().to_string(), e.message().to_string()),
        };
        let over_call = match super::super::catalog::handle_call(
            &bus,
            &crate::command_roster::CommandRoster::new(),
            super::super::catalog::CallRequest {
                name: CANCEL.to_string(),
                args: command_args(cancel_args(target)),
                request_id: None,
            },
        ) {
            super::super::catalog::CallOutcome::Answered(result) => flatten(result),
            super::super::catalog::CallOutcome::Relay { .. } => {
                panic!("데몬 자기 이름은 중계되지 않는다")
            }
        };
        let over_agent = flatten(super::super::agent::handle_agent(
            slot.get().expect("표"),
            super::super::agent::AgentRequest {
                verb: "cancelQueuedInput".to_string(),
                args: command_args(cancel_args(target)),
            },
        ));
        vec![over_bus, over_call, over_agent]
    }

    /// ★세 입구가 비보유자를 **같게** 거절한다★ — 검사 자리가 공통 입구 하나라서다. 하나라도 통과하면 그
    /// 입구로는 뷰어가 쥔 입력을 남이 흔든다. 버스는 호출한 연결을, 연결 없는 두 라우트는 `None` 을 싣는다 —
    /// `/control/call` 이 시행 14 의 길이다(TRD §7-1).
    #[test]
    fn the_three_entrances_refuse_a_non_holder_alike() {
        let (manager, ids) = manager_with(&["alpha"]);
        let alpha = ids[0];
        let lease = HeldBy::new(alpha, 7);
        let (slot, seen) = gated_slot(manager, lease.clone());

        let answers = cancel_through_three_entrances(&slot, "alpha", 2);

        for (code, text) in &answers {
            assert_eq!(code, "CONFLICT", "{answers:?}");
            assert!(
                text.starts_with(INPUT_LOCKED_REFUSAL),
                "WS 와 같은 거절 문구: {answers:?}"
            );
        }
        // 버스(연결 있음)는 WS 글자 그대로 · 연결 없는 둘은 기다릴 것을 말하는 꼬리를 달고, 명부 안내는 없다.
        assert_eq!(answers[0].1, INPUT_LOCKED_REFUSAL, "{answers:?}");
        for (_, text) in &answers[1..] {
            assert!(text.contains(CONNECTIONLESS_LEASE_TAIL), "{answers:?}");
            assert!(!text.contains("to see the roster"), "{answers:?}");
        }
        assert!(
            seen.lock().expect("recorder poisoned").is_empty(),
            "거절된 취소는 본문에 닿지 않는다"
        );
        assert_eq!(
            lease.asked(),
            vec![(alpha, Some(2)), (alpha, None), (alpha, None)],
            "임대는 푼 id 로 한 번씩 · 버스 = 호출한 연결 · CLI 두 라우트 = None"
        );
    }

    /// ★본문은 이름이 아니라 **검사한 id** 를 받는다★ — 본문이 이름을 다시 풀면 그 사이의 개명 하나로
    /// 검사한 에이전트와 취소하는 에이전트가 갈린다. 아무도 이 에이전트의 임대를 안 쥐었으면 `None`
    /// 호출자도 통과하고, 보유자는 자기 연결로 통과한다.
    #[test]
    fn the_verb_body_receives_the_resolved_id_from_every_entrance() {
        let (manager, ids) = manager_with(&["alpha", "beta"]);
        let (alpha, beta) = (ids[0], ids[1]);
        let (slot, seen) = gated_slot(manager, HeldBy::new(beta, 7));

        let answers = cancel_through_three_entrances(&slot, "alpha", 2);
        assert!(
            answers.iter().all(|(code, _)| code == "OK"),
            "이 에이전트의 임대는 아무도 안 쥐었다: {answers:?}"
        );
        let by_holder = DaemonLocalCommands::new(slot.clone())
            .run(CANCEL, &mut cancel_args("beta"), "bus", Some(7))
            .expect("이 표의 이름");
        assert!(by_holder.is_ok(), "보유자는 통과: {by_holder:?}");

        let seen = seen.lock().expect("recorder poisoned");
        let targets: Vec<&str> = seen
            .iter()
            .map(|args| args["target"].as_str().expect("문자열 target"))
            .collect();
        let (alpha, beta) = (alpha.to_string(), beta.to_string());
        assert_eq!(
            targets,
            vec![
                alpha.as_str(),
                alpha.as_str(),
                alpha.as_str(),
                beta.as_str()
            ]
        );
        assert!(
            seen.iter().all(|args| args["input_id"] == "q1"),
            "다른 칸은 손대지 않는다: {seen:?}"
        );
    }

    /// 풀리지 않는 지목은 **임대를 보기 전에** 본문과 같은 답(NOT_FOUND)으로 끝난다 — 본문으로 흘리면 그 사이
    /// 생긴 같은 이름의 에이전트가 검문 없이 실행된다.
    #[test]
    fn an_unresolved_target_is_answered_before_the_lease_is_consulted() {
        let (manager, ids) = manager_with(&["alpha"]);
        let lease = HeldBy::new(ids[0], 7);
        let (slot, seen) = gated_slot(manager, lease.clone());

        let err = DaemonLocalCommands::new(slot)
            .run(CANCEL, &mut cancel_args("ghost"), "bus", Some(2))
            .expect("이 표의 이름")
            .expect_err("없는 에이전트");

        assert_eq!(err.code(), ErrorCode::NotFound, "{}", err.message());
        assert!(lease.asked().is_empty(), "임대를 보지 않는다");
        assert!(seen.lock().expect("recorder poisoned").is_empty());
    }

    /// ★문자열이 아니거나 공백뿐인 `target` 은 동사가 값으로 반려한다★ — 검문이 손대지 않고 넘겨도 실물
    /// 본문이 아무것도 실행하지 않는다는 것을 실물 표로 잰다(검문이 풀면 공백이 「그런 에이전트 없음」으로
    /// 뭉개진다).
    #[test]
    fn a_blank_or_non_string_target_is_left_for_the_verb_to_refuse() {
        let (manager, ids) = manager_with(&["alpha"]);
        let lease = HeldBy::new(ids[0], 7);
        let table = make_daemon_table(manager, Arc::new(RosterBroadcastSlot::new()), lease.clone());

        for target in [json!("   "), json!(5)] {
            let err = call_daemon_command(
                &table,
                CANCEL,
                &mut json!({ "target": target, "input_id": "q1" }),
                "bus",
                Some(2),
            )
            .expect("이 표의 이름")
            .expect_err("반려");
            assert_eq!(
                err.code(),
                ErrorCode::InvalidArgument,
                "{target}: {}",
                err.message()
            );
        }
        assert!(lease.asked().is_empty(), "검문은 이 둘을 풀지 않는다");
    }

    /// ★조회와 다른 동사는 임대를 보지 않는다★ — 목록은 읽기이고, 명부 동사는 입력을 안 움직인다. 누가
    /// 임대를 쥐었어도 비보유자의 조회는 임대 거절이 아니라 동사 자신의 답(잠든 에이전트 = NOT_FOUND)을 받는다.
    #[test]
    fn listing_and_roster_verbs_do_not_consult_the_lease() {
        let (manager, ids) = manager_with(&["alpha"]);
        let lease = HeldBy::new(ids[0], 7);
        let table = make_daemon_table(manager, Arc::new(RosterBroadcastSlot::new()), lease.clone());

        let listed = call_daemon_command(
            &table,
            "agent.listQueuedInputs",
            &mut json!({ "target": "alpha" }),
            "cli",
            None,
        )
        .expect("이 표의 이름")
        .expect_err("잠든 에이전트는 목록을 안 쥔다");
        assert_eq!(listed.code(), ErrorCode::NotFound, "{}", listed.message());
        call_daemon_command(
            &table,
            "agent.rename",
            &mut json!({ "target": "alpha", "name": "gamma" }),
            "cli",
            None,
        )
        .expect("이 표의 이름")
        .expect("명부 동사는 임대와 무관");

        assert!(lease.asked().is_empty(), "{:?}", lease.asked());
    }

    /// ★검문은 입력 영향 명령이 **필수 문자열 `target`** 을 갖는다는 데 기대 선다★ — 그 칸이 선택이 되면
    /// 빠진 요청이 검문을 지나 본문에 닿고, 본문이 기본 대상을 고르는 순간 임대 없이 실행된다. 이름이 표에
    /// 없으면 검문이 아무것도 안 지킨다.
    #[test]
    fn every_input_affecting_command_is_held_and_takes_a_required_string_target() {
        let table = make_daemon_table(
            manager(),
            Arc::new(RosterBroadcastSlot::new()),
            Arc::new(NoInputLeases),
        );
        for name in INPUT_AFFECTING {
            let spec = table
                .specs()
                .find(|spec| spec.name == *name)
                .unwrap_or_else(|| panic!("표에 없는 입력 영향 명령: {name}"));
            let schema: serde_json::Value =
                serde_json::from_str(spec.args_schema).expect("선언 스키마");
            assert!(
                schema["required"]
                    .as_array()
                    .is_some_and(|required| required.iter().any(|f| f == "target")),
                "{name}: target 이 필수여야 한다 — {schema}"
            );
            assert_eq!(
                schema["properties"]["target"]["type"], "string",
                "{name}: target 은 문자열이어야 한다 — {schema}"
            );
        }
    }
}
