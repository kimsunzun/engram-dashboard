//! 커서 밑 판정(TRD S21-slot-docking §2-2 「따라오는 창」 · §5-3 · §6-1) — 세계 + 커서 + 커서 밑 우리 창 → 붙기 ·
//! 띠 · 없음, 그리고 안내 창에 그릴 것. 판정과 그림이 같은 `geometry` 값에서 나온다(칠한 곳 = 붙는 곳 — PRD R6a).
//!
//! 앞서는 차례 = ⓪ 탭바 구역(곧바로 붙기 — 조사 A5 · A7 · 기다림 없음) > 바깥 띠 > 칸 띠(`geometry::choose`). 끄는
//! 창(끄는 탭이 든 창)은 대상이 아니다.

use ts_rs::TS;

use super::geometry::{bands_at, choose, zone_in_canvas, BandHit, WindowTargets};
use super::strip::{self, client_point, strip_zone};
use super::units::{PhysPoint, PhysRect, SizeDip};
use super::world::World;
use crate::layout::chrome::{PointCss, RectCss};
use crate::layout::dock::DockAnchor;
use crate::layout::geometry::PxRect;
use crate::layout::manager::{ViewId, WindowLabel};

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// 그 창 탭바 구역 — 곧바로 그 탭바에 붙는다(R21). 놓을 때는 다시 보지 않는다 — 붙기는 창이 움직일 때만이다
    /// (§6-1).
    Attach(WindowLabel),
    /// 그 창 보이는 탭 `view` 의 띠 — 놓으면 `Seam{target: view, hit.anchor, hit.side, hit.share}`.
    Band {
        label: WindowLabel,
        view: ViewId,
        hit: BandHit,
    },
    /// 우리 창 밖 · 칸 가운데 · 띠를 셀 재료가 없다 — 놓으면 끄는 창이 그대로 남는다(R6 · R22).
    Nothing,
}

/// 안내 창에 그릴 것(`dock:overlay` 몸) — 좌표 = 안내 창 CSS px. 안내 창은 커서 밑 우리 창의 안쪽 사각형
/// (`Overlay::at`)에 같은 배율로 놓이므로 그 창 뷰포트 CSS px 와 같다. 사각형은 반열림 · 서로 겹칠 수 있다(칸
/// 모서리의 두 띠).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct OverlayView {
    /// 커서 밑 칸의 유효 띠 영역.
    pub bands: Vec<RectCss>,
    /// 그 창의 유효 바깥 띠 영역.
    pub outer: Vec<RectCss>,
    /// 고른 띠의 붙을 사각형 — `null` = 고른 띠가 없다.
    pub preview: Option<RectCss>,
}

/// 안내 창을 어디에 무엇으로 — `at` = 그 창 안쪽 사각형(물리).
#[derive(Debug, Clone, PartialEq)]
pub struct Overlay {
    pub over: WindowLabel,
    pub at: PhysRect,
    pub view: OverlayView,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Judgement {
    pub target: Target,
    /// `None` = 안내 창을 치운다(우리 창 밖 · 탭바 구역 · 캔버스 밖 · 칠할 띠가 없다).
    pub overlay: Option<Overlay>,
}

/// 커서 `cursor` 에서의 판정 — `under` = 커서 밑 맨 위 우리 창(끄는 창 · 안내 창을 건너뛴 것 — 실행기의
/// `probe` · 우리 창이 아니면 `None`) · `original` = 끄는 창의 원래 크기(§5-4 · 아직 · 끝내 못 읽었으면 `None` —
/// 그러면 띠를 내지 않는다).
///
/// 그 창의 OS 기하(`WorldWindow::geom`)나 크롬 보고가 없으면 붙기도 띠도 없다. 띠는 커서가 그 창 캔버스 안일 때만
/// 그린다 — 바깥 띠 넷 · 커서 밑 칸의 띠 · 고른 띠의 붙을 사각형(§5-3).
// ADR-0295
pub fn judge(
    world: &World,
    cursor: PhysPoint,
    under: Option<&str>,
    original: Option<SizeDip>,
) -> Judgement {
    let nothing = Judgement {
        target: Target::Nothing,
        overlay: None,
    };
    let Some(w) = under.and_then(|label| world.window(label)) else {
        return nothing;
    };
    let dragging = world
        .dragged
        .as_ref()
        .is_some_and(|d| d.window.as_deref() == Some(w.label.as_str()));
    let (false, Some(geom), Some(chrome)) = (dragging, w.geom, w.chrome.as_ref()) else {
        return nothing;
    };
    let css = client_point(cursor, geom.inner.origin(), geom.scale);
    let origin = chrome.canvas_origin;
    let p = PointCss {
        x: css.x - origin.x,
        y: css.y - origin.y,
    };
    let zone = zone_in_canvas(strip_zone(chrome.tab_bar), origin);
    if strip::contains(&zone, p) {
        return Judgement {
            target: Target::Attach(w.label.clone()),
            overlay: None,
        };
    }
    let (Some(canvas), Some(layout), Some(dragged), Some(original)) = (
        w.canvas,
        w.layout.as_ref(),
        world.dragged.as_ref(),
        original,
    ) else {
        return nothing;
    };
    if !(0.0..f64::from(canvas.w)).contains(&p.x) || !(0.0..f64::from(canvas.h)).contains(&p.y) {
        return nothing;
    }
    let targets = WindowTargets {
        canvas,
        layout,
        incoming: &dragged.layout,
        original,
        zone: Some(zone),
    };
    let mut bands = bands_at(&targets, p);
    let chosen = choose(&bands, p);
    let to_css = |r: &PxRect| RectCss {
        x0: f64::from(r.x0) + origin.x,
        y0: f64::from(r.y0) + origin.y,
        x1: f64::from(r.x1) + origin.x,
        y1: f64::from(r.y1) + origin.y,
    };
    let areas = |outer: bool| -> Vec<RectCss> {
        bands
            .iter()
            .filter(|b| (b.anchor == DockAnchor::Outer) == outer)
            .flat_map(|b| b.areas.iter().map(to_css))
            .collect()
    };
    let view = OverlayView {
        bands: areas(false),
        outer: areas(true),
        preview: chosen.map(|i| to_css(&bands[i].landing)),
    };
    let overlay = (!view.bands.is_empty() || !view.outer.is_empty()).then(|| Overlay {
        over: w.label.clone(),
        at: geom.inner,
        view,
    });
    let target = match chosen {
        Some(i) => Target::Band {
            label: w.label.clone(),
            view: w.active,
            hit: bands.swap_remove(i),
        },
        None => Target::Nothing,
    };
    Judgement { target, overlay }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock::units::WindowGeom;
    use crate::dock::world::{DraggedTab, WorldWindow};
    use crate::layout::chrome::DockChrome;
    use crate::layout::dock::DockSide;
    use crate::layout::manager::WindowAttrs;
    use crate::layout::{CanvasPx, LayoutNode, SlotContent, SplitDir};
    use uuid::Uuid;

    const MAIN: &str = "main";
    const DRAGGING: &str = "slot-popup-1";
    const MAIN_ACTIVE: u128 = 70;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn slot(n: u128) -> LayoutNode {
        LayoutNode::Slot {
            id: id(n),
            content: SlotContent::Empty,
        }
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> RectCss {
        RectCss { x0, y0, x1, y1 }
    }

    fn geom(origin: PhysPoint, scale: f64) -> WindowGeom {
        let inner = PhysRect::at(
            origin,
            SizeDip {
                w: 1200.0,
                h: 828.0,
            }
            .to_phys(scale),
        );
        WindowGeom {
            outer: PhysRect {
                x0: inner.x0 - 8,
                y0: inner.y0 - 31,
                x1: inner.x1 + 8,
                y1: inner.y1 + 8,
            },
            inner,
            scale,
        }
    }

    // 메인 = 좌우 반분(칸 1 · 2) · 캔버스 1200 × 800 · 캔버스 원점 `canvas_origin` · 끌기 영역 = 뷰포트 (0,0)–(600,28) ·
    // 안쪽 원점 `origin` · 배율 `scale`. 끄는 창 = 탭 하나(칸 9) 팝아웃.
    fn world_at(origin: PhysPoint, scale: f64, canvas_origin: PointCss) -> World {
        let window =
            |label: &str, active: u128, layout: LayoutNode, geom: Option<WindowGeom>| WorldWindow {
                label: label.to_owned(),
                tabs: vec![id(active)],
                active: id(active),
                layout: Some(layout),
                canvas: Some(CanvasPx { w: 1200, h: 800 }),
                chrome: Some(DockChrome {
                    canvas_origin,
                    tab_bar: rect(0.0, 0.0, 600.0, 28.0),
                }),
                attrs: WindowAttrs::default(),
                geom,
            };
        let main_layout = LayoutNode::Split {
            id: id(100),
            dir: SplitDir::LeftRight,
            ratio: 0.5,
            a: Box::new(slot(1)),
            b: Box::new(slot(2)),
        };
        World {
            windows: vec![
                window(MAIN, MAIN_ACTIVE, main_layout, Some(geom(origin, scale))),
                window(
                    DRAGGING,
                    90,
                    slot(9),
                    Some(geom(PhysPoint { x: 5000, y: 0 }, 1.0)),
                ),
            ],
            lease: None,
            dragged: Some(DraggedTab {
                view: id(90),
                window: Some(DRAGGING.to_owned()),
                layout: slot(9),
            }),
            monitors: Vec::new(),
            parked: Default::default(),
        }
    }

    fn world() -> World {
        world_at(
            PhysPoint { x: 100, y: 50 },
            1.0,
            PointCss { x: 0.0, y: 28.0 },
        )
    }

    const ORIGINAL: Option<SizeDip> = Some(SizeDip { w: 400.0, h: 300.0 });

    // 배율 1 · 안쪽 원점 (100, 50) 창의 뷰포트 CSS 점을 짚는 물리 커서.
    fn at(x: i32, y: i32) -> PhysPoint {
        PhysPoint {
            x: 100 + x,
            y: 50 + y,
        }
    }

    fn band_of(j: &Judgement) -> Option<(DockAnchor, DockSide)> {
        match &j.target {
            Target::Band { hit, .. } => Some((hit.anchor, hit.side)),
            _ => None,
        }
    }

    #[test]
    fn no_window_of_ours_under_the_cursor_is_nothing() {
        let w = world();
        for under in [None, Some("nope")] {
            let j = judge(&w, at(300, 400), under, ORIGINAL);
            assert_eq!(
                j,
                Judgement {
                    target: Target::Nothing,
                    overlay: None
                },
                "{under:?}"
            );
        }
    }

    #[test]
    fn the_window_holding_the_dragged_tab_is_never_a_target() {
        let w = world();
        let j = judge(&w, PhysPoint { x: 5010, y: 10 }, Some(DRAGGING), ORIGINAL);
        assert_eq!(j.target, Target::Nothing);
        assert_eq!(j.overlay, None);
    }

    #[test]
    fn without_its_os_geometry_or_chrome_a_window_is_nothing() {
        let mut w = world();
        w.window_mut(MAIN).unwrap().geom = None;
        assert_eq!(
            judge(&w, at(10, 10), Some(MAIN), ORIGINAL).target,
            Target::Nothing
        );
        let mut w = world();
        w.window_mut(MAIN).unwrap().chrome = None;
        assert_eq!(
            judge(&w, at(10, 10), Some(MAIN), ORIGINAL).target,
            Target::Nothing
        );
    }

    #[test]
    fn the_strip_zone_attaches_at_once_even_over_the_canvas() {
        let w = world();
        for (x, y, attach, why) in [
            (300, 10, true, "탭바 위"),
            (300, -15, true, "탭바 위 15"),
            (300, 42, true, "탭바 아래 14 — 캔버스 위인데도 붙기(R6a ⓪)"),
            (300, 43, false, "탭바 아래 15 = 바깥 위 띠"),
            (600, 10, false, "끌기 영역 오른쪽 변(`+` 자리)"),
        ] {
            let j = judge(&w, at(x, y), Some(MAIN), ORIGINAL);
            assert_eq!(j.target == Target::Attach(MAIN.to_owned()), attach, "{why}");
            if attach {
                assert_eq!(j.overlay, None, "{why}: 붙으면 안내를 치운다");
            }
        }
        assert_eq!(
            judge(&w, at(300, 10), Some(MAIN), None).target,
            Target::Attach(MAIN.to_owned()),
            "붙기는 원래 크기가 없어도 된다"
        );
    }

    #[test]
    fn a_fractional_strip_edge_is_one_boundary_for_attach_band_and_paint() {
        // 배율 125% · 탭바 끝 28.125(구역 끝 43.125) · 캔버스 원점 28 — 커서 물리 y 54 = CSS 43.2 는 정확한 구역 밖이지만
        // 넓힌 경계(캔버스 16) 안이라 붙기다. 그 아래 줄부터 바깥 위 띠 — 붙기도 띠도 아닌 줄이 없다.
        let mut w = world_at(PhysPoint { x: 0, y: 0 }, 1.25, PointCss { x: 0.0, y: 28.0 });
        w.window_mut(MAIN)
            .unwrap()
            .chrome
            .as_mut()
            .unwrap()
            .tab_bar
            .y1 = 28.125;
        let cursor = |y: i32| PhysPoint { x: 375, y };
        assert_eq!(
            judge(&w, cursor(54), Some(MAIN), ORIGINAL).target,
            Target::Attach(MAIN.to_owned())
        );
        assert_eq!(
            band_of(&judge(&w, cursor(55), Some(MAIN), ORIGINAL)),
            Some((DockAnchor::Outer, DockSide::Top)),
            "CSS 44 = 캔버스 16"
        );
        // 캔버스 위 띠 끝(20 = CSS 48 = 물리 60)까지 줄마다 붙기 또는 칠해진 바깥 위 띠다.
        for y in 0..60 {
            let j = judge(&w, cursor(y), Some(MAIN), ORIGINAL);
            match (&j.target, &j.overlay) {
                (Target::Attach(_), None) => {}
                (Target::Band { hit, .. }, Some(o)) => {
                    assert_eq!(
                        (hit.anchor, hit.side),
                        (DockAnchor::Outer, DockSide::Top),
                        "y {y}"
                    );
                    let css = PointCss {
                        x: 300.0,
                        y: f64::from(y) / 1.25,
                    };
                    assert!(
                        o.view.outer.iter().any(|r| strip::contains(r, css)),
                        "y {y}: 칠해져 있다"
                    );
                }
                other => panic!("y {y}: {other:?}"),
            }
        }
    }

    #[test]
    fn a_band_targets_the_visible_tab_with_the_share_its_preview_used() {
        let w = world();
        let j = judge(&w, at(10, 428), Some(MAIN), ORIGINAL);
        let Target::Band { label, view, hit } = &j.target else {
            panic!("띠여야: {j:?}")
        };
        assert_eq!((label.as_str(), *view), (MAIN, id(MAIN_ACTIVE)));
        assert_eq!((hit.anchor, hit.side), (DockAnchor::Outer, DockSide::Left));
        assert_eq!(hit.share, 400.0 / 1200.0);
        let overlay = j.overlay.expect("안내");
        assert_eq!(overlay.over, MAIN);
        assert_eq!(overlay.at, w.window(MAIN).unwrap().geom.unwrap().inner);
        assert_eq!(
            overlay.view.preview,
            Some(rect(0.0, 28.0, 400.0, 828.0)),
            "붙을 사각형 = 캔버스 (0,0)–(400,800) · 뷰포트로 옮김"
        );
    }

    #[test]
    fn over_a_slot_centre_the_bands_are_painted_but_nothing_is_chosen() {
        let w = world();
        let j = judge(&w, at(300, 428), Some(MAIN), ORIGINAL);
        assert_eq!(j.target, Target::Nothing);
        let view = j.overlay.expect("띠는 칠한다(R3)").view;
        assert_eq!(view.preview, None);
        assert!(
            view.outer.contains(&rect(1180.0, 28.0, 1200.0, 828.0)),
            "{:?}",
            view.outer
        );
        // 칠한 칸 띠는 커서 밑 칸 1 의 것뿐이다.
        assert!(!view.bands.is_empty());
        assert!(view.bands.iter().all(|r| r.x1 <= 600.0), "{:?}", view.bands);
        assert!(
            view.bands.contains(&rect(20.0, 48.0, 80.0, 808.0)),
            "칸 1 왼쪽 띠: {:?}",
            view.bands
        );
    }

    #[test]
    fn the_overlay_paints_exactly_what_is_hit() {
        // 커서를 캔버스에 훑으며 — 고른 띠가 있으면 그 칸 · 바깥 영역이 칠해져 있고 미리보기가 있다.
        let w = world();
        for y in (28..828).step_by(13) {
            for x in (0..1200).step_by(13) {
                let j = judge(&w, at(x, y), Some(MAIN), ORIGINAL);
                let p = (f64::from(x), f64::from(y));
                let painted = |rs: &[RectCss]| {
                    rs.iter()
                        .any(|r| strip::contains(r, PointCss { x: p.0, y: p.1 }))
                };
                match (&j.target, &j.overlay) {
                    (Target::Band { hit, .. }, Some(o)) => {
                        let rs = if hit.anchor == DockAnchor::Outer {
                            &o.view.outer
                        } else {
                            &o.view.bands
                        };
                        assert!(painted(rs), "{p:?}");
                        assert!(o.view.preview.is_some());
                    }
                    (Target::Nothing, Some(o)) => {
                        assert!(
                            !painted(&o.view.outer) && !painted(&o.view.bands),
                            "{p:?}: 칠했는데 안 골림"
                        );
                        assert_eq!(o.view.preview, None);
                    }
                    (Target::Attach(_), None) => assert!(x < 600 && y < 43, "{p:?}"),
                    other => panic!("{p:?}: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn without_the_original_size_there_are_no_bands() {
        let w = world();
        let j = judge(&w, at(10, 428), Some(MAIN), None);
        assert_eq!(
            j,
            Judgement {
                target: Target::Nothing,
                overlay: None
            }
        );
    }

    #[test]
    fn outside_the_canvas_there_is_nothing_to_paint() {
        let w = world();
        for (x, y) in [(900, 10), (1200, 400), (300, 828)] {
            let j = judge(&w, at(x, y), Some(MAIN), ORIGINAL);
            assert_eq!(
                j,
                Judgement {
                    target: Target::Nothing,
                    overlay: None
                },
                "{x},{y}"
            );
        }
    }

    #[test]
    fn the_cursor_reads_through_the_window_origin_and_scale() {
        // 같은 뷰포트 CSS 점 = 같은 판정 — 배율 1 · 1.25 · 2, 왼쪽 모니터의 창 포함.
        for (origin, scale) in [
            (PhysPoint { x: 100, y: 50 }, 1.0),
            (PhysPoint { x: 100, y: 50 }, 1.25),
            (PhysPoint { x: -2500, y: -40 }, 2.0),
        ] {
            let w = world_at(origin, scale, PointCss { x: 0.0, y: 28.0 });
            let phys = |x: f64, y: f64| PhysPoint {
                x: origin.x + (x * scale).round() as i32,
                y: origin.y + (y * scale).round() as i32,
            };
            let ctx = format!("배율 {scale}");
            assert_eq!(
                judge(&w, phys(300.0, 10.0), Some(MAIN), ORIGINAL).target,
                Target::Attach(MAIN.to_owned()),
                "{ctx}"
            );
            assert_eq!(
                band_of(&judge(&w, phys(10.0, 428.0), Some(MAIN), ORIGINAL)),
                Some((DockAnchor::Outer, DockSide::Left)),
                "{ctx}"
            );
            assert_eq!(
                band_of(&judge(&w, phys(900.0, 92.0), Some(MAIN), ORIGINAL)),
                Some((DockAnchor::Slot(id(2)), DockSide::Top)),
                "{ctx}"
            );
            assert_eq!(
                judge(&w, phys(300.0, 428.0), Some(MAIN), ORIGINAL).target,
                Target::Nothing,
                "{ctx}"
            );
        }
    }

    #[test]
    fn an_offset_canvas_measures_the_bands_from_its_own_origin() {
        // 캔버스가 뷰포트 x 40 에서 시작한다 — 바깥 왼쪽 띠 = 뷰포트 x 40..72.
        let w = world_at(
            PhysPoint { x: 100, y: 50 },
            1.0,
            PointCss { x: 40.0, y: 28.0 },
        );
        assert_eq!(
            band_of(&judge(&w, at(40, 428), Some(MAIN), ORIGINAL)),
            Some((DockAnchor::Outer, DockSide::Left))
        );
        assert_eq!(
            band_of(&judge(&w, at(72, 428), Some(MAIN), ORIGINAL)),
            Some((DockAnchor::Slot(id(1)), DockSide::Left)),
            "바깥 띠 바로 오른쪽"
        );
        assert_eq!(
            judge(&w, at(39, 428), Some(MAIN), ORIGINAL).overlay,
            None,
            "캔버스 왼쪽 밖"
        );
        let j = judge(&w, at(40, 428), Some(MAIN), ORIGINAL);
        assert_eq!(
            j.overlay.unwrap().view.preview,
            Some(rect(40.0, 28.0, 440.0, 828.0))
        );
    }
}
