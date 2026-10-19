//! 도킹 띠(TRD S21-slot-docking §5-3) — 창 하나의 캔버스 위 바깥 띠 · 칸 띠 · 그 유효 영역 · 붙을 사각형 · 커서 점의
//! 띠 고르기. 좌표 = 그 창 캔버스의 CSS px(캔버스 왼쪽 위 원점) · 띠 · 영역 · 붙을 사각형은 반열림 정수 사각형.
//!
//! ★칠한 곳 = 붙는 곳(PRD R6a)★ — 안내 창이 칠하는 사각형(`BandHit::areas` · `landing`)과 [`choose`] 가 보는
//! 사각형이 같은 값이다. 붙을 사각형과 놓은 결과는 같은 몫(`BandHit::share`)으로 같은 `docked_tree` 를 지난다(PRD
//! R4 · R7) — 쓰기는 그 `share` 를 그대로 `Seam` 에 싣고 다시 셈하지 않는다.
//!
//! 앞서는 차례(R6a) = 탭바 구역 > 바깥 띠 > 칸 띠. 막힌 띠(R8 — 놓으면 새로 면적 0 칸이 생김)는 없는 것으로 친다 —
//! 칠하지도 고르지도 않고, 막힌 바깥 띠는 밑 칸 띠를 덮지 않는다(AC36).
//!
//! 진입점: [`effective_bands`](창 하나의 유효 띠 전부) · [`bands_at`](바깥 띠 + 점 밑 칸의 띠 — 안내 창이 그리는
//! 것) → [`choose`].

use uuid::Uuid;

use super::sizing::attach_share;
use super::units::SizeDip;
use crate::layout::chrome::{PointCss, RectCss};
use crate::layout::dock::{docked_tree, incoming_rect, DockAnchor, DockSide};
use crate::layout::geometry::{self as layout_geometry, px_edge, PxRect};
use crate::layout::{tree, CanvasPx, LayoutNode};

/// 칸 띠 두께 = 칸 가장자리 10%(ADR-0295 결정 3 · PRD R3 — VS Code `editorDropTarget.ts` 의 0.1 꼴).
// ADR-0295
pub const BAND_SHARE: f64 = 0.10;

/// 칸 띠의 최소 두께(CSS px) — 포인터로 짚을 수 있는 가장 얇은 띠(G5 — 피어 Lumino 뿌리 위 가장자리). 폭 50 칸
/// (AC39)에서도 좌우 띠가 칸 절반을 넘지 않는다.
// ADR-0295
pub const BAND_MIN_PX: u32 = 12;

/// 바깥 띠 두께(CSS px — PRD R5). 탭바 구역이 캔버스 위로 겹치는 폭(15)보다 커서 바깥 위 띠의 유효 부분이 탭바
/// 구역 바로 아래에 남는다(AC44).
pub const OUTER_BAND_PX: u32 = 32;

/// 동률 깨기 순서 — 바깥 띠 · 칸 띠 모서리 둘 다(R6a ③).
pub const SIDE_ORDER: [DockSide; 4] = [
    DockSide::Left,
    DockSide::Right,
    DockSide::Top,
    DockSide::Bottom,
];

/// 띠 하나 — 고른 띠의 판정 · 그림 · 쓰기 재료가 다 여기 있다.
#[derive(Debug, Clone, PartialEq)]
pub struct BandHit {
    pub anchor: DockAnchor,
    pub side: DockSide,
    /// 띠 사각형 전체(앞선 영역을 빼기 전) — `side` 쪽 변이 칸(바깥 띠면 캔버스) 변과 같다. [`choose`] 의 정규화
    /// 거리(변까지 거리 ÷ 두께) 재료.
    pub band: PxRect,
    /// 붙는 몫(`sizing::attach_share`) — `landing` 이 이 값으로 셈했고 놓기의 `Seam.share` 도 이 값이다.
    pub share: f64,
    /// 놓으면 끼운 탭이 차지할 사각형(미리보기 — R4).
    pub landing: PxRect,
    /// 유효 영역 — 띠에서 앞서는 영역을 뺀 사각형 목록(비지 않는다 · 서로 겹치지 않는다). 칠하는 곳이자 짚는 곳.
    pub areas: Vec<PxRect>,
}

/// 창 하나 몫의 입력 — `targets` 가 세계에서 만든다.
pub struct WindowTargets<'a> {
    pub canvas: CanvasPx,
    /// 그 창에 보이는 탭의 배치.
    pub layout: &'a LayoutNode,
    /// 끄는 탭의 배치.
    pub incoming: &'a LayoutNode,
    /// 끄는 창의 원래 크기(§5-4).
    pub original: SizeDip,
    /// 그 창 탭바 구역 — [`zone_in_canvas`] 의 값(캔버스 좌표 · 정수 경계 · 위로 음수일 수 있다). 붙기 판정이 쓴
    /// 경계와 같아야 한다. `None` = 크롬 보고가 없다.
    pub zone: Option<RectCss>,
}

/// 칸 `frame` 의 `side` 띠 — 두께 = `max(round(0.1 × 칸 길이), BAND_MIN_PX)`(좌우 = 폭 · 위아래 = 높이) · 칸 틀
/// 안으로 자른다(두께가 칸보다 크면 칸 전체). `round` 는 .5 를 0 에서 멀리(G19) — 그린 띠와 판정이 같은 정수다.
// ADR-0295
pub fn slot_band(frame: PxRect, side: DockSide) -> PxRect {
    let w = frame.x1.saturating_sub(frame.x0);
    let h = frame.y1.saturating_sub(frame.y0);
    match side {
        DockSide::Left => PxRect {
            x1: frame.x0 + band_thickness(w),
            ..frame
        },
        DockSide::Right => PxRect {
            x0: frame.x1 - band_thickness(w),
            ..frame
        },
        DockSide::Top => PxRect {
            y1: frame.y0 + band_thickness(h),
            ..frame
        },
        DockSide::Bottom => PxRect {
            y0: frame.y1 - band_thickness(h),
            ..frame
        },
    }
}

fn band_thickness(len: u32) -> u32 {
    let ten_percent = (f64::from(len) * BAND_SHARE).round() as u32;
    ten_percent.max(BAND_MIN_PX).min(len)
}

/// 캔버스 네 변의 바깥 띠([`SIDE_ORDER`] 순) — 두께 [`OUTER_BAND_PX`] · 캔버스를 넘지 않는다.
pub fn outer_strips(canvas: CanvasPx) -> [(DockSide, PxRect); 4] {
    let (w, h) = (canvas.w, canvas.h);
    let (tw, th) = (OUTER_BAND_PX.min(w), OUTER_BAND_PX.min(h));
    [
        (
            DockSide::Left,
            PxRect {
                x0: 0,
                y0: 0,
                x1: tw,
                y1: h,
            },
        ),
        (
            DockSide::Right,
            PxRect {
                x0: w - tw,
                y0: 0,
                x1: w,
                y1: h,
            },
        ),
        (
            DockSide::Top,
            PxRect {
                x0: 0,
                y0: 0,
                x1: w,
                y1: th,
            },
        ),
        (
            DockSide::Bottom,
            PxRect {
                x0: 0,
                y0: h - th,
                x1: w,
                y1: h,
            },
        ),
    ]
}

/// 보이는 탭 배치 `layout` 의 칸 틀(캔버스 CSS px · 트리 전위 순) — 이웃 칸이 같은 정수 경계를 나눈다.
pub fn frames(layout: &LayoutNode, canvas: CanvasPx) -> Vec<(Uuid, PxRect)> {
    layout_geometry::compute(layout)
        .slots
        .iter()
        .map(|r| (r.slot_id, layout_geometry::frame_px(canvas.w, canvas.h, r)))
        .collect()
}

/// 대상 트리 `target` 의 `anchor` 의 `side` 에 `incoming` 을 몫 `share` 로 끼웠을 때 끼운 것이 차지할 캔버스 px
/// 사각형 — 미리보기이자 놓은 뒤 화면이 그릴 끼운 칸들의 외접 사각형이다(PRD R4). 결과에 새로 면적 0 칸이 생기거나
/// (R8) 기준 칸이 없으면 `None`. `share` 는 `tree::RATIO_MIN ..= 0.5` 여야 한다(`docked_tree` 계약).
// 정규화 외접 사각형의 네 경계를 각각 `px_edge` 로 옮긴다 — `px_edge` 가 단조라 놓은 뒤 그려질 칸 틀들
// (`frame_px`)의 외접 사각형과 같은 정수가 나온다.
// ADR-0227
pub fn landing_rect(
    target: &LayoutNode,
    anchor: DockAnchor,
    side: DockSide,
    incoming: &LayoutNode,
    canvas: CanvasPx,
    share: f64,
) -> Option<PxRect> {
    let result = docked_tree(target, anchor, side, incoming, share).ok()?;
    let r = incoming_rect(
        &layout_geometry::compute(&result),
        &tree::slot_ids(incoming),
    )?;
    Some(PxRect {
        x0: px_edge(canvas.w, r.x0),
        y0: px_edge(canvas.h, r.y0),
        x1: px_edge(canvas.w, r.x1),
        y1: px_edge(canvas.h, r.y1),
    })
}

/// 사각형 `r` 에서 `cut` 을 뺀 나머지 — 서로 겹치지 않는 사각형 넷까지(위 · 아래 통째 · 그 사이 왼쪽 · 오른쪽).
/// 빈 조각은 싣지 않는다.
pub fn subtract(r: PxRect, cut: PxRect) -> Vec<PxRect> {
    if is_empty(&r) {
        return Vec::new();
    }
    if is_empty(&cut) || !overlaps(&r, &cut) {
        return vec![r];
    }
    let (y0, y1) = (r.y0.max(cut.y0), r.y1.min(cut.y1));
    [
        PxRect { y1: cut.y0, ..r },
        PxRect { y0: cut.y1, ..r },
        PxRect {
            x1: cut.x0,
            y0,
            y1,
            ..r
        },
        PxRect {
            x0: cut.x1,
            y0,
            y1,
            ..r
        },
    ]
    .into_iter()
    .filter(|p| !is_empty(p))
    .collect()
}

fn subtract_all(r: PxRect, cuts: &[PxRect]) -> Vec<PxRect> {
    cuts.iter().fold(vec![r], |pieces, cut| {
        pieces.into_iter().flat_map(|p| subtract(p, *cut)).collect()
    })
}

/// 창 하나의 유효 띠 전부 — 바깥 띠([`SIDE_ORDER`]) 뒤에 칸마다([`frames`] 순) 그 칸의 띠([`SIDE_ORDER`]). 막힌
/// 띠와 앞선 영역에 전부 덮인 띠는 없다.
// ADR-0295
pub fn effective_bands(w: &WindowTargets) -> Vec<BandHit> {
    collect(w, |_| true)
}

/// 바깥 띠 + 점 `p`(캔버스 CSS px)를 품은 칸의 띠 — 안내 창에 그리는 것(§5-3)이자 [`choose`] 가 그 점에서 볼
/// 전부다. 순서 · 값은 [`effective_bands`] 에서 그 칸 밖의 칸 띠를 뺀 것과 같다.
pub fn bands_at(w: &WindowTargets, p: PointCss) -> Vec<BandHit> {
    collect(w, |frame| contains_px(frame, (p.x, p.y)))
}

fn collect(w: &WindowTargets, keep: impl Fn(&PxRect) -> bool) -> Vec<BandHit> {
    let zone = zone_cut(w.zone);
    let whole = PxRect {
        x0: 0,
        y0: 0,
        x1: w.canvas.w,
        y1: w.canvas.h,
    };
    let mut out: Vec<BandHit> = outer_strips(w.canvas)
        .into_iter()
        .filter_map(|(side, strip)| {
            band_hit(w, DockAnchor::Outer, side, strip, whole, zone.as_slice())
        })
        .collect();
    // 칸 띠에서 빼는 것 = 탭바 구역 + 유효 바깥 띠 영역 전부(R6a ① · 막힌 바깥 띠는 이미 빠졌다 — AC36).
    let cuts: Vec<PxRect> = zone
        .into_iter()
        .chain(out.iter().flat_map(|b| b.areas.iter().copied()))
        .collect();
    for (slot, frame) in frames(w.layout, w.canvas) {
        if !keep(&frame) {
            continue;
        }
        for side in SIDE_ORDER {
            let band = slot_band(frame, side);
            out.extend(band_hit(
                w,
                DockAnchor::Slot(slot),
                side,
                band,
                frame,
                &cuts,
            ));
        }
    }
    out
}

fn band_hit(
    w: &WindowTargets,
    anchor: DockAnchor,
    side: DockSide,
    band: PxRect,
    target: PxRect,
    cuts: &[PxRect],
) -> Option<BandHit> {
    let areas = subtract_all(band, cuts);
    if areas.is_empty() {
        return None;
    }
    let share = attach_share(w.original, side, target);
    let landing = landing_rect(w.layout, anchor, side, w.incoming, w.canvas, share)?;
    Some(BandHit {
        anchor,
        side,
        band,
        share,
        landing,
        areas,
    })
}

/// 탭바 구역(그 창 뷰포트 CSS — `strip::strip_zone`)을 캔버스 좌표로 옮기고 정수 경계로 바깥으로 넓힌 것. 붙기 판정 ·
/// 띠에서 빼기 · 칠하기가 모두 이 경계 하나를 쓴다(칠한 곳 = 붙는 곳). 넓힌 1 px 미만은 붙기 쪽이다.
// 소수 경계(배율 125% 의 탭바 등)를 붙기에는 그대로, 띠에는 정수로 쓰면 그 사이 줄이 붙기도 띠도 아니게 된다
// (예 — 구역 끝 43.125 · 커서 43.2 가 바깥 위 띠 위인데 `Nothing`).
// ADR-0295
pub fn zone_in_canvas(zone: RectCss, canvas_origin: PointCss) -> RectCss {
    RectCss {
        x0: (zone.x0 - canvas_origin.x).floor(),
        y0: (zone.y0 - canvas_origin.y).floor(),
        x1: (zone.x1 - canvas_origin.x).ceil(),
        y1: (zone.y1 - canvas_origin.y).ceil(),
    }
}

// 캔버스 밖(음수)을 잘라 정수 사각형으로 — 정수가 아닌 값이 와도 바깥으로 넓혀 구역 안 점을 품은 영역이 남지 않게.
fn zone_cut(zone: Option<RectCss>) -> Option<PxRect> {
    let z = zone?;
    let lo = |v: f64| v.floor().max(0.0) as u32;
    let hi = |v: f64| v.ceil().max(0.0) as u32;
    let r = PxRect {
        x0: lo(z.x0),
        y0: lo(z.y0),
        x1: hi(z.x1),
        y1: hi(z.y1),
    };
    (!is_empty(&r)).then_some(r)
}

/// 점 `p`(캔버스 CSS px)에서 고르는 띠의 자리 — ① 점을 품은 바깥 띠 영역이 있으면 그중 정규화 거리(띠 변까지 거리
/// ÷ 두께)가 가장 작은 것 ② 아니면 점을 품은 칸 띠 영역 중 같은 기준 ③ 같으면 `bands` 의 앞의 것(그래서
/// [`SIDE_ORDER`]) ④ 없으면 `None`(칸 가운데 · 캔버스 밖 — R6). 탭바 구역은 이 함수 앞에서 붙기로 갔다.
// ADR-0295
pub fn choose(bands: &[BandHit], p: PointCss) -> Option<usize> {
    let p = (p.x, p.y);
    let nearest = |outer: bool| {
        let mut best: Option<(usize, f64)> = None;
        for (i, b) in bands.iter().enumerate() {
            if (b.anchor == DockAnchor::Outer) != outer
                || !b.areas.iter().any(|a| contains_px(a, p))
            {
                continue;
            }
            let d = edge_gap(p, &b.band, b.side) / thickness(&b.band, b.side);
            if best.is_none_or(|(_, x)| d < x) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| i)
    };
    nearest(true).or_else(|| nearest(false))
}

fn is_empty(r: &PxRect) -> bool {
    r.x0 >= r.x1 || r.y0 >= r.y1
}

fn overlaps(a: &PxRect, b: &PxRect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

fn contains_px(r: &PxRect, p: (f64, f64)) -> bool {
    (f64::from(r.x0)..f64::from(r.x1)).contains(&p.0)
        && (f64::from(r.y0)..f64::from(r.y1)).contains(&p.1)
}

/// 점에서 `frame` 의 `side` 변까지 — 안쪽이면 0 이상.
fn edge_gap(p: (f64, f64), frame: &PxRect, side: DockSide) -> f64 {
    match side {
        DockSide::Left => p.0 - f64::from(frame.x0),
        DockSide::Right => f64::from(frame.x1) - p.0,
        DockSide::Top => p.1 - f64::from(frame.y0),
        DockSide::Bottom => f64::from(frame.y1) - p.1,
    }
}

// 점을 품은 띠만 부른다 — 그러면 두께가 양수라 나눗셈이 선다.
fn thickness(r: &PxRect, side: DockSide) -> f64 {
    match side {
        DockSide::Left | DockSide::Right => f64::from(r.x1 - r.x0),
        DockSide::Top | DockSide::Bottom => f64::from(r.y1 - r.y0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::geometry::has_area;
    use crate::layout::{SlotContent, SplitDir};

    const CANVAS: CanvasPx = CanvasPx { w: 1200, h: 800 };

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

    // 반분 사슬 — 늘 b 쪽이 다음 단이다(`layout::dock` 시험과 같은 모양).
    fn half_chain(dir: SplitDir, depth: u128, base: u128) -> LayoutNode {
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

    // ── 띠 두께 (R3 · G5 · G19) ───────────────────────────────────────────────

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
        assert_eq!(slot_band(frame, DockSide::Left), px(100, 50, 160, 450));
        assert_eq!(slot_band(frame, DockSide::Right), px(640, 50, 700, 450));
        assert_eq!(slot_band(frame, DockSide::Top), px(100, 50, 700, 90));
        assert_eq!(slot_band(frame, DockSide::Bottom), px(100, 410, 700, 450));
        let thin = px(10, 0, 18, 400);
        assert_eq!(slot_band(thin, DockSide::Left), thin);
        assert_eq!(slot_band(thin, DockSide::Right), thin);
    }

    #[test]
    fn outer_strips_are_32_px_and_shrink_to_a_tiny_canvas() {
        assert_eq!(
            outer_strips(CANVAS),
            [
                (DockSide::Left, px(0, 0, 32, 800)),
                (DockSide::Right, px(1168, 0, 1200, 800)),
                (DockSide::Top, px(0, 0, 1200, 32)),
                (DockSide::Bottom, px(0, 768, 1200, 800)),
            ]
        );
        assert_eq!(
            outer_strips(CanvasPx { w: 20, h: 10 }),
            [
                (DockSide::Left, px(0, 0, 20, 10)),
                (DockSide::Right, px(0, 0, 20, 10)),
                (DockSide::Top, px(0, 0, 20, 10)),
                (DockSide::Bottom, px(0, 0, 20, 10)),
            ]
        );
    }

    // ── 사각형 빼기 ───────────────────────────────────────────────────────────

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

    // ── 고르기 — 좌우 반분 · 탭바 구역 있음 (옛 골든 `split` 의 살아남는 기하 · R6a ①②③) ─────

    #[test]
    fn picking_on_a_left_right_split_follows_the_band_order_and_half_open_edges() {
        // 칸 1 = [0,600) · 칸 2 = [600,1200) — 좌우 띠 60 · 위아래 띠 80 · 바깥 띠 32 · 탭바 구역 = x<600 · y<15.
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
                pt(1190.0, 25.0),
                Some((0, Right)),
                "바깥 모서리 — 가까운 변(오른쪽)",
            ),
            (
                pt(1180.0, 795.0),
                Some((0, Bottom)),
                "바깥 모서리 — 가까운 변(아래)",
            ),
            (
                pt(20.0, 400.0),
                Some((0, Left)),
                "바깥 띠가 밑 칸 띠를 이긴다",
            ),
            (pt(300.0, 14.9), None, "탭바 구역 안 — 띠가 아니다"),
            (
                pt(300.0, 15.0),
                Some((0, Top)),
                "탭바 구역 바로 아래 = 바깥 위 띠(AC44)",
            ),
            (pt(900.0, 31.9), Some((0, Top)), "바깥 위 띠 마지막"),
            (
                pt(900.0, 32.0),
                Some((2, Top)),
                "바깥 위 띠 바로 아래 = 칸 위 띠",
            ),
            (
                pt(32.0, 400.0),
                Some((1, Left)),
                "바깥 왼쪽 띠 바로 오른쪽 = 칸 왼쪽 띠",
            ),
            (
                pt(1167.9, 400.0),
                Some((2, Right)),
                "바깥 오른쪽 띠 바로 앞 = 칸 오른쪽 띠",
            ),
            (
                pt(1168.0, 400.0),
                Some((0, Right)),
                "바깥 오른쪽 띠 첫 픽셀",
            ),
            (pt(900.0, 768.0), Some((0, Bottom)), "바깥 아래 띠 첫 픽셀"),
            (
                pt(900.0, 767.9),
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
            (pt(59.9, 400.0), Some((1, Left)), "왼쪽 띠 마지막"),
            (pt(60.0, 400.0), None, "왼쪽 띠 오른쪽 변은 밖"),
            (pt(540.0, 400.0), Some((1, Right)), "오른쪽 띠 첫 픽셀"),
            (pt(539.9, 400.0), None, "오른쪽 띠 바로 앞"),
            (pt(300.0, 79.9), Some((1, Top)), "위 띠 마지막"),
            (pt(300.0, 80.0), None, "위 띠 아래 변은 밖"),
            (pt(300.0, 720.0), Some((1, Bottom)), "아래 띠 첫 픽셀"),
            (pt(300.0, 719.9), None, "아래 띠 바로 위"),
            (
                pt(650.0, 40.0),
                Some((2, Top)),
                "칸 모서리 — 정규화 거리(위 0.5 < 왼쪽 0.83)",
            ),
            (
                pt(610.0, 70.0),
                Some((2, Left)),
                "칸 모서리 — 정규화 거리(왼쪽 0.17 < 위 0.88)",
            ),
            (
                pt(630.0, 40.0),
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
    fn a_point_on_an_area_is_painted_there_and_only_there() {
        // 칠한 곳 = 붙는 곳: 영역마다 그 안 픽셀 가운데를 짚으면 그 띠(모서리는 둘 중 정규화 거리)가 골리고, 영역 밖
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
                        "{p:?}: 칠한 띠 {inside:?} 위인데 안 골림"
                    ),
                }
            }
        }
    }

    // ── 막힌 띠 · 유효 영역 (R8 · R6a · AC36 · AC39) ──────────────────────────

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
        assert_eq!(band.band, px(1150, 0, 1162, 800));
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
            "탭바 구역(붙기는 이 함수 앞)"
        );
        assert_eq!(
            picked(&w, pt(300.0, 20.0)),
            Some((0, DockSide::Top)),
            "바깥 위 띠의 유효 부분"
        );
        assert_eq!(
            picked(&w, pt(300.0, 35.0)),
            Some((1, DockSide::Top)),
            "「가」 위 띠의 남은 부분"
        );
        assert_eq!(picked(&w, pt(300.0, 45.0)), None);
        let top = effective_bands(&w)
            .into_iter()
            .find(|b| b.anchor == DockAnchor::Slot(id(1)) && b.side == DockSide::Top)
            .unwrap();
        assert_eq!(top.band, px(0, 0, 1200, 40));
        assert_eq!(
            top.areas,
            vec![px(32, 32, 1168, 40)],
            "탭바 구역 · 바깥 위 · 왼쪽 · 오른쪽 띠를 뺀 것"
        );
    }

    #[test]
    fn a_low_top_row_slot_has_its_top_band_fully_covered() {
        // 「가」 높이 100 → 위 띠 12 — 바깥 위 띠(32)에 다 덮여 어디서도 칠해지지 않는다(AC36).
        let layout = split(100, SplitDir::TopBottom, 0.125, slot(1), slot(2));
        let incoming = slot(9);
        let w = targets(&layout, &incoming, dip(300.0, 300.0));
        assert!(!effective_bands(&w)
            .iter()
            .any(|b| b.anchor == DockAnchor::Slot(id(1)) && b.side == DockSide::Top));
        assert_eq!(picked(&w, pt(900.0, 20.0)), Some((0, DockSide::Top)));
    }

    #[test]
    fn a_blocked_outer_top_band_does_not_cover_the_slot_band_beneath() {
        // AC36 마지막 — 레이아웃 전체 위에 붙이면 아래 반쪽의 깊은 사슬이 무너진다(바깥 위 띠가 막힘). 그래도
        // 「가」 위 띠는 탭바 구역 바로 아래부터 칠해지고 골린다.
        let incoming = slot(9);
        let original = dip(300.0, 300.0);
        let share = attach_share(original, DockSide::Top, px(0, 0, 1200, 800));
        let layout = (40..120)
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
            .find(|l| docked_tree(l, DockAnchor::Outer, DockSide::Top, &incoming, share).is_err())
            .expect("바깥 위가 막히는 깊이가 있다");
        let w = targets(&layout, &incoming, original);
        let all = effective_bands(&w);
        assert!(!all
            .iter()
            .any(|b| b.anchor == DockAnchor::Outer && b.side == DockSide::Top));
        let top = all
            .iter()
            .find(|b| b.anchor == DockAnchor::Slot(id(1)) && b.side == DockSide::Top)
            .expect("「가」 위 띠는 막히지 않았다");
        assert!(top.areas.contains(&px(32, 15, 1168, 40)), "{:?}", top.areas);
        assert_eq!(picked(&w, pt(300.0, 20.0)), Some((1, DockSide::Top)));
        assert_eq!(
            picked(&w, pt(300.0, 14.0)),
            None,
            "탭바 구역은 그대로 빠진다"
        );
    }

    // ── 미리보기 = 놓은 결과 (R4 · R7 · §5-4) ─────────────────────────────────

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
                    let result =
                        docked_tree(&layout, b.anchor, b.side, &incoming, b.share).unwrap();
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
}
