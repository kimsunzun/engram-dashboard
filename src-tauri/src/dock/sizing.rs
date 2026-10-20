//! 끌기의 크기 · 자리 셈(TRD S21-slot-docking §5-4 · §5-5) — 원래 크기 · 커서에 가장 가까운 모니터 · 떼어진 창 ·
//! 창째 끄는 창의 자리. 붙는 몫 · 떼어진 창 크기(Chrome `CalculateDraggedWindowSize`)의 정본은 `layout::bands` ·
//! `layout::drag_size` 로 옮겨 갔고 옛 끌기 엔진이 그대로 컴파일되게 여기서 다시 내보낸다(TRD 7판 §4-5 — 이 파일은
//! 옛 엔진과 함께 U4c 가 지운다).
//!
//! 크기 단위: 붙는 몫 · 원래 크기 = 논리(DIP) px 의 캔버스(내용 영역) · 떼어진 창 크기 = 논리 px 의 **바깥** 크기
//! (Chrome 의 `GetRestoredBounds` · `GetWindowBoundsInScreen` 이 창 바깥 경계라서) · 자리 = 물리 px.

pub use crate::layout::bands::{attach_share, MAXIMIZED_INSET_DIP};
pub use crate::layout::drag_size::dragged_size;

use super::units::{Monitor, PhysPoint, PhysSize, SizeDip, WindowGeom};
use crate::layout::chrome::PointCss;
use crate::layout::drag_size;
use crate::layout::manager::WindowBounds;

/// 원래 크기 — 끄는 창 OS 안쪽 크기 ÷ 배율 − `canvas_origin`(그 창 크롬 재기 — 탭바 높이 · 왼쪽 여백)(§5-4 · D4 ·
/// E4). 캔버스 보고(100 ms 디바운스)는 셸이 크기를 막 바꾼 창에서 낡았으므로 쓰지 않는다. 배율이 0 이하 · 유한하지
/// 않으면 `None`(띠를 내지 않는다). 탭바가 안쪽보다 크면 0 으로 친다.
pub fn original_size(client: PhysSize, scale: f64, canvas_origin: PointCss) -> Option<SizeDip> {
    if !(scale.is_finite() && scale > 0.0) {
        return None;
    }
    Some(SizeDip {
        w: (f64::from(client.w) / scale - canvas_origin.x).max(0.0),
        h: (f64::from(client.h) / scale - canvas_origin.y).max(0.0),
    })
}

/// 출처 창의 복원(보통) 바깥 크기 — 떼어진 창 크기의 바탕(TDC#609-610 `initial_window_size_`). `bounds` = 모델의
/// 보통 자리(안쪽 논리 크기 — 최대화 중이어도 보통 크기)에 지금 틀 두께를 더한다 · 없으면 지금 바깥 크기
/// (`layout::drag_size::restored_size` 에 이 창 OS 기하를 넣은 것).
pub fn restored_size(bounds: Option<WindowBounds>, geom: &WindowGeom) -> SizeDip {
    let outer = geom.outer.size();
    let inner = geom.inner.size();
    let dip = |v: i32| f64::from(v) / geom.scale;
    drag_size::restored_size(
        bounds,
        SizeDip {
            w: dip(outer.w - inner.w),
            h: dip(outer.h - inner.h),
        },
        SizeDip {
            w: dip(outer.w),
            h: dip(outer.h),
        },
    )
}

/// 점 `p` 에 가장 가까운 모니터의 자리 — 품은 모니터(반열림)가 있으면 그것 · 아니면 덮은 픽셀까지 거리가 가장 짧은
/// 것 · 같으면 목록 앞의 것(C6 — Chrome `GetDisplayNearestPoint`). Tauri `monitor_from_point` 는 점을 품은 모니터만
/// 돌려주므로(tao `MONITOR_DEFAULTTONULL`) 따로 센다. 목록이 비면 `None`.
pub fn monitor_near(monitors: &[Monitor], p: PhysPoint) -> Option<usize> {
    let mut best: Option<(usize, i64)> = None;
    for (i, m) in monitors.iter().enumerate() {
        let Some(d) = m.rect.distance_sq(p) else {
            continue;
        };
        if best.is_none_or(|(_, b)| d < b) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

/// 커서에 가장 가까운 모니터의 작업 영역 크기(DIP — 그 모니터 배율로).
pub fn work_area_near(monitors: &[Monitor], cursor: PhysPoint) -> Option<SizeDip> {
    let m = &monitors[monitor_near(monitors, cursor)?];
    let size = m.work.size();
    Some(SizeDip {
        w: f64::from(size.w) / m.scale,
        h: f64::from(size.h) / m.scale,
    })
}

/// 떼어진 창의 바깥 원점(물리) = 커서 − (틀 오프셋 + (탭 끌기 영역 원점 + 잡은 자리) × 배율) — 탭 안에서 잡은 상대
/// 자리가 커서 밑에 그대로 남는다(조사 E3 · TDC#2913-2934 · #1789-1814).
///
/// `frame_offset` = 그 창 안쪽 − 바깥 원점([`WindowGeom::frame_offset`] — Alt+Tab 빼기 비트가 틀을 바꾸므로 들인
/// 뒤에 읽은 값) · `strip_origin` = 그 창 크롬 재기의 `tab_bar` 왼쪽 위(뷰포트 CSS) · `grab` = 떼어진 창 탭에서 잡은
/// 자리(`Grab.single_x` · `Grab.y`) · `scale` = 커서 모니터 배율.
pub fn detached_origin(
    cursor: PhysPoint,
    scale: f64,
    frame_offset: PhysPoint,
    strip_origin: PointCss,
    grab: PointCss,
) -> PhysPoint {
    let axis = |c: i32, off: i32, strip: f64, g: f64| {
        (f64::from(c) - f64::from(off) - (strip + g) * scale).round() as i32
    };
    PhysPoint {
        x: axis(cursor.x, frame_offset.x, strip_origin.x, grab.x),
        y: axis(cursor.y, frame_offset.y, strip_origin.y, grab.y),
    }
}

/// 탭 하나 창을 창째 끌 때의 바깥 원점 = 시작 원점 + (커서 − 누른 점)(C4 — Chrome `Drag()` 의 `drag_offset`).
/// 최대화였으면 `start_origin` = 복원한 원점이다. 커서 밑으로 다시 맞추지 않는다.
pub fn whole_window_origin(
    start_origin: PhysPoint,
    down: PhysPoint,
    cursor: PhysPoint,
) -> PhysPoint {
    PhysPoint {
        x: start_origin
            .x
            .saturating_add(cursor.x.saturating_sub(down.x)),
        y: start_origin
            .y
            .saturating_add(cursor.y.saturating_sub(down.y)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock::units::PhysRect;

    fn dip(w: f64, h: f64) -> SizeDip {
        SizeDip { w, h }
    }

    fn r(x0: i32, y0: i32, x1: i32, y1: i32) -> PhysRect {
        PhysRect { x0, y0, x1, y1 }
    }

    fn p(x: i32, y: i32) -> PhysPoint {
        PhysPoint { x, y }
    }

    fn monitor(rect: PhysRect, scale: f64) -> Monitor {
        Monitor {
            rect,
            work: PhysRect {
                y1: rect.y1 - 48,
                ..rect
            },
            scale,
        }
    }

    // ── 원래 크기 · 복원 크기 (§5-4 · §5-5) ──────────────────────────────────

    #[test]
    fn the_original_is_the_inner_size_in_dip_minus_the_canvas_origin() {
        let origin = PointCss { x: 0.0, y: 36.0 };
        assert_eq!(
            original_size(PhysSize { w: 1000, h: 750 }, 1.25, origin),
            Some(dip(800.0, 564.0))
        );
        assert_eq!(
            original_size(
                PhysSize { w: 800, h: 600 },
                1.0,
                PointCss { x: 2.0, y: 30.0 }
            ),
            Some(dip(798.0, 570.0))
        );
        assert_eq!(
            original_size(PhysSize { w: 20, h: 20 }, 1.0, origin),
            Some(dip(20.0, 0.0)),
            "탭바보다 작으면 0"
        );
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                original_size(PhysSize { w: 800, h: 600 }, bad, origin),
                None,
                "{bad}"
            );
        }
    }

    #[test]
    fn the_restored_size_adds_the_frame_to_the_model_bounds() {
        let geom = WindowGeom {
            outer: r(0, 0, 1920, 1040),
            inner: r(8, 39, 1912, 1032),
            scale: 1.0,
        };
        let bounds = WindowBounds::new(100.0, 100.0, 800.0, 600.0);
        assert_eq!(restored_size(bounds, &geom), dip(816.0, 647.0));
        assert_eq!(
            restored_size(None, &geom),
            dip(1920.0, 1040.0),
            "보통 자리를 못 봤다"
        );
        let scaled = WindowGeom { scale: 2.0, ..geom };
        assert_eq!(restored_size(bounds, &scaled), dip(808.0, 623.5));
    }

    // ── 모니터 (C6) ───────────────────────────────────────────────────────────

    fn two_monitors() -> Vec<Monitor> {
        vec![
            monitor(r(0, 0, 1920, 1080), 1.0),
            monitor(r(1920, 0, 3200, 1024), 1.25),
        ]
    }

    #[test]
    fn the_monitor_holding_the_point_wins() {
        let ms = two_monitors();
        assert_eq!(monitor_near(&ms, p(0, 0)), Some(0));
        assert_eq!(monitor_near(&ms, p(1919, 500)), Some(0));
        assert_eq!(
            monitor_near(&ms, p(1920, 500)),
            Some(1),
            "경계 = 오른쪽 모니터"
        );
    }

    #[test]
    fn with_no_monitor_holding_the_point_the_nearest_wins() {
        let ms = two_monitors();
        assert_eq!(
            monitor_near(&ms, p(2500, 1050)),
            Some(1),
            "오른쪽 모니터 아래 틈"
        );
        assert_eq!(monitor_near(&ms, p(-50, 300)), Some(0));
        assert_eq!(monitor_near(&ms, p(5000, -10)), Some(1));
        assert_eq!(monitor_near(&[], p(0, 0)), None);
    }

    #[test]
    fn a_distance_tie_goes_to_the_first_monitor() {
        let ms = vec![
            monitor(r(0, 0, 100, 100), 1.0),
            monitor(r(201, 0, 301, 100), 1.0),
        ];
        assert_eq!(monitor_near(&ms, p(150, 50)), Some(0));
    }

    #[test]
    fn the_work_area_is_read_in_that_monitors_dip() {
        let ms = two_monitors();
        assert_eq!(work_area_near(&ms, p(10, 10)), Some(dip(1920.0, 1032.0)));
        assert_eq!(work_area_near(&ms, p(4000, 10)), Some(dip(1024.0, 780.8)));
        assert_eq!(work_area_near(&[], p(0, 0)), None);
    }

    // ── 자리 (E3 · C4) ────────────────────────────────────────────────────────

    #[test]
    fn the_detached_origin_keeps_the_grab_under_the_cursor() {
        let strip = PointCss { x: 4.0, y: 6.0 };
        let grab = PointCss { x: 50.0, y: 10.0 };
        assert_eq!(
            detached_origin(p(1000, 500), 1.0, p(8, 31), strip, grab),
            p(1000 - 8 - 54, 500 - 31 - 16)
        );
        assert_eq!(
            detached_origin(p(1000, 500), 1.5, p(8, 31), strip, grab),
            p(1000 - 8 - 81, 500 - 31 - 24)
        );
        assert_eq!(
            detached_origin(
                p(-1000, 500),
                1.25,
                p(0, 0),
                strip,
                PointCss { x: 50.5, y: 10.0 }
            ),
            p(-1068, 480),
            "−1000 − 68.125 → 반올림"
        );
    }

    #[test]
    fn the_whole_window_moves_by_the_cursor_delta_since_the_press() {
        assert_eq!(
            whole_window_origin(p(100, 50), p(300, 60), p(310, 75)),
            p(110, 65)
        );
        assert_eq!(
            whole_window_origin(p(-1800, 0), p(-1700, 10), p(-1750, 0)),
            p(-1850, -10)
        );
    }
}
