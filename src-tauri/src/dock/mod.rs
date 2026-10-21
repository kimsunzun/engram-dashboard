//! 슬롯 도킹의 대기 창 상태(TRD S21-slot-docking §2-5) — 대기 창 명부([`parked`]) · 그 창 웹뷰와의 wire
//! ([`wire`]) · 앱 종료 표지. 레이아웃 권위(`crate::layout` · ADR-0035)는 여기서 바꾸지 않는다.

pub mod parked;
pub mod wire;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use engram_dashboard_base::sync;

use parked::{Measure, Parked};
use wire::WindowRole;

use crate::layout::LayoutError;

/// 셸에 하나인 도킹 상태 — 대기 창 명부와 앱 종료 표지.
///
/// - 대기 창 명부([`parked`] · TRD §2-5): 잎 락 — 쥔 채 레이아웃 락 · emit · 창 호출을 하지 않는다. 레이아웃 락을 쥔
///   채 잡는 것은 된다(차례 = 레이아웃 → 명부).
/// - 앱 종료 표지: 종료 요청을 받으면 선다([`Self::begin_exit`]) — 그 뒤 사라진 예비 창은 다시 짓지 않는다
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
        self.parked().record(label, measure)
    }

    /// 지은 예비 창의 지어짐을 세운다(뜻 = [`Parked::mark_built`]).
    pub fn mark_built(&self, label: &str) -> bool {
        self.parked().mark_built(label)
    }

    /// 앱이 끝나는 중이다 — 사라진 예비 창을 다시 짓지 않는다. 한 번 서면 내리지 않는다.
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
            dock.window_role("slot-popup-9", || false),
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

    // ── 예비 준비됨 · 재기 받기 ──

    #[test]
    fn a_reserved_spare_is_ready_once_marked_built() {
        const SPARE: &str = "slot-popup-1";
        let dock = DockState::new();
        assert!(!dock.mark_built(SPARE), "명부에 없다");
        dock.parked().reserve_spare(SPARE.to_owned());
        assert_eq!(dock.parked().ready_spare(), None);
        assert!(dock.mark_built(SPARE));
        assert_eq!(
            dock.parked()
                .ready_spare()
                .map(|(label, _)| label.to_owned()),
            Some(SPARE.to_owned())
        );
        assert!(dock.parked.try_lock().is_ok(), "돌아올 때 명부 락을 놓았다");
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
        assert!(dock.parked.try_lock().is_ok(), "돌아올 때 명부 락을 놓았다");
    }
}
