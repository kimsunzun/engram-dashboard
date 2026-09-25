//! 셸 기하 — 레이아웃 트리를 뷰 정규화 사각형(경계 꼴)과 정수 CSS px 로 옮기는 순수 계산(ADR-0227).
//! Tauri·락·I/O 무의존이라 단독 headless 테스트가 된다(ADR-0012 격리).
//!
//! 셸과 화면이 함께 쓸 칸·분할 기하다. 진입점 = `compute`(트리 → 칸·분할 사각형) ·
//! `frame_px`/`px_edge`(정규화 → 정수 px) · `content_rect`(틀 − 안쪽 여백).
//!
//! ★불변식 — 맞닿는 경계는 비트 단위로 같다★: 자식은 부모 경계 값을 그대로 복사하고, 새로 계산하는
//! 값은 분할 경계 `at` 하나뿐이다. a 의 끝 경계와 b 의 시작 경계가 같은 `at` 이라 깊이와 무관하게
//! `==` 가 선다. 경계를 `x + w` 로 다시 만들지 말 것 — 부동소수 끝자리에서 갈려 화면에 머리카락 틈이 난다.

use ts_rs::TS;
use uuid::Uuid;

use super::tree::clamp_ratio;
use super::types::{LayoutNode, SplitDir};

/// 한 칸의 사각형 — 뷰 기준 정규화 [0,1] 경계 꼴(`x0 <= x1`, `y0 <= y1`).
/// 이웃 칸과 맞닿는 경계 값은 비트 단위로 같다(`src-tauri/src/layout/geometry.rs` 모듈 헤더 불변식).
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

/// 한 분할 노드 — 이 분할 자신의 상자(두 자식을 합친 영역, 뷰 기준 정규화 [0,1] 경계 꼴) + 경계 좌표 `at`.
/// `at` 은 `dir` 이 `left_right` 면 x 축 값(a = 왼쪽), `top_bottom` 이면 y 축 값(a = 위)이다.
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

/// `compute` 결과. 두 목록 모두 트리 전위 순(노드 → a 서브트리 → b 서브트리)이다 — id 정렬은 소비자 몫.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutGeometry {
    pub slots: Vec<SlotRect>,
    pub splits: Vec<SplitRect>,
}

/// 정수 CSS px 경계 꼴(캔버스 원점 기준). 폭 = `x1 - x0` — 반올림한 두 경계의 차다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PxRect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

/// 칸 틀 안쪽 여백(CSS px, 소수 허용 — 배율에 따라 테두리 폭이 소수로 잴 수 있다).
// `UiMetrics`(웹뷰 보고 wire)에 실려 가므로 serde·ts-rs 를 단다. `PxRect`·`RectF64` 는 셸 내부라 안 단다.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct Insets {
    pub t: f64,
    pub r: f64,
    pub b: f64,
    pub l: f64,
}

/// f64 경계 꼴 사각형(`x0 <= x1`, `y0 <= y1`). 단위는 만든 함수가 정한다 — `content_rect` 는 CSS px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectF64 {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// 루트 칸 = (0,0,1,1). 트리의 비율은 `[RATIO_MIN, RATIO_MAX]` 로 클램프해 읽고, NaN 은 0.5 로 읽는다
/// (±∞ 는 클램프가 한계로 보낸다). 그래서 모든 좌표가 유한하고 [0,1] 안이다.
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
            // ADR-0140: LeftRight = x 축 분할(a 왼쪽) · TopBottom = y 축 분할(a 위).
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

/// 축 구간 `[lo, hi]` 를 비율 `ratio`(a 쪽 몫)로 나누는 경계 좌표 — `compute` 가 모든 분할에 쓰는 바로 그 식.
/// 분할 가드가 새 경계를 미리 잴 때도 이것을 불러 실제 계산과 한 비트도 갈리지 않게 한다.
// ADR-0227
pub fn boundary(lo: f64, hi: f64, ratio: f64) -> f64 {
    // 쓰기 경로가 한계·유한을 보장해도 트리에 직접 심은 값을 여기서도 거른다 — 0.0 등이 면적 0 잎을
    // 만들지 않게 자르고, NaN 은 클램프를 그대로 통과해 자손 좌표 전체로 번지므로 반분으로 읽는다.
    let t = if ratio.is_nan() {
        0.5
    } else {
        clamp_ratio(ratio)
    };
    lo + (hi - lo) * t
}

/// 정규화 경계 `e` 를 캔버스 축 길이 `canvas` 위의 정수 px 경계로 = `round(canvas · e)`.
///
/// 동률은 0 에서 먼 쪽이다 — 입력이 음수가 아니므로 CSS `round(nearest)`(+∞ 쪽)와 같다. 칸 폭은 이 값
/// 둘의 차로 구하고 폭을 따로 반올림하지 않는다 — 그래서 이웃 칸의 px 경계가 언제나 같은 정수다.
/// 범위 밖 `e` 는 `as` 캐스트 포화를 따른다(음수·NaN → 0, `u32` 초과 → `u32::MAX`).
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

/// 틀을 여백만큼 줄인 콘텐츠 영역(CSS px). 폭·높이는 0 아래로 내려가지 않는다(`x1 >= x0`, `y1 >= y0`).
/// 틀이 여백 합보다 좁으면 `x0` 는 `frame.x0 + l` 그대로라 틀 오른쪽 경계 너머에 놓일 수 있다.
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

    // 비율을 이진 표현이 안 되는 값으로 — 끝자리 오차가 실제로 생기는 트리.
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

    // ── 단일 슬롯 · 방향 ───────────────────────────────────────────────────────

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

    // ── 이웃 경계 비트 동일 · 분할 상자 ─────────────────────────────────────────

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

        // 루트 LR 경계: 왼쪽 열의 오른쪽 끝 = 오른쪽 열의 왼쪽 끝.
        assert!(same(s1.x1, s4.x0) && same(s1.x1, s5.x0));
        assert!(same(s3.x1, s4.x0), "깊은 잎의 오른쪽 끝도 같은 값");
        // TB(0.7) 경계: s1 아래 = s2·s3 위.
        assert!(same(s1.y1, s2.y0) && same(s1.y1, s3.y0));
        // LR(0.35) 경계.
        assert!(same(s2.x1, s3.x0));
        // TB(0.45) 경계.
        assert!(same(s4.y1, s5.y0));
        // 부모 끝 경계는 복사된다 — 3단 아래 잎도 뷰 끝(1.0)을 정확히 갖는다.
        assert!(same(s2.y1, 1.0) && same(s3.y1, 1.0) && same(s5.y1, 1.0));
        assert!(same(s1.x0, 0.0) && same(s2.x0, 0.0));

        // 분할 상자 = 두 자식을 합친 영역 · at = 두 자식 사이 경계.
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
        // 얕은 잎(s2·s3)이 깊은 잎(s1·s4) 뒤에 오고 id 도 단조가 아니게 — 너비 우선·id 정렬·역 id 정렬·
        // 후위 순 어느 것으로 회귀해도 두 목록 중 하나 이상이 어긋난다.
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

    // ── 정확값(직사각형 상자 안의 중첩) ─────────────────────────────────────────

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

    // 정사각형이 아닌 상자 안의 분할, 원점이 0 이 아닌 상자 안의 같은 방향 분할(103) 포함 — `at` 을
    // 다른 축의 길이·원점으로 구하거나 원점을 빠뜨리는 실수가 값으로 드러난다. 비율은 이진 표현이
    // 정확한 값이라 `==` 로 단언한다.
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

    // ── px 변환 ─────────────────────────────────────────────────────────────

    #[test]
    fn px_edge_rounds_half_away_from_zero() {
        assert_eq!(px_edge(100, 0.25), 25);
        assert_eq!(px_edge(1920, 0.0), 0);
        assert_eq!(px_edge(1920, 1.0), 1920);
        assert_eq!(px_edge(0, 0.7), 0);
        // .5 동률은 올린다 — 짝수 쪽 반올림이었다면 2.5 → 2.
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
        // 2×1 캔버스 — 틀 폭이 1·1·0, 높이가 1 이 되게 고른 크기.
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

    // ── 비율 한계 ───────────────────────────────────────────────────────────

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
        // 루트 NaN + 원점이 0 이 아닌 상자 안의 NaN — 0.5 는 뷰가 아니라 그 분할 상자 기준이다.
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
