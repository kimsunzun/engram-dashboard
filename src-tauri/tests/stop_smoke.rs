//! Isolation harness — graceful StopDaemon WS round trip (real process) end-to-end smoke.
//!
//! Subject: when `daemon_client::stop::send_stop(data_dir)` is **actually** called, does a live daemon
//! die gracefully (WS Auth → StopDaemon{force} → daemon self-exit).
//!
//! ★ENGRAM_DATA_DIR isolation (unlike the WMI path)★: this test **spawns directly with std::process::Command**,
//!   so the env is inherited and it is fully isolated to a temp folder — it does not touch the production `.engram-dev`.
//!   (A direct spawn is enough for the purpose of verifying the send_stop round trip — the detached nature of WMI is
//!   unrelated to send_stop behavior.)
//!
//! Run: `cargo test -p engram-dashboard --test stop_smoke -- --ignored --test-threads=1`
//!   (skipped by #[ignore] in the default `cargo test` — needs the daemon exe + a real process. Kept as a verification asset.)

#![cfg(windows)]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use std::path::PathBuf;

use engram_dashboard_lib::daemon_client::stop::{send_stop, StopOutcome};
use engram_dashboard_lib::discovery::daemon_status;

/// The candidates of discovery::locate_daemon_exe are next to current_exe (the test exe is in deps/) · based on cwd, so in tests
/// it relies on which folder cargo runs the test in (tests only — the production path stays locate_daemon_exe as is).
fn daemon_exe_path() -> PathBuf {
    // This test binary is in target/<profile>/deps/, so two levels above it is the <profile> folder.
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop(); // deps
    p.pop(); // <profile> (debug/release)
    p.join("engram-dashboard-daemon.exe")
}

fn wait_alive(data_dir: &Path, timeout: Duration) -> Option<u32> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let st = daemon_status(data_dir);
        if st.alive {
            return st.pid;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

/// ★Why tasklist instead of daemon_status (load-bearing)★: this test spawns the daemon directly with std::process, so
/// the parent (the test) holds the process HANDLE of the Child. As long as that handle is open, **even if the daemon exits (exit code 0)**
/// OpenProcess+GetProcessTimes (= the API agent liveness uses) keeps returning a creation_time for the dead PID
/// (Windows behavior — a handle keeps the process object alive). So
/// daemon_status becomes false-live (a test harness artifact — the production tray does not keep the daemon as a child but
/// polls after a WMI detached spawn, so it is irrelevant). tasklist shows **only active processes**, so it sees whether it has "actually exited"
/// regardless of holding a handle → the source of truth for graceful self-exit.
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

#[test]
#[ignore = "실프로세스 graceful StopDaemon WS 왕복 — 데몬 exe 필요(Windows 전용, 수동 통합)"]
fn send_stop_makes_real_daemon_self_exit() {
    let exe = daemon_exe_path();
    assert!(
        exe.is_file(),
        "데몬 exe 없음({}) — 먼저 `cargo build -p engram-dashboard-daemon`",
        exe.display()
    );

    let tmp = tempfile::tempdir().expect("tempdir 생성");
    let data_dir = tmp.path().to_path_buf();

    // The debug-build daemon is a console app — it inherits stdout/stderr so that with RUST_LOG=debug an Auth rejection/parse failure
    // is visible on the console.
    let mut child: Child = Command::new(&exe)
        .env("ENGRAM_DATA_DIR", &data_dir)
        .spawn()
        .expect("데몬 spawn");

    // RAII cleanup — even on an assertion failure/panic, reclaim the child + its PID with taskkill.
    struct Guard<'a> {
        child: &'a mut Child,
        pid: u32,
    }
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            // child.kill covers only the direct child — if anything is still alive, taskkill /T takes the whole tree.
            let _ = self.child.kill();
            force_kill(self.pid);
        }
    }
    let spawn_pid = child.id();
    let guard = Guard {
        child: &mut child,
        pid: spawn_pid,
    };

    let daemon_pid = wait_alive(&data_dir, Duration::from_secs(15))
        .expect("데몬이 15s 내 daemon.json 발행 + alive — 빌드/포트/mutex 확인");
    assert_eq!(
        daemon_pid, spawn_pid,
        "발행된 daemon.json pid 가 우리가 spawn 한 데몬과 일치(stale 잔존 아님)"
    );

    assert!(
        pid_in_tasklist(daemon_pid),
        "send_stop 전 데몬 pid={daemon_pid} 가 OS 에 살아있어야"
    );

    let outcome = send_stop(&data_dir).expect("send_stop 이 에러 없이 반환(일방 발사 송신 성공)");
    assert_eq!(
        outcome,
        StopOutcome::DaemonClosed,
        "데몬이 graceful 하게 연결을 닫아 DaemonClosed 여야(꺼짐 확정 신호). \
         Timeout 이면 데몬이 3s 내 연결을 안 닫은 것 — StopDaemon 처리/타이밍 의심"
    );

    let gone = wait_pid_gone(daemon_pid, Duration::from_secs(10));

    // Reclaim the exit code with try_wait so no zombie is left.
    let exit_code = guard.child.try_wait().ok().flatten();

    assert!(
        gone,
        "send_stop 후 10s 내 데몬 pid={daemon_pid} 가 OS 에서 사라져야(graceful self-exit). \
         아직 살아있으면 핸드셰이크/Auth 거부/StopDaemon 미처리 의심 — RUST_LOG=debug 로 데몬 \
         콘솔 확인. exit_code={exit_code:?}"
    );
    eprintln!("[stop_smoke] graceful self-exit 확인 — pid={daemon_pid} exit_code={exit_code:?}");

    // The guard Drop cleans up the child before deleting tmp → the tmp (tempdir) Drop deletes the temp folder.
}

/// Diagnosis: tells apart whether send_stop's close timing causes StopDaemon to go unprocessed (to pin down the QA root cause).
///
/// Takes a scenario as an argument and changes only the close behavior after sending Auth → StopDaemon:
///   - "noclose"   : no close after flush (connection kept) — diag baseline.
///   - "delayclose": close after waiting 250ms after flush — gives the daemon time to read StopDaemon.
/// If in both the daemon vanishes from the OS (graceful self-exit), the problem is send_stop's **immediate close timing**.
fn diag_send_with_close_mode(mode: &str) -> bool {
    use std::net::TcpStream;
    use tungstenite::Message;

    let exe = daemon_exe_path();
    assert!(exe.is_file(), "데몬 exe 없음 — cargo build -p ...-daemon");
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().to_path_buf();
    let mut child = Command::new(&exe)
        .env("ENGRAM_DATA_DIR", &data_dir)
        .spawn()
        .unwrap();
    let spawn_pid = child.id();

    let daemon_pid = wait_alive(&data_dir, Duration::from_secs(15)).expect("alive");
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
#[ignore = "진단 — send_stop close-race 가설 확인(close 안 함)"]
fn diag_noclose_lets_daemon_exit() {
    assert!(
        diag_send_with_close_mode("noclose"),
        "close 안 하면 데몬이 self-exit 하는가"
    );
}

#[test]
#[ignore = "진단 — send_stop close-race 가설 확인(250ms 후 close)"]
fn diag_delayclose_lets_daemon_exit() {
    assert!(
        diag_send_with_close_mode("delayclose"),
        "close 전 데몬이 read 할 시간을 주면 self-exit 하는가"
    );
}
