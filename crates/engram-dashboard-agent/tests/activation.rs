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

use engram_dashboard_agent::backend;
use engram_dashboard_agent::failure::AgentFailureKind;
use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::persistence::{FilePresetStore, FileProfileStore};
use engram_dashboard_agent::preset::PresetRegistry;
use engram_dashboard_agent::profile::{
    AgentCommand, AgentOutputFormat, AgentProfile, ProfileRegistry, ProfileStore, SpawnMode,
};
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, ControlEndpoint, NoopControlChannel,
    ProvisionError, StatusSink,
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
    make_manager_with_control(tag, Arc::new(NoopControlChannel))
}

fn make_manager_with_control(
    tag: &str,
    control: Arc<dyn ControlChannel>,
) -> (AgentManager, CountingSink, Arc<ProfileRegistry>) {
    let (manager, sink, profiles, _store) = make_manager_with_store(tag, control);
    (manager, sink, profiles)
}

/// 디스크에 무엇이 남았는지를 보는 항목용 — 명부와 같은 저장소를 함께 돌려준다.
fn make_manager_with_store(
    tag: &str,
    control: Arc<dyn ControlChannel>,
) -> (
    AgentManager,
    CountingSink,
    Arc<ProfileRegistry>,
    Arc<FileProfileStore>,
) {
    let sink = CountingSink::new();
    let sink_dyn: Arc<dyn StatusSink> = Arc::new(sink.clone());
    let store = Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-test-activate-{tag}-{}", Uuid::new_v4())),
    ));
    let profiles = Arc::new(ProfileRegistry::new(store.clone()));
    let preset_store = Arc::new(FilePresetStore::new(std::env::temp_dir().join(format!(
        "engram-test-activate-preset-{tag}-{}",
        Uuid::new_v4()
    ))));
    let presets = Arc::new(PresetRegistry::new(preset_store));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager =
        AgentManager::new_with_control(sink_dyn, profiles.clone(), presets, tracker, control);
    (manager, sink, profiles, store)
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

    let sid_before = profiles.get(id).and_then(|p| p.backend_session_id);
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
        profiles.get(id).and_then(|p| p.backend_session_id),
        sid_before,
        "resume 실패로 새 sid 가 발급되면 안 됨(fresh-fallback 폐지)"
    );
    assert_eq!(
        profiles.get(id).map(|p| p.old_session_ids.len()),
        Some(old_sids_before),
        "resume 실패로 옛 sid 가 이력으로 밀리면 안 됨(Fresh 의 비우기 미호출)"
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
/// 셸은 두 세션 축이 모두 false 라 `--resume` 플래그 조립까지는 보지 않는다 — 그건
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
/// 깨진다. 옛 reaper 가 UserKill → DeleteProfile 로 backend_session_id 째 지워 실제로 그랬다.
///
/// 여기서 증명되는 건 **프로필 조회 경로가 온전하다**까지다 — 실제 `--resume <sid>` 조립은
///   `backend::claude::tests::build_command_spec_resume_emits_resume_flag_with_sid` 가 실증한다.
#[test]
fn user_kill_then_reactivate_finds_profile_and_resumes() {
    let (manager, _sink, profiles) = make_manager("kill-reactivate");

    let (profile, batch, count) = long_lived_profile("kill-reactivate");
    let id = profile.id;

    // ★seeded 프로필을 spawn/activate/kill 전부에 넘긴다★ — 명부와 인자를 한 값으로 맞춰 이 항목이
    //   재는 것이 손잡이 보존이 아니라 **이어받기 배선**임을 분명히 한다.
    //   ★옛 사유(「안 그러면 심어둔 sid 가 덮여 유실된다」)는 낡았다★ — `upsert_preserving_hierarchy` 가
    //   이제 `backend_session_id` 와 그 이력을 live 에서 보존한다(사용자 결정). 그 보존 자체의 회귀망은
    //   `profile.rs` 의 `spawn_preserving_upsert_does_not_revert_a_handle_recorded_after_the_snapshot`
    //   이고 여기서 겸하지 않는다.
    //   ★**그 보존이 claude 에서도 동작을 바꾼다 — 「claude 는 그대로다」로 읽지 말 것**★: 재활성화가
    //   쓰는 sid 는 `ensure_position` 이 아니라 spawn 이 **명부에서** 읽어 오는데, 등록이
    //   더 이상 그 칸을 스냅샷으로 덮지 않으므로 이제 **`SessionTracker` 가 관측한 드리프트 sid** 가
    //   읽힌다(그 관측기는 `observe_session_id(id, None, ..)` 로 무조건 쓰고, `assigns_sid` 게이트
    //   뒤에서만 붙으므로 실제 대상이 claude 다). 즉 낡은 스냅샷으로 재활성화해도 드리프트한 쪽을
    //   이어받는다 — 의도한 개선이지만 **무변화가 아니다**. 이 항목은 명부와 인자가 같아 그 차이를
    //   재지 않는다(그래서 여기 적어 둔다).
    //   auto_restore=true 는 kill 수거의 다운그레이드를 관측하기 위함.
    let sid = Uuid::new_v4();
    let mut seeded = profile.clone();
    seeded.backend_session_id = Some(sid);
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
        profiles.get(id).and_then(|p| p.backend_session_id),
        Some(sid),
        "유저 kill 로 backend_session_id 가 유실됨 — 재활성화 resume 불가(ADR-0083 회귀)"
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

// ── 마지막 실패 기록(ADR-0172) ────────────────────────────────────────────────────

/// 이어받기가 조기 종료로 실패하면 그 자리에서 종류가 붙는다 — 사전 판정 없이 관측만으로.
///
/// ★shell 프로필이라 종류가 「이어받기 직후 조기 종료」다★: 문구를 알아보는 지식은 claude backend 에만
///   있고(그 단위 테스트가 별도), shell 은 침묵하므로 맥락 기본값으로 떨어진다 — 그 기본값이 재시도
///   가능이라는 것이 fail-open 의 실물이다.
#[test]
fn resume_early_exit_records_a_typed_last_failure() {
    let (manager, _sink, profiles) = make_manager("record-early-exit");

    let (profile, batch, count) = always_early_exit_profile("record-early-exit");
    let id = profile.id;
    profiles.upsert(profile.clone());
    assert_eq!(
        profiles.get(id).and_then(|p| p.last_failure),
        None,
        "전제: 시도 전에는 기록이 없다(사전 판정을 하지 않는다)"
    );

    let result = manager.activate_profile(&profile, SpawnMode::Resume);
    assert!(result.is_err(), "전제: 조기 종료는 Err 로 끝난다");

    let recorded = profiles.get(id).and_then(|p| p.last_failure);
    assert_eq!(
        recorded,
        Some(AgentFailureKind::EarlyExitAfterResume),
        "실패한 자리에서 종류가 붙어야 한다 — got {recorded:?}"
    );

    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// 프로세스를 아예 못 띄운 활성화도 기록한다(Fresh 갈래 — resume 만 덮으면 이쪽이 샌다).
#[test]
fn an_unspawnable_profile_records_a_spawn_failure() {
    let (manager, _sink, profiles) = make_manager("record-spawn-fail");

    let profile = AgentProfile::new(
        "activate-nonexistent".into(),
        AgentCommand::Shell {
            // 실재하지 않는 실행파일 — PTY open 이 실패해 spawn 이 Err 로 끝난다.
            program: format!("engram-not-a-real-program-{}.exe", Uuid::new_v4()),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    let id = profile.id;
    profiles.upsert(profile.clone());

    let result = manager.activate_profile(&profile, SpawnMode::Fresh);
    assert!(result.is_err(), "전제: 없는 실행파일은 Err 로 끝난다");
    assert_eq!(
        profiles.get(id).and_then(|p| p.last_failure),
        Some(AgentFailureKind::SpawnFailed)
    );
}

/// ★이미 떠 있는 에이전트를 다시 복원해도 실패 도장이 찍히지 않는다★ — 그리고 이 경로는 **결정적**이다.
///
/// `restore_one` 은 `activate_profile` 의 "이미 실행 중" 선제 필터를 지나지 않으므로, 산 에이전트를 두고
/// 복원을 돌리면 `spawn_agent` 의 이중-spawn 가드에 그대로 부딪힌다 — 스레드 인터리빙을 기다릴 필요 없이
/// 그 갈래(`SpawnOutcome::Moot`)가 활성화 기록 경로로 들어온다. 옛 형태는 두 스레드를 경쟁시켜 승자·패자를
/// 기대했는데, 선제 필터 뒤에서 디스케줄만 나면 둘 다 통과해 통과 조건이 흔들렸다(간헐 red) — 되살리지 말 것.
///
/// ★"두 번째 프로세스가 안 떴다" 를 **시간 창으로 재지 않는다**★: 파일 append 로 세면 부재 단언이
/// "아직 안 썼다" 와 구분되지 않아, 대기를 얼마로 잡든 경합 아래선 헛통과할 수 있다(그래서 이 파일의 다른
/// 픽스처가 쓰는 count 대기를 여기선 쓰지 않는다). 대신 **epoch** 을 본다: 두 번째 spawn 은 반드시
/// `epoch_for_spawn` 을 지나 값을 올리므로, 값이 그대로면 아무것도 안 떴다는 증거가 즉시·확정적으로 선다.
#[test]
fn restoring_over_a_live_agent_never_stamps_it_as_failed() {
    let (manager, _sink, profiles) = make_manager("restore-over-live");

    let (profile, batch, count) = long_lived_profile("restore-over-live");
    let id = profile.id;
    profiles.upsert(profile.clone());
    manager
        .activate_profile(&profile, SpawnMode::Fresh)
        .expect("최초 활성화는 성공한다");
    assert!(
        wait_until(Duration::from_secs(5), || {
            manager.list_agents().iter().any(|a| a.id == id)
        }),
        "전제: 세션이 명부에 올라 살아 있다"
    );
    assert_eq!(
        profiles.get(id).and_then(|p| p.last_failure),
        None,
        "전제: 성공한 활성화는 기록을 남기지 않는다"
    );
    let epoch_before = profiles.get(id).map(|p| p.epoch);

    // spawn 이 auto_restore 를 true 로 올려 두므로 이 프로필은 복원 대상이다.
    let reports = manager.restore_all();
    let handled = reports.iter().any(|r| r.agent_id == id);
    assert!(
        handled,
        "전제: 복원이 이 프로필을 훑어야 한다 — got {reports:?}"
    );

    assert_eq!(
        profiles.get(id).and_then(|p| p.last_failure),
        None,
        "중복 요청은 그 항목의 실패가 아니다 — 산 에이전트에 도장이 찍히면 정상 종료 순간 화면에 뜬다"
    );
    assert_eq!(
        profiles.get(id).map(|p| p.epoch),
        epoch_before,
        "epoch 이 올랐다 = 두 번째 화신이 떴다(할 일 없는 요청이 프로세스를 띄우면 안 된다)"
    );
    assert_eq!(
        manager.list_agents().iter().filter(|a| a.id == id).count(),
        1,
        "명부의 화신도 하나뿐이어야 한다"
    );

    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// ★사용자가 끊은 것은 활성화 실패가 아니다★ — 조기종료 창 안에서 kill 이 와도 기록이 남지 않는다.
///
/// 기록되면 트리에 「이어받은 직후 종료됐습니다」가 남아, 방금 스스로 끈 항목이 고장 난 것처럼 보인다
/// (그 표시는 다음 활성화가 성립할 때까지 안 지워진다). HEAD 에선 이 경로가 일시적 Err 만 냈다.
///
/// ★거짓 red 가 없는 모양★: 단언은 「기록이 없다」 하나이고, 그것은 두 인터리빙 **모두에서** 참이다 —
/// kill 이 창 안에 들면 이 갈래가(기록 안 함), 창을 넘겨 들면 성립 갈래가(지움) 같은 결과를 만든다.
/// 어느 쪽을 탔는지는 활성화 반환값으로 구분해 아래에서 보고만 한다.
#[test]
fn a_kill_inside_the_resume_window_is_not_recorded_as_a_failure() {
    let (manager, _sink, profiles) = make_manager("kill-in-window");

    let (profile, batch, count) = long_lived_profile("kill-in-window");
    let id = profile.id;
    profiles.upsert(profile.clone());

    let manager = Arc::new(manager);
    let activator = {
        let m = manager.clone();
        let p = profile.clone();
        // Resume 모드라 조기종료 창(3s)만큼 폴링하며 블록된다 — 그 사이 본 스레드가 끊는다.
        std::thread::spawn(move || m.activate_profile(&p, SpawnMode::Resume))
    };

    assert!(
        wait_until(Duration::from_secs(5), || {
            manager.list_agents().iter().any(|a| a.id == id)
        }),
        "전제: 세션이 명부에 올라야 끊을 수 있다"
    );
    let _ = manager.kill_agent(id);

    let outcome = activator.join().expect("활성화 스레드");
    assert_eq!(
        profiles.get(id).and_then(|p| p.last_failure),
        None,
        "사용자 종료는 활성화 실패가 아니다 — 도장이 남으면 안 된다(결말: {})",
        if outcome.is_err() {
            "창 안에서 관측됨"
        } else {
            "창을 넘겨 성립으로 판정됨"
        }
    );

    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

// ── 세션 id 발급 축(ADR-0185) ─────────────────────────────────────────────────

/// provision 을 떨어뜨려 spawn 을 **통로 생성 전에** 끊는 시험대 장치. 그 중단점이 sid 발급보다 뒤라,
/// 이 장치를 낀 spawn 은 프로세스를 하나도 안 띄우고도 발급 지점을 실제로 지나간다.
struct FailingControl;

impl ControlChannel for FailingControl {
    fn provision(
        &self,
        _id: AgentId,
        _epoch: u32,
        _needs: engram_dashboard_agent::types::ControlChannelNeeds,
    ) -> Result<Option<ControlEndpoint>, ProvisionError> {
        Err(ProvisionError(
            "시험대: 통로 생성 전에 spawn 을 끊는다".into(),
        ))
    }

    fn revoke(&self, _id: AgentId, _epoch: u32) {}
}

/// ★이 항목이 **실제로** 잰다고 주장하는 것 — 두 가지다★:
///   ① Fresh 스폰은 어느 백엔드든 세션 id 를 **영속하지 않는다**(ADR-0226) — 명부에도 디스크에도 없다.
///      codex 행이 재는 것도 이것뿐이다(등록됐고 비어 있다).
///   ② 우리가 발급하는 백엔드(claude)의 옛 손잡이는 지워지지 않고 **이력으로** 간다.
///
/// ★이름과 달리 「자기 id 를 발급하는 백엔드(codex)에 우리 uuid 를 건네지 않는다」(ADR-0185)는 여기서
///   못 잰다★ — 스폰 때는 어느 백엔드도 영속하지 않으므로, 발급 판정이 이어받기 축으로 갈려도 codex 행은
///   그대로 초록이다. 그 가드는 `manager` 의 `only_a_backend_that_assigns_ids_gets_one_handed_over`(판정)와
///   `the_first_submission_latch_is_wired_through_the_spawn_path`(spawn 이 그 판정을 쓴다)다.
/// ★「우리가 뽑아 건넸다」는 대조군도 여기 없다★ — 뽑은 값은 첫 제출 전에는 영속되지 않아 명부로는 안
///   보인다. 그 짝은 `fresh_claude_spec_carries_the_minted_session_id`(argv 에 실렸다)와
///   `d1_a_fresh_claude_persists_its_minted_id_only_after_the_first_submission`(첫 제출 뒤에야 영속)이다.
/// ★② 가 이 항목의 공회전을 잡는다★ — spawn 이 비우기 **이전**에 죽으면 ① 은 그대로 초록이지만, 옛
///   값이 이력으로 가지 않는다.
///
/// ★spawn 은 **프로세스를 하나도 안 띄운 채** 실패하도록 몰아 둔다 — 그래도 비우기 지점은 지난다★:
///   비우기는 spawn 흐름에서 통로 생성보다 **앞**이라, 뒤에서 끊어도 이 단언은 그 지점을 그대로 잰다.
/// ★두 행이 서로 다른 자리에서 멈춘다★:
///   - 제어 채널을 **쓰는** 백엔드는 아래 [`FailingControl`] 이 provision 에서 fail-closed 로 끊는다.
///   - 제어 채널을 **안 쓰는** 백엔드는 provision 을 아예 안 타므로, 존재하지 않는 cwd 가 통로 생성을
///     떨어뜨린다.
/// ★cwd 하나로 두 행을 다 막을 수 없다(실측 2026-09-14 — 되살리지 마라)★: 존재하지 않는 cwd 를 줘도
///   ConPTY 통로는 그대로 떠서 실 `cmd.exe /c claude` 가 기동했다. 그래서 그 행에는 제어 채널 쪽 장치가
///   따로 필요하다.
// ADR-0226
#[test]
fn we_do_not_mint_a_session_id_for_a_backend_that_mints_its_own() {
    let (manager, _sink, profiles, store) =
        make_manager_with_store("mint-axis", Arc::new(FailingControl));
    let nowhere = std::env::temp_dir().join(format!("engram-no-such-dir-{}", Uuid::new_v4()));

    let codex = AgentProfile::new(
        "codex-app-server".into(),
        AgentCommand::Codex {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        },
        nowhere.clone(),
        vec![],
        false,
    );
    let mut claude = AgentProfile::new(
        "claude-terminal".into(),
        AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        },
        nowhere,
        vec![],
        false,
    );
    let claude_old = Uuid::new_v4();
    claude.backend_session_id = Some(claude_old);

    for p in [&codex, &claude] {
        if let Ok(outcome) = manager.spawn_agent(p, SpawnMode::Fresh) {
            if let Some(info) = outcome.into_started() {
                let _ = manager.kill_agent(info.id);
                panic!(
                    "{:?}: 통로가 실제로 떴다 — 격리 전제(존재하지 않는 cwd 로 spawn 실패)가 깨졌다",
                    p.command
                );
            }
        }
    }

    // ★먼저 명부에 실제로 올랐는지 본다★ — 아래 `None` 은 「영속 안 됨」과 「프로필이 아예 없음」을
    //   구별하지 못해서, 이 줄이 없으면 등록조차 안 된 경우에도 초록이 된다.
    assert!(
        profiles.get(codex.id).is_some() && profiles.get(claude.id).is_some(),
        "프로필이 명부에 없다 — 아래 단언이 영속 여부가 아니라 부재를 재게 된다"
    );
    assert_eq!(
        profiles.get(codex.id).and_then(|p| p.backend_session_id),
        None,
        "codex 의 Fresh 스폰이 세션 id 를 명부에 적었다 — 영속은 첫 제출의 몫이다(ADR-0226). 이 시험대는 \
         spawn 을 통로 생성 전에 끊으므로 수령 경로가 받아 적을 수도 없다 — 그렇다면 격리 전제가 먼저 깨진 것이다"
    );

    let got = profiles.get(claude.id).expect("claude 프로필");
    assert_eq!(
        got.backend_session_id, None,
        "Fresh 스폰이 세션 id 를 명부에 적었다 — 영속은 첫 제출의 몫이다(ADR-0226)"
    );
    assert!(
        got.old_session_ids.contains(&claude_old),
        "옛 손잡이가 이력으로 가지 않았다 — spawn 이 비우기 지점 이전에 죽어 위 단언이 공회전한다"
    );
    let on_disk = store
        .load()
        .into_iter()
        .find(|p| p.id == claude.id)
        .expect("claude 프로필이 디스크에 있다");
    assert_eq!(
        on_disk.backend_session_id, None,
        "디스크에 세션 id 가 남았다 — 0턴 세션의 id 가 영속되면 다음 활성화가 그 id 로 이어받기에 실패한다"
    );
}

/// ★발급 지점에 닿는다는 것을 영속 없이 잰다★ — 뽑은 값은 첫 제출 전에는 명부에 없으므로, 「우리가 뽑아
/// 건넸다」는 argv 에서만 보인다. 순수 함수라 프로세스를 안 띄운다.
// ADR-0226
#[test]
fn fresh_claude_spec_carries_the_minted_session_id() {
    let minted = ProfileRegistry::mint_session_id();
    for output_format in [AgentOutputFormat::Terminal, AgentOutputFormat::StreamJson] {
        let command = AgentCommand::Claude {
            extra_args: vec![],
            output_format,
        };
        let spec = backend::build_command_spec(
            &command,
            SpawnMode::Fresh,
            Some(minted),
            None,
            std::env::temp_dir(),
            vec![],
            None,
        );
        let at = spec
            .args
            .iter()
            .position(|a| a == "--session-id")
            .unwrap_or_else(|| panic!("{command:?}: `--session-id` 가 없다: {:?}", spec.args));
        assert_eq!(
            spec.args.get(at + 1),
            Some(&minted.to_string()),
            "{command:?}: 뽑은 값이 실리지 않았다: {:?}",
            spec.args
        );
        assert!(
            !spec.args.iter().any(|a| a == "--resume"),
            "{command:?}: Fresh 인데 이어받기 플래그가 실렸다: {:?}",
            spec.args
        );
    }
}

/// ★손잡이 읽기는 spawn 안에서 한 번이고 그 읽기가 권위다(ADR-0226)★ — 비어 있으면 저장 손잡이로
/// 이어받는 백엔드는 **프로세스를 띄우기 전에** 거절한다.
///
/// 입구의 머리 판정을 안 지나는 `spawn_agent(Resume)` 를 직접 불러 「판정과 읽기 사이에 칸이 비었다」를
/// 결정론으로 재현한다.
/// ★거절 문구로 가른다★ — 이 시험대는 provision 에서도 끊으므로([`FailingControl`]), 거절이 사라지면 같은
///   `Err` 라도 문구가 provision 실패로 바뀐다. 운영에서는 그 자리를 지나 claude 가 이어받기 플래그 없이
///   떠 모르는 id 의 새 대화를 연다.
/// ★shell 은 옛 길이다★ — 저장 손잡이로 이어받지 않으므로 손잡이 없이도 뜬다(ADR-0082 회귀 시험대가 그
///   길에 기댄다).
// ADR-0226
#[test]
fn a_resume_whose_handle_is_gone_is_refused_before_any_process() {
    let (manager, _sink, profiles) =
        make_manager_with_control("handle-gone", Arc::new(FailingControl));

    let commands = [
        AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        },
        AgentCommand::Codex {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        },
        AgentCommand::Codex {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        },
    ];
    for command in commands {
        let p = AgentProfile::new(
            "handle-gone".into(),
            command,
            std::env::temp_dir(),
            vec![],
            false,
        );
        profiles.upsert(p.clone());
        let refusal = match manager.spawn_agent(&p, SpawnMode::Resume) {
            Ok(outcome) => {
                if let Some(info) = outcome.into_started() {
                    let _ = manager.kill_agent(info.id);
                }
                panic!("{:?}: 손잡이 없는 Resume 이 떴다", p.command);
            }
            Err(e) => e.to_string(),
        };
        assert!(
            refusal.contains("이어받을 손잡이가 spawn 도중 사라졌다"),
            "{:?}: 띄우기 전 거절이 아니다 — {refusal}",
            p.command
        );
        assert_eq!(
            profiles.get(p.id).and_then(|q| q.backend_session_id),
            None,
            "{:?}: 거절이 손잡이를 만들어 냈다",
            p.command
        );
    }

    let (shell, batch, count) = always_early_exit_profile("handle-gone-shell");
    profiles.upsert(shell.clone());
    let outcome = manager
        .spawn_agent(&shell, SpawnMode::Resume)
        .expect("shell 은 손잡이 없이도 옛 길로 뜬다");
    assert!(
        outcome.into_started().is_some(),
        "shell 의 Resume 이 새 화신을 안 만들었다"
    );
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

// ── D1 런타임 통합 — 가짜 claude 를 PATH 로 띄운다(ADR-0226) ─────────────────────────

/// 가짜 `claude.cmd` 를 담은 폴더를 만든다. 반환: (그 폴더, 인자 기록 파일).
///
/// 프로필 env 의 `PATH` 앞에 이 폴더를 붙이면 백엔드의 `cmd.exe /c claude …` 가 이것을 찾는다 — 백엔드는
/// 띄우기 전에 절대 경로를 찾지 않고 이름만 넘기므로 자식의 `PATH` 로 풀린다(실 claude 를 안 띄운다).
/// ★받은 인자를 한 줄씩 기록 파일에 남긴다★ — 가짜의 표준출력은 PTY 로 가서 시험이 못 읽는다.
/// ★`--resume` 이면 실 claude 의 거절 문구를 찍고 exit 1, 아니면 입력을 읽으며 산다★ — 앞엣것의 문구·종료
///   코드는 실측이다(`backend::claude` 의 그 문구 상수 doc).
/// ★에이전트 cwd 는 이 폴더와 **다른** 곳이어야 한다★ — cmd 는 현재 폴더를 `PATH` 보다 먼저 찾으므로, 같으면
///   `PATH` 가 먹었는지 가려진다.
fn fake_claude_dir(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("engram-fake-claude-{tag}-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("가짜 폴더");
    let script = "@echo off\r\n\
         >>\"%~dp0args.log\" echo %*\r\n\
         echo %* | findstr /C:\"--resume\" >nul\r\n\
         if not errorlevel 1 (\r\n\
         echo No conversation found with session ID: fake\r\n\
         exit /b 1\r\n\
         )\r\n\
         :loop\r\n\
         set /p line=\r\n\
         if errorlevel 1 ping -n 2 127.0.0.1 >nul\r\n\
         goto loop\r\n";
    std::fs::write(dir.join("claude.cmd"), script).expect("가짜 claude.cmd write");
    let args_log = dir.join("args.log");
    (dir, args_log)
}

/// 가짜 claude 를 `PATH` 앞에 둔 claude 터미널 프로필 — cwd 는 가짜 폴더와 다른 새 폴더다.
fn claude_on_fake_path(name: &str, fake_dir: &std::path::Path) -> (AgentProfile, PathBuf) {
    let cwd = std::env::temp_dir().join(format!("engram-fake-claude-cwd-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&cwd).expect("에이전트 cwd");
    let path = format!(
        "{};{}",
        fake_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let profile = AgentProfile::new(
        name.into(),
        AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        },
        cwd.clone(),
        vec![("PATH".into(), path)],
        false,
    );
    (profile, cwd)
}

/// 가짜가 받은 인자 줄들(스폰 1회 = 1줄).
fn fake_spawns(args_log: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(args_log)
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// 인자 줄에서 `flag` 바로 뒤의 uuid.
fn uuid_after(line: &str, flag: &str) -> Option<Uuid> {
    let mut words = line.split_whitespace();
    words.find(|w| *w == flag)?;
    words.next().and_then(|w| Uuid::parse_str(w).ok())
}

fn stored_on_disk(store: &FileProfileStore, id: AgentId) -> Option<Uuid> {
    store
        .load()
        .into_iter()
        .find(|p| p.id == id)
        .and_then(|p| p.backend_session_id)
}

/// ★D1 을 실 프로세스로 잰다 — 0턴 세션은 id 를 남기지 않고, 첫 제출이 그 id 를 영속한다(ADR-0226)★.
///
/// 스폰 두 번을 차례로 태운다:
///   1. 대화 없는 옛 id(쓰레기)로 이어받기 — 가짜가 `--resume` 을 받아 거절 문구와 함께 죽는다. Phase A 만
///      착지한 지금은 새 대화로 넘어가지 않고 실패로 끝나며(ADR-0082), 쓰레기는 명부에 그대로 남는다.
///   2. 같은 프로필의 Fresh — 쓰레기가 이력으로 가고, 가짜가 `--session-id <뽑은 값>` 으로 떠 산다.
///      결말은 `Ok`·Running 이고 1 의 실패 기록이 지워진다. 뽑은 값은 **첫 제출(CR) 전에는** 명부에도
///      디스크에도 없고, CR 이 든 첫 입력 뒤에야 둘 다에 앉는다.
/// ★가짜가 실제로 떴는지를 인자 기록으로 먼저 본다★ — 안 떴으면 아래 「영속 안 됨」 단언이 공회전한다.
// ADR-0226
#[test]
fn d1_a_fresh_claude_persists_its_minted_id_only_after_the_first_submission() {
    let (manager, _sink, profiles, store) =
        make_manager_with_store("d1-first-submission", Arc::new(NoopControlChannel));
    let (fake_dir, args_log) = fake_claude_dir("d1");
    let (mut profile, cwd) = claude_on_fake_path("d1-claude", &fake_dir);
    let garbage = Uuid::new_v4();
    profile.backend_session_id = Some(garbage);
    let id = profile.id;
    profiles.upsert(profile.clone());

    // 1. 쓰레기 id 로 이어받기 — Phase A 에서는 실패로 끝난다.
    let resumed = manager.activate_profile(&profile, SpawnMode::Resume);
    assert!(
        resumed.is_err(),
        "대화 없는 id 의 이어받기가 성공으로 보고됐다: {resumed:?}"
    );
    assert!(
        wait_until(Duration::from_secs(10), || manager.list_agents().is_empty()),
        "죽은 이어받기 화신이 수거되지 않았다"
    );
    let spawns = fake_spawns(&args_log);
    assert_eq!(
        spawns.len(),
        1,
        "가짜 claude 가 정확히 한 번 떴어야 한다(PATH 가 안 먹었으면 0): {spawns:?}"
    );
    assert_eq!(
        uuid_after(&spawns[0], "--resume"),
        Some(garbage),
        "저장된 id 로 이어받지 않았다: {spawns:?}"
    );
    assert_eq!(
        profiles.get(id).and_then(|p| p.backend_session_id),
        Some(garbage),
        "Phase A 의 이어받기 실패는 손잡이를 건드리지 않는다"
    );

    // 2. 같은 프로필의 Fresh.
    let info = manager
        .activate_profile(&profile, SpawnMode::Fresh)
        .expect("Fresh 활성화");
    assert!(
        matches!(info.status, AgentStatus::Running),
        "Fresh 결말이 Running 이 아니다: {:?}",
        info.status
    );
    // 연결 축이 없는 통로의 Fresh 는 자식이 뜬 순간 돌아오므로 가짜가 인자를 남길 때까지 기다린다.
    wait_until(Duration::from_secs(10), || {
        fake_spawns(&args_log).len() >= 2
    });
    let spawns = fake_spawns(&args_log);
    assert_eq!(
        spawns.len(),
        2,
        "Fresh 가 가짜를 한 번 더 띄웠어야 한다: {spawns:?}"
    );
    let minted = uuid_after(&spawns[1], "--session-id")
        .unwrap_or_else(|| panic!("Fresh 가 `--session-id <uuid>` 로 뜨지 않았다: {spawns:?}"));
    assert_ne!(minted, garbage, "Fresh 가 옛 id 를 재사용했다(ADR-0076)");

    let got = profiles.get(id).expect("프로필");
    assert_eq!(
        got.backend_session_id, None,
        "제출 전인데 뽑은 id 가 명부에 적혔다"
    );
    assert!(
        got.old_session_ids.contains(&garbage),
        "쓰레기 id 가 이력으로 가지 않았다"
    );
    assert_eq!(
        got.last_failure, None,
        "Fresh 성립이 앞선 실패 기록을 지우지 않았다"
    );
    assert_eq!(
        stored_on_disk(&store, id),
        None,
        "제출 전인데 디스크에 id 가 있다"
    );

    // CR 없는 키 입력은 턴을 열지 않는다.
    manager.write_stdin(id, b"hello").expect("키 입력");
    assert_eq!(
        profiles.get(id).and_then(|p| p.backend_session_id),
        None,
        "CR 없는 입력이 제출로 세어졌다"
    );

    // 첫 제출 — 이 호출이 돌아온 뒤에는 명부와 디스크 둘 다에 앉아 있어야 한다.
    manager.write_stdin(id, b"\r").expect("제출");
    assert_eq!(
        profiles.get(id).and_then(|p| p.backend_session_id),
        Some(minted),
        "첫 제출 뒤에도 뽑은 id 가 명부에 없다 — 래치가 세션에 안 실렸거나 offer 가 빠졌다"
    );
    assert_eq!(
        stored_on_disk(&store, id),
        Some(minted),
        "첫 제출 뒤에도 뽑은 id 가 디스크에 없다"
    );
    // 스폰 시점 스냅샷의 Running 은 연결 축 없는 통로에서 늘 참이라, 가짜가 실제로 살아 있는지를 따로 본다.
    assert!(
        manager
            .list_agents()
            .iter()
            .any(|a| a.id == id && matches!(a.status, AgentStatus::Running)),
        "Fresh 화신이 입력 뒤에 살아 있지 않다 — 가짜의 `--session-id` 갈래가 죽었다"
    );

    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(10), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&fake_dir);
    let _ = std::fs::remove_dir_all(&cwd);
}

/// ★손잡이 없는 claude 이어받기 요청은 새 대화로 연다 — 사용자 결정 D1 의 귀결(ADR-0226)★.
///
/// 결말은 `Ok`·Running 이고 가짜는 `--resume` 없이 `--session-id` 로 뜬다. 뽑은 값은 제출 전이라
/// 영속되지 않는다 — 옛 길(없으면 뽑아 영속하고 `--resume <새 값>` 으로 띄움)은 대화 없는 id 를 남기고 죽었다.
// ADR-0226
#[test]
fn a_handleless_claude_resume_opens_a_new_conversation_without_persisting() {
    let (manager, _sink, profiles, store) =
        make_manager_with_store("handleless-resume", Arc::new(NoopControlChannel));
    let (fake_dir, args_log) = fake_claude_dir("handleless");
    let (profile, cwd) = claude_on_fake_path("handleless-claude", &fake_dir);
    let id = profile.id;
    profiles.upsert(profile.clone());

    let info = manager
        .activate_profile(&profile, SpawnMode::Resume)
        .expect("손잡이 없는 이어받기 요청은 새 대화로 선다");
    assert!(
        matches!(info.status, AgentStatus::Running),
        "결말이 Running 이 아니다: {:?}",
        info.status
    );

    wait_until(Duration::from_secs(10), || {
        !fake_spawns(&args_log).is_empty()
    });
    let spawns = fake_spawns(&args_log);
    assert_eq!(
        spawns.len(),
        1,
        "가짜가 정확히 한 번 떴어야 한다: {spawns:?}"
    );
    assert!(
        uuid_after(&spawns[0], "--session-id").is_some() && !spawns[0].contains("--resume"),
        "새 대화 argv 가 아니다: {spawns:?}"
    );
    assert_eq!(profiles.get(id).and_then(|p| p.backend_session_id), None);
    assert_eq!(stored_on_disk(&store, id), None, "대화 없는 id 가 영속됐다");
    assert_eq!(profiles.get(id).and_then(|p| p.last_failure), None);

    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(10), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&fake_dir);
    let _ = std::fs::remove_dir_all(&cwd);
}
