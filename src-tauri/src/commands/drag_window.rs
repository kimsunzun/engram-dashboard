//! 끌기 세션 포트의 창 쪽 손(TRD S21-slot-docking §2-4 · §8) — 세션 포트(`dock::ports::Ports`)의 창 메서드가 같은
//! 이름으로 부르는 함수 묶음. OS 일은 platform 함수(`window` · `move_loop` · `input`)에 넘기고, 여기는 label → 창 ·
//! 단위 옮기기 · tao 보임 깃발 맞추기만 한다(ADR-0266 — OS 가름은 platform 안에만).
//!
//! - 메인 스레드에서 동기로, 어느 락도 쥐지 않고 부른다. 돌아올 때 창 놓기가 끝났다 — Tauri 창 호출 · 게터는 메인
//!   스레드에서 부르면 이벤트 루프 줄을 거치지 않고 그 자리에서 돈다(tauri-runtime-wry 2.11.3 `send_user_message`).
//! - 보임 — 숨은 창을 날 Win32 로 보이거나 되돌리면 뒤이어 Tauri `show()` 로 tao 보임 깃발을 참으로 맞춘다(깃발이
//!   이미 참이면 tao 는 아무것도 하지 않는다 — tao 0.35.3 `apply_diff`). 붙기를 정할 때 끄는 창을 숨기는
//!   [`DragWindows::hide`] 는 날 `SW_HIDE`(스파이크 S4 가 잰 길)라 깃발이 참으로 남고, 그 창이 예비로 돌아가면
//!   [`DragWindows::give_back`] 의 Tauri `hide()` 가 깃발을 거짓으로 맞춘다. 활성은 platform 으로만 준다 — Tauri
//!   `set_focus` 를 쓰지 않는다(`ports` 머리 · TRD §6-3 F7 a).
//! - 안내 창에는 tao · Tauri 깃발 setter 를 부르지 않는다 — 보이기 · 숨기기 · 자리 · z 는 platform 함수로만 한다(TRD
//!   §2-5 「tao 깃발 불변식」 · `commands::dock` 의 안내 창 짓기).
//! - 이동 루프 알림은 사건 줄에 넣기만 한다 — 그 알림은 `begin_move_loop` · `end_move_loop` · 루프 도는 창을 부수는
//!   호출 안에서도 동기로 온다(platform `move_loop` 머리).
//! - 실패는 warn 한 줄을 남기고 돌아온다 — 결과를 돌려주는 것은 [`DragWindows::begin_move_loop`] 하나다(세션이 그
//!   실패로 갈린다). 읽기([`DragWindows::geom`] · [`DragWindows::probe`])는 없는 창이면 `None` 이다.
// ADR-0295

use std::fmt::Display;

use engram_dashboard_platform::input;
use engram_dashboard_platform::move_loop::{self, LoopEvent, MoveLoopEnd, MoveLoopOpts};
use engram_dashboard_platform::window::{self as os, NativeWindow, ZOrder};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use crate::commands::dock::DOCK_OVERLAY_LABEL;
use crate::dock::exec::Inbox;
use crate::dock::ports::{DetachPlan, Placement, WholePlan};
use crate::dock::session::{Event, LoopEnd};
use crate::dock::sizing::monitor_near;
use crate::dock::targets::{Overlay, OverlayView};
use crate::dock::units::{Monitor, PhysPoint, PhysRect, PhysSize, WindowGeom};
use crate::layout::chrome::RectCss;
use crate::layout::manager::WindowLabel;

/// 안내 창 그림 사건 — 몸 = 안내 창 CSS px 의 [`OverlayView`] · `null` = 치웠다. 받는 쪽 = 안내 창 웹뷰.
pub const EVT_OVERLAY: &str = "dock:overlay";

/// 끌기 세션의 창 손 — 앱 손잡이와 사건 줄(이동 루프 끝 알림을 넣는 곳)을 쥔다. 메서드는 `Ports` 의 같은 이름
/// 메서드 하나씩에 맞는다.
pub struct DragWindows {
    app: AppHandle,
    inbox: Inbox,
}

impl DragWindows {
    pub fn new(app: AppHandle, inbox: Inbox) -> Self {
        Self { app, inbox }
    }

    pub fn cursor(&self) -> PhysPoint {
        cursor()
    }

    /// 모니터 목록 — 세계(`World::monitors`)의 재료. 못 읽으면 warn 한 줄 뒤 빈 목록(「가장 가까운 모니터」가 없다).
    pub fn monitors(&self) -> Vec<Monitor> {
        match self.app.available_monitors() {
            Ok(list) => list
                .iter()
                .map(|m| {
                    let work = m.work_area();
                    Monitor {
                        rect: phys_rect(
                            m.position().x,
                            m.position().y,
                            m.size().width,
                            m.size().height,
                        ),
                        work: phys_rect(
                            work.position.x,
                            work.position.y,
                            work.size.width,
                            work.size.height,
                        ),
                        scale: m.scale_factor(),
                    }
                })
                .collect(),
            Err(e) => {
                tracing::warn!(module = "dock", "모니터 목록을 못 읽었다: {e}");
                Vec::new()
            }
        }
    }

    /// 점 밑 맨 위 우리 창 — `skip`(끄는 창)과 안내 창을 건너뛴다. 보이지 않거나 최소화된 창은 platform
    /// `top_level_at` 이 건너뛴다. 맨 위가 다른 앱 창 · 바탕화면이면 `None`(가려진 자리 — PRD R15).
    // ADR-0295
    pub fn probe(&self, at: PhysPoint, skip: &str) -> Option<WindowLabel> {
        let windows = self.app.webview_windows();
        let skips: Vec<NativeWindow> = [skip, DOCK_OVERLAY_LABEL]
            .iter()
            .filter_map(|label| windows.get(*label).and_then(NativeWindow::of))
            .collect();
        let hit = os::top_level_at(os_point(at), &skips)?;
        windows
            .into_iter()
            .find(|(label, w)| label != DOCK_OVERLAY_LABEL && NativeWindow::of(w) == Some(hit))
            .map(|(label, _)| label)
    }

    /// 그 창의 OS 기하(물리 px) — 없거나 · 숨었거나 · 최소화된 창은 `None`(세계의 「그 창은 띠도 붙기도 없다」).
    pub fn geom(&self, label: &str) -> Option<WindowGeom> {
        const OP: &str = "geom";
        let window = self.app.get_webview_window(label)?;
        if !os::is_window_visible(&window) {
            return None;
        }
        match window.is_minimized() {
            Ok(false) => {}
            Ok(true) => return None,
            Err(e) => {
                failed(OP, label, e);
                return None;
            }
        }
        match read_geom(&window) {
            Ok(geom) => Some(geom),
            Err(e) => {
                failed(OP, label, e);
                None
            }
        }
    }

    /// 숨은 채 들인 떼어진 창을 놓고 보이고 활성으로(뜻 = `Ports::place_detached`). 차례 = 숨은 채 platform
    /// `place_window(show: false)` → 놓은 창 배율이 셈에 쓴 배율과 다르면 한 번 더 → Tauri `show()` → platform 활성.
    // ADR-0295
    pub fn place_detached(&self, label: &str, plan: &DetachPlan, cursor: PhysPoint) {
        const OP: &str = "place_detached";
        let Some(window) = self.window(OP, label) else {
            return;
        };
        let before = match read_geom(&window) {
            Ok(geom) => geom,
            Err(e) => {
                failed(OP, label, e);
                return;
            }
        };
        let monitors = self.monitors();
        let scale = cursor_scale(&monitors, cursor).unwrap_or(before.scale);
        let rect = plan.outer_rect(cursor, &monitors, scale, before.frame_offset());
        if let Err(e) = os::place_window(&window, Some(os_rect(rect)), ZOrder::Keep, false, false) {
            failed(OP, label, e);
            return;
        }
        // 배율이 다른 모니터로 옮기면 OS 가 배율을 바꾸고, tao 는 그 처리에서 크기를 「옛 크기 ÷ 옛 배율 × 새 배율」로
        //   다시 놓으며 틀 두께도 바뀐다(TRD §5-5 E3) — 바뀐 배율 · 틀 두께로 한 번 더 놓는다. 숨은 창이 놓는 자리에서
        //   배율을 받는지는 재지 않았다(M1) — 받지 않으면 다시 놓지 않고, 새면 증상 = 보일 때 크기가 배율 비만큼 어긋난다.
        //   창 배율이 그대로여도 셈에 쓴 배율(커서 모니터)과 다르면 그 배율로 셈한 크기 · 잡은 자리가 틀리므로 같이 다시 놓는다.
        match read_geom(&window) {
            Ok(after) if needs_replace(before.scale, scale, after.scale) => {
                tracing::debug!(
                    module = "dock",
                    window = %label,
                    before = before.scale,
                    computed = scale,
                    after = after.scale,
                    "떼어진 창의 배율이 셈과 달라 다시 놓는다"
                );
                let again = plan.outer_rect(cursor, &monitors, after.scale, after.frame_offset());
                if let Err(e) =
                    os::place_window(&window, Some(os_rect(again)), ZOrder::Keep, false, false)
                {
                    failed(OP, label, e);
                    return;
                }
            }
            Ok(_) => {}
            Err(e) => failed(OP, label, e),
        }
        if let Err(e) = window.show() {
            failed(OP, label, e);
            return;
        }
        if let Err(e) = os::place_window(&window, None, ZOrder::Keep, false, true) {
            failed(OP, label, e);
        }
    }

    pub fn place_whole(&self, label: &str, plan: WholePlan) {
        const OP: &str = "place_whole";
        let Some(window) = self.window(OP, label) else {
            return;
        };
        let placed = match plan {
            WholePlan::Restore(rect) => {
                os::restore_placement(&window, os_rect(rect), false, None, false)
            }
            WholePlan::Move(origin) => os::move_window(&window, os_point(origin)),
        };
        if let Err(e) = placed {
            failed(OP, label, e);
        }
    }

    /// 뜻 = `Ports::restore_placement` — 숨어 있던 창(되살린 남긴 창)이면 뒤이어 Tauri `show()`.
    pub fn restore_placement(
        &self,
        label: &str,
        start: Placement,
        behind: Option<&str>,
        activate: bool,
    ) {
        const OP: &str = "restore_placement";
        let Some(window) = self.window(OP, label) else {
            return;
        };
        let after = behind
            .and_then(|b| self.app.get_webview_window(b))
            .and_then(|b| NativeWindow::of(&b));
        let was_hidden = !os::is_window_visible(&window);
        let placed = os::restore_placement(
            &window,
            os_rect(start.normal),
            start.maximized,
            after,
            activate,
        );
        // 실패해도 Tauri `show()` 는 돈다 — platform 이 z 단계에서 실패하면 창은 이미 보이는데, 여기서 돌아가면
        //   tao 보임 깃발이 거짓으로 남는다.
        if was_hidden {
            if let Err(e) = window.show() {
                failed(OP, label, e);
            }
        }
        if let Err(e) = placed {
            failed(OP, label, e);
        }
    }

    /// 날 `SW_HIDE` 로 숨긴다 — 이동 루프 안에서 붙기를 정할 때 끄는 창을 숨기는 길(스파이크 S4). tao 보임 깃발은
    /// 참으로 남는다(모듈 머리 「보임」).
    pub fn hide(&self, label: &str) {
        const OP: &str = "hide";
        let Some(window) = self.window(OP, label) else {
            return;
        };
        if let Err(e) = os::hide_window(&window) {
            failed(OP, label, e);
        }
    }

    /// 자리 · 크기 · z 를 두고 활성화 없이 보인다 — 이미 보이면 무동작.
    pub fn show_in_place(&self, label: &str) {
        const OP: &str = "show_in_place";
        let Some(window) = self.window(OP, label) else {
            return;
        };
        if os::is_window_visible(&window) {
            return;
        }
        if let Err(e) = os::place_window(&window, None, ZOrder::Keep, true, false) {
            failed(OP, label, e);
            return;
        }
        if let Err(e) = window.show() {
            failed(OP, label, e);
        }
    }

    /// 앞으로 · 활성 — OS 가 전경을 거절하면 그대로 둔다(platform `place_window`).
    pub fn focus(&self, label: &str) {
        const OP: &str = "focus";
        let Some(window) = self.window(OP, label) else {
            return;
        };
        if let Err(e) = os::place_window(&window, None, ZOrder::Keep, false, true) {
            failed(OP, label, e);
        }
    }

    pub fn raise_no_activate(&self, label: &str) {
        const OP: &str = "raise_no_activate";
        let Some(window) = self.window(OP, label) else {
            return;
        };
        if let Err(e) = os::place_window(&window, None, ZOrder::Top, false, false) {
            failed(OP, label, e);
        }
    }

    /// 그 창의 OS 이동 루프를 올린다(뜻 = `Ports::begin_move_loop`). 끝 알림은 그때 커서를 읽어
    /// `Event::LoopEnded` 로 줄에 넣는다. `Err` = 문구 하나(로그는 부른 쪽이 남긴다).
    // ADR-0295
    pub fn begin_move_loop(
        &self,
        label: &str,
        hide_on_escape: bool,
        session: u64,
        loop_no: u32,
    ) -> Result<(), String> {
        let window = self
            .app
            .get_webview_window(label)
            .ok_or_else(|| "창이 없다".to_owned())?;
        let inbox = self.inbox.clone();
        let name = label.to_owned();
        let on_event = Box::new(move |ev: LoopEvent| match ev {
            LoopEvent::Entered => {
                tracing::debug!(module = "dock", session, loop_no, window = %name, "이동 루프에 들어갔다");
            }
            LoopEvent::Ended { end, hid } => {
                tracing::debug!(module = "dock", session, loop_no, window = %name, ?end, hid, "이동 루프 끝 알림");
                inbox.post(Event::LoopEnded {
                    session,
                    loop_no,
                    end: loop_end(end),
                    cursor: cursor(),
                });
            }
        });
        move_loop::begin_move_loop(&window, MoveLoopOpts { hide_on_escape }, on_event)
            .map_err(|e| e.to_string())
    }

    pub fn end_move_loop(&self, label: &str) {
        let Some(window) = self.window("end_move_loop", label) else {
            return;
        };
        move_loop::end_move_loop(&window);
    }

    pub fn close_window(&self, label: &str) {
        const OP: &str = "close_window";
        let Some(window) = self.window(OP, label) else {
            return;
        };
        if let Err(e) = window.close() {
            failed(OP, label, e);
        }
    }

    /// 뜻 = `Ports::overlay` — `Some` = 대상 창 안쪽 사각형을 덮게 항상 위 · 활성화 없이 놓고 보인 뒤(배율이 바뀌었으면
    /// 한 번 더) 그림을 안내 창 CSS px 로 옮겨 민다 · `None` = 보이면 숨기고 `null` 을 민다(다음에 보일 때 지난 그림이
    /// 남지 않게).
    // ADR-0295
    pub fn overlay(&self, overlay: Option<Overlay>) {
        match overlay {
            Some(overlay) => self.draw_overlay(overlay),
            None => self.clear_overlay(),
        }
    }

    /// 예비로 되돌리는 창의 창 몫 — Tauri `hide()`. 이미 숨었어도 부른다: 날 `SW_HIDE`(붙기 · 루프 Esc 훅)로 숨은
    /// 창은 tao 보임 깃발이 참이라 맞춰 두어야 다음 들이기의 Tauri `show()` 가 돈다.
    pub fn give_back(&self, label: &str) {
        const OP: &str = "give_back";
        let Some(window) = self.window(OP, label) else {
            return;
        };
        if let Err(e) = window.hide() {
            failed(OP, label, e);
        }
    }

    fn draw_overlay(&self, overlay: Overlay) {
        const OP: &str = "overlay";
        let Some(guide) = self.window(OP, DOCK_OVERLAY_LABEL) else {
            return;
        };
        let Some(target) = self.window(OP, &overlay.over) else {
            return;
        };
        let (Some(target_scale), Some(before)) = (scale_of(OP, &target), scale_of(OP, &guide))
        else {
            return;
        };
        let rect = Some(os_rect(overlay.at));
        if let Err(e) = os::place_window(&guide, rect, ZOrder::Topmost, true, false) {
            failed(OP, DOCK_OVERLAY_LABEL, e);
            return;
        }
        let Some(after) = scale_of(OP, &guide) else {
            return;
        };
        // 배율이 바뀌면 tao 가 그 처리에서 안내 창 크기를 다시 놓는다(TRD §2-5) — 같은 사각형을 한 번 더 입힌다.
        if scale_moved(before, after) {
            tracing::debug!(
                module = "dock",
                over = %overlay.over,
                before,
                after,
                "안내 창의 배율이 바뀌어 다시 놓는다"
            );
            if let Err(e) = os::place_window(&guide, rect, ZOrder::Topmost, true, false) {
                failed(OP, DOCK_OVERLAY_LABEL, e);
                return;
            }
        }
        self.emit_overlay(Some(overlay_css(&overlay.view, target_scale, after)));
    }

    fn clear_overlay(&self) {
        let Some(guide) = self.app.get_webview_window(DOCK_OVERLAY_LABEL) else {
            return;
        };
        if !os::is_window_visible(&guide) {
            return;
        }
        if let Err(e) = os::hide_window(&guide) {
            failed("overlay", DOCK_OVERLAY_LABEL, e);
        }
        self.emit_overlay(None);
    }

    fn emit_overlay(&self, view: Option<OverlayView>) {
        if let Err(e) = self.app.emit_to(DOCK_OVERLAY_LABEL, EVT_OVERLAY, view) {
            failed("overlay", DOCK_OVERLAY_LABEL, e);
        }
    }

    fn window(&self, op: &'static str, label: &str) -> Option<WebviewWindow> {
        let window = self.app.get_webview_window(label);
        if window.is_none() {
            failed(op, label, "창이 없다");
        }
        window
    }
}

/// 화면의 커서(물리 px) — 어느 스레드에서나. OS 가 답하지 않으면(보안 데스크톱 따위) warn 한 줄 뒤 `(0, 0)`.
pub fn cursor() -> PhysPoint {
    match input::cursor_pos() {
        Some(p) => PhysPoint { x: p.x, y: p.y },
        None => {
            tracing::warn!(module = "dock", "커서 자리를 못 읽었다 — (0, 0) 으로 친다");
            PhysPoint::default()
        }
    }
}

fn failed(op: &'static str, label: &str, e: impl Display) {
    tracing::warn!(module = "dock", op, window = %label, "끌기 창 호출 실패: {e}");
}

fn scale_of(op: &'static str, window: &WebviewWindow) -> Option<f64> {
    window
        .scale_factor()
        .map_err(|e| failed(op, window.label(), e))
        .ok()
}

fn read_geom(window: &WebviewWindow) -> tauri::Result<WindowGeom> {
    let (outer_at, outer) = (window.outer_position()?, window.outer_size()?);
    let (inner_at, inner) = (window.inner_position()?, window.inner_size()?);
    Ok(WindowGeom {
        outer: phys_rect(outer_at.x, outer_at.y, outer.width, outer.height),
        inner: phys_rect(inner_at.x, inner_at.y, inner.width, inner.height),
        scale: window.scale_factor()?,
    })
}

fn loop_end(end: MoveLoopEnd) -> LoopEnd {
    match end {
        MoveLoopEnd::Released { .. } => LoopEnd::Released,
        MoveLoopEnd::Ended => LoopEnd::Ended,
        MoveLoopEnd::Canceled { .. } => LoopEnd::Canceled,
        MoveLoopEnd::Destroyed => LoopEnd::Destroyed,
    }
}

// 배율은 DPI ÷ 96 이라 같은 배율이면 같은 값이 온다 — 문턱은 셈 잡음을 같다고 치려는 것뿐이다.
fn scale_moved(before: f64, after: f64) -> bool {
    (before - after).abs() > 1e-9
}

// 떼어진 창을 한 번 더 놓을지 — 놓은 뒤 창 배율이 놓기 전과 달라졌거나, 놓기 전과 같아도 셈에 쓴 배율(커서 모니터)과
//   다르면 놓는다. 뒤의 것 = 창 배율은 그대로인데 커서 모니터 배율로 크기 · 잡은 자리를 셈한 경우(노트북 150% + 외부
//   100% — 그대로 두면 창이 배율 비만큼 크거나 작게 나온다).
fn needs_replace(before: f64, computed: f64, after: f64) -> bool {
    scale_moved(before, after) || scale_moved(computed, after)
}

fn cursor_scale(monitors: &[Monitor], cursor: PhysPoint) -> Option<f64> {
    monitor_near(monitors, cursor).map(|i| monitors[i].scale)
}

// 대상 창 CSS px → 안내 창 CSS px(TRD §2-5) — 둘은 같은 물리 사각형에 놓이므로 대상 배율 ÷ 안내 창 배율만큼 늘인다.
fn overlay_css(view: &OverlayView, target_scale: f64, overlay_scale: f64) -> OverlayView {
    let k = target_scale / overlay_scale;
    let scaled = |r: &RectCss| RectCss {
        x0: r.x0 * k,
        y0: r.y0 * k,
        x1: r.x1 * k,
        y1: r.y1 * k,
    };
    OverlayView {
        bands: view.bands.iter().map(scaled).collect(),
        outer: view.outer.iter().map(scaled).collect(),
        preview: view.preview.as_ref().map(scaled),
    }
}

fn phys_rect(x: i32, y: i32, w: u32, h: u32) -> PhysRect {
    PhysRect::at(
        PhysPoint { x, y },
        PhysSize {
            w: i32::try_from(w).unwrap_or(i32::MAX),
            h: i32::try_from(h).unwrap_or(i32::MAX),
        },
    )
}

fn os_rect(r: PhysRect) -> os::PhysRect {
    let size = r.size();
    os::PhysRect {
        x: r.x0,
        y: r.y0,
        width: size.w,
        height: size.h,
    }
}

fn os_point(p: PhysPoint) -> os::PhysPoint {
    os::PhysPoint { x: p.x, y: p.y }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn css(x0: f64, y0: f64, x1: f64, y1: f64) -> RectCss {
        RectCss { x0, y0, x1, y1 }
    }

    fn monitor(x0: i32, w: i32, scale: f64) -> Monitor {
        let rect = PhysRect {
            x0,
            y0: 0,
            x1: x0 + w,
            y1: 1000,
        };
        Monitor {
            rect,
            work: rect,
            scale,
        }
    }

    #[test]
    fn every_platform_loop_end_lands_on_its_session_end() {
        for (end, want) in [
            (
                MoveLoopEnd::Released {
                    before_entry: false,
                },
                LoopEnd::Released,
            ),
            (
                MoveLoopEnd::Released { before_entry: true },
                LoopEnd::Released,
            ),
            (MoveLoopEnd::Ended, LoopEnd::Ended),
            (MoveLoopEnd::Canceled { esc: true }, LoopEnd::Canceled),
            (MoveLoopEnd::Canceled { esc: false }, LoopEnd::Canceled),
            (MoveLoopEnd::Destroyed, LoopEnd::Destroyed),
        ] {
            assert_eq!(loop_end(end), want, "{end:?}");
        }
    }

    #[test]
    fn only_a_real_scale_change_asks_for_a_second_placement() {
        assert!(!scale_moved(1.25, 1.25));
        assert!(!scale_moved(1.5, 1.5 + 1e-12), "셈 잡음은 같은 배율");
        assert!(scale_moved(1.0, 1.5));
        assert!(scale_moved(2.0, 1.75));
    }

    #[test]
    fn detached_window_is_placed_again_when_its_scale_differs_from_the_computed_one() {
        // (놓기 전, 셈에 쓴 배율, 놓은 뒤) → 다시 놓나
        assert!(!needs_replace(1.0, 1.0, 1.0), "셋 다 같으면 그대로");
        assert!(needs_replace(1.0, 1.5, 1.5), "놓으며 창 배율이 바뀜");
        assert!(
            needs_replace(1.5, 1.0, 1.5),
            "창 배율은 그대로인데 셈에 쓴 배율과 다름"
        );
        assert!(
            needs_replace(1.0, 1.5, 1.0),
            "커서 모니터 배율로 셈했으나 창은 옛 배율"
        );
        assert!(!needs_replace(1.5, 1.5 + 1e-12, 1.5), "셈 잡음은 같은 배율");
    }

    #[test]
    fn overlay_content_stretches_by_target_over_overlay_scale() {
        let view = OverlayView {
            bands: vec![css(10.0, 20.0, 30.0, 40.0)],
            outer: vec![css(0.0, 0.0, 8.0, 100.0), css(92.0, 0.0, 100.0, 100.0)],
            preview: Some(css(0.0, 0.0, 50.0, 100.0)),
        };

        assert_eq!(overlay_css(&view, 1.5, 1.5), view, "같은 배율이면 그대로");

        let stretched = overlay_css(&view, 1.5, 1.0);
        assert_eq!(stretched.bands, [css(15.0, 30.0, 45.0, 60.0)]);
        assert_eq!(
            stretched.outer,
            [css(0.0, 0.0, 12.0, 150.0), css(138.0, 0.0, 150.0, 150.0)]
        );
        assert_eq!(stretched.preview, Some(css(0.0, 0.0, 75.0, 150.0)));

        let shrunk = overlay_css(
            &OverlayView {
                preview: None,
                ..view
            },
            1.0,
            2.0,
        );
        assert_eq!(shrunk.bands, [css(5.0, 10.0, 15.0, 20.0)]);
        assert_eq!(shrunk.preview, None);
    }

    #[test]
    fn the_cursor_monitor_scale_is_the_nearest_monitors() {
        let monitors = [monitor(0, 1920, 1.0), monitor(1920, 2560, 1.5)];
        assert_eq!(
            cursor_scale(&monitors, PhysPoint { x: 2000, y: 10 }),
            Some(1.5)
        );
        assert_eq!(
            cursor_scale(&monitors, PhysPoint { x: -50, y: 10 }),
            Some(1.0),
            "어느 모니터 밖이면 가장 가까운 것"
        );
        assert_eq!(cursor_scale(&[], PhysPoint::default()), None);
    }

    #[test]
    fn rects_cross_between_edge_form_and_origin_size_form() {
        let rect = phys_rect(-1920, 40, 800, 600);
        assert_eq!(
            rect,
            PhysRect {
                x0: -1920,
                y0: 40,
                x1: -1120,
                y1: 640
            }
        );
        assert_eq!(
            os_rect(rect),
            os::PhysRect {
                x: -1920,
                y: 40,
                width: 800,
                height: 600
            }
        );
        assert_eq!(
            os_point(PhysPoint { x: -3, y: 7 }),
            os::PhysPoint { x: -3, y: 7 }
        );
        assert_eq!(
            phys_rect(0, 0, u32::MAX, 5).x1,
            i32::MAX,
            "i32 를 넘는 크기는 한계에서 멈춘다"
        );
    }
}
