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

// Halving chain — b is always the next level (same shape as the `layout::dock` tests).
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

fn dip(w: f64, h: f64) -> SizeCss {
    SizeCss { w, h }
}

fn frame_of(frames: &[(Uuid, PxRect)], n: u128) -> PxRect {
    frames
        .iter()
        .find(|(s, _)| *s == id(n))
        .expect("칸 있어야")
        .1
}

// Tab bar inside the window (drag area) = (0,0)–(600,28) · canvas origin = (0,28) → tab bar zone in canvas coordinates = (0,−43)–(600,15).
const ZONE: RectCss = RectCss {
    x0: 0.0,
    y0: -43.0,
    x1: 600.0,
    y1: 15.0,
};

fn targets<'a>(
    layout: &'a LayoutNode,
    incoming: &'a LayoutNode,
    original: SizeCss,
) -> WindowTargets<'a> {
    WindowTargets {
        canvas: CANVAS,
        layout,
        incoming,
        original,
        zone: Some(ZONE),
    }
}

// Band picked at a point — (area number · 0 if outside, side).
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

// ── tab bar zone (R2 · C3) ────────────────────────────────────────────────────

// Drag area = from 4 left of the first tab, width 500 (`+` is to its right · C3) · tab bar row 0..28.
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
    // Tab bar end 28.125 at scale 125% → zone end 43.125 · canvas origin 28 → canvas 15.125 → 16.
    let zone = strip_zone(rect(0.0, 0.0, 600.5, 28.125));
    assert_eq!(
        zone_in_canvas(zone, pt(0.25, 28.0)),
        rect(-1.0, -43.0, 601.0, 16.0)
    );
}

// ── band thickness (R3 · G5 · G19) ────────────────────────────────────────────

#[test]
fn the_band_is_ten_percent_rounded_half_away_from_zero_over_a_12_px_floor() {
    for (len, want) in [
        (600, 60),
        (1205, 121), // 120.5
        (125, 13),   // 12.5
        (135, 14),   // 13.5
        (124, 12),
        (115, 12), // 11.5 → 12
        (50, 12),  // AC39 — minimum thickness instead of 5
        (8, 8),    // If the area is narrower than the minimum thickness, the whole area
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
    // Area (0,272)–(882,544) · canvas 882 × 544 — the left, right and bottom sides are canvas sides. Thickness is
    // measured by the length of the area frame (left/right 88 · top/bottom 27).
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
    // A side blocked by the outer band is not pushed (AC36) · an area that does not touch the canvas side stays as is.
    assert_eq!(
        band_frame(frame, canvas, &[DockSide::Left, DockSide::Right]),
        px(20, 272, 862, 544)
    );
    assert_eq!(
        band_frame(px(100, 50, 700, 450), CANVAS, &all),
        px(100, 50, 700, 450)
    );
    // An area that fits entirely within the outer band's share = empty band frame · the band is empty too.
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

// ── rectangle subtraction ─────────────────────────────────────────────────────

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
    // Gathering the pieces again gives the same pixel count as r − cut.
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

// ── area lookup ───────────────────────────────────────────────────────────────

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
    // Neighboring areas share the same integer boundary, so one point gives one area.
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

// ── picking — left/right halving · tab bar zone present (R6a ①②③) ─────────────

#[test]
fn picking_on_a_left_right_split_follows_the_band_order_and_half_open_edges() {
    // Area 1 = [0,600) · area 2 = [600,1200) — left/right bands 60 · top/bottom bands 80 · outer band 20 · tab bar zone = x<600 · y<15.
    // An area band on a canvas side stands inside the outer band — area 1 band frame = (20,20)–(600,780) · area 2 = (600,20)–(1180,780).
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
    // Region = where it attaches: pointing at the middle of any pixel in a region picks that band (at a corner, the one with the smaller
    // normalized distance), and a point outside the region does not pick that band.
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

// ── area zone lines (user decision 2026-10-19) ────────────────────────────────

// Zone of the `side` side of area `f` — the two corners of the area's side and the two corners on the same side of the center `c`.
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

// Convex polygon — boundary included · winding direction does not matter.
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
    // Area touching a canvas side — thickness measured from the band frame (inside the outer band) by the area frame length (left/right 88 · top/bottom 27).
    let frame = px(0, 272, 882, 544);
    assert_eq!(
        slot_centre(frame, px(20, 272, 862, 524)),
        rect(108.0, 299.0, 774.0, 497.0)
    );
}

#[test]
fn a_narrow_slot_shrinks_its_centre_to_the_middle_line() {
    // Width 20 → the left and right bands of 12 overlap — the center = the line segment x 20 · top/bottom come in only by the same ratio (20 ÷ 24).
    let frame = px(10, 0, 30, 400);
    let c = slot_centre(frame, frame);
    assert_eq!((c.x0, c.x1), (20.0, 20.0));
    let inset = 40.0 * 20.0 / 24.0;
    assert!((c.y0 - inset).abs() < 1e-9, "{c:?}");
    assert!((c.y1 - (400.0 - inset)).abs() < 1e-9, "{c:?}");
    // If the area is narrower than the minimum thickness (band = whole area), they meet at the area center.
    let frame = px(10, 0, 18, 400);
    let c = slot_centre(frame, frame);
    assert_eq!((c.x0, c.x1, c.y0, c.y1), (14.0, 14.0, 20.0, 380.0));
}

#[test]
fn the_centre_diagonals_are_where_neighbouring_slot_bands_meet() {
    // A point picked by an area band is inside the trapezoid of that side (band frame side · two diagonals · center side), and a point that is
    // in no band and not in the tab bar zone is inside the center — including the width-20 area of the second layout (the center is a segment), the
    // canvas bottom side area of the third (top/bottom bands 27) and the canvas left side area of the fourth (width 30 — band frame width 10 · the left/right bands are the whole band frame).
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
    // Canvas 882 × 544 · top/bottom halving [left/right halving (A · B), C] — 「다」 (C) = (0,272)–(882,544) · top/bottom bands 27. If the bottom band
    // attaches to the area frame side, y 517..544 overlaps the outer bottom band (the last 20) — it stands inside the band frame (y 497..524) and remains.
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
    // Bottom side of the drawn center = top side of the bottom band.
    let room = band_frame(px(0, 272, 882, 544), canvas, &outer_sides(&all));
    assert_eq!(slot_centre(px(0, 272, 882, 544), room).y1, 497.0);
}

#[test]
fn zone_lines_follow_the_slot_under_the_point_and_hide_on_the_outer_band() {
    // Left/right halving 1200 × 800 — area 1 band frame = inside the outer band (20) (20,20)–(600,780) · band thickness 60 × 80.
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
    // The same holds with a list that counts only the point's bands.
    for (x, y) in [(300.0, 400.0), (900.0, 64.0), (10.0, 400.0)] {
        let p = pt(x, y);
        assert_eq!(slot_zones(&w, &bands_at(&w, p), p), zones(x, y), "{p:?}");
    }
}

#[test]
fn in_a_single_slot_window_the_zone_lines_stand_inside_the_outer_band() {
    // One area = the whole canvas — area bands left/right 120 · top/bottom 80 · band frame (20,20)–(1180,780).
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
    // The dragged layout shares the target's area id, so `docked_tree` rejects every side (the same shape as the R8 block, made deterministic).
    let layout = slot(1);
    let w = targets(&layout, &layout, dip(400.0, 300.0));
    let all = effective_bands(&w);
    assert!(all.is_empty());
    for (x, y) in [(10.0, 400.0), (50.0, 400.0), (600.0, 400.0)] {
        assert_eq!(slot_zones(&w, &all, pt(x, y)), None, "{x},{y}");
    }
}

// ── blocked bands · effective region (R8 · R6a · AC36 · AC39) ─────────────────

#[test]
fn a_band_that_would_leave_a_zero_area_slot_is_not_there() {
    // Tab to insert = left/right halving chain of 53 levels (every area has size on its own) — attaching left/right squeezes it into 1/3 width and the deep area
    // collapses, while attaching top/bottom keeps the width, so it stands.
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
            // One area = the whole canvas, so the outer band and the area band have the same share.
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
    // Pointing at the area's right band position picks nothing — dropping opens a new window (R8 · AC15).
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
    // AC39 — width-50 area: left/right bands = minimum thickness 12 · attach width = min(original, 25).
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
    // AC36 — moving up over the top-row area 「가」 (area 1 · height 400 → top band 40).
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
    // 「가」 height 100 → top band 12 — attached to the area frame side it would be fully covered by the outer top band (20), but it stands below that (y 20..32).
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

// Layout for the end of AC36 — attaching above the whole layout collapses the deep chain of the bottom half (the outer top band is blocked). Also uses the golden.
pub(super) fn blocked_outer_top_layout(incoming: &LayoutNode, original: SizeCss) -> LayoutNode {
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
    // End of AC36 — still, the top band of 「가」 is effective from just below the tab bar zone and gets picked.
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

// ── preview = result of the drop (R4 · R7) ────────────────────────────────────

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
                // Length in the splitting direction ≈ min(original, half the area)(±1 px — rounding of the normalized boundary).
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

// ── attach share (R7 · AC2 · AC39 · AC44) ─────────────────────────────────────

// Share × area length = attach length (px). The real area boundary does the rounding, so the computed value is checked as is here.
fn attached_len(original: SizeCss, side: DockSide, target: PxRect) -> f64 {
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
        px(9, 9, 3, 3), // Inverted rectangle — read as length 0
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

// ── constant export (TRD §4-5 · §8-1 ⑪) ───────────────────────────────────────

/// Value bundle of the generated `DockConstants.ts` — key = Rust constant name · order = declaration order (serde keeps it). The golden header
/// `_constants` is this bundle too.
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

// The name must start with `export_bindings_` — in the same group as the ts-rs export, it rewrites the file every time the shell `lib_unit` runs, and the CI
// generated-output sync gate catches any mismatch with the committed copy. It is not exported with the ts-rs `TS` derive because a value (`as const`) is needed.
#[test]
fn export_bindings_dock_constants() {
    let path = dock_constants_path();
    let text = render_dock_constants();
    assert_eq!(text, render_dock_constants(), "두 번 뜨면 같은 바이트");
    std::fs::write(&path, &text).expect("DockConstants.ts 쓰기");
    let written = std::fs::read_to_string(&path).expect("DockConstants.ts 읽기");
    // If another export (such as the ts-rs `DockConstants` type) overwrites the same path, the signature here breaks — there is a race with parallel tests,
    // so it is not always caught (the real safety net = the CI generated-output sync gate).
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

// NTFS is case-insensitive, this repository has `core.ignorecase=true` and the CI runner is Windows too — two generated files that differ only in case
// overwrite each other (TRD §4-5 rule). Reading the directory and comparing would always pass, since the overlapping two are already one file, so we look at the git index.
// `--others` is there to include new generated files that are not committed yet.
#[test]
fn generated_bindings_never_differ_only_by_case() {
    // The shell test's cwd is `src-tauri/`, so the path is relative to that (`bindings`) — `src-tauri/bindings` would become an empty list.
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
