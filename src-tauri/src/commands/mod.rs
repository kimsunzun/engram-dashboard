pub mod agent;
pub mod autostart;
pub mod discovery;
pub mod first_paint;
pub mod layout;
pub mod popout;
pub mod settings;
// 스냅 레이아웃 오버레이 invoke `report_max_button_rect`(TRD S26-window-chrome §8-4) — 셸 기능 `snap-overlay` 뒤(기본 꺼짐 ·
//   사용자 결정 Q1 2026-10-24).
#[cfg(feature = "snap-overlay")]
pub mod snap_overlay;
// 예비 창의 셸 쪽 창 관리 — 예비 창 풀 · 짓기 · 역할 답(TRD S21-slot-docking §3-9).
pub mod spare;
pub mod state;
// 탭 놓기 invoke 둘(`tab_drop` · `window_frame`) — 끌기가 끝난 뒤의 명령 하나 · 창 사실(ADR-0296).
pub mod tab_drop;
pub mod tray;
pub mod usage;
// ADR-0155: 웹뷰 몫 명령의 보고·결말 회수(TRD §6 Step 4).
pub mod view_bus;
// 창 상태 포트의 Tauri 어댑터 · invoke `window_control`(TRD S26-window-chrome §6-2 · §6-4).
pub mod window_ctl;

pub use agent::*;
pub use autostart::*;
pub use discovery::*;
pub use first_paint::*;
pub use layout::*;
pub use popout::*;
pub use settings::*;
pub use spare::*;
pub use state::*;
pub use tray::*;
pub use usage::*;
pub use view_bus::*;
