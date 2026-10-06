//! claude 의 사용량 능동 조회 — 짧게 뜨는 `claude -p`(stream-json)에 `get_usage` 제어 요청 한 줄을 보내고
//! 그 응답을 [`UsageObservation`] 으로 푼다. 부르는 쪽은 [`CLAUDE_USAGE_PROBE`] 를 [`UsageProbe`] 로만 본다(ADR-0248).
//!
//! ★모델 호출이 없는 요청이다 — 사용자 메시지를 한 줄도 쓰지 않는다★: stdin 에 가는 것은 제어 요청 한 줄뿐이다.
//!   stdin 은 응답을 받을 때까지 열어 둔다(닫으면 CLI 가 끝나 응답을 못 받을 수 있다). 응답을 받으면 stdin 을 닫고
//!   잠깐 기다린 뒤 끝낸다([`finish_after_answer`]).
//! ★응답 모양은 실험 기능이다★(SDK 메서드 이름부터 `…_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET`) —
//!   실측·출처 = `docs/research/claude-usage-query-method-survey-2026-09-27.md` §3·§7(claude 2.1.280). 그래서
//!   칸 하나가 틀리면 그 칸만 버리고, 응답의 뼈대가 깨졌을 때만 [`ProbeError::Parse`] 다.
//! ★응답·오류 문구를 로그와 오류 문자열에 싣지 않는다★ — 계정 정보가 실릴 수 있다. 싣는 것은 우리가 쓴
//!   문장과 수치뿐이다. 상류 원문은 [`UsageDetail::upstream`] 전용 칸에만 싣고, 그것도 이름 붙은 칸의 값뿐이다
//!   (오류 응답의 `error` · 한도 두 칸 `rate_limits_available`·`rate_limits` · 「한도 정보 없음」의 `subscription_type`).
// ADR-0004

use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use chrono::DateTime;
use engram_dashboard_platform::shell::console_command;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use super::usage::USAGE_VENDOR;
use super::CLAUDE_PROGRAM;
use crate::usage::{
    display_text, finish_after_answer, has_word, resets_at_from_epoch_secs, used_pct_from_percent,
    ProbeChild, ProbeCommand, ProbeEnv, ProbeError, ProbeFailure, ScopedWindowObs, ScratchDir,
    UpstreamText, UsageDetail, UsageObservation, UsagePolicy, UsageProbe, UsageSource,
    UsageVendorKey, WindowObs,
};

/// 자동 조회 사이의 최소 간격(사용자 결정 2026-09-27 — Codex 와 같은 값).
const CLAUDE_COOLDOWN: Duration = Duration::from_secs(15 * 60);

/// 조회 하나의 시한. 실측 1.2–1.4초(claude 2.1.280 · 탐침) — 느린 기동·네트워크를 넉넉히 덮는 멈춤 방지다.
const CLAUDE_PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// 에이전트 스폰의 stream-json 모드와 **같아야 하는** 핵심 인자 — 그 모드에서 CLI 가 이 조합을 요구한다
/// (`--verbose` 가 빠지면 즉사 — `super::ClaudeBackend::build_spec` 의 그 줄 주석). 스폰 인자에서 파생하지 않고
/// 따로 적는다 — 스폰 쪽은 권한·세션·제어 채널 인자가 섞여 있어, 파생하면 그 변경이 조회로 새어 든다.
/// 두 벌이 어긋나는 것은 시험이 잡는다.
const STREAM_JSON_CORE: [&str; 7] = [
    "-p",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--replay-user-messages",
    "--verbose",
];

const SETTINGS_FILE: &str = "settings.json";
const MCP_CONFIG_FILE: &str = "mcp.json";
/// 훅을 전부 끈다 — 사용자·프로젝트 설정의 훅이 조회 프로세스에서 돌지 않게.
const SETTINGS_JSON: &str = r#"{"disableAllHooks":true}"#;
/// MCP 서버 없음 — `--strict-mcp-config` 와 짝이라 다른 MCP 설정은 읽지 않는다.
const MCP_CONFIG_JSON: &str = r#"{"mcpServers":{}}"#;

/// 데몬 env 에서 `CLAUDE` 로 시작하는 키를 벗기되 남기는 것 — 기본 로그인을 가리키는 둘이다. 이것까지 벗기면
/// 데몬의 기본 계정이 아닌 로그인으로 조회할 수 있다(TRD §3 #9).
const KEPT_CLAUDE_ENV: [&str; 2] = ["CLAUDE_CONFIG_DIR", "CLAUDE_CODE_OAUTH_TOKEN"];
const STRIPPED_ENV_PREFIX: &str = "CLAUDE";

/// 조회 프로세스에 더하는 env — 탐침(`get-usage-probe.mjs`)이 실측한 조합 그대로다(`MAX_THINKING_TOKENS` 는
/// 에이전트 스폰의 stream-json 모드와 같은 값). ★각 키가 무엇을 끄는지는 이름 이상으로 확인하지 않았다★.
const PROBE_ENV: [(&str, &str); 4] = [
    ("MAX_THINKING_TOKENS", "8000"),
    ("ENABLE_CLAUDEAI_MCP_SERVERS", "false"),
    ("CLAUDE_CODE_AUTO_CONNECT_IDE", "0"),
    ("CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL", "1"),
];

/// 외부 문자열(plan·모델 이름) 칸의 글자 수 상한 — 표시용 이름이라 이보다 길면 잘라 싣는다.
const PLAN_MAX_CHARS: usize = 32;
const LABEL_MAX_CHARS: usize = 48;
/// 모델별 창 목록의 상한. 오늘 응답은 한두 개다.
const MODEL_SCOPED_MAX: usize = 16;

/// 이름 붙은 모델별 주간 창 칸(SDK 타입의 고정 칸)과 붙이는 표시 이름. `model_scoped[]` 항목과 달리 응답이 이름을
/// 주지 않아 우리가 붙인다.
const NAMED_MODEL_WINDOWS: [(&str, &str); 3] = [
    ("seven_day_opus", "Opus"),
    ("seven_day_sonnet", "Sonnet"),
    ("seven_day_oauth_apps", "OAuth apps"),
];

/// 응답 분류 낱말([`UsageDetail::kind`]) — wire 로 나가고 화면이 번역 없이 보인다.
const KIND_RATE_LIMITS_NULL: &str = "rate_limits_null";
const KIND_CLAUDE_ERROR: &str = "claude_error";
const KIND_LIMITS_UNAVAILABLE: &str = "limits_unavailable";

/// claude 사용량 조회기. 부르는 쪽은 싱글턴 [`CLAUDE_USAGE_PROBE`] 를 쓴다.
pub(crate) struct ClaudeUsageProbe;

pub(crate) static CLAUDE_USAGE_PROBE: ClaudeUsageProbe = ClaudeUsageProbe;

impl UsageProbe for ClaudeUsageProbe {
    fn key(&self) -> UsageVendorKey {
        USAGE_VENDOR
    }

    fn policy(&self) -> UsagePolicy {
        UsagePolicy {
            cooldown: CLAUDE_COOLDOWN,
            timeout: CLAUDE_PROBE_TIMEOUT,
        }
    }

    fn query(&self, env: &ProbeEnv<'_>) -> Result<UsageObservation, ProbeFailure> {
        query_with(env, std::env::vars_os().map(|(key, _)| key))
    }
}

/// `daemon_env_keys` = 데몬 프로세스 env 의 키 전부(운영 = 프로세스 env · 시험 = 대역).
fn query_with(
    env: &ProbeEnv<'_>,
    daemon_env_keys: impl IntoIterator<Item = OsString>,
) -> Result<UsageObservation, ProbeFailure> {
    env.require_time_left()?;
    // ★선언 순서가 drop 순서를 정한다 — 폴더가 자식보다 먼저다★: 자식이 먼저 drop(트리 kill + 종료 대기)돼야
    //   Windows 가 그 작업 폴더와 설정 파일을 지울 수 있다.
    let scratch = ScratchDir::create(env.scratch_root)?;
    let command = probe_command(scratch.path(), daemon_env_keys)?;
    let request_id = Uuid::new_v4().to_string();
    let mut child = env.spawner.spawn(&command, env.deadline)?;
    // 감싼 셸이 대상을 못 찾고 벌써 끝났으면 쓰기가 먼저 실패한다 — 곧바로 돌아가면 「설치 안 됨」을 싣는 종료
    //   상태를 못 본다. 오류를 들고 EOF 까지 읽은 뒤 종료로 가른다.
    let write_error = match child.write_line(&request_line(&request_id), env.deadline) {
        Ok(()) => None,
        Err(err @ ProbeError::Io(_)) => Some(err),
        Err(err) => return Err(err.into()),
    };
    let response = await_response(child.as_mut(), &request_id, env.deadline, write_error)?;
    // ★곧바로 죽이지 않는다★ — stdin 을 닫으면 claude 는 스스로 끝나지만 1초쯤 걸리고, 그 사이 전역 설정 파일
    //   (`.claude.json`)을 쓴다(실측 2026-09-27 · 2.1.280 · 로그인 조회 2회 = 닫고 끝나기까지 1060·862 ms, 종료
    //   코드 0, 두 번 다 그 사이 수정 시각이 바뀌었다). 답 직후 kill 하면 설정 폴더에 `.claude.json.lock`·
    //   `.claude.json.tmp.<pid>.*` 가 남았다(같은 날 · 로그아웃 조회). 유예가 그 1초를 덮는다 — 넘기면 끊는다.
    finish_after_answer(child, env.deadline);
    observation_from_response(&response)
}

/// 설정 파일 둘을 `scratch` 에 쓰고 그 폴더에서 뜨는 조회 명령을 만든다.
///
/// 두 설정을 인라인 JSON 이 아니라 파일 경로로 넘기는 이유 = Windows 는 `cmd.exe /c` 를 한 겹 거치고, 그 인용
/// 규칙이 따옴표·중괄호 덩어리를 조용히 깨뜨린다(에이전트 스폰의 `--settings` 주석과 같은 사정).
fn probe_command(
    scratch: &Path,
    daemon_env_keys: impl IntoIterator<Item = OsString>,
) -> Result<ProbeCommand, ProbeError> {
    let settings = scratch.join(SETTINGS_FILE);
    let mcp_config = scratch.join(MCP_CONFIG_FILE);
    write_config(&settings, SETTINGS_JSON)?;
    write_config(&mcp_config, MCP_CONFIG_JSON)?;

    let mut args: Vec<String> = STREAM_JSON_CORE.iter().map(|a| (*a).to_owned()).collect();
    args.push("--no-session-persistence".to_owned());
    args.push("--settings".to_owned());
    args.push(path_arg(&settings)?);
    args.push("--strict-mcp-config".to_owned());
    // ★`--mcp-config` 는 맨 끝에 둔다★ — 그 플래그는 값을 여럿 받아(2.1.280 `--help`: `<configs...>`) 뒤따르는
    //   맨 값까지 설정으로 흡수한다.
    args.push("--mcp-config".to_owned());
    args.push(path_arg(&mcp_config)?);
    // `--permission-mode bypassPermissions` 는 일부러 없다 — 모델 턴이 없어 권한을 물을 일이 없다.
    let (program, args) = console_command(CLAUDE_PROGRAM, args);

    Ok(ProbeCommand {
        program,
        args,
        cwd: scratch.to_path_buf(),
        env_set: PROBE_ENV
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
        env_remove: stripped_env_keys(daemon_env_keys),
    })
}

/// 데몬 env 에서 벗길 키 — `CLAUDE` 로 시작하는(ASCII 대소문자 무시) 키 중 [`KEPT_CLAUDE_ENV`] 가 아닌 것.
/// ★UTF-8 이 아닌 키는 벗기지 못한다★ — [`ProbeCommand::env_remove`] 가 문자열이라 그 키를 가리킬 수 없다.
fn stripped_env_keys(daemon_env_keys: impl IntoIterator<Item = OsString>) -> Vec<String> {
    daemon_env_keys
        .into_iter()
        .filter_map(|key| key.into_string().ok())
        .filter(|key| {
            key.get(..STRIPPED_ENV_PREFIX.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(STRIPPED_ENV_PREFIX))
        })
        .filter(|key| {
            !KEPT_CLAUDE_ENV
                .iter()
                .any(|kept| key.eq_ignore_ascii_case(kept))
        })
        .collect()
}

fn write_config(path: &Path, content: &str) -> Result<(), ProbeError> {
    fs::write(path, content)
        .map_err(|e| ProbeError::Io(format!("조회 설정 파일을 쓰지 못했다: {e}")))
}

/// 유니코드가 아닌 경로는 거절한다 — 손실 변환한 경로는 다른(없는) 파일을 가리킨다.
fn path_arg(path: &Path) -> Result<String, ProbeError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| ProbeError::Io("조회 임시 폴더 경로가 유니코드가 아니다".to_owned()))
}

/// `skip_behaviors` = 플랜 한도만 필요한 호출자용 — 7일치 사용 기록 훑기를 건너뛴다(SDK 타입 주석).
fn request_line(request_id: &str) -> String {
    json!({
        "type": "control_request",
        "request_id": request_id,
        "request": { "subtype": "get_usage", "skip_behaviors": true },
    })
    .to_string()
}

/// 우리 요청의 응답 줄이 올 때까지 읽는다 — 그 밖의 줄(초기화 알림·다른 요청의 응답·JSON 아닌 줄)은 버린다.
/// `write_error` = 요청 쓰기가 실패한 입출력 오류 — 응답 없이 끝나고 「설치 안 됨」도 아니면 이것을 돌려준다.
fn await_response(
    child: &mut dyn ProbeChild,
    request_id: &str,
    deadline: Instant,
    write_error: Option<ProbeError>,
) -> Result<Value, ProbeError> {
    let mut read_error = write_error;
    loop {
        let line = match child.read_line(deadline) {
            Ok(Some(line)) => line,
            Ok(None) => return Err(ended_without_response(child, deadline, read_error)),
            // 상한에 걸린 한 줄이거나 읽기 실패다 — 앞의 경우 스트림은 다음 줄로 이어지고, 뒤의 경우 곧 EOF 가 온다.
            Err(err @ ProbeError::Io(_)) => {
                read_error = Some(err);
                continue;
            }
            Err(err) => return Err(err),
        };
        match serde_json::from_str::<Value>(&line) {
            Ok(value) if is_response_to(&value, request_id) => return Ok(value),
            Ok(_) => {}
            // 우리 응답인데 JSON 이 아니면(비유한 수 리터럴 등) 기다려도 다른 응답은 안 온다 — 시한까지 끌지 않는다.
            // 요청 id 는 우리가 뽑은 uuid 라 다른 줄에 우연히 실릴 수 없다.
            Err(_) if line.contains(request_id) && line.contains("control_response") => {
                return Err(ProbeError::Parse(
                    "claude 사용량 응답 줄이 JSON 이 아니다".to_owned(),
                ));
            }
            Err(_) => {}
        }
    }
}

/// 응답 줄 = `{"type":"control_response","response":{"subtype":…,"request_id":…}}`(탐침 실측). 요청 id 는
/// `response` 안에 있다 — 봉투 최상위에 실려 와도 받는다.
fn is_response_to(line: &Value, request_id: &str) -> bool {
    let id_matches = |holder: Option<&Value>| {
        holder
            .and_then(|h| h.get("request_id"))
            .and_then(Value::as_str)
            == Some(request_id)
    };
    line.get("type").and_then(Value::as_str) == Some("control_response")
        && (id_matches(line.get("response")) || id_matches(Some(line)))
}

/// 응답 없이 stdout 이 닫혔다 — 종료 상태로 「설치 안 됨」을 가른다.
fn ended_without_response(
    child: &mut dyn ProbeChild,
    deadline: Instant,
    read_error: Option<ProbeError>,
) -> ProbeError {
    let exit = match child.wait_exit(deadline) {
        Ok(exit) => exit,
        Err(err) => return err,
    };
    if exit.looks_not_installed() {
        return ProbeError::NotInstalled;
    }
    // stderr 내용은 싣지 않는다 — 가린 것은 토큰 모양뿐이고 계정·경로가 남을 수 있다. 줄 수만 적는다.
    tracing::debug!(
        code = ?exit.code(),
        stderr_lines = child.stderr_tail().len(),
        "claude 사용량 조회: 응답 없이 끝났다"
    );
    if let Some(err) = read_error {
        return err;
    }
    ProbeError::Upstream(match exit.code() {
        Some(code) => format!("claude 가 사용량 응답 없이 끝났다(종료 코드 {code})"),
        None => "claude 가 사용량 응답 없이 끝났다(종료 코드 없음)".to_owned(),
    })
}

/// 우리 요청의 응답 줄 → 관측.
///
/// - `response.subtype == "error"` → [`classify_error`] · 그 밖의 모르는 subtype → `Parse`.
/// - 성공이면 `response.response` 가 사용량이다: `rate_limits` · 그 안의 `five_hour`/`seven_day`
///   (`{utilization 0–100, resets_at ISO 8601}`) · 모델별 주간 창 · `subscription_type` → plan.
/// - 두 칸의 뜻은 CLI 2.1.280 의 응답 스키마 설명이 가른다: `rate_limits_available` = 「plan 한도가 적용되지 않으면
///   `false`(API 키·Bedrock·Vertex·profile 권한 없는 토큰)」, `rate_limits` = 「CLI 가 받아 오지 못하면 `null`」.
///   그래서 `null` 하나만으로는 「조회 실패」고, 명시적 `false` 만이 「이 계정엔 한도가 없다」다.
/// - `rate_limits_available` 이 bool `false` → 「한도 정보 없음」(`limits_unavailable` — 근거 =
///   [`limits_unavailable_detail`]). 창은 전부 `None` 이고 `rate_limits` 는 모양이 무엇이든(빠져도) 읽지 않는다 —
///   plan 은 그대로 읽는다.
/// - 그 밖(`true`·칸 없음·bool 아님 — bool 이 아니면 없는 것으로 친다)에서 `rate_limits` 가 명시적 `null` 이면
///   `Upstream`(= 조회 실패)이다. 받는 쪽이 들고 있던 값을 유지한다. ★CLI 쪽 조회가 429 를 받아도 오류 subtype 이
///   아니라 이 `null` 로 온다(CLI 코드 정독)★ — 그래서 `RateLimited` 로 가르지 못하고 `Upstream` 으로 접는다.
///   [`classify_error`] 의 한도 문구 표는 다른 버전이 오류 subtype 으로 답할 때를 위해 남긴다. ★상류 오류 문구가 없는
///   실패라 원문 = 받은 두 칸 그대로다★([`KIND_RATE_LIMITS_NULL`] — `rate_limits_available` 칸이 없으면 그 칸은 뺀다).
/// - ★그 밖에 `rate_limits` 칸이 빠졌거나 객체도 `null` 도 아니면 `Parse` 다★ — 그것을 「창 없음」으로 읽으면 모양이
///   바뀐 응답이 성공으로 들어가 들고 있던 값을 전부 지운다. 실패면 받는 쪽이 값을 유지한다. 근거는 분류뿐이다.
/// - `None` 칸 = 안 실렸다(받는 쪽이 들고 있던 값을 유지한다). 두 칸이 다 안 읽히는 창도 `None` 으로 접는다.
///   ★모델별 창은 `rate_limits` 객체를 읽었으면 비어도 `Some` 이다★ — `None` 으로 접으면 사라진 모델 창이 받는
///   쪽에 영영 남는다.
fn observation_from_response(line: &Value) -> Result<UsageObservation, ProbeFailure> {
    let response = line
        .get("response")
        .and_then(Value::as_object)
        .ok_or_else(|| parse_error("응답 봉투에 response 객체가 없다"))?;
    match response.get("subtype").and_then(Value::as_str) {
        Some("success") => {}
        Some("error") => return Err(classify_error(response.get("error"))),
        _ => return Err(parse_error("응답의 subtype 을 모른다").into()),
    }
    let usage = response
        .get("response")
        .and_then(Value::as_object)
        .ok_or_else(|| parse_error("성공 응답에 사용량 객체가 없다"))?;
    let flag = usage.get("rate_limits_available");
    let limits_unavailable =
        (flag.and_then(Value::as_bool) == Some(false)).then(|| limits_unavailable_detail(usage));
    let (five_hour, weekly, model_scoped) = if limits_unavailable.is_some() {
        // `false` 인데 `rate_limits` 에 값이 실려 와도 읽지 않는다 — 「정보 없음」 관측이 창을 나르면 모순이다.
        (None, None, None)
    } else {
        match usage.get("rate_limits") {
            Some(Value::Null) => {
                let received = match flag {
                    Some(flag) => format!("rate_limits_available: {flag}, rate_limits: null"),
                    None => "rate_limits: null".to_owned(),
                };
                return Err(ProbeFailure::with_detail(
                    ProbeError::Upstream(
                        "claude 가 사용량을 받아 오지 못했다(rate_limits 가 null)".to_owned(),
                    ),
                    UsageDetail {
                        kind: KIND_RATE_LIMITS_NULL,
                        code: None,
                        upstream: Some(UpstreamText::new(&received)),
                    },
                ));
            }
            Some(Value::Object(limits)) => (
                active_window(limits.get("five_hour")),
                active_window(limits.get("seven_day")),
                Some(model_scoped_windows(limits)),
            ),
            Some(_) => return Err(parse_error("rate_limits 가 객체도 null 도 아니다").into()),
            None => return Err(parse_error("사용량 객체에 rate_limits 칸이 없다").into()),
        }
    };
    let plan = usage
        .get("subscription_type")
        .and_then(Value::as_str)
        .and_then(|plan| display_text(plan, PLAN_MAX_CHARS));
    Ok(UsageObservation {
        vendor: USAGE_VENDOR,
        five_hour,
        weekly,
        model_scoped,
        plan,
        source: UsageSource::Active,
        limits_unavailable,
    })
}

fn parse_error(what: &str) -> ProbeError {
    ProbeError::Parse(format!("claude 사용량 응답: {what}"))
}

/// 「한도 정보 없음」의 근거 — 원문 = 판정한 칸과 `subscription_type` 을 받은 그대로(`null` 포함 · 칸이 없으면 뺀다).
/// plan 칸은 원인을 가르는 단서라 함께 싣는다(로그아웃한 CLI 는 `null` 로 답한다 — 실측).
fn limits_unavailable_detail(usage: &Map<String, Value>) -> UsageDetail {
    let received = match usage.get("subscription_type") {
        Some(plan) => format!("rate_limits_available: false, subscription_type: {plan}"),
        None => "rate_limits_available: false".to_owned(),
    };
    UsageDetail {
        kind: KIND_LIMITS_UNAVAILABLE,
        code: None,
        upstream: Some(UpstreamText::new(&received)),
    }
}

/// 창 객체 하나 — `utilization` 은 이미 0–100, `resets_at` 은 ISO 8601(RFC 3339) 문자열. 틀린 칸은 그 칸만 버린다.
fn active_window(value: Option<&Value>) -> Option<WindowObs> {
    let fields = value?.as_object()?;
    let window = WindowObs {
        used_pct: fields
            .get("utilization")
            .and_then(Value::as_f64)
            .and_then(used_pct_from_percent),
        resets_at: fields
            .get("resets_at")
            .and_then(Value::as_str)
            .and_then(epoch_secs_from_iso),
    };
    (window.used_pct.is_some() || window.resets_at.is_some()).then_some(window)
}

/// 시간대 표기가 없는 시각은 받지 않는다 — 어느 시간대인지 모르는 값을 epoch 로 옮기면 틀린 리셋을 그린다.
fn epoch_secs_from_iso(text: &str) -> Option<i64> {
    let at = DateTime::parse_from_rfc3339(text).ok()?;
    resets_at_from_epoch_secs(at.timestamp())
}

/// 모델별 주간 창 — `model_scoped[]`(`{display_name, utilization, resets_at}`) 뒤에 값이 있는 이름 붙은 칸
/// ([`NAMED_MODEL_WINDOWS`]). 같은 이름(ASCII 대소문자 무시)은 먼저 온 것만 남긴다.
/// ★`model_scoped` 항목 모양은 SDK 타입에 없다★ — CLI 가 타입보다 먼저 싣는 칸이라, 이름이 문자열인 항목만 받는다.
fn model_scoped_windows(limits: &Map<String, Value>) -> Vec<ScopedWindowObs> {
    let listed = limits
        .get("model_scoped")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let label = entry
                .get("display_name")
                .and_then(Value::as_str)
                .and_then(|name| display_text(name, LABEL_MAX_CHARS))?;
            Some((label, active_window(Some(entry))?))
        });
    let named = NAMED_MODEL_WINDOWS
        .iter()
        .filter_map(|(key, label)| Some(((*label).to_owned(), active_window(limits.get(*key))?)));

    let mut windows: Vec<ScopedWindowObs> = Vec::new();
    for (label, window) in listed.chain(named) {
        if windows.len() >= MODEL_SCOPED_MAX {
            break;
        }
        if windows.iter().any(|w| w.label.eq_ignore_ascii_case(&label)) {
            continue;
        }
        windows.push(ScopedWindowObs { label, window });
    }
    windows
}

/// 오류 응답 → 분류 + 근거([`KIND_CLAUDE_ERROR`] · 원문 = `error` 칸 값). `error` 가 문자열이 아니면 JSON 표기를 그대로
/// 문구로 본다 — 분류와 원문 모두. `error` 가 없거나 `null` 이거나 공백뿐이면 원문도 없다.
fn classify_error(error: Option<&Value>) -> ProbeFailure {
    let text = match error {
        Some(Value::String(text)) => Some(text.clone()),
        None | Some(Value::Null) => None,
        Some(other) => Some(other.to_string()),
    };
    let classified = classify_error_text(&text.as_deref().unwrap_or_default().to_ascii_lowercase());
    tracing::debug!(kind = %classified, "claude 사용량 조회: 오류 응답");
    ProbeFailure::with_detail(
        classified,
        UsageDetail {
            kind: KIND_CLAUDE_ERROR,
            code: None,
            upstream: text.as_deref().and_then(UpstreamText::non_blank),
        },
    )
}

/// ★이 문구 표는 추정이다★ — 한도·로그아웃 오류의 실제 문구를 아직 못 봤다(TRD §6 #4). 틀려도 떨어지는 곳은
///   `Upstream`(= 조회 실패)이다. ★모르는 오류를 한도로 읽지 않는 것은 의도다★ — 「거절됨」으로 보이면 사용자를
///   속이고 새로고침을 막는다. `Upstream` 문구는 고정이다 — 상류 문구를 되풀이하지 않는다.
/// `text` = 소문자로 접은 오류 문구.
fn classify_error_text(text: &str) -> ProbeError {
    const UNSUPPORTED: &str = "not supported in this context";
    const RATE_LIMITED: [&str; 5] = [
        "rate limit",
        "rate_limit",
        "rate-limit",
        "ratelimit",
        "too many requests",
    ];
    // ★맨 `oauth` 를 되살리지 말 것★ — 조회 끝점 경로(`/api/oauth/usage`)를 싣는 5xx 가 로그인 거절로 읽힌다.
    const UNAUTHENTICATED: [&str; 10] = [
        "not logged in",
        "logged out",
        "log in",
        "login",
        "unauthorized",
        "unauthenticated",
        "authenticat",
        "oauth token",
        "oauth_token",
        "invalid api key",
    ];
    if text.contains(UNSUPPORTED) {
        return ProbeError::Unsupported;
    }
    if RATE_LIMITED.iter().any(|w| has_word(text, w, false)) || has_word(text, "429", true) {
        return ProbeError::RateLimited { retry_after: None };
    }
    if UNAUTHENTICATED.iter().any(|w| has_word(text, w, false)) || has_word(text, "401", true) {
        return ProbeError::Unauthenticated;
    }
    ProbeError::Upstream("claude 가 사용량 조회를 분류 안 되는 오류로 거절했다".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use crate::backend::claude::ClaudeBackend;
    use crate::backend::AgentBackend;
    use crate::profile::{AgentCommand, AgentOutputFormat, SpawnMode};
    use crate::usage::testing::{ChildScript, ScriptedSpawner, TempRoot};
    use crate::usage::{ExitInfo, ProbeSpawner};

    const SHORT: Duration = Duration::from_millis(300);
    const ROOMY: Duration = Duration::from_secs(4);

    /// 2026-09-27T05:00:00Z · 2026-10-01T09:00:00Z.
    const FIVE_HOUR_RESET: i64 = 1_790_485_200;
    const WEEKLY_RESET: i64 = 1_790_845_200;

    /// 대역 데몬 env 의 키 — 벗길 것 · 남길 것 · 무관한 것이 섞였다.
    const DAEMON_KEYS: [&str; 10] = [
        "CLAUDECODE",
        "CLAUDE_CODE_ENTRYPOINT",
        "claude_lower_case",
        "CLAUDE_CONFIG_DIR",
        "Claude_Code_OAuth_Token",
        "CLAUDE_CODE_AUTO_CONNECT_IDE",
        "PATH",
        "ANTHROPIC_API_KEY",
        "XCLAUDE",
        "CLAUD",
    ];

    fn daemon_keys() -> Vec<OsString> {
        DAEMON_KEYS.iter().map(OsString::from).collect()
    }

    fn run_with_deadline(
        spawner: &ScriptedSpawner,
        root: &TempRoot,
        within: Duration,
    ) -> Result<UsageObservation, ProbeFailure> {
        let env = ProbeEnv {
            spawner,
            deadline: Instant::now() + within,
            scratch_root: root.path(),
        };
        query_with(&env, daemon_keys())
    }

    fn run(script: ChildScript) -> (Result<UsageObservation, ProbeFailure>, ScriptedSpawner) {
        let root = TempRoot::new("claude-probe");
        let spawner = ScriptedSpawner::new(script);
        let result = run_with_deadline(&spawner, &root, ROOMY);
        assert_scratch_gone(&root);
        (result, spawner)
    }

    fn assert_scratch_gone(root: &TempRoot) {
        let left: Vec<_> = fs::read_dir(root.path())
            .expect("시험 루트")
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert!(left.is_empty(), "조회 뒤 임시 폴더가 남았다: {left:?}");
    }

    fn request_id_of(written: &[String]) -> String {
        let first = written.first().expect("조회기가 요청 줄을 먼저 쓴다");
        let line: Value = serde_json::from_str(first).expect("요청 줄은 JSON 이다");
        line["request_id"]
            .as_str()
            .expect("요청 id 는 문자열이다")
            .to_owned()
    }

    fn success_line(written: &[String], usage: Value) -> String {
        json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": request_id_of(written), "response": usage },
        })
        .to_string()
    }

    fn error_line(written: &[String], error: Value) -> String {
        json!({
            "type": "control_response",
            "response": { "subtype": "error", "request_id": request_id_of(written), "error": error },
        })
        .to_string()
    }

    fn answering(usage: Value) -> ChildScript {
        ChildScript::new().reply(move |written| success_line(written, usage))
    }

    /// 탐침이 본 모양(수치는 가짜) — 두 창 · plan · 키만 있고 값이 null 인 모델 창 · 타입에 없는 칸.
    fn typical_usage() -> Value {
        json!({
            "subscription_type": "max",
            "rate_limits_available": true,
            "rate_limits": {
                "five_hour": { "utilization": 42.5, "resets_at": "2026-09-27T05:00:00.123456+00:00" },
                "seven_day": { "utilization": 13, "resets_at": "2026-10-01T09:00:00Z" },
                "seven_day_opus": null,
                "seven_day_sonnet": null,
                "seven_day_oauth_apps": null,
                "model_scoped": [],
                "extra_usage": { "is_enabled": false, "monthly_limit": null },
            },
            "session": { "cost": 0 },
        })
    }

    fn window(used_pct: Option<f64>, resets_at: Option<i64>) -> Option<WindowObs> {
        Some(WindowObs {
            used_pct,
            resets_at,
        })
    }

    fn observe(usage: Value) -> Result<UsageObservation, ProbeFailure> {
        observation_from_response(&json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": "r", "response": usage },
        }))
    }

    fn rate_limits(limits: Value) -> Result<UsageObservation, ProbeFailure> {
        observe(json!({ "rate_limits": limits }))
    }

    /// 근거가 분류 하나뿐인 `Parse` 인가.
    fn is_bare_parse(result: &Result<UsageObservation, ProbeFailure>) -> bool {
        matches!(
            result,
            Err(ProbeFailure {
                error: ProbeError::Parse(_),
                detail: None,
            })
        )
    }

    fn upstream_of(detail: &UsageDetail) -> Option<&str> {
        detail.upstream.as_ref().map(UpstreamText::as_str)
    }

    // ── 정책 ──

    #[test]
    fn policy_and_key_are_the_claude_values() {
        assert_eq!(CLAUDE_USAGE_PROBE.key(), USAGE_VENDOR);
        assert_eq!(CLAUDE_USAGE_PROBE.key().as_str(), "claude");
        assert_eq!(
            CLAUDE_USAGE_PROBE.policy(),
            UsagePolicy {
                cooldown: Duration::from_secs(900),
                timeout: Duration::from_secs(15),
            }
        );
    }

    // ── 해석 ──

    #[test]
    fn success_carries_both_windows_and_the_plan() {
        let (result, spawner) = run(answering(typical_usage()));
        let obs = result.expect("조회");
        assert_eq!(obs.vendor, USAGE_VENDOR);
        assert_eq!(obs.source, UsageSource::Active);
        assert_eq!(obs.five_hour, window(Some(42.5), Some(FIVE_HOUR_RESET)));
        assert_eq!(obs.weekly, window(Some(13.0), Some(WEEKLY_RESET)));
        assert_eq!(
            obs.model_scoped,
            Some(vec![]),
            "null 모델 창·빈 목록 = 모델별 창이 비었다(안 실림이 아니다)"
        );
        assert_eq!(obs.plan.as_deref(), Some("max"));
        assert!(spawner.stdin_closed(), "응답을 받고 stdin 을 닫지 않았다");
        assert!(spawner.child_dropped(), "응답을 받고 자식을 죽이지 않았다");
        assert!(!spawner.cut_at_deadline());
    }

    /// 응답 뒤에 stdin 을 닫아도 안 끝나는 자식 — 유예 뒤에 끊기지만 조회는 성공이다.
    #[test]
    fn a_child_that_lingers_after_the_answer_is_cut_but_the_query_succeeds() {
        let (result, spawner) = run(answering(typical_usage()).hang_after_lines());
        assert_eq!(result.expect("조회").plan.as_deref(), Some("max"));
        assert!(spawner.stdin_closed());
        assert!(spawner.cut_at_deadline(), "유예 뒤에 끊었어야 한다");
        assert!(spawner.child_dropped());
    }

    /// 마감이 이미 지났으면 임시 폴더도 자식도 만들지 않는다.
    #[test]
    fn a_passed_deadline_times_out_before_anything_is_created() {
        let root = TempRoot::new("claude-probe-late");
        let spawner = ScriptedSpawner::new(answering(typical_usage()));
        assert_eq!(
            run_with_deadline(&spawner, &root, Duration::ZERO),
            Err(ProbeError::Timeout.into())
        );
        assert!(spawner.commands().is_empty(), "기동을 시도했다");
        assert_scratch_gone(&root);
    }

    fn assert_no_windows(obs: &UsageObservation) {
        assert_eq!(obs.five_hour, None);
        assert_eq!(obs.weekly, None);
        assert_eq!(obs.model_scoped, None);
        assert_eq!(obs.source, UsageSource::Active);
    }

    /// `rate_limits: null` 실패 — 근거 = [`KIND_RATE_LIMITS_NULL`] + 받은 두 칸 그대로(`upstream`).
    fn assert_fetch_failed(
        result: Result<UsageObservation, ProbeFailure>,
        upstream: &str,
        why: &str,
    ) {
        match result {
            Err(ProbeFailure {
                error: ProbeError::Upstream(_),
                detail: Some(detail),
            }) => {
                assert_eq!(detail.kind, KIND_RATE_LIMITS_NULL, "{why}");
                assert_eq!(detail.code, None, "{why}");
                assert_eq!(upstream_of(&detail), Some(upstream), "{why}");
            }
            other => panic!("{why}: {other:?}"),
        }
    }

    /// 「한도 정보 없음」 — 근거 = [`KIND_LIMITS_UNAVAILABLE`] + 판정한 칸과 plan 칸을 받은 그대로(`upstream`).
    fn assert_unavailable(obs: &UsageObservation, upstream: &str) {
        let detail = obs.limits_unavailable.as_ref().expect("한도 정보 없음");
        assert_eq!(detail.kind, KIND_LIMITS_UNAVAILABLE);
        assert_eq!(detail.code, None);
        assert_eq!(upstream_of(detail), Some(upstream));
        assert_no_windows(obs);
    }

    /// 로그아웃한 CLI 가 답하는 모양(실측).
    #[test]
    fn logged_out_shape_is_limits_unavailable_without_a_plan() {
        let obs = observe(json!({
            "subscription_type": null,
            "rate_limits_available": false,
            "rate_limits": null,
        }))
        .expect("관측");
        assert_unavailable(
            &obs,
            "rate_limits_available: false, subscription_type: null",
        );
        assert_eq!(obs.plan, None);
    }

    /// profile 권한이 없는 토큰 — plan 은 알지만 한도는 적용되지 않는다.
    #[test]
    fn unavailable_with_a_plan_still_reads_the_plan() {
        let obs = observe(json!({
            "subscription_type": "max",
            "rate_limits_available": false,
            "rate_limits": null,
        }))
        .expect("관측");
        assert_unavailable(
            &obs,
            r#"rate_limits_available: false, subscription_type: "max""#,
        );
        assert_eq!(obs.plan.as_deref(), Some("max"));
    }

    /// 원문 칸도 상한을 넘지 않는다 — 받은 plan 이 아무리 길어도.
    #[test]
    fn a_huge_plan_is_cut_in_the_unavailable_detail() {
        let obs = observe(json!({
            "subscription_type": "가".repeat(10_000),
            "rate_limits_available": false,
        }))
        .expect("관측");
        let detail = obs.limits_unavailable.expect("한도 정보 없음");
        let upstream = upstream_of(&detail).expect("원문");
        assert_eq!(upstream.chars().count(), 200);
        assert!(upstream.starts_with("rate_limits_available: false, subscription_type: \"가"));
    }

    #[test]
    fn unavailable_ignores_windows_carried_in_rate_limits() {
        let mut usage = typical_usage();
        usage["rate_limits_available"] = json!(false);
        let obs = observe(usage).expect("관측");
        assert_unavailable(
            &obs,
            r#"rate_limits_available: false, subscription_type: "max""#,
        );
        assert_eq!(obs.plan.as_deref(), Some("max"));
    }

    #[test]
    fn unavailable_does_not_read_rate_limits_of_any_shape() {
        let without_limits = json!({ "subscription_type": "max", "rate_limits_available": false });
        let obs = observe(without_limits).expect("관측");
        assert_unavailable(
            &obs,
            r#"rate_limits_available: false, subscription_type: "max""#,
        );

        for limits in [json!("none"), json!([]), json!(0)] {
            let obs = observe(json!({ "rate_limits_available": false, "rate_limits": limits }))
                .expect("관측");
            assert_unavailable(&obs, "rate_limits_available: false");
        }
    }

    #[test]
    fn null_rate_limits_without_a_false_flag_is_a_fetch_failure() {
        assert_fetch_failed(
            observe(json!({ "rate_limits_available": true, "rate_limits": null })),
            "rate_limits_available: true, rate_limits: null",
            "true",
        );
        assert_fetch_failed(
            observe(json!({ "subscription_type": "max", "rate_limits": null })),
            "rate_limits: null",
            "칸 없음",
        );
    }

    /// 실패의 로그용 문구(`Display`)에는 근거 낱말만 실린다 — 받은 두 칸은 원문 칸에만 있다.
    #[test]
    fn a_fetch_failure_shows_only_the_kind_in_its_display() {
        let failure = observe(json!({ "rate_limits_available": true, "rate_limits": null }))
            .expect_err("조회 실패");
        let shown = failure.to_string();
        assert!(shown.contains(KIND_RATE_LIMITS_NULL), "{shown}");
        assert!(!shown.contains("rate_limits_available"), "{shown}");
    }

    #[test]
    fn available_with_windows_is_not_unavailable() {
        let obs = observe(typical_usage()).expect("관측");
        assert_eq!(obs.limits_unavailable, None);
        assert_eq!(obs.five_hour, window(Some(42.5), Some(FIVE_HOUR_RESET)));
        assert_eq!(obs.weekly, window(Some(13.0), Some(WEEKLY_RESET)));
    }

    /// bool 이 아닌 `rate_limits_available` 은 칸이 없는 것과 같다 — `rate_limits` 가 판정을 가른다.
    #[test]
    fn a_non_bool_flag_is_read_as_missing() {
        for flag in [
            json!("false"),
            json!(0),
            json!(1),
            json!({}),
            json!([false]),
        ] {
            let mut usage = typical_usage();
            usage["rate_limits_available"] = flag.clone();
            let obs = observe(usage).expect("관측");
            assert_eq!(obs.limits_unavailable, None, "{flag}");
            assert_eq!(
                obs.five_hour,
                window(Some(42.5), Some(FIVE_HOUR_RESET)),
                "{flag}"
            );

            assert_fetch_failed(
                observe(json!({ "rate_limits_available": flag.clone(), "rate_limits": null })),
                &format!("rate_limits_available: {flag}, rate_limits: null"),
                &flag.to_string(),
            );

            for missing_or_bad in [
                json!({ "rate_limits_available": flag.clone() }),
                json!({ "rate_limits_available": flag.clone(), "rate_limits": "none" }),
            ] {
                assert!(is_bare_parse(&observe(missing_or_bad)), "{flag}");
            }
        }
    }

    #[test]
    fn model_scoped_list_then_named_windows_without_duplicates() {
        let obs = rate_limits(json!({
            "five_hour": { "utilization": 1, "resets_at": null },
            "seven_day_opus": { "utilization": 3, "resets_at": null },
            "seven_day_sonnet": { "utilization": 55.25, "resets_at": "2026-10-01T09:00:00Z" },
            "seven_day_oauth_apps": null,
            "model_scoped": [
                { "display_name": "Fable", "utilization": 73, "resets_at": "2026-10-01T09:00:00Z" },
                { "display_name": "Ghost", "utilization": null, "resets_at": null },
                { "display_name": "  opus  ", "utilization": 9, "resets_at": null },
                { "utilization": 50 },
                { "display_name": 7, "utilization": 50 },
                "not an entry",
            ],
        }))
        .expect("관측");
        let scoped = obs.model_scoped.expect("모델별 창");
        let labels: Vec<&str> = scoped.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Fable", "opus", "Sonnet"],
            "중복 Opus 는 목록 쪽이 이긴다"
        );
        assert_eq!(
            scoped[0].window,
            WindowObs {
                used_pct: Some(73.0),
                resets_at: Some(WEEKLY_RESET)
            }
        );
        assert_eq!(
            scoped[1].window,
            WindowObs {
                used_pct: Some(9.0),
                resets_at: None
            }
        );
        assert_eq!(
            scoped[2].window,
            WindowObs {
                used_pct: Some(55.25),
                resets_at: Some(WEEKLY_RESET)
            }
        );
        assert_eq!(obs.five_hour, window(Some(1.0), None));
        assert_eq!(obs.weekly, None, "빠진 창 = 없음");
    }

    #[test]
    fn model_scoped_list_is_capped() {
        let entries: Vec<Value> = (0..40)
            .map(|i| json!({ "display_name": format!("model-{i}"), "utilization": i }))
            .collect();
        let obs = rate_limits(json!({ "model_scoped": entries })).expect("관측");
        assert_eq!(obs.model_scoped.expect("모델별 창").len(), MODEL_SCOPED_MAX);
    }

    #[test]
    fn iso_reset_times_parse_with_offsets_and_bad_ones_drop_only_that_field() {
        let reset = |text: Value| {
            rate_limits(json!({ "five_hour": { "utilization": 20, "resets_at": text } }))
                .expect("관측")
                .five_hour
                .expect("사용률이 남아 창은 있다")
                .resets_at
        };
        assert_eq!(reset(json!("2026-09-27T05:00:00Z")), Some(FIVE_HOUR_RESET));
        assert_eq!(
            reset(json!("2026-09-27T14:00:00+09:00")),
            Some(FIVE_HOUR_RESET)
        );
        assert_eq!(
            reset(json!("2026-09-27T05:00:00.999999999Z")),
            Some(FIVE_HOUR_RESET)
        );
        assert_eq!(reset(json!("9999-12-31T23:59:59Z")), Some(253_402_300_799));
        for bad in [
            json!("2026-09-27T05:00:00"),
            json!("2026-09-27"),
            json!("2026-13-45T00:00:00Z"),
            json!("next tuesday"),
            json!(""),
            json!("1969-12-31T23:59:59Z"),
            json!("1970-01-01T00:00:00Z"),
            json!(1_790_485_200),
            json!(null),
            json!(["2026-09-27T05:00:00Z"]),
        ] {
            assert_eq!(reset(bad.clone()), None, "{bad}");
        }
    }

    #[test]
    fn broken_envelopes_are_parse_errors() {
        let envelope = |response: Value| {
            observation_from_response(&json!({ "type": "control_response", "response": response }))
        };
        let cases = [
            observation_from_response(&json!({ "type": "control_response", "request_id": "r" })),
            observation_from_response(
                &json!({ "type": "control_response", "response": "success" }),
            ),
            envelope(json!({ "request_id": "r" })),
            envelope(json!({ "subtype": "partial", "request_id": "r" })),
            envelope(json!({ "subtype": 1, "request_id": "r" })),
            envelope(json!({ "subtype": "success", "request_id": "r" })),
            envelope(json!({ "subtype": "success", "request_id": "r", "response": [1] })),
            observe(json!({ "rate_limits_available": true })),
            observe(json!({ "subscription_type": "max" })),
            observe(json!({ "rate_limits": "none" })),
            observe(json!({ "rate_limits": [] })),
            observe(json!({ "rate_limits": 0 })),
        ];
        for (i, case) in cases.into_iter().enumerate() {
            assert!(is_bare_parse(&case), "{i}: {case:?}");
        }
    }

    /// 칸 하나가 어떻게 틀려도 패닉하지 않고, 그 칸만 버린다.
    #[test]
    fn hostile_fields_never_panic_and_drop_only_that_field() {
        let window_of = |w: Value| {
            rate_limits(json!({ "five_hour": w }))
                .expect("관측")
                .five_hour
        };
        assert_eq!(
            window_of(json!({ "utilization": -5, "resets_at": null })),
            window(Some(0.0), None)
        );
        assert_eq!(
            window_of(json!({ "utilization": 130 })),
            window(Some(100.0), None)
        );
        assert_eq!(
            window_of(json!({ "utilization": 1e308 })),
            window(Some(100.0), None)
        );
        assert_eq!(
            window_of(json!({ "utilization": -1e308 })),
            window(Some(0.0), None)
        );
        assert_eq!(
            window_of(json!({ "utilization": 18_446_744_073_709_551_615_u64 })),
            window(Some(100.0), None)
        );
        assert_eq!(
            window_of(json!({ "utilization": 0.55 })),
            window(Some(0.55), None),
            "이미 0–100 이다"
        );
        assert_eq!(
            window_of(json!({ "utilization": "42", "resets_at": 5 })),
            None
        );
        assert_eq!(
            window_of(json!({ "utilization": null, "resets_at": null })),
            None
        );
        assert_eq!(window_of(json!({})), None);
        assert_eq!(window_of(json!([42])), None);
        assert_eq!(window_of(json!("42%")), None);
        assert_eq!(window_of(json!(null)), None);

        for limits in [
            json!({ "model_scoped": { "display_name": "Fable", "utilization": 1 } }),
            json!({ "model_scoped": "Fable" }),
            json!({ "model_scoped": [null, 1, [], {}] }),
            json!({ "model_scoped": [{ "display_name": "", "utilization": 1 }] }),
            json!({ "model_scoped": [{ "display_name": "   ", "utilization": 1 }] }),
            json!({ "model_scoped": [{ "display_name": "bad\u{7}name", "utilization": 1 }] }),
            json!({ "seven_day_opus": "3%" , "seven_day_sonnet": [], "seven_day_oauth_apps": 0 }),
        ] {
            let obs = rate_limits(limits.clone()).expect("관측");
            assert_eq!(obs.model_scoped, Some(vec![]), "{limits}");
        }

        let long = "x".repeat(10_000);
        let obs = observe(json!({
            "subscription_type": long,
            "rate_limits": { "model_scoped": [{ "display_name": "가".repeat(500), "utilization": 1 }] },
        }))
        .expect("관측");
        assert_eq!(obs.plan.map(|p| p.chars().count()), Some(PLAN_MAX_CHARS));
        let scoped = obs.model_scoped.expect("모델별 창");
        assert_eq!(
            scoped[0].label.chars().count(),
            LABEL_MAX_CHARS,
            "글자 단위로 자른다"
        );

        for plan in [
            json!(5),
            json!(""),
            json!(null),
            json!("ma\nx"),
            json!({ "tier": "max" }),
        ] {
            let obs = observe(json!({ "subscription_type": plan, "rate_limits_available": false }))
                .expect("관측");
            assert_eq!(obs.plan, None, "{plan}");
        }
        let obs =
            observe(json!({ "subscription_type": " team\n", "rate_limits_available": false }))
                .expect("관측");
        assert_eq!(obs.plan.as_deref(), Some("team"));
    }

    const NON_JSON_RESPONSE: &str = r#"{"type":"control_response","response":{"subtype":"success","request_id":"REQUEST_ID","response":{"rate_limits":{"five_hour":{"utilization":NUMBER}}}}}"#;

    /// JSON 이 아닌 수(비유한 리터럴·범위 밖 지수)가 실린 우리 응답은 시한까지 기다리지 않고 `Parse` 다.
    #[test]
    fn a_non_json_response_line_is_a_parse_error_without_waiting() {
        for number in ["NaN", "Infinity", "1e400"] {
            let script = ChildScript::new()
                .reply(move |written| {
                    NON_JSON_RESPONSE
                        .replace("REQUEST_ID", &request_id_of(written))
                        .replace("NUMBER", number)
                })
                .hang_after_lines();
            let root = TempRoot::new("claude-probe-nan");
            let spawner = ScriptedSpawner::new(script);
            let result = run_with_deadline(&spawner, &root, SHORT);
            assert!(is_bare_parse(&result), "{number}: {result:?}");
            assert!(!spawner.cut_at_deadline(), "{number}: 시한까지 기다렸다");
            assert_scratch_gone(&root);
        }
    }

    // ── 오류 분류 ──

    fn kind(err: &ProbeError) -> &'static str {
        match err {
            ProbeError::RateLimited { retry_after: None } => "rate",
            ProbeError::Unauthenticated => "auth",
            ProbeError::Unsupported => "unsupported",
            ProbeError::Upstream(_) => "upstream",
            other => panic!("분류 밖: {other:?}"),
        }
    }

    #[test]
    fn error_text_classification_table() {
        let cases = [
            ("get_usage is not supported in this context", "unsupported"),
            ("rate limit exceeded", "rate"),
            ("rate_limit_error", "rate"),
            ("request was rate-limited", "rate"),
            ("too many requests", "rate"),
            ("api error: 429 {\"type\":\"error\"}", "rate"),
            ("status=429", "rate"),
            ("not logged in · please run /login", "auth"),
            ("oauth token has expired", "auth"),
            ("invalid claude_code_oauth_token", "auth"),
            ("authentication_error", "auth"),
            ("invalid api key", "auth"),
            ("api error: 401", "auth"),
            // 한도 문구가 로그인 안내와 함께 와도 한도다.
            ("rate limited — log in again later", "rate"),
            ("", "upstream"),
            ("unauthenticated", "auth"),
            ("request req_14290a failed", "upstream"),
            ("internal server error 4010", "upstream"),
            ("not supported", "upstream"),
            // 다른 낱말 속의 같은 조각은 문구가 아니다.
            ("moderate limits apply", "upstream"),
            ("catalog index failed", "upstream"),
            ("bloglogin", "upstream"),
            // 조회 끝점 경로의 `oauth` 는 로그인 문구가 아니다.
            (
                "api error: 500 internal server error (get /api/oauth/usage)",
                "upstream",
            ),
            (
                "502 bad gateway from https://api.example.test/api/oauth/usage",
                "upstream",
            ),
            ("/api/oauth/token returned 503", "upstream"),
        ];
        for (text, expected) in cases {
            assert_eq!(kind(&classify_error_text(text)), expected, "{text:?}");
        }
    }

    /// ★상류 문구는 원문 칸에만 실린다★ — 분류 넷 모두 근거 = [`KIND_CLAUDE_ERROR`] + `error` 칸 값(토큰 모양은 가림)이고,
    ///   실패의 `Display`·`Debug` 에는 안 보인다.
    #[test]
    fn error_responses_carry_the_upstream_text_only_in_the_detail() {
        let secret = "someone@example.com org-1234";
        let token = "sk-ant-abcdefghijklmnopqrstuvwxyz0123";
        for (error, expected, upstream) in [
            (
                json!(format!("Rate limit reached for {secret} {token}")),
                "rate",
                Some(format!("Rate limit reached for {secret} ***")),
            ),
            (
                json!(format!("Not logged in as {secret}")),
                "auth",
                Some(format!("Not logged in as {secret}")),
            ),
            (
                json!("get_usage is not supported in this context"),
                "unsupported",
                Some("get_usage is not supported in this context".to_owned()),
            ),
            (
                json!(format!("boom {secret}")),
                "upstream",
                Some(format!("boom {secret}")),
            ),
            (
                json!({ "message": format!("OAuth token expired for {secret}") }),
                "auth",
                Some(format!(
                    r#"{{"message":"OAuth token expired for {secret}"}}"#
                )),
            ),
            (json!(null), "upstream", None),
            (json!(""), "upstream", None),
            (json!("   "), "upstream", None),
        ] {
            let (result, spawner) =
                run(ChildScript::new().reply(move |written| error_line(written, error)));
            let err = result.expect_err("오류 응답");
            assert_eq!(kind(&err.error), expected);
            let detail = err.detail.as_ref().expect("근거");
            assert_eq!(detail.kind, KIND_CLAUDE_ERROR);
            assert_eq!(detail.code, None);
            assert_eq!(upstream_of(detail), upstream.as_deref());
            let shown = format!("{err} {err:?}");
            assert!(
                !shown.contains("example.com")
                    && !shown.contains("org-1234")
                    && !shown.contains("sk-ant"),
                "{shown}"
            );
            assert!(shown.contains(KIND_CLAUDE_ERROR), "{shown}");
            assert!(spawner.stdin_closed(), "오류 응답도 답이다 — 유예를 준다");
            assert!(spawner.child_dropped());
        }
    }

    // ── 대화 ──

    #[test]
    fn the_request_is_one_get_usage_control_line_with_a_fresh_uuid() {
        let (result, spawner) = run(answering(typical_usage()));
        result.expect("조회");
        let written = spawner.written();
        assert_eq!(
            written.len(),
            1,
            "제어 요청 한 줄만 쓴다(사용자 메시지 없음): {written:?}"
        );
        let line: Value = serde_json::from_str(&written[0]).expect("JSON");
        let id = line["request_id"].as_str().expect("요청 id");
        assert_eq!(Uuid::parse_str(id).expect("uuid").get_version_num(), 4);
        assert_eq!(
            line,
            json!({
                "type": "control_request",
                "request_id": id,
                "request": { "subtype": "get_usage", "skip_behaviors": true },
            })
        );
    }

    #[test]
    fn other_lines_and_other_request_ids_are_ignored() {
        let script = ChildScript::new()
            .line(r#"{"type":"system","subtype":"init"}"#)
            .line("plain text that is not json")
            .line(r#"{"type":"control_response","response":{"subtype":"error","request_id":"someone-else","error":"rate limit"}}"#)
            .line(r#"{"type":"control_request","request_id":"x","request":{"subtype":"can_use_tool"}}"#)
            .line(r#"[1,2,3]"#)
            .reply(|written| {
                // 우리 id 를 싣되 control_response 가 아닌 줄도 버린다.
                json!({ "type": "user", "request_id": request_id_of(written) }).to_string()
            })
            .reply(|written| success_line(written, typical_usage()));
        let (result, _) = run(script);
        assert_eq!(result.expect("조회").plan.as_deref(), Some("max"));
    }

    #[test]
    fn a_top_level_request_id_is_accepted() {
        let script = ChildScript::new().reply(|written| {
            json!({
                "type": "control_response",
                "request_id": request_id_of(written),
                "response": { "subtype": "success", "response": typical_usage() },
            })
            .to_string()
        });
        let (result, _) = run(script);
        assert!(result.is_ok(), "{result:?}");
    }

    /// 읽기 오류 한 번을 먼저 내는 자식 — 실물 스포너의 「상한을 넘은 한 줄」 모양(대본 대역은 오류 줄을 못 낸다).
    struct IoFirstSpawner {
        then_answer: bool,
    }

    struct IoFirstChild {
        request: String,
        io_sent: bool,
        then_answer: bool,
    }

    impl ProbeSpawner for IoFirstSpawner {
        fn spawn(
            &self,
            _cmd: &ProbeCommand,
            _deadline: Instant,
        ) -> Result<Box<dyn ProbeChild>, ProbeError> {
            Ok(Box::new(IoFirstChild {
                request: String::new(),
                io_sent: false,
                then_answer: self.then_answer,
            }))
        }
    }

    impl ProbeChild for IoFirstChild {
        fn write_line(&mut self, line: &str, _deadline: Instant) -> Result<(), ProbeError> {
            self.request = line.to_owned();
            Ok(())
        }

        fn close_stdin(&mut self) {}

        fn read_line(&mut self, _deadline: Instant) -> Result<Option<String>, ProbeError> {
            if !self.io_sent {
                self.io_sent = true;
                return Err(ProbeError::Io("한 줄이 상한을 넘었다".to_owned()));
            }
            if std::mem::take(&mut self.then_answer) {
                let written = std::slice::from_ref(&self.request);
                return Ok(Some(success_line(written, typical_usage())));
            }
            Ok(None)
        }

        fn wait_exit(&mut self, _deadline: Instant) -> Result<ExitInfo, ProbeError> {
            Ok(ExitInfo::new(Some(0), false))
        }

        fn wait_exit_code(&mut self, _deadline: Instant) -> Result<Option<i32>, ProbeError> {
            Ok(Some(0))
        }

        fn stderr_tail(&self) -> Vec<String> {
            Vec::new()
        }
    }

    #[test]
    fn a_read_error_is_skipped_and_reported_only_if_no_response_follows() {
        for (then_answer, expect_ok) in [(true, true), (false, false)] {
            let root = TempRoot::new("claude-probe-io");
            let spawner = IoFirstSpawner { then_answer };
            let env = ProbeEnv {
                spawner: &spawner,
                deadline: Instant::now() + ROOMY,
                scratch_root: root.path(),
            };
            let result = query_with(&env, daemon_keys());
            if expect_ok {
                assert!(result.is_ok(), "{result:?}");
            } else {
                assert!(
                    matches!(
                        result,
                        Err(ProbeFailure {
                            error: ProbeError::Io(_),
                            detail: None
                        })
                    ),
                    "{result:?}"
                );
            }
            assert_scratch_gone(&root);
        }
    }

    #[test]
    fn a_silent_child_times_out_and_is_cut() {
        let root = TempRoot::new("claude-probe-silent");
        let spawner = ScriptedSpawner::new(
            ChildScript::new()
                .line(r#"{"type":"system","subtype":"init"}"#)
                .hang_after_lines(),
        );
        let started = Instant::now();
        let result = run_with_deadline(&spawner, &root, SHORT);
        assert_eq!(result, Err(ProbeError::Timeout.into()));
        assert!(started.elapsed() >= SHORT, "마감 전에 돌아왔다");
        assert!(spawner.cut_at_deadline());
        assert!(spawner.child_dropped());
        assert_scratch_gone(&root);
    }

    #[test]
    fn a_child_that_never_reads_stdin_times_out() {
        let root = TempRoot::new("claude-probe-blocked");
        let spawner = ScriptedSpawner::new(ChildScript::new().block_writes());
        assert_eq!(
            run_with_deadline(&spawner, &root, SHORT),
            Err(ProbeError::Timeout.into())
        );
        assert!(spawner.cut_at_deadline());
        assert!(spawner.written().is_empty());
        assert_scratch_gone(&root);
    }

    #[test]
    fn eof_without_a_response_is_classified_by_the_exit() {
        let (result, spawner) = run(ChildScript::new().exit_not_installed().stderr_tail(&["x"]));
        assert_eq!(result, Err(ProbeError::NotInstalled.into()));
        assert!(spawner.child_dropped());

        let (result, _) = run(ChildScript::new()
            .line(r#"{"type":"system"}"#)
            .exit_code(1)
            .stderr_tail(&["Error: someone@example.com is not allowed"]));
        match result {
            Err(ProbeFailure {
                error: ProbeError::Upstream(text),
                detail: None,
            }) => {
                assert!(text.contains("종료 코드 1"), "{text}");
                assert!(!text.contains("example.com"), "stderr 를 실었다: {text}");
            }
            other => panic!("{other:?}"),
        }

        let root = TempRoot::new("claude-probe-linger");
        let spawner = ScriptedSpawner::new(ChildScript::new().never_exit());
        assert_eq!(
            run_with_deadline(&spawner, &root, SHORT),
            Err(ProbeError::Timeout.into())
        );
        assert!(spawner.cut_at_deadline());
        assert_scratch_gone(&root);
    }

    /// ★요청 쓰기의 실패가 「설치 안 됨」을 가리지 않는다★ — 감싼 셸이 벌써 끝나 쓰기가 실패해도 종료로 가른다.
    ///   설치된 쪽의 실패면 쓰기 오류 그대로다.
    #[test]
    fn a_failed_request_write_still_reads_the_exit() {
        let (result, spawner) = run(ChildScript::new().fail_writes().exit_not_installed());
        assert_eq!(result, Err(ProbeError::NotInstalled.into()));
        assert!(spawner.written().is_empty());

        let (result, _) = run(ChildScript::new().fail_writes().exit_code(1));
        assert!(
            matches!(
                result,
                Err(ProbeFailure {
                    error: ProbeError::Io(_),
                    detail: None
                })
            ),
            "{result:?}"
        );
    }

    /// 스포너 seam 의 실패는 근거 없이 올라온다(`?`) — 받는 쪽이 분류 낱말로 채운다.
    #[test]
    fn a_spawn_failure_passes_through_and_cleans_up() {
        let root = TempRoot::new("claude-probe-missing");
        let spawner = ScriptedSpawner::failing(ProbeError::NotInstalled);
        assert_eq!(
            run_with_deadline(&spawner, &root, ROOMY),
            Err(ProbeFailure {
                error: ProbeError::NotInstalled,
                detail: None,
            })
        );
        assert_eq!(spawner.commands().len(), 1);
        assert_scratch_gone(&root);
    }

    // ── 기동 명세 ──

    #[test]
    fn the_command_runs_in_its_own_scratch_dir_with_hooks_and_mcp_off() {
        let root = TempRoot::new("claude-probe-command");
        let seen_files: Arc<Mutex<Option<(PathBuf, String, String)>>> = Arc::default();
        let seen = seen_files.clone();
        let scan_root = root.path().to_path_buf();
        let spawner = ScriptedSpawner::new(ChildScript::new().reply(move |written| {
            // 자식이 떠 있는 동안 — 설정 파일이 제자리에 있어야 한다.
            let dir = fs::read_dir(&scan_root)
                .expect("루트")
                .flatten()
                .map(|e| e.path())
                .next()
                .expect("임시 폴더");
            let settings = fs::read_to_string(dir.join(SETTINGS_FILE)).expect("설정 파일");
            let mcp = fs::read_to_string(dir.join(MCP_CONFIG_FILE)).expect("MCP 설정 파일");
            *seen.lock().expect("잠금") = Some((dir, settings, mcp));
            success_line(written, typical_usage())
        }));
        run_with_deadline(&spawner, &root, ROOMY).expect("조회");

        let commands = spawner.commands();
        let [cmd] = commands.as_slice() else {
            panic!("자식은 하나: {commands:?}")
        };
        let (dir, settings, mcp) = seen_files
            .lock()
            .expect("잠금")
            .take()
            .expect("대화 중 파일");
        assert_eq!(cmd.cwd, dir, "작업 폴더 = 그 조회의 임시 폴더");
        assert!(cmd.cwd.starts_with(root.path()));
        assert!(!cmd.cwd.exists(), "조회 뒤 임시 폴더가 남았다");
        assert_eq!(
            serde_json::from_str::<Value>(&settings).expect("JSON"),
            json!({ "disableAllHooks": true })
        );
        assert_eq!(
            serde_json::from_str::<Value>(&mcp).expect("JSON"),
            json!({ "mcpServers": {} })
        );

        let path = |name: &str| dir.join(name).to_str().expect("유니코드").to_owned();
        let expected_args: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--replay-user-messages",
            "--verbose",
            "--no-session-persistence",
            "--settings",
            &path(SETTINGS_FILE),
            "--strict-mcp-config",
            "--mcp-config",
            &path(MCP_CONFIG_FILE),
        ]
        .iter()
        .map(|a| (*a).to_owned())
        .collect();
        let (program, args) = console_command("claude", expected_args);
        assert_eq!(cmd.program, program);
        assert_eq!(cmd.args, args);
        assert!(
            !cmd.args
                .iter()
                .any(|a| a.contains("bypassPermissions") || a == "--permission-mode"),
            "조회에는 권한 우회 인자가 없다: {:?}",
            cmd.args
        );

        assert_eq!(
            cmd.env_remove,
            [
                "CLAUDECODE",
                "CLAUDE_CODE_ENTRYPOINT",
                "claude_lower_case",
                "CLAUDE_CODE_AUTO_CONNECT_IDE"
            ],
            "CLAUDE* 만 벗기고 로그인 둘은 남긴다(대소문자 무시)"
        );
        let set: Vec<(&str, &str)> = cmd
            .env_set
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            set,
            [
                ("MAX_THINKING_TOKENS", "8000"),
                ("ENABLE_CLAUDEAI_MCP_SERVERS", "false"),
                ("CLAUDE_CODE_AUTO_CONNECT_IDE", "0"),
                ("CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL", "1"),
            ]
        );
    }

    #[test]
    fn non_utf8_env_keys_are_skipped() {
        #[cfg(windows)]
        let odd = {
            use std::os::windows::ffi::OsStringExt;
            OsString::from_wide(&[0x43, 0x4C, 0x41, 0x55, 0x44, 0x45, 0xD800])
        };
        #[cfg(unix)]
        let odd = {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(b"CLAUDE\xFF".to_vec())
        };
        assert_eq!(
            stripped_env_keys([odd, OsString::from("CLAUDE_X")]),
            ["CLAUDE_X"]
        );
    }

    /// 운영 경로(`query`)는 데몬 프로세스 env 를 읽는다 — 무엇이 있든 벗기는 것은 `CLAUDE*` 뿐이고 로그인 둘은 남는다.
    #[test]
    fn query_strips_only_claude_keys_from_the_process_env() {
        let root = TempRoot::new("claude-probe-process-env");
        let spawner = ScriptedSpawner::new(answering(typical_usage()));
        let env = ProbeEnv {
            spawner: &spawner,
            deadline: Instant::now() + ROOMY,
            scratch_root: root.path(),
        };
        CLAUDE_USAGE_PROBE.query(&env).expect("조회");
        let expected = stripped_env_keys(std::env::vars_os().map(|(key, _)| key));
        let cmd = spawner.commands().pop().expect("명령");
        assert_eq!(cmd.env_remove, expected);
        for key in &cmd.env_remove {
            assert!(key.to_ascii_uppercase().starts_with("CLAUDE"), "{key}");
            assert!(
                !KEPT_CLAUDE_ENV.iter().any(|k| key.eq_ignore_ascii_case(k)),
                "{key}"
            );
        }
    }

    /// ★두 벌의 stream-json 핵심 인자가 같아야 한다★ — 에이전트 스폰이 그 조합을 바꾸면(CLI 요구가 바뀐 것이다)
    ///   이 시험이 깨져 조회 쪽도 같이 고치게 한다. 조회 인자는 스폰에서 파생하지 않는다.
    #[test]
    fn stream_json_core_args_match_the_agent_spawn() {
        let spawn = ClaudeBackend.build_spec(
            &AgentCommand::Claude {
                extra_args: vec![],
                output_format: AgentOutputFormat::StreamJson,
            },
            SpawnMode::Fresh,
            None,
            None,
            PathBuf::from("."),
            vec![],
            None,
        );
        assert!(
            spawn
                .args
                .windows(STREAM_JSON_CORE.len())
                .any(|w| w == STREAM_JSON_CORE),
            "스폰의 stream-json 핵심 인자가 조회 쪽과 어긋났다: {:?}",
            spawn.args
        );
        let root = TempRoot::new("claude-probe-parity");
        let probe = probe_command(root.path(), Vec::new()).expect("명령");
        assert!(probe
            .args
            .windows(STREAM_JSON_CORE.len())
            .any(|w| w == STREAM_JSON_CORE));
        assert_eq!(probe.program, spawn.program, "같은 셸 감싸기를 거친다");
        let thinking = |env: &[(String, String)]| {
            env.iter()
                .find(|(k, _)| k == "MAX_THINKING_TOKENS")
                .map(|(_, v)| v.clone())
        };
        assert_eq!(thinking(&probe.env_set), thinking(&spawn.env));
    }
}
