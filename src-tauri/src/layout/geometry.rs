//! Shell geometry — pure computation that converts a layout tree into view-normalized rectangles (boundary form) and integer CSS px (ADR-0227).
//! It has no dependency on Tauri, locks or I/O, so it becomes a standalone headless test (ADR-0012 isolation).
//!
//! This is the area and split geometry that the shell and the screen use together. Entry points = `compute` (tree → area and split rectangles) ·
//! `frame_px`/`px_edge` (normalized → integer px) · `content_rect` (frame − inner margin).
//!
//! ★Invariant — touching boundaries are bit-for-bit equal★: a child copies the parent's boundary values as they are, and the only
//! value computed anew is the split boundary `at`. The end boundary of a and the start boundary of b are the same `at`, so
//! `==` holds regardless of depth. Do not rebuild a boundary as `x + w` — it diverges in the floating-point last digits and leaves a hairline gap on screen.

use ts_rs::TS;
use uuid::Uuid;

use super::tree::clamp_ratio;
use super::types::{LayoutNode, SplitDir};

/// Rectangle of one area — view-normalized [0,1] boundary form (`x0 <= x1`, `y0 <= y1`).
/// Boundary values that touch a neighboring area are bit-for-bit equal (invariant in the module header of `src-tauri/src/layout/geometry.rs`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct SlotRect {
    #[ts(type = "string")]
    pub slot_id: Uuid,
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// One split node — this split's own box (the region of both children combined, view-normalized [0,1] boundary form) + boundary coordinate `at`.
/// `at` is an x-axis value if `dir` is `left_right` (a = left), and a y-axis value if `top_bottom` (a = top).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct SplitRect {
    #[ts(type = "string")]
    pub split_id: Uuid,
    pub dir: SplitDir,
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub at: f64,
}

/// Result of `compute`. Both lists are in tree preorder (node → a subtree → b subtree) — sorting by id is the consumer's job.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutGeometry {
    pub slots: Vec<SlotRect>,
    pub splits: Vec<SplitRect>,
}

/// Integer CSS px boundary form (relative to the canvas origin). Width = `x1 - x0` — the difference of two rounded boundaries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PxRect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

/// Inner margin of the area frame (CSS px, fractions allowed — depending on scale the border width can measure as a fraction).
// It rides in `UiMetrics` (the webview report wire), so it carries serde and ts-rs. `PxRect` and `RectF64` are shell-internal, so they do not.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct Insets {
    pub t: f64,
    pub r: f64,
    pub b: f64,
    pub l: f64,
}

/// f64 boundary-form rectangle (`x0 <= x1`, `y0 <= y1`). The unit is set by the function that makes it — `content_rect` gives CSS px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectF64 {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// Root area = (0,0,1,1). Tree ratios are read clamped to `[RATIO_MIN, RATIO_MAX]`, and NaN is read as 0.5
/// (±∞ is sent to the limit by the clamp). So every coordinate is finite and inside [0,1].
pub fn compute(node: &LayoutNode) -> LayoutGeometry {
    let mut out = LayoutGeometry {
        slots: Vec::new(),
        splits: Vec::new(),
    };
    let root = RectF64 {
        x0: 0.0,
        y0: 0.0,
        x1: 1.0,
        y1: 1.0,
    };
    walk(node, root, &mut out);
    out
}

fn walk(node: &LayoutNode, r: RectF64, out: &mut LayoutGeometry) {
    match node {
        LayoutNode::Slot { id, .. } => out.slots.push(SlotRect {
            slot_id: *id,
            x0: r.x0,
            y0: r.y0,
            x1: r.x1,
            y1: r.y1,
        }),
        LayoutNode::Split {
            id,
            dir,
            ratio,
            a,
            b,
        } => {
            // ADR-0140: LeftRight = x-axis split (a left) · TopBottom = y-axis split (a top).
            let (at, ra, rb) = match dir {
                SplitDir::LeftRight => {
                    let at = boundary(r.x0, r.x1, *ratio);
                    (at, RectF64 { x1: at, ..r }, RectF64 { x0: at, ..r })
                }
                SplitDir::TopBottom => {
                    let at = boundary(r.y0, r.y1, *ratio);
                    (at, RectF64 { y1: at, ..r }, RectF64 { y0: at, ..r })
                }
            };
            out.splits.push(SplitRect {
                split_id: *id,
                dir: *dir,
                x0: r.x0,
                y0: r.y0,
                x1: r.x1,
                y1: r.y1,
                at,
            });
            walk(a, ra, out);
            walk(b, rb, out);
        }
    }
}

/// Boundary coordinate that splits the axis interval `[lo, hi]` by ratio `ratio` (a's share) — exactly the formula `compute` uses for every split.
/// The split guard also calls this when it measures a new boundary ahead of time, so it never differs from the real computation by even one bit.
// ADR-0227
pub fn boundary(lo: f64, hi: f64, ratio: f64) -> f64 {
    // Even though the write path guarantees limits and finiteness, values planted directly in the tree are also filtered here — 0.0 etc. are cut so they do not make
    // zero-size leaves, and NaN passes the clamp unchanged and spreads to all descendant coordinates, so it is read as a halving.
    let t = if ratio.is_nan() {
        0.5
    } else {
        clamp_ratio(ratio)
    };
    lo + (hi - lo) * t
}

/// Strictly `x0 < x1` · `y0 < y1` on both axes. A zero-size area arises only in a tree
/// deep enough for the f64 boundary to collapse.
// ADR-0227
pub(crate) fn has_area(r: &SlotRect) -> bool {
    r.x0 < r.x1 && r.y0 < r.y1
}

/// Normalized boundary `e` as an integer px boundary on canvas axis length `canvas` = `round(canvas · e)`.
///
/// Ties go to the side farther from 0 — since the input is non-negative, this equals CSS `round(nearest)` (toward +∞). Area width is got from the difference
/// of two such values and the width is not rounded separately — so the px boundaries of neighboring areas are always the same integer.
/// An out-of-range `e` follows the `as` cast saturation (negative·NaN → 0, over `u32` → `u32::MAX`).
pub fn px_edge(canvas: u32, e: f64) -> u32 {
    (f64::from(canvas) * e).round() as u32
}

pub fn frame_px(canvas_w: u32, canvas_h: u32, r: &SlotRect) -> PxRect {
    PxRect {
        x0: px_edge(canvas_w, r.x0),
        y0: px_edge(canvas_h, r.y0),
        x1: px_edge(canvas_w, r.x1),
        y1: px_edge(canvas_h, r.y1),
    }
}

/// Content region (CSS px) of the frame shrunk by the margin. Width and height do not go below 0 (`x1 >= x0`, `y1 >= y0`).
/// If the frame is narrower than the sum of margins, `x0` stays `frame.x0 + l` as is, so it can lie beyond the frame's right boundary.
pub fn content_rect(frame: &PxRect, insets: &Insets) -> RectF64 {
    let x0 = f64::from(frame.x0) + insets.l;
    let y0 = f64::from(frame.y0) + insets.t;
    RectF64 {
        x0,
        y0,
        x1: (f64::from(frame.x1) - insets.r).max(x0),
        y1: (f64::from(frame.y1) - insets.b).max(y0),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tree::{RATIO_MAX, RATIO_MIN};
    use super::super::types::SlotContent;
    use super::*;

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

    fn slot_of(g: &LayoutGeometry, n: u128) -> SlotRect {
        *g.slots
            .iter()
            .find(|s| s.slot_id == id(n))
            .expect("슬롯 있어야")
    }

    fn split_of(g: &LayoutGeometry, n: u128) -> SplitRect {
        *g.splits
            .iter()
            .find(|s| s.split_id == id(n))
            .expect("분할 있어야")
    }

    fn same(a: f64, b: f64) -> bool {
        a.to_bits() == b.to_bits()
    }

    // Ratios as values with no binary representation — a tree where last-digit error actually arises.
    fn nested() -> LayoutNode {
        split(
            100,
            SplitDir::LeftRight,
            0.3,
            split(
                101,
                SplitDir::TopBottom,
                0.7,
                slot(1),
                split(102, SplitDir::LeftRight, 0.35, slot(2), slot(3)),
            ),
            split(103, SplitDir::TopBottom, 0.45, slot(4), slot(5)),
        )
    }

    // ── single slot · direction ───────────────────────────────────────────────

    #[test]
    fn single_slot_is_unit_square() {
        let g = compute(&slot(1));
        assert_eq!(
            g.slots,
            vec![SlotRect {
                slot_id: id(1),
                x0: 0.0,
                y0: 0.0,
                x1: 1.0,
                y1: 1.0,
            }]
        );
        assert!(g.splits.is_empty());
    }

    #[test]
    fn left_right_puts_a_on_the_left() {
        let g = compute(&split(100, SplitDir::LeftRight, 0.25, slot(1), slot(2)));
        let a = slot_of(&g, 1);
        let b = slot_of(&g, 2);
        assert_eq!((a.x0, a.y0, a.x1, a.y1), (0.0, 0.0, 0.25, 1.0), "a = 왼쪽");
        assert_eq!(
            (b.x0, b.y0, b.x1, b.y1),
            (0.25, 0.0, 1.0, 1.0),
            "b = 오른쪽"
        );
        assert_eq!(
            split_of(&g, 100),
            SplitRect {
                split_id: id(100),
                dir: SplitDir::LeftRight,
                x0: 0.0,
                y0: 0.0,
                x1: 1.0,
                y1: 1.0,
                at: 0.25,
            }
        );
    }

    #[test]
    fn top_bottom_puts_a_on_top() {
        let g = compute(&split(100, SplitDir::TopBottom, 0.75, slot(1), slot(2)));
        let a = slot_of(&g, 1);
        let b = slot_of(&g, 2);
        assert_eq!((a.x0, a.y0, a.x1, a.y1), (0.0, 0.0, 1.0, 0.75), "a = 위");
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (0.0, 0.75, 1.0, 1.0), "b = 아래");
        let s = split_of(&g, 100);
        assert_eq!(s.dir, SplitDir::TopBottom);
        assert_eq!(s.at, 0.75, "위아래 분할의 at 은 y 축 값");
    }

    // ── neighbor boundaries bit-identical · split boxes ───────────────────────

    #[test]
    fn shared_boundaries_are_bit_identical_in_three_level_nesting() {
        let g = compute(&nested());
        let (s1, s2, s3, s4, s5) = (
            slot_of(&g, 1),
            slot_of(&g, 2),
            slot_of(&g, 3),
            slot_of(&g, 4),
            slot_of(&g, 5),
        );

        // Root LR boundary: right end of the left column = left end of the right column.
        assert!(same(s1.x1, s4.x0) && same(s1.x1, s5.x0));
        assert!(same(s3.x1, s4.x0), "깊은 잎의 오른쪽 끝도 같은 값");
        // TB(0.7) boundary: bottom of s1 = top of s2·s3.
        assert!(same(s1.y1, s2.y0) && same(s1.y1, s3.y0));
        // LR(0.35) boundary.
        assert!(same(s2.x1, s3.x0));
        // TB(0.45) boundary.
        assert!(same(s4.y1, s5.y0));
        // The parent's end boundary is copied — a leaf three levels down also has exactly the view end (1.0).
        assert!(same(s2.y1, 1.0) && same(s3.y1, 1.0) && same(s5.y1, 1.0));
        assert!(same(s1.x0, 0.0) && same(s2.x0, 0.0));

        // Split box = region of both children combined · at = boundary between the two children.
        let root = split_of(&g, 100);
        assert_eq!((root.x0, root.y0, root.x1, root.y1), (0.0, 0.0, 1.0, 1.0));
        assert!(same(root.at, s1.x1));
        let inner = split_of(&g, 102);
        assert!(same(inner.x0, s2.x0) && same(inner.x1, s3.x1));
        assert!(same(inner.y0, s2.y0) && same(inner.y1, s2.y1));
        assert!(same(inner.at, s2.x1) && same(inner.at, s3.x0));
        let right = split_of(&g, 103);
        assert!(same(right.x0, s4.x0) && same(right.x1, 1.0));
        assert!(same(right.at, s4.y1));
    }

    #[test]
    fn deep_chain_keeps_boundaries_bit_identical_and_ordered() {
        const DEPTH: u128 = 12;
        let mut node = slot(DEPTH);
        for i in (0..DEPTH).rev() {
            node = split(100 + i, SplitDir::LeftRight, 0.3, slot(i), node);
        }
        let g = compute(&node);
        assert_eq!(g.slots.len() as u128, DEPTH + 1);
        for i in 0..DEPTH as usize {
            let (a, b) = (g.slots[i], g.slots[i + 1]);
            assert!(same(a.x1, b.x0), "{i} 단 경계가 갈렸다");
            assert!(same(g.splits[i].at, a.x1));
            assert!(a.x0 < a.x1, "{i} 단 잎 폭이 양수여야");
        }
        assert!(same(g.slots[DEPTH as usize].x1, 1.0));
    }

    #[test]
    fn lists_are_in_tree_pre_order() {
        // Shallow leaves (s2·s3) come after deep leaves (s1·s4) and ids are not monotonic either — if it regressed to breadth-first, id order, reverse id order or
        // postorder, at least one of the two lists would be off.
        let g = compute(&split(
            103,
            SplitDir::LeftRight,
            0.5,
            split(
                100,
                SplitDir::TopBottom,
                0.5,
                slot(5),
                split(102, SplitDir::LeftRight, 0.5, slot(1), slot(4)),
            ),
            split(101, SplitDir::TopBottom, 0.5, slot(2), slot(3)),
        ));
        let slots: Vec<Uuid> = g.slots.iter().map(|s| s.slot_id).collect();
        let splits: Vec<Uuid> = g.splits.iter().map(|s| s.split_id).collect();
        assert_eq!(slots, vec![id(5), id(1), id(4), id(2), id(3)]);
        assert_eq!(splits, vec![id(103), id(100), id(102), id(101)]);
    }

    // ── exact values (nesting inside a rectangular box) ───────────────────────

    fn rect(n: u128, x0: f64, y0: f64, x1: f64, y1: f64) -> SlotRect {
        SlotRect {
            slot_id: id(n),
            x0,
            y0,
            x1,
            y1,
        }
    }

    fn srect(n: u128, dir: SplitDir, x0: f64, y0: f64, x1: f64, y1: f64, at: f64) -> SplitRect {
        SplitRect {
            split_id: id(n),
            dir,
            x0,
            y0,
            x1,
            y1,
            at,
        }
    }

    // Includes a split inside a non-square box and a same-direction split (103) inside a box whose origin is not 0 — a mistake of computing `at` with
    // the other axis's length·origin or dropping the origin shows up in the values. The ratios are values with an
    // exact binary representation, so they are asserted with `==`.
    #[test]
    fn nested_left_right_exact_values() {
        let g = compute(&split(
            100,
            SplitDir::LeftRight,
            0.25,
            split(101, SplitDir::TopBottom, 0.5, slot(1), slot(2)),
            split(
                102,
                SplitDir::TopBottom,
                0.75,
                split(103, SplitDir::LeftRight, 0.5, slot(3), slot(4)),
                slot(5),
            ),
        ));
        assert_eq!(
            g.slots,
            vec![
                rect(1, 0.0, 0.0, 0.25, 0.5),
                rect(2, 0.0, 0.5, 0.25, 1.0),
                rect(3, 0.25, 0.0, 0.625, 0.75),
                rect(4, 0.625, 0.0, 1.0, 0.75),
                rect(5, 0.25, 0.75, 1.0, 1.0),
            ]
        );
        assert_eq!(
            g.splits,
            vec![
                srect(100, SplitDir::LeftRight, 0.0, 0.0, 1.0, 1.0, 0.25),
                srect(101, SplitDir::TopBottom, 0.0, 0.0, 0.25, 1.0, 0.5),
                srect(102, SplitDir::TopBottom, 0.25, 0.0, 1.0, 1.0, 0.75),
                srect(103, SplitDir::LeftRight, 0.25, 0.0, 1.0, 0.75, 0.625),
            ]
        );
    }

    #[test]
    fn nested_top_bottom_exact_values() {
        let g = compute(&split(
            100,
            SplitDir::TopBottom,
            0.25,
            split(101, SplitDir::LeftRight, 0.5, slot(1), slot(2)),
            split(
                102,
                SplitDir::LeftRight,
                0.75,
                split(103, SplitDir::TopBottom, 0.5, slot(3), slot(4)),
                slot(5),
            ),
        ));
        assert_eq!(
            g.slots,
            vec![
                rect(1, 0.0, 0.0, 0.5, 0.25),
                rect(2, 0.5, 0.0, 1.0, 0.25),
                rect(3, 0.0, 0.25, 0.75, 0.625),
                rect(4, 0.0, 0.625, 0.75, 1.0),
                rect(5, 0.75, 0.25, 1.0, 1.0),
            ]
        );
        assert_eq!(
            g.splits,
            vec![
                srect(100, SplitDir::TopBottom, 0.0, 0.0, 1.0, 1.0, 0.25),
                srect(101, SplitDir::LeftRight, 0.0, 0.0, 1.0, 0.25, 0.5),
                srect(102, SplitDir::LeftRight, 0.0, 0.25, 1.0, 1.0, 0.75),
                srect(103, SplitDir::TopBottom, 0.0, 0.25, 0.75, 1.0, 0.625),
            ]
        );
    }

    // ── px conversion ───────────────────────────────────────────────────────

    #[test]
    fn px_edge_rounds_half_away_from_zero() {
        assert_eq!(px_edge(100, 0.25), 25);
        assert_eq!(px_edge(1920, 0.0), 0);
        assert_eq!(px_edge(1920, 1.0), 1920);
        assert_eq!(px_edge(0, 0.7), 0);
        // A .5 tie rounds up — with round-half-to-even, 2.5 → 2.
        assert_eq!(px_edge(3, 0.5), 2, "1.5 → 2");
        assert_eq!(px_edge(5, 0.5), 3, "2.5 → 3");
        assert_eq!(px_edge(10, 0.14), 1, "1.4 → 1");
    }

    #[test]
    fn neighbor_frames_share_px_edges_on_odd_canvases() {
        let g = compute(&nested());
        for (w, h) in [(1001, 777), (997, 333), (7, 5)] {
            let f = |n| frame_px(w, h, &slot_of(&g, n));
            let (f1, f2, f3, f4, f5) = (f(1), f(2), f(3), f(4), f(5));
            assert_eq!(f1.x1, f4.x0);
            assert_eq!(f3.x1, f4.x0);
            assert_eq!(f2.x1, f3.x0);
            assert_eq!(f1.y1, f2.y0);
            assert_eq!(f4.y1, f5.y0);
            assert_eq!((f1.x0, f1.y0), (0, 0));
            assert_eq!((f5.x1, f5.y1), (w, h), "폭 합 = 캔버스");
        }
    }

    #[test]
    fn content_is_frame_minus_insets() {
        let frame = PxRect {
            x0: 10,
            y0: 20,
            x1: 110,
            y1: 70,
        };
        let insets = Insets {
            t: 1.0,
            r: 2.0,
            b: 3.0,
            l: 4.5,
        };
        assert_eq!(
            content_rect(&frame, &insets),
            RectF64 {
                x0: 14.5,
                y0: 21.0,
                x1: 108.0,
                y1: 67.0,
            }
        );
    }

    #[test]
    fn zero_and_one_px_frames_have_non_negative_content_and_no_overlap() {
        // 2×1 canvas — a size chosen so the frame widths come out 1·1·0 and the height 1.
        let g = compute(&split(
            100,
            SplitDir::LeftRight,
            0.5,
            slot(1),
            split(101, SplitDir::LeftRight, 0.5, slot(2), slot(3)),
        ));
        let f = |n| frame_px(2, 1, &slot_of(&g, n));
        let (f1, f2, f3) = (f(1), f(2), f(3));
        assert_eq!(f2.x1 - f2.x0, 1, "1px 틀");
        assert_eq!(f3.x1 - f3.x0, 0, "0px 틀");
        assert_eq!(f1.x1, f2.x0);
        assert_eq!(f2.x1, f3.x0);
        for fr in [f1, f2, f3] {
            assert!(fr.x0 <= fr.x1 && fr.y0 <= fr.y1);
            assert_eq!(fr.y1 - fr.y0, 1);
        }

        let border = Insets {
            t: 1.0,
            r: 1.0,
            b: 1.0,
            l: 1.0,
        };
        for fr in [f1, f2, f3] {
            let c = content_rect(&fr, &border);
            assert_eq!(c.x1 - c.x0, 0.0, "폭 0 — 음수 아님");
            assert_eq!(c.y1 - c.y0, 0.0, "높이 0 — 음수 아님");
        }
    }

    // ── ratio limits ────────────────────────────────────────────────────────

    #[test]
    fn out_of_range_ratios_are_clamped() {
        for (ratio, want) in [
            (0.0, RATIO_MIN),
            (-0.5, RATIO_MIN),
            (f64::NEG_INFINITY, RATIO_MIN),
            (1.0, RATIO_MAX),
            (1.5, RATIO_MAX),
            (f64::INFINITY, RATIO_MAX),
        ] {
            let g = compute(&split(100, SplitDir::LeftRight, ratio, slot(1), slot(2)));
            assert_eq!(split_of(&g, 100).at, want, "ratio {ratio}");
            let (a, b) = (slot_of(&g, 1), slot_of(&g, 2));
            assert!(a.x0 < a.x1 && b.x0 < b.x1, "ratio {ratio}: 면적 0 잎 없음");
        }
    }

    #[test]
    fn nan_ratio_reads_as_half() {
        // Root NaN + NaN inside a box whose origin is not 0 — 0.5 is relative to that split box, not to the view.
        let g = compute(&split(
            100,
            SplitDir::TopBottom,
            f64::NAN,
            slot(1),
            split(
                101,
                SplitDir::LeftRight,
                0.25,
                slot(2),
                split(102, SplitDir::LeftRight, f64::NAN, slot(3), slot(4)),
            ),
        ));
        assert_eq!(split_of(&g, 100).at, 0.5);
        let inner = split_of(&g, 102);
        assert_eq!((inner.x0, inner.x1), (0.25, 1.0));
        assert_eq!(inner.at, 0.625, "0.25 + (1 − 0.25) · 0.5");

        let inside = |v: f64| v.is_finite() && (0.0..=1.0).contains(&v);
        for s in &g.slots {
            assert!([s.x0, s.y0, s.x1, s.y1].into_iter().all(inside), "{s:?}");
        }
        for s in &g.splits {
            assert!(
                [s.x0, s.y0, s.x1, s.y1, s.at].into_iter().all(inside),
                "{s:?}"
            );
        }
    }
}
