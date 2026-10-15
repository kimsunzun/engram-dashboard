//! 슬롯 도킹의 끌기 쪽(TRD S21-slot-docking §1-3 · §2) — 레이아웃 권위(`crate::layout` · ADR-0035)를 부르는
//! 손님이다. 레이아웃은 여기서 바꾸지 않는다 — 바꾸는 길은 적용 서비스(`layout::apply`) 하나다.

pub mod hit;
pub mod types;
