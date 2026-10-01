//! 조회 시험 대역 — 대본대로 움직이는 가짜 [`ProbeSpawner`] 와 시험용 임시 루트. 벤더 조회기의 단위 시험이
//! 실 CLI 없이 대화·시한·kill 을 재는 데 쓴다.
//!
//! ```ignore
//! let root = TempRoot::new("vendor-probe");
//! let spawner = ScriptedSpawner::new(
//!     ChildScript::new()
//!         .line(r#"{"type":"system"}"#)
//!         // 조회기가 쓴 줄(요청 id 가 든)을 보고 응답을 만든다.
//!         .reply(|written| response_for(&written[0])),
//! );
//! let env = ProbeEnv {
//!     spawner: &spawner,
//!     deadline: Instant::now() + Duration::from_secs(5),
//!     scratch_root: root.path(),
//! };
//! let obs = PROBE.query(&env).expect("조회");
//! assert!(!spawner.commands()[0].args.iter().any(|a| a == "--forbidden"));
//! assert!(spawner.stdin_closed() && spawner.child_dropped());
//! ```
//!
//! 대본의 뜻 — 실물 [`super::OsProbeSpawner`] 와 같은 규칙을 따른다:
//!   - 마감이 이미 지난 동작은 곧바로 `Timeout` 이고 「마감 kill」로 기록된다. 한 번 끊긴 뒤에는 전부 `Timeout`.
//!   - 「막힘」(`hang_after_lines` · `block_writes` · `never_exit`)은 **마감까지 실제로 잔다** — 그래서 막힘을
//!     재는 시험은 짧은 마감(수백 ms)을 넘긴다. ★마감이 [`FAKE_HANG_MAX`] 보다 멀면 패닉한다★ — 조회기가
//!     `env.deadline` 을 안 넘기고 먼 마감을 스스로 만든 것이라, 그대로 두면 시험이 멈춘다.
//!   - 쓰기는 막힘·실패(`fail_writes`)가 아니면 늘 받아들여 기록한다(내용 검사는 시험이
//!     [`ScriptedSpawner::written`] 으로). `close_stdin` 뒤의 쓰기는 `Io` 다. stdin 을 닫아도 대본은 그대로 간다 — 「닫으면 끝나는 자식」은 대본의
//!     기본값(곧바로 EOF · 종료)이고, 「닫아도 안 끝나는 자식」은 막힘으로 적는다.
//!   - 마감이 이미 지난 `spawn` 은 대본을 쓰지 않고 `Timeout` 이다(명령은 기록된다).
//!   - 자식은 하나다 — 두 번째 `spawn` 은 패닉한다(조회 한 번에 자식 하나).

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::{ExitInfo, ProbeChild, ProbeCommand, ProbeError, ProbeSpawner};

/// 가짜 자식이 「막힘」으로 잘 수 있는 가장 긴 시간.
pub(crate) const FAKE_HANG_MAX: Duration = Duration::from_secs(5);

/// 시험마다 새로 만드는 임시 폴더. drop 하면 지운다.
pub(crate) struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    pub(crate) fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("engram-usage-{label}-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&path).expect("시험 임시 루트");
        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

type ReplyFn = Box<dyn FnOnce(&[String]) -> String + Send>;

enum Out {
    Line(String),
    Reply(ReplyFn),
}

enum StdoutEnd {
    Eof,
    Hang,
}

enum Exit {
    Now(ExitInfo),
    Never,
}

/// 가짜 자식 하나의 대본. 기본 = 쓰기 받음 · 줄 없음 · 곧바로 stdout EOF · 종료 코드 0.
pub(crate) struct ChildScript {
    stdout: VecDeque<Out>,
    stdout_end: StdoutEnd,
    block_writes: bool,
    fail_writes: bool,
    exit: Exit,
    stderr_tail: Vec<String>,
}

impl Default for ChildScript {
    fn default() -> Self {
        Self::new()
    }
}

impl ChildScript {
    pub(crate) fn new() -> Self {
        Self {
            stdout: VecDeque::new(),
            stdout_end: StdoutEnd::Eof,
            block_writes: false,
            fail_writes: false,
            exit: Exit::Now(ExitInfo::new(Some(0), false)),
            stderr_tail: Vec::new(),
        }
    }

    /// `read_line` 이 차례로 돌려줄 줄 하나.
    pub(crate) fn line(mut self, line: impl Into<String>) -> Self {
        self.stdout.push_back(Out::Line(line.into()));
        self
    }

    /// 읽히는 순간에 만드는 줄 — 그때까지 조회기가 쓴 줄 전부를 받는다(요청 id 를 되돌려야 할 때).
    pub(crate) fn reply(mut self, make: impl FnOnce(&[String]) -> String + Send + 'static) -> Self {
        self.stdout.push_back(Out::Reply(Box::new(make)));
        self
    }

    /// 줄을 다 준 뒤 EOF 대신 마감까지 아무것도 안 준다(살아서 말이 없는 자식).
    pub(crate) fn hang_after_lines(mut self) -> Self {
        self.stdout_end = StdoutEnd::Hang;
        self
    }

    /// stdin 을 안 읽는 자식 — 모든 `write_line` 이 마감까지 막힌다.
    pub(crate) fn block_writes(mut self) -> Self {
        self.block_writes = true;
        self
    }

    /// stdin 이 이미 닫힌 자식(벌써 끝난 셸 등) — 모든 `write_line` 이 곧바로 `Io` 다.
    pub(crate) fn fail_writes(mut self) -> Self {
        self.fail_writes = true;
        self
    }

    pub(crate) fn exit_code(mut self, code: i32) -> Self {
        self.exit = Exit::Now(ExitInfo::new(Some(code), false));
        self
    }

    /// 감싸는 셸이 「그런 명령 없음」으로 끝난 것처럼([`ExitInfo::looks_not_installed`]).
    pub(crate) fn exit_not_installed(mut self) -> Self {
        self.exit = Exit::Now(ExitInfo::new(Some(9009), true));
        self
    }

    /// `wait_exit` 가 마감까지 안 돌아온다(stdout 을 닫고도 사는 자식).
    pub(crate) fn never_exit(mut self) -> Self {
        self.exit = Exit::Never;
        self
    }

    pub(crate) fn stderr_tail(mut self, lines: &[&str]) -> Self {
        self.stderr_tail = lines.iter().map(|line| (*line).to_owned()).collect();
        self
    }
}

#[derive(Default)]
struct Record {
    commands: Vec<ProbeCommand>,
    written: Vec<String>,
    stdin_closed: bool,
    cut_at_deadline: bool,
    exit_judged: bool,
    dropped: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 자식 하나를 대본대로 내주고, 받은 명령·쓰인 줄·끊김을 기록하는 가짜 스포너.
pub(crate) struct ScriptedSpawner {
    script: Mutex<Option<Result<ChildScript, ProbeError>>>,
    record: Arc<Mutex<Record>>,
}

impl ScriptedSpawner {
    pub(crate) fn new(script: ChildScript) -> Self {
        Self::with(Ok(script))
    }

    /// `spawn` 이 이 오류로 실패한다(명령은 기록된다).
    pub(crate) fn failing(err: ProbeError) -> Self {
        Self::with(Err(err))
    }

    fn with(script: Result<ChildScript, ProbeError>) -> Self {
        Self {
            script: Mutex::new(Some(script)),
            record: Arc::new(Mutex::new(Record::default())),
        }
    }

    /// 받은 기동 명세(지금까지 전부).
    pub(crate) fn commands(&self) -> Vec<ProbeCommand> {
        lock(&self.record).commands.clone()
    }

    /// 조회기가 `write_line` 으로 쓴 줄(개행 없이, 쓴 차례대로).
    pub(crate) fn written(&self) -> Vec<String> {
        lock(&self.record).written.clone()
    }

    /// 조회기가 stdin 을 닫았나(`close_stdin`).
    pub(crate) fn stdin_closed(&self) -> bool {
        lock(&self.record).stdin_closed
    }

    /// 어느 동작이 마감에 걸려 트리 kill 이 났나.
    pub(crate) fn cut_at_deadline(&self) -> bool {
        lock(&self.record).cut_at_deadline
    }

    /// 조회기가 「프로그램 없음」 판정이 붙는 종료 대기(`wait_exit`)를 불렀나 — `wait_exit_code` 는 세지 않는다.
    pub(crate) fn exit_judged(&self) -> bool {
        lock(&self.record).exit_judged
    }

    /// 자식이 drop 됐나(= 트리 kill + 종료 대기가 돌았다).
    pub(crate) fn child_dropped(&self) -> bool {
        lock(&self.record).dropped
    }
}

impl ProbeSpawner for ScriptedSpawner {
    fn spawn(
        &self,
        cmd: &ProbeCommand,
        deadline: Instant,
    ) -> Result<Box<dyn ProbeChild>, ProbeError> {
        lock(&self.record).commands.push(cmd.clone());
        if Instant::now() >= deadline {
            return Err(ProbeError::Timeout);
        }
        let script = lock(&self.script)
            .take()
            .expect("가짜 스포너: 대본의 자식은 하나다 — 조회 한 번에 spawn 을 두 번 불렀다");
        let script = script?;
        Ok(Box::new(ScriptedChild {
            script,
            record: self.record.clone(),
            cut: false,
        }))
    }
}

struct ScriptedChild {
    script: ChildScript,
    record: Arc<Mutex<Record>>,
    cut: bool,
}

impl ScriptedChild {
    /// 마감 규칙 공통부 — `Some` 이면 그 오류로 곧바로 끝낸다.
    fn gate(&mut self, deadline: Instant) -> Option<ProbeError> {
        if self.cut {
            return Some(ProbeError::Timeout);
        }
        if Instant::now() >= deadline {
            return Some(self.cut());
        }
        None
    }

    fn cut(&mut self) -> ProbeError {
        self.cut = true;
        lock(&self.record).cut_at_deadline = true;
        ProbeError::Timeout
    }

    fn exit_after(&mut self, deadline: Instant) -> Result<ExitInfo, ProbeError> {
        if let Some(err) = self.gate(deadline) {
            return Err(err);
        }
        match self.script.exit {
            Exit::Now(exit) => Ok(exit),
            Exit::Never => Err(self.hang(deadline)),
        }
    }

    fn hang(&mut self, deadline: Instant) -> ProbeError {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            left <= FAKE_HANG_MAX,
            "가짜 자식: 막힘을 마감({left:?} 뒤)까지 자야 하는데 너무 멀다 — 조회기가 env.deadline 을 안 넘긴 것 같다"
        );
        thread::sleep(left);
        self.cut()
    }
}

impl ProbeChild for ScriptedChild {
    fn write_line(&mut self, line: &str, deadline: Instant) -> Result<(), ProbeError> {
        if let Some(err) = self.gate(deadline) {
            return Err(err);
        }
        if lock(&self.record).stdin_closed {
            return Err(ProbeError::Io("stdin 이 닫혔다".to_owned()));
        }
        if self.script.fail_writes {
            return Err(ProbeError::Io("stdin 쓰기 실패".to_owned()));
        }
        if self.script.block_writes {
            return Err(self.hang(deadline));
        }
        lock(&self.record).written.push(line.to_owned());
        Ok(())
    }

    fn close_stdin(&mut self) {
        lock(&self.record).stdin_closed = true;
    }

    fn read_line(&mut self, deadline: Instant) -> Result<Option<String>, ProbeError> {
        if let Some(err) = self.gate(deadline) {
            return Err(err);
        }
        match self.script.stdout.pop_front() {
            Some(Out::Line(line)) => Ok(Some(line)),
            Some(Out::Reply(make)) => {
                let written = lock(&self.record).written.clone();
                Ok(Some(make(&written)))
            }
            None => match self.script.stdout_end {
                StdoutEnd::Eof => Ok(None),
                StdoutEnd::Hang => Err(self.hang(deadline)),
            },
        }
    }

    fn wait_exit(&mut self, deadline: Instant) -> Result<ExitInfo, ProbeError> {
        lock(&self.record).exit_judged = true;
        self.exit_after(deadline)
    }

    fn wait_exit_code(&mut self, deadline: Instant) -> Result<Option<i32>, ProbeError> {
        self.exit_after(deadline).map(|exit| exit.code())
    }

    fn stderr_tail(&self) -> Vec<String> {
        self.script.stderr_tail.clone()
    }
}

impl Drop for ScriptedChild {
    fn drop(&mut self) {
        lock(&self.record).dropped = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHORT: Duration = Duration::from_millis(50);

    fn command() -> ProbeCommand {
        ProbeCommand {
            program: "prog".into(),
            args: vec!["--a".into()],
            cwd: PathBuf::from("."),
            env_set: vec![("K".into(), "v".into())],
            env_remove: vec!["GONE".into()],
        }
    }

    fn far() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }

    #[test]
    fn a_scripted_conversation_records_what_the_probe_did() {
        let spawner = ScriptedSpawner::new(
            ChildScript::new()
                .line("hello")
                .reply(|written| format!("echo:{}", written.join("|")))
                .exit_code(3)
                .stderr_tail(&["warn"]),
        );
        let mut child = spawner.spawn(&command(), far()).expect("기동");
        assert_eq!(child.read_line(far()), Ok(Some("hello".into())));
        child.write_line("req-1", far()).expect("쓰기");
        assert_eq!(child.read_line(far()), Ok(Some("echo:req-1".into())));
        assert_eq!(child.read_line(far()), Ok(None));
        assert_eq!(child.read_line(far()), Ok(None));
        let exit = child.wait_exit(far()).expect("종료");
        assert_eq!(exit.code(), Some(3));
        assert!(!exit.looks_not_installed());
        assert_eq!(child.stderr_tail(), vec!["warn".to_owned()]);
        assert!(!spawner.child_dropped());

        drop(child);
        assert_eq!(spawner.commands(), vec![command()]);
        assert_eq!(spawner.written(), vec!["req-1".to_owned()]);
        assert!(!spawner.cut_at_deadline());
        assert!(spawner.child_dropped());
    }

    #[test]
    fn a_silent_child_is_cut_at_the_deadline_and_stays_cut() {
        let spawner = ScriptedSpawner::new(ChildScript::new().hang_after_lines());
        let mut child = spawner.spawn(&command(), far()).expect("기동");
        let started = Instant::now();
        assert_eq!(
            child.read_line(Instant::now() + SHORT),
            Err(ProbeError::Timeout)
        );
        assert!(started.elapsed() >= SHORT, "마감 전에 돌아왔다");
        assert!(spawner.cut_at_deadline());
        assert_eq!(child.write_line("x", far()), Err(ProbeError::Timeout));
        assert_eq!(child.wait_exit(far()), Err(ProbeError::Timeout));
    }

    #[test]
    fn blocked_writes_and_a_child_that_never_exits_hit_the_deadline() {
        let spawner = ScriptedSpawner::new(ChildScript::new().block_writes());
        let mut child = spawner.spawn(&command(), far()).expect("기동");
        assert_eq!(
            child.write_line("x", Instant::now() + SHORT),
            Err(ProbeError::Timeout)
        );
        assert!(spawner.written().is_empty());
        assert!(spawner.cut_at_deadline());

        let spawner = ScriptedSpawner::new(ChildScript::new().never_exit());
        let mut child = spawner.spawn(&command(), far()).expect("기동");
        assert_eq!(child.read_line(far()), Ok(None));
        assert_eq!(
            child.wait_exit(Instant::now() + SHORT),
            Err(ProbeError::Timeout)
        );
        assert!(spawner.cut_at_deadline());
    }

    #[test]
    fn a_passed_deadline_cuts_at_once() {
        let spawner = ScriptedSpawner::new(ChildScript::new().line("never read"));
        let mut child = spawner.spawn(&command(), far()).expect("기동");
        assert_eq!(child.read_line(Instant::now()), Err(ProbeError::Timeout));
        assert!(spawner.cut_at_deadline());
    }

    #[test]
    fn not_installed_and_spawn_failures_are_scriptable() {
        let spawner = ScriptedSpawner::new(ChildScript::new().exit_not_installed());
        let mut child = spawner.spawn(&command(), far()).expect("기동");
        assert!(child.wait_exit(far()).expect("종료").looks_not_installed());

        let spawner = ScriptedSpawner::failing(ProbeError::NotInstalled);
        assert!(matches!(
            spawner.spawn(&command(), far()),
            Err(ProbeError::NotInstalled)
        ));
        assert_eq!(
            spawner.commands().len(),
            1,
            "실패한 spawn 의 명령도 기록한다"
        );
    }

    #[test]
    fn a_spawn_past_the_deadline_times_out_without_using_the_script() {
        let spawner = ScriptedSpawner::new(ChildScript::new().line("hello"));
        assert!(matches!(
            spawner.spawn(&command(), Instant::now()),
            Err(ProbeError::Timeout)
        ));
        assert_eq!(spawner.commands().len(), 1);
        assert!(
            !spawner.cut_at_deadline(),
            "띄운 자식이 없으니 끊은 것도 없다"
        );
        let mut child = spawner.spawn(&command(), far()).expect("대본은 그대로다");
        assert_eq!(child.read_line(far()), Ok(Some("hello".into())));
    }

    #[test]
    fn closing_stdin_is_recorded_and_refuses_later_writes() {
        let spawner = ScriptedSpawner::new(ChildScript::new());
        let mut child = spawner.spawn(&command(), far()).expect("기동");
        child.write_line("before", far()).expect("쓰기");
        child.close_stdin();
        assert!(spawner.stdin_closed());
        assert!(matches!(
            child.write_line("after", far()),
            Err(ProbeError::Io(_))
        ));
        assert_eq!(spawner.written(), vec!["before".to_owned()]);
    }

    #[test]
    fn failing_writes_and_the_judging_wait_are_scriptable() {
        let spawner = ScriptedSpawner::new(ChildScript::new().fail_writes().exit_code(4));
        let mut child = spawner.spawn(&command(), far()).expect("기동");
        assert!(matches!(
            child.write_line("x", far()),
            Err(ProbeError::Io(_))
        ));
        assert!(spawner.written().is_empty());
        assert_eq!(child.wait_exit_code(far()), Ok(Some(4)));
        assert!(!spawner.exit_judged(), "판정 없는 대기는 세지 않는다");
        child.wait_exit(far()).expect("종료");
        assert!(spawner.exit_judged());
    }

    #[test]
    #[should_panic(expected = "너무 멀다")]
    fn a_far_deadline_on_a_hang_panics_instead_of_stalling_the_test() {
        let spawner = ScriptedSpawner::new(ChildScript::new().hang_after_lines());
        let mut child = spawner.spawn(&command(), far()).expect("기동");
        let _ = child.read_line(far());
    }
}
