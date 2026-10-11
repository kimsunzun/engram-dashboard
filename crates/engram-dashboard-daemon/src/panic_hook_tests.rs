//! 데몬 패닉 훅(`init_logging_and_panic_hook`)의 자식 프로세스 시험(TRD S21-panic-policy §6).
//!
//! 훅은 프로세스 전역이고 표시 밖 갈래는 프로세스를 끝낸다 — 그래서 이 시험 바이너리가 자기 자신을 갈래
//! 하나로 다시 띄우고, 훅은 그 자식에서만 건다. ★자식 몸통([`child_process_entry`])은 갈래 환경 변수가 없으면
//! 곧장 돌아간다★ — 보통의 시험 실행에서 그 몸통이 공유 시험 프로세스에 끝내는 훅을 걸면, 그 뒤로 이 바이너리의
//! 어느 패닉이든 시험 실행 전체를 끝낸다.
//!
//! 끝냄의 종료 코드는 OS 마다 다르다(Windows = `0xC0000409` · Unix = SIGABRT 라 코드 없음). 그래서 코드를
//! 못 박지 않고 「성공 아님 · libtest 의 보통 실패 코드(101) 아님 · 마지막 줄 없음 · 로그 파일의 사유 줄」을
//! 함께 본다.
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

/// 훅이 끝내지도 돌아가지도 않고 멎었다고 볼 상한.
const CHILD_LIMIT: Duration = Duration::from_secs(60);

/// 자식이 끝난 뒤 그 파이프가 닫히기를 기다리는 상한 — 파이프 핸들을 물려받은 다른 프로세스가 살아 있으면
/// 닫히지 않는다.
const DRAIN_LIMIT: Duration = Duration::from_secs(10);

/// 시한을 넘겨 끈 자식이 실제로 사라지기를 기다리는 상한.
const KILL_LIMIT: Duration = Duration::from_secs(10);

/// libtest 가 잡은 패닉으로 시험이 실패했을 때의 종료 코드. 훅이 끝냈다면 이 코드에 닿지 않는다.
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

    /// 갈래마다 다르게 둔다 — 로그의 사유 줄이 그 갈래의 패닉인지 가른다.
    fn message(self) -> &'static str {
        match self {
            Scenario::ThreadOutsideAnyMark => "표시 밖 스레드 패닉",
            Scenario::UncontainedInsideContain => "표시 끔 안 패닉",
            Scenario::InsideContain => "가둔 패닉",
        }
    }
}

/// 자식 프로세스의 몸통 — 부모([`run_child`])가 `--exact` 로 이것만 고르고 갈래 · 데이터 폴더를 환경 변수로
/// 준다. 데몬 `run()` 과 같은 함수로 로그 · 훅을 건 뒤 갈래의 패닉을 내고, 살아남으면 [`SURVIVED`] 를 찍는다.
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

/// 자식마다 새 데이터 폴더 — drop 이 지운다(단언이 실패해도).
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

/// libtest 가 [`child_process_entry`] 를 부르는 이름 = 모듈 경로에서 crate 이름을 뗀 것.
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
        // 사유 줄(ERROR)이 부모 환경의 필터에 걸러지지 않게 · 기본 훅의 백트레이스 수집이 늦추지 않게 고정한다.
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
    // 시한을 넘기면 정리 결과와 무관하게 실패다 — 늦게 끝난 자식의 상태로 판정을 잇지 않는다.
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

    // 줄 머리로 찾지 않는다 — libtest 가 같은 줄에 `test <이름> ... ` 을 줄바꿈 없이 먼저 찍는다.
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
