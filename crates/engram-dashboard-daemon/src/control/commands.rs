//! 데몬이 **직접 실행할 수 있는** 명령의 표 — 실물 의존이 들어오는 유일한 자리(규칙 T-1).
//!
//! ★이 표가 `/control/agent` 의 배달 대상이다★: 이웃 `agent.rs` 가 `{verb}` 를 `agent.<verb>` 로 찾아
//!   여기 꽂힌 핸들러를 부른다. 표가 슬롯에 안 꽂혀 있으면 그 라우트는 503 이다.
//! ★선언은 여기 없다★ — `agent.*` 의 계약은 core 가 소유하고(ADR-0134 결정 1: 선언이 사는 곳이 곧
//!   주인이다) 이 파일은 그 선언에 데몬의 실물(매니저 · 명부 통지 팬아웃)을 꽂기만 한다. 데몬 자기
//!   명령(`mail.*`)이 생기면 그때 선언 블록이 이 crate 로 들어온다.
//!
//! 진입점: [`make_daemon_table`].
//!
//! tauri import 0(daemon crate).
// ADR-0134

use std::sync::Arc;

use engram_dashboard_command::CommandTable;
use engram_dashboard_core::agent::commands::{make_table, RosterChanged};
use engram_dashboard_core::agent::manager::AgentManager;

use super::mcp_server::RosterBroadcastSlot;

/// `agent.*` 표에 데몬 실물을 꽂는다.
///
/// ★blocking 계약이 그대로 딸려 온다★: 핸들러 본문은 프로필 락을 쥔 채 디스크를 쓰고 resume 조기
///   종료를 폴링한다(core `make_table` doc). 이 표를 부르는 어댑터는 async 런타임 스레드가 아니라
///   blocking 풀에서 불러야 한다.
/// ★`broadcast` 가 값이 아니라 **슬롯**인 이유(load-bearing)★: 명부 통지 팬아웃은 연결 레지스트리에서
///   파생돼 이 표보다 **뒤에** 조립된다. 조립 시점의 값을 받으면 그때 비어 있던 슬롯이 프로세스 수명
///   내내 "통지 없음" 으로 굳고, 증상은 에러도 로그도 없이 **명부를 바꿔도 트리가 옛 명부를 보여
///   주는 것**이다(core `RosterChanged` 포트 doc). 슬롯을 받으면 읽는 시점이 호출 때로 밀려 **표 조립
///   순서**는 그 증상을 만들 수 없다 — 인자 형태가 순서 규율을 산문 대신 타입으로 지고 있다.
///   ★단 이게 닫는 건 조립 순서뿐이다★: 서버가 뜨고 나서 이 슬롯이 채워지기 전에 도착한 변경 요청은
///   여전히 통지를 건너뛴 채 성공(ok)으로 응답한다 — 그 창은 슬롯을 늦게 채우는 조립이 있는 한 남는다.
// ADR-0134
pub fn make_daemon_table(
    manager: Arc<AgentManager>,
    broadcast: Arc<RosterBroadcastSlot>,
) -> CommandTable {
    make_table(manager, Arc::new(LateRosterBroadcast(broadcast)))
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
