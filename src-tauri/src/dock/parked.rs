//! 대기 창 명부(TRD S21-slot-docking §2-5) — 모델 밖에서 기다리는 우리 창(예비 창 · 끄는 중 비워 남긴 창)의 label →
//! 역할 · 재기. 자료뿐이다 — 창을 만들거나 만지지 않는다.
//!
//! 「대기 중인가」는 label 의 성질이 아니라 지금 상태라 접두가 아니라 이 명부로 가른다(B7) — 예비 창은 들이면 보통
//! 팝아웃이 되는데 label 은 바꿀 수 없고, 남긴 창은 원래 팝아웃 label 그대로다.
//!
//! - 명부에 든 창은 트레이 보이기 · 숨기기 · `--hidden` 과 자리 기록에서 빠진다([`Parked::contains`]).
//! - 그 창 웹뷰의 재기 보고는 모델 대신 여기 적힌다([`Parked::record`]) — 모델은 모르는 label 을 거절해, 적지 않으면
//!   들일 때(`AdoptWindow`) 옮겨 담을 재기가 없다.
// ADR-0295

use std::collections::BTreeMap;

use crate::layout::chrome::DockChrome;
use crate::layout::manager::{check_metrics, reported_canvas, FrameMeasures, WindowLabel};
use crate::layout::{LayoutError, UiMetrics};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkedRole {
    /// 떼어 낼 창 — 끌기가 꺼내 모델에 들인다. 명부에 많아야 하나다(G11 — [`Parked::reserve_spare`]).
    Spare,
    /// 끄는 중 비워 남긴 출처 창(R10) — 되돌리기가 다시 들이거나 끌기 끝에 닫는다.
    Kept,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParkedEntry {
    pub role: ParkedRole,
    /// `false` = 짓기 전에 적어 둔 예비 창(짓는 동안의 트레이 · 자동 시작도 그 창을 보지 않게) — 짓는 쪽이 지은 뒤
    /// [`Parked::mark_built`] 로 세운다.
    pub built: bool,
    pub measures: FrameMeasures,
}

impl ParkedEntry {
    fn ready(&self) -> bool {
        let m = &self.measures;
        self.role == ParkedRole::Spare
            && self.built
            && m.canvas.is_some()
            && m.metrics.is_some()
            && m.chrome.is_some()
    }
}

/// 웹뷰가 보낸 재기 하나 — 받는 규칙은 모델 항목과 같다(`ViewManager::set_window_canvas` · `set_ui_metrics` ·
/// `set_dock_chrome`).
#[derive(Debug, Clone, PartialEq)]
pub enum Measure {
    Canvas { w: u32, h: u32 },
    Metrics(UiMetrics),
    Chrome(DockChrome),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parked {
    entries: BTreeMap<WindowLabel, ParkedEntry>,
}

impl Parked {
    /// 짓기 전 예비 창 자리를 적는다(역할 `Spare` · 안 지어짐 · 재기 없음). 명부에 예비(짓는 중 포함)가 이미 있으면
    /// 적지 않고 `false` — 짓지 않는다(예비는 하나 · G11).
    pub fn reserve_spare(&mut self, label: WindowLabel) -> bool {
        if self.has_spare() {
            return false;
        }
        self.entries.insert(
            label,
            ParkedEntry {
                role: ParkedRole::Spare,
                built: false,
                measures: FrameMeasures::default(),
            },
        );
        true
    }

    /// 그 창이 지어졌다 — label 이 명부에 없으면(짓는 동안 사라져 빠졌다) `false`.
    pub fn mark_built(&mut self, label: &str) -> bool {
        match self.entries.get_mut(label) {
            Some(entry) => {
                entry.built = true;
                true
            }
            None => false,
        }
    }

    /// 이미 지어진 창을 그 역할 · 재기로 명부에 둔다(남긴 창 = `Kept` · 예비로 되돌린 창 = `Spare`) — 이미 있으면
    /// 역할 · 재기를 바꾼다.
    pub fn park(&mut self, label: WindowLabel, role: ParkedRole, measures: FrameMeasures) {
        self.entries.insert(
            label,
            ParkedEntry {
                role,
                built: true,
                measures,
            },
        );
    }

    pub fn remove(&mut self, label: &str) -> Option<ParkedEntry> {
        self.entries.remove(label)
    }

    pub fn get(&self, label: &str) -> Option<&ParkedEntry> {
        self.entries.get(label)
    }

    /// 명부의 창인가 — 트레이 · 자리 기록이 빼는 창.
    pub fn contains(&self, label: &str) -> bool {
        self.entries.contains_key(label)
    }

    /// 명부에 예비(짓는 중 포함)가 있나 — 예비를 새로 지을지.
    pub fn has_spare(&self) -> bool {
        self.entries.values().any(|e| e.role == ParkedRole::Spare)
    }

    /// 떼기에 쓸 수 있는 예비 창 — 지어졌고 역할이 `Spare` 이고 재기 셋(캔버스 · 지표 · 크롬)이 다 왔다(§2-5 「예비 창
    /// 준비됨」). 재기가 덜 온 예비는 첫 떼기의 `original` · 떼어진 창 자리를 셀 수 없다(§5-4 · §5-5).
    pub fn ready_spare(&self) -> Option<(&str, &FrameMeasures)> {
        self.entries
            .iter()
            .find_map(|(label, e)| e.ready().then_some((label.as_str(), &e.measures)))
    }

    /// 그 창이 떼기에 쓸 수 있는 예비 창인가(뜻 = [`Self::ready_spare`]) — 명부에 없으면 `false`.
    pub fn is_ready_spare(&self, label: &str) -> bool {
        self.entries.get(label).is_some_and(ParkedEntry::ready)
    }

    /// 명부의 창이 보낸 재기를 적는다 — `None` = 명부에 없는 label(부르는 쪽이 모델에 적는다). `Some(Err)` = 범위
    /// 밖 지표라 거절하고 직전 값을 유지한다. 0 크기 캔버스는 무시하고 `Some(Ok)` 다.
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
            Measure::Chrome(chrome) => {
                m.chrome = Some(chrome);
                Ok(())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::chrome::{PointCss, RectCss};
    use crate::layout::geometry::Insets;
    use crate::layout::CanvasPx;

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

    fn chrome() -> DockChrome {
        DockChrome {
            canvas_origin: PointCss { x: 0.0, y: 28.0 },
            tab_bar: RectCss {
                x0: 0.0,
                y0: 0.0,
                x1: 640.0,
                y1: 28.0,
            },
        }
    }

    fn full() -> FrameMeasures {
        FrameMeasures {
            canvas: Some(CanvasPx { w: 800, h: 600 }),
            metrics: Some(metrics()),
            chrome: Some(chrome()),
        }
    }

    fn report_all(parked: &mut Parked, label: &str) {
        for measure in [
            Measure::Canvas { w: 800, h: 600 },
            Measure::Metrics(metrics()),
            Measure::Chrome(chrome()),
        ] {
            assert_eq!(parked.record(label, measure), Some(Ok(())));
        }
    }

    // ── 예비 자리 · 지어짐 ──

    #[test]
    fn a_spare_is_reserved_unbuilt_and_only_one_spare_at_a_time() {
        let mut parked = Parked::default();
        assert!(!parked.has_spare());
        assert!(parked.reserve_spare(P1.to_owned()));
        assert_eq!(
            parked.get(P1),
            Some(&ParkedEntry {
                role: ParkedRole::Spare,
                built: false,
                measures: FrameMeasures::default(),
            })
        );
        assert!(parked.has_spare(), "짓는 중도 예비다");
        assert!(!parked.reserve_spare(P2.to_owned()), "예비는 하나");
        assert!(!parked.contains(P2));
    }

    #[test]
    fn a_kept_window_does_not_count_as_a_spare() {
        let mut parked = Parked::default();
        parked.park(P1.to_owned(), ParkedRole::Kept, full());
        assert!(!parked.has_spare());
        assert!(parked.reserve_spare(P2.to_owned()));
    }

    #[test]
    fn marking_built_needs_the_entry() {
        let mut parked = Parked::default();
        assert!(!parked.mark_built(P1), "짓는 동안 빠졌다");
        parked.reserve_spare(P1.to_owned());
        assert!(parked.mark_built(P1));
        assert!(parked.get(P1).unwrap().built);
    }

    // ── 준비된 예비 ──

    // 표: (역할, 지어짐, 캔버스, 지표, 크롬) → 준비됨
    #[test]
    fn a_spare_is_ready_only_when_built_with_all_three_measures() {
        let rows = [
            (ParkedRole::Spare, true, true, true, true, true),
            (ParkedRole::Spare, false, true, true, true, false),
            (ParkedRole::Spare, true, false, true, true, false),
            (ParkedRole::Spare, true, true, false, true, false),
            (ParkedRole::Spare, true, true, true, false, false),
            (ParkedRole::Kept, true, true, true, true, false),
        ];
        for (role, built, canvas, metrics_in, chrome_in, ready) in rows {
            let measures = FrameMeasures {
                canvas: canvas.then_some(CanvasPx { w: 800, h: 600 }),
                metrics: metrics_in.then(metrics),
                chrome: chrome_in.then(chrome),
            };
            let mut parked = Parked::default();
            parked.park(P1.to_owned(), role, measures.clone());
            if !built {
                parked.entries.get_mut(P1).unwrap().built = false;
            }
            let expected = ready.then_some((P1, &measures));
            let row = format!(
                "{role:?} built={built} canvas={canvas} metrics={metrics_in} chrome={chrome_in}"
            );
            assert_eq!(parked.ready_spare(), expected, "{row}");
            assert_eq!(parked.is_ready_spare(P1), ready, "{row}");
        }
        assert!(!Parked::default().is_ready_spare(P1), "명부에 없다");
    }

    #[test]
    fn a_reserved_spare_becomes_ready_after_it_is_built_and_reported() {
        let mut parked = Parked::default();
        parked.reserve_spare(P1.to_owned());
        report_all(&mut parked, P1);
        assert_eq!(parked.ready_spare(), None, "아직 안 지어짐");
        parked.mark_built(P1);
        assert_eq!(parked.ready_spare(), Some((P1, &full())));
    }

    #[test]
    fn the_ready_spare_is_found_beside_a_kept_window() {
        let mut parked = Parked::default();
        parked.park(P1.to_owned(), ParkedRole::Kept, full());
        parked.park(P2.to_owned(), ParkedRole::Spare, full());
        assert_eq!(parked.ready_spare(), Some((P2, &full())));
    }

    // ── 재기 받기 ──

    #[test]
    fn a_label_outside_the_registry_is_left_to_the_model() {
        let mut parked = Parked::default();
        assert_eq!(parked.record(P1, Measure::Canvas { w: 800, h: 600 }), None);
        assert_eq!(parked, Parked::default());
    }

    #[test]
    fn each_measure_lands_in_its_own_slot() {
        let mut parked = Parked::default();
        parked.reserve_spare(P1.to_owned());
        report_all(&mut parked, P1);
        assert_eq!(parked.get(P1).unwrap().measures, full());
        assert!(
            !parked.get(P1).unwrap().built,
            "재기는 지어짐을 세우지 않는다"
        );
    }

    #[test]
    fn a_zero_canvas_is_ignored_and_keeps_the_previous_value() {
        let mut parked = Parked::default();
        parked.park(P1.to_owned(), ParkedRole::Kept, full());
        for (w, h) in [(0, 600), (800, 0), (0, 0)] {
            assert_eq!(parked.record(P1, Measure::Canvas { w, h }), Some(Ok(())));
        }
        assert_eq!(
            parked.get(P1).unwrap().measures.canvas,
            Some(CanvasPx { w: 800, h: 600 })
        );
    }

    #[test]
    fn out_of_range_metrics_are_refused_and_keep_the_previous_value() {
        let mut parked = Parked::default();
        parked.park(P1.to_owned(), ParkedRole::Kept, full());
        let bad = UiMetrics {
            min_pane_px: 0,
            ..metrics()
        };
        assert!(matches!(
            parked.record(P1, Measure::Metrics(bad)),
            Some(Err(LayoutError::InvalidMetrics(_)))
        ));
        assert_eq!(parked.get(P1).unwrap().measures.metrics, Some(metrics()));
    }

    // ── 빼기 ──

    #[test]
    fn park_replaces_role_and_measures_and_remove_takes_the_entry() {
        let mut parked = Parked::default();
        parked.reserve_spare(P1.to_owned());
        parked.park(P1.to_owned(), ParkedRole::Kept, full());
        assert_eq!(
            parked.get(P1),
            Some(&ParkedEntry {
                role: ParkedRole::Kept,
                built: true,
                measures: full(),
            })
        );
        assert!(parked.contains(P1));
        assert_eq!(parked.remove(P1).map(|e| e.role), Some(ParkedRole::Kept));
        assert!(!parked.contains(P1));
        assert_eq!(parked.remove(P1), None);
    }
}
