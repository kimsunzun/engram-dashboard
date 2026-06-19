//! 격리 하네스 — graceful StopDaemon WS 왕복(실프로세스) end-to-end smoke.
//!
//! 검증 대상: `discovery::send_stop(data_dir)` 을 **실제로** 부르면 살아있는 데몬이
//! graceful 하게(WS Auth → StopDaemon{force} → 데몬 self-exit) 죽는가.
//!
//! 흐름:
//!   1) 임시 폴더를 ENGRAM_DATA_DIR 로 주입해 데몬 .exe 를 직접 spawn(std::process — env 상속).
//!      → 데몬은 default_data_dir() 최우선인 ENGRAM_DATA_DIR 을 보고 그 임시 폴더에 daemon.json 발행.
//!   2) daemon.json 발행 + 데몬 alive 폴링으로 확인(pid 회수).
//!   3) send_stop(임시 폴더) 실호출 — daemon.json 을 읽어 ws://host:port 로 Auth+StopDaemon 일방 발사.
//!   4) 단언: (a) send_stop 이 Err 없이 반환 (b) 데몬이 몇 초 내 OS 프로세스 목록에서 사라짐(tasklist).
//!   5) 정리: 잔존 시 taskkill /F + 임시 폴더 삭제(tempfile Drop).
//!
//! ★현재 알려진 결과(QA 2026-06-19): send_stop_makes_real_daemon_self_exit 는 FAIL 한다★ —
//!   send_stop 이 flush 직후 즉시 ws.close() 하는데, 데몬은 auth 성공 후 첫 outbound(Hello 등)를
//!   write 하려다 닫힌 소켓에 10053 으로 실패 → write_task 종료 → read_task abort(데몬 ws.rs §536
//!   "하나라도 끝나면 상대 abort") → **StopDaemon Text 프레임이 read 되기 전에 read_task 가 죽어**
//!   dispatch 안 됨 → 데몬 생존. 아래 diag_noclose/diag_delayclose 는 PASS(데몬에 read 할 시간을 주면
//!   graceful self-exit, exit code 0). 즉 결함은 데몬/StopDaemon 처리가 아니라 send_stop 의 **즉시
//!   close 타이밍**이다. 메인이 send_stop 수정(예: close 전 짧은 grace 대기 또는 ack 1프레임 read)
//!   후 이 테스트가 PASS 하면 회귀 안전망이 된다. (이 테스트는 그 수정의 검증 자산으로 보존.)
//!
//! ★ENGRAM_DATA_DIR 격리(WMI 경로와 다름)★: 트레이의 운영 spawn 은 WMI Win32_Process.Create 라
//!   자식이 env 를 상속하지 못해 ENGRAM_DATA_DIR 격리가 닿지 않는다(ws_e2e real_wmi_spawn_smoke 참조).
//!   그러나 이 테스트는 **std::process::Command 로 직접 spawn** 하므로 env 가 상속돼 임시 폴더로 완전
//!   격리된다 — 운영 `.engram-data` 를 건드리지 않는다. (직접 spawn 은 send_stop 왕복 검증 목적상
//!   충분하다 — WMI 의 detached 성질은 send_stop 동작과 무관.)
//!
//! 실행: `cargo test -p engram-dashboard-discovery --test stop_smoke -- --ignored`
//!   (기본 `cargo test` 에선 #[ignore] 로 빠진다 — 데몬 exe + 실프로세스 필요. 검증 자산이라 보존.)

#![cfg(windows)]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use std::path::PathBuf;

use engram_dashboard_discovery::{daemon_status, send_stop, StopOutcome};

/// 빌드된 데몬 .exe 경로. discovery::locate_daemon_exe 는 current_exe(deps/) / cwd 기준이라 통합
/// 테스트 실행 디렉토리(crate 폴더)에선 워크스페이스 target 을 못 짚는다. 여기선 CARGO_MANIFEST_DIR
/// (= crates/engram-dashboard-discovery)에서 워크스페이스 루트로 올라가 target/<profile>/exe 를 직접
/// 계산한다(테스트 한정 — 운영 경로는 locate_daemon_exe 그대로).
fn daemon_exe_path() -> PathBuf {
    // 이 테스트 바이너리가 target/<profile>/deps/ 에 있으니 그 두 단계 위가 <profile> 폴더.
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop(); // deps
    p.pop(); // <profile> (debug/release)
    p.join("engram-dashboard-daemon.exe")
}

/// daemon.json 발행 + alive 까지 폴링 대기(상한). 못 뜨면 None.
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

/// OS 레벨로 PID 가 활성 프로세스 목록에 있는가(tasklist 진실원천).
///
/// ★왜 daemon_status 대신 tasklist 인가(load-bearing)★: 본 테스트는 데몬을 std::process 로 직접
/// spawn 하므로 부모(테스트)가 Child 의 프로세스 HANDLE 을 쥔다. 그 핸들이 열려 있는 한 데몬이
/// **종료(exit code 0)해도** OpenProcess+GetProcessTimes(=core liveness 가 쓰는 API)가 죽은 PID 에
/// 대해 계속 creation_time 을 돌려준다(Windows 동작 — 핸들이 프로세스 객체를 유지). 그래서
/// daemon_status 는 false-live 가 된다(테스트 하네스 인공물 — 운영 트레이는 데몬을 자식으로 두지
/// 않고 WMI detached spawn 후 폴링하므로 무관). tasklist 는 **활성 프로세스만** 보여줘 핸들 보유와
/// 무관하게 "실제 종료" 여부를 본다 → graceful self-exit 의 진실원천.
fn pid_in_tasklist(pid: u32) -> bool {
    let out = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .expect("tasklist 실행");
    let s = String::from_utf8_lossy(&out.stdout);
    s.contains(&format!("\"{pid}\""))
}

/// send_stop 후 데몬 프로세스가 OS 에서 사라질 때까지 폴링(상한). 사라지면 true.
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

/// 잔존 데몬 강제 정리(테스트 실패/패닉 경로에서도 좀비 방지).
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

    // 임시 폴더를 ENGRAM_DATA_DIR 로 주입(완전 격리 — 운영 .engram-data 미오염).
    let tmp = tempfile::tempdir().expect("tempdir 생성");
    let data_dir = tmp.path().to_path_buf();

    // 데몬 직접 spawn(env 상속 → 임시 폴더에 daemon.json). 디버그 빌드 데몬은 콘솔 앱 —
    // stdout/stderr 를 상속해 RUST_LOG=debug 시 Auth 거부/파싱 실패가 콘솔에 보인다.
    let mut child: Child = Command::new(&exe)
        .env("ENGRAM_DATA_DIR", &data_dir)
        .spawn()
        .expect("데몬 spawn");

    // RAII 정리 — 단언 실패/패닉에도 child + 그 PID 를 taskkill 로 회수.
    struct Guard<'a> {
        child: &'a mut Child,
        pid: u32,
    }
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            // child.kill 은 직접 자식만 — 혹시 살아있으면 taskkill /T 로 트리째.
            let _ = self.child.kill();
            force_kill(self.pid);
        }
    }
    let spawn_pid = child.id();
    let guard = Guard {
        child: &mut child,
        pid: spawn_pid,
    };

    // daemon.json 발행 + alive 확인(pid 회수). 데몬은 단일-인스턴스 mutex 를 잡고 부팅한다.
    let daemon_pid = wait_alive(&data_dir, Duration::from_secs(15))
        .expect("데몬이 15s 내 daemon.json 발행 + alive — 빌드/포트/mutex 확인");
    assert_eq!(
        daemon_pid, spawn_pid,
        "발행된 daemon.json pid 가 우리가 spawn 한 데몬과 일치(stale 잔존 아님)"
    );

    // send_stop 직전엔 OS 에 살아있어야(검증 전제).
    assert!(
        pid_in_tasklist(daemon_pid),
        "send_stop 전 데몬 pid={daemon_pid} 가 OS 에 살아있어야"
    );

    // ★핵심: send_stop 실호출★ — 살아있는 데몬에 graceful StopDaemon 일방 발사.
    // (a) 에러 없이 반환 + (a') 데몬이 graceful 하게 연결을 닫아 DaemonClosed 여야 한다.
    //     DaemonClosed = drain read 에서 데몬이 self-exit 하며 연결을 닫은 것을 관측 = 꺼짐 확정 신호
    //     (트레이가 이 신호로 PID probe race 없이 아이콘을 회색 확정한다 — StopOutcome 주석).
    let outcome = send_stop(&data_dir).expect("send_stop 이 에러 없이 반환(일방 발사 송신 성공)");
    assert_eq!(
        outcome,
        StopOutcome::DaemonClosed,
        "데몬이 graceful 하게 연결을 닫아 DaemonClosed 여야(꺼짐 확정 신호). \
         Timeout 이면 데몬이 3s 내 연결을 안 닫은 것 — StopDaemon 처리/타이밍 의심"
    );

    // (b) 데몬이 graceful self-exit 해 OS 프로세스 목록에서 사라져야 한다(진실원천 = tasklist;
    //     daemon_status 는 본 테스트 하네스의 핸들 보유로 false-live 라 쓰지 않음 — pid_in_tasklist
    //     주석 참조). 일방 발사라도 flush 로 StopDaemon 이 도달하면 데몬은 shutdown_all + self-exit.
    let gone = wait_pid_gone(daemon_pid, Duration::from_secs(10));

    // wait()로 종료코드 회수(좀비 방지) — self-exit 했으면 ExitStatus(0).
    let exit_code = guard.child.try_wait().ok().flatten();

    assert!(
        gone,
        "send_stop 후 10s 내 데몬 pid={daemon_pid} 가 OS 에서 사라져야(graceful self-exit). \
         아직 살아있으면 핸드셰이크/Auth 거부/StopDaemon 미처리 의심 — RUST_LOG=debug 로 데몬 \
         콘솔 확인. exit_code={exit_code:?}"
    );
    eprintln!("[stop_smoke] graceful self-exit 확인 — pid={daemon_pid} exit_code={exit_code:?}");

    // guard Drop 이 tmp 삭제 전에 child 정리 → tmp(tempdir) Drop 이 임시 폴더 삭제.
}

/// 진단: send_stop 의 close 타이밍이 StopDaemon 미처리를 일으키는지 가린다(QA 근본원인 확정용).
///
/// 시나리오를 인자로 받아 Auth → StopDaemon 송신 후 close 거동만 바꾼다:
///   - "noclose"   : flush 후 close 안 함(연결 유지) — diag baseline.
///   - "delayclose": flush 후 250ms 대기 뒤 close — 데몬이 StopDaemon 을 read 할 시간 부여.
/// 둘 다 데몬이 OS 에서 사라지면(graceful self-exit) 문제는 send_stop 의 **즉시 close 타이밍**이다
/// (데몬측: write_task 가 auth 후 첫 write 에서 10053 → read_task abort → StopDaemon read 전 종료).
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
        &std::fs::read(data_dir.join("daemon.json")).unwrap(),
    )
    .unwrap();

    let url = format!("ws://{}:{}", info.host, info.port);
    let stream = TcpStream::connect(format!("{}:{}", info.host, info.port)).unwrap();
    let (mut ws, _r) = tungstenite::client(&url, stream).unwrap();
    let auth = serde_json::to_string(&engram_dashboard_protocol::AgentCommand::Auth {
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
    // tasklist 진실원천(핸들 잔존 인공물 회피 — pid_in_tasklist 주석).
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
