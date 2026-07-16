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

use engram_dashboard_core::agent::manager::{default_shell, AgentManager};
use engram_dashboard_core::agent::preset::PresetRegistry;
use engram_dashboard_core::agent::profile::{
    AgentCommand, AgentProfile, ProfileRegistry, SpawnMode,
};
use engram_dashboard_core::agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, ControlEndpoint, OutputFrame, OutputPayload,
    OutputSink, ProvisionError, SinkError, SinkId, StatusSink,
};
use engram_dashboard_core::persistence::{FilePresetStore, FileProfileStore};

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
        // S15 B5 payload-generic: 콘솔 바이트만 수집(구조화 이벤트는 이 headless 테스트가 안 다룸).
        if let OutputPayload::Bytes(b) = frame.payload {
            self.output.lock().unwrap().extend_from_slice(b);
        }
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

// ── FIX 6: 제어 채널 provision 레이스 가드 — 같은 id 동시 spawn 이 서로를 짓밟지 않는다 ──────────
//
// 테스트용 카운팅 ControlChannel: provision/revoke 횟수와 현재 산 (id,epoch) 토큰 수를 관측한다.
// 실 DaemonControlChannel 없이 core 의 예약(SpawnReservation) 인과만 격리 검증한다(ADR-0012 seam).
#[derive(Clone, Default)]
struct CountingControl {
    live: Arc<Mutex<std::collections::HashSet<(AgentId, u32)>>>,
    provisions: Arc<std::sync::atomic::AtomicUsize>,
}

impl ControlChannel for CountingControl {
    fn provision(
        &self,
        id: AgentId,
        epoch: u32,
    ) -> Result<Option<ControlEndpoint>, ProvisionError> {
        self.provisions
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.live.lock().unwrap().insert((id, epoch));
        Ok(Some(ControlEndpoint {
            url: "http://127.0.0.1:1/mcp".into(),
            token: format!("tok-{id}-{epoch}"),
            config_path: PathBuf::from("."),
        }))
    }
    fn revoke(&self, id: AgentId, epoch: u32) {
        self.live.lock().unwrap().remove(&(id, epoch));
    }
}

impl CountingControl {
    fn live_count(&self) -> usize {
        self.live.lock().unwrap().len()
    }
}

#[test]
fn concurrent_same_id_spawn_does_not_clobber() {
    // 같은 AgentId 프로필을 N 스레드가 동시에 spawn 한다. 예약 가드(SpawnReservation) + 이중-spawn
    //   가드가 협력해 **정확히 하나만** 세션을 등록하고 나머지는 깨끗이 Err 로 거부돼야 한다(상대의
    //   세션/자원을 짓밟지 않음).
    // ★round-2 F3 반영★: 이 테스트는 shell 프로필을 쓴다 — shell 은 supports_control_channel=false 라
    //   manager 가 provision 을 **아예 부르지 않는다**. 따라서 CountingControl.provisions 는 0 이고
    //   live_count 도 0 이다(registry 미접촉). 여기서 검증하는 건 SpawnReservation 의 exactly-one-wins
    //   인과 그 자체다(제어 채널 소비 backend 의 provision-race 는 그 가드가 동일하게 커버한다).
    let control = CountingControl::default();
    let control_dyn: Arc<dyn ControlChannel> = Arc::new(control.clone());
    let status_dyn: Arc<dyn StatusSink> = Arc::new(RecordingSink::new());
    let store = Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-test-race-{}", Uuid::new_v4())),
    ));
    let profiles = Arc::new(ProfileRegistry::new(store));
    let preset_store = Arc::new(FilePresetStore::new(
        std::env::temp_dir().join(format!("engram-test-race-preset-{}", Uuid::new_v4())),
    ));
    let presets = Arc::new(PresetRegistry::new(preset_store));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            sessions_dir: None,
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = Arc::new(AgentManager::new_with_control(
        status_dyn,
        profiles,
        presets,
        tracker,
        control_dyn,
    ));

    // 오래 사는 셸 프로필(동일 id) — spawn 성공분이 즉시 종료돼 reaper 가 토큰을 지우기 전에 관측하려고.
    let profile = AgentProfile::new(
        "race".into(),
        AgentCommand::Shell {
            program: default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        false,
    );

    // N 스레드 동시 spawn(같은 id). barrier 로 최대한 동시에 진입시킨다.
    const N: usize = 8;
    let barrier = Arc::new(std::sync::Barrier::new(N));
    let ok_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    std::thread::scope(|s| {
        for _ in 0..N {
            let manager = manager.clone();
            let profile = profile.clone();
            let barrier = barrier.clone();
            let ok_count = ok_count.clone();
            s.spawn(move || {
                barrier.wait();
                if manager.spawn_agent(&profile, SpawnMode::Fresh).is_ok() {
                    ok_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            });
        }
    });

    // ★핵심(FIX 6)★: 정확히 하나만 성공해 세션을 등록했다(나머지는 예약/이중-spawn 가드로 거부).
    assert_eq!(
        ok_count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "같은 id 동시 spawn 은 정확히 1개만 성공해야(예약 가드)"
    );
    assert_eq!(
        manager.list_agents().len(),
        1,
        "세션 맵에 정확히 1개(상대 세션을 짓밟지 않음)"
    );
    // shell 은 provision 을 안 부른다(F3) — registry 미접촉. provision 호출 0, 산 토큰 0.
    assert_eq!(
        control.provisions.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "shell 은 supports_control_channel=false 라 provision 미호출(F3)"
    );
    assert_eq!(
        control.live_count(),
        0,
        "shell 경로는 산 제어 채널 토큰을 만들지 않는다(registry 미접촉)"
    );

    // 정리: 성공한 세션 kill.
    let id = manager.list_agents()[0].id;
    manager.kill_agent(id).expect("kill ok");
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
}

// ── round-2 F3: backend-conditional provisioning (core seam) ──────────────────────────────
// 제어 채널을 소비하지 않는 backend(shell)는 provision 을 **아예 부르지 않는다** — 따라서 항상 실패하는
// ControlChannel 을 주입해도 셸 스폰은 성공한다. 반대로 소비 backend(claude)는 provision Err 에
// fail-closed(스폰 중단)한다. 두 인과를 core 레벨에서 격리 검증한다(seam — 실 DaemonControlChannel 불요).
#[derive(Clone, Default)]
struct FailingControl {
    calls: Arc<std::sync::atomic::AtomicUsize>,
}
impl ControlChannel for FailingControl {
    fn provision(
        &self,
        _id: AgentId,
        _epoch: u32,
    ) -> Result<Option<ControlEndpoint>, ProvisionError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(ProvisionError("injected".into()))
    }
    fn revoke(&self, _id: AgentId, _epoch: u32) {}
}

fn make_manager_with(control: Arc<dyn ControlChannel>) -> Arc<AgentManager> {
    let status_dyn: Arc<dyn StatusSink> = Arc::new(RecordingSink::new());
    let store = Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-test-f3-{}", Uuid::new_v4())),
    ));
    let profiles = Arc::new(ProfileRegistry::new(store));
    let preset_store = Arc::new(FilePresetStore::new(
        std::env::temp_dir().join(format!("engram-test-f3-preset-{}", Uuid::new_v4())),
    ));
    let presets = Arc::new(PresetRegistry::new(preset_store));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            sessions_dir: None,
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    Arc::new(AgentManager::new_with_control(
        status_dyn, profiles, presets, tracker, control,
    ))
}

#[test]
fn shell_spawn_ignores_failing_control_channel() {
    // shell(supports_control_channel=false) → provision 미호출 → 실패하는 채널이어도 스폰 성공.
    let control = FailingControl::default();
    let manager = make_manager_with(Arc::new(control.clone()));
    let profile = AgentProfile::new(
        "f3-shell".into(),
        AgentCommand::Shell {
            program: default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    let res = manager.spawn_agent(&profile, SpawnMode::Fresh);
    assert!(res.is_ok(), "셸은 provision 을 안 부르므로 스폰 성공(F3)");
    assert_eq!(
        control.calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "셸 경로는 provision 을 호출하지 않아야(F3 backend-conditional)"
    );
    let id = manager.list_agents()[0].id;
    manager.kill_agent(id).expect("kill ok");
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
}

#[test]
fn claude_spawn_fails_closed_on_provision_error() {
    // claude(supports_control_channel=true) → provision Err → transport open 전에 스폰 중단(fail-closed).
    //   ★claude 바이너리 불요★: provision Err 가 `?` 로 조기 반환하므로 실제 프로세스 spawn 에 닿지 않는다.
    let control = FailingControl::default();
    let manager = make_manager_with(Arc::new(control.clone()));
    let profile = AgentProfile::new(
        "f3-claude".into(),
        AgentCommand::Claude {
            extra_args: vec![],
            output_format: engram_dashboard_core::agent::profile::ClaudeOutputFormat::Terminal,
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    let res = manager.spawn_agent(&profile, SpawnMode::Fresh);
    assert!(res.is_err(), "claude 는 provision Err 에 fail-closed(F3)");
    assert_eq!(
        control.calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "claude 경로는 provision 을 정확히 1회 호출해야"
    );
    assert!(
        manager.list_agents().is_empty(),
        "fail-closed spawn 은 세션을 등록하지 않아야"
    );
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
    // ADR-0061: 프리셋 레지스트리(이 테스트와 무관 — 빈 상태). 임시 디렉토리 store 로 배선.
    let preset_store = Arc::new(FilePresetStore::new(
        std::env::temp_dir().join(format!("engram-test-headless-preset-{}", Uuid::new_v4())),
    ));
    let presets = Arc::new(PresetRegistry::new(preset_store));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            sessions_dir: None,
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = AgentManager::new(status_dyn, profiles, presets, tracker);

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
    //    ADR-0019: 맵 제거가 reaper(비동기)로 옮겨졌다. kill_agent 는 join_pump(5s)까지만 동기로
    //    기다리고, 실제 sessions 맵 제거·통지는 pump 가 보낸 ReapMsg 를 reaper 가 소비해 수행한다.
    //    따라서 반환 직후가 아니라 폴링으로 "사라짐"을 단언한다(hang 없이 5s 내 완료).
    let kill_started = Instant::now();
    manager.kill_agent(info.id).expect("kill_agent failed");
    let removed = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let kill_elapsed = kill_started.elapsed();

    assert!(removed, "kill 후 세션이 남아있음 — reaper 맵 제거 실패");
    assert!(
        kill_elapsed < Duration::from_secs(5),
        "kill→reap 가 5s 안에 끝나지 않음(hang 의심): {kill_elapsed:?}"
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
