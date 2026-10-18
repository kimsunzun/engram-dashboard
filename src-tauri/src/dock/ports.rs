//! 끌기 세션의 포트(TRD S21-slot-docking §2-4) — 세션(`session`)이 바깥(레이아웃 · OS 창 · 대기 창 명부 · 웹뷰
//! 사건 · 시한 · 폴러)에 닿는 유일한 길. Tauri · platform 을 모른다 — 실물 어댑터는 셸이 갖고, 시험은 기록하는
//! 가짜를 쓴다(ADR-0012).
//!
//! ★계약 — 어댑터가 지킬 것★
//! - 세션은 메인 스레드에서만 돈다. 포트는 그 스레드에서 동기로 불리고, 돌아올 때 할 일을 끝냈다(창 놓기 · 쓰기가
//!   반영됐다 — 이어 읽는 `geom` · `world` 가 그 결과를 본다). ★예외 = [`Ports::close_window`]★ — Tauri 창 닫기는
//!   이벤트 루프 프록시를 거쳐 비동기로 돌아, 돌아온 때 창이 아직 남아 있을 수 있다.
//! - 포트는 세션을 다시 부르지 않는다. 포트를 부르는 동안 난 사건(`begin_move_loop` · `end_move_loop` · 창을 없애는
//!   호출 안에서 동기로 오는 루프 끝 알림 · `SetWindowPos` 가 낸 `Moved` · 창 소멸)은 실행기 줄에 넣기만 하고, 지금
//!   사건 처리가 끝난 뒤 처리한다. 그래서 실행기는 줄을 쥔 채(빌린 채) 포트를 부르지 않는다.
//! - 레이아웃 락 · 명부 락을 쥔 채 OS 를 부르거나 실행기 줄에 넣지 않는다(`layout::apply` 머리 「락 규율」 그대로 ·
//!   넣기는 그 자리에서 세션을 돌릴 수 있다 — `exec::Inbox::post`).
//! - 보임 — tao 보임 깃발이 OS 보임과 맞게 한다. 숨은 창을 날 Win32 로 보이거나 되돌렸으면(`show_in_place` ·
//!   `restore_placement`) 뒤이어 Tauri `show()`(깃발이 거짓으로 남으면 트레이 숨기기가 아무것도 하지 않는다). 붙기를
//!   정할 때의 숨기기([`Ports::hide`])는 날 `SW_HIDE`(이동 루프 안에서 잰 길 — 스파이크 S4)라 깃발이 참으로 남는다 —
//!   그 창이 예비로 돌아가면 [`Ports::give_back`] 이 Tauri `hide()` 로 깃발을 거짓으로 맞추고(그래야 다음 들이기의
//!   Tauri `show()` 가 돈다), 남긴 창([`Ports::park_kept`])은 날 숨김 그대로 기다리며 tao 깃발 setter 를 부르지
//!   않는다(깃발이 참이라 setter 하나가 창을 다시 보인다). 안내 창도 지은 뒤 tao 깃발 setter 를 부르지 않고 날
//!   Win32 로만 보이고 숨긴다(`commands::dock` 의 안내 창 짓기).
//! - 실패를 돌려주는 포트는 셋뿐이다(`lease_tab` · `dock` · `begin_move_loop`) — 세션이 그 결과로 갈린다. 나머지는
//!   실패하면 어댑터가 warn 한 줄을 남기고 돌아온다(세션은 이어 간다 · 끝 줄에 셈하지 않는다).
// ADR-0295

use super::sizing::{detached_origin, dragged_size, work_area_near};
use super::targets::Overlay;
use super::units::{Monitor, PhysPoint, PhysRect, SizeDip, WindowGeom};
use super::wire::{TabDragEvent, TabDragFeed};
use super::world::World;
use crate::layout::apply::{DockDest, DockOutcome, DockRequest, OnEmptied};
use crate::layout::chrome::PointCss;
use crate::layout::dock::{DockAnchor, DockSide};
use crate::layout::manager::{
    DockError, DockGuard, DockMove, FocusPolicy, FrameMeasures, LeaseId, ViewId, WindowLabel,
    WindowSeed,
};

/// 이동 루프 동안 커서가 멈춘 채 이만큼 지나면 커서 밑 우리 창을 앞으로 올린다(조사 E6 · AC43).
pub const RAISE_AFTER_MS: u64 = 750;

/// 셸이 본 놓기에 탭을 쥔 창이 지금 자리로 답하기를 기다리는 시한(G18 · E7) — 넘기면 지금 모델 자리로 끝낸다.
pub const RELEASE_CONFIRM_MS: u64 = 1_000;

/// 세션이 쓰는 레이아웃 쓰기 — 늘 표식을 쥐고(`keep_lease`) 비운 창을 닫지 않는다([`OnEmptied::Keep`]). 창을 여는
/// `NewWindow` 는 이 타입에 없다(메인 스레드에서 새 창을 지으면 교착 — TRD §7 B9).
#[derive(Debug, Clone, PartialEq)]
pub struct DockWrite {
    pub view: ViewId,
    pub lease: LeaseId,
    pub to: SessionDock,
    pub focus: FocusPolicy,
    /// 옮긴 탭이 떼는 창의 활성이었고 그 창이 남을 때 그 창의 다음 활성(뜻 = `ViewManager::dock`).
    pub successor: Option<ViewId>,
}

/// [`DockWrite`] 의 목적지 — 뜻은 `layout::apply::DockDest` 의 같은 이름.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionDock {
    /// `share` = 띠 판정의 `BandHit::share` 그대로(다시 셈하지 않는다 — 칠한 곳 = 붙는 곳).
    Seam {
        target: ViewId,
        anchor: DockAnchor,
        side: DockSide,
        share: f64,
    },
    TabBar {
        window: WindowLabel,
        index: Option<usize>,
    },
    AdoptWindow {
        label: WindowLabel,
        seed: Option<WindowSeed>,
        measures: FrameMeasures,
    },
}

impl DockWrite {
    /// 적용 서비스(`layout::apply::dock_tab`)에 넘길 요청.
    pub fn into_request(self) -> DockRequest {
        let to = match self.to {
            SessionDock::Seam {
                target,
                anchor,
                side,
                share,
            } => DockDest::Seam {
                target,
                anchor,
                side,
                share,
            },
            SessionDock::TabBar { window, index } => DockDest::TabBar { window, index },
            SessionDock::AdoptWindow {
                label,
                seed,
                measures,
            } => DockDest::AdoptWindow {
                label,
                seed,
                measures,
            },
        };
        DockRequest {
            view: self.view,
            to,
            guard: DockGuard {
                lease: Some(self.lease),
                keep_lease: true,
                ..DockGuard::NONE
            },
            focus: self.focus,
            mv: DockMove {
                successor: self.successor,
            },
            on_emptied: OnEmptied::Keep,
        }
    }
}

/// 창의 보통 자리와 최대화 — 시작 때 적어 두었다가 되돌린다. `normal` = 바깥 사각형(화면 물리 px).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub normal: PhysRect,
    pub maximized: bool,
}

/// 탭 하나 창을 창째 끌기 전에 놓는 자리(C4 · TRD §5-5) — 세션이 시작 때의 값으로 셈했다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WholePlan {
    /// 최대화였다 — 최대화를 풀며 이 바깥 사각형으로(`restore_placement` · 활성 그대로).
    Restore(PhysRect),
    /// 보통이다 — 크기는 두고 바깥 원점만 옮긴다.
    Move(PhysPoint),
}

/// 떼어진 창 놓기의 재료 — 세션이 아는 값. 어댑터가 놓는 순간의 OS 값(모니터 · 틀 두께 · 배율)으로
/// [`Self::outer_rect`] 를 부른다.
///
/// - `restored`: 출처 창 복원 바깥 크기(DIP — 시작 때 한 번 · `sizing::restored_size`).
/// - `maximized`: 떼는 창(탭을 쥔 창)이 최대화면 그 바깥 크기(DIP).
/// - `strip_origin`: 떼어진 창 크롬 재기의 탭 끌기 영역 왼쪽 위(뷰포트 CSS).
/// - `grab`: 떼어진 창 탭에서 잡은 자리(`Grab.single_x` · `Grab.y`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetachPlan {
    pub restored: SizeDip,
    pub maximized: Option<SizeDip>,
    pub strip_origin: PointCss,
    pub grab: PointCss,
}

impl DetachPlan {
    /// 떼어진 창 바깥 사각형(물리) — 크기 = `dragged_size`(작업 영역 = 커서에 가장 가까운 모니터 · 모니터가 없으면
    /// 자르지 않는다)를 `scale` 로 · 원점 = `detached_origin`. `scale` = 그 창이 놓일 배율(처음엔 커서 모니터 배율 ·
    /// 놓은 뒤 그 창 배율이 셈에 쓴 배율과 다르면 그 배율로 한 번 더) · `frame_offset` = 그 창의 지금 틀 두께.
    // ADR-0295
    pub fn outer_rect(
        &self,
        cursor: PhysPoint,
        monitors: &[Monitor],
        scale: f64,
        frame_offset: PhysPoint,
    ) -> PhysRect {
        let work = work_area_near(monitors, cursor).unwrap_or(SizeDip {
            w: f64::INFINITY,
            h: f64::INFINITY,
        });
        let size = dragged_size(self.restored, self.maximized, work).to_phys(scale);
        let origin = detached_origin(cursor, scale, frame_offset, self.strip_origin, self.grab);
        PhysRect::at(origin, size)
    }
}

/// 세션이 바깥에 닿는 포트 — 계약은 모듈 머리. 창은 label 로 부른다(없는 창이면 무동작 · 읽기는 `None`).
pub trait Ports {
    // ── 레이아웃 ──

    /// 지금 세계 — 레이아웃 락 안에서 모델 · 명부를 뜨고, 락을 놓은 뒤 모델 창마다의 OS 기하와 모니터 목록을
    /// 채운다(`world::snapshot` + 기하).
    fn world(&mut self) -> World;
    fn lease_tab(&mut self, view: ViewId) -> Result<LeaseId, DockError>;
    fn release_lease(&mut self, lease: LeaseId);
    fn dock(&mut self, write: DockWrite) -> Result<DockOutcome, DockError>;
    fn switch_tab(&mut self, window: &str, view: ViewId);
    fn close_tab(&mut self, window: &str, view: ViewId);

    // ── 창 ──

    fn cursor(&mut self) -> PhysPoint;
    /// 점 밑 맨 위 우리 창 — `skip`(끄는 창)과 안내 창을 건너뛴다. 우리 창이 아니면 `None`.
    fn probe(&mut self, at: PhysPoint, skip: &str) -> Option<WindowLabel>;
    fn geom(&mut self, label: &str) -> Option<WindowGeom>;
    /// 떼어진 창(숨은 채 들였다)을 놓고 보이고 활성으로 — 숨은 채 platform `place_window(rect, show: false)` 로 놓고
    /// → Tauri `show()`(포커스 없이 지은 창이라 활성화 없이 보인다) → platform 활성. 자리 = [`DetachPlan::outer_rect`]
    /// 를 커서 모니터 배율로 셈한 것 — 놓은 뒤 그 창 배율이 셈에 쓴 배율과 다르면 그 창 배율로 한 번 더 놓는다(배율이
    /// 다른 모니터로 뗄 때 · 창 배율은 그대로인데 커서 모니터 배율과 다를 때).
    fn place_detached(&mut self, label: &str, plan: &DetachPlan, cursor: PhysPoint);
    fn place_whole(&mut self, label: &str, plan: WholePlan);
    /// 보통 자리 · 최대화를 되돌린다 — `behind` = 그 창 바로 뒤로(`None` = z 그대로) · `activate` = 거짓이면 활성화
    /// 없이(최대화는 OS 가 활성으로 만들 수 있다 — platform `restore_placement`). 숨어 있던 창(되살린 남긴 창)이면
    /// 뒤이어 Tauri `show()`(모듈 머리 「보임」).
    fn restore_placement(
        &mut self,
        label: &str,
        start: Placement,
        behind: Option<&str>,
        activate: bool,
    );
    /// 날 `SW_HIDE` 로 숨긴다(platform `hide_window` — 이동 루프 안에서 잰 길 · 스파이크 S4). tao 보임 깃발은 참으로
    /// 남는다(모듈 머리 「보임」).
    fn hide(&mut self, label: &str);
    /// 자리 · 크기 · z 를 두고 보이기만 한다(활성화 없음) — 이미 보이면 무동작.
    fn show_in_place(&mut self, label: &str);
    /// 앞으로 · 활성 — platform 으로만 한다. ★Tauri `set_focus` 를 쓰지 말 것★ — 전경이 거절되면 tao 가 Alt 를
    /// 합성해 누르고, 앱 전체 키 감시가 그 Alt 를 「그 밖의 키 = 놓기」로 읽어 끌기를 끝낸다(TRD §6-3 F7 a).
    fn focus(&mut self, label: &str);
    fn raise_no_activate(&mut self, label: &str);
    /// 그 창의 OS 이동 루프를 올린다. `Ok` 면 끝 알림이 정확히 한 번 `Event::LoopEnded{session, loop_no, ..}` 로
    /// 온다(돌아오기 전에 줄에 들 수 있다 · 커서는 알림을 받는 때 읽는다). `Err` 면 알림이 오지 않는다.
    /// `hide_on_escape` = 루프 중 Esc 를 보면 그 창을 숨긴다(새로 뗀 창).
    fn begin_move_loop(
        &mut self,
        label: &str,
        hide_on_escape: bool,
        session: u64,
        loop_no: u32,
    ) -> Result<(), String>;
    /// 끝은 `LoopEnded` 로 온다 — 이미 끝났으면 무동작.
    fn end_move_loop(&mut self, label: &str);
    fn close_window(&mut self, label: &str);
    /// 그 창의 지금 자리를 모델에 적는다(다음 부팅 자리).
    fn record_placement(&mut self, label: &str);
    /// 안내 창 — `Some` = 그 창 안쪽에 놓고 `dock:overlay` 로 그림을 민다(배율이 바뀌면 한 번 더 놓는다) · `None` =
    /// 치운다.
    fn overlay(&mut self, overlay: Option<Overlay>);

    // ── 대기 창 명부 ──
    // 명부의 창은 숨긴 채 기다린다 — 화면 밖에 보이는 채로 두지 않는다(사용자 결정 2026-10-19).

    /// 모델에 들인 창(`dock` 의 `AdoptWindow` 뒤 — 앞서 알리면 그 창이 「불러오는 중」으로 깜빡인다)을 명부에서
    /// 뺀다 — 명부 항목을 지우고 `dock:spare{adopted}`. 창은 숨은 채 둔다 — 숨은 최상위 창은 작업 표시줄 · Alt+Tab
    /// 에 나오지 않아 되돌릴 깃발이 없다. 보이기는 뒤이은 `place_detached`(떼기) · `restore_placement`(남긴 창
    /// 되살리기) 몫이다.
    fn unpark(&mut self, label: &str);
    /// 비운 떼어진 창을 예비로 — Tauri `hide()` 로 숨기고(이미 숨었어도 — 붙기의 [`Self::hide`] · 루프 Esc 훅의 날
    /// 숨기기는 tao 깃발을 참으로 남긴다) 명부 `Spare` · `dock:spare{parked}`.
    fn give_back(&mut self, label: &str, measures: FrameMeasures);
    /// 비운 출처 창을 되돌리기용으로 남긴다 — 붙기를 정할 때 [`Self::hide`] 로 숨은 그대로(창 호출 없음) 명부
    /// `Kept` · `dock:spare{parked}`.
    fn park_kept(&mut self, label: &str, measures: FrameMeasures);
    /// 명부에 예비(짓는 중 포함)가 없을 때만 작업 스레드에서 새로 짓는다.
    fn fill_spare(&mut self);

    // ── 웹뷰 사건 ──

    /// `dock:drag` — `to` = `None` 이면 우리 웹뷰 전부(명부 창 포함 · 안내 창 제외) · `Some` 이면 그 창 하나.
    fn drag(&mut self, event: TabDragEvent, to: Option<&str>);
    /// `dock:feed` — 그 창 하나에.
    fn feed(&mut self, to: &str, feed: TabDragFeed);

    // ── 시한 · 폴러 ──

    /// [`RAISE_AFTER_MS`] 뒤 `Event::RaiseDue{seq}` 를 줄에 넣는다(앞 시한을 지우지 않는다 — 세션이 seq 로 가른다).
    fn arm_raise(&mut self, seq: u32);
    /// [`RELEASE_CONFIRM_MS`] 뒤 `Event::ReleaseDue{seq}`.
    fn arm_release(&mut self, seq: u32);
    /// 8 ms 마다 커서 · 주 버튼을 읽어 `Event::Pointer{gen, ..}` 를 넣는다 — 시작 뒤 첫 표본은 늘 넣고 그 뒤는 바뀔
    /// 때만(첫 표본까지 거르면 붙는 사이에 뗀 버튼을 커서가 움직일 때까지 못 본다). 다음 `poll_start` 나 `poll_stop`
    /// 까지. `label` = 그 끌기를 쥔 창(어댑터의 진단 몫).
    fn poll_start(&mut self, label: &str, gen: u32);
    fn poll_stop(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(x0: i32, w: i32, h: i32, scale: f64) -> Monitor {
        let rect = PhysRect {
            x0,
            y0: 0,
            x1: x0 + w,
            y1: h,
        };
        Monitor {
            rect,
            work: PhysRect { y1: h - 40, ..rect },
            scale,
        }
    }

    fn plan(maximized: Option<SizeDip>) -> DetachPlan {
        DetachPlan {
            restored: SizeDip {
                w: 1000.0,
                h: 700.0,
            },
            maximized,
            strip_origin: PointCss { x: 4.0, y: 0.0 },
            grab: PointCss { x: 40.0, y: 12.0 },
        }
    }

    #[test]
    fn the_detached_rect_keeps_the_grab_under_the_cursor_at_the_given_scale() {
        let monitors = [monitor(0, 1920, 1080, 1.0)];
        let rect = plan(None).outer_rect(
            PhysPoint { x: 500, y: 300 },
            &monitors,
            1.0,
            PhysPoint { x: 8, y: 31 },
        );
        assert_eq!(
            rect,
            PhysRect {
                x0: 500 - 8 - 44,
                y0: 300 - 31 - 12,
                x1: 500 - 8 - 44 + 1000,
                y1: 300 - 31 - 12 + 700,
            }
        );

        let at_150 = plan(None).outer_rect(
            PhysPoint { x: 500, y: 300 },
            &monitors,
            1.5,
            PhysPoint { x: 12, y: 45 },
        );
        assert_eq!((at_150.x0, at_150.y0), (500 - 12 - 66, 300 - 45 - 18));
        assert_eq!(at_150.size().w, 1500);
    }

    #[test]
    fn the_work_area_of_the_cursor_monitor_caps_and_a_maximized_half_wins() {
        let monitors = [monitor(0, 900, 600, 1.0), monitor(900, 1920, 1080, 1.0)];
        let small = plan(None).outer_rect(
            PhysPoint { x: 100, y: 100 },
            &monitors,
            1.0,
            PhysPoint::default(),
        );
        assert_eq!(small.size().w, 900 - 20, "작업 영역 − 2 × 10");
        assert_eq!(small.size().h, 560 - 20);

        let half = plan(Some(SizeDip {
            w: 2400.0,
            h: 1200.0,
        }))
        .outer_rect(
            PhysPoint { x: 100, y: 100 },
            &monitors,
            1.0,
            PhysPoint::default(),
        );
        assert_eq!(
            (half.size().w, half.size().h),
            (1200, 600),
            "최대화 절반이 이긴다(C5)"
        );
    }

    #[test]
    fn without_monitors_the_restored_size_stands() {
        let rect = plan(None).outer_rect(PhysPoint::default(), &[], 1.25, PhysPoint::default());
        assert_eq!((rect.size().w, rect.size().h), (1250, 875));
    }

    #[test]
    fn a_session_write_keeps_the_lease_and_the_emptied_window() {
        let view = ViewId::from_u128(3);
        let req = DockWrite {
            view,
            lease: LeaseId(9),
            to: SessionDock::TabBar {
                window: "main".to_owned(),
                index: Some(0),
            },
            focus: FocusPolicy::Reveal,
            successor: Some(ViewId::from_u128(4)),
        }
        .into_request();
        assert_eq!(req.view, view);
        assert_eq!(
            req.to,
            DockDest::TabBar {
                window: "main".to_owned(),
                index: Some(0)
            }
        );
        assert_eq!(req.guard.lease, Some(LeaseId(9)));
        assert!(req.guard.keep_lease);
        assert_eq!(req.guard.expect_source, None);
        assert_eq!(req.guard.expect_target, None);
        assert_eq!(req.mv.successor, Some(ViewId::from_u128(4)));
        assert_eq!(req.on_emptied, OnEmptied::Keep);
        assert_eq!(req.focus, FocusPolicy::Reveal);
    }
}
