pub mod agent;
pub mod autostart;
pub mod discovery;
pub mod first_paint;
pub mod layout;
pub mod popout;
pub mod settings;
// Snap layout overlay invoke `report_max_button_rect` (TRD S26-window-chrome §8-4) — behind the shell feature `snap-overlay` (default off ·
//   user decision Q1 2026-10-24).
#[cfg(feature = "snap-overlay")]
pub mod snap_overlay;
// The shell-side window management of spare windows — spare window pool · building · role answer (TRD S21-slot-docking §3-9).
pub mod spare;
pub mod state;
// Two tab-drop invokes (`tab_drop` · `window_frame`) — the one command after a drag ends · window facts (ADR-0296).
pub mod tab_drop;
pub mod tray;
pub mod usage;
// ADR-0155: reports of webview-owned commands · outcome retrieval (TRD §6 Step 4).
pub mod view_bus;
// The Tauri adapter of the window-state port · invoke `window_control` (TRD S26-window-chrome §6-2 · §6-4).
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
