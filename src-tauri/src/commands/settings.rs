//! The Tauri adapter of shell settings · per-window theme — four settings commands · a boot query command · the settings notification · theme push · the UI language hook's ports.
//! No logic in this file: setting values · validation · storage belong to `crate::settings`, the per-window effective theme to `crate::theme`; what remains is translation into the Tauri world (query response · counting live webviews · `emit`/`emit_to`) (same division as `commands/layout.rs`). The four settings commands are a second shell calling the same service · write entry point as the bus `settings.*` (`layout::commands`) (ADR-0081 decision 3).
//! Why two places read the per-window theme:
//! - Boot = the frontend pulls (`get_ui_settings`): the shell does not know when a window has finished loading its scripts, so pushing would lose the first value — the race layout went through (ADR-0102).
//! - After that = the shell pushes (`window.setTheme` · `theme.default` write · boot · restore acceptance): the frontend does not know when the value changed.
//! Both call the same [`EffectiveThemes`] so answers do not diverge, and both pick the value by window label — if only one side knew the window, it would see a different theme at boot and at push.
// ADR-0167

use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State, Window};

use crate::commands::layout::TauriEvents;
use crate::layout::LayoutState;
use crate::settings::{
    reset_and_notify, set_and_notify, ResetOutcome, SetOutcome, SettingsEvents, SettingsSchema,
    SettingsService, SettingsSnapshot,
};
use crate::theme::{EffectiveThemes, ThemeControl, ThemeWindows, UiSettingsPayload, UiTheme};
use crate::tray::TauriTrayText;
use crate::ui_language::LanguageHook;
use crate::webview_env;

/// Each window receives its own value — sent by pointing at the destination, so the receiving side must also subscribe by its own label (`src/theme/uiSettings.ts` · reason = `theme::deliver_per_window`).
const EVT_UI_SETTINGS_UPDATED: &str = "ui:settings-updated";

/// The settings changed — the same envelope to all webviews (settings do not differ per window). Payload = [`SettingsSnapshot`] (`rev` + only the changed keys); the per-window theme goes by [`EVT_UI_SETTINGS_UPDATED`], not this event.
const EVT_SETTINGS_CHANGED: &str = "settings:changed";

/// Boot query — the frontend pulls once per window. It does not take the window label as an argument: a webview declaring it itself could, with one wrongly written label, take another window's theme; the injected [`Window`] is the sole authority (same clause in `commands/view_bus.rs`, whose doc also carries the **window** label vs webview label relation).
/// The push-order lock is intentionally not taken: this query answers only the calling window and pushes nothing, so there is no **inter-window order** to reverse; a lock would only queue a window's boot behind someone else's push.
/// The receiver's latch (`pushed` in `src/theme/uiSettings.ts`) covers one direction only — a query answer overwriting a newer notification. Not covered: a push read "light" and paused, the value changed to "e-ink", a new window's query received and drew "e-ink", then that push's old notification reverts the window to "light". Not a defect — the side that changed the value pushes again after saving, correcting it; fixing it would need the query to carry a generation too, changing the answer's shape.
#[tauri::command]
pub fn get_ui_settings(
    window: Window,
    themes: State<'_, Arc<EffectiveThemes>>,
) -> UiSettingsPayload {
    themes.for_window(window.label())
}

/// The human path of `settings.get` — `key` = an exact key or a prefix ending in `.`, omitted = everything. Error messages are the service's as they are (kinds = bus-side codes NOT_FOUND · INVALID_ARGUMENT); the two writes ([`settings_set`] · [`settings_reset`]) keep messages as they are too and add two kinds — CONFLICT (a newer-version file) · INTERNAL.
#[tauri::command]
pub fn settings_get(
    settings: State<'_, Arc<SettingsService>>,
    key: Option<String>,
) -> Result<SettingsSnapshot, String> {
    settings.get(key.as_deref()).map_err(|e| e.to_string())
}

/// The human path of `settings.set`. If something actually changed [`EVT_SETTINGS_CHANGED`] goes to all webviews, and for `theme.default` every window's effective theme is pushed again. Runs on the blocking pool: this write waits for the disk `sync_all`; a synchronous command runs on the main thread (window events stop meanwhile), and calling it directly from an async body would stall other tasks on that runtime worker.
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

/// Run the settings write to the end on the blocking pool. The write outlives the answer — even if the waiting side disappears, the write · notification go through (the value remains on disk).
async fn off_the_runtime<T: Send + 'static>(
    write: impl FnOnce() -> Result<T, crate::settings::SettingsError> + Send + 'static,
) -> Result<T, String> {
    match tauri::async_runtime::spawn_blocking(write).await {
        Ok(outcome) => outcome.map_err(|e| e.to_string()),
        Err(e) => Err(format!("settings write did not finish: {e}")),
    }
}

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

/// A handle attaching the Tauri window side to the shell's single effective theme — held by the command table · settings notification · restore coordinator · boot. `themes` = the very instance `lib.rs` managed in the builder (a different one would diverge the two pushes' order locks).
pub fn theme_control(app: AppHandle, themes: Arc<EffectiveThemes>) -> ThemeControl {
    ThemeControl::new(themes, Arc::new(TauriThemeWindows { app }))
}

struct TauriThemeWindows {
    app: AppHandle,
}

impl ThemeWindows for TauriThemeWindows {
    // The list comes from Tauri, not the layout registry — the two diverge both ways: a new window (slot pop-out `move_slot_to_window` · empty new window `create_window` · docking new window) and restore acceptance (TRD S21-storage §6-7 ②) make the webview first and the model later, while boot restore (`state::placement::open_restored_popouts`) makes the webview of a pop-out already in the model later. This counts the webviews alive now; why its label equals the label the webview subscribes with is in the same clause of `view_commands`.
    fn labels(&self) -> Vec<String> {
        self.app.webview_windows().into_keys().collect()
    }

    // Called holding the theme gate — no Rust-side receiver (it goes only to the webview), so no path retakes that lock. Painting the background either only hands it to the event loop (another thread) or changes just the color on the spot (main thread — tauri-runtime-wry 2.11.3 `send_user_message`); it takes none of the shell's locks.
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

pub struct TauriSettingsEvents {
    app: AppHandle,
    themes: ThemeControl,
}

impl TauriSettingsEvents {
    pub fn new(app: AppHandle, themes: ThemeControl) -> Self {
        Self { app, themes }
    }
}

// Both swallow failure (log only): the value is already on disk and in memory, and a lost notification is no reason to undo the write; the frontend catches up by pulling `settings_get` · `get_ui_settings`.
impl SettingsEvents for TauriSettingsEvents {
    // Called under the notification order lock (trait doc) — `emit` only hands script execution to the event loop and does not wait for it.
    fn changed(&self, change: &SettingsSnapshot) {
        if let Err(e) = self.app.emit(EVT_SETTINGS_CHANGED, change) {
            tracing::warn!(
                module = "settings",
                event = EVT_SETTINGS_CHANGED,
                rev = change.rev,
                "failed to send the settings change notification: {e}"
            );
        }
    }

    fn theme_default_changed(&self) {
        self.themes.push();
    }

    // Runs on the blocking pool (the callers' contract). The tray relabel inside is posted to the main thread, not waited for.
    fn ui_language_changed(&self) {
        let (Some(hook), Some(settings), Some(layout)) = (
            self.app.try_state::<Arc<LanguageHook>>(),
            self.app.try_state::<Arc<SettingsService>>(),
            self.app.try_state::<LayoutState>(),
        ) else {
            // All three are managed in the builder (`lib.rs`) — reaching here means that registration is gone.
            tracing::error!(
                module = "settings",
                "no language hook, settings or layout state — the new UI language was not applied to the shell text"
            );
            return;
        };
        hook.apply(
            &settings,
            &layout,
            &TauriEvents { app: &self.app },
            &TauriTrayText(&self.app),
        );
    }
}
