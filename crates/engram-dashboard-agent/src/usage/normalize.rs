//! The normalization a producer (each vendor's interpreter) applies before putting an external value into a [`super::WindowObs`] field.
//!
//! ★All are total functions — an unacceptable value becomes `None`, never a panic★: a panic on the usage-query side is outside the
//!   containment mark and ends the daemon, and one in the pump interpreter ends that agent as `Failed` (ADR-0292) — a changed external
//!   response shape must not do that. The caller empties **only the field** that got `None`, not the whole window · observation.

use serde_json::Value;

/// Decimal places carried in the usage-rate field (10^4 = fourth place).
///
/// ★Do not drop the rounding★: 0–1 × 100 attaches binary floating-point error (0.55 × 100 = 55.00000000000001), and the receiver shows
///   `floor(100 − used)`, which would give 44 instead of 45. The fourth place fully keeps vendors' precision (about two decimal places).
const PCT_SCALE: f64 = 10_000.0;

/// Upper bound of epoch seconds accepted as a reset time — 9999-12-31T23:59:59Z: writable with an ISO 8601 four-digit year and no overflow
/// on the receiving side (an ms-unit clock).
const MAX_EPOCH_SECS: i64 = 253_402_300_799;

/// Usage rate already in 0–100 units → field value. Non-finite (NaN · ±inf) → `None` · outside [0, 100] → clamped · rounded to the fourth
/// decimal place (grounds = [`PCT_SCALE`]).
pub fn used_pct_from_percent(percent: f64) -> Option<f64> {
    if !percent.is_finite() {
        return None;
    }
    let rounded = (percent.clamp(0.0, 100.0) * PCT_SCALE).round() / PCT_SCALE;
    // −0 stays as `-0.0` in serialization — fold it to 0.
    Some(if rounded == 0.0 { 0.0 } else { rounded })
}

/// Usage rate in 0–1 units → field value (after ×100, same rule as [`used_pct_from_percent`]).
/// ★A finite huge value can become infinity at ×100★ — the non-finite check runs **before** multiplying, so it clamps to 100 (0 on the
///   negative side), not `None`.
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
/// ★Not accepting 0 is intended★ — a real reset cannot be in 1970 and 0 is a common 「no value」 placeholder; accepted, the receiver would
///   draw 「already reset」.
pub fn resets_at_from_epoch_secs(secs: i64) -> Option<i64> {
    (1..=MAX_EPOCH_SECS).contains(&secs).then_some(secs)
}

/// JSON value → reset-time field value. Accepts **only integer JSON numbers** (range: [`resets_at_from_epoch_secs`]).
/// ★A number with a decimal point is `None`★ — no float → integer conversion on external values (all vendor shapes confirmed today are
///   integers). A string · `null` · any other type is `None` too.
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

    #[test]
    fn fraction_rounding_removes_binary_float_residue() {
        assert_eq!(
            0.55_f64 * 100.0,
            55.00000000000001,
            "premise: the error really appears"
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
        let pct = used_pct_from_fraction(-0.0).expect("finite number");
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
        assert_eq!(parse("18446744073709551615"), None, "u64 max = outside i64");
        assert_eq!(parse("1e300"), None);
    }
}
