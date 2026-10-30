//! 끌기의 크기 셈 정본(TRD S21-slot-docking §4-2 · §4-5 · PRD R7 · R22) — 새 창 크기(Chrome
//! `CalculateDraggedWindowSize`) · 끄는 탭의 원래 크기. 단위 = 논리(DIP) px.
//!
//! ★웹 거울과 한 벌이다★ — 출처 웹뷰가 dragstart 에 같은 규칙(`dock/dragSize.ts`)으로 셈해 싣는다. 둘은 골든
//! `bands_golden.json` 의 `drag_size` 표를 함께 먹는다. 상수는 `bands.rs` 머리 한 벌이다.

use super::bands::{SizeCss, MAXIMIZED_INSET_DIP};
use super::geometry::Insets;
use super::manager::WindowBounds;

/// 출처 창의 복원(보통) 크기 — 모델의 보통 자리 크기(`bounds` — 안쪽 논리 크기 · 최대화 중이어도 보통 크기)에
/// `frame`(그 크기에 더할 틀 — 안쪽 크기를 셀 때는 0)을 더한다. 모델 값이 없으면 `current`(같은 쪽의 지금 크기)다.
pub fn restored_size(bounds: Option<WindowBounds>, frame: SizeCss, current: SizeCss) -> SizeCss {
    match bounds {
        Some(b) => SizeCss {
            w: b.w() + frame.w,
            h: b.h() + frame.h,
        },
        None => current,
    }
}

/// 새 창 크기 — Chrome `CalculateDraggedWindowSize`(TDC#2760-2791) 그대로: `restored` 에서 ① 작업 영역 `work` 보다
/// 큰 변은 `max(0, work − 2 × MAXIMIZED_INSET_DIP)` ② 출처가 최대화(`maximized` = 그 크기)면 각 변을 그 절반 이상으로.
/// ★②가 마지막이라 이긴다(C5)★ — 순서를 바꾸지 말 것. 세 크기는 같은 쪽(셋 다 바깥 · 셋 다 안쪽)이어야 한다.
// ADR-0295
pub fn dragged_size(restored: SizeCss, maximized: Option<SizeCss>, work: SizeCss) -> SizeCss {
    let mut s = restored;
    if s.w > work.w {
        s.w = (work.w - 2.0 * MAXIMIZED_INSET_DIP).max(0.0);
    }
    if s.h > work.h {
        s.h = (work.h - 2.0 * MAXIMIZED_INSET_DIP).max(0.0);
    }
    if let Some(m) = maximized {
        s.w = s.w.max(m.w / 2.0);
        s.h = s.h.max(m.h / 2.0);
    }
    s
}

/// 끄기 시작 때 출처 창의 사실 — 출처 웹뷰가 `window:frame` · `screen.avail*` · 뷰포트에서 읽는 것과 같다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceWindow {
    /// 복원(보통) 안쪽 크기(`window:frame.restored`) — 모르면 `None`(지금 크기로 친다).
    pub restored: Option<SizeCss>,
    pub maximized: bool,
    /// 창 틀 = 바깥 − 안쪽(`window:frame.insets`).
    pub insets: Insets,
    /// 지금 안쪽 크기(뷰포트) — 최대화면 최대화 안쪽 크기다.
    pub viewport: SizeCss,
    /// 끄는 창 모니터의 작업 영역(바깥 크기의 한계 — `screen.availWidth` · `availHeight`).
    pub work: SizeCss,
    /// 새 창(보통 상태로 뜬다)의 크롬 = 뷰포트 − 탭 캔버스(제목 줄 높이 · 왼쪽 여백 따위) + 제목 줄 높이 차(보통 − 출처의
    /// 지금 — 출처가 최대화면 0 이 아니다 · TRD S26-window-chrome §7-6). 그 차는 부르는 쪽이 더해 넘긴다 — 웹 거울은 더한다
    /// (`useTabDragSource` 의 `rowGrowth`). 셸에는 아직 운영 소비자가 없다(골든만 — 첫 소비자는 LLM 명령 T-65).
    pub chrome: SizeCss,
    /// 탭 하나뿐인 팝아웃 — 새 창을 만들지 않고 그 창을 옮긴다(Q34).
    pub single: bool,
}

/// [`drag_sizes`] 의 답.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragSizes {
    /// 새 창 안쪽 크기(R22) — `None` = 탭 하나뿐인 팝아웃(그 창을 크기 그대로 옮긴다).
    pub size: Option<SizeCss>,
    /// 끄는 탭의 원래 크기(R7 — 붙는 몫의 재료) = 캔버스 크기. 음수는 0 으로 친다.
    pub original: SizeCss,
}

/// 끄는 탭의 새 창 크기와 원래 크기(TRD §4-2 「크기」).
///
/// - 탭이 둘 이상인 창 — `size` = [`dragged_size`] 를 안쪽 크기로 옮긴 것: 복원 크기(없으면 지금 크기) · 작업 영역 −
///   틀 · 최대화면 그 안쪽 크기. `original` = `size` − 크롬.
/// - 탭 하나뿐인 팝아웃 — `size` 없음 · `original` = (최대화면 복원 크기 · 아니면 지금 크기) − 크롬.
pub fn drag_sizes(s: &SourceWindow) -> DragSizes {
    let canvas = |v: SizeCss| SizeCss {
        w: (v.w - s.chrome.w).max(0.0),
        h: (v.h - s.chrome.h).max(0.0),
    };
    if s.single {
        let shown = match (s.maximized, s.restored) {
            (true, Some(r)) => r,
            _ => s.viewport,
        };
        return DragSizes {
            size: None,
            original: canvas(shown),
        };
    }
    let work = SizeCss {
        w: s.work.w - (s.insets.l + s.insets.r),
        h: s.work.h - (s.insets.t + s.insets.b),
    };
    let size = dragged_size(
        s.restored.unwrap_or(s.viewport),
        s.maximized.then_some(s.viewport),
        work,
    );
    DragSizes {
        size: Some(size),
        original: canvas(size),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dip(w: f64, h: f64) -> SizeCss {
        SizeCss { w, h }
    }

    const NO_FRAME: Insets = Insets {
        t: 0.0,
        r: 0.0,
        b: 0.0,
        l: 0.0,
    };

    // ── 복원 크기 ─────────────────────────────────────────────────────────────

    #[test]
    fn the_restored_size_adds_the_frame_to_the_model_bounds() {
        let bounds = WindowBounds::new(100.0, 100.0, 800.0, 600.0);
        assert_eq!(
            restored_size(bounds, dip(16.0, 47.0), dip(1920.0, 1040.0)),
            dip(816.0, 647.0)
        );
        assert_eq!(
            restored_size(None, dip(16.0, 47.0), dip(1920.0, 1040.0)),
            dip(1920.0, 1040.0),
            "보통 자리를 못 봤다"
        );
        assert_eq!(
            restored_size(bounds, dip(0.0, 0.0), dip(1.0, 1.0)),
            dip(800.0, 600.0),
            "안쪽 크기 = 틀 0"
        );
    }

    // ── 새 창 크기 (C5) ───────────────────────────────────────────────────────

    #[test]
    fn a_restored_size_inside_the_work_area_is_kept() {
        assert_eq!(
            dragged_size(dip(800.0, 600.0), None, dip(1920.0, 1032.0)),
            dip(800.0, 600.0)
        );
    }

    #[test]
    fn a_side_larger_than_the_work_area_shrinks_to_it_less_both_insets() {
        assert_eq!(
            dragged_size(dip(2000.0, 600.0), None, dip(1920.0, 1032.0)),
            dip(1900.0, 600.0)
        );
        assert_eq!(
            dragged_size(dip(800.0, 1100.0), None, dip(1920.0, 1032.0)),
            dip(800.0, 1012.0)
        );
        assert_eq!(
            dragged_size(dip(30.0, 30.0), None, dip(15.0, 25.0)),
            dip(0.0, 5.0),
            "작업 영역이 여백보다 작으면 0"
        );
        assert_eq!(
            dragged_size(dip(1920.0, 600.0), None, dip(1920.0, 1032.0)).w,
            1920.0,
            "같으면 줄이지 않는다"
        );
    }

    #[test]
    fn a_maximized_source_floors_each_side_at_half_its_size() {
        assert_eq!(
            dragged_size(
                dip(300.0, 200.0),
                Some(dip(1936.0, 1056.0)),
                dip(1920.0, 1032.0)
            ),
            dip(968.0, 528.0)
        );
        assert_eq!(
            dragged_size(
                dip(1200.0, 800.0),
                Some(dip(1936.0, 1056.0)),
                dip(1920.0, 1032.0)
            ),
            dip(1200.0, 800.0),
            "절반보다 크면 그대로"
        );
    }

    #[test]
    fn the_half_floor_comes_last_and_beats_the_work_area_cap() {
        // 작은 모니터(작업 영역 800 × 600)로 커서가 갔고 큰 최대화 창(2560 × 1400)에서 뗐다 — 작업 영역 상한 뒤에
        // 절반 하한이 와서 이긴다(TDC#2760-2791).
        assert_eq!(
            dragged_size(
                dip(1000.0, 700.0),
                Some(dip(2560.0, 1400.0)),
                dip(800.0, 600.0)
            ),
            dip(1280.0, 700.0)
        );
    }

    // ── 새 창 크기 · 원래 크기 (§4-2) ─────────────────────────────────────────

    fn source() -> SourceWindow {
        SourceWindow {
            restored: Some(dip(1000.0, 700.0)),
            maximized: false,
            insets: Insets {
                t: 31.0,
                r: 8.0,
                b: 8.0,
                l: 8.0,
            },
            viewport: dip(1000.0, 700.0),
            work: dip(1920.0, 1032.0),
            chrome: dip(0.0, 36.0),
            single: false,
        }
    }

    #[test]
    fn a_multi_tab_window_gives_its_restored_inner_size_and_the_canvas_inside_it() {
        assert_eq!(
            drag_sizes(&source()),
            DragSizes {
                size: Some(dip(1000.0, 700.0)),
                original: dip(1000.0, 664.0),
            }
        );
        let unknown = SourceWindow {
            restored: None,
            viewport: dip(900.0, 500.0),
            ..source()
        };
        assert_eq!(
            drag_sizes(&unknown).size,
            Some(dip(900.0, 500.0)),
            "복원 크기를 모르면 지금 크기"
        );
    }

    #[test]
    fn the_work_area_cap_is_taken_inside_the_frame() {
        // 작업 영역 1920 × 1032 − 틀(16 × 39) = 1904 × 993 → 넘는 변은 그 − 20.
        let s = SourceWindow {
            restored: Some(dip(1910.0, 1000.0)),
            ..source()
        };
        assert_eq!(drag_sizes(&s).size, Some(dip(1884.0, 973.0)));
    }

    #[test]
    fn a_maximized_multi_tab_window_floors_at_half_its_maximized_inner_size() {
        let s = SourceWindow {
            restored: Some(dip(300.0, 200.0)),
            maximized: true,
            viewport: dip(1920.0, 1001.0),
            ..source()
        };
        assert_eq!(
            drag_sizes(&s),
            DragSizes {
                size: Some(dip(960.0, 500.5)),
                original: dip(960.0, 464.5),
            }
        );
    }

    #[test]
    fn a_single_tab_popout_moves_whole_and_reports_its_shown_canvas() {
        let s = SourceWindow {
            single: true,
            viewport: dip(640.0, 480.0),
            ..source()
        };
        assert_eq!(
            drag_sizes(&s),
            DragSizes {
                size: None,
                original: dip(640.0, 444.0),
            }
        );
        let maximized = SourceWindow {
            maximized: true,
            viewport: dip(1920.0, 1001.0),
            ..s
        };
        assert_eq!(
            drag_sizes(&maximized).original,
            dip(1000.0, 664.0),
            "최대화면 복원 크기 − 크롬"
        );
        let maximized_unknown = SourceWindow {
            restored: None,
            ..maximized
        };
        assert_eq!(drag_sizes(&maximized_unknown).original, dip(1920.0, 965.0));
    }

    #[test]
    fn the_canvas_never_goes_negative() {
        let s = SourceWindow {
            restored: Some(dip(10.0, 20.0)),
            chrome: dip(30.0, 36.0),
            insets: NO_FRAME,
            ..source()
        };
        assert_eq!(drag_sizes(&s).original, dip(0.0, 0.0));
    }
}
