//! 예비 창 상태(TRD S21-slot-docking §3-9) — 예비 창 풀([`pool`]) · 그 창 웹뷰와의 wire([`wire`]) · 앱 종료 표지.
//! 레이아웃 권위(`crate::layout` · ADR-0035)는 여기서 바꾸지 않는다.

pub mod pool;
pub mod wire;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use engram_dashboard_base::sync;

use pool::{Measure, SparePool};
use wire::WindowRole;

use crate::layout::LayoutError;

/// 셸에 하나인 예비 창 상태 — 예비 창 풀과 앱 종료 표지.
///
/// - 예비 창 풀([`pool`]): 잎 락 — 쥔 채 레이아웃 락 · emit · 창 호출을 하지 않는다. 레이아웃 락을 쥔 채 잡는 것은
///   된다(차례 = 레이아웃 → 풀).
/// - 앱 종료 표지: 종료 요청을 받으면 선다([`Self::begin_exit`]) — 그 뒤 사라진 예비 창은 다시 짓지 않는다
///   (TRD §3-9 「다시 짓기」). 내리지 않는다.
#[derive(Debug, Default)]
pub struct SpareState {
    pool: Mutex<SparePool>,
    exiting: AtomicBool,
}

impl SpareState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 풀 가드 — 문장 하나 안에서 쓰고 놓는다(쥔 채 할 수 없는 것 = 구조체 문서).
    pub fn pool(&self) -> MutexGuard<'_, SparePool> {
        sync::lock(&self.pool)
    }

    pub fn is_spare(&self, label: &str) -> bool {
        self.pool().contains(label)
    }

    /// 풀의 창이 보낸 재기를 적는다 — `None` = 풀에 없다(부르는 쪽이 모델에 적는다 · 뜻 = [`SparePool::record`]).
    /// 풀 락은 돌아오기 전에 놓는다 — 부르는 쪽이 이어서 레이아웃 락을 잡는다.
    pub fn record_measure(&self, label: &str, measure: Measure) -> Option<Result<(), LayoutError>> {
        self.pool().record(label, measure)
    }

    /// 지은 예비 창의 지어짐을 세운다(뜻 = [`SparePool::mark_built`]).
    pub fn mark_built(&self, label: &str) -> bool {
        self.pool().mark_built(label)
    }

    /// 앱이 끝나는 중이다 — 사라진 예비 창을 다시 짓지 않는다. 한 번 서면 내리지 않는다.
    pub fn begin_exit(&self) {
        self.exiting.store(true, Ordering::SeqCst);
    }

    pub fn exiting(&self) -> bool {
        self.exiting.load(Ordering::SeqCst)
    }

    /// 창 `label` 의 역할 — 풀의 창이면 `Spare`, 아니면 `in_model` 로 `Model` · `Unknown`. `in_model` 은 풀 락을
    /// 놓은 뒤 부르므로 레이아웃 락을 잡아도 된다.
    pub fn window_role(&self, label: &str, in_model: impl FnOnce() -> bool) -> WindowRole {
        if self.is_spare(label) {
            WindowRole::Spare
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
        let spare = SpareState::new();
        assert!(!spare.exiting());
        spare.begin_exit();
        spare.begin_exit();
        assert!(spare.exiting());
    }

    #[test]
    fn a_pool_window_is_spare_without_asking_the_model() {
        let spare = SpareState::new();
        spare.pool().reserve_spare("slot-popup-1".to_owned());
        let role = spare.window_role("slot-popup-1", || {
            panic!("풀의 창은 모델을 묻지 않는다");
        });
        assert_eq!(role, WindowRole::Spare);
    }

    #[test]
    fn a_window_outside_the_pool_is_model_or_unknown() {
        let spare = SpareState::new();
        assert_eq!(spare.window_role("main", || true), WindowRole::Model);
        assert_eq!(
            spare.window_role("slot-popup-9", || false),
            WindowRole::Unknown
        );
    }

    #[test]
    fn the_model_question_runs_with_the_pool_unlocked() {
        let spare = SpareState::new();
        let role = spare.window_role("main", || {
            assert!(spare.pool.try_lock().is_ok(), "풀 락을 쥔 채 묻지 않는다");
            true
        });
        assert_eq!(role, WindowRole::Model);
    }

    // ── 예비 준비됨 · 재기 받기 ──

    #[test]
    fn a_reserved_spare_is_ready_once_marked_built() {
        const SPARE: &str = "slot-popup-1";
        let spare = SpareState::new();
        assert!(!spare.mark_built(SPARE), "풀에 없다");
        spare.pool().reserve_spare(SPARE.to_owned());
        assert_eq!(spare.pool().ready_spare(), None);
        assert!(spare.mark_built(SPARE));
        assert_eq!(
            spare
                .pool()
                .ready_spare()
                .map(|(label, _)| label.to_owned()),
            Some(SPARE.to_owned())
        );
        assert!(spare.pool.try_lock().is_ok(), "돌아올 때 풀 락을 놓았다");
    }

    #[test]
    fn a_measure_for_a_window_outside_the_pool_is_not_taken() {
        let spare = SpareState::new();
        assert_eq!(
            spare.record_measure("main", Measure::Canvas { w: 800, h: 600 }),
            None
        );
        spare.pool().reserve_spare("slot-popup-1".to_owned());
        assert_eq!(
            spare.record_measure("slot-popup-1", Measure::Canvas { w: 800, h: 600 }),
            Some(Ok(()))
        );
        assert!(spare.pool.try_lock().is_ok(), "돌아올 때 풀 락을 놓았다");
    }
}
