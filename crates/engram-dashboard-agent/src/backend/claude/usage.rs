//! claude's usage limit — interpreting the stream-json `rate_limit_event` line and deciding a non-default profile.
//!
//! ★The pickup shape differs by CLI version and only part of it is documented★ (measured · source =
//!   `docs/research/usage-limit-display-ui-survey-2026-09-26.md` §4-2):
//!   - Old shape (`fixtures/claude_text.jsonl:3`) — has only `rate_limit_info.{rateLimitType, resetsAt}` and
//!     no utilization.
//!   - 2.1.280 capture — `utilization` (0–1) is added to the same top level, and
//!     `unifiedWindows.{five_hour,seven_day}.{utilization,resetsAt}`, which is not in the SDK types, carries both windows.
//!   So both shapes are accepted loosely, and if one field is wrong only that field is dropped.

use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Map, Value};

use crate::transport::OutputDecoder;
use crate::usage::{
    env_overrides_daemon, resets_at_from_json, used_pct_from_fraction, UsageGate, UsageObservation,
    UsageSource, UsageVendorKey, WindowObs,
};

/// The canonical word for claude in the usage field — the same as the wire's backend word (`AgentBackendKind` lowercase).
pub const USAGE_VENDOR: UsageVendorKey = UsageVendorKey::new("claude");

/// If the spec sets even one of these differently from the daemon's own value, that agent's usage is not picked up — these are env that can bring it up with an
/// account · credential · endpoint other than the daemon's default login.
///
/// - `CLAUDE_CONFIG_DIR` = the folder that holds its own state (including the account login) (the very key that [`super::config_dir`] reads).
///   ★An empty value is treated differently from that side★ — that side folds an empty value into 「없음」 (none), but this decision sees it as 「있다」 (present) and compares it with the daemon value
///   byte for byte (if the daemon has no such key, they differ). If it errs, it was chosen to err on the side of not picking up
///   ([`env_overrides_daemon`]).
/// - The rest = credentials (`CLAUDE_CODE_OAUTH_TOKEN`·`ANTHROPIC_API_KEY`·`ANTHROPIC_AUTH_TOKEN`) and endpoints
///   (`ANTHROPIC_BASE_URL`·`CLAUDE_CODE_USE_BEDROCK`·`CLAUDE_CODE_USE_VERTEX`). Even with the same login folder, if these
///   differ there is no guarantee that the agent's limit is that of the default login.
const ACCOUNT_ENV_KEYS: &[&str] = &[
    "CLAUDE_CONFIG_DIR",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
];

/// Whether an unknown window kind was already logged — logged only once in this process (the same line would repeat on every event).
static UNKNOWN_WINDOW_TYPE_LOGGED: AtomicBool = AtomicBool::new(false);

/// Whether a window with no fields left was already logged — once per process, for the same reason as above.
static EMPTY_WINDOW_LOGGED: AtomicBool = AtomicBool::new(false);

/// `rate_limit_event` line (one line already read as JSON) → observation. `None` if no window is carried.
///
/// - If the top-level `rateLimitType` ∈ {`five_hour`, `seven_day`}, the `utilization`·`resetsAt` in the same place are that window.
///   Other kinds are dropped — the per-model weekly windows (`seven_day_opus` etc.) fall here too: this line carries one window at a time, and
///   the observation's `model_scoped: Some` replaces the whole list, so carrying just one would erase the other models' windows.
/// - If `unifiedWindows` has the same window, it wins **field by field**. If its field is missing or wrong, the top-level field is used.
/// - Utilization is 0–1 → 0–100, reset is integer epoch seconds only. Normalization rules = the tools of [`crate::usage`].
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
        limits_unavailable: None,
    })
}

/// The fields of one window object. Dropping field by field is silent, and it is logged only when **no** field is left.
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

/// If both fields are empty, that window is 「안 실렸다」 (not carried) — in a pickup observation, a `None` window leaves the receiver's held value in place.
fn carried(window: WindowObs) -> Option<WindowObs> {
    (window.used_pct.is_some() || window.resets_at.is_some()).then_some(window)
}

/// ★The value is not logged★ — it is a string decided by an external process, so even through masking we cannot decide what might be carried.
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

/// Does the claude started with this spec run under the **same account** as the daemon — `true` only when none of [`ACCOUNT_ENV_KEYS`] differs from
/// the daemon value. If `false`, that agent's pickup is blocked.
/// `daemon_env` = key → that env value of the daemon process itself (production = process env · tests = a stand-in). The per-key comparison rule · the direction when wrong
/// = [`env_overrides_daemon`].
///
/// ★Only env is looked at — the hole this decision cannot see★: the settings file passed with `--settings` or the `env` · `apiKeyHelper` block
///   in the project `.claude/settings.json`. The observation of an agent whose account was changed that way is mixed into the default field.
pub(super) fn collects_default_account_usage(
    spec_env: &[(String, String)],
    daemon_env: impl Fn(&str) -> Option<OsString>,
) -> bool {
    !ACCOUNT_ENV_KEYS
        .iter()
        .any(|key| env_overrides_daemon(spec_env, key, daemon_env(key).as_deref()))
}

/// Set the spawn path's decoder as the pickup decision says — if picked up, return it **as is**, if not, block it with [`UsageGate`].
/// The daemon-side value of the decision reads the process env here.
pub(super) fn gate_decoder(
    decoder: Box<dyn OutputDecoder>,
    spec_env: &[(String, String)],
) -> Box<dyn OutputDecoder> {
    if collects_default_account_usage(spec_env, |key| std::env::var_os(key)) {
        return decoder;
    }
    // Neither which key it was nor the value is logged — they contain credentials and user folder names.
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

    /// Exactly the 2.1.280 capture (no identifiers — research doc §4-2).
    const NEW_SHAPE: &str = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","rateLimitType":"seven_day","utilization":0.83,"resetsAt":1790856000,"surpassedThreshold":0.75,"isUsingOverage":false,"unifiedWindows":{"five_hour":{"utilization":0.67,"resetsAt":1790425800},"seven_day":{"utilization":0.83,"resetsAt":1790856000}}}}"#;

    // ── Shapes ──

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

    /// Old shape — reset only, without utilization. Exactly `fixtures/claude_text.jsonl:3`.
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

        // If its field is wrong, only that field falls back to the top level.
        let obs = parse(r#"{"rate_limit_info":{"rateLimitType":"seven_day","utilization":0.5,"resetsAt":1790000000,"unifiedWindows":{"seven_day":{"utilization":"0.6","resetsAt":1790856000}}}}"#)
            .expect("관측");
        assert_eq!(obs.weekly, window(Some(50.0), Some(1_790_856_000)));
    }

    // ── Dropping ──

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
        // Even for an unknown kind, an accompanying `unifiedWindows` is accepted.
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

        // If the window kind field itself is wrong, there is no top-level window.
        assert_eq!(
            parse(
                r#"{"rate_limit_info":{"rateLimitType":5,"utilization":0.4,"resetsAt":1790425800}}"#
            ),
            None
        );
        // If `unifiedWindows` is not an object, or that window is not an object, it is as if only that side is absent.
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
        // Negative · over 1 → clamp · huge number → clamp · out-of-range epoch → that field absent.
        let obs = parse(r#"{"rate_limit_info":{"unifiedWindows":{"five_hour":{"utilization":-0.3,"resetsAt":-5},"seven_day":{"utilization":1e308,"resetsAt":99999999999999999}}}}"#)
            .expect("관측");
        assert_eq!(obs.five_hour, window(Some(0.0), None));
        assert_eq!(obs.weekly, window(Some(100.0), None));

        // If all fields are wrong, there is neither a window nor an observation.
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

    // ── Non-default profile decision ──

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// Daemon env stand-in — a key not in the given pairs is 「데몬에 없다」 (absent from the daemon).
    fn daemon(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let pairs: Vec<(String, OsString)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    /// ★The list is hard-coded as literals — if it iterated [`ACCOUNT_ENV_KEYS`], this test would not break even if a key were deleted★.
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
        // If it is not in the spec, it inherits the daemon value — the same account whatever the daemon has.
        assert!(collects_default_account_usage(
            &env(&[]),
            daemon(&[("CLAUDE_CONFIG_DIR", "C:\\cfg"), ("ANTHROPIC_API_KEY", "k")])
        ));
    }

    /// `super::config_dir` folds an empty value into 「없음」 (none), but this decision sees it as 「있다」 (present) — the side of not picking up.
    #[test]
    fn empty_value_counts_as_overridden() {
        assert!(!collects_default_account_usage(
            &env(&[("CLAUDE_CONFIG_DIR", "")]),
            daemon(&[])
        ));
    }

    /// If picked up, return the decoder as is; if not, return it blocked — the daemon-side value is the real process env.
    #[test]
    fn gate_decoder_blocks_only_when_not_collecting() {
        use crate::backend::claude::ClaudeStreamDecoder;
        let line = format!("{NEW_SHAPE}\n");

        // If the spec changes nothing, it picks up whatever the process env is.
        let mut d = gate_decoder(Box::new(ClaudeStreamDecoder::new()), &[]);
        d.decode(line.as_bytes());
        assert_eq!(d.take_usage().len(), 1);

        // A value that cannot be in the process env — so always 「다르다」 (different).
        let spec = env(&[(
            "ANTHROPIC_BASE_URL",
            "http://engram-usage-gate-test.invalid",
        )]);
        let mut d = gate_decoder(Box::new(ClaudeStreamDecoder::new()), &spec);
        d.decode(line.as_bytes());
        assert!(d.take_usage().is_empty());
    }
}
