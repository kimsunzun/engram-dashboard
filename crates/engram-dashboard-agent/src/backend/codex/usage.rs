//! codex usage limits — interpretation of the app-server `account/rateLimits/updated` notification and the non-default profile decision.
//! The rules that tell buckets and windows apart ([`DEFAULT_LIMIT_ID`] · [`names_default_bucket`] · [`windows_by_duration`]) are shared
//! with the sibling `usage_probe` (the query response decoder).
//!
//! ★The shape has three sources of different confidence★:
//!   - Field names · types = codex-cli 0.156.1's JSON Schema (certain — that struct's doc in `protocol.rs`).
//!   - `primary`/`secondary` do not mean 5-hour/weekly = `account/rateLimits/read` measurement (certain —
//!     `docs/research/usage-limit-display-ui-survey-2026-09-26.md` §4-1). Windows are told apart by duration (`windowDurationMins`).
//!   - Default bucket id = [`DEFAULT_LIMIT_ID`] (likely — that doc).
//!   Only that this notification arrives during a turn was measured (`docs/process/S21-codex-backend/trd-phase2a.md` L7); the carried
//!   values' shape and the arrival frequency are unmeasured.

use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use super::protocol::{AccountRateLimitsUpdatedNotification, RateLimitSnapshot, RateLimitWindow};
use super::thread_lock::CODEX_HOME_ENV;
use crate::transport::OutputDecoder;
use crate::usage::{
    env_overrides_daemon, resets_at_from_epoch_secs, used_pct_from_percent, UsageGate,
    UsageObservation, UsageSource, UsageVendorKey, WindowObs,
};

/// The canonical word that points at codex in the usage field — the same as the wire's backend word (`AgentBackendKind` lowercase).
pub const USAGE_VENDOR: UsageVendorKey = UsageVendorKey::new("codex");

/// The `limitId` of the account default limit bucket.
///
/// ★Not a value we saw in our own measurement★ — grounds = the schema description (`rateLimitsByLimitId`: "keyed by metered `limit_id`
///   (for example, `codex`)") and the peer t3code using it as the default bucket. If wrong, pickup emits nothing (the query fills the value) —
///   better than values of another bucket mixing into the default field.
pub(super) const DEFAULT_LIMIT_ID: &str = "codex";

const FIVE_HOUR_WINDOW_MINS: i64 = 5 * 60;

const WEEKLY_WINDOW_MINS: i64 = 7 * 24 * 60;

/// Logged once per process — an unknown window length (else one line per notification).
static UNKNOWN_WINDOW_DURATION_LOGGED: AtomicBool = AtomicBool::new(false);

/// Logged once per process — a non-default bucket.
static OTHER_BUCKET_LOGGED: AtomicBool = AtomicBool::new(false);

/// Logged once per process — a window with no fields left.
static EMPTY_WINDOW_LOGGED: AtomicBool = AtomicBool::new(false);

/// Logged once per process — a notification without `limitId` read as the default bucket.
static MISSING_LIMIT_ID_LOGGED: AtomicBool = AtomicBool::new(false);

/// `account/rateLimits/updated` notification → observation. `None` if it is not the default bucket or no window is carried.
///
/// - Windows are told apart **by duration**: 300 min → 5-hour · 10080 min → weekly · anything else (including no duration) is discarded.
///   If two windows have the same duration, the front slot (`primary`) **among those carrying a value** wins; a front slot with no usable
///   value is filled by the back slot (`secondary`). Two of the same duration signals that upstream meaning changed, and the query gives
///   the whole thing soon.
/// - A non-default bucket (per-model limit) is discarded whole. ★It is not moved into a per-model window★ — one notification carries one
///   bucket, but the observation's `model_scoped: Some` fully replaces the list, so it would erase the other model windows. Moved into the
///   default window, that value would look like the account limit.
/// - `planType` is not carried over — the query carries the plan.
/// - Usage is already 0–100, reset is epoch seconds. Normalization = the tools of [`crate::usage`].
pub(super) fn observation_from_rate_limits_updated(
    notification: &AccountRateLimitsUpdatedNotification,
) -> Option<UsageObservation> {
    let snapshot = &notification.rate_limits;
    if !is_default_bucket(snapshot.limit_id.as_ref()) {
        note_other_bucket();
        return None;
    }
    let (five_hour, weekly) = windows_by_duration(snapshot);
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

/// ★Reading a notification without `limitId` as the default bucket is an unmeasured assumption★ (rule = [`names_default_bucket`]) — we
///   have not seen this notification without that field. So we write once when using the assumption (if wrong, per-model bucket values mix
///   into the default field).
fn is_default_bucket(limit_id: Option<&Value>) -> bool {
    if limit_id.is_none() {
        note_missing_limit_id();
    }
    names_default_bucket(limit_id)
}

/// Does the snapshot's `limitId` point at the default bucket. Compared literally.
/// ★If absent or `null`, it is read as the default bucket★ — the pre-split single-bucket shape (the schema says the query response's
///   `rateLimits` carries it) has no such field. If the type is wrong we do not know which bucket it is, so `false`.
pub(super) fn names_default_bucket(limit_id: Option<&Value>) -> bool {
    match limit_id {
        None | Some(Value::Null) => true,
        Some(Value::String(id)) => id == DEFAULT_LIMIT_ID,
        Some(_) => false,
    }
}

pub(super) fn windows_by_duration(
    snapshot: &RateLimitSnapshot,
) -> (Option<WindowObs>, Option<WindowObs>) {
    let mut five_hour = None;
    let mut weekly = None;
    for window in [&snapshot.primary, &snapshot.secondary]
        .into_iter()
        .flatten()
    {
        let slot = match window.window_duration_mins {
            Some(FIVE_HOUR_WINDOW_MINS) => &mut five_hour,
            Some(WEEKLY_WINDOW_MINS) => &mut weekly,
            other => {
                note_unknown_duration(other);
                continue;
            }
        };
        if slot.is_none() {
            *slot = carried(window);
        }
    }
    (five_hour, weekly)
}

/// If both fields are empty, that window is `None` — meaning set by the observation's source (pickup = not carried · query = absent —
/// [`UsageObservation`]). Dropping field by field is quiet; only when **no** field is left is it logged.
fn carried(window: &RateLimitWindow) -> Option<WindowObs> {
    let obs = WindowObs {
        used_pct: window.used_percent.and_then(used_pct_from_percent),
        resets_at: window.resets_at.and_then(resets_at_from_epoch_secs),
    };
    if obs.used_pct.is_none() && obs.resets_at.is_none() {
        note_empty_window();
        return None;
    }
    Some(obs)
}

fn note_unknown_duration(minutes: Option<i64>) {
    if UNKNOWN_WINDOW_DURATION_LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    tracing::debug!(
        window_duration_mins = ?minutes,
        "dropping an unknown window length in a codex usage-limit snapshot (logged only once per process)"
    );
}

fn note_empty_window() {
    if EMPTY_WINDOW_LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    tracing::debug!(
        "dropping a codex usage-limit snapshot window with no readable field (logged only once per process)"
    );
}

fn note_missing_limit_id() {
    if MISSING_LIMIT_ID_LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    tracing::debug!(
        "codex account/rateLimits/updated has no limitId, reading it as the default bucket — unmeasured assumption (logged only once per process)"
    );
}

/// ★The value is not written★ — it is a string decided by an external process, so even after masking we cannot decide what will be carried.
fn note_other_bucket() {
    if OTHER_BUCKET_LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    tracing::debug!(
        "dropping a non-default bucket of codex account/rateLimits/updated (logged only once per process)"
    );
}

/// If the spec sets any one of these differently from the daemon's own value, pickup is skipped for that agent — these envs can bring it
/// up on an account · credential · endpoint other than the daemon's default login.
///
/// - [`CODEX_HOME_ENV`] = moves the whole state folder (including the account login).
/// - The rest = credentials (`OPENAI_API_KEY` · `CODEX_API_KEY`) and an endpoint (`OPENAI_BASE_URL`) — with the same state folder, differing
///   values still give no guarantee the limit is the default login's.
const ACCOUNT_ENV_KEYS: &[&str] = &[
    CODEX_HOME_ENV,
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "OPENAI_BASE_URL",
];

/// Does the codex launched with this spec run on the **same account** as the daemon — `true` only when none of [`ACCOUNT_ENV_KEYS`]
/// differs from the daemon's value; `false` blocks pickup for that agent.
/// `daemon_env` = key → that env value of the daemon process itself (production = the process env · test = a stand-in). Per-key comparison
/// rule and the direction of erring = [`env_overrides_daemon`].
///
/// ★Only env is looked at — a hole this decision cannot see★: `-c` (config override) · `--profile` in the profile's extra arguments.
///   Observations of an agent that changed account · endpoint that way mix into the default field.
pub(super) fn collects_default_account_usage(
    spec_env: &[(String, String)],
    daemon_env: impl Fn(&str) -> Option<OsString>,
) -> bool {
    !ACCOUNT_ENV_KEYS
        .iter()
        .any(|key| env_overrides_daemon(spec_env, key, daemon_env(key).as_deref()))
}

/// Set the spawn path's decoder per the pickup decision — if it picks up, return it **as is**; if not, block it with [`UsageGate`].
/// The decision's daemon-side value is read here from the process env.
pub(super) fn gate_decoder(
    decoder: Box<dyn OutputDecoder>,
    spec_env: &[(String, String)],
) -> Box<dyn OutputDecoder> {
    if collects_default_account_usage(spec_env, |key| std::env::var_os(key)) {
        return decoder;
    }
    // Neither which key it was nor its value is written — credentials and user folder names are in there.
    tracing::debug!(
        "codex env may not be the default account — turning off usage pickup for this agent"
    );
    Box::new(UsageGate::blocking(decoder))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(params: &str) -> Option<UsageObservation> {
        let value: Value = serde_json::from_str(params).expect("test params are JSON");
        let notification: AccountRateLimitsUpdatedNotification =
            serde_json::from_value(value).expect("the envelope reads");
        observation_from_rate_limits_updated(&notification)
    }

    fn parse_snapshot(snapshot: &str) -> Option<UsageObservation> {
        parse(&format!(r#"{{"rateLimits":{snapshot}}}"#))
    }

    fn window(used_pct: Option<f64>, resets_at: Option<i64>) -> Option<WindowObs> {
        Some(WindowObs {
            used_pct,
            resets_at,
        })
    }

    /// The default bucket of the query measurement (research doc §4-1) — the numbers are that measured value and `limitId` is not in the
    ///   measurement so we put in [`DEFAULT_LIMIT_ID`].
    const DEFAULT_BUCKET: &str = r#"{"limitId":"codex","planType":"plus","primary":{"usedPercent":37,"windowDurationMins":300,"resetsAt":1790424706},"secondary":{"usedPercent":58,"windowDurationMins":10080,"resetsAt":1790739378},"credits":null}"#;

    // ── Shape ──

    #[test]
    fn default_bucket_carries_both_windows_by_duration() {
        let obs = parse_snapshot(DEFAULT_BUCKET).expect("observation");
        assert_eq!(obs.vendor, USAGE_VENDOR);
        assert_eq!(obs.source, UsageSource::Passive);
        assert_eq!(obs.five_hour, window(Some(37.0), Some(1_790_424_706)));
        assert_eq!(obs.weekly, window(Some(58.0), Some(1_790_739_378)));
        assert_eq!(
            obs.model_scoped, None,
            "pickup does not carry per-model windows"
        );
        assert_eq!(obs.plan, None, "the query carries the plan");
    }

    /// ★Duration tells them apart, not slot name★ — a bucket where `primary` is weekly was measured (research doc §4-1).
    #[test]
    fn primary_can_be_the_weekly_window() {
        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":12,"windowDurationMins":10080,"resetsAt":1791018755},"secondary":null}"#,
        )
        .expect("observation");
        assert_eq!(obs.five_hour, None);
        assert_eq!(obs.weekly, window(Some(12.0), Some(1_791_018_755)));

        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":58,"windowDurationMins":10080},"secondary":{"usedPercent":37,"windowDurationMins":300}}"#,
        )
        .expect("observation");
        assert_eq!(obs.five_hour, window(Some(37.0), None));
        assert_eq!(obs.weekly, window(Some(58.0), None));
    }

    #[test]
    fn a_missing_limit_id_reads_as_the_default_bucket() {
        let obs = parse_snapshot(r#"{"primary":{"usedPercent":5,"windowDurationMins":300}}"#)
            .expect("observation");
        assert_eq!(obs.five_hour, window(Some(5.0), None));
        let obs = parse_snapshot(
            r#"{"limitId":null,"primary":{"usedPercent":5,"windowDurationMins":300}}"#,
        )
        .expect("observation");
        assert_eq!(obs.five_hour, window(Some(5.0), None));
    }

    /// Sparse update — if only one window comes, the other window is 「not carried」.
    #[test]
    fn a_sparse_update_carries_only_what_came() {
        let obs = parse_snapshot(
            r#"{"limitId":"codex","secondary":{"usedPercent":90,"windowDurationMins":10080}}"#,
        )
        .expect("observation");
        assert_eq!(obs.five_hour, None);
        assert_eq!(obs.weekly, window(Some(90.0), None));
    }

    #[test]
    fn same_duration_twice_keeps_the_first_slot() {
        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":10,"windowDurationMins":300},"secondary":{"usedPercent":99,"windowDurationMins":300}}"#,
        )
        .expect("observation");
        assert_eq!(obs.five_hour, window(Some(10.0), None));
        assert_eq!(obs.weekly, None);

        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":"10","windowDurationMins":300},"secondary":{"usedPercent":99,"windowDurationMins":300}}"#,
        )
        .expect("observation");
        assert_eq!(obs.five_hour, window(Some(99.0), None));
    }

    // ── Discard ──

    #[test]
    fn null_or_absent_windows_are_no_observation() {
        assert_eq!(parse_snapshot(r#"{"primary":null,"secondary":null}"#), None);
        assert_eq!(
            parse_snapshot(r#"{"limitId":"codex","planType":"plus"}"#),
            None
        );
        assert_eq!(parse_snapshot("{}"), None);
    }

    #[test]
    fn unknown_or_missing_duration_is_dropped() {
        assert_eq!(
            parse_snapshot(r#"{"primary":{"usedPercent":40,"windowDurationMins":43200}}"#),
            None,
            "monthly window — not one of our fields"
        );
        assert_eq!(
            parse_snapshot(r#"{"primary":{"usedPercent":40,"windowDurationMins":60}}"#),
            None
        );
        assert_eq!(
            parse_snapshot(r#"{"primary":{"usedPercent":40,"resetsAt":1790424706}}"#),
            None,
            "without a length, do not guess from the slot name"
        );
        assert_eq!(
            parse_snapshot(r#"{"primary":{"usedPercent":40,"windowDurationMins":null}}"#),
            None
        );
        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":40,"windowDurationMins":43200},"secondary":{"usedPercent":7,"windowDurationMins":10080}}"#,
        )
        .expect("observation");
        assert_eq!(obs.five_hour, None);
        assert_eq!(obs.weekly, window(Some(7.0), None));
    }

    /// A per-model bucket — in the measured shape (`base_model_inference`, research doc §4-1) `primary` is weekly, and that value must not
    ///   go into the account weekly field.
    #[test]
    fn other_buckets_are_dropped_whole() {
        assert_eq!(
            parse_snapshot(
                r#"{"limitId":"base_model_inference","limitName":"gpt-reserve","primary":{"usedPercent":0,"windowDurationMins":10080,"resetsAt":1791018755},"secondary":null}"#
            ),
            None
        );
        assert_eq!(
            parse_snapshot(
                r#"{"limitId":"Codex","primary":{"usedPercent":1,"windowDurationMins":300}}"#
            ),
            None,
            "the bucket id is compared character for character"
        );
        // If the type of the bucket field is wrong, we cannot tell which bucket it is.
        for bad in ["5", "true", "[\"codex\"]", "{\"id\":\"codex\"}"] {
            assert_eq!(
                parse_snapshot(&format!(
                    r#"{{"limitId":{bad},"primary":{{"usedPercent":1,"windowDurationMins":300}}}}"#
                )),
                None,
                "{bad}"
            );
        }
    }

    #[test]
    fn wrong_typed_fields_drop_only_that_field() {
        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":"37","windowDurationMins":300,"resetsAt":1790424706}}"#,
        )
        .expect("observation");
        assert_eq!(obs.five_hour, window(None, Some(1_790_424_706)));

        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":37,"windowDurationMins":300,"resetsAt":"2026-09-27T00:00:00Z"}}"#,
        )
        .expect("observation");
        assert_eq!(obs.five_hour, window(Some(37.0), None));

        // A reset or duration with a fractional part is not an integer — only that field is missing (without a duration the window cannot
        //   be told apart).
        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":37.5,"windowDurationMins":300,"resetsAt":1790424706.0}}"#,
        )
        .expect("observation");
        assert_eq!(obs.five_hour, window(Some(37.5), None));
        assert_eq!(
            parse_snapshot(r#"{"primary":{"usedPercent":37,"windowDurationMins":300.0}}"#),
            None
        );
        assert_eq!(
            parse_snapshot(r#"{"primary":{"usedPercent":37,"windowDurationMins":"300"}}"#),
            None
        );

        // If the window slot is not an object, only that window is missing — we do not read an array in field order and make a fake window.
        for bad in ["[37,300,1790424706]", "\"x\"", "5", "true"] {
            let obs = parse_snapshot(&format!(
                r#"{{"primary":{bad},"secondary":{{"usedPercent":7,"windowDurationMins":10080}}}}"#
            ))
            .unwrap_or_else(|| panic!("{bad}: the neighbouring window survives"));
            assert_eq!(obs.five_hour, None, "{bad}");
            assert_eq!(obs.weekly, window(Some(7.0), None), "{bad}");
        }
    }

    #[test]
    fn broken_numbers_never_panic() {
        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":-30,"windowDurationMins":300,"resetsAt":-5},"secondary":{"usedPercent":1e308,"windowDurationMins":10080,"resetsAt":99999999999999999}}"#,
        )
        .expect("observation");
        assert_eq!(obs.five_hour, window(Some(0.0), None));
        assert_eq!(obs.weekly, window(Some(100.0), None));

        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":18446744073709551615,"windowDurationMins":300,"resetsAt":18446744073709551615}}"#,
        )
        .expect("observation");
        assert_eq!(obs.five_hour, window(Some(100.0), None), "u64 max");

        assert_eq!(
            parse_snapshot(
                r#"{"primary":{"usedPercent":1,"windowDurationMins":18446744073709551615}}"#
            ),
            None
        );
        assert_eq!(
            parse_snapshot(r#"{"primary":{"usedPercent":1,"windowDurationMins":-300}}"#),
            None
        );

        // If all fields are wrong, there is no window and no observation (epoch 0 is the 「no value」 placeholder).
        assert_eq!(
            parse_snapshot(
                r#"{"primary":{"usedPercent":null,"windowDurationMins":300,"resetsAt":0}}"#
            ),
            None
        );
        assert_eq!(
            parse_snapshot(
                r#"{"primary":{"usedPercent":{},"windowDurationMins":300,"resetsAt":[1]}}"#
            ),
            None
        );
        assert_eq!(
            parse_snapshot(r#"{"primary":{"windowDurationMins":300}}"#),
            None,
            "no panic even if the required field `usedPercent` is missing"
        );
    }

    // ── Non-default profile decision ──

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// Daemon env stand-in — a key not in the given pairs is 「absent in the daemon」.
    fn daemon(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let pairs: Vec<(String, OsString)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    /// ★Pin the list as literal text — if we iterate [`ACCOUNT_ENV_KEYS`], deleting one key would not break this test★.
    const EXPECTED_KEYS: [&str; 4] = [
        "CODEX_HOME",
        "OPENAI_API_KEY",
        "CODEX_API_KEY",
        "OPENAI_BASE_URL",
    ];

    #[test]
    fn each_account_key_overridden_does_not_collect() {
        for key in EXPECTED_KEYS {
            assert!(
                !collects_default_account_usage(&env(&[(key, "x")]), daemon(&[])),
                "{key}: absent in the daemon but present in the spec"
            );
            assert!(
                !collects_default_account_usage(&env(&[(key, "x")]), daemon(&[(key, "y")])),
                "{key}: differs from the daemon value"
            );
        }
        // Key case is not distinguished — the spawn loads it into the child env under that name.
        assert!(!collects_default_account_usage(
            &env(&[("codex_home", "D:\\alt-codex")]),
            daemon(&[("CODEX_HOME", "C:\\codex-home")])
        ));
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
            &env(&[("CLAUDE_CONFIG_DIR", "C:\\other")]),
            daemon(&[])
        ));
        // If the spec does not have it, the daemon value is inherited — whatever the daemon has, it is the same account.
        assert!(collects_default_account_usage(
            &env(&[]),
            daemon(&[("CODEX_HOME", "C:\\codex-home"), ("OPENAI_API_KEY", "k")])
        ));
    }

    #[test]
    fn gate_decoder_blocks_only_when_not_collecting() {
        use crate::backend::codex::decoder::CodexAppServerDecoder;
        let line = format!(
            "{}\n",
            serde_json::json!({
                "method": "account/rateLimits/updated",
                "params": {"rateLimits": serde_json::from_str::<Value>(DEFAULT_BUCKET).expect("json")},
            })
        );

        let mut d = gate_decoder(Box::new(CodexAppServerDecoder::new()), &[]);
        d.decode(line.as_bytes());
        assert_eq!(d.take_usage().len(), 1);

        // A value that cannot be in the process env — so it is always 「different」.
        let spec = env(&[("OPENAI_BASE_URL", "http://engram-usage-gate-test.invalid")]);
        let mut d = gate_decoder(Box::new(CodexAppServerDecoder::new()), &spec);
        d.decode(line.as_bytes());
        assert!(d.take_usage().is_empty());
    }
}
