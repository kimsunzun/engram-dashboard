//! 탭바 구역 — 떼어진 창이 닿으면 곧바로 붙는 자리이자 출처 · 붙은 창 웹뷰가 「나갔나」를 보는 자리(TRD
//! S21-slot-docking §5-2 · C3). 화면 물리 점을 창 뷰포트 CSS 점으로 옮기는 셈도 여기다.
//!
//! ★나갈 때 · 들어올 때 같은 사각형이다(PRD D19)★ — 웹뷰는 셸이 준 [`strip_zone`] 값에 대해 점이 안인지만 보고,
//! 셸은 이동 루프 동안 같은 함수를 부른다. 이력(hysteresis)이 없다.

use super::units::PhysPoint;
use crate::layout::chrome::{PointCss, RectCss};

/// 탭바 위아래로 넓히는 폭(CSS px) — Chrome `kVerticalDetachMagnetism`(TDC#401 · 조사 A5). 터치 값(50)은 쓰지
/// 않는다 — 마우스만 다룬다.
// ADR-0295
pub const DETACH_MAGNETISM_PX: f64 = 15.0;

/// 탭바 구역 — `area`(그 창 크롬 보고의 `tab_bar` = 탭 끌기 영역 사각형 · 뷰포트 CSS px)의 가로 그대로 · 세로 위아래
/// [`DETACH_MAGNETISM_PX`]. Chrome `DoesTabStripContain`(TDC#1461-1477 · `DoesRectContainVerticalPointExpanded`
/// TDC#234) 그대로라 가로는 탭바 줄 전체도 탭들 범위도 아니다(G7).
// ADR-0295
pub fn strip_zone(area: RectCss) -> RectCss {
    RectCss {
        x0: area.x0,
        y0: area.y0 - DETACH_MAGNETISM_PX,
        x1: area.x1,
        y1: area.y1 + DETACH_MAGNETISM_PX,
    }
}

/// 반열림 판정(왼쪽 · 위 포함) — 이웃 사각형이 같은 경계를 나눠도 점 하나가 한쪽에만 든다. NaN 점은 어디에도 없다.
pub fn contains(r: &RectCss, p: PointCss) -> bool {
    (r.x0..r.x1).contains(&p.x) && (r.y0..r.y1).contains(&p.y)
}

/// 화면 물리 점 `p` 를 안쪽 원점이 `inner_origin`(물리) · 배율이 `scale` 인 창의 뷰포트 CSS 점으로 = `(p − 원점) ÷
/// 배율`. 창이 다른 모니터에 걸쳐 있어도 그 창 배율 하나로 읽는다 — 웹뷰가 그 배율 하나로 그린다.
pub fn client_point(p: PhysPoint, inner_origin: PhysPoint, scale: f64) -> PointCss {
    PointCss {
        x: (f64::from(p.x) - f64::from(inner_origin.x)) / scale,
        y: (f64::from(p.y) - f64::from(inner_origin.y)) / scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> RectCss {
        RectCss { x0, y0, x1, y1 }
    }

    fn pt(x: f64, y: f64) -> PointCss {
        PointCss { x, y }
    }

    // 끌기 영역 = 첫 탭 왼쪽 4 부터 폭 500(`+` 는 그 오른쪽 · C3) · 탭바 줄 0..28.
    const AREA: RectCss = RectCss {
        x0: 4.0,
        y0: 0.0,
        x1: 504.0,
        y1: 28.0,
    };

    #[test]
    fn the_zone_is_the_drag_area_widened_15_up_and_down_only() {
        assert_eq!(strip_zone(AREA), rect(4.0, -15.0, 504.0, 43.0));
    }

    #[test]
    fn the_zone_is_half_open_on_both_axes() {
        let z = strip_zone(AREA);
        for (p, inside, why) in [
            (pt(4.0, 10.0), true, "끌기 영역 왼쪽 변"),
            (pt(3.999, 10.0), false, "첫 탭 왼쪽 앞"),
            (pt(503.999, 10.0), true, "끌기 영역 마지막"),
            (pt(504.0, 10.0), false, "끌기 영역 오른쪽 변 = `+` 자리"),
            (pt(200.0, -15.0), true, "위 15 의 첫 줄"),
            (pt(200.0, -15.001), false, "위 15 밖"),
            (pt(200.0, 42.999), true, "아래 15 의 마지막"),
            (pt(200.0, 43.0), false, "아래 15 = 떼기"),
            (pt(f64::NAN, 10.0), false, "NaN"),
        ] {
            assert_eq!(contains(&z, p), inside, "{why}");
        }
    }

    #[test]
    fn a_screen_point_maps_to_css_through_the_inner_origin_and_scale() {
        let origin = PhysPoint { x: 100, y: 50 };
        let p = PhysPoint { x: 350, y: 150 };
        for (scale, want) in [
            (1.0, pt(250.0, 100.0)),
            (1.25, pt(200.0, 80.0)),
            (2.0, pt(125.0, 50.0)),
        ] {
            assert_eq!(client_point(p, origin, scale), want, "배율 {scale}");
        }
    }

    #[test]
    fn a_window_on_a_left_monitor_reads_negative_screen_coordinates() {
        let origin = PhysPoint { x: -1900, y: -40 };
        assert_eq!(
            client_point(PhysPoint { x: -1650, y: 60 }, origin, 1.25),
            pt(200.0, 80.0)
        );
        assert_eq!(
            client_point(PhysPoint { x: -1910, y: -40 }, origin, 2.0),
            pt(-5.0, 0.0),
            "창 왼쪽 밖은 음수"
        );
    }
}
