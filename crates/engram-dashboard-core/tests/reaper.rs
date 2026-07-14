//! ③ reaper 종료 분류 통합테스트 — 실 셸 spawn 으로 ADR-0019 disposition 을 단언 검증.
//!
//! 검증(TRD §테스트, ADR-0083 개정 — 자동 삭제 폐지: 모든 종료는 시체 보존):
//!   - 자연 종료(cmd /c exit 0)   → 세션 맵에서 reap + 프로필 시체 보존(auto_restore=false) + 통지.
//!   - 크래시(cmd /c exit 1)       → 프로필 유지 + auto_restore=false(예약 복귀).
//!   - 유저 kill                   → 세션 수거 + 프로필 시체 보존(claude_session_id 유지, ADR-0083).
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
use engram_dashboard_core::agent::preset::PresetRegistry;
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
use engram_dashboard_core::persistence::{FilePresetStore, FileProfileStore};

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
    // ADR-0061: 프리셋 레지스트리(reaper 무관 — 빈 상태). 임시 디렉토리 store 로 배선.
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

// ── 자연 종료(exit 0) → reap(맵 제거) + 프로필 시체 보존(auto_restore=false) + 통지 ─────────
// ADR-0083: 정상 exit(code0) 도 삭제 아님. 세션은 맵에서 수거하되 프로필은 시체로 보존한다.
#[test]
fn natural_exit_zero_reaps_and_keeps_profile_corpse() {
    let (manager, sink, profiles) = make_manager("exit0");
    let profile = exit_profile(0);
    let id = profile.id;

    let updates_before = sink.list_update_count();
    manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn failed");

    // 셸이 즉시 exit 0 → pump EOF → finish(Exited{0}) → hook → reaper.
    // 세션은 맵에서 제거되지만(수거) 프로필은 시체로 보존돼야 한다(ADR-0083).
    let removed = wait_until(Duration::from_secs(15), || manager.list_agents().is_empty());
    if !removed {
        let agents = manager.list_agents();
        eprintln!(
            "PROBE exit0 still present: {:?}",
            agents.iter().map(|a| &a.status).collect::<Vec<_>>()
        );
    }
    assert!(removed, "exit0: reaper 가 세션을 맵에서 제거하지 못함");
    // ADR-0083: 프로필 유지 + auto_restore=false 로 다운그레이드(시체 보존, 삭제 아님).
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

// ── 유저 kill → 세션은 맵에서 수거, 프로필은 시체 보존(ADR-0083) ─────────────────────
// ADR-0083(유저 실측 버그 수정): 유저 kill 후 우클릭 재활성화가 "실패"(profile not found)로 깨지던
// 원인 = reaper 가 UserKill → DeleteProfile → profiles.remove(claude_session_id 포함 삭제)였다.
// 이제 유저 kill 도 시체 보존 — 세션만 맵에서 수거하고 프로필 + claude_session_id 는 남겨 재활성화
// resume 가 가능하게 한다. auto_restore 는 false 로 다운그레이드(부팅 자동복원 대상에서만 제외).
#[test]
fn user_kill_keeps_profile_corpse_with_session_id() {
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
        true, // auto_restore=true 로 시작 → kill 수거가 false 로 다운그레이드하는지 단언 가능.
    );
    let id = profile.id;
    // claude_session_id 를 심어둔다 — 유저 kill 후에도 살아남아 재활성화 resume 가능함을 단언한다.
    // ★spawn 은 넘긴 프로필을 upsert_preserving_hierarchy 로 그대로 심으므로(session_id 포함), sid 를
    //   심은 seeded 프로필로 spawn 해야 유실되지 않는다★(원본 profile 은 claude_session_id=None).
    let sid = Uuid::new_v4();
    let mut seeded = profile.clone();
    seeded.claude_session_id = Some(sid);
    profiles.upsert(seeded.clone());

    let info = manager
        .spawn_agent(&seeded, SpawnMode::Fresh)
        .expect("spawn failed");

    // 초기 프롬프트가 뜰 때까지(살아있음 확인) 잠깐 폴링.
    assert!(
        wait_until(Duration::from_secs(2), || manager.list_agents().len() == 1),
        "userkill: spawn 직후 세션이 없음"
    );

    manager.kill_agent(info.id).expect("kill_agent failed");

    // (1) 세션은 맵에서 수거된다(이 부분은 ADR-0083 로도 불변).
    assert!(
        wait_until(Duration::from_secs(5), || manager.list_agents().is_empty()),
        "userkill: reaper 가 세션을 맵에서 제거하지 못함"
    );
    // (2) ADR-0083: 프로필 유지 + auto_restore=false 다운그레이드(시체 보존).
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
    // (3) ★재활성화 resume 성립 조건★: claude_session_id 가 그대로 남아야 --resume 로 이어받는다.
    assert_eq!(
        profiles.get(id).and_then(|p| p.claude_session_id),
        Some(sid),
        "userkill: claude_session_id 가 유실됨 — 재활성화 resume 불가(ADR-0083 회귀)"
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

    // 현재 세션의 프로필도 등록. auto_restore=true 로 둬서 "잘못된 disposition = false 다운그레이드"를
    // 검출 가능하게 한다(ADR-0083: 처분은 삭제가 아니라 다운그레이드이므로 존재 여부론 구분 불가).
    let mut profile = exit_profile(0);
    profile.id = id;
    profile.auto_restore = true;
    profiles.upsert(profile);

    let updates_before = sink.list_update_count();

    // 늦게 도착한 옛 epoch=0 의 유령 done. 만약 잘못 처리되면 disposition(auto_restore 다운그레이드)
    // + 통지가 일어난다 — epoch 불일치로 remove 전에 return 돼 둘 다 안 일어나야 한다.
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
    // (b) disposition 미발생 — auto_restore 다운그레이드가 일어나지 않아 true 그대로여야 한다.
    //     (프로필은 어느 처분이든 보존되므로 존재 여부론 구분 불가 → auto_restore 로 판정. ADR-0083.)
    assert!(
        profiles.get(id).map(|p| p.auto_restore).unwrap_or(false),
        "epoch race: epoch 불일치인데 disposition(auto_restore 다운그레이드)이 적용됨"
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

    // auto_restore=true 로 둬서 1회차 disposition(false 다운그레이드)이 실제로 관측되게 한다.
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

    // 1회차: remove Some 승자 → disposition(ADR-0083: 시체 보존 + auto_restore 다운그레이드) + 통지 1회.
    deps.reap_one(done.clone());
    assert!(
        !deps.sessions.read().unwrap().contains_key(&id),
        "idempotency: 1회차에 세션이 맵에서 제거되지 않음"
    );
    // ADR-0083: 삭제가 아니라 시체 보존 — 프로필 유지 + auto_restore=false.
    assert!(
        profiles.get(id).map(|p| !p.auto_restore).unwrap_or(false),
        "idempotency: 1회차에 disposition(프로필 유지 + auto_restore=false)이 적용되지 않음"
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
    // 2회차 no-op → 프로필 상태(보존 + auto_restore=false)가 흔들리지 않아야 한다.
    assert!(
        profiles.get(id).map(|p| !p.auto_restore).unwrap_or(false),
        "idempotency: 2회차에 프로필 상태가 흔들림(유지 + auto_restore=false 여야 함)"
    );
}

// ── ADR-0084 apply_disposition epoch-guard: stale reap 이 재활성화된 산 세션을 강등 못 함 ──────
//
// 시나리오(레이스 모사, 실 PTY 없이 결정적):
//   1) 프로필 epoch=E(=0), auto_restore=true 로 산 세션이 돌던 상태.
//   2) 그 세션이 죽어 reaper 가 sessions.remove(epoch=E) 까지 마쳤다(맵에서 빠짐).
//   3) remove 와 lock-free apply_disposition 사이 창에서 **재활성화**가 일어나 프로필 epoch 를
//      E+1 로 bump(manager.rs activate_profile Resume 갈래 = bump_epoch) + 새 산 세션이 붙었다.
//   4) 뒤늦게 도착한 옛 reap 의 apply_disposition(reaped_epoch=E)이 실행된다.
//   기대: p.epoch(E+1) != reaped_epoch(E) → 다운그레이드 스킵 → auto_restore=true 유지(산 세션
//         이 부팅 복원 대상에서 탈락하지 않는다). epoch-guard 가 없으면 여기서 false 로 강등된다.
//
// ★맵을 비워두고 reap_one 을 부르는 대신, epoch=E 세션을 맵에 남겨 sessions.remove 를 통과시켜야
//   apply_disposition 까지 도달한다★ — reaped_epoch=E 로 맞춰 remove 승자가 되게 하되, **프로필**
//   epoch 만 E+1 로 올려 disposition 계층의 불일치를 만든다(remove 가드는 세션 epoch, disposition
//   가드는 프로필 epoch 를 본다 — 이 테스트가 겨냥하는 건 후자).
#[test]
fn stale_disposition_does_not_downgrade_reactivated_live_session() {
    let (profiles, _sink, deps) = make_reaper_deps("disp-epoch-guard");
    let id = Uuid::new_v4();

    // 죽은 세션(epoch=0)을 맵에 넣어 reaped_epoch=0 이 sessions.remove 승자가 되게 한다.
    let status_dyn: Arc<dyn StatusSink> = Arc::new(_sink.clone());
    let dead = make_test_session(id, 0, status_dyn);
    deps.sessions.write().unwrap().insert(id, dead);

    // 재활성화가 일어난 산 세션을 모사 — 프로필 epoch 를 1 로 올리고 auto_restore=true 로 둔다.
    let mut profile = exit_profile(0);
    profile.id = id;
    profile.epoch = 1; // 재활성화 bump 후 상태(reaped_epoch=0 과 불일치).
    profile.auto_restore = true;
    profiles.upsert(profile);

    // 옛 reap(reaped_epoch=0)이 뒤늦게 도착. sessions.remove(epoch=0)는 성공하지만,
    // apply_disposition epoch-guard 가 p.epoch(1) != reaped_epoch(0) 로 다운그레이드를 스킵해야 한다.
    let stale = ReapMsg {
        id,
        epoch: 0,
        reason: TerminalReason::Exited { code: Some(0) },
        intent_at_finish: TerminationIntent::None,
        shutting_down_at_finish: false,
    };
    deps.reap_one(stale);

    // ★핵심 단언★: epoch 불일치이므로 산 세션의 auto_restore 는 true 로 남아야 한다(강등 안 됨).
    //   epoch-guard 가 없으면(옛 코드) 여기서 false 로 강등돼 부팅 복원에서 누락됐을 것이다.
    assert!(
        profiles.get(id).map(|p| p.auto_restore).unwrap_or(false),
        "ADR-0084: epoch 불일치(재활성화) stale reap 이 산 세션 auto_restore 를 강등하면 안 됨"
    );
}

// ── ADR-0084 대조군: epoch 일치 stale 없음 → 정상 다운그레이드(가드가 정상 종료를 막지 않음) ──────
#[test]
fn matching_epoch_disposition_downgrades_as_before() {
    let (profiles, _sink, deps) = make_reaper_deps("disp-epoch-match");
    let id = Uuid::new_v4();

    // 죽은 세션(epoch=0) + 프로필 epoch=0(재활성화 없음) — reaped_epoch 과 일치.
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

    // epoch 일치 → 정상 다운그레이드(auto_restore=false). 가드가 정상 경로를 막지 않음을 단언.
    assert!(
        profiles.get(id).map(|p| !p.auto_restore).unwrap_or(false),
        "ADR-0084: epoch 일치 시 정상 종료는 auto_restore 를 false 로 다운그레이드해야 함(가드가 정상 경로를 막지 않음)"
    );
}
