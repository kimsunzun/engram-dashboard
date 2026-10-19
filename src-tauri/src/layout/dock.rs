//! 도킹 트리 연산 — 탭 하나의 배치(`incoming`)를 대상 탭 트리의 슬롯 옆이나 바깥 변에 끼운 **결과 트리**를 새로
//! 만든다(PRD R7–R9 · TRD S21-slot-docking §1-2). `tree`·`geometry` 만 알고 Tauri·락·I/O 를 모른다 →
//! 단독 headless 테스트가 된다(ADR-0012 격리).
//!
//! 진입점 = [`docked_tree`](결과 트리 · 새 면적 0 거절) · [`incoming_rect`](결과 기하에서 끼운 것의 사각형).
//! 가이드 · 놓기 판정 · 적용이 같은 `docked_tree` 를 쓴다 — 칠한 사각형과 놓은 결과가 한 식에서 나온다(PRD R4).
//!
//! ★불변식★: 입력 노드의 슬롯 id · 분할 id 를 바꾸지 않는다 — 새로 뽑는 id 는 감싸는 분할 하나뿐이다(ADR-0223).

use std::collections::HashSet;

use uuid::Uuid;

use super::geometry::{self, has_area, LayoutGeometry, RectF64};
use super::tree;
use super::types::{LayoutNode, SlotContent, SplitDir};

/// 붙일 쪽 — 이름이 결과 배치를 말한다. `Left` = 기준의 왼쪽에 새것 · `Right` = 오른쪽 · `Top` = 위 · `Bottom` = 아래.
// ★`#[serde(rename_all)]` 을 달지 말 것★ — 버스 선언 매크로는 rename 을 못 달아 variant 이름이 그대로 wire 값이
// 된다. invoke 와 버스가 같은 철자(`Left` …)를 쓰게 둔다(TRD S21-slot-docking §3-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export)]
pub enum DockSide {
    Left,
    Right,
    Top,
    Bottom,
}

/// 끼울 기준 — `Slot` = 대상 트리의 그 슬롯 하나 · `Outer` = 대상 트리 전체(루트 — 바깥 변).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockAnchor {
    Slot(Uuid),
    Outer,
}

/// 고정 새것 몫 — 시험만 쓴다. 끌기의 몫은 이 값이 아니라 `dock::sizing::attach_share` 가 낸다.
pub const DOCK_NEW_SHARE: f64 = 1.0 / 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockTreeError {
    /// 기준 슬롯이 대상 트리에 없다.
    SlotNotFound(Uuid),
    /// 대상과 끼울 것에 같은 슬롯 id 가 있다 — 결과에서 칸 하나를 두 자리로 읽게 된다(슬롯 id 는 창 간 전역 고유 —
    /// ADR-0035).
    SlotIdOverlap,
    /// 결과에 **새로** 면적 0 인 칸이 생긴다(PRD R8).
    ZeroArea,
}

/// `target` 의 `anchor` 를 새 분할로 감싸 `side` 쪽에 `incoming` 을 끼운 결과 트리. 입력은 건드리지 않는다.
///
/// - 모양: `Left`·`Right` = `LeftRight` 분할, `Top`·`Bottom` = `TopBottom` 분할. `share` = 감싼 자리에서 새것이
///   차지하는 몫 — `tree::RATIO_MIN ≤ share ≤ 0.5` 여야 한다(아래면 기하가 다른 비율로 읽는다 · 밖이면 디버그
///   빌드에서 단언한다). 새것이 a(왼쪽/위)면 비율 `share`, b 면 `1 − share`(비율 = a 몫 — ADR-0140). 감싼 기준
///   안쪽의 비율은 그대로라 슬롯 옆이면 그 슬롯만, 바깥 변이면 대상 전체가 안쪽 비율을 지킨 채 줄어든다.
/// - id: 감싸는 분할만 새 id 다. 대상 · `incoming` 의 슬롯 id · 분할 id · 내용은 그대로 옮긴다 — 출처에서 지우는
///   것은 호출자 몫이다.
/// - 오류(이 순서로 보고 처음 걸린 것을 낸다):
///   1. 기준 슬롯이 `target` 에 없으면 `SlotNotFound`.
///   2. 두 트리에 같은 슬롯 id 가 있으면 `SlotIdOverlap`.
///   3. 결과에서 면적 0 인 칸 중 원래 자리(대상 칸은 `target` 기하, 끼운 칸은 `incoming` 단독 기하)에서는 면적이
///      있던 칸이 하나라도 있으면 `ZeroArea` — 원래부터 면적 0 이던 칸은 사유가 아니다. 좌표는 정규화 레이아웃
///      좌표다(화면 px 아님).
pub fn docked_tree(
    target: &LayoutNode,
    anchor: DockAnchor,
    side: DockSide,
    incoming: &LayoutNode,
    share: f64,
) -> Result<LayoutNode, DockTreeError> {
    // ADR-0295
    debug_assert!((tree::RATIO_MIN..=0.5).contains(&share), "새것 몫 {share}");
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
        id: Uuid::new_v4(), // ADR-0223 — 분할마다 새 id
        dir,
        ratio,
        a,
        b,
    };

    // 원래부터 면적 0 이던 칸은 빼고 본다 — 안 빼면 그런 칸 하나가 그 탭으로의 도킹을 전부 막는다
    // (`ViewManager::set_split_ratio` ⑤ 와 같은 태도).
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

/// 결과 기하에서 `incoming_slots` 가 차지하는 정규화 사각형 = 그 칸들의 외접 사각형(PRD R4 「놓았을 때의 실제
/// 사각형」). 경계를 새로 계산하지 않고 칸 경계 값을 고르기만 하므로, 끼운 서브트리의 칸을 전부 주면 결과 기하의
/// 그 서브트리 상자와 비트 단위로 같다(`geometry` 머리 불변식). 목록이 비었거나 `result` 에 없는 id 가 하나라도
/// 있으면 `None`. `result` 의 칸 id 는 고유하다고 본다 — `docked_tree` 결과는 `SlotIdOverlap` 거절로 그것이 선다.
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

    // TRD §1-2 분할 모양 표 — (방향, 새것이 a 인가, 비율 = a 몫).
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

    // Split(100, 좌우 0.3){1, Split(101, 위아래 0.6){2, 3}}
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

    // Split(200, 좌우 0.3){21, 22}
    fn split_tab() -> LayoutNode {
        split(200, SplitDir::LeftRight, 0.3, slot(21), slot(22))
    }

    fn split_ids(node: &LayoutNode) -> Vec<Uuid> {
        tree::list_splits(node).into_iter().map(|s| s.id).collect()
    }

    // 결과의 분할 = 입력의 분할 전부 + 새것 하나 — 그 새 id 를 돌려준다.
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

    // 반분 사슬 — 늘 b 쪽이 다음 단이다. 반환 = (트리, 가장 깊은 칸 id). 칸 id = `base..=base+depth`,
    // 분할 id = `base+1001..`.
    fn half_chain(dir: SplitDir, depth: u128, base: u128) -> (LayoutNode, Uuid) {
        let mut node = slot(base);
        for i in 1..=depth {
            node = split(base + 1000 + i, dir, 0.5, slot(base + i), node);
        }
        (node, id(base))
    }

    // ── 몫 · 철자 ──────────────────────────────────────────────────────────────

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
        // ADR-0295 — 붙는 몫(PRD R7)은 부르는 쪽이 고른다.
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

    // ── 모양 · id 보존 ─────────────────────────────────────────────────────────

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
        // AC5 · R9 — 끼운 서브트리는 출처 노드 그대로(슬롯 id · 분할 id · 비율).
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
            // 기준은 대상 트리에서만 찾는다 — 끼울 쪽의 칸은 기준이 아니다.
            assert_eq!(
                docked_tree(&t, DockAnchor::Slot(id(21)), side, &moved, DOCK_NEW_SHARE),
                Err(DockTreeError::SlotNotFound(id(21)))
            );
        }
    }

    #[test]
    fn a_slot_id_in_both_trees_is_slot_id_overlap() {
        let t = target();
        // 칸 3 이 양쪽에 있다.
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
        // 판정 순서 — 기준 슬롯이 먼저다.
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
        // 판정 순서 — 겹침이 면적 0 보다 먼저다(이 기준은 원래 면적 0 이라 겹침이 없으면 `ZeroArea`).
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

    // ── 비율 · 기하 (R7 · AC3 · AC4) ──────────────────────────────────────────

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
            // 둘이 원래 칸을 정확히 나눠 갖는다 — 맞닿는 경계도 같은 값.
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
        // AC4 — 기존 칸들은 대상 영역 안에서 서로의 비율을 그대로 갖는다.
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
            // 대상이 차지한 영역 = 남은 2/3.
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

    // ── 새 면적 0 (R8 · AC15) ─────────────────────────────────────────────────

    #[test]
    fn a_deep_child_of_the_incoming_tab_that_would_collapse_is_zero_area() {
        // 대상: 좌우 반분 사슬 40 단 — 가장 깊은 칸 폭 2^-40(오른쪽 끝).
        let (t, deepest) = half_chain(SplitDir::LeftRight, 40, 10_000);
        assert_eq!(zero_area_count(&t), 0, "전제: 대상은 전부 면적이 있다");
        // 끼울 것: 좌우 반분 사슬 20 단 — 단독으로는 가장 깊은 칸도 폭 2^-20.
        let (moved, _) = half_chain(SplitDir::LeftRight, 20, 30_000);
        assert_eq!(
            zero_area_count(&moved),
            0,
            "전제: 끼울 것도 전부 면적이 있다"
        );

        // 대조: 칸 하나면 붙는다 — 끼울 자리의 1/3 자체에는 면적이 있다.
        let lone = docked_tree(
            &t,
            DockAnchor::Slot(deepest),
            DockSide::Right,
            &slot(9),
            DOCK_NEW_SHARE,
        )
        .unwrap();
        assert_eq!(zero_area_count(&lone), 0);
        // 사슬이면 그 안쪽 깊은 칸이 무너진다.
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
        // 대조: 넓은 자리(바깥 변)에는 같은 사슬이 붙는다 — 일괄 거절이 아니다.
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
        // 대상: 좌우 반분 사슬 53 단 — 단독으로는 전부 면적이 있지만 1.0 쪽 끝 칸들이 f64 한계 바로 앞이다.
        let (t, _) = half_chain(SplitDir::LeftRight, 53, 10_000);
        assert_eq!(
            zero_area_count(&t),
            0,
            "전제: 대상은 단독으로 전부 면적이 있다"
        );
        // 전제: 바깥 왼쪽에 붙이면 대상이 2/3 로 줄며 대상 칸이 무너지고, 끼운 칸에는 면적이 있다.
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
        // 대조: 얕은 사슬은 같은 자리에 붙는다.
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
            // 늘 1.0 쪽 끝 칸에 붙인다 — 새것이 b 면 방금 붙인 칸, a 면 처음 칸이 그 끝이다.
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
            // f64 한계에서 멈췄다(일찍 거절한 게 아니다).
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
        // 위 = 칸 X · 아래 = 심은 위아래 반분 사슬 60 단(쓰기 경로로는 못 만든다 — 끝 쪽 칸들의 높이가 0).
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

        // 끼울 쪽에 원래 있던 면적 0 칸도 사유가 아니다.
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
        // 기준이 원래 면적 0 이면 끼운 칸이 새로 면적 0 이 된다 — 그건 새 면적 0 이다.
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
