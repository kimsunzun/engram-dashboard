pub mod agent;
pub mod autostart;
pub mod discovery;
// 도킹의 셸 쪽 창 관리 — 끌기 세션 밖(TRD S21-slot-docking §2-5).
pub mod dock;
// 탭 끌기 세션의 포트 어댑터 · 실행기 세우기(TRD S21-slot-docking §2-4).
pub mod drag;
pub mod drag_window;
pub mod first_paint;
pub mod layout;
pub mod popout;
pub mod settings;
pub mod state;
// 탭 놓기 invoke 둘(`tab_drop` · `window_frame`) — 끌기가 끝난 뒤의 명령 하나 · 창 사실(ADR-0296).
pub mod tab_drop;
pub mod tray;
pub mod usage;
// ADR-0155: 웹뷰 몫 명령의 보고·결말 회수(TRD §6 Step 4).
pub mod view_bus;

pub use agent::*;
pub use autostart::*;
pub use discovery::*;
pub use dock::*;
pub use first_paint::*;
pub use layout::*;
pub use popout::*;
pub use settings::*;
pub use state::*;
pub use tray::*;
pub use usage::*;
pub use view_bus::*;
