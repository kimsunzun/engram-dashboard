//! 끌기 영역 만들기와 그 위의 판정(TRD S21-slot-docking §2-4) — Tauri · 락 · OS 호출 0 이라 같은 입력이면 같은
//! 답이다(PRD R6a ③).
//!
//! 진입점: [`regions_for`](창 하나의 스냅숏 → 웹뷰에 보낼 [`DockRegions`] + 놓기 기대값 [`Expect`]) ·
//! [`pick`](영역 위의 점 → [`Picked`]) · [`bumps`](마지막 보낸 것과 견줘 `rev` · `bar_rev` 를 올릴지) ·
//! [`visible_tab`](창에 보이는 탭 — 스냅숏을 뜨는 쪽이 부른다).
//!
//! ★규칙의 정본은 `dock::hit` 하나다★ — [`pick`] 은 영역 값으로 `hit::reorder_*`(순서 이동 구역)를, 아니면
//! `hit::WindowFrame` 을 다시 세워 `hit::locate` → `hit::choose_seam` 을 부르는 어댑터이고, 웹뷰의
//! `pickTarget`(`dockPick.ts`)은 같은 영역 위의 거울이다. 둘은 골든 `pick_golden.json` 을 함께 먹는다 — 규칙을
//! 고치면 골든과 TS 판을 같은 변경에서 고친다.

use uuid::Uuid;

use super::hit::{self, Locate, WindowFrame};
use super::types::{
    Band, DockRegions, OuterBand, Picked, ReorderRegion, SlotRegion, StripStart, TabBarRegion,
};
use crate::layout::chrome::{DockChrome, PointCss, RectCss};
use crate::layout::dock::{DockAnchor, DockSide};
use crate::layout::geometry::{self, PxRect};
use crate::layout::manager::{succession, TargetExpect, ViewId, WindowLabel};
use crate::layout::{CanvasPx, LayoutNode};

/// 창 하나의 영역 재료 — 레이아웃 락 안의 모델 값과 측정(크롬 · 캔버스)이다. 창 게터(위치 · 크기 · 보임)는 싣지
/// 않는다(TRD §1-3 「창 게터」).
#[derive(Debug, Clone, PartialEq)]
pub struct WindowSnap {
    pub label: WindowLabel,
    /// 그 창의 탭 목록(모델 순서).
    pub tabs: Vec<ViewId>,
    /// 그 창의 **모델** 활성 탭 — 끄는 탭일 수 있다.
    pub active: ViewId,
    /// 지금 보이는 탭과 그 배치 — 탭은 [`visible_tab`] 이 고른 것. `None` = 보이는 탭 없음.
    pub visible: Option<(ViewId, LayoutNode)>,
    /// 캔버스 크기 보고. `None` = 아직 없음.
    pub canvas: Option<CanvasPx>,
    /// 크롬 보고. `None` = 아직 없음.
    pub chrome: Option<DockChrome>,
    /// 출처 창을 끌기 동안 숨겼다 — 장부 값(`hide()` 직후의 보임 게터는 늦을 수 있어 게터를 읽지 않는다).
    pub hidden: bool,
    pub is_source: bool,
}

/// 한 창의 놓기 기대값 재료 — 영역을 만든 순간의 모델 값이다. 놓기는 이 값을 `DockGuard::expect_target` 에 실어
/// 모델이 같은 락 안에서 다시 대조한다(R13c).
#[derive(Debug, Clone, PartialEq)]
pub struct Expect {
    /// 보이는 탭의 배치. `None` = 보이는 탭 없음(이음새 놓기 없음).
    pub layout: Option<LayoutNode>,
    /// 그 창의 모델 활성 — 같은 창 도킹에서 끄는 탭이 활성이면 끄는 탭 자신이다(화면에 대신 보이는 승계 탭이 아니다).
    pub active: ViewId,
    /// 그 창의 탭 목록(모델 순서).
    pub tabs: Vec<ViewId>,
}

impl Expect {
    /// 이음새 놓기의 기대값 `Seam{보이는 탭 배치, 모델 활성}`. `None` = 보이는 탭이 없다.
    pub fn seam(&self) -> Option<TargetExpect> {
        self.layout.clone().map(|layout| TargetExpect::Seam {
            layout,
            active: self.active,
        })
    }

    /// 탭바 놓기의 기대값 `Tabs(탭 목록)` — 활성은 보지 않는다(TRD §1-3 「놓기 판정」 4b).
    pub fn tabs(&self) -> TargetExpect {
        TargetExpect::Tabs(self.tabs.clone())
    }
}

/// 창에 지금 보이는 탭 — 모델 활성이 끄는 탭이면 떼어진 자리에 보이는 승계 탭([`succession`] — 탭 닫기와 같은
/// 규칙), 아니면 모델 활성. `None` = 끄는 탭 말고는 탭이 없다(메인 마지막 탭을 뗐다 · 탭 하나뿐인 팝아웃).
pub fn visible_tab(tabs: &[ViewId], active: ViewId, source_view: ViewId) -> Option<ViewId> {
    if active == source_view {
        succession(tabs, source_view)
    } else {
        Some(active)
    }
}

/// 창 하나의 영역과 기대값. `source_view` = 끄는 탭 · `incoming` = 끌기 시작 때의 그 배치(끼울 것) · `blocked` =
/// 출처 배치가 그 뒤 바뀌었다 · `strip` = 끌기 시작 때 출처 웹뷰가 잰 탭바(출처 창에만 쓴다). `None` = 숨긴 출처
/// 창(영역을 보내지 않는다).
///
/// 영역의 `drag_id` · `rev` · `bar_rev` 는 비워 둔다(빈 문자열 · 0) — 장부가 [`bumps`] 로 정해 채운다. 기대값은
/// `blocked` 와 무관하게 그 창의 모델 값이다. `reorder` 는 출처 창 · 막히지 않음 · `strip` 이 그 창 모델 탭 목록과
/// 맞음(TRD §2-4 조건 ①–⑥)일 때만 선다 — 아니면 그 창 탭바는 `tab_bar` 로 판정한다.
pub fn regions_for(
    w: &WindowSnap,
    source_view: ViewId,
    incoming: &LayoutNode,
    blocked: bool,
    strip: Option<&StripStart>,
) -> Option<(DockRegions, Expect)> {
    if w.is_source && w.hidden {
        return None;
    }
    // 스냅숏을 뜨는 쪽이 끄는 탭 자신을 보이는 탭으로 채우면 시험에서 잡는다.
    debug_assert_eq!(
        w.visible.as_ref().map(|(view, _)| *view),
        visible_tab(&w.tabs, w.active, source_view),
        "WindowSnap.visible 은 visible_tab 이 고른 탭이어야 한다"
    );
    let expect = Expect {
        layout: w.visible.as_ref().map(|(_, layout)| layout.clone()),
        active: w.active,
        tabs: w.tabs.clone(),
    };
    let mut regions = DockRegions {
        drag_id: String::new(),
        rev: 0,
        bar_rev: 0,
        blocked,
        view_id: None,
        canvas_origin: PointCss { x: 0.0, y: 0.0 },
        canvas: CanvasPx { w: 0, h: 0 },
        outer: Vec::new(),
        slots: Vec::new(),
        tab_bar: None,
        reorder: None,
    };
    if blocked {
        return Some((regions, expect));
    }
    if w.is_source {
        regions.reorder = strip.and_then(|s| reorder_region(s, &w.tabs, source_view));
    }
    regions.view_id = w.visible.as_ref().map(|(view, _)| *view);
    if let Some(canvas) = w.canvas {
        regions.canvas = canvas;
    }
    if let Some(chrome) = &w.chrome {
        regions.canvas_origin = chrome.canvas_origin;
        regions.tab_bar = Some(tab_bar_region(chrome, source_view));
    }
    if let (Some(_), Some(canvas), Some((_, layout))) = (&w.chrome, w.canvas, &w.visible) {
        regions.outer = outer_bands(layout, incoming, canvas);
        regions.slots = slot_regions(layout, incoming, canvas);
    }
    Some((regions, expect))
}

/// 영역 위의 점 `p`(그 창 뷰포트 CSS px) — 순서 이동 구역 안이면 `hit::reorder_*` 로 `Reorder`, 아니면
/// `hit::locate` → `hit::choose_seam` 을 영역 값 위에서 부른다(안쪽 원점 (0, 0) · 배율 1). `source_view` = 끄는 탭.
///
/// `scroll` = 그 판정 때 그 창 탭바 `scrollLeft`(CSS px) — 순서 이동 자리 · 탭바 자리(`index` · `over`)를 점의 내용
/// x(`p.x + scroll`)로 낸다. `None` = 영역마다 잰 때의 것(`reorder.scroll` · `tab_bar.scroll`). 구역 · 탭바 상자 ·
/// 캔버스 검사는 스크롤과 무관하게 `p` 그대로다.
pub fn pick(r: &DockRegions, p: PointCss, scroll: Option<f64>, source_view: ViewId) -> Picked {
    if let Some(re) = &r.reorder {
        if hit::contains_css(&re.zone, (p.x, p.y)) {
            let dragged = usize::try_from(re.dragged).unwrap_or(usize::MAX);
            let left = hit::reorder_left(
                &re.heads,
                dragged,
                re.grab_x,
                p.x + scroll.unwrap_or(re.scroll),
            );
            return Picked::Reorder {
                index: u32::try_from(hit::reorder_index(&re.heads, dragged, left))
                    .unwrap_or(u32::MAX),
            };
        }
    }
    let chrome = DockChrome {
        canvas_origin: r.canvas_origin,
        // 탭바가 없으면 빈 상자 — 반열림이라 어느 점도 품지 않는다.
        tab_bar: r.tab_bar.as_ref().map_or(
            RectCss {
                x0: 0.0,
                y0: 0.0,
                x1: 0.0,
                y1: 0.0,
            },
            |b| b.rect,
        ),
        tabs: r
            .tab_bar
            .as_ref()
            .map_or_else(Vec::new, |b| b.heads.clone()),
        scroll: Some(scroll.unwrap_or_else(|| r.tab_bar.as_ref().map_or(0.0, |b| b.scroll))),
    };
    let slots: Vec<(Uuid, PxRect)> = r
        .slots
        .iter()
        .map(|s| (s.slot_id, s.frame.into()))
        .collect();
    let frame = WindowFrame {
        label: "",
        inner_origin: (0.0, 0.0),
        scale: 1.0,
        chrome: &chrome,
        canvas: r.canvas,
        // 보이는 탭이 없으면 끄는 탭을 넣는다 — `locate` 의 「끄는 탭 자신 = 없음」이 캔버스 위를 가이드 없음으로 둔다.
        visible_view: r.view_id.unwrap_or(source_view),
        slots: &slots,
    };
    let loc = hit::locate((p.x, p.y), Some(&frame), source_view);
    if let Locate::TabBar { index, over, .. } = loc {
        return Picked::TabBar {
            index: u32::try_from(index).unwrap_or(u32::MAX),
            over,
        };
    }
    let outer: Vec<(DockSide, PxRect)> =
        r.outer.iter().map(|o| (o.side, o.landing.into())).collect();
    let bands: Vec<(DockSide, PxRect)> = loc
        .slot()
        .and_then(|slot| r.slots.iter().find(|s| s.slot_id == slot))
        .map_or_else(Vec::new, |s| {
            s.bands.iter().map(|b| (b.side, b.rect.into())).collect()
        });
    match hit::choose_seam(&loc, &outer, &bands) {
        Some((DockAnchor::Slot(slot), side)) => Picked::Seam {
            anchor: Some(slot),
            side,
        },
        Some((DockAnchor::Outer, side)) => Picked::Seam { anchor: None, side },
        None => Picked::None,
    }
}

/// [`bumps`] 의 답 — 장부가 그 창의 번호를 올릴지.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bumps {
    pub rev: bool,
    pub bar_rev: bool,
}

/// 그 창에 마지막으로 보낸 (영역, 기대값) `sent` 와 새로 만든 `next` 를 견준다(TRD §1-3 「영역」) — 영역의
/// `drag_id` · `rev` · `bar_rev` 는 보지 않는다. `sent` = `None`(처음 보냄)이면 둘 다 올린다.
///
/// - `rev` — 영역이나 기대값(보이는 탭 배치 · 모델 활성 · 탭 목록)이 바뀌었다. 기하가 같아도 기대값만 바뀌면
///   오른다(대상 칸 내용만 바뀐 경우 — 안 올리면 본 가이드 그대로 놓아도 모델 대조가 막는다).
/// - `bar_rev` — 탭바 영역(기하)이나 순서 이동 영역(생김 · 빠짐)이나 탭 목록이 바뀌었다(이때 `rev` 도 오른다).
///   활성 전환은 올리지 않는다.
///
/// 탭바 영역의 `scroll` 은 견주지 않는다 — 기하가 아니라서, 견주면 스크롤만 바뀐 다시 잼마다 번호만 헛돈다.
pub fn bumps(sent: Option<(&DockRegions, &Expect)>, next: (&DockRegions, &Expect)) -> Bumps {
    let Some((sent_r, sent_e)) = sent else {
        return Bumps {
            rev: true,
            bar_rev: true,
        };
    };
    let (next_r, next_e) = next;
    let bar_rev = !same_bar(&sent_r.tab_bar, &next_r.tab_bar)
        || sent_r.reorder != next_r.reorder
        || sent_e.tabs != next_e.tabs;
    Bumps {
        rev: bar_rev || !same_body(sent_r, next_r) || sent_e != next_e,
        bar_rev,
    }
}

// 구조 분해라 칸이 늘면 컴파일이 여기를 짚는다.
fn same_body(a: &DockRegions, b: &DockRegions) -> bool {
    let DockRegions {
        drag_id: _,
        rev: _,
        bar_rev: _,
        blocked,
        view_id,
        canvas_origin,
        canvas,
        outer,
        slots,
        tab_bar,
        reorder,
    } = a;
    *blocked == b.blocked
        && *view_id == b.view_id
        && *canvas_origin == b.canvas_origin
        && *canvas == b.canvas
        && *outer == b.outer
        && *slots == b.slots
        && same_bar(tab_bar, &b.tab_bar)
        && *reorder == b.reorder
}

// `scroll` 만 빼고 견준다(`bumps` 문서) — 구조 분해라 칸이 늘면 컴파일이 여기를 짚는다.
fn same_bar(a: &Option<TabBarRegion>, b: &Option<TabBarRegion>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            let TabBarRegion {
                rect,
                heads,
                insert_xs,
                scroll: _,
            } = a;
            *rect == b.rect && *heads == b.heads && *insert_xs == b.insert_xs
        }
        _ => false,
    }
}

fn tab_bar_region(chrome: &DockChrome, source_view: ViewId) -> TabBarRegion {
    // 크롬은 떼어진 표시보다 늦게 접힐 수 있어 끄는 머리가 남아 있을 수 있다 — id 로 거른다. 기하는 다음 크롬
    // 보고가 고친다(그 틈에도 그림과 판정이 같은 영역이라 보인 그대로 들어간다 — TRD §2-4).
    let heads: Vec<_> = chrome
        .tabs
        .iter()
        .filter(|t| t.view_id != source_view)
        .cloned()
        .collect();
    let insert_xs = (0..=heads.len())
        .map(|index| hit::tab_insert_x(chrome, source_view, index))
        .collect();
    TabBarRegion {
        rect: chrome.tab_bar,
        heads,
        insert_xs,
        scroll: chrome.scroll.unwrap_or(0.0),
    }
}

// TRD §2-4 조건 ④–⑥(① `strip` 있음 · ② 막히지 않음 · ③ 숨긴 출처 아님 · 출처 창은 부르는 쪽이 본다). 하나라도 어기면
// `None` — 얼린 기하가 모델과 갈라졌으니 그 창 탭바는 `tab_bar` 로 판정한다.
fn reorder_region(
    strip: &StripStart,
    tabs: &[ViewId],
    source_view: ViewId,
) -> Option<ReorderRegion> {
    let heads = &strip.heads;
    if !heads.iter().map(|h| h.view_id).eq(tabs.iter().copied()) {
        return None;
    }
    let mut at_source = heads
        .iter()
        .enumerate()
        .filter(|(_, h)| h.view_id == source_view)
        .map(|(i, _)| i);
    let (Some(dragged), None) = (at_source.next(), at_source.next()) else {
        return None;
    };
    let finite = |r: &RectCss| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite());
    let shaped = finite(&strip.bar)
        && strip.grab_x.is_finite()
        && strip.scroll.is_finite()
        && heads
            .iter()
            .all(|h| finite(&h.rect) && h.rect.x0 <= h.rect.x1)
        && heads.windows(2).all(|p| p[0].rect.x0 <= p[1].rect.x0);
    if !shaped {
        return None;
    }
    Some(ReorderRegion {
        zone: hit::reorder_zone(strip.bar),
        heads: heads.clone(),
        dragged: u32::try_from(dragged).ok()?,
        grab_x: strip.grab_x,
        scroll: strip.scroll,
    })
}

fn outer_bands(layout: &LayoutNode, incoming: &LayoutNode, canvas: CanvasPx) -> Vec<OuterBand> {
    let strips = hit::outer_strips(canvas);
    hit::valid_bands(layout, DockAnchor::Outer, incoming, canvas)
        .into_iter()
        .filter_map(|(side, landing)| {
            strips
                .iter()
                .find(|(s, _)| *s == side)
                .map(|&(_, strip)| OuterBand {
                    side,
                    strip: strip.into(),
                    landing: landing.into(),
                })
        })
        .collect()
}

fn slot_regions(layout: &LayoutNode, incoming: &LayoutNode, canvas: CanvasPx) -> Vec<SlotRegion> {
    geometry::compute(layout)
        .slots
        .iter()
        .map(|r| SlotRegion {
            slot_id: r.slot_id,
            frame: geometry::frame_px(canvas.w, canvas.h, r).into(),
            bands: hit::valid_bands(layout, DockAnchor::Slot(r.slot_id), incoming, canvas)
                .into_iter()
                .map(|(side, rect)| Band {
                    side,
                    anchor: Some(r.slot_id),
                    rect: rect.into(),
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::dock::hit::SIDE_ORDER;
    use crate::layout::chrome::TabHeader;
    use crate::layout::dock::docked_tree;
    use crate::layout::{SlotContent, SplitDir};

    const SRC: u128 = 99;
    const CANVAS: CanvasPx = CanvasPx { w: 1200, h: 800 };
    const BAR_H: f64 = 28.0;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn ids(ns: &[u128]) -> Vec<Uuid> {
        ns.iter().map(|&n| id(n)).collect()
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

    // 반분 사슬 — 늘 b 쪽이 다음 단이다(`dock::hit` 시험과 같은 모양).
    fn half_chain(dir: SplitDir, depth: u128, base: u128) -> LayoutNode {
        let mut node = slot(base);
        for i in 1..=depth {
            node = split(base + 1000 + i, dir, 0.5, slot(base + i), node);
        }
        node
    }

    fn halves() -> LayoutNode {
        split(100, SplitDir::LeftRight, 0.5, slot(1), slot(2))
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> RectCss {
        RectCss { x0, y0, x1, y1 }
    }

    // 탭바 = 뷰포트 (0,0)–(600,28) · 캔버스 원점 = (0,28).
    fn chrome(heads: &[(u128, f64, f64)]) -> DockChrome {
        DockChrome {
            canvas_origin: PointCss { x: 0.0, y: BAR_H },
            tab_bar: rect(0.0, 0.0, 600.0, BAR_H),
            tabs: heads
                .iter()
                .map(|&(n, x0, x1)| TabHeader {
                    view_id: id(n),
                    rect: rect(x0, 0.0, x1, BAR_H),
                })
                .collect(),
            scroll: None,
        }
    }

    // 출처 웹뷰가 끌기 시작 때 잰 탭바 — 머리는 크롬과 같은 꼴.
    fn strip(bar: RectCss, heads: &[(u128, f64, f64)], grab_x: f64, scroll: f64) -> StripStart {
        StripStart {
            bar,
            heads: chrome(heads).tabs,
            grab_x,
            scroll,
        }
    }

    fn snap(tabs: &[u128], active: u128, layout: LayoutNode) -> WindowSnap {
        let visible = visible_tab(&ids(tabs), id(active), id(SRC)).map(|v| (v, layout));
        WindowSnap {
            label: "main".into(),
            tabs: ids(tabs),
            active: id(active),
            visible,
            canvas: Some(CANVAS),
            chrome: Some(chrome(&[(10, 0.0, 100.0), (11, 100.0, 200.0)])),
            hidden: false,
            is_source: false,
        }
    }

    fn regions(w: &WindowSnap) -> (DockRegions, Expect) {
        regions_for(w, id(SRC), &slot(9), false, None).expect("숨긴 출처가 아니다")
    }

    // ── 창 하나의 영역 ────────────────────────────────────────────────────────────

    #[test]
    fn a_hidden_source_window_gets_no_regions() {
        let mut w = snap(&[SRC], SRC, slot(1));
        w.is_source = true;
        w.hidden = true;
        assert_eq!(regions_for(&w, id(SRC), &slot(9), false, None), None);
        assert_eq!(regions_for(&w, id(SRC), &slot(9), true, None), None);
        w.hidden = false;
        assert!(regions_for(&w, id(SRC), &slot(9), false, None).is_some());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "visible_tab")]
    fn a_snapshot_showing_the_dragged_tab_trips_in_debug() {
        let mut w = snap(&[10, SRC], SRC, slot(1));
        w.visible = Some((id(SRC), slot(1)));
        let _ = regions_for(&w, id(SRC), &slot(9), false, None);
    }

    #[test]
    fn slots_and_outer_are_the_hit_geometry_of_the_visible_tab() {
        let layout = halves();
        let (r, _) = regions(&snap(&[10, 11], 10, layout.clone()));
        assert_eq!(r.drag_id, "");
        assert_eq!((r.rev, r.bar_rev, r.blocked), (0, 0, false));
        assert_eq!(r.view_id, Some(id(10)));
        assert_eq!(r.canvas, CANVAS);
        assert_eq!(r.canvas_origin, PointCss { x: 0.0, y: BAR_H });
        let geo = geometry::compute(&layout);
        assert_eq!(r.slots.len(), 2);
        for (s, g) in r.slots.iter().zip(&geo.slots) {
            assert_eq!(s.slot_id, g.slot_id);
            assert_eq!(s.frame, geometry::frame_px(CANVAS.w, CANVAS.h, g).into());
            let want: Vec<Band> =
                hit::valid_bands(&layout, DockAnchor::Slot(g.slot_id), &slot(9), CANVAS)
                    .into_iter()
                    .map(|(side, rect)| Band {
                        side,
                        anchor: Some(g.slot_id),
                        rect: rect.into(),
                    })
                    .collect();
            assert_eq!(s.bands, want);
            assert_eq!(s.bands.len(), 4);
        }
        let strips = hit::outer_strips(CANVAS);
        let landings = hit::valid_bands(&layout, DockAnchor::Outer, &slot(9), CANVAS);
        assert_eq!(r.outer.len(), 4);
        for ((o, (side, strip)), (landing_side, landing)) in
            r.outer.iter().zip(strips).zip(landings)
        {
            assert_eq!((o.side, o.side), (side, landing_side));
            assert_eq!(o.strip, strip.into());
            assert_eq!(o.landing, landing.into());
        }
    }

    #[test]
    fn a_band_that_would_leave_a_zero_area_slot_is_not_carried() {
        // 끼울 탭 = 좌우 반분 사슬 53 단 — 좌우로 붙이면 1/3 폭에 눌려 깊은 칸이 무너지고(R8) 위아래는 선다.
        let incoming = half_chain(SplitDir::LeftRight, 53, 30_000);
        let target = slot(1);
        let w = snap(&[10], 10, target.clone());
        let (r, _) = regions_for(&w, id(SRC), &incoming, false, None).unwrap();
        let outer: Vec<_> = r.outer.iter().map(|o| o.side).collect();
        let bands: Vec<_> = r.slots[0].bands.iter().map(|b| b.side).collect();
        for side in SIDE_ORDER {
            let ok = |anchor| docked_tree(&target, anchor, side, &incoming).is_ok();
            assert_eq!(outer.contains(&side), ok(DockAnchor::Outer), "{side:?}");
            assert_eq!(
                bands.contains(&side),
                ok(DockAnchor::Slot(id(1))),
                "{side:?}"
            );
        }
        assert!(!outer.contains(&DockSide::Right) && !bands.contains(&DockSide::Right));
        assert!(outer.contains(&DockSide::Top) && bands.contains(&DockSide::Bottom));
    }

    #[test]
    fn blocked_empties_the_regions_but_keeps_the_expectation() {
        let w = snap(&[10, 11], 10, halves());
        let (r, e) = regions_for(&w, id(SRC), &slot(9), true, None).unwrap();
        assert_eq!(
            r,
            DockRegions {
                drag_id: String::new(),
                rev: 0,
                bar_rev: 0,
                blocked: true,
                view_id: None,
                canvas_origin: PointCss { x: 0.0, y: 0.0 },
                canvas: CanvasPx { w: 0, h: 0 },
                outer: vec![],
                slots: vec![],
                tab_bar: None,
                reorder: None,
            }
        );
        assert_eq!(e, regions(&w).1);
    }

    #[test]
    fn without_chrome_or_canvas_there_are_no_slots_or_outer() {
        let full = snap(&[10], 10, halves());

        let mut no_chrome = full.clone();
        no_chrome.chrome = None;
        let (r, _) = regions(&no_chrome);
        assert!(r.slots.is_empty() && r.outer.is_empty());
        assert_eq!(r.tab_bar, None, "탭바는 크롬이 있어야");
        assert_eq!(r.canvas, CANVAS);
        assert_eq!(r.canvas_origin, PointCss { x: 0.0, y: 0.0 });
        assert_eq!(r.view_id, Some(id(10)));

        let mut no_canvas = full.clone();
        no_canvas.canvas = None;
        let (r, _) = regions(&no_canvas);
        assert!(r.slots.is_empty() && r.outer.is_empty());
        assert!(r.tab_bar.is_some(), "탭바는 크롬만 있으면");
        assert_eq!(r.canvas, CanvasPx { w: 0, h: 0 });
        assert_eq!(r.canvas_origin, PointCss { x: 0.0, y: BAR_H });

        let mut neither = full;
        neither.chrome = None;
        neither.canvas = None;
        let (r, _) = regions(&neither);
        assert!(r.slots.is_empty() && r.outer.is_empty() && r.tab_bar.is_none());
    }

    #[test]
    fn the_dragged_head_is_filtered_by_id_even_while_the_chrome_still_shows_it() {
        let mut w = snap(&[10, SRC, 11], 10, halves());
        w.is_source = true;
        w.chrome = Some(chrome(&[
            (10, 0.0, 100.0),
            (SRC, 100.0, 200.0),
            (11, 200.0, 300.0),
        ]));
        let (r, _) = regions(&w);
        let bar = r.tab_bar.unwrap();
        assert_eq!(
            bar.heads.iter().map(|h| h.view_id).collect::<Vec<_>>(),
            ids(&[10, 11])
        );
        assert_eq!(bar.rect, rect(0.0, 0.0, 600.0, BAR_H));
        assert_eq!(bar.insert_xs, vec![0.0, 200.0, 300.0]);
    }

    #[test]
    fn insert_xs_are_tab_insert_x_for_every_slot_from_the_bar_left() {
        let mut c = chrome(&[(11, 144.0, 244.0), (SRC, 244.0, 344.0), (10, 44.0, 144.0)]);
        c.tab_bar = rect(40.0, 0.0, 640.0, BAR_H);
        let mut w = snap(&[10, 11, SRC], 10, halves());
        w.chrome = Some(c.clone());
        let bar = regions(&w).0.tab_bar.unwrap();
        let want: Vec<f64> = (0..=2).map(|i| hit::tab_insert_x(&c, id(SRC), i)).collect();
        assert_eq!(bar.insert_xs, want);
        assert_eq!(bar.insert_xs, vec![4.0, 104.0, 204.0]);
        // 머리는 크롬 보고 순서 그대로 — 끼울 자리는 가운데 x 로 센다.
        assert_eq!(
            bar.heads.iter().map(|h| h.view_id).collect::<Vec<_>>(),
            ids(&[11, 10])
        );
    }

    #[test]
    fn the_visible_tab_is_the_successor_while_the_dragged_tab_is_active() {
        assert_eq!(
            visible_tab(&ids(&[10, SRC, 11]), id(SRC), id(SRC)),
            Some(id(11)),
            "오른쪽 먼저"
        );
        assert_eq!(
            visible_tab(&ids(&[10, SRC]), id(SRC), id(SRC)),
            Some(id(10)),
            "없으면 왼쪽"
        );
        assert_eq!(
            visible_tab(&ids(&[SRC]), id(SRC), id(SRC)),
            None,
            "메인 마지막 탭"
        );
        assert_eq!(
            visible_tab(&ids(&[10, SRC]), id(10), id(SRC)),
            Some(id(10)),
            "활성이 끄는 탭이 아니면 그대로"
        );
        for tabs in [&[10, SRC, 11][..], &[10, SRC][..], &[SRC, 11][..]] {
            assert_eq!(
                visible_tab(&ids(tabs), id(SRC), id(SRC)),
                succession(&ids(tabs), id(SRC)),
                "탭 닫기와 같은 승계"
            );
        }
    }

    #[test]
    fn the_seam_expectation_is_the_visible_layout_with_the_model_active() {
        // 출처 창에서 끄는 탭이 활성 — 보이는 탭은 승계 탭 11 이고 기대값의 활성은 끄는 탭 자신이다.
        let layout = halves();
        let mut w = snap(&[10, SRC, 11], SRC, layout.clone());
        w.is_source = true;
        let (r, e) = regions(&w);
        assert_eq!(r.view_id, Some(id(11)));
        assert_eq!(
            e.seam(),
            Some(TargetExpect::Seam {
                layout,
                active: id(SRC),
            })
        );
        assert_eq!(e.tabs(), TargetExpect::Tabs(ids(&[10, SRC, 11])));

        // 보이는 탭이 없으면 이음새 기대값도 없다(탭바는 있다).
        let mut lone = snap(&[SRC], SRC, slot(1));
        lone.is_source = true;
        let (r, e) = regions(&lone);
        assert_eq!(r.view_id, None);
        assert!(r.slots.is_empty() && r.outer.is_empty());
        assert_eq!(e.seam(), None);
        assert_eq!(e.tabs(), TargetExpect::Tabs(ids(&[SRC])));
        assert_eq!(
            r.tab_bar.map(|b| b.insert_xs.len()),
            Some(3),
            "머리 10 · 11 + 끝"
        );
    }

    // ── 순서 이동 영역 (TRD §2-4 「출처 창의 reorder」 · §2-5) ──────────────────────

    const BAR: RectCss = RectCss {
        x0: 0.0,
        y0: 0.0,
        x1: 600.0,
        y1: BAR_H,
    };
    // 모델 탭 [10, SRC, 11] 의 머리 — 끄는 머리 폭 120.
    const SRC_HEADS: [(u128, f64, f64); 3] =
        [(10, 0.0, 100.0), (SRC, 100.0, 220.0), (11, 220.0, 300.0)];

    fn source(tabs: &[u128]) -> WindowSnap {
        let mut w = snap(tabs, 10, halves());
        w.is_source = true;
        w
    }

    fn reorder_of(w: &WindowSnap, s: &StripStart) -> Option<ReorderRegion> {
        regions_for(w, id(SRC), &slot(9), false, Some(s))
            .expect("숨긴 출처가 아니다")
            .0
            .reorder
    }

    #[test]
    fn the_source_window_freezes_the_start_strip_as_its_reorder_region() {
        let w = source(&[10, SRC, 11]);
        let s = strip(BAR, &SRC_HEADS, 30.0, 12.5);
        let (r, _) = regions_for(&w, id(SRC), &slot(9), false, Some(&s)).unwrap();
        assert_eq!(
            r.reorder,
            Some(ReorderRegion {
                zone: hit::reorder_zone(BAR),
                heads: s.heads.clone(),
                dragged: 1,
                grab_x: 30.0,
                scroll: 12.5,
            })
        );
        // 탭바는 그대로 실린다 — reorder 가 빠지는 순간의 물러날 자리.
        let plain = regions_for(&w, id(SRC), &slot(9), false, None).unwrap().0;
        assert!(r.tab_bar.is_some());
        assert_eq!(r.tab_bar, plain.tab_bar);
        assert_eq!(plain.reorder, None, "① strip 없음");
    }

    #[test]
    fn blocked_a_hidden_source_and_other_windows_have_no_reorder() {
        let s = strip(BAR, &SRC_HEADS, 30.0, 0.0);
        let w = source(&[10, SRC, 11]);
        let (blocked, _) = regions_for(&w, id(SRC), &slot(9), true, Some(&s)).unwrap();
        assert_eq!(blocked.reorder, None, "② blocked");
        let mut hidden = w.clone();
        hidden.hidden = true;
        assert_eq!(
            regions_for(&hidden, id(SRC), &slot(9), false, Some(&s)),
            None,
            "③ 숨긴 출처는 영역 자체가 없다"
        );
        let mut other = w.clone();
        other.is_source = false;
        assert_eq!(reorder_of(&other, &s), None, "다른 창");
    }

    #[test]
    fn a_strip_that_disagrees_with_the_model_tabs_has_no_reorder() {
        let s = |heads: &[(u128, f64, f64)]| strip(BAR, heads, 30.0, 0.0);
        let w = source(&[10, SRC, 11]);
        assert!(reorder_of(&w, &s(&SRC_HEADS)).is_some(), "전제");
        // ④ 끄는 동안 다른 경로가 탭을 더하거나 빼거나 옮겼다.
        let added = [
            (10, 0.0, 100.0),
            (SRC, 100.0, 220.0),
            (11, 220.0, 300.0),
            (12, 300.0, 400.0),
        ];
        assert_eq!(reorder_of(&w, &s(&added)), None, "더함");
        assert_eq!(reorder_of(&w, &s(&SRC_HEADS[..2])), None, "뺌");
        let moved = [(11, 0.0, 80.0), (SRC, 80.0, 200.0), (10, 200.0, 300.0)];
        assert_eq!(reorder_of(&w, &s(&moved)), None, "옮김");
        // ⑤ 끄는 머리가 정확히 하나 — 목록이 모델과 같아도 없거나 둘이면 없다.
        let none = [(10, 0.0, 100.0), (11, 100.0, 200.0)];
        assert_eq!(
            reorder_of(&source(&[10, 11]), &s(&none)),
            None,
            "끄는 머리 없음"
        );
        let twice = [(10, 0.0, 100.0), (SRC, 100.0, 200.0), (SRC, 200.0, 300.0)];
        assert_eq!(
            reorder_of(&source(&[10, SRC, SRC]), &s(&twice)),
            None,
            "끄는 머리 둘"
        );
    }

    #[test]
    fn a_strip_out_of_order_or_not_finite_has_no_reorder() {
        let w = source(&[10, SRC, 11]);
        let with = |change: &dyn Fn(&mut StripStart)| {
            let mut s = strip(BAR, &SRC_HEADS, 30.0, 0.0);
            change(&mut s);
            reorder_of(&w, &s)
        };
        assert!(with(&|_| {}).is_some(), "전제");
        // ⑥ x0 오름차순 · 폭 ≥ 0 · 모든 수가 유한.
        assert_eq!(
            with(&|s| s.heads[2].rect = rect(50.0, 0.0, 90.0, BAR_H)),
            None,
            "x0 가 내려감"
        );
        assert!(
            with(&|s| {
                s.heads[1].rect.x1 = 100.0;
                s.heads[2].rect.x0 = 100.0;
            })
            .is_some(),
            "폭 0 · 같은 x0 는 받는다"
        );
        assert_eq!(with(&|s| s.heads[0].rect.x1 = -1.0), None, "폭 음수");
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(with(&|s| s.grab_x = bad), None, "grab_x {bad}");
            assert_eq!(with(&|s| s.scroll = bad), None, "scroll {bad}");
            assert_eq!(with(&|s| s.bar.y1 = bad), None, "bar {bad}");
            assert_eq!(with(&|s| s.heads[1].rect.y0 = bad), None, "머리 {bad}");
        }
    }

    // 손으로 세운 출처 창 영역 — 바깥 띠가 없어 구역 밑에 칸 1 의 위 띠가 드러나 있다(`regions_for` 가 만드는 영역에서는
    // 바깥 위 띠가 늘 그 자리를 덮는다).
    fn hand_source() -> DockRegions {
        let heads = chrome(&SRC_HEADS).tabs;
        DockRegions {
            drag_id: String::new(),
            rev: 1,
            bar_rev: 1,
            blocked: false,
            view_id: Some(id(10)),
            canvas_origin: PointCss { x: 0.0, y: BAR_H },
            canvas: CanvasPx { w: 600, h: 400 },
            outer: vec![],
            slots: vec![SlotRegion {
                slot_id: id(1),
                frame: PxRect {
                    x0: 0,
                    y0: 0,
                    x1: 600,
                    y1: 400,
                }
                .into(),
                bands: vec![Band {
                    side: DockSide::Top,
                    anchor: Some(id(1)),
                    rect: PxRect {
                        x0: 0,
                        y0: 0,
                        x1: 600,
                        y1: 133,
                    }
                    .into(),
                }],
            }],
            tab_bar: Some(TabBarRegion {
                rect: BAR,
                heads: heads
                    .iter()
                    .filter(|h| h.view_id != id(SRC))
                    .cloned()
                    .collect(),
                insert_xs: vec![0.0, 220.0, 300.0],
                scroll: 0.0,
            }),
            reorder: Some(ReorderRegion {
                zone: hit::reorder_zone(BAR),
                heads,
                dragged: 1,
                grab_x: 30.0,
                scroll: 0.0,
            }),
        }
    }

    fn at(x: f64, y: f64) -> PointCss {
        PointCss { x, y }
    }

    const TOP_OF_1: Picked = Picked::Seam {
        anchor: Some(Uuid::from_u128(1)),
        side: DockSide::Top,
    };

    #[test]
    fn the_reorder_zone_is_picked_before_the_tab_bar_and_every_band() {
        let r = hand_source();
        let pick_at = |r: &DockRegions, x, y| pick(r, at(x, y), None, id(SRC));
        assert_eq!(pick_at(&r, 150.0, 10.0), Picked::Reorder { index: 1 });
        assert_eq!(
            pick_at(&r, 150.0, 42.999),
            Picked::Reorder { index: 1 },
            "칸 위 띠 위"
        );
        assert_eq!(
            pick_at(&r, 150.0, 43.0),
            TOP_OF_1,
            "구역 아래 경계 = 지금 판정"
        );
        // 구역이 없으면 같은 점이 그대로 6a 판정이다.
        let mut plain = r.clone();
        plain.reorder = None;
        assert_eq!(
            pick_at(&plain, 150.0, 10.0),
            Picked::TabBar {
                index: 1,
                over: None,
            }
        );
        assert_eq!(pick_at(&plain, 150.0, 42.999), TOP_OF_1);
    }

    #[test]
    fn the_scroll_moves_the_index_but_not_the_zone_the_bar_box_or_the_canvas() {
        let r = hand_source();
        // 순서 이동 — 내용 x 150 → 왼쪽 120 · 자리 1 / 내용 x 250 → 왼쪽 180(끝 고정) · 자리 2.
        assert_eq!(
            pick(&r, at(150.0, 10.0), None, id(SRC)),
            Picked::Reorder { index: 1 }
        );
        assert_eq!(
            pick(&r, at(150.0, 10.0), Some(100.0), id(SRC)),
            Picked::Reorder { index: 2 }
        );
        // `None` = 영역의 reorder.scroll.
        let mut scrolled = r.clone();
        scrolled.reorder.as_mut().unwrap().scroll = 100.0;
        assert_eq!(
            pick(&scrolled, at(150.0, 10.0), None, id(SRC)),
            Picked::Reorder { index: 2 }
        );
        assert_eq!(
            pick(&scrolled, at(150.0, 10.0), Some(0.0), id(SRC)),
            Picked::Reorder { index: 1 }
        );
        // 탭바 — 같은 점 · 다른 스크롤 = 다른 자리 · over. `None` = 영역의 tab_bar.scroll.
        let mut plain = r.clone();
        plain.reorder = None;
        let bar = |index, over: u128| Picked::TabBar {
            index,
            over: Some(id(over)),
        };
        assert_eq!(pick(&plain, at(60.0, 10.0), None, id(SRC)), bar(1, 10));
        assert_eq!(
            pick(&plain, at(60.0, 10.0), Some(200.0), id(SRC)),
            bar(2, 11)
        );
        plain.tab_bar.as_mut().unwrap().scroll = 200.0;
        assert_eq!(pick(&plain, at(60.0, 10.0), None, id(SRC)), bar(2, 11));
        assert_eq!(pick(&plain, at(60.0, 10.0), Some(0.0), id(SRC)), bar(1, 10));
        // 구역 · 탭바 상자 · 캔버스 검사는 점 그대로.
        assert_eq!(pick(&r, at(150.0, 43.0), Some(500.0), id(SRC)), TOP_OF_1);
        assert_eq!(
            pick(&r, at(150.0, 42.999), Some(-500.0), id(SRC)),
            Picked::Reorder { index: 0 }
        );
        assert_eq!(
            pick(&plain, at(600.0, 10.0), Some(-100.0), id(SRC)),
            Picked::None
        );
    }

    // ── rev · bar_rev ────────────────────────────────────────────────────────────

    const BOTH: Bumps = Bumps {
        rev: true,
        bar_rev: true,
    };
    const REV: Bumps = Bumps {
        rev: true,
        bar_rev: false,
    };
    const NEITHER: Bumps = Bumps {
        rev: false,
        bar_rev: false,
    };

    fn bump(a: &WindowSnap, b: &WindowSnap) -> Bumps {
        let (ra, ea) = regions(a);
        let (rb, eb) = regions(b);
        bumps(Some((&ra, &ea)), (&rb, &eb))
    }

    #[test]
    fn the_first_send_bumps_both_and_an_unchanged_one_neither() {
        let w = snap(&[10, 11], 10, halves());
        let (r, e) = regions(&w);
        assert_eq!(bumps(None, (&r, &e)), BOTH);
        assert_eq!(bump(&w, &w), NEITHER);
        // 장부가 채운 번호 · id 는 견주지 않는다.
        let mut sent = r.clone();
        sent.drag_id = "d-1".into();
        sent.rev = 7;
        sent.bar_rev = 3;
        assert_eq!(bumps(Some((&sent, &e)), (&r, &e)), NEITHER);
    }

    #[test]
    fn an_active_switch_bumps_rev_but_not_bar_rev() {
        let before = snap(&[10, 11], 10, halves());
        let after = snap(&[10, 11], 11, slot(5));
        assert_eq!(bump(&before, &after), REV);
        // 보이는 탭 · 기하가 같아도 모델 활성만 바뀌면(같은 창 끌기에서 활성이 끄는 탭으로) rev 는 오른다.
        let a = snap(&[10, SRC, 11], 11, halves());
        let mut b = snap(&[10, SRC, 11], SRC, halves());
        b.visible = a.visible.clone();
        assert_eq!(regions(&a).0, regions(&b).0, "전제: 영역은 같다");
        assert_eq!(bump(&a, &b), REV);
    }

    #[test]
    fn a_content_only_change_bumps_rev_with_the_same_geometry() {
        let before = snap(&[10], 10, halves());
        let changed = split(
            100,
            SplitDir::LeftRight,
            0.5,
            LayoutNode::Slot {
                id: id(1),
                content: SlotContent::AgentList,
            },
            slot(2),
        );
        let after = snap(&[10], 10, changed);
        assert_eq!(regions(&before).0, regions(&after).0, "전제: 영역은 같다");
        assert_eq!(bump(&before, &after), REV);
    }

    #[test]
    fn tab_order_or_tab_bar_geometry_bumps_both() {
        let before = snap(&[10, 11], 10, halves());
        assert_eq!(bump(&before, &snap(&[11, 10], 10, halves())), BOTH);
        let mut moved = before.clone();
        moved.chrome = Some(chrome(&[(10, 0.0, 120.0), (11, 120.0, 240.0)]));
        assert_eq!(bump(&before, &moved), BOTH);
        // 막히면 탭바가 비므로 둘 다 — 탭바가 이미 없던 창은 rev 만.
        let (r, e) = regions(&before);
        let (blocked, be) = regions_for(&before, id(SRC), &slot(9), true, None).unwrap();
        assert_eq!(bumps(Some((&r, &e)), (&blocked, &be)), BOTH);
        let mut barless = r.clone();
        barless.tab_bar = None;
        assert_eq!(bumps(Some((&barless, &e)), (&blocked, &be)), REV);
    }

    #[test]
    fn a_reorder_region_appearing_or_leaving_bumps_both() {
        let w = source(&[10, SRC, 11]);
        let s = strip(BAR, &SRC_HEADS, 30.0, 0.0);
        let (with, e) = regions_for(&w, id(SRC), &slot(9), false, Some(&s)).unwrap();
        let (without, plain_e) = regions_for(&w, id(SRC), &slot(9), false, None).unwrap();
        assert_eq!(e, plain_e, "전제: 기대값은 같다");
        assert_eq!(bumps(Some((&without, &e)), (&with, &e)), BOTH, "생김");
        assert_eq!(bumps(Some((&with, &e)), (&without, &e)), BOTH, "빠짐");
        assert_eq!(bumps(Some((&with, &e)), (&with, &e)), NEITHER);
    }

    #[test]
    fn a_tab_bar_scroll_alone_bumps_neither() {
        let before = snap(&[10, 11], 10, halves());
        let mut after = before.clone();
        after.chrome.as_mut().unwrap().scroll = Some(140.0);
        let (ra, _) = regions(&before);
        let (rb, _) = regions(&after);
        assert_eq!(
            ra.tab_bar.as_ref().map(|b| b.scroll),
            Some(0.0),
            "크롬 scroll 없음 = 0"
        );
        assert_eq!(
            rb.tab_bar.as_ref().map(|b| b.scroll),
            Some(140.0),
            "전제: 실린다"
        );
        assert_eq!(bump(&before, &after), NEITHER);
    }

    // ── pick = 골든 ──────────────────────────────────────────────────────────────

    const GOLDEN: &str = include_str!("pick_golden.json");

    #[derive(serde::Deserialize)]
    struct Golden {
        source_view: Uuid,
        regions: BTreeMap<String, DockRegions>,
        cases: Vec<Case>,
    }

    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        regions: String,
        point: PointCss,
        /// 그 판정의 탭바 스크롤 — 없음 = `pick` 에 `None`.
        scroll: Option<f64>,
        expect: Picked,
        /// 순서 이동 사례의 끄는 머리 왼쪽(`hit::reorder_left`) — 웹뷰가 그리는 자리도 같은 파일로 묶는다.
        left: Option<f64>,
    }

    fn golden() -> Golden {
        serde_json::from_str(GOLDEN).expect("골든이 wire 꼴로 읽혀야")
    }

    // 골든의 영역마다 그것을 만든 창 스냅숏 · 끼울 것 · blocked · 시작 측정.
    fn fixture(name: &str) -> (WindowSnap, LayoutNode, bool, Option<StripStart>) {
        let with_heads = |tabs: &[u128], active: u128, layout, heads: &[(u128, f64, f64)]| {
            let mut w = snap(tabs, active, layout);
            w.chrome = Some(chrome(heads));
            w
        };
        let three_heads = [(10, 0.0, 100.0), (SRC, 100.0, 200.0), (11, 200.0, 300.0)];
        let small = CanvasPx { w: 400, h: 300 };
        match name {
            // 출처 창(탭 셋 · 끄는 탭은 활성 아님) — 크롬에 끄는 머리가 아직 남아 있다.
            "split" => {
                let mut w = with_heads(&[10, SRC, 11], 10, halves(), &three_heads);
                w.is_source = true;
                (w, slot(9), false, None)
            }
            // 좌우로 붙이면 면적 0 이 생기는 끼울 것 — 바깥 · 칸의 좌우 띠가 빠진다.
            "invalid_outer" => (
                with_heads(&[20], 20, slot(1), &[(20, 0.0, 100.0)]),
                half_chain(SplitDir::LeftRight, 53, 30_000),
                false,
                None,
            ),
            // 3px 칸 둘(세로로 얇은 칸 · 가로로 얇은 칸) — 그 칸의 마주 보는 띠가 1px 이다.
            "thin" => {
                let left = split(
                    101,
                    SplitDir::TopBottom,
                    0.5,
                    slot(1),
                    split(102, SplitDir::TopBottom, 0.01, slot(2), slot(3)),
                );
                let right = split(103, SplitDir::LeftRight, 0.01, slot(4), slot(5));
                let layout = split(100, SplitDir::LeftRight, 0.5, left, right);
                let mut w = with_heads(&[30], 30, layout, &[(30, 0.0, 100.0)]);
                w.canvas = Some(CanvasPx { w: 600, h: 600 });
                (w, slot(9), false, None)
            }
            // 메인 마지막 탭을 끈다 — 보이는 탭 없음.
            "no_visible" => {
                let mut w = with_heads(&[SRC], SRC, slot(1), &[(SRC, 0.0, 100.0)]);
                w.is_source = true;
                (w, slot(9), false, None)
            }
            "no_chrome" => {
                let mut w = snap(&[40], 40, slot(1));
                w.chrome = None;
                (w, slot(9), false, None)
            }
            "no_canvas" => {
                let heads = [(40, 0.0, 100.0), (41, 100.0, 200.0)];
                let mut w = with_heads(&[40, 41], 40, slot(1), &heads);
                w.canvas = None;
                (w, slot(9), false, None)
            }
            "blocked" => {
                let mut w = with_heads(&[10, SRC, 11], 10, halves(), &three_heads);
                w.is_source = true;
                (w, slot(9), true, None)
            }
            // 캔버스 원점 (40, 28) · 탭바 x0 40 — 원점을 빼먹은 거울이 다른 답을 낸다.
            "offset" => {
                let mut c = chrome(&[(50, 44.0, 144.0), (51, 144.0, 244.0)]);
                c.canvas_origin = PointCss { x: 40.0, y: BAR_H };
                c.tab_bar = rect(40.0, 0.0, 640.0, BAR_H);
                let mut w = snap(&[50, 51], 50, halves());
                w.chrome = Some(c);
                (w, slot(9), false, None)
            }
            // 출처 창 · 머리 넷(폭 100 · 120 · 80 · 160 — 끄는 머리 120) · `offset` 과 같은 원점.
            "source_strip" => {
                let heads = [
                    (50, 40.0, 140.0),
                    (SRC, 140.0, 260.0),
                    (51, 260.0, 340.0),
                    (52, 340.0, 500.0),
                ];
                let bar = rect(40.0, 0.0, 640.0, BAR_H);
                let mut c = chrome(&heads);
                c.canvas_origin = PointCss { x: 40.0, y: BAR_H };
                c.tab_bar = bar;
                let mut w = snap(&[50, SRC, 51, 52], 50, halves());
                w.chrome = Some(c);
                w.is_source = true;
                (w, slot(9), false, Some(strip(bar, &heads, 30.0, 0.0)))
            }
            // 탭 하나 팝아웃을 끈다 — 머리 하나 · 보이는 탭 없음.
            "source_lone" => {
                let heads = [(SRC, 40.0, 160.0)];
                let bar = rect(40.0, 0.0, 640.0, BAR_H);
                let mut c = chrome(&heads);
                c.canvas_origin = PointCss { x: 40.0, y: BAR_H };
                c.tab_bar = bar;
                let mut w = snap(&[SRC], SRC, slot(1));
                w.chrome = Some(c);
                w.is_source = true;
                (w, slot(9), false, Some(strip(bar, &heads, 20.0, 0.0)))
            }
            // 1 ULP 경계 — 잡은 자리 0 이라 점 x = 끄는 머리 왼쪽(아래 `the_ulp_region_...` 시험이 전제를 잰다).
            "source_ulp" => {
                let heads = [(80, 40.2, 90.4), (SRC, 90.4, 190.4), (81, 190.4, 256.4)];
                let bar = rect(0.0, 0.0, 400.0, BAR_H);
                let mut c = chrome(&heads);
                c.tab_bar = bar;
                let mut w = snap(&[80, SRC, 81], 80, slot(1));
                w.chrome = Some(c);
                w.canvas = Some(small);
                w.is_source = true;
                (w, slot(9), false, Some(strip(bar, &heads, 0.0, 0.0)))
            }
            // 넘친 출처 탭바 — 머리 다섯(내용 좌표 0..500)이 폭 300 탭바에 · 시작 때 120 밀려 있었다.
            "source_overflow" => {
                let heads = [
                    (60, 0.0, 100.0),
                    (61, 100.0, 200.0),
                    (SRC, 200.0, 300.0),
                    (62, 300.0, 400.0),
                    (63, 400.0, 500.0),
                ];
                let bar = rect(0.0, 0.0, 300.0, BAR_H);
                let mut c = chrome(&heads);
                c.tab_bar = bar;
                c.scroll = Some(120.0);
                let mut w = snap(&[60, 61, SRC, 62, 63], 60, slot(1));
                w.chrome = Some(c);
                w.canvas = Some(small);
                w.is_source = true;
                (w, slot(9), false, Some(strip(bar, &heads, 50.0, 120.0)))
            }
            // 넘친 다른 창 탭바 — 머리 넷(내용 좌표 0..400)이 폭 250 탭바에 · 크롬을 잰 때 80 밀려 있었다.
            "target_overflow" => {
                let heads = [
                    (70, 0.0, 100.0),
                    (71, 100.0, 200.0),
                    (72, 200.0, 300.0),
                    (73, 300.0, 400.0),
                ];
                let mut c = chrome(&heads);
                c.tab_bar = rect(0.0, 0.0, 250.0, BAR_H);
                c.scroll = Some(80.0);
                let mut w = snap(&[70, 71, 72, 73], 70, slot(1));
                w.chrome = Some(c);
                w.canvas = Some(small);
                (w, slot(9), false, None)
            }
            other => panic!("골든 영역 {other} 을 만든 스냅숏이 없다"),
        }
    }

    #[test]
    fn pick_answers_every_golden_case() {
        let g = golden();
        assert_eq!(g.source_view, id(SRC));
        let mut wrong = Vec::new();
        for c in &g.cases {
            let r = g
                .regions
                .get(&c.regions)
                .unwrap_or_else(|| panic!("{}: 영역 {} 이 골든에 없다", c.name, c.regions));
            let got = pick(r, c.point, c.scroll, g.source_view);
            if got != c.expect {
                wrong.push(format!(
                    "{}: {:?} · 스크롤 {:?} → {got:?} (골든 {:?})",
                    c.name, c.point, c.scroll, c.expect
                ));
            }
            let left = match (&c.expect, &r.reorder) {
                (Picked::Reorder { .. }, Some(re)) => Some(hit::reorder_left(
                    &re.heads,
                    usize::try_from(re.dragged).unwrap(),
                    re.grab_x,
                    c.point.x + c.scroll.unwrap_or(re.scroll),
                )),
                _ => None,
            };
            if left != c.left {
                wrong.push(format!(
                    "{}: 끄는 머리 왼쪽 {left:?} (골든 {:?} — 순서 이동 사례에만 싣는다)",
                    c.name, c.left
                ));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[test]
    fn every_golden_region_is_what_regions_for_builds() {
        let g = golden();
        let mut wrong = Vec::new();
        for (name, r) in &g.regions {
            let (w, incoming, blocked, strip) = fixture(name);
            let (mut built, _) =
                regions_for(&w, id(SRC), &incoming, blocked, strip.as_ref()).unwrap();
            // 장부가 채우는 칸만 골든 값으로 맞추고 나머지는 전부 견준다(`same_body` 가 빼는 탭바 스크롤까지).
            built.drag_id.clone_from(&r.drag_id);
            (built.rev, built.bar_rev) = (r.rev, r.bar_rev);
            if built != *r {
                wrong.push(format!(
                    "골든 영역 {name} 이 regions_for 와 다르다 — 지금 값:\n{}",
                    serde_json::to_string(&built).unwrap()
                ));
            }
            if !g.cases.iter().any(|c| c.regions == *name) {
                wrong.push(format!("{name} 을 쓰는 사례가 없다"));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    // 끄는 탭이 영역에 없어야 `source_view` 를 받지 않는 거울도 같은 답을 낸다. 순서 이동 영역의 머리만 예외다 — 거기서는
    // 끄는 머리를 id 가 아니라 `dragged` 자리로 찾는다.
    #[test]
    fn the_dragged_tab_appears_in_no_golden_region_but_its_reorder_heads() {
        let g = golden();
        for (name, r) in &g.regions {
            let mut seen = vec![r.view_id];
            seen.extend(r.slots.iter().map(|s| Some(s.slot_id)));
            seen.extend(
                r.slots
                    .iter()
                    .flat_map(|s| s.bands.iter().map(|b| b.anchor)),
            );
            if let Some(bar) = &r.tab_bar {
                seen.extend(bar.heads.iter().map(|h| Some(h.view_id)));
            }
            assert!(!seen.contains(&Some(g.source_view)), "{name}");
            if let Some(re) = &r.reorder {
                let at = usize::try_from(re.dragged).unwrap();
                assert_eq!(re.heads[at].view_id, g.source_view, "{name}");
            }
        }
    }

    #[test]
    fn the_golden_covers_every_kind_of_answer() {
        let g = golden();
        let has = |want: &dyn Fn(&Picked) -> bool| g.cases.iter().any(|c| want(&c.expect));
        for side in SIDE_ORDER {
            assert!(
                has(&|p| *p == Picked::Seam { anchor: None, side }),
                "바깥 {side:?}"
            );
            assert!(
                has(&|p| matches!(p, Picked::Seam { anchor: Some(_), side: s } if *s == side)),
                "칸 {side:?}"
            );
        }
        assert!(has(&|p| *p == Picked::None));
        assert!(has(&|p| matches!(p, Picked::TabBar { over: Some(_), .. })));
        assert!(has(&|p| matches!(p, Picked::TabBar { over: None, .. })));
        assert!(has(&|p| matches!(p, Picked::Reorder { .. })));
        assert!(g.cases.iter().any(|c| c.scroll.is_some()), "스크롤 사례");
    }

    // 골든 `source_ulp` 사례의 전제 — 이웃 가운데가 짧은 소수 점에서 정확히 1 ULP 떨어져 있다. 1 ULP 옆 값을 골든에
    // 바로 적지 않는 것은 serde_json(기본 기능 — `float_roundtrip` 없음)이 17 자리 소수를 1 ULP 어긋나게 읽어서다
    // (실측 2026-10-17: `119.99999999999999` → `120.0`). 골든의 수는 짧은 소수로 둔다.
    #[test]
    fn the_ulp_region_centers_are_one_ulp_off_the_short_points() {
        assert_eq!((40.2_f64 + 90.4) / 2.0, 65.3_f64.next_up());
        assert_eq!(190.4_f64 - 90.4, 100.0, "끄는 머리 폭");
        assert_eq!((190.4_f64 + 256.4) / 2.0, 223.4_f64.next_down());
        assert_eq!(123.4_f64 + 100.0, 223.4);
    }

    // 스크롤 0 = 스크롤 없음 — 넘친 영역 밖의 옛 사례는 `Some(0)` 을 줘도 답이 같다(6a 판정 그대로).
    #[test]
    fn a_zero_scroll_answers_like_no_scroll_where_nothing_was_scrolled() {
        let g = golden();
        for c in g.cases.iter().filter(|c| c.scroll.is_none()) {
            let r = &g.regions[&c.regions];
            let measured = r.tab_bar.as_ref().map_or(0.0, |b| b.scroll);
            let frozen = r.reorder.as_ref().map_or(0.0, |re| re.scroll);
            if measured == 0.0 && frozen == 0.0 {
                assert_eq!(
                    pick(r, c.point, Some(0.0), g.source_view),
                    c.expect,
                    "{}",
                    c.name
                );
            }
        }
    }
}
