//! 사용량 조회기의 실 CLI 스모크 — 설치·로그인된 claude·codex 에 **운영 경로 그대로** 조회를 보낸다: 등록부
//! (`usage_probe_for`) → [`UsageProbe::query`] → 실 [`OsProbeSpawner`](Windows = `cmd.exe /c <cli> …`), 마감 =
//! 조회기 정책의 시한(받는 쪽이 만드는 것과 같다).
//!
//! ★전부 `#[ignore]` 다★ — 로그인된 CLI 와 계정의 조회 제한을 쓴다(CI 러너에는 둘 다 없다). 로컬에서만:
//!   `cargo test -p engram-dashboard-agent --test usage_probe_smoke -- --ignored --nocapture --test-threads=1`
//!   `--test-threads=1` = 조회를 겹쳐 보내지 않는다(조회 제한을 한꺼번에 쓰지 않게).
//! ★찍는 것은 가린 요약뿐이다★ — 창 수치·리셋 시각·plan·모델 이름·줄의 모양(키와 타입). env 값·토큰·문자열
//!   값은 찍지 않는다. 예외 = 로그아웃 시험의 상류 문구(빈 설정 폴더라 계정 정보가 없다 — 토큰 모양·경로는 가린다).
//! ★파일을 OS 로 막지 않는다★ — 조회 명령의 OS 분기는 조회기 안에 있다. 줍기 모양 수집(`…_passive_parser`)만
//!   조회기 밖에서 CLI 를 띄우므로 같은 분기를 [`cli`] 한 함수에 둔다.
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

/// 조회 한 번이 돌아와야 하는 상한 = 시한 + 자식 drop 의 종료 대기([`UsageProbe::query`] 계약) + 여유.
const RETURN_SLACK: Duration = Duration::from_secs(3);

/// 줍기 모양 수집의 모델 턴 한 번의 시한.
const TURN_TIMEOUT: Duration = Duration::from_secs(120);
/// 턴이 끝난 뒤 늦게 오는 알림을 더 기다리는 시간.
const TURN_TAIL: Duration = Duration::from_millis(1500);
const TURN_PROMPT: &str = "Reply with just OK.";

// ── 운영 경로 조회 ───────────────────────────────────────────────────────────────

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

/// 빈 설정 폴더 = 로그인 없음. 분류를 단정하지 않는다 — 응답 모양 수집이 목적이다(TRD §6 #4). 재는 계약은
/// 둘이다: 마감 안에 돌아온다 · 성공으로 돌아오면 「한도 정보 없음」이다.
/// ★실측(claude 2.1.280 · 2026-09-27) = 오류가 아니라 성공 응답이다★ — `rate_limits: null` ·
/// `subscription_type: null` · `rate_limits_available: false`(로그인된 쪽은 `true`). 상류 오류 문구는 없었다.
#[test]
#[ignore = "실 claude CLI 를 쓴다"]
fn claude_logged_out_answer_shape() {
    let root = TempTree::fresh("claude-logged-out");
    let config = TempTree::fresh("claude-empty-config");
    let config_dir = config.path().to_str().expect("유니코드 경로").to_owned();
    // 데몬 env 에서 남기는 로그인 키와 API 키 경로도 뺀다 — 남으면 빈 설정 폴더여도 로그인된 채로 조회한다.
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
    // 답 직후 kill 하면 CLI 가 쓰던 설정 파일의 잠금·임시 파일이 남았다 — 조회가 자식에게 끝낼 유예를 주는지 본다.
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

/// TRD §6 #18 — 개발 빌드의 데이터 폴더는 저장소 안이라, CLI 가 작업 폴더에서 위로 올라가며 저장소의
/// `.claude/settings.json`·CLAUDE.md 를 읽을 수 있다. 그래도 조회가 서는지 잰다.
#[test]
#[ignore = "실 claude CLI·로그인·계정의 조회 제한을 쓴다"]
fn claude_get_usage_answers_with_the_scratch_root_inside_the_repo() {
    let root = TempTree::at(repo_root().join(".engram-data").join("usage-probe-smoke"));
    let run = run_probe("claude", root.path(), Tweaks::default());
    run.report("claude get_usage (scratch root inside the repo)", &root);
    run.result
        .expect("저장소 안 작업 폴더에서도 claude 조회가 성공해야 한다");
}

/// 답 뒤 유예(`FINISH_GRACE`)의 실측 — 조회기의 대화를 운영 명령 그대로 돌리되, 답을 받고 stdin 을 닫은 뒤의
/// 대기만 [`EXIT_WINDOW`] 로 늘려 claude 가 **스스로** 끝나기까지를 잰다(창 안에 안 끝나면 그때 끊긴다). 그 사이
/// 전역 설정 파일(`.claude.json`)은 수정 시각과 잠금 파일의 있고 없음만 본다 — 내용은 읽지 않는다.
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

/// stdin 을 닫은 뒤 claude 가 스스로 끝나기를 기다리는 상한(운영 유예보다 훨씬 길다).
const EXIT_WINDOW: Duration = Duration::from_secs(10);

/// claude 의 전역 설정 파일 — `CLAUDE_CONFIG_DIR` 가 있으면 그 안, 없으면 홈 폴더의 `.claude.json`(조회기는 그
/// env 를 남긴다). ★미확인★: CLI 의 경로 규칙을 소스로 확인하지 않았다 — 파일이 없으면 시각을 못 잰다.
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

/// 설정 파일의 한 시점 — 수정 시각과 옆 잠금 파일의 있고 없음(내용은 읽지 않는다).
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

/// 실 스포너를 감싸 stdin 을 닫은 뒤의 마감만 [`EXIT_WINDOW`] 로 늘린다 — 그 밖은 조회기의 대화 그대로다.
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

// ── 줍기 모양 수집 — 짧은 실 턴의 원 줄을 줍기 해석기에 먹인다 ─────────────────────────

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
    // 운영 통로는 우리 요청의 응답을 대기표가 가져가고 나머지 줄을 번역기에 넘긴다 — 응답 줄은 먹이지 않는다.
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

// ── 조회 구동 ─────────────────────────────────────────────────────────────────────

/// 조회 명령에 더하는 env 조정 — 조회기가 만든 명령에 얹는다(`env_set` 이 나중에 적용된다).
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

    /// 로그아웃 시험 전용 — 상류 문구를 가려서 찍는다(분류 표가 추정이라 실제 문구가 필요하다).
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

// ── 기록 겹 — 실 스포너를 감싸 줄·종료 상태를 붙든다 ─────────────────────────────────

/// 조회기가 밖에 내는 상류 원문은 이름 붙은 칸의 값 하나(가린 원문 칸)뿐이다 — 응답 줄의 모양 수집은 이 겹에서만
/// 된다. 자식은 여전히 실 [`OsProbeSpawner`] 가 띄운다.
struct Recorder {
    tweaks: Tweaks,
    log: Arc<Mutex<ChildLog>>,
}

#[derive(Default)]
struct ChildLog {
    command: Option<String>,
    lines: Vec<String>,
    exit: Option<ExitInfo>,
    /// 답을 받은 뒤의 판정 없는 종료 대기가 받은 코드(`Some(None)` = 코드 없이 끝났다).
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

/// 필드 drop(트리 kill)보다 먼저 돈다 — 그 시점까지 들어온 stderr 줄 수를 적는다(내용은 적지 않는다).
impl Drop for RecordingChild {
    fn drop(&mut self) {
        let count = self.inner.stderr_tail().len();
        if let Ok(mut log) = self.log.lock() {
            log.stderr_lines = count;
        }
    }
}

// ── 줍기 모양 수집용 실 턴 ─────────────────────────────────────────────────────────

/// 조회기 밖에서 CLI 를 띄울 때의 명령 모양 — 운영의 `console_command` 와 같은 규칙이다(Windows 의 `claude`·`codex`
/// 는 확장자 없는 npm shim 이라 `cmd.exe` 가 해석해야 뜬다).
fn cli(program: &str, args: Vec<String>) -> (String, Vec<String>) {
    if cfg!(windows) {
        let mut wrapped = vec!["/c".to_owned(), program.to_owned()];
        wrapped.extend(args);
        ("cmd.exe".to_owned(), wrapped)
    } else {
        (program.to_owned(), args)
    }
}

/// 데몬 env 에서 조회기가 벗기는 것과 같은 규칙 — `CLAUDE` 로 시작하는 키 중 기본 로그인 둘이 아닌 것.
/// 부모가 claude 세션이면 그 세션의 표식 env 가 자식 CLI 의 동작을 바꾼다.
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

/// stream-json 턴 하나의 stdout 줄 전량 — 에이전트 스폰의 stream-json 핵심 인자 + 조회기와 같은 훅·MCP 차단.
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
    // `--mcp-config` 는 값을 여럿 받는 플래그라 맨 끝에 둔다(조회기의 같은 자리 주석).
    args.push("--mcp-config".to_owned());
    args.push(path(&mcp));
    let (program, args) = cli("claude", args);
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

/// app-server 턴 하나의 stdout 줄 전량 — 악수 → 스레드 → 턴 → `turn/completed`.
///
/// ★스레드의 작업 폴더 = 저장소 루트★ — 처음 보는 폴더면 codex 가 그 폴더를 자기 설정 파일에 신뢰로 적는다
/// (`commands.rs` 의 codex 정책 주석 — 실측 0.155.1). 매번 새 임시 폴더를 쓰면 사용자 설정에 지워진 경로가 쌓인다.
fn codex_turn_lines(scratch: &Path, deadline: Instant) -> Result<Vec<String>, ProbeError> {
    let (program, args) = cli("codex", vec!["app-server".to_owned(), "--stdio".to_owned()]);
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

/// `done` 인 줄까지 읽는다(그 줄 포함). 서버 **요청**(id + method)은 거절로 답한다 — 안 답하면 상대가 멈춘다.
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

/// 끝 신호 뒤에 늦게 오는 줄을 [`TURN_TAIL`] 동안 더 줍는다. 마감을 넘기면 자식이 끊긴다 — 여기서는 그게 끝이다.
fn read_tail(child: &mut dyn ProbeChild, lines: &mut Vec<String>) {
    let until = Instant::now() + TURN_TAIL;
    while let Ok(Some(line)) = child.read_line(until) {
        lines.push(line);
    }
}

// ── 가린 요약 ─────────────────────────────────────────────────────────────────────

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

/// epoch 초 → `YYYY-MM-DDTHH:MM:SSZ (+남은 시간)`. 달력 변환 = 그레고리력 일수 공식(proleptic).
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

/// 줄의 종류 — claude = `type[/subtype]` · codex = `method`(알림·요청) 또는 `response#id`/`error#id`.
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

/// 조회의 답 줄 — claude `control_response` · codex 응답·오류.
fn is_answer_line(line: &str) -> bool {
    let kind = kind_of(line);
    kind.starts_with("control_response") || is_rpc_response(line)
}

/// 값을 뺀 줄의 모양. 참거짓은 보이고, 문자열 값은 식별 낱말(아래 목록)만 보이고 나머지는 `str`, 수는 `num`.
fn shape_of_line(line: &str) -> String {
    match serde_json::from_str::<Value>(line) {
        Ok(value) => shape(&value, None),
        Err(_) => format!("non-json({} bytes)", line.len()),
    }
}

/// 값을 찍어도 되는 문자열 칸 — 종류를 가르는 낱말뿐이다(계정·경로·문구가 실리지 않는 칸).
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

/// `rate_limit_info` 의 수치 칸 — 계정 정보가 없는 칸만.
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

/// `params.rateLimits.limitId` 가 있나 · 무엇인가(짧은 식별 낱말일 때만 값을 보인다).
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

/// 상류 문구를 찍기 전에 토큰 모양·사용자 경로를 가리고 자른다.
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

// ── 임시 폴더 ─────────────────────────────────────────────────────────────────────

fn repo_root() -> PathBuf {
    dunce::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).expect("저장소 루트")
}

/// 시험이 만든 폴더 하나 — drop 하면 통째로 지운다.
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

    /// 지난 실행이 남긴 같은 폴더가 있으면 지우고 새로 만든다.
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
