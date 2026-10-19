//! 슬롯 도킹의 끌기 쪽(TRD S21-slot-docking §2 · §3) — 레이아웃 권위(`crate::layout` · ADR-0035)를 부르는
//! 손님이다. 레이아웃은 여기서 바꾸지 않는다 — 바꾸는 길은 적용 서비스(`layout::apply`) 하나다.

pub mod exec;
pub mod geometry;
pub mod parked;
pub mod ports;
pub mod session;
pub mod sizing;
pub mod strip;
pub mod targets;
pub mod units;
pub mod wire;
pub mod world;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use engram_dashboard_base::sync;

use parked::{Measure, Parked, ParkedRole};
use wire::WindowRole;

use crate::layout::manager::FrameMeasures;
use crate::layout::LayoutError;

/// 셸에 하나인 도킹 상태 — 대기 창 명부와 앱 종료 표지.
///
/// - 대기 창 명부([`parked`] · TRD §2-5): 잎 락 — 쥔 채 레이아웃 락 · emit · 창 호출을 하지 않는다. 레이아웃 락을 쥔
///   채 잡는 것은 된다(차례 = 레이아웃 → 명부 · 세계 뜨기 [`world::snapshot`] 가 둘을 함께 본다).
/// - 앱 종료 표지: 종료 요청을 받으면 선다([`Self::begin_exit`]) — 그 뒤 사라진 예비 창 · 안내 창은 다시 짓지 않는다
///   (TRD §2-5 「사라지면」). 내리지 않는다.
#[derive(Debug, Default)]
pub struct DockState {
    parked: Mutex<Parked>,
    exiting: AtomicBool,
}

impl DockState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 명부 가드 — 문장 하나 안에서 쓰고 놓는다(쥔 채 할 수 없는 것 = 구조체 문서).
    pub fn parked(&self) -> MutexGuard<'_, Parked> {
        sync::lock(&self.parked)
    }

    pub fn is_parked(&self, label: &str) -> bool {
        self.parked().contains(label)
    }

    /// 명부의 창이 보낸 재기를 적는다 — `None` = 명부에 없다(부르는 쪽이 모델에 적는다 · 뜻 = [`Parked::record`]).
    /// 명부 락은 돌아오기 전에 놓는다 — 부르는 쪽이 이어서 레이아웃 락을 잡는다.
    pub fn record_parked(&self, label: &str, measure: Measure) -> Option<Result<(), LayoutError>> {
        self.update_parked(label, |parked| parked.record(label, measure))
    }

    /// 지은 예비 창의 지어짐을 세운다(뜻 = [`Parked::mark_built`]).
    pub fn mark_built(&self, label: &str) -> bool {
        self.update_parked(label, |parked| parked.mark_built(label))
    }

    /// 이미 지어진 창을 그 역할 · 재기로 명부에 둔다(뜻 = [`Parked::park`]).
    pub fn park(&self, label: &str, role: ParkedRole, measures: FrameMeasures) {
        self.update_parked(label, |parked| {
            parked.park(label.to_owned(), role, measures);
        });
    }

    // 명부를 바꾸고, 그 바꿈으로 `label` 이 떼기에 쓸 수 있는 예비가 됐으면 debug 한 줄 — 셸 쪽 「예비 준비됨」을 GUI
    //   실측에서 볼 길이 이 줄뿐이다. 이미 준비된 예비의 재기 다시 보고는 남기지 않는다. 로그는 명부 락을 놓은 뒤.
    // ADR-0295
    fn update_parked<R>(&self, label: &str, change: impl FnOnce(&mut Parked) -> R) -> R {
        let (out, became_ready) = self.change_parked(label, change);
        if became_ready {
            tracing::debug!(module = "dock", label, "예비 창 준비됨 — 떼기에 쓸 수 있다");
        }
        out
    }

    fn change_parked<R>(&self, label: &str, change: impl FnOnce(&mut Parked) -> R) -> (R, bool) {
        let mut parked = self.parked();
        let before = parked.is_ready_spare(label);
        let out = change(&mut parked);
        (out, !before && parked.is_ready_spare(label))
    }

    #[cfg(test)]
    pub fn parked_unlocked(&self) -> bool {
        self.parked.try_lock().is_ok()
    }

    /// 앱이 끝나는 중이다 — 사라진 예비 창 · 안내 창을 다시 짓지 않는다. 한 번 서면 내리지 않는다.
    pub fn begin_exit(&self) {
        self.exiting.store(true, Ordering::SeqCst);
    }

    pub fn exiting(&self) -> bool {
        self.exiting.load(Ordering::SeqCst)
    }

    /// 창 `label` 의 역할 — 명부의 창이면 `Parked`, 아니면 `in_model` 로 `Model` · `Unknown`. `in_model` 은 명부 락을
    /// 놓은 뒤 부르므로 레이아웃 락을 잡아도 된다.
    pub fn window_role(&self, label: &str, in_model: impl FnOnce() -> bool) -> WindowRole {
        if self.is_parked(label) {
            WindowRole::Parked
        } else if in_model() {
            WindowRole::Model
        } else {
            WindowRole::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_exit_mark_stays_once_set() {
        let dock = DockState::new();
        assert!(!dock.exiting());
        dock.begin_exit();
        dock.begin_exit();
        assert!(dock.exiting());
    }

    #[test]
    fn a_parked_window_is_parked_without_asking_the_model() {
        let dock = DockState::new();
        dock.parked().reserve_spare("slot-popup-1".to_owned());
        let role = dock.window_role("slot-popup-1", || {
            panic!("명부의 창은 모델을 묻지 않는다");
        });
        assert_eq!(role, WindowRole::Parked);
    }

    #[test]
    fn a_window_outside_the_registry_is_model_or_unknown() {
        let dock = DockState::new();
        assert_eq!(dock.window_role("main", || true), WindowRole::Model);
        assert_eq!(
            dock.window_role("dock-overlay", || false),
            WindowRole::Unknown
        );
    }

    #[test]
    fn the_model_question_runs_with_the_registry_unlocked() {
        let dock = DockState::new();
        let role = dock.window_role("main", || {
            assert!(
                dock.parked.try_lock().is_ok(),
                "명부 락을 쥔 채 묻지 않는다"
            );
            true
        });
        assert_eq!(role, WindowRole::Model);
    }

    // ── 예비 준비됨 ──

    fn full_measures() -> FrameMeasures {
        FrameMeasures {
            canvas: Some(crate::layout::CanvasPx { w: 800, h: 600 }),
            metrics: Some(crate::layout::UiMetrics {
                frame_insets: crate::layout::geometry::Insets {
                    t: 1.0,
                    r: 1.0,
                    b: 1.0,
                    l: 1.0,
                },
                min_pane_px: 50,
            }),
            chrome: Some(crate::layout::chrome::DockChrome {
                canvas_origin: crate::layout::chrome::PointCss { x: 0.0, y: 28.0 },
                tab_bar: crate::layout::chrome::RectCss {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 640.0,
                    y1: 28.0,
                },
                tabs: Vec::new(),
                scroll: None,
            }),
        }
    }

    fn report(dock: &DockState, label: &str, measure: Measure) -> bool {
        dock.change_parked(label, |p| p.record(label, measure)).1
    }

    #[test]
    fn a_built_spare_becomes_ready_once_with_its_last_missing_piece() {
        const SPARE: &str = "slot-popup-1";
        let dock = DockState::new();
        dock.parked().reserve_spare(SPARE.to_owned());
        let m = full_measures();
        assert!(!report(&dock, SPARE, Measure::Canvas { w: 800, h: 600 }));
        assert!(!report(&dock, SPARE, Measure::Metrics(m.metrics.unwrap())));
        assert!(
            !report(&dock, SPARE, Measure::Chrome(m.chrome.clone().unwrap())),
            "재기는 다 왔으나 아직 안 지어짐"
        );
        assert_eq!(
            dock.change_parked(SPARE, |p| p.mark_built(SPARE)),
            (true, true),
            "지어짐이 마지막 조각"
        );
        assert!(
            !report(&dock, SPARE, Measure::Chrome(m.chrome.unwrap())),
            "이미 준비된 예비의 다시 보고는 세지 않는다"
        );
        assert!(dock.mark_built(SPARE));
        assert!(dock.parked().is_ready_spare(SPARE));
    }

    #[test]
    fn a_window_given_back_with_full_measures_is_ready_and_a_kept_one_is_not() {
        let dock = DockState::new();
        let back = dock.change_parked("slot-popup-2", |p| {
            p.park(
                "slot-popup-2".to_owned(),
                ParkedRole::Spare,
                full_measures(),
            );
        });
        assert!(back.1);
        let kept = dock.change_parked("slot-popup-3", |p| {
            p.park("slot-popup-3".to_owned(), ParkedRole::Kept, full_measures());
        });
        assert!(!kept.1);
        dock.park("slot-popup-3", ParkedRole::Kept, FrameMeasures::default());
        assert_eq!(
            dock.parked().get("slot-popup-3").map(|e| e.role),
            Some(ParkedRole::Kept)
        );
        assert!(dock.parked_unlocked(), "돌아올 때 명부 락을 놓았다");
    }

    #[test]
    fn a_measure_for_a_window_outside_the_registry_is_not_taken() {
        let dock = DockState::new();
        assert_eq!(
            dock.record_parked("main", Measure::Canvas { w: 800, h: 600 }),
            None
        );
        dock.parked().reserve_spare("slot-popup-1".to_owned());
        assert_eq!(
            dock.record_parked("slot-popup-1", Measure::Canvas { w: 800, h: 600 }),
            Some(Ok(()))
        );
    }
}
