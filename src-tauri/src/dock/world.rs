//! 끌기 세션이 보는 세계(TRD S21-slot-docking §2-3 「세계」) — 레이아웃 락 안에서 뜬 모델 값(창별 탭 · 활성 · 보이는
//! 탭 배치 · 캔버스 · 크롬 · 창 속성 · 표식과 그 탭의 배치) · 대기 창 명부(역할 · 재기 — §2-5)와 실행기가 메인
//! 스레드에서 채우는 OS 기하(창마다 · 모니터 목록). 판정(`targets`)과 세션의 입력이다.
//!
//! ★OS 기하는 [`snapshot`] 이 채우지 않는다★ — 레이아웃 락을 쥔 채 OS 를 부르지 않게, 실행기가 락을 놓은 뒤
//! `WorldWindow::geom` · `World::monitors` 를 채운다. 비어 있으면 그 창은 띠도 붙기도 없다(판정 = `targets`).

use super::parked::Parked;
use super::units::{Monitor, WindowGeom};
use crate::layout::chrome::DockChrome;
use crate::layout::manager::{LeaseId, ViewId, WindowAttrs, WindowLabel};
use crate::layout::{CanvasPx, LayoutNode, ViewManager};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct World {
    /// 모델의 창 전부 — label 순(표 순회 순서가 판정 · 할 일 순서로 새지 않게).
    pub windows: Vec<WorldWindow>,
    pub lease: Option<LeaseId>,
    /// 표식이 가리키는 끄는 탭 — `None` = 표식이 없거나 그 탭이 모델에 없다.
    pub dragged: Option<DraggedTab>,
    /// 실행기가 채운다 — 비어 있으면 「가장 가까운 모니터」가 없다.
    pub monitors: Vec<Monitor>,
    /// 대기 창 명부 — 모델 밖 창이라 `windows` 에 없다. 떼기의 「예비 창 준비됨」과 들일 때 싣는 재기를 여기서 본다.
    pub parked: Parked,
}

/// 끄는 탭 — 끄는 동안 모델이 그때그때 바뀌므로(Chrome 꼴 · PRD R2) 그 탭은 늘 어느 한 창에 있다.
#[derive(Debug, Clone, PartialEq)]
pub struct DraggedTab {
    pub view: ViewId,
    /// 그 탭이 지금 든 창 — 끄는 창이다(판정 대상에서 빠진다). `None` = 소유 불변식이 깨졌다.
    pub window: Option<WindowLabel>,
    pub layout: LayoutNode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorldWindow {
    pub label: WindowLabel,
    pub tabs: Vec<ViewId>,
    pub active: ViewId,
    /// 보이는 탭(= 활성 탭)의 배치 — 끄는 탭은 늘 끄는 창에 있으므로 다른 창의 보이는 탭은 그 창 활성이다. `None` =
    /// 활성 탭의 View 가 모델에 없다(소유 불변식이 깨졌다 — 그 창은 띠가 없다).
    pub layout: Option<LayoutNode>,
    pub canvas: Option<CanvasPx>,
    pub chrome: Option<DockChrome>,
    pub attrs: WindowAttrs,
    /// 실행기가 채운다 — `None` = 아직 안 읽었다 · 못 읽었다(사라짐 · 숨김 · 최소화).
    pub geom: Option<WindowGeom>,
}

impl WorldWindow {
    /// 활성 탭의 자리 — 붙기 전 활성 자리(C8) · `Pending::Attach` 의 재료.
    pub fn active_index(&self) -> Option<usize> {
        self.tabs.iter().position(|t| *t == self.active)
    }
}

impl World {
    pub fn window(&self, label: &str) -> Option<&WorldWindow> {
        self.windows.iter().find(|w| w.label == label)
    }

    pub fn window_mut(&mut self, label: &str) -> Option<&mut WorldWindow> {
        self.windows.iter_mut().find(|w| w.label == label)
    }
}

/// 레이아웃 락을 쥔 채 모델 값과 명부를 뜬다 — OS 기하 · 모니터는 비어 있다(모듈 머리).
pub fn snapshot(mgr: &ViewManager, parked: &Parked) -> World {
    let lease = mgr.lease();
    let dragged = lease.and_then(|(_, view)| {
        Some(DraggedTab {
            view,
            window: mgr.owner_of(view).cloned(),
            layout: mgr.views.get(&view)?.layout.clone(),
        })
    });
    let mut windows: Vec<WorldWindow> = mgr
        .windows
        .iter()
        .map(|(label, wt)| WorldWindow {
            label: label.clone(),
            tabs: wt.tabs.clone(),
            active: wt.active,
            layout: mgr.views.get(&wt.active).map(|v| v.layout.clone()),
            canvas: wt.canvas,
            chrome: wt.chrome.clone(),
            attrs: wt.attrs,
            geom: None,
        })
        .collect();
    windows.sort_by(|a, b| a.label.cmp(&b.label));
    World {
        windows,
        lease: lease.map(|(id, _)| id),
        dragged,
        monitors: Vec::new(),
        parked: parked.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock::parked::ParkedRole;
    use crate::layout::chrome::{PointCss, RectCss};
    use crate::layout::manager::FrameMeasures;
    use crate::layout::MAIN_WINDOW_LABEL;

    const P1: &str = "slot-popup-1";
    const P2: &str = "slot-popup-2";

    fn chrome() -> DockChrome {
        DockChrome {
            canvas_origin: PointCss { x: 0.0, y: 28.0 },
            tab_bar: RectCss {
                x0: 0.0,
                y0: 0.0,
                x1: 640.0,
                y1: 28.0,
            },
            tabs: Vec::new(),
            scroll: None,
        }
    }

    #[test]
    fn every_window_is_taken_in_label_order_with_its_active_tabs_layout() {
        let mut mgr = ViewManager::new();
        let second = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let p2 = mgr.create_window(P2).unwrap();
        let p1 = mgr.create_window(P1).unwrap();
        mgr.set_window_canvas(P1, 800, 600).unwrap();
        mgr.set_dock_chrome(P1, chrome()).unwrap();

        let world = snapshot(&mgr, &Parked::default());

        let labels: Vec<&str> = world.windows.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, [MAIN_WINDOW_LABEL, P1, P2]);
        let main = world.window(MAIN_WINDOW_LABEL).unwrap();
        assert_eq!(main.tabs, mgr.windows[MAIN_WINDOW_LABEL].tabs);
        assert_eq!(main.active, second);
        assert_eq!(main.active_index(), Some(1));
        assert_eq!(main.layout.as_ref(), Some(&mgr.views[&second].layout));
        let w1 = world.window(P1).unwrap();
        assert_eq!(w1.active, p1);
        assert_eq!(w1.canvas, Some(CanvasPx { w: 800, h: 600 }));
        assert_eq!(w1.chrome, Some(chrome()));
        assert_eq!(world.window(P2).unwrap().active, p2);
        assert!(
            world.windows.iter().all(|w| w.geom.is_none()),
            "OS 기하는 실행기 몫"
        );
        assert!(world.monitors.is_empty());
        assert_eq!((world.lease, &world.dragged), (None, &None));
        assert_eq!(world.parked, Parked::default());
    }

    #[test]
    fn the_registry_rides_beside_the_model_windows() {
        let mgr = ViewManager::new();
        let mut parked = Parked::default();
        let measures = FrameMeasures {
            canvas: Some(CanvasPx { w: 800, h: 600 }),
            metrics: None,
            chrome: Some(chrome()),
        };
        parked.park(P1.to_owned(), ParkedRole::Spare, measures.clone());

        let world = snapshot(&mgr, &parked);

        assert_eq!(world.parked, parked);
        assert!(world.window(P1).is_none(), "명부의 창은 모델 창이 아니다");
        assert_eq!(world.parked.get(P1).map(|e| &e.measures), Some(&measures));
    }

    #[test]
    fn the_leased_tab_is_the_dragged_tab_with_its_window() {
        let mut mgr = ViewManager::new();
        mgr.create_window(P1).unwrap();
        let tab = mgr.create_tab(P1, None).unwrap();
        let lease = mgr.lease_tab(tab).unwrap();

        let world = snapshot(&mgr, &Parked::default());

        assert_eq!(world.lease, Some(lease));
        assert_eq!(
            world.dragged,
            Some(DraggedTab {
                view: tab,
                window: Some(P1.to_owned()),
                layout: mgr.views[&tab].layout.clone(),
            })
        );
    }

    #[test]
    fn the_executor_fills_the_os_geometry_by_label() {
        let mgr = ViewManager::new();
        let mut world = snapshot(&mgr, &Parked::default());
        let geom = WindowGeom {
            outer: crate::dock::units::PhysRect {
                x0: 0,
                y0: 0,
                x1: 800,
                y1: 600,
            },
            inner: crate::dock::units::PhysRect {
                x0: 8,
                y0: 31,
                x1: 792,
                y1: 592,
            },
            scale: 1.0,
        };
        world.window_mut(MAIN_WINDOW_LABEL).unwrap().geom = Some(geom);
        assert_eq!(world.window(MAIN_WINDOW_LABEL).unwrap().geom, Some(geom));
        assert!(world.window_mut("nope").is_none());
    }
}
