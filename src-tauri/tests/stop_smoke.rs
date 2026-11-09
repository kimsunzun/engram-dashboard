//! Isolation harness — the stop path against a real daemon process, end to end.
//!
//! Subject: does one stop flight (`daemon_client::shutdown::stop_flight` on production's ports — `daemon.json`, the ws
//! dialer, the OS) make a live daemon exit gracefully: the stop handshake with the daemon's own version →
//! `StopDaemon{force}` → the daemon's `shutdown_all` → self-exit. The single flight, stopping mode and the link around
//! it are unit-tested in memory (`daemon_client::shutdown` tests); this measures the wire and the process.
//!
//! ★ENGRAM_DATA_DIR isolation (unlike the WMI path)★: this test **spawns directly with std::process::Command**,
//!   so the env is inherited and it is fully isolated to a temp folder — it does not touch the production `.engram-dev`.
//!   (A direct spawn is enough to verify the stop round trip — the detached nature of WMI is unrelated to it.)
//!
//! Run: `cargo test -p engram-dashboard --test stop_smoke -- --ignored --test-threads=1`
//!   (skipped by #[ignore] in the default `cargo test` — needs the daemon exe + a real process. Kept as a verification asset.)

#![cfg(windows)]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use std::path::PathBuf;
use std::sync::Arc;

use engram_dashboard_lib::daemon_client::shutdown::{
    stop_flight, StopOutcome, StopPlan, StopPorts,
};
use engram_dashboard_lib::discovery::daemon_status;
use engram_dashboard_transport::{Clock, SystemClock};

/// The candidates of discovery::locate_daemon_exe are next to current_exe (the test exe is in deps/) · based on cwd, so in tests
/// it relies on which folder cargo runs the test in (tests only — the production path stays locate_daemon_exe as is).
fn daemon_exe_path() -> PathBuf {
    // This test binary is in target/<profile>/deps/, so two levels above it is the <profile> folder.
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop(); // deps
    p.pop(); // <profile> (debug/release)
    p.join("engram-dashboard-daemon.exe")
}

/// Waits until `daemon.json` names a pid that is in the OS — whatever its protocol version (`daemon_status` reports the
/// pid of an incompatible record too, only `alive` is false for it). The data folder is a fresh temp folder, so the
/// record can only be the daemon this test started.
fn wait_published(data_dir: &Path, timeout: Duration) -> Option<u32> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(pid) = daemon_status(data_dir)
            .pid
            .filter(|&pid| pid_in_tasklist(pid))
        {
            return Some(pid);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

/// ★Why tasklist instead of daemon_status (load-bearing)★: this test spawns the daemon directly with std::process, so
/// the parent (the test) holds the process HANDLE of the Child. As long as that handle is open, **even if the daemon exits (exit code 0)**
/// OpenProcess+GetProcessTimes keeps returning a creation_time for the dead PID (Windows behavior — a handle keeps the
/// process object alive). tasklist shows **only active processes**, so it sees whether it has "actually exited"
/// regardless of holding a handle → an independent witness next to the stop path's own exit-code check.
fn pid_in_tasklist(pid: u32) -> bool {
    let out = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .expect("tasklist 실행");
    let s = String::from_utf8_lossy(&out.stdout);
    s.contains(&format!("\"{pid}\""))
}

fn wait_pid_gone(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !pid_in_tasklist(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

fn force_kill(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F", "/T"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// RAII cleanup — even on an assertion failure/panic, reclaim the child + the published daemon with taskkill.
struct Guard {
    child: Child,
    pids: Vec<u32>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        // child.kill covers only the direct child — if anything is still alive, taskkill /T takes the whole tree.
        let _ = self.child.kill();
        for &pid in &self.pids {
            force_kill(pid);
        }
    }
}

/// What one stop against a real daemon showed.
struct Stopped {
    outcome: StopOutcome,
    /// The pid `daemon.json` named — the process the stop aimed at.
    published: u32,
    /// The pid this test spawned.
    spawned: u32,
    /// The published process left the OS within 10 s.
    gone: bool,
    /// The spawned process's exit status, if it has exited.
    exit: Option<std::process::ExitStatus>,
}

/// Where the daemon under test writes its endpoint record.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RecordAt {
    /// This app's layout (`DataLayout::daemon_file`).
    Current,
    /// The data folder root — daemons before the per-component layout (ADR-0264, which moves no old data), `v0.3.2`
    /// among them. This app never reads there, so the test copies the record to where it does.
    Root,
}

/// Copies a root-level record (a pre-ADR-0264 daemon's) to this app's location once it names a live pid.
fn adopt_root_record(data_dir: &Path, timeout: Duration) {
    let root = data_dir.join("daemon.json");
    let target = engram_dashboard_lib::discovery::DataLayout::new(data_dir).daemon_file();
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let live = std::fs::read(&root).ok().filter(|bytes| {
            engram_dashboard_protocol::DaemonInfo::parse(bytes)
                .is_ok_and(|info| pid_in_tasklist(info.pid))
        });
        if let Some(bytes) = live {
            std::fs::create_dir_all(target.parent().expect("a parent")).expect("the run folder");
            std::fs::write(&target, bytes).expect("the copied record");
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("no live record at {} within {timeout:?}", root.display());
}

/// Spawns `exe` on a temp data folder and runs one `User` stop flight against it.
fn stop_a_real_daemon(exe: &Path, record_at: RecordAt) -> Stopped {
    assert!(exe.is_file(), "데몬 exe 없음({})", exe.display());
    let tmp = tempfile::tempdir().expect("tempdir 생성");
    let data_dir = tmp.path().to_path_buf();

    // The debug-build daemon is a console app — it inherits stdout/stderr so that with RUST_LOG=debug an Auth
    // rejection/parse failure is visible on the console.
    let child = Command::new(exe)
        .env("ENGRAM_DATA_DIR", &data_dir)
        .spawn()
        .expect("데몬 spawn");
    let spawned = child.id();
    let mut guard = Guard {
        child,
        pids: vec![spawned],
    };

    if record_at == RecordAt::Root {
        adopt_root_record(&data_dir, Duration::from_secs(15));
    }
    let published = wait_published(&data_dir, Duration::from_secs(15)).unwrap_or_else(|| {
        panic!(
            "데몬이 15s 내 daemon.json 발행 — 빌드/포트/mutex 확인 (last status: {:?})",
            daemon_status(&data_dir)
        )
    });
    guard.pids.push(published);

    let rt = tokio::runtime::Runtime::new().expect("a runtime for the stop flight");
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let outcome = rt.block_on(async {
        stop_flight(
            &StopPorts::local(&data_dir),
            &clock,
            StopPlan::User,
            clock.now(),
        )
        .await
    });
    let gone = wait_pid_gone(published, Duration::from_secs(10));
    let exit = guard.child.try_wait().ok().flatten();
    eprintln!(
        "[stop_smoke] {} → outcome={outcome:?} published={published} spawned={spawned} gone={gone} exit={exit:?}",
        exe.display()
    );
    // The guard cleans up the child before tmp is deleted.
    drop(guard);
    Stopped {
        outcome,
        published,
        spawned,
        gone,
        exit,
    }
}

#[test]
#[ignore = "실프로세스 stop 경로 왕복 — 데몬 exe 필요(Windows 전용, 수동 통합)"]
fn stop_flight_makes_a_real_daemon_exit_gracefully() {
    let exe = daemon_exe_path();
    assert!(
        exe.is_file(),
        "데몬 exe 없음({}) — 먼저 `cargo build -p engram-dashboard-daemon`",
        exe.display()
    );
    let stopped = stop_a_real_daemon(&exe, RecordAt::Current);
    let exit = stopped.exit;
    assert_eq!(
        stopped.published, stopped.spawned,
        "the record names the daemon this test spawned"
    );
    assert_eq!(
        stopped.outcome,
        StopOutcome::Closed,
        "the daemon must stop gracefully — `Killed` means StopDaemon did not land, `Timeout` that it never exited \
         (RUST_LOG=debug on the daemon console). exit={exit:?}"
    );
    assert!(
        stopped.gone,
        "the daemon process must leave the OS. exit={exit:?}"
    );
    assert!(
        exit.is_some_and(|status| status.success()),
        "a graceful self-exit exits with 0: {exit:?}"
    );
}

/// TRD S21 trd-3-2-u5 §5 QA: the cross-version case — a shipped daemon of an older protocol (tag `v0.3.2`, protocol 6)
/// accepts the stop handshake in its own version and exits.
///
/// ★It measures the wire, not discovery★ — `v0.3.2` writes its record at the data folder root, where this app does not
/// look (ADR-0264 moved the record and moves no old data); the test copies the record over ([`RecordAt::Root`]).
///
/// ★The old daemon is not built here★ — building a tag needs its own checkout and build; point
/// `ENGRAM_OLD_DAEMON_EXE` at an `engram-dashboard-daemon.exe` built from `v0.3.2`. Without it the test prints
/// `SKIPPED` with that instruction and returns — it then measured nothing (user decision 2026-11-09: the build is run only
/// for changes to the stop handshake; the qa binding says when).
#[test]
#[ignore = "실프로세스 cross-version stop — ENGRAM_OLD_DAEMON_EXE(v0.3.2 데몬 exe) 필요"]
fn stop_flight_makes_an_older_daemon_exit() {
    let Some(exe) = std::env::var_os("ENGRAM_OLD_DAEMON_EXE").map(PathBuf::from) else {
        eprintln!(
            "SKIPPED stop_flight_makes_an_older_daemon_exit: set ENGRAM_OLD_DAEMON_EXE to an engram-dashboard-daemon.exe built from tag v0.3.2 (protocol 6)"
        );
        return;
    };
    let stopped = stop_a_real_daemon(&exe, RecordAt::Root);
    // An older daemon awaits `shutdown_all` inside its handler before it closes — still well inside the 10 s window.
    assert_eq!(
        stopped.outcome,
        StopOutcome::Closed,
        "exit={:?}",
        stopped.exit
    );
    assert!(
        stopped.gone,
        "the older daemon must leave the OS. exit={:?}",
        stopped.exit
    );
}

/// Diagnosis: tells apart whether the close timing right after `StopDaemon` leaves it unprocessed (pinned down the QA
/// root cause of the old sync client — kept as the evidence behind `link::stop`'s "no close right behind StopDaemon").
///
/// Takes a scenario as an argument and changes only the close behavior after sending Auth → StopDaemon:
///   - "noclose"   : no close after flush (connection kept) — diag baseline.
///   - "delayclose": close after waiting 250ms after flush — gives the daemon time to read StopDaemon.
/// If in both the daemon vanishes from the OS (graceful self-exit), the problem is an **immediate close**.
fn diag_send_with_close_mode(mode: &str) -> bool {
    use std::net::TcpStream;
    use tokio_tungstenite::tungstenite::{self, Message};

    let exe = daemon_exe_path();
    assert!(exe.is_file(), "데몬 exe 없음 — cargo build -p ...-daemon");
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().to_path_buf();
    let mut child = Command::new(&exe)
        .env("ENGRAM_DATA_DIR", &data_dir)
        .spawn()
        .unwrap();
    let spawn_pid = child.id();

    let daemon_pid = wait_published(&data_dir, Duration::from_secs(15)).expect("published");
    let info = engram_dashboard_protocol::DaemonInfo::parse(
        &std::fs::read(engram_dashboard_lib::discovery::DataLayout::new(&data_dir).daemon_file())
            .unwrap(),
    )
    .unwrap();

    let url = format!("ws://{}:{}", info.host, info.port);
    let stream = TcpStream::connect(format!("{}:{}", info.host, info.port)).unwrap();
    let (mut ws, _r) = tungstenite::client(&url, stream).unwrap();
    let auth = serde_json::to_string(&engram_dashboard_protocol::handshake::AuthFrame::Auth {
        token: info.token.clone(),
        protocol_version: engram_dashboard_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    let stop = serde_json::to_string(&engram_dashboard_protocol::AgentCommand::StopDaemon {
        force: true,
        kill_agents: true,
        request_id: engram_dashboard_protocol::RequestId::new(),
    })
    .unwrap();
    ws.send(Message::Text(auth.into())).unwrap();
    ws.send(Message::Text(stop.into())).unwrap();
    ws.flush().unwrap();
    eprintln!("[diag:{mode}] sent Auth+StopDaemon, flushed (pid={daemon_pid})");
    if mode == "delayclose" {
        std::thread::sleep(Duration::from_millis(250));
        let _ = ws.close(None);
        eprintln!("[diag:{mode}] closed after 250ms");
    }
    let gone = wait_pid_gone(daemon_pid, Duration::from_secs(10));
    eprintln!(
        "[diag:{mode}] gone={gone} exit={:?}",
        child.try_wait().ok().flatten()
    );
    let _ = child.kill();
    force_kill(spawn_pid);
    gone
}

#[test]
#[ignore = "진단 — StopDaemon 직후 close-race 가설 확인(close 안 함)"]
fn diag_noclose_lets_daemon_exit() {
    assert!(
        diag_send_with_close_mode("noclose"),
        "close 안 하면 데몬이 self-exit 하는가"
    );
}

#[test]
#[ignore = "진단 — StopDaemon 직후 close-race 가설 확인(250ms 후 close)"]
fn diag_delayclose_lets_daemon_exit() {
    assert!(
        diag_send_with_close_mode("delayclose"),
        "close 전 데몬이 read 할 시간을 주면 self-exit 하는가"
    );
}
