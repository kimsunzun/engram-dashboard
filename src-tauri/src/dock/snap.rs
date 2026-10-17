//! 끌기 스냅숏 뜨기(TRD S21-slot-docking §1-3 「스냅숏 번호」 · 「시작」 2 · §2-4) — 모델 값으로 장부의 재료
//! ([`Snap`] · [`StartMaterial`])를 만든다. 실행기가 레이아웃 락을 쥔 채, 스냅숏 번호를 다는 같은 구간 안에서
//! 부른다(번호를 다는 문 = `DockState::snapshot` · `DockState::start_material`). 창 게터 · emit · 락 0 이다.
//!
//! ★끄는 탭은 모델 표식([`ViewManager::lease`])에서 읽는다★ — 장부나 락 밖에 둔 값으로 뜨면 장부가 들고 있다
//! 이어 먹는 세션 전 스냅숏이 다른 끌기의 탭으로 떠질 수 있다(그 결과 = [`Snap`] 문서). 그래서 창마다의
//! `visible` 은 늘 그 표식의 탭으로 [`visible_tab`] 이 고른 것이다 — 어긋나면 debug 빌드의 `regions::regions_for`
//! 가 멈춘다.

use super::book::Snap;
use super::regions::{visible_tab, WindowSnap};
use crate::layout::manager::{LeaseId, ViewId, WindowLabel, WindowTabs, MAIN_WINDOW_LABEL};
use crate::layout::{LayoutNode, ViewManager};

/// `dock_drag_start` 가 `lease_tab` 과 같은 레이아웃 락 구간에서 뜨는 시작 재료 — [`super::book::Input::Start`]
/// 의 칸 그대로다. 표식 · 끄는 탭도 모델 표식에서 읽는다(시작 스냅숏과 같은 탭이도록).
#[derive(Debug, Clone, PartialEq)]
pub struct StartMaterial {
    pub lease: LeaseId,
    pub view: ViewId,
    pub source_label: WindowLabel,
    /// 끄는 탭의 지금 배치 — 놓기 사전조건(`DockGuard::expect_source`)의 기준.
    pub expect_source: LayoutNode,
    /// 출처가 탭 하나뿐인 팝아웃 — main 은 탭이 하나여도 아니다(메인은 빈 화면으로 남는다).
    pub single_tab_popout: bool,
    pub snap: Snap,
}

/// 모든 창의 영역 재료와 끄는 탭의 지금 배치 — 끄는 탭 = 모델 표식의 탭. 표식이 없으면(도킹 성공이 내렸다 ·
/// 그 탭이 지워졌다) `source_layout` 이 `None` 이고 창마다 모델 활성을 보이는 탭으로 싣는다. 창은 label 순이다.
pub fn snapshot(mgr: &ViewManager, number: u64) -> Snap {
    let dragged = mgr.lease().map(|(_, view)| view);
    let source_layout = dragged
        .and_then(|view| mgr.views.get(&view))
        .map(|view| view.layout.clone());
    let mut windows: Vec<WindowSnap> = mgr
        .windows
        .iter()
        .filter_map(|(label, wt)| window_snap(mgr, label, wt, dragged))
        .collect();
    // 표 순회 순서(HashMap)가 할 일 순서로 새지 않게 — 같은 모델이면 같은 할 일 열이다.
    windows.sort_by(|a, b| a.label.cmp(&b.label));
    Snap {
        number,
        source_layout,
        windows,
    }
}

/// 시작 재료 — 표식을 세운 뒤 같은 레이아웃 락 구간에서 부른다. `None` = 표식이 없거나 그 탭이 어느 창의 탭도
/// 아니다(`lease_tab` 이 성공한 같은 구간이면 오지 않는다).
pub fn start_material(mgr: &ViewManager, number: u64) -> Option<StartMaterial> {
    let (lease, view) = mgr.lease()?;
    let source_label = mgr.owner_of(view)?.clone();
    let source = mgr.windows.get(&source_label)?;
    let expect_source = mgr.views.get(&view)?.layout.clone();
    let single_tab_popout = source_label != MAIN_WINDOW_LABEL && source.tabs.len() == 1;
    Some(StartMaterial {
        lease,
        view,
        source_label,
        expect_source,
        single_tab_popout,
        snap: snapshot(mgr, number),
    })
}

fn window_snap(
    mgr: &ViewManager,
    label: &WindowLabel,
    wt: &WindowTabs,
    dragged: Option<ViewId>,
) -> Option<WindowSnap> {
    let shown = match dragged {
        Some(source) => visible_tab(&wt.tabs, wt.active, source),
        None => Some(wt.active),
    };
    let visible = match shown {
        Some(view) => {
            let Some(v) = mgr.views.get(&view) else {
                // 탭 목록이 모델에 없는 View 를 가리킨다(소유 불변식이 깨졌다) — `visible` 을 비워 실으면 `regions_for`
                // 대조가 debug 빌드를 멈추므로 그 창을 빼고, 장부는 닫힌 창으로 다룬다.
                tracing::warn!(
                    label = %label,
                    view = %view,
                    "보이는 탭의 View 가 모델에 없다 — 그 창의 영역 재료를 뺀다"
                );
                return None;
            };
            Some((view, v.layout.clone()))
        }
        None => None,
    };
    Some(WindowSnap {
        label: label.clone(),
        tabs: wt.tabs.clone(),
        active: wt.active,
        visible,
        canvas: wt.canvas,
        chrome: wt.chrome.clone(),
        hidden: false,
        is_source: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock::regions::regions_for;
    use crate::layout::chrome::{DockChrome, PointCss, RectCss};
    use crate::layout::CanvasPx;

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

    fn main_tabs(mgr: &ViewManager) -> Vec<ViewId> {
        mgr.windows[MAIN_WINDOW_LABEL].tabs.clone()
    }

    fn win<'a>(snap: &'a Snap, label: &str) -> &'a WindowSnap {
        snap.windows
            .iter()
            .find(|w| w.label == label)
            .unwrap_or_else(|| panic!("스냅숏에 {label} 이 없다"))
    }

    fn shown(snap: &Snap, label: &str) -> Option<ViewId> {
        win(snap, label).visible.as_ref().map(|(view, _)| *view)
    }

    // 모든 창이 장부의 대조 규칙과 같은 탭 · 그 탭의 배치를 싣고, 장부가 덮을 두 칸은 비어 있다. 그리고 장부가 그
    // 스냅숏으로 영역을 만들 때 debug 대조가 멈추지 않는다.
    fn assert_consistent(mgr: &ViewManager, snap: &Snap) {
        let dragged = mgr.lease().map(|(_, view)| view);
        assert_eq!(
            snap.windows.len(),
            mgr.windows.len(),
            "창을 빠뜨리지 않는다"
        );
        for w in &snap.windows {
            let wt = &mgr.windows[&w.label];
            assert_eq!((&w.tabs, w.active), (&wt.tabs, wt.active), "{}", w.label);
            let expected = match dragged {
                Some(source) => visible_tab(&wt.tabs, wt.active, source),
                None => Some(wt.active),
            };
            assert_eq!(
                w.visible.as_ref().map(|(view, _)| *view),
                expected,
                "{}",
                w.label
            );
            if let Some((view, layout)) = &w.visible {
                assert_eq!(layout, &mgr.views[view].layout, "{}", w.label);
            }
            assert!(!w.hidden && !w.is_source, "{}: 장부 몫", w.label);
            if let (Some(source), Some(incoming)) = (dragged, &snap.source_layout) {
                regions_for(w, source, incoming, false, None);
            }
        }
    }

    #[test]
    fn an_active_dragged_tab_shows_its_successor_and_other_windows_their_active() {
        let mut mgr = ViewManager::new();
        mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let [_, b, c] = main_tabs(&mgr)[..] else {
            panic!("탭 셋")
        };
        mgr.switch_tab(MAIN_WINDOW_LABEL, b).unwrap();
        let p = mgr.create_window(P1).unwrap();
        mgr.lease_tab(b).unwrap();

        let snap = snapshot(&mgr, 7);

        assert_eq!(snap.number, 7);
        assert_eq!(snap.source_layout.as_ref(), Some(&mgr.views[&b].layout));
        assert_eq!(
            shown(&snap, MAIN_WINDOW_LABEL),
            Some(c),
            "오른쪽 이웃이 잇는다"
        );
        assert_eq!(shown(&snap, P1), Some(p));
        assert_consistent(&mgr, &snap);

        // 오른쪽 끝이면 왼쪽 이웃.
        mgr.release_lease(mgr.lease().unwrap().0);
        mgr.switch_tab(MAIN_WINDOW_LABEL, c).unwrap();
        mgr.lease_tab(c).unwrap();
        let snap = snapshot(&mgr, 8);
        assert_eq!(shown(&snap, MAIN_WINDOW_LABEL), Some(b));
        assert_consistent(&mgr, &snap);
    }

    #[test]
    fn a_dragged_tab_that_is_not_active_leaves_the_active_shown() {
        let mut mgr = ViewManager::new();
        let a = mgr.windows[MAIN_WINDOW_LABEL].active;
        let b = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        mgr.lease_tab(a).unwrap();

        let snap = snapshot(&mgr, 1);

        assert_eq!(shown(&snap, MAIN_WINDOW_LABEL), Some(b));
        assert_eq!(win(&snap, MAIN_WINDOW_LABEL).active, b);
        assert_consistent(&mgr, &snap);
    }

    #[test]
    fn the_main_last_tab_and_a_single_tab_popout_show_nothing() {
        let mut mgr = ViewManager::new();
        let only = mgr.windows[MAIN_WINDOW_LABEL].active;
        mgr.create_window(P1).unwrap();
        mgr.lease_tab(only).unwrap();
        let snap = snapshot(&mgr, 1);
        assert_eq!(shown(&snap, MAIN_WINDOW_LABEL), None, "메인은 빈 화면");
        assert_eq!(
            win(&snap, MAIN_WINDOW_LABEL).active,
            only,
            "모델 활성은 끄는 탭 그대로"
        );
        assert_consistent(&mgr, &snap);

        let mut mgr = ViewManager::new();
        let lone = mgr.create_window(P1).unwrap();
        mgr.lease_tab(lone).unwrap();
        let snap = snapshot(&mgr, 1);
        assert_eq!(shown(&snap, P1), None);
        assert_consistent(&mgr, &snap);
    }

    #[test]
    fn without_a_lease_there_is_no_source_layout_and_every_window_shows_its_active() {
        let mut mgr = ViewManager::new();
        let a = mgr.windows[MAIN_WINDOW_LABEL].active;
        mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        mgr.switch_tab(MAIN_WINDOW_LABEL, a).unwrap();

        let snap = snapshot(&mgr, 1);

        assert_eq!(snap.source_layout, None);
        assert_eq!(shown(&snap, MAIN_WINDOW_LABEL), Some(a));
        assert_consistent(&mgr, &snap);
        assert_eq!(start_material(&mgr, 2), None);
    }

    #[test]
    fn removing_the_dragged_tab_leaves_no_source_layout() {
        let mut mgr = ViewManager::new();
        mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let [a, b] = main_tabs(&mgr)[..] else {
            panic!("탭 둘")
        };
        mgr.lease_tab(b).unwrap();
        mgr.close_tab(MAIN_WINDOW_LABEL, b).unwrap();

        let snap = snapshot(&mgr, 1);

        assert_eq!(
            snap.source_layout, None,
            "그 탭이 사라졌다 — 장부가 SourceGone 으로 끝낸다"
        );
        assert_eq!(shown(&snap, MAIN_WINDOW_LABEL), Some(a));
        assert_consistent(&mgr, &snap);
    }

    #[test]
    fn measurements_are_carried_per_window_and_absent_until_reported() {
        let mut mgr = ViewManager::new();
        mgr.create_window(P1).unwrap();
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1200, 800).unwrap();
        mgr.set_dock_chrome(MAIN_WINDOW_LABEL, chrome()).unwrap();
        mgr.set_dock_chrome(P1, chrome()).unwrap();

        let snap = snapshot(&mgr, 1);

        let main = win(&snap, MAIN_WINDOW_LABEL);
        assert_eq!(main.canvas, Some(CanvasPx { w: 1200, h: 800 }));
        assert_eq!(main.chrome, Some(chrome()));
        let p = win(&snap, P1);
        assert_eq!((p.canvas, p.chrome.clone()), (None, Some(chrome())));
    }

    #[test]
    fn windows_come_in_label_order() {
        let mut mgr = ViewManager::new();
        mgr.create_window(P2).unwrap();
        mgr.create_window(P1).unwrap();
        let labels: Vec<_> = snapshot(&mgr, 1)
            .windows
            .into_iter()
            .map(|w| w.label)
            .collect();
        assert_eq!(labels, [MAIN_WINDOW_LABEL, P1, P2]);
    }

    #[test]
    fn start_material_reads_the_lease_its_window_and_the_layout() {
        let mut mgr = ViewManager::new();
        let m = mgr.windows[MAIN_WINDOW_LABEL].active;
        let p = mgr.create_window(P1).unwrap();
        let lease = mgr.lease_tab(p).unwrap();

        let got = start_material(&mgr, 5).unwrap();

        assert_eq!((got.lease, got.view), (lease, p));
        assert_eq!(got.source_label, P1);
        assert_eq!(got.expect_source, mgr.views[&p].layout);
        assert!(got.single_tab_popout);
        assert_eq!(got.snap, snapshot(&mgr, 5));
        assert_eq!(got.snap.source_layout.as_ref(), Some(&got.expect_source));

        // 탭이 둘인 팝아웃 · 탭 하나인 main 은 숨기지 않는다.
        mgr.create_tab(P1, None).unwrap();
        assert!(!start_material(&mgr, 6).unwrap().single_tab_popout);
        mgr.release_lease(lease);
        mgr.lease_tab(m).unwrap();
        let got = start_material(&mgr, 7).unwrap();
        assert_eq!(got.source_label, MAIN_WINDOW_LABEL);
        assert!(!got.single_tab_popout);
    }
}
