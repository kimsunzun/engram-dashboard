//! Canonical size computation for a drag (TRD S21-slot-docking §4-2 · §4-5 · PRD R7 · R22) — new window size (Chrome `CalculateDraggedWindowSize`) · original size of the dragged tab; unit = logical (DIP) px. One set with the web mirror: the source webview computes it with the same rule (`dock/dragSize.ts`) at dragstart and sends it; both consume the `drag_size` table of the golden `bands_golden.json`. The constants are one set at the head of `bands.rs`.

use super::bands::{SizeCss, MAXIMIZED_INSET_DIP};
use super::geometry::Insets;
use super::manager::WindowBounds;

/// Restored (normal) size of the source window — the model's normal place size (`bounds` — inner logical size · normal size even while maximized) plus `frame` (the frame added to that size — 0 when computing the inner size); no model value → `current` (the current size of the same side).
pub fn restored_size(bounds: Option<WindowBounds>, frame: SizeCss, current: SizeCss) -> SizeCss {
    match bounds {
        Some(b) => SizeCss {
            w: b.w() + frame.w,
            h: b.h() + frame.h,
        },
        None => current,
    }
}

/// New window size — exactly Chrome `CalculateDraggedWindowSize` (TDC#2760-2791): from `restored`, ① a side larger than the work area `work` becomes `max(0, work − 2 × MAXIMIZED_INSET_DIP)` ② if the source is maximized (`maximized` = that size), each side is raised to at least half of it. ② comes last, so it wins (C5) — do not change the order. The three sizes must be on the same side (all outer · all inner).
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceWindow {
    pub restored: Option<SizeCss>,
    pub maximized: bool,
    pub insets: Insets,
    pub viewport: SizeCss,
    pub work: SizeCss,
    /// Chrome of the new window (it opens in the normal state) = viewport − tab canvas (title bar height · left margin etc.) + title bar height difference (normal − the source's current; not 0 if the source is maximized · TRD S26-window-chrome §7-6). The caller adds the difference and passes it in; the web mirror adds it (`rowGrowth` of `useTabDragSource`). The shell has no production consumer yet (golden only — the first is the LLM command T-65).
    pub chrome: SizeCss,
    pub single: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragSizes {
    pub size: Option<SizeCss>,
    pub original: SizeCss,
}

/// New window size and original size of the dragged tab (TRD §4-2 「size」). A window with two or more tabs: `size` = [`dragged_size`] converted to inner size (restored size, or current if none · work area − frame · if maximized, that inner size); `original` = `size` − chrome. A popout with one tab: no `size`; `original` = (restored size if maximized, else current size) − chrome.
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

    // ── restored size ─────────────────────────────────────────────────────────

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
            "the normal place was not seen"
        );
        assert_eq!(
            restored_size(bounds, dip(0.0, 0.0), dip(1.0, 1.0)),
            dip(800.0, 600.0),
            "inner size = frame 0"
        );
    }

    // ── new window size (C5) ──────────────────────────────────────────────────

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
            "0 if the work area is smaller than the margin"
        );
        assert_eq!(
            dragged_size(dip(1920.0, 600.0), None, dip(1920.0, 1032.0)).w,
            1920.0,
            "not reduced if equal"
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
            "unchanged if larger than half"
        );
    }

    #[test]
    fn the_half_floor_comes_last_and_beats_the_work_area_cap() {
        // The cursor went to a small monitor (work area 800 × 600) and the tab was torn off a large maximized window (2560 × 1400) — the half
        // lower bound comes after the work area upper bound and wins (TDC#2760-2791).
        assert_eq!(
            dragged_size(
                dip(1000.0, 700.0),
                Some(dip(2560.0, 1400.0)),
                dip(800.0, 600.0)
            ),
            dip(1280.0, 700.0)
        );
    }

    // ── new window size · original size (§4-2) ────────────────────────────────

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
            "the current size if the restore size is unknown"
        );
    }

    #[test]
    fn the_work_area_cap_is_taken_inside_the_frame() {
        // Work area 1920 × 1032 − frame (16 × 39) = 1904 × 993 → a side that exceeds it becomes that − 20.
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
            "if maximized, restore size − chrome"
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
