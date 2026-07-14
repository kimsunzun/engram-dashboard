//! 수동 활성화(activate_profile) 통합테스트 — ADR-0082(fresh-fallback 폐지, 이어받기 전용).
//!
//! 배경(번복된 결정): ADR-0076/0077 은 "resume 조기종료 → fresh-fallback(새 대화 자동 생성)" 이었다.
//! ADR-0082 가 이를 폐지했다 — resume 실패/조기종료는 **아무것도 kill·재spawn 하지 않고** Failed(시체)
//! 종점으로 남기고 원인을 로그로 남긴다(LLM 이 읽어 에스컬레이션). 또한 산 에이전트 재활성화는
//! 무해한 "이미 실행 중" 신호를 돌려주고 산 에이전트를 절대 건드리지 않는다(a4aac1a 회귀 수정).
//!
//! 이 파일은 그 두 결정을 실 spawn 으로 결정적으로 단언한다:
//!   ① resume 조기종료 → Failed(Err), 자동 fresh 없음, epoch 불변, 단 1회만 spawn, 프로필은
//!      시체로 보존(삭제 아님, auto_restore=true→false 다운그레이드).
//!   ② 이미 실행 중 재활성화 → 원본 세션 생존·kill 안 됨·epoch 불변·재spawn 없음(run-count 불변),
//!      무해한 AgentInfo 반환.
//!
//! ★실 claude 없이 조기종료를 결정적으로 모사★: 실 claude 를 CI/단위에서 못 띄우므로, resume 자리
//!   (첫 spawn)가 `exit 1` 로 조기종료하는 셸 프로필을 쓴다. 옛 fresh-fallback 이 살아 있었다면
//!   둘째 spawn(fresh 자리)이 일어났겠지만, ADR-0082 에선 둘째 spawn 이 아예 없어야 한다 — 이를
//!   run-count 파일(매 실행 append)로 "정확히 1회 spawn" 을 단언해 fresh-fallback 부재를 증명한다.
//!
//! Windows 전용(cmd.exe 배치)이라 #[cfg(windows)]. 단일 spawn·전역 경합 없음 → default.

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

/// agent_list_updated·status 전이를 세는 경량 sink(reaper.rs CountingSink 동형).
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

/// 테스트용 manager 구성(reaper.rs make_manager 동형, tag 로 store 격리). 세션 추적 비활성.
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

/// "매 실행마다 run-count 파일에 한 줄 append 후 즉시 `exit 1`" 하는 배치 프로필을 만든다.
/// 반환: (프로필, batch 경로, count 경로).
///
/// ★목적(ADR-0082 fresh-fallback 부재 증명)★: 이 배치는 언제 실행돼도 조기종료(exit 1)한다.
///   옛 fresh-fallback 이 살아 있었다면 resume 조기종료 후 둘째 spawn(fresh 자리)이 일어나
///   count 가 2 가 됐을 것이다. ADR-0082 에선 fresh 재spawn 이 없어 **정확히 1회**만 실행된다.
///
/// ★왜 인라인 복합 cmd 가 아니라 배치 파일인가★: portable-pty CommandBuilder 가 `>`·`&` 를 개별
///   quoting 해 ConPTY 통과 중 깨뜨린다(옛 test 실측). 배치는 cmd 가 직접 파싱하니 결정적이다.
fn always_early_exit_profile(tag: &str) -> (AgentProfile, PathBuf, PathBuf) {
    let uniq = Uuid::new_v4();
    let count = std::env::temp_dir().join(format!("engram-activate-count-{tag}-{uniq}.tmp"));
    let batch = std::env::temp_dir().join(format!("engram-activate-exit-{tag}-{uniq}.cmd"));

    // 매 실행: count 파일에 "x" 한 줄 append(>>) 후 exit 1(비정상 조기종료 = resume 실패 모사).
    let script = format!(
        "@echo off\r\n\
         echo x>>\"{c}\"\r\n\
         exit /b 1\r\n",
        c = count.display()
    );
    std::fs::write(&batch, script).expect("배치 파일 write");

    // ★auto_restore=true 로 시작★: reaper 가 조기종료를 KeepDisableAutoRestore 로 판정해
    //   **프로필을 지우지 않고 auto_restore 를 false 로 내리는지**(=시체로 보존) 를 아래 테스트가
    //   단언한다. false 로 태어나면 다운그레이드가 no-op 이라 그 계약을 증명 못 한다 → true 로 둔다.
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

/// count 파일의 실행 횟수(append 된 줄 수)를 센다. 없으면 0.
fn run_count(path: &PathBuf) -> usize {
    std::fs::read_to_string(path)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

/// 윈도(3s) 넘게 사는 배치 프로필(재활성화 가드 테스트용 산 에이전트). ping 으로 ≈19s 생존.
/// 반환: (프로필, batch 경로, count 경로).
///
/// ★매 start 마다 count 파일에 한 줄 append★(always_early_exit_profile 과 동형): 재활성화가
///   산 에이전트를 **재spawn 하지 않음**을 count 로 증명한다. count 는 ping(≈19s) **전**에 쓰므로
///   start 직후 즉시 1 이 된다. kill+replace(같은 epoch 로 위장) 회귀가 있었다면 count 가 2 가 된다.
fn long_lived_profile(tag: &str) -> (AgentProfile, PathBuf, PathBuf) {
    let uniq = Uuid::new_v4();
    let count = std::env::temp_dir().join(format!("engram-activate-live-count-{tag}-{uniq}.tmp"));
    let batch = std::env::temp_dir().join(format!("engram-activate-live-{tag}-{uniq}.cmd"));
    // 매 실행: count 에 "x" append(>>) → ping 20회(≈19s, 조기종료 윈도 3s 초과 생존).
    //   append 를 ping 앞에 둬 start 직후 count 가 즉시 반영되게 한다(테스트가 곧바로 읽음).
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

/// ★ADR-0082 핵심 ①★: resume spawn 이 조기종료하는 프로필을 activate_profile(Resume)로 활성화하면
/// **fresh-fallback 없이** Failed(Err)로 끝난다 — 자동으로 새 대화를 만들지 않는다. 검증:
///   (a) activate_profile 이 Err 반환(Failed 시체 → Err 번역).
///   (b) 배치가 **정확히 1회**만 실행됨(fresh 재spawn 이 있었다면 2회 — fresh-fallback 부재 증명).
///   (c) epoch 불변(0) — fallback 이 없앤 epoch++ 가 일어나지 않음.
///   (d) claude_session_id/old_session_ids 불변 — 새 sid 발급(fresh) 없음.
///   (e) 프로필(시체) 생존 — reaper 가 삭제하지 않음(KeepDisableAutoRestore).
///   (f) auto_restore=true→false 다운그레이드 — 삭제가 아니라 "시체로 보존"임을 결정적으로 단언.
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

    // activate_profile(Resume): resume 자리 spawn → exit 1(조기종료) → resume_no_fallback 이
    //   early_terminal_status 로 감지 → Failed → Err. 둘째(fresh) spawn 은 없어야 한다.
    let result = manager.activate_profile(&profile, SpawnMode::Resume);

    // (a) Failed 종점 → Err(자동 fresh 로 살아남지 않는다).
    assert!(
        result.is_err(),
        "resume 조기종료는 fresh-fallback 없이 Err(Failed 시체)여야 함 — got Ok: {result:?}"
    );

    // reaper 가 조기종료 세션을 수거해 맵에서 사라질 때까지 잠깐 대기(비동기).
    let _ = wait_until(Duration::from_secs(5), || {
        !manager.list_agents().iter().any(|a| a.id == id)
    });

    // (b) 배치가 정확히 1회만 실행됨 — fresh 재spawn 이 없다는 결정적 증거.
    //     (옛 fresh-fallback 이었다면 fresh 자리에서 한 번 더 spawn 돼 2가 됐을 것.)
    assert_eq!(
        run_count(&count),
        1,
        "resume 자리 1회만 실행돼야 함(fresh-fallback 이 없어야 함) — got {}",
        run_count(&count)
    );

    // (c) ★ADR-0084 갱신★: 재활성화(Resume)는 성패와 무관하게 진입 시 epoch 를 bump 한다(맵 교체
    //     불변식 — activate_profile Resume 갈래). resume 이 조기종료로 Failed 가 돼도 그 사이 새 세션이
    //     맵에 insert→reap 됐으므로 epoch++ 가 옳다. 옛 ADR-0082 가정(epoch 불변=0)은 폐기됐다.
    //     (fresh-fallback 이 하던 "kill 후 fresh 재spawn 시 bump" 와는 다른 경로 — 여기선 재활성화 진입
    //      자체가 bump 주체이고, 자동 fresh 재생성은 여전히 없다(위 run-count==1 로 별도 단언).)
    assert_eq!(
        profiles.get(id).map(|p| p.epoch),
        Some(1),
        "재활성화(Resume) 진입은 epoch 를 0→1 로 bump 해야 함(ADR-0084 맵 교체 불변식)"
    );

    // (d) sid 이력 불변 — 새 sid 발급(fresh)이 없어 claude_session_id/old_session_ids 가 그대로.
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

    // (e) ★시체로 보존 — 프로필이 삭제되지 않는다★. reaper 가 조기종료(exit≠0, intent=None)를
    //     KeepDisableAutoRestore 로 판정하므로 프로필이 살아남아야 한다(fresh-fallback 폐지의 헤드라인
    //     "시체 보존"의 결정적 단언 — 이제 reaper 유닛테스트에만 기대지 않는다).
    assert!(
        profiles.get(id).is_some(),
        "resume 실패 후 프로필(시체)이 삭제되면 안 됨 — KeepDisableAutoRestore 로 보존돼야 함"
    );

    // (f) ★삭제가 아니라 다운그레이드★. auto_restore=true 로 태어난 프로필이 조기종료 수거로
    //     false 로 내려가야 한다(부팅 복원 대상에서 빠짐). 프로필이 지워졌거나 그대로 true 면 실패.
    assert_eq!(
        profiles.get(id).map(|p| p.auto_restore),
        Some(false),
        "resume 실패 시체는 auto_restore=false 로 다운그레이드돼야 함(삭제 아님)"
    );

    // 정리.
    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// ★ADR-0082 핵심 ②★: 이미 실행 중인 에이전트를 재활성화하면 산 에이전트를 **절대 건드리지 않고**
/// 무해한 AgentInfo(이미 실행 중 신호)를 돌려준다. 검증:
///   (a) 재활성화가 Ok(AgentInfo) 반환.
///   (b) 원본 세션이 kill 되지 않고 목록에 그대로 살아 있음(종점 상태 아님).
///   (c) epoch 불변 — 맵 교체(fresh)가 일어나지 않음(a4aac1a 회귀의 핵심 신호).
///   (d) ★run-count==1 (재활성화 전후 불변)★ — 배치가 딱 1회만 start 됨을 단언한다. 재활성화가
///       산 에이전트를 **재spawn 하지 않음**의 결정적 증거: 같은 epoch 로 위장한 kill+replace 회귀가
///       있었다면 배치가 두 번째로 start 돼 count 가 2 가 됐을 것이다(epoch 검사만으론 못 잡는 구멍).
#[test]
fn reactivate_running_agent_leaves_it_alive_epoch_unchanged() {
    let (manager, _sink, profiles) = make_manager("reactivate-live");

    let (profile, batch, count) = long_lived_profile("reactivate-live");
    let id = profile.id;
    profiles.upsert(profile.clone());

    // 최초 활성화(세션 없음 → Fresh 갈래) → 오래 사는 세션 spawn.
    let first = manager
        .activate_profile(&profile, SpawnMode::Fresh)
        .expect("최초 활성화는 Ok(살아있는 세션)여야 함");
    let epoch_after_first = first.epoch;

    // 세션이 실제로 살아 목록에 잡힐 때까지 대기(조기종료 아님을 확인).
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

    // (d-전) ★배치가 딱 1회 start★. 첫 활성화로 배치가 count 에 한 줄 append 했다. 배치의 append 는
    //   PTY 스폰 프로세스라 약간 지연될 수 있으니 1 에 도달할 때까지 대기한 뒤, 정확히 1 인지 단언한다.
    assert!(
        wait_until(Duration::from_secs(3), || run_count(&count) == 1),
        "최초 활성화로 배치가 1회 start 돼야 함 — got {}",
        run_count(&count)
    );

    // ★재활성화★: 같은 프로필을 다시 활성화(Resume 요청). 산 에이전트를 건드리면 안 된다.
    //   옛 회귀에선 여기서 이중-spawn 가드 Err → fresh-fallback → 산 세션 kill → epoch++ 였다.

    // [추가 하드닝 ①] 재활성화 전 레지스트리 epoch 를 먼저 읽어둔다 — 재활성화 후 레지스트리 epoch
    //   도 불변임을 단언한다(반환된 AgentInfo.epoch 만 보면 레지스트리 맵 교체를 못 잡는다).
    let reg_epoch_before = profiles.get(id).map(|p| p.epoch);

    let reactivated = manager
        .activate_profile(&profile, SpawnMode::Resume)
        .expect("재활성화는 무해한 Ok(이미 실행 중 AgentInfo)여야 함 — 죽으면 Err/회귀");

    // [추가 하드닝 ②] activate_profile 은 산 에이전트가 있을 때 재spawn 없이 동기 반환한다.
    //   재spawn(회귀)이 있었다면 새 배치 프로세스가 count 파일에 append 해 2가 된다. spawn 은 동기지만
    //   배치 실행(append)은 비동기이므로, 넉넉한 창(2s)을 주고 count 가 2로 오르지 '않음'을 확인한다.
    //   (결정적 축은 위의 epoch 불변; 이 count 는 보조 heuristic — 비동기 이벤트의 '부재'는 창으로 확인.)
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

    // (a)(c) 반환된 info 가 산 세션 그대로: id 동일 + epoch 불변(맵 교체 없음).
    assert_eq!(reactivated.id, id, "재활성화는 같은 에이전트를 가리켜야 함");
    assert_eq!(
        reactivated.epoch, epoch_after_first,
        "재활성화로 epoch 가 bump 되면 안 됨(맵 교체=fresh 없음, a4aac1a 회귀 신호)"
    );
    assert!(
        !matches!(
            reactivated.status,
            AgentStatus::Failed { .. } | AgentStatus::Killed | AgentStatus::Exited { .. }
        ),
        "재활성화 후 산 에이전트가 종점 상태면 파괴된 것 — 살아있어야 함: {:?}",
        reactivated.status
    );

    // [추가 하드닝 ①] 레지스트리 epoch 도 불변이어야 한다 — 반환된 AgentInfo.epoch 만 보면
    //   레지스트리 맵 교체(epoch 내부 bump)를 잡지 못한다.
    assert_eq!(
        profiles.get(id).map(|p| p.epoch),
        reg_epoch_before,
        "재활성화로 레지스트리 epoch 가 bump 되면 안 됨(맵 교체=fresh 없음)"
    );

    // (b) 잠깐 뒤에도 원본 세션이 여전히 살아 목록에 있음(fresh-fallback 이 kill 하지 않았다).
    //     epoch 도 여전히 동일해야 한다(비동기 reaper 가 옛 세션을 수거하지 않았다).
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

    // (d-후) ★재spawn 없음의 결정적 증거★: 재활성화 뒤에도 배치는 여전히 1회만 start 됐다.
    //   재활성화가 산 에이전트를 kill+replace(같은 epoch 로 위장) 했다면 배치가 두 번째로 start 돼
    //   count 가 2 가 됐을 것이다 — epoch 검사만으론 못 잡는 구멍을 이 count 가 닫는다.
    assert_eq!(
        run_count(&count),
        1,
        "재활성화는 산 에이전트를 재spawn 하면 안 됨(배치 start 는 여전히 1회여야 함) — got {}",
        run_count(&count)
    );

    // 정리.
    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// ★ADR-0084 핵심★: 죽은 시체를 같은 슬롯에서 재활성화(Resume)하면 **epoch 가 엄격히 증가**한다.
/// reap 으로 세션이 맵에서 빠졌다가 재활성화로 새 세션이 같은 AgentId 로 들어오는 건 맵 교체이므로
/// ADR-0007("같은 AgentId 맵 교체마다 epoch +1")를 적용한다. epoch 가 안 오르면 프론트 구독
/// (deps [viewId,agentId,epoch])이 재발화하지 않아 resume 출력이 화면에 안 붙는다(빈 슬롯).
///
/// ★step 1 이 없으면(bump_epoch 미호출) 이 테스트는 실패한다★: spawn_agent 은 프로필 epoch 를 읽기만
///   하므로, bump 가 없으면 재활성화된 새 세션이 죽은 세션과 동일 epoch(E)를 갖는다 → 아래 `> E` 단언 실패.
///
/// 실 claude 없이 셸로 모사: 셸은 needs_session=false 라 --resume 플래그를 붙이진 않지만(그건 별도
///   backend 단위 테스트가 실증), 이 테스트가 겨냥하는 "재활성화 = 맵 교체 = epoch++" 는 backend
///   무관하게 성립한다. 셸은 조기종료하지 않아 Resume 재활성화가 Ok(살아있는 세션)로 성공한다.
#[test]
fn reactivate_after_kill_bumps_epoch() {
    let (manager, _sink, profiles) = make_manager("reactivate-epoch-bump");

    // 윈도(EARLY_EXIT_WINDOW)를 넘겨 사는 셸(≈19s) — 재활성화 시 조기종료로 오판되지 않게 한다.
    let (profile, batch, count) = long_lived_profile("reactivate-epoch-bump");
    let id = profile.id;
    profiles.upsert(profile.clone());

    // 1) 최초 활성화(세션 없음 → Fresh 갈래) → 오래 사는 셸 spawn. epoch=E(신규 프로필이라 0).
    let first = manager
        .activate_profile(&profile, SpawnMode::Fresh)
        .expect("최초 활성화는 Ok(살아있는 세션)여야 함");
    let epoch_e = first.epoch;
    assert_eq!(
        epoch_e, 0,
        "신규 프로필 첫 spawn(Fresh)은 epoch=0(재활성화 아님 → bump 없음)"
    );
    assert!(
        wait_until(Duration::from_secs(3), || {
            manager.list_agents().iter().any(|a| a.id == id)
        }),
        "최초 활성화 세션이 살아있어야 함"
    );

    // 2) 유저 kill → reaper 가 세션을 맵에서 수거(시체 보존, ADR-0083). 프로필 epoch 는 아직 E.
    manager.kill_agent(id).expect("kill_agent failed");
    assert!(
        wait_until(Duration::from_secs(5), || {
            !manager.list_agents().iter().any(|a| a.id == id)
        }),
        "유저 kill 후 세션이 맵에서 수거돼야 함"
    );
    // kill 수거 완료(맵 비었고 프로필은 시체) 확인 — epoch 는 여전히 E(재활성화 전).
    assert_eq!(
        profiles.get(id).map(|p| p.epoch),
        Some(epoch_e),
        "kill 만으로는 epoch 가 오르지 않는다(재활성화 respawn 이 bump 의 주체)"
    );

    // 3) ★재활성화(Resume)★: 시체를 같은 슬롯에서 재spawn = 맵 교체 → epoch++ 여야 한다.
    let reactivated = manager
        .activate_profile(&profile, SpawnMode::Resume)
        .expect("재활성화가 resume 경로로 Ok(살아있는 세션)여야 함(셸은 조기종료 안 함)");

    // (a) 반환된 산 세션의 epoch 가 죽은 세션(E)보다 엄격히 크다(맵 교체 재구독 트리거, ADR-0007).
    assert!(
        reactivated.epoch > epoch_e,
        "ADR-0084: 재활성화 세션 epoch({}) 가 죽은 세션 epoch({}) 보다 커야 함(맵 교체=epoch++)",
        reactivated.epoch,
        epoch_e
    );
    // (b) 레지스트리 epoch 도 함께 올랐다(반환 info 만 보면 맵 교체를 못 잡는다).
    assert_eq!(
        profiles.get(id).map(|p| p.epoch),
        Some(reactivated.epoch),
        "재활성화 후 레지스트리 epoch 와 세션 epoch 가 일치해야 함(bump 가 spawn_agent 읽기 전에 반영)"
    );

    // 정리.
    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}

/// ★ADR-0083 회귀★: 유저 kill 후 재활성화가 "profile not found"(=화면 "실패")로 깨지던 버그를 막는다.
/// 옛 동작: 유저 kill → reaper `(UserKill,_) => DeleteProfile` → `profiles.remove`(claude_session_id
/// 포함 삭제) → 재활성화 시 프로필이 없어 resume 진입도 못 하고 실패. ADR-0083 은 유저 kill 도 시체로
/// 보존하므로, kill 후에도 프로필 + claude_session_id 가 남아 재활성화가 resume 경로로 정상 진입한다.
///
/// 검증(★이 테스트가 증명하는 건 "프로필 조회 경로가 온전함"까지다 — 실제 `--resume <sid>` 조립은
/// backend/claude.rs 의 `build_command_spec(Resume, sid)` 단위 테스트가 실증한다. ADR-0084 로 그
/// 백엔드 단위 테스트가 추가돼 이 통합 테스트가 --resume 조립을 오버셀할 필요가 없어졌다):
///   (a) 유저 kill 후 세션은 맵에서 수거되지만 프로필은 보존되고 auto_restore=false 로 다운그레이드.
///   (b) claude_session_id 가 그대로 남아 있다(--resume 로 이어받기 위한 필수 조건 — 보존만 단언).
///   (c) `activate_profile(Resume)` 가 "profile not found" 없이 **프로필 조회를 통과해** 재활성화된다
///       — 셸은 조기종료하지 않으므로 Ok(살아있는 세션). 즉 이 테스트의 결정적 단언은 "삭제로 조회
///       경로가 깨지지 않았다"이지, 셸이 실제 --resume 를 부착한다는 게 아니다(셸은 needs_session=false).
#[test]
fn user_kill_then_reactivate_finds_profile_and_resumes() {
    let (manager, _sink, profiles) = make_manager("kill-reactivate");

    // 윈도(EARLY_EXIT_WINDOW)를 넘겨 사는 셸(≈19s) — 재활성화 시 조기종료로 오판되지 않게 한다.
    let (profile, batch, count) = long_lived_profile("kill-reactivate");
    let id = profile.id;

    // ★claude_session_id 를 심은 프로필★: 유저 kill 후에도 이 sid 가 살아남아야 재활성화 resume 가
    //   성립한다(ADR-0083 의 헤드라인 — 시체 + sid 보존). auto_restore=true 로 둬서 kill 수거가 false 로
    //   다운그레이드하는지도 함께 단언한다. ★spawn 경로가 upsert_preserving_hierarchy 로 넘긴 프로필을
    //   그대로 레지스트리에 심으므로(session_id 포함), activate/kill 에도 이 seeded 프로필을 써야 sid 가
    //   보존된다★(원본 profile 은 claude_session_id=None 이라 그걸 넘기면 spawn 이 sid 를 덮어써 유실).
    let sid = Uuid::new_v4();
    let mut seeded = profile.clone();
    seeded.claude_session_id = Some(sid);
    seeded.auto_restore = true;
    profiles.upsert(seeded.clone());

    // 1) 최초 활성화(세션 없음 → Fresh 갈래) → 오래 사는 셸 spawn.
    manager
        .activate_profile(&seeded, SpawnMode::Fresh)
        .expect("최초 활성화는 Ok(살아있는 세션)여야 함");
    assert!(
        wait_until(Duration::from_secs(3), || {
            manager.list_agents().iter().any(|a| a.id == id)
        }),
        "최초 활성화 세션이 살아있어야 함"
    );

    // 2) 유저 kill(UserKill intent 태깅) → reaper 가 세션 수거 + 시체 보존(ADR-0083).
    manager.kill_agent(id).expect("kill_agent failed");
    assert!(
        wait_until(Duration::from_secs(5), || {
            !manager.list_agents().iter().any(|a| a.id == id)
        }),
        "유저 kill 후 세션이 맵에서 수거돼야 함"
    );

    // (a) 프로필 보존 + auto_restore=false 다운그레이드(삭제 아님).
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
    // (b) claude_session_id 보존 — 재활성화 resume 의 필수 조건.
    assert_eq!(
        profiles.get(id).and_then(|p| p.claude_session_id),
        Some(sid),
        "유저 kill 로 claude_session_id 가 유실됨 — 재활성화 resume 불가(ADR-0083 회귀)"
    );

    // (c) ★재활성화가 "profile not found" 없이 resume 경로로 진입★. 프로필이 살아 있으므로 조회 경로가
    //   온전하고, 셸은 조기종료하지 않아 Ok(살아있는 세션)로 재활성화된다. 옛 버그였다면 프로필이 없어
    //   재활성화가 실패(진입 불가)했을 것이다.
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

    // 정리.
    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&count);
    let _ = std::fs::remove_file(&batch);
}
