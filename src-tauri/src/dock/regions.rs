//! 끌기 영역 만들기와 그 위의 판정(TRD S21-slot-docking §2-4) — Tauri · 락 · OS 호출 0 이라 같은 입력이면 같은
//! 답이다(PRD R6a ③).
//!
//! 진입점: [`regions_for`](창 하나의 스냅숏 → 웹뷰에 보낼 [`DockRegions`] + 놓기 기대값 [`Expect`]) ·
//! [`pick`](영역 위의 점 → [`Picked`]) · [`bumps`](마지막 보낸 것과 견줘 `rev` · `bar_rev` 를 올릴지) ·
//! [`visible_tab`](창에 보이는 탭 — 스냅숏을 뜨는 쪽이 부른다).
//!
//! ★규칙의 정본은 `dock::hit` 하나다★ — [`pick`] 은 영역에서 `hit::WindowFrame` 을 다시 세워 `hit::locate` →
//! `hit::choose_seam` 을 부르는 어댑터이고, 웹뷰의 `pickTarget`(`dockPick.ts`)은 같은 영역 위의 거울이다. 둘은 골든
//! `pick_golden.json` 을 함께 먹는다 — 규칙을 고치면 골든과 TS 판을 같은 변경에서 고친다.

use uuid::Uuid;

use super::hit::{self, Locate, WindowFrame};
use super::types::{Band, DockRegions, OuterBand, Picked, SlotRegion, TabBarRegion};
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
/// 출처 배치가 그 뒤 바뀌었다. `None` = 숨긴 출처 창(영역을 보내지 않는다).
///
/// 영역의 `drag_id` · `rev` · `bar_rev` 는 비워 둔다(빈 문자열 · 0) — 장부가 [`bumps`] 로 정해 채운다. 기대값은
/// `blocked` 와 무관하게 그 창의 모델 값이다.
pub fn regions_for(
    w: &WindowSnap,
    source_view: ViewId,
    incoming: &LayoutNode,
    blocked: bool,
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
    };
    if blocked {
        return Some((regions, expect));
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

/// 영역 위의 점 `p`(그 창 뷰포트 CSS px) — `hit::locate` → `hit::choose_seam` 을 영역 값 위에서 부른다(안쪽 원점
/// (0, 0) · 배율 1). `source_view` = 끄는 탭.
pub fn pick(r: &DockRegions, p: PointCss, source_view: ViewId) -> Picked {
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
/// - `bar_rev` — 탭바 영역이나 탭 목록이 바뀌었다(이때 `rev` 도 오른다). 활성 전환은 올리지 않는다.
pub fn bumps(sent: Option<(&DockRegions, &Expect)>, next: (&DockRegions, &Expect)) -> Bumps {
    let Some((sent_r, sent_e)) = sent else {
        return Bumps {
            rev: true,
            bar_rev: true,
        };
    };
    let (next_r, next_e) = next;
    let bar_rev = sent_r.tab_bar != next_r.tab_bar || sent_e.tabs != next_e.tabs;
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
    } = a;
    *blocked == b.blocked
        && *view_id == b.view_id
        && *canvas_origin == b.canvas_origin
        && *canvas == b.canvas
        && *outer == b.outer
        && *slots == b.slots
        && *tab_bar == b.tab_bar
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
    }
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
        regions_for(w, id(SRC), &slot(9), false).expect("숨긴 출처가 아니다")
    }

    // ── 창 하나의 영역 ────────────────────────────────────────────────────────────

    #[test]
    fn a_hidden_source_window_gets_no_regions() {
        let mut w = snap(&[SRC], SRC, slot(1));
        w.is_source = true;
        w.hidden = true;
        assert_eq!(regions_for(&w, id(SRC), &slot(9), false), None);
        assert_eq!(regions_for(&w, id(SRC), &slot(9), true), None);
        w.hidden = false;
        assert!(regions_for(&w, id(SRC), &slot(9), false).is_some());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "visible_tab")]
    fn a_snapshot_showing_the_dragged_tab_trips_in_debug() {
        let mut w = snap(&[10, SRC], SRC, slot(1));
        w.visible = Some((id(SRC), slot(1)));
        let _ = regions_for(&w, id(SRC), &slot(9), false);
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
        let (r, _) = regions_for(&w, id(SRC), &incoming, false).unwrap();
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
        let (r, e) = regions_for(&w, id(SRC), &slot(9), true).unwrap();
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
        let (blocked, be) = regions_for(&before, id(SRC), &slot(9), true).unwrap();
        assert_eq!(bumps(Some((&r, &e)), (&blocked, &be)), BOTH);
        let mut barless = r.clone();
        barless.tab_bar = None;
        assert_eq!(bumps(Some((&barless, &e)), (&blocked, &be)), REV);
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
        expect: Picked,
    }

    fn golden() -> Golden {
        serde_json::from_str(GOLDEN).expect("골든이 wire 꼴로 읽혀야")
    }

    // 골든의 영역마다 그것을 만든 창 스냅숏 · 끼울 것 · blocked.
    fn fixture(name: &str) -> (WindowSnap, LayoutNode, bool) {
        let with_heads = |tabs: &[u128], active: u128, layout, heads: &[(u128, f64, f64)]| {
            let mut w = snap(tabs, active, layout);
            w.chrome = Some(chrome(heads));
            w
        };
        let three_heads = [(10, 0.0, 100.0), (SRC, 100.0, 200.0), (11, 200.0, 300.0)];
        match name {
            // 출처 창(탭 셋 · 끄는 탭은 활성 아님) — 크롬에 끄는 머리가 아직 남아 있다.
            "split" => {
                let mut w = with_heads(&[10, SRC, 11], 10, halves(), &three_heads);
                w.is_source = true;
                (w, slot(9), false)
            }
            // 좌우로 붙이면 면적 0 이 생기는 끼울 것 — 바깥 · 칸의 좌우 띠가 빠진다.
            "invalid_outer" => (
                with_heads(&[20], 20, slot(1), &[(20, 0.0, 100.0)]),
                half_chain(SplitDir::LeftRight, 53, 30_000),
                false,
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
                (w, slot(9), false)
            }
            // 메인 마지막 탭을 끈다 — 보이는 탭 없음.
            "no_visible" => {
                let mut w = with_heads(&[SRC], SRC, slot(1), &[(SRC, 0.0, 100.0)]);
                w.is_source = true;
                (w, slot(9), false)
            }
            "no_chrome" => {
                let mut w = snap(&[40], 40, slot(1));
                w.chrome = None;
                (w, slot(9), false)
            }
            "no_canvas" => {
                let heads = [(40, 0.0, 100.0), (41, 100.0, 200.0)];
                let mut w = with_heads(&[40, 41], 40, slot(1), &heads);
                w.canvas = None;
                (w, slot(9), false)
            }
            "blocked" => {
                let mut w = with_heads(&[10, SRC, 11], 10, halves(), &three_heads);
                w.is_source = true;
                (w, slot(9), true)
            }
            // 캔버스 원점 (40, 28) · 탭바 x0 40 — 원점을 빼먹은 거울이 다른 답을 낸다.
            "offset" => {
                let mut c = chrome(&[(50, 44.0, 144.0), (51, 144.0, 244.0)]);
                c.canvas_origin = PointCss { x: 40.0, y: BAR_H };
                c.tab_bar = rect(40.0, 0.0, 640.0, BAR_H);
                let mut w = snap(&[50, 51], 50, halves());
                w.chrome = Some(c);
                (w, slot(9), false)
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
            let got = pick(r, c.point, g.source_view);
            if got != c.expect {
                wrong.push(format!(
                    "{}: {:?} → {got:?} (골든 {:?})",
                    c.name, c.point, c.expect
                ));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[test]
    fn every_golden_region_is_what_regions_for_builds() {
        let g = golden();
        for (name, r) in &g.regions {
            let (w, incoming, blocked) = fixture(name);
            let (built, _) = regions_for(&w, id(SRC), &incoming, blocked).unwrap();
            assert!(
                same_body(r, &built),
                "골든 영역 {name} 이 regions_for 와 다르다 — 지금 값:\n{}",
                serde_json::to_string(&built).unwrap()
            );
            assert!(
                g.cases.iter().any(|c| c.regions == *name),
                "{name} 을 쓰는 사례가 없다"
            );
        }
    }

    // 끄는 탭이 영역에 없어야 `source_view` 를 받지 않는 거울도 같은 답을 낸다.
    #[test]
    fn the_dragged_tab_appears_in_no_golden_region() {
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
    }
}
