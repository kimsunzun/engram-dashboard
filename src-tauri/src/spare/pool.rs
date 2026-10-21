//! 예비 창 풀(TRD S21-slot-docking §3-9) — 모델 밖에서 기다리는 우리 창(예비 창)의 label → 지어짐 · 재기. 지금은 그
//! 명부뿐이고 §3-9 「풀」의 상태 기계(`Empty` · `Building` · `Built` · `Claimed`)는 U4d′ 가 세운다. 자료뿐이다 — 창을 만들거나
//! 만지지 않는다.
//!
//! TODO(U4d′): 맡기기 · 확정 상태 기계로 바꾼다(TRD §3-9).
//!
//! 「예비 창인가」는 label 의 성질이 아니라 지금 상태라 접두가 아니라 이 풀로 가른다 — 예비 창은 들이면 보통
//! 팝아웃이 되는데 label 은 바꿀 수 없다.
//!
//! - 풀에 든 창은 트레이 보이기 · 숨기기 · `--hidden` 과 자리 기록에서 빠진다([`SparePool::contains`]).
//! - 그 창 웹뷰의 재기 보고는 모델 대신 여기 적힌다([`SparePool::record`]) — 모델은 모르는 label 을 거절해, 적지
//!   않으면 들일 때 옮겨 담을 재기가 없다.
// ADR-0295 · ADR-0299

use std::collections::BTreeMap;

use crate::layout::manager::{check_metrics, reported_canvas, WindowLabel};
use crate::layout::{CanvasPx, LayoutError, UiMetrics};

/// 예비 창 웹뷰가 보고한 재기 둘 — 모델 창 항목의 `canvas` · `metrics` 와 같은 꼴 · 같은 받는 규칙이다. `None` = 아직
/// 보고가 없다.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpareMeasures {
    pub canvas: Option<CanvasPx>,
    pub metrics: Option<UiMetrics>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpareEntry {
    /// `false` = 짓기 전에 적어 둔 예비 창(짓는 동안의 트레이 · 자동 시작도 그 창을 보지 않게) — 짓는 쪽이 지은 뒤
    /// [`SparePool::mark_built`] 로 세운다.
    // TODO(U4d′): 맡기기 · 확정 상태 기계가 대신한다(TRD §3-9).
    pub built: bool,
    pub measures: SpareMeasures,
}

/// 웹뷰가 보낸 재기 하나 — 받는 규칙은 모델 항목과 같다(`ViewManager::set_window_canvas` · `set_ui_metrics`).
#[derive(Debug, Clone, PartialEq)]
pub enum Measure {
    Canvas { w: u32, h: u32 },
    Metrics(UiMetrics),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SparePool {
    entries: BTreeMap<WindowLabel, SpareEntry>,
}

impl SparePool {
    /// 짓기 전 예비 창 자리를 적는다(안 지어짐 · 재기 없음). 풀에 예비(짓는 중 포함)가 이미 있으면 적지 않고
    /// `false` — 짓지 않는다(예비는 하나).
    pub fn reserve_spare(&mut self, label: WindowLabel) -> bool {
        if self.has_spare() {
            return false;
        }
        self.entries.insert(
            label,
            SpareEntry {
                built: false,
                measures: SpareMeasures::default(),
            },
        );
        true
    }

    /// 그 창이 지어졌다 — label 이 풀에 없으면(짓는 동안 사라져 빠졌다) `false`.
    pub fn mark_built(&mut self, label: &str) -> bool {
        match self.entries.get_mut(label) {
            Some(entry) => {
                entry.built = true;
                true
            }
            None => false,
        }
    }

    pub fn remove(&mut self, label: &str) -> Option<SpareEntry> {
        self.entries.remove(label)
    }

    // TODO(U4d′): 맡기기 · 확정 상태 기계가 대신한다(TRD §3-9).
    pub fn get(&self, label: &str) -> Option<&SpareEntry> {
        self.entries.get(label)
    }

    /// 풀의 창인가 — 트레이 · 자리 기록이 빼는 창.
    pub fn contains(&self, label: &str) -> bool {
        self.entries.contains_key(label)
    }

    /// 풀에 예비(짓는 중 포함)가 있나 — 예비를 새로 지을지.
    pub fn has_spare(&self) -> bool {
        !self.entries.is_empty()
    }

    /// 쓸 수 있는 예비 창 — 지어졌다(TRD §9-3 I3). 재기는 묻지 않는다 — 페이지는 아직 적재 중일 수 있다.
    // TODO(U4d′): 맡기기 · 확정 상태 기계가 대신한다(TRD §3-9).
    pub fn ready_spare(&self) -> Option<(&str, &SpareMeasures)> {
        self.entries
            .iter()
            .find_map(|(label, e)| e.built.then_some((label.as_str(), &e.measures)))
    }

    /// 풀의 창이 보낸 재기를 적는다 — `None` = 풀에 없는 label(부르는 쪽이 모델에 적는다). `Some(Err)` = 범위 밖
    /// 지표라 거절하고 직전 값을 유지한다. 0 크기 캔버스는 무시하고 `Some(Ok)` 다.
    pub fn record(&mut self, label: &str, measure: Measure) -> Option<Result<(), LayoutError>> {
        let m = &mut self.entries.get_mut(label)?.measures;
        Some(match measure {
            Measure::Canvas { w, h } => {
                if let Some(canvas) = reported_canvas(w, h) {
                    m.canvas = Some(canvas);
                }
                Ok(())
            }
            Measure::Metrics(metrics) => check_metrics(&metrics).map(|()| {
                m.metrics = Some(metrics);
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::geometry::Insets;

    const P1: &str = "slot-popup-1";
    const P2: &str = "slot-popup-2";

    fn metrics() -> UiMetrics {
        UiMetrics {
            frame_insets: Insets {
                t: 1.0,
                r: 1.0,
                b: 1.0,
                l: 1.0,
            },
            min_pane_px: 50,
        }
    }

    fn full() -> SpareMeasures {
        SpareMeasures {
            canvas: Some(CanvasPx { w: 800, h: 600 }),
            metrics: Some(metrics()),
        }
    }

    fn report_all(pool: &mut SparePool, label: &str) {
        for measure in [
            Measure::Canvas { w: 800, h: 600 },
            Measure::Metrics(metrics()),
        ] {
            assert_eq!(pool.record(label, measure), Some(Ok(())));
        }
    }

    fn built_with_full_measures() -> SparePool {
        let mut pool = SparePool::default();
        pool.reserve_spare(P1.to_owned());
        pool.mark_built(P1);
        report_all(&mut pool, P1);
        pool
    }

    // ── 예비 자리 · 지어짐 ──

    #[test]
    fn a_spare_is_reserved_unbuilt_and_only_one_spare_at_a_time() {
        let mut pool = SparePool::default();
        assert!(!pool.has_spare());
        assert!(pool.reserve_spare(P1.to_owned()));
        assert_eq!(
            pool.get(P1),
            Some(&SpareEntry {
                built: false,
                measures: SpareMeasures::default(),
            })
        );
        assert!(pool.has_spare(), "짓는 중도 예비다");
        assert!(!pool.reserve_spare(P2.to_owned()), "예비는 하나");
        assert!(!pool.contains(P2));
    }

    #[test]
    fn marking_built_needs_the_entry() {
        let mut pool = SparePool::default();
        assert!(!pool.mark_built(P1), "짓는 동안 빠졌다");
        pool.reserve_spare(P1.to_owned());
        assert!(pool.mark_built(P1));
        assert!(pool.get(P1).unwrap().built);
    }

    // ── 준비된 예비 ──

    #[test]
    fn a_spare_is_ready_once_built_with_or_without_measures() {
        let mut pool = SparePool::default();
        pool.reserve_spare(P1.to_owned());
        assert_eq!(pool.ready_spare(), None, "아직 안 지어짐");
        pool.mark_built(P1);
        assert_eq!(
            pool.ready_spare(),
            Some((P1, &SpareMeasures::default())),
            "재기를 기다리지 않는다"
        );
        report_all(&mut pool, P1);
        assert_eq!(pool.ready_spare(), Some((P1, &full())));
    }

    #[test]
    fn measures_reported_before_building_do_not_make_it_ready() {
        let mut pool = SparePool::default();
        pool.reserve_spare(P1.to_owned());
        report_all(&mut pool, P1);
        assert_eq!(pool.ready_spare(), None);
        assert_eq!(SparePool::default().ready_spare(), None, "풀이 비었다");
    }

    // ── 재기 받기 ──

    #[test]
    fn a_label_outside_the_pool_is_left_to_the_model() {
        let mut pool = SparePool::default();
        assert_eq!(pool.record(P1, Measure::Canvas { w: 800, h: 600 }), None);
        assert_eq!(pool, SparePool::default());
    }

    #[test]
    fn each_measure_lands_in_its_own_slot() {
        let mut pool = SparePool::default();
        pool.reserve_spare(P1.to_owned());
        report_all(&mut pool, P1);
        assert_eq!(pool.get(P1).unwrap().measures, full());
        assert!(
            !pool.get(P1).unwrap().built,
            "재기는 지어짐을 세우지 않는다"
        );
    }

    #[test]
    fn a_zero_canvas_is_ignored_and_keeps_the_previous_value() {
        let mut pool = built_with_full_measures();
        for (w, h) in [(0, 600), (800, 0), (0, 0)] {
            assert_eq!(pool.record(P1, Measure::Canvas { w, h }), Some(Ok(())));
        }
        assert_eq!(
            pool.get(P1).unwrap().measures.canvas,
            Some(CanvasPx { w: 800, h: 600 })
        );
    }

    #[test]
    fn out_of_range_metrics_are_refused_and_keep_the_previous_value() {
        let mut pool = built_with_full_measures();
        let bad = UiMetrics {
            min_pane_px: 0,
            ..metrics()
        };
        assert!(matches!(
            pool.record(P1, Measure::Metrics(bad)),
            Some(Err(LayoutError::InvalidMetrics(_)))
        ));
        assert_eq!(pool.get(P1).unwrap().measures.metrics, Some(metrics()));
    }

    // ── 빼기 ──

    #[test]
    fn remove_takes_the_entry_and_frees_the_spare_place() {
        let mut pool = built_with_full_measures();
        assert!(pool.contains(P1));
        assert_eq!(
            pool.remove(P1),
            Some(SpareEntry {
                built: true,
                measures: full(),
            })
        );
        assert!(!pool.contains(P1));
        assert_eq!(pool.remove(P1), None);
        assert!(pool.reserve_spare(P2.to_owned()), "빠진 뒤 다시 적는다");
    }
}
