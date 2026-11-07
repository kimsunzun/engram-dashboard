//! Real-CLI smoke of the usage querier — sends a query to the installed, logged-in claude · codex through **the production path as is**: registry
//! (`usage_probe_for`) → [`UsageProbe::query`] → real [`OsProbeSpawner`] (Windows = `cmd.exe /c <cli> …`), deadline =
//! the deadline of the querier policy (the same as what the receiving side makes).
//!
//! ★All are `#[ignore]`★ — they use a logged-in CLI and the account's query limit (a CI runner has neither). Locally only:
//!   `cargo test -p engram-dashboard-agent --test usage_probe_smoke -- --ignored --nocapture --test-threads=1`
//!   `--test-threads=1` = do not send queries overlapping (so the query limit is not used all at once).
//! ★What is printed is only a masked summary★ — window numbers · reset times · plan · model names · the shape of lines (keys and types). env values · tokens · string
//!   values are not printed. Exception = the upstream phrase of the logout test (an empty config folder has no account information — token shapes · paths are masked).
//! ★Do not gate the file by OS★ — the OS branching of the query command is inside the querier. Only the pickup-shape collection (`…_passive_parser`)
//!   launches the CLI outside the querier, so it calls the same wrapping as production ([`console_command`]) as is.
// ADR-0230

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use uuid::Uuid;

use engram_dashboard_agent::backend::{output_decoder, usage_probe_for};
use engram_dashboard_agent::profile::{AgentCommand, AgentOutputFormat};
use engram_dashboard_agent::usage::{
    ExitInfo, OsProbeSpawner, ProbeChild, ProbeCommand, ProbeEnv, ProbeError, ProbeFailure,
    ProbeSpawner, ScratchDir, UsageObservation, UsageProbe, UsageSource, WindowObs, KILL_WAIT,
};
use engram_dashboard_platform::shell::console_command;

/// The bound within which one query must return = deadline + the child drop's wait for exit ([`UsageProbe::query`] contract) + leeway.
const RETURN_SLACK: Duration = Duration::from_secs(3);

/// The deadline of one model turn for the pickup-shape collection.
const TURN_TIMEOUT: Duration = Duration::from_secs(120);
/// Time to wait further for a notification that arrives late after the turn ends.
const TURN_TAIL: Duration = Duration::from_millis(1500);
const TURN_PROMPT: &str = "Reply with just OK.";

// ── Production-path query ─────────────────────────────────────────────────────

#[test]
#[ignore = "실 claude CLI·로그인·계정의 조회 제한을 쓴다"]
fn claude_get_usage_answers_through_the_operating_command() {
    let root = TempTree::fresh("claude");
    let run = run_probe("claude", root.path(), Tweaks::default());
    run.report("claude get_usage (temp scratch root)", &root);
    let obs = run
        .result
        .expect("claude 조회가 운영 명령 그대로 성공해야 한다");
    assert_eq!(obs.source, UsageSource::Active);
    assert_eq!(obs.vendor.as_str(), "claude");
}

#[test]
#[ignore = "실 codex CLI·로그인·계정의 조회 제한을 쓴다"]
fn codex_rate_limits_read_answers_through_the_operating_command() {
    let root = TempTree::fresh("codex");
    let run = run_probe("codex", root.path(), Tweaks::default());
    run.report("codex account/rateLimits/read", &root);
    let obs = run
        .result
        .expect("codex 조회가 운영 명령 그대로 성공해야 한다");
    assert_eq!(obs.source, UsageSource::Active);
    assert_eq!(obs.vendor.as_str(), "codex");
}

/// An empty config folder = no login. We do not assert the classification — the aim is collecting the response shape (TRD §6 #4). The contracts measured are
/// two: it returns within the deadline · if it returns as success it is 「한도 정보 없음」 (no limit information).
/// ★Measured (claude 2.1.280 · 2026-09-27) = it is a success response, not an error★ — `rate_limits: null` ·
/// `subscription_type: null` · `rate_limits_available: false` (`true` on the logged-in side). There was no upstream error phrase.
#[test]
#[ignore = "실 claude CLI 를 쓴다"]
fn claude_logged_out_answer_shape() {
    let root = TempTree::fresh("claude-logged-out");
    let config = TempTree::fresh("claude-empty-config");
    let config_dir = config.path().to_str().expect("유니코드 경로").to_owned();
    // Also strip the login keys and API-key paths that the daemon env leaves — if they remain, it queries still logged in even with an empty config folder.
    let login_env = [
        "CLAUDE_CODE_OAUTH_TOKEN",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
    ];
    let present: Vec<&str> = login_env
        .iter()
        .copied()
        .filter(|key| std::env::var_os(key).is_some())
        .collect();
    eprintln!(
        "[logged-out] login env keys present in the test process (values not shown): {present:?}"
    );
    let tweaks = Tweaks {
        env_set: vec![("CLAUDE_CONFIG_DIR".to_owned(), config_dir)],
        env_remove: login_env.iter().map(|k| (*k).to_owned()).collect(),
    };
    let timeout = policy_timeout("claude");
    let run = run_probe("claude", root.path(), tweaks);
    run.report("claude get_usage (empty CLAUDE_CONFIG_DIR)", &root);
    run.report_upstream_text();
    let entries = config.entries();
    eprintln!("[logged-out] config dir entries after the run: {entries:?}");
    // Killing right after the answer left lock · temp files of the config file the CLI was writing — checks whether the query gives the child a grace period to finish.
    let leftovers: Vec<&String> = entries
        .iter()
        .filter(|name| {
            name.as_str() == ".claude.json.lock" || name.starts_with(".claude.json.tmp.")
        })
        .collect();
    eprintln!("[logged-out] lock/tmp leftovers in the config dir: {leftovers:?}");
    assert!(
        run.elapsed <= timeout + KILL_WAIT + RETURN_SLACK,
        "마감 안에 돌아와야 한다: {:?}",
        run.elapsed
    );
    if let Ok(obs) = &run.result {
        assert!(
            obs.limits_unavailable.is_some(),
            "로그아웃 성공 응답은 「한도 정보 없음」이어야 한다"
        );
    }
}

/// TRD §6 #18 — the development build's data folder is inside the repository, so the CLI can climb up from its working folder and read the repository's
/// `.claude/settings.json`·CLAUDE.md. We measure that the query still stands.
#[test]
#[ignore = "실 claude CLI·로그인·계정의 조회 제한을 쓴다"]
fn claude_get_usage_answers_with_the_scratch_root_inside_the_repo() {
    let root = TempTree::at(repo_root().join(".engram-dev").join("usage-probe-smoke"));
    let run = run_probe("claude", root.path(), Tweaks::default());
    run.report("claude get_usage (scratch root inside the repo)", &root);
    run.result
        .expect("저장소 안 작업 폴더에서도 claude 조회가 성공해야 한다");
}

/// Measuring the grace after the answer (`FINISH_GRACE`) — runs the querier's conversation with the production command as is, but only the wait after
/// receiving the answer and closing stdin is extended to [`EXIT_WINDOW`], to measure how long claude takes to end **by itself** (if it does not end within the window, it is cut then). In the meantime
/// the global config file (`.claude.json`) is looked at only for modification time and the presence of a lock file — the content is not read.
#[test]
#[ignore = "실 claude CLI·로그인·계정의 조회 제한을 쓴다"]
fn claude_exit_after_stdin_close_measurement() {
    let root = TempTree::fresh("claude-exit");
    let (config, config_source) = claude_global_config();
    let config = config.as_deref();
    let before = ConfigMark::of(config);
    let spawner = PatientSpawner {
        config: config.map(Path::to_path_buf),
        log: Arc::default(),
    };
    let probe = probe("claude");
    let started = Instant::now();
    let env = ProbeEnv {
        spawner: &spawner,
        deadline: started + probe.policy().timeout,
        scratch_root: root.path(),
    };
    let result = probe.query(&env);
    let returned = started.elapsed();
    let after = ConfigMark::of(config);
    let log = std::mem::take(&mut *spawner.log.lock().expect("기록 락"));

    eprintln!("== claude exit after stdin close (measurement)");
    eprintln!(
        "   result = {} · query returned in {} ms",
        result.as_ref().map_or_else(error_kind, |_| "OK".to_owned()),
        returned.as_millis()
    );
    let since_close = |at: Option<Instant>| match (log.closed_at, at) {
        (Some(closed), Some(at)) => {
            format!("{} ms", at.saturating_duration_since(closed).as_millis())
        }
        (Some(_), None) => format!("not within {} ms", EXIT_WINDOW.as_millis()),
        (None, _) => "stdin never closed".to_owned(),
    };
    eprintln!("   close → stdout EOF = {}", since_close(log.eof_at));
    eprintln!(
        "   close → exit = {} · exit code = {:?}",
        since_close(log.exited_at),
        log.exit_code
    );
    eprintln!(
        "   config source = {config_source} · file found = {}",
        config.is_some()
    );
    let at_close = log.at_close.unwrap_or_default();
    let at_exit = log.at_exit.unwrap_or_default();
    eprintln!(
        "   .claude.json modified: spawn→close = {} · close→exit = {} · exit→return = {}",
        before.modified != at_close.modified,
        at_close.modified != at_exit.modified,
        at_exit.modified != after.modified,
    );
    eprintln!(
        "   .claude.json.lock present: before = {} · at close = {} · at exit = {} · after return = {}",
        before.lock_present, at_close.lock_present, at_exit.lock_present, after.lock_present
    );
    eprintln!("   leftover scratch = {:?}", root.entries());
}

/// Upper bound on waiting for claude to end by itself after stdin is closed (far longer than the production grace period).
const EXIT_WINDOW: Duration = Duration::from_secs(10);

/// claude's global config file — inside `CLAUDE_CONFIG_DIR` if set, otherwise `.claude.json` in the home folder (the querier leaves that
/// env). ★Unverified★: the CLI's path rule was not checked against the source — if the file does not exist the time cannot be measured.
fn claude_global_config() -> (Option<PathBuf>, &'static str) {
    let (dir, source) = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) => (Some(PathBuf::from(dir)), "CLAUDE_CONFIG_DIR"),
        None => (
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(PathBuf::from),
            "home",
        ),
    };
    let path = dir.map(|dir| dir.join(".claude.json"));
    (path.filter(|p| p.is_file()), source)
}

/// One point in time of the config file — modification time and the presence of a neighboring lock file (the content is not read).
#[derive(Default, Clone, Copy)]
struct ConfigMark {
    modified: Option<SystemTime>,
    lock_present: bool,
}

impl ConfigMark {
    fn of(config: Option<&Path>) -> Self {
        let Some(config) = config else {
            return Self::default();
        };
        let mut lock = config.as_os_str().to_owned();
        lock.push(".lock");
        Self {
            modified: fs::metadata(config).and_then(|m| m.modified()).ok(),
            lock_present: Path::new(&lock).exists(),
        }
    }
}

#[derive(Default)]
struct PatientLog {
    closed_at: Option<Instant>,
    eof_at: Option<Instant>,
    exited_at: Option<Instant>,
    exit_code: Option<Option<i32>>,
    at_close: Option<ConfigMark>,
    at_exit: Option<ConfigMark>,
}

/// Wraps the real spawner and extends only the deadline after stdin is closed to [`EXIT_WINDOW`] — everything else is the querier's conversation as is.
struct PatientSpawner {
    config: Option<PathBuf>,
    log: Arc<Mutex<PatientLog>>,
}

impl ProbeSpawner for PatientSpawner {
    fn spawn(
        &self,
        cmd: &ProbeCommand,
        deadline: Instant,
    ) -> Result<Box<dyn ProbeChild>, ProbeError> {
        Ok(Box::new(PatientChild {
            inner: OsProbeSpawner.spawn(cmd, deadline)?,
            config: self.config.clone(),
            log: Arc::clone(&self.log),
        }))
    }
}

struct PatientChild {
    inner: Box<dyn ProbeChild>,
    config: Option<PathBuf>,
    log: Arc<Mutex<PatientLog>>,
}

impl PatientChild {
    fn patient(&self, deadline: Instant) -> Instant {
        self.log
            .lock()
            .expect("기록 락")
            .closed_at
            .map_or(deadline, |closed| closed + EXIT_WINDOW)
    }

    fn exited(&self, code: Option<i32>) {
        let mark = ConfigMark::of(self.config.as_deref());
        let mut log = self.log.lock().expect("기록 락");
        log.exited_at.get_or_insert_with(Instant::now);
        log.exit_code = Some(code);
        log.at_exit = Some(mark);
    }
}

impl ProbeChild for PatientChild {
    fn write_line(&mut self, line: &str, deadline: Instant) -> Result<(), ProbeError> {
        self.inner.write_line(line, deadline)
    }

    fn close_stdin(&mut self) {
        let mark = ConfigMark::of(self.config.as_deref());
        {
            let mut log = self.log.lock().expect("기록 락");
            log.closed_at = Some(Instant::now());
            log.at_close = Some(mark);
        }
        self.inner.close_stdin();
    }

    fn read_line(&mut self, deadline: Instant) -> Result<Option<String>, ProbeError> {
        let got = self.inner.read_line(self.patient(deadline));
        if let Ok(None) = got {
            let mut log = self.log.lock().expect("기록 락");
            if log.closed_at.is_some() {
                log.eof_at.get_or_insert_with(Instant::now);
            }
        }
        got
    }

    fn wait_exit(&mut self, deadline: Instant) -> Result<ExitInfo, ProbeError> {
        let got = self.inner.wait_exit(self.patient(deadline));
        if let Ok(exit) = &got {
            self.exited(exit.code());
        }
        got
    }

    fn wait_exit_code(&mut self, deadline: Instant) -> Result<Option<i32>, ProbeError> {
        let got = self.inner.wait_exit_code(self.patient(deadline));
        if let Ok(code) = &got {
            self.exited(*code);
        }
        got
    }

    fn stderr_tail(&self) -> Vec<String> {
        self.inner.stderr_tail()
    }
}

// ── Pickup-shape collection — feeds the raw lines of a short real turn to the pickup interpreter ─────────

#[test]
#[ignore = "실 claude 모델 턴 하나를 쓴다"]
fn claude_stream_turn_rate_limit_event_through_the_passive_parser() {
    let root = TempTree::fresh("claude-turn");
    let scratch = ScratchDir::create(root.path()).expect("임시 폴더");
    let started = Instant::now();
    let lines = claude_turn_lines(scratch.path(), started + TURN_TIMEOUT).expect("claude 턴");
    let elapsed = started.elapsed();
    drop(scratch);

    let events: Vec<&String> = lines
        .iter()
        .filter(|line| kind_of(line) == "rate_limit_event")
        .collect();
    eprintln!(
        "[claude turn] {} ms · {} stdout lines · kinds = {:?}",
        elapsed.as_millis(),
        lines.len(),
        line_kinds(&lines)
    );
    eprintln!("[claude turn] rate_limit_event lines: {}", events.len());
    for line in &events {
        eprintln!("[claude turn]   shape = {}", shape_of_line(line));
        eprintln!("[claude turn]   values = {}", rate_limit_info_values(line));
    }

    let mut decoder = output_decoder(&AgentCommand::Claude {
        extra_args: vec![],
        output_format: AgentOutputFormat::StreamJson,
    })
    .expect("stream-json 디코더");
    for line in &lines {
        decoder.decode(format!("{line}\n").as_bytes());
    }
    decoder.flush();
    let observed = decoder.take_usage();
    eprintln!("[claude turn] passive observations: {}", observed.len());
    for obs in &observed {
        eprintln!("{}", summarize(obs));
    }
    eprintln!("[claude turn] leftover scratch: {:?}", root.entries());
}

#[test]
#[ignore = "실 codex 모델 턴 하나를 쓴다"]
fn codex_turn_rate_limits_updated_through_the_passive_parser() {
    let root = TempTree::fresh("codex-turn");
    let scratch = ScratchDir::create(root.path()).expect("임시 폴더");
    let started = Instant::now();
    let lines = codex_turn_lines(scratch.path(), started + TURN_TIMEOUT).expect("codex 턴");
    let elapsed = started.elapsed();
    drop(scratch);

    let updates: Vec<&String> = lines
        .iter()
        .filter(|line| kind_of(line) == "account/rateLimits/updated")
        .collect();
    eprintln!(
        "[codex turn] {} ms · {} stdout lines · kinds = {:?}",
        elapsed.as_millis(),
        lines.len(),
        line_kinds(&lines)
    );
    eprintln!(
        "[codex turn] account/rateLimits/updated lines: {}",
        updates.len()
    );
    for line in &updates {
        eprintln!("[codex turn]   shape = {}", shape_of_line(line));
        eprintln!("[codex turn]   limitId = {}", codex_limit_id(line));
    }

    let mut decoder = output_decoder(&AgentCommand::Codex {
        extra_args: vec![],
        output_format: AgentOutputFormat::StreamJson,
    })
    .expect("app-server 디코더");
    // In the production path the wait ticket takes the response to our request and passes the remaining lines to the translator — response lines are not fed.
    for line in lines.iter().filter(|line| !is_rpc_response(line)) {
        decoder.decode(format!("{line}\n").as_bytes());
    }
    decoder.flush();
    let observed = decoder.take_usage();
    eprintln!("[codex turn] passive observations: {}", observed.len());
    for obs in &observed {
        eprintln!("{}", summarize(obs));
    }
    eprintln!("[codex turn] leftover scratch: {:?}", root.entries());
}

// ── Query driving ─────────────────────────────────────────────────────────────

/// Env adjustments added to the query command — layered on the command the querier made (`env_set` is applied later).
#[derive(Default)]
struct Tweaks {
    env_set: Vec<(String, String)>,
    env_remove: Vec<String>,
}

struct ProbeRun {
    result: Result<UsageObservation, ProbeFailure>,
    elapsed: Duration,
    log: ChildLog,
}

fn probe(word: &str) -> &'static dyn UsageProbe {
    usage_probe_for(word).unwrap_or_else(|| panic!("{word} 조회기가 등록부에 없다"))
}

fn policy_timeout(word: &str) -> Duration {
    probe(word).policy().timeout
}

fn run_probe(word: &str, scratch_root: &Path, tweaks: Tweaks) -> ProbeRun {
    let probe = probe(word);
    let spawner = Recorder::new(tweaks);
    let started = Instant::now();
    let env = ProbeEnv {
        spawner: &spawner,
        deadline: started + probe.policy().timeout,
        scratch_root,
    };
    let result = probe.query(&env);
    let elapsed = started.elapsed();
    let log = spawner.take_log();
    ProbeRun {
        result,
        elapsed,
        log,
    }
}

impl ProbeRun {
    fn report(&self, title: &str, root: &TempTree) {
        eprintln!("== {title}");
        eprintln!(
            "   command = {}",
            self.log
                .command
                .as_deref()
                .map_or_else(|| "(not spawned)".to_owned(), redact)
        );
        match &self.result {
            Ok(obs) => eprintln!(
                "   OK in {} ms\n{}",
                self.elapsed.as_millis(),
                summarize(obs)
            ),
            Err(err) => eprintln!(
                "   ERR in {} ms: {} ({err})",
                self.elapsed.as_millis(),
                error_kind(err)
            ),
        }
        eprintln!(
            "   stdout lines = {} · kinds = {:?}",
            self.log.lines.len(),
            line_kinds(&self.log.lines)
        );
        for line in self.log.lines.iter().filter(|l| is_answer_line(l)) {
            eprintln!("   answer shape = {}", shape_of_line(line));
        }
        eprintln!(
            "   exit = {:?} · exit after the answer = {:?} · stderr tail lines = {}",
            self.log.exit.map(|e| (e.code(), e.looks_not_installed())),
            self.log.exit_code,
            self.log.stderr_lines
        );
        eprintln!("   leftover scratch = {:?}", root.entries());
    }

    /// Only for the logout test — prints the upstream phrase masked (the classification table is a guess, so the real phrase is needed).
    fn report_upstream_text(&self) {
        for line in &self.log.lines {
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                eprintln!("   non-JSON line: {}", redact(line));
                continue;
            };
            let texts = [
                value.pointer("/response/error"),
                value.pointer("/error"),
                value.pointer("/result"),
                value.pointer("/message"),
            ];
            for text in texts.into_iter().flatten() {
                let text = match text {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                eprintln!("   upstream text [{}]: {}", kind_of(line), redact(&text));
            }
        }
    }
}

fn error_kind(err: &ProbeFailure) -> String {
    match &err.error {
        ProbeError::NotInstalled => "NotInstalled".into(),
        ProbeError::Unauthenticated => "Unauthenticated".into(),
        ProbeError::RateLimited { retry_after } => format!("RateLimited{{{retry_after:?}}}"),
        ProbeError::Unsupported => "Unsupported".into(),
        ProbeError::Timeout => "Timeout".into(),
        ProbeError::Spawn(_) => "Spawn".into(),
        ProbeError::Io(_) => "Io".into(),
        ProbeError::Parse(_) => "Parse".into(),
        ProbeError::Upstream(_) => "Upstream".into(),
    }
}

// ── Recording layer — wraps the real spawner and holds the lines · exit status ─────────────────

/// The upstream original text the querier puts out is only the value of one named field (the masked original-text field) — collecting the shape of response lines is done only in
/// this layer. The child is still spawned by the real [`OsProbeSpawner`].
struct Recorder {
    tweaks: Tweaks,
    log: Arc<Mutex<ChildLog>>,
}

#[derive(Default)]
struct ChildLog {
    command: Option<String>,
    lines: Vec<String>,
    exit: Option<ExitInfo>,
    /// The code received by the wait for exit without a decision after the answer (`Some(None)` = ended without a code).
    exit_code: Option<Option<i32>>,
    stderr_lines: usize,
}

impl Recorder {
    fn new(tweaks: Tweaks) -> Self {
        Self {
            tweaks,
            log: Arc::default(),
        }
    }

    fn take_log(&self) -> ChildLog {
        std::mem::take(&mut *self.log.lock().expect("기록 락"))
    }
}

impl ProbeSpawner for Recorder {
    fn spawn(
        &self,
        cmd: &ProbeCommand,
        deadline: Instant,
    ) -> Result<Box<dyn ProbeChild>, ProbeError> {
        let mut cmd = cmd.clone();
        cmd.env_remove
            .extend(self.tweaks.env_remove.iter().cloned());
        cmd.env_set.extend(self.tweaks.env_set.iter().cloned());
        self.log.lock().expect("기록 락").command =
            Some(format!("{} {}", cmd.program, cmd.args.join(" ")));
        let inner = OsProbeSpawner.spawn(&cmd, deadline)?;
        Ok(Box::new(RecordingChild {
            inner,
            log: Arc::clone(&self.log),
        }))
    }
}

struct RecordingChild {
    inner: Box<dyn ProbeChild>,
    log: Arc<Mutex<ChildLog>>,
}

impl ProbeChild for RecordingChild {
    fn write_line(&mut self, line: &str, deadline: Instant) -> Result<(), ProbeError> {
        self.inner.write_line(line, deadline)
    }

    fn close_stdin(&mut self) {
        self.inner.close_stdin();
    }

    fn read_line(&mut self, deadline: Instant) -> Result<Option<String>, ProbeError> {
        let got = self.inner.read_line(deadline);
        if let Ok(Some(line)) = &got {
            self.log.lock().expect("기록 락").lines.push(line.clone());
        }
        got
    }

    fn wait_exit(&mut self, deadline: Instant) -> Result<ExitInfo, ProbeError> {
        let got = self.inner.wait_exit(deadline);
        if let Ok(exit) = &got {
            self.log.lock().expect("기록 락").exit = Some(*exit);
        }
        got
    }

    fn wait_exit_code(&mut self, deadline: Instant) -> Result<Option<i32>, ProbeError> {
        let got = self.inner.wait_exit_code(deadline);
        if let Ok(code) = &got {
            self.log.lock().expect("기록 락").exit_code = Some(*code);
        }
        got
    }

    fn stderr_tail(&self) -> Vec<String> {
        self.inner.stderr_tail()
    }
}

/// Runs before the field drop (tree kill) — records the number of stderr lines that had come in by then (the content is not recorded).
impl Drop for RecordingChild {
    fn drop(&mut self) {
        let count = self.inner.stderr_tail().len();
        if let Ok(mut log) = self.log.lock() {
            log.stderr_lines = count;
        }
    }
}

// ── Real turn for pickup-shape collection ─────────────────────────────────────

/// The same rule as what the querier strips from the daemon env — keys starting with `CLAUDE` other than the two default logins.
/// If the parent is a claude session, that session's marker env changes the behavior of the child CLI.
fn claude_env_to_strip() -> Vec<String> {
    std::env::vars_os()
        .filter_map(|(key, _)| key.into_string().ok())
        .filter(|key| {
            key.get(..6)
                .is_some_and(|head| head.eq_ignore_ascii_case("CLAUDE"))
        })
        .filter(|key| {
            !["CLAUDE_CONFIG_DIR", "CLAUDE_CODE_OAUTH_TOKEN"]
                .iter()
                .any(|kept| key.eq_ignore_ascii_case(kept))
        })
        .collect()
}

/// All stdout lines of one stream-json turn — the core stream-json arguments of agent spawn + the same hook · MCP blocking as the querier.
fn claude_turn_lines(scratch: &Path, deadline: Instant) -> Result<Vec<String>, ProbeError> {
    let settings = scratch.join("settings.json");
    let mcp = scratch.join("mcp.json");
    fs::write(&settings, r#"{"disableAllHooks":true}"#).expect("설정 파일");
    fs::write(&mcp, r#"{"mcpServers":{}}"#).expect("MCP 설정 파일");
    let path = |p: &Path| p.to_str().expect("유니코드 경로").to_owned();
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--replay-user-messages",
        "--verbose",
        "--no-session-persistence",
        "--settings",
    ]
    .iter()
    .map(|a| (*a).to_owned())
    .collect();
    args.push(path(&settings));
    args.push("--strict-mcp-config".to_owned());
    // `--mcp-config` is a flag that takes several values, so put it at the very end (the comment at the same place in the querier).
    args.push("--mcp-config".to_owned());
    args.push(path(&mcp));
    let (program, args) = console_command("claude", args);
    let mut child = OsProbeSpawner.spawn(
        &ProbeCommand {
            program,
            args,
            cwd: scratch.to_path_buf(),
            env_set: Vec::new(),
            env_remove: claude_env_to_strip(),
        },
        deadline,
    )?;
    let user = json!({
        "type": "user",
        "message": {"role": "user", "content": [{"type": "text", "text": TURN_PROMPT}]},
    });
    child.write_line(&user.to_string(), deadline)?;
    let mut lines = Vec::new();
    read_until(child.as_mut(), &mut lines, deadline, |line| {
        kind_of(line).starts_with("result")
    })?;
    read_tail(child.as_mut(), &mut lines);
    Ok(lines)
}

/// All stdout lines of one app-server turn — handshake → thread → turn → `turn/completed`.
///
/// ★The thread's working folder = the repository root★ — if it is a folder seen for the first time, codex writes that folder into its own config file as trusted
/// (the codex policy comment in `commands.rs` — measured 0.155.1). Using a fresh temp folder every time piles deleted paths into the user's config.
fn codex_turn_lines(scratch: &Path, deadline: Instant) -> Result<Vec<String>, ProbeError> {
    let (program, args) =
        console_command("codex", vec!["app-server".to_owned(), "--stdio".to_owned()]);
    let mut child = OsProbeSpawner.spawn(
        &ProbeCommand {
            program,
            args,
            cwd: scratch.to_path_buf(),
            env_set: Vec::new(),
            env_remove: Vec::new(),
        },
        deadline,
    )?;
    let mut lines = Vec::new();
    let initialize = json!({"id": 1, "method": "initialize",
        "params": {"clientInfo": {"name": "engram-dashboard-smoke", "version": "0"}}});
    child.write_line(&initialize.to_string(), deadline)?;
    read_until(child.as_mut(), &mut lines, deadline, |l| {
        is_response_to(l, 1)
    })?;
    child.write_line(&json!({"method": "initialized"}).to_string(), deadline)?;
    let cwd = repo_root().to_str().expect("유니코드 경로").to_owned();
    let start = json!({"id": 2, "method": "thread/start",
        "params": {"cwd": cwd, "approvalPolicy": "never", "sandbox": "read-only"}});
    child.write_line(&start.to_string(), deadline)?;
    read_until(child.as_mut(), &mut lines, deadline, |l| {
        is_response_to(l, 2)
    })?;
    let thread_id = lines
        .iter()
        .rev()
        .find(|l| is_response_to(l, 2))
        .and_then(|l| serde_json::from_str::<Value>(l).ok())
        .and_then(|v| v.pointer("/result/thread/id")?.as_str().map(str::to_owned))
        .ok_or_else(|| ProbeError::Parse("thread/start 응답에 thread.id 가 없다".to_owned()))?;
    let turn = json!({"id": 3, "method": "turn/start",
        "params": {"threadId": thread_id, "input": [{"type": "text", "text": TURN_PROMPT}]}});
    child.write_line(&turn.to_string(), deadline)?;
    read_until(child.as_mut(), &mut lines, deadline, |l| {
        kind_of(l) == "turn/completed"
    })?;
    read_tail(child.as_mut(), &mut lines);
    Ok(lines)
}

/// Reads up to the line that is `done` (including that line). A server **request** (id + method) is answered with a refusal — if we do not answer, the other side stalls.
fn read_until(
    child: &mut dyn ProbeChild,
    lines: &mut Vec<String>,
    deadline: Instant,
    done: impl Fn(&str) -> bool,
) -> Result<(), ProbeError> {
    loop {
        let Some(line) = child.read_line(deadline)? else {
            return Err(ProbeError::Upstream(
                "stdout 이 끝날 때까지 기다리던 줄이 안 왔다".to_owned(),
            ));
        };
        if let Ok(value) = serde_json::from_str::<Value>(&line) {
            if let (Some(id), Some(_)) = (value.get("id"), value.get("method")) {
                let refusal =
                    json!({"id": id, "error": {"code": -32601, "message": "smoke refuses"}});
                child.write_line(&refusal.to_string(), deadline)?;
            }
        }
        let finished = done(&line);
        lines.push(line);
        if finished {
            return Ok(());
        }
    }
}

/// Picks up lines that arrive late after the end signal for [`TURN_TAIL`] more. If the deadline is exceeded the child is cut — here that is the end.
fn read_tail(child: &mut dyn ProbeChild, lines: &mut Vec<String>) {
    let until = Instant::now() + TURN_TAIL;
    while let Ok(Some(line)) = child.read_line(until) {
        lines.push(line);
    }
}

// ── Masked summary ────────────────────────────────────────────────────────────

fn summarize(obs: &UsageObservation) -> String {
    let mut out = format!(
        "   vendor={} source={:?} plan={} limits_unavailable={}\n   five_hour: {}\n   weekly:    {}\n",
        obs.vendor.as_str(),
        obs.source,
        obs.plan.as_deref().unwrap_or("-"),
        obs.limits_unavailable.as_ref().map_or("-", |detail| detail.kind),
        window_text(obs.five_hour.as_ref()),
        window_text(obs.weekly.as_ref()),
    );
    match &obs.model_scoped {
        None => out.push_str("   model_scoped: none"),
        Some(list) => {
            out.push_str(&format!("   model_scoped: {} entries", list.len()));
            for scoped in list {
                out.push_str(&format!(
                    "\n     - {:?}: {}",
                    scoped.label,
                    window_text(Some(&scoped.window))
                ));
            }
        }
    }
    out
}

fn window_text(window: Option<&WindowObs>) -> String {
    let Some(window) = window else {
        return "absent".to_owned();
    };
    format!(
        "used_pct={} resets_at={}",
        window
            .used_pct
            .map_or_else(|| "-".to_owned(), |p| format!("{p}")),
        window.resets_at.map_or_else(|| "-".to_owned(), iso_utc)
    )
}

/// epoch seconds → `YYYY-MM-DDTHH:MM:SSZ (+남은 시간)`. Calendar conversion = the Gregorian day-count formula (proleptic).
fn iso_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let left = secs - now;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z (in {}h{:02}m)",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        left.div_euclid(3600),
        left.rem_euclid(3600) / 60
    )
}

/// Kind of line — claude = `type[/subtype]` · codex = `method` (notification · request) or `response#id`/`error#id`.
fn kind_of(line: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return format!("non-json({} bytes)", line.len());
    };
    if let Some(kind) = value.get("type").and_then(Value::as_str) {
        let subtype = value
            .get("subtype")
            .or_else(|| value.pointer("/response/subtype"))
            .and_then(Value::as_str);
        return match subtype {
            Some(sub) => format!("{kind}/{sub}"),
            None => kind.to_owned(),
        };
    }
    if let Some(method) = value.get("method").and_then(Value::as_str) {
        return method.to_owned();
    }
    match (value.get("id"), value.get("error")) {
        (Some(id), Some(_)) => format!("error#{id}"),
        (Some(id), None) => format!("response#{id}"),
        _ => "other".to_owned(),
    }
}

fn line_kinds(lines: &[String]) -> Vec<String> {
    lines.iter().map(|line| kind_of(line)).collect()
}

fn is_response_to(line: &str, id: i64) -> bool {
    serde_json::from_str::<Value>(line)
        .is_ok_and(|v| v.get("method").is_none() && v.get("id").and_then(Value::as_i64) == Some(id))
}

fn is_rpc_response(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .is_ok_and(|v| v.get("id").is_some() && v.get("method").is_none())
}

/// The answer line of a query — claude `control_response` · codex response · error.
fn is_answer_line(line: &str) -> bool {
    let kind = kind_of(line);
    kind.starts_with("control_response") || is_rpc_response(line)
}

/// The shape of a line with values removed. Booleans are visible, string values show only identifying words (list below) and the rest `str`, numbers `num`.
fn shape_of_line(line: &str) -> String {
    match serde_json::from_str::<Value>(line) {
        Ok(value) => shape(&value, None),
        Err(_) => format!("non-json({} bytes)", line.len()),
    }
}

/// String fields whose values may be printed — only words that tell the kind apart (fields that carry no account · path · phrase).
const SHOWN_STRING_KEYS: [&str; 9] = [
    "type",
    "subtype",
    "method",
    "rateLimitType",
    "status",
    "limitId",
    "kind",
    "group",
    "severity",
];

fn shape(value: &Value, key: Option<&str>) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(_) => "num".to_owned(),
        Value::String(s)
            if key.is_some_and(|k| SHOWN_STRING_KEYS.contains(&k)) && s.len() <= 40 =>
        {
            format!("{s:?}")
        }
        Value::String(_) => "str".to_owned(),
        Value::Array(items) => match items.first() {
            None => "[]".to_owned(),
            Some(first) => format!("[{} ×{}]", shape(first, None), items.len()),
        },
        Value::Object(map) => {
            let fields: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{k}: {}", shape(v, Some(k))))
                .collect();
            format!("{{{}}}", fields.join(", "))
        }
    }
}

/// Numeric fields of `rate_limit_info` — only those with no account information.
fn rate_limit_info_values(line: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return "non-json".to_owned();
    };
    let Some(info) = value.get("rate_limit_info").and_then(Value::as_object) else {
        return "no rate_limit_info".to_owned();
    };
    let pick = |k: &str| info.get(k).map_or_else(|| "-".to_owned(), Value::to_string);
    format!(
        "rateLimitType={} status={} utilization={} resetsAt={}",
        pick("rateLimitType"),
        pick("status"),
        pick("utilization"),
        pick("resetsAt")
    )
}

/// Whether `params.rateLimits.limitId` exists · what it is (the value is shown only when it is a short identifying word).
fn codex_limit_id(line: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return "non-json".to_owned();
    };
    match value.pointer("/params/rateLimits/limitId") {
        None => "ABSENT (field missing)".to_owned(),
        Some(Value::Null) => "null".to_owned(),
        Some(Value::String(id))
            if id.len() <= 40
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') =>
        {
            format!("{id:?}")
        }
        Some(other) => format!("present but not a short id ({})", shape(other, None)),
    }
}

/// Masks token shapes · user paths and truncates before printing an upstream phrase.
fn redact(text: &str) -> String {
    let mut out = engram_dashboard_base::logging::mask_secrets(text);
    for (dir, label) in [
        (std::env::temp_dir(), "<tmp>"),
        (
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(PathBuf::from)
                .unwrap_or_default(),
            "<home>",
        ),
    ] {
        let Some(dir) = dir.to_str().filter(|d| d.len() > 3) else {
            continue;
        };
        let dir = dir.trim_end_matches(&['\\', '/'][..]);
        out = out.replace(dir, label);
        out = out.replace(&dir.replace('\\', "/"), label);
        out = out.replace(&dir.replace('\\', "\\\\"), label);
    }
    out.chars().take(240).collect()
}

// ── Temp folder ───────────────────────────────────────────────────────────────

fn repo_root() -> PathBuf {
    dunce::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).expect("저장소 루트")
}

/// One folder the test made — deleted whole on drop.
struct TempTree {
    path: PathBuf,
}

impl TempTree {
    fn fresh(tag: &str) -> Self {
        Self::at(std::env::temp_dir().join(format!(
            "engram-usage-smoke-{tag}-{}",
            Uuid::new_v4().simple()
        )))
    }

    /// If a same folder left by a previous run exists, delete it and make it anew.
    fn at(path: PathBuf) -> Self {
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("시험 폴더");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn entries(&self) -> Vec<String> {
        fs::read_dir(&self.path)
            .map(|dir| {
                dir.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
