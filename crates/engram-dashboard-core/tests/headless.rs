//! ② 격리 통합테스트 — AgentManager 전체 흐름을 실 셸 spawn 으로 단언 검증.
//!
//! (구 examples/headless.rs 이관 — "로그 eyeball" 을 RecordingSink 기반 명시 단언으로 전환.)
//!
//! 검증 기준(구 주석에서 단언으로 이전):
//!   spawn(실 셸) → subscribe → 일정 시간 내 PTY out 1개 이상 수신 → write(echo) →
//!   resize 성공 → kill → status 가 종점 Killed 도달 → kill 후 list count=0 →
//!   kill→list 가 타임아웃(5s) 내 완료(hang 없음).
//!
//! 실 OS 프로세스(default shell)를 spawn 한다. 가볍고 named-mutex/전역 경합 없는 단일
//! spawn 이라 default(자동 실행)로 둔다 — `cargo test -p engram-dashboard-core` 에 잡힌다.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_core::persistence::FileProfileStore;
use engram_dashboard_core::pty::manager::{default_shell, AgentManager};
use engram_dashboard_core::pty::profile::{AgentCommand, AgentProfile, ProfileRegistry, SpawnMode};
use engram_dashboard_core::pty::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_core::pty::types::{
    AgentId, AgentInfo, AgentStatus, OutputFrame, OutputSink, SinkError, SinkId, StatusSink,
};

// ── RecordingSink ────────────────────────────────────────────────────────────
// OutputSink + StatusSink 양쪽을 구현하는 기록형 테스트 sink.
// 로그(eyeball) 대신 받은 출력 바이트와 status 전이를 Mutex<Vec<..>> 에 push 해 단언에 쓴다.

#[derive(Clone)]
struct RecordingSink {
    id: SinkId,
    /// 수신한 PTY 출력 바이트 누적(전 프레임 concat). echo substring 검색용.
    output: Arc<Mutex<Vec<u8>>>,
    /// 수신한 status 전이 순서. 종점/순서 단언용.
    statuses: Arc<Mutex<Vec<AgentStatus>>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            output: Arc::new(Mutex::new(Vec::new())),
            statuses: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn output_len(&self) -> usize {
        self.output.lock().unwrap().len()
    }

    fn output_contains(&self, needle: &str) -> bool {
        let buf = self.output.lock().unwrap();
        let text = String::from_utf8_lossy(&buf);
        text.contains(needle)
    }

    fn statuses(&self) -> Vec<AgentStatus> {
        self.statuses.lock().unwrap().clone()
    }
}

impl OutputSink for RecordingSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        // S12 raw 경계화: frame.data 는 이미 raw 바이트.
        self.output.lock().unwrap().extend_from_slice(frame.data);
        Ok(())
    }

    fn sink_id(&self) -> SinkId {
        self.id
    }
}

impl StatusSink for RecordingSink {
    fn status_changed(&self, _id: AgentId, status: AgentStatus, _epoch: u32) {
        self.statuses.lock().unwrap().push(status);
    }

    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

/// 조건이 참이 될 때까지 짧게 폴링(최대 `timeout`). 실 PTY 출력은 비동기라 고정 sleep 대신
/// 조건 폴링으로 빠르고 안정적으로 기다린다.
fn wait_until<F: Fn() -> bool>(timeout: Duration, cond: F) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    cond()
}

#[test]
fn manager_spawn_write_resize_kill() {
    let status_sink = RecordingSink::new();
    let status_dyn: Arc<dyn StatusSink> = Arc::new(status_sink.clone());

    // 프로필 영속화는 임시 디렉토리(테스트별 unique), 세션 추적 비활성(shell 이라 세션 파일 없음).
    let store = Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-test-headless-{}", Uuid::new_v4())),
    ));
    let profiles = Arc::new(ProfileRegistry::new(store));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            sessions_dir: None,
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = AgentManager::new(status_dyn, profiles, tracker);

    // 1) spawn — 기본 셸(Fresh). 생성 직후 Running.
    let profile = AgentProfile::new(
        "headless".into(),
        AgentCommand::Shell {
            program: default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn failed");

    // spawn 직후 목록에 1개.
    assert_eq!(
        manager.list_agents().len(),
        1,
        "spawn 후 에이전트 1개여야 함"
    );

    // 2) subscribe — 이후 PTY 출력이 RecordingSink.send 로 흘러온다.
    let out_sink = RecordingSink::new();
    let _sid = manager
        .subscribe(info.id, Arc::new(out_sink.clone()))
        .expect("subscribe failed");

    // 3) 일정 시간(2s) 내 PTY 출력 1개 이상 수신(초기 프롬프트). eyeball → 단언.
    let got_output = wait_until(Duration::from_secs(2), || out_sink.output_len() > 0);
    assert!(got_output, "2s 내 PTY 초기 출력을 수신하지 못함");

    // 4) stdin write — echo 결과가 출력에 보여야 함(셸 에코 또는 명령 실행 출력).
    manager
        .write_stdin(info.id, b"echo headless-test\r\n")
        .expect("write_stdin failed");
    let echoed = wait_until(Duration::from_secs(3), || {
        out_sink.output_contains("headless-test")
    });
    assert!(
        echoed,
        "echo 입력이 PTY 출력에 반영되지 않음(headless-test 미수신)"
    );

    // 5) resize — 성공해야 함.
    manager.resize(info.id, 100, 30).expect("resize failed");

    // 6) kill → 7) list count=0 가 타임아웃(5s) 내 완료되어야 함(hang 없음).
    //    kill_agent 자체가 내부 join(5s) 후 sessions 맵에서 제거한다.
    let kill_started = Instant::now();
    manager.kill_agent(info.id).expect("kill_agent failed");
    let remaining = manager.list_agents().len();
    let kill_elapsed = kill_started.elapsed();

    assert_eq!(
        remaining, 0,
        "kill 후 세션이 남아있음 — sessions 맵 제거 실패"
    );
    assert!(
        kill_elapsed < Duration::from_secs(5),
        "kill→list 가 5s 안에 끝나지 않음(hang 의심): {kill_elapsed:?}"
    );

    // 8) status 가 종점 Killed 에 도달해야 함. 전이는 status_sink 에 기록됨.
    //    종점 알림은 pump 단독(ADR-0005)이라 약간 비동기일 수 있어 폴링.
    let reached_killed = wait_until(Duration::from_secs(2), || {
        matches!(status_sink.statuses().last(), Some(AgentStatus::Killed))
    });
    let seq = status_sink.statuses();
    assert!(
        reached_killed,
        "status 종점 Killed 미도달 — 전이 기록: {seq:?}"
    );
    // 종점이 Killed 이고, 그 전에 Exiting 과도기를 거쳤어야 함(kill 인과: Exiting→Killed).
    assert!(
        seq.iter().any(|s| matches!(s, AgentStatus::Exiting)),
        "kill 전이에 Exiting 과도기가 없음: {seq:?}"
    );
}
