//! 세션 시험 — 가짜 포트가 실제 `ViewManager` · 대기 창 명부를 쥐고(쓰기는 모델에 그대로 반영된다) 부른 차례와
//! 인자를 적는다. OS 기하 · 커서 · 커서 밑 창은 시험이 정한다.

use std::collections::HashMap;
use std::sync::mpsc;

use super::*;
use crate::dock::parked::{Measure, Parked, ParkedRole};
use crate::dock::ports::{DetachPlan, DockWrite, Placement, Ports, SessionDock, WholePlan};
use crate::dock::targets::{self, Overlay};
use crate::dock::units::{Monitor, PhysRect, WindowGeom};
use crate::dock::world;
use crate::layout::apply::DockOutcome;
use crate::layout::chrome::{DockChrome, PointCss, RectCss};
use crate::layout::dock::{DockAnchor, DockSide};
use crate::layout::geometry::Insets;
use crate::layout::manager::{DockError, DockTo, FrameMeasures, LeaseId};
use crate::layout::{CanvasPx, UiMetrics, ViewManager, WindowBounds, MAIN_WINDOW_LABEL};

const MAIN: &str = MAIN_WINDOW_LABEL;
const W: &str = "slot-popup-1";
const SOLO: &str = "slot-popup-2";
const SPARE: &str = "slot-popup-9";

const AREA: RectCss = RectCss {
    x0: 0.0,
    y0: 0.0,
    x1: 600.0,
    y1: 28.0,
};
const GRAB: Grab = Grab {
    x_ratio: 0.25,
    y: 12.0,
    single_x: 40.0,
};
// 그 창 탭바 구역(탭 끌기 영역 (0,0)-(600,28) · 캔버스 원점 (0,28)) — 뷰포트 좌표.
const ZONE: RectCss = RectCss {
    x0: 0.0,
    y0: -15.0,
    x1: 600.0,
    y1: 43.0,
};

#[derive(Debug, Clone, PartialEq)]
enum Call {
    Lease(ViewId),
    Release(LeaseId),
    Dock(DockWrite),
    Switch(String, ViewId),
    CloseTab(String, ViewId),
    PlaceDetached(String, DetachPlan, PhysPoint),
    PlaceWhole(String, WholePlan),
    Restore(String, Placement, Option<String>, bool),
    Hide(String),
    Show(String),
    Focus(String),
    Raise(String),
    BeginLoop(String, bool, u64, u32),
    EndLoop(String),
    CloseWindow(String),
    Record(String),
    /// 안내 창을 놓는 창(`None` = 치움).
    Overlay(Option<String>),
    Unpark(String),
    GiveBack(String, FrameMeasures),
    ParkKept(String, FrameMeasures),
    FillSpare,
    Drag(TabDragEvent, Option<String>),
    Feed(String, TabDragFeed),
    ArmRaise(u32),
    ArmRelease(u32),
    PollStart(String, u32),
    PollStop,
}

struct Fake {
    mgr: ViewManager,
    parked: Parked,
    geoms: HashMap<String, WindowGeom>,
    monitors: Vec<Monitor>,
    cursor: PhysPoint,
    under: Option<String>,
    dock_fails: bool,
    loop_fails: bool,
    calls: Vec<Call>,
}

impl Fake {
    fn take(&mut self) -> Vec<Call> {
        std::mem::take(&mut self.calls)
    }

    fn tabs(&self, label: &str) -> Vec<ViewId> {
        self.mgr.windows[label].tabs.clone()
    }

    fn active(&self, label: &str) -> ViewId {
        self.mgr.windows[label].active
    }
}

impl Ports for Fake {
    fn world(&mut self) -> World {
        let mut world = world::snapshot(&self.mgr, &self.parked);
        for w in &mut world.windows {
            w.geom = self.geoms.get(&w.label).copied();
        }
        world.monitors = self.monitors.clone();
        world
    }

    fn lease_tab(&mut self, view: ViewId) -> Result<LeaseId, DockError> {
        self.calls.push(Call::Lease(view));
        self.mgr.lease_tab(view)
    }

    fn release_lease(&mut self, lease: LeaseId) {
        self.calls.push(Call::Release(lease));
        self.mgr.release_lease(lease);
    }

    fn dock(&mut self, write: DockWrite) -> Result<DockOutcome, DockError> {
        self.calls.push(Call::Dock(write.clone()));
        if self.dock_fails {
            return Err(DockError::TargetChanged);
        }
        let to = match write.to.clone() {
            SessionDock::Seam {
                target,
                anchor,
                side,
                share,
            } => DockTo::Seam {
                target,
                anchor,
                side,
                share,
            },
            SessionDock::TabBar { window, index } => DockTo::TabBar { window, index },
            SessionDock::AdoptWindow {
                label,
                seed,
                measures,
            } => DockTo::NewWindow {
                label,
                seed,
                measures,
            },
        };
        let req = write.into_request();
        let docked = self.mgr.dock(req.view, to, &req.guard, req.focus, req.mv)?;
        Ok(DockOutcome {
            window: docked.window,
            slot_ids: docked.slot_ids,
            emptied: docked.emptied,
            closed_window: None,
            main_refilled: docked.main_refilled,
            reused: false,
        })
    }

    fn switch_tab(&mut self, window: &str, view: ViewId) {
        self.calls.push(Call::Switch(window.to_owned(), view));
        self.mgr.switch_tab(window, view).unwrap();
    }

    fn close_tab(&mut self, window: &str, view: ViewId) {
        self.calls.push(Call::CloseTab(window.to_owned(), view));
        self.mgr.close_tab(window, view).unwrap();
    }

    fn cursor(&mut self) -> PhysPoint {
        self.cursor
    }

    fn probe(&mut self, _at: PhysPoint, skip: &str) -> Option<WindowLabel> {
        self.under.clone().filter(|u| u != skip)
    }

    fn geom(&mut self, label: &str) -> Option<WindowGeom> {
        self.geoms.get(label).copied()
    }

    fn place_detached(&mut self, label: &str, plan: &DetachPlan, cursor: PhysPoint) {
        self.calls
            .push(Call::PlaceDetached(label.to_owned(), *plan, cursor));
    }

    fn place_whole(&mut self, label: &str, plan: WholePlan) {
        self.calls.push(Call::PlaceWhole(label.to_owned(), plan));
    }

    fn restore_placement(
        &mut self,
        label: &str,
        start: Placement,
        behind: Option<&str>,
        activate: bool,
    ) {
        self.calls.push(Call::Restore(
            label.to_owned(),
            start,
            behind.map(str::to_owned),
            activate,
        ));
    }

    fn hide(&mut self, label: &str) {
        self.calls.push(Call::Hide(label.to_owned()));
    }

    fn show_in_place(&mut self, label: &str) {
        self.calls.push(Call::Show(label.to_owned()));
    }

    fn focus(&mut self, label: &str) {
        self.calls.push(Call::Focus(label.to_owned()));
    }

    fn raise_no_activate(&mut self, label: &str) {
        self.calls.push(Call::Raise(label.to_owned()));
    }

    fn begin_move_loop(
        &mut self,
        label: &str,
        hide_on_escape: bool,
        session: u64,
        loop_no: u32,
    ) -> Result<(), String> {
        self.calls.push(Call::BeginLoop(
            label.to_owned(),
            hide_on_escape,
            session,
            loop_no,
        ));
        if self.loop_fails {
            Err("시작 신호를 못 올렸다".to_owned())
        } else {
            Ok(())
        }
    }

    fn end_move_loop(&mut self, label: &str) {
        self.calls.push(Call::EndLoop(label.to_owned()));
    }

    fn close_window(&mut self, label: &str) {
        self.calls.push(Call::CloseWindow(label.to_owned()));
    }

    fn record_placement(&mut self, label: &str) {
        self.calls.push(Call::Record(label.to_owned()));
    }

    fn overlay(&mut self, overlay: Option<Overlay>) {
        self.calls.push(Call::Overlay(overlay.map(|o| o.over)));
    }

    fn unpark(&mut self, label: &str) {
        self.calls.push(Call::Unpark(label.to_owned()));
        self.parked.remove(label);
    }

    fn give_back(&mut self, label: &str, measures: FrameMeasures) {
        self.calls
            .push(Call::GiveBack(label.to_owned(), measures.clone()));
        self.parked
            .park(label.to_owned(), ParkedRole::Spare, measures);
    }

    fn park_kept(&mut self, label: &str, measures: FrameMeasures) {
        self.calls
            .push(Call::ParkKept(label.to_owned(), measures.clone()));
        self.parked
            .park(label.to_owned(), ParkedRole::Kept, measures);
    }

    fn fill_spare(&mut self) {
        self.calls.push(Call::FillSpare);
    }

    fn drag(&mut self, event: TabDragEvent, to: Option<&str>) {
        self.calls.push(Call::Drag(event, to.map(str::to_owned)));
    }

    fn feed(&mut self, to: &str, feed: TabDragFeed) {
        self.calls.push(Call::Feed(to.to_owned(), feed));
    }

    fn arm_raise(&mut self, seq: u32) {
        self.calls.push(Call::ArmRaise(seq));
    }

    fn arm_release(&mut self, seq: u32) {
        self.calls.push(Call::ArmRelease(seq));
    }

    fn poll_start(&mut self, label: &str, gen: u32) {
        self.calls.push(Call::PollStart(label.to_owned(), gen));
    }

    fn poll_stop(&mut self) {
        self.calls.push(Call::PollStop);
    }
}

// ── 판 ──

/// 탭 id — main = [m0, m1](m0 이 활성 — 누르는 순간 활성 · 누르기 전 활성은 m1) · W = [w0, w1, w2](w1 활성) · SOLO =
/// [s0](탭 하나 팝아웃).
struct Ids {
    m0: ViewId,
    m1: ViewId,
    w0: ViewId,
    w1: ViewId,
    w2: ViewId,
    s0: ViewId,
}

fn chrome() -> DockChrome {
    DockChrome {
        canvas_origin: PointCss { x: 0.0, y: 28.0 },
        tab_bar: AREA,
    }
}

fn metrics() -> UiMetrics {
    UiMetrics {
        frame_insets: Insets {
            t: 1.0,
            r: 1.0,
            b: 1.0,
            l: 1.0,
        },
        min_pane_px: 80,
    }
}

fn spare_measures() -> FrameMeasures {
    FrameMeasures {
        canvas: Some(CanvasPx { w: 1200, h: 800 }),
        metrics: Some(metrics()),
        chrome: Some(chrome()),
    }
}

// 안쪽 왼쪽 위가 (x, y) 인 1200 × 828 창 · 틀 = 옆 8 · 위 31 · 아래 8 · 배율 1.
fn geom_at(x: i32, y: i32) -> WindowGeom {
    WindowGeom {
        outer: PhysRect {
            x0: x - 8,
            y0: y - 31,
            x1: x + 1208,
            y1: y + 836,
        },
        inner: PhysRect {
            x0: x,
            y0: y,
            x1: x + 1200,
            y1: y + 828,
        },
        scale: 1.0,
    }
}

fn ready_spare(parked: &mut Parked) {
    parked.reserve_spare(SPARE.to_owned());
    parked.mark_built(SPARE);
    parked.record(SPARE, Measure::Canvas { w: 1200, h: 800 });
    parked.record(SPARE, Measure::Metrics(metrics()));
    parked.record(SPARE, Measure::Chrome(chrome()));
}

fn fake(main_tabs: usize) -> (Fake, Ids) {
    let mut mgr = ViewManager::new();
    let m0 = mgr.windows[MAIN].active;
    let m1 = if main_tabs > 1 {
        let m1 = mgr.create_tab(MAIN, None).unwrap();
        mgr.switch_tab(MAIN, m0).unwrap();
        m1
    } else {
        m0
    };
    let w0 = mgr.create_window(W).unwrap();
    let w1 = mgr.create_tab(W, None).unwrap();
    let w2 = mgr.create_tab(W, None).unwrap();
    mgr.switch_tab(W, w1).unwrap();
    let s0 = mgr.create_window(SOLO).unwrap();
    for label in [MAIN, W, SOLO] {
        mgr.set_window_canvas(label, 1200, 800).unwrap();
        mgr.set_dock_chrome(label, chrome()).unwrap();
    }
    let mut parked = Parked::default();
    ready_spare(&mut parked);
    let geoms = [
        (MAIN, geom_at(100, 100)),
        (W, geom_at(2000, 100)),
        (SOLO, geom_at(100, 1100)),
        (SPARE, geom_at(-5000, 0)),
    ]
    .into_iter()
    .map(|(l, g)| (l.to_owned(), g))
    .collect();
    let monitor = PhysRect {
        x0: 0,
        y0: 0,
        x1: 3840,
        y1: 2160,
    };
    let fake = Fake {
        mgr,
        parked,
        geoms,
        monitors: vec![Monitor {
            rect: monitor,
            work: PhysRect {
                y1: 2120,
                ..monitor
            },
            scale: 1.0,
        }],
        cursor: PhysPoint { x: 600, y: 400 },
        under: None,
        dock_fails: false,
        loop_fails: false,
        calls: Vec::new(),
    };
    (
        fake,
        Ids {
            m0,
            m1,
            w0,
            w1,
            w2,
            s0,
        },
    )
}

// 커서 자리 — W 탭바 구역 · W 왼쪽 바깥 띠 · 메인 탭바 구역 · 우리 창 밖.
const OVER_W_STRIP: PhysPoint = PhysPoint { x: 2300, y: 110 };
const OVER_W_BAND: PhysPoint = PhysPoint { x: 2010, y: 500 };
const OVER_MAIN_STRIP: PhysPoint = PhysPoint { x: 400, y: 110 };
const NOWHERE: PhysPoint = PhysPoint { x: 3500, y: 2000 };

fn point(f: &mut Fake, at: PhysPoint, under: Option<&str>) {
    f.cursor = at;
    f.under = under.map(str::to_owned);
}

// ── 사건 ──

fn begin_at(
    d: &mut Drag,
    f: &mut Fake,
    window: &str,
    view: ViewId,
    initial_active: Option<ViewId>,
    area: RectCss,
) -> BeginReply {
    let (tx, rx) = mpsc::channel();
    d.handle(
        Event::Begin {
            window: window.to_owned(),
            args: TabDragBegin {
                view,
                down: PointCss { x: 50.0, y: 10.0 },
                area,
                grab: GRAB,
                initial_active,
            },
            reply: Reply::new(move |r| {
                let _ = tx.send(r);
            }),
        },
        f,
    );
    rx.try_recv().expect("begin 은 늘 답한다")
}

fn begin(
    d: &mut Drag,
    f: &mut Fake,
    window: &str,
    view: ViewId,
    initial_active: Option<ViewId>,
) -> BeginReply {
    begin_at(d, f, window, view, initial_active, AREA)
}

fn sid(d: &Drag) -> u64 {
    d.session_id().expect("세션이 있다")
}

/// 답 — `Err` = 답 없이 버려졌다.
fn leave(d: &mut Drag, f: &mut Fake, from: &str) -> Result<LeaveReply, mpsc::TryRecvError> {
    let session = sid(d);
    leave_as(d, f, session, from)
}

fn leave_as(
    d: &mut Drag,
    f: &mut Fake,
    session: u64,
    from: &str,
) -> Result<LeaveReply, mpsc::TryRecvError> {
    let (tx, rx) = mpsc::channel();
    d.handle(
        Event::Leave {
            session,
            from: from.to_owned(),
            reply: Reply::new(move |r| {
                let _ = tx.send(r);
            }),
        },
        f,
    );
    rx.try_recv()
}

fn reorder(d: &mut Drag, f: &mut Fake, from: &str, seq: u32, index: u32) {
    let session = sid(d);
    d.handle(
        Event::Reorder {
            session,
            from: from.to_owned(),
            seq,
            index,
        },
        f,
    );
}

fn end(d: &mut Drag, f: &mut Fake, from: &str, how: EndHow, src: EndSrc, at: Option<u32>) {
    let session = sid(d);
    d.handle(
        Event::End {
            session,
            from: from.to_owned(),
            how,
            src,
            at,
        },
        f,
    );
}

fn moved(d: &mut Drag, f: &mut Fake, label: &str) {
    d.handle(
        Event::Moved {
            label: label.to_owned(),
        },
        f,
    );
}

fn loop_no(d: &Drag) -> u32 {
    match phase(d) {
        Phase::Moving { loop_no, .. } => *loop_no,
        other => panic!("Moving 이 아니다: {other:?}"),
    }
}

fn loop_end(d: &mut Drag, f: &mut Fake, end: LoopEnd) {
    let (session, loop_no) = (sid(d), loop_no(d));
    let cursor = f.cursor;
    d.handle(
        Event::LoopEnded {
            session,
            loop_no,
            end,
            cursor,
        },
        f,
    );
}

fn pointer(d: &mut Drag, f: &mut Fake, cursor: PhysPoint, down: bool) {
    let gen = d.session.as_ref().unwrap().poll_gen;
    d.handle(Event::Pointer { gen, cursor, down }, f);
}

fn phase(d: &Drag) -> &Phase {
    &d.session.as_ref().expect("세션이 있다").phase
}

// ── 기대 ──

fn s(label: &str) -> String {
    label.to_owned()
}

fn active(session: u64) -> TabDragEvent {
    TabDragEvent {
        session,
        phase: TabDragPhase::Active,
    }
}

fn ended(session: u64) -> TabDragEvent {
    TabDragEvent {
        session,
        phase: TabDragPhase::Ended,
    }
}

fn dock(view: ViewId, to: SessionDock, focus: FocusPolicy, successor: Option<ViewId>) -> Call {
    Call::Dock(DockWrite {
        view,
        lease: LeaseId(1),
        to,
        focus,
        successor,
    })
}

fn tab_bar(window: &str, index: usize) -> SessionDock {
    SessionDock::TabBar {
        window: window.to_owned(),
        index: Some(index),
    }
}

fn adopt_spare() -> SessionDock {
    SessionDock::AdoptWindow {
        label: s(SPARE),
        seed: None,
        measures: spare_measures(),
    }
}

fn feed(session: u64, view: ViewId, step: FeedStep) -> TabDragFeed {
    TabDragFeed {
        session,
        view,
        step,
    }
}

fn attach_feed(session: u64, view: ViewId, x: f64, y: f64) -> TabDragFeed {
    feed(
        session,
        view,
        FeedStep::Attach {
            zone: ZONE,
            grab: GRAB,
            x,
            y,
        },
    )
}

/// 공통 마무리 — `show` = 탭을 쥔 창 · `closing` = 닫는 남긴 창 · `last` = 결과별 끝(앞으로 · 자리 기록).
fn finish_calls(
    session: u64,
    show: Option<&str>,
    closing: Option<&str>,
    last: Option<Call>,
) -> Vec<Call> {
    let mut calls = vec![
        Call::Overlay(None),
        Call::PollStop,
        Call::Release(LeaseId(1)),
    ];
    calls.extend(show.map(|w| Call::Show(s(w))));
    calls.extend(closing.map(|w| Call::CloseWindow(s(w))));
    calls.extend(last);
    calls.extend([Call::Drag(ended(session), None), Call::FillSpare]);
    calls
}

fn main_start() -> Placement {
    Placement {
        normal: geom_at(100, 100).outer,
        maximized: false,
    }
}

fn spare_plan() -> DetachPlan {
    let outer = geom_at(100, 100).outer.size();
    DetachPlan {
        restored: SizeDip {
            w: f64::from(outer.w),
            h: f64::from(outer.h),
        },
        maximized: None,
        strip_origin: PointCss { x: 0.0, y: 0.0 },
        grab: PointCss { x: 40.0, y: 12.0 },
    }
}

/// 메인 m0 을 끌어 예비 창으로 뗀다 — 호출 기록은 비운 채 돌려준다.
fn detached(main_tabs: usize) -> (Drag, Fake, Ids) {
    let (mut f, ids) = fake(main_tabs);
    let mut d = Drag::new();
    let initial = (main_tabs > 1).then_some(ids.m1);
    begin(&mut d, &mut f, MAIN, ids.m0, initial);
    assert_eq!(leave(&mut d, &mut f, MAIN), Ok(LeaveReply::Detached));
    f.take();
    (d, f, ids)
}

/// 떼어진 m0 을 W 탭바에 붙인다(W 의 붙기 전 활성 = w1 · 자리 1).
fn attached_to_w() -> (Drag, Fake, Ids) {
    let (mut d, mut f, ids) = detached(2);
    point(&mut f, OVER_W_STRIP, Some(W));
    moved(&mut d, &mut f, SPARE);
    loop_end(&mut d, &mut f, LoopEnd::Ended);
    f.take();
    (d, f, ids)
}

// ── A1 · A2 시작 ──

#[test]
fn a1_two_tabs_begin_in_the_strip_with_the_canvas_rounded_zone() {
    let (mut f, ids) = fake(2);
    f.mgr
        .set_dock_chrome(
            MAIN,
            DockChrome {
                canvas_origin: PointCss { x: 0.0, y: 28.5 },
                ..chrome()
            },
        )
        .unwrap();
    let mut d = Drag::new();
    let area = RectCss {
        x0: 4.5,
        y0: 0.25,
        x1: 504.5,
        y1: 28.25,
    };
    let reply = begin_at(&mut d, &mut f, MAIN, ids.m0, Some(ids.m1), area);
    assert_eq!(
        reply,
        BeginReply::Strip {
            session: 1,
            zone: RectCss {
                x0: 4.0,
                y0: -15.5,
                x1: 505.0,
                y1: 43.5,
            },
        },
        "판정과 같은 정수 경계(캔버스 좌표)를 뷰포트로 되돌린다"
    );
    assert_eq!(
        f.take(),
        vec![
            Call::Lease(ids.m0),
            Call::Drag(active(1), None),
            Call::FillSpare,
        ]
    );
    assert_eq!(
        phase(&d),
        &Phase::Strip {
            holder: s(MAIN),
            feed: Feed::Pointer,
            releasing: None,
            last_reorder_seq: 0,
        }
    );
}

#[test]
fn a1_a_tab_that_cannot_be_leased_is_rejected_without_a_session() {
    let (mut f, _) = fake(2);
    let mut d = Drag::new();
    let ghost = ViewId::from_u128(404);
    let reply = begin(&mut d, &mut f, MAIN, ghost, None);
    assert!(matches!(reply, BeginReply::Rejected { .. }), "{reply:?}");
    assert_eq!(f.take(), vec![Call::Lease(ghost)]);
    assert_eq!(d.session_id(), None);
}

#[test]
fn a1_a_single_tab_window_follows_whole_from_its_origin_plus_the_cursor_travel() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    f.cursor = PhysPoint { x: 170, y: 1125 };
    let reply = begin(&mut d, &mut f, SOLO, ids.s0, None);
    assert_eq!(reply, BeginReply::WholeWindow { session: 1 });
    // 누른 점 = 안쪽 (100,1100) + (50,10) · 원점 (92,1069) + (커서 − 누른 점)(20,15).
    assert_eq!(
        f.take(),
        vec![
            Call::Lease(ids.s0),
            Call::Drag(active(1), None),
            Call::FillSpare,
            Call::PlaceWhole(s(SOLO), WholePlan::Move(PhysPoint { x: 112, y: 1084 })),
            Call::BeginLoop(s(SOLO), false, 1, 1),
        ]
    );
    assert_eq!(
        phase(&d),
        &Phase::Moving {
            window: s(SOLO),
            kind: MoveKind::Whole { main: false },
            pending: None,
            raise: None,
            loop_no: 1,
        }
    );
    assert_eq!(
        d.session.as_ref().unwrap().original,
        Some(SizeDip {
            w: 1200.0,
            h: 800.0
        }),
        "놓은 직후 OS 안쪽 크기 − 캔버스 원점"
    );
}

#[test]
fn a1_a_maximized_single_tab_window_restores_to_the_dragged_size_first() {
    let (mut f, ids) = fake(2);
    {
        let attrs = &mut f.mgr.windows.get_mut(SOLO).unwrap().attrs;
        attrs.maximized = true;
        attrs.bounds = WindowBounds::new(300.0, 200.0, 800.0, 600.0);
    }
    // 최대화된 바깥 = 3840 × 2120.
    f.geoms.insert(
        s(SOLO),
        WindowGeom {
            outer: PhysRect {
                x0: -8,
                y0: -8,
                x1: 3832,
                y1: 2112,
            },
            inner: PhysRect {
                x0: 0,
                y0: 23,
                x1: 3824,
                y1: 2104,
            },
            scale: 1.0,
        },
    );
    let mut d = Drag::new();
    f.cursor = PhysPoint { x: 60, y: 43 };
    begin(&mut d, &mut f, SOLO, ids.s0, None);
    // 누른 점 = (0,23) + (50,10) = (50,33) · 커서 이동 (10,10) · 크기 = max(복원 816×639, 최대화 절반 1920×1060).
    let plan = WholePlan::Restore(PhysRect {
        x0: 310,
        y0: 210,
        x1: 310 + 1920,
        y1: 210 + 1060,
    });
    assert_eq!(f.take()[3], Call::PlaceWhole(s(SOLO), plan));
    let start = d.session.as_ref().unwrap().origin.start;
    assert_eq!(
        start,
        Placement {
            normal: PhysRect {
                x0: 300,
                y0: 200,
                x1: 300 + 816,
                y1: 200 + 639,
            },
            maximized: true,
        },
        "되돌릴 자리 = 모델의 보통 자리 원점 + 복원 바깥 크기"
    );
}

#[test]
fn a1_a_move_loop_that_does_not_start_ends_the_drag_where_the_window_stands() {
    let (mut f, ids) = fake(2);
    f.loop_fails = true;
    let mut d = Drag::new();
    begin(&mut d, &mut f, SOLO, ids.s0, None);
    let calls = f.take();
    assert_eq!(
        calls[5..],
        finish_calls(1, Some(SOLO), None, Some(Call::Record(s(SOLO))))[..]
    );
    assert_eq!(d.session_id(), None);
    assert_eq!(f.mgr.lease(), None);
}

#[test]
fn a2_a_second_begin_is_busy_and_leaves_the_drag_alone() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, Some(ids.m1));
    f.take();
    let reply = begin(&mut d, &mut f, W, ids.w0, None);
    assert_eq!(reply, BeginReply::Busy);
    assert_eq!(f.take(), vec![]);
    assert_eq!(sid(&d), 1);
}

// ── A3 순서 이동 ──

#[test]
fn a3_a_reorder_from_the_holder_writes_the_strip_and_only_newer_seqs_count() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, Some(ids.m1));
    f.take();
    reorder(&mut d, &mut f, MAIN, 2, 1);
    assert_eq!(
        f.take(),
        vec![dock(ids.m0, tab_bar(MAIN, 1), FocusPolicy::Reveal, None)]
    );
    assert_eq!(f.tabs(MAIN), vec![ids.m1, ids.m0]);
    assert_eq!(f.mgr.lease().map(|l| l.1), Some(ids.m0), "표식을 이어 쥔다");

    reorder(&mut d, &mut f, MAIN, 2, 0);
    reorder(&mut d, &mut f, MAIN, 1, 0);
    assert_eq!(f.take(), vec![], "받은 번호 이하는 늦게 온 것");
    reorder(&mut d, &mut f, W, 9, 0);
    assert_eq!(f.take(), vec![], "holder 가 아닌 창");
    d.handle(
        Event::Reorder {
            session: 99,
            from: s(MAIN),
            seq: 9,
            index: 0,
        },
        &mut f,
    );
    assert_eq!(f.take(), vec![], "다른 세션");
    assert_eq!(d.session.as_ref().unwrap().counts.reorders, 1);
}

// ── A4 · A5 떼기 ──

#[test]
fn a4_leave_adopts_the_spare_places_it_under_the_cursor_and_starts_its_loop() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, Some(ids.m1));
    f.take();
    assert_eq!(leave(&mut d, &mut f, MAIN), Ok(LeaveReply::Detached));
    assert_eq!(
        f.take(),
        vec![
            dock(ids.m0, adopt_spare(), FocusPolicy::Reveal, Some(ids.m1)),
            Call::Unpark(s(SPARE)),
            Call::PollStop,
            Call::Drag(active(1), Some(s(SPARE))),
            Call::PlaceDetached(s(SPARE), spare_plan(), PhysPoint { x: 600, y: 400 }),
            Call::BeginLoop(s(SPARE), true, 1, 1),
        ]
    );
    assert_eq!(f.tabs(SPARE), vec![ids.m0]);
    assert_eq!(
        f.active(MAIN),
        ids.m1,
        "떼는 창의 다음 활성 = 누르기 전 활성"
    );
    assert!(!f.parked.contains(SPARE));
    assert_eq!(
        phase(&d),
        &Phase::Moving {
            window: s(SPARE),
            kind: MoveKind::Detached,
            pending: None,
            raise: None,
            loop_no: 1,
        }
    );
    let session = d.session.as_ref().unwrap();
    assert_eq!(
        session.original,
        Some(SizeDip {
            w: 1200.0,
            h: 800.0
        })
    );
    assert_eq!(session.counts.detaches, 1);
}

#[test]
fn a4_not_ready_keeps_the_strip_and_the_next_leave_detaches_once_the_spare_is_ready() {
    let (mut f, ids) = fake(2);
    f.parked = Parked::default();
    f.parked.reserve_spare(SPARE.to_owned());
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, Some(ids.m1));
    f.take();
    assert_eq!(leave(&mut d, &mut f, MAIN), Ok(LeaveReply::NotReady));
    assert_eq!(f.take(), vec![]);
    assert!(matches!(phase(&d), Phase::Strip { .. }));

    ready_spare(&mut f.parked);
    assert_eq!(leave(&mut d, &mut f, MAIN), Ok(LeaveReply::Detached));
    assert!(matches!(phase(&d), Phase::Moving { .. }));
}

#[test]
fn a4_a_failed_adopt_answers_ended_and_finishes_in_the_strip() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, Some(ids.m1));
    f.take();
    f.dock_fails = true;
    assert_eq!(leave(&mut d, &mut f, MAIN), Ok(LeaveReply::Ended));
    let mut want = vec![dock(
        ids.m0,
        adopt_spare(),
        FocusPolicy::Reveal,
        Some(ids.m1),
    )];
    want.extend(finish_calls(1, Some(MAIN), None, None));
    assert_eq!(f.take(), want);
    assert_eq!(d.session_id(), None);
}

#[test]
fn a4_a_detached_loop_that_does_not_start_leaves_a_new_window() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, Some(ids.m1));
    f.take();
    f.loop_fails = true;
    assert_eq!(leave(&mut d, &mut f, MAIN), Ok(LeaveReply::Detached));
    let calls = f.take();
    assert_eq!(calls[5], Call::BeginLoop(s(SPARE), true, 1, 1));
    assert_eq!(
        calls[6..],
        finish_calls(1, Some(SPARE), None, Some(Call::Record(s(SPARE))))[..]
    );
    assert_eq!(f.tabs(SPARE), vec![ids.m0], "탭은 떼어진 창에 남는다");
}

#[test]
fn a5_leave_while_releasing_is_not_ready() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, Some(ids.m1));
    end(&mut d, &mut f, W, EndHow::Drop, EndSrc::Key, None);
    f.take();
    assert_eq!(leave(&mut d, &mut f, MAIN), Ok(LeaveReply::NotReady));
    assert_eq!(f.take(), vec![]);
}

#[test]
fn a0_a_leave_of_another_session_or_window_is_dropped_unanswered() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, Some(ids.m1));
    f.take();
    assert_eq!(
        leave_as(&mut d, &mut f, 99, MAIN),
        Err(mpsc::TryRecvError::Disconnected)
    );
    assert_eq!(
        leave(&mut d, &mut f, W),
        Err(mpsc::TryRecvError::Disconnected)
    );
    assert_eq!(f.take(), vec![]);
    assert!(matches!(phase(&d), Phase::Strip { .. }));
}

// ── A6 · A7 · A8 탭바 안 끝 ──

#[test]
fn a6_a_pointer_drop_at_the_model_spot_just_finishes() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, Some(ids.m1));
    f.take();
    end(&mut d, &mut f, MAIN, EndHow::Drop, EndSrc::Pointer, Some(0));
    assert_eq!(f.take(), finish_calls(1, Some(MAIN), None, None));
    assert_eq!(d.session_id(), None);
    assert_eq!(f.mgr.lease(), None);
}

#[test]
fn a6_a_drop_whose_spot_differs_from_the_model_writes_it_first() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, Some(ids.m1));
    f.take();
    end(&mut d, &mut f, MAIN, EndHow::Drop, EndSrc::Key, Some(1));
    let mut want = vec![dock(ids.m0, tab_bar(MAIN, 1), FocusPolicy::Reveal, None)];
    want.extend(finish_calls(1, Some(MAIN), None, None));
    assert_eq!(f.take(), want);
    assert_eq!(f.tabs(MAIN), vec![ids.m1, ids.m0]);
}

#[test]
fn a7_a_key_drop_from_another_window_asks_the_holder_and_its_answer_ends() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, Some(ids.m1));
    f.take();
    end(&mut d, &mut f, W, EndHow::Drop, EndSrc::Key, None);
    assert_eq!(
        f.take(),
        vec![
            Call::PollStop,
            Call::Feed(s(MAIN), feed(1, ids.m0, FeedStep::Release)),
            Call::ArmRelease(1),
        ]
    );
    end(&mut d, &mut f, W, EndHow::Drop, EndSrc::Key, None);
    end(&mut d, &mut f, MAIN, EndHow::Cancel, EndSrc::Key, None);
    assert_eq!(f.take(), vec![], "놓기를 이미 정했다");
    end(&mut d, &mut f, MAIN, EndHow::Drop, EndSrc::Release, Some(0));
    assert_eq!(f.take(), finish_calls(1, Some(MAIN), None, None));
}

#[test]
fn a6_a_release_answer_without_a_confirm_request_is_ignored() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, Some(ids.m1));
    f.take();
    end(&mut d, &mut f, MAIN, EndHow::Drop, EndSrc::Release, Some(1));
    assert_eq!(f.take(), vec![]);
    assert!(d.session_id().is_some());
}

#[test]
fn a8_esc_in_the_source_strip_reverts_the_spot_and_the_initial_selection() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, Some(ids.m1));
    reorder(&mut d, &mut f, MAIN, 1, 1);
    f.take();
    end(&mut d, &mut f, W, EndHow::Cancel, EndSrc::Key, None);
    let mut want = vec![
        dock(ids.m0, tab_bar(MAIN, 0), FocusPolicy::Keep, None),
        Call::Switch(s(MAIN), ids.m1),
    ];
    want.extend(finish_calls(
        1,
        Some(MAIN),
        None,
        Some(Call::Focus(s(MAIN))),
    ));
    assert_eq!(f.take(), want, "Esc 는 어느 창에서 와도 되돌린다");
    assert_eq!(f.tabs(MAIN), vec![ids.m0, ids.m1]);
    assert_eq!(f.active(MAIN), ids.m1);
}

#[test]
fn a8_capture_lost_reverts_only_from_the_holder() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, None);
    f.take();
    end(&mut d, &mut f, W, EndHow::Cancel, EndSrc::CaptureLost, None);
    assert_eq!(f.take(), vec![], "다른 창의 캡처 잃음");
    end(
        &mut d,
        &mut f,
        MAIN,
        EndHow::Cancel,
        EndSrc::CaptureLost,
        None,
    );
    assert_eq!(
        f.take(),
        finish_calls(1, Some(MAIN), None, Some(Call::Focus(s(MAIN)))),
        "끄는 탭이 활성이었다 — 고칠 것 없음"
    );
    assert_eq!(d.session_id(), None);
}

// ── A9 · A10 · A11 붙은 창 ──

#[test]
fn a9_the_poller_feeds_the_cursor_in_the_holder_viewport() {
    let (mut d, mut f, ids) = attached_to_w();
    pointer(&mut d, &mut f, PhysPoint { x: 2350, y: 120 }, true);
    assert_eq!(
        f.take(),
        vec![Call::Feed(
            s(W),
            feed(1, ids.m0, FeedStep::Move { x: 350.0, y: 20.0 })
        )]
    );
    d.handle(
        Event::Pointer {
            gen: 0,
            cursor: PhysPoint { x: 1, y: 1 },
            down: true,
        },
        &mut f,
    );
    assert_eq!(f.take(), vec![], "옛 세대");
}

#[test]
fn a10_no_answer_to_the_release_confirm_ends_at_the_model_spot() {
    let (mut d, mut f, ids) = attached_to_w();
    pointer(&mut d, &mut f, OVER_W_STRIP, false);
    assert_eq!(
        f.take(),
        vec![
            Call::PollStop,
            Call::Feed(s(W), feed(1, ids.m0, FeedStep::Release)),
            Call::ArmRelease(1),
        ]
    );
    pointer(&mut d, &mut f, OVER_W_STRIP, true);
    d.handle(Event::ReleaseDue { seq: 7 }, &mut f);
    assert_eq!(f.take(), vec![], "놓기 확인 중의 폴러 · 다른 시한");
    d.handle(Event::ReleaseDue { seq: 1 }, &mut f);
    assert_eq!(
        f.take(),
        finish_calls(1, Some(W), None, Some(Call::Focus(s(W))))
    );
    assert_eq!(f.tabs(W)[0], ids.m0, "붙은 자리에 남는다");
}

#[test]
fn a11_the_holder_window_gone_ends_the_drag() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, None);
    f.take();
    d.handle(Event::WindowGone { label: s(W) }, &mut f);
    assert_eq!(f.take(), vec![], "다른 창");
    d.handle(Event::WindowGone { label: s(MAIN) }, &mut f);
    assert_eq!(f.take(), finish_calls(1, Some(MAIN), None, None));
    assert_eq!(d.session_id(), None);
}

// ── A12 · A13 따라오는 창 ──

#[test]
fn a12_over_a_band_draws_the_guide_and_rearms_the_raise() {
    let (mut d, mut f, _) = detached(2);
    point(&mut f, OVER_W_BAND, Some(W));
    moved(&mut d, &mut f, SPARE);
    point(&mut f, NOWHERE, None);
    moved(&mut d, &mut f, SPARE);
    assert_eq!(
        f.take(),
        vec![
            Call::Overlay(Some(s(W))),
            Call::ArmRaise(1),
            Call::Overlay(None),
            Call::ArmRaise(2),
        ]
    );
    moved(&mut d, &mut f, W);
    assert_eq!(f.take(), vec![], "끄는 창의 것만");
}

#[test]
fn a12_reaching_a_strip_hides_the_dragging_window_and_ends_its_loop() {
    let (mut d, mut f, _) = detached(2);
    point(&mut f, OVER_W_STRIP, Some(W));
    moved(&mut d, &mut f, SPARE);
    assert_eq!(
        f.take(),
        vec![
            Call::Overlay(None),
            Call::Hide(s(SPARE)),
            Call::EndLoop(s(SPARE)),
        ]
    );
    assert!(matches!(
        phase(&d),
        Phase::Moving {
            pending: Some(Pending::Attach { to, active_before: Some(1) }),
            ..
        } if to == W
    ));
    moved(&mut d, &mut f, SPARE);
    end(&mut d, &mut f, MAIN, EndHow::Cancel, EndSrc::Key, None);
    assert_eq!(f.take(), vec![], "처음 정한 것이 이긴다");
}

#[test]
fn a13_the_raise_lifts_our_window_under_the_cursor_then_the_dragging_one() {
    let (mut d, mut f, _) = detached(2);
    point(&mut f, NOWHERE, None);
    moved(&mut d, &mut f, SPARE);
    f.take();
    d.handle(Event::RaiseDue { seq: 1 }, &mut f);
    assert_eq!(f.take(), vec![], "커서 밑에 우리 창이 없다");
    point(&mut f, NOWHERE, Some(MAIN));
    moved(&mut d, &mut f, SPARE);
    f.take();
    d.handle(Event::RaiseDue { seq: 1 }, &mut f);
    assert_eq!(f.take(), vec![], "옛 시한");
    d.handle(Event::RaiseDue { seq: 2 }, &mut f);
    assert_eq!(f.take(), vec![Call::Raise(s(MAIN)), Call::Raise(s(SPARE))]);
}

// ── A14 · A15 루프 중 키 ──

#[test]
fn a14_a_key_drop_during_the_loop_drops_at_the_loop_end_cursor() {
    let (mut d, mut f, ids) = detached(2);
    end(&mut d, &mut f, MAIN, EndHow::Drop, EndSrc::Key, None);
    assert_eq!(f.take(), vec![Call::EndLoop(s(SPARE))]);
    point(&mut f, NOWHERE, None);
    loop_end(&mut d, &mut f, LoopEnd::Ended);
    let mut want = vec![Call::Overlay(None)];
    want.extend(finish_calls(
        1,
        Some(SPARE),
        None,
        Some(Call::Record(s(SPARE))),
    ));
    assert_eq!(f.take(), want);
    assert_eq!(f.tabs(SPARE), vec![ids.m0], "새 창으로 남는다");
}

#[test]
fn a15_esc_during_the_loop_reverts_at_the_loop_end() {
    let (mut d, mut f, ids) = detached(2);
    end(&mut d, &mut f, SPARE, EndHow::Cancel, EndSrc::Key, None);
    assert_eq!(f.take(), vec![Call::Overlay(None), Call::EndLoop(s(SPARE))]);
    loop_end(&mut d, &mut f, LoopEnd::Ended);
    let mut want = vec![
        dock(ids.m0, tab_bar(MAIN, 0), FocusPolicy::Keep, None),
        Call::GiveBack(s(SPARE), spare_measures()),
    ];
    want.extend(finish_calls(
        1,
        Some(MAIN),
        None,
        Some(Call::Focus(s(MAIN))),
    ));
    assert_eq!(f.take(), want, "활성은 출처로 돌아간다");
    assert_eq!(f.tabs(MAIN), vec![ids.m0, ids.m1]);
    assert_eq!(f.active(MAIN), ids.m1);
    assert!(f.parked.ready_spare().is_some(), "예비로 돌아갔다");
}

// ── A16 놓기 ──

#[test]
fn a16_a_release_over_a_band_docks_with_the_judged_share_and_gives_the_spare_back() {
    let (mut d, mut f, ids) = detached(2);
    point(&mut f, OVER_W_BAND, Some(W));
    moved(&mut d, &mut f, SPARE);
    f.take();
    let original = d.session.as_ref().unwrap().original;
    let world = f.world();
    let Target::Band { hit, .. } = targets::judge(&world, OVER_W_BAND, Some(W), original).target
    else {
        panic!("띠 위다");
    };
    assert_eq!(hit.share, 0.5);
    loop_end(&mut d, &mut f, LoopEnd::Released);
    let mut want = vec![
        Call::Overlay(None),
        dock(
            ids.m0,
            SessionDock::Seam {
                target: ids.w1,
                anchor: DockAnchor::Outer,
                side: DockSide::Left,
                share: hit.share,
            },
            FocusPolicy::Reveal,
            None,
        ),
        Call::GiveBack(s(SPARE), spare_measures()),
    ];
    want.extend(finish_calls(1, None, None, Some(Call::Focus(s(W)))));
    assert_eq!(f.take(), want);
    assert!(!f.mgr.windows.contains_key(SPARE));
    assert!(
        f.mgr.views.get(&ids.m0).is_none(),
        "끄는 탭은 대상 탭에 합쳐졌다"
    );
}

#[test]
fn a16_a_release_elsewhere_leaves_the_new_window_and_records_its_place() {
    let (mut d, mut f, ids) = detached(2);
    point(&mut f, NOWHERE, None);
    loop_end(&mut d, &mut f, LoopEnd::Released);
    let mut want = vec![Call::Overlay(None)];
    want.extend(finish_calls(
        1,
        Some(SPARE),
        None,
        Some(Call::Record(s(SPARE))),
    ));
    assert_eq!(f.take(), want);
    assert_eq!(f.tabs(SPARE), vec![ids.m0]);
}

#[test]
fn a16_a_release_on_a_strip_is_not_an_attach() {
    let (mut d, mut f, ids) = detached(2);
    point(&mut f, OVER_W_STRIP, Some(W));
    loop_end(&mut d, &mut f, LoopEnd::Released);
    let mut want = vec![Call::Overlay(None)];
    want.extend(finish_calls(
        1,
        Some(SPARE),
        None,
        Some(Call::Record(s(SPARE))),
    ));
    assert_eq!(f.take(), want, "붙기는 움직일 때만");
    assert_eq!(f.tabs(SPARE), vec![ids.m0]);
}

#[test]
fn a16_a_failed_seam_leaves_the_tab_in_the_dragging_window() {
    let (mut d, mut f, ids) = detached(2);
    point(&mut f, OVER_W_BAND, Some(W));
    f.dock_fails = true;
    loop_end(&mut d, &mut f, LoopEnd::Released);
    let calls = f.take();
    assert!(matches!(calls[1], Call::Dock(_)));
    assert_eq!(
        calls[2..],
        finish_calls(1, Some(SPARE), None, Some(Call::Record(s(SPARE))))[..]
    );
    assert_eq!(f.tabs(SPARE), vec![ids.m0]);
}

// ── A17 붙기 ──

#[test]
fn a17_attach_puts_the_tab_first_in_w_returns_the_spare_and_feeds_w() {
    let (mut d, mut f, ids) = detached(2);
    point(&mut f, OVER_W_STRIP, Some(W));
    moved(&mut d, &mut f, SPARE);
    f.take();
    loop_end(&mut d, &mut f, LoopEnd::Ended);
    assert_eq!(
        f.take(),
        vec![
            dock(ids.m0, tab_bar(W, 0), FocusPolicy::Reveal, None),
            Call::GiveBack(s(SPARE), spare_measures()),
            Call::Focus(s(W)),
            Call::Feed(s(W), attach_feed(1, ids.m0, 300.0, 10.0)),
            Call::PollStart(s(W), 1),
        ]
    );
    assert_eq!(f.tabs(W), vec![ids.m0, ids.w0, ids.w1, ids.w2]);
    assert_eq!(
        phase(&d),
        &Phase::Strip {
            holder: s(W),
            feed: Feed::Shell,
            releasing: None,
            last_reorder_seq: 0,
        }
    );
    let session = d.session.as_ref().unwrap();
    assert_eq!(session.before_attach, Some((s(W), Some(1))));
    assert_eq!(session.counts.attaches, 1);
}

#[test]
fn a17_a_failed_attach_shows_the_hidden_window_with_the_tab() {
    let (mut d, mut f, ids) = detached(2);
    point(&mut f, OVER_W_STRIP, Some(W));
    moved(&mut d, &mut f, SPARE);
    f.take();
    f.dock_fails = true;
    loop_end(&mut d, &mut f, LoopEnd::Ended);
    let mut want = vec![dock(ids.m0, tab_bar(W, 0), FocusPolicy::Reveal, None)];
    want.extend(finish_calls(
        1,
        Some(SPARE),
        None,
        Some(Call::Record(s(SPARE))),
    ));
    assert_eq!(f.take(), want);
}

// ── A18 · A19 루프 끝 ──

#[test]
fn a18_a_loop_canceled_before_entry_reverts_into_the_source() {
    let (mut d, mut f, ids) = detached(2);
    loop_end(&mut d, &mut f, LoopEnd::Canceled);
    let mut want = vec![
        dock(ids.m0, tab_bar(MAIN, 0), FocusPolicy::Keep, None),
        Call::GiveBack(s(SPARE), spare_measures()),
    ];
    want.extend(finish_calls(
        1,
        Some(MAIN),
        None,
        Some(Call::Focus(s(MAIN))),
    ));
    assert_eq!(f.take(), want);
    assert_eq!(f.tabs(MAIN), vec![ids.m0, ids.m1]);
    assert_eq!(f.active(MAIN), ids.m1);
}

#[test]
fn a18_a_late_loop_end_of_another_loop_is_ignored() {
    let (mut d, mut f, _) = detached(2);
    let session = sid(&d);
    d.handle(
        Event::LoopEnded {
            session,
            loop_no: 99,
            end: LoopEnd::Canceled,
            cursor: NOWHERE,
        },
        &mut f,
    );
    d.handle(
        Event::LoopEnded {
            session: 99,
            loop_no: 1,
            end: LoopEnd::Canceled,
            cursor: NOWHERE,
        },
        &mut f,
    );
    assert_eq!(f.take(), vec![]);
    assert!(matches!(phase(&d), Phase::Moving { .. }));
}

#[test]
fn a19_the_dragging_window_gone_ends_once() {
    let (mut d, mut f, _) = detached(2);
    let session = sid(&d);
    d.handle(Event::WindowGone { label: s(SPARE) }, &mut f);
    assert_eq!(
        f.take(),
        finish_calls(1, Some(SPARE), None, None),
        "aborted — 자리 기록 없음"
    );
    d.handle(
        Event::LoopEnded {
            session,
            loop_no: 1,
            end: LoopEnd::Destroyed,
            cursor: NOWHERE,
        },
        &mut f,
    );
    assert_eq!(f.take(), vec![], "남은 쪽은 세션이 없다");
}

#[test]
fn a19_a_destroyed_loop_end_aborts() {
    let (mut d, mut f, _) = detached(2);
    loop_end(&mut d, &mut f, LoopEnd::Destroyed);
    assert_eq!(f.take(), finish_calls(1, Some(SPARE), None, None));
    assert_eq!(d.session_id(), None);
}

#[test]
fn a0_events_without_a_session_are_ignored() {
    let (mut f, _) = fake(2);
    let mut d = Drag::new();
    d.handle(Event::Moved { label: s(MAIN) }, &mut f);
    d.handle(Event::RaiseDue { seq: 1 }, &mut f);
    assert_eq!(
        leave_as(&mut d, &mut f, 1, MAIN),
        Err(mpsc::TryRecvError::Disconnected)
    );
    assert_eq!(f.take(), vec![]);
}

// ── 길 — 여러 사건 ──

#[test]
fn p7_c8_redetaching_from_w_restores_w_selection_counted_without_the_dragged_tab() {
    let (mut d, mut f, ids) = attached_to_w();
    assert_eq!(leave(&mut d, &mut f, W), Ok(LeaveReply::Detached));
    let calls = f.take();
    assert_eq!(
        calls[0],
        dock(ids.m0, adopt_spare(), FocusPolicy::Reveal, Some(ids.w1)),
        "W = [m0, w0, w1, w2] 에서 m0 을 뺀 목록의 자리 1 = w1"
    );
    assert_eq!(calls[3], Call::Drag(active(1), Some(s(SPARE))));
    assert_eq!(f.active(W), ids.w1);
    assert_eq!(d.session.as_ref().unwrap().before_attach, None);
    assert_eq!(d.session.as_ref().unwrap().counts.detaches, 2);
}

#[test]
fn p7_e11_esc_while_attached_returns_the_tab_and_w_selection_and_focuses_the_source() {
    let (mut d, mut f, ids) = attached_to_w();
    end(&mut d, &mut f, W, EndHow::Cancel, EndSrc::Key, None);
    let mut want = vec![
        dock(ids.m0, tab_bar(MAIN, 0), FocusPolicy::Keep, None),
        Call::Switch(s(W), ids.w1),
    ];
    want.extend(finish_calls(
        1,
        Some(MAIN),
        None,
        Some(Call::Focus(s(MAIN))),
    ));
    assert_eq!(
        f.take(),
        want,
        "붙었던 W 가 쥐던 활성은 출처로 간다(Chrome RevertDrag)"
    );
    assert_eq!(f.tabs(MAIN), vec![ids.m0, ids.m1]);
    assert_eq!(f.active(MAIN), ids.m1);
    assert_eq!(f.tabs(W), vec![ids.w0, ids.w1, ids.w2]);
    assert_eq!(f.active(W), ids.w1);
}

#[test]
fn p7_a_revert_whose_write_fails_leaves_the_tab_in_w_and_focuses_nothing() {
    let (mut d, mut f, ids) = attached_to_w();
    f.dock_fails = true;
    end(&mut d, &mut f, W, EndHow::Cancel, EndSrc::Key, None);
    let mut want = vec![dock(ids.m0, tab_bar(MAIN, 0), FocusPolicy::Keep, None)];
    want.extend(finish_calls(1, Some(W), None, None));
    let calls = f.take();
    assert_eq!(calls, want);
    assert!(
        !calls.iter().any(|c| matches!(c, Call::Focus(_))),
        "탭이 출처로 돌아가지 않았으면 아무 창도 활성으로 하지 않는다"
    );
    assert_eq!(f.tabs(W)[0], ids.m0, "탭은 지금 창에 남는다");
    assert_eq!(d.session_id(), None);
}

#[test]
fn p8_the_release_confirm_lets_the_holder_spot_win_over_a_late_reorder() {
    let (mut d, mut f, ids) = attached_to_w();
    pointer(&mut d, &mut f, OVER_W_STRIP, false);
    f.take();
    reorder(&mut d, &mut f, W, 5, 2);
    assert_eq!(
        f.take(),
        vec![dock(ids.m0, tab_bar(W, 2), FocusPolicy::Reveal, None)],
        "놓기 확인 중에도 holder 의 마지막 순서 이동은 쓴다"
    );
    end(&mut d, &mut f, W, EndHow::Drop, EndSrc::Release, Some(1));
    let mut want = vec![dock(ids.m0, tab_bar(W, 1), FocusPolicy::Reveal, None)];
    want.extend(finish_calls(1, Some(W), None, Some(Call::Focus(s(W)))));
    assert_eq!(f.take(), want);
    assert_eq!(f.tabs(W), vec![ids.w0, ids.m0, ids.w1, ids.w2]);
}

#[test]
fn p8_a_pointer_drop_is_not_taken_from_a_shell_fed_strip() {
    let (mut d, mut f, _) = attached_to_w();
    end(&mut d, &mut f, W, EndHow::Drop, EndSrc::Pointer, Some(0));
    end(&mut d, &mut f, W, EndHow::Cancel, EndSrc::CaptureLost, None);
    assert_eq!(f.take(), vec![]);
    assert!(d.session_id().is_some());
}

#[test]
fn p12_a_single_tab_window_attached_to_w_is_kept_and_closed_at_the_end() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, SOLO, ids.s0, None);
    point(&mut f, OVER_W_STRIP, Some(W));
    moved(&mut d, &mut f, SOLO);
    let calls = f.take();
    assert_eq!(
        calls[5..],
        [
            Call::Overlay(None),
            Call::Hide(s(SOLO)),
            Call::EndLoop(s(SOLO)),
        ]
    );
    loop_end(&mut d, &mut f, LoopEnd::Ended);
    let calls = f.take();
    assert_eq!(
        calls[0],
        dock(ids.s0, tab_bar(W, 0), FocusPolicy::Reveal, None)
    );
    assert!(matches!(&calls[1], Call::ParkKept(l, _) if l == SOLO));
    assert_eq!(calls[2], Call::Focus(s(W)));
    assert!(!f.mgr.windows.contains_key(SOLO));
    assert_eq!(
        d.session
            .as_ref()
            .unwrap()
            .emptied
            .as_ref()
            .map(|e| e.label.as_str()),
        Some(SOLO)
    );

    end(&mut d, &mut f, W, EndHow::Drop, EndSrc::Key, Some(0));
    assert_eq!(
        f.take(),
        finish_calls(1, Some(W), Some(SOLO), Some(Call::Focus(s(W))))
    );
}

#[test]
fn p12_esc_after_a_single_tab_window_attached_readopts_it_where_it_was() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, SOLO, ids.s0, None);
    point(&mut f, OVER_W_STRIP, Some(W));
    moved(&mut d, &mut f, SOLO);
    loop_end(&mut d, &mut f, LoopEnd::Ended);
    f.take();
    let kept = d.session.as_ref().unwrap().emptied.clone().unwrap();
    end(&mut d, &mut f, W, EndHow::Cancel, EndSrc::Key, None);
    let start = Placement {
        normal: geom_at(100, 1100).outer,
        maximized: false,
    };
    let mut want = vec![
        dock(
            ids.s0,
            SessionDock::AdoptWindow {
                label: s(SOLO),
                seed: Some(kept.seed),
                measures: kept.measures,
            },
            FocusPolicy::Keep,
            None,
        ),
        Call::Unpark(s(SOLO)),
        Call::Restore(s(SOLO), start, None, true),
        Call::Switch(s(W), ids.w1),
    ];
    want.extend(finish_calls(
        1,
        Some(SOLO),
        None,
        Some(Call::Focus(s(SOLO))),
    ));
    assert_eq!(f.take(), want);
    assert_eq!(f.tabs(SOLO), vec![ids.s0]);
    assert_eq!(f.active(W), ids.w1);
    assert!(!f.parked.contains(SOLO));
}

#[test]
fn p12_a_single_tab_window_dropped_on_a_band_closes() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, SOLO, ids.s0, None);
    point(&mut f, OVER_W_BAND, Some(W));
    moved(&mut d, &mut f, SOLO);
    f.take();
    loop_end(&mut d, &mut f, LoopEnd::Released);
    let calls = f.take();
    assert!(
        matches!(&calls[1], Call::Dock(DockWrite { to: SessionDock::Seam { target, .. }, .. }) if *target == ids.w1)
    );
    assert_eq!(
        calls[2..],
        finish_calls(1, None, Some(SOLO), Some(Call::Focus(s(W))))[..],
        "R10 · AC26 — 비운 창은 끝에 닫는다"
    );
}

#[test]
fn p12_esc_during_a_whole_window_loop_restores_its_start() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, SOLO, ids.s0, None);
    f.take();
    end(&mut d, &mut f, MAIN, EndHow::Cancel, EndSrc::Key, None);
    loop_end(&mut d, &mut f, LoopEnd::Ended);
    let start = Placement {
        normal: geom_at(100, 1100).outer,
        maximized: false,
    };
    let mut want = vec![
        Call::Overlay(None),
        Call::EndLoop(s(SOLO)),
        Call::Restore(s(SOLO), start, None, true),
    ];
    want.extend(finish_calls(
        1,
        Some(SOLO),
        None,
        Some(Call::Focus(s(SOLO))),
    ));
    assert_eq!(f.take(), want);
}

#[test]
fn p13_main_last_tab_follows_whole_refills_restores_behind_w_and_closes_the_refill_on_return() {
    let (mut f, ids) = fake(1);
    let mut d = Drag::new();
    assert_eq!(
        begin(&mut d, &mut f, MAIN, ids.m0, None),
        BeginReply::WholeWindow { session: 1 }
    );
    assert_eq!(
        phase(&d),
        &Phase::Moving {
            window: s(MAIN),
            kind: MoveKind::Whole { main: true },
            pending: None,
            raise: None,
            loop_no: 1,
        }
    );
    point(&mut f, OVER_W_STRIP, Some(W));
    moved(&mut d, &mut f, MAIN);
    let calls = f.take();
    assert_eq!(
        calls[5..],
        [Call::Overlay(None), Call::EndLoop(s(MAIN))],
        "메인은 숨기지 않는다"
    );
    loop_end(&mut d, &mut f, LoopEnd::Ended);
    let refill = d
        .session
        .as_ref()
        .unwrap()
        .main_refill
        .expect("메인에 빈 탭");
    assert_eq!(
        f.take(),
        vec![
            dock(ids.m0, tab_bar(W, 0), FocusPolicy::Reveal, None),
            Call::Restore(s(MAIN), main_start(), Some(s(W)), false),
            Call::Focus(s(W)),
            Call::Feed(s(W), attach_feed(1, ids.m0, 300.0, 10.0)),
            Call::PollStart(s(W), 1),
        ]
    );
    assert_eq!(f.tabs(MAIN), vec![refill]);

    // W 에서 다시 떼어(예비 창) 메인 탭바에 붙인다.
    assert_eq!(leave(&mut d, &mut f, W), Ok(LeaveReply::Detached));
    f.take();
    point(&mut f, OVER_MAIN_STRIP, Some(MAIN));
    moved(&mut d, &mut f, SPARE);
    assert_eq!(
        f.take(),
        vec![
            Call::Overlay(None),
            Call::Hide(s(SPARE)),
            Call::EndLoop(s(SPARE)),
        ]
    );
    loop_end(&mut d, &mut f, LoopEnd::Ended);
    assert_eq!(
        f.take(),
        vec![
            dock(ids.m0, tab_bar(MAIN, 0), FocusPolicy::Reveal, None),
            Call::GiveBack(s(SPARE), spare_measures()),
            Call::Focus(s(MAIN)),
            Call::Feed(s(MAIN), attach_feed(1, ids.m0, 300.0, 10.0)),
            Call::PollStart(s(MAIN), 2),
            Call::CloseTab(s(MAIN), refill),
        ]
    );
    assert_eq!(
        f.tabs(MAIN),
        vec![ids.m0],
        "G14 — 손대지 않은 빈 탭을 닫는다"
    );
    assert_eq!(d.session.as_ref().unwrap().main_refill, None);
}

#[test]
fn p13_a_refill_that_is_no_longer_the_untouched_active_tab_stays() {
    let (mut f, ids) = fake(1);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, None);
    point(&mut f, OVER_W_STRIP, Some(W));
    moved(&mut d, &mut f, MAIN);
    loop_end(&mut d, &mut f, LoopEnd::Ended);
    let refill = d.session.as_ref().unwrap().main_refill.unwrap();
    let other = f.mgr.create_tab(MAIN, None).unwrap();
    leave(&mut d, &mut f, W).unwrap();
    point(&mut f, OVER_MAIN_STRIP, Some(MAIN));
    moved(&mut d, &mut f, SPARE);
    f.take();
    loop_end(&mut d, &mut f, LoopEnd::Ended);
    assert!(!f.take().contains(&Call::CloseTab(s(MAIN), refill)));
    assert_eq!(f.tabs(MAIN), vec![ids.m0, refill, other]);
}

#[test]
fn p13_esc_while_main_last_tab_is_attached_returns_it_and_closes_the_refill() {
    let (mut f, ids) = fake(1);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, None);
    point(&mut f, OVER_W_STRIP, Some(W));
    moved(&mut d, &mut f, MAIN);
    loop_end(&mut d, &mut f, LoopEnd::Ended);
    let refill = d.session.as_ref().unwrap().main_refill.unwrap();
    f.take();
    end(&mut d, &mut f, W, EndHow::Cancel, EndSrc::Key, None);
    let mut want = vec![
        dock(ids.m0, tab_bar(MAIN, 0), FocusPolicy::Keep, None),
        Call::CloseTab(s(MAIN), refill),
        Call::Switch(s(W), ids.w1),
        Call::Restore(s(MAIN), main_start(), None, true),
    ];
    want.extend(finish_calls(
        1,
        Some(MAIN),
        None,
        Some(Call::Focus(s(MAIN))),
    ));
    assert_eq!(f.take(), want);
    assert_eq!(f.tabs(MAIN), vec![ids.m0]);
    assert_eq!(f.active(W), ids.w1);
}

#[test]
fn p13_esc_while_main_follows_restores_main() {
    let (mut f, ids) = fake(1);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, None);
    f.take();
    loop_end(&mut d, &mut f, LoopEnd::Canceled);
    let mut want = vec![Call::Restore(s(MAIN), main_start(), None, true)];
    want.extend(finish_calls(
        1,
        Some(MAIN),
        None,
        Some(Call::Focus(s(MAIN))),
    ));
    assert_eq!(f.take(), want);
    assert_eq!(f.tabs(MAIN), vec![ids.m0]);
}

/// 메인 마지막 탭 m0 을 창째 끌어 W 에 붙였다가 예비 창으로 떼어 메인에 다시 붙이고(첫 빈 탭은 G14 로 닫힌다) 메인에서
/// 또 뗀다 — 메인에는 그 떼기가 채운 둘째 빈 탭만 남는다. 호출 기록은 비운 채 그 빈 탭과 돌려준다.
fn redetached_from_main() -> (Drag, Fake, Ids, ViewId) {
    let (mut f, ids) = fake(1);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, None);
    point(&mut f, OVER_W_STRIP, Some(W));
    moved(&mut d, &mut f, MAIN);
    loop_end(&mut d, &mut f, LoopEnd::Ended);
    assert_eq!(leave(&mut d, &mut f, W), Ok(LeaveReply::Detached));
    point(&mut f, OVER_MAIN_STRIP, Some(MAIN));
    moved(&mut d, &mut f, SPARE);
    loop_end(&mut d, &mut f, LoopEnd::Ended);
    assert_eq!(f.tabs(MAIN), vec![ids.m0]);
    assert_eq!(leave(&mut d, &mut f, MAIN), Ok(LeaveReply::Detached));
    let refill = d
        .session
        .as_ref()
        .unwrap()
        .main_refill
        .expect("메인을 비운 떼기가 채운 빈 탭");
    assert_eq!(f.tabs(MAIN), vec![refill]);
    f.take();
    (d, f, ids, refill)
}

#[test]
fn p13_reattaching_to_main_closes_the_blank_tab_its_redetach_left() {
    let (mut d, mut f, ids, refill) = redetached_from_main();
    point(&mut f, OVER_MAIN_STRIP, Some(MAIN));
    moved(&mut d, &mut f, SPARE);
    f.take();
    loop_end(&mut d, &mut f, LoopEnd::Ended);
    assert_eq!(
        f.take(),
        vec![
            dock(ids.m0, tab_bar(MAIN, 0), FocusPolicy::Reveal, None),
            Call::GiveBack(s(SPARE), spare_measures()),
            Call::Focus(s(MAIN)),
            Call::Feed(s(MAIN), attach_feed(1, ids.m0, 300.0, 10.0)),
            Call::PollStart(s(MAIN), 3),
            Call::CloseTab(s(MAIN), refill),
        ]
    );
    pointer(&mut d, &mut f, OVER_MAIN_STRIP, false);
    end(&mut d, &mut f, MAIN, EndHow::Drop, EndSrc::Release, Some(0));
    assert_eq!(d.session_id(), None);
    assert_eq!(f.tabs(MAIN), vec![ids.m0], "G14 — 빈 탭이 남지 않는다");
}

#[test]
fn p13_esc_after_redetaching_main_last_tab_closes_the_blank_tab_it_left() {
    let (mut d, mut f, ids, refill) = redetached_from_main();
    loop_end(&mut d, &mut f, LoopEnd::Canceled);
    let mut want = vec![
        dock(ids.m0, tab_bar(MAIN, 0), FocusPolicy::Keep, None),
        Call::GiveBack(s(SPARE), spare_measures()),
        Call::CloseTab(s(MAIN), refill),
        Call::Restore(s(MAIN), main_start(), None, true),
    ];
    want.extend(finish_calls(
        1,
        Some(MAIN),
        None,
        Some(Call::Focus(s(MAIN))),
    ));
    assert_eq!(f.take(), want);
    assert_eq!(f.tabs(MAIN), vec![ids.m0], "G14 — 빈 탭이 남지 않는다");
}

#[test]
fn ids_count_up_across_sessions() {
    let (mut f, ids) = fake(2);
    let mut d = Drag::new();
    begin(&mut d, &mut f, MAIN, ids.m0, Some(ids.m1));
    end(&mut d, &mut f, MAIN, EndHow::Drop, EndSrc::Pointer, None);
    assert_eq!(d.session_id(), None);
    let reply = begin(&mut d, &mut f, MAIN, ids.m0, Some(ids.m1));
    assert!(
        matches!(reply, BeginReply::Strip { session: 2, .. }),
        "{reply:?}"
    );
    assert_eq!(f.mgr.lease().map(|l| l.0), Some(LeaseId(2)));
}
