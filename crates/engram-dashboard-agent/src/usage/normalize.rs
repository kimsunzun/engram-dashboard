//! The normalization a producer (each vendor's interpreter) applies before putting an external value into a [`super::WindowObs`] field.
//!
//! ★All of them are total functions — a value that cannot be accepted becomes `None` and does not panic★: a panic on the usage-query side is outside the containment
//!   mark, so it ends the daemon, and a panic in the pump interpreter ends that agent as `Failed` (ADR-0292) — an external response whose shape
//!   changed must not do that. The caller empties **only the field** that got `None` and does not discard the whole window · observation.

use serde_json::Value;

/// Number of decimal places carried in the usage-rate field (10^4 = fourth place).
///
/// ★Do not drop the rounding★: multiplying a 0–1 value by 100 attaches binary floating-point error (0.55 × 100 =
///   55.00000000000001). The receiver shows the remaining amount `floor(100 − used)`, so that error would
///   make 44 (true value 45). With the fourth place, the precision vendors give (around two decimal places) is fully kept.
const PCT_SCALE: f64 = 10_000.0;

/// Upper bound of epoch seconds accepted as a reset time — 9999-12-31T23:59:59Z. A range that can be written with an ISO 8601 four-digit year and
/// does not overflow when moved to the receiving side (an ms-unit clock).
const MAX_EPOCH_SECS: i64 = 253_402_300_799;

/// Usage rate already in 0–100 units → field value. Non-finite (NaN · ±inf) → `None` · outside [0, 100] → clamped · rounded to the fourth decimal place
/// (grounds = [`PCT_SCALE`]).
pub fn used_pct_from_percent(percent: f64) -> Option<f64> {
    if !percent.is_finite() {
        return None;
    }
    let rounded = (percent.clamp(0.0, 100.0) * PCT_SCALE).round() / PCT_SCALE;
    // −0 stays as `-0.0` in serialization — fold it to 0.
    Some(if rounded == 0.0 { 0.0 } else { rounded })
}

/// Usage rate in 0–1 units → field value (after ×100, the same rule as [`used_pct_from_percent`]).
/// ★A finite huge value can become infinity at ×100★ — the non-finite check is done **before** multiplying, so that value is
///   clamped to 100, not `None` (to 0 on the negative side).
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

/// epoch seconds → reset-time field value. 1970 and below (0 · negative) and above [`MAX_EPOCH_SECS`] → `None`.
/// ★Not accepting 0 is intended★ — a real reset cannot be in 1970, and 0 is commonly used as a 「값 없음」 (no value) placeholder.
///   If accepted, the receiver would draw that window as 「이미 리셋됨」 (already reset).
pub fn resets_at_from_epoch_secs(secs: i64) -> Option<i64> {
    (1..=MAX_EPOCH_SECS).contains(&secs).then_some(secs)
}

/// JSON value → reset-time field value. Accepts **only integer JSON numbers** (range in [`resets_at_from_epoch_secs`]).
/// ★A number with a decimal point is `None`★ — we do not do float → integer conversion on external values. All vendor
///   shapes confirmed today are integers. A string · `null` · any other type is also `None`.
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

    // ── Usage rate ──

    /// Without this rounding, the receiver's floor makes 45 look like 44 (grounds of `PCT_SCALE`).
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

    // ── Reset time ──

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
