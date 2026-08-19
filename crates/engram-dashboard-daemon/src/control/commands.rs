//! 데몬이 **직접 실행할 수 있는** 명령의 표 — 실물 의존이 들어오는 유일한 자리(규칙 T-1).
//!
//! ★이 표를 부르는 표면은 **셋**이고 검문은 **하나**다★: 계열 라우트(`/control/agent` — 이웃 `agent.rs`
//!   가 `{verb}` 를 `agent.<verb>` 로 찾는다) · 전체 이름 라우트(`/control/call` — 이웃 `catalog.rs`) ·
//!   명령 버스 배달의 1단계(`command_delivery`). 셋 다 [`call_daemon_command`] 하나로 들어오므로 입구
//!   검문(ADR-0157)을 건너뛰는 표면이 없다. 표가 슬롯에 안 꽂혀 있으면 두 라우트는 503 이고 배달은
//!   1단계 미스다.
//! ★선언은 여기 없다★ — `agent.*` 의 계약은 core 가 소유하고(ADR-0155 결정 1: 선언이 사는 곳이 곧
//!   주인이다) 이 파일은 그 선언에 데몬의 실물(매니저 · 명부 통지 팬아웃)을 꽂기만 한다. 데몬 자기
//!   명령(`mail.*`)이 생기면 그때 선언 블록이 이 crate 로 들어온다.
//!
//! 진입점: [`make_daemon_table`](조립) · [`call_daemon_command`](두 표면의 공통 입구) ·
//! [`DaemonLocalCommands`](배달 1단계 어댑터).
//!
//! tauri import 0(daemon crate).
// ADR-0155

use std::sync::Arc;

use engram_dashboard_command::{
    CommandDecl, CommandError, CommandFuture, CommandTable, Effect, ErrorCode,
};
use engram_dashboard_core::agent::commands::{make_table, RosterChanged};
use engram_dashboard_core::agent::manager::AgentManager;
use futures_util::FutureExt as _;

use crate::command_delivery::LocalCommands;

use super::mcp_server::{CommandTableSlot, RosterBroadcastSlot};

/// `agent.*` 표에 데몬 실물을 꽂는다.
///
/// ★blocking 계약이 그대로 딸려 온다★: 핸들러 본문은 프로필 락을 쥔 채 디스크를 쓰고 resume 조기
///   종료를 폴링한다(core `make_table` doc). 그래서 이 표는 [`call_daemon_command`] 로만 부르고, 그
///   호출은 blocking 풀 위에 있어야 한다.
/// ★`broadcast` 가 값이 아니라 **슬롯**인 이유(load-bearing)★: 명부 통지 팬아웃은 연결 레지스트리에서
///   파생돼 이 표보다 **뒤에** 조립된다. 조립 시점의 값을 받으면 그때 비어 있던 슬롯이 프로세스 수명
///   내내 "통지 없음" 으로 굳고, 증상은 에러도 로그도 없이 **명부를 바꿔도 트리가 옛 명부를 보여
///   주는 것**이다(core `RosterChanged` 포트 doc). 슬롯을 받으면 읽는 시점이 호출 때로 밀려 **표 조립
///   순서**는 그 증상을 만들 수 없다 — 인자 형태가 순서 규율을 산문 대신 타입으로 지고 있다.
///   ★단 이게 닫는 건 조립 순서뿐이다★: 서버가 뜨고 나서 이 슬롯이 채워지기 전에 도착한 변경 요청은
///   여전히 통지를 건너뛴 채 성공(ok)으로 응답한다 — 그 창은 슬롯을 늦게 채우는 조립이 있는 한 남는다.
// ADR-0155
pub fn make_daemon_table(
    manager: Arc<AgentManager>,
    broadcast: Arc<RosterBroadcastSlot>,
) -> CommandTable {
    make_table(manager, Arc::new(LateRosterBroadcast(broadcast)))
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
///   3초 폴링하고, 이름 변경·계층 이동은 프로필 락을 쥔 채 디스크에 저장한다(core `make_table` doc).
///   그래서 async 런타임 스레드가 아니라 blocking 풀에서 불러야 한다 — 제어 라우트는
///   `mcp_server::control_agent_handler` 의 `spawn_blocking`, 배달은 `command_delivery::run_locally` 가
///   그 자리다.
/// ★`entrance` 는 로그 라벨이다★ — 아래 계약 위반(async 핸들러 반입)은 오류 답장 하나로만 보이므로,
///   어느 표면에서 났는지가 로그에 없으면 원인을 못 찾는다.
// ADR-0155
// ADR-0157
pub fn call_daemon_command(
    table: &CommandTable,
    name: &str,
    args: &mut serde_json::Value,
    entrance: &'static str,
) -> Option<Result<serde_json::Value, CommandError>> {
    // 표에 없는 이름은 **검문보다 먼저** 갈라낸다: `check_args` 는 모르는 이름을 통과시키므로(대조할
    //   선언이 이 표에 없다) 순서를 뒤집으면 남의 이름이 "인자 이상 없음" 을 지나 아래까지 온다.
    if !table.contains(name) {
        return None;
    }
    if let Err(rejection) = table.check_args(name, args) {
        return Some(Err(rejection));
    }
    // 바로 위 `contains` 를 통과했고 표는 이 호출 동안 불변이라 `None` 갈래에 닿지 않는다.
    let future = table.call(name, args)?;
    Some(drive_to_completion(future, name, entrance))
}

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

    fn run(
        &self,
        name: &str,
        args: &mut serde_json::Value,
    ) -> Option<Result<serde_json::Value, CommandError>> {
        call_daemon_command(self.0.get()?, name, args, "bus")
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

    use engram_dashboard_command::testing::block_on;
    use engram_dashboard_command::CommandError;
    use engram_dashboard_core::agent::preset::{Preset, PresetRegistry, PresetStore};
    use engram_dashboard_core::agent::profile::{AgentProfile, ProfileRegistry, ProfileStore};
    use engram_dashboard_core::agent::session_tracker::{SessionTracker, TrackerConfig};
    use engram_dashboard_core::agent::types::{
        AgentId, AgentInfo, AgentStatus, StatusSink, CLI_AGENT_VERBS,
    };
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
                    sessions_dir: None,
                    enabled: false,
                    poll_interval: Duration::from_secs(1),
                },
                Arc::new(|_, _| {}),
            )),
        ))
    }

    fn call(
        table: &CommandTable,
        name: &str,
        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, CommandError> {
        let future = table.call(name, &mut args).expect("표에 있는 이름");
        block_on(future)
    }

    /// 명부에 항목을 하나 더하는 동사 — 통지가 걸리는 가장 싼 자리다(띄우지 않으므로 백엔드가 없다).
    fn register_one(table: &CommandTable) {
        call(table, "agent.new", json!({ "cwd": "C:/work/probe" })).expect("등록 성공");
    }

    /// ★CLI 동사 명단에서 기대값을 **파생**한다★ — 손으로 적으면 core 에 동사가 늘어도 이 단언이 옛
    ///   명단을 그대로 통과시킨다. 선언 없이 늘어난 동사는 CLI 가 부를 수 없는 채로 남는다.
    #[test]
    fn the_daemon_table_holds_every_cli_agent_verb() {
        let table = make_daemon_table(manager(), Arc::new(RosterBroadcastSlot::new()));

        let mut held: Vec<&str> = table.specs().map(|spec| spec.name).collect();
        held.sort_unstable();
        let mut wanted: Vec<String> = CLI_AGENT_VERBS
            .iter()
            .map(|verb| format!("agent.{verb}"))
            .collect();
        wanted.sort();
        assert_eq!(held, wanted, "CLI 동사와 표의 이름이 일대일이어야 한다");
    }

    /// 붙을 클라이언트가 없는 조립(스모크 bin·격리 하네스)이 실재한다 — 그 조립에서 명부를 바꾸는
    /// 동사가 패닉하면 데몬 자체가 못 뜬다.
    #[test]
    fn a_missing_broadcast_slot_does_not_panic_the_table() {
        let table = make_daemon_table(manager(), Arc::new(RosterBroadcastSlot::new()));

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

        let direct = call_daemon_command(&table, "agent.new", &mut typo.clone(), "cli")
            .expect("이 표의 이름")
            .expect_err("모르는 칸은 반려");
        let over_bus = bus
            .run("agent.new", &mut typo.clone())
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
        assert!(bus.run("tab.create", &mut args).is_none());
        assert_eq!(args, json!({ "window": "main", "unknown": 1 }));
    }

    /// 표가 아직 안 꽂힌 조립(스모크 bin · 배선 순서상 이른 시점)에서는 1단계가 통째로 미스다 —
    /// 패닉도 빈 답장도 아니다.
    #[test]
    fn an_unfilled_table_slot_is_a_stage_one_miss() {
        let bus = DaemonLocalCommands::new(Arc::new(CommandTableSlot::new()));

        assert!(bus.claim("agent.list").is_none());
        assert!(bus.run("agent.list", &mut json!({})).is_none());
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

    /// ★조립 순서가 통지를 죽이지 못한다는 것을 못박는다★: 슬롯이 **빈 채로** 표를 만들고 나중에
    ///   채운다 — 어댑터가 조립 시점의 값을 잡아 두는 형태로 바뀌면 여기서 0 이 나온다. 그 회귀는
    ///   런타임에 무신호라(에러도 로그도 없이 트리만 옛 명부를 보여 준다) 이 테스트 말고는 잡을 것이
    ///   없다.
    #[test]
    fn the_notifier_reads_the_slot_when_called_not_when_the_table_is_built() {
        let slot = Arc::new(RosterBroadcastSlot::new());
        let table = make_daemon_table(manager(), slot.clone());

        let broadcast = Arc::new(CountingBroadcast::default());
        slot.set(broadcast.clone());
        register_one(&table);

        assert_eq!(
            broadcast.calls(),
            1,
            "표 조립 뒤에 채운 팬아웃도 통지를 받아야 한다"
        );
    }
}
