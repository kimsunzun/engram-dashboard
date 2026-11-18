//! Child-process tests of the daemon panic hook (`init_logging_and_panic_hook`) (TRD S21-panic-policy §6).
//!
//! The hook is process-global and the out-of-mark branch ends the process, so this test binary relaunches itself as one branch and the hook is installed only in that child. ★The child body ([`child_process_entry`]) returns at once without the branch environment variable★ — otherwise an ordinary test run would install the ending hook in the shared test process and any later panic would end the whole run.
//!
//! The exit code of the ending differs by OS (Windows `0xC0000409` · Unix SIGABRT, no code), so instead of pinning it we check together: not success · not libtest's usual failure code (101) · no last line · the reason line in the log file.
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

/// After this cap the hook is deemed stalled (neither ended nor returned).
const CHILD_LIMIT: Duration = Duration::from_secs(60);

/// Cap on waiting for that pipe to close after the child ended — it does not close if another process that inherited the pipe handle is alive.
const DRAIN_LIMIT: Duration = Duration::from_secs(10);

/// Cap on waiting for a child killed after the deadline to disappear.
const KILL_LIMIT: Duration = Duration::from_secs(10);

/// Exit code when the test failed from a panic caught by libtest; not reached if the hook ended the process.
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

    /// Set per branch — tells whether the log's reason line is that branch's panic.
    fn message(self) -> &'static str {
        match self {
            Scenario::ThreadOutsideAnyMark => "thread panic outside any mark",
            Scenario::UncontainedInsideContain => "panic inside with the mark off",
            Scenario::InsideContain => "contained panic",
        }
    }
}

/// The child process body — the parent ([`run_child`]) selects only this with `--exact` and passes the branch · data folder through environment variables. It installs logging · the hook with the same function as the daemon `run()`, raises the branch's panic, and prints [`SURVIVED`] if it survives.
#[test]
fn child_process_entry() {
    let Ok(name) = std::env::var(SCENARIO_ENV) else {
        return;
    };
    let scenario = Scenario::ALL
        .into_iter()
        .find(|s| s.name() == name)
        .unwrap_or_else(|| panic!("unknown branch {name}"));
    let data_dir = std::env::var_os(DATA_DIR_ENV).expect("data folder environment variable");
    let log = crate::init_logging_and_panic_hook(&DataLayout::new(data_dir))
        .expect("the file log must open");
    println!("{LOG_PREFIX}{}", log.display());

    let message = scenario.message();
    match scenario {
        Scenario::ThreadOutsideAnyMark => {
            let probe = std::thread::Builder::new()
                .name(PROBE_THREAD.to_string())
                .spawn(move || -> u32 { panic!("{message}") })
                .expect("spawn thread");
            let _ = probe.join();
        }
        Scenario::UncontainedInsideContain => {
            let _ = contain(move || uncontained(move || -> u32 { panic!("{message}") }));
        }
        Scenario::InsideContain => {
            let caught = contain(move || -> u32 { panic!("{message}") }).expect_err("must contain");
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
            "status: {:?}\n── stdout ──\n{}\n── stderr ──\n{}\n── log ──\n{}",
            self.status, self.stdout, self.stderr, self.log
        )
    }
}

/// A new data folder per child — erased by drop (even on assertion failure).
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

/// The name by which libtest calls [`child_process_entry`] = the module path with the crate name stripped.
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
        Ok(Err(e)) => panic!("could not read child {pipe}: {e}"),
        Err(RecvTimeoutError::Timeout) => {
            panic!("child {pipe} did not close within {DRAIN_LIMIT:?} — a process that inherited that pipe is alive")
        }
        Err(RecvTimeoutError::Disconnected) => {
            panic!("the thread reading child {pipe} ended without a result")
        }
    }
}

fn run_child(scenario: Scenario) -> Ran {
    let dir = ScratchDir::new(scenario);
    let mut child = Command::new(std::env::current_exe().expect("test binary path"))
        .args([
            child_test_name().as_str(),
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(SCENARIO_ENV, scenario.name())
        .env(DATA_DIR_ENV, &dir.0)
        // Fixed so the reason line (ERROR) is not filtered by the parent environment's filter and the default hook's backtrace collection does not delay it.
        .env("RUST_LOG", "error")
        .env("RUST_BACKTRACE", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn child");
    let stdout = drain(child.stdout.take().expect("stdout pipe"));
    let stderr = drain(child.stderr.take().expect("stderr pipe"));

    let mut status = None;
    let ended = engram_dashboard_base::testing::wait_until(CHILD_LIMIT, || {
        status = child.try_wait().expect("child status");
        status.is_some()
    });
    // Exceeding the deadline is a failure regardless of the cleanup result — do not continue deciding from a child that ended late.
    if !ended {
        let cleanup = match child.kill() {
            Err(e) => format!("kill failed: {e}"),
            Ok(()) => {
                let reaped = engram_dashboard_base::testing::wait_until(KILL_LIMIT, || {
                    matches!(child.try_wait(), Ok(Some(_)))
                });
                if reaped {
                    "killed and reaped".to_string()
                } else {
                    format!("not reaped within {KILL_LIMIT:?} after kill")
                }
            }
        };
        panic!("the child did not end within {CHILD_LIMIT:?} (cleanup: {cleanup})");
    }
    let status = status.expect("an ended child has a status");
    let stdout = drained(stdout, "stdout");
    let stderr = drained(stderr, "stderr");

    // Not at the head of the line — libtest first prints `test <name> ... ` on the same line without a line break.
    let Some(log_path) = stdout
        .split_once(LOG_PREFIX)
        .and_then(|(_, rest)| rest.lines().next())
    else {
        panic!("the child did not print the log path (status {status:?})\n── stdout ──\n{stdout}\n── stderr ──\n{stderr}");
    };
    let log_path = Path::new(log_path);
    assert!(
        log_path.starts_with(DataLayout::new(&dir.0).logs_dir()),
        "the daemon log is under logs in that data folder: {}",
        log_path.display()
    );
    let log = std::fs::read_to_string(log_path)
        .unwrap_or_else(|e| panic!("read log {}: {e}", log_path.display()));
    Ran {
        status,
        stdout,
        stderr,
        log,
    }
}

fn assert_reason_logged(ran: &Ran, scenario: Scenario) {
    let reason = format!("thread panicked: {}", scenario.message());
    assert!(
        ran.log.contains(&reason),
        "the log has no reason line 「{reason}」\n{}",
        ran.report()
    );
    assert!(
        ran.log.contains("panic_hook_tests.rs"),
        "the reason line carries the panic location\n{}",
        ran.report()
    );
}

fn assert_ended_by_the_hook(ran: &Ran, scenario: Scenario) {
    assert!(!ran.status.success(), "must have ended\n{}", ran.report());
    assert_ne!(
        ran.status.code(),
        Some(LIBTEST_FAILED),
        "it is a failure libtest caught — the hook did not end it\n{}",
        ran.report()
    );
    assert!(
        !ran.stdout.contains(SURVIVED),
        "it ran past the panic\n{}",
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
        "the reason line carries the panicking thread\n{}",
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
    assert!(
        ran.status.success(),
        "must have kept running\n{}",
        ran.report()
    );
    let caught = format!("{CAUGHT_PREFIX}{}", Scenario::InsideContain.message());
    assert!(
        ran.stdout.contains(&caught),
        "the sentence contain caught\n{}",
        ran.report()
    );
    assert!(ran.stdout.contains(SURVIVED), "{}", ran.report());
    assert_reason_logged(&ran, Scenario::InsideContain);
}
