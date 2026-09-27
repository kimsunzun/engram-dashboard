//! 능동 조회(probe)의 벤더 중립 seam — 조회기([`UsageProbe`])와 그것이 자식 프로세스를 다루는 손잡이
//! ([`ProbeSpawner`]·[`ProbeChild`]).
//!
//! ★벤더 지식(명령·인자·env·대화 모양·쿨타임·시한 값)은 여기 없다★ — 각 벤더 backend 의 `usage_probe.rs` 가
//!   [`UsageProbe`] 를 구현하며 진다. 이 모듈은 「짧게 뜨는 자식 하나와 줄 단위로 말하고, 마감 하나 안에
//!   끝낸다」는 모양만 정한다. 실물 스포너는 [`super::OsProbeSpawner`], 시험 대역은 `usage::testing`.
// ADR-0004

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{UsageDetail, UsageObservation, UsageVendorKey};

/// 벤더 하나의 조회 정책 — 값은 각 벤더 backend 가 정한다(받는 쪽에는 벤더 정책이 없다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsagePolicy {
    /// 자동 조회 사이의 최소 간격.
    pub cooldown: Duration,
    /// 조회 하나 전체(기동·쓰기·읽기·종료 대기)의 시한. 받는 쪽이 이것으로 [`ProbeEnv::deadline`] 을 만든다.
    pub timeout: Duration,
}

/// 조회 실패의 분류 — 받는 쪽이 보이는 상태로 접는다. 조회기는 이것을 [`ProbeFailure`] 에 싸서 돌려준다.
///
/// ★문자열 칸은 로그로 간다 — 넣을 수 있는 것 = 우리가 쓴 문장과 OS 오류 문구뿐이다★: 자격증명·토큰·env 값은
///   물론, 상류가 준 문구(오류 문구·응답에 실린 계정 식별자·stderr)도 넣지 않는다. 상류 원문은
///   [`UsageDetail::upstream`] 전용 칸에만 산다 — 다듬는 규칙과 행선지는 [`super::UpstreamText`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProbeError {
    /// 조회할 프로그램이 이 PC 에 없다.
    #[error("not installed")]
    NotInstalled,
    /// 로그인이 안 됐거나 인증이 거절됐다.
    #[error("unauthenticated")]
    Unauthenticated,
    /// 상류가 조회를 한도로 거절했다. `retry_after` = 상류가 알려 준 대기(없으면 모른다).
    #[error("rate limited")]
    RateLimited { retry_after: Option<Duration> },
    /// 이 설치·모드에서는 조회 기능이 없다.
    #[error("unsupported")]
    Unsupported,
    /// 마감을 넘겼다 — 넘긴 자리가 기동·쓰기·읽기·종료 대기 어디든 이 하나다.
    #[error("timed out")]
    Timeout,
    /// 자식을 띄우지 못했다(프로그램이 없는 경우는 [`ProbeError::NotInstalled`]).
    #[error("spawn failed: {0}")]
    Spawn(String),
    /// 파이프·파일 입출력이 실패했다.
    #[error("io error: {0}")]
    Io(String),
    /// 응답 모양이 깨졌다.
    #[error("parse error: {0}")]
    Parse(String),
    /// 상류가 오류를 줬는데 위 어느 분류에도 안 든다.
    #[error("upstream error: {0}")]
    Upstream(String),
}

impl ProbeError {
    /// 분류 낱말 — [`UsageDetail::kind`] 의 중립 실패 몫. ★wire 로 나가고 화면이 번역 없이 보인다★ — 철자를 바꾸면
    /// 보이는 낱말이 바뀐다.
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

/// 조회 한 번의 실패 — 분류([`ProbeError`])와 그 근거([`UsageDetail`]).
///
/// - `detail: None` = 근거가 분류 하나뿐이다 — 받는 쪽이 [`ProbeError::kind_word`] 로 채운다. 스포너·임시 폴더처럼
///   seam 에서 `?` 로 올라온 실패가 이 모양이다(`From<ProbeError>`).
/// - ★`Display` = 분류 문구 + `detail.kind` 뿐이다 — 상류 원문은 안 찍는다★. `Debug` 도 원문 대신 글자 수를 찍는다.
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

/// 벤더 하나의 능동 조회기. 싱글턴은 각 벤더 backend 에 산다.
pub trait UsageProbe: Send + Sync {
    /// 이 조회기가 채우는 칸의 벤더 키(정본 철자).
    fn key(&self) -> UsageVendorKey;

    fn policy(&self) -> UsagePolicy;

    /// 조회 한 번. ★blocking★ — 부르는 쪽이 전용 스레드에서 부른다.
    ///
    /// - 결과의 `source` 는 [`super::UsageSource::Active`], `vendor` 는 [`UsageProbe::key`] 다.
    /// - ★맨 먼저 [`ProbeEnv::require_time_left`] 를 부르고, `env.deadline` 을 기동과 자식의 모든 동작에 그대로
    ///   넘긴다★ — 그러면 이 호출은 늦어도 `deadline + KILL_WAIT`([`super::KILL_WAIT`]) + 임시 폴더 지우기의 짧은
    ///   재시도([`super::ScratchDir`] 의 drop — 약 0.2초) 안에 돌아온다(마지막 몫 = 자식 drop 의 종료 대기와 그
    ///   뒤의 폴더 지우기). 더 긴 마감을 스스로 만들지 않는다.
    /// - 답을 받으면 자식을 [`finish_after_answer`] 로 끝낸다.
    /// - 외부 데이터에 패닉하지 않는다(릴리즈는 `panic = "abort"`).
    fn query(&self, env: &ProbeEnv<'_>) -> Result<UsageObservation, ProbeFailure>;
}

/// 조회 한 번에 받는 쪽이 넘기는 것.
pub struct ProbeEnv<'a> {
    pub spawner: &'a dyn ProbeSpawner,
    /// 조회 하나 전체의 마감 — 조회기는 이것을 기동과 자식의 동작마다 그대로 넘긴다.
    pub deadline: Instant,
    /// 조회마다 임시 폴더를 만들 부모 폴더([`super::ScratchDir::create`]). 받는 쪽 소유의 전용 폴더다.
    pub scratch_root: &'a Path,
}

impl ProbeEnv<'_> {
    /// 마감이 이미 지났으면 [`ProbeError::Timeout`] — 조회기가 임시 폴더를 만들기 **전에** 부른다.
    pub fn require_time_left(&self) -> Result<(), ProbeError> {
        if Instant::now() >= self.deadline {
            return Err(ProbeError::Timeout);
        }
        Ok(())
    }
}

/// 자식 하나의 기동 명세.
///
/// 에이전트용 `CommandSpec` 과 따로 두는 이유 = 저쪽은 env 를 더하기만 한다. 조회는 물려받은 env 를 벗겨야
/// 한다([`ProbeCommand::env_remove`]).
#[derive(Clone, PartialEq, Eq)]
pub struct ProbeCommand {
    pub program: String,
    pub args: Vec<String>,
    /// 자식의 작업 폴더 — 있는 폴더여야 한다(없으면 [`ProbeError::Spawn`]).
    pub cwd: PathBuf,
    /// 더할 env. ★[`ProbeCommand::env_remove`] 보다 나중에 적용된다★ — 두 목록에 같은 키가 있으면 남는다.
    pub env_set: Vec<(String, String)>,
    /// 물려받은 env 에서 뺄 키(정확한 이름 — 패턴이 아니다). 대소문자 구분은 OS 규칙을 따른다.
    pub env_remove: Vec<String>,
}

/// ★env 값을 찍지 않는다★ — 키만 보인다. 값에 자격증명이 실릴 수 있다.
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

/// 자식을 띄우는 손잡이 — 실물 = [`super::OsProbeSpawner`].
pub trait ProbeSpawner: Send + Sync {
    /// 세 파이프를 연 자식 하나를 띄운다.
    ///
    /// - 마감이 이미 지났으면 띄우지 않고 [`ProbeError::Timeout`].
    /// - 기동 자체가 「그런 프로그램 없음」으로 실패하면 [`ProbeError::NotInstalled`](POSIX 등). ★셸로 감싼 명령
    ///   (Windows `cmd.exe /c <prog>`)은 기동이 성공한다★ — 감싼 대상이 없다는 것은 종료 상태로 온다
    ///   ([`ExitInfo::looks_not_installed`]). 띄우기 전에 찾아보지 않는다.
    fn spawn(
        &self,
        cmd: &ProbeCommand,
        deadline: Instant,
    ) -> Result<Box<dyn ProbeChild>, ProbeError>;
}

/// 떠 있는 자식 하나.
///
/// ★세 동작 모두 같은 규칙의 마감 아래 있다★: 마감까지 끝나지 않으면(또는 이미 지났으면) 자식을 **트리째**
///   끊고 [`ProbeError::Timeout`] 이다. 한 번 마감으로 끊긴 뒤에는 모든 동작이 곧바로 `Timeout` 이다.
/// ★drop = 트리 kill + 유계 종료 대기([`super::KILL_WAIT`])★ — 조회기는 답을 받으면 [`finish_after_answer`] 로
///   끝낸다(그 끝이 drop 이다). 임시 폴더는 자식보다 **먼저** 선언해야 drop 순서(자식 → 폴더)가 「죽인 뒤
///   지운다」가 된다.
pub trait ProbeChild: Send {
    /// `line` 끝에 `\n` 을 붙여 stdin 에 쓴다. stdin 은 [`ProbeChild::close_stdin`] 이나 drop 까지 열려 있다.
    fn write_line(&mut self, line: &str, deadline: Instant) -> Result<(), ProbeError>;

    /// stdin 을 닫는다(자식은 EOF 를 본다). 이미 닫혔으면 아무것도 안 한다. 이후의 `write_line` 은 `Err(Io)`.
    fn close_stdin(&mut self);

    /// stdout 한 줄(끝의 `\n`·`\r\n` 을 뗀 것). `None` = stdout EOF(이후로도 계속 `None`).
    /// UTF-8 이 아닌 바이트는 대체 문자로 바뀐다. 개행 없이 상한에 닿은 줄 하나(상한을 넘는 줄 · 개행 없이
    /// 상한을 딱 채운 조각)는 `Err(Io)` 이고 스트림은 다음 줄로 이어진다.
    fn read_line(&mut self, deadline: Instant) -> Result<Option<String>, ProbeError>;

    /// 자식(직접 띄운 프로세스)이 끝나기를 기다린다. stdout 을 닫고도 살아 있는 자식도 마감에 끊긴다.
    /// ★stdout EOF 를 본 뒤에 부른다★ — 덜 읽은 채 기다리면 자식이 출력에 막혀 마감까지 안 끝날 수 있다.
    /// 「프로그램 없음」 판정([`ExitInfo::looks_not_installed`])도 이 안에서 같은 마감 아래 돈다 — 답을 이미
    /// 받았으면 판정 없는 [`ProbeChild::wait_exit_code`] 를 쓴다.
    fn wait_exit(&mut self, deadline: Instant) -> Result<ExitInfo, ProbeError>;

    /// [`ProbeChild::wait_exit`] 와 같되 「프로그램 없음」 판정을 하지 않는다(`None` = 코드 없이 끝났다). 판정은
    /// 파일 시스템 찾기를 돌릴 수 있어(스포너 몫) 답을 받은 뒤의 종료 대기([`finish_after_answer`])는 이것을 쓴다.
    fn wait_exit_code(&mut self, deadline: Instant) -> Result<Option<i32>, ProbeError>;

    /// stderr 의 마지막 몇 줄 — 토큰 모양은 가렸고 줄마다 잘랐다. 스포너는 이 내용을 로그에 싣지 않는다.
    ///
    /// ★가린 것은 토큰 모양뿐이다★ — 계정 식별자·경로 같은 것은 그대로 남을 수 있다. 로그에 실을지는 그것을
    ///   아는 조회기가 정하고, [`ProbeError`] 문자열에는 넣지 않는다. 자식이 막 끝난 직후에는 마지막 줄이 아직
    ///   안 들어와 있을 수 있다.
    fn stderr_tail(&self) -> Vec<String>;
}

/// 자식의 종료 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitInfo {
    code: Option<i32>,
    not_installed: bool,
}

impl ExitInfo {
    /// `not_installed` = 스포너가 이 종료를 「띄우려던 프로그램이 없다」로 판정했나(OS 규칙은 스포너 몫).
    pub const fn new(code: Option<i32>, not_installed: bool) -> Self {
        Self {
            code,
            not_installed,
        }
    }

    /// `None` = 코드 없이 끝났다(POSIX 신호 등).
    pub const fn code(&self) -> Option<i32> {
        self.code
    }

    /// 감싸는 셸이 「그런 명령이 없다」로 끝났나 — 조회기는 응답 없이 stdout EOF 를 본 뒤 이것이 참이면
    /// [`ProbeError::NotInstalled`] 로 접는다. ★거짓이라고 설치된 것은 아니다★ — 스포너가 판정을 마감 안에
    /// 못 끝냈거나, 같은 대상에 대한 앞선 판정이 아직 돌고 있어 새로 판정하지 않았을 수 있다(판정 규칙 = 스포너
    /// 몫).
    pub const fn looks_not_installed(&self) -> bool {
        self.not_installed
    }
}

/// 답을 받은 뒤 자식을 끝낸다 — stdin 을 닫고, 남은 stdout 을 흘리며 스스로 끝나기를 [`FINISH_GRACE`](마감을
/// 넘지 않게) 기다린 뒤 drop(트리 kill)한다. 기다림이 마감에 걸려도 조회의 실패가 아니다 — 답은 이미 받았다.
///
/// 곧바로 kill 하지 않는 이유 = CLI 가 제 파일 쓰기를 끝내게 하려는 것이다 — 답 직후 kill 하면 설정 폴더에
/// 잠금·임시 파일이 남는 것이 실측됐다(벤더와 실측 기록은 부르는 조회기 쪽 주석).
pub fn finish_after_answer(mut child: Box<dyn ProbeChild>, deadline: Instant) {
    let grace = deadline.min(Instant::now() + FINISH_GRACE);
    child.close_stdin();
    loop {
        match child.read_line(grace) {
            // 상한에 걸린 줄도 흘린다 — 스트림은 다음 줄로 이어진다.
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

/// [`finish_after_answer`] 의 유예 상한 — 조회 CLI 가 stdin EOF 뒤 스스로 끝나기까지 걸린 시간(실측 최대 약 1.1초,
/// 그 사이 제 설정 파일을 쓴다)을 덮는 값이다. 넘기면 트리째 끊는다. 벤더와 실측 기록은 부르는 조회기 쪽 주석.
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

    /// ★답을 받은 뒤에는 「프로그램 없음」 판정을 부르지 않는다★ — 0 이 아닌 종료여도(판정이 찾기를 돌릴 자리).
    #[test]
    fn finish_never_asks_for_the_not_installed_judgment() {
        let spawner = ScriptedSpawner::new(ChildScript::new().exit_code(1));
        let child = spawner.spawn(&command(), far()).expect("기동");
        finish_after_answer(child, far());
        assert!(!spawner.exit_judged());
        assert!(!spawner.cut_at_deadline());
        assert!(spawner.child_dropped());
    }

    /// stdin 을 닫아도 안 끝나는 자식은 유예 뒤에 끊는다 — 마감까지 끌지 않는다.
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

    /// 분류 낱말은 wire 로 나간다 — 바뀌면 화면의 낱말이 바뀐다.
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
