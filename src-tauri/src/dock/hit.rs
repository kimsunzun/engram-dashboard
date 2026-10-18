//! 도킹 대상 판정(hit-test) — 커서 아래가 탭바 · 바깥 띠 · 칸의 어느 띠인가를 값만으로 가른다(TRD
//! S21-slot-docking §2). Tauri · 락 · OS 호출 0 이라 같은 입력이면 같은 답이다(PRD R6a ③). 셸의 놓기 판정
//! (`dock::regions::pick`)이 이 함수들을 부르고, 웹뷰의 가이드(`pickTarget`)는 같은 규칙의 거울이다(골든
//! `dock/pick_golden.json`).
//!
//! 진입점: [`locate`](1 단계 — 어디 위인가) → [`choose_seam`](2 단계 — 놓을 이음새 · 무효 바깥 변은 밑 칸 띠로
//! 넘어감). 띠 재료 = [`valid_bands`] · [`landing_rect`](띠 사각형 = 놓았을 때 끼운 것이 차지할 사각형 — R3 · R4).
//! 그림 재료 = [`outer_strips`](바깥 띠) · [`tab_insert_x`](탭바 끼울 자리).
//! 출처 창 탭바 안 순서 이동 = [`reorder_zone`](구역) · [`reorder_left`](끄는 머리 왼쪽) · [`reorder_index`](놓일
//! 자리) — 셸 `pick` 의 첫 단계가 부르고 그림도 같은 값으로 그린다(TRD §2-5).
//!
//! ★그린 사각형 = 판정 사각형(R6a ③)★ — 칸 · 탭바 · 바깥 띠 · 칸 띠 전부 반열림(왼쪽 · 위 포함, 오른쪽 · 아래
//! 제외)으로 그 사각형에 드는지만 본다. 겹치는 곳(모서리)은 정규화 거리가 작은 쪽, 동률은 언제나 [`SIDE_ORDER`].
//! 탭바 자리는 머리 가운데 x 와 같으면 그 머리 오른쪽. 부동소수 비교만 하고 반올림하지 않는다.

use uuid::Uuid;

use crate::layout::chrome::{DockChrome, RectCss, TabHeader};
use crate::layout::dock::{docked_tree, incoming_rect, DockAnchor, DockSide, DOCK_NEW_SHARE};
use crate::layout::geometry::{self, px_edge, PxRect};
use crate::layout::manager::WindowLabel;
use crate::layout::{tree, CanvasPx, LayoutNode};

/// 바깥 띠 두께(CSS px — PRD R5). Eclipse 4 의 35px 와 같은 크기대이고 탭바 높이(28px)보다 조금 두껍다(TRD §2-3).
pub const OUTER_BAND_PX: u32 = 32;

/// 동률 깨기 순서 — 바깥 띠 · 칸 띠 모서리 둘 다.
pub const SIDE_ORDER: [DockSide; 4] = [
    DockSide::Left,
    DockSide::Right,
    DockSide::Top,
    DockSide::Bottom,
];

/// 판정할 창 하나 — 커서 아래 맨 위 창은 브라우저 엔진이 고른다(끌기 사건을 받은 창이 그 창이다 · TRD §2-2
/// 규칙 1).
pub struct WindowFrame<'a> {
    pub label: &'a str,
    /// 웹뷰 뷰포트의 왼쪽 위 — [`locate`] 의 `cursor` 와 같은 좌표계. 실행 경로(`dock::regions::pick`)는 뷰포트
    /// CSS px 를 그대로 넣으므로 (0, 0) 이다.
    pub inner_origin: (f64, f64),
    /// `cursor` 단위 ÷ CSS px — 실행 경로는 1 이다(이미 CSS px).
    pub scale: f64,
    pub chrome: &'a DockChrome,
    /// 캔버스 크기(CSS px — ADR-0227 보고).
    pub canvas: CanvasPx,
    /// 그 창에 지금 보이는 탭 — 출처 창이면 떼어진 표시를 반영한 탭.
    pub visible_view: Uuid,
    /// 보이는 탭의 칸 틀(`geometry::frame_px` — 캔버스 CSS px).
    pub slots: &'a [(Uuid, PxRect)],
}

/// 1 단계 판정 — 어디 위인가.
#[derive(Debug, Clone, PartialEq)]
pub enum Locate {
    /// 가이드 없음 — 놓으면 새 창(PRD R22).
    Nothing,
    /// 탭바 위(R21). `index` = 끄는 탭을 뺀 탭 목록에서 끼울 자리 · `over` = 커서를 품은 머리(끄는 탭은 아니다 —
    /// R6b 전환 후보).
    TabBar {
        label: WindowLabel,
        index: usize,
        over: Option<Uuid>,
    },
    /// 캔버스 바깥 띠(R5 · R6a ①). `sides` = 커서를 품은 바깥 띠 전부 — 캔버스 변에 가까운 것 먼저(동률은
    /// [`SIDE_ORDER`]) · 비지 않는다. `slot` = 그 밑 칸과 틀(못 찾으면 `None`) · `p` = 캔버스 CSS px 의 커서 —
    /// 품은 변이 전부 무효면 [`choose_seam`] 이 이 칸의 띠로 넘어간다.
    Outer {
        label: WindowLabel,
        view: Uuid,
        sides: Vec<DockSide>,
        slot: Option<(Uuid, PxRect)>,
        p: (f64, f64),
    },
    /// 칸 위 — 어느 띠인지는 [`choose_seam`]. `frame` = 그 칸 틀 · `p` = 캔버스 CSS px 의 커서.
    Slot {
        label: WindowLabel,
        view: Uuid,
        slot: Uuid,
        frame: PxRect,
        p: (f64, f64),
    },
}

impl Locate {
    /// 커서 밑 칸 — 바깥 띠 위여도 그 밑 칸이다. [`choose_seam`] 에 줄 그 칸의 띠를 고르는 데 쓴다.
    pub fn slot(&self) -> Option<Uuid> {
        match self {
            Locate::Outer { slot, .. } => slot.map(|(id, _)| id),
            Locate::Slot { slot, .. } => Some(*slot),
            Locate::Nothing | Locate::TabBar { .. } => None,
        }
    }
}

/// `cursor` 가 `top` 의 어디 위인가. 뷰포트 CSS px = `(cursor − top.inner_origin) / top.scale` — 실행 경로는
/// 뷰포트 CSS px 를 원점 (0, 0) · 배율 1 로 넣는다(TRD §2-2 규칙 2). `source_view` = 끄는 탭.
///
/// 탭바 단계 — 「탭바 안」은 뷰포트 점으로, `index` · `over` 는 그 점의 내용 x(뷰포트 x + `chrome.scroll` · 없음 =
/// 0)로 머리(내용 좌표)와 견준다(TRD §2-2 규칙 3).
pub fn locate(cursor: (f64, f64), top: Option<&WindowFrame>, source_view: Uuid) -> Locate {
    let Some(w) = top else {
        return Locate::Nothing;
    };
    let css = (
        (cursor.0 - w.inner_origin.0) / w.scale,
        (cursor.1 - w.inner_origin.1) / w.scale,
    );
    let chrome = w.chrome;
    if contains_css(&chrome.tab_bar, css) {
        let content = (css.0 + chrome.scroll.unwrap_or(0.0), css.1);
        let heads = || chrome.tabs.iter().filter(move |t| t.view_id != source_view);
        return Locate::TabBar {
            label: w.label.to_owned(),
            index: heads().filter(|t| center_x(&t.rect) <= content.0).count(),
            over: heads()
                .find(|t| contains_css(&t.rect, content))
                .map(|t| t.view_id),
        };
    }

    let p = (
        css.0 - chrome.canvas_origin.x,
        css.1 - chrome.canvas_origin.y,
    );
    let (cw, ch) = (f64::from(w.canvas.w), f64::from(w.canvas.h));
    if !(0.0..cw).contains(&p.0) || !(0.0..ch).contains(&p.1) {
        return Locate::Nothing;
    }
    if w.visible_view == source_view {
        return Locate::Nothing;
    }
    let under = w.slots.iter().find(|(_, f)| contains_px(f, p)).copied();
    // 칸 띠보다 먼저 본다 — 창 변에 붙은 작은 칸 위에서도 바깥 띠가 이긴다(R6a ① · AC4).
    let sides = outer_sides(p, w.canvas);
    if !sides.is_empty() {
        return Locate::Outer {
            label: w.label.to_owned(),
            view: w.visible_view,
            sides,
            slot: under,
            p,
        };
    }
    under.map_or(Locate::Nothing, |(slot, frame)| Locate::Slot {
        label: w.label.to_owned(),
        view: w.visible_view,
        slot,
        frame,
        p,
    })
}

/// 2 단계 — 놓을 이음새 `(기준, 변)`. `outer` = 대상의 유효한 바깥 변(`valid_bands(.., Outer, ..)`) · `bands` = 커서
/// 밑 칸([`Locate::slot`])의 유효한 띠(`valid_bands(.., Slot(그 칸), ..)`). 둘 다 그려진 띠 그대로라 빠진 띠에는
/// 붙지 않는다(R8 · AC15).
///
/// 바깥 띠 위면 품은 변 중 가까운 것부터 유효한 첫 변이다. 품은 변이 전부 빠졌을 때만 밑 칸의 띠로 넘어간다 —
/// 칠하지 않은 바깥 변 자리에서도 그 아래 칠해진 칸 띠는 고를 수 있다(메인 결정 — TRD §2-4). `None` = 이음새
/// 없음(칸 가운데 · 탭바 · 가이드 없음).
pub fn choose_seam(
    loc: &Locate,
    outer: &[(DockSide, PxRect)],
    bands: &[(DockSide, PxRect)],
) -> Option<(DockAnchor, DockSide)> {
    let (slot, frame, p) = match loc {
        Locate::Outer { sides, slot, p, .. } => {
            if let Some(side) = sides.iter().find(|s| outer.iter().any(|(v, _)| v == *s)) {
                return Some((DockAnchor::Outer, *side));
            }
            let (slot, frame) = (*slot)?;
            (slot, frame, *p)
        }
        Locate::Slot { slot, frame, p, .. } => (*slot, *frame, *p),
        Locate::Nothing | Locate::TabBar { .. } => return None,
    };
    choose_band(p, frame, bands).map(|side| (DockAnchor::Slot(slot), side))
}

/// 칸 `frame` 안의 점 `p`(캔버스 CSS px)가 `bands` 중 어느 띠인가. `None` = 가운데(R6 — 놓으면 새 창). 두 띠가
/// 겹치는 모서리는 정규화 거리(칸 변까지 거리 ÷ 띠 두께)가 작은 띠(R6a ②). `frame` 밖으로 삐져나간 띠는 다른 칸
/// (또는 바깥 변)의 띠가 잘못 짝지어진 것이라 보지 않는다.
pub fn choose_band(p: (f64, f64), frame: PxRect, bands: &[(DockSide, PxRect)]) -> Option<DockSide> {
    let mut best: Option<(DockSide, f64)> = None;
    for side in SIDE_ORDER {
        for (_, r) in bands.iter().filter(|(s, _)| *s == side) {
            if !within(r, &frame) || !contains_px(r, p) {
                continue;
            }
            let d = edge_gap(p, &frame, side) / thickness(r, side);
            if best.is_none_or(|(_, b)| d < b) {
                best = Some((side, d));
            }
        }
    }
    best.map(|(side, _)| side)
}

/// 대상 트리 `target` 의 `anchor` 의 `side` 에 `incoming` 을 끼웠을 때 끼운 것이 차지할 캔버스 px 사각형 — 띠의
/// 사각형이자 미리보기 사각형이다(PRD R3 · R4). 결과에 새로 면적 0 칸이 생기거나(R8) 기준 칸이 없으면 `None`.
// 정규화 외접 사각형의 네 경계를 각각 `px_edge` 로 옮긴다 — `px_edge` 가 단조라 놓은 뒤 그려질 칸 틀들
// (`frame_px`)의 외접 사각형과 같은 정수가 나온다.
// ADR-0227
pub fn landing_rect(
    target: &LayoutNode,
    anchor: DockAnchor,
    side: DockSide,
    incoming: &LayoutNode,
    canvas: CanvasPx,
) -> Option<PxRect> {
    let result = docked_tree(target, anchor, side, incoming, DOCK_NEW_SHARE).ok()?;
    let r = incoming_rect(&geometry::compute(&result), &tree::slot_ids(incoming))?;
    Some(PxRect {
        x0: px_edge(canvas.w, r.x0),
        y0: px_edge(canvas.h, r.y0),
        x1: px_edge(canvas.w, r.x1),
        y1: px_edge(canvas.h, r.y1),
    })
}

/// `anchor` 의 네 변 중 붙을 수 있는 변과 그 [`landing_rect`]([`SIDE_ORDER`] 순). 빠진 변은 그리지도 고르지도
/// 않는다(R8 · AC15).
pub fn valid_bands(
    target: &LayoutNode,
    anchor: DockAnchor,
    incoming: &LayoutNode,
    canvas: CanvasPx,
) -> Vec<(DockSide, PxRect)> {
    SIDE_ORDER
        .into_iter()
        .filter_map(|side| landing_rect(target, anchor, side, incoming, canvas).map(|r| (side, r)))
        .collect()
}

/// 캔버스 네 변의 바깥 띠([`SIDE_ORDER`] 순 · 캔버스 CSS px). 그림과 [`locate`] 의 판정이 같은 이 사각형을 쓴다.
/// 두께는 캔버스를 넘지 않는다.
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

/// [`locate`] 의 탭바 자리 `index` 에 그릴 끼울 x — 탭바 상자 왼쪽(`chrome.tab_bar.x0`)에서 잰 CSS px. 끄는 탭을
/// 뺀 머리를 가운데 x 순으로 놓고 그 자리 머리의 왼쪽 변이다. 자리가 끝이면 마지막 머리의 오른쪽 변, 머리가 없으면 0.
pub fn tab_insert_x(chrome: &DockChrome, source_view: Uuid, index: usize) -> f64 {
    let mut heads: Vec<&RectCss> = chrome
        .tabs
        .iter()
        .filter(|t| t.view_id != source_view)
        .map(|t| &t.rect)
        .collect();
    heads.sort_by(|a, b| center_x(a).total_cmp(&center_x(b)));
    let x = match heads.get(index) {
        Some(r) => r.x0,
        None => heads.last().map_or(chrome.tab_bar.x0, |r| r.x1),
    };
    x - chrome.tab_bar.x0
}

/// 순서 이동 구역의 탭바 위아래 여유(CSS px — PRD R23). Chrome 탭 스트립의 떼기 문턱
/// `kVerticalDetachMagnetism = 15` 와 같다(사용자 결정 Q20 「Chrome 처럼 · 약 15 px」 · TRD §2-5).
pub const REORDER_SLACK_PX: f64 = 15.0;

/// 순서 이동 구역 — 탭바 상자 `bar`(뷰포트)의 x 그대로 · y 위아래로 [`REORDER_SLACK_PX`] 넓힌 사각형. 판정은 다른
/// 사각형과 같은 반열림이다. 들어올 때 · 나갈 때 같은 경계다(이력 없음 — PRD D19).
pub fn reorder_zone(bar: RectCss) -> RectCss {
    RectCss {
        x0: bar.x0,
        y0: bar.y0 - REORDER_SLACK_PX,
        x1: bar.x1,
        y1: bar.y1 + REORDER_SLACK_PX,
    }
}

/// 끄는 머리의 왼쪽 x(내용 좌표) — 커서를 따르되(`x − grab_x`) 머리들이 차지한 범위 `[heads[0].x0,
/// heads[n−1].x1)` 안에 머문다. `heads` = 얼린 머리 전부(끄는 머리 포함 · 화면 순서 · 내용 좌표) · `dragged` = 그
/// 안 끄는 머리 자리 · `grab_x` = 잡은 자리(끄는 머리 왼쪽에서 잰 커서) · `x` = 점의 내용 x(뷰포트 x + 탭바 스크롤).
/// 범위가 끄는 머리 폭보다 좁으면 `heads[0].x0` · `heads` 가 비면 `x − grab_x` · `dragged` 가 범위 밖이면 폭 0 으로
/// 친다(영역은 그런 값을 싣지 않는다).
// `f64::min` · `Math.min` 은 NaN · ±0 에서 서로 다른 답을 낼 수 있어 비교로 쓴다 — TS 거울(`reorderLeft`)과 같은 식.
pub fn reorder_left(heads: &[TabHeader], dragged: usize, grab_x: f64, x: f64) -> f64 {
    let follow = x - grab_x;
    let (Some(first), Some(last)) = (heads.first(), heads.last()) else {
        return follow;
    };
    let hi = last.rect.x1 - head_width(heads, dragged);
    let left = if follow < hi { follow } else { hi };
    if left > first.rect.x0 {
        left
    } else {
        first.rect.x0
    }
}

/// 끄는 머리 왼쪽이 `left` 일 때 끄는 탭이 놓일 자리 — 끄는 탭을 뺀 결과 탭 목록의 자리(`DockTo::TabBar` 의
/// `index` 와 같은 뜻 · 제자리 = `dragged`). 왼쪽 이웃은 가운데 ≤ `left` 면 왼쪽에 남고, 오른쪽 이웃은 가운데 <
/// `left + 폭` 이면 왼쪽으로 넘어온다 — 같으면 비키지 않는다(Lumino `TabBar` 와 같은 문턱 · TRD §2-5 ③).
pub fn reorder_index(heads: &[TabHeader], dragged: usize, left: f64) -> usize {
    let right = left + head_width(heads, dragged);
    heads
        .iter()
        .enumerate()
        .filter(|(i, h)| {
            let m = center_x(&h.rect);
            (*i < dragged && m <= left) || (*i > dragged && m < right)
        })
        .count()
}

fn head_width(heads: &[TabHeader], i: usize) -> f64 {
    heads.get(i).map_or(0.0, |h| h.rect.x1 - h.rect.x0)
}

fn center_x(r: &RectCss) -> f64 {
    (r.x0 + r.x1) / 2.0
}

pub(super) fn contains_css(r: &RectCss, p: (f64, f64)) -> bool {
    (r.x0..r.x1).contains(&p.0) && (r.y0..r.y1).contains(&p.1)
}

fn contains_px(r: &PxRect, p: (f64, f64)) -> bool {
    (f64::from(r.x0)..f64::from(r.x1)).contains(&p.0)
        && (f64::from(r.y0)..f64::from(r.y1)).contains(&p.1)
}

fn within(inner: &PxRect, outer: &PxRect) -> bool {
    outer.x0 <= inner.x0 && inner.x1 <= outer.x1 && outer.y0 <= inner.y0 && inner.y1 <= outer.y1
}

fn outer_sides(p: (f64, f64), canvas: CanvasPx) -> Vec<DockSide> {
    let full = PxRect {
        x0: 0,
        y0: 0,
        x1: canvas.w,
        y1: canvas.h,
    };
    let mut hit: Vec<(DockSide, f64)> = outer_strips(canvas)
        .into_iter()
        .filter(|(_, strip)| contains_px(strip, p))
        .map(|(side, _)| (side, edge_gap(p, &full, side)))
        .collect();
    // 안정 정렬이라 동률은 `SIDE_ORDER` 그대로 남는다. `total_cmp` 를 쓰지 않는 것은 -0.0 과 0.0 을 같게 보기 위해서다.
    hit.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    hit.into_iter().map(|(side, _)| side).collect()
}

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
    use crate::layout::chrome::PointCss;
    use crate::layout::{SlotContent, SplitDir};

    const SOURCE: u128 = 99;
    const VISIBLE: u128 = 500;
    const CANVAS: CanvasPx = CanvasPx { w: 1200, h: 800 };
    const BAR_H: f64 = 28.0;

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

    fn frames(layout: &LayoutNode, canvas: CanvasPx) -> Vec<(Uuid, PxRect)> {
        geometry::compute(layout)
            .slots
            .iter()
            .map(|r| (r.slot_id, geometry::frame_px(canvas.w, canvas.h, r)))
            .collect()
    }

    fn frame_of(frames: &[(Uuid, PxRect)], n: u128) -> PxRect {
        frames
            .iter()
            .find(|(s, _)| *s == id(n))
            .expect("칸 있어야")
            .1
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> RectCss {
        RectCss { x0, y0, x1, y1 }
    }

    fn px(x0: u32, y0: u32, x1: u32, y1: u32) -> PxRect {
        PxRect { x0, y0, x1, y1 }
    }

    // 탭바 = 창 안쪽 (0,0)–(600,28) · 캔버스 원점 = (0,28).
    fn chrome(tabs: &[(u128, f64, f64)]) -> DockChrome {
        DockChrome {
            canvas_origin: PointCss { x: 0.0, y: BAR_H },
            tab_bar: rect(0.0, 0.0, 600.0, BAR_H),
            tabs: tabs
                .iter()
                .map(|&(n, x0, x1)| TabHeader {
                    view_id: id(n),
                    rect: rect(x0, 0.0, x1, BAR_H),
                })
                .collect(),
            scroll: None,
        }
    }

    fn window<'a>(
        chrome: &'a DockChrome,
        slots: &'a [(Uuid, PxRect)],
        origin: (f64, f64),
        scale: f64,
    ) -> WindowFrame<'a> {
        WindowFrame {
            label: "main",
            inner_origin: origin,
            scale,
            chrome,
            canvas: CANVAS,
            visible_view: id(VISIBLE),
            slots,
        }
    }

    // 배율 1 · 원점 (0,0) 창에서 캔버스 점 `p` 를 짚는 물리 커서.
    fn at(p: (f64, f64)) -> (f64, f64) {
        (p.0, p.1 + BAR_H)
    }

    fn slot_at(l: Locate) -> Option<u128> {
        match l {
            Locate::Slot { slot, .. } => Some(slot.as_u128()),
            _ => None,
        }
    }

    fn outer_at(l: Locate) -> Option<DockSide> {
        match l {
            Locate::Outer { sides, .. } => sides.first().copied(),
            _ => None,
        }
    }

    // 사각형의 두께 축 경계 둘레 표본 — 시작 변 · 끝 변의 ±0.5 와 그 변 위.
    fn edge_samples(a0: u32, a1: u32) -> [f64; 6] {
        let (a0, a1) = (f64::from(a0), f64::from(a1));
        [a0 - 0.5, a0, a0 + 0.5, a1 - 0.5, a1, a1 + 0.5]
    }

    // `side` 띠의 두께 축 값 `v` 와 가로 축 가운데로 만든 점.
    fn point_on(side: DockSide, r: &PxRect, v: f64) -> (f64, f64) {
        match side {
            DockSide::Left | DockSide::Right => (v, (f64::from(r.y0) + f64::from(r.y1)) / 2.0),
            DockSide::Top | DockSide::Bottom => ((f64::from(r.x0) + f64::from(r.x1)) / 2.0, v),
        }
    }

    fn along(side: DockSide, r: &PxRect) -> (u32, u32) {
        match side {
            DockSide::Left | DockSide::Right => (r.x0, r.x1),
            DockSide::Top | DockSide::Bottom => (r.y0, r.y1),
        }
    }

    // ── 1 단계: 창 · 좌표 · 캔버스 ──────────────────────────────────────────────

    #[test]
    fn no_top_window_is_nothing() {
        assert_eq!(locate((10.0, 10.0), None, id(SOURCE)), Locate::Nothing);
    }

    #[test]
    fn the_cursor_maps_through_the_window_origin_and_its_one_scale() {
        // 좌우 반분 — 칸 경계 = 캔버스 x 600. 셋째 경우는 왼쪽 모니터에서 시작해 물리 x 0 을 넘는 창(W4 · AC12) —
        // 넘은 쪽도 그 창 배율 하나로 읽는다.
        let layout = split(100, SplitDir::LeftRight, 0.5, slot(1), slot(2));
        let fr = frames(&layout, CANVAS);
        let c = chrome(&[]);
        for (origin, scale) in [
            ((0.0, 0.0), 1.0),
            ((100.0, 50.0), 1.5),
            ((-900.0, -40.0), 1.5),
            ((7.0, 3.0), 1.25),
        ] {
            let w = window(&c, &fr, origin, scale);
            let phys = |x: f64, y: f64| (origin.0 + x * scale, origin.1 + (y + BAR_H) * scale);
            let pick = |x: f64| slot_at(locate(phys(x, 400.0), Some(&w), id(SOURCE)));
            assert_eq!(pick(600.0), Some(2), "{origin:?} {scale}: 경계는 오른쪽 칸");
            assert_eq!(pick(599.0), Some(1), "{origin:?} {scale}: 1 CSS px 왼쪽");
            assert_eq!(pick(601.0), Some(2), "{origin:?} {scale}: 1 CSS px 오른쪽");
            match locate(phys(300.0, 400.0), Some(&w), id(SOURCE)) {
                Locate::Slot { p, frame, .. } => {
                    assert_eq!(p, (300.0, 400.0), "{origin:?} {scale}");
                    assert_eq!(frame, frame_of(&fr, 1));
                }
                other => panic!("{other:?}"),
            }
        }
        // 물리 x 0 = 캔버스 x 600 인 창에서 물리 1 px 왼쪽(= 0.67 CSS px)은 왼쪽 칸.
        let w = window(&c, &fr, (-900.0, -40.0), 1.5);
        assert_eq!(slot_at(locate((0.0, 600.0), Some(&w), id(SOURCE))), Some(2));
        assert_eq!(
            slot_at(locate((-1.0, 600.0), Some(&w), id(SOURCE))),
            Some(1)
        );
    }

    #[test]
    fn inner_origin_is_the_client_area_not_the_outer_frame() {
        // 바깥 틀 (100,100) · 클라이언트 (112,145)(테두리 12 · 제목줄 45 물리 px) · 배율 1.5. 판정은 클라이언트
        // 원점을 받아야 맞고, 바깥 원점을 주면 틀 두께 ÷ 배율 = (8, 30) CSS px 만큼 밀린다.
        let (outer, inner, scale) = ((100.0, 100.0), (112.0, 145.0), 1.5);
        let layout = split(100, SplitDir::LeftRight, 0.5, slot(1), slot(2));
        let fr = frames(&layout, CANVAS);
        let c = chrome(&[]);
        let client = (596.0, 400.0 + BAR_H); // 캔버스 (596, 400) — 칸 경계 600 의 4px 왼쪽
        let cursor = (inner.0 + client.0 * scale, inner.1 + client.1 * scale);
        let right = window(&c, &fr, inner, scale);
        assert!(matches!(
            locate(cursor, Some(&right), id(SOURCE)),
            Locate::Slot { slot, p, .. } if slot == id(1) && p == (596.0, 400.0)
        ));
        let wrong = window(&c, &fr, outer, scale);
        assert!(matches!(
            locate(cursor, Some(&wrong), id(SOURCE)),
            Locate::Slot { slot, p, .. } if slot == id(2) && p == (604.0, 430.0)
        ));
    }

    #[test]
    fn the_canvas_origin_is_subtracted_on_both_axes() {
        // 캔버스가 창 왼쪽에서 40px 들어와 있다(옆 막대) — 칸 · 바깥 띠 · 캔버스 밖이 전부 캔버스 원점 기준이다.
        let layout = split(100, SplitDir::LeftRight, 0.5, slot(1), slot(2));
        let fr = frames(&layout, CANVAS);
        let mut c = chrome(&[]);
        c.canvas_origin = PointCss { x: 40.0, y: BAR_H };
        let w = window(&c, &fr, (0.0, 0.0), 1.0);
        let l = |x: f64, y: f64| locate((x, y + BAR_H), Some(&w), id(SOURCE));
        assert_eq!(slot_at(l(640.0, 400.0)), Some(2), "캔버스 x 600");
        assert_eq!(slot_at(l(639.0, 400.0)), Some(1));
        assert_eq!(
            outer_at(l(50.0, 400.0)),
            Some(DockSide::Left),
            "캔버스 x 10"
        );
        assert_eq!(slot_at(l(72.0, 400.0)), Some(1), "캔버스 x 32 — 띠 밖");
        assert_eq!(
            outer_at(l(1239.0, 400.0)),
            Some(DockSide::Right),
            "캔버스 x 1199"
        );
        assert_eq!(l(20.0, 400.0), Locate::Nothing, "캔버스 왼쪽 밖");
        assert_eq!(l(1240.0, 400.0), Locate::Nothing, "캔버스 오른쪽 밖");
        match l(50.0, 400.0) {
            Locate::Outer { p, slot, .. } => {
                assert_eq!(p, (10.0, 400.0));
                assert_eq!(slot, Some((id(1), frame_of(&fr, 1))));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn outside_the_canvas_is_nothing() {
        let layout = slot(1);
        let fr = frames(&layout, CANVAS);
        let mut c = chrome(&[]);
        c.canvas_origin = PointCss { x: 0.0, y: 30.0 }; // 탭바(…28)와 캔버스 사이 2px 틈
        let w = window(&c, &fr, (0.0, 0.0), 1.0);
        for cursor in [
            (100.0, 29.0),     // 틈
            (-0.5, 400.0),     // 왼쪽 밖
            (1200.0, 400.0),   // 오른쪽 변 = 밖(반열림)
            (100.0, 830.0),    // 아래 변 = 밖
            (f64::NAN, 400.0), // 읽을 수 없는 좌표
        ] {
            assert_eq!(
                locate(cursor, Some(&w), id(SOURCE)),
                Locate::Nothing,
                "{cursor:?}"
            );
        }
        assert_eq!(
            outer_at(locate((100.0, 30.0), Some(&w), id(SOURCE))),
            Some(DockSide::Top),
            "캔버스 위 변 = 안"
        );
    }

    #[test]
    fn over_the_dragged_tab_itself_only_the_tab_bar_counts() {
        let layout = slot(1);
        let fr = frames(&layout, CANVAS);
        let c = chrome(&[(10, 0.0, 100.0)]);
        let mut w = window(&c, &fr, (0.0, 0.0), 1.0);
        w.visible_view = id(SOURCE);
        assert_eq!(
            locate(at((600.0, 400.0)), Some(&w), id(SOURCE)),
            Locate::Nothing
        );
        assert_eq!(
            locate(at((5.0, 400.0)), Some(&w), id(SOURCE)),
            Locate::Nothing,
            "바깥 띠도 아님"
        );
        assert!(matches!(
            locate((150.0, 10.0), Some(&w), id(SOURCE)),
            Locate::TabBar { index: 1, .. }
        ));
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
        let c = chrome(&[]);
        let w = window(&c, &fr, (0.0, 0.0), 1.0);
        let (f1, f2, f3) = (frame_of(&fr, 1), frame_of(&fr, 2), frame_of(&fr, 3));
        assert_eq!((f1.x1, f2.y1), (f2.x0, f3.y0));
        let pick = |x: f64, y: f64| slot_at(locate(at((x, y)), Some(&w), id(SOURCE)));
        let (bx, by) = (f64::from(f1.x1), f64::from(f2.y1));
        assert_eq!(pick(bx, 200.0), Some(2));
        assert_eq!(pick(bx - 0.001, 200.0), Some(1));
        assert_eq!(pick(800.0, by), Some(3));
        assert_eq!(pick(800.0, by - 0.001), Some(2));
    }

    // ── 1 단계: 바깥 띠 (R5 · R6a ① · AC4) ──────────────────────────────────────

    #[test]
    fn the_outer_band_is_32_css_px_from_each_canvas_edge() {
        assert_eq!(OUTER_BAND_PX, 32);
        let layout = slot(1);
        let fr = frames(&layout, CANVAS);
        let c = chrome(&[]);
        let w = window(&c, &fr, (0.0, 0.0), 1.0);
        let l = |x: f64, y: f64| locate(at((x, y)), Some(&w), id(SOURCE));
        // 띠 = 반열림 [0, 32) · [1168, 1200) — 그린 사각형과 같다.
        assert_eq!(outer_at(l(0.0, 400.0)), Some(DockSide::Left));
        assert_eq!(outer_at(l(31.999, 400.0)), Some(DockSide::Left));
        assert_eq!(slot_at(l(32.0, 400.0)), Some(1));
        assert_eq!(slot_at(l(1167.999, 400.0)), Some(1));
        assert_eq!(outer_at(l(1168.0, 400.0)), Some(DockSide::Right));
        assert_eq!(outer_at(l(1199.9, 400.0)), Some(DockSide::Right));
        assert_eq!(outer_at(l(600.0, 31.0)), Some(DockSide::Top));
        assert_eq!(slot_at(l(600.0, 32.0)), Some(1));
        assert_eq!(slot_at(l(600.0, 767.999)), Some(1));
        assert_eq!(outer_at(l(600.0, 768.0)), Some(DockSide::Bottom));
        match l(0.0, 400.0) {
            Locate::Outer {
                label, view, slot, ..
            } => {
                assert_eq!((label.as_str(), view), ("main", id(VISIBLE)));
                assert_eq!(slot, Some((id(1), frame_of(&fr, 1))), "밑 칸을 싣는다");
            }
            other => panic!("{other:?}"),
        }
        // 같은 CSS 두께는 배율과 무관하다 — 배율 2 에서도 물리 64px 까지가 띠.
        let w2 = window(&c, &fr, (0.0, 0.0), 2.0);
        let l2 = |x: f64| locate((x, (400.0 + BAR_H) * 2.0), Some(&w2), id(SOURCE));
        assert_eq!(outer_at(l2(63.0)), Some(DockSide::Left));
        assert_eq!(slot_at(l2(64.0)), Some(1));
    }

    #[test]
    fn the_outer_band_beats_a_small_slot_on_the_window_edge() {
        // AC4 — 아래 칸 높이 80px. 그 칸의 아래 띠 자리라도 바깥 32px 안이면 바깥 띠다.
        let layout = split(100, SplitDir::TopBottom, 0.9, slot(1), slot(2));
        let fr = frames(&layout, CANVAS);
        assert_eq!(frame_of(&fr, 2), px(0, 720, 1200, 800));
        let c = chrome(&[]);
        let w = window(&c, &fr, (0.0, 0.0), 1.0);
        assert_eq!(
            outer_at(locate(at((600.0, 790.0)), Some(&w), id(SOURCE))),
            Some(DockSide::Bottom)
        );
        assert_eq!(
            slot_at(locate(at((600.0, 760.0)), Some(&w), id(SOURCE))),
            Some(2)
        );
    }

    #[test]
    fn an_outer_corner_takes_the_nearest_edge_and_ties_go_left_right_top_bottom() {
        let layout = slot(1);
        let fr = frames(&layout, CANVAS);
        let c = chrome(&[]);
        let w = window(&c, &fr, (0.0, 0.0), 1.0);
        let o = |x: f64, y: f64| outer_at(locate(at((x, y)), Some(&w), id(SOURCE)));
        assert_eq!(o(10.0, 5.0), Some(DockSide::Top));
        assert_eq!(o(5.0, 10.0), Some(DockSide::Left));
        assert_eq!(o(10.0, 10.0), Some(DockSide::Left), "왼쪽 = 위 → 왼쪽");
        assert_eq!(
            o(1190.0, 10.0),
            Some(DockSide::Right),
            "오른쪽 = 위 → 오른쪽"
        );
        assert_eq!(o(10.0, 790.0), Some(DockSide::Left), "왼쪽 = 아래 → 왼쪽");
        assert_eq!(
            o(1190.0, 790.0),
            Some(DockSide::Right),
            "오른쪽 = 아래 → 오른쪽"
        );
        // 좁은 캔버스(두 띠가 겹친다) — 가장 가까운 변.
        let narrow = CanvasPx { w: 40, h: 800 };
        let fr = frames(&layout, narrow);
        let mut w = window(&c, &fr, (0.0, 0.0), 1.0);
        w.canvas = narrow;
        let o = |x: f64| outer_at(locate(at((x, 400.0)), Some(&w), id(SOURCE)));
        assert_eq!(o(15.0), Some(DockSide::Left));
        assert_eq!(o(20.0), Some(DockSide::Left), "동률");
        assert_eq!(o(25.0), Some(DockSide::Right));
    }

    #[test]
    fn an_outer_corner_carries_every_strip_it_is_in_nearest_first() {
        let layout = slot(1);
        let fr = frames(&layout, CANVAS);
        let c = chrome(&[]);
        let w = window(&c, &fr, (0.0, 0.0), 1.0);
        let sides = |x: f64, y: f64| match locate(at((x, y)), Some(&w), id(SOURCE)) {
            Locate::Outer { sides, .. } => sides,
            other => panic!("{other:?}"),
        };
        assert_eq!(sides(1190.0, 20.0), vec![DockSide::Right, DockSide::Top]);
        assert_eq!(
            sides(1180.0, 790.0),
            vec![DockSide::Bottom, DockSide::Right]
        );
        assert_eq!(
            sides(10.0, 10.0),
            vec![DockSide::Left, DockSide::Top],
            "동률 = 고정 순서"
        );
        assert_eq!(sides(10.0, 400.0), vec![DockSide::Left]);
    }

    #[test]
    fn outer_strips_are_the_band_thickness_and_shrink_to_a_tiny_canvas() {
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
        assert_eq!(outer_strips(CanvasPx { w: 0, h: 0 })[1].1, px(0, 0, 0, 0));
    }

    // ── 그린 것 = 판정 (R6a ③) ──────────────────────────────────────────────────

    #[test]
    fn what_is_painted_is_exactly_what_is_hit_on_all_four_sides() {
        // 홀수 캔버스 · 비율이 끝자리를 남기는 칸 — 각 띠의 두께 축 양 끝 변 위와 ±0.5 에서 「그린 사각형에 드는가」
        // 와 「그 띠로 판정되는가」가 같다.
        let canvas = CanvasPx { w: 1001, h: 777 };
        let inside = |a0: u32, a1: u32, v: f64| f64::from(a0) <= v && v < f64::from(a1);

        // 바깥 띠 — 다른 띠와 안 겹치는 가로 축 가운데에서.
        let fr = frames(&slot(1), canvas);
        let c = chrome(&[]);
        let mut w = window(&c, &fr, (0.0, 0.0), 1.0);
        w.canvas = canvas;
        for (side, strip) in outer_strips(canvas) {
            let (a0, a1) = along(side, &strip);
            for v in edge_samples(a0, a1) {
                let p = point_on(side, &strip, v);
                if !(0.0..f64::from(canvas.w)).contains(&p.0)
                    || !(0.0..f64::from(canvas.h)).contains(&p.1)
                {
                    continue; // 캔버스 밖은 띠가 아니다(위 시험)
                }
                let hit = outer_at(locate(at(p), Some(&w), id(SOURCE))) == Some(side);
                assert_eq!(hit, inside(a0, a1, v), "바깥 {side:?} @ {p:?}");
            }
        }

        // 칸 띠 — 칸마다 그 칸 안에서.
        let target = split(
            100,
            SplitDir::LeftRight,
            0.3,
            slot(1),
            split(101, SplitDir::TopBottom, 0.6, slot(2), slot(3)),
        );
        let fr = frames(&target, canvas);
        for n in [1, 2, 3] {
            let frame = frame_of(&fr, n);
            let bands = valid_bands(&target, DockAnchor::Slot(id(n)), &slot(9), canvas);
            assert_eq!(bands.len(), 4);
            for &(side, band) in &bands {
                let (a0, a1) = along(side, &band);
                for v in edge_samples(a0, a1) {
                    let p = point_on(side, &band, v);
                    if !contains_px(&frame, p) {
                        continue; // 칸 밖은 그 칸의 판정이 아니다
                    }
                    let hit = choose_band(p, frame, &bands) == Some(side);
                    assert_eq!(hit, inside(a0, a1, v), "칸 {n} {side:?} @ {p:?}");
                }
            }
        }
    }

    // ── 1 단계: 탭바 (R21 · R6b) ─────────────────────────────────────────────────

    // 크롬이 아직 끄는 탭(99)의 머리를 싣고 있다 — A(10) · 99 · B(11) · C(12), 각 폭 100.
    fn stale_tabs() -> DockChrome {
        chrome(&[
            (10, 0.0, 100.0),
            (SOURCE, 100.0, 200.0),
            (11, 200.0, 300.0),
            (12, 300.0, 400.0),
        ])
    }

    fn tab_bar_at(c: &DockChrome, x: f64) -> (usize, Option<u128>) {
        let fr = frames(&slot(1), CANVAS);
        let w = window(c, &fr, (0.0, 0.0), 1.0);
        match locate((x, 10.0), Some(&w), id(SOURCE)) {
            Locate::TabBar { label, index, over } => {
                assert_eq!(label, "main");
                (index, over.map(|v| v.as_u128()))
            }
            other => panic!("{x}: {other:?}"),
        }
    }

    #[test]
    fn the_tab_bar_index_counts_header_centers_and_a_center_tie_goes_right() {
        let c = stale_tabs();
        assert_eq!(tab_bar_at(&c, 0.0), (0, Some(10)));
        assert_eq!(tab_bar_at(&c, 49.9), (0, Some(10)));
        assert_eq!(
            tab_bar_at(&c, 50.0),
            (1, Some(10)),
            "가운데와 같으면 오른쪽"
        );
        assert_eq!(tab_bar_at(&c, 250.0), (2, Some(11)));
        assert_eq!(tab_bar_at(&c, 399.0), (3, Some(12)));
        assert_eq!(tab_bar_at(&c, 599.0), (3, None), "머리 뒤 빈 탭바 = 맨 뒤");
    }

    #[test]
    fn the_dragged_tabs_header_is_dropped_by_id_from_index_and_over() {
        // 같은 기하에서 그 머리만 뺀 크롬과 답이 같다 — 거르지 않으면 B 위의 자리가 3 이 된다.
        let stale = stale_tabs();
        let without = chrome(&[(10, 0.0, 100.0), (11, 200.0, 300.0), (12, 300.0, 400.0)]);
        for x in [
            0.0, 60.0, 120.0, 150.0, 199.0, 200.0, 250.0, 260.0, 350.0, 500.0,
        ] {
            assert_eq!(tab_bar_at(&stale, x), tab_bar_at(&without, x), "{x}");
        }
        assert_eq!(
            tab_bar_at(&stale, 150.0),
            (1, None),
            "끄는 탭 머리는 over 가 아니다"
        );
        assert_eq!(tab_bar_at(&stale, 260.0).0, 2);
    }

    #[test]
    fn until_the_next_report_a_stale_bar_still_has_the_old_geometry() {
        // 떼어진 표시가 그 머리를 숨기면 웹뷰 탭바는 왼쪽으로 당겨지지만 다음 보고 전까지 셸의 크롬은 옛 자리다 —
        // 같은 커서가 다른 자리를 낸다. 그 틈에도 옛 탭바를 비우지 않는다: 웹뷰는 같은 영역으로 그리고 셸도 같은
        // 영역으로 판정하므로 놓기는 보인 자리에 들어가고, 접은 뒤의 보고가 `rev` 를 올려 맞춘다(TRD §2-4). 이 함수는
        // 받은 크롬만 믿는다.
        let stale = stale_tabs();
        let reflowed = chrome(&[(10, 0.0, 100.0), (11, 100.0, 200.0), (12, 200.0, 300.0)]);
        assert_eq!(tab_bar_at(&stale, 150.0), (1, None));
        assert_eq!(tab_bar_at(&reflowed, 150.0), (2, Some(11)));
    }

    #[test]
    fn the_tab_bar_is_half_open_and_wins_over_the_canvas() {
        let c = stale_tabs();
        let fr = frames(&slot(1), CANVAS);
        let w = window(&c, &fr, (0.0, 0.0), 1.0);
        assert!(matches!(
            locate((50.0, 0.0), Some(&w), id(SOURCE)),
            Locate::TabBar { .. }
        ));
        assert!(matches!(
            locate((50.0, 27.9), Some(&w), id(SOURCE)),
            Locate::TabBar { .. }
        ));
        // 아래 변(28)은 탭바 밖 = 캔버스 첫 줄(바깥 위 띠).
        assert_eq!(
            outer_at(locate((50.0, 28.0), Some(&w), id(SOURCE))),
            Some(DockSide::Top)
        );
        // 오른쪽 변(600)은 탭바 밖이고 캔버스 위도 아니다.
        assert_eq!(locate((600.0, 10.0), Some(&w), id(SOURCE)), Locate::Nothing);
        // 탭바 위 판정도 배율을 지난다.
        let w2 = window(&c, &fr, (1000.0, 500.0), 1.5);
        assert!(matches!(
            locate((1000.0 + 250.0 * 1.5, 500.0 + 10.0 * 1.5), Some(&w2), id(SOURCE)),
            Locate::TabBar { index: 2, over: Some(o), .. } if o == id(11)
        ));
    }

    #[test]
    fn tab_insert_x_marks_the_left_edge_of_the_header_at_that_index() {
        let mut c = stale_tabs();
        c.tab_bar.x0 = 8.0;
        for (index, want) in [(0, 0.0), (1, 200.0), (2, 300.0), (3, 400.0), (9, 400.0)] {
            assert_eq!(tab_insert_x(&c, id(SOURCE), index), want - 8.0, "{index}");
        }
        // 보고 순서가 화면 순서와 달라도 가운데 x 로 놓는다.
        c.tabs.reverse();
        assert_eq!(tab_insert_x(&c, id(SOURCE), 1), 192.0);
        let only_source = chrome(&[(SOURCE, 0.0, 100.0)]);
        assert_eq!(tab_insert_x(&only_source, id(SOURCE), 0), 0.0);
        // locate 의 자리와 짝이 맞는다 — 커서 왼쪽 머리들의 뒤.
        let c = stale_tabs();
        let (index, _) = tab_bar_at(&c, 260.0);
        assert_eq!(tab_insert_x(&c, id(SOURCE), index), 300.0);
    }

    #[test]
    fn the_tab_bar_compares_heads_at_the_points_content_x() {
        // 머리는 내용 좌표 · 탭바가 150 밀려 있다 — 뷰포트 50 = 내용 200(B 의 왼쪽 변).
        let mut c = stale_tabs();
        c.scroll = Some(150.0);
        assert_eq!(tab_bar_at(&c, 50.0), (1, Some(11)));
        assert_eq!(tab_bar_at(&c, 99.9), (1, Some(11)));
        assert_eq!(
            tab_bar_at(&c, 100.0),
            (2, Some(11)),
            "내용 250 = B 가운데 → 오른쪽"
        );
        // 「탭바 안」은 뷰포트 그대로 — 내용 x 가 상자 밖이어도 탭바다.
        assert_eq!(tab_bar_at(&c, 599.0), (3, None));
        let fr = frames(&slot(1), CANVAS);
        let w = window(&c, &fr, (0.0, 0.0), 1.0);
        assert_eq!(locate((600.0, 10.0), Some(&w), id(SOURCE)), Locate::Nothing);
        // 스크롤 없음 = 0.
        c.scroll = None;
        assert_eq!(tab_bar_at(&c, 50.0), (1, Some(10)));
    }

    // ── 1 단계: 순서 이동 (PRD R23 · TRD §2-5) ──────────────────────────────────

    // 출처 탭바의 얼린 머리 — A(10) [40,140) · 끄는 탭 [140,260) · B(11) [260,340) · C(12) [340,500).
    // 끄는 머리 폭 120 · 가운데 A 90 · B 300 · C 420.
    fn strip_heads() -> Vec<TabHeader> {
        [
            (10, 40.0, 140.0),
            (SOURCE, 140.0, 260.0),
            (11, 260.0, 340.0),
            (12, 340.0, 500.0),
        ]
        .iter()
        .map(|&(n, x0, x1)| TabHeader {
            view_id: id(n),
            rect: rect(x0, 0.0, x1, BAR_H),
        })
        .collect()
    }

    #[test]
    fn the_reorder_zone_widens_the_bar_by_the_slack_up_and_down_only() {
        let zone = reorder_zone(rect(40.0, 0.0, 640.0, BAR_H));
        assert_eq!(zone, rect(40.0, -15.0, 640.0, BAR_H + 15.0));
        for (p, inside) in [
            ((40.0, -15.0), true),
            ((639.999, 42.999), true),
            ((300.0, -15.001), false),
            ((300.0, 43.0), false),
            ((39.999, 10.0), false),
            ((640.0, 10.0), false),
        ] {
            assert_eq!(contains_css(&zone, p), inside, "{p:?}");
        }
    }

    #[test]
    fn the_dragged_head_follows_the_cursor_and_stops_at_both_ends() {
        let h = strip_heads();
        // 잡은 자리 30 — 커서 내용 x 170 이면 제자리(140).
        assert_eq!(reorder_left(&h, 1, 30.0, 170.0), 140.0);
        assert_eq!(reorder_left(&h, 1, 30.0, 200.5), 170.5);
        assert_eq!(reorder_left(&h, 1, 30.0, 0.0), 40.0, "왼쪽 끝 = 첫 머리 x0");
        assert_eq!(reorder_left(&h, 1, 30.0, 70.0), 40.0);
        assert_eq!(reorder_left(&h, 1, 30.0, 409.0), 379.0);
        assert_eq!(
            reorder_left(&h, 1, 30.0, 410.0),
            380.0,
            "오른쪽 끝 = 마지막 x1 − 폭"
        );
        assert_eq!(reorder_left(&h, 1, 30.0, 5000.0), 380.0);
    }

    #[test]
    fn a_neighbour_moves_only_once_the_dragged_edge_passes_its_center() {
        let h = strip_heads();
        assert_eq!(reorder_index(&h, 1, 140.0), 1, "제자리");
        // 왼쪽 이웃 A — 왼쪽 변이 가운데 90 과 같으면 그대로, 1 ULP 작으면 비킨다.
        assert_eq!(reorder_index(&h, 1, 90.0), 1);
        assert_eq!(reorder_index(&h, 1, 90.0_f64.next_down()), 0);
        // 오른쪽 이웃 B — 오른쪽 변(왼쪽 + 120)이 가운데 300 과 같으면 그대로, 1 ULP 크면 비킨다.
        assert_eq!(reorder_index(&h, 1, 180.0), 1);
        let past = 300.0_f64.next_up() - 120.0;
        assert_eq!(
            past + 120.0,
            300.0_f64.next_up(),
            "전제: 오른쪽 변이 정확히 1 ULP 위"
        );
        assert_eq!(reorder_index(&h, 1, past), 2);
        // C 까지 — 오른쪽 변 420 은 그대로, 넘으면 맨 끝.
        assert_eq!(reorder_index(&h, 1, 300.0), 2);
        assert_eq!(reorder_index(&h, 1, 300.5), 3);
        // 양 끝 고정값에서 맨 왼쪽 0 · 맨 오른쪽 n − 1.
        assert_eq!(reorder_index(&h, 1, 40.0), 0);
        assert_eq!(reorder_index(&h, 1, 380.0), 3);
    }

    #[test]
    fn the_first_and_last_heads_reach_both_ends_too() {
        let h = strip_heads();
        // 맨 왼쪽 머리 A 를 끈다(폭 100 · 범위 40..400).
        assert_eq!(reorder_left(&h, 0, 10.0, 0.0), 40.0);
        assert_eq!(reorder_index(&h, 0, 40.0), 0, "제자리");
        assert_eq!(reorder_left(&h, 0, 10.0, 1000.0), 400.0);
        assert_eq!(reorder_index(&h, 0, 400.0), 3);
        // 맨 오른쪽 머리 C 를 끈다(폭 160 · 범위 40..340).
        assert_eq!(reorder_left(&h, 3, 0.0, 1000.0), 340.0);
        assert_eq!(reorder_index(&h, 3, 340.0), 3, "제자리");
        assert_eq!(reorder_index(&h, 3, 40.0), 0);
    }

    #[test]
    fn a_lone_head_always_stays_put() {
        let h = vec![TabHeader {
            view_id: id(SOURCE),
            rect: rect(40.0, 0.0, 160.0, BAR_H),
        }];
        for x in [-100.0, 0.0, 60.0, 300.0, 5000.0] {
            assert_eq!(reorder_left(&h, 0, 20.0, x), 40.0, "{x}");
            assert_eq!(reorder_index(&h, 0, reorder_left(&h, 0, 20.0, x)), 0, "{x}");
        }
    }

    #[test]
    fn zero_width_heads_still_have_a_center_and_a_threshold() {
        // 끄는 머리 폭 0 — 오른쪽 변 = 왼쪽 변. 이웃 폭 0 은 가운데 = x0.
        let h: Vec<TabHeader> = [
            (10, 0.0, 100.0),
            (SOURCE, 100.0, 100.0),
            (11, 100.0, 200.0),
            (12, 200.0, 200.0),
        ]
        .iter()
        .map(|&(n, x0, x1)| TabHeader {
            view_id: id(n),
            rect: rect(x0, 0.0, x1, BAR_H),
        })
        .collect();
        assert_eq!(reorder_left(&h, 1, 0.0, 500.0), 200.0, "범위 끝 − 폭 0");
        assert_eq!(reorder_index(&h, 1, 50.0), 1);
        assert_eq!(reorder_index(&h, 1, 49.999), 0);
        assert_eq!(
            reorder_index(&h, 1, 150.0),
            1,
            "오른쪽 변 = 가운데 150 → 그대로"
        );
        assert_eq!(reorder_index(&h, 1, 150.001), 2);
        assert_eq!(
            reorder_index(&h, 1, 200.0),
            2,
            "폭 0 머리 가운데 200 = 오른쪽 변 → 그대로"
        );
        assert_eq!(reorder_index(&h, 1, 200.0_f64.next_up()), 3);
    }

    #[test]
    fn heads_without_the_dragged_one_count_it_as_zero_width() {
        // 영역은 이런 값을 싣지 않는다(`regions_for` 조건 ⑤) — 그래도 패닉 없이 같은 식으로 답한다.
        assert_eq!(reorder_left(&[], 0, 30.0, 100.0), 70.0);
        let h = strip_heads();
        assert_eq!(reorder_left(&h, 9, 0.0, 1000.0), 500.0);
        assert_eq!(reorder_index(&h, 9, 500.0), 4);
        assert_eq!(reorder_index(&[], 0, 10.0), 0);
    }

    // ── 2 단계: 띠 고르기 (R6 · R6a ② ③) ────────────────────────────────────────

    // 300×240 칸 — 띠 두께 100(좌우) · 80(위아래).
    const FRAME: PxRect = PxRect {
        x0: 0,
        y0: 0,
        x1: 300,
        y1: 240,
    };

    fn all_bands() -> Vec<(DockSide, PxRect)> {
        vec![
            (DockSide::Left, px(0, 0, 100, 240)),
            (DockSide::Right, px(200, 0, 300, 240)),
            (DockSide::Top, px(0, 0, 300, 80)),
            (DockSide::Bottom, px(0, 160, 300, 240)),
        ]
    }

    #[test]
    fn each_band_and_the_center_one_px_either_side_of_the_band_edges() {
        // 띠는 그린 반열림 사각형 그대로 — 왼쪽 · 위 띠는 안쪽 변이 끝 변이라 빠지고, 오른쪽 · 아래 띠는 안쪽 변이
        // 시작 변이라 든다.
        let b = all_bands();
        let c = |x: f64, y: f64| choose_band((x, y), FRAME, &b);
        assert_eq!(c(99.0, 120.0), Some(DockSide::Left));
        assert_eq!(c(100.0, 120.0), None);
        assert_eq!(c(101.0, 120.0), None);
        assert_eq!(c(199.0, 120.0), None);
        assert_eq!(c(200.0, 120.0), Some(DockSide::Right));
        assert_eq!(c(201.0, 120.0), Some(DockSide::Right));
        assert_eq!(c(299.9, 120.0), Some(DockSide::Right));
        assert_eq!(c(150.0, 79.0), Some(DockSide::Top));
        assert_eq!(c(150.0, 80.0), None);
        assert_eq!(c(150.0, 159.0), None);
        assert_eq!(c(150.0, 160.0), Some(DockSide::Bottom));
        assert_eq!(c(0.0, 120.0), Some(DockSide::Left), "칸 바깥 변은 포함");
        assert_eq!(c(150.0, 0.0), Some(DockSide::Top));
        assert_eq!(c(150.0, 120.0), None, "가운데(R6)");
    }

    #[test]
    fn a_one_px_right_band_is_hit_on_its_only_pixel() {
        // 폭 3px 칸 — 오른쪽 띠 = [2, 3) 한 칸.
        let canvas = CanvasPx { w: 3, h: 240 };
        let target = slot(1);
        let frame = frame_of(&frames(&target, canvas), 1);
        let bands = valid_bands(&target, DockAnchor::Slot(id(1)), &slot(9), canvas);
        assert_eq!(
            bands,
            vec![
                (DockSide::Left, px(0, 0, 1, 240)),
                (DockSide::Right, px(2, 0, 3, 240)),
                (DockSide::Top, px(0, 0, 3, 80)),
                (DockSide::Bottom, px(0, 160, 3, 240)),
            ]
        );
        let c = |x: f64| choose_band((x, 120.0), frame, &bands);
        assert_eq!(c(2.0), Some(DockSide::Right));
        assert_eq!(c(2.5), Some(DockSide::Right));
        assert_eq!(c(1.999), None);
        assert_eq!(c(0.5), Some(DockSide::Left));
    }

    #[test]
    fn a_corner_goes_to_the_smaller_normalized_distance_and_ties_are_fixed() {
        let b = all_bands();
        let c = |x: f64, y: f64| choose_band((x, y), FRAME, &b);
        assert_eq!(c(10.0, 10.0), Some(DockSide::Left), "0.1 < 0.125");
        assert_eq!(c(10.0, 5.0), Some(DockSide::Top), "0.0625 < 0.1");
        assert_eq!(c(50.0, 40.0), Some(DockSide::Left), "0.5 = 0.5 → 왼쪽");
        assert_eq!(
            c(250.0, 40.0),
            Some(DockSide::Right),
            "오른쪽 = 위 → 오른쪽"
        );
        assert_eq!(
            c(250.0, 200.0),
            Some(DockSide::Right),
            "오른쪽 = 아래 → 오른쪽"
        );
        assert_eq!(c(50.0, 200.0), Some(DockSide::Left), "왼쪽 = 아래 → 왼쪽");
        // 두께로 나눈다 — 칸 변까지 거리만 보면 위(30 < 35)지만 정규화하면 왼쪽(0.35 < 0.375).
        assert_eq!(c(35.0, 30.0), Some(DockSide::Left), "0.35 < 0.375");
        assert_eq!(c(40.0, 30.0), Some(DockSide::Top), "0.375 < 0.4");
        // 띠 목록의 순서와 무관하다.
        let mut rev = all_bands();
        rev.reverse();
        assert_eq!(choose_band((50.0, 40.0), FRAME, &rev), Some(DockSide::Left));
    }

    #[test]
    fn a_band_left_out_is_never_chosen() {
        // 빠진 띠 자리는 가운데이고, 모서리에선 남은 띠가 이긴다.
        let b: Vec<_> = all_bands()
            .into_iter()
            .filter(|(s, _)| *s != DockSide::Left)
            .collect();
        assert_eq!(choose_band((50.0, 120.0), FRAME, &b), None);
        assert_eq!(choose_band((10.0, 10.0), FRAME, &b), Some(DockSide::Top));
        assert_eq!(choose_band((50.0, 120.0), FRAME, &[]), None);
        // 두께 0 인 띠도 고르지 않는다.
        let flat = [(DockSide::Left, px(0, 0, 0, 240))];
        assert_eq!(choose_band((0.0, 120.0), FRAME, &flat), None);
    }

    #[test]
    fn a_band_that_spills_out_of_the_frame_is_ignored() {
        // 잘못 짝지은 띠(바깥 변의 결과 사각형 · 이웃 칸의 띠)는 점을 품어도 고르지 않는다.
        let outer_left = (DockSide::Left, px(0, 0, 400, 800));
        assert_eq!(choose_band((10.0, 120.0), FRAME, &[outer_left]), None);
        let neighbour_left = (DockSide::Left, px(250, 0, 350, 240));
        assert_eq!(choose_band((260.0, 120.0), FRAME, &[neighbour_left]), None);
        // 칸 안에 든 띠는 그대로 — 칸 틀과 같은 사각형도 든다.
        let whole = (DockSide::Top, FRAME);
        assert_eq!(
            choose_band((150.0, 120.0), FRAME, &[whole]),
            Some(DockSide::Top)
        );
        let mut mixed = all_bands();
        mixed.push(outer_left);
        assert_eq!(choose_band((150.0, 120.0), FRAME, &mixed), None);
        assert_eq!(
            choose_band((10.0, 120.0), FRAME, &mixed),
            Some(DockSide::Left)
        );
    }

    // ── 2 단계: 이음새 고르기 — 무효 바깥 변은 밑 칸 띠로 ─────────────────────────

    #[test]
    fn choose_seam_takes_a_valid_outer_edge_and_falls_through_an_invalid_one() {
        let target = slot(1);
        let fr = frames(&target, CANVAS);
        let c = chrome(&[]);
        let w = window(&c, &fr, (0.0, 0.0), 1.0);
        let all_outer = valid_bands(&target, DockAnchor::Outer, &slot(9), CANVAS);
        let slot_bands = valid_bands(&target, DockAnchor::Slot(id(1)), &slot(9), CANVAS);
        let loc = locate(at((10.0, 400.0)), Some(&w), id(SOURCE));
        assert_eq!(loc.slot(), Some(id(1)));
        // 유효한 바깥 변 = 바깥.
        assert_eq!(
            choose_seam(&loc, &all_outer, &slot_bands),
            Some((DockAnchor::Outer, DockSide::Left))
        );
        // 그 변이 빠졌으면 밑 칸의 왼쪽 띠(칸 x 10 은 그 띠 안).
        let no_left: Vec<_> = all_outer
            .iter()
            .copied()
            .filter(|(s, _)| *s != DockSide::Left)
            .collect();
        assert_eq!(
            choose_seam(&loc, &no_left, &slot_bands),
            Some((DockAnchor::Slot(id(1)), DockSide::Left))
        );
        // 밑 칸의 띠도 없으면 이음새 없음.
        assert_eq!(choose_seam(&loc, &no_left, &[]), None);
        // 밑 칸을 못 찾은 바깥 띠(칸 목록이 비었다)도 넘어갈 곳이 없다.
        let w_empty = window(&c, &[], (0.0, 0.0), 1.0);
        let lone = locate(at((10.0, 400.0)), Some(&w_empty), id(SOURCE));
        assert!(matches!(lone, Locate::Outer { slot: None, .. }));
        assert_eq!(lone.slot(), None);
        assert_eq!(choose_seam(&lone, &no_left, &slot_bands), None);
        assert_eq!(
            choose_seam(&lone, &all_outer, &[]),
            Some((DockAnchor::Outer, DockSide::Left))
        );
        // 칸 위 = 그 칸의 띠 · 탭바 · 없음 = 이음새 없음.
        let on_slot = locate(at((600.0, 100.0)), Some(&w), id(SOURCE));
        assert_eq!(
            choose_seam(&on_slot, &all_outer, &slot_bands),
            Some((DockAnchor::Slot(id(1)), DockSide::Top))
        );
        let center = locate(at((600.0, 400.0)), Some(&w), id(SOURCE));
        assert_eq!(choose_seam(&center, &all_outer, &slot_bands), None);
        let bar = locate((50.0, 10.0), Some(&w), id(SOURCE));
        assert!(matches!(bar, Locate::TabBar { .. }));
        assert_eq!(choose_seam(&bar, &all_outer, &slot_bands), None);
        assert_eq!(choose_seam(&Locate::Nothing, &all_outer, &slot_bands), None);
    }

    #[test]
    fn a_zero_area_outer_edge_falls_through_to_the_painted_slot_band_beneath() {
        // 끼울 탭 = 좌우 반분 사슬 53 단 — 좌우로 붙이면 1/3 폭에 눌려 깊은 칸이 무너진다(바깥 오른쪽 · 칸 오른쪽
        // 둘 다 무효), 위아래는 선다.
        let incoming = half_chain(SplitDir::LeftRight, 53, 30_000);
        let target = slot(1);
        let outer = valid_bands(&target, DockAnchor::Outer, &incoming, CANVAS);
        let bands = valid_bands(&target, DockAnchor::Slot(id(1)), &incoming, CANVAS);
        assert!(
            !outer.iter().any(|(s, _)| *s == DockSide::Right),
            "전제: 바깥 오른쪽 무효"
        );
        assert!(!bands.iter().any(|(s, _)| *s == DockSide::Right));
        let fr = frames(&target, CANVAS);
        let c = chrome(&[]);
        let w = window(&c, &fr, (0.0, 0.0), 1.0);
        // 바깥 오른쪽 띠 · 칸 위 띠 안 → 칸 위 띠.
        let upper = locate(at((1190.0, 100.0)), Some(&w), id(SOURCE));
        assert_eq!(outer_at(upper.clone()), Some(DockSide::Right));
        assert_eq!(
            choose_seam(&upper, &outer, &bands),
            Some((DockAnchor::Slot(id(1)), DockSide::Top))
        );
        // 바깥 오른쪽 띠 · 칸의 빠진 오른쪽 띠 자리 → 이음새 없음(놓으면 새 창).
        let mid = locate(at((1190.0, 400.0)), Some(&w), id(SOURCE));
        assert_eq!(outer_at(mid.clone()), Some(DockSide::Right));
        assert_eq!(choose_seam(&mid, &outer, &bands), None);
        // 모서리 — 가까운 오른쪽 바깥 변은 무효지만 같이 품은 위 바깥 변(칠해져 있다)이 이긴다. 밑 칸으로
        // 넘어가는 것은 품은 바깥 변이 전부 무효일 때뿐이다.
        assert!(
            outer.iter().any(|(s, _)| *s == DockSide::Top),
            "전제: 바깥 위 유효"
        );
        let corner = locate(at((1190.0, 20.0)), Some(&w), id(SOURCE));
        assert!(
            matches!(&corner, Locate::Outer { sides, .. } if *sides == [DockSide::Right, DockSide::Top])
        );
        assert_eq!(
            choose_seam(&corner, &outer, &bands),
            Some((DockAnchor::Outer, DockSide::Top))
        );
        let only_right_invalid: Vec<_> = outer
            .iter()
            .copied()
            .filter(|(s, _)| *s != DockSide::Top)
            .collect();
        assert_eq!(
            choose_seam(&corner, &only_right_invalid, &bands),
            Some((DockAnchor::Slot(id(1)), DockSide::Top)),
            "품은 바깥 변이 전부 무효 → 밑 칸 위 띠"
        );
    }

    // ── 띠 = 놓았을 때의 결과 (R3 · R4 · R8 · AC15) ─────────────────────────────

    #[test]
    fn a_band_is_exactly_where_the_incoming_tab_will_be_drawn() {
        let target = split(
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
            let before = frames(&target, canvas);
            for anchor in [
                DockAnchor::Slot(id(1)),
                DockAnchor::Slot(id(2)),
                DockAnchor::Slot(id(3)),
                DockAnchor::Outer,
            ] {
                let bands = valid_bands(&target, anchor, &incoming, canvas);
                assert_eq!(bands.len(), 4, "{canvas:?} {anchor:?}");
                for &(side, band) in &bands {
                    let ctx = format!("{canvas:?} {anchor:?} {side:?}");
                    // 놓은 뒤 화면이 그릴 끼운 칸 틀들의 외접 사각형.
                    let result =
                        docked_tree(&target, anchor, side, &incoming, DOCK_NEW_SHARE).unwrap();
                    let after = frames(&result, canvas);
                    let (a, b) = (frame_of(&after, 21), frame_of(&after, 22));
                    let drawn = px(
                        a.x0.min(b.x0),
                        a.y0.min(b.y0),
                        a.x1.max(b.x1),
                        a.y1.max(b.y1),
                    );
                    assert_eq!(band, drawn, "{ctx}");
                    assert_eq!(
                        landing_rect(&target, anchor, side, &incoming, canvas),
                        Some(band)
                    );

                    let DockAnchor::Slot(anchor_id) = anchor else {
                        continue;
                    };
                    // 띠는 칸 바깥 변에 붙고, 띠와 놓은 뒤의 기준 칸이 원래 칸을 정확히 나눠 갖는다.
                    let frame = frame_of(&before, anchor_id.as_u128());
                    let kept = frame_of(&after, anchor_id.as_u128());
                    let (want_band, want_kept) = match side {
                        DockSide::Left => (
                            px(frame.x0, frame.y0, band.x1, frame.y1),
                            px(band.x1, frame.y0, frame.x1, frame.y1),
                        ),
                        DockSide::Right => (
                            px(band.x0, frame.y0, frame.x1, frame.y1),
                            px(frame.x0, frame.y0, band.x0, frame.y1),
                        ),
                        DockSide::Top => (
                            px(frame.x0, frame.y0, frame.x1, band.y1),
                            px(frame.x0, band.y1, frame.x1, frame.y1),
                        ),
                        DockSide::Bottom => (
                            px(frame.x0, band.y0, frame.x1, frame.y1),
                            px(frame.x0, frame.y0, frame.x1, band.y0),
                        ),
                    };
                    assert_eq!((band, kept), (want_band, want_kept), "{ctx}");
                    // 띠 두께 ≈ 칸의 1/3(R3).
                    let (thick, whole) = match side {
                        DockSide::Left | DockSide::Right => {
                            (band.x1 - band.x0, frame.x1 - frame.x0)
                        }
                        DockSide::Top | DockSide::Bottom => {
                            (band.y1 - band.y0, frame.y1 - frame.y0)
                        }
                    };
                    assert!(
                        (f64::from(thick) - f64::from(whole) / 3.0).abs() <= 1.0,
                        "{ctx}: {thick}/{whole}"
                    );
                    // 그린 띠의 가운데를 짚으면 그 띠가 골린다.
                    let mid = (
                        (f64::from(band.x0) + f64::from(band.x1)) / 2.0,
                        (f64::from(band.y0) + f64::from(band.y1)) / 2.0,
                    );
                    assert_eq!(choose_band(mid, frame, &bands), Some(side), "{ctx}");
                }
            }
        }
    }

    #[test]
    fn an_outer_landing_is_a_third_of_the_whole_canvas() {
        let target = split(100, SplitDir::LeftRight, 0.3, slot(1), slot(2));
        assert_eq!(
            valid_bands(&target, DockAnchor::Outer, &slot(9), CANVAS),
            vec![
                (DockSide::Left, px(0, 0, 400, 800)),
                (DockSide::Right, px(800, 0, 1200, 800)),
                (DockSide::Top, px(0, 0, 1200, 267)),
                (DockSide::Bottom, px(0, 533, 1200, 800)),
            ]
        );
    }

    #[test]
    fn a_band_that_would_leave_a_zero_area_slot_is_left_out_and_not_hit() {
        // 끼울 탭 = 좌우 반분 사슬 53 단(단독으로는 전부 면적이 있다). 좌우로 붙이면 1/3 폭에 눌려 깊은 칸이
        // 무너지고, 위아래로 붙이면 폭이 그대로라 선다.
        let incoming = half_chain(SplitDir::LeftRight, 53, 30_000);
        assert!(
            geometry::compute(&incoming)
                .slots
                .iter()
                .all(|r| r.x0 < r.x1 && r.y0 < r.y1),
            "전제: 끼울 것은 단독으로 전부 면적이 있다"
        );
        let target = slot(1);
        let anchor = DockAnchor::Slot(id(1));
        assert!(
            docked_tree(&target, anchor, DockSide::Right, &incoming, DOCK_NEW_SHARE).is_err(),
            "전제: 오른쪽은 면적 0"
        );
        let bands = valid_bands(&target, anchor, &incoming, CANVAS);
        for side in SIDE_ORDER {
            let ok = docked_tree(&target, anchor, side, &incoming, DOCK_NEW_SHARE).is_ok();
            assert_eq!(bands.iter().any(|(s, _)| *s == side), ok, "{side:?}");
        }
        assert!(bands.iter().any(|(s, _)| *s == DockSide::Top));
        assert!(bands.iter().any(|(s, _)| *s == DockSide::Bottom));
        assert_eq!(
            landing_rect(&target, anchor, DockSide::Right, &incoming, CANVAS),
            None
        );

        // 칸 위에서 오른쪽 띠 자리를 짚으면 가운데다 — 놓으면 새 창(R8 · AC15).
        let fr = frames(&target, CANVAS);
        let c = chrome(&[]);
        let w = window(&c, &fr, (0.0, 0.0), 1.0);
        let loc = locate(at((1100.0, 400.0)), Some(&w), id(SOURCE));
        assert!(matches!(loc, Locate::Slot { .. }));
        assert_eq!(choose_seam(&loc, &[], &bands), None);
    }

    #[test]
    fn a_missing_anchor_slot_has_no_bands() {
        let target = slot(1);
        assert!(valid_bands(&target, DockAnchor::Slot(id(77)), &slot(9), CANVAS).is_empty());
    }
}
