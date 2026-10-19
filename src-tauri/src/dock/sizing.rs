//! 끌기의 크기 · 자리 셈(TRD S21-slot-docking §5-4 · §5-5) — 띠에 붙는 몫 · 원래 크기 · 떼어진 창 크기(Chrome
//! `CalculateDraggedWindowSize`) · 커서에 가장 가까운 모니터 · 떼어진 창 · 창째 끄는 창의 자리.
//!
//! 크기 단위: 붙는 몫 · 원래 크기 = 논리(DIP) px 의 캔버스(내용 영역) · 떼어진 창 크기 = 논리 px 의 **바깥** 크기
//! (Chrome 의 `GetRestoredBounds` · `GetWindowBoundsInScreen` 이 창 바깥 경계라서) · 자리 = 물리 px.

use super::units::{Monitor, PhysPoint, PhysSize, SizeDip, WindowGeom};
use crate::layout::chrome::PointCss;
use crate::layout::dock::DockSide;
use crate::layout::geometry::PxRect;
use crate::layout::manager::WindowBounds;
use crate::layout::tree;

/// 붙는 몫 — 칸(바깥 띠면 캔버스) `target` 의 `side` 쪽에 붙을 때 새것이 감싼 자리에서 차지하는 몫. 나누는 방향의
/// 원래 길이 `L` 과 칸 길이 `T` 로 `min(L, T ÷ 2) ÷ T` 를 `[RATIO_MIN, 0.5]` 로 자른 것(ADR-0295 결정 4 · PRD R7).
///
/// ★어떤 입력에도 유한한 `[tree::RATIO_MIN, 0.5]` 값이다★ — 미리보기(`geometry::landing_rect`)와 쓰기(`Seam.share`)가
/// 같은 이 값을 `docked_tree` 에 넘기고, 쓰기 쪽은 범위 밖을 `DockError::InvalidShare` 로 거절한다. 그래서 칸 길이
/// 0 이면 0.5(상한이 이긴다) · `L` 이 NaN · +∞ 면 상한 · 0 이하면 하한이다.
// ADR-0295
pub fn attach_share(original: SizeDip, side: DockSide, target: PxRect) -> f64 {
    let (l, t) = match side {
        DockSide::Left | DockSide::Right => (original.w, target.x1.saturating_sub(target.x0)),
        DockSide::Top | DockSide::Bottom => (original.h, target.y1.saturating_sub(target.y0)),
    };
    if t == 0 {
        return 0.5;
    }
    let t = f64::from(t);
    // `f64::min` 은 NaN 이 아닌 쪽을 돌려준다 — NaN `L` 은 상한으로 간다.
    let share = l.min(t / 2.0) / t;
    share.clamp(tree::RATIO_MIN, 0.5)
}

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
/// 보통 자리(안쪽 논리 크기 — 최대화 중이어도 보통 크기)에 지금 틀 두께를 더한다 · 없으면 지금 바깥 크기.
pub fn restored_size(bounds: Option<WindowBounds>, geom: &WindowGeom) -> SizeDip {
    let outer = geom.outer.size();
    let inner = geom.inner.size();
    let dip = |v: i32| f64::from(v) / geom.scale;
    match bounds {
        Some(b) => SizeDip {
            w: b.w() + dip(outer.w - inner.w),
            h: b.h() + dip(outer.h - inner.h),
        },
        None => SizeDip {
            w: dip(outer.w),
            h: dip(outer.h),
        },
    }
}

/// Chrome `kMaximizedWindowInset`(TDC#129) — 떼어진 창을 작업 영역보다 이만큼(DIP) 안쪽으로 줄여 만들자마자
/// 최대화되지 않게.
pub const MAXIMIZED_INSET_DIP: f64 = 10.0;

/// 떼어진 창 바깥 크기(DIP) — Chrome `CalculateDraggedWindowSize`(TDC#2760-2791) 그대로: `restored` 에서 ① 작업
/// 영역 `work` 보다 큰 변은 `max(0, work − 20)` ② 떼는 창이 최대화(`maximized` = 그 바깥 크기)면 각 변을 그 절반
/// 이상으로. ★②가 마지막이라 이긴다(C5)★ — 순서를 바꾸지 말 것. 떼기마다 그때 부른다(작업 영역 = 그때 커서 ·
/// 최대화 = 그때 떼는 창).
// ADR-0295
pub fn dragged_size(restored: SizeDip, maximized: Option<SizeDip>, work: SizeDip) -> SizeDip {
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

    fn px(x0: u32, y0: u32, x1: u32, y1: u32) -> PxRect {
        PxRect { x0, y0, x1, y1 }
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

    // ── 붙는 몫 (R7 · AC2 · AC39 · AC44) ──────────────────────────────────────

    // 몫 × 칸 길이 = 붙는 길이(px). 반올림은 실제 칸 경계가 하므로 여기서는 셈 값 그대로 본다.
    fn attached_len(original: SizeDip, side: DockSide, target: PxRect) -> f64 {
        let t = match side {
            DockSide::Left | DockSide::Right => target.x1 - target.x0,
            DockSide::Top | DockSide::Bottom => target.y1 - target.y0,
        };
        attach_share(original, side, target) * f64::from(t)
    }

    #[test]
    fn the_share_is_the_original_length_capped_at_half_the_target() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        for (why, original, side, target, want) in [
            (
                "AC2 — 칸 1200 에 원래 400",
                dip(400.0, 300.0),
                DockSide::Right,
                px(0, 0, 1200, 800),
                400.0,
            ),
            (
                "AC2 — 칸 600 에 원래 400 = 절반",
                dip(400.0, 300.0),
                DockSide::Right,
                px(600, 0, 1200, 800),
                300.0,
            ),
            (
                "AC39 — 600 에 200",
                dip(200.0, 999.0),
                DockSide::Right,
                px(0, 0, 600, 800),
                200.0,
            ),
            (
                "AC39 — 600 에 300",
                dip(300.0, 999.0),
                DockSide::Right,
                px(0, 0, 600, 800),
                300.0,
            ),
            (
                "AC39 — 600 에 400",
                dip(400.0, 999.0),
                DockSide::Left,
                px(0, 0, 600, 800),
                300.0,
            ),
            (
                "AC39 — 작은 칸 50 에 200",
                dip(200.0, 999.0),
                DockSide::Right,
                px(0, 0, 50, 800),
                25.0,
            ),
            (
                "AC39 — 작은 칸 50 에 20",
                dip(20.0, 999.0),
                DockSide::Left,
                px(0, 0, 50, 800),
                20.0,
            ),
            (
                "AC44 — 바깥 위 띠 · 높이 300 · 레이아웃 높이 800",
                dip(999.0, 300.0),
                DockSide::Top,
                px(0, 0, 1200, 800),
                300.0,
            ),
            (
                "AC44 — 레이아웃 높이 500 이면 절반 250",
                dip(999.0, 300.0),
                DockSide::Top,
                px(0, 0, 1200, 500),
                250.0,
            ),
            (
                "아래 = 높이",
                dip(999.0, 120.0),
                DockSide::Bottom,
                px(0, 100, 400, 700),
                120.0,
            ),
        ] {
            assert!(close(attached_len(original, side, target), want), "{why}");
        }
    }

    #[test]
    fn only_the_split_axis_length_counts() {
        let target = px(0, 0, 1000, 600);
        let a = attach_share(dip(200.0, 10.0), DockSide::Left, target);
        let b = attach_share(dip(200.0, 5000.0), DockSide::Left, target);
        assert_eq!(a, b);
        assert_eq!(a, 0.2);
    }

    #[test]
    fn a_tiny_original_is_held_at_the_smallest_share() {
        assert_eq!(
            attach_share(dip(3.0, 3.0), DockSide::Left, px(0, 0, 1000, 600)),
            tree::RATIO_MIN
        );
    }

    #[test]
    fn the_share_is_finite_and_in_range_for_every_input() {
        let lengths = [
            0.0,
            -0.0,
            -5.0,
            1e-300,
            0.4,
            12.0,
            300.0,
            1e9,
            f64::MAX,
            f64::MIN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ];
        let targets = [
            px(0, 0, 0, 0),
            px(10, 10, 10, 10),
            px(0, 0, 1, 1),
            px(0, 0, 2, 3),
            px(5, 7, 605, 407),
            px(0, 0, u32::MAX, u32::MAX),
            px(9, 9, 3, 3), // 뒤집힌 사각형 — 길이 0 으로 읽는다
        ];
        for l in lengths {
            for t in targets {
                for side in [
                    DockSide::Left,
                    DockSide::Right,
                    DockSide::Top,
                    DockSide::Bottom,
                ] {
                    let s = attach_share(dip(l, l), side, t);
                    assert!(
                        s.is_finite() && (tree::RATIO_MIN..=0.5).contains(&s),
                        "L {l} · 칸 {t:?} · {side:?} → {s}"
                    );
                }
            }
        }
    }

    #[test]
    fn edge_inputs_go_to_the_documented_ends() {
        let t = px(0, 0, 600, 400);
        assert_eq!(
            attach_share(dip(100.0, 100.0), DockSide::Left, px(0, 0, 0, 400)),
            0.5,
            "칸 0"
        );
        assert_eq!(
            attach_share(dip(f64::NAN, 0.0), DockSide::Left, t),
            0.5,
            "NaN"
        );
        assert_eq!(
            attach_share(dip(f64::INFINITY, 0.0), DockSide::Left, t),
            0.5,
            "+∞"
        );
        assert_eq!(
            attach_share(dip(0.0, 0.0), DockSide::Left, t),
            tree::RATIO_MIN,
            "0"
        );
        assert_eq!(
            attach_share(dip(-1.0, 0.0), DockSide::Left, t),
            tree::RATIO_MIN,
            "음수"
        );
        assert_eq!(
            attach_share(dip(f64::NEG_INFINITY, 0.0), DockSide::Left, t),
            tree::RATIO_MIN,
            "−∞"
        );
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

    // ── 떼어진 창 크기 (C5) ───────────────────────────────────────────────────

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
