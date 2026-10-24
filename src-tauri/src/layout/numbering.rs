//! 창 번호의 순수 셈(TRD S26-window-chrome §3-4 · §4-1 · PRD R1 · D1–D3) — 새 창이 받을 번호(`smallest_free`) ·
//! 저장 번호 고치기(`resolve_saved`) · 저장값 범위 판정(`saved_number`). 셈만 하고 번호를 쥐지 않는다
//! (Tauri · 모델을 모른다).

use std::collections::BTreeSet;

/// 메인 창의 번호 — 메인은 늘 살아 있고 늘 이 번호다(D1). 저장 파일의 메인 칸 값은 보지 않는다.
pub const MAIN_WINDOW_NUMBER: u32 = 1;

/// 저장 파일에서 받아들이는 번호의 상한 — 세 자리까지 보이는 꼴을 AC38 이 잰다(TRD §3-4 M3).
/// 실행 중 새 창의 번호는 이 상한을 보지 않는다.
pub const MAX_SAVED_NUMBER: u32 = 999;

fn in_saved_range(n: u32) -> bool {
    (1..=MAX_SAVED_NUMBER).contains(&n)
}

/// 새 창(팝아웃)이 받을 번호 = `held` 에도 없고 메인의 1 도 아닌 가장 작은 양의 정수. `held` 의 순서 · 겹침 ·
/// 0 · 1 은 상관없다. 1 은 `held` 에 없어도 주지 않는다 — 부르는 쪽이 메인을 빠뜨려도 1 이 둘이 되지 않는다.
/// 부르는 쪽은 살아 있는 창 번호와 닫히는 창 번호를 함께 넘긴다.
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

/// 저장 파일의 번호 칸 하나를 읽는다 — JSON 정수 `1..=MAX_SAVED_NUMBER` 만 `Some`. 칸 없음(`Null`) · 0 · 음수 ·
/// 실수(`2.0` 포함) · 문자열 · 불 · 상한 넘음은 `None` 이다. `None` 이어도 그 창을 건너뛰지 않는다 —
/// `resolve_saved` 가 빈 번호를 준다(AC25 · AC38).
pub fn saved_number(raw: &serde_json::Value) -> Option<u32> {
    raw.as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&n| in_saved_range(n))
}

/// 저장 순서의 팝아웃 번호들(`saved` — 메인 제외)을 고쳐 같은 길이 · 같은 순서의 번호를 돌려준다(D3).
///
/// 1. 범위 밖 · 메인의 1 = 없음.
/// 2. 같은 번호를 둘 이상 적었으면 저장 순서에서 앞 창만 갖는다.
/// 3. `held`(다른 창이 이미 쥔 번호)에 든 번호 = 없음 — 이미 쥔 창이 지킨다(D2).
/// 4. 바른 번호를 먼저 다 세운 뒤 없음인 창이 저장 순서대로 비어 있는 가장 작은 번호를 받는다.
///
/// 메인의 1 은 `held` 에 없어도 주지 않는다. 고쳐진 창(경고 몫)은 `saved[i] != Some(out[i])` 로 가른다.
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

    /// 고친 번호가 서로 겹치지 않고, 메인의 1 · `held` 와도 겹치지 않는다.
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
        // AC25: 두 팝아웃이 모두 3 · 팝아웃의 1 · 0 · 범위 밖 · 바른 5 — 어느 경우에도 같은 번호 둘이 없다.
        let saved = [Some(3), Some(3), Some(1), Some(0), Some(5), Some(1000)];
        assert_eq!(resolve(&saved, &[1]), vec![3, 2, 4, 6, 5, 7]);
    }
}
