//! 화면 물리 점을 창 뷰포트 CSS 점으로 옮기는 셈(TRD S21-slot-docking §5-2). 탭바 구역(떼어진 창이 닿으면 곧바로
//! 붙는 자리)의 정본은 `layout::bands` 로 옮겨 갔고, 옛 끌기 엔진이 그대로 컴파일되게 여기서 다시 내보낸다(TRD 7판
//! §4-5 — 이 파일은 옛 엔진과 함께 U4c 가 지운다).

pub use crate::layout::bands::{contains, strip_zone, STRIP_ZONE_PX as DETACH_MAGNETISM_PX};

use super::units::PhysPoint;
use crate::layout::chrome::PointCss;

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

    fn pt(x: f64, y: f64) -> PointCss {
        PointCss { x, y }
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
