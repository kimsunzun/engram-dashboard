//! 생산자(각 벤더의 해석기)가 외부 값을 [`super::WindowObs`] 칸에 싣기 전에 거치는 정규화.
//!
//! ★전부 전함수(total)다 — 받을 수 없는 값은 `None` 이 되고 패닉하지 않는다★: 릴리즈는 `panic = "abort"`
//!   라 모양 하나 바뀐 외부 응답이 데몬을 죽이면 안 된다. 호출자는 `None` 을 받은 **그 칸만** 비우고 창·관측
//!   전체를 버리지 않는다.

use serde_json::Value;

/// 사용률 칸에 싣는 소수 자릿수(10^4 = 넷째 자리).
///
/// ★반올림을 빼지 말 것★: 0–1 값을 ×100 하면 이진 부동소수 오차가 붙는다(0.55 × 100 =
///   55.00000000000001). 받는 쪽이 남은 양 `floor(100 − used)` 를 보여 주므로 그 오차가 44 를
///   만든다(참값 45). 넷째 자리면 벤더가 주는 정밀도(소수 둘째 자리 안팎)는 다 남는다.
const PCT_SCALE: f64 = 10_000.0;

/// 리셋 시각으로 받는 epoch 초의 상한 — 9999-12-31T23:59:59Z. ISO 8601 네 자리 연도로 적을 수 있고
/// 받는 쪽(ms 단위 시계)으로 옮겨도 넘치지 않는 범위다.
const MAX_EPOCH_SECS: i64 = 253_402_300_799;

/// 이미 0–100 단위인 사용률 → 칸 값. 비유한(NaN·±inf) → `None` · [0, 100] 밖 → 절단 · 소수 넷째 자리
/// 반올림(근거 = [`PCT_SCALE`]).
pub fn used_pct_from_percent(percent: f64) -> Option<f64> {
    if !percent.is_finite() {
        return None;
    }
    let rounded = (percent.clamp(0.0, 100.0) * PCT_SCALE).round() / PCT_SCALE;
    // −0 은 직렬화에 `-0.0` 으로 남는다 — 0 으로 접는다.
    Some(if rounded == 0.0 { 0.0 } else { rounded })
}

/// 0–1 단위 사용률 → 칸 값(×100 뒤 [`used_pct_from_percent`] 와 같은 규칙).
/// ★유한한 거대 값은 ×100 에서 무한대가 될 수 있다★ — 비유한 판정을 곱하기 **전에** 하므로 그 값은
///   `None` 이 아니라 100 으로 절단된다(음수 쪽은 0).
pub fn used_pct_from_fraction(fraction: f64) -> Option<f64> {
    if !fraction.is_finite() {
        return None;
    }
    let percent = fraction * 100.0;
    if percent.is_finite() {
        used_pct_from_percent(percent)
    } else {
        Some(if percent > 0.0 { 100.0 } else { 0.0 })
    }
}

/// epoch 초 → 리셋 시각 칸 값. 1970 이하(0·음수)와 [`MAX_EPOCH_SECS`] 초과 → `None`.
/// ★0 을 받지 않는 것은 의도다★ — 실제 리셋이 1970 에 있을 리 없고, 0 은 「값 없음」 자리채움으로 흔히 쓰인다.
///   받아 두면 받는 쪽이 그 창을 「이미 리셋됨」으로 그린다.
pub fn resets_at_from_epoch_secs(secs: i64) -> Option<i64> {
    (1..=MAX_EPOCH_SECS).contains(&secs).then_some(secs)
}

/// JSON 값 → 리셋 시각 칸 값. **정수 JSON 수만** 받는다(범위는 [`resets_at_from_epoch_secs`]).
/// ★소수점이 있는 수는 `None` 이다★ — 부동소수 → 정수 변환을 외부 값에 하지 않는다. 오늘 확인된 벤더
///   모양은 전부 정수다. 문자열·`null`·그 밖 타입도 `None`.
pub fn resets_at_from_json(value: &Value) -> Option<i64> {
    let secs = match value.as_i64() {
        Some(secs) => secs,
        None => i64::try_from(value.as_u64()?).ok()?,
    };
    resets_at_from_epoch_secs(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 사용률 ──

    /// 이 반올림이 없으면 받는 쪽의 내림이 45 를 44 로 보인다(`PCT_SCALE` 근거).
    #[test]
    fn fraction_rounding_removes_binary_float_residue() {
        assert_eq!(
            0.55_f64 * 100.0,
            55.00000000000001,
            "전제: 오차가 실제로 붙는다"
        );
        assert_eq!(used_pct_from_fraction(0.55), Some(55.0));
        assert_eq!(used_pct_from_fraction(0.67), Some(67.0));
        assert_eq!(used_pct_from_fraction(0.83), Some(83.0));
        assert_eq!(used_pct_from_fraction(0.123456), Some(12.3456));
    }

    #[test]
    fn out_of_range_percent_is_clamped_not_dropped() {
        assert_eq!(used_pct_from_percent(-5.0), Some(0.0));
        assert_eq!(used_pct_from_percent(130.0), Some(100.0));
        assert_eq!(used_pct_from_fraction(-0.2), Some(0.0));
        assert_eq!(used_pct_from_fraction(1.3), Some(100.0));
        assert_eq!(used_pct_from_percent(0.0), Some(0.0));
        assert_eq!(used_pct_from_percent(100.0), Some(100.0));
    }

    #[test]
    fn non_finite_percent_is_none() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(used_pct_from_percent(bad), None, "{bad}");
            assert_eq!(used_pct_from_fraction(bad), None, "{bad}");
        }
    }

    #[test]
    fn huge_finite_fraction_saturates_instead_of_becoming_none() {
        assert_eq!(used_pct_from_fraction(f64::MAX), Some(100.0));
        assert_eq!(used_pct_from_fraction(f64::MIN), Some(0.0));
        assert_eq!(used_pct_from_percent(f64::MAX), Some(100.0));
    }

    #[test]
    fn negative_zero_is_folded_to_positive_zero() {
        let pct = used_pct_from_fraction(-0.0).expect("유한수");
        assert!(pct == 0.0 && pct.is_sign_positive(), "{pct:?}");
    }

    // ── 리셋 시각 ──

    #[test]
    fn epoch_range_is_positive_and_bounded() {
        assert_eq!(
            resets_at_from_epoch_secs(1_790_856_000),
            Some(1_790_856_000)
        );
        assert_eq!(resets_at_from_epoch_secs(1), Some(1));
        assert_eq!(
            resets_at_from_epoch_secs(MAX_EPOCH_SECS),
            Some(MAX_EPOCH_SECS)
        );
        assert_eq!(resets_at_from_epoch_secs(0), None);
        assert_eq!(resets_at_from_epoch_secs(-1), None);
        assert_eq!(resets_at_from_epoch_secs(i64::MIN), None);
        assert_eq!(resets_at_from_epoch_secs(MAX_EPOCH_SECS + 1), None);
        assert_eq!(resets_at_from_epoch_secs(i64::MAX), None);
    }

    #[test]
    fn json_epoch_accepts_integers_only() {
        let parse = |s: &str| resets_at_from_json(&serde_json::from_str::<Value>(s).expect("json"));
        assert_eq!(parse("1790856000"), Some(1_790_856_000));
        assert_eq!(parse("1790856000.0"), None);
        assert_eq!(parse("1790856000.5"), None);
        assert_eq!(parse("\"1790856000\""), None);
        assert_eq!(parse("null"), None);
        assert_eq!(parse("true"), None);
        assert_eq!(parse("-1790856000"), None);
        assert_eq!(parse("18446744073709551615"), None, "u64 최대 = i64 밖");
        assert_eq!(parse("1e300"), None);
    }
}
