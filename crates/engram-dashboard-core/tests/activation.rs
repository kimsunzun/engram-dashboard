//! 수동 활성화(activate_profile) 통합테스트 — ADR-0082(fresh-fallback 폐지, 이어받기 전용).
//!
//! 실 claude 를 CI/단위에서 못 띄우므로 cmd.exe 배치 프로필로 모사한다(격리). Windows 전용이라
//! #[cfg(windows)]. 단일 spawn·전역 경합 없음 → default.

#![cfg(windows)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::preset::PresetRegistry;
use engram_dashboard_core::agent::profile::{
    AgentCommand, AgentProfile, ProfileRegistry, SpawnMode,
};
use engram_dashboard_core::agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_core::agent::types::{AgentId, AgentInfo, AgentStatus, StatusSink};
use engram_dashboard_core::persistence::{FilePresetStore, FileProfileStore};

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
        std::thread::sleep(Duration::from_millis(50));
    }
    cond()
}

fn make_manager(tag: &str) -> (AgentManager, CountingSink, Arc<ProfileRegistry>) {
    let sink = CountingSink::new();
    let sink_dyn: Arc<dyn StatusSink> = Arc::new(sink.clone());
    let store = Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-test-activate-{tag}-{}", Uuid::new_v4())),
    ));
    let profiles = Arc::new(ProfileRegistry::new(store));
    let preset_store = Arc::new(FilePresetStore::new(std::env::temp_dir().join(format!(
        "engram-test-activate-preset-{tag}-{}",
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

/// 반환: (프로필, batch 경로, count 경로).
///
/// ★run-count 가 ADR-0082 의 증거인 이유★: 이 배치는 실행될 때마다 count 에 한 줄을 남기고
///   조기종료(exit 1)한다. 옛 fresh-fallback 이 살아 있었다면 둘째 spawn(fresh 자리)이 일어나
///   count 가 2 가 됐을 것이다 — "정확히 1" 이 fresh-fallback 부재의 결정적 증거다.
///
/// ★왜 인라인 복합 cmd 가 아니라 배치 파일인가★: portable-pty CommandBuilder 가 `>`·`&` 를 개별
///   quoting 해 ConPTY 통과 중 깨뜨린다(옛 test 실측). 배치는 cmd 가 직접 파싱하니 결정적이다.
fn always_early_exit_profile(tag: &str) -> (AgentProfile, PathBuf, PathBuf) {
    let uniq = Uuid::new_v4();
    let count = std::env::temp_dir().join(format!("engram-activate-count-{tag}-{uniq}.tmp"));
    let batch = std::env::temp_dir().join(format!("engram-activate-exit-{tag}-{uniq}.cmd"));

    let script = format!(
        "@echo off\r\n\
         echo x>>\"{c}\"\r\n\
         exit /b 1\r\n",
        c = count.display()
    );
    std::fs::write(&batch, script).expect("배치 파일 write");

    // 마지막 인자 auto_restore=true — false 로 태어나면 다운그레이드가 no-op 이라 관측할 수 없다.
    let profile = AgentProfile::new(
        "activate-exit".into(),
        AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), batch.to_string_lossy().to_string()],
        },
        PathBuf::from("."),
        vec![],
        true,
    );
    (profile, batch, count)
}

/// 파일이 없으면 0(아직 한 번도 실행되지 않음).
fn run_count(path: &PathBuf) -> usize {
    std::fs::read_to_string(path)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

/// EARLY_EXIT_WINDOW(3s)를 넘겨 사는 배치 — ping 20회로 ≈19s 생존해 조기종료로 오판되지 않는다.
/// 반환: (프로필, batch 경로, count 경로).
///
/// ★count 를 ping **앞**에 두는 이유★: start 직후 곧바로 1 이 되어야 테스트가 즉시 읽을 수 있다.
///   kill+replace(같은 epoch 로 위장) 회귀가 있었다면 count 가 2 가 된다.
fn long_lived_profile(tag: &str) -> (AgentProfile, PathBuf, PathBuf) {
    let uniq = Uuid::new_v4();
    let count = std::env::temp_dir().join(format!("engram-activate-live-count-{tag}-{uniq}.tmp"));
    let batch = std::env::temp_dir().join(format!("engram-activate-live-{tag}-{uniq}.cmd"));
    let script = format!(
        "@echo off\r\n\
         echo x>>\"{c}\"\r\n\
         ping -n 20 127.0.0.1 >nul\r\n",
        c = count.display()
    );
    std::fs::write(&batch, script).expect("배치 write");
    let profile = AgentProfile::new(
        "activate-live".into(),
        AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), batch.to_string_lossy().to_string()],
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    (profile, batch, count)
}

/// ★ADR-0082 회귀 가드 ①★ — resume 조기종료가 자동 fresh 로 대체되지 않는다.
#[test]
fn activate_resume_early_exit_ends_failed_no_fresh_fallback() {
    let (manager, _sink, profiles) = make_manager("resume-no-fallback");

    let (profile, batch, count) = always_early_exit_profile("resume-no-fallback");
    let id = profile.id;
    profiles.upsert(profile.clone());

    let sid_before = profiles.get(id).and_then(|p| p.claude_session_id);
    let old_sids_before = profiles
        .get(id)
        .map(|p| p.old_session_ids.len())
        .unwrap_or(0);

    let result = manager.activate_profile(&profile, SpawnMode::Resume);

    assert!(
        result.is_err(),
        "resume 조기종료는 fresh-fallback 없이 Err(Failed 시체)여야 함 — got Ok: {result:?}"
    );

    // 수거는 비동기라 기다려만 준다 — 수거 자체는 여기서 단언 대상이 아니다.
    let _ = wait_until(Duration::from_secs(5), || {
        !manager.list_agents().iter().any(|a| a.id == id)
    });

    assert_eq!(
        run_count(&count),
        1,
        "resume 자리 1회만 실행돼야 함(fresh-fallback 이 없어야 함) — got {}",
        run_count(&count)
    );

    assert_eq!(
        profiles.get(id).and_then(|p| p.claude_session_id),
        sid_before,
        "resume 실패로 새 sid 가 발급되면 안 됨(fresh-fallback 폐지)"
    );
    assert_eq!(
        profiles.get(id).map(|p| p.old_session_ids.len()),
        Some(old_sids_before),
        "resume 실패로 옛 sid 가 이력으로 밀리면 안 됨(new_session_id 미호출)"
    );

    assert!(
        profiles.get(id).is_some(),
        "resume 실패 후 프로필(시체)이 삭제되면 안 됨 — KeepDisableAutoRestore 로 보존돼야 함"
    );

    assert_eq!(
        profiles.get(id).map(|p| p.auto_restore),
        Some(false),
        "resume 실패 시체는 auto_restore=false 로 다운그레이드돼야 함(삭제 아님)"
    );

    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// ★ADR-0082 회귀 가드 ②★ — 산 에이전트 재활성화가 그 에이전트를 파괴하지 않는다(a4aac1a).
#[test]
fn reactivate_running_agent_leaves_it_alive_epoch_unchanged() {
    let (manager, _sink, profiles) = make_manager("reactivate-live");

    let (profile, batch, count) = long_lived_profile("reactivate-live");
    let id = profile.id;
    profiles.upsert(profile.clone());

    let first = manager
        .activate_profile(&profile, SpawnMode::Fresh)
        .expect("최초 활성화는 Ok(살아있는 세션)여야 함");
    let epoch_after_first = first.epoch;

    assert!(
        wait_until(Duration::from_secs(3), || {
            manager.list_agents().iter().any(|a| {
                a.id == id
                    && !matches!(
                        a.status,
                        AgentStatus::Failed { .. }
                            | AgentStatus::Killed
                            | AgentStatus::Exited { .. }
                    )
            })
        }),
        "최초 활성화 세션이 살아있어야 함"
    );

    // 배치의 append 는 PTY 스폰 프로세스라 지연될 수 있어 1 에 도달할 때까지 기다린다.
    assert!(
        wait_until(Duration::from_secs(3), || run_count(&count) == 1),
        "최초 활성화로 배치가 1회 start 돼야 함 — got {}",
        run_count(&count)
    );

    // 반환된 AgentInfo.epoch 만 보면 레지스트리 맵 교체를 못 잡으므로 레지스트리 값도 따로 떠 둔다.
    let reg_epoch_before = profiles.get(id).map(|p| p.epoch);

    let reactivated = manager
        .activate_profile(&profile, SpawnMode::Resume)
        .expect("재활성화는 무해한 Ok(이미 실행 중 AgentInfo)여야 함 — 죽으면 Err/회귀");

    // 배치 실행(append)은 비동기라 '부재'는 창으로만 확인된다 — 2s 를 주고 count 가 안 오름을 본다.
    //   결정적 축은 아래 epoch 불변이고, 이 count 는 보조 heuristic 이다.
    let respawned = wait_until(Duration::from_secs(2), || run_count(&count) >= 2);
    assert!(
        !respawned,
        "재활성화가 재spawn 을 유발하면 안 됨 — count 가 2로 오름(회귀 신호)"
    );
    assert_eq!(
        run_count(&count),
        1,
        "재활성화 후 배치 실행은 정확히 1회여야 함(재spawn 없음)"
    );

    assert_eq!(reactivated.id, id, "재활성화는 같은 에이전트를 가리켜야 함");
    assert_eq!(
        reactivated.epoch, epoch_after_first,
        "산 세션 재활성화는 화신 표식을 갈지 않는다(맵 교체=fresh 없음, a4aac1a 회귀 신호)"
    );
    assert!(
        !matches!(
            reactivated.status,
            AgentStatus::Failed { .. } | AgentStatus::Killed | AgentStatus::Exited { .. }
        ),
        "재활성화 후 산 에이전트가 종점 상태면 파괴된 것 — 살아있어야 함: {:?}",
        reactivated.status
    );

    assert_eq!(
        profiles.get(id).map(|p| p.epoch),
        reg_epoch_before,
        "산 세션 재활성화는 레지스트리 표식도 갈지 않는다(맵 교체=fresh 없음)"
    );

    // 300ms 는 비동기 reaper 가 뒤늦게 옛 세션을 수거하는 회귀를 잡기 위한 창이다.
    std::thread::sleep(Duration::from_millis(300));
    let live = manager.list_agents();
    let entry = live.iter().find(|a| a.id == id);
    assert!(
        entry.is_some_and(|a| a.epoch == epoch_after_first
            && !matches!(
                a.status,
                AgentStatus::Failed { .. } | AgentStatus::Killed | AgentStatus::Exited { .. }
            )),
        "재활성화 후 원본 세션이 kill 되거나 epoch 가 바뀌면 안 됨 — got {entry:?}"
    );

    assert_eq!(
        run_count(&count),
        1,
        "재활성화는 산 에이전트를 재spawn 하면 안 됨(배치 start 는 여전히 1회여야 함) — got {}",
        run_count(&count)
    );

    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// ★ADR-0113 배선 가드★ — spawn 이 턴 관측 표에 자기 화신을 **등록**한다.
///
/// 이 등록이 그 화신의 첫 출력 신호보다 먼저라는 것이 표의 "표식이 다르면 버린다" 규칙을 안전하게 만든다
/// (`turn::TurnObservations::register`). 배선이 빠지면 앞 화신의 항목이 남아 있는 동안 새 화신의 신호가
/// 전부 버려지고, 그 에이전트는 미관측(=턴 아님)으로 답해 턴 중에 우편이 들어간다.
/// ★셸을 쓰는 것이 판별력의 근거다★: 셸 백엔드는 턴 신호를 하나도 내지 않으므로(분류자 침묵), 여기 항목이
/// 있다면 그건 오직 등록이 만든 것이다.
/// ★안 재는 것(정직 범위)★: 등록이 **첫 신호보다 먼저**라는 순서 자체는 여기서 못 잰다 — 그걸 뒤집으려면
/// 앞 화신의 `finish` 를 막아야 하는데 이 하네스엔 그 손잡이가 없다. 순서는 호출 지점(sessions 맵 insert
/// 전)이 구조적으로 보장한다.
#[test]
fn spawn_registers_the_incarnation_in_the_turn_table() {
    let (manager, _sink, profiles) = make_manager("turn-register");

    let (profile, batch, count) = long_lived_profile("turn-register");
    let id = profile.id;
    profiles.upsert(profile.clone());

    let info = manager
        .activate_profile(&profile, SpawnMode::Fresh)
        .expect("활성화는 Ok");
    assert!(
        manager.turns().get(id, info.epoch).is_some(),
        "spawn 이 이 화신의 자리를 표에 잡아야 한다"
    );
    assert!(
        !manager.turns().is_in_turn(id, info.epoch),
        "등록만으로는 턴 중이 아니다(신호는 라이브 emit 에서만 온다)"
    );

    manager.kill_agent(id).expect("kill_agent failed");
    assert!(
        wait_until(Duration::from_secs(5), || {
            manager.turns().get(id, info.epoch).is_none()
        }),
        "종료가 자기 항목을 거둬야(안 거두면 죽은 에이전트 앞 파킹이 안 풀린다)"
    );

    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// ★ADR-0084 회귀 가드★ — 시체를 같은 슬롯에서 재활성화하면 화신 표식이 달라진다.
///
/// 셸은 needs_session=false 라 `--resume` 플래그 조립까지는 보지 않는다 — 그건
///   `backend::claude::tests::build_command_spec_resume_emits_resume_flag_with_sid` 몫이고,
///   이 테스트가 겨냥하는 "재활성화 = 맵 교체 = 새 표식" 은 backend 무관하게 성립한다.
///   ★재는 것은 **다름**뿐이다 — 커진다가 아니다★: 표식은 화신마다 뽑는 난수라 대소에 뜻이 없다
///   (`ProfileRegistry::epoch_for_spawn`). 단언을 `assert!(new > old)` 로 조이면 절반의 확률로 붉어진다.
#[test]
fn reactivate_after_kill_bumps_epoch() {
    let (manager, _sink, profiles) = make_manager("reactivate-epoch-bump");

    let (profile, batch, count) = long_lived_profile("reactivate-epoch-bump");
    let id = profile.id;
    profiles.upsert(profile.clone());

    let first = manager
        .activate_profile(&profile, SpawnMode::Fresh)
        .expect("최초 활성화는 Ok(살아있는 세션)여야 함");
    let epoch_e = first.epoch;
    assert!(
        wait_until(Duration::from_secs(3), || {
            manager.list_agents().iter().any(|a| a.id == id)
        }),
        "최초 활성화 세션이 살아있어야 함"
    );

    manager.kill_agent(id).expect("kill_agent failed");
    assert!(
        wait_until(Duration::from_secs(5), || {
            !manager.list_agents().iter().any(|a| a.id == id)
        }),
        "유저 kill 후 세션이 맵에서 수거돼야 함"
    );
    assert_eq!(
        profiles.get(id).map(|p| p.epoch),
        Some(epoch_e),
        "kill 만으로는 표식이 갈리지 않는다(재활성화 respawn 이 발급의 주체)"
    );

    let reactivated = manager
        .activate_profile(&profile, SpawnMode::Resume)
        .expect("재활성화가 resume 경로로 Ok(살아있는 세션)여야 함(셸은 조기종료 안 함)");

    assert_ne!(
        reactivated.epoch, epoch_e,
        "ADR-0084: 재활성화 세션의 화신 표식이 죽은 세션의 것과 달라야 함(맵 교체 = 새 화신)"
    );
    assert_eq!(
        profiles.get(id).map(|p| p.epoch),
        Some(reactivated.epoch),
        "재활성화 후 레지스트리 표식과 세션 표식이 일치해야 함(발급이 spawn_agent 읽기 전에 반영)"
    );

    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// ★ADR-0083 회귀 가드★ — 유저 kill 이 프로필을 지우면 재활성화가 "profile not found"(화면엔 "실패")로
/// 깨진다. 옛 reaper 가 UserKill → DeleteProfile 로 claude_session_id 째 지워 실제로 그랬다.
///
/// 여기서 증명되는 건 **프로필 조회 경로가 온전하다**까지다 — 실제 `--resume <sid>` 조립은
///   `backend::claude::tests::build_command_spec_resume_emits_resume_flag_with_sid` 가 실증한다.
#[test]
fn user_kill_then_reactivate_finds_profile_and_resumes() {
    let (manager, _sink, profiles) = make_manager("kill-reactivate");

    let (profile, batch, count) = long_lived_profile("kill-reactivate");
    let id = profile.id;

    // ★seeded 프로필을 spawn/activate/kill 전부에 넘겨야 한다★: spawn 은 넘겨받은 스냅샷을
    //   upsert_preserving_hierarchy 로 그대로 심으므로, claude_session_id=None 인 원본을 넘기면
    //   심어둔 sid 가 덮여 유실된다. auto_restore=true 는 kill 수거의 다운그레이드를 관측하기 위함.
    let sid = Uuid::new_v4();
    let mut seeded = profile.clone();
    seeded.claude_session_id = Some(sid);
    seeded.auto_restore = true;
    profiles.upsert(seeded.clone());

    manager
        .activate_profile(&seeded, SpawnMode::Fresh)
        .expect("최초 활성화는 Ok(살아있는 세션)여야 함");
    assert!(
        wait_until(Duration::from_secs(3), || {
            manager.list_agents().iter().any(|a| a.id == id)
        }),
        "최초 활성화 세션이 살아있어야 함"
    );

    manager.kill_agent(id).expect("kill_agent failed");
    assert!(
        wait_until(Duration::from_secs(5), || {
            !manager.list_agents().iter().any(|a| a.id == id)
        }),
        "유저 kill 후 세션이 맵에서 수거돼야 함"
    );

    assert!(
        wait_until(Duration::from_secs(2), || {
            profiles.get(id).map(|p| !p.auto_restore).unwrap_or(false)
        }),
        "유저 kill 시체는 프로필 유지 + auto_restore=false 여야 함(ADR-0083 — 삭제 아님)"
    );
    assert!(
        profiles.get(id).is_some(),
        "유저 kill 후 프로필이 삭제됨 — 시체로 보존돼야 함(ADR-0083 회귀)"
    );
    assert_eq!(
        profiles.get(id).and_then(|p| p.claude_session_id),
        Some(sid),
        "유저 kill 로 claude_session_id 가 유실됨 — 재활성화 resume 불가(ADR-0083 회귀)"
    );

    let reactivated = manager
        .activate_profile(&seeded, SpawnMode::Resume)
        .expect("재활성화가 profile not found 없이 resume 경로로 진입해 Ok 여야 함(ADR-0083)");
    assert_eq!(
        reactivated.id, id,
        "재활성화는 같은 에이전트(보존된 시체 프로필)를 가리켜야 함"
    );
    assert!(
        !matches!(
            reactivated.status,
            AgentStatus::Failed { .. } | AgentStatus::Killed | AgentStatus::Exited { .. }
        ),
        "재활성화된 세션이 종점 상태면 resume 진입 실패 — 살아있어야 함: {:?}",
        reactivated.status
    );

    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}
