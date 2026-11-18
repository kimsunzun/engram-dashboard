//! Unit tests of [`super`]. `info` · `FakeLiveness` are also used by the stop path's tests ([`crate::daemon_client::link::stop`] · [`crate::daemon_client::shutdown`]).

use super::*;
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

// ── data-folder writability probe (ADR-0134 decision 4) ─────────────────────────────────

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

/// A real race: the pre-check undoes the folder it created, so when two calls overlap, right after B sees A's folder as "there" A deletes it and B's probe trips with NotFound — timing, not permission, so a perfectly good folder must not be decided "not writable". The tray "turn daemon on" and the boot ensure are not serialized (commands/discovery.rs).
/// Do not create the target folder in advance: then no one undoes it and the interleaving never happens (the test would pass proving nothing).
#[test]
fn concurrent_checks_racing_on_folder_creation_do_not_produce_a_false_failure() {
    let parent = fresh_probe_dir("concurrent");
    std::fs::create_dir_all(&parent).expect("create parent folder");
    let target = parent.join("data");

    // Mix the shell pre-check (create and undo) with a daemon start (create and keep — calls the daemon's version through the dev dependency) — two actors that really overlap. The error types differ, so match by wording.
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
        "overlapping checks must not fail each other: {results:?}"
    );
    assert_eq!(left, 0, "no probe leftovers even when overlapping");
}

/// The pre-check leaves no folder, intermediates included: `create_dir_all` can create in a chain, so deleting only the leaf leaves the rest permanently. A deep path that makes it create ancestors that did not exist is used (a pre-created parent would not reproduce this damage).
#[test]
fn check_writable_leaves_no_folder_behind_including_intermediates() {
    let root = fresh_probe_dir("nocreate");
    std::fs::create_dir_all(&root).expect("create root folder");
    let target = root.join("a").join("b").join("data");

    check_data_dir_writable(&target).expect("passes when it can be created");

    let leaf = target.exists();
    let mid_b = root.join("a").join("b").exists();
    let mid_a = root.join("a").exists();
    let left = probe_leftovers(&root);
    let _ = std::fs::remove_dir_all(&root);

    assert!(!leaf, "must not leave the target folder behind");
    assert!(
        !mid_b && !mid_a,
        "intermediate folders must be rolled back too (a={mid_a}, a/b={mid_b})"
    );
    assert_eq!(left, 0, "must not leave the probe behind either");
    assert!(root.parent().is_some(), "the root itself is not touched");
}

/// It undoes even when it fails midway — an early return would leave already-created layers. To fail only the last layer use a name Windows rejects (`?` — `ERROR_INVALID_NAME`).
#[cfg(windows)]
#[test]
fn check_writable_unwinds_what_it_created_even_when_creation_fails_midway() {
    let root = fresh_probe_dir("partial-unwind");
    std::fs::create_dir_all(&root).expect("create root folder");
    let mid = root.join("a");
    let target = mid.join("b?");

    let err = check_data_dir_writable(&target).unwrap_err();

    let mid_left = mid.exists();
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        matches!(err, DiscoveryError::DataDirUnwritable { .. }),
        "a name that cannot be created must fail: {err:?}"
    );
    assert!(
        !mid_left,
        "the failure path must also roll back the intermediate folders we created"
    );
}

/// An already existing intermediate folder is not undone — the observable half of the contract that records only layers we won (not deleting one someone else created first is a race, swept probabilistically by `concurrent_checks_...`).
#[test]
fn check_writable_keeps_intermediate_folders_it_did_not_create() {
    let root = fresh_probe_dir("keep-mid");
    let mid = root.join("a");
    std::fs::create_dir_all(&mid).expect("create up to the intermediate folder in advance");
    let target = mid.join("b").join("data");

    check_data_dir_writable(&target).expect("passes when it can be created");

    let mid_left = mid.exists();
    let ours_left = mid.join("b").exists();
    let _ = std::fs::remove_dir_all(&root);
    assert!(mid_left, "an intermediate folder that already existed must remain");
    assert!(!ours_left, "the layer we created must be rolled back");
}

#[test]
fn check_writable_does_not_remove_pre_existing_folders() {
    let root = fresh_probe_dir("preexisting");
    let target = root.join("data");
    std::fs::create_dir_all(&target).expect("create up to the target in advance");
    check_data_dir_writable(&target).expect("passes");
    let survived = target.exists();
    let _ = std::fs::remove_dir_all(&root);
    assert!(survived, "the check must not delete a folder that already existed");
}

/// R10: if the target path is a file, `create_dir_all` surely fails — passing by looking at the parent would show the user a timeout instead of a message.
#[test]
fn check_writable_rejects_a_path_that_exists_as_a_file() {
    let parent = fresh_probe_dir("asfile");
    std::fs::create_dir_all(&parent).expect("create parent folder");
    let target = parent.join("data");
    std::fs::write(&target, b"not a folder").expect("create a file with the same name");
    let err = check_data_dir_writable(&target).unwrap_err();
    let _ = std::fs::remove_dir_all(&parent);
    assert!(
        matches!(err, DiscoveryError::DataDirUnwritable { .. }),
        "must fail when a file with the same name exists: {err:?}"
    );
}

#[test]
fn check_writable_fails_when_the_folder_cannot_be_created() {
    let blocker = fresh_probe_dir("blocked");
    std::fs::create_dir_all(blocker.parent().unwrap()).ok();
    std::fs::write(&blocker, b"x").expect("create blocker file");
    let err = check_data_dir_writable(&blocker.join("a").join("b")).unwrap_err();
    let _ = std::fs::remove_file(&blocker);
    assert!(
        matches!(err, DiscoveryError::DataDirUnwritable { .. }),
        "{err:?}"
    );
}

/// Partial record = not ready yet (ADR-0135): the daemon writes daemon.json in place (atomic replacement is impossible — it holds the file), so the client can see half-written content; it must go to `Parse` rather than a hard failure so polling continues (the (c) branch of `ensure_with` ignores only `Parse`).
#[test]
fn real_reader_treats_a_partially_written_file_as_not_ready() {
    let dir = fresh_probe_dir("partial-json");
    std::fs::create_dir_all(&dir).expect("create folder");
    let path = dir.join(DAEMON_FILE);
    let full = serde_json::to_vec_pretty(&info(1234, PROTOCOL_VERSION)).expect("serialize");

    std::fs::write(&path, &full[..full.len() / 2]).expect("write only half");
    let partial = FileReader { path: path.clone() }.read();
    // The window right after the length was cut to 0.
    std::fs::write(&path, b"").expect("empty file");
    let empty = FileReader { path: path.clone() }.read();
    // After it is fully written it reads as is — this also pins that the two above are not "never readable".
    std::fs::write(&path, &full).expect("write all");
    let whole = FileReader { path }.read();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        matches!(partial, Err(DiscoveryError::Parse(_))),
        "a partial file must be Parse (= not ready yet): {partial:?}"
    );
    assert!(
        matches!(empty, Err(DiscoveryError::Parse(_))),
        "an empty file must be Parse too: {empty:?}"
    );
    assert!(matches!(whole, Ok(Some(_))), "must be readable once complete: {whole:?}");
}

#[test]
fn public_entry_points_read_the_record_from_the_layout_daemon_file() {
    let dir = fresh_probe_dir("layout-entry");
    let layout = DataLayout::new(&dir);
    std::fs::create_dir_all(layout.daemon_file().parent().unwrap()).expect("folder");
    // Own process = a live pid. Creation time 0 = decided by PID alone.
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
        token: engram_dashboard_protocol::Secret::new("t".repeat(64)),
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

struct FakeReader {
    seq: RefCell<std::collections::VecDeque<Result<Option<DaemonInfo>, DiscoveryError>>>,
    calls: Cell<usize>,
}
impl FakeReader {
    fn new(seq: Vec<Result<Option<DaemonInfo>, DiscoveryError>>) -> Self {
        Self {
            seq: RefCell::new(seq.into()),
            calls: Cell::new(0),
        }
    }
}
impl DaemonReader for FakeReader {
    fn read(&self) -> Result<Option<DaemonInfo>, DiscoveryError> {
        self.calls.set(self.calls.get() + 1);
        // Ok(None) after the sequence is exhausted is for simulating the timeout path.
        self.seq.borrow_mut().pop_front().unwrap_or(Ok(None))
    }
}

// Always succeeds — the spawn failure path is looked at separately by spawn_failure_propagates.
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

/// Does not embed the base `ManualClock` — that would newly require the base test feature in this crate's dev dependencies (TRD S21 boundary 1-1 §3-3).
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
        // It does not actually sleep, so the polling timeout is reached immediately.
        self.slept.fetch_add(1, Ordering::SeqCst);
        *self.now.lock().unwrap() += dur;
    }
}

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
    .expect("succeeds when the file is live");
    assert_eq!(got.pid, 100);
    assert_eq!(spawner.count.get(), 0, "no spawn when live");
}

/// attach does not pass the gate (ADR-0134): even with an unusable folder it must still attach to an already running daemon — attaching needs only reading, and a gate in front would make a healthy daemon unusable.
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
            reason: "test".into(),
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
    .expect("must attach to a live daemon regardless of the gate");
    assert_eq!(got.pid, 100);
    assert_eq!(gate_calls.get(), 0, "the attach path does not call the gate");
}

#[test]
fn pre_spawn_gate_failure_blocks_spawn_and_propagates() {
    let reader = FakeReader::new(vec![Ok(None)]);
    let spawner = CountingSpawner::ok();
    let liveness = FakeLiveness { dead: vec![] };
    let clock = FakeClock::new();
    let mut pre_spawn = || {
        Err(DiscoveryError::DataDirUnwritable {
            path: "X".into(),
            reason: "test".into(),
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
    assert_eq!(spawner.count.get(), 0, "no spawn when the gate blocks");
}

/// R3 regression guard: when the gate blocks, someone else's portfile must survive. The old implementation deleted the file right after the stale decision and ran the gate afterward, so a blocking gate left only an irreversible deletion (on a network share the file may belong to a live daemon on another computer).
#[test]
fn gate_failure_leaves_an_existing_portfile_intact() {
    let dir = fresh_probe_dir("portfile-survives");
    std::fs::create_dir_all(&dir).expect("create folder");
    let path = dir.join(DAEMON_FILE);
    let existing = info(4321, PROTOCOL_VERSION);
    std::fs::write(
        &path,
        serde_json::to_vec(&existing).expect("serialize existing portfile"),
    )
    .expect("write existing portfile");

    let reader = FileReader { path: path.clone() };
    let spawner = CountingSpawner::ok();
    // A situation where the local decision sees "dead" — the very branch where the old implementation went on to delete.
    let liveness = FakeLiveness { dead: vec![4321] };
    let clock = FakeClock::new();
    let mut pre_spawn = || {
        Err(DiscoveryError::DataDirUnwritable {
            path: dir.display().to_string(),
            reason: "test".into(),
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
    assert!(survived, "even when the gate blocks, someone else's portfile must not be deleted");
    assert_eq!(
        same,
        Some(serde_json::to_vec(&existing).unwrap()),
        "the contents must stay unchanged too"
    );
    assert_eq!(spawner.count.get(), 0);
}

#[test]
fn a_dead_looking_portfile_is_never_deleted() {
    let dir = fresh_probe_dir("portfile-kept");
    std::fs::create_dir_all(&dir).expect("create folder");
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
    assert!(survived, "a dead decision alone does not delete someone else's portfile");
    assert_eq!(spawner.count.get(), 1, "it spawns instead");
}

#[test]
fn stale_file_triggers_cleanup_and_spawn_then_polls_new() {
    let reader = FakeReader::new(vec![
        Ok(Some(info(7, PROTOCOL_VERSION))), // (a) old file, pid 7 = dead
        Ok(None),                            // (c) not written yet
        Ok(None),
        Ok(Some(info(200, PROTOCOL_VERSION))), // (c) new daemon live
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
    .expect("new daemon discovered");
    assert_eq!(got.pid, 200);
    assert_eq!(spawner.count.get(), 1, "one spawn when stale");
    assert!(
        reader.calls.get() >= 2,
        "sees the new file by polling without deleting the old file"
    );
}

#[test]
fn missing_file_spawns_and_polls() {
    let reader = FakeReader::new(vec![
        Ok(None),                              // (a) absent
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
        Duration::from_millis(200), // 50ms interval → timeout after a few polls
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

// ── polling branches (reviewer finding): consecutive broken json · consecutive version mismatch ─────────────────────

#[test]
fn polling_keeps_going_on_repeated_corrupt_then_timeout() {
    let reader = FakeReader::new(vec![
        Ok(None),                                     // (a) absent
        Err(DiscoveryError::Parse("partial".into())), // (c) being written
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
        Ok(Some(info(900, PROTOCOL_VERSION + 1))), // (c) version mismatch
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
    // A version mismatch in the polling stage, unlike (a), does not end at once with an error but keeps polling → final Timeout.
    assert!(matches!(err, DiscoveryError::Timeout(_)), "{err:?}");
}

// ── M1 recovery safety net: stale deleted but the old daemon is actually live → recover ───────────────────

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

// Only the first is_dead call answers dead — the misjudgment scenario (seeing the same (pid,start) as dead in (a) and live at the timeout recheck) holds only that way (real: a network share where a remote daemon pid is measured with a local OpenProcess).
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
        Ok(Some(info_with_start(42, PROTOCOL_VERSION, 777))), // (a) first decided dead → delete + keep
        Ok(None),                                             // (c) no new file appears
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
    .expect("recovers if the old daemon is actually live");
    assert_eq!(got.pid, 42, "recovers the info of the old daemon seen as dead");
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
        "must carry the version instead of a cause-less Timeout: {err:?}"
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

// ── ADR-0021: daemon_status (attach-only, spawn 0) ───────────────────

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
    assert!(!s.alive, "a version mismatch cannot attach, so alive=false");
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

// ── read_live_daemon (attach info including token, no-spawn) — ADR-0021 hot-swap tracking ──────

#[test]
fn read_live_returns_full_info_with_token() {
    let reader = FakeReader::new(vec![Ok(Some(info(666, PROTOCOL_VERSION)))]);
    let liveness = FakeLiveness { dead: vec![] };
    let got = read_live_with(&reader, &liveness).expect("Some when the daemon is live");
    assert_eq!(got.pid, 666);
    assert_eq!(got.host, "127.0.0.1");
    assert_eq!(got.port, 12345);
    assert_eq!(
        got.token.expose(),
        "t".repeat(64),
        "the reconnect attach must carry the token"
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

// ── locate_daemon_exe (branch injectable via tempfile) ─────────────────────────────────

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
    assert_eq!(got, first, "the first existing candidate wins");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── ensure_daemon (real entry point) canonicalize failure → ExeNotFound ──────────────────

#[test]
fn ensure_daemon_missing_exe_is_exe_not_found() {
    let data_dir = std::env::temp_dir();
    let missing = std::env::temp_dir().join("engram-definitely-missing-daemon.exe");
    let _ = std::fs::remove_file(&missing);
    let err = ensure_daemon(&data_dir, &missing, Duration::from_millis(50), false).unwrap_err();
    assert!(matches!(err, DiscoveryError::ExeNotFound(_)), "{err:?}");
}

// ── real WMI spawn smoke (real process) — Windows only ─────────────────────────────────
// Run: `cargo test -p engram-dashboard --test lib_unit -- --ignored real_wmi --test-threads=1`. In this shell lib suite only these two create processes (WMI launch · `taskkill`); the default run skips them with `#[ignore]`. They share the same `daemon.json` backup and the single instance, so overlapping would put one's backup holding the other's daemon record — both hold `DATA_DIR_ENV_LOCK` through their whole body so they do not overlap, and `--test-threads=1` is a double safety if that lock is removed.
// If an existing daemon is alive our spawn is rejected by the single-instance lock and the check is meaningless, so skip (return) then.
// Limit: this smoke touches the production data_dir (`.engram-dev`) — minimized by backup/restore, not fully isolated — so it is for local manual verification, not CI.

/// The daemon exe the test launches = the one in `target\<profile>`. The production [`locate_daemon_exe`] does not fit tests: its first candidate (next to the current exe) is `deps\` where the test exe lives, and the cwd candidate depends on where cargo runs the test.
// ADR-0282
#[cfg(windows)]
fn test_daemon_exe() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let profile_dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("the test exe is under target\\<profile>\\deps");
    profile_dir.join(engram_dashboard_platform::env::exe_file_name(
        "engram-dashboard-daemon",
    ))
}

#[cfg(windows)]
#[test]
#[ignore = "real WMI Win32_Process.Create — needs the daemon exe (manual integration, Windows-only)"]
fn real_wmi_spawn_smoke() {
    let _env = DATA_DIR_ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let exe = test_daemon_exe();
    let exe_abs = dunce::canonicalize(&exe).unwrap_or_else(|e| {
        panic!(
            "daemon exe({}) — run `cargo build -p engram-dashboard-daemon` first: {e}",
            exe.display()
        )
    });

    // The default path the WMI-spawned daemon actually uses (env not inherited).
    let data_dir = default_data_dir();
    std::fs::create_dir_all(&data_dir).expect("create data_dir");
    let daemon_path = DataLayout::new(&data_dir).daemon_file();

    let backup = std::fs::read(&daemon_path).ok();
    if let Some(bytes) = &backup {
        if let Ok(prev) = DaemonInfo::parse(bytes) {
            if !RealLiveness.is_dead(prev.pid, prev.start_time) {
                eprintln!(
                    "real_wmi_spawn_smoke: an existing daemon (pid={}) is alive, so the spawn is refused by \
                     single-instance — the check is meaningless, skipping",
                    prev.pid
                );
                return;
            }
        }
    }
    // The daemon too overwrites it if stale, but we clear it out explicitly and move on.
    let _ = std::fs::remove_file(&daemon_path);

    WmiSpawner { console: false }
        .spawn(&exe_abs)
        .expect("WMI Win32_Process.Create succeeds (RV=0, windowless)");

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

    let info = result.expect("the daemon spawned by WMI must publish daemon.json");
    assert!(info.port != 0, "the spawned daemon publishes a valid port");
    assert_eq!(
        info.protocol_version, PROTOCOL_VERSION,
        "protocol_version of the spawned daemon matches"
    );
}

// ── CreateFlags diagnostic matrix (measured) — pin down which flag causes RV=21 ─────────────
// Demonstrates the root cause of the ADR-0021 #1 bug (windowless spawn RV=21). After each spawn, poll daemon.json for the PID → kill at once (daemons must not accumulate).
#[cfg(windows)]
#[test]
#[ignore = "real WMI Win32_Process.Create flag matrix — needs the daemon exe (manual diagnosis)"]
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
            "daemon exe({}) — run `cargo build -p engram-dashboard-daemon` first: {e}",
            exe.display()
        )
    });

    let data_dir = default_data_dir();
    std::fs::create_dir_all(&data_dir).expect("create data_dir");
    let daemon_path = DataLayout::new(&data_dir).daemon_file();

    let backup = std::fs::read(&daemon_path).ok();
    if let Some(bytes) = &backup {
        if let Ok(prev) = DaemonInfo::parse(bytes) {
            if !RealLiveness.is_dead(prev.pid, prev.start_time) {
                eprintln!(
                    "real_wmi_spawn_flag_matrix: an existing daemon (pid={}) is alive, skipping",
                    prev.pid
                );
                return;
            }
        }
    }

    let run_case = |label: &str, flags: Option<i32>| -> u32 {
        let _ = std::fs::remove_file(&daemon_path);
        let rv = engram_dashboard_platform::spawn::wmi_create_raw(&exe_abs, flags)
            .expect("the WMI create call itself must succeed");
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
                            eprintln!("[flag-matrix] {label}: killing daemon pid={}", info.pid);
                            break;
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        rv
    };

    let rv_none = run_case("None(ProcessStartup omitted)", None);
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

    // DETACHED is only observed (no assertion — for reference on alternative b).
    eprintln!(
        "[flag-matrix] summary: None={rv_none} NEW_CONSOLE={rv_new_console} \
         DETACHED={rv_detached} NO_WINDOW={rv_no_window}"
    );
    assert_eq!(
        rv_none, 0,
        "the adopted windowless option (ProcessStartup omitted) must be RV=0"
    );
    assert_eq!(
        rv_new_console, 0,
        "console=true (CREATE_NEW_CONSOLE) must be RV=0"
    );
    assert_ne!(
        rv_no_window, 0,
        "the old bug flag CREATE_NO_WINDOW must be refused (RV!=0) — bug reproduced"
    );
}
