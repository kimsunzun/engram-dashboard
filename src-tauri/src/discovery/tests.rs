//! [`super`] 의 단위 시험. `info` · `FakeReader` · `FakeLiveness` 는 형제 [`crate::daemon_client::stop`] 의 시험도 쓴다.

use super::*;
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

// ── 데이터 폴더 쓰기 가능 프로브(ADR-0134 결정 4) ─────────────────────────────────

/// 프로브 전용 유니크 폴더(테스트 병렬 실행에서 서로 밟지 않게).
fn fresh_probe_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "engram-writable-probe-{tag}-{}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// 폴더 안에 프로브 잔여물이 하나라도 있나(이름이 호출마다 달라 접두사로 센다).
fn probe_leftovers(dir: &Path) -> usize {
    use engram_dashboard_base::writable::WRITE_PROBE_PREFIX;
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with(WRITE_PROBE_PREFIX)
                })
                .count()
        })
        .unwrap_or(0)
}

/// ★실재하는 경합을 겨눈다★: 사전 점검은 **자기가 만든 폴더를 되돌리므로**, 두 호출이 겹치면
/// A 가 만든 폴더를 B 가 "있다"고 본 직후 A 가 지워 B 의 프로브가 NotFound 로 넘어진다. 그건
/// 타이밍이지 권한이 아니라서 멀쩡한 폴더를 "쓰기 불가"로 판정하면 안 된다.
/// 트레이 "데몬 켜기"와 부팅 ensure 는 직렬화되지 않는다(commands/discovery.rs).
///
/// ★대상 폴더를 미리 만들지 말 것★: 만들어 두면 아무도 되돌리지 않아 위 인터리빙이 아예 일어나지
/// 않는다(그러면 이 테스트는 통과해도 아무것도 증명하지 못한다).
#[test]
fn concurrent_checks_racing_on_folder_creation_do_not_produce_a_false_failure() {
    let parent = fresh_probe_dir("concurrent");
    std::fs::create_dir_all(&parent).expect("상위 폴더 생성");
    let target = parent.join("data");

    // 셸 사전 점검(만들고 되돌림)과 데몬 기동(만들고 유지 — 데몬 판을 dev 의존으로 부른다)을 섞는다 — 실제로
    // 겹치는 두 주체다. 오류 타입이 달라 문구로 맞춘다.
    let results: Vec<Result<(), String>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..12)
            .map(|i| {
                let target = &target;
                s.spawn(move || {
                    if i % 3 == 0 {
                        engram_dashboard_daemon::data_dir::ensure_data_dir_writable(target)
                            .map_err(|e| e.to_string())
                    } else {
                        check_data_dir_writable(target).map_err(|e| e.to_string())
                    }
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let left = probe_leftovers(&parent) + probe_leftovers(&target);
    let _ = std::fs::remove_dir_all(&parent);
    assert!(
        results.iter().all(|r| r.is_ok()),
        "겹친 점검이 서로를 실패시키면 안 됨: {results:?}"
    );
    assert_eq!(left, 0, "겹쳐도 프로브 잔여물은 없어야");
}

/// ★사전 점검은 폴더를 남기지 않는다 — 중간 폴더까지★: `create_dir_all` 은 줄줄이 만들 수 있어
/// 잎 하나만 지우면 나머지가 영구히 남는다. 그래서 **없던 조상들을 만들게 하는** 깊은 경로로 본다
/// (바로 위 폴더를 미리 만들어 두면 이 손상 모드가 재현되지 않는다).
#[test]
fn check_writable_leaves_no_folder_behind_including_intermediates() {
    let root = fresh_probe_dir("nocreate");
    std::fs::create_dir_all(&root).expect("루트 폴더 생성");
    let target = root.join("a").join("b").join("data");

    check_data_dir_writable(&target).expect("만들 수 있으면 통과");

    let leaf = target.exists();
    let mid_b = root.join("a").join("b").exists();
    let mid_a = root.join("a").exists();
    let left = probe_leftovers(&root);
    let _ = std::fs::remove_dir_all(&root);

    assert!(!leaf, "대상 폴더를 남기면 안 됨");
    assert!(
        !mid_b && !mid_a,
        "중간 폴더도 되돌려야(a={mid_a}, a/b={mid_b})"
    );
    assert_eq!(left, 0, "프로브도 남기면 안 됨");
    assert!(root.parent().is_some(), "루트 자체는 손대지 않는다");
}

/// ★중간에서 실패해도 되돌린다★: 조기 반환으로 빠져나가면 이미 만든 겹이 그대로 남는다.
/// 마지막 겹만 실패하게 만들려고 Windows 가 거부하는 이름을 쓴다(`?` — `ERROR_INVALID_NAME`).
#[cfg(windows)]
#[test]
fn check_writable_unwinds_what_it_created_even_when_creation_fails_midway() {
    let root = fresh_probe_dir("partial-unwind");
    std::fs::create_dir_all(&root).expect("루트 폴더 생성");
    let mid = root.join("a");
    let target = mid.join("b?");

    let err = check_data_dir_writable(&target).unwrap_err();

    let mid_left = mid.exists();
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        matches!(err, DiscoveryError::DataDirUnwritable { .. }),
        "만들 수 없는 이름은 실패여야: {err:?}"
    );
    assert!(
        !mid_left,
        "실패 경로에서도 우리가 만든 중간 폴더를 되돌려야"
    );
}

/// 이미 있던 **중간** 폴더는 되돌리지 않는다 — 우리가 이긴 겹만 기록한다는 계약의 관측 가능한 절반
/// (남이 먼저 만든 겹을 지우지 않는다는 쪽은 경합이라 `concurrent_checks_...` 가 확률적으로 훑는다).
#[test]
fn check_writable_keeps_intermediate_folders_it_did_not_create() {
    let root = fresh_probe_dir("keep-mid");
    let mid = root.join("a");
    std::fs::create_dir_all(&mid).expect("중간 폴더까지 미리 생성");
    let target = mid.join("b").join("data");

    check_data_dir_writable(&target).expect("만들 수 있으면 통과");

    let mid_left = mid.exists();
    let ours_left = mid.join("b").exists();
    let _ = std::fs::remove_dir_all(&root);
    assert!(mid_left, "이미 있던 중간 폴더는 남아야");
    assert!(!ours_left, "우리가 만든 겹은 되돌려야");
}

/// 이미 있던 폴더는 되돌리지 않는다 — 우리가 만든 것만 되돌린다는 계약의 반대편.
#[test]
fn check_writable_does_not_remove_pre_existing_folders() {
    let root = fresh_probe_dir("preexisting");
    let target = root.join("data");
    std::fs::create_dir_all(&target).expect("대상까지 미리 생성");
    check_data_dir_writable(&target).expect("통과");
    let survived = target.exists();
    let _ = std::fs::remove_dir_all(&root);
    assert!(survived, "이미 있던 폴더를 검사가 지우면 안 됨");
}

/// ★R10★: 대상 경로가 **파일**이면 `create_dir_all` 이 반드시 실패한다 — 상위를 보고 통과시키면
/// 사용자는 메시지 대신 시간 초과를 본다.
#[test]
fn check_writable_rejects_a_path_that_exists_as_a_file() {
    let parent = fresh_probe_dir("asfile");
    std::fs::create_dir_all(&parent).expect("상위 폴더 생성");
    let target = parent.join("data");
    std::fs::write(&target, b"not a folder").expect("같은 이름 파일 생성");
    let err = check_data_dir_writable(&target).unwrap_err();
    let _ = std::fs::remove_dir_all(&parent);
    assert!(
        matches!(err, DiscoveryError::DataDirUnwritable { .. }),
        "같은 이름의 파일이 있으면 실패여야: {err:?}"
    );
}

#[test]
fn check_writable_fails_when_the_folder_cannot_be_created() {
    let blocker = fresh_probe_dir("blocked");
    std::fs::create_dir_all(blocker.parent().unwrap()).ok();
    std::fs::write(&blocker, b"x").expect("blocker 파일 생성");
    let err = check_data_dir_writable(&blocker.join("a").join("b")).unwrap_err();
    let _ = std::fs::remove_file(&blocker);
    assert!(
        matches!(err, DiscoveryError::DataDirUnwritable { .. }),
        "{err:?}"
    );
}

/// ★부분 기록 = 아직 준비 안 됨(ADR-0135)★: 데몬이 daemon.json 을 **제자리에** 쓰므로(원자적 교체
/// 불가 — 데몬이 그 파일을 붙잡고 있다) 클라이언트가 반쯤 쓰인 내용을 실제로 볼 수 있다. 그때 하드
/// 실패가 아니라 `Parse` 로 갈려야 폴링이 계속된다(`ensure_with` 의 (c) 갈래가 `Parse` 만 무시한다).
#[test]
fn real_reader_treats_a_partially_written_file_as_not_ready() {
    let dir = fresh_probe_dir("partial-json");
    std::fs::create_dir_all(&dir).expect("폴더 생성");
    let path = dir.join(DAEMON_FILE);
    let full = serde_json::to_vec_pretty(&info(1234, PROTOCOL_VERSION)).expect("직렬화");

    std::fs::write(&path, &full[..full.len() / 2]).expect("절반만 기록");
    let partial = FileReader { path: path.clone() }.read();
    // 길이를 0으로 줄인 직후의 창.
    std::fs::write(&path, b"").expect("빈 파일");
    let empty = FileReader { path: path.clone() }.read();
    // 온전히 쓰인 뒤에는 그대로 읽힌다 — 위 둘이 "영영 못 읽는다"가 아님을 함께 못 박는다.
    std::fs::write(&path, &full).expect("전체 기록");
    let whole = FileReader { path }.read();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        matches!(partial, Err(DiscoveryError::Parse(_))),
        "부분 파일은 Parse(= 아직 준비 안 됨)여야: {partial:?}"
    );
    assert!(
        matches!(empty, Err(DiscoveryError::Parse(_))),
        "빈 파일도 Parse 여야: {empty:?}"
    );
    assert!(matches!(whole, Ok(Some(_))), "완성되면 읽혀야: {whole:?}");
}

#[test]
fn public_entry_points_read_the_record_from_the_layout_daemon_file() {
    let dir = fresh_probe_dir("layout-entry");
    let layout = DataLayout::new(&dir);
    std::fs::create_dir_all(layout.daemon_file().parent().unwrap()).expect("폴더");
    // 자기 프로세스 = 살아 있는 pid. 생성시각 0 = PID 단독 판정.
    let mut me = info(std::process::id(), PROTOCOL_VERSION);
    me.start_time = 0;
    std::fs::write(layout.daemon_file(), serde_json::to_vec(&me).unwrap()).unwrap();

    let status = daemon_status(&dir);
    let live = read_live_daemon(&dir);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(status.alive, "{status:?}");
    assert_eq!(status.pid, Some(std::process::id()));
    assert_eq!(live.map(|i| i.pid), Some(std::process::id()));
}

pub(crate) fn info(pid: u32, version: u32) -> DaemonInfo {
    DaemonInfo {
        pid,
        host: "127.0.0.1".into(),
        port: 12345,
        token: "t".repeat(64),
        protocol_version: version,
        start_time: 0,
    }
}

pub(crate) struct FakeLiveness {
    pub(crate) dead: Vec<u32>,
}
impl PidLiveness for FakeLiveness {
    fn is_dead(&self, pid: u32, _start_time: u64) -> bool {
        self.dead.contains(&pid)
    }
}

pub(crate) struct FakeReader {
    seq: RefCell<std::collections::VecDeque<Result<Option<DaemonInfo>, DiscoveryError>>>,
    calls: Cell<usize>,
}
impl FakeReader {
    pub(crate) fn new(seq: Vec<Result<Option<DaemonInfo>, DiscoveryError>>) -> Self {
        Self {
            seq: RefCell::new(seq.into()),
            calls: Cell::new(0),
        }
    }
}
impl DaemonReader for FakeReader {
    fn read(&self) -> Result<Option<DaemonInfo>, DiscoveryError> {
        self.calls.set(self.calls.get() + 1);
        // 시퀀스 소진 후 Ok(None) 은 timeout 경로 모사용이다.
        self.seq.borrow_mut().pop_front().unwrap_or(Ok(None))
    }
}

// 항상 성공한다 — spawn 실패 경로는 spawn_failure_propagates 가 따로 본다.
struct CountingSpawner {
    count: Cell<usize>,
}
impl CountingSpawner {
    fn ok() -> Self {
        Self {
            count: Cell::new(0),
        }
    }
}
impl Spawner for CountingSpawner {
    fn spawn(&self, _exe: &Path) -> Result<(), DiscoveryError> {
        self.count.set(self.count.get() + 1);
        Ok(())
    }
}

/// ★base `ManualClock` 을 품지 않는다★ — 품으려면 이 crate 의 dev 의존에 base 시험 기능을 새로 켜야
/// 한다(TRD S21 경계 1-1 §3-3).
struct FakeClock {
    now: Mutex<Instant>,
    slept: AtomicUsize,
}
impl FakeClock {
    fn new() -> Self {
        Self {
            now: Mutex::new(Instant::now()),
            slept: AtomicUsize::new(0),
        }
    }
}
impl engram_dashboard_base::time::Clock for FakeClock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }
}
impl Clock for FakeClock {
    fn sleep(&self, dur: Duration) {
        // 실제로 자지 않아 폴링 timeout 이 즉시 도달한다.
        self.slept.fetch_add(1, Ordering::SeqCst);
        *self.now.lock().unwrap() += dur;
    }
}

/// 대부분의 ensure_with 테스트는 spawn 관문을 다루지 않는다 — 통과시킨다.
fn noop_pre_spawn() -> impl FnMut() -> Result<(), DiscoveryError> {
    || Ok(())
}

#[test]
fn live_existing_file_returns_without_spawn() {
    let reader = FakeReader::new(vec![Ok(Some(info(100, PROTOCOL_VERSION)))]);
    let spawner = CountingSpawner::ok();
    let liveness = FakeLiveness { dead: vec![] };
    let clock = FakeClock::new();

    let got = ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut noop_pre_spawn(),
        Duration::from_secs(5),
    )
    .expect("live 파일이면 성공");
    assert_eq!(got.pid, 100);
    assert_eq!(spawner.count.get(), 0, "live 면 spawn 금지");
}

/// ★attach 는 관문을 지나지 않는다(ADR-0134)★: 폴더가 못 쓰는 상태여도 이미 도는 데몬에는
/// 붙어야 한다 — 붙는 데는 읽기만 필요하다. 관문이 앞에 서면 잘 도는 데몬을 못 쓰게 만든다.
#[test]
fn attach_to_live_daemon_skips_the_pre_spawn_gate() {
    let reader = FakeReader::new(vec![Ok(Some(info(100, PROTOCOL_VERSION)))]);
    let spawner = CountingSpawner::ok();
    let liveness = FakeLiveness { dead: vec![] };
    let clock = FakeClock::new();
    let gate_calls = Cell::new(0usize);
    let mut pre_spawn = || {
        gate_calls.set(gate_calls.get() + 1);
        Err(DiscoveryError::DataDirUnwritable {
            path: "X".into(),
            reason: "테스트".into(),
        })
    };

    let got = ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut pre_spawn,
        Duration::from_secs(5),
    )
    .expect("live 데몬엔 관문과 무관하게 붙어야");
    assert_eq!(got.pid, 100);
    assert_eq!(gate_calls.get(), 0, "attach 경로는 관문을 부르지 않는다");
}

/// 반대 방향: spawn 하러 가는 경로에서는 관문이 서고, 실패하면 spawn 자체가 없다.
#[test]
fn pre_spawn_gate_failure_blocks_spawn_and_propagates() {
    let reader = FakeReader::new(vec![Ok(None)]);
    let spawner = CountingSpawner::ok();
    let liveness = FakeLiveness { dead: vec![] };
    let clock = FakeClock::new();
    let mut pre_spawn = || {
        Err(DiscoveryError::DataDirUnwritable {
            path: "X".into(),
            reason: "테스트".into(),
        })
    };

    let err = ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut pre_spawn,
        Duration::from_secs(5),
    )
    .unwrap_err();
    assert!(
        matches!(err, DiscoveryError::DataDirUnwritable { .. }),
        "{err:?}"
    );
    assert_eq!(spawner.count.get(), 0, "관문이 막으면 spawn 하지 않는다");
}

/// ★R3 회귀 방지★: 관문이 막았을 때 남의 portfile 이 살아남아야 한다. 옛 구현은 stale 판정 직후
/// 파일을 지우고 그 다음에 관문을 돌려서, 관문이 막으면 되돌릴 수 없는 삭제만 남았다.
/// (네트워크 공유에선 그 파일이 **다른 컴퓨터의 살아있는 데몬**의 것일 수 있다.)
#[test]
fn gate_failure_leaves_an_existing_portfile_intact() {
    let dir = fresh_probe_dir("portfile-survives");
    std::fs::create_dir_all(&dir).expect("폴더 생성");
    let path = dir.join(DAEMON_FILE);
    let existing = info(4321, PROTOCOL_VERSION);
    std::fs::write(
        &path,
        serde_json::to_vec(&existing).expect("기존 portfile 직렬화"),
    )
    .expect("기존 portfile 작성");

    let reader = FileReader { path: path.clone() };
    let spawner = CountingSpawner::ok();
    // 로컬 판정이 "죽었다"고 보는 상황 — 옛 구현이 삭제로 넘어가던 바로 그 분기.
    let liveness = FakeLiveness { dead: vec![4321] };
    let clock = FakeClock::new();
    let mut pre_spawn = || {
        Err(DiscoveryError::DataDirUnwritable {
            path: dir.display().to_string(),
            reason: "테스트".into(),
        })
    };

    let err = ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut pre_spawn,
        Duration::from_millis(50),
    )
    .unwrap_err();

    let survived = path.exists();
    let same = std::fs::read(&path).ok();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        matches!(err, DiscoveryError::DataDirUnwritable { .. }),
        "{err:?}"
    );
    assert!(survived, "관문이 막아도 남의 portfile 을 지우면 안 된다");
    assert_eq!(
        same,
        Some(serde_json::to_vec(&existing).unwrap()),
        "내용까지 그대로여야"
    );
    assert_eq!(spawner.count.get(), 0);
}

/// dead 로 판정해도 파일은 그대로 둔다 — 폴링이 새 파일을 보면 되고, 이긴 데몬이 덮어쓴다.
#[test]
fn a_dead_looking_portfile_is_never_deleted() {
    let dir = fresh_probe_dir("portfile-kept");
    std::fs::create_dir_all(&dir).expect("폴더 생성");
    let path = dir.join(DAEMON_FILE);
    std::fs::write(
        &path,
        serde_json::to_vec(&info(4321, PROTOCOL_VERSION)).unwrap(),
    )
    .unwrap();

    let reader = FileReader { path: path.clone() };
    let spawner = CountingSpawner::ok();
    let liveness = FakeLiveness { dead: vec![4321] };
    let clock = FakeClock::new();

    let _ = ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut noop_pre_spawn(),
        Duration::from_millis(50),
    );

    let survived = path.exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(survived, "dead 판정만으로 남의 portfile 을 지우지 않는다");
    assert_eq!(spawner.count.get(), 1, "대신 spawn 은 한다");
}

#[test]
fn stale_file_triggers_cleanup_and_spawn_then_polls_new() {
    let reader = FakeReader::new(vec![
        Ok(Some(info(7, PROTOCOL_VERSION))), // (a) 옛 파일, pid 7 = dead
        Ok(None),                            // (c) 아직 안 써짐
        Ok(None),
        Ok(Some(info(200, PROTOCOL_VERSION))), // (c) 새 데몬 live
    ]);
    let spawner = CountingSpawner::ok();
    let liveness = FakeLiveness { dead: vec![7] };
    let clock = FakeClock::new();

    let got = ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut noop_pre_spawn(),
        Duration::from_secs(5),
    )
    .expect("새 데몬 발견 성공");
    assert_eq!(got.pid, 200);
    assert_eq!(spawner.count.get(), 1, "stale 면 spawn 1회");
    assert!(
        reader.calls.get() >= 2,
        "옛 파일을 지우지 않고 폴링으로 새 파일을 본다"
    );
}

#[test]
fn missing_file_spawns_and_polls() {
    let reader = FakeReader::new(vec![
        Ok(None),                              // (a) 없음
        Ok(Some(info(300, PROTOCOL_VERSION))), // (c)
    ]);
    let spawner = CountingSpawner::ok();
    let liveness = FakeLiveness { dead: vec![] };
    let clock = FakeClock::new();

    let got = ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut noop_pre_spawn(),
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(got.pid, 300);
    assert_eq!(spawner.count.get(), 1);
}

#[test]
fn timeout_when_daemon_never_writes() {
    let reader = FakeReader::new(vec![Ok(None)]);
    let spawner = CountingSpawner::ok();
    let liveness = FakeLiveness { dead: vec![] };
    let clock = FakeClock::new();

    let err = ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut noop_pre_spawn(),
        Duration::from_millis(200), // 50ms 간격 → 몇 번 폴링 후 timeout
    )
    .unwrap_err();
    assert!(matches!(err, DiscoveryError::Timeout(_)), "{err:?}");
}

#[test]
fn version_mismatch_live_daemon_errors_without_spawn() {
    let reader = FakeReader::new(vec![Ok(Some(info(400, PROTOCOL_VERSION + 1)))]);
    let spawner = CountingSpawner::ok();
    let liveness = FakeLiveness { dead: vec![] };
    let clock = FakeClock::new();

    let err = ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut noop_pre_spawn(),
        Duration::from_secs(5),
    )
    .unwrap_err();
    assert!(
        matches!(err, DiscoveryError::VersionMismatch { .. }),
        "{err:?}"
    );
    assert_eq!(spawner.count.get(), 0);
}

#[test]
fn dead_pid_with_old_protocol_spawns_instead_of_erroring() {
    let reader = FakeReader::new(vec![
        Ok(Some(info(600, PROTOCOL_VERSION - 1))),
        Ok(Some(info(601, PROTOCOL_VERSION))),
    ]);
    let spawner = CountingSpawner::ok();
    let liveness = FakeLiveness { dead: vec![600] };
    let clock = FakeClock::new();

    let got = ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut noop_pre_spawn(),
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(got.pid, 601);
    assert_eq!(spawner.count.get(), 1);
}

#[test]
fn dead_pid_with_newer_protocol_also_spawns_instead_of_erroring() {
    let reader = FakeReader::new(vec![
        Ok(Some(info(610, PROTOCOL_VERSION + 1))),
        Ok(Some(info(611, PROTOCOL_VERSION))),
    ]);
    let spawner = CountingSpawner::ok();
    let liveness = FakeLiveness { dead: vec![610] };
    let clock = FakeClock::new();

    let got = ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut noop_pre_spawn(),
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(got.pid, 611);
    assert_eq!(spawner.count.get(), 1);
}

#[test]
fn corrupt_existing_file_cleans_and_spawns() {
    let reader = FakeReader::new(vec![
        Err(DiscoveryError::Parse("bad".into())),
        Ok(Some(info(500, PROTOCOL_VERSION))),
    ]);
    let spawner = CountingSpawner::ok();
    let liveness = FakeLiveness { dead: vec![] };
    let clock = FakeClock::new();

    let got = ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut noop_pre_spawn(),
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(got.pid, 500);
    assert_eq!(spawner.count.get(), 1);
}

#[test]
fn spawn_failure_propagates() {
    struct FailingSpawner;
    impl Spawner for FailingSpawner {
        fn spawn(&self, _exe: &Path) -> Result<(), DiscoveryError> {
            Err(DiscoveryError::SpawnFailed { rv: 9 })
        }
    }
    let reader = FakeReader::new(vec![Ok(None)]);
    let liveness = FakeLiveness { dead: vec![] };
    let clock = FakeClock::new();

    let err = ensure_with(
        &reader,
        &FailingSpawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut noop_pre_spawn(),
        Duration::from_secs(5),
    )
    .unwrap_err();
    assert!(
        matches!(err, DiscoveryError::SpawnFailed { rv: 9 }),
        "{err:?}"
    );
}

// ── 폴링 분기(리뷰어 지적): 깨진 json 연속·버전 불일치 연속 ─────────────────────

#[test]
fn polling_keeps_going_on_repeated_corrupt_then_timeout() {
    let reader = FakeReader::new(vec![
        Ok(None),                                     // (a) 없음
        Err(DiscoveryError::Parse("partial".into())), // (c) 쓰는 중
        Err(DiscoveryError::Parse("partial".into())),
        Err(DiscoveryError::Parse("partial".into())),
    ]);
    let spawner = CountingSpawner::ok();
    let liveness = FakeLiveness { dead: vec![] };
    let clock = FakeClock::new();

    let err = ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut noop_pre_spawn(),
        Duration::from_millis(200),
    )
    .unwrap_err();
    assert!(matches!(err, DiscoveryError::Timeout(_)), "{err:?}");
    assert_eq!(spawner.count.get(), 1);
}

#[test]
fn polling_keeps_going_on_repeated_version_mismatch_then_timeout() {
    let reader = FakeReader::new(vec![
        Ok(None),                                  // (a)
        Ok(Some(info(900, PROTOCOL_VERSION + 1))), // (c) 버전 불일치
        Ok(Some(info(901, PROTOCOL_VERSION + 1))),
    ]);
    let spawner = CountingSpawner::ok();
    let liveness = FakeLiveness { dead: vec![] };
    let clock = FakeClock::new();

    let err = ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut noop_pre_spawn(),
        Duration::from_millis(200),
    )
    .unwrap_err();
    // 폴링 단계의 버전 불일치는 (a) 와 달리 즉시 에러로 끝내지 않고 계속 폴링 → 최종 Timeout.
    assert!(matches!(err, DiscoveryError::Timeout(_)), "{err:?}");
}

// ── M1 복구 안전망: stale 삭제했으나 옛 데몬이 사실 live → 복구 ───────────────────

struct StartTimeLiveness {
    live: Vec<(u32, u64)>,
}
impl PidLiveness for StartTimeLiveness {
    fn is_dead(&self, pid: u32, start_time: u64) -> bool {
        !self.live.contains(&(pid, start_time))
    }
}

fn info_with_start(pid: u32, version: u32, start: u64) -> DaemonInfo {
    let mut i = info(pid, version);
    i.start_time = start;
    i
}

// is_dead 첫 호출만 dead 로 답한다 — 같은 (pid,start) 를 (a) 에서는 dead, timeout 재검사에서는
// live 로 보는 오판 시나리오가 그래야 성립한다(실물 = 원격 데몬 pid 를 로컬 OpenProcess 로 재는
// 네트워크 공유).
struct FlipLiveness {
    calls: Cell<usize>,
}
impl PidLiveness for FlipLiveness {
    fn is_dead(&self, _pid: u32, _start: u64) -> bool {
        let n = self.calls.get();
        self.calls.set(n + 1);
        n == 0
    }
}

#[test]
fn timeout_recovers_old_daemon_if_still_live() {
    let reader = FakeReader::new(vec![
        Ok(Some(info_with_start(42, PROTOCOL_VERSION, 777))), // (a) 처음엔 dead 판정 → 삭제+보관
        Ok(None),                                             // (c) 새 파일 안 나옴
    ]);
    let spawner = CountingSpawner::ok();
    let liveness = FlipLiveness {
        calls: Cell::new(0),
    };
    let clock = FakeClock::new();

    let got = ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut noop_pre_spawn(),
        Duration::from_millis(150),
    )
    .expect("옛 데몬이 사실 live 면 복구");
    assert_eq!(got.pid, 42, "dead 로 봤던 옛 데몬 정보를 복구");
}

#[test]
fn timeout_reports_version_mismatch_if_old_daemon_is_live_but_mismatched() {
    let reader = FakeReader::new(vec![
        Ok(Some(info_with_start(44, PROTOCOL_VERSION - 1, 999))),
        Ok(None),
    ]);
    let spawner = CountingSpawner::ok();
    let liveness = FlipLiveness {
        calls: Cell::new(0),
    };
    let clock = FakeClock::new();

    let err = ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut noop_pre_spawn(),
        Duration::from_millis(150),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            DiscoveryError::VersionMismatch { daemon, expected }
                if daemon == PROTOCOL_VERSION - 1 && expected == PROTOCOL_VERSION
        ),
        "원인 없는 Timeout 대신 버전을 실어야: {err:?}"
    );
}

#[test]
fn timeout_does_not_recover_if_old_daemon_really_dead() {
    let reader = FakeReader::new(vec![
        Ok(Some(info_with_start(43, PROTOCOL_VERSION, 888))),
        Ok(None),
    ]);
    let spawner = CountingSpawner::ok();
    let liveness = StartTimeLiveness { live: vec![] };
    let clock = FakeClock::new();

    let err = ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        Path::new("daemon.exe"),
        &mut noop_pre_spawn(),
        Duration::from_millis(150),
    )
    .unwrap_err();
    assert!(matches!(err, DiscoveryError::Timeout(_)), "{err:?}");
}

// ── ADR-0021: daemon_status / daemon_stop (attach-only, spawn 0) ───────────────────

struct CountingKiller {
    killed: RefCell<Vec<u32>>,
}
impl CountingKiller {
    fn new() -> Self {
        Self {
            killed: RefCell::new(Vec::new()),
        }
    }
}
impl ProcessKiller for CountingKiller {
    fn kill(&self, pid: u32) -> Result<(), DiscoveryError> {
        self.killed.borrow_mut().push(pid);
        Ok(())
    }
}

#[test]
fn status_live_file_reports_alive_with_pid_port() {
    let reader = FakeReader::new(vec![Ok(Some(info(111, PROTOCOL_VERSION)))]);
    let liveness = FakeLiveness { dead: vec![] };
    let s = status_with(&reader, &liveness);
    assert!(s.alive);
    assert_eq!(s.pid, Some(111));
    assert_eq!(s.port, Some(12345));
}

#[test]
fn status_dead_file_reports_not_alive_but_keeps_pid() {
    let reader = FakeReader::new(vec![Ok(Some(info(222, PROTOCOL_VERSION)))]);
    let liveness = FakeLiveness { dead: vec![222] };
    let s = status_with(&reader, &liveness);
    assert!(!s.alive);
    assert_eq!(s.pid, Some(222));
}

#[test]
fn status_version_mismatch_is_not_alive() {
    let reader = FakeReader::new(vec![Ok(Some(info(333, PROTOCOL_VERSION + 1)))]);
    let liveness = FakeLiveness { dead: vec![] };
    let s = status_with(&reader, &liveness);
    assert!(!s.alive, "버전 불일치는 붙을 수 없으므로 alive=false");
    assert_eq!(s.pid, Some(333));
}

#[test]
fn status_missing_file_is_not_alive_no_pid() {
    let reader = FakeReader::new(vec![Ok(None)]);
    let liveness = FakeLiveness { dead: vec![] };
    let s = status_with(&reader, &liveness);
    assert!(!s.alive);
    assert_eq!(s.pid, None);
    assert_eq!(s.port, None);
}

// ── read_live_daemon (token 포함 attach 정보, no-spawn) — ADR-0021 hot-swap 추적 ──────

#[test]
fn read_live_returns_full_info_with_token() {
    let reader = FakeReader::new(vec![Ok(Some(info(666, PROTOCOL_VERSION)))]);
    let liveness = FakeLiveness { dead: vec![] };
    let got = read_live_with(&reader, &liveness).expect("live 데몬이면 Some");
    assert_eq!(got.pid, 666);
    assert_eq!(got.host, "127.0.0.1");
    assert_eq!(got.port, 12345);
    assert_eq!(
        got.token,
        "t".repeat(64),
        "재연결 attach 에 token 이 실려야 함"
    );
}

#[test]
fn read_live_dead_daemon_is_none() {
    let reader = FakeReader::new(vec![Ok(Some(info(777, PROTOCOL_VERSION)))]);
    let liveness = FakeLiveness { dead: vec![777] };
    assert!(read_live_with(&reader, &liveness).is_none());
}

#[test]
fn read_live_version_mismatch_is_none() {
    let reader = FakeReader::new(vec![Ok(Some(info(888, PROTOCOL_VERSION + 1)))]);
    let liveness = FakeLiveness { dead: vec![] };
    assert!(read_live_with(&reader, &liveness).is_none());
}

#[test]
fn read_live_missing_or_broken_is_none() {
    let liveness = FakeLiveness { dead: vec![] };
    let none_reader = FakeReader::new(vec![Ok(None)]);
    assert!(read_live_with(&none_reader, &liveness).is_none());
    let broken_reader = FakeReader::new(vec![Err(DiscoveryError::Parse("bad".into()))]);
    assert!(read_live_with(&broken_reader, &liveness).is_none());
}

#[test]
fn stop_live_daemon_kills_pid() {
    let reader = FakeReader::new(vec![Ok(Some(info(444, PROTOCOL_VERSION)))]);
    let liveness = FakeLiveness { dead: vec![] };
    let killer = CountingKiller::new();
    let got = stop_with(&reader, &liveness, &killer).unwrap();
    assert_eq!(got, Some(444));
    assert_eq!(killer.killed.borrow().as_slice(), &[444]);
}

#[test]
fn stop_dead_daemon_does_not_kill() {
    let reader = FakeReader::new(vec![Ok(Some(info(555, PROTOCOL_VERSION)))]);
    let liveness = FakeLiveness { dead: vec![555] };
    let killer = CountingKiller::new();
    let got = stop_with(&reader, &liveness, &killer).unwrap();
    assert_eq!(got, None);
    assert!(killer.killed.borrow().is_empty(), "죽은 데몬은 kill 금지");
}

#[test]
fn stop_missing_file_is_noop() {
    let reader = FakeReader::new(vec![Ok(None)]);
    let liveness = FakeLiveness { dead: vec![] };
    let killer = CountingKiller::new();
    let got = stop_with(&reader, &liveness, &killer).unwrap();
    assert_eq!(got, None);
    assert!(killer.killed.borrow().is_empty());
}

// ── locate_daemon_exe (tempfile 주입 가능 분기) ─────────────────────────────────

#[test]
fn locate_daemon_exe_no_candidates_returns_exe_not_found() {
    let bogus = std::env::temp_dir().join("engram-no-such-daemon-dir-xyz");
    let _ = std::fs::remove_dir_all(&bogus);
    let candidates = vec![bogus.join("engram-dashboard-daemon.exe")];
    let err = locate_in(&candidates).unwrap_err();
    assert!(matches!(err, DiscoveryError::ExeNotFound(_)), "{err:?}");
}

#[test]
fn locate_daemon_exe_picks_first_existing() {
    let dir = std::env::temp_dir().join("engram-locate-test");
    let _ = std::fs::create_dir_all(&dir);
    let first = dir.join("first-daemon.exe");
    std::fs::write(&first, b"x").unwrap();
    let second = dir.join("second-daemon.exe");
    std::fs::write(&second, b"x").unwrap();
    let got = locate_in(&[first.clone(), second]).unwrap();
    assert_eq!(got, first, "첫 존재 후보 우선");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── ensure_daemon (real 진입점) canonicalize 실패 → ExeNotFound ──────────────────

#[test]
fn ensure_daemon_missing_exe_is_exe_not_found() {
    let data_dir = std::env::temp_dir();
    let missing = std::env::temp_dir().join("engram-definitely-missing-daemon.exe");
    let _ = std::fs::remove_file(&missing);
    let err = ensure_daemon(&data_dir, &missing, Duration::from_millis(50), false).unwrap_err();
    assert!(matches!(err, DiscoveryError::ExeNotFound(_)), "{err:?}");
}

// ── 실제 WMI spawn smoke(실프로세스) — Windows 전용 ─────────────────────────────────
//
// 실행: `cargo test -p engram-dashboard --test lib_unit -- --ignored real_wmi --test-threads=1`. 이 셸 lib 시험에서
//   프로세스를 만드는 것은 이 둘뿐이다(WMI 띄우기 · `taskkill`) — 기본 실행은 `#[ignore]` 로 건너뛴다.
//   둘이 같은 `daemon.json` 백업과 단일 인스턴스를 나눠 쓴다 — 겹치면 한쪽 백업이 다른 쪽 데몬의 기록을 담는다.
//   둘 다 몸통 내내 `DATA_DIR_ENV_LOCK` 을 쥐어 이미 겹치지 않고, `--test-threads=1` 은 그 잠금이 빠질 때를 대비한
//   이중 안전이다(`--ignored real_wmi` 로는 이 둘만 돈다).
//
// ★기존 데몬이 살아있으면 단일 인스턴스 잠금으로 우리 spawn 이 거부돼 검증이 무의미하므로
//   그 경우 skip(return) 한다.★
//
// 한계(은폐 금지): 이 smoke 는 운영 data_dir(`.engram-dev`)을 건드리므로(백업/복원으로 최소화하나
//   완전 격리는 아님) CI 보다는 로컬 수동 검증용이다.

/// 시험이 띄울 데몬 exe = `target\<profile>` 의 것. 운영 [`locate_daemon_exe`] 는 시험에 맞지 않는다 — 첫 후보(지금
/// exe 옆)는 시험 exe 가 사는 `deps\` 이고, cwd 후보는 cargo 가 시험을 어느 폴더에서 돌리나에 기댄다.
// ADR-0282
#[cfg(windows)]
fn test_daemon_exe() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let profile_dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("시험 exe 는 target\\<profile>\\deps 아래에 있다");
    profile_dir.join(engram_dashboard_platform::env::exe_file_name(
        "engram-dashboard-daemon",
    ))
}

#[cfg(windows)]
#[test]
#[ignore = "실제 WMI Win32_Process.Create — 데몬 exe 필요(수동 통합, Windows 전용)"]
fn real_wmi_spawn_smoke() {
    let _env = DATA_DIR_ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let exe = test_daemon_exe();
    let exe_abs = dunce::canonicalize(&exe).unwrap_or_else(|e| {
        panic!(
            "daemon exe({}) — 먼저 `cargo build -p engram-dashboard-daemon` 필요: {e}",
            exe.display()
        )
    });

    // WMI-spawn 데몬이 실제로 쓰는 default 경로(env 미상속).
    let data_dir = default_data_dir();
    std::fs::create_dir_all(&data_dir).expect("data_dir 생성");
    let daemon_path = DataLayout::new(&data_dir).daemon_file();

    let backup = std::fs::read(&daemon_path).ok();
    if let Some(bytes) = &backup {
        if let Ok(prev) = DaemonInfo::parse(bytes) {
            if !RealLiveness.is_dead(prev.pid, prev.start_time) {
                eprintln!(
                    "real_wmi_spawn_smoke: 기존 데몬(pid={})이 살아있어 단일-인스턴스로 spawn 이 \
                     거부됨 — 검증 무의미하므로 skip",
                    prev.pid
                );
                return;
            }
        }
    }
    // 데몬도 stale 이면 덮어쓰지만 명확히 비우고 간다.
    let _ = std::fs::remove_file(&daemon_path);

    WmiSpawner { console: false }
        .spawn(&exe_abs)
        .expect("WMI Win32_Process.Create 성공(RV=0, windowless)");

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut spawned: Option<DaemonInfo> = None;
    while Instant::now() < deadline {
        if let Ok(bytes) = std::fs::read(&daemon_path) {
            if let Ok(info) = DaemonInfo::parse(&bytes) {
                if !RealLiveness.is_dead(info.pid, info.start_time) {
                    spawned = Some(info);
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let result = spawned.clone();
    if let Some(info) = &spawned {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &info.pid.to_string(), "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    match backup {
        Some(bytes) => {
            let _ = std::fs::write(&daemon_path, bytes);
        }
        None => {
            let _ = std::fs::remove_file(&daemon_path);
        }
    }

    let info = result.expect("WMI spawn 한 데몬이 daemon.json 을 발행해야");
    assert!(info.port != 0, "spawn 한 데몬은 유효 포트 발행");
    assert_eq!(
        info.protocol_version, PROTOCOL_VERSION,
        "spawn 한 데몬의 protocol_version 일치"
    );
}

// ── CreateFlags 진단 매트릭스(실측) — 어느 플래그가 RV=21 을 유발하는지 확정 ─────────────
//
// ADR-0021 #1 버그(windowless spawn RV=21) 의 근본 원인을 실증한다.
//
// 각 spawn 직후 daemon.json 폴링으로 PID 회수 → 즉시 kill(데몬 누적 방지).
#[cfg(windows)]
#[test]
#[ignore = "실제 WMI Win32_Process.Create 플래그 매트릭스 — 데몬 exe 필요(수동 진단)"]
fn real_wmi_spawn_flag_matrix() {
    const CREATE_NEW_CONSOLE: i32 = 0x0000_0010;
    const DETACHED_PROCESS: i32 = 0x0000_0008;
    const CREATE_NO_WINDOW: i32 = 0x0800_0000;

    let _env = DATA_DIR_ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let exe = test_daemon_exe();
    let exe_abs = dunce::canonicalize(&exe).unwrap_or_else(|e| {
        panic!(
            "daemon exe({}) — 먼저 `cargo build -p engram-dashboard-daemon` 필요: {e}",
            exe.display()
        )
    });

    let data_dir = default_data_dir();
    std::fs::create_dir_all(&data_dir).expect("data_dir 생성");
    let daemon_path = DataLayout::new(&data_dir).daemon_file();

    let backup = std::fs::read(&daemon_path).ok();
    if let Some(bytes) = &backup {
        if let Ok(prev) = DaemonInfo::parse(bytes) {
            if !RealLiveness.is_dead(prev.pid, prev.start_time) {
                eprintln!(
                    "real_wmi_spawn_flag_matrix: 기존 데몬(pid={})이 살아있어 skip",
                    prev.pid
                );
                return;
            }
        }
    }

    let run_case = |label: &str, flags: Option<i32>| -> u32 {
        let _ = std::fs::remove_file(&daemon_path);
        let rv = engram_dashboard_platform::spawn::wmi_create_raw(&exe_abs, flags)
            .expect("WMI create 호출 자체는 성공해야");
        eprintln!("[flag-matrix] {label}: ReturnValue={rv}");
        if rv == 0 {
            let deadline = Instant::now() + Duration::from_secs(8);
            while Instant::now() < deadline {
                if let Ok(bytes) = std::fs::read(&daemon_path) {
                    if let Ok(info) = DaemonInfo::parse(&bytes) {
                        if !RealLiveness.is_dead(info.pid, info.start_time) {
                            let _ = std::process::Command::new("taskkill")
                                .args(["/PID", &info.pid.to_string(), "/F"])
                                .stdout(std::process::Stdio::null())
                                .stderr(std::process::Stdio::null())
                                .status();
                            eprintln!("[flag-matrix] {label}: 데몬 pid={} kill", info.pid);
                            break;
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        rv
    };

    let rv_none = run_case("None(ProcessStartup 생략)", None);
    let rv_new_console = run_case("CREATE_NEW_CONSOLE(0x10)", Some(CREATE_NEW_CONSOLE));
    let rv_detached = run_case("DETACHED_PROCESS(0x08)", Some(DETACHED_PROCESS));
    let rv_no_window = run_case("CREATE_NO_WINDOW(0x08000000)", Some(CREATE_NO_WINDOW));

    match backup {
        Some(bytes) => {
            let _ = std::fs::write(&daemon_path, bytes);
        }
        None => {
            let _ = std::fs::remove_file(&daemon_path);
        }
    }

    // DETACHED 는 관찰만 한다(단언 안 함 — 대안 b 참고용).
    eprintln!(
        "[flag-matrix] 요약: None={rv_none} NEW_CONSOLE={rv_new_console} \
         DETACHED={rv_detached} NO_WINDOW={rv_no_window}"
    );
    assert_eq!(
        rv_none, 0,
        "windowless 채택안(ProcessStartup 생략)은 RV=0 이어야"
    );
    assert_eq!(
        rv_new_console, 0,
        "console=true(CREATE_NEW_CONSOLE)는 RV=0 이어야"
    );
    assert_ne!(
        rv_no_window, 0,
        "기존 버그 플래그 CREATE_NO_WINDOW 는 거부(RV!=0)되어야 — 버그 재현"
    );
}
