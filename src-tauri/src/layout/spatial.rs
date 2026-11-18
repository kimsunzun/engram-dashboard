//! Slot spatial target derivation (ADR-0068) — pure logic yielding direction · neighbor · order from the layout tree. Zero Tauri dependency · zero pixels: it knows no measured rects · `getBoundingClientRect` · window size; the basis for turning a spatial instruction such as "bottom-right" or "right of this slot" into a slot id comes from the logical drawing (tree structure) alone, so a standalone headless test is possible (ADR-0012).
//! 1. The normalized rectangle of a leaf slot is `geometry::compute`'s boundary form as is — the only geometry source inside the shell (ADR-0227).
//! 2. From those rectangles: edge adjacency (neighbor) and order (ordinal).
//!    - neighbor: two slots are adjacent if they touch on the relevant axis (boundary coordinates exactly equal) and the interval overlap on the orthogonal axis is positive.
//!    - ordinal: GLOBAL lexicographic sort by each leaf rect's center `(center_y, center_x)` (top→bottom, ties left→right), 0-based — not tree pre-order but a global center sort: numbered by position on screen. Compared by `f64::total_cmp`; equal centers keep tree pre-order (stable sort) → deterministic. Row/column cohesion is not guaranteed: in an asymmetric split a full-height column can slip between the slots of the left column (order is by center_y alone).
//!    - A zero-size leaf (`x0 >= x1` or `y0 >= y1` — only in a tree deep enough for the f64 boundary to collapse) neither has nor becomes a neighbor and is dropped from corner token candidates; ordering uses the same rule, so it stays in the list.
//! The public surface is only `neighbors`+`ordinal` (ADR-0068): the rectangle is just this computation's input and is not exported.

use uuid::Uuid;

use super::geometry::{self, has_area, SlotRect};
use super::types::LayoutNode;

fn center_x(r: &SlotRect) -> f64 {
    (r.x0 + r.x1) / 2.0
}

fn center_y(r: &SlotRect) -> f64 {
    (r.y0 + r.y1) / 2.0
}

/// Neighbors by direction of one slot (each = adjacent slot id or None). Derived from the logical drawing (pixel-independent). ADR-0068.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct Neighbors {
    #[ts(type = "string | null")]
    pub up: Option<Uuid>,
    #[ts(type = "string | null")]
    pub down: Option<Uuid>,
    #[ts(type = "string | null")]
    pub left: Option<Uuid>,
    #[ts(type = "string | null")]
    pub right: Option<Uuid>,
}

/// Spatial target derivation info for one leaf slot (carried per slot in ViewSnapshot). ADR-0068.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct SlotSpatial {
    #[ts(type = "string")]
    pub slot_id: Uuid,
    pub neighbors: Neighbors,
    /// Order index — global lexicographic by center point `(center_y, center_x)` (top→bottom, ties left→right), 0-based.
    /// For details and the cohesion non-guarantee see the module header §Computation outline.
    #[ts(type = "number")]
    pub ordinal: u32,
}

/// If several candidates exist (e.g. two slots stacked vertically on the right), pick the one with the largest orthogonal overlap (one representative neighbor — the natural target of a directional move).
fn neighbor_in_dir(rects: &[SlotRect], idx: usize, dir: Dir) -> Option<Uuid> {
    let me = &rects[idx];
    if !has_area(me) {
        return None;
    }
    let mut best: Option<(Uuid, f64)> = None;
    for (j, other) in rects.iter().enumerate() {
        if j == idx || !has_area(other) {
            continue;
        }
        // ADR-0227: boundaries are compared exactly (`==`) — the boundary of two touching areas is always one split's `at`, which geometry copies to both descendants, so true adjacency is bit-for-bit equal at any area size. Never go back to an absolute tolerance: an area thinner than it would make the area beyond it look adjacent and skip the thin one, and an orthogonal overlap with the thin area under the threshold would make the neighbor vanish (silently wrong, no panic).
        // An area touching only at a corner can enter the candidates with a few-ulp overlap (larger the deeper the path) when its two endpoints are computed separately by different split paths. The opposite side of my edge is fully covered by leaves, so a true neighbor with far larger overlap pushes it out under the max-overlap rule below — but if my own area is about to collapse to a few ulp thickness this margin vanishes and ties or reversals can occur.
        let (adjacent, overlap) = match dir {
            Dir::Right => (
                me.x1 == other.x0,
                overlap_len(me.y0, me.y1, other.y0, other.y1),
            ),
            Dir::Left => (
                me.x0 == other.x1,
                overlap_len(me.y0, me.y1, other.y0, other.y1),
            ),
            Dir::Down => (
                me.y1 == other.y0,
                overlap_len(me.x0, me.x1, other.x0, other.x1),
            ),
            Dir::Up => (
                me.y0 == other.y1,
                overlap_len(me.x0, me.x1, other.x0, other.x1),
            ),
        };
        if adjacent && overlap > 0.0 {
            match best {
                Some((_, bo)) if bo >= overlap => {}
                _ => best = Some((other.slot_id, overlap)),
            }
        }
    }
    best.map(|(id, _)| id)
}

fn overlap_len(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    a1.min(b1) - a0.max(b0)
}

#[derive(Debug, Clone, Copy)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// Return order = ordinal order.
pub fn compute_spatial(node: &LayoutNode) -> Vec<SlotSpatial> {
    let rects = geometry::compute(node).slots;

    let mut order: Vec<usize> = (0..rects.len()).collect();
    order.sort_by(|&i, &j| {
        let (a, b) = (&rects[i], &rects[j]);
        center_y(a)
            .total_cmp(&center_y(b))
            .then_with(|| center_x(a).total_cmp(&center_x(b)))
    });

    order
        .iter()
        .enumerate()
        .map(|(ord, &i)| SlotSpatial {
            slot_id: rects[i].slot_id,
            neighbors: Neighbors {
                up: neighbor_in_dir(&rects, i, Dir::Up),
                down: neighbor_in_dir(&rects, i, Dir::Down),
                left: neighbor_in_dir(&rects, i, Dir::Left),
                right: neighbor_in_dir(&rects, i, Dir::Right),
            },
            ordinal: ord as u32,
        })
        .collect()
}

/// Spatial/direction token — the symbolic surface by which an LLM/person passes an instruction such as "bottom-right" or "right of this slot" (ADR-0068): tmux-style edge tokens (4 corners) + directions relative to the focus slot (4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpatialToken {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Left,
    Right,
    Up,
    Down,
}

impl SpatialToken {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "top-left" | "topleft" => Some(Self::TopLeft),
            "top-right" | "topright" => Some(Self::TopRight),
            "bottom-left" | "bottomleft" => Some(Self::BottomLeft),
            "bottom-right" | "bottomright" => Some(Self::BottomRight),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "up" | "top" => Some(Self::Up),
            "down" | "bottom" => Some(Self::Down),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::TopLeft => "top-left",
            Self::TopRight => "top-right",
            Self::BottomLeft => "bottom-left",
            Self::BottomRight => "bottom-right",
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

/// The edge token picks "bottom-right" exactly (ADR-0068 acceptance criterion): minimize the distance to the matching corner coordinate of each slot rect — in an L-shape (one area on the left + two areas top and bottom on the right) bottom-right picks the lower right slot.
pub fn resolve_spatial(
    node: &LayoutNode,
    focused: Option<Uuid>,
    token: SpatialToken,
) -> Option<Uuid> {
    let rects = geometry::compute(node).slots;
    match token {
        SpatialToken::TopLeft => corner_slot(&rects, 0.0, 0.0),
        SpatialToken::TopRight => corner_slot(&rects, 1.0, 0.0),
        SpatialToken::BottomLeft => corner_slot(&rects, 0.0, 1.0),
        SpatialToken::BottomRight => corner_slot(&rects, 1.0, 1.0),
        SpatialToken::Left => relative_neighbor(&rects, focused, Dir::Left),
        SpatialToken::Right => relative_neighbor(&rects, focused, Dir::Right),
        SpatialToken::Up => relative_neighbor(&rects, focused, Dir::Up),
        SpatialToken::Down => relative_neighbor(&rects, focused, Dir::Down),
    }
}

/// Pick the slot whose rect corner is closest to corner `(cx,cy)` (a corner of the unit square): the rect corner in that corner's direction is the representative point (bottom-right → the rect's (right,bottom)), minimizing Euclidean distance. Equal distances → the slot seen first (tree pre-order). A zero-size leaf is not a candidate (same rule as the neighbor exclusion).
fn corner_slot(rects: &[SlotRect], cx: f64, cy: f64) -> Option<Uuid> {
    let mut best: Option<(Uuid, f64)> = None;
    for r in rects.iter().filter(|r| has_area(r)) {
        let px = if cx >= 0.5 { r.x1 } else { r.x0 };
        let py = if cy >= 0.5 { r.y1 } else { r.y0 };
        let d = (px - cx).powi(2) + (py - cy).powi(2);
        match best {
            Some((_, bd)) if bd <= d => {}
            _ => best = Some((r.slot_id, d)),
        }
    }
    best.map(|(id, _)| id)
}

fn relative_neighbor(rects: &[SlotRect], focused: Option<Uuid>, dir: Dir) -> Option<Uuid> {
    let fid = focused?;
    let idx = rects.iter().position(|r| r.slot_id == fid)?;
    neighbor_in_dir(rects, idx, dir)
}

#[cfg(test)]
mod tests {
    use super::super::types::{SlotContent, SplitDir};
    use super::*;

    fn single() -> (LayoutNode, Uuid) {
        let node = LayoutNode::new_empty_slot();
        let id = super::super::tree::first_slot_id(&node);
        (node, id)
    }

    fn spatial_of(list: &[SlotSpatial], id: Uuid) -> &SlotSpatial {
        list.iter()
            .find(|s| s.slot_id == id)
            .expect("the slot must exist")
    }

    // `split_in_tree` always splits at 0.5, so a tree whose ratio · shape is set directly is built with this helper.
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

    fn rect_of(node: &LayoutNode, n: u128) -> SlotRect {
        geometry::compute(node)
            .slots
            .into_iter()
            .find(|s| s.slot_id == id(n))
            .expect("the slot must exist")
    }

    // ── single slot ────────────────────────────────────────────────────────────

    #[test]
    fn single_slot_has_no_neighbors_ordinal_zero() {
        let (node, id) = single();
        let sp = compute_spatial(&node);
        assert_eq!(sp.len(), 1);
        let s = spatial_of(&sp, id);
        assert_eq!(s.ordinal, 0);
        assert_eq!(s.neighbors, Neighbors::default_none());
    }

    // ── LeftRight split (left/right neighbors) ───────────────────────────────────

    #[test]
    fn left_right_split_has_left_right_neighbors() {
        let (mut node, left) = single();
        let right =
            super::super::tree::split_in_tree(&mut node, left, SplitDir::LeftRight).unwrap();
        let sp = compute_spatial(&node);

        let l = spatial_of(&sp, left);
        let r = spatial_of(&sp, right);
        assert_eq!(
            l.neighbors.right,
            Some(right),
            "right of the left slot = the right slot"
        );
        assert_eq!(l.neighbors.left, None);
        assert_eq!(l.neighbors.up, None);
        assert_eq!(l.neighbors.down, None);
        assert_eq!(
            r.neighbors.left,
            Some(left),
            "left of the right slot = the left slot"
        );
        assert_eq!(r.neighbors.right, None);
        assert_eq!(l.ordinal, 0);
        assert_eq!(r.ordinal, 1);
    }

    // ── TopBottom split (up/down neighbors) ──────────────────────────────────────

    #[test]
    fn top_bottom_split_has_up_down_neighbors() {
        let (mut node, top) = single();
        let bottom =
            super::super::tree::split_in_tree(&mut node, top, SplitDir::TopBottom).unwrap();
        let sp = compute_spatial(&node);

        let t = spatial_of(&sp, top);
        let b = spatial_of(&sp, bottom);
        assert_eq!(
            t.neighbors.down,
            Some(bottom),
            "below the top slot = the bottom slot"
        );
        assert_eq!(t.neighbors.up, None);
        assert_eq!(t.neighbors.left, None);
        assert_eq!(
            b.neighbors.up,
            Some(top),
            "above the bottom slot = the top slot"
        );
        assert_eq!(b.neighbors.down, None);
        assert_eq!(t.ordinal, 0);
        assert_eq!(b.ordinal, 1);
    }

    // ── L-shape (one area on the left + two areas top and bottom on the right) — bottom-right acceptance criterion ────────────────

    /// Tree: Split{LeftRight, a=Slot(left), b=Split{TopBottom, a=Slot(rt), b=Slot(rb)}} — left = full height on the left, rt = top right, rb = bottom right.
    fn l_shape() -> (LayoutNode, Uuid, Uuid, Uuid) {
        let (mut node, left) = single();
        let rroot =
            super::super::tree::split_in_tree(&mut node, left, SplitDir::LeftRight).unwrap();
        let rb = super::super::tree::split_in_tree(&mut node, rroot, SplitDir::TopBottom).unwrap();
        (node, left, rroot, rb)
    }

    #[test]
    fn l_shape_bottom_right_neighbors() {
        let (node, left, rt, rb) = l_shape();
        let sp = compute_spatial(&node);

        let s_rb = spatial_of(&sp, rb);
        assert_eq!(
            s_rb.neighbors.up,
            Some(rt),
            "above bottom-right = top-right"
        );
        assert_eq!(
            s_rb.neighbors.left,
            Some(left),
            "left of bottom-right = left (full height, so it overlaps)"
        );
        assert_eq!(s_rb.neighbors.down, None);
        assert_eq!(s_rb.neighbors.right, None);

        let s_rt = spatial_of(&sp, rt);
        assert_eq!(
            s_rt.neighbors.down,
            Some(rb),
            "below top-right = bottom-right"
        );
        assert_eq!(s_rt.neighbors.left, Some(left), "left of top-right = left");

        // left (full left side): on the right both rt/rb are candidates → one of the larger overlap (both equal at 0.5). Only existence is asserted.
        let s_left = spatial_of(&sp, left);
        assert!(
            s_left.neighbors.right == Some(rt) || s_left.neighbors.right == Some(rb),
            "right of the left is one of the two right slots"
        );
        assert_eq!(s_left.neighbors.left, None);
    }

    #[test]
    fn l_shape_bottom_right_token_resolves_to_rb() {
        // ★ADR-0068 acceptance criterion★: the "bottom-right" token resolves to the bottom-right slot (rb).
        let (node, left, rt, rb) = l_shape();
        assert_eq!(
            resolve_spatial(&node, None, SpatialToken::BottomRight),
            Some(rb),
            "bottom-right → the bottom-right slot"
        );
        assert_eq!(
            resolve_spatial(&node, None, SpatialToken::TopRight),
            Some(rt),
            "top-right → the top-right slot"
        );
        // top-left / bottom-left → left (full height, so the left is closest for both the top and bottom corners).
        assert_eq!(
            resolve_spatial(&node, None, SpatialToken::TopLeft),
            Some(left),
            "top-left → the left slot"
        );
        assert_eq!(
            resolve_spatial(&node, None, SpatialToken::BottomLeft),
            Some(left),
            "bottom-left → the left slot"
        );
    }

    #[test]
    fn l_shape_ordinal_reading_order() {
        // ordinal = global lexicographic by center (center_y first): left(0.5) · rt(0.25) · rb(0.75), sorted rt → left → rb. The full-height left column slips between the right rt/rb — living proof that column cohesion is not guaranteed (module header).
        let (node, left, rt, rb) = l_shape();
        let sp = compute_spatial(&node);
        assert_eq!(spatial_of(&sp, rt).ordinal, 0, "rt (top) first");
        assert_eq!(spatial_of(&sp, left).ordinal, 1);
        assert_eq!(spatial_of(&sp, rb).ordinal, 2, "rb (bottom) last");
    }

    // ── relative direction (from focus) ──────────────────────────────────────────

    #[test]
    fn relative_direction_from_focus() {
        let (mut node, left) = single();
        let right =
            super::super::tree::split_in_tree(&mut node, left, SplitDir::LeftRight).unwrap();
        assert_eq!(
            resolve_spatial(&node, Some(left), SpatialToken::Right),
            Some(right)
        );
        assert_eq!(
            resolve_spatial(&node, Some(right), SpatialToken::Left),
            Some(left)
        );
        assert_eq!(
            resolve_spatial(&node, Some(right), SpatialToken::Right),
            None
        );
        assert_eq!(resolve_spatial(&node, None, SpatialToken::Right), None);
    }

    #[test]
    fn relative_direction_unknown_focus_is_none() {
        let (node, _id) = single();
        assert_eq!(
            resolve_spatial(&node, Some(Uuid::new_v4()), SpatialToken::Left),
            None,
            "a focused not in the tree → None"
        );
    }

    // ── extreme ratio defense (FIX-4 · ADR-0068 §0) ──────────────────────────────

    #[test]
    fn degenerate_ratio_produces_no_zero_area_leaf() {
        // FIX-4 invariant check: even with ratio=0.0 (extreme), geometry's ratio clamp (`[RATIO_MIN, RATIO_MAX]`) prevents a zero-area leaf. Without it the a-side leaf would have width 0 and this assertion break, and since a zero-size leaf drops out of neighbors the neighbor assertion below would break too (load-bearing: passes only while the guard is alive).
        let (mut node, left) = single();
        let right =
            super::super::tree::split_in_tree(&mut node, left, SplitDir::LeftRight).unwrap();
        // Extreme value — plant directly in the tree a value that never comes in through the write path.
        if let LayoutNode::Split { ratio, .. } = &mut node {
            *ratio = 0.0;
        } else {
            panic!("after split the root must be Split");
        }

        for r in geometry::compute(&node).slots {
            assert!(
                has_area(&r),
                "leaf {:?} must have a non-degenerate area ({r:?}) — ratio clamp guard",
                r.slot_id
            );
        }

        let sp = compute_spatial(&node);
        assert_eq!(sp.len(), 2, "both slots produced (no panic)");
        assert_eq!(spatial_of(&sp, left).neighbors.right, Some(right));
        assert_eq!(spatial_of(&sp, right).neighbors.left, Some(left));
    }

    // ── very thin leaf (exact adjacency — no absolute tolerance) ─────────────────────

    /// Top half = a 0.1 LR chain of five levels (ancestor 100 + four inside), so the innermost area has width 1e-5 and is split in half again (1·2). 3 = width 9e-5, 4·5·6·7 = areas growing wider to its right. Bottom half = one full-width area (8).
    fn thin_tree() -> LayoutNode {
        let lr = SplitDir::LeftRight;
        let chain = split(
            100,
            lr,
            0.1,
            split(
                101,
                lr,
                0.1,
                split(
                    102,
                    lr,
                    0.1,
                    split(
                        103,
                        lr,
                        0.1,
                        split(104, lr, 0.1, split(105, lr, 0.5, slot(1), slot(2)), slot(3)),
                        slot(4),
                    ),
                    slot(5),
                ),
                slot(6),
            ),
            slot(7),
        );
        split(106, SplitDir::TopBottom, 0.5, chain, slot(8))
    }

    #[test]
    fn thin_leaves_keep_exact_neighbors() {
        let tree = thin_tree();
        for n in [1, 2, 3] {
            let r = rect_of(&tree, n);
            assert!(
                r.x1 - r.x0 < 1e-4,
                "the width of leaf {n} must be below 1e-4 — {r:?}"
            );
        }

        let sp = compute_spatial(&tree);
        let nb = |n| spatial_of(&sp, id(n)).neighbors;
        let some = |n| Some(id(n));
        assert_eq!(
            nb(1),
            Neighbors {
                up: None,
                down: some(8),
                left: None,
                right: some(2),
            }
        );
        assert_eq!(
            nb(2),
            Neighbors {
                up: None,
                down: some(8),
                left: some(1),
                right: some(3),
            },
            "does not jump past the thin area (3), and does not lose the overlap (5e-6) with the area below"
        );
        assert_eq!(
            nb(3),
            Neighbors {
                up: None,
                down: some(8),
                left: some(2),
                right: some(4),
            }
        );
        assert_eq!(nb(4).left, some(3));
        assert_eq!(nb(8).up, some(7), "largest overlap = the widest area");

        // The top row has the same center_y, so it is split by center_x alone — left→right even at a 1e-6 difference.
        let order: Vec<Uuid> = sp.iter().map(|s| s.slot_id).collect();
        assert_eq!(order, (1..=8).map(id).collect::<Vec<_>>());
    }

    // ── corner contact ───────────────────────────────────────────────────────────

    #[test]
    fn corner_only_touch_is_not_a_neighbor() {
        // 2×2 grid — the two diagonal areas touch at a single corner point (orthogonal overlap exactly 0). For both root directions.
        for (root, row) in [
            (SplitDir::TopBottom, SplitDir::LeftRight),
            (SplitDir::LeftRight, SplitDir::TopBottom),
        ] {
            // 1 = origin area, 4 = its diagonal. 2 is beside 1 (same root half), 3 is on the opposite root side of 1.
            let grid = split(
                100,
                root,
                0.5,
                split(101, row, 0.5, slot(1), slot(2)),
                split(102, row, 0.5, slot(3), slot(4)),
            );
            let sp = compute_spatial(&grid);
            for s in &sp {
                let n = s.neighbors;
                let all = [n.up, n.down, n.left, n.right];
                let diagonal = match s.slot_id {
                    x if x == id(1) => id(4),
                    x if x == id(4) => id(1),
                    x if x == id(2) => id(3),
                    _ => id(2),
                };
                assert!(
                    !all.contains(&Some(diagonal)),
                    "{root:?}: the diagonal {diagonal:?} of {:?} is not a neighbour",
                    s.slot_id
                );
                assert_eq!(
                    all.iter().filter(|x| x.is_some()).count(),
                    2,
                    "{root:?}: each grid cell has exactly two neighbours sharing an edge"
                );
            }
        }
    }

    /// The left column splits horizontally at 0.65 and the right column at 0.3 + (1 − 0.3)·0.5 — the same height in decimal but computed separately, so the right boundary is 1 ulp lower. 1 = top left · 2 = bottom left · 3·4·5 = top·middle·bottom right.
    fn ulp_tree() -> LayoutNode {
        split(
            100,
            SplitDir::LeftRight,
            0.5,
            split(101, SplitDir::TopBottom, 0.65, slot(1), slot(2)),
            split(
                102,
                SplitDir::TopBottom,
                0.3,
                slot(3),
                split(103, SplitDir::TopBottom, 0.5, slot(4), slot(5)),
            ),
        )
    }

    #[test]
    fn one_ulp_corner_candidate_loses_to_true_neighbor() {
        let tree = ulp_tree();
        let (lt, lb, rb) = (rect_of(&tree, 1), rect_of(&tree, 2), rect_of(&tree, 5));
        // Measure the premise — without the 1 ulp mismatch this test measures nothing.
        assert_eq!(
            lt.x1, rb.x0,
            "same root boundary — passes the adjacency decision"
        );
        assert_eq!(
            lt.y1.to_bits() - rb.y0.to_bits(),
            1,
            "the top boundary of 5 is exactly 1 ulp lower than the bottom boundary of 1 — a corner touch looks like a positive overlap"
        );
        assert!(rb.y0 < lt.y1 && lb.y1 > rb.y0);

        let sp = compute_spatial(&tree);
        // The left candidates of 5 in pre-order are 1 (overlap 1 ulp, false) first and 2 (true) later — a "pick the first seen" rule would pick 1.
        assert_eq!(
            spatial_of(&sp, id(5)).neighbors.left,
            Some(id(2)),
            "largest-overlap rule — the true neighbour, not the 1 ulp candidate"
        );
        // From 1's side too, 5 is only a 1 ulp candidate — the right neighbor is 4, which has the largest overlap.
        assert_eq!(spatial_of(&sp, id(1)).neighbors.right, Some(id(4)));
        assert_eq!(spatial_of(&sp, id(2)).neighbors.right, Some(id(5)));
    }

    // ── zero-size leaf (value planted directly in the tree — defense) ────────────────

    /// An LR chain always splitting the b side at 0.9 — at the 17th level the boundary sticks to 1.0 by rounding and the last b leaf (18) gets width exactly 0. Leaves = 1..=17 (a of each level) + 18.
    fn collapsed_chain() -> LayoutNode {
        const DEPTH: u128 = 17;
        let mut node = slot(DEPTH + 1);
        for k in (1..=DEPTH).rev() {
            node = split(100 + k, SplitDir::LeftRight, 0.9, slot(k), node);
        }
        node
    }

    #[test]
    fn zero_area_leaf_is_isolated_but_ordered() {
        let tree = collapsed_chain();
        let g = geometry::compute(&tree);
        let z = rect_of(&tree, 18);
        assert_eq!(z.x0, z.x1, "18 must have width 0 — {z:?}");
        assert_eq!(
            g.slots.iter().filter(|r| !has_area(r)).count(),
            1,
            "18 is the only zero-area leaf"
        );
        // 17 and 18 share boundary 1.0 exactly — without the skipping they would be neighbors.
        assert_eq!(rect_of(&tree, 17).x1, z.x0);

        let sp = compute_spatial(&tree);
        assert_eq!(
            sp.len(),
            18,
            "a zero-area leaf is in the list too (no panic)"
        );
        assert_eq!(spatial_of(&sp, id(18)).neighbors, Neighbors::default_none());
        for s in &sp {
            let n = s.neighbors;
            assert!(
                ![n.up, n.down, n.left, n.right].contains(&Some(id(18))),
                "a zero-area leaf must not be a neighbour of {:?}",
                s.slot_id
            );
        }
        assert_eq!(spatial_of(&sp, id(17)).neighbors.right, None);
        assert_eq!(spatial_of(&sp, id(17)).neighbors.left, Some(id(16)));

        // 17 and 18 are equal even at the center point — ties go by tree pre-order (17 first), so the order equals pre-order.
        let r17 = rect_of(&tree, 17);
        assert_eq!(
            (center_y(&r17), center_x(&r17)),
            (center_y(&z), center_x(&z))
        );
        let order: Vec<Uuid> = sp.iter().map(|s| s.slot_id).collect();
        assert_eq!(order, (1..=18).map(id).collect::<Vec<_>>());

        assert_eq!(
            compute_spatial(&tree),
            sp,
            "computing twice gives the same result"
        );
    }

    #[test]
    fn corner_token_skips_zero_area_leaf() {
        // A corner token picks the area seen first in pre-order on a distance tie, so for a zero-size leaf to take a corner it must be first in pre-order — the a area at the origin side. The origin-side boundary does not stick by rounding but becomes 0 only by underflow, so a 0.1 LR chain of 324 levels is stacked. z(1000) = innermost a leaf · k = b leaf of the k-th level.
        const DEPTH: u128 = 324;
        let lr = SplitDir::LeftRight;
        let mut node = split(10_000 + DEPTH, lr, 0.1, slot(1000), slot(DEPTH));
        for k in (1..DEPTH).rev() {
            node = split(10_000 + k, lr, 0.1, node, slot(k));
        }

        let g = geometry::compute(&node);
        let z = rect_of(&node, 1000);
        assert_eq!(
            (z.x0, z.x1),
            (0.0, 0.0),
            "z must have width 0 at the origin"
        );
        assert_eq!(g.slots[0].slot_id, id(1000), "z is first in preorder");
        assert_eq!(
            g.slots.iter().filter(|r| !has_area(r)).count(),
            1,
            "z is the only zero-area leaf"
        );

        for token in [SpatialToken::TopLeft, SpatialToken::BottomLeft] {
            assert_eq!(
                resolve_spatial(&node, None, token),
                Some(id(DEPTH)),
                "{token:?} → the next area, skipping z"
            );
        }
        assert_eq!(
            resolve_spatial(&node, None, SpatialToken::TopRight),
            Some(id(1))
        );

        let sp = compute_spatial(&node);
        assert_eq!(sp.len(), DEPTH as usize + 1);
        assert_eq!(
            spatial_of(&sp, id(1000)).neighbors,
            Neighbors::default_none()
        );
        let inner = spatial_of(&sp, id(DEPTH)).neighbors;
        assert_eq!(inner.left, None, "z is nobody's neighbour");
        assert_eq!(inner.right, Some(id(DEPTH - 1)));
        assert_eq!(
            compute_spatial(&node),
            sp,
            "computing twice gives the same result"
        );
    }

    // ── determinism ──────────────────────────────────────────────────────────────

    #[test]
    fn ordering_is_deterministic_and_complete() {
        let nan = split(
            100,
            SplitDir::TopBottom,
            f64::NAN,
            slot(1),
            split(101, SplitDir::LeftRight, f64::NAN, slot(2), slot(3)),
        );
        for tree in [thin_tree(), ulp_tree(), collapsed_chain(), nan] {
            let first = compute_spatial(&tree);
            assert_eq!(compute_spatial(&tree), first, "same tree = same result");
            let ordinals: Vec<u32> = first.iter().map(|s| s.ordinal).collect();
            assert_eq!(ordinals, (0..first.len() as u32).collect::<Vec<_>>());
            assert_eq!(
                first.len(),
                geometry::compute(&tree).slots.len(),
                "every leaf is in the list"
            );
        }
    }

    #[test]
    fn token_parse_kebab_and_aliases() {
        assert_eq!(
            SpatialToken::parse("bottom-right"),
            Some(SpatialToken::BottomRight)
        );
        assert_eq!(
            SpatialToken::parse("BottomRight"),
            Some(SpatialToken::BottomRight)
        );
        assert_eq!(
            SpatialToken::parse(" top-left "),
            Some(SpatialToken::TopLeft)
        );
        assert_eq!(SpatialToken::parse("up"), Some(SpatialToken::Up));
        assert_eq!(SpatialToken::parse("top"), Some(SpatialToken::Up));
        assert_eq!(SpatialToken::parse("nonsense"), None);
    }
}

impl Neighbors {
    #[cfg(test)]
    fn default_none() -> Self {
        Neighbors {
            up: None,
            down: None,
            left: None,
            right: None,
        }
    }
}
