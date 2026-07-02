//! ③ reaper 종료 분류 통합테스트 — 실 셸 spawn 으로 ADR-0019 disposition 을 단언 검증.
//!
//! 검증(TRD §테스트):
//!   - 자연 종료(cmd /c exit 0)   → 세션 맵에서 reap + 프로필 삭제 + agent-list-updated 통지.
//!   - 크래시(cmd /c exit 1)       → 프로필 유지 + auto_restore=false(예약 복귀).
//!   - 유저 kill                   → 프로필 삭제(intent 태깅 경로).
//!   - shutdown_all 중 종료        → 프로필 유지(disposition 스킵), 맵 제거는 됨.
//!
//! epoch race·idempotency 는 reaper 의 reap_one 로직 특성(epoch 검증 + remove Some 승자)이라
//! src/agent/reaper.rs 의 decide unit + 아래 user_kill/shutdown 시나리오가 함께 보장한다.
//! (실 PTY 로 두 done 을 인위 중복 발행하기는 불안정 — 단일 supervisor 가 직렬 소비하므로 구조상
//! remove Some 승자 1명만 disposition·통지하며, 이는 reap_one 의 `removed.is_none() return` 가
//! 보장한다.)
//!
//! 모두 단일 셸 spawn(named-mutex/전역 경합 없음)이라 default 로 둔다.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_core::agent::backend::{AgentBackend, ShellBackend};
use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::output_core::OutputCore;
use engram_dashboard_core::agent::profile::{
    AgentCommand, AgentProfile, ProfileRegistry, SpawnMode,
};
use engram_dashboard_core::agent::reaper::ReaperDeps;
use engram_dashboard_core::agent::session::AgentSession;
use engram_dashboard_core::agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_core::agent::transport::api::ApiTransport;
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo, AgentStatus, ReapMsg, StatusSink, TerminalReason, TerminationIntent,
};
use engram_dashboard_core::persistence::FileProfileStore;

/// agent_list_updated 호출 횟수만 세는 경량 status sink.
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

/// 테스트용 manager + (status sink, profiles) 구성. 세션 추적 비활성(shell).
fn make_manager(tag: &str) -> (AgentManager, CountingSink, Arc<ProfileRegistry>) {
    let sink = CountingSink::new();
    let sink_dyn: Arc<dyn StatusSink> = Arc::new(sink.clone());
    let store = Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-test-reaper-{tag}-{}", Uuid::new_v4())),
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
    let manager = AgentManager::new(sink_dyn, profiles.clone(), tracker);
    (manager, sink, profiles)
}

/// cmd /c exit <code> 로 즉시 종료하는 셸 프로필.
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

// ── 자연 종료(exit 0) → reap + 프로필 삭제 + 통지 ──────────────────────────────
#[test]
fn natural_exit_zero_reaps_and_deletes_profile() {
    let (manager, sink, profiles) = make_manager("exit0");
    let profile = exit_profile(0);
    let id = profile.id;

    let updates_before = sink.list_update_count();
    manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn failed");

    // 셸이 즉시 exit 0 → pump EOF → finish(Exited{0}) → hook → reaper.
    // 맵에서 제거되고 프로필이 삭제돼야 한다.
    let removed = wait_until(Duration::from_secs(15), || manager.list_agents().is_empty());
    if !removed {
        let agents = manager.list_agents();
        eprintln!(
            "PROBE exit0 still present: {:?}",
            agents.iter().map(|a| &a.status).collect::<Vec<_>>()
        );
    }
    assert!(removed, "exit0: reaper 가 세션을 제거하지 못함");
    assert!(
        wait_until(Duration::from_secs(2), || profiles.get(id).is_none()),
        "exit0: 정상 종료인데 프로필이 삭제되지 않음(DeleteProfile)"
    );
    // 통지가 최소 1회 더 발생(reaper 의 agent_list_updated).
    assert!(
        sink.list_update_count() > updates_before,
        "exit0: reaper 가 agent_list_updated 를 통지하지 않음"
    );
}

// ── 크래시(exit 1) → 프로필 유지 + auto_restore=false ─────────────────────────
#[test]
fn crash_exit_one_keeps_profile_disables_auto_restore() {
    let (manager, _sink, profiles) = make_manager("exit1");
    let profile = exit_profile(1);
    let id = profile.id;

    manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn failed");

    // ★결정성(blocker 1 수정 핵심)★: spawn 은 활성화이므로 auto_restore=true 플립이 **start_pump
    //   전에** 일어난다(manager.spawn_session 5.5단계). exit1 은 start_pump 직후 즉시 크래시(EOF→
    //   finish→reaper)지만, reaper 의 false 다운그레이드는 항상 그 true 플립 **이후**다(순서: 플립
    //   true → start_pump → reaper false). 따라서 즉시크래시여도 최종값은 타이밍 무관하게 false 로
    //   결정적이다(예전 구조: 플립이 start_pump 후라 reaper false 를 true 로 덮어쓰는 race 존재).
    //   reaper 가 KeepDisableAutoRestore 적용 → 프로필 유지 + auto_restore=false.
    assert!(
        wait_until(Duration::from_secs(5), || manager.list_agents().is_empty()),
        "exit1: reaper 가 세션을 제거하지 못함"
    );
    // 프로필 유지 AND auto_restore=false 를 한 술어로 단언(중간에 삭제되는 회귀까지 함께 차단).
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

// ── 유저 kill → 프로필 삭제(intent 태깅) ───────────────────────────────────────
#[test]
fn user_kill_deletes_profile() {
    let (manager, _sink, profiles) = make_manager("userkill");
    // 오래 사는 셸(즉시 종료 금지) — kill 로만 끝나게.
    let profile = AgentProfile::new(
        "reaper-kill".into(),
        AgentCommand::Shell {
            program: engram_dashboard_core::agent::manager::default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    let id = profile.id;
    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn failed");

    // 초기 프롬프트가 뜰 때까지(살아있음 확인) 잠깐 폴링.
    assert!(
        wait_until(Duration::from_secs(2), || manager.list_agents().len() == 1),
        "userkill: spawn 직후 세션이 없음"
    );

    manager.kill_agent(info.id).expect("kill_agent failed");

    // intent=UserKill 태깅 경로 → reaper 가 DeleteProfile.
    assert!(
        wait_until(Duration::from_secs(5), || manager.list_agents().is_empty()),
        "userkill: reaper 가 세션을 제거하지 못함"
    );
    assert!(
        wait_until(Duration::from_secs(2), || profiles.get(id).is_none()),
        "userkill: 유저 kill 인데 프로필이 삭제되지 않음"
    );
}

// ── shutdown_all 중 종료 → 프로필 유지(disposition 스킵), 맵 제거는 됨 ────────────
#[test]
fn shutdown_all_keeps_profiles_for_boot_restore() {
    let (manager, _sink, profiles) = make_manager("shutdown");
    let profile = AgentProfile::new(
        "reaper-shutdown".into(),
        AgentCommand::Shell {
            program: engram_dashboard_core::agent::manager::default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    let id = profile.id;
    manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn failed");
    assert!(
        wait_until(Duration::from_secs(2), || manager.list_agents().len() == 1),
        "shutdown: spawn 직후 세션이 없음"
    );

    // spawn 으로 auto_restore=true 가 됐는지 확인(부팅 복원 대상).
    assert!(
        profiles.get(id).map(|p| p.auto_restore).unwrap_or(false),
        "shutdown: spawn 후 auto_restore 가 true 가 아님(활성화 규칙)"
    );

    // shutdown_all: shutting_down=true 를 먼저 set 한 뒤 각 kill. finish hook 이 true 를 snapshot →
    // reaper 가 KeepAsIs(손 안 댐) → 프로필 유지 + auto_restore=true 잔류(부팅 복원).
    manager.shutdown_all();

    assert!(
        wait_until(Duration::from_secs(5), || manager.list_agents().is_empty()),
        "shutdown: 세션 맵 제거 실패"
    );
    // 프로필은 유지되고 auto_restore=true 그대로(다운그레이드 안 됨).
    let p = profiles
        .get(id)
        .expect("shutdown: 프로필이 삭제됨(유지돼야 함)");
    assert!(
        p.auto_restore,
        "shutdown: auto_restore 가 false 로 떨어짐 — 부팅 복원 대상에서 탈락(KeepAsIs 위반)"
    );
}

// ── 결정적 reap_one 단언(타이밍 무관) ─────────────────────────────────────────────
//
// 아래 두 테스트는 실 PTY/spawn 없이 sessions 맵을 직접 구성하고 ReapMsg 를 reap_one 에 직접
// 먹인다. epoch race·idempotency 는 reap_one 의 write-lock 구간(epoch 검증 + remove Some 승자)
// 특성이라, 맵 상태를 직접 만들어 호출하면 sleep 없이 결정적으로 단언된다(flaky 0).

/// 테스트용 reaper deps 한 벌. 맵·프로필·sink 를 모두 공유한 ReaperDeps 를 만든다.
/// status sink 통지(agent_list_updated) 횟수는 CountingSink 로 센다.
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
    };
    // sessions 맵은 deps.sessions 로 접근(ReaperDeps 필드 pub) — 중복 핸들 반환을 피해 타입 단순화.
    (profiles, sink, deps)
}

/// 주어진 id/epoch 로 PTY 없는 테스트 세션을 만든다. ApiTransport(no-op 껍데기)를 끼워
/// 실 자원·pump 없이 맵에 넣을 수 있는 AgentSession 을 구성한다(start/kill 미호출).
fn make_test_session(
    id: AgentId,
    epoch: u32,
    status_sink: Arc<dyn StatusSink>,
) -> Arc<AgentSession> {
    let core = Arc::new(OutputCore::new(id, epoch, status_sink));
    let intent = Arc::new(AtomicU8::new(TerminationIntent::None as u8));
    // ApiTransport(no-op)라 caps 내용은 무관 — 합성 경로를 만족시키는 더미로 셸 caps 주입.
    Arc::new(AgentSession::new(
        id,
        PathBuf::from("."),
        epoch,
        80,
        24,
        intent,
        // FIX 5: capabilities 는 이제 command 를 받는다(mode 별 caps). 이 더미 세션엔 셸 command 로 충분.
        ShellBackend.capabilities(
            &engram_dashboard_core::agent::profile::AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
        ),
        // 이 테스트 세션은 write_input 을 안 쓰지만 생성자가 encoder 를 요구 → Raw 더미.
        engram_dashboard_core::agent::backend::InputEncoder::Raw,
        core,
        Box::new(ApiTransport::new()),
    ))
}

// ── epoch race: 늦게 온 옛 epoch done 이 재spawn 된 현재(epoch bump) 세션을 제거 못 함 ──
#[test]
fn epoch_mismatch_does_not_reap_current_session() {
    let (profiles, sink, deps) = make_reaper_deps("epoch-race");
    let id = Uuid::new_v4();

    // 맵에 epoch=1 의 "현재" 세션을 직접 구성(재spawn 으로 bump 된 상태를 모사).
    let status_dyn: Arc<dyn StatusSink> = Arc::new(sink.clone());
    let session = make_test_session(id, 1, status_dyn);
    deps.sessions.write().unwrap().insert(id, session);

    // 현재 세션의 프로필도 등록(disposition 이 잘못 일어나면 사라질 대상).
    let mut profile = exit_profile(0);
    profile.id = id;
    profiles.upsert(profile);

    let updates_before = sink.list_update_count();

    // 늦게 도착한 옛 epoch=0 의 유령 done. 정상 종료(exit0)라 만약 처리되면 DeleteProfile 이 일어난다.
    let stale = ReapMsg {
        id,
        epoch: 0,
        reason: TerminalReason::Exited { code: Some(0) },
        intent_at_finish: TerminationIntent::None,
        shutting_down_at_finish: false,
    };
    deps.reap_one(stale);

    // (a) 현재 세션은 맵에 그대로 남는다(epoch 불일치 → remove 안 함).
    assert!(
        deps.sessions.read().unwrap().contains_key(&id),
        "epoch race: epoch 불일치 done 이 현재(epoch=1) 세션을 잘못 제거함"
    );
    // (b) disposition 미발생 — 프로필이 삭제되지 않았다.
    assert!(
        profiles.get(id).is_some(),
        "epoch race: epoch 불일치인데 disposition(DeleteProfile)이 적용됨"
    );
    // (b') 통지(agent_list_updated)도 안 일어났다.
    assert_eq!(
        sink.list_update_count(),
        updates_before,
        "epoch race: epoch 불일치인데 agent_list_updated 통지가 발생함"
    );
}

// ── idempotency: 같은 (id,epoch) done 을 두 번 reap → 정확히 1회만 처리 ──────────────
#[test]
fn duplicate_reap_processes_exactly_once() {
    let (profiles, sink, deps) = make_reaper_deps("idempotency");
    let id = Uuid::new_v4();

    let status_dyn: Arc<dyn StatusSink> = Arc::new(sink.clone());
    let session = make_test_session(id, 0, status_dyn);
    deps.sessions.write().unwrap().insert(id, session);

    let mut profile = exit_profile(0);
    profile.id = id;
    profiles.upsert(profile);

    let updates_before = sink.list_update_count();

    let done = ReapMsg {
        id,
        epoch: 0,
        reason: TerminalReason::Exited { code: Some(0) },
        intent_at_finish: TerminationIntent::None,
        shutting_down_at_finish: false,
    };

    // 1회차: remove Some 승자 → disposition(DeleteProfile) + 통지 1회.
    deps.reap_one(done.clone());
    assert!(
        !deps.sessions.read().unwrap().contains_key(&id),
        "idempotency: 1회차에 세션이 맵에서 제거되지 않음"
    );
    assert!(
        profiles.get(id).is_none(),
        "idempotency: 1회차에 DeleteProfile 이 적용되지 않음"
    );
    assert_eq!(
        sink.list_update_count(),
        updates_before + 1,
        "idempotency: 1회차 통지가 정확히 1회가 아님"
    );

    // 2회차: 같은 done 재투입 → 맵에 없으니 epoch 검사에서 return(no-op). 통지·disposition 추가 0.
    deps.reap_one(done);
    assert_eq!(
        sink.list_update_count(),
        updates_before + 1,
        "idempotency: 2회차 중복 reap 이 통지를 추가로 발생시킴(정확히 1회 위반)"
    );
    assert!(
        profiles.get(id).is_none(),
        "idempotency: 2회차에 프로필 상태가 흔들림"
    );
}
