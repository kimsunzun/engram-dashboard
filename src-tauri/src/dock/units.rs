//! 끌기 세션의 단위 타입 — 화면 물리 px(점 · 크기 · 사각형) · 논리(DIP) 크기 · 단조 ms · OS 에서 잰 창 기하와 모니터
//! (TRD S21-slot-docking §3 「units.rs」 · E8).
//!
//! 물리 사각형은 이 crate 의 다른 사각형(`PxRect` · `RectCss`)처럼 경계 꼴 `[x0, x1) × [y0, y1)` 반열림이다. OS 층
//! crate 의 같은 이름 타입(원점 + 크기 꼴)은 실행기 어댑터가 옮긴다 — 순수 모듈이 그 crate 에 기대지 않게.

/// 화면의 물리 px 점(가상 화면 좌표 — 주 모니터 왼쪽 위 원점 · 왼쪽 · 위 모니터면 음수).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PhysPoint {
    pub x: i32,
    pub y: i32,
}

/// 물리 px 크기 — 음수가 아니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PhysSize {
    pub w: i32,
    pub h: i32,
}

/// 화면의 물리 px 사각형 — 경계 꼴 · `x0 <= x1` · `y0 <= y1` · 점을 품는지는 반열림이다(왼쪽 · 위 포함).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PhysRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl PhysRect {
    /// 원점 + 크기 — 끝 경계는 `i32` 한계에서 멈춘다.
    pub fn at(origin: PhysPoint, size: PhysSize) -> Self {
        PhysRect {
            x0: origin.x,
            y0: origin.y,
            x1: origin.x.saturating_add(size.w),
            y1: origin.y.saturating_add(size.h),
        }
    }

    pub fn origin(self) -> PhysPoint {
        PhysPoint {
            x: self.x0,
            y: self.y0,
        }
    }

    pub fn size(self) -> PhysSize {
        PhysSize {
            w: self.x1.saturating_sub(self.x0),
            h: self.y1.saturating_sub(self.y0),
        }
    }

    pub fn contains(self, p: PhysPoint) -> bool {
        (self.x0..self.x1).contains(&p.x) && (self.y0..self.y1).contains(&p.y)
    }

    /// 두 사각형이 픽셀 하나라도 함께 덮나 — 변만 맞닿은 것은 겹침이 아니다.
    pub fn intersects(self, other: PhysRect) -> bool {
        self.x0 < other.x1 && other.x0 < self.x1 && self.y0 < other.y1 && other.y0 < self.y1
    }

    pub fn center(self) -> PhysPoint {
        PhysPoint {
            x: mid(self.x0, self.x1),
            y: mid(self.y0, self.y1),
        }
    }

    /// 점에서 이 사각형이 덮는 가장 가까운 픽셀까지의 거리 제곱 — 품으면 0. 빈 사각형이면 `None`.
    pub fn distance_sq(self, p: PhysPoint) -> Option<i64> {
        if self.x0 >= self.x1 || self.y0 >= self.y1 {
            return None;
        }
        let gap = |v: i32, lo: i32, hi: i32| -> i64 {
            let (v, lo, hi) = (i64::from(v), i64::from(lo), i64::from(hi));
            if v < lo {
                lo - v
            } else if v >= hi {
                v - (hi - 1)
            } else {
                0
            }
        };
        let (dx, dy) = (gap(p.x, self.x0, self.x1), gap(p.y, self.y0, self.y1));
        Some(dx * dx + dy * dy)
    }
}

fn mid(a: i32, b: i32) -> i32 {
    ((i64::from(a) + i64::from(b)) / 2) as i32
}

/// 논리(DIP) px 크기 — 웹뷰 확대 1 의 CSS px 와 같다(TRD §5 머리).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SizeDip {
    pub w: f64,
    pub h: f64,
}

impl SizeDip {
    /// 배율 `scale` 의 물리 크기 — 가장 가까운 정수(.5 는 0 에서 멀리).
    pub fn to_phys(self, scale: f64) -> PhysSize {
        PhysSize {
            w: (self.w * scale).round() as i32,
            h: (self.h * scale).round() as i32,
        }
    }
}

/// 단조 시계의 ms — 원점은 실행기 몫이고 순수 쪽은 차만 본다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct MonoMs(pub u64);

impl MonoMs {
    /// `earlier` 부터 지난 ms — 거꾸로면 0.
    pub fn since(self, earlier: MonoMs) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

/// 우리 창 하나의 OS 기하 — 실행기가 메인 스레드에서 읽어 싣는다. `outer` = 틀 포함 바깥 · `inner` = 웹뷰가 그리는
/// 안쪽(클라이언트) · `scale` = 그 창 배율(물리 ÷ 논리 · 0 보다 크다).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowGeom {
    pub outer: PhysRect,
    pub inner: PhysRect,
    pub scale: f64,
}

impl WindowGeom {
    /// 안쪽 원점 − 바깥 원점(제목 표시줄 · 테두리 몫). 제목 표시줄 높이를 상수로 두지 않는 까닭 = TRD §5-5 · §9.
    pub fn frame_offset(&self) -> PhysPoint {
        PhysPoint {
            x: self.inner.x0.saturating_sub(self.outer.x0),
            y: self.inner.y0.saturating_sub(self.outer.y0),
        }
    }
}

/// 모니터 하나 — `rect` = 모니터 전체 · `work` = 작업 영역(작업 표시줄을 뺀 것) · 둘 다 물리 px · `scale` = 그
/// 모니터 배율(0 보다 크다).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Monitor {
    pub rect: PhysRect,
    pub work: PhysRect,
    pub scale: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x0: i32, y0: i32, x1: i32, y1: i32) -> PhysRect {
        PhysRect { x0, y0, x1, y1 }
    }

    fn p(x: i32, y: i32) -> PhysPoint {
        PhysPoint { x, y }
    }

    #[test]
    fn a_rect_contains_its_top_left_and_not_its_right_or_bottom_edge() {
        let rect = r(-10, 5, 10, 25);
        assert!(rect.contains(p(-10, 5)));
        assert!(rect.contains(p(9, 24)));
        assert!(!rect.contains(p(10, 5)));
        assert!(!rect.contains(p(0, 25)));
        assert!(
            !r(0, 0, 0, 10).contains(p(0, 0)),
            "폭 0 은 아무것도 품지 않는다"
        );
    }

    #[test]
    fn touching_rects_do_not_intersect() {
        let a = r(0, 0, 100, 100);
        assert!(!a.intersects(r(100, 0, 200, 100)));
        assert!(!a.intersects(r(0, 100, 100, 200)));
        assert!(a.intersects(r(99, 99, 200, 200)));
        assert!(a.intersects(r(-50, -50, 1, 1)));
    }

    #[test]
    fn at_origin_and_size_round_trip() {
        let rect = PhysRect::at(p(-1920, 40), PhysSize { w: 800, h: 600 });
        assert_eq!(rect, r(-1920, 40, -1120, 640));
        assert_eq!(rect.origin(), p(-1920, 40));
        assert_eq!(rect.size(), PhysSize { w: 800, h: 600 });
        assert_eq!(rect.center(), p(-1520, 340));
    }

    #[test]
    fn distance_is_to_the_nearest_covered_pixel() {
        let rect = r(0, 0, 100, 50);
        assert_eq!(rect.distance_sq(p(50, 20)), Some(0));
        assert_eq!(
            rect.distance_sq(p(100, 20)),
            Some(1),
            "오른쪽 변 = 마지막 픽셀 다음"
        );
        assert_eq!(rect.distance_sq(p(-3, 20)), Some(9));
        assert_eq!(rect.distance_sq(p(103, 53)), Some(4 * 4 + 4 * 4));
        assert_eq!(r(0, 0, 0, 5).distance_sq(p(0, 0)), None);
    }

    #[test]
    fn a_dip_size_rounds_to_physical_half_away_from_zero() {
        let s = SizeDip { w: 801.0, h: 600.2 };
        assert_eq!(s.to_phys(1.0), PhysSize { w: 801, h: 600 });
        assert_eq!(s.to_phys(1.25), PhysSize { w: 1001, h: 750 });
        assert_eq!(s.to_phys(1.5), PhysSize { w: 1202, h: 900 });
        assert_eq!(s.to_phys(2.0), PhysSize { w: 1602, h: 1200 });
    }

    #[test]
    fn mono_ms_since_never_goes_negative() {
        assert_eq!(MonoMs(1_500).since(MonoMs(1_000)), 500);
        assert_eq!(MonoMs(1_000).since(MonoMs(1_500)), 0);
    }

    #[test]
    fn the_frame_offset_is_inner_origin_minus_outer_origin() {
        let g = WindowGeom {
            outer: r(100, 200, 900, 800),
            inner: r(108, 239, 892, 792),
            scale: 1.25,
        };
        assert_eq!(g.frame_offset(), p(8, 39));
    }
}
