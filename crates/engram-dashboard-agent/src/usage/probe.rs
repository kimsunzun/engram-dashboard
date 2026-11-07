//! The vendor-neutral seam of active query (probe) — the querier ([`UsageProbe`]) and the handle it uses to handle a child process
//! ([`ProbeSpawner`]·[`ProbeChild`]).
//!
//! ★Vendor knowledge (command · arguments · env · conversation shape · cooldown · deadline values) is not here★ — the `usage_probe.rs` of each vendor backend
//!   carries it by implementing [`UsageProbe`]. This module fixes only the shape 「짧게 뜨는 자식 하나와 줄 단위로 말하고, 마감 하나 안에
//!   끝낸다」 (talks line by line with one short-lived child and finishes within one deadline). The real spawner is [`super::OsProbeSpawner`], the test double is `usage::testing`.
// ADR-0004

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{UsageDetail, UsageObservation, UsageVendorKey};

/// The query policy of one vendor — the values are decided by each vendor backend (the receiving side has no vendor policy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsagePolicy {
    /// Minimum interval between automatic queries.
    pub cooldown: Duration,
    /// Deadline of one whole query (start · write · read · wait for exit). The receiving side makes [`ProbeEnv::deadline`] from this.
    pub timeout: Duration,
}

/// Classification of a query failure — the receiving side folds it into a visible state. The querier wraps this in [`ProbeFailure`] and returns it.
///
/// ★String fields go to logs — what may be put in = only sentences we wrote and OS error messages★: not credentials · tokens · env values,
///   and not text the upstream gave (error messages · account identifiers carried in responses · stderr) either. The upstream original text
///   lives only in the dedicated field [`UsageDetail::upstream`] — the trimming rules and destinations are in [`super::UpstreamText`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProbeError {
    /// The program to query is not on this PC.
    #[error("not installed")]
    NotInstalled,
    /// Not logged in, or authentication was refused.
    #[error("unauthenticated")]
    Unauthenticated,
    /// Upstream refused the query as over the limit. `retry_after` = the wait upstream told us (unknown if absent).
    #[error("rate limited")]
    RateLimited { retry_after: Option<Duration> },
    /// This installation · mode has no query feature.
    #[error("unsupported")]
    Unsupported,
    /// The deadline was exceeded — wherever it was exceeded, whether start · write · read · wait for exit, this is the one.
    #[error("timed out")]
    Timeout,
    /// Could not spawn the child (the case where the program is missing is [`ProbeError::NotInstalled`]).
    #[error("spawn failed: {0}")]
    Spawn(String),
    /// Pipe · file I/O failed.
    #[error("io error: {0}")]
    Io(String),
    /// The response shape is broken.
    #[error("parse error: {0}")]
    Parse(String),
    /// Upstream gave an error that fits none of the classifications above.
    #[error("upstream error: {0}")]
    Upstream(String),
}

impl ProbeError {
    /// Classification word — the share of [`UsageDetail::kind`] for neutral failures. ★Goes out over the wire and the screen shows it without translation★ — changing the spelling
    /// changes the visible word.
    pub const fn kind_word(&self) -> &'static str {
        match self {
            Self::NotInstalled => "not_installed",
            Self::Unauthenticated => "unauthenticated",
            Self::RateLimited { .. } => "rate_limited",
            Self::Unsupported => "unsupported",
            Self::Timeout => "timeout",
            Self::Spawn(_) => "spawn",
            Self::Io(_) => "io",
            Self::Parse(_) => "parse",
            Self::Upstream(_) => "upstream",
        }
    }
}

/// The failure of one query — the classification ([`ProbeError`]) and its grounds ([`UsageDetail`]).
///
/// - `detail: None` = the grounds are the classification alone — the receiving side fills it with [`ProbeError::kind_word`]. A failure that came up through `?` at the seam,
///   like the spawner · scratch folder, has this shape (`From<ProbeError>`).
/// - ★`Display` = the classification phrase + `detail.kind` only — the upstream original text is not printed★. `Debug` also prints the character count instead of the original text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeFailure {
    pub error: ProbeError,
    pub detail: Option<UsageDetail>,
}

impl ProbeFailure {
    pub fn with_detail(error: ProbeError, detail: UsageDetail) -> Self {
        Self {
            error,
            detail: Some(detail),
        }
    }
}

impl From<ProbeError> for ProbeFailure {
    fn from(error: ProbeError) -> Self {
        Self {
            error,
            detail: None,
        }
    }
}

impl fmt::Display for ProbeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail {
            Some(detail) => write!(f, "{} [{}]", self.error, detail.kind),
            None => write!(f, "{}", self.error),
        }
    }
}

/// The active querier of one vendor. The singleton lives in each vendor backend.
pub trait UsageProbe: Send + Sync {
    /// The vendor key (canonical spelling) of the entry this querier fills.
    fn key(&self) -> UsageVendorKey;

    fn policy(&self) -> UsagePolicy;

    /// One query. ★blocking★ — the caller calls it on a dedicated thread.
    ///
    /// - The result's `source` is [`super::UsageSource::Active`], and `vendor` is [`UsageProbe::key`].
    /// - ★Call [`ProbeEnv::require_time_left`] first, and pass `env.deadline` unchanged to startup and to every action of
    ///   the child★ — then this call returns within at most `deadline + KILL_WAIT` ([`super::KILL_WAIT`]) + the short
    ///   retry of deleting the scratch folder (the drop of [`super::ScratchDir`] — about 0.2 s) (the last share = the child drop's wait for exit and the
    ///   folder deletion after it). Do not make a longer deadline yourself.
    /// - Once the answer is received, end the child with [`finish_after_answer`].
    /// - Do not panic on external data — a query is outside the containment mark, so a panic means daemon exit (ADR-0292).
    fn query(&self, env: &ProbeEnv<'_>) -> Result<UsageObservation, ProbeFailure>;
}

/// What the receiving side passes for one query.
pub struct ProbeEnv<'a> {
    pub spawner: &'a dyn ProbeSpawner,
    /// The deadline of one whole query — the querier passes it unchanged to startup and to each action of the child.
    pub deadline: Instant,
    /// The parent folder in which a scratch folder is made per query ([`super::ScratchDir::create`]). A dedicated folder owned by the receiving side.
    pub scratch_root: &'a Path,
}

impl ProbeEnv<'_> {
    /// If the deadline has already passed, [`ProbeError::Timeout`] — call it **before** the querier makes the scratch folder.
    pub fn require_time_left(&self) -> Result<(), ProbeError> {
        if Instant::now() >= self.deadline {
            return Err(ProbeError::Timeout);
        }
        Ok(())
    }
}

/// Startup spec of one child.
///
/// Why it is kept apart from the agent's `CommandSpec` = that one only adds env. A query must strip the inherited env
/// ([`ProbeCommand::env_remove`]).
#[derive(Clone, PartialEq, Eq)]
pub struct ProbeCommand {
    pub program: String,
    pub args: Vec<String>,
    /// The child's working folder — must be an existing folder (otherwise [`ProbeError::Spawn`]).
    pub cwd: PathBuf,
    /// Env to add. ★Applied after [`ProbeCommand::env_remove`]★ — if both lists have the same key, it stays.
    pub env_set: Vec<(String, String)>,
    /// Keys to remove from the inherited env (exact names — not patterns). Case sensitivity follows the OS rules.
    pub env_remove: Vec<String>,
}

/// ★Does not print env values★ — only keys are shown. Values may carry credentials.
impl fmt::Debug for ProbeCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let set_keys: Vec<&str> = self.env_set.iter().map(|(k, _)| k.as_str()).collect();
        f.debug_struct("ProbeCommand")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("cwd", &self.cwd)
            .field("env_set_keys", &set_keys)
            .field("env_remove", &self.env_remove)
            .finish()
    }
}

/// The handle that spawns children — the real one = [`super::OsProbeSpawner`].
pub trait ProbeSpawner: Send + Sync {
    /// Spawns one child with three pipes open.
    ///
    /// - If the deadline has already passed, do not spawn and return [`ProbeError::Timeout`].
    /// - If startup itself fails as 「그런 프로그램 없음」 (no such program), [`ProbeError::NotInstalled`] (POSIX etc.). ★A command wrapped in a shell
    ///   (Windows `cmd.exe /c <prog>`) starts successfully★ — that the wrapped target is missing arrives as the exit status
    ///   ([`ExitInfo::looks_not_installed`]). We do not look for it before spawning.
    fn spawn(
        &self,
        cmd: &ProbeCommand,
        deadline: Instant,
    ) -> Result<Box<dyn ProbeChild>, ProbeError>;
}

/// One child that is up.
///
/// ★All three actions are under the same rule of deadline★: if not finished by the deadline (or if already past), the child is cut **tree and all**
///   and it is [`ProbeError::Timeout`]. Once cut by the deadline, every action is immediately `Timeout`.
/// ★drop = tree kill + bounded wait for exit ([`super::KILL_WAIT`])★ — once the querier has the answer it ends the child with [`finish_after_answer`]
///   (that end is the drop). The scratch folder must be declared **before** the child so the drop order (child → folder) becomes 「죽인 뒤
///   지운다」 (delete after killing).
pub trait ProbeChild: Send {
    /// Appends `\n` to the end of `line` and writes it to stdin. stdin stays open until [`ProbeChild::close_stdin`] or drop.
    fn write_line(&mut self, line: &str, deadline: Instant) -> Result<(), ProbeError>;

    /// Closes stdin (the child sees EOF). If already closed, does nothing. A later `write_line` is `Err(Io)`.
    fn close_stdin(&mut self);

    /// One line of stdout (with the trailing `\n` · `\r\n` stripped). `None` = stdout EOF (and `None` continues afterward).
    /// Bytes that are not UTF-8 are replaced by the replacement character. One line that hit the bound without a newline (a line over the bound · a piece that exactly
    /// fills the bound without a newline) is `Err(Io)` and the stream continues with the next line.
    fn read_line(&mut self, deadline: Instant) -> Result<Option<String>, ProbeError>;

    /// Waits for the child (the directly spawned process) to end. A child still alive after stdout closes is also cut at the deadline.
    /// ★Call it after seeing stdout EOF★ — if you wait with unread output, the child may be blocked on output and not end until the deadline.
    /// The 「프로그램 없음」 (program missing) decision ([`ExitInfo::looks_not_installed`]) also runs inside this under the same deadline — if the answer has already
    /// been received, use [`ProbeChild::wait_exit_code`], which makes no decision.
    fn wait_exit(&mut self, deadline: Instant) -> Result<ExitInfo, ProbeError>;

    /// Same as [`ProbeChild::wait_exit`] but makes no 「프로그램 없음」 (program missing) decision (`None` = ended without a code). The decision
    /// may run a file-system search (the spawner's share), so the wait for exit after the answer ([`finish_after_answer`]) uses this.
    fn wait_exit_code(&mut self, deadline: Instant) -> Result<Option<i32>, ProbeError>;

    /// The last few lines of stderr — token shapes masked and each line truncated. The spawner does not put this content in logs.
    ///
    /// ★Only token shapes are masked★ — things like account identifiers · paths may remain as is. Whether to log it is decided by the querier that
    ///   knows it, and it is not put into [`ProbeError`] strings. Right after the child just ended, the last line may
    ///   not have come in yet.
    fn stderr_tail(&self) -> Vec<String>;
}

/// The child's exit status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitInfo {
    code: Option<i32>,
    not_installed: bool,
}

impl ExitInfo {
    /// `not_installed` = whether the spawner decided this exit means 「띄우려던 프로그램이 없다」 (the program it tried to spawn is missing) (the OS rules are the spawner's share).
    pub const fn new(code: Option<i32>, not_installed: bool) -> Self {
        Self {
            code,
            not_installed,
        }
    }

    /// `None` = ended without a code (POSIX signal etc.).
    pub const fn code(&self) -> Option<i32> {
        self.code
    }

    /// Whether the wrapping shell ended with 「그런 명령이 없다」 (no such command) — after the querier sees stdout EOF with no response, if this is true it folds to
    /// [`ProbeError::NotInstalled`]. ★False does not mean it is installed★ — the spawner may not have finished the decision within the
    /// deadline, or an earlier decision on the same target may still be running so it did not decide anew (the decision rule = the spawner's
    /// share).
    pub const fn looks_not_installed(&self) -> bool {
        self.not_installed
    }
}

/// Ends the child after the answer is received — closes stdin, drains the remaining stdout and waits for it to end by itself up to [`FINISH_GRACE`] (not past the
/// deadline), then drops it (tree kill). Even if the wait hits the deadline it is not a query failure — the answer has already been received.
///
/// Why not kill right away = to let the CLI finish its own file writes — killing right after the answer was measured to leave
/// locks · temp files in the config folder (vendor and measurement record are in the comment of the calling querier).
pub fn finish_after_answer(mut child: Box<dyn ProbeChild>, deadline: Instant) {
    let grace = deadline.min(Instant::now() + FINISH_GRACE);
    child.close_stdin();
    loop {
        match child.read_line(grace) {
            // A line that hit the bound is also drained — the stream continues with the next line.
            Ok(Some(_)) | Err(ProbeError::Io(_)) => {}
            Ok(None) => break,
            Err(_) => {
                tracing::debug!(
                    "사용량 조회: 답을 받은 뒤 자식이 유예 안에 stdout 을 닫지 않아 끊는다"
                );
                return;
            }
        }
    }
    if child.wait_exit_code(grace).is_err() {
        tracing::debug!("사용량 조회: 답을 받은 뒤 자식이 유예 안에 끝나지 않아 끊는다");
    }
}

/// The grace bound of [`finish_after_answer`] — a value covering the time the query CLI takes to end by itself after stdin EOF (measured max about 1.1 s,
/// during which it writes its own config file). If exceeded, the whole tree is cut. Vendor and measurement record are in the comment of the calling querier.
pub(crate) const FINISH_GRACE: Duration = Duration::from_secs(2);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::testing::{ChildScript, ScriptedSpawner};

    fn command() -> ProbeCommand {
        ProbeCommand {
            program: "prog".into(),
            args: Vec::new(),
            cwd: PathBuf::from("."),
            env_set: Vec::new(),
            env_remove: Vec::new(),
        }
    }

    fn far() -> Instant {
        Instant::now() + Duration::from_secs(4)
    }

    #[test]
    fn finish_closes_stdin_drains_stdout_and_waits_for_the_exit() {
        let spawner = ScriptedSpawner::new(ChildScript::new().line("late line").exit_code(0));
        let child = spawner.spawn(&command(), far()).expect("기동");
        let started = Instant::now();
        finish_after_answer(child, far());
        assert!(
            started.elapsed() < FINISH_GRACE,
            "스스로 끝난 자식을 기다렸다"
        );
        assert!(spawner.stdin_closed());
        assert!(!spawner.cut_at_deadline());
        assert!(spawner.child_dropped());
    }

    /// ★After the answer is received, do not call the 「프로그램 없음」 (program missing) decision★ — even for a non-zero exit (the decision is where a search would run).
    #[test]
    fn finish_never_asks_for_the_not_installed_judgment() {
        let spawner = ScriptedSpawner::new(ChildScript::new().exit_code(1));
        let child = spawner.spawn(&command(), far()).expect("기동");
        finish_after_answer(child, far());
        assert!(!spawner.exit_judged());
        assert!(!spawner.cut_at_deadline());
        assert!(spawner.child_dropped());
    }

    /// A child that does not end even after stdin is closed is cut after the grace period — it is not dragged to the deadline.
    #[test]
    fn finish_cuts_a_child_that_ignores_stdin_close_after_the_grace() {
        for script in [
            ChildScript::new().hang_after_lines(),
            ChildScript::new().never_exit(),
        ] {
            let spawner = ScriptedSpawner::new(script);
            let child = spawner.spawn(&command(), far()).expect("기동");
            let started = Instant::now();
            finish_after_answer(child, far());
            let took = started.elapsed();
            assert!(took >= FINISH_GRACE, "유예 전에 끊었다: {took:?}");
            assert!(took < FINISH_GRACE * 4, "유예를 넘겨 기다렸다: {took:?}");
            assert!(spawner.stdin_closed());
            assert!(spawner.cut_at_deadline());
            assert!(spawner.child_dropped());
        }
    }

    #[test]
    fn the_grace_never_outlasts_the_deadline() {
        let spawner = ScriptedSpawner::new(ChildScript::new().hang_after_lines());
        let child = spawner.spawn(&command(), far()).expect("기동");
        let started = Instant::now();
        finish_after_answer(child, Instant::now() + Duration::from_millis(100));
        assert!(started.elapsed() < FINISH_GRACE, "마감을 넘겨 기다렸다");
        assert!(spawner.cut_at_deadline());
    }

    #[test]
    fn require_time_left_refuses_a_passed_deadline() {
        let spawner = ScriptedSpawner::new(ChildScript::new());
        let env = |deadline| ProbeEnv {
            spawner: &spawner,
            deadline,
            scratch_root: Path::new("."),
        };
        assert_eq!(
            env(Instant::now()).require_time_left(),
            Err(ProbeError::Timeout)
        );
        assert_eq!(env(far()).require_time_left(), Ok(()));
    }

    /// The classification word goes out over the wire — if changed, the word on screen changes.
    #[test]
    fn kind_words_are_the_wire_words() {
        let cases = [
            (ProbeError::NotInstalled, "not_installed"),
            (ProbeError::Unauthenticated, "unauthenticated"),
            (
                ProbeError::RateLimited {
                    retry_after: Some(Duration::from_secs(1)),
                },
                "rate_limited",
            ),
            (ProbeError::Unsupported, "unsupported"),
            (ProbeError::Timeout, "timeout"),
            (ProbeError::Spawn("x".into()), "spawn"),
            (ProbeError::Io("x".into()), "io"),
            (ProbeError::Parse("x".into()), "parse"),
            (ProbeError::Upstream("x".into()), "upstream"),
        ];
        for (error, word) in cases {
            assert_eq!(error.kind_word(), word, "{error:?}");
        }
    }

    #[test]
    fn a_bare_probe_error_becomes_a_failure_without_detail() {
        let failure = ProbeFailure::from(ProbeError::Timeout);
        assert_eq!(failure.error, ProbeError::Timeout);
        assert_eq!(failure.detail, None);
        assert_eq!(failure.to_string(), "timed out");
    }

    #[test]
    fn display_and_debug_carry_the_kind_but_never_the_upstream_text() {
        let failure = ProbeFailure::with_detail(
            ProbeError::Upstream("우리 문장".into()),
            UsageDetail {
                kind: "vendor_class",
                code: Some(-32600),
                upstream: Some(crate::usage::UpstreamText::new(
                    "someone@example.com org-1234",
                )),
            },
        );
        let shown = failure.to_string();
        assert_eq!(shown, "upstream error: 우리 문장 [vendor_class]");
        let all = format!("{failure} {failure:?} {failure:#?}");
        assert!(
            !all.contains("example.com") && !all.contains("org-1234"),
            "{all}"
        );
    }

    #[test]
    fn debug_shows_env_keys_but_never_values() {
        let cmd = ProbeCommand {
            program: "prog".into(),
            args: vec!["--flag".into()],
            cwd: PathBuf::from("."),
            env_set: vec![("SOME_TOKEN".into(), "value-that-must-not-leak".into())],
            env_remove: vec!["GONE".into()],
        };
        let shown = format!("{cmd:?}");
        assert!(shown.contains("SOME_TOKEN"), "{shown}");
        assert!(!shown.contains("value-that-must-not-leak"), "{shown}");
    }
}
