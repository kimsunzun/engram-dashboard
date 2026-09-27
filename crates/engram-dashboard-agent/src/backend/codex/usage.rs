//! codex 의 사용량 한도 — app-server `account/rateLimits/updated` 알림 해석과 비기본 프로필 판정. 버킷·창을
//! 가르는 규칙([`DEFAULT_LIMIT_ID`]·[`names_default_bucket`]·[`windows_by_duration`])은 조회 응답을 푸는 형제
//! `usage_probe` 도 쓴다 — 두 출처가 같은 규칙으로 창을 가른다.
//!
//! ★모양의 출처가 셋이고 확신도가 다르다★:
//!   - 칸 이름·타입 = codex-cli 0.156.1 의 JSON Schema(확실 — `protocol.rs` 의 그 구조체 doc).
//!   - `primary`/`secondary` 가 5시간/주간을 뜻하지 않는다 = `account/rateLimits/read` 실측(확실 —
//!     `docs/research/usage-limit-display-ui-survey-2026-09-26.md` §4-1). 창은 길이(`windowDurationMins`)로 가른다.
//!   - 기본 버킷 id = [`DEFAULT_LIMIT_ID`](가능성 높음 — 그 doc).
//!   이 알림이 턴 중에 온다는 것까지만 실측됐다(`docs/process/S21-codex-backend/trd-phase2a.md` L7) — 실린 값의
//!   모양과 도착 빈도는 미측정이다.

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

/// 사용량 칸에서 codex 를 가리키는 정본 낱말 — wire 의 백엔드 낱말(`AgentBackendKind` 소문자)과 같다.
pub const USAGE_VENDOR: UsageVendorKey = UsageVendorKey::new("codex");

/// 계정 기본 한도 버킷의 `limitId`.
///
/// ★우리 실측으로 본 값이 아니다★ — 근거 = 스키마 설명(`rateLimitsByLimitId`: "keyed by metered `limit_id`
///   (for example, `codex`)")과 피어 t3code 가 이 값을 기본 버킷으로 고르는 것. 틀렸다면 증상 = 줍기가 아무것도
///   안 낸다(값은 조회가 채운다). 다른 버킷의 값이 기본 칸에 섞이는 반대쪽보다 그 증상이 낫다.
pub(super) const DEFAULT_LIMIT_ID: &str = "codex";

/// 5시간 창의 길이(분).
const FIVE_HOUR_WINDOW_MINS: i64 = 5 * 60;

/// 주간 창의 길이(분).
const WEEKLY_WINDOW_MINS: i64 = 7 * 24 * 60;

/// 모르는 창 길이를 이미 로그에 적었나 — 이 프로세스에서 한 번만 적는다(알림마다 같은 줄이 반복된다).
static UNKNOWN_WINDOW_DURATION_LOGGED: AtomicBool = AtomicBool::new(false);

/// 기본이 아닌 버킷을 이미 로그에 적었나 — 위와 같은 이유로 프로세스당 한 번.
static OTHER_BUCKET_LOGGED: AtomicBool = AtomicBool::new(false);

/// 칸이 하나도 안 남은 창을 이미 로그에 적었나 — 위와 같은 이유로 프로세스당 한 번.
static EMPTY_WINDOW_LOGGED: AtomicBool = AtomicBool::new(false);

/// `limitId` 없는 알림을 기본 버킷으로 읽었다고 이미 적었나 — 위와 같은 이유로 프로세스당 한 번.
static MISSING_LIMIT_ID_LOGGED: AtomicBool = AtomicBool::new(false);

/// `account/rateLimits/updated` 알림 → 관측. 기본 버킷이 아니거나 실린 창이 하나도 없으면 `None`.
///
/// - 창은 **길이로** 가른다: 300분 → 5시간 · 10080분 → 주간 · 그 밖(길이 없음 포함)은 버린다. 같은 길이의 창이
///   둘이면 **값이 실린 쪽 중** 앞자리(`primary`)가 이긴다 — 앞자리가 값 없이 오면(칸이 다 비거나 틀리면) 그
///   창은 뒷자리(`secondary`)가 채운다. 같은 길이가 둘인 것은 상류 의미가 바뀐 신호라 어느 쪽이 맞는지 모르고,
///   조회가 곧 전체를 준다.
/// - 기본이 아닌 버킷(모델별 한도)은 통째로 버린다. ★모델별 창으로 옮기지 않는다★ — 알림 하나는 버킷 하나만
///   싣는데 관측의 `model_scoped: Some` 은 목록 전량 교체라, 싣는 순간 나머지 모델 창을 지운다. 기본 창으로 옮기면
///   그 버킷의 값이 계정 한도로 보인다.
/// - `planType` 은 옮기지 않는다 — 플랜은 조회가 싣는다.
/// - 사용률은 이미 0–100, 리셋은 epoch 초. 정규화 규칙 = [`crate::usage`] 의 도구들.
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
    })
}

/// ★`limitId` 없는 알림을 기본 버킷으로 읽는 것은 미측정 가정이다★(규칙 = [`names_default_bucket`]) — 이 알림이
///   그 칸 없이 오는 것을 본 적이 없다. 그래서 그 가정을 쓸 때 한 번 적는다(틀렸다면 증상 = 모델별 버킷의 값이 기본
///   칸에 섞인다).
fn is_default_bucket(limit_id: Option<&Value>) -> bool {
    if limit_id.is_none() {
        note_missing_limit_id();
    }
    names_default_bucket(limit_id)
}

/// 스냅숏의 `limitId` 가 기본 버킷을 가리키나. 글자 그대로 대조한다.
/// ★없거나 `null` 이면 기본 버킷으로 읽는다★ — 버킷이 나뉘기 전 모양(조회 응답의 `rateLimits` 가 그 옛 단일 버킷
///   모양을 그대로 싣는다고 스키마가 적는다)이 이 칸을 싣지 않는다. 타입이 틀리면 어느 버킷인지 모르므로 `false`.
pub(super) fn names_default_bucket(limit_id: Option<&Value>) -> bool {
    match limit_id {
        None | Some(Value::Null) => true,
        Some(Value::String(id)) => id == DEFAULT_LIMIT_ID,
        Some(_) => false,
    }
}

/// 스냅숏의 두 자리 → `(5시간, 주간)`. 규칙 = [`observation_from_rate_limits_updated`] 의 첫 항목.
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

/// 두 칸이 다 비면 그 창은 `None` 이다 — 그 뜻은 관측의 출처가 정한다(줍기 = 안 실렸다 · 조회 = 없다 —
/// [`UsageObservation`]). 칸마다 따로 버리는 것은 조용하고, 칸이 **하나도** 안 남으면 그때만 로그에 적는다.
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
        "codex 사용량 한도 스냅숏의 모르는 창 길이를 버린다(프로세스당 한 번만 적는다)"
    );
}

fn note_empty_window() {
    if EMPTY_WINDOW_LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    tracing::debug!(
        "codex 사용량 한도 스냅숏의 창에 읽히는 칸이 하나도 없어 버린다(프로세스당 한 번만 적는다)"
    );
}

fn note_missing_limit_id() {
    if MISSING_LIMIT_ID_LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    tracing::debug!(
        "codex account/rateLimits/updated 에 limitId 가 없어 기본 버킷으로 읽는다 — 미측정 가정(프로세스당 한 번만 적는다)"
    );
}

/// ★값을 적지 않는다★ — 외부 프로세스가 정한 문자열이라 마스킹을 거쳐도 무엇이 실릴지 우리가 정하지 못한다.
fn note_other_bucket() {
    if OTHER_BUCKET_LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    tracing::debug!(
        "codex account/rateLimits/updated 의 기본이 아닌 버킷이라 버린다(프로세스당 한 번만 적는다)"
    );
}

/// 명세가 이 중 하나라도 데몬 자신의 값과 다르게 두면 그 에이전트는 줍지 않는다 — 데몬의 기본 로그인이 아닌
/// 계정·자격증명·엔드포인트로 뜰 수 있는 env 다.
///
/// - [`CODEX_HOME_ENV`] = 상태 폴더(계정 로그인 포함)를 통째로 옮긴다.
/// - 나머지 = 자격증명(`OPENAI_API_KEY`·`CODEX_API_KEY`)과 엔드포인트(`OPENAI_BASE_URL`). 상태 폴더가 같아도
///   이것들이 다르면 그 에이전트의 한도가 기본 로그인의 것이라는 보장이 없다.
const ACCOUNT_ENV_KEYS: &[&str] = &[
    CODEX_HOME_ENV,
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "OPENAI_BASE_URL",
];

/// 이 명세로 뜬 codex 가 데몬과 **같은 계정**으로 도나 — [`ACCOUNT_ENV_KEYS`] 가 하나도 데몬 값과 다르지
/// 않을 때만 `true`. `false` 면 그 에이전트의 줍기를 막는다.
/// `daemon_env` = 키 → 데몬 프로세스 자신의 그 env 값(운영 = 프로세스 env · 시험 = 대역). 키마다 비교 규칙·틀릴
/// 때의 방향 = [`env_overrides_daemon`].
///
/// ★env 만 본다 — 이 판정이 못 보는 구멍★: 프로필의 추가 인자로 넘긴 `-c`(설정 덮어쓰기)·`--profile`. 그렇게
///   계정·엔드포인트를 바꾼 에이전트의 관측은 기본 칸에 섞인다.
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
    tracing::debug!("codex 기본 계정이 아닐 수 있는 env — 이 에이전트의 사용량 줍기를 끈다");
    Box::new(UsageGate::blocking(decoder))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(params: &str) -> Option<UsageObservation> {
        let value: Value = serde_json::from_str(params).expect("시험 params 는 JSON 이다");
        let notification: AccountRateLimitsUpdatedNotification =
            serde_json::from_value(value).expect("봉투는 읽힌다");
        observation_from_rate_limits_updated(&notification)
    }

    /// `{"rateLimits": <snapshot>}` — 스냅숏만 바꿔 끼운다.
    fn parse_snapshot(snapshot: &str) -> Option<UsageObservation> {
        parse(&format!(r#"{{"rateLimits":{snapshot}}}"#))
    }

    fn window(used_pct: Option<f64>, resets_at: Option<i64>) -> Option<WindowObs> {
        Some(WindowObs {
            used_pct,
            resets_at,
        })
    }

    /// 조회 실측(조사 문서 §4-1)의 기본 버킷 — 수치는 그 실측값이고 `limitId` 는 실측에 없어 [`DEFAULT_LIMIT_ID`] 를 넣었다.
    const DEFAULT_BUCKET: &str = r#"{"limitId":"codex","planType":"plus","primary":{"usedPercent":37,"windowDurationMins":300,"resetsAt":1790424706},"secondary":{"usedPercent":58,"windowDurationMins":10080,"resetsAt":1790739378},"credits":null}"#;

    // ── 모양 ──

    #[test]
    fn default_bucket_carries_both_windows_by_duration() {
        let obs = parse_snapshot(DEFAULT_BUCKET).expect("관측");
        assert_eq!(obs.vendor, USAGE_VENDOR);
        assert_eq!(obs.source, UsageSource::Passive);
        assert_eq!(obs.five_hour, window(Some(37.0), Some(1_790_424_706)));
        assert_eq!(obs.weekly, window(Some(58.0), Some(1_790_739_378)));
        assert_eq!(obs.model_scoped, None, "줍기는 모델별 창을 싣지 않는다");
        assert_eq!(obs.plan, None, "플랜은 조회가 싣는다");
    }

    /// ★자리 이름이 아니라 길이가 가른다★ — `primary` 가 주간인 버킷이 실측됐다(조사 문서 §4-1).
    #[test]
    fn primary_can_be_the_weekly_window() {
        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":12,"windowDurationMins":10080,"resetsAt":1791018755},"secondary":null}"#,
        )
        .expect("관측");
        assert_eq!(obs.five_hour, None);
        assert_eq!(obs.weekly, window(Some(12.0), Some(1_791_018_755)));

        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":58,"windowDurationMins":10080},"secondary":{"usedPercent":37,"windowDurationMins":300}}"#,
        )
        .expect("관측");
        assert_eq!(obs.five_hour, window(Some(37.0), None));
        assert_eq!(obs.weekly, window(Some(58.0), None));
    }

    #[test]
    fn a_missing_limit_id_reads_as_the_default_bucket() {
        let obs = parse_snapshot(r#"{"primary":{"usedPercent":5,"windowDurationMins":300}}"#)
            .expect("관측");
        assert_eq!(obs.five_hour, window(Some(5.0), None));
        let obs = parse_snapshot(
            r#"{"limitId":null,"primary":{"usedPercent":5,"windowDurationMins":300}}"#,
        )
        .expect("관측");
        assert_eq!(obs.five_hour, window(Some(5.0), None));
    }

    /// 희소 갱신 — 한 창만 오면 다른 창은 「안 실렸다」다.
    #[test]
    fn a_sparse_update_carries_only_what_came() {
        let obs = parse_snapshot(
            r#"{"limitId":"codex","secondary":{"usedPercent":90,"windowDurationMins":10080}}"#,
        )
        .expect("관측");
        assert_eq!(obs.five_hour, None);
        assert_eq!(obs.weekly, window(Some(90.0), None));
    }

    #[test]
    fn same_duration_twice_keeps_the_first_slot() {
        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":10,"windowDurationMins":300},"secondary":{"usedPercent":99,"windowDurationMins":300}}"#,
        )
        .expect("관측");
        assert_eq!(obs.five_hour, window(Some(10.0), None));
        assert_eq!(obs.weekly, None);

        // 앞자리가 값 없이 오면 그 창은 뒷자리가 채운다.
        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":"10","windowDurationMins":300},"secondary":{"usedPercent":99,"windowDurationMins":300}}"#,
        )
        .expect("관측");
        assert_eq!(obs.five_hour, window(Some(99.0), None));
    }

    // ── 버림 ──

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
            "월간 창 — 우리 칸에 없다"
        );
        assert_eq!(
            parse_snapshot(r#"{"primary":{"usedPercent":40,"windowDurationMins":60}}"#),
            None
        );
        assert_eq!(
            parse_snapshot(r#"{"primary":{"usedPercent":40,"resetsAt":1790424706}}"#),
            None,
            "길이가 없으면 자리 이름으로 추측하지 않는다"
        );
        assert_eq!(
            parse_snapshot(r#"{"primary":{"usedPercent":40,"windowDurationMins":null}}"#),
            None
        );
        // 모르는 길이 옆의 아는 길이는 산다.
        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":40,"windowDurationMins":43200},"secondary":{"usedPercent":7,"windowDurationMins":10080}}"#,
        )
        .expect("관측");
        assert_eq!(obs.five_hour, None);
        assert_eq!(obs.weekly, window(Some(7.0), None));
    }

    /// 모델별 버킷 — 실측 모양(조사 문서 §4-1의 `base_model_inference`)이면 `primary` 가 주간인데, 그 값이 계정
    /// 주간 칸에 들어가면 안 된다.
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
            "버킷 id 는 글자 그대로 대조한다"
        );
        // 버킷 칸의 타입이 틀리면 어느 버킷인지 모른다.
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
        .expect("관측");
        assert_eq!(obs.five_hour, window(None, Some(1_790_424_706)));

        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":37,"windowDurationMins":300,"resetsAt":"2026-09-27T00:00:00Z"}}"#,
        )
        .expect("관측");
        assert_eq!(obs.five_hour, window(Some(37.0), None));

        // 소수점 있는 리셋·길이는 정수가 아니다 — 그 칸만 없다(길이가 없으면 창을 못 가른다).
        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":37.5,"windowDurationMins":300,"resetsAt":1790424706.0}}"#,
        )
        .expect("관측");
        assert_eq!(obs.five_hour, window(Some(37.5), None));
        assert_eq!(
            parse_snapshot(r#"{"primary":{"usedPercent":37,"windowDurationMins":300.0}}"#),
            None
        );
        assert_eq!(
            parse_snapshot(r#"{"primary":{"usedPercent":37,"windowDurationMins":"300"}}"#),
            None
        );

        // 창 자리가 객체가 아니면 그 창만 없다 — 배열을 칸 순서대로 읽어 가짜 창을 만들지 않는다.
        for bad in ["[37,300,1790424706]", "\"x\"", "5", "true"] {
            let obs = parse_snapshot(&format!(
                r#"{{"primary":{bad},"secondary":{{"usedPercent":7,"windowDurationMins":10080}}}}"#
            ))
            .unwrap_or_else(|| panic!("{bad}: 옆 창은 산다"));
            assert_eq!(obs.five_hour, None, "{bad}");
            assert_eq!(obs.weekly, window(Some(7.0), None), "{bad}");
        }
    }

    #[test]
    fn broken_numbers_never_panic() {
        // 음수 → 0 · 거대 수 → 100 · 범위 밖 epoch → 그 칸 없음.
        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":-30,"windowDurationMins":300,"resetsAt":-5},"secondary":{"usedPercent":1e308,"windowDurationMins":10080,"resetsAt":99999999999999999}}"#,
        )
        .expect("관측");
        assert_eq!(obs.five_hour, window(Some(0.0), None));
        assert_eq!(obs.weekly, window(Some(100.0), None));

        let obs = parse_snapshot(
            r#"{"primary":{"usedPercent":18446744073709551615,"windowDurationMins":300,"resetsAt":18446744073709551615}}"#,
        )
        .expect("관측");
        assert_eq!(obs.five_hour, window(Some(100.0), None), "u64 최대");

        // 창 길이가 i64 밖이면 모르는 길이다.
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

        // 칸이 다 틀리면 창도 관측도 없다(epoch 0 은 「값 없음」 자리채움이다).
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
            "required 칸 `usedPercent` 가 빠져도 패닉하지 않는다"
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
                "{key}: 데몬에 없는데 명세에 있다"
            );
            assert!(
                !collects_default_account_usage(&env(&[(key, "x")]), daemon(&[(key, "y")])),
                "{key}: 데몬 값과 다르다"
            );
        }
        // 키 대소문자는 가리지 않는다 — 스폰이 그 이름으로 자식 env 에 싣는다.
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
        // 명세에 없으면 데몬 값을 물려받는다 — 데몬에 무엇이 있든 같은 계정이다.
        assert!(collects_default_account_usage(
            &env(&[]),
            daemon(&[("CODEX_HOME", "C:\\codex-home"), ("OPENAI_API_KEY", "k")])
        ));
    }

    /// 주우면 디코더를 그대로, 줍지 않으면 막아서 돌려준다 — 데몬 쪽 값은 실제 프로세스 env 다.
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

        // 명세가 아무것도 안 바꾸면 프로세스 env 가 무엇이든 줍는다.
        let mut d = gate_decoder(Box::new(CodexAppServerDecoder::new()), &[]);
        d.decode(line.as_bytes());
        assert_eq!(d.take_usage().len(), 1);

        // 프로세스 env 에 있을 리 없는 값 — 그래서 늘 「다르다」.
        let spec = env(&[("OPENAI_BASE_URL", "http://engram-usage-gate-test.invalid")]);
        let mut d = gate_decoder(Box::new(CodexAppServerDecoder::new()), &spec);
        d.decode(line.as_bytes());
        assert!(d.take_usage().is_empty());
    }
}
