//! The Tauri adapter of shell settings · per-window theme — four settings commands · a boot query command · the settings notification · theme push.
//!
//! ★There is no logic in this file★: setting values · validation · storage are owned by `crate::settings`, the per-window effective theme by `crate::theme`,
//! and what remains here is only translation into the Tauri world (query response · **counting live webviews** · `emit`/`emit_to`)
//! (the same division as `commands/layout.rs`).
//! The four settings commands are a second shell that calls the same service · the same write entry point as the bus `settings.*` (`layout::commands`)
//! (ADR-0081 decision 3).
//!
//! ## ★Why there are two places that read the per-window theme★
//! - **Boot = the frontend pulls** (`get_ui_settings`). The shell does not know when a window has finished loading its scripts, so
//!   if it pushed, the first value would be lost — the very race layout went through (ADR-0102).
//! - **After that = the shell pushes** (`window.setTheme` · `theme.default` write · boot · restore acceptance). The frontend does not know when
//!   the value changed.
//!
//! Both call the same [`EffectiveThemes`], so the answers do not diverge. And both **pick the value by window label** —
//! if only one side knew the window, that window would see a different theme at boot and at push.
// ADR-0167

use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State, Window};

use crate::settings::{
    reset_and_notify, set_and_notify, ResetOutcome, SetOutcome, SettingsEvents, SettingsSchema,
    SettingsService, SettingsSnapshot,
};
use crate::theme::{EffectiveThemes, ThemeControl, ThemeWindows, UiSettingsPayload, UiTheme};
use crate::webview_env;

/// Each window receives **its own value** — it is sent by pointing at the destination, so the receiving side must also subscribe by its own label
/// (`src/theme/uiSettings.ts` · reason = `theme::deliver_per_window`).
const EVT_UI_SETTINGS_UPDATED: &str = "ui:settings-updated";

/// The settings changed — **the same envelope to all webviews** (settings do not differ per window). Payload = [`SettingsSnapshot`]
/// (`rev` + only the changed keys). The per-window theme is carried not by this event but by [`EVT_UI_SETTINGS_UPDATED`].
const EVT_SETTINGS_CHANGED: &str = "settings:changed";

/// Boot query — the frontend pulls once per window.
///
/// ★It does not take the window label as an argument★: if the webview declared it itself, one wrongly written label would take another window's
/// theme. The [`Window`] that Tauri injects is the sole authority for that value (the same clause in `commands/view_bus.rs` —
/// that doc also carries the relation between the **window** label and the webview label).
///
/// ★Do not take the push-order lock here (intended)★: this query answers only the one window that called and pushes nothing, so there is
/// nothing to reverse in the **order between windows**. If a lock were taken here, a window's boot would just queue behind someone else's push.
///
/// What the receiver's latch (`pushed` in `src/theme/uiSettings.ts`) covers is **one direction only** — a query answer overwriting
/// a notification newer than it. The reverse is not covered: while a push has read "light" and paused, the value changes to "e-ink", a new window's
/// query receives "e-ink" and draws it, then the old notification of that push arrives and reverts that window to "light".
/// ★Even so it is not a defect★ — the side that changed the value pushes again after saving, so that push corrects it. To fix it, the query would have to carry a generation
/// too, and that changes the answer's shape.
#[tauri::command]
pub fn get_ui_settings(
    window: Window,
    themes: State<'_, Arc<EffectiveThemes>>,
) -> UiSettingsPayload {
    themes.for_window(window.label())
}

/// The human path of `settings.get` — `key` = an exact key or a prefix ending in `.` · if omitted, everything.
/// The error messages are the service's as they are (kinds = bus-side codes NOT_FOUND · INVALID_ARGUMENT). The two writes ([`settings_set`] ·
/// [`settings_reset`]) also keep the messages as they are and have two more kinds — CONFLICT (a file written by a newer version) · INTERNAL.
#[tauri::command]
pub fn settings_get(
    settings: State<'_, Arc<SettingsService>>,
    key: Option<String>,
) -> Result<SettingsSnapshot, String> {
    settings.get(key.as_deref()).map_err(|e| e.to_string())
}

/// The human path of `settings.set`. If something actually changed, [`EVT_SETTINGS_CHANGED`] goes out to all webviews,
/// and if it is `theme.default`, the effective theme of every window is pushed again.
///
/// ★Runs on the blocking pool★ — this write waits until the disk `sync_all`. A synchronous command runs on the main thread, so
/// window events stop in the meantime, and calling it directly from an async body would stall other tasks sitting on that runtime worker.
#[tauri::command]
pub async fn settings_set(
    app: AppHandle,
    settings: State<'_, Arc<SettingsService>>,
    themes: State<'_, Arc<EffectiveThemes>>,
    key: String,
    value: String,
) -> Result<SetOutcome, String> {
    let events = TauriSettingsEvents::new(app.clone(), theme_control(app, Arc::clone(&themes)));
    let settings = Arc::clone(&settings);
    off_the_runtime(move || set_and_notify(&settings, &events, &key, &value)).await
}

/// The human path of `settings.reset` — the notification · blocking-pool reasons are the same as [`settings_set`].
#[tauri::command]
pub async fn settings_reset(
    app: AppHandle,
    settings: State<'_, Arc<SettingsService>>,
    themes: State<'_, Arc<EffectiveThemes>>,
    key: String,
) -> Result<ResetOutcome, String> {
    let events = TauriSettingsEvents::new(app.clone(), theme_control(app, Arc::clone(&themes)));
    let settings = Arc::clone(&settings);
    off_the_runtime(move || reset_and_notify(&settings, &events, &key)).await
}

/// Run the settings write to the end on the blocking pool. ★The write outlives the answer★ — even if the waiting side disappears, the write ·
/// notification goes through to the end (the value remains on disk).
async fn off_the_runtime<T: Send + 'static>(
    write: impl FnOnce() -> Result<T, crate::settings::SettingsError> + Send + 'static,
) -> Result<T, String> {
    match tauri::async_runtime::spawn_blocking(write).await {
        Ok(outcome) => outcome.map_err(|e| e.to_string()),
        Err(e) => Err(format!("설정 쓰기가 끝나지 못했다: {e}")),
    }
}

/// The human path of `settings.schema` — `key` is the same as [`settings_get`].
#[tauri::command]
pub fn settings_schema(
    settings: State<'_, Arc<SettingsService>>,
    key: Option<String>,
) -> Result<SettingsSchema, String> {
    settings
        .schema(key.as_deref())
        .map(|items| SettingsSchema { items })
        .map_err(|e| e.to_string())
}

/// A handle that attaches the Tauri window side to the shell's single effective theme — held by the command table · settings notification · restore coordinator · boot.
/// `themes` = the very instance `lib.rs` managed in the builder (if different, the order locks of the two pushes diverge).
pub fn theme_control(app: AppHandle, themes: Arc<EffectiveThemes>) -> ThemeControl {
    ThemeControl::new(themes, Arc::new(TauriThemeWindows { app }))
}

/// The window side of the theme push — the list of live webviews and `emit_to` to one window.
struct TauriThemeWindows {
    app: AppHandle,
}

impl ThemeWindows for TauriThemeWindows {
    // ★The list is taken from Tauri — not the layout registry★. The two diverge in both directions — a new window (slot pop-out
    //   `move_slot_to_window` · empty new window `create_window` · docking new window) and restore acceptance (TRD S21-storage §6-7 ②) make the webview
    //   first and put it in the model later, while boot restore (`state::placement::open_restored_popouts`) makes the webview of a pop-out already in the model
    //   later. What is counted here is 「지금 살아 있는
    //   웹뷰」 (the webviews alive now), and the grounds that its label equals the value the webview subscribes with are in the same clause of `view_commands`.
    fn labels(&self) -> Vec<String> {
        self.app.webview_windows().into_keys().collect()
    }

    // Called while holding the theme gate — there is no Rust-side receiver (it goes out only to the webview), so there is no path to retake that lock. Painting the background also
    //   either only hands it to the event loop (another thread) or changes just the color on the spot (main thread — tauri-runtime-wry 2.11.3
    //   `send_user_message`) — it does not take the shell's locks.
    fn send(&self, label: &str, payload: UiSettingsPayload) -> Result<(), String> {
        if let (Some(window), Some(theme)) = (
            self.app.get_webview_window(label),
            UiTheme::from_wire(&payload.theme),
        ) {
            webview_env::repaint(&window, theme);
        }
        self.app
            .emit_to(label, EVT_UI_SETTINGS_UPDATED, payload)
            .map_err(|e| e.to_string())
    }
}

/// The real implementation of the settings-write notification — the human path (settings command) and the LLM path (bus table) use the same implementation.
pub struct TauriSettingsEvents {
    app: AppHandle,
    themes: ThemeControl,
}

impl TauriSettingsEvents {
    pub fn new(app: AppHandle, themes: ThemeControl) -> Self {
        Self { app, themes }
    }
}

// Both swallow failure (log only) — the value is already on disk and in memory, and losing a notification is no reason to undo that write.
//   The frontend catches up by pulling with `settings_get` · `get_ui_settings`.
impl SettingsEvents for TauriSettingsEvents {
    // Called under the notification order lock (trait doc) — `emit` only hands the webview script execution to the event loop and does not wait for that
    //   execution.
    fn changed(&self, change: &SettingsSnapshot) {
        if let Err(e) = self.app.emit(EVT_SETTINGS_CHANGED, change) {
            tracing::warn!(
                module = "settings",
                event = EVT_SETTINGS_CHANGED,
                rev = change.rev,
                "설정 변경 알림을 못 보냈다: {e}"
            );
        }
    }

    fn theme_default_changed(&self) {
        self.themes.push();
    }
}
