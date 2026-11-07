//! Child-process tests of the daemon panic hook (`init_logging_and_panic_hook`) (TRD S21-panic-policy §6).
//!
//! The hook is process-global and the out-of-mark branch ends the process — so this test binary relaunches itself as one branch
//! and the hook is installed only in that child. ★The child body ([`child_process_entry`]) returns at once if there is no branch environment variable★
//! — if in an ordinary test run that body installed the ending hook in the shared test process, from then on any panic of this binary
//! would end the whole test run.
//!
//! The exit code of the ending differs by OS (Windows = `0xC0000409` · Unix = SIGABRT so no code). So instead of
//! pinning the code, we look together at
//! 「성공 아님 · libtest 의 보통 실패 코드(101) 아님 · 마지막 줄 없음 · 로그 파일의 사유 줄」 (not success · not libtest's usual failure code (101) · no last line · the reason line in the log file).
// ADR-0292

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use engram_dashboard_base::panic::{contain, uncontained};

use crate::data_dir::DataLayout;

const SCENARIO_ENV: &str = "ENGRAM_PANIC_HOOK_CHILD_SCENARIO";
const DATA_DIR_ENV: &str = "ENGRAM_PANIC_HOOK_CHILD_DATA_DIR";

const LOG_PREFIX: &str = "PANIC-HOOK-CHILD-LOG=";
const CAUGHT_PREFIX: &str = "PANIC-HOOK-CHILD-CAUGHT=";
const SURVIVED: &str = "PANIC-HOOK-CHILD-SURVIVED";
const PROBE_THREAD: &str = "panic-hook-probe";

/// The cap after which the hook is deemed to have neither ended nor returned but stalled.
const CHILD_LIMIT: Duration = Duration::from_secs(60);

/// The cap on waiting for that pipe to close after the child ended — it does not close if another process that inherited the pipe handle is
/// still alive.
const DRAIN_LIMIT: Duration = Duration::from_secs(10);

/// The cap on waiting for a child killed after the deadline to actually disappear.
const KILL_LIMIT: Duration = Duration::from_secs(10);

/// The exit code when the test failed because of a panic caught by libtest. If the hook ended the process, this code is not reached.
const LIBTEST_FAILED: i32 = 101;

#[derive(Clone, Copy, Debug)]
enum Scenario {
    ThreadOutsideAnyMark,
    UncontainedInsideContain,
    InsideContain,
}

impl Scenario {
    const ALL: [Scenario; 3] = [
        Scenario::ThreadOutsideAnyMark,
        Scenario::UncontainedInsideContain,
        Scenario::InsideContain,
    ];

    fn name(self) -> &'static str {
        match self {
            Scenario::ThreadOutsideAnyMark => "thread-outside-any-mark",
            Scenario::UncontainedInsideContain => "uncontained-inside-contain",
            Scenario::InsideContain => "inside-contain",
        }
    }

    /// Set differently for each branch — tells whether the reason line in the log is that branch's panic.
    fn message(self) -> &'static str {
        match self {
            Scenario::ThreadOutsideAnyMark => "표시 밖 스레드 패닉",
            Scenario::UncontainedInsideContain => "표시 끔 안 패닉",
            Scenario::InsideContain => "가둔 패닉",
        }
    }
}

/// The child process body — the parent ([`run_child`]) selects only this with `--exact` and passes the branch · data folder through environment variables.
/// It installs logging · the hook with the same function as the daemon `run()`, then raises the branch's panic, and if it survives prints [`SURVIVED`].
#[test]
fn child_process_entry() {
    let Ok(name) = std::env::var(SCENARIO_ENV) else {
        return;
    };
    let scenario = Scenario::ALL
        .into_iter()
        .find(|s| s.name() == name)
        .unwrap_or_else(|| panic!("모르는 갈래 {name}"));
    let data_dir = std::env::var_os(DATA_DIR_ENV).expect("데이터 폴더 환경 변수");
    let log =
        crate::init_logging_and_panic_hook(&DataLayout::new(data_dir)).expect("파일 로그가 열려야");
    println!("{LOG_PREFIX}{}", log.display());

    let message = scenario.message();
    match scenario {
        Scenario::ThreadOutsideAnyMark => {
            let probe = std::thread::Builder::new()
                .name(PROBE_THREAD.to_string())
                .spawn(move || -> u32 { panic!("{message}") })
                .expect("스레드 띄우기");
            let _ = probe.join();
        }
        Scenario::UncontainedInsideContain => {
            let _ = contain(move || uncontained(move || -> u32 { panic!("{message}") }));
        }
        Scenario::InsideContain => {
            let caught = contain(move || -> u32 { panic!("{message}") }).expect_err("가둬야");
            println!("{CAUGHT_PREFIX}{}", caught.message());
        }
    }
    println!("{SURVIVED}");
}

struct Ran {
    status: ExitStatus,
    stdout: String,
    stderr: String,
    log: String,
}

impl Ran {
    fn report(&self) -> String {
        format!(
            "상태: {:?}\n── stdout ──\n{}\n── stderr ──\n{}\n── 로그 ──\n{}",
            self.status, self.stdout, self.stderr, self.log
        )
    }
}

/// A new data folder per child — erased by drop (even if an assertion fails).
struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new(scenario: Scenario) -> Self {
        Self(std::env::temp_dir().join(format!(
            "engram-panic-hook-{}-{}",
            scenario.name(),
            uuid::Uuid::new_v4()
        )))
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The name by which libtest calls [`child_process_entry`] = the module path with the crate name stripped off.
fn child_test_name() -> String {
    let module = module_path!().split_once("::").map_or("", |(_, rest)| rest);
    format!("{module}::child_process_entry")
}

fn drain(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<io::Result<String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let read = pipe
            .read_to_end(&mut bytes)
            .map(|_| String::from_utf8_lossy(&bytes).into_owned());
        let _ = tx.send(read);
    });
    rx
}

fn drained(rx: mpsc::Receiver<io::Result<String>>, pipe: &str) -> String {
    match rx.recv_timeout(DRAIN_LIMIT) {
        Ok(Ok(text)) => text,
        Ok(Err(e)) => panic!("자식 {pipe} 를 읽지 못했다: {e}"),
        Err(RecvTimeoutError::Timeout) => {
            panic!("자식 {pipe} 가 {DRAIN_LIMIT:?} 안에 닫히지 않았다 — 그 파이프를 물려받은 프로세스가 살아 있다")
        }
        Err(RecvTimeoutError::Disconnected) => {
            panic!("자식 {pipe} 를 읽던 스레드가 결과 없이 끝났다")
        }
    }
}

fn run_child(scenario: Scenario) -> Ran {
    let dir = ScratchDir::new(scenario);
    let mut child = Command::new(std::env::current_exe().expect("시험 바이너리 경로"))
        .args([
            child_test_name().as_str(),
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(SCENARIO_ENV, scenario.name())
        .env(DATA_DIR_ENV, &dir.0)
        // Fixed so the reason line (ERROR) is not filtered by the parent environment's filter · and the default hook's backtrace collection does not delay it.
        .env("RUST_LOG", "error")
        .env("RUST_BACKTRACE", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("자식 띄우기");
    let stdout = drain(child.stdout.take().expect("stdout 파이프"));
    let stderr = drain(child.stderr.take().expect("stderr 파이프"));

    let mut status = None;
    let ended = engram_dashboard_base::testing::wait_until(CHILD_LIMIT, || {
        status = child.try_wait().expect("자식 상태");
        status.is_some()
    });
    // Exceeding the deadline is a failure regardless of the cleanup result — do not continue the decision from the state of a child that ended late.
    if !ended {
        let cleanup = match child.kill() {
            Err(e) => format!("끄기 실패: {e}"),
            Ok(()) => {
                let reaped = engram_dashboard_base::testing::wait_until(KILL_LIMIT, || {
                    matches!(child.try_wait(), Ok(Some(_)))
                });
                if reaped {
                    "끄고 거둠".to_string()
                } else {
                    format!("끈 뒤 {KILL_LIMIT:?} 안에 거두지 못함")
                }
            }
        };
        panic!("자식이 {CHILD_LIMIT:?} 안에 끝나지 않았다(정리: {cleanup})");
    }
    let status = status.expect("끝난 자식은 상태가 있다");
    let stdout = drained(stdout, "stdout");
    let stderr = drained(stderr, "stderr");

    // Do not look for it at the head of the line — libtest first prints `test <이름> ... ` (name) on the same line without a line break.
    let Some(log_path) = stdout
        .split_once(LOG_PREFIX)
        .and_then(|(_, rest)| rest.lines().next())
    else {
        panic!("자식이 로그 경로를 찍지 않았다(상태 {status:?})\n── stdout ──\n{stdout}\n── stderr ──\n{stderr}");
    };
    let log_path = Path::new(log_path);
    assert!(
        log_path.starts_with(DataLayout::new(&dir.0).logs_dir()),
        "데몬 로그는 그 데이터 폴더의 logs 아래다: {}",
        log_path.display()
    );
    let log = std::fs::read_to_string(log_path)
        .unwrap_or_else(|e| panic!("로그 {} 읽기: {e}", log_path.display()));
    Ran {
        status,
        stdout,
        stderr,
        log,
    }
}

fn assert_reason_logged(ran: &Ran, scenario: Scenario) {
    let reason = format!("스레드 panic: {}", scenario.message());
    assert!(
        ran.log.contains(&reason),
        "로그에 사유 줄 「{reason}」이 없다\n{}",
        ran.report()
    );
    assert!(
        ran.log.contains("panic_hook_tests.rs"),
        "사유 줄은 패닉 자리를 단다\n{}",
        ran.report()
    );
}

fn assert_ended_by_the_hook(ran: &Ran, scenario: Scenario) {
    assert!(!ran.status.success(), "끝났어야\n{}", ran.report());
    assert_ne!(
        ran.status.code(),
        Some(LIBTEST_FAILED),
        "libtest 가 잡은 실패다 — 훅이 끝내지 않았다\n{}",
        ran.report()
    );
    assert!(
        !ran.stdout.contains(SURVIVED),
        "패닉 뒤를 돌았다\n{}",
        ran.report()
    );
    assert_reason_logged(ran, scenario);
}

#[test]
fn a_panic_on_a_thread_outside_any_mark_is_logged_and_ends_the_daemon() {
    let ran = run_child(Scenario::ThreadOutsideAnyMark);
    assert_ended_by_the_hook(&ran, Scenario::ThreadOutsideAnyMark);
    assert!(
        ran.log.contains(PROBE_THREAD),
        "사유 줄은 패닉한 스레드를 단다\n{}",
        ran.report()
    );
}

#[test]
fn a_panic_under_uncontained_inside_contain_ends_the_daemon() {
    let ran = run_child(Scenario::UncontainedInsideContain);
    assert_ended_by_the_hook(&ran, Scenario::UncontainedInsideContain);
}

#[test]
fn a_panic_inside_contain_is_logged_and_the_daemon_keeps_running() {
    let ran = run_child(Scenario::InsideContain);
    assert!(ran.status.success(), "계속 돌았어야\n{}", ran.report());
    let caught = format!("{CAUGHT_PREFIX}{}", Scenario::InsideContain.message());
    assert!(
        ran.stdout.contains(&caught),
        "contain 이 잡은 문장\n{}",
        ran.report()
    );
    assert!(ran.stdout.contains(SURVIVED), "{}", ran.report());
    assert_reason_logged(&ran, Scenario::InsideContain);
}
