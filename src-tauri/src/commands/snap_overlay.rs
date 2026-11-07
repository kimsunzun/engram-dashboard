// ADR-0304 — snap overlay feature
//! The Tauri adapter of the snap layout overlay — invoke `report_max_button_rect` (TRD S26-window-chrome §8-4). ★Behind the shell feature
//! `snap-overlay` · default off★ (user decision Q1 2026-10-24) — this file · its module declaration (`commands/mod.rs`) · the invoke registration
//! (`lib.rs`) are compiled only with that feature. In a build where it is off, the window fact `caption.snap_overlay` is false (`caption_chrome` of
//! `commands/tab_drop.rs`), so the web does not call this invoke.
//!
//! - When the web reports its own maximize-button rectangle (physical px of this window's client), platform `window::snap` places or moves the overlay
//!   there, and with `rect: null` it lifts it. The overlay takes the cursor, so hover changes are returned to that window as the event [`EVT_SNAP_HOVER`]
//!   (body = bool) — the web draws the button hover with that value (`src/components/layout/titleRow/snapOverlay.ts`).
//! - It does not take the window label as an argument — the window Tauri injects is the sole authority for that value (the same rule as `report_window_min_width`).
//!   It is not a bus command — it is a measurement report (ADR-0227).
//! - ★Do not change it to async★ — a synchronous command runs on the main thread, and the overlay window must be built on the parent window's thread (main)
//!   (`window::snap` head of platform 「스레드」). If called from another thread, platform's parent-window-thread guard (`on_parent_thread`)
//!   does nothing and rejects with `Err` — then the overlay never stands.
//! - Do not put in an OS `cfg` — what differs per OS is done by platform (ADR-0266 · TRD §8-3).

use engram_dashboard_platform::window::{snap, PhysRect};
use tauri::{Emitter, WebviewWindow};

/// Overlay hover change — shell → that window. Body = bool (true = entered). Paired with the same-named constant on the web (`snapOverlay.ts`).
pub const EVT_SNAP_HOVER: &str = "window:snap-hover";

/// Maximize-button rectangle — physical px of this window's client (the value the web fitted to the outside by multiplying CSS px by `devicePixelRatio`).
#[derive(Debug, Clone, Copy, serde::Deserialize)]
pub struct MaxButtonRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// `invoke('report_max_button_rect', { rect })` — if `rect` is present, place or move the overlay, and if `null`, lift it. If the width · height is
/// less than 1 or the OS call fails, it is `Err` (that window's overlay stays as is).
#[tauri::command]
pub fn report_max_button_rect(
    window: WebviewWindow,
    rect: Option<MaxButtonRect>,
) -> Result<(), String> {
    let Some(r) = rect else {
        return snap::remove_overlay(&window).map_err(|e| e.to_string());
    };
    let emitter = window.clone();
    snap::place_overlay(
        &window,
        PhysRect {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        },
        Box::new(move |hover| {
            if let Err(e) = emitter.emit_to(emitter.label(), EVT_SNAP_HOVER, hover) {
                tracing::warn!(window = emitter.label(), "{EVT_SNAP_HOVER} emit 실패: {e}");
            }
        }),
    )
    .map_err(|e| e.to_string())
}
