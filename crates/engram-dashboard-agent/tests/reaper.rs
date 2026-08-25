//! ③ reaper 종료 분류 통합테스트 — 실 셸 spawn 으로 ADR-0019 disposition 을 단언 검증.
//!
//! 셸을 띄우는 앞쪽 네 테스트도 단일 spawn(named-mutex/전역 경합 없음)이라 default 로 둔다.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_agent::backend::{AgentBackend, ShellBackend};
use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
use engram_dashboard_agent::persistence::{FilePresetStore, FileProfileStore};
use engram_dashboard_agent::preset::PresetRegistry;
use engram_dashboard_agent::profile::{AgentCommand, AgentProfile, ProfileRegistry, SpawnMode};
use engram_dashboard_agent::reaper::ReaperDeps;
use engram_dashboard_agent::session::AgentSession;
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::transport::api::ApiTransport;
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, ReapMsg, StatusSink, TerminalReason, TerminationIntent,
};

#[derive(Clone)]
struct CountingSink {
    list_updates: Arc<AtomicUsize>,
    statuses: Arc<Mutex<Vec<AgentStatus>>>,
}

impl CountingSink {
    fn new() -> Self {
        Self {
            list_updates: Arc::new(AtomicUsize::new(0)),
            statuses: Arc::new(Mutex::new(Vec::new())),
        }
    }
    fn list_update_count(&self) -> usize {
        self.list_updates.load(Ordering::SeqCst)
    }
}

impl StatusSink for CountingSink {
    fn status_changed(&self, _id: AgentId, status: AgentStatus, _epoch: u32) {
        self.statuses.lock().unwrap().push(status);
    }
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {
        self.list_updates.fetch_add(1, Ordering::SeqCst);
    }
}

fn wait_until<F: Fn() -> bool>(timeout: Duration, cond: F) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    cond()
}

fn make_manager(tag: &str) -> (AgentManager, CountingSink, Arc<ProfileRegistry>) {
    let sink = CountingSink::new();
    let sink_dyn: Arc<dyn StatusSink> = Arc::new(sink.clone());
    let store = Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-test-reaper-{tag}-{}", Uuid::new_v4())),
    ));
    let profiles = Arc::new(ProfileRegistry::new(store));
    let preset_store = Arc::new(FilePresetStore::new(std::env::temp_dir().join(format!(
        "engram-test-reaper-preset-{tag}-{}",
        Uuid::new_v4()
    ))));
    let presets = Arc::new(PresetRegistry::new(preset_store));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            sessions_dir: None,
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = AgentManager::new(sink_dyn, profiles.clone(), presets, tracker);
    (manager, sink, profiles)
}

#[cfg(windows)]
fn exit_profile(code: i32) -> AgentProfile {
    AgentProfile::new(
        "reaper-test".into(),
        AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), format!("exit {code}")],
        },
        PathBuf::from("."),
        vec![],
        false,
    )
}

#[cfg(not(windows))]
fn exit_profile(code: i32) -> AgentProfile {
    AgentProfile::new(
        "reaper-test".into(),
        AgentCommand::Shell {
            program: "sh".into(),
            args: vec!["-c".into(), format!("exit {code}")],
        },
        PathBuf::from("."),
        vec![],
        false,
    )
}

// ── 자연 종료(exit 0) ────────────────────────────────────────────────────────
#[test]
fn natural_exit_zero_reaps_and_keeps_profile_corpse() {
    let (manager, sink, profiles) = make_manager("exit0");
    let profile = exit_profile(0);
    let id = profile.id;

    let updates_before = sink.list_update_count();
    manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn failed")
        .into_started()
        .expect("이 호출은 실제로 띄운다(중복 요청 아님)");

    let removed = wait_until(Duration::from_secs(15), || manager.list_agents().is_empty());
    if !removed {
        let agents = manager.list_agents();
        eprintln!(
            "PROBE exit0 still present: {:?}",
            agents.iter().map(|a| &a.status).collect::<Vec<_>>()
        );
    }
    assert!(removed, "exit0: reaper 가 세션을 맵에서 제거하지 못함");
    assert!(
        wait_until(Duration::from_secs(2), || {
            profiles.get(id).map(|p| !p.auto_restore).unwrap_or(false)
        }),
        "exit0: 정상 종료 시체는 프로필 유지 + auto_restore=false 여야 함(ADR-0083 — 삭제 아님)"
    );
    assert!(
        profiles.get(id).is_some(),
        "exit0: 정상 종료인데 프로필이 삭제됨 — 시체로 보존돼야 함(ADR-0083)"
    );
    assert!(
        sink.list_update_count() > updates_before,
        "exit0: reaper 가 agent_list_updated 를 통지하지 않음"
    );
}

// ── 크래시(exit 1) ───────────────────────────────────────────────────────────
#[test]
fn crash_exit_one_keeps_profile_disables_auto_restore() {
    let (manager, _sink, profiles) = make_manager("exit1");
    let profile = exit_profile(1);
    let id = profile.id;

    manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn failed")
        .into_started()
        .expect("이 호출은 실제로 띄운다(중복 요청 아님)");

    assert!(
        wait_until(Duration::from_secs(5), || manager.list_agents().is_empty()),
        "exit1: reaper 가 세션을 제거하지 못함"
    );
    assert!(
        wait_until(Duration::from_secs(2), || {
            profiles.get(id).map(|p| !p.auto_restore).unwrap_or(false)
        }),
        "exit1: 크래시 후 프로필 유지 + auto_restore=false 가 아님(존재해야 하며 false 여야 함)"
    );
    assert!(
        profiles.get(id).is_some(),
        "exit1: 크래시인데 프로필이 삭제됨(유지돼야 함)"
    );
}

// ── 유저 kill ────────────────────────────────────────────────────────────────
#[test]
fn user_kill_keeps_profile_corpse_with_session_id() {
    let (manager, _sink, profiles) = make_manager("userkill");
    // 오래 사는 셸(즉시 종료 금지) — kill 로만 끝나게.
    let profile = AgentProfile::new(
        "reaper-kill".into(),
        AgentCommand::Shell {
            program: engram_dashboard_agent::manager::default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        true, // auto_restore=true 로 시작 → kill 수거가 false 로 다운그레이드하는지 단언 가능.
    );
    let id = profile.id;
    let sid = Uuid::new_v4();
    let mut seeded = profile.clone();
    seeded.claude_session_id = Some(sid);
    profiles.upsert(seeded.clone());

    let info = manager
        .spawn_agent(&seeded, SpawnMode::Fresh)
        .expect("spawn failed")
        .into_started()
        .expect("이 호출은 실제로 띄운다(중복 요청 아님)");

    assert!(
        wait_until(Duration::from_secs(2), || manager.list_agents().len() == 1),
        "userkill: spawn 직후 세션이 없음"
    );

    manager.kill_agent(info.id).expect("kill_agent failed");

    assert!(
        wait_until(Duration::from_secs(5), || manager.list_agents().is_empty()),
        "userkill: reaper 가 세션을 맵에서 제거하지 못함"
    );
    assert!(
        wait_until(Duration::from_secs(2), || {
            profiles.get(id).map(|p| !p.auto_restore).unwrap_or(false)
        }),
        "userkill: 유저 kill 시체는 프로필 유지 + auto_restore=false 여야 함(ADR-0083 — 삭제 아님)"
    );
    assert!(
        profiles.get(id).is_some(),
        "userkill: 유저 kill 인데 프로필이 삭제됨 — 시체로 보존돼야 함(ADR-0083)"
    );
    assert_eq!(
        profiles.get(id).and_then(|p| p.claude_session_id),
        Some(sid),
        "userkill: claude_session_id 가 유실됨 — 재활성화 resume 불가(ADR-0083 회귀)"
    );
}

// ── shutdown_all 중 종료 ─────────────────────────────────────────────────────
#[test]
fn shutdown_all_keeps_profiles_for_boot_restore() {
    let (manager, _sink, profiles) = make_manager("shutdown");
    let profile = AgentProfile::new(
        "reaper-shutdown".into(),
        AgentCommand::Shell {
            program: engram_dashboard_agent::manager::default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    let id = profile.id;
    manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn failed")
        .into_started()
        .expect("이 호출은 실제로 띄운다(중복 요청 아님)");
    assert!(
        wait_until(Duration::from_secs(2), || manager.list_agents().len() == 1),
        "shutdown: spawn 직후 세션이 없음"
    );

    assert!(
        profiles.get(id).map(|p| p.auto_restore).unwrap_or(false),
        "shutdown: spawn 후 auto_restore 가 true 가 아님(활성화 규칙)"
    );

    manager.shutdown_all();

    assert!(
        wait_until(Duration::from_secs(5), || manager.list_agents().is_empty()),
        "shutdown: 세션 맵 제거 실패"
    );
    let p = profiles
        .get(id)
        .expect("shutdown: 프로필이 삭제됨(유지돼야 함)");
    assert!(
        p.auto_restore,
        "shutdown: auto_restore 가 false 로 떨어짐 — 부팅 복원 대상에서 탈락(KeepAsIs 위반)"
    );
}

// ── 결정적 reap_one 단언(타이밍 무관) ─────────────────────────────────────────────

fn make_reaper_deps(tag: &str) -> (Arc<ProfileRegistry>, CountingSink, ReaperDeps) {
    let sink = CountingSink::new();
    let sink_dyn: Arc<dyn StatusSink> = Arc::new(sink.clone());
    let store = Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-test-reaper-{tag}-{}", Uuid::new_v4())),
    ));
    let profiles = Arc::new(ProfileRegistry::new(store));
    let sessions: Arc<RwLock<HashMap<AgentId, Arc<AgentSession>>>> =
        Arc::new(RwLock::new(HashMap::new()));
    let deps = ReaperDeps {
        sessions,
        profiles: profiles.clone(),
        status_sink: sink_dyn,
        control: Arc::new(engram_dashboard_agent::types::NoopControlChannel),
    };
    (profiles, sink, deps)
}

/// ApiTransport(껍데기)를 끼워 실 자원·pump 없이 맵에 넣는 세션 — start/kill 을 부르지 않는다.
fn make_test_session(
    id: AgentId,
    epoch: u32,
    status_sink: Arc<dyn StatusSink>,
) -> Arc<AgentSession> {
    let core = Arc::new(OutputCore::new(
        id,
        epoch,
        status_sink,
        TurnWiring::detached(),
    ));
    let intent = Arc::new(AtomicU8::new(TerminationIntent::None as u8));
    // ApiTransport 라 caps 내용은 무관 — 합성 경로를 만족시키는 더미로 셸 caps 를 넣는다.
    let shell_cmd = engram_dashboard_agent::profile::AgentCommand::Shell {
        program: "cmd.exe".into(),
        args: vec![],
    };
    Arc::new(AgentSession::new(
        id,
        PathBuf::from("."),
        epoch,
        80,
        24,
        intent,
        ShellBackend.capabilities(&shell_cmd),
        // 이 테스트 세션은 write_input 을 안 쓰지만 생성자가 encoder 를 요구 → Raw 더미.
        engram_dashboard_agent::backend::InputEncoder::Raw,
        // ★리터럴로 쓰지 마라★: "산 세션 값 == 백엔드 파생값" 이 우편 자격 방어의 전제라, 하네스가
        //   실물과 반대값을 들면 그 전제가 인트리에서 먼저 깨진다. 같은 명령에서 파생시킨다.
        engram_dashboard_agent::backend::reads_messages(&shell_cmd),
        core,
        Box::new(ApiTransport::new()),
    ))
}

// ── epoch race ───────────────────────────────────────────────────────────────
#[test]
fn epoch_mismatch_does_not_reap_current_session() {
    let (profiles, sink, deps) = make_reaper_deps("epoch-race");
    let id = Uuid::new_v4();

    // epoch=1 = 재spawn 으로 표식이 갈린 "현재" 세션(값 자체엔 뜻이 없다 — 0 과 다르기만 하면 된다).
    let status_dyn: Arc<dyn StatusSink> = Arc::new(sink.clone());
    let session = make_test_session(id, 1, status_dyn);
    deps.sessions.write().unwrap().insert(id, session);

    // 프로필은 어느 처분이든 보존되므로 존재 여부론 구분이 안 된다 — auto_restore=true 로 두고
    //   잘못된 다운그레이드가 일어나는지로 판정한다.
    let mut profile = exit_profile(0);
    profile.id = id;
    profile.auto_restore = true;
    profiles.upsert(profile);

    let updates_before = sink.list_update_count();

    // 늦게 도착한 옛 epoch=0 의 유령 done.
    let stale = ReapMsg {
        id,
        epoch: 0,
        reason: TerminalReason::Exited { code: Some(0) },
        intent_at_finish: TerminationIntent::None,
        shutting_down_at_finish: false,
    };
    deps.reap_one(stale);

    assert!(
        deps.sessions.read().unwrap().contains_key(&id),
        "epoch race: epoch 불일치 done 이 현재(epoch=1) 세션을 잘못 제거함"
    );
    assert!(
        profiles.get(id).map(|p| p.auto_restore).unwrap_or(false),
        "epoch race: epoch 불일치인데 disposition(auto_restore 다운그레이드)이 적용됨"
    );
    assert_eq!(
        sink.list_update_count(),
        updates_before,
        "epoch race: epoch 불일치인데 agent_list_updated 통지가 발생함"
    );
}

// ── idempotency ──────────────────────────────────────────────────────────────
#[test]
fn duplicate_reap_processes_exactly_once() {
    let (profiles, sink, deps) = make_reaper_deps("idempotency");
    let id = Uuid::new_v4();

    let status_dyn: Arc<dyn StatusSink> = Arc::new(sink.clone());
    let session = make_test_session(id, 0, status_dyn);
    deps.sessions.write().unwrap().insert(id, session);

    // auto_restore=true 라야 1회차 다운그레이드가 관측된다.
    let mut profile = exit_profile(0);
    profile.id = id;
    profile.auto_restore = true;
    profiles.upsert(profile);

    let updates_before = sink.list_update_count();

    let done = ReapMsg {
        id,
        epoch: 0,
        reason: TerminalReason::Exited { code: Some(0) },
        intent_at_finish: TerminationIntent::None,
        shutting_down_at_finish: false,
    };

    deps.reap_one(done.clone());
    assert!(
        !deps.sessions.read().unwrap().contains_key(&id),
        "idempotency: 1회차에 세션이 맵에서 제거되지 않음"
    );
    assert!(
        profiles.get(id).map(|p| !p.auto_restore).unwrap_or(false),
        "idempotency: 1회차에 disposition(프로필 유지 + auto_restore=false)이 적용되지 않음"
    );
    assert_eq!(
        sink.list_update_count(),
        updates_before + 1,
        "idempotency: 1회차 통지가 정확히 1회가 아님"
    );

    // 2회차 no-op 의 실제 근거는 맵 조회 실패다 — epoch 검사 구간에서 그대로 return 한다.
    deps.reap_one(done);
    assert_eq!(
        sink.list_update_count(),
        updates_before + 1,
        "idempotency: 2회차 중복 reap 이 통지를 추가로 발생시킴(정확히 1회 위반)"
    );
    assert!(
        profiles.get(id).map(|p| !p.auto_restore).unwrap_or(false),
        "idempotency: 2회차에 프로필 상태가 흔들림(유지 + auto_restore=false 여야 함)"
    );
}

// ── ADR-0084 apply_disposition epoch-guard ───────────────────────────────────
//
// ★맵을 비워두고 reap_one 을 부르면 안 된다★ — epoch=0 세션을 맵에 남겨 sessions.remove 를
//   통과시켜야 apply_disposition 까지 도달한다. remove 가드는 **세션** epoch 를, disposition 가드는
//   **프로필** epoch 를 보므로, 세션은 맞추고 프로필만 1 로 올려 후자만 불일치시킨다.
#[test]
fn stale_disposition_does_not_downgrade_reactivated_live_session() {
    let (profiles, _sink, deps) = make_reaper_deps("disp-epoch-guard");
    let id = Uuid::new_v4();

    let status_dyn: Arc<dyn StatusSink> = Arc::new(_sink.clone());
    let dead = make_test_session(id, 0, status_dyn);
    deps.sessions.write().unwrap().insert(id, dead);

    // 재활성화가 일어난 산 세션을 모사 — auto_restore=true 라야 잘못된 강등이 드러난다.
    let mut profile = exit_profile(0);
    profile.id = id;
    profile.epoch = 1; // 재활성화로 화신 표식이 갈린 상태(reaped_epoch=0 과 불일치).
    profile.auto_restore = true;
    profiles.upsert(profile);

    // 옛 reap(reaped_epoch=0)이 뒤늦게 도착.
    let stale = ReapMsg {
        id,
        epoch: 0,
        reason: TerminalReason::Exited { code: Some(0) },
        intent_at_finish: TerminationIntent::None,
        shutting_down_at_finish: false,
    };
    deps.reap_one(stale);

    assert!(
        profiles.get(id).map(|p| p.auto_restore).unwrap_or(false),
        "ADR-0084: epoch 불일치(재활성화) stale reap 이 산 세션 auto_restore 를 강등하면 안 됨"
    );
}

// ── ADR-0084 대조군(epoch 일치) ───────────────────────────────────────────────
#[test]
fn matching_epoch_disposition_downgrades_as_before() {
    let (profiles, _sink, deps) = make_reaper_deps("disp-epoch-match");
    let id = Uuid::new_v4();

    let status_dyn: Arc<dyn StatusSink> = Arc::new(_sink.clone());
    let dead = make_test_session(id, 0, status_dyn);
    deps.sessions.write().unwrap().insert(id, dead);

    let mut profile = exit_profile(0);
    profile.id = id;
    profile.epoch = 0; // 재활성화 없음 → reaped_epoch=0 과 일치.
    profile.auto_restore = true;
    profiles.upsert(profile);

    let done = ReapMsg {
        id,
        epoch: 0,
        reason: TerminalReason::Exited { code: Some(0) },
        intent_at_finish: TerminationIntent::None,
        shutting_down_at_finish: false,
    };
    deps.reap_one(done);

    assert!(
        profiles.get(id).map(|p| !p.auto_restore).unwrap_or(false),
        "ADR-0084: epoch 일치 시 정상 종료는 auto_restore 를 false 로 다운그레이드해야 함(가드가 정상 경로를 막지 않음)"
    );
}
