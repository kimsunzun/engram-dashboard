//! 탭 끌기 세션 포트의 셸 어댑터와 실행기 세우기(TRD S21-slot-docking §2-4) — 세션 포트(`dock::ports::Ports`)를
//! 레이아웃 적용 서비스(`layout::apply`) · 대기 창 명부(`DockState`) · 창 손(`commands::drag_window`) · 웹뷰 사건 ·
//! 시한 · 폴러로 채운다. 진입점 = [`install`] — 사건 줄 · 시한 · 폴러 · 포트를 짓고 세션을 메인 스레드의 받는 쪽으로
//! 세운다.
//!
//! ★락 규율★ — 포트 계약(`ports` 머리) 그대로:
//! - 레이아웃 락 · 명부 락은 포트 호출 하나 안에서 잠깐 쥐고 놓는다. 둘을 함께 쥐는 것은 세계 뜨기(`world`) 하나이고
//!   차례는 레이아웃 → 명부다(`DockState` 문서).
//! - 쥔 채 창 손 · emit · 창 짓기 · 줄 넣기를 하지 않는다 — 창 손의 호출은 그 자리에서 사건(`Moved` · 루프 끝)을 낳고,
//!   메인 스레드의 줄 넣기는 세션을 그 자리에서 돌릴 수 있다(`exec::Inbox::post`).
//! - 레이아웃 쓰기는 적용 서비스로만 한다 — 사람 · LLM 명령과 같은 길(ADR-0081 · CLAUDE.md 「LLM-우선 제어」). 표식
//!   (lease)만 모델을 직접 부른다 — 적용 서비스에 그 길이 없고 레이아웃 판(`version`)을 바꾸지 않는다(`commands::dock`
//!   의 시작 · 끝과 같은 길).
// ADR-0295

use std::fmt::Display;
use std::sync::Arc;
use std::time::Duration;

use engram_dashboard_base::sync;
use engram_dashboard_platform::input;
use tauri::{AppHandle, Emitter, EventTarget, Manager, State};

use crate::commands::dock::{spawn_spare_build, DOCK_OVERLAY_LABEL};
use crate::commands::drag_window::{self, DragWindows};
use crate::commands::layout::{RouterSubs, TauriEvents};
use crate::commands::popout::{PopupCounter, TauriWindowHost};
use crate::daemon_client::DaemonClient;
use crate::dock::exec::{Driver, Inbox, InputSource, Job, MainThread, Poller, Timer};
use crate::dock::parked::ParkedRole;
use crate::dock::ports::{
    DetachPlan, DockWrite, Placement, Ports, WholePlan, RAISE_AFTER_MS, RELEASE_CONFIRM_MS,
};
use crate::dock::session::Event;
use crate::dock::targets::Overlay;
use crate::dock::units::{Monitor, PhysPoint, WindowGeom};
use crate::dock::wire::{SpareEvent, SpareStep, TabDragEvent, TabDragFeed};
use crate::dock::world::{self, World};
use crate::dock::DockState;
use crate::layout::apply::{self, DockOutcome};
use crate::layout::manager::{DockError, FrameMeasures, LeaseId, ViewId, WindowLabel};
use crate::layout::{LayoutState, WindowHost};
use crate::output_router::OutputRouter;

/// 끌기 세션이 섰다 · 끝났다 — 몸 = [`TabDragEvent`]. 받는 쪽 = 우리 웹뷰(안내 창 제외).
pub const EVT_DRAG: &str = "dock:drag";
/// 끄는 탭을 쥔 창 웹뷰 하나에 셸이 미는 단계 — 몸 = [`TabDragFeed`].
pub const EVT_FEED: &str = "dock:feed";
/// 대기 창 명부의 창 모드가 바뀌었다 — 몸 = [`SpareEvent`] · 받는 쪽 = 그 창 웹뷰(웹뷰 쪽 이름 = `dockSession.ts` 의
/// `EVT_SPARE`).
pub const EVT_SPARE: &str = "dock:spare";

const SHELL_STATE_MISSING: &str =
    "끌기 쓰기에 쓸 셸 상태(출력 라우터 · 데몬 클라이언트 · 창 label 발급기)가 등록돼 있지 않다";

/// 끌기 실행기를 세운다 — ★메인 스레드(Tauri `setup`)에서 한 번★ 부른다(세션은 부른 스레드에 산다 — `exec` 머리).
/// 도킹 상태(`Arc<DockState>`) · 레이아웃 상태를 먼저 `manage` 해 두어야 한다. 돌려준 줄은 부르는 쪽이 `manage` 하고
/// invoke · 창 사건을 넣는다. `Err` = 그 상태가 없다 · 시한 스레드를 못 띄웠다.
// ADR-0295
pub fn install(app: &AppHandle) -> Result<Inbox, String> {
    let (Some(dock), Some(layout)) = (
        app.try_state::<Arc<DockState>>(),
        app.try_state::<LayoutState>(),
    ) else {
        return Err("도킹 상태 · 레이아웃 상태가 등록돼 있지 않다".to_owned());
    };
    let inbox = Inbox::new(TauriMain(app.clone()));
    let timer =
        Timer::new(inbox.clone()).map_err(|e| format!("끌기 시한 스레드를 못 띄웠다: {e}"))?;
    let ports = ShellPorts {
        app: app.clone(),
        layout: layout.inner().clone(),
        dock: Arc::clone(dock.inner()),
        windows: DragWindows::new(app.clone(), inbox.clone()),
        timer,
        poller: Poller::new(inbox.clone(), Arc::new(PlatformInput)),
    };
    inbox.install(Driver::new(ports));
    tracing::info!(module = "dock", "탭 끌기 실행기를 세웠다");
    Ok(inbox)
}

struct TauriMain(AppHandle);

impl MainThread for TauriMain {
    fn run(&self, job: Job) {
        if let Err(e) = self.0.run_on_main_thread(job) {
            tracing::warn!(
                module = "dock",
                "끌기 사건 비우기를 메인 스레드에 못 올렸다 — 사건은 줄에 남는다: {e}"
            );
        }
    }
}

struct PlatformInput;

impl InputSource for PlatformInput {
    fn cursor(&self) -> PhysPoint {
        drag_window::cursor()
    }

    fn primary_down(&self) -> bool {
        input::primary_button_down()
    }
}

struct ShellPorts {
    app: AppHandle,
    layout: LayoutState,
    dock: Arc<DockState>,
    windows: DragWindows,
    timer: Timer,
    poller: Poller,
}

impl ShellPorts {
    // 데몬 클라이언트를 못 만든 셸은 다른 레이아웃 명령처럼 끌기 쓰기도 실패한다(빌더 · setup 이 등록한다).
    fn subs(&self) -> Option<(State<'_, Arc<OutputRouter>>, State<'_, Arc<DaemonClient>>)> {
        Some((self.app.try_state()?, self.app.try_state()?))
    }

    fn spare_step(&self, label: &str, step: SpareStep) {
        if let Err(e) = self.app.emit_to(label, EVT_SPARE, SpareEvent { step }) {
            emit_failed(EVT_SPARE, Some(label), e);
        }
    }
}

impl Ports for ShellPorts {
    // ── 레이아웃 ──

    fn world(&mut self) -> World {
        let windows = &self.windows;
        assemble_world(
            &self.layout,
            &self.dock,
            |label| windows.geom(label),
            || windows.monitors(),
        )
    }

    fn lease_tab(&mut self, view: ViewId) -> Result<LeaseId, DockError> {
        sync::lock(&self.layout.0).lease_tab(view)
    }

    fn release_lease(&mut self, lease: LeaseId) {
        sync::lock(&self.layout.0).release_lease(lease);
    }

    fn dock(&mut self, write: DockWrite) -> Result<DockOutcome, DockError> {
        let (Some((router, client)), Some(labels)) =
            (self.subs(), self.app.try_state::<Arc<PopupCounter>>())
        else {
            return Err(DockError::WindowOpen(SHELL_STATE_MISSING.to_owned()));
        };
        apply::dock_tab(
            &self.layout,
            &RouterSubs {
                router: &router,
                client: &client,
            },
            &TauriEvents { app: &self.app },
            &TauriWindowHost { app: &self.app },
            &**labels,
            write.into_request(),
        )
    }

    fn switch_tab(&mut self, window: &str, view: ViewId) {
        let done = match self.subs() {
            Some((router, client)) => apply::switch_tab(
                &self.layout,
                &RouterSubs {
                    router: &router,
                    client: &client,
                },
                &TauriEvents { app: &self.app },
                window,
                view,
            ),
            None => Err(SHELL_STATE_MISSING.to_owned()),
        };
        if let Err(e) = done {
            layout_write_failed("switch_tab", window, view, e);
        }
    }

    fn close_tab(&mut self, window: &str, view: ViewId) {
        let done = match self.subs() {
            Some((router, client)) => apply::close_tab(
                &self.layout,
                &RouterSubs {
                    router: &router,
                    client: &client,
                },
                &TauriEvents { app: &self.app },
                &TauriWindowHost { app: &self.app },
                window,
                view,
            ),
            None => Err(SHELL_STATE_MISSING.to_owned()),
        };
        if let Err(e) = done {
            layout_write_failed("close_tab", window, view, e);
        }
    }

    // ── 창 ──

    fn cursor(&mut self) -> PhysPoint {
        self.windows.cursor()
    }

    fn probe(&mut self, at: PhysPoint, skip: &str) -> Option<WindowLabel> {
        self.windows.probe(at, skip)
    }

    fn geom(&mut self, label: &str) -> Option<WindowGeom> {
        self.windows.geom(label)
    }

    fn place_detached(&mut self, label: &str, plan: &DetachPlan, cursor: PhysPoint) {
        self.windows.place_detached(label, plan, cursor);
    }

    fn place_whole(&mut self, label: &str, plan: WholePlan) {
        self.windows.place_whole(label, plan);
    }

    fn restore_placement(
        &mut self,
        label: &str,
        start: Placement,
        behind: Option<&str>,
        activate: bool,
    ) {
        self.windows
            .restore_placement(label, start, behind, activate);
    }

    fn hide(&mut self, label: &str) {
        self.windows.hide(label);
    }

    fn show_in_place(&mut self, label: &str) {
        self.windows.show_in_place(label);
    }

    fn focus(&mut self, label: &str) {
        self.windows.focus(label);
    }

    fn raise_no_activate(&mut self, label: &str) {
        self.windows.raise_no_activate(label);
    }

    fn begin_move_loop(
        &mut self,
        label: &str,
        hide_on_escape: bool,
        session: u64,
        loop_no: u32,
    ) -> Result<(), String> {
        self.windows
            .begin_move_loop(label, hide_on_escape, session, loop_no)
    }

    fn end_move_loop(&mut self, label: &str) {
        self.windows.end_move_loop(label);
    }

    fn close_window(&mut self, label: &str) {
        self.windows.close_window(label);
    }

    fn record_placement(&mut self, label: &str) {
        TauriWindowHost { app: &self.app }.record_placement(label);
    }

    fn overlay(&mut self, overlay: Option<Overlay>) {
        self.windows.overlay(overlay);
    }

    // ── 대기 창 명부 ──

    fn unpark(&mut self, label: &str) {
        let was_parked = self.dock.parked().remove(label).is_some();
        tracing::debug!(
            module = "dock",
            window = %label,
            was_parked,
            "모델에 들인 창을 명부에서 뺐다"
        );
        self.spare_step(label, SpareStep::Adopted);
    }

    fn give_back(&mut self, label: &str, measures: FrameMeasures) {
        self.windows.give_back(label);
        self.dock.park(label, ParkedRole::Spare, measures);
        tracing::debug!(
            module = "dock",
            window = %label,
            "비운 떼어진 창을 예비로 되돌렸다"
        );
        self.spare_step(label, SpareStep::Parked);
    }

    fn park_kept(&mut self, label: &str, measures: FrameMeasures) {
        self.dock.park(label, ParkedRole::Kept, measures);
        tracing::debug!(
            module = "dock",
            window = %label,
            "비운 출처 창을 남긴 창으로 명부에 뒀다"
        );
        self.spare_step(label, SpareStep::Parked);
    }

    fn fill_spare(&mut self) {
        let has_spare = self.dock.parked().has_spare();
        if !has_spare {
            spawn_spare_build(&self.app);
        }
    }

    // ── 웹뷰 사건 ──

    fn drag(&mut self, event: TabDragEvent, to: Option<&str>) {
        let sent = match to {
            None => self.app.emit_filter(EVT_DRAG, event, beside_overlay),
            Some(label) => self.app.emit_to(label, EVT_DRAG, event),
        };
        if let Err(e) = sent {
            emit_failed(EVT_DRAG, to, e);
        }
    }

    fn feed(&mut self, to: &str, feed: TabDragFeed) {
        if let Err(e) = self.app.emit_to(to, EVT_FEED, feed) {
            emit_failed(EVT_FEED, Some(to), e);
        }
    }

    // ── 시한 · 폴러 ──

    fn arm_raise(&mut self, seq: u32) {
        self.timer.arm(
            Duration::from_millis(RAISE_AFTER_MS),
            Event::RaiseDue { seq },
        );
    }

    fn arm_release(&mut self, seq: u32) {
        self.timer.arm(
            Duration::from_millis(RELEASE_CONFIRM_MS),
            Event::ReleaseDue { seq },
        );
    }

    fn poll_start(&mut self, label: &str, gen: u32) {
        self.poller.start(label, gen);
    }

    fn poll_stop(&mut self) {
        self.poller.stop();
    }
}

// OS 기하 · 모니터는 두 락을 다 놓은 뒤에 읽는다 — 블록을 넓혀 락 안으로 들이지 말 것(`ports` 머리).
fn assemble_world(
    layout: &LayoutState,
    dock: &DockState,
    mut geom: impl FnMut(&str) -> Option<WindowGeom>,
    monitors: impl FnOnce() -> Vec<Monitor>,
) -> World {
    let mut world = {
        let mgr = sync::lock(&layout.0);
        let parked = dock.parked();
        world::snapshot(&mgr, &parked)
    };
    for w in &mut world.windows {
        w.geom = geom(&w.label);
    }
    world.monitors = monitors();
    world
}

// 안내 창을 대상으로 건 리스너만 거른다 — 대상 없이(`Any`) 건 리스너는 Tauri 가 거르개와 무관하게 늘 넣는다(tauri
//   2.11.3 `event/listener.rs` 의 `match_any_or_filter`). 웹뷰는 자기 창 대상으로 듣는다(`dock:spare` 와 같다).
fn beside_overlay(target: &EventTarget) -> bool {
    match target {
        EventTarget::AnyLabel { label }
        | EventTarget::Window { label }
        | EventTarget::Webview { label }
        | EventTarget::WebviewWindow { label } => label != DOCK_OVERLAY_LABEL,
        _ => true,
    }
}

fn emit_failed(event: &'static str, to: Option<&str>, e: impl Display) {
    tracing::warn!(module = "dock", event, to, "끌기 사건 emit 실패: {e}");
}

fn layout_write_failed(op: &'static str, window: &str, view: ViewId, e: impl Display) {
    tracing::warn!(
        module = "dock",
        op,
        window = %window,
        view = %view,
        "끌기 레이아웃 쓰기 실패: {e}"
    );
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::dock::units::PhysRect;
    use crate::layout::MAIN_WINDOW_LABEL;

    const POPUP: &str = "slot-popup-1";
    const SPARE: &str = "slot-popup-9";

    fn geom() -> WindowGeom {
        WindowGeom {
            outer: PhysRect {
                x0: 0,
                y0: 0,
                x1: 800,
                y1: 600,
            },
            inner: PhysRect {
                x0: 8,
                y0: 31,
                x1: 792,
                y1: 592,
            },
            scale: 1.25,
        }
    }

    fn monitor() -> Monitor {
        let rect = PhysRect {
            x0: 0,
            y0: 0,
            x1: 1920,
            y1: 1080,
        };
        Monitor {
            rect,
            work: PhysRect { y1: 1040, ..rect },
            scale: 1.25,
        }
    }

    #[test]
    fn the_world_is_taken_under_both_locks_and_filled_after_both_are_released() {
        let layout = LayoutState::new();
        sync::lock(&layout.0).create_window(POPUP).unwrap();
        let dock = DockState::new();
        dock.parked().reserve_spare(SPARE.to_owned());
        let mut asked = Vec::new();

        let world = assemble_world(
            &layout,
            &dock,
            |label| {
                assert!(layout.0.try_lock().is_ok(), "레이아웃 락을 놓고 읽는다");
                assert!(dock.parked_unlocked(), "명부 락을 놓고 읽는다");
                asked.push(label.to_owned());
                (label == MAIN_WINDOW_LABEL).then(geom)
            },
            || {
                assert!(layout.0.try_lock().is_ok());
                assert!(dock.parked_unlocked());
                vec![monitor()]
            },
        );

        assert_eq!(asked, [MAIN_WINDOW_LABEL, POPUP], "모델 창마다 label 순");
        assert_eq!(world.window(MAIN_WINDOW_LABEL).unwrap().geom, Some(geom()));
        assert_eq!(world.window(POPUP).unwrap().geom, None, "못 읽은 창");
        assert_eq!(world.monitors, [monitor()]);
        assert!(world.parked.contains(SPARE));
        assert!(
            world.window(SPARE).is_none(),
            "명부의 창은 기하를 묻지 않는다"
        );
    }

    #[test]
    fn a_session_wide_drag_event_skips_only_listeners_aimed_at_the_overlay() {
        for aimed in [
            EventTarget::labeled(DOCK_OVERLAY_LABEL),
            EventTarget::window(DOCK_OVERLAY_LABEL),
            EventTarget::webview(DOCK_OVERLAY_LABEL),
            EventTarget::webview_window(DOCK_OVERLAY_LABEL),
        ] {
            assert!(!beside_overlay(&aimed), "{aimed:?}");
        }
        for other in [
            EventTarget::labeled(MAIN_WINDOW_LABEL),
            EventTarget::webview_window(SPARE),
            EventTarget::app(),
            EventTarget::any(),
        ] {
            assert!(beside_overlay(&other), "{other:?}");
        }
    }

    #[test]
    fn the_spare_event_body_is_the_step_word_the_webview_reads() {
        assert_eq!(
            serde_json::to_value(SpareEvent {
                step: SpareStep::Adopted
            })
            .unwrap(),
            json!({"step": "adopted"})
        );
        assert_eq!(
            serde_json::to_value(SpareEvent {
                step: SpareStep::Parked
            })
            .unwrap(),
            json!({"step": "parked"})
        );
    }
}
