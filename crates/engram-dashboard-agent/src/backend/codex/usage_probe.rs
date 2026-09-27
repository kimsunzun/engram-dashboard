//! codex 의 사용량 능동 조회 — 짧게 뜨는 `codex app-server --stdio` 와 `initialize` → `initialized` →
//! `account/rateLimits/read` 를 주고받고 그 응답을 [`UsageObservation`] 으로 푼다. 부르는 쪽은
//! [`CODEX_USAGE_PROBE`] 를 [`UsageProbe`] 로만 본다.
//!
//! ★모델 턴이 없다★ — 스레드를 열지 않는다(`thread/start` 없음). 읽기 응답(또는 거절)을 받으면 stdin 을 닫고
//!   잠깐 기다린 뒤 끝낸다([`finish_after_answer`]).
//! ★대화 통로(`transport`)의 요청 기계를 쓰지 않는다★ — 그쪽의 대기표·라이터 스레드는 통로의 공유 상태에 묶여
//!   있다. 여기서는 요청을 하나씩 쓰고 그 응답을 한 스레드에서 읽는다.
//! ★응답의 `accountId` 는 읽지도 않는다 — 로그·오류 문자열·관측 어디에도 없다★(TRD §1-3): 상류가 준 문자열
//!   (응답 · RPC 오류 문구 · stderr)은 분류에만 쓰고, 싣는 것은 우리가 쓴 문장과 수치뿐이다. serde 오류 문구도
//!   받은 값을 되풀이할 수 있어 싣지 않는다. 예외 = 표시 칸(plan · 버킷 이름)에 싣는 짧은 이름.
//! 모양의 출처 = codex-cli 0.156.1 의 JSON Schema(`protocol` 의 `GetAccountRateLimitsResponse` doc).
// ADR-0004

use std::path::Path;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use super::protocol::{
    classify, error_response_line, method, notification_line, request_line, ClientInfo,
    GetAccountRateLimitsParams, GetAccountRateLimitsResponse, Inbound, InitializeParams,
    RateLimitSnapshot, RequestId, RpcError, CLIENT_NAME, METHOD_NOT_FOUND,
};
use super::usage::{names_default_bucket, windows_by_duration, DEFAULT_LIMIT_ID, USAGE_VENDOR};
use super::{APP_SERVER_STDIO_FLAG, APP_SERVER_SUBCOMMAND, CODEX_PROGRAM};
use crate::backend::console_command;
use crate::usage::{
    display_text, finish_after_answer, has_word, ProbeChild, ProbeCommand, ProbeEnv, ProbeError,
    ScopedWindowObs, ScratchDir, UsageObservation, UsagePolicy, UsageProbe, UsageSource,
    UsageVendorKey,
};

/// 자동 조회 사이의 최소 간격(사용자 결정 2026-09-27 — Claude 와 같은 값).
const CODEX_COOLDOWN: Duration = Duration::from_secs(15 * 60);

/// 조회 하나의 시한 — 멈춤 방지다. 실측 전체 0.74–0.94초(0.156.1 · 3회)지만 데몬 재시작 직후 대화 이어받기에서
/// `initialize` 약 9.7초가 한 번 관측됐다(원인 모름 — TRD §6 #5).
const CODEX_PROBE_TIMEOUT: Duration = Duration::from_secs(30);

const INITIALIZE_ID: i64 = 1;
const READ_ID: i64 = 2;

/// 서버 요청을 거절할 때 싣는 문구 — 상대의 로그로 간다.
const REFUSAL_MESSAGE: &str = "engram-dashboard 사용량 조회는 서버 요청을 처리하지 않는다";

/// 외부 문자열(plan·버킷 이름) 칸의 글자 수 상한 — 표시용 이름이라 이보다 길면 잘라 싣는다.
const PLAN_MAX_CHARS: usize = 32;
const LABEL_MAX_CHARS: usize = 48;
/// 모델별 창 목록의 상한. 오늘 응답의 비기본 버킷은 하나다.
const MODEL_SCOPED_MAX: usize = 16;

/// codex 사용량 조회기. 부르는 쪽은 싱글턴 [`CODEX_USAGE_PROBE`] 를 쓴다.
pub(crate) struct CodexUsageProbe;

pub(crate) static CODEX_USAGE_PROBE: CodexUsageProbe = CodexUsageProbe;

impl UsageProbe for CodexUsageProbe {
    fn key(&self) -> UsageVendorKey {
        USAGE_VENDOR
    }

    fn policy(&self) -> UsagePolicy {
        UsagePolicy {
            cooldown: CODEX_COOLDOWN,
            timeout: CODEX_PROBE_TIMEOUT,
        }
    }

    fn query(&self, env: &ProbeEnv<'_>) -> Result<UsageObservation, ProbeError> {
        env.require_time_left()?;
        // ★선언 순서가 drop 순서를 정한다 — 폴더가 자식보다 먼저다★: 자식이 먼저 drop(트리 kill + 종료 대기)돼야
        //   Windows 가 그 작업 폴더를 지울 수 있다.
        let scratch = ScratchDir::create(env.scratch_root)?;
        let mut child = env
            .spawner
            .spawn(&probe_command(scratch.path()), env.deadline)?;
        let answer = converse(child.as_mut(), env.deadline)?;
        finish_after_answer(child, env.deadline);
        observation_from_result(answer?)
    }
}

/// env 는 데몬 것을 그대로 물려받는다(TRD §3 #9 — 기본 로그인을 본다). 대화용 인자(`--cd`·정책·`-c`)는 없다 —
/// 스레드를 열지 않는다.
fn probe_command(scratch: &Path) -> ProbeCommand {
    let (program, args) = console_command(
        CODEX_PROGRAM,
        vec![
            APP_SERVER_SUBCOMMAND.to_owned(),
            APP_SERVER_STDIO_FLAG.to_owned(),
        ],
    );
    ProbeCommand {
        program,
        args,
        cwd: scratch.to_path_buf(),
        env_set: Vec::new(),
        env_remove: Vec::new(),
    }
}

/// 서버가 우리 요청 하나에 준 답 — 성공이면 `result`, 오류 응답이면 그것을 분류한 것([`classify_rpc_error`]).
/// 바깥 `Result` 의 오류(답을 못 받음)와 가른다 — 답을 받았으면 자식에게 끝낼 유예를 준다.
type Answer = Result<Value, ProbeError>;

/// 악수 → 읽기 요청 → 그 답. 요청마다 그 응답이 올 때까지 기다린 뒤 다음 줄을 쓴다. 악수가 거절되면 그 거절이
/// 답이다(읽기 요청을 안 보낸다).
fn converse(child: &mut dyn ProbeChild, deadline: Instant) -> Result<Answer, ProbeError> {
    let initialize = InitializeParams {
        client_info: ClientInfo {
            name: CLIENT_NAME.to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
    };
    let initialize_line = request_text(INITIALIZE_ID, method::INITIALIZE, &initialize)?;
    // 감싼 셸이 대상을 못 찾고 벌써 끝났으면 첫 쓰기가 먼저 실패한다 — 곧바로 돌아가면 「설치 안 됨」을 싣는 종료
    //   상태를 못 본다. 오류를 들고 EOF 까지 읽은 뒤 종료로 가른다.
    let write_error = match send(child, &initialize_line, deadline) {
        Ok(()) => None,
        Err(err @ ProbeError::Io(_)) => Some(err),
        Err(err) => return Err(err),
    };
    if let Err(rejected) = await_result(child, INITIALIZE_ID, deadline, write_error)? {
        return Ok(Err(rejected));
    }
    send(child, &notification_line(method::INITIALIZED), deadline)?;
    send(
        child,
        &request_text(
            READ_ID,
            method::ACCOUNT_RATE_LIMITS_READ,
            &GetAccountRateLimitsParams::default(),
        )?,
        deadline,
    )?;
    await_result(child, READ_ID, deadline, None)
}

fn request_text<P: Serialize>(id: i64, method: &str, params: &P) -> Result<String, ProbeError> {
    request_line(&RequestId::Num(id), method, params)
        .map_err(|_| ProbeError::Io("codex 사용량 조회 요청 줄을 만들지 못했다".to_owned()))
}

/// `protocol` 의 줄 도구는 끝 개행까지 붙여 주고 [`ProbeChild::write_line`] 도 붙인다 — 하나를 떼지 않으면 빈 줄이
/// 한 줄 더 나간다.
fn send(child: &mut dyn ProbeChild, line: &str, deadline: Instant) -> Result<(), ProbeError> {
    child.write_line(line.strip_suffix('\n').unwrap_or(line), deadline)
}

/// 우리 요청 `id` 의 응답이 올 때까지 읽는다 — 오류 응답도 답이다(분류해서 돌려준다).
///
/// - 서버 **요청**은 거절 응답으로 답한다 — 답하지 않으면 상대가 그 답을 기다리며 멈출 수 있다.
/// - 알림 · 다른 id 의 응답 · 봉투로 안 읽히는 줄은 버린다.
/// - ★우리 id 를 단 줄이 봉투로 안 읽히면(성공과 실패를 함께 싣거나 오류 본문이 깨짐) 곧바로 `Parse` 다★ —
///   기다려도 다른 응답은 오지 않는다. JSON 자체가 아닌 줄은 id 를 읽을 수 없어 버린다(그때는 시한까지 간다).
/// - `write_error` = 그 요청의 쓰기가 실패한 입출력 오류 — 응답 없이 끝나고 「설치 안 됨」도 아니면 이것을 돌려준다.
fn await_result(
    child: &mut dyn ProbeChild,
    id: i64,
    deadline: Instant,
    write_error: Option<ProbeError>,
) -> Result<Answer, ProbeError> {
    let ours = RequestId::Num(id);
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
        match classify(&line) {
            Ok(Inbound::Response { id, result }) if id == ours => return Ok(Ok(result)),
            Ok(Inbound::Error { id, error }) if id == ours => {
                return Ok(Err(classify_rpc_error(&error)))
            }
            Ok(Inbound::Request { id, .. }) => refuse(child, &id, deadline)?,
            Ok(_) => {}
            Err(_) if is_reply_to(&line, id) => {
                return Err(parse_error("우리 요청의 응답 줄이 봉투로 읽히지 않는다"));
            }
            Err(_) => {}
        }
    }
}

/// `method` 없이 `id` 가 `id` 이고 `result`·`error` 중 하나라도 실은 줄인가.
fn is_reply_to(line: &str, id: i64) -> bool {
    let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    !obj.contains_key("method")
        && obj.get("id").and_then(Value::as_i64) == Some(id)
        && (obj.contains_key("result") || obj.contains_key("error"))
}

/// ★성공을 위장하지 않는다★ — 승인 요청에 성공 응답을 돌려주면 그것이 자동 승인이다(`transport` 의 같은 자리).
/// 무슨 요청이었는지는 적지 않는다 — 상류가 정한 문자열이다.
fn refuse(child: &mut dyn ProbeChild, id: &RequestId, deadline: Instant) -> Result<(), ProbeError> {
    tracing::debug!("codex 사용량 조회: 서버 요청을 거절한다");
    send(
        child,
        &error_response_line(id, METHOD_NOT_FOUND, REFUSAL_MESSAGE),
        deadline,
    )
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
    // stderr 내용은 싣지 않는다 — 가린 것은 토큰 모양뿐이고 계정 식별자·경로가 남을 수 있다. 줄 수만 적는다.
    tracing::debug!(
        code = ?exit.code(),
        stderr_lines = child.stderr_tail().len(),
        "codex 사용량 조회: 응답 없이 끝났다"
    );
    if let Some(err) = read_error {
        return err;
    }
    ProbeError::Upstream(match exit.code() {
        Some(code) => format!("codex 가 사용량 응답 없이 끝났다(종료 코드 {code})"),
        None => "codex 가 사용량 응답 없이 끝났다(종료 코드 없음)".to_owned(),
    })
}

/// ★이 문구 표는 추정이다★ — codex 의 로그인 안 됨 오류 문구를 아직 못 봤다(TRD §6 #5 — 3단계 실 스모크가 모은다).
///   틀려도 떨어지는 곳은 `Upstream`(= 조회 실패)이다. 한도 문구를 [`ProbeError::RateLimited`] 로 가르지 않는 것은
///   TRD §3 #7 의 결정이다. `Upstream` 문구는 고정이다 — 상류 문구를 되풀이하지 않는다(코드 번호만 싣는다).
fn classify_rpc_error(error: &RpcError) -> ProbeError {
    const UNAUTHENTICATED: [&str; 10] = [
        "authenticat",
        "unauthenticated",
        "unauthorized",
        "not logged in",
        "logged out",
        "log in",
        "login",
        "log out",
        "sign in",
        "signed out",
    ];
    let text = error.message.to_ascii_lowercase();
    let classified = if UNAUTHENTICATED.iter().any(|w| has_word(&text, w, false))
        || has_word(&text, "401", true)
    {
        ProbeError::Unauthenticated
    } else {
        ProbeError::Upstream(format!(
            "codex 가 사용량 조회를 오류로 거절했다(코드 {})",
            error.code
        ))
    };
    tracing::debug!(kind = %classified, "codex 사용량 조회: 오류 응답");
    classified
}

/// 읽기 응답의 `result` → 관측.
///
/// - ★기본 버킷 = `rateLimitsByLimitId` 의 [`DEFAULT_LIMIT_ID`] 가 먼저다★ — 최상위 `rateLimits` 는 스키마가 「하위
///   호환 단일 버킷 보기」라 적어 다른 버킷을 비출 수 있다(피어 t3code 도 맵을 먼저 읽는다). 맵에 없을 때만 최상위를
///   쓰되, 그 `limitId` 가 기본 버킷을 가리킬 때만이다([`names_default_bucket`]).
/// - ★기본 버킷을 못 찾으면 `Parse` 다★ — 그것을 「창 없음」으로 읽으면 다른 버킷만 실린 응답이 성공으로 들어가
///   들고 있던 값을 전부 지운다. 실패면 받는 쪽이 값을 유지한다. `result` 가 객체가 아니어도 `Parse`.
/// - 창은 길이로 가른다(규칙 = 형제 `usage` 의 [`windows_by_duration`]). 관측은 `Active` 라 `None` 칸 = 「없다」.
/// - 맵의 나머지 버킷 → 모델별 창([`model_scoped_windows`]) · `planType` → plan(기본 버킷 것, 없으면 최상위 것).
fn observation_from_result(result: Value) -> Result<UsageObservation, ProbeError> {
    let response: GetAccountRateLimitsResponse =
        serde_json::from_value(result).map_err(|_| parse_error("응답 result 가 객체가 아니다"))?;
    let buckets = response.rate_limits_by_limit_id.unwrap_or_default();
    let top_level = response.rate_limits.as_ref();
    let default = buckets
        .iter()
        .find(|(id, _)| id == DEFAULT_LIMIT_ID)
        .map(|(_, snapshot)| snapshot)
        .or_else(|| top_level.filter(|snapshot| names_default_bucket(snapshot.limit_id.as_ref())))
        .ok_or_else(|| parse_error("기본 버킷의 스냅숏이 없다"))?;
    let (five_hour, weekly) = windows_by_duration(default);
    let plan = [Some(default), top_level]
        .into_iter()
        .flatten()
        .filter_map(|snapshot| snapshot.plan_type.as_deref())
        .find_map(|plan| display_text(plan, PLAN_MAX_CHARS));
    Ok(UsageObservation {
        vendor: USAGE_VENDOR,
        five_hour,
        weekly,
        model_scoped: model_scoped_windows(&buckets),
        plan,
        source: UsageSource::Active,
        limits_unavailable: false,
    })
}

fn parse_error(what: &str) -> ProbeError {
    ProbeError::Parse(format!("codex 사용량 응답: {what}"))
}

/// 기본이 아닌 버킷마다 그 **주간** 창 하나(D8) — 이름 = `limitName`, 쓸 수 없으면 버킷 id. 주간 창이 없거나 이름을
/// 못 붙이는 버킷은 뺀다. 같은 이름(ASCII 대소문자 무시)은 먼저 온 것만 남긴다(버킷 id 순). 하나도 없으면 `None`.
fn model_scoped_windows(buckets: &[(String, RateLimitSnapshot)]) -> Option<Vec<ScopedWindowObs>> {
    let mut windows: Vec<ScopedWindowObs> = Vec::new();
    for (id, snapshot) in buckets {
        if windows.len() >= MODEL_SCOPED_MAX {
            break;
        }
        if id == DEFAULT_LIMIT_ID {
            continue;
        }
        let label = snapshot
            .limit_name
            .as_deref()
            .and_then(|name| display_text(name, LABEL_MAX_CHARS))
            .or_else(|| display_text(id, LABEL_MAX_CHARS));
        let (Some(label), (_, Some(window))) = (label, windows_by_duration(snapshot)) else {
            continue;
        };
        if windows.iter().any(|w| w.label.eq_ignore_ascii_case(&label)) {
            continue;
        }
        windows.push(ScopedWindowObs { label, window });
    }
    (!windows.is_empty()).then_some(windows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use crate::usage::testing::{ChildScript, ScriptedSpawner, TempRoot};
    use crate::usage::{ExitInfo, ProbeSpawner, WindowObs};

    const SHORT: Duration = Duration::from_millis(300);
    const ROOMY: Duration = Duration::from_secs(4);

    const FIVE_HOUR_RESET: i64 = 1_790_424_706;
    const WEEKLY_RESET: i64 = 1_790_739_378;
    const MODEL_RESET: i64 = 1_791_018_755;

    /// 가짜 계정 식별자 — 이 값이 어느 결과에도 보이면 안 된다.
    const ACCOUNT_ID: &str = "acct-SECRET-7f3a91";

    fn run_with_deadline(
        spawner: &ScriptedSpawner,
        root: &TempRoot,
        within: Duration,
    ) -> Result<UsageObservation, ProbeError> {
        let env = ProbeEnv {
            spawner,
            deadline: Instant::now() + within,
            scratch_root: root.path(),
        };
        CODEX_USAGE_PROBE.query(&env)
    }

    fn run(script: ChildScript) -> (Result<UsageObservation, ProbeError>, ScriptedSpawner) {
        let root = TempRoot::new("codex-probe");
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

    /// 실 `initialize` 응답의 칸 넷(값은 가짜).
    fn init_response() -> String {
        json!({
            "id": 1,
            "result": {
                "codexHome": "C:\\Users\\someone\\.codex",
                "platformFamily": "windows",
                "platformOs": "windows",
                "userAgent": "engram-dashboard/0.0.0 (test)",
            },
        })
        .to_string()
    }

    fn read_response(result: Value) -> String {
        json!({ "id": 2, "result": result }).to_string()
    }

    fn answering(result: Value) -> ChildScript {
        ChildScript::new()
            .line(init_response())
            .line(read_response(result))
    }

    fn window_json(used: i64, mins: i64, resets: i64) -> Value {
        json!({ "usedPercent": used, "windowDurationMins": mins, "resetsAt": resets })
    }

    fn codex_bucket() -> Value {
        json!({
            "limitId": "codex",
            "limitName": null,
            "planType": "plus",
            "primary": window_json(37, 300, FIVE_HOUR_RESET),
            "secondary": window_json(58, 10080, WEEKLY_RESET),
            "credits": null,
        })
    }

    /// 조사 문서 §4-1 의 실측 모양(수치는 그 실측 · 식별자는 가짜) + 스키마의 나머지 칸.
    fn model_bucket() -> Value {
        json!({
            "limitId": "base_model_inference",
            "limitName": "gpt-reserve",
            "normalModelSlug": "gpt-5.6-luna",
            "planType": "plus",
            "primary": window_json(0, 10080, MODEL_RESET),
            "secondary": null,
        })
    }

    fn typical_result() -> Value {
        json!({
            "accountId": ACCOUNT_ID,
            "rateLimits": codex_bucket(),
            "rateLimitsByLimitId": { "codex": codex_bucket(), "base_model_inference": model_bucket() },
            "rateLimitResetCredits": { "availableCount": 0, "credits": null },
            "ordinaryUsageAllowed": true,
        })
    }

    fn window(used_pct: Option<f64>, resets_at: Option<i64>) -> Option<WindowObs> {
        Some(WindowObs {
            used_pct,
            resets_at,
        })
    }

    fn model_window(used_pct: f64) -> WindowObs {
        WindowObs {
            used_pct: Some(used_pct),
            resets_at: Some(MODEL_RESET),
        }
    }

    fn labels(obs: &UsageObservation) -> Vec<String> {
        obs.model_scoped
            .iter()
            .flatten()
            .map(|w| w.label.clone())
            .collect()
    }

    fn observe(result: Value) -> Result<UsageObservation, ProbeError> {
        observation_from_result(result)
    }

    // ── 정책 ──

    #[test]
    fn policy_and_key_are_the_codex_values() {
        assert_eq!(CODEX_USAGE_PROBE.key(), USAGE_VENDOR);
        assert_eq!(CODEX_USAGE_PROBE.key().as_str(), "codex");
        assert_eq!(
            CODEX_USAGE_PROBE.policy(),
            UsagePolicy {
                cooldown: Duration::from_secs(900),
                timeout: Duration::from_secs(30),
            }
        );
    }

    // ── 해석 ──

    #[test]
    fn success_reads_the_codex_bucket_and_the_other_buckets_weekly_window() {
        let (result, spawner) = run(answering(typical_result()));
        let obs = result.expect("조회");
        assert_eq!(obs.vendor, USAGE_VENDOR);
        assert_eq!(obs.source, UsageSource::Active);
        assert_eq!(obs.five_hour, window(Some(37.0), Some(FIVE_HOUR_RESET)));
        assert_eq!(obs.weekly, window(Some(58.0), Some(WEEKLY_RESET)));
        assert_eq!(
            obs.model_scoped,
            Some(vec![ScopedWindowObs {
                label: "gpt-reserve".to_owned(),
                window: model_window(0.0),
            }])
        );
        assert_eq!(obs.plan.as_deref(), Some("plus"));
        assert!(spawner.stdin_closed(), "응답을 받고 stdin 을 닫지 않았다");
        assert!(spawner.child_dropped(), "응답을 받고 자식을 죽이지 않았다");
        assert!(!spawner.cut_at_deadline());
    }

    /// 응답 뒤에 stdin 을 닫아도 안 끝나는 자식 — 유예 뒤에 끊기지만 조회는 성공이다.
    #[test]
    fn a_child_that_lingers_after_the_answer_is_cut_but_the_query_succeeds() {
        let (result, spawner) = run(answering(typical_result()).hang_after_lines());
        assert_eq!(result.expect("조회").plan.as_deref(), Some("plus"));
        assert!(spawner.stdin_closed());
        assert!(spawner.cut_at_deadline(), "유예 뒤에 끊었어야 한다");
        assert!(spawner.child_dropped());
    }

    /// 마감이 이미 지났으면 임시 폴더도 자식도 만들지 않는다.
    #[test]
    fn a_passed_deadline_times_out_before_anything_is_created() {
        let root = TempRoot::new("codex-probe-late");
        let spawner = ScriptedSpawner::new(answering(typical_result()));
        assert_eq!(
            run_with_deadline(&spawner, &root, Duration::ZERO),
            Err(ProbeError::Timeout)
        );
        assert!(spawner.commands().is_empty(), "기동을 시도했다");
        assert_scratch_gone(&root);
    }

    /// 조사 문서 §4-1 의 실측처럼 맵에 기본 버킷이 없으면 최상위를 쓰고, 맵의 버킷은 모델별 창이 된다.
    #[test]
    fn without_the_codex_bucket_in_the_map_the_top_level_view_is_used() {
        let mut top_level = codex_bucket();
        top_level["limitId"] = Value::Null;
        for by_limit_id in [
            json!({ "base_model_inference": model_bucket() }),
            json!(null),
            json!({}),
        ] {
            let has_model = by_limit_id.get("base_model_inference").is_some();
            let obs =
                observe(json!({ "rateLimits": top_level, "rateLimitsByLimitId": by_limit_id }))
                    .expect("관측");
            assert_eq!(obs.five_hour, window(Some(37.0), Some(FIVE_HOUR_RESET)));
            assert_eq!(obs.weekly, window(Some(58.0), Some(WEEKLY_RESET)));
            assert_eq!(obs.plan.as_deref(), Some("plus"));
            assert_eq!(
                labels(&obs),
                if has_model {
                    vec!["gpt-reserve".to_owned()]
                } else {
                    vec![]
                }
            );
        }
        // 맵 칸이 아예 없어도, `limitId` 칸이 없어도 같다.
        let mut bare = codex_bucket();
        bare.as_object_mut().expect("객체").remove("limitId");
        let obs = observe(json!({ "rateLimits": bare })).expect("관측");
        assert_eq!(obs.weekly, window(Some(58.0), Some(WEEKLY_RESET)));
        assert_eq!(obs.model_scoped, None);
    }

    /// ★최상위가 다른 버킷을 비춰도 맵의 기본 버킷이 이긴다★ — 최상위 값이 계정 창에 들어가면 안 된다.
    #[test]
    fn the_map_wins_over_a_top_level_view_of_another_limit() {
        let obs = observe(json!({
            "rateLimits": model_bucket(),
            "rateLimitsByLimitId": { "codex": codex_bucket(), "base_model_inference": model_bucket() },
        }))
        .expect("관측");
        assert_eq!(obs.five_hour, window(Some(37.0), Some(FIVE_HOUR_RESET)));
        assert_eq!(obs.weekly, window(Some(58.0), Some(WEEKLY_RESET)));
        assert_eq!(labels(&obs), ["gpt-reserve"]);

        // 맵이 이기는 것은 맵의 창이 비어도 그렇다 — 최상위로 채우지 않는다.
        let mut empty = codex_bucket();
        empty["primary"] = Value::Null;
        empty["secondary"] = Value::Null;
        let obs = observe(json!({
            "rateLimits": codex_bucket(),
            "rateLimitsByLimitId": { "codex": empty },
        }))
        .expect("관측");
        assert_eq!(obs.five_hour, None);
        assert_eq!(obs.weekly, None);
    }

    #[test]
    fn no_default_bucket_anywhere_is_a_parse_error() {
        for result in [
            json!({ "rateLimits": model_bucket() }),
            json!({ "rateLimits": model_bucket(), "rateLimitsByLimitId": { "base_model_inference": model_bucket() } }),
            json!({ "rateLimitsByLimitId": { "base_model_inference": model_bucket() } }),
            json!({ "rateLimits": null }),
            json!({}),
            json!({ "rateLimits": { "limitId": "Codex", "primary": window_json(1, 300, FIVE_HOUR_RESET) } }),
            json!({ "rateLimits": { "limitId": 5, "primary": window_json(1, 300, FIVE_HOUR_RESET) } }),
        ] {
            let got = observe(result.clone());
            assert!(
                matches!(got, Err(ProbeError::Parse(_))),
                "{result}: {got:?}"
            );
        }
    }

    #[test]
    fn null_windows_are_absent_in_an_active_observation() {
        let obs = observe(json!({
            "rateLimits": { "limitId": "codex", "planType": "free", "primary": null, "secondary": null },
        }))
        .expect("관측");
        assert_eq!(obs.five_hour, None);
        assert_eq!(obs.weekly, None);
        assert_eq!(obs.model_scoped, None);
        assert_eq!(obs.plan.as_deref(), Some("free"));
        assert_eq!(obs.source, UsageSource::Active);
    }

    #[test]
    fn unknown_or_missing_durations_are_dropped() {
        let bucket = |primary: Value, secondary: Value| {
            observe(json!({ "rateLimits": { "primary": primary, "secondary": secondary } }))
                .expect("관측")
        };
        let obs = bucket(
            window_json(40, 43_200, 5),
            window_json(7, 10080, WEEKLY_RESET),
        );
        assert_eq!(obs.five_hour, None, "월간 창 — 우리 칸에 없다");
        assert_eq!(obs.weekly, window(Some(7.0), Some(WEEKLY_RESET)));

        let obs = bucket(
            json!({ "usedPercent": 40, "resetsAt": FIVE_HOUR_RESET }),
            json!(null),
        );
        assert_eq!(
            obs.five_hour, None,
            "길이가 없으면 자리 이름으로 추측하지 않는다"
        );
        assert_eq!(obs.weekly, None);

        // 모델 버킷의 창이 주간이 아니면 그 버킷은 모델별 창이 없다.
        let obs = observe(json!({
            "rateLimits": codex_bucket(),
            "rateLimitsByLimitId": {
                "five_hour_only": { "primary": window_json(3, 300, FIVE_HOUR_RESET) },
                "monthly": { "primary": window_json(3, 43_200, FIVE_HOUR_RESET) },
            },
        }))
        .expect("관측");
        assert_eq!(obs.model_scoped, None);
    }

    /// ★배열이 칸 순서대로 읽혀 그럴듯한 가짜 창이 되는 일이 어느 층에서도 없다★.
    #[test]
    fn arrays_where_objects_are_expected_never_become_windows() {
        let array_bucket = json!([
            "codex",
            [37, 300, FIVE_HOUR_RESET],
            [58, 10080, WEEKLY_RESET]
        ]);
        let parse_cases = [
            json!([{ "rateLimits": codex_bucket() }]),
            json!({ "rateLimits": array_bucket }),
            json!({ "rateLimitsByLimitId": { "codex": array_bucket } }),
            json!({ "rateLimits": array_bucket, "rateLimitsByLimitId": [["codex", codex_bucket()]] }),
        ];
        for result in parse_cases {
            let got = observe(result.clone());
            assert!(
                matches!(got, Err(ProbeError::Parse(_))),
                "{result}: {got:?}"
            );
        }

        // 맵 칸이나 그 항목이 객체가 아니면 그 자리만 없다 — 최상위의 기본 버킷을 쓴다.
        for by_limit_id in [
            json!([["codex", codex_bucket()]]),
            json!({ "codex": array_bucket }),
            json!({ "codex": "codex" }),
            json!(7),
        ] {
            let obs = observe(
                json!({ "rateLimits": codex_bucket(), "rateLimitsByLimitId": by_limit_id }),
            )
            .expect("관측");
            assert_eq!(
                obs.weekly,
                window(Some(58.0), Some(WEEKLY_RESET)),
                "{by_limit_id}"
            );
            assert_eq!(obs.model_scoped, None, "{by_limit_id}");
        }

        // 창 자리가 배열이면 그 창만 없다 · 모델 버킷이 배열이면 그 버킷만 없다.
        let obs = observe(json!({
            "rateLimits": {
                "limitId": "codex",
                "primary": [37, 300, FIVE_HOUR_RESET],
                "secondary": window_json(58, 10080, WEEKLY_RESET),
            },
            "rateLimitsByLimitId": {
                "base_model_inference": ["gpt-reserve", [0, 10080, MODEL_RESET]],
                "other": { "limitName": "other", "primary": [0, 10080, MODEL_RESET] },
            },
        }))
        .expect("관측");
        assert_eq!(obs.five_hour, None);
        assert_eq!(obs.weekly, window(Some(58.0), Some(WEEKLY_RESET)));
        assert_eq!(obs.model_scoped, None);
    }

    #[test]
    fn model_scoped_labels_are_named_trimmed_deduplicated_and_capped() {
        let weekly = |used: i64| window_json(used, 10080, MODEL_RESET);
        let obs = observe(json!({
            "rateLimits": codex_bucket(),
            "rateLimitsByLimitId": {
                "codex": codex_bucket(),
                "b_no_name": { "primary": weekly(1) },
                "a_named": { "limitName": "  Reserve  ", "secondary": weekly(2) },
                "c_bad_name": { "limitName": "bad\u{7}name", "primary": weekly(3) },
                "d_dup": { "limitName": "reserve", "primary": weekly(4) },
                "e_number_name": { "limitName": 5, "primary": weekly(5) },
                "f_no_window": { "limitName": "empty", "primary": null },
            },
        }))
        .expect("관측");
        assert_eq!(
            labels(&obs),
            ["Reserve", "b_no_name", "c_bad_name", "e_number_name"],
            "버킷 id 순 · 이름 없으면 id · 같은 이름은 먼저 온 것"
        );
        let scoped = obs.model_scoped.expect("모델별 창");
        assert_eq!(scoped[0].window, model_window(2.0));
        assert_eq!(scoped[2].window, model_window(3.0));

        let many: serde_json::Map<String, Value> = (0..40)
            .map(|i| (format!("bucket-{i:02}"), json!({ "primary": weekly(i) })))
            .collect();
        let obs = observe(json!({ "rateLimits": codex_bucket(), "rateLimitsByLimitId": many }))
            .expect("관측");
        assert_eq!(obs.model_scoped.expect("모델별 창").len(), MODEL_SCOPED_MAX);

        let obs = observe(json!({
            "rateLimits": codex_bucket(),
            "rateLimitsByLimitId": { "long": { "limitName": "가".repeat(500), "primary": weekly(1) } },
        }))
        .expect("관측");
        let scoped = obs.model_scoped.expect("모델별 창");
        assert_eq!(
            scoped[0].label.chars().count(),
            LABEL_MAX_CHARS,
            "글자 단위로 자른다"
        );
    }

    #[test]
    fn the_plan_comes_from_the_default_bucket_then_the_top_level() {
        let plan = |default_plan: Value, top_plan: Value| {
            let mut default = codex_bucket();
            default["planType"] = default_plan;
            let mut top = model_bucket();
            top["planType"] = top_plan;
            observe(json!({ "rateLimits": top, "rateLimitsByLimitId": { "codex": default } }))
                .expect("관측")
                .plan
        };
        assert_eq!(plan(json!("pro"), json!("plus")).as_deref(), Some("pro"));
        assert_eq!(plan(json!(null), json!("plus")).as_deref(), Some("plus"));
        assert_eq!(plan(json!(" team\n"), json!(null)).as_deref(), Some("team"));
        assert_eq!(
            plan(json!("x".repeat(10_000)), json!(null)).map(|p| p.chars().count()),
            Some(PLAN_MAX_CHARS)
        );
        for bad in [
            json!(5),
            json!(""),
            json!("pl\nus"),
            json!({ "tier": "pro" }),
            json!(null),
        ] {
            assert_eq!(plan(bad.clone(), json!(null)), None, "{bad}");
        }
    }

    // ── 오류 ──

    fn kind(err: &ProbeError) -> &'static str {
        match err {
            ProbeError::Unauthenticated => "auth",
            ProbeError::Upstream(_) => "upstream",
            other => panic!("분류 밖: {other:?}"),
        }
    }

    fn rpc_error(message: &str) -> RpcError {
        RpcError {
            code: -32600,
            message: message.to_owned(),
            data: None,
        }
    }

    #[test]
    fn rpc_error_text_classification_table() {
        let cases = [
            (
                "codex account authentication required to read rate limits",
                "auth",
            ),
            (
                "chatgpt authentication required to read rate limits",
                "auth",
            ),
            (
                "failed to fetch codex rate limits: unexpected status 401 Unauthorized",
                "auth",
            ),
            ("Not logged in. Run `codex login`", "auth"),
            ("status=401", "auth"),
            ("please sign in again", "auth"),
            (
                "failed to fetch codex rate limits: 429 Too Many Requests",
                "upstream",
            ),
            ("internal error", "upstream"),
            ("", "upstream"),
            ("request req_14010a failed", "upstream"),
            ("catalog index failed", "upstream"),
            ("bloglogin", "upstream"),
        ];
        for (text, expected) in cases {
            assert_eq!(
                kind(&classify_rpc_error(&rpc_error(text))),
                expected,
                "{text:?}"
            );
        }
    }

    /// 읽기 요청의 오류 응답도, 악수의 오류 응답도 같은 분류를 타고 — 상류 문구를 되풀이하지 않는다.
    #[test]
    fn rpc_errors_are_classified_and_never_echo_the_upstream_text() {
        let error_line = |id: i64, message: String| {
            json!({ "id": id, "error": { "code": -32600, "message": message } }).to_string()
        };
        let cases = [
            (
                2,
                format!("codex account authentication required ({ACCOUNT_ID})"),
                "auth",
            ),
            (
                2,
                format!("failed to fetch codex rate limits for {ACCOUNT_ID}"),
                "upstream",
            ),
            (1, format!("boom {ACCOUNT_ID}"), "upstream"),
        ];
        for (id, message, expected) in cases {
            let mut script = ChildScript::new();
            if id == 2 {
                script = script.line(init_response());
            }
            let (result, spawner) = run(script.line(error_line(id, message)));
            let err = result.expect_err("오류 응답");
            assert_eq!(kind(&err), expected);
            let shown = format!("{err} {err:?}");
            assert!(!shown.contains(ACCOUNT_ID), "{shown}");
            assert!(spawner.stdin_closed(), "거절도 답이다 — 유예를 준다");
            assert!(spawner.child_dropped());
            if id == 1 {
                assert_eq!(
                    spawner.written().len(),
                    1,
                    "악수가 실패하면 읽기 요청을 안 보낸다"
                );
            }
        }
    }

    // ── 대화 ──

    #[test]
    fn the_dialogue_is_initialize_then_initialized_then_one_read() {
        let script = ChildScript::new().line(init_response()).reply(|written| {
            assert_eq!(
                written.len(),
                3,
                "읽기 응답 전에 쓴 줄 = 악수 둘 + 읽기: {written:?}"
            );
            read_response(typical_result())
        });
        let (result, spawner) = run(script);
        result.expect("조회");
        let written = spawner.written();
        let lines: Vec<Value> = written
            .iter()
            .map(|line| {
                assert!(!line.ends_with('\n'), "끝 개행이 두 번 나간다: {line:?}");
                serde_json::from_str(line).expect("JSON")
            })
            .collect();
        assert_eq!(
            lines,
            [
                json!({
                    "id": 1,
                    "method": "initialize",
                    "params": { "clientInfo": { "name": "engram-dashboard", "version": env!("CARGO_PKG_VERSION") } },
                }),
                json!({ "method": "initialized" }),
                json!({ "id": 2, "method": "account/rateLimits/read", "params": {} }),
            ]
        );
    }

    #[test]
    fn server_requests_are_refused_and_the_dialogue_continues() {
        let script = ChildScript::new()
            .line(r#"{"id":7,"method":"item/tool/requestUserInput","params":{}}"#)
            .line(init_response())
            .line(r#"{"id":"srv-1","method":"item/commandExecution/requestApproval","params":{"x":1}}"#)
            .line(read_response(typical_result()));
        let (result, spawner) = run(script);
        assert_eq!(result.expect("조회").plan.as_deref(), Some("plus"));
        let written: Vec<Value> = spawner
            .written()
            .iter()
            .map(|line| serde_json::from_str(line).expect("JSON"))
            .collect();
        assert_eq!(written.len(), 5, "{written:?}");
        let refusal = |id: Value| json!({ "id": id, "error": { "code": METHOD_NOT_FOUND, "message": REFUSAL_MESSAGE } });
        assert_eq!(written[1], refusal(json!(7)), "받은 id 를 그대로 싣는다");
        assert_eq!(written[2]["method"], "initialized");
        assert_eq!(written[3]["method"], "account/rateLimits/read");
        assert_eq!(written[4], refusal(json!("srv-1")));
    }

    #[test]
    fn notifications_other_ids_and_junk_lines_are_ignored() {
        let script = ChildScript::new()
            .line(r#"{"method":"account/rateLimits/updated","params":{"rateLimits":{"limitId":"codex"}}}"#)
            .line(read_response(json!({ "rateLimits": model_bucket() })))
            .line("Warning: not json")
            .line("[1,2,3]")
            .line(r#"{"id":9,"result":{},"error":{"code":1,"message":"x"}}"#)
            .line(init_response())
            .line(init_response())
            .line(r#"{"id":"2","result":{"rateLimits":{"limitId":"base_model_inference"}}}"#)
            .line(r#"{"id":3,"error":{"code":-1,"message":"not logged in"}}"#)
            .line(r#"{"method":"thread/started","params":{},"emittedAtMs":1}"#)
            .line(read_response(typical_result()));
        let (result, _) = run(script);
        let obs = result.expect("조회");
        assert_eq!(obs.weekly, window(Some(58.0), Some(WEEKLY_RESET)));
    }

    /// 우리 id 를 단 응답이 봉투로 안 읽히면 시한까지 기다리지 않는다.
    #[test]
    fn a_broken_reply_to_our_request_is_a_parse_error_without_waiting() {
        for broken in [
            r#"{"id":2,"result":{},"error":{"code":1,"message":"x"}}"#,
            r#"{"id":2,"error":"just text"}"#,
            r#"{"id":2,"error":{"message":"no code"}}"#,
        ] {
            let root = TempRoot::new("codex-probe-broken");
            let spawner = ScriptedSpawner::new(
                ChildScript::new()
                    .line(init_response())
                    .line(broken)
                    .hang_after_lines(),
            );
            let result = run_with_deadline(&spawner, &root, SHORT);
            assert!(
                matches!(result, Err(ProbeError::Parse(_))),
                "{broken}: {result:?}"
            );
            assert!(!spawner.cut_at_deadline(), "{broken}: 시한까지 기다렸다");
            assert_scratch_gone(&root);
        }
    }

    /// 읽기 오류를 먼저 내는 자식 — 실물 스포너의 「상한을 넘은 한 줄」 모양(대본 대역은 오류 줄을 못 낸다).
    struct IoFirstSpawner {
        then_answer: bool,
    }

    struct IoFirstChild {
        lines: VecDeque<String>,
        io_sent: bool,
    }

    impl ProbeSpawner for IoFirstSpawner {
        fn spawn(
            &self,
            _cmd: &ProbeCommand,
            _deadline: Instant,
        ) -> Result<Box<dyn ProbeChild>, ProbeError> {
            let lines = if self.then_answer {
                VecDeque::from([init_response(), read_response(typical_result())])
            } else {
                VecDeque::new()
            };
            Ok(Box::new(IoFirstChild {
                lines,
                io_sent: false,
            }))
        }
    }

    impl ProbeChild for IoFirstChild {
        fn write_line(&mut self, _line: &str, _deadline: Instant) -> Result<(), ProbeError> {
            Ok(())
        }

        fn close_stdin(&mut self) {}

        fn read_line(&mut self, _deadline: Instant) -> Result<Option<String>, ProbeError> {
            if !self.io_sent {
                self.io_sent = true;
                return Err(ProbeError::Io("한 줄이 상한을 넘었다".to_owned()));
            }
            Ok(self.lines.pop_front())
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
            let root = TempRoot::new("codex-probe-io");
            let spawner = IoFirstSpawner { then_answer };
            let env = ProbeEnv {
                spawner: &spawner,
                deadline: Instant::now() + ROOMY,
                scratch_root: root.path(),
            };
            let result = CODEX_USAGE_PROBE.query(&env);
            if expect_ok {
                assert!(result.is_ok(), "{result:?}");
            } else {
                assert!(matches!(result, Err(ProbeError::Io(_))), "{result:?}");
            }
            assert_scratch_gone(&root);
        }
    }

    #[test]
    fn a_silent_child_times_out_and_is_cut() {
        for script in [
            ChildScript::new().hang_after_lines(),
            ChildScript::new().line(init_response()).hang_after_lines(),
        ] {
            let root = TempRoot::new("codex-probe-silent");
            let spawner = ScriptedSpawner::new(script);
            let started = Instant::now();
            let result = run_with_deadline(&spawner, &root, SHORT);
            assert_eq!(result, Err(ProbeError::Timeout));
            assert!(started.elapsed() >= SHORT, "마감 전에 돌아왔다");
            assert!(spawner.cut_at_deadline());
            assert!(spawner.child_dropped());
            assert_scratch_gone(&root);
        }
    }

    #[test]
    fn a_child_that_never_reads_stdin_times_out() {
        let root = TempRoot::new("codex-probe-blocked");
        let spawner = ScriptedSpawner::new(ChildScript::new().block_writes());
        assert_eq!(
            run_with_deadline(&spawner, &root, SHORT),
            Err(ProbeError::Timeout)
        );
        assert!(spawner.cut_at_deadline());
        assert!(spawner.written().is_empty());
        assert_scratch_gone(&root);
    }

    #[test]
    fn eof_without_a_response_is_classified_by_the_exit() {
        let (result, spawner) = run(ChildScript::new().exit_not_installed().stderr_tail(&["x"]));
        assert_eq!(result, Err(ProbeError::NotInstalled));
        assert!(spawner.child_dropped());

        // 악수 뒤에 끝나도 같다 — stderr 는 싣지 않는다.
        let stderr = format!("Error: account {ACCOUNT_ID} is not allowed");
        let (result, _) = run(ChildScript::new()
            .line(init_response())
            .exit_code(1)
            .stderr_tail(&[stderr.as_str()]));
        match result {
            Err(ProbeError::Upstream(text)) => {
                assert!(text.contains("종료 코드 1"), "{text}");
                assert!(!text.contains(ACCOUNT_ID), "stderr 를 실었다: {text}");
            }
            other => panic!("{other:?}"),
        }

        let root = TempRoot::new("codex-probe-linger");
        let spawner = ScriptedSpawner::new(ChildScript::new().never_exit());
        assert_eq!(
            run_with_deadline(&spawner, &root, SHORT),
            Err(ProbeError::Timeout)
        );
        assert!(spawner.cut_at_deadline());
        assert_scratch_gone(&root);
    }

    /// ★첫 쓰기의 실패가 「설치 안 됨」을 가리지 않는다★ — 감싼 셸이 벌써 끝나 악수 쓰기가 실패해도 종료로 가른다.
    ///   설치된 쪽의 실패면 쓰기 오류 그대로다.
    #[test]
    fn a_failed_handshake_write_still_reads_the_exit() {
        let (result, spawner) = run(ChildScript::new().fail_writes().exit_not_installed());
        assert_eq!(result, Err(ProbeError::NotInstalled));
        assert!(spawner.written().is_empty());

        let (result, _) = run(ChildScript::new().fail_writes().exit_code(1));
        assert!(matches!(result, Err(ProbeError::Io(_))), "{result:?}");
    }

    #[test]
    fn a_spawn_failure_passes_through_and_cleans_up() {
        let root = TempRoot::new("codex-probe-missing");
        let spawner = ScriptedSpawner::failing(ProbeError::NotInstalled);
        assert_eq!(
            run_with_deadline(&spawner, &root, ROOMY),
            Err(ProbeError::NotInstalled)
        );
        assert_eq!(spawner.commands().len(), 1);
        assert_scratch_gone(&root);
    }

    // ── 기동 명세 ──

    #[test]
    fn the_command_is_the_app_server_in_its_own_scratch_dir_with_the_daemon_env() {
        let root = TempRoot::new("codex-probe-command");
        let seen_dir: Arc<Mutex<Option<PathBuf>>> = Arc::default();
        let seen = seen_dir.clone();
        let scan_root = root.path().to_path_buf();
        let spawner =
            ScriptedSpawner::new(ChildScript::new().line(init_response()).reply(move |_| {
                // 자식이 떠 있는 동안 — 임시 폴더가 제자리에 있어야 한다.
                let dir = fs::read_dir(&scan_root)
                    .expect("루트")
                    .flatten()
                    .map(|e| e.path())
                    .next()
                    .expect("임시 폴더");
                *seen.lock().expect("잠금") = Some(dir);
                read_response(typical_result())
            }));
        run_with_deadline(&spawner, &root, ROOMY).expect("조회");

        let commands = spawner.commands();
        let [cmd] = commands.as_slice() else {
            panic!("자식은 하나: {commands:?}")
        };
        let dir = seen_dir.lock().expect("잠금").take().expect("대화 중 폴더");
        assert_eq!(cmd.cwd, dir, "작업 폴더 = 그 조회의 임시 폴더");
        assert!(cmd.cwd.starts_with(root.path()));
        assert!(!cmd.cwd.exists(), "조회 뒤 임시 폴더가 남았다");

        let (program, args) =
            console_command("codex", vec!["app-server".to_owned(), "--stdio".to_owned()]);
        assert_eq!(cmd.program, program);
        assert_eq!(cmd.args, args);
        assert!(cmd.env_set.is_empty(), "env 는 데몬 것을 그대로 물려받는다");
        assert!(
            cmd.env_remove.is_empty(),
            "env 는 데몬 것을 그대로 물려받는다"
        );
    }

    // ── 계정 식별자 ──

    /// ★응답의 `accountId` 가 관측에도, 어떤 실패의 문구에도 보이지 않는다★.
    #[test]
    fn the_account_id_never_reaches_an_observation_or_an_error() {
        let obs = observe(typical_result()).expect("관측");
        assert!(!format!("{obs:?}").contains(ACCOUNT_ID), "{obs:?}");

        let failing = [
            json!({ "accountId": ACCOUNT_ID, "rateLimits": model_bucket() }),
            json!({ "accountId": ACCOUNT_ID, "rateLimits": [ACCOUNT_ID] }),
            json!({ "accountId": ACCOUNT_ID }),
            json!([ACCOUNT_ID]),
            json!(ACCOUNT_ID),
        ];
        for result in failing {
            let err = observe(result.clone()).expect_err("실패");
            let shown = format!("{err} {err:?}");
            assert!(!shown.contains(ACCOUNT_ID), "{result}: {shown}");
        }

        // 대화 전체로도 — 관측에 싣는 표시 칸(plan·버킷 이름) 밖의 어느 칸에 실려 와도.
        let mut result = typical_result();
        result["rateLimits"]["accountId"] = json!(ACCOUNT_ID);
        result["rateLimitsByLimitId"]["codex"]["accountId"] = json!(ACCOUNT_ID);
        result["rateLimitsByLimitId"]["codex"]["normalModelSlug"] = json!(ACCOUNT_ID);
        let (got, _) = run(answering(result));
        let obs = got.expect("조회");
        assert!(!format!("{obs:?}").contains(ACCOUNT_ID), "{obs:?}");
    }

    // ── 무패닉 ──

    /// 어느 자리에 무엇이 와도 패닉하지 않는다 — 결과는 관측이거나 `Parse` 다.
    #[test]
    fn hostile_results_never_panic() {
        let weird = [
            json!(null),
            json!(true),
            json!(0),
            json!(-1),
            json!(1e308),
            json!(-1e308),
            json!(18_446_744_073_709_551_615_u64),
            json!(i64::MIN),
            json!(0.5),
            json!(""),
            json!("x".repeat(5_000)),
            json!("\u{0}\u{7f}\u{202e}"),
            json!([]),
            json!([1, 2, 3]),
            json!({}),
            json!({ "a": { "b": [] } }),
        ];
        let base = typical_result();
        let paths: [&[&str]; 13] = [
            &[],
            &["rateLimits"],
            &["rateLimitsByLimitId"],
            &["rateLimitsByLimitId", "codex"],
            &["rateLimitsByLimitId", "base_model_inference"],
            &["rateLimitsByLimitId", "codex", "primary"],
            &["rateLimitsByLimitId", "codex", "limitId"],
            &["rateLimitsByLimitId", "codex", "planType"],
            &["rateLimitsByLimitId", "codex", "primary", "usedPercent"],
            &[
                "rateLimitsByLimitId",
                "codex",
                "primary",
                "windowDurationMins",
            ],
            &["rateLimitsByLimitId", "codex", "secondary", "resetsAt"],
            &["rateLimitsByLimitId", "base_model_inference", "limitName"],
            &[
                "rateLimitsByLimitId",
                "base_model_inference",
                "primary",
                "usedPercent",
            ],
        ];
        for path in paths {
            for value in &weird {
                let mut result = base.clone();
                match path.split_last() {
                    None => result = value.clone(),
                    Some((last, parents)) => {
                        let mut at = &mut result;
                        for key in parents {
                            at = &mut at[*key];
                        }
                        at[*last] = value.clone();
                    }
                }
                match observe(result) {
                    Ok(obs) => {
                        for w in [obs.five_hour, obs.weekly].into_iter().flatten() {
                            assert!(
                                w.used_pct.is_none_or(|p| (0.0..=100.0).contains(&p)),
                                "{path:?}: {w:?}"
                            );
                        }
                    }
                    Err(ProbeError::Parse(_)) => {}
                    Err(other) => panic!("{path:?} {value}: {other:?}"),
                }
            }
        }

        // 대화 쪽 — 봉투로 안 읽히는 줄 묶음 뒤에도 응답을 받는다.
        let mut script = ChildScript::new();
        for junk in [
            "",
            "{",
            "null",
            r#"{"id":null,"result":{}}"#,
            r#"{"id":{"n":2},"result":{}}"#,
            r#"{"id":2.0,"result":{}}"#,
            r#"{"method":7,"id":2}"#,
            r#"{"method":"x","id":[2]}"#,
            "\u{feff}{\"id\":1}",
        ] {
            script = script.line(junk);
        }
        let (result, _) = run(script
            .line(init_response())
            .line(read_response(typical_result())));
        assert!(result.is_ok(), "{result:?}");
    }
}
