//! 도킹 띠 기하의 옛 자리 — 정본은 `layout::bands` 로 옮겨 갔다(TRD S21-slot-docking 7판 §4-5). 옛 끌기 엔진이
//! 그대로 컴파일되게 다시 내보낸다 — 이 파일은 옛 엔진과 함께 U4c 가 지운다.

pub use crate::layout::bands::{
    band_frame, bands_at, choose, effective_bands, frame_at, frames, landing_rect, outer_sides,
    outer_strips, slot_band, slot_centre, slot_zones, subtract, zone_in_canvas, BandHit, SlotZones,
    WindowTargets, BAND_MIN_PX, BAND_SHARE, OUTER_BAND_PX, SIDE_ORDER,
};
