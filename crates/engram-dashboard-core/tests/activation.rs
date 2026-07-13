//! ④ 수동 활성화 fresh-fallback 통합테스트 — activate_profile(Resume)이 조기종료 시
//!   fresh-fallback 으로 새 세션을 살려내는지 실 spawn 으로 단언 검증(ADR-0076).
//!
//! 배경(재현 버그): 이어받을 수 없는 세션(빈/미대화/손상 — 실 claude 는 "No conversation found
//! with session ID ..." 로 즉사)을 활성화하면 resume spawn 이 조기종료하는데, 예전엔 fresh-fallback
//! 이 **부팅 복원(restore_one) 경로에만** 있어 수동 활성화(SpawnProfile→spawn_agent)는 그냥 죽었다.
//! 이 fix 로 activate_profile 이 restore_one 과 동일한 resume→조기종료→fresh-fallback 규율을 공유한다.
//!
//! ★실 claude 없이 조기종료를 결정적으로 모사★: 실 claude 를 CI/단위에서 못 띄우므로, "첫 spawn
//! (resume 자리)은 즉시 종료, 둘째 spawn(fresh-fallback 자리)은 오래 산다" 를 marker 파일로 만든다.
//!   1st spawn(resume): marker 없음 → marker 생성 후 `exit 1`(조기종료 = resume 실패 신호).
//!   2nd spawn(fresh):  marker 있음 → `ping` 으로 윈도(3s) 넘게 생존 → fresh-fallback 성공.
//! 이렇게 하면 activate_profile → resume_with_fresh_fallback → fallback_fresh → 생존 fresh 세션의
//! **실제 프로덕션 배선**을 그대로 탄다(모드 분기·early_terminal_status·remove_session·respawn 포함).
//! sid 발급(옛→새 uuid, old_session_ids push)은 claude 전용이라 여기선 검증 대상이 아니고,
//! profile.rs 의 new_session_id 단위테스트(ADR-0076)가 그 인과를 결정적으로 보장한다.
//!
//! Windows 전용 marker 스크립트(cmd.exe)라 #[cfg(windows)]. 단일 spawn·전역 경합 없음 → default.

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

/// "1회차는 즉시 exit 1, 2회차부터는 오래 생존" 하는 marker .cmd 배치 파일을 디스크에 쓰고,
/// 그걸 `cmd /c <batch>` 로 실행하는 프로필을 만든다. 반환: (프로필, batch 경로, marker 경로).
///
/// ★왜 인라인 `cmd /c "if...else..."` 가 아니라 배치 파일인가★: portable-pty 의 CommandBuilder 는
///   args 를 개별로 quoting 해 ConPTY 에 넘기므로, `&`·`>`·괄호가 섞인 복합 한 줄은 통과 중 깨진다
///   (실측: marker 미생성). 배치 파일은 cmd 가 파일을 직접 파싱하니 quoting 우려가 없어 결정적이다.
///
/// batch 로직: marker 있으면 `ping -n 20`(≈19s 생존 = 조기종료 윈도 3s 초과), 없으면 marker 생성 후
///   `exit 1`(비정상 조기종료 = resume 실패 모사). 첫 실행(resume 자리)=exit1, 둘째(fresh 자리)=생존.
fn marker_flip_profile(tag: &str) -> (AgentProfile, PathBuf, PathBuf) {
    let uniq = Uuid::new_v4();
    let marker = std::env::temp_dir().join(format!("engram-activate-marker-{tag}-{uniq}.tmp"));
    let batch = std::env::temp_dir().join(format!("engram-activate-flip-{tag}-{uniq}.cmd"));

    // 배치 내용: @echo off + marker 존재 분기. marker 는 절대경로라 실행 cwd 와 무관.
    let script = format!(
        "@echo off\r\n\
         if exist \"{m}\" (\r\n\
         ping -n 20 127.0.0.1 >nul\r\n\
         ) else (\r\n\
         type nul > \"{m}\"\r\n\
         exit /b 1\r\n\
         )\r\n",
        m = marker.display()
    );
    std::fs::write(&batch, script).expect("배치 파일 write");

    let profile = AgentProfile::new(
        "activate-flip".into(),
        AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), batch.to_string_lossy().to_string()],
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    (profile, batch, marker)
}

/// ★핵심 회귀★: resume spawn 이 조기종료하는 프로필을 activate_profile(Resume)로 활성화하면,
/// fresh-fallback 이 자동으로 새 세션을 respawn 해 **살아남는다**(죽지 않는다). 예전엔 수동 활성화
/// 경로에 fallback 이 없어 그대로 죽었다(이 세션의 재현 버그).
#[test]
fn activate_resume_early_exit_falls_back_to_surviving_fresh() {
    let (manager, _sink, profiles) = make_manager("resume-fallback");

    // 회차 구분 marker + 배치 파일(테스트별 unique — 잔여 오염 방지). marker 는 아직 없어야
    // 1회차(resume 자리)가 exit 1 분기로 간다(marker_flip_profile 이 새 uniq 로 만들어 보장).
    let (profile, batch, marker) = marker_flip_profile("resume-fallback");
    let id = profile.id;
    profiles.upsert(profile.clone());

    // activate_profile(Resume): 1회차(resume 자리) spawn → marker 생성 + exit 1(조기종료) →
    //   resume_with_fresh_fallback 이 early_terminal_status 로 감지 → fallback_fresh →
    //   2회차(fresh 자리) spawn → marker 있음 → ping 으로 생존 → AgentInfo(Ok) 반환.
    let info = manager
        .activate_profile(&profile, SpawnMode::Resume)
        .expect(
            "activate_profile 은 fresh-fallback 으로 살아남아 Ok(AgentInfo)여야 함(죽으면 Err)",
        );

    // (a) 반환된 세션이 살아있다 — 종점(Failed/Killed/Exited)이 아니라 Running/Started 여야 한다.
    //     fresh-fallback 이 안 돌았다면 여기서 세션 자체가 없거나 종점이었을 것.
    assert!(
        !matches!(
            info.status,
            AgentStatus::Failed { .. } | AgentStatus::Killed | AgentStatus::Exited { .. }
        ),
        "fresh-fallback 세션이 종점 상태 — 살아있어야 함: {:?}",
        info.status
    );

    // (b) 맵에 살아있는 세션이 계속 존재한다(조기종료 자리 세션이 아니라 respawn 된 fresh 세션).
    //     짧게 폴링해 respawn 직후 리스트에 잡히는지 확인(hang 없음).
    assert!(
        wait_until(Duration::from_secs(2), || {
            manager
                .list_agents()
                .iter()
                .any(|a| a.id == id && !matches!(a.status, AgentStatus::Failed { .. }))
        }),
        "활성화 후 살아있는 세션이 목록에 없음 — fresh-fallback 실패(죽음)"
    );

    // (c) marker 가 생성됐다 = resume 자리(1회차)가 실제로 spawn 돼 조기종료를 거쳤다는 증거.
    //     (fallback 없이 resume 만 성공했다면 이 marker 는 여전히 없었을 것 = 검증 유효성 담보.)
    assert!(
        marker.exists(),
        "marker 미생성 — resume 자리 spawn 이 조기종료 경로를 타지 않음(테스트 전제 붕괴)"
    );

    // 정리: 세션 kill + marker/배치 삭제(다음 실행 오염 방지).
    let _ = manager.kill_agent(id);
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_file(&marker);
    let _ = std::fs::remove_file(&batch);
}

// NB: "resume·fresh 둘 다 실패 → 종점 Failed → Err" 브랜치는 여기서 단언하지 않는다.
// fallback_fresh 는 fresh **spawn 자체가 Err** 일 때만 Failed 를 돌려준다 — fresh 가 spawn 은
// 성공하고 곧바로 조기종료하는 경우(shell `exit 1`)는 FreshFallback(Started)로 보고되며, 그때
// agent_info_by_id 는 reaper 의 세션 제거와 race 한다(NotFound 가 될 수도, 안 될 수도). shell 로
// "spawn 자체 실패"를 결정적으로 만들기 어렵고 이 race 가 flaky 를 유발하므로, Err 종점은
// spawn_agent 의 이중 spawn 가드/Err 경로 단위테스트에 맡기고 여기선 생존 회귀만 결정적으로 단언한다.
