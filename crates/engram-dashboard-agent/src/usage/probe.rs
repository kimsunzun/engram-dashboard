//! The vendor-neutral seam of active query (probe) — the querier ([`UsageProbe`]) and the child handle ([`ProbeSpawner`]·[`ProbeChild`]).
//!
//! ★Vendor knowledge (command · args · env · conversation shape · cooldown · deadline) is not here★ — each vendor's `usage_probe.rs`
//!   implements [`UsageProbe`]. This module fixes only 「line-by-line talk with one short-lived child within one deadline」.
//!   Real spawner: [`super::OsProbeSpawner`]; test double: `usage::testing`.
// ADR-0004

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{UsageDetail, UsageObservation, UsageVendorKey};

/// The query policy of one vendor — the values are decided by each vendor backend (the receiving side has no vendor policy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsagePolicy {
    pub cooldown: Duration,
    /// Deadline of one whole query (start · write · read · wait for exit). The receiving side makes [`ProbeEnv::deadline`] from this.
    pub timeout: Duration,
}

/// Classification of a query failure — the receiving side folds it into a visible state; the querier wraps it in [`ProbeFailure`].
///
/// ★String fields go to logs — only sentences we wrote and OS error messages★: no credentials · tokens · env values, and no upstream
///   text (error messages · account identifiers · stderr). Upstream text lives only in [`UsageDetail::upstream`] ([`super::UpstreamText`]).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProbeError {
    #[error("not installed")]
    NotInstalled,
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
    #[error("io error: {0}")]
    Io(String),
    #[error("parse error: {0}")]
    Parse(String),
    /// Upstream gave an error that fits none of the classifications above.
    #[error("upstream error: {0}")]
    Upstream(String),
}

impl ProbeError {
    /// Classification word — [`UsageDetail::kind`] for neutral failures. ★Goes over the wire, shown untranslated★ — a respelling changes
    ///   the visible word.
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
/// - `detail: None` = classification alone — the receiving side fills [`ProbeError::kind_word`] (a `?` failure at the seam, via
///   `From<ProbeError>`).
/// - ★`Display` = classification phrase + `detail.kind` only — upstream text is not printed★; `Debug` prints its character count.
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
    /// - ★Call [`ProbeEnv::require_time_left`] first and pass `env.deadline` unchanged to startup and every child action★ — then this returns
    ///   within `deadline + KILL_WAIT` ([`super::KILL_WAIT`]) + the scratch-folder delete retry (~0.2 s). Do not make a longer deadline.
    /// - Once the answer is received, end the child with [`finish_after_answer`].
    /// - Do not panic on external data — a query is outside the containment mark, so a panic means daemon exit (ADR-0292).
    fn query(&self, env: &ProbeEnv<'_>) -> Result<UsageObservation, ProbeFailure>;
}

pub struct ProbeEnv<'a> {
    pub spawner: &'a dyn ProbeSpawner,
    /// The deadline of one whole query — the querier passes it unchanged to startup and to each action of the child.
    pub deadline: Instant,
    /// The parent folder in which a scratch folder is made per query ([`super::ScratchDir::create`]). A dedicated folder owned by the
    ///   receiving side.
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

/// Startup spec of one child. Kept apart from `CommandSpec` because a query must strip the inherited env ([`ProbeCommand::env_remove`]).
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
    /// - Deadline already passed → do not spawn, [`ProbeError::Timeout`].
    /// - Startup failing as 「no such program」 → [`ProbeError::NotInstalled`] (POSIX etc.). ★A shell-wrapped command (Windows `cmd.exe /c
    ///   <prog>`) starts successfully★ — a missing target arrives as the exit status ([`ExitInfo::looks_not_installed`]); no lookup before
    ///   spawning.
    fn spawn(
        &self,
        cmd: &ProbeCommand,
        deadline: Instant,
    ) -> Result<Box<dyn ProbeChild>, ProbeError>;
}

/// One child that is up.
///
/// ★All three actions share the deadline★: past it, the child is cut **tree and all** and it is [`ProbeError::Timeout`]; every later
///   action is `Timeout`.
/// ★drop = tree kill + bounded wait ([`super::KILL_WAIT`])★ — the querier ends the child with [`finish_after_answer`] (that end is the drop).
///   Declare the scratch folder **before** the child so the drop order (child → folder) is 「delete after killing」.
pub trait ProbeChild: Send {
    /// Appends `\n` to the end of `line` and writes it to stdin. stdin stays open until [`ProbeChild::close_stdin`] or drop.
    fn write_line(&mut self, line: &str, deadline: Instant) -> Result<(), ProbeError>;

    /// Closes stdin (the child sees EOF). If already closed, does nothing. A later `write_line` is `Err(Io)`.
    fn close_stdin(&mut self);

    /// One line of stdout (trailing `\n` · `\r\n` stripped). `None` = stdout EOF (stays `None`). Non-UTF-8 bytes become the replacement
    /// character. A line that hit the bound without a newline is `Err(Io)`; the stream continues with the next line.
    fn read_line(&mut self, deadline: Instant) -> Result<Option<String>, ProbeError>;

    /// Waits for the directly spawned process to end; a child still alive after stdout closes is cut at the deadline.
    /// ★Call it after seeing stdout EOF★ — with unread output the child may block on output until the deadline.
    /// Also runs the 「program missing」 decision ([`ExitInfo::looks_not_installed`]) under the same deadline — after the answer use
    /// [`ProbeChild::wait_exit_code`], which makes no decision.
    fn wait_exit(&mut self, deadline: Instant) -> Result<ExitInfo, ProbeError>;

    /// Same as [`ProbeChild::wait_exit`] without the 「program missing」 decision (`None` = ended without a code); the decision may run a
    /// file-system search, so the wait after the answer ([`finish_after_answer`]) uses this.
    fn wait_exit_code(&mut self, deadline: Instant) -> Result<Option<i32>, ProbeError>;

    /// The last few stderr lines — token shapes masked, each line truncated. The spawner does not log this.
    ///
    /// ★Only token shapes are masked★ — account identifiers · paths may remain. Whether to log it is the querier's call; it never goes into
    ///   [`ProbeError`] strings. Right after the child ended, the last line may not have arrived yet.
    fn stderr_tail(&self) -> Vec<String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitInfo {
    code: Option<i32>,
    not_installed: bool,
}

impl ExitInfo {
    /// `not_installed` = whether the spawner decided this exit means 「the program it tried to spawn is missing」 (the OS rules are the
    ///   spawner's share).
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

    /// Whether the wrapping shell ended with 「no such command」 — true after stdout EOF with no response → [`ProbeError::NotInstalled`].
    /// ★False does not mean installed★ — the decision may not have finished within the deadline, or an earlier one on the same target may
    ///   still run.
    pub const fn looks_not_installed(&self) -> bool {
        self.not_installed
    }
}

/// Ends the child after the answer — closes stdin, drains stdout, waits up to [`FINISH_GRACE`] (not past the deadline) for it to end by itself,
/// then drops it (tree kill). Hitting the deadline here is not a query failure — the answer is already received.
///
/// Why not kill right away: killing right after the answer was measured to leave locks · temp files in the CLI's config folder
/// (vendor and record: the calling querier's comment).
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
                    "usage query: after receiving the answer the child did not close stdout within the grace period, cutting it off"
                );
                return;
            }
        }
    }
    if child.wait_exit_code(grace).is_err() {
        tracing::debug!("usage query: after receiving the answer the child did not exit within the grace period, cutting it off");
    }
}

/// The grace bound of [`finish_after_answer`] — covers the CLI's self-exit after stdin EOF (measured max ~1.1 s, while it writes its
/// config file). If exceeded, the whole tree is cut. Vendor and record: the calling querier's comment.
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
        let child = spawner.spawn(&command(), far()).expect("start");
        let started = Instant::now();
        finish_after_answer(child, far());
        assert!(
            started.elapsed() < FINISH_GRACE,
            "waited for a child that exited on its own"
        );
        assert!(spawner.stdin_closed());
        assert!(!spawner.cut_at_deadline());
        assert!(spawner.child_dropped());
    }

    /// ★After the answer is received, do not call the 「program missing」 decision★ — even for a non-zero exit (the decision is where a
    ///   search would run).
    #[test]
    fn finish_never_asks_for_the_not_installed_judgment() {
        let spawner = ScriptedSpawner::new(ChildScript::new().exit_code(1));
        let child = spawner.spawn(&command(), far()).expect("start");
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
            let child = spawner.spawn(&command(), far()).expect("start");
            let started = Instant::now();
            finish_after_answer(child, far());
            let took = started.elapsed();
            assert!(
                took >= FINISH_GRACE,
                "cut off before the grace period: {took:?}"
            );
            assert!(
                took < FINISH_GRACE * 4,
                "waited past the grace period: {took:?}"
            );
            assert!(spawner.stdin_closed());
            assert!(spawner.cut_at_deadline());
            assert!(spawner.child_dropped());
        }
    }

    #[test]
    fn the_grace_never_outlasts_the_deadline() {
        let spawner = ScriptedSpawner::new(ChildScript::new().hang_after_lines());
        let child = spawner.spawn(&command(), far()).expect("start");
        let started = Instant::now();
        finish_after_answer(child, Instant::now() + Duration::from_millis(100));
        assert!(started.elapsed() < FINISH_GRACE, "waited past the deadline");
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
            ProbeError::Upstream("our sentence".into()),
            UsageDetail {
                kind: "vendor_class",
                code: Some(-32600),
                upstream: Some(crate::usage::UpstreamText::new(
                    "someone@example.com org-1234",
                )),
            },
        );
        let shown = failure.to_string();
        assert_eq!(shown, "upstream error: our sentence [vendor_class]");
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
