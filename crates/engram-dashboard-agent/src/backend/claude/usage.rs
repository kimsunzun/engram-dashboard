//! claude 의 사용량 한도 — stream-json `rate_limit_event` 줄 해석과 비기본 프로필 판정.
//!
//! ★줍기 모양은 CLI 버전마다 다르고 문서화된 것은 일부뿐이다★(실측·출처 =
//!   `docs/research/usage-limit-display-ui-survey-2026-09-26.md` §4-2):
//!   - 옛 모양(`fixtures/claude_text.jsonl:3`) — `rate_limit_info.{rateLimitType, resetsAt}` 만 있고
//!     사용률이 없다.
//!   - 2.1.280 캡처 — 같은 최상위 칸에 `utilization`(0–1)이 더해지고, SDK 타입에 없는
//!     `unifiedWindows.{five_hour,seven_day}.{utilization,resetsAt}` 가 두 창을 함께 싣는다.
//!   그래서 두 모양을 다 느슨하게 받고, 칸 하나가 틀리면 그 칸만 버린다.

use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Map, Value};

use crate::transport::OutputDecoder;
use crate::usage::{
    env_overrides_daemon, resets_at_from_json, used_pct_from_fraction, UsageGate, UsageObservation,
    UsageSource, UsageVendorKey, WindowObs,
};

/// 사용량 칸에서 claude 를 가리키는 정본 낱말 — wire 의 백엔드 낱말(`AgentBackendKind` 소문자)과 같다.
pub const USAGE_VENDOR: UsageVendorKey = UsageVendorKey::new("claude");

/// 명세가 이 중 하나라도 데몬 자신의 값과 다르게 두면 그 에이전트는 줍지 않는다 — 데몬의 기본 로그인이 아닌
/// 계정·자격증명·엔드포인트로 뜰 수 있는 env 다.
///
/// - `CLAUDE_CONFIG_DIR` = 자기 상태(계정 로그인 포함)를 두는 폴더([`super::config_dir`] 가 읽는 그 키).
///   ★빈 값은 그쪽과 다르게 다룬다★ — 그쪽은 빈 값을 「없음」으로 접지만 이 판정은 「있다」로 보고 데몬 값과
///   바이트 그대로 견준다(데몬에 그 키가 없으면 다르다). 틀린다면 줍지 않는 쪽으로 틀리게 골랐다
///   ([`env_overrides_daemon`]).
/// - 나머지 = 자격증명(`CLAUDE_CODE_OAUTH_TOKEN`·`ANTHROPIC_API_KEY`·`ANTHROPIC_AUTH_TOKEN`)과 엔드포인트
///   (`ANTHROPIC_BASE_URL`·`CLAUDE_CODE_USE_BEDROCK`·`CLAUDE_CODE_USE_VERTEX`). 로그인 폴더가 같아도 이것들이
///   다르면 그 에이전트의 한도가 기본 로그인의 것이라는 보장이 없다.
const ACCOUNT_ENV_KEYS: &[&str] = &[
    "CLAUDE_CONFIG_DIR",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
];

/// 모르는 창 종류를 이미 로그에 적었나 — 이 프로세스에서 한 번만 적는다(이벤트마다 같은 줄이 반복된다).
static UNKNOWN_WINDOW_TYPE_LOGGED: AtomicBool = AtomicBool::new(false);

/// 칸이 하나도 안 남은 창을 이미 로그에 적었나 — 위와 같은 이유로 프로세스당 한 번.
static EMPTY_WINDOW_LOGGED: AtomicBool = AtomicBool::new(false);

/// `rate_limit_event` 줄(이미 JSON 으로 읽힌 한 줄) → 관측. 실린 창이 하나도 없으면 `None`.
///
/// - 최상위 `rateLimitType` ∈ {`five_hour`, `seven_day`} 이면 같은 자리의 `utilization`·`resetsAt` 가 그 창이다.
///   그 밖의 종류는 버린다 — 모델별 주간 창(`seven_day_opus` 등)도 여기 든다: 이 줄은 창을 하나씩 싣는데
///   관측의 `model_scoped: Some` 은 목록 전량 교체라, 하나만 실으면 나머지 모델 창을 지운다.
/// - `unifiedWindows` 의 같은 창이 있으면 **칸마다** 그쪽이 이긴다. 그쪽 칸이 없거나 틀리면 최상위 칸을 쓴다.
/// - 사용률은 0–1 → 0–100, 리셋은 정수 epoch 초만. 정규화 규칙 = [`crate::usage`] 의 도구들.
pub(super) fn observation_from_rate_limit_event(line: &Value) -> Option<UsageObservation> {
    let info = line.get("rate_limit_info")?.as_object()?;

    let mut five_hour = WindowObs {
        used_pct: None,
        resets_at: None,
    };
    let mut weekly = five_hour;
    match info.get("rateLimitType").and_then(Value::as_str) {
        Some("five_hour") => five_hour = window_fields(info),
        Some("seven_day") => weekly = window_fields(info),
        Some(_) => note_unknown_window_type(),
        None => {}
    }
    if let Some(unified) = info.get("unifiedWindows").and_then(Value::as_object) {
        if let Some(w) = unified.get("five_hour").and_then(Value::as_object) {
            five_hour = prefer(window_fields(w), five_hour);
        }
        if let Some(w) = unified.get("seven_day").and_then(Value::as_object) {
            weekly = prefer(window_fields(w), weekly);
        }
    }

    let five_hour = carried(five_hour);
    let weekly = carried(weekly);
    if five_hour.is_none() && weekly.is_none() {
        return None;
    }
    Some(UsageObservation {
        vendor: USAGE_VENDOR,
        five_hour,
        weekly,
        model_scoped: None,
        plan: None,
        source: UsageSource::Passive,
    })
}

/// 창 객체 하나의 칸. 칸마다 따로 버리는 것은 조용하고, 칸이 **하나도** 안 남으면 그때만 로그에 적는다.
fn window_fields(fields: &Map<String, Value>) -> WindowObs {
    let window = WindowObs {
        used_pct: fields
            .get("utilization")
            .and_then(Value::as_f64)
            .and_then(used_pct_from_fraction),
        resets_at: fields.get("resetsAt").and_then(resets_at_from_json),
    };
    if window.used_pct.is_none() && window.resets_at.is_none() {
        note_empty_window();
    }
    window
}

fn prefer(primary: WindowObs, fallback: WindowObs) -> WindowObs {
    WindowObs {
        used_pct: primary.used_pct.or(fallback.used_pct),
        resets_at: primary.resets_at.or(fallback.resets_at),
    }
}

/// 두 칸이 다 비면 그 창은 「안 실렸다」다 — 줍기 관측에서 `None` 창은 받는 쪽이 들고 있던 값을 둔다.
fn carried(window: WindowObs) -> Option<WindowObs> {
    (window.used_pct.is_some() || window.resets_at.is_some()).then_some(window)
}

/// ★값을 적지 않는다★ — 외부 프로세스가 정한 문자열이라 마스킹을 거쳐도 무엇이 실릴지 우리가 정하지 못한다.
fn note_unknown_window_type() {
    if UNKNOWN_WINDOW_TYPE_LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    tracing::debug!(
        "claude rate_limit_event 의 모르는 창 종류라 버린다(프로세스당 한 번만 적는다)"
    );
}

fn note_empty_window() {
    if EMPTY_WINDOW_LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    tracing::debug!(
        "claude rate_limit_event 의 창에 읽히는 칸이 하나도 없어 버린다(프로세스당 한 번만 적는다)"
    );
}

/// 이 명세로 뜬 claude 가 데몬과 **같은 계정**으로 도나 — [`ACCOUNT_ENV_KEYS`] 가 하나도 데몬 값과 다르지
/// 않을 때만 `true`. `false` 면 그 에이전트의 줍기를 막는다.
/// `daemon_env` = 키 → 데몬 프로세스 자신의 그 env 값(운영 = 프로세스 env · 시험 = 대역). 키마다 비교 규칙·틀릴
/// 때의 방향 = [`env_overrides_daemon`].
///
/// ★env 만 본다 — 이 판정이 못 보는 구멍★: `--settings` 로 넘긴 설정 파일이나 프로젝트 `.claude/settings.json`
///   안의 `env`·`apiKeyHelper` 블록. 그렇게 계정을 바꾼 에이전트의 관측은 기본 칸에 섞인다.
pub(super) fn collects_default_account_usage(
    spec_env: &[(String, String)],
    daemon_env: impl Fn(&str) -> Option<OsString>,
) -> bool {
    !ACCOUNT_ENV_KEYS
        .iter()
        .any(|key| env_overrides_daemon(spec_env, key, daemon_env(key).as_deref()))
}

/// 스폰 경로의 디코더를 줍기 판정대로 둔다 — 주우면 **그대로** 돌려주고, 줍지 않으면 [`UsageGate`] 로 막는다.
/// 판정의 데몬 쪽 값은 여기서 프로세스 env 를 읽는다.
pub(super) fn gate_decoder(
    decoder: Box<dyn OutputDecoder>,
    spec_env: &[(String, String)],
) -> Box<dyn OutputDecoder> {
    if collects_default_account_usage(spec_env, |key| std::env::var_os(key)) {
        return decoder;
    }
    // 어느 키였는지도 값도 적지 않는다 — 자격증명과 사용자 폴더 이름이 든다.
    tracing::debug!("claude 기본 계정이 아닐 수 있는 env — 이 에이전트의 사용량 줍기를 끈다");
    Box::new(UsageGate::blocking(decoder))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Option<UsageObservation> {
        let value: Value = serde_json::from_str(line).expect("시험 줄은 JSON 이다");
        observation_from_rate_limit_event(&value)
    }

    fn window(used_pct: Option<f64>, resets_at: Option<i64>) -> Option<WindowObs> {
        Some(WindowObs {
            used_pct,
            resets_at,
        })
    }

    /// 2.1.280 캡처 그대로(식별자 없음 — 조사 문서 §4-2).
    const NEW_SHAPE: &str = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","rateLimitType":"seven_day","utilization":0.83,"resetsAt":1790856000,"surpassedThreshold":0.75,"isUsingOverage":false,"unifiedWindows":{"five_hour":{"utilization":0.67,"resetsAt":1790425800},"seven_day":{"utilization":0.83,"resetsAt":1790856000}}}}"#;

    // ── 모양 ──

    #[test]
    fn new_shape_carries_both_windows() {
        let obs = parse(NEW_SHAPE).expect("관측");
        assert_eq!(obs.vendor, USAGE_VENDOR);
        assert_eq!(obs.source, UsageSource::Passive);
        assert_eq!(obs.five_hour, window(Some(67.0), Some(1_790_425_800)));
        assert_eq!(obs.weekly, window(Some(83.0), Some(1_790_856_000)));
        assert_eq!(obs.model_scoped, None, "줍기는 모델별 창을 싣지 않는다");
        assert_eq!(obs.plan, None);
    }

    /// 옛 모양 — 사용률 없이 리셋만. `fixtures/claude_text.jsonl:3` 그대로다.
    #[test]
    fn old_shape_carries_reset_only() {
        let line = include_str!("fixtures/claude_text.jsonl")
            .lines()
            .nth(2)
            .expect("fixture 3번째 줄");
        assert!(line.contains("rate_limit_event"), "전제: 그 줄이 맞다");
        let obs = parse(line).expect("관측");
        assert_eq!(obs.five_hour, window(None, Some(1_782_567_000)));
        assert_eq!(obs.weekly, None);
    }

    #[test]
    fn top_level_only_window_without_unified() {
        let obs = parse(r#"{"rate_limit_info":{"rateLimitType":"five_hour","utilization":0.55,"resetsAt":1790425800}}"#)
            .expect("관측");
        assert_eq!(obs.five_hour, window(Some(55.0), Some(1_790_425_800)));
        assert_eq!(obs.weekly, None);
    }

    #[test]
    fn unified_only_without_rate_limit_type() {
        let obs = parse(r#"{"rate_limit_info":{"unifiedWindows":{"seven_day":{"utilization":0.1,"resetsAt":1790856000}}}}"#)
            .expect("관측");
        assert_eq!(obs.five_hour, None);
        assert_eq!(obs.weekly, window(Some(10.0), Some(1_790_856_000)));
    }

    #[test]
    fn unified_wins_field_by_field() {
        let obs = parse(r#"{"rate_limit_info":{"rateLimitType":"seven_day","utilization":0.5,"resetsAt":1790000000,"unifiedWindows":{"seven_day":{"utilization":0.6,"resetsAt":1790856000}}}}"#)
            .expect("관측");
        assert_eq!(obs.weekly, window(Some(60.0), Some(1_790_856_000)));

        // 그쪽 칸이 틀리면 그 칸만 최상위로 되돌아간다.
        let obs = parse(r#"{"rate_limit_info":{"rateLimitType":"seven_day","utilization":0.5,"resetsAt":1790000000,"unifiedWindows":{"seven_day":{"utilization":"0.6","resetsAt":1790856000}}}}"#)
            .expect("관측");
        assert_eq!(obs.weekly, window(Some(50.0), Some(1_790_856_000)));
    }

    // ── 버림 ──

    #[test]
    fn empty_or_missing_info_is_no_observation() {
        assert_eq!(
            parse(r#"{"type":"rate_limit_event","rate_limit_info":{}}"#),
            None
        );
        assert_eq!(parse(r#"{"type":"rate_limit_event"}"#), None);
        assert_eq!(parse(r#"{"rate_limit_info":null}"#), None);
        assert_eq!(parse(r#"{"rate_limit_info":[1,2]}"#), None);
        assert_eq!(parse(r#"{"rate_limit_info":"five_hour"}"#), None);
    }

    #[test]
    fn unknown_window_type_is_dropped() {
        assert_eq!(
            parse(
                r#"{"rate_limit_info":{"rateLimitType":"seven_day_opus","utilization":0.9,"resetsAt":1790856000}}"#
            ),
            None
        );
        assert_eq!(
            parse(
                r#"{"rate_limit_info":{"rateLimitType":"overage","utilization":0.9,"resetsAt":1790856000}}"#
            ),
            None
        );
        // 모르는 종류여도 함께 실린 `unifiedWindows` 는 받는다.
        let obs = parse(r#"{"rate_limit_info":{"rateLimitType":"seven_day_opus","utilization":0.9,"unifiedWindows":{"five_hour":{"utilization":0.2}}}}"#)
            .expect("관측");
        assert_eq!(obs.five_hour, window(Some(20.0), None));
        assert_eq!(obs.weekly, None);
    }

    #[test]
    fn wrong_typed_fields_drop_only_that_field() {
        let obs = parse(r#"{"rate_limit_info":{"rateLimitType":"five_hour","utilization":"55%","resetsAt":1790425800}}"#)
            .expect("관측");
        assert_eq!(obs.five_hour, window(None, Some(1_790_425_800)));

        let obs = parse(r#"{"rate_limit_info":{"rateLimitType":"five_hour","utilization":0.4,"resetsAt":"2026-09-27T00:00:00Z"}}"#)
            .expect("관측");
        assert_eq!(obs.five_hour, window(Some(40.0), None));

        // 창 종류 칸 자체가 틀리면 최상위 창이 없다.
        assert_eq!(
            parse(
                r#"{"rate_limit_info":{"rateLimitType":5,"utilization":0.4,"resetsAt":1790425800}}"#
            ),
            None
        );
        // `unifiedWindows` 가 객체가 아니거나 그 창이 객체가 아니면 그쪽만 없는 셈이다.
        let obs = parse(r#"{"rate_limit_info":{"rateLimitType":"five_hour","utilization":0.4,"unifiedWindows":{"five_hour":[0.9],"seven_day":null}}}"#)
            .expect("관측");
        assert_eq!(obs.five_hour, window(Some(40.0), None));
        assert_eq!(obs.weekly, None);
        let obs = parse(r#"{"rate_limit_info":{"rateLimitType":"five_hour","utilization":0.4,"unifiedWindows":"x"}}"#)
            .expect("관측");
        assert_eq!(obs.five_hour, window(Some(40.0), None));
    }

    #[test]
    fn broken_numbers_never_panic() {
        // 음수·1 초과 → 절단 · 거대 수 → 절단 · 범위 밖 epoch → 그 칸 없음.
        let obs = parse(r#"{"rate_limit_info":{"unifiedWindows":{"five_hour":{"utilization":-0.3,"resetsAt":-5},"seven_day":{"utilization":1e308,"resetsAt":99999999999999999}}}}"#)
            .expect("관측");
        assert_eq!(obs.five_hour, window(Some(0.0), None));
        assert_eq!(obs.weekly, window(Some(100.0), None));

        // 칸이 다 틀리면 창도 관측도 없다.
        assert_eq!(
            parse(
                r#"{"rate_limit_info":{"rateLimitType":"five_hour","utilization":null,"resetsAt":0}}"#
            ),
            None
        );
        assert_eq!(
            parse(
                r#"{"rate_limit_info":{"rateLimitType":"five_hour","utilization":{},"resetsAt":18446744073709551615}}"#
            ),
            None
        );
    }

    // ── 비기본 프로필 판정 ──

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// 데몬 env 대역 — 준 쌍에 없는 키는 「데몬에 없다」.
    fn daemon(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let pairs: Vec<(String, OsString)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    /// ★목록을 글자로 박는다 — [`ACCOUNT_ENV_KEYS`] 를 돌면 키 하나를 지워도 이 시험이 안 깨진다★.
    const EXPECTED_KEYS: [&str; 7] = [
        "CLAUDE_CONFIG_DIR",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
    ];

    #[test]
    fn each_account_key_overridden_does_not_collect() {
        for key in EXPECTED_KEYS {
            assert!(
                !collects_default_account_usage(&env(&[(key, "x")]), daemon(&[])),
                "{key}: 데몬에 없는데 명세에 있다"
            );
            assert!(
                !collects_default_account_usage(&env(&[(key, "x")]), daemon(&[(key, "y")])),
                "{key}: 데몬 값과 다르다"
            );
        }
    }

    #[test]
    fn same_as_daemon_collects() {
        let pairs: Vec<(&str, &str)> = EXPECTED_KEYS.iter().map(|k| (*k, "same")).collect();
        assert!(collects_default_account_usage(&env(&pairs), daemon(&pairs)));
    }

    #[test]
    fn unset_on_both_sides_collects() {
        assert!(collects_default_account_usage(&env(&[]), daemon(&[])));
        assert!(collects_default_account_usage(
            &env(&[("OTHER_ENV", "x")]),
            daemon(&[])
        ));
        // 명세에 없으면 데몬 값을 물려받는다 — 데몬에 무엇이 있든 같은 계정이다.
        assert!(collects_default_account_usage(
            &env(&[]),
            daemon(&[("CLAUDE_CONFIG_DIR", "C:\\cfg"), ("ANTHROPIC_API_KEY", "k")])
        ));
    }

    /// `super::config_dir` 는 빈 값을 「없음」으로 접지만 이 판정은 「있다」로 본다 — 줍지 않는 쪽.
    #[test]
    fn empty_value_counts_as_overridden() {
        assert!(!collects_default_account_usage(
            &env(&[("CLAUDE_CONFIG_DIR", "")]),
            daemon(&[])
        ));
    }

    /// 주우면 디코더를 그대로, 줍지 않으면 막아서 돌려준다 — 데몬 쪽 값은 실제 프로세스 env 다.
    #[test]
    fn gate_decoder_blocks_only_when_not_collecting() {
        use crate::backend::claude::ClaudeStreamDecoder;
        let line = format!("{NEW_SHAPE}\n");

        // 명세가 아무것도 안 바꾸면 프로세스 env 가 무엇이든 줍는다.
        let mut d = gate_decoder(Box::new(ClaudeStreamDecoder::new()), &[]);
        d.decode(line.as_bytes());
        assert_eq!(d.take_usage().len(), 1);

        // 프로세스 env 에 있을 리 없는 값 — 그래서 늘 「다르다」.
        let spec = env(&[(
            "ANTHROPIC_BASE_URL",
            "http://engram-usage-gate-test.invalid",
        )]);
        let mut d = gate_decoder(Box::new(ClaudeStreamDecoder::new()), &spec);
        d.decode(line.as_bytes());
        assert!(d.take_usage().is_empty());
    }
}
