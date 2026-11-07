//! Pure computation of window numbers (TRD S26-window-chrome §3-4 · §4-1 · PRD R1 · D1–D3) — the number a new window gets (`smallest_free`) ·
//! fixing a saved number (`resolve_saved`) · range decision on a saved value (`saved_number`). It only computes and does not hold numbers
//! (it knows nothing of Tauri or the model).
// ADR-0300 — window number

use std::collections::BTreeSet;

/// Number of the main window — main is always alive and always has this number (D1). The main entry of the saved file is not looked at.
pub const MAIN_WINDOW_NUMBER: u32 = 1;

/// Upper bound of a number accepted from the saved file — AC38 measures the form that shows up to three digits (TRD §3-4 M3).
/// The number of a new window at run time does not look at this bound.
pub const MAX_SAVED_NUMBER: u32 = 999;

fn in_saved_range(n: u32) -> bool {
    (1..=MAX_SAVED_NUMBER).contains(&n)
}

/// The number a new window (popout) gets = the smallest positive integer that is neither in `held` nor the main's 1. The order · duplicates ·
/// 0 · 1 in `held` do not matter. 1 is not given even if it is not in `held` — even if the caller leaves out main, 1 does not end up twice.
/// The caller passes both the numbers of live windows and the numbers of windows that are closing.
// ADR-0300 — window number
pub fn smallest_free(held: impl IntoIterator<Item = u32>) -> u32 {
    let held: BTreeSet<u32> = held.into_iter().collect();
    let mut want = MAIN_WINDOW_NUMBER + 1;
    for &n in held.range(want..) {
        if n != want {
            break;
        }
        want += 1;
    }
    want
}

/// Reads one number field of the saved file — only a JSON integer in `1..=MAX_SAVED_NUMBER` is `Some`. A missing field (`Null`) · 0 · negative ·
/// real number (including `2.0`) · string · boolean · over the bound is `None`. Even with `None` that window is not skipped —
/// `resolve_saved` gives it a free number (AC25 · AC38).
pub fn saved_number(raw: &serde_json::Value) -> Option<u32> {
    raw.as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&n| in_saved_range(n))
}

/// Fixes the popout numbers in saved order (`saved` — main excluded) and returns numbers of the same length · same order (D3).
///
/// 1. Out of range · main's 1 = none.
/// 2. If the same number is written two or more times, only the earlier window in saved order keeps it.
/// 3. A number in `held` (numbers other windows already hold) = none — the window that already holds it keeps it (D2).
/// 4. After all valid numbers are set first, windows that are none get the smallest free number in saved order.
///
/// Main's 1 is not given even if it is not in `held`. A fixed window (the warning part) is told apart by `saved[i] != Some(out[i])`.
pub fn resolve_saved(saved: &[Option<u32>], held: impl IntoIterator<Item = u32>) -> Vec<u32> {
    let mut taken: BTreeSet<u32> = held.into_iter().collect();
    let kept: Vec<Option<u32>> = saved
        .iter()
        .map(|&s| {
            let n = s.filter(|&n| in_saved_range(n) && n != MAIN_WINDOW_NUMBER)?;
            taken.insert(n).then_some(n)
        })
        .collect();
    kept.into_iter()
        .map(|k| {
            k.unwrap_or_else(|| {
                let n = smallest_free(taken.iter().copied());
                taken.insert(n);
                n
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The fixed numbers do not overlap each other, and do not overlap main's 1 or `held`.
    fn assert_disjoint(out: &[u32], held: &[u32]) {
        let mut seen: BTreeSet<u32> = held.iter().copied().collect();
        seen.insert(MAIN_WINDOW_NUMBER);
        for &n in out {
            assert!(
                seen.insert(n),
                "{n} 이 겹친다 — out {out:?} · held {held:?}"
            );
        }
    }

    fn resolve(saved: &[Option<u32>], held: &[u32]) -> Vec<u32> {
        let out = resolve_saved(saved, held.iter().copied());
        assert_eq!(out.len(), saved.len());
        assert_disjoint(&out, held);
        out
    }

    // ── smallest_free ──

    #[test]
    fn smallest_free_with_only_main_is_two() {
        assert_eq!(smallest_free([MAIN_WINDOW_NUMBER]), 2);
    }

    #[test]
    fn smallest_free_after_contiguous_run_is_next() {
        assert_eq!(smallest_free([1, 2, 3]), 4);
    }

    #[test]
    fn smallest_free_fills_the_lowest_gap() {
        assert_eq!(smallest_free([1, 3]), 2);
        assert_eq!(smallest_free([1, 2, 4, 5, 7]), 3);
    }

    #[test]
    fn smallest_free_ignores_order_duplicates_and_zero() {
        assert_eq!(smallest_free([3, 1, 1, 0, 2, 3]), 4);
        assert_eq!(smallest_free([0, 1]), 2);
    }

    #[test]
    fn smallest_free_never_gives_main_number_even_if_held_lacks_it() {
        assert_eq!(smallest_free([]), 2);
        assert_eq!(smallest_free([0]), 2);
        assert_eq!(smallest_free([2, 3]), 4);
        assert_eq!(smallest_free([3]), 2);
    }

    // ── saved_number ──

    #[test]
    fn saved_number_accepts_integers_in_range() {
        assert_eq!(saved_number(&json!(1)), Some(1));
        assert_eq!(saved_number(&json!(2)), Some(2));
        assert_eq!(
            saved_number(&json!(MAX_SAVED_NUMBER)),
            Some(MAX_SAVED_NUMBER)
        );
    }

    #[test]
    fn saved_number_rejects_everything_else() {
        for raw in [
            json!(null),
            json!(0),
            json!(-1),
            json!(MAX_SAVED_NUMBER + 1),
            json!(u64::from(u32::MAX) + 2),
            json!(1.5),
            json!(2.0),
            json!("x"),
            json!("2"),
            json!(true),
            json!([2]),
            json!({ "n": 2 }),
        ] {
            assert_eq!(saved_number(&raw), None, "{raw}");
        }
    }

    // ── resolve_saved ──

    #[test]
    fn resolve_old_file_numbers_in_saved_order_after_main() {
        assert_eq!(resolve(&[None, None], &[1]), vec![2, 3]);
    }

    #[test]
    fn resolve_keeps_valid_distinct_numbers() {
        assert_eq!(resolve(&[Some(3), Some(5)], &[1]), vec![3, 5]);
    }

    #[test]
    fn resolve_duplicate_keeps_first_and_moves_later() {
        assert_eq!(resolve(&[Some(3), Some(3)], &[1]), vec![3, 2]);
    }

    #[test]
    fn resolve_popout_claiming_main_is_moved() {
        assert_eq!(resolve(&[Some(1)], &[1]), vec![2]);
    }

    #[test]
    fn resolve_out_of_range_is_moved_to_smallest_free() {
        assert_eq!(resolve(&[Some(0)], &[1]), vec![2]);
        assert_eq!(resolve(&[Some(MAX_SAVED_NUMBER + 1)], &[1]), vec![2]);
        assert_eq!(resolve(&[Some(u32::MAX)], &[1]), vec![2]);
        assert_eq!(
            resolve(&[Some(MAX_SAVED_NUMBER)], &[1]),
            vec![MAX_SAVED_NUMBER]
        );
    }

    #[test]
    fn resolve_unreadable_raw_values_are_moved_in_saved_order() {
        let saved: Vec<Option<u32>> = [json!(-1), json!("x"), json!(0), json!(4)]
            .iter()
            .map(saved_number)
            .collect();
        assert_eq!(resolve(&saved, &[1]), vec![2, 3, 5, 4]);
    }

    #[test]
    fn resolve_number_already_held_steps_aside() {
        assert_eq!(resolve(&[Some(2), Some(3)], &[1, 2]), vec![4, 3]);
    }

    #[test]
    fn resolve_places_valid_numbers_before_filling_gaps() {
        assert_eq!(resolve(&[None, Some(2)], &[1]), vec![3, 2]);
        assert_eq!(resolve(&[None, Some(3), None], &[1]), vec![2, 3, 4]);
    }

    #[test]
    fn resolve_never_gives_main_number_even_if_held_lacks_it() {
        assert_eq!(resolve(&[None], &[]), vec![2]);
        assert_eq!(resolve(&[Some(1), None], &[]), vec![2, 3]);
    }

    #[test]
    fn resolve_empty_input_is_empty() {
        assert_eq!(resolve(&[], &[1]), Vec::<u32>::new());
    }

    #[test]
    fn resolve_mixed_case_matches_ac25_example() {
        // AC25: both popouts are 3 · popout's 1 · 0 · out of range · valid 5 — in no case are there two of the same number.
        let saved = [Some(3), Some(3), Some(1), Some(0), Some(5), Some(1000)];
        assert_eq!(resolve(&saved, &[1]), vec![3, 2, 4, 6, 5, 7]);
    }
}
