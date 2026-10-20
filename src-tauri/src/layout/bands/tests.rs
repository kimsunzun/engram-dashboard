use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use super::*;
use crate::layout::geometry::has_area;
use crate::layout::{SlotContent, SplitDir};

const CANVAS: CanvasPx = CanvasPx { w: 1200, h: 800 };

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

pub(super) fn slot(n: u128) -> LayoutNode {
    LayoutNode::Slot {
        id: id(n),
        content: SlotContent::Empty,
    }
}

pub(super) fn split(
    n: u128,
    dir: SplitDir,
    ratio: f64,
    a: LayoutNode,
    b: LayoutNode,
) -> LayoutNode {
    LayoutNode::Split {
        id: id(n),
        dir,
        ratio,
        a: Box::new(a),
        b: Box::new(b),
    }
}

// 반분 사슬 — 늘 b 쪽이 다음 단이다(`layout::dock` 시험과 같은 모양).
pub(super) fn half_chain(dir: SplitDir, depth: u128, base: u128) -> LayoutNode {
    let mut node = slot(base);
    for i in 1..=depth {
        node = split(base + 1000 + i, dir, 0.5, slot(base + i), node);
    }
    node
}

fn px(x0: u32, y0: u32, x1: u32, y1: u32) -> PxRect {
    PxRect { x0, y0, x1, y1 }
}

fn pt(x: f64, y: f64) -> PointCss {
    PointCss { x, y }
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> RectCss {
    RectCss { x0, y0, x1, y1 }
}

fn dip(w: f64, h: f64) -> SizeDip {
    SizeDip { w, h }
}

fn frame_of(frames: &[(Uuid, PxRect)], n: u128) -> PxRect {
    frames
        .iter()
        .find(|(s, _)| *s == id(n))
        .expect("칸 있어야")
        .1
}

// 창 안쪽 탭바(끌기 영역) = (0,0)–(600,28) · 캔버스 원점 = (0,28) → 캔버스 좌표의 탭바 구역 = (0,−43)–(600,15).
const ZONE: RectCss = RectCss {
    x0: 0.0,
    y0: -43.0,
    x1: 600.0,
    y1: 15.0,
};

fn targets<'a>(
    layout: &'a LayoutNode,
    incoming: &'a LayoutNode,
    original: SizeDip,
) -> WindowTargets<'a> {
    WindowTargets {
        canvas: CANVAS,
        layout,
        incoming,
        original,
        zone: Some(ZONE),
    }
}

// 점에서 고른 띠 — (칸 번호 · 바깥이면 0, 변).
fn picked(w: &WindowTargets, p: PointCss) -> Option<(u128, DockSide)> {
    let all = effective_bands(w);
    let near = bands_at(w, p);
    let a = choose(&all, p).map(|i| &all[i]);
    let b = choose(&near, p).map(|i| &near[i]);
    assert_eq!(a, b, "{p:?}: 전부에서 고른 것 = 점 밑 칸만으로 고른 것");
    a.map(|h| match h.anchor {
        DockAnchor::Outer => (0, h.side),
        DockAnchor::Slot(s) => (s.as_u128(), h.side),
    })
}

// ── 탭바 구역 (R2 · C3) ───────────────────────────────────────────────────────

// 끌기 영역 = 첫 탭 왼쪽 4 부터 폭 500(`+` 는 그 오른쪽 · C3) · 탭바 줄 0..28.
const AREA: RectCss = RectCss {
    x0: 4.0,
    y0: 0.0,
    x1: 504.0,
    y1: 28.0,
};

#[test]
fn the_zone_is_the_drag_area_widened_15_up_and_down_only() {
    assert_eq!(strip_zone(AREA), rect(4.0, -15.0, 504.0, 43.0));
}

#[test]
fn the_zone_is_half_open_on_both_axes() {
    let z = strip_zone(AREA);
    for (p, inside, why) in [
        (pt(4.0, 10.0), true, "끌기 영역 왼쪽 변"),
        (pt(3.999, 10.0), false, "첫 탭 왼쪽 앞"),
        (pt(503.999, 10.0), true, "끌기 영역 마지막"),
        (pt(504.0, 10.0), false, "끌기 영역 오른쪽 변 = `+` 자리"),
        (pt(200.0, -15.0), true, "위 15 의 첫 줄"),
        (pt(200.0, -15.001), false, "위 15 밖"),
        (pt(200.0, 42.999), true, "아래 15 의 마지막"),
        (pt(200.0, 43.0), false, "아래 15 = 떼기"),
        (pt(f64::NAN, 10.0), false, "NaN"),
    ] {
        assert_eq!(contains(&z, p), inside, "{why}");
    }
}

#[test]
fn the_zone_in_the_canvas_widens_to_whole_pixels() {
    // 배율 125% 의 탭바 끝 28.125 → 구역 끝 43.125 · 캔버스 원점 28 → 캔버스 15.125 → 16.
    let zone = strip_zone(rect(0.0, 0.0, 600.5, 28.125));
    assert_eq!(
        zone_in_canvas(zone, pt(0.25, 28.0)),
        rect(-1.0, -43.0, 601.0, 16.0)
    );
}

// ── 띠 두께 (R3 · G5 · G19) ───────────────────────────────────────────────────

#[test]
fn the_band_is_ten_percent_rounded_half_away_from_zero_over_a_12_px_floor() {
    for (len, want) in [
        (600, 60),
        (1205, 121), // 120.5
        (125, 13),   // 12.5
        (135, 14),   // 13.5
        (124, 12),
        (115, 12), // 11.5 → 12
        (50, 12),  // AC39 — 5 보다 최소 두께
        (8, 8),    // 칸이 최소 두께보다 좁으면 칸 전체
        (0, 0),
    ] {
        assert_eq!(band_thickness(len), want, "칸 길이 {len}");
    }
}

#[test]
fn the_band_thickness_is_exact_integer_rounding_for_every_length() {
    for len in 0..=20_000u32 {
        let want = ((len + 5) / 10).max(BAND_MIN_PX).min(len);
        assert_eq!(band_thickness(len), want, "칸 길이 {len}");
    }
}

#[test]
fn each_slot_band_hugs_its_own_edge() {
    let frame = px(100, 50, 700, 450);
    let band = |side| slot_band(frame, frame, side);
    assert_eq!(band(DockSide::Left), px(100, 50, 160, 450));
    assert_eq!(band(DockSide::Right), px(640, 50, 700, 450));
    assert_eq!(band(DockSide::Top), px(100, 50, 700, 90));
    assert_eq!(band(DockSide::Bottom), px(100, 410, 700, 450));
    let thin = px(10, 0, 18, 400);
    assert_eq!(slot_band(thin, thin, DockSide::Left), thin);
    assert_eq!(slot_band(thin, thin, DockSide::Right), thin);
}

#[test]
fn a_slot_on_the_canvas_edge_moves_its_band_inside_the_outer_band() {
    // 칸 (0,272)–(882,544) · 캔버스 882 × 544 — 왼쪽 · 오른쪽 · 아래 변이 캔버스 변이다. 두께는 칸 틀 길이로
    // 센다(좌우 88 · 위아래 27).
    let canvas = CanvasPx { w: 882, h: 544 };
    let frame = px(0, 272, 882, 544);
    let all = SIDE_ORDER;
    let room = band_frame(frame, canvas, &all);
    assert_eq!(room, px(20, 272, 862, 524));
    assert_eq!(
        slot_band(frame, room, DockSide::Bottom),
        px(20, 497, 862, 524)
    );
    assert_eq!(
        slot_band(frame, room, DockSide::Left),
        px(20, 272, 108, 524)
    );
    assert_eq!(slot_band(frame, room, DockSide::Top), px(20, 272, 862, 299));
    // 바깥 띠가 막힌 변은 밀지 않는다(AC36) · 캔버스 변에 안 닿은 칸은 그대로.
    assert_eq!(
        band_frame(frame, canvas, &[DockSide::Left, DockSide::Right]),
        px(20, 272, 862, 544)
    );
    assert_eq!(
        band_frame(px(100, 50, 700, 450), CANVAS, &all),
        px(100, 50, 700, 450)
    );
    // 바깥 띠 몫 안에 다 드는 칸 = 빈 띠 틀 · 띠도 빈다.
    let sliver = px(0, 0, 15, 544);
    let room = band_frame(sliver, canvas, &all);
    assert!(is_empty(&room), "{room:?}");
    assert!(is_empty(&slot_band(sliver, room, DockSide::Left)));
}

#[test]
fn outer_strips_are_20_px_and_shrink_to_a_tiny_canvas() {
    assert_eq!(
        outer_strips(CANVAS),
        [
            (DockSide::Left, px(0, 0, 20, 800)),
            (DockSide::Right, px(1180, 0, 1200, 800)),
            (DockSide::Top, px(0, 0, 1200, 20)),
            (DockSide::Bottom, px(0, 780, 1200, 800)),
        ]
    );
    assert_eq!(
        outer_strips(CanvasPx { w: 12, h: 10 }),
        [
            (DockSide::Left, px(0, 0, 12, 10)),
            (DockSide::Right, px(0, 0, 12, 10)),
            (DockSide::Top, px(0, 0, 12, 10)),
            (DockSide::Bottom, px(0, 0, 12, 10)),
        ]
    );
}

// ── 사각형 빼기 ───────────────────────────────────────────────────────────────

#[test]
fn subtracting_keeps_exactly_the_uncovered_pixels() {
    let r = px(10, 10, 30, 30);
    assert_eq!(subtract(r, px(40, 40, 50, 50)), vec![r], "안 겹침");
    assert_eq!(subtract(r, px(30, 10, 40, 30)), vec![r], "변만 맞닿음");
    assert!(subtract(r, px(0, 0, 100, 100)).is_empty(), "다 덮임");
    assert_eq!(
        subtract(r, px(0, 0, 100, 15)),
        vec![px(10, 15, 30, 30)],
        "위 몫"
    );
    assert_eq!(
        subtract(r, px(15, 15, 25, 25)),
        vec![
            px(10, 10, 30, 15),
            px(10, 25, 30, 30),
            px(10, 15, 15, 25),
            px(25, 15, 30, 25),
        ],
        "가운데 구멍"
    );
    assert!(
        subtract(px(5, 5, 5, 9), px(0, 0, 1, 1)).is_empty(),
        "빈 사각형"
    );
    // 조각을 다시 모으면 r − cut 의 픽셀 수와 같다.
    for cut in [px(0, 12, 40, 20), px(20, 0, 25, 100), px(12, 12, 13, 13)] {
        let area: u32 = subtract(r, cut)
            .iter()
            .map(|p| (p.x1 - p.x0) * (p.y1 - p.y0))
            .sum();
        let ix = |a0: u32, a1: u32, b0: u32, b1: u32| a1.min(b1).saturating_sub(a0.max(b0));
        let covered = ix(r.x0, r.x1, cut.x0, cut.x1) * ix(r.y0, r.y1, cut.y0, cut.y1);
        assert_eq!(area, 400 - covered, "{cut:?}");
    }
}

// ── 칸 찾기 ───────────────────────────────────────────────────────────────────

#[test]
fn frame_at_finds_the_slot_under_a_point_and_nothing_outside() {
    let layout = split(100, SplitDir::LeftRight, 0.5, slot(1), slot(2));
    assert_eq!(
        frame_at(&layout, CANVAS, pt(10.0, 10.0)),
        Some((id(1), px(0, 0, 600, 800)))
    );
    assert_eq!(
        frame_at(&layout, CANVAS, pt(600.0, 10.0)),
        Some((id(2), px(600, 0, 1200, 800))),
        "경계는 오른쪽 칸"
    );
    assert_eq!(frame_at(&layout, CANVAS, pt(1200.0, 10.0)), None);
    assert_eq!(frame_at(&layout, CANVAS, pt(10.0, -1.0)), None);
}

#[test]
fn shared_slot_edges_belong_to_the_right_and_lower_slot() {
    // 이웃 칸이 같은 정수 경계를 나눠 점 하나에 칸 하나다.
    let layout = split(
        100,
        SplitDir::LeftRight,
        0.3,
        slot(1),
        split(101, SplitDir::TopBottom, 0.6, slot(2), slot(3)),
    );
    let fr = frames(&layout, CANVAS);
    let (f1, f2, f3) = (frame_of(&fr, 1), frame_of(&fr, 2), frame_of(&fr, 3));
    assert_eq!((f1.x1, f2.y1), (f2.x0, f3.y0));
    let at = |x: f64, y: f64| frame_at(&layout, CANVAS, pt(x, y)).map(|(s, _)| s.as_u128());
    let (bx, by) = (f64::from(f1.x1), f64::from(f2.y1));
    assert_eq!(at(bx, 200.0), Some(2));
    assert_eq!(at(bx - 0.001, 200.0), Some(1));
    assert_eq!(at(800.0, by), Some(3));
    assert_eq!(at(800.0, by - 0.001), Some(2));
}

// ── 고르기 — 좌우 반분 · 탭바 구역 있음 (R6a ①②③) ─────────────────────────────

#[test]
fn picking_on_a_left_right_split_follows_the_band_order_and_half_open_edges() {
    // 칸 1 = [0,600) · 칸 2 = [600,1200) — 좌우 띠 60 · 위아래 띠 80 · 바깥 띠 20 · 탭바 구역 = x<600 · y<15.
    // 캔버스 변의 칸 띠는 바깥 띠 안쪽에 선다 — 칸 1 띠 틀 = (20,20)–(600,780) · 칸 2 = (600,20)–(1180,780).
    let layout = split(100, SplitDir::LeftRight, 0.5, slot(1), slot(2));
    let incoming = slot(9);
    let w = targets(&layout, &incoming, dip(400.0, 300.0));
    use DockSide::{Bottom, Left, Right, Top};
    for (p, want, why) in [
        (pt(10.0, 400.0), Some((0, Left)), "바깥 왼쪽"),
        (pt(1190.0, 400.0), Some((0, Right)), "바깥 오른쪽"),
        (
            pt(900.0, 10.0),
            Some((0, Top)),
            "바깥 위 — 탭바 구역 오른쪽",
        ),
        (pt(600.0, 790.0), Some((0, Bottom)), "바깥 아래"),
        (
            pt(16.0, 784.0),
            Some((0, Left)),
            "바깥 모서리 동률 = 왼쪽 먼저",
        ),
        (
            pt(1190.0, 15.0),
            Some((0, Right)),
            "바깥 모서리 — 가까운 변(오른쪽)",
        ),
        (
            pt(1185.0, 795.0),
            Some((0, Bottom)),
            "바깥 모서리 — 가까운 변(아래)",
        ),
        (pt(19.9, 400.0), Some((0, Left)), "바깥 왼쪽 띠 마지막"),
        (pt(300.0, 14.9), None, "탭바 구역 안 — 띠가 아니다"),
        (
            pt(300.0, 15.0),
            Some((0, Top)),
            "탭바 구역 바로 아래 = 바깥 위 띠(AC44)",
        ),
        (pt(900.0, 19.9), Some((0, Top)), "바깥 위 띠 마지막"),
        (
            pt(900.0, 20.0),
            Some((2, Top)),
            "바깥 위 띠 바로 아래 = 칸 위 띠",
        ),
        (
            pt(20.0, 400.0),
            Some((1, Left)),
            "바깥 왼쪽 띠 바로 오른쪽 = 칸 왼쪽 띠",
        ),
        (
            pt(1179.9, 400.0),
            Some((2, Right)),
            "바깥 오른쪽 띠 바로 앞 = 칸 오른쪽 띠",
        ),
        (
            pt(1180.0, 400.0),
            Some((0, Right)),
            "바깥 오른쪽 띠 첫 픽셀",
        ),
        (pt(900.0, 780.0), Some((0, Bottom)), "바깥 아래 띠 첫 픽셀"),
        (
            pt(900.0, 779.9),
            Some((2, Bottom)),
            "바깥 아래 띠 바로 위 = 칸 아래 띠",
        ),
        (pt(300.0, 400.0), None, "칸 1 가운데"),
        (pt(900.0, 400.0), None, "칸 2 가운데"),
        (pt(600.0, 400.0), Some((2, Left)), "맞닿은 변 = 오른쪽 칸"),
        (
            pt(599.9, 400.0),
            Some((1, Right)),
            "맞닿은 변 바로 앞 = 왼쪽 칸",
        ),
        (pt(79.9, 400.0), Some((1, Left)), "왼쪽 띠 마지막"),
        (pt(80.0, 400.0), None, "왼쪽 띠 오른쪽 변은 밖"),
        (pt(540.0, 400.0), Some((1, Right)), "오른쪽 띠 첫 픽셀"),
        (pt(539.9, 400.0), None, "오른쪽 띠 바로 앞"),
        (pt(300.0, 99.9), Some((1, Top)), "위 띠 마지막"),
        (pt(300.0, 100.0), None, "위 띠 아래 변은 밖"),
        (pt(300.0, 700.0), Some((1, Bottom)), "아래 띠 첫 픽셀"),
        (pt(300.0, 699.9), None, "아래 띠 바로 위"),
        (
            pt(650.0, 60.0),
            Some((2, Top)),
            "칸 모서리 — 정규화 거리(위 0.5 < 왼쪽 0.83)",
        ),
        (
            pt(610.0, 90.0),
            Some((2, Left)),
            "칸 모서리 — 정규화 거리(왼쪽 0.17 < 위 0.88)",
        ),
        (
            pt(630.0, 60.0),
            Some((2, Left)),
            "칸 모서리 동률(30/60 = 40/80) = 왼쪽 먼저",
        ),
        (pt(1200.0, 400.0), None, "캔버스 오른쪽 변 밖"),
        (pt(600.0, 800.0), None, "캔버스 아래 변 밖"),
        (pt(-0.1, 400.0), None, "캔버스 왼쪽 밖"),
    ] {
        assert_eq!(picked(&w, p), want, "{why} {p:?}");
    }
}

#[test]
fn a_point_on_an_area_is_chosen_there_and_only_there() {
    // 영역 = 붙는 곳: 영역마다 그 안 픽셀 가운데를 짚으면 그 띠(모서리는 둘 중 정규화 거리)가 골리고, 영역 밖
    // 점에서는 그 띠가 골리지 않는다.
    let layout = split(
        100,
        SplitDir::LeftRight,
        0.3,
        slot(1),
        split(101, SplitDir::TopBottom, 0.6, slot(2), slot(3)),
    );
    let incoming = slot(9);
    let w = targets(&layout, &incoming, dip(250.0, 250.0));
    let all = effective_bands(&w);
    for y in (0..800).step_by(7) {
        for x in (0..1200).step_by(7) {
            let p = pt(f64::from(x) + 0.5, f64::from(y) + 0.5);
            let inside: Vec<usize> = (0..all.len())
                .filter(|&i| all[i].areas.iter().any(|a| contains_px(a, (p.x, p.y))))
                .collect();
            match choose(&all, p) {
                Some(i) => assert!(inside.contains(&i), "{p:?}"),
                None => assert!(
                    inside.is_empty(),
                    "{p:?}: 띠 영역 {inside:?} 위인데 안 골림"
                ),
            }
        }
    }
}

// ── 칸 구역 선 (사용자 결정 2026-10-19) ───────────────────────────────────────

// 칸 `f` 의 `side` 변 구역 — 칸 변의 두 모서리와 가운데 `c` 의 같은 쪽 두 모서리.
fn side_zone(f: PxRect, c: RectCss, side: DockSide) -> [(f64, f64); 4] {
    let (x0, y0, x1, y1) = (
        f64::from(f.x0),
        f64::from(f.y0),
        f64::from(f.x1),
        f64::from(f.y1),
    );
    match side {
        DockSide::Left => [(x0, y1), (x0, y0), (c.x0, c.y0), (c.x0, c.y1)],
        DockSide::Right => [(x1, y0), (x1, y1), (c.x1, c.y1), (c.x1, c.y0)],
        DockSide::Top => [(x0, y0), (x1, y0), (c.x1, c.y0), (c.x0, c.y0)],
        DockSide::Bottom => [(x1, y1), (x0, y1), (c.x0, c.y1), (c.x1, c.y1)],
    }
}

// 볼록 다각형 — 경계 포함 · 감는 방향 무관.
fn convex_holds(poly: &[(f64, f64)], p: PointCss) -> bool {
    let cross: Vec<f64> = (0..poly.len())
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            (b.0 - a.0) * (p.y - a.1) - (b.1 - a.1) * (p.x - a.0)
        })
        .collect();
    cross.iter().all(|&c| c >= -1e-9) || cross.iter().all(|&c| c <= 1e-9)
}

#[test]
fn the_centre_is_the_slot_less_its_band_thickness() {
    let frame = px(100, 50, 700, 450);
    assert_eq!(slot_centre(frame, frame), rect(160.0, 90.0, 640.0, 410.0));
    // 캔버스 변에 닿은 칸 — 띠 틀(바깥 띠 안쪽)에서 칸 틀 길이로 센 두께(좌우 88 · 위아래 27)만큼.
    let frame = px(0, 272, 882, 544);
    assert_eq!(
        slot_centre(frame, px(20, 272, 862, 524)),
        rect(108.0, 299.0, 774.0, 497.0)
    );
}

#[test]
fn a_narrow_slot_shrinks_its_centre_to_the_middle_line() {
    // 폭 20 → 좌우 띠 12 가 겹친다 — 가운데 = x 20 선분 · 위아래는 같은 비(20 ÷ 24)만큼만 들어온다.
    let frame = px(10, 0, 30, 400);
    let c = slot_centre(frame, frame);
    assert_eq!((c.x0, c.x1), (20.0, 20.0));
    let inset = 40.0 * 20.0 / 24.0;
    assert!((c.y0 - inset).abs() < 1e-9, "{c:?}");
    assert!((c.y1 - (400.0 - inset)).abs() < 1e-9, "{c:?}");
    // 칸이 최소 두께보다 좁으면(띠 = 칸 전체) 칸 가운데에서 만난다.
    let frame = px(10, 0, 18, 400);
    let c = slot_centre(frame, frame);
    assert_eq!((c.x0, c.x1, c.y0, c.y1), (14.0, 14.0, 20.0, 380.0));
}

#[test]
fn the_centre_diagonals_are_where_neighbouring_slot_bands_meet() {
    // 칸 띠가 골린 점은 그 변의 사다리꼴(띠 틀 변 · 대각선 둘 · 가운데 변) 안이고, 어느 띠도 탭바 구역도 아닌 점은
    // 가운데 안이다 — 둘째 배치의 폭 20 칸(가운데가 선분) · 셋째의 캔버스 아래 변 칸(위아래 띠 27) · 넷째의
    // 캔버스 왼쪽 변 칸(폭 30 — 띠 틀 폭 10 · 좌우 띠가 띠 틀 전체) 포함.
    let layouts = [
        split(
            100,
            SplitDir::LeftRight,
            0.3,
            slot(1),
            split(101, SplitDir::TopBottom, 0.6, slot(2), slot(3)),
        ),
        split(
            100,
            SplitDir::LeftRight,
            0.5,
            slot(1),
            split(101, SplitDir::LeftRight, 20.0 / 600.0, slot(2), slot(3)),
        ),
        split(
            100,
            SplitDir::TopBottom,
            0.66,
            split(101, SplitDir::LeftRight, 0.5, slot(1), slot(2)),
            slot(3),
        ),
        split(100, SplitDir::LeftRight, 30.0 / 1200.0, slot(1), slot(2)),
    ];
    let incoming = slot(9);
    for layout in &layouts {
        let w = targets(layout, &incoming, dip(250.0, 250.0));
        let all = effective_bands(&w);
        let outer = outer_sides(&all);
        for y in (0..800).step_by(7) {
            for x in (0..1200).step_by(3) {
                let p = pt(f64::from(x) + 0.5, f64::from(y) + 0.5);
                let (slot_id, frame) = frame_at(layout, CANVAS, p).expect("캔버스 안");
                let room = band_frame(frame, CANVAS, &outer);
                let c = slot_centre(frame, room);
                let in_zone = p.x >= ZONE.x0 && p.x < ZONE.x1 && p.y < ZONE.y1;
                match choose(&all, p).map(|i| &all[i]) {
                    Some(b) if b.anchor == DockAnchor::Outer => {}
                    Some(b) => {
                        assert_eq!(b.anchor, DockAnchor::Slot(slot_id), "{p:?}");
                        assert!(
                            convex_holds(&side_zone(room, c, b.side), p),
                            "{p:?}: {:?} 띠인데 그 구역 밖 — 띠 틀 {room:?} · 가운데 {c:?}",
                            b.side
                        );
                    }
                    None if in_zone => {}
                    None => assert!(
                        c.x0 <= p.x && p.x <= c.x1 && c.y0 <= p.y && p.y <= c.y1,
                        "{p:?}: 어느 띠도 아닌데 가운데 {c:?} 밖"
                    ),
                }
            }
        }
    }
}

#[test]
fn an_edge_slot_keeps_a_reachable_edge_band_inside_the_outer_band() {
    // 캔버스 882 × 544 · 위아래 반분 [좌우 반분 (가 · 나), 다] — 「다」 = (0,272)–(882,544) · 위아래 띠 27. 아래 띠가
    // 칸 틀 변에 붙으면 y 517..544 가 바깥 아래 띠(마지막 20)와 겹친다 — 띠 틀 안쪽(y 497..524)에 서서 남는다.
    let canvas = CanvasPx { w: 882, h: 544 };
    let layout = split(
        100,
        SplitDir::TopBottom,
        0.5,
        split(101, SplitDir::LeftRight, 0.5, slot(1), slot(2)),
        slot(3),
    );
    let incoming = slot(9);
    let w = WindowTargets {
        canvas,
        ..targets(&layout, &incoming, dip(400.0, 300.0))
    };
    assert_eq!(frame_of(&frames(&layout, canvas), 3), px(0, 272, 882, 544));
    let all = effective_bands(&w);
    let bottom = all
        .iter()
        .find(|b| b.anchor == DockAnchor::Slot(id(3)) && b.side == DockSide::Bottom)
        .expect("「다」 아래 띠");
    assert_eq!(bottom.band, px(20, 497, 862, 524));
    assert_eq!(bottom.areas, vec![px(20, 497, 862, 524)]);
    let pick = |x, y| {
        let p = pt(x, y);
        let all = effective_bands(&w);
        let near = bands_at(&w, p);
        let a = choose(&all, p).map(|i| (all[i].anchor, all[i].side));
        assert_eq!(
            a,
            choose(&near, p).map(|i| (near[i].anchor, near[i].side)),
            "{p:?}"
        );
        a
    };
    let slot3 = DockAnchor::Slot(id(3));
    assert_eq!(pick(441.0, 510.0), Some((slot3, DockSide::Bottom)));
    assert_eq!(
        pick(441.0, 497.0),
        Some((slot3, DockSide::Bottom)),
        "아래 띠 첫 줄"
    );
    assert_eq!(
        pick(441.0, 523.9),
        Some((slot3, DockSide::Bottom)),
        "아래 띠 마지막"
    );
    assert_eq!(
        pick(441.0, 524.0),
        Some((DockAnchor::Outer, DockSide::Bottom)),
        "마지막 20 = 바깥 아래 띠"
    );
    assert_eq!(
        pick(441.0, 543.9),
        Some((DockAnchor::Outer, DockSide::Bottom))
    );
    assert_eq!(pick(441.0, 496.9), None, "아래 띠 바로 위");
    assert_eq!(pick(441.0, 408.0), None, "「다」 가운데");
    // 그린 가운데의 아래 변 = 아래 띠의 위 변.
    let room = band_frame(px(0, 272, 882, 544), canvas, &outer_sides(&all));
    assert_eq!(slot_centre(px(0, 272, 882, 544), room).y1, 497.0);
}

#[test]
fn zone_lines_follow_the_slot_under_the_point_and_hide_on_the_outer_band() {
    // 좌우 반분 1200 × 800 — 칸 1 띠 틀 = 바깥 띠(20) 안쪽 (20,20)–(600,780) · 띠 두께 60 × 80.
    let layout = split(100, SplitDir::LeftRight, 0.5, slot(1), slot(2));
    let incoming = slot(9);
    let w = targets(&layout, &incoming, dip(400.0, 300.0));
    let all = effective_bands(&w);
    let zones = |x, y| slot_zones(&w, &all, pt(x, y));
    assert_eq!(
        zones(300.0, 400.0),
        Some(SlotZones {
            slot: px(20, 20, 600, 780),
            centre: rect(80.0, 100.0, 540.0, 700.0),
        }),
        "칸 가운데 — 선은 그리고 고른 띠는 없다"
    );
    assert_eq!(choose(&all, pt(300.0, 400.0)), None);
    assert_eq!(
        zones(900.0, 64.0).map(|z| z.slot),
        Some(px(600, 20, 1180, 780)),
        "칸 2 위 띠 — 그 칸의 선"
    );
    assert_eq!(zones(10.0, 400.0), None, "바깥 띠 — 레이아웃 틀만");
    assert_eq!(zones(1200.0, 400.0), None, "캔버스 밖");
    // 점의 띠만 셈한 목록으로도 같다.
    for (x, y) in [(300.0, 400.0), (900.0, 64.0), (10.0, 400.0)] {
        let p = pt(x, y);
        assert_eq!(slot_zones(&w, &bands_at(&w, p), p), zones(x, y), "{p:?}");
    }
}

#[test]
fn in_a_single_slot_window_the_zone_lines_stand_inside_the_outer_band() {
    // 칸 하나 = 캔버스 전체 — 칸 띠 좌우 120 · 위아래 80 · 띠 틀 (20,20)–(1180,780).
    let layout = slot(1);
    let incoming = slot(9);
    let w = targets(&layout, &incoming, dip(400.0, 300.0));
    let all = effective_bands(&w);
    let zones = |x, y| slot_zones(&w, &all, pt(x, y));
    assert_eq!(zones(10.0, 400.0), None);
    assert_eq!(
        zones(50.0, 400.0).map(|z| z.slot),
        Some(px(20, 20, 1180, 780))
    );
    assert_eq!(
        zones(600.0, 400.0).map(|z| z.centre),
        Some(rect(140.0, 100.0, 1060.0, 700.0))
    );
}

#[test]
fn an_edge_slots_zone_lines_start_from_its_band_frame() {
    let canvas = CanvasPx { w: 882, h: 544 };
    let layout = split(
        100,
        SplitDir::TopBottom,
        0.5,
        split(101, SplitDir::LeftRight, 0.5, slot(1), slot(2)),
        slot(3),
    );
    let incoming = slot(9);
    let w = WindowTargets {
        canvas,
        ..targets(&layout, &incoming, dip(400.0, 300.0))
    };
    let all = effective_bands(&w);
    let i = choose(&all, pt(441.0, 510.0)).expect("칸 3 아래 띠");
    assert_eq!(
        (all[i].anchor, all[i].side, all[i].share),
        (DockAnchor::Slot(id(3)), DockSide::Bottom, 0.5),
        "min(원래 높이 300, 칸 높이 272 ÷ 2) ÷ 272"
    );
    assert_eq!(all[i].landing, px(0, 408, 882, 544), "칸 3 아래 반");
    assert_eq!(
        slot_zones(&w, &all, pt(441.0, 510.0)),
        Some(SlotZones {
            slot: px(20, 272, 862, 524),
            centre: rect(108.0, 299.0, 774.0, 497.0),
        })
    );
    assert_eq!(slot_zones(&w, &all, pt(441.0, 530.0)), None, "바깥 아래 띠");
}

#[test]
fn a_slot_with_no_band_left_draws_no_lines() {
    // 끄는 배치가 대상과 칸 id 를 나눠 `docked_tree` 가 모든 변을 거절한다(R8 의 막힘과 같은 꼴을 결정적으로).
    let layout = slot(1);
    let w = targets(&layout, &layout, dip(400.0, 300.0));
    let all = effective_bands(&w);
    assert!(all.is_empty());
    for (x, y) in [(10.0, 400.0), (50.0, 400.0), (600.0, 400.0)] {
        assert_eq!(slot_zones(&w, &all, pt(x, y)), None, "{x},{y}");
    }
}

// ── 막힌 띠 · 유효 영역 (R8 · R6a · AC36 · AC39) ──────────────────────────────

#[test]
fn a_band_that_would_leave_a_zero_area_slot_is_not_there() {
    // 끼울 탭 = 좌우 반분 사슬 53 단(단독으로는 전부 면적이 있다) — 좌우로 붙이면 1/3 폭에 눌려 깊은 칸이
    // 무너지고 위아래로 붙이면 폭이 그대로라 선다.
    let incoming = half_chain(SplitDir::LeftRight, 53, 30_000);
    assert!(layout_geometry::compute(&incoming)
        .slots
        .iter()
        .all(has_area));
    let layout = slot(1);
    let w = WindowTargets {
        zone: None,
        ..targets(&layout, &incoming, dip(400.0, 300.0))
    };
    let sides: Vec<(DockAnchor, DockSide)> = effective_bands(&w)
        .iter()
        .map(|b| (b.anchor, b.side))
        .collect();
    for anchor in [DockAnchor::Outer, DockAnchor::Slot(id(1))] {
        for side in SIDE_ORDER {
            // 칸 하나 = 캔버스 전체라 바깥 · 칸 띠의 몫이 같다.
            let share = attach_share(w.original, side, px(0, 0, 1200, 800));
            let ok = docked_tree(&layout, anchor, side, &incoming, share).is_ok();
            assert_eq!(sides.contains(&(anchor, side)), ok, "{anchor:?} {side:?}");
        }
    }
    assert!(
        !sides.contains(&(DockAnchor::Slot(id(1)), DockSide::Right)),
        "전제: 오른쪽은 막힘"
    );
    assert!(sides.contains(&(DockAnchor::Slot(id(1)), DockSide::Top)));
    // 칸 오른쪽 띠 자리를 짚어도 고를 것이 없다 — 놓으면 새 창(R8 · AC15).
    assert_eq!(picked(&w, pt(1150.0, 400.0)), None);
}

#[test]
fn a_missing_anchor_slot_has_no_landing() {
    assert_eq!(
        landing_rect(
            &slot(1),
            DockAnchor::Slot(id(77)),
            DockSide::Left,
            &slot(9),
            CANVAS,
            0.25
        ),
        None
    );
}

#[test]
fn a_small_slot_still_gets_a_minimum_band_it_can_dock_to() {
    // AC39 — 폭 50 칸: 좌우 띠 = 최소 두께 12 · 붙는 폭 = min(원래, 25).
    let layout = split(100, SplitDir::LeftRight, 1150.0 / 1200.0, slot(1), slot(2));
    let incoming = slot(9);
    let w = WindowTargets {
        zone: None,
        ..targets(&layout, &incoming, dip(200.0, 300.0))
    };
    assert_eq!(
        frame_of(&frames(&layout, CANVAS), 2),
        px(1150, 0, 1200, 800)
    );
    let all = effective_bands(&w);
    let band = all
        .iter()
        .find(|b| b.anchor == DockAnchor::Slot(id(2)) && b.side == DockSide::Left)
        .expect("작은 칸의 왼쪽 띠");
    assert_eq!(band.band, px(1150, 20, 1162, 780), "위아래는 바깥 띠 안쪽");
    assert_eq!(band.landing.x1 - band.landing.x0, 25);
    assert_eq!(picked(&w, pt(1155.0, 400.0)), Some((2, DockSide::Left)));
}

#[test]
fn a_top_row_slot_gives_way_to_the_zone_then_the_outer_top_band() {
    // AC36 — 맨 위 줄 칸 「가」(칸 1 · 높이 400 → 위 띠 40) 위로 올리며.
    let layout = split(100, SplitDir::TopBottom, 0.5, slot(1), slot(2));
    let incoming = slot(9);
    let w = targets(&layout, &incoming, dip(300.0, 300.0));
    assert_eq!(
        picked(&w, pt(300.0, 10.0)),
        None,
        "탭바 구역(탭바 판정은 이 함수 앞)"
    );
    assert_eq!(
        picked(&w, pt(300.0, 17.0)),
        Some((0, DockSide::Top)),
        "바깥 위 띠의 유효 부분"
    );
    assert_eq!(
        picked(&w, pt(300.0, 20.0)),
        Some((1, DockSide::Top)),
        "「가」 위 띠 = 바깥 위 띠 바로 아래부터"
    );
    assert_eq!(picked(&w, pt(300.0, 59.9)), Some((1, DockSide::Top)));
    assert_eq!(picked(&w, pt(300.0, 60.0)), None);
    let top = effective_bands(&w)
        .into_iter()
        .find(|b| b.anchor == DockAnchor::Slot(id(1)) && b.side == DockSide::Top)
        .unwrap();
    assert_eq!(top.band, px(20, 20, 1180, 60));
    assert_eq!(
        top.areas,
        vec![top.band],
        "띠 틀 안이라 탭바 구역 · 바깥 띠와 겹치지 않는다"
    );
}

#[test]
fn a_low_top_row_slot_keeps_its_top_band_just_inside_the_outer_band() {
    // 「가」 높이 100 → 위 띠 12 — 칸 틀 변에 붙으면 바깥 위 띠(20)에 다 덮이지만 그 아래(y 20..32)에 선다.
    let layout = split(100, SplitDir::TopBottom, 0.125, slot(1), slot(2));
    let incoming = slot(9);
    let w = targets(&layout, &incoming, dip(300.0, 300.0));
    let top = effective_bands(&w)
        .into_iter()
        .find(|b| b.anchor == DockAnchor::Slot(id(1)) && b.side == DockSide::Top)
        .expect("「가」 위 띠");
    assert_eq!(top.areas, vec![px(20, 20, 1180, 32)]);
    assert_eq!(picked(&w, pt(900.0, 19.9)), Some((0, DockSide::Top)));
    assert_eq!(picked(&w, pt(900.0, 20.0)), Some((1, DockSide::Top)));
    assert_eq!(picked(&w, pt(900.0, 31.9)), Some((1, DockSide::Top)));
    assert_eq!(picked(&w, pt(900.0, 32.0)), None);
}

// AC36 마지막의 배치 — 레이아웃 전체 위에 붙이면 아래 반쪽의 깊은 사슬이 무너진다(바깥 위 띠가 막힘). 골든도 쓴다.
pub(super) fn blocked_outer_top_layout(incoming: &LayoutNode, original: SizeDip) -> LayoutNode {
    let share = attach_share(original, DockSide::Top, px(0, 0, 1200, 800));
    (40..120)
        .map(|depth| {
            split(
                100,
                SplitDir::TopBottom,
                0.5,
                slot(1),
                half_chain(SplitDir::TopBottom, depth, 10_000),
            )
        })
        .take_while(|l| layout_geometry::compute(l).slots.iter().all(has_area))
        .find(|l| docked_tree(l, DockAnchor::Outer, DockSide::Top, incoming, share).is_err())
        .expect("바깥 위가 막히는 깊이가 있다")
}

#[test]
fn a_blocked_outer_top_band_does_not_cover_the_slot_band_beneath() {
    // AC36 마지막 — 그래도 「가」 위 띠는 탭바 구역 바로 아래부터 유효하고 골린다.
    let incoming = slot(9);
    let original = dip(300.0, 300.0);
    let layout = blocked_outer_top_layout(&incoming, original);
    let w = targets(&layout, &incoming, original);
    let all = effective_bands(&w);
    assert!(!all
        .iter()
        .any(|b| b.anchor == DockAnchor::Outer && b.side == DockSide::Top));
    let top = all
        .iter()
        .find(|b| b.anchor == DockAnchor::Slot(id(1)) && b.side == DockSide::Top)
        .expect("「가」 위 띠는 막히지 않았다");
    assert_eq!(top.band, px(20, 0, 1180, 40), "막힌 위 변은 밀지 않는다");
    assert!(top.areas.contains(&px(20, 15, 1180, 40)), "{:?}", top.areas);
    assert_eq!(picked(&w, pt(300.0, 20.0)), Some((1, DockSide::Top)));
    assert_eq!(
        picked(&w, pt(300.0, 14.0)),
        None,
        "탭바 구역은 그대로 빠진다"
    );
}

// ── 미리보기 = 놓은 결과 (R4 · R7) ─────────────────────────────────────────────

#[test]
fn the_landing_is_exactly_where_the_incoming_tab_will_be_drawn() {
    let layout = split(
        100,
        SplitDir::LeftRight,
        0.3,
        slot(1),
        split(101, SplitDir::TopBottom, 0.6, slot(2), slot(3)),
    );
    let incoming = split(200, SplitDir::LeftRight, 0.3, slot(21), slot(22));
    for canvas in [
        CANVAS,
        CanvasPx { w: 1001, h: 777 },
        CanvasPx { w: 997, h: 333 },
    ] {
        for original in [dip(100.0, 80.0), dip(400.0, 300.0), dip(5000.0, 5000.0)] {
            let w = WindowTargets {
                canvas,
                zone: None,
                ..targets(&layout, &incoming, original)
            };
            let all = effective_bands(&w);
            assert!(all.len() >= 8, "{canvas:?} {original:?}");
            for b in &all {
                let ctx = format!("{canvas:?} {original:?} {:?} {:?}", b.anchor, b.side);
                let result = docked_tree(&layout, b.anchor, b.side, &incoming, b.share).unwrap();
                let after = frames(&result, canvas);
                let (l, r) = (frame_of(&after, 21), frame_of(&after, 22));
                let drawn = px(
                    l.x0.min(r.x0),
                    l.y0.min(r.y0),
                    l.x1.max(r.x1),
                    l.y1.max(r.y1),
                );
                assert_eq!(b.landing, drawn, "{ctx}");
                // 나누는 방향 길이 ≈ min(원래, 칸 절반)(±1 px — 정규화 경계의 반올림).
                let target = match b.anchor {
                    DockAnchor::Outer => px(0, 0, canvas.w, canvas.h),
                    DockAnchor::Slot(s) => frame_of(&frames(&layout, canvas), s.as_u128()),
                };
                let (got, whole, orig) = match b.side {
                    DockSide::Left | DockSide::Right => (
                        b.landing.x1 - b.landing.x0,
                        target.x1 - target.x0,
                        original.w,
                    ),
                    DockSide::Top | DockSide::Bottom => (
                        b.landing.y1 - b.landing.y0,
                        target.y1 - target.y0,
                        original.h,
                    ),
                };
                let want = orig.min(f64::from(whole) / 2.0);
                assert!(
                    (f64::from(got) - want).abs() <= 1.0,
                    "{ctx}: {got} vs {want}"
                );
            }
        }
    }
}

#[test]
fn ac2_a_400_wide_tab_takes_400_of_a_1200_slot_and_300_of_a_600_slot() {
    let incoming = slot(9);
    for (layout, slot_no, want) in [
        (slot(1), 1, px(800, 0, 1200, 800)),
        (
            split(100, SplitDir::LeftRight, 0.5, slot(1), slot(2)),
            2,
            px(900, 0, 1200, 800),
        ),
    ] {
        let w = WindowTargets {
            zone: None,
            ..targets(&layout, &incoming, dip(400.0, 300.0))
        };
        let right = effective_bands(&w)
            .into_iter()
            .find(|b| b.anchor == DockAnchor::Slot(id(slot_no)) && b.side == DockSide::Right)
            .unwrap();
        assert_eq!(right.landing, want);
    }
}

#[test]
fn ac44_the_outer_top_landing_is_the_original_height_capped_at_half() {
    let layout = split(100, SplitDir::LeftRight, 0.5, slot(1), slot(2));
    let incoming = slot(9);
    for (h, want) in [(300.0, 300), (700.0, 400)] {
        let w = targets(&layout, &incoming, dip(999.0, h));
        let top = effective_bands(&w)
            .into_iter()
            .find(|b| b.anchor == DockAnchor::Outer && b.side == DockSide::Top)
            .unwrap();
        assert_eq!(top.landing, px(0, 0, 1200, want), "원래 높이 {h}");
    }
}

// ── 붙는 몫 (R7 · AC2 · AC39 · AC44) ──────────────────────────────────────────

// 몫 × 칸 길이 = 붙는 길이(px). 반올림은 실제 칸 경계가 하므로 여기서는 셈 값 그대로 본다.
fn attached_len(original: SizeDip, side: DockSide, target: PxRect) -> f64 {
    let t = match side {
        DockSide::Left | DockSide::Right => target.x1 - target.x0,
        DockSide::Top | DockSide::Bottom => target.y1 - target.y0,
    };
    attach_share(original, side, target) * f64::from(t)
}

#[test]
fn the_share_is_the_original_length_capped_at_half_the_target() {
    let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
    for (why, original, side, target, want) in [
        (
            "AC2 — 칸 1200 에 원래 400",
            dip(400.0, 300.0),
            DockSide::Right,
            px(0, 0, 1200, 800),
            400.0,
        ),
        (
            "AC2 — 칸 600 에 원래 400 = 절반",
            dip(400.0, 300.0),
            DockSide::Right,
            px(600, 0, 1200, 800),
            300.0,
        ),
        (
            "AC39 — 600 에 200",
            dip(200.0, 999.0),
            DockSide::Right,
            px(0, 0, 600, 800),
            200.0,
        ),
        (
            "AC39 — 600 에 300",
            dip(300.0, 999.0),
            DockSide::Right,
            px(0, 0, 600, 800),
            300.0,
        ),
        (
            "AC39 — 600 에 400",
            dip(400.0, 999.0),
            DockSide::Left,
            px(0, 0, 600, 800),
            300.0,
        ),
        (
            "AC39 — 작은 칸 50 에 200",
            dip(200.0, 999.0),
            DockSide::Right,
            px(0, 0, 50, 800),
            25.0,
        ),
        (
            "AC39 — 작은 칸 50 에 20",
            dip(20.0, 999.0),
            DockSide::Left,
            px(0, 0, 50, 800),
            20.0,
        ),
        (
            "AC44 — 바깥 위 띠 · 높이 300 · 레이아웃 높이 800",
            dip(999.0, 300.0),
            DockSide::Top,
            px(0, 0, 1200, 800),
            300.0,
        ),
        (
            "AC44 — 레이아웃 높이 500 이면 절반 250",
            dip(999.0, 300.0),
            DockSide::Top,
            px(0, 0, 1200, 500),
            250.0,
        ),
        (
            "아래 = 높이",
            dip(999.0, 120.0),
            DockSide::Bottom,
            px(0, 100, 400, 700),
            120.0,
        ),
    ] {
        assert!(close(attached_len(original, side, target), want), "{why}");
    }
}

#[test]
fn only_the_split_axis_length_counts() {
    let target = px(0, 0, 1000, 600);
    let a = attach_share(dip(200.0, 10.0), DockSide::Left, target);
    let b = attach_share(dip(200.0, 5000.0), DockSide::Left, target);
    assert_eq!(a, b);
    assert_eq!(a, 0.2);
}

#[test]
fn a_tiny_original_is_held_at_the_smallest_share() {
    assert_eq!(
        attach_share(dip(3.0, 3.0), DockSide::Left, px(0, 0, 1000, 600)),
        tree::RATIO_MIN
    );
}

#[test]
fn the_share_is_finite_and_in_range_for_every_input() {
    let lengths = [
        0.0,
        -0.0,
        -5.0,
        1e-300,
        0.4,
        12.0,
        300.0,
        1e9,
        f64::MAX,
        f64::MIN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ];
    let targets = [
        px(0, 0, 0, 0),
        px(10, 10, 10, 10),
        px(0, 0, 1, 1),
        px(0, 0, 2, 3),
        px(5, 7, 605, 407),
        px(0, 0, u32::MAX, u32::MAX),
        px(9, 9, 3, 3), // 뒤집힌 사각형 — 길이 0 으로 읽는다
    ];
    for l in lengths {
        for t in targets {
            for side in SIDE_ORDER {
                let s = attach_share(dip(l, l), side, t);
                assert!(
                    s.is_finite() && (tree::RATIO_MIN..=ATTACH_SHARE_MAX).contains(&s),
                    "L {l} · 칸 {t:?} · {side:?} → {s}"
                );
            }
        }
    }
}

#[test]
fn edge_inputs_go_to_the_documented_ends() {
    let t = px(0, 0, 600, 400);
    assert_eq!(
        attach_share(dip(100.0, 100.0), DockSide::Left, px(0, 0, 0, 400)),
        ATTACH_SHARE_MAX,
        "칸 0"
    );
    assert_eq!(
        attach_share(dip(f64::NAN, 0.0), DockSide::Left, t),
        ATTACH_SHARE_MAX,
        "NaN — 크기를 모른다"
    );
    assert_eq!(
        attach_share(dip(f64::INFINITY, 0.0), DockSide::Left, t),
        ATTACH_SHARE_MAX,
        "+∞"
    );
    assert_eq!(
        attach_share(dip(0.0, 0.0), DockSide::Left, t),
        tree::RATIO_MIN,
        "0"
    );
    assert_eq!(
        attach_share(dip(-1.0, 0.0), DockSide::Left, t),
        tree::RATIO_MIN,
        "음수"
    );
    assert_eq!(
        attach_share(dip(f64::NEG_INFINITY, 0.0), DockSide::Left, t),
        tree::RATIO_MIN,
        "−∞"
    );
}

// ── 상수 내보내기 (TRD §4-5 · §8-1 ⑪) ─────────────────────────────────────────

/// 생성물 `DockConstants.ts` 의 값 묶음 — 키 = Rust 상수 이름 · 차례 = 선언 차례(serde 가 지킨다). 골든 머리
/// `_constants` 도 이 묶음이다.
#[derive(serde::Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) struct DockConstantsValues {
    band_share: f64,
    band_min_px: u32,
    outer_band_px: u32,
    strip_zone_px: f64,
    attach_share_max: f64,
    maximized_inset_dip: f64,
    side_order: [DockSide; 4],
}

pub(super) fn current_constants() -> DockConstantsValues {
    DockConstantsValues {
        band_share: BAND_SHARE,
        band_min_px: BAND_MIN_PX,
        outer_band_px: OUTER_BAND_PX,
        strip_zone_px: STRIP_ZONE_PX,
        attach_share_max: ATTACH_SHARE_MAX,
        maximized_inset_dip: MAXIMIZED_INSET_DIP,
        side_order: SIDE_ORDER,
    }
}

const CONSTANTS_HEAD: &str = "export const DOCK_CONSTANTS = ";
const CONSTANTS_TAIL: &str = " as const;\n";

fn dock_constants_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bindings/DockConstants.ts")
}

fn render_dock_constants() -> String {
    let values = serde_json::to_string_pretty(&current_constants()).expect("직렬화");
    format!(
        "// This file was generated by the test `export_bindings_dock_constants` \
         (src-tauri/src/layout/bands/tests.rs). Do not edit this file manually.\n\
         // The values are the `pub const`s at the head of src-tauri/src/layout/bands.rs.\n\
         {CONSTANTS_HEAD}{values}{CONSTANTS_TAIL}\
         export type DockConstants = typeof DOCK_CONSTANTS;\n"
    )
}

// 이름이 `export_bindings_` 로 시작해야 한다 — ts-rs 내보내기와 같은 무리로 셸 `lib_unit` 이 돌 때마다 다시 쓰고, CI
// 생성물 동기 게이트가 커밋본과의 어긋남을 잡는다. ts-rs `TS` derive 로 내보내지 않는 것은 값(`as const`)이 필요해서다.
#[test]
fn export_bindings_dock_constants() {
    let path = dock_constants_path();
    let text = render_dock_constants();
    assert_eq!(text, render_dock_constants(), "두 번 뜨면 같은 바이트");
    std::fs::write(&path, &text).expect("DockConstants.ts 쓰기");
    let written = std::fs::read_to_string(&path).expect("DockConstants.ts 읽기");
    // 같은 경로를 다른 내보내기(ts-rs 의 `DockConstants` 타입 등)가 덮어쓰면 여기 서명이 깨진다 — 병렬 시험과의 경주가
    // 있어 늘 잡히지는 않는다(진짜 그물 = CI 생성물 동기 게이트).
    assert!(
        written.contains("export const DOCK_CONSTANTS")
            && written.contains("typeof DOCK_CONSTANTS"),
        "내용 서명이 없다 — 다른 생성기가 같은 파일을 쓴다:\n{written}"
    );
    assert_eq!(written, text);
    let json = written
        .split_once(CONSTANTS_HEAD)
        .and_then(|(_, rest)| rest.split_once(CONSTANTS_TAIL))
        .map(|(json, _)| json)
        .expect("DOCK_CONSTANTS 몸");
    let read: serde_json::Value = serde_json::from_str(json).expect("DOCK_CONSTANTS 몸은 JSON");
    assert_eq!(
        read,
        serde_json::to_value(current_constants()).unwrap(),
        "생성물 값 = 지금 pub const"
    );
    assert_eq!(read["OUTER_BAND_PX"], 20, "키 이름 = Rust 상수 이름");
    assert_eq!(read["SIDE_ORDER"][0], "Left");
}

// NTFS 는 대소문자를 가리지 않고 이 저장소는 `core.ignorecase=true` 이며 CI 러너도 Windows 다 — 대소문자만 다른 생성물
// 둘은 서로를 덮어쓴다(TRD §4-5 규칙). 디렉터리를 읽어 견주면 겹친 둘이 이미 한 파일이라 늘 통과하므로 git 색인을 본다.
// `--others` 는 아직 커밋하지 않은 새 생성물도 넣으려는 것이다.
#[test]
fn generated_bindings_never_differ_only_by_case() {
    // 셸 시험의 cwd 는 `src-tauri/` 라 경로는 그 기준(`bindings`)이다 — `src-tauri/bindings` 는 빈 목록이 된다.
    let out = Command::new("git")
        .arg("-C")
        .arg(env!("CARGO_MANIFEST_DIR"))
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "bindings",
        ])
        .output()
        .expect("git 실행");
    assert!(
        out.status.success(),
        "git ls-files 실패: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let listed = String::from_utf8(out.stdout).expect("UTF-8 경로");
    let files: Vec<&str> = listed.split('\0').filter(|f| !f.is_empty()).collect();
    assert!(
        files.contains(&"bindings/DockConstants.ts"),
        "빈 목록 · 엉뚱한 경로의 눈먼 통과를 막는 짝 — 목록 {} 개",
        files.len()
    );
    let mut by_fold: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for f in &files {
        by_fold.entry(f.to_lowercase()).or_default().push(f);
    }
    let clashes: Vec<&Vec<&str>> = by_fold.values().filter(|v| v.len() > 1).collect();
    assert!(clashes.is_empty(), "대소문자만 다른 생성물: {clashes:?}");
}
