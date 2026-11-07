//! Docking tree operation — builds the **result tree** in which one tab's layout (`incoming`) is inserted next to a slot of the target tab's tree or at its
//! outer side (PRD R7–R9 · TRD S21-slot-docking 5.8th edition §1-2 「트리 연산」). It knows only `tree` and `geometry` and knows nothing of Tauri, locks or I/O →
//! it becomes a standalone headless test (ADR-0012 isolation).
//!
//! Entry points = [`docked_tree`](result tree · reject new zero size) · [`incoming_rect`](rectangle of the inserted part in the result geometry).
//! The guide · drop decision · apply all use the same `docked_tree` — the painted rectangle and the dropped result come out of one formula (PRD R4).
//!
//! ★Invariant★: the slot ids and split ids of input nodes are not changed — the only id drawn anew is the wrapping split (ADR-0223).

use std::collections::HashSet;

use ts_rs::TS;
use uuid::Uuid;

use super::geometry::{self, has_area, LayoutGeometry, RectF64};
use super::tree;
use super::types::{LayoutNode, SlotContent, SplitDir};

/// Side to attach — the name says the result layout. `Left` = new one to the left of the reference · `Right` = right · `Top` = above · `Bottom` = below.
/// Wire value = the variant name as is (`Left` …).
// ★Do not attach `#[serde(rename_all)]`★ — the declaration macro of the bus command that carries this (`tab.dock` · T-65) cannot attach a rename, so
// the variant name becomes the wire value as is. The drop invoke (`Seam` of `tab_drop`) carries it with the same spelling too.
// TRD S21-slot-docking §3-1
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum DockSide {
    Left,
    Right,
    Top,
    Bottom,
}

/// Reference to insert at — `Slot` = that one slot of the target tree · `Outer` = the whole target tree (root — outer side).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockAnchor {
    Slot(Uuid),
    Outer,
}

/// Fixed share of the new one — used only by tests. The drag's share comes from `layout::bands::attach_share`, not from this value.
pub const DOCK_NEW_SHARE: f64 = 1.0 / 3.0;

/// Upper bound of the share the new one takes in a docking split — a model contract (the lower bound is `tree::RATIO_MIN`). [`docked_tree`] asserts it in debug
/// builds and `ViewManager::dock` rejects with `InvalidShare`. The drag-side upper bound `layout::bands::ATTACH_SHARE_MAX`
/// must not be larger than this (compile assertion there).
pub const SHARE_MAX: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockTreeError {
    /// The reference slot is not in the target tree.
    SlotNotFound(Uuid),
    /// The target and the inserted one have the same slot id — the result would read one area as two places (slot ids are globally unique across windows —
    /// ADR-0035).
    SlotIdOverlap,
    /// The result has an area that is **newly** zero size (PRD R8).
    ZeroArea,
}

/// The result tree where the `anchor` of `target` is wrapped in a new split and `incoming` is inserted on the `side` side. The input is not touched.
///
/// - Shape: `Left`·`Right` = `LeftRight` split, `Top`·`Bottom` = `TopBottom` split. `share` = the share the new one
///   takes in the wrapped place — must satisfy `tree::RATIO_MIN ≤ share ≤` [`SHARE_MAX`] (below that, the geometry reads a different ratio · outside, a debug
///   build asserts). If the new one is a (left/top), ratio is `share`; if b, `1 − share` (ratio = a's share — ADR-0140). The ratio
///   inside the wrapped reference is unchanged, so beside a slot only that slot, and at an outer side the whole target, shrinks keeping its inner ratio.
/// - id: only the wrapping split gets a new id. The slot ids · split ids · content of the target and `incoming` are moved as they are — removing
///   from the source is the caller's job.
/// - Errors (checked in this order, the first one hit is returned):
///   1. `SlotNotFound` if the reference slot is not in `target`.
///   2. `SlotIdOverlap` if the two trees have the same slot id.
///   3. `ZeroArea` if, among the areas that are zero size in the result, any one had size in its original place (target areas by the `target` geometry, inserted
///      areas by the standalone `incoming` geometry) — an area that was zero size from the start is not a reason. Coordinates are normalized layout
///      coordinates (not screen px).
pub fn docked_tree(
    target: &LayoutNode,
    anchor: DockAnchor,
    side: DockSide,
    incoming: &LayoutNode,
    share: f64,
) -> Result<LayoutNode, DockTreeError> {
    // ADR-0295
    debug_assert!(
        (tree::RATIO_MIN..=SHARE_MAX).contains(&share),
        "새것 몫 {share}"
    );
    let (dir, new_is_a) = match side {
        DockSide::Left => (SplitDir::LeftRight, true),
        DockSide::Right => (SplitDir::LeftRight, false),
        DockSide::Top => (SplitDir::TopBottom, true),
        DockSide::Bottom => (SplitDir::TopBottom, false),
    };

    let mut result = target.clone();
    let based = match anchor {
        DockAnchor::Outer => &mut result,
        DockAnchor::Slot(id) => {
            slot_node_mut(&mut result, id).ok_or(DockTreeError::SlotNotFound(id))?
        }
    };
    let target_slots: HashSet<Uuid> = tree::slot_ids(target).into_iter().collect();
    if tree::slot_ids(incoming)
        .iter()
        .any(|s| target_slots.contains(s))
    {
        return Err(DockTreeError::SlotIdOverlap);
    }
    let original = Box::new(std::mem::replace(
        based,
        LayoutNode::Slot {
            id: Uuid::nil(),
            content: SlotContent::Empty,
        },
    ));
    let moved = Box::new(incoming.clone());
    let (a, b, ratio) = if new_is_a {
        (moved, original, share)
    } else {
        (original, moved, 1.0 - share)
    };
    *based = LayoutNode::Split {
        id: Uuid::new_v4(), // ADR-0223 — new id per split
        dir,
        ratio,
        a,
        b,
    };

    // Areas that were zero size from the start are left out of the check — without that, one such area would block all docking to that tab
    // (same stance as `ViewManager::set_split_ratio` ⑤).
    // ADR-0227
    let had_area: HashSet<Uuid> = [target, incoming]
        .into_iter()
        .flat_map(|n| geometry::compute(n).slots)
        .filter(has_area)
        .map(|r| r.slot_id)
        .collect();
    let collapses = geometry::compute(&result)
        .slots
        .iter()
        .any(|r| !has_area(r) && had_area.contains(&r.slot_id));
    if collapses {
        return Err(DockTreeError::ZeroArea);
    }
    Ok(result)
}

/// The normalized rectangle that `incoming_slots` take in the result geometry = the bounding rectangle of those areas (PRD R4 「놓았을 때의 실제
/// 사각형」). It does not recompute boundaries but only picks the area boundary values, so if all areas of the inserted subtree are given, it is bit-for-bit equal to that
/// subtree's box in the result geometry (`geometry` header invariant). `None` if the list is empty or any one id is not in `result`.
/// Area ids in `result` are assumed unique — for a `docked_tree` result that holds because of the `SlotIdOverlap` rejection.
pub fn incoming_rect(result: &LayoutGeometry, incoming_slots: &[Uuid]) -> Option<RectF64> {
    let mut out: Option<RectF64> = None;
    for id in incoming_slots {
        let r = result.slots.iter().find(|s| s.slot_id == *id)?;
        out = Some(match out {
            None => RectF64 {
                x0: r.x0,
                y0: r.y0,
                x1: r.x1,
                y1: r.y1,
            },
            Some(acc) => RectF64 {
                x0: acc.x0.min(r.x0),
                y0: acc.y0.min(r.y0),
                x1: acc.x1.max(r.x1),
                y1: acc.y1.max(r.y1),
            },
        });
    }
    out
}

fn slot_node_mut(node: &mut LayoutNode, slot_id: Uuid) -> Option<&mut LayoutNode> {
    if matches!(node, LayoutNode::Slot { id, .. } if *id == slot_id) {
        return Some(node);
    }
    match node {
        LayoutNode::Slot { .. } => None,
        LayoutNode::Split { a, b, .. } => {
            slot_node_mut(a, slot_id).or_else(|| slot_node_mut(b, slot_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::geometry::SlotRect;

    const SIDES: [DockSide; 4] = [
        DockSide::Left,
        DockSide::Right,
        DockSide::Top,
        DockSide::Bottom,
    ];

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn slot(n: u128) -> LayoutNode {
        LayoutNode::Slot {
            id: id(n),
            content: SlotContent::Empty,
        }
    }

    fn split(n: u128, dir: SplitDir, ratio: f64, a: LayoutNode, b: LayoutNode) -> LayoutNode {
        LayoutNode::Split {
            id: id(n),
            dir,
            ratio,
            a: Box::new(a),
            b: Box::new(b),
        }
    }

    // TRD S21-slot-docking 5.8th edition §1-2 「트리 연산」 split shape — (direction, whether the new one is a, ratio = a's share).
    fn shape(side: DockSide) -> (SplitDir, bool, f64) {
        match side {
            DockSide::Left => (SplitDir::LeftRight, true, 1.0 / 3.0),
            DockSide::Right => (SplitDir::LeftRight, false, 1.0 - 1.0 / 3.0),
            DockSide::Top => (SplitDir::TopBottom, true, 1.0 / 3.0),
            DockSide::Bottom => (SplitDir::TopBottom, false, 1.0 - 1.0 / 3.0),
        }
    }

    fn wrapped(fresh: Uuid, side: DockSide, based: LayoutNode, moved: LayoutNode) -> LayoutNode {
        let (dir, new_is_a, ratio) = shape(side);
        let (a, b) = if new_is_a {
            (moved, based)
        } else {
            (based, moved)
        };
        LayoutNode::Split {
            id: fresh,
            dir,
            ratio,
            a: Box::new(a),
            b: Box::new(b),
        }
    }

    // Split(100, left/right 0.3){1, Split(101, top/bottom 0.6){2, 3}}
    fn target() -> LayoutNode {
        split(
            100,
            SplitDir::LeftRight,
            0.3,
            slot(1),
            split(101, SplitDir::TopBottom, 0.6, slot(2), slot(3)),
        )
    }

    fn agent_slot(n: u128) -> LayoutNode {
        LayoutNode::Slot {
            id: id(n),
            content: SlotContent::Agent {
                agent_id: "moved".into(),
            },
        }
    }

    // Split(200, left/right 0.3){21, 22}
    fn split_tab() -> LayoutNode {
        split(200, SplitDir::LeftRight, 0.3, slot(21), slot(22))
    }

    fn split_ids(node: &LayoutNode) -> Vec<Uuid> {
        tree::list_splits(node).into_iter().map(|s| s.id).collect()
    }

    // Splits of the result = all splits of the input + one new one — returns that new id.
    fn the_new_split(result: &LayoutNode, target: &LayoutNode, moved: &LayoutNode) -> Uuid {
        let old: HashSet<Uuid> = split_ids(target)
            .into_iter()
            .chain(split_ids(moved))
            .collect();
        let now: HashSet<Uuid> = split_ids(result).into_iter().collect();
        assert!(old.is_subset(&now), "입력의 분할 id 가 남아 있어야");
        let fresh: Vec<Uuid> = now.difference(&old).copied().collect();
        assert_eq!(fresh.len(), 1, "새 분할은 하나뿐: {fresh:?}");
        fresh[0]
    }

    fn rect_of(g: &LayoutGeometry, slot_id: Uuid) -> SlotRect {
        *g.slots
            .iter()
            .find(|s| s.slot_id == slot_id)
            .expect("칸 있어야")
    }

    fn along(r: &SlotRect, dir: SplitDir) -> (f64, f64) {
        match dir {
            SplitDir::LeftRight => (r.x0, r.x1),
            SplitDir::TopBottom => (r.y0, r.y1),
        }
    }

    fn across(r: &SlotRect, dir: SplitDir) -> (f64, f64) {
        match dir {
            SplitDir::LeftRight => (r.y0, r.y1),
            SplitDir::TopBottom => (r.x0, r.x1),
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    fn zero_area_count(node: &LayoutNode) -> usize {
        geometry::compute(node)
            .slots
            .iter()
            .filter(|r| !has_area(r))
            .count()
    }

    // Halving chain — b is always the next level. Returns = (tree, deepest area id). Area ids = `base..=base+depth`,
    // split ids = `base+1001..`.
    fn half_chain(dir: SplitDir, depth: u128, base: u128) -> (LayoutNode, Uuid) {
        let mut node = slot(base);
        for i in 1..=depth {
            node = split(base + 1000 + i, dir, 0.5, slot(base + i), node);
        }
        (node, id(base))
    }

    // ── share · spelling ──────────────────────────────────────────────────────

    #[test]
    fn the_new_share_is_a_third_and_both_ratios_escape_the_clamp() {
        assert_eq!(DOCK_NEW_SHARE, 1.0 / 3.0);
        assert!(close(1.0 - DOCK_NEW_SHARE, 2.0 / 3.0));
        for r in [DOCK_NEW_SHARE, 1.0 - DOCK_NEW_SHARE] {
            assert!((tree::RATIO_MIN..=tree::RATIO_MAX).contains(&r), "{r}");
            assert_eq!(tree::clamp_ratio(r), r, "기하가 다른 비율로 읽으면 안 된다");
        }
    }

    #[test]
    fn the_given_share_is_the_new_pieces_part_of_the_wrapped_place_on_each_side() {
        // ADR-0295 — the attach share (PRD R7) is chosen by the caller.
        let (t, moved) = (target(), slot(9));
        for share in [tree::RATIO_MIN, 0.25, 0.5] {
            for anchor in [DockAnchor::Slot(id(2)), DockAnchor::Outer] {
                for side in SIDES {
                    let result = docked_tree(&t, anchor, side, &moved, share).unwrap();
                    let fresh = the_new_split(&result, &t, &moved);
                    let (dir, new_is_a, _) = shape(side);
                    let ratio = if new_is_a { share } else { 1.0 - share };
                    let at = format!("{share} {anchor:?} {side:?}");
                    assert_eq!(tree::split_ratio(&result, fresh), Some(ratio), "{at}");
                    assert_eq!(
                        tree::clamp_ratio(ratio),
                        ratio,
                        "{at}: 기하가 다른 비율로 읽으면 안 된다"
                    );
                    let g = geometry::compute(&result);
                    let s = *g.splits.iter().find(|s| s.split_id == fresh).unwrap();
                    let whole = match dir {
                        SplitDir::LeftRight => s.x1 - s.x0,
                        SplitDir::TopBottom => s.y1 - s.y0,
                    };
                    let (n0, n1) = along(&rect_of(&g, id(9)), dir);
                    assert!(close((n1 - n0) / whole, share), "{at}");
                }
            }
        }
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "새것 몫")]
    fn a_share_over_half_is_a_caller_bug() {
        let _ = docked_tree(&target(), DockAnchor::Outer, DockSide::Left, &slot(9), 0.6);
    }

    #[test]
    fn dock_side_spells_variant_names_on_the_wire() {
        for (side, want) in SIDES.into_iter().zip(["Left", "Right", "Top", "Bottom"]) {
            let json = serde_json::to_string(&side).unwrap();
            assert_eq!(json, format!("\"{want}\""));
            assert_eq!(serde_json::from_str::<DockSide>(&json).unwrap(), side);
        }
    }

    // ── shape · id preservation ───────────────────────────────────────────────

    #[test]
    fn outer_wraps_the_whole_target_on_each_side() {
        let (t, moved) = (target(), agent_slot(9));
        for side in SIDES {
            let result = docked_tree(&t, DockAnchor::Outer, side, &moved, DOCK_NEW_SHARE).unwrap();
            let fresh = the_new_split(&result, &t, &moved);
            assert_eq!(
                result,
                wrapped(fresh, side, t.clone(), moved.clone()),
                "{side:?}"
            );
        }
    }

    #[test]
    fn a_slot_anchor_wraps_only_that_slot_on_each_side() {
        let (t, moved) = (target(), agent_slot(9));
        for side in SIDES {
            let result =
                docked_tree(&t, DockAnchor::Slot(id(2)), side, &moved, DOCK_NEW_SHARE).unwrap();
            let fresh = the_new_split(&result, &t, &moved);
            let want = split(
                100,
                SplitDir::LeftRight,
                0.3,
                slot(1),
                split(
                    101,
                    SplitDir::TopBottom,
                    0.6,
                    wrapped(fresh, side, slot(2), moved.clone()),
                    slot(3),
                ),
            );
            assert_eq!(result, want, "{side:?}");
        }
    }

    #[test]
    fn a_split_incoming_tab_lands_whole_with_its_ids_and_ratio() {
        // AC5 · R9 — the inserted subtree is the source nodes as they are (slot ids · split ids · ratio).
        let (t, moved) = (target(), split_tab());
        for anchor in [DockAnchor::Slot(id(1)), DockAnchor::Outer] {
            for side in SIDES {
                let result = docked_tree(&t, anchor, side, &moved, DOCK_NEW_SHARE).unwrap();
                let mut want_slots = tree::slot_ids(&t);
                want_slots.extend(tree::slot_ids(&moved));
                let mut got_slots = tree::slot_ids(&result);
                want_slots.sort();
                got_slots.sort();
                assert_eq!(got_slots, want_slots, "{anchor:?} {side:?}");
                the_new_split(&result, &t, &moved);
                assert_eq!(tree::split_ratio(&result, id(200)), Some(0.3));

                let g = geometry::compute(&result);
                let (l, r) = (rect_of(&g, id(21)), rect_of(&g, id(22)));
                let (w_l, w_r) = (l.x1 - l.x0, r.x1 - r.x0);
                assert!(close(w_l / (w_l + w_r), 0.3), "{anchor:?} {side:?}");
                assert_eq!((l.y0, l.y1), (r.y0, r.y1));
            }
        }
    }

    #[test]
    fn a_missing_anchor_slot_is_slot_not_found() {
        let (t, moved) = (target(), split_tab());
        for side in SIDES {
            assert_eq!(
                docked_tree(&t, DockAnchor::Slot(id(77)), side, &moved, DOCK_NEW_SHARE),
                Err(DockTreeError::SlotNotFound(id(77)))
            );
            // The reference is looked up only in the target tree — an area on the inserted side is not a reference.
            assert_eq!(
                docked_tree(&t, DockAnchor::Slot(id(21)), side, &moved, DOCK_NEW_SHARE),
                Err(DockTreeError::SlotNotFound(id(21)))
            );
        }
    }

    #[test]
    fn a_slot_id_in_both_trees_is_slot_id_overlap() {
        let t = target();
        // Area 3 is on both sides.
        let shared = split(300, SplitDir::LeftRight, 0.5, slot(40), slot(3));
        for anchor in [
            DockAnchor::Slot(id(1)),
            DockAnchor::Slot(id(3)),
            DockAnchor::Outer,
        ] {
            for side in SIDES {
                assert_eq!(
                    docked_tree(&t, anchor, side, &shared, DOCK_NEW_SHARE),
                    Err(DockTreeError::SlotIdOverlap),
                    "{anchor:?} {side:?}"
                );
            }
        }
        assert_eq!(
            docked_tree(&t, DockAnchor::Outer, DockSide::Left, &t, DOCK_NEW_SHARE),
            Err(DockTreeError::SlotIdOverlap),
            "자기 자신"
        );
        // Decision order — the reference slot comes first.
        assert_eq!(
            docked_tree(
                &t,
                DockAnchor::Slot(id(77)),
                DockSide::Left,
                &shared,
                DOCK_NEW_SHARE
            ),
            Err(DockTreeError::SlotNotFound(id(77)))
        );
        // Decision order — overlap comes before zero size (this reference is zero size originally, so without an overlap it would be `ZeroArea`).
        let (chain, _) = half_chain(SplitDir::TopBottom, 60, 10_000);
        let flat = geometry::compute(&chain)
            .slots
            .into_iter()
            .find(|r| !has_area(r))
            .expect("전제: 면적 0 칸이 심겼다");
        let both = LayoutNode::Slot {
            id: flat.slot_id,
            content: SlotContent::Empty,
        };
        assert_eq!(
            docked_tree(
                &chain,
                DockAnchor::Slot(flat.slot_id),
                DockSide::Right,
                &both,
                DOCK_NEW_SHARE
            ),
            Err(DockTreeError::SlotIdOverlap)
        );
    }

    // ── ratio · geometry (R7 · AC3 · AC4) ─────────────────────────────────────

    #[test]
    fn next_to_a_slot_only_that_slot_splits_two_thirds_to_one_third() {
        let (t, moved) = (target(), slot(9));
        let before = geometry::compute(&t);
        let orig = rect_of(&before, id(2));
        for side in SIDES {
            let (dir, new_is_a, _) = shape(side);
            let after = geometry::compute(
                &docked_tree(&t, DockAnchor::Slot(id(2)), side, &moved, DOCK_NEW_SHARE).unwrap(),
            );
            for other in [id(1), id(3)] {
                assert_eq!(
                    rect_of(&after, other),
                    rect_of(&before, other),
                    "{side:?}: 다른 칸은 그대로"
                );
            }
            let (kept, new) = (rect_of(&after, id(2)), rect_of(&after, id(9)));
            assert_eq!(across(&kept, dir), across(&orig, dir), "{side:?}");
            assert_eq!(across(&new, dir), across(&orig, dir), "{side:?}");
            let (o, k, n) = (along(&orig, dir), along(&kept, dir), along(&new, dir));
            let (first, second) = if new_is_a { (n, k) } else { (k, n) };
            // The two divide the original area exactly between them — the touching boundary has the same value too.
            assert_eq!(
                (first.0, first.1, second.1),
                (o.0, second.0, o.1),
                "{side:?}"
            );
            let len = o.1 - o.0;
            assert!(close((n.1 - n.0) / len, 1.0 / 3.0), "{side:?}: 새것 1/3");
            assert!(close((k.1 - k.0) / len, 2.0 / 3.0), "{side:?}: 기존 2/3");
        }
    }

    #[test]
    fn at_the_outer_edge_the_whole_layout_shrinks_to_two_thirds_keeping_inner_ratios() {
        // AC4 — existing areas keep their ratios to each other inside the target region.
        let (t, moved) = (target(), slot(9));
        let before = geometry::compute(&t);
        for side in SIDES {
            let (dir, new_is_a, _) = shape(side);
            let after = geometry::compute(
                &docked_tree(&t, DockAnchor::Outer, side, &moved, DOCK_NEW_SHARE).unwrap(),
            );
            let new = rect_of(&after, id(9));
            let (n0, n1) = along(&new, dir);
            assert!(close(n1 - n0, 1.0 / 3.0), "{side:?}: 새것 1/3");
            assert_eq!(across(&new, dir), (0.0, 1.0), "{side:?}");
            // The region the target occupies = the remaining 2/3.
            let (lo, hi) = if new_is_a { (n1, 1.0) } else { (0.0, n0) };
            for s in [id(1), id(2), id(3)] {
                let (o, a) = (rect_of(&before, s), rect_of(&after, s));
                let (oa, aa) = (along(&o, dir), along(&a, dir));
                let norm = |v: f64| (v - lo) / (hi - lo);
                assert!(
                    close(norm(aa.0), oa.0) && close(norm(aa.1), oa.1),
                    "{side:?} {s}"
                );
                assert_eq!(
                    across(&a, dir),
                    across(&o, dir),
                    "{side:?} {s}: 다른 축은 그대로"
                );
            }
        }
    }

    // ── new zero size (R8 · AC15) ─────────────────────────────────────────────

    #[test]
    fn a_deep_child_of_the_incoming_tab_that_would_collapse_is_zero_area() {
        // Target: left/right halving chain of 40 levels — deepest area width 2^-40 (at the right end).
        let (t, deepest) = half_chain(SplitDir::LeftRight, 40, 10_000);
        assert_eq!(zero_area_count(&t), 0, "전제: 대상은 전부 면적이 있다");
        // To insert: left/right halving chain of 20 levels — on its own even the deepest area has width 2^-20.
        let (moved, _) = half_chain(SplitDir::LeftRight, 20, 30_000);
        assert_eq!(
            zero_area_count(&moved),
            0,
            "전제: 끼울 것도 전부 면적이 있다"
        );

        // Contrast: a single area attaches — 1/3 of the place to insert into has size by itself.
        let lone = docked_tree(
            &t,
            DockAnchor::Slot(deepest),
            DockSide::Right,
            &slot(9),
            DOCK_NEW_SHARE,
        )
        .unwrap();
        assert_eq!(zero_area_count(&lone), 0);
        // With a chain, the deep area inside it collapses.
        assert_eq!(
            docked_tree(
                &t,
                DockAnchor::Slot(deepest),
                DockSide::Right,
                &moved,
                DOCK_NEW_SHARE
            ),
            Err(DockTreeError::ZeroArea)
        );
        // Contrast: the same chain attaches to a wide place (outer side) — it is not a blanket rejection.
        let wide = docked_tree(
            &t,
            DockAnchor::Outer,
            DockSide::Left,
            &moved,
            DOCK_NEW_SHARE,
        )
        .unwrap();
        assert_eq!(zero_area_count(&wide), 0);
    }

    #[test]
    fn a_target_slot_that_would_newly_collapse_is_zero_area() {
        // Target: left/right halving chain of 53 levels — on its own everything has size, but the areas at the 1.0 end are right at the f64 limit.
        let (t, _) = half_chain(SplitDir::LeftRight, 53, 10_000);
        assert_eq!(
            zero_area_count(&t),
            0,
            "전제: 대상은 단독으로 전부 면적이 있다"
        );
        // Premise: attaching at the outer left shrinks the target to 2/3 and the target areas collapse, while the inserted area has size.
        let g = geometry::compute(&wrapped(id(1), DockSide::Left, t.clone(), slot(9)));
        assert!(has_area(&rect_of(&g, id(9))), "전제: 끼운 칸은 면적이 있다");
        assert!(
            g.slots.iter().any(|r| r.slot_id != id(9) && !has_area(r)),
            "전제: 대상 칸이 새로 무너진다"
        );
        assert_eq!(
            docked_tree(
                &t,
                DockAnchor::Outer,
                DockSide::Left,
                &slot(9),
                DOCK_NEW_SHARE
            ),
            Err(DockTreeError::ZeroArea)
        );
        // Contrast: a shallow chain attaches to the same place.
        let (shallow, _) = half_chain(SplitDir::LeftRight, 40, 20_000);
        let ok = docked_tree(
            &shallow,
            DockAnchor::Outer,
            DockSide::Left,
            &slot(9),
            DOCK_NEW_SHARE,
        )
        .unwrap();
        assert_eq!(zero_area_count(&ok), 0);
    }

    #[test]
    fn repeated_docking_stops_with_zero_area_at_the_representation_limit() {
        for side in SIDES {
            let (_, new_is_a, _) = shape(side);
            // Always attach to the area at the 1.0 end — if the new one is b, it is the area just attached; if a, it is the first area.
            let mut tree_now = slot(1);
            let mut anchor = id(1);
            let mut docked: u128 = 0;
            let err = loop {
                let incoming = slot(1_000 + docked);
                match docked_tree(
                    &tree_now,
                    DockAnchor::Slot(anchor),
                    side,
                    &incoming,
                    DOCK_NEW_SHARE,
                ) {
                    Ok(next) => {
                        tree_now = next;
                        if !new_is_a {
                            anchor = id(1_000 + docked);
                        }
                        docked += 1;
                        assert!(docked < 300, "{side:?}: 거절이 안 선다");
                    }
                    Err(e) => break e,
                }
            };
            assert_eq!(err, DockTreeError::ZeroArea, "{side:?}");
            // It stopped at the f64 limit (it was not rejected early).
            assert!(docked > 20, "{side:?}: {docked} 번에서 멈춤");
            assert_eq!(
                zero_area_count(&tree_now),
                0,
                "{side:?}: 받아들인 결과는 전부 면적이 있다"
            );
        }
    }

    #[test]
    fn a_slot_that_already_had_zero_area_elsewhere_does_not_block_docking() {
        // Top = area X · bottom = planted top/bottom halving chain of 60 levels (cannot be made by the write path — the areas at the end have height 0).
        let (chain, _) = half_chain(SplitDir::TopBottom, 60, 10_000);
        let t = split(1, SplitDir::TopBottom, 0.5, slot(2), chain);
        let zero = zero_area_count(&t);
        assert!(zero > 0, "전제: 면적 0 칸이 심겼다");
        let result = docked_tree(
            &t,
            DockAnchor::Slot(id(2)),
            DockSide::Right,
            &slot(9),
            DOCK_NEW_SHARE,
        )
        .unwrap();
        assert_eq!(
            zero_area_count(&result),
            zero,
            "원래 있던 면적 0 칸만 남는다"
        );

        // A zero-size area that was already on the inserted side is not a reason either.
        let (moved, _) = half_chain(SplitDir::TopBottom, 60, 30_000);
        let moved_zero = zero_area_count(&moved);
        assert!(moved_zero > 0, "전제: 끼울 것에 면적 0 칸이 심겼다");
        let result = docked_tree(
            &slot(1),
            DockAnchor::Outer,
            DockSide::Left,
            &moved,
            DOCK_NEW_SHARE,
        )
        .unwrap();
        assert_eq!(zero_area_count(&result), moved_zero);
    }

    #[test]
    fn docking_next_to_a_slot_that_already_had_zero_area_is_zero_area() {
        // If the reference is zero size originally, the inserted area becomes newly zero size — that counts as a new zero size.
        let (t, _) = half_chain(SplitDir::TopBottom, 60, 10_000);
        let flat = geometry::compute(&t)
            .slots
            .into_iter()
            .find(|r| !has_area(r))
            .expect("전제: 면적 0 칸이 심겼다");
        for side in SIDES {
            assert_eq!(
                docked_tree(
                    &t,
                    DockAnchor::Slot(flat.slot_id),
                    side,
                    &slot(9),
                    DOCK_NEW_SHARE
                ),
                Err(DockTreeError::ZeroArea),
                "{side:?}"
            );
        }
    }

    // ── incoming_rect (R4) ─────────────────────────────────────────────────────

    #[test]
    fn incoming_rect_is_bit_identical_to_the_box_of_the_docked_subtree() {
        let t = target();
        for moved in [agent_slot(9), split_tab()] {
            for anchor in [DockAnchor::Slot(id(2)), DockAnchor::Outer] {
                for side in SIDES {
                    let result = docked_tree(&t, anchor, side, &moved, DOCK_NEW_SHARE).unwrap();
                    let fresh = the_new_split(&result, &t, &moved);
                    let g = geometry::compute(&result);
                    let s = *g.splits.iter().find(|s| s.split_id == fresh).unwrap();
                    let (_, new_is_a, _) = shape(side);
                    let want = match (s.dir, new_is_a) {
                        (SplitDir::LeftRight, true) => (s.x0, s.y0, s.at, s.y1),
                        (SplitDir::LeftRight, false) => (s.at, s.y0, s.x1, s.y1),
                        (SplitDir::TopBottom, true) => (s.x0, s.y0, s.x1, s.at),
                        (SplitDir::TopBottom, false) => (s.x0, s.at, s.x1, s.y1),
                    };
                    let got = incoming_rect(&g, &tree::slot_ids(&moved)).unwrap();
                    let bits = |r: (f64, f64, f64, f64)| {
                        [r.0.to_bits(), r.1.to_bits(), r.2.to_bits(), r.3.to_bits()]
                    };
                    assert_eq!(
                        bits((got.x0, got.y0, got.x1, got.y1)),
                        bits(want),
                        "{anchor:?} {side:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn incoming_rect_of_one_slot_is_that_slot_and_of_nothing_is_none() {
        let result = docked_tree(
            &target(),
            DockAnchor::Slot(id(2)),
            DockSide::Right,
            &split_tab(),
            DOCK_NEW_SHARE,
        )
        .unwrap();
        let g = geometry::compute(&result);
        let r = rect_of(&g, id(21));
        assert_eq!(
            incoming_rect(&g, &[id(21)]),
            Some(RectF64 {
                x0: r.x0,
                y0: r.y0,
                x1: r.x1,
                y1: r.y1,
            })
        );
        assert_eq!(incoming_rect(&g, &[]), None);
        assert_eq!(
            incoming_rect(&g, &[id(21), id(77)]),
            None,
            "없는 id 가 섞이면 없음"
        );
    }
}
