//! Window theme — the value ([`UiTheme`]) · the global theme ([`global_theme`] — config `theme.default`) · **the effective theme per window**
//! ([`EffectiveThemes`]) · per-window delivery ([`deliver_per_window`]) · the read·write·push handle ([`ThemeControl`]).
//!
//! - The window theme lives in the screen-state model (the `ViewManager` window entry, `WindowAttrs::theme`), dies with the window, and is carried in `state.json` by the recorder.
//! - ★The effective value is computed only in [`EffectiveThemes`]★: the boot pull (`get_ui_settings`), the push (`ui:settings-updated`) and `window.getTheme` must see the same source, or a window would get a different theme from boot and from push.
//! - ★Lock order = theme gate › {settings-state lock · `ViewManager` lock · out-of-model window theme table}★: only the gate is outer; the other three are each taken briefly, one at a time, never overlapping. The side that changes the theme finishes saving, releases all locks, then pushes.
//! - The sending side is injected by the caller ([`ThemeWindows`]) — Tauri is not brought into this module.
// ADR-0265 (TRD S21-storage §6-3; TRD §6-3)

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use engram_dashboard_base::sync;

use crate::layout::LayoutState;
use crate::settings::{SettingsService, THEME_DEFAULT};

/// The value used when `theme.default` does not read as a [`UiTheme`]; never reached while the table's choices match [`UiTheme::as_wire`] (this module's test checks it).
pub const DEFAULT_THEME: UiTheme = UiTheme::Dark;

/// The screen theme — the three values are all there is.
///
/// ★Do not remove `EInk`★: e-ink is not a brightness variant but a separate intent that **neutralizes color**, so it does not
/// fold into dark/light (ADR-0062).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiTheme {
    Dark,
    Light,
    EInk,
}

impl UiTheme {
    pub const ALL: [UiTheme; 3] = [UiTheme::Dark, UiTheme::Light, UiTheme::EInk];

    /// The string the frontend sets as is into `data-theme` of `document.documentElement`.
    ///
    /// ★It must have the **same spelling** as the `:root[data-theme='…']` selectors in `src/styles/theme.css`★ — if they diverge, the styles
    /// simply do not attach without a single error, and from the screen alone you cannot tell whether the spelling is wrong or the file was not read.
    pub const fn as_wire(self) -> &'static str {
        match self {
            UiTheme::Dark => "dark",
            UiTheme::Light => "light",
            UiTheme::EInk => "e-ink",
        }
    }

    /// The inverse of [`Self::as_wire`] — accepts only those three spellings (case-sensitive).
    pub fn from_wire(raw: &str) -> Option<Self> {
        match raw {
            "dark" => Some(UiTheme::Dark),
            "light" => Some(UiTheme::Light),
            "e-ink" => Some(UiTheme::EInk),
            _ => None,
        }
    }

    /// The color (RGB) the shell paints on the window · webview background — the color seen before the page is first painted (`webview_env::Backdrop`).
    ///
    /// ★It must be the same value as that theme's `--bg` in `src/styles/theme.css` (the page background — `html, body, #root` in `src/index.css`)★ —
    /// if they diverge, the background is seen to change once when the window appears. This module's test reads that file and puts them side by side.
    pub const fn page_background(self) -> (u8, u8, u8) {
        match self {
            UiTheme::Dark => (0x0a, 0x0a, 0x0a),
            UiTheme::Light => (0xf5, 0xf5, 0xf5),
            UiTheme::EInk => (0xff, 0xff, 0xff),
        }
    }
}

/// The config's global theme; [`DEFAULT_THEME`] if the value is not a [`UiTheme`] spelling.
pub fn global_theme(settings: &SettingsService) -> UiTheme {
    settings
        .effective(THEME_DEFAULT)
        .as_deref()
        .and_then(UiTheme::from_wire)
        .unwrap_or(DEFAULT_THEME)
}

/// One window's theme — `own` = set only for that window (`None` = follows the global) · `effective` = the value painted on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowTheme {
    pub own: Option<UiTheme>,
    pub effective: UiTheme,
}

impl WindowTheme {
    /// Effective theme = that window's theme ?? the global theme (TRD S21-storage §5-6).
    // ADR-0265
    pub fn resolve(own: Option<UiTheme>, global: UiTheme) -> Self {
        WindowTheme {
            own,
            effective: own.unwrap_or(global),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ThemeError {
    /// It is not in the layout model.
    #[error("window not found: {0} — window labels come from window.list")]
    UnknownWindow(String),
    /// Only the write is emitted — a read reads the single theme value even from a poisoned model (harmless even for a half-changed model).
    #[error("layout lock is poisoned; could not write the window theme")]
    Poisoned,
    /// Written but **the targeted window itself** did not receive the notification ([`ThemeControl::set`]) — the written value stays (it is not rolled back).
    /// `reason` = that window's failure reason · `others_failed` = the number of other windows that did not receive it in the same push.
    #[error(
        "{window}: the theme was set (window.getTheme answers the new value) but could not be sent to that window: {reason}{}",
        others_note(.others_failed)
    )]
    Undelivered {
        window: String,
        reason: String,
        others_failed: usize,
    },
}

fn others_note(others_failed: &usize) -> String {
    match *others_failed {
        0 => String::new(),
        n => format!(" — {n} other windows did not receive it either"),
    }
}

/// The value going out to the frontend; the boot query (`get_ui_settings`) and the push (`ui:settings-updated`) use the same shape. The field name is a wire contract (`src/theme/uiSettings.ts` reads `theme`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UiSettingsPayload {
    pub theme: String,
}

impl From<UiTheme> for UiSettingsPayload {
    fn from(theme: UiTheme) -> Self {
        UiSettingsPayload {
            theme: theme.as_wire().to_string(),
        }
    }
}

/// The window list and delivery to one window — production = the Tauri adapter (`commands::settings`), test = a fake (ADR-0012).
/// ★[`EffectiveThemes::push_effective_themes`] calls it holding the theme gate★ — an implementation that calls that function again deadlocks.
pub trait ThemeWindows: Send + Sync {
    /// All labels of the webview windows alive now.
    fn labels(&self) -> Vec<String>;
    /// Send only to that one window.
    fn send(&self, label: &str, payload: UiSettingsPayload) -> Result<(), String>;
}

/// The cap on windows spelled out in the failure text, so the text and log do not grow with the window count.
const MAX_FAILED_DETAILS: usize = 3;

/// One window that did not receive the notification — `reason` = the reason [`ThemeWindows::send`] gave, as is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedWindow {
    pub label: String,
    pub reason: String,
}

/// The failure of [`deliver_per_window`] — "the notification did not arrive" (ADR-0166 decision 6).
/// ★It carries the windows that did not receive it as values, not text★, so whether the window [`ThemeControl::set`] targeted is among them is decided here. The human-readable `Display` is made only at the edge (log line, command error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Undelivered {
    /// There is no window at all to send to.
    NoWindows,
    /// All the windows that did not receive it among the `tried` windows that were attempted — in attempt order and never empty.
    Refused {
        tried: usize,
        failed: Vec<FailedWindow>,
    },
}

impl Undelivered {
    /// All the windows that did not receive it — empty if [`Self::NoWindows`] (no window was attempted).
    pub fn failed(&self) -> &[FailedWindow] {
        match self {
            Undelivered::NoWindows => &[],
            Undelivered::Refused { failed, .. } => failed,
        }
    }
}

impl fmt::Display for Undelivered {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (tried, failed) = match self {
            Undelivered::NoWindows => {
                return f.write_str("there is no window to receive the notification")
            }
            Undelivered::Refused { tried, failed } => (tried, failed),
        };
        write!(f, "{}/{tried} windows did not receive it — ", failed.len())?;
        for (i, window) in failed.iter().take(MAX_FAILED_DETAILS).enumerate() {
            if i > 0 {
                f.write_str(" · ")?;
            }
            write!(f, "{}: {}", window.label, window.reason)?;
        }
        if failed.len() > MAX_FAILED_DETAILS {
            write!(f, " · and {} more", failed.len() - MAX_FAILED_DETAILS)?;
        }
        Ok(())
    }
}

/// Send **that window's value** to each window — `targets` = (window label, its effective theme); the sender is injected by the caller.
/// ★Do not scatter one envelope over all windows★: the value may differ per window. The receiver must also subscribe by its own label — Tauri wakes **every** listener registered as `Any` regardless of the filter, and the default target of JS `listen()` is `Any` (`view_commands::TauriViewDispatch` carries the same pair).
/// `Err` is the two branches of [`Undelivered`]. ★One failed window does not stop the rest★: stopping would leave the others at the old value, with a failure answer that cannot say which were updated.
pub fn deliver_per_window<E>(targets: &[(String, UiTheme)], mut emit: E) -> Result<(), Undelivered>
where
    E: FnMut(&str, UiSettingsPayload) -> Result<(), String>,
{
    if targets.is_empty() {
        return Err(Undelivered::NoWindows);
    }

    let failed: Vec<FailedWindow> = targets
        .iter()
        .filter_map(|(label, theme)| {
            emit(label, (*theme).into())
                .err()
                .map(|reason| FailedWindow {
                    label: label.clone(),
                    reason,
                })
        })
        .collect();
    if failed.is_empty() {
        return Ok(());
    }
    Err(Undelivered::Refused {
        tried: targets.len(),
        failed,
    })
}

/// The single place that computes the **effective theme = that window's theme ?? config `theme.default`** per window; the window theme write path is here too.
/// ★One per shell★: the gate lock that keeps the push order is inside it, so two instances could not see each other's pushes. The config and model must also be the one per-shell instance, or a write and a push would see different window themes. (TRD S21-storage §5-6)
pub struct EffectiveThemes {
    settings: Arc<SettingsService>,
    layout: LayoutState,
    /// ★Bind everything from the read to the notification into one block★ — for the reason, see [`Self::push_effective_themes`].
    gate: Mutex<()>,
    /// The own theme of a window created before it enters the model ([`Self::stage`]) — read only when the model does not know that window.
    staged: Mutex<HashMap<String, UiTheme>>,
}

impl EffectiveThemes {
    pub fn new(settings: Arc<SettingsService>, layout: LayoutState) -> Self {
        Self {
            settings,
            layout,
            gate: Mutex::new(()),
            staged: Mutex::new(HashMap::new()),
        }
    }

    /// Decide in advance the own theme of window `window`, which is not yet in the model (`own` = `None` decides nothing; it follows the global). Until the model knows that window, the boot pull, push and window background ([`Self::effective_for`]) use this value; then the model's wins.
    /// For runtime restore acceptance, which creates the popout before putting it into the model — without this the window would paint the global theme and pull the first value. Withdraw it with [`Self::unstage`]. (TRD S21-storage §6-7 ②)
    pub fn stage(&self, window: &str, own: Option<UiTheme>) {
        let mut staged = sync::lock(&self.staged);
        match own {
            Some(theme) => staged.insert(window.to_string(), theme),
            None => staged.remove(window),
        };
    }

    /// Withdraw [`Self::stage`] — for a window that was never staged, nothing happens.
    pub fn unstage(&self, window: &str) {
        sync::lock(&self.staged).remove(window);
    }

    /// One window's theme — the answer of `window.getTheme`. Takes no gate (it does not push).
    pub fn window_theme(&self, window: &str) -> Result<WindowTheme, ThemeError> {
        let own = self.own_theme(window)?;
        Ok(WindowTheme::resolve(own, global_theme(&self.settings)))
    }

    /// Boot pull — answers one window and pushes nothing. ★It also answers for an unknown window★: a window that loaded its script before entering the model (a popout being created) still needs its first value — the [`Self::stage`] value, else the global.
    pub fn for_window(&self, window: &str) -> UiSettingsPayload {
        self.effective_for(window).into()
    }

    /// The same answer as [`Self::for_window`], as a theme value — used by the side that picks the background color · first theme when creating a window.
    pub fn effective_for(&self, window: &str) -> UiTheme {
        let own = match self.own_theme(window) {
            Ok(own) => own,
            Err(_) => sync::lock(&self.staged).get(window).copied(),
        };
        WindowTheme::resolve(own, global_theme(&self.settings)).effective
    }

    /// Write one window's theme (`None` = erase; it follows the global). ★It does not push★ — that is [`ThemeControl::set`]. `attrs_rev` goes up, not `version`, and only when changed.
    pub fn set_window_theme(
        &self,
        window: &str,
        theme: Option<UiTheme>,
    ) -> Result<WindowTheme, ThemeError> {
        self.layout
            .0
            .lock()
            .map_err(|_| ThemeError::Poisoned)?
            .set_window_theme(window, theme)
            .map_err(|_| ThemeError::UnknownWindow(window.to_string()))?;
        Ok(WindowTheme::resolve(theme, global_theme(&self.settings)))
    }

    /// Recompute the effective value of all windows and push per window. `Err` = that of [`deliver_per_window`]; the warn log is also left here (the caller does not log it again).
    /// ★If the read and the notification are split, the old value wins★: A reads "light" and stalls before notifying, the value changes to "e-ink", B reads and pushes it, then A's notification sets the windows back to "light". The gate serializes read-to-notify, so last notification = last read. The writing side calls this **after** finishing the save.
    /// ★Under the gate, take the settings-state lock · window list · `ViewManager` lock · out-of-model theme table one at a time and release each, then send★ — never two overlapping, and only the gate is held while sending. The caller holds no lock.
    // ADR-0265
    pub fn push_effective_themes(&self, windows: &dyn ThemeWindows) -> Result<(), Undelivered> {
        // Keep running through a poisoned lock: it guards only the order, so there is no state to break, and unwrapping would block every later push forever.
        let _order = sync::lock(&self.gate);
        let global = global_theme(&self.settings);
        let labels = windows.labels();
        let model: HashMap<&str, Option<UiTheme>> = {
            let mgr = sync::lock(&self.layout.0);
            labels
                .iter()
                .filter_map(|label| {
                    let attrs = mgr.window_attrs(label).ok()?;
                    Some((label.as_str(), attrs.theme))
                })
                .collect()
        };
        let staged = sync::lock(&self.staged).clone();
        let targets: Vec<(String, UiTheme)> = labels
            .iter()
            .map(|label| {
                let own = match model.get(label.as_str()) {
                    Some(own) => *own,
                    None => staged.get(label).copied(),
                };
                (label.clone(), WindowTheme::resolve(own, global).effective)
            })
            .collect();
        deliver_per_window(&targets, |label, payload| windows.send(label, payload)).inspect_err(
            |undelivered| {
                tracing::warn!(
                    module = "theme",
                    "could not send the per-window theme notification: {undelivered}"
                );
            },
        )
    }

    // A poisoned model is still read: it is a single theme value (same judgment as `LiveSource::revision` of `state::boot_plugin`).
    fn own_theme(&self, window: &str) -> Result<Option<UiTheme>, ThemeError> {
        let mgr = sync::lock(&self.layout.0);
        mgr.window_attrs(window)
            .map(|attrs| attrs.theme)
            .map_err(|_| ThemeError::UnknownWindow(window.to_string()))
    }
}

/// The handle that reads, writes and pushes the theme — bundles the shell's one [`EffectiveThemes`] and the window-side port; the command table, settings notification, restore coordinator and boot hold the same one.
/// ★A lost notification does not roll back the write★: the value is already in the model (and in `state.json` when saving is on), and windows catch up on the next pull or push. Only the one window targeted by [`Self::set`] gets a different answer; other failures end in the log.
#[derive(Clone)]
pub struct ThemeControl {
    themes: Arc<EffectiveThemes>,
    windows: Arc<dyn ThemeWindows>,
}

impl ThemeControl {
    pub fn new(themes: Arc<EffectiveThemes>, windows: Arc<dyn ThemeWindows>) -> Self {
        Self { themes, windows }
    }

    pub fn get(&self, window: &str) -> Result<WindowTheme, ThemeError> {
        self.themes.window_theme(window)
    }

    /// Write that window's theme and push all windows again. ★Call it holding no lock★.
    /// [`ThemeError::Undelivered`] = written but **that window** did not receive it (the value stays). Other windows failing is a success, as is that window not being in the live list ([`ThemeWindows::labels`]) — it pulls the value when it comes up ([`EffectiveThemes::for_window`]).
    pub fn set(&self, window: &str, theme: Option<UiTheme>) -> Result<WindowTheme, ThemeError> {
        let written = self.themes.set_window_theme(window, theme)?;
        if let Err(undelivered) = self.themes.push_effective_themes(self.windows.as_ref()) {
            let failed = undelivered.failed();
            // ADR-0166 decision 6: if the notification could not go out, do not answer with success — the window this answer speaks of is the one targeted.
            if let Some(own) = failed.iter().find(|missed| missed.label == window) {
                return Err(ThemeError::Undelivered {
                    window: window.to_string(),
                    reason: own.reason.clone(),
                    others_failed: failed.len() - 1,
                });
            }
        }
        Ok(written)
    }

    pub fn stage(&self, window: &str, own: Option<UiTheme>) {
        self.themes.stage(window, own);
    }

    pub fn unstage(&self, window: &str) {
        self.themes.unstage(window);
    }

    /// Push the effective theme to all windows. ★Call it holding no lock★.
    pub fn push(&self) {
        // The push already logged the failure; boot, restore acceptance and the `theme.default` notification do not distinguish their answer by delivery.
        let _ = self.themes.push_effective_themes(self.windows.as_ref());
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::layout::MAIN_WINDOW_LABEL;

    const POPUP: &str = "slot-popup-1";
    /// A window whose webview is alive but is not yet in the model (a popout being created).
    const UNMODELED: &str = "slot-popup-2";
    /// The reason [`Recording`] gives for a window that does not receive.
    const CLOSED: &str = "the window is already closed";

    /// The config folder for one test — opens writing like shell `setup`. Cleaned up when done.
    struct Config(PathBuf);

    impl Config {
        fn new(tag: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            Config(std::env::temp_dir().join(format!(
                "engram-theme-{tag}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            )))
        }

        fn service(&self) -> Arc<SettingsService> {
            let service = SettingsService::load_from_dir(&self.0);
            service.enable_writes();
            Arc::new(service)
        }
    }

    impl Drop for Config {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    /// Fix the window list in advance and leave what was sent in order. `refuse` = windows that do not receive.
    #[derive(Default)]
    struct Recording {
        labels: Vec<String>,
        refuse: Vec<String>,
        sent: Mutex<Vec<(String, String)>>,
    }

    impl Recording {
        fn new(labels: &[&str]) -> Self {
            Recording {
                labels: labels.iter().map(|label| label.to_string()).collect(),
                ..Self::default()
            }
        }

        fn refusing(labels: &[&str], refuse: &[&str]) -> Self {
            Recording {
                refuse: refuse.iter().map(|label| label.to_string()).collect(),
                ..Self::new(labels)
            }
        }

        fn take(&self) -> Vec<(String, String)> {
            std::mem::take(&mut *self.sent.lock().unwrap())
        }
    }

    impl ThemeWindows for Recording {
        fn labels(&self) -> Vec<String> {
            self.labels.clone()
        }

        fn send(&self, label: &str, payload: UiSettingsPayload) -> Result<(), String> {
            self.sent
                .lock()
                .unwrap()
                .push((label.to_string(), payload.theme));
            if self.refuse.iter().any(|refused| refused == label) {
                return Err(CLOSED.to_string());
            }
            Ok(())
        }
    }

    fn sent(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(label, theme)| (label.to_string(), theme.to_string()))
            .collect()
    }

    struct Rig {
        _config: Config,
        settings: Arc<SettingsService>,
        layout: LayoutState,
        themes: EffectiveThemes,
    }

    /// main + one popout are in the model.
    fn rig(tag: &str) -> Rig {
        let config = Config::new(tag);
        let settings = config.service();
        let layout = LayoutState::new();
        layout.0.lock().unwrap().create_window(POPUP).unwrap();
        let themes = EffectiveThemes::new(settings.clone(), layout.clone());
        Rig {
            _config: config,
            settings,
            layout,
            themes,
        }
    }

    /// All three values must stay alive — if e-ink is folded into dark/light, that theme's intent (neutralizing color) disappears (ADR-0062).
    #[test]
    fn every_theme_name_round_trips() {
        for (raw, expected) in [
            ("dark", UiTheme::Dark),
            ("light", UiTheme::Light),
            ("e-ink", UiTheme::EInk),
        ] {
            assert_eq!(UiTheme::from_wire(raw), Some(expected));
            // The spelling the frontend sets in `data-theme` = the selectors of `src/styles/theme.css`.
            assert_eq!(expected.as_wire(), raw);
        }
    }

    #[test]
    fn the_page_background_is_the_themes_bg_token_in_theme_css() {
        let css = include_str!("../../src/styles/theme.css");
        for theme in [UiTheme::Dark, UiTheme::Light, UiTheme::EInk] {
            let selector = format!(":root[data-theme='{}']", theme.as_wire());
            let token = css
                .lines()
                .filter(|line| line.starts_with(&selector))
                .find_map(|line| line.split("--bg:").nth(1))
                .and_then(|rest| rest.split(';').next())
                .unwrap_or_else(|| panic!("no --bg in the {selector} block"))
                .trim();
            let (r, g, b) = theme.page_background();
            assert_eq!(
                token.to_ascii_lowercase(),
                format!("#{r:02x}{g:02x}{b:02x}"),
                "{selector}"
            );
        }
    }

    /// ★The choices of the settings table and the spelling of this enum are the same★ — if they diverge, that value of `theme.default` is silently folded into
    /// [`DEFAULT_THEME`] on screen (the settings answer succeeds but the screen does not change).
    #[test]
    fn every_theme_default_choice_is_a_ui_theme_spelling() {
        let config = Config::new("choices");
        let schema = config
            .service()
            .schema(Some(THEME_DEFAULT))
            .expect("a key in the table");
        let choices = schema[0].choices.clone().expect("a choice key");
        assert_eq!(choices.len(), 3);
        for choice in &choices {
            assert!(UiTheme::from_wire(choice).is_some(), "{choice}");
        }
        assert_eq!(schema[0].default, DEFAULT_THEME.as_wire());
    }

    #[test]
    fn the_effective_theme_is_the_windows_own_else_the_global() {
        assert_eq!(
            WindowTheme::resolve(Some(UiTheme::EInk), UiTheme::Light).effective,
            UiTheme::EInk
        );
        assert_eq!(
            WindowTheme::resolve(None, UiTheme::Light),
            WindowTheme {
                own: None,
                effective: UiTheme::Light
            }
        );
    }

    #[test]
    fn a_window_without_its_own_theme_follows_theme_default() {
        let rig = rig("follow");
        assert_eq!(
            rig.themes.window_theme(MAIN_WINDOW_LABEL),
            Ok(WindowTheme {
                own: None,
                effective: DEFAULT_THEME
            })
        );
        rig.settings.set(THEME_DEFAULT, "light").unwrap();
        assert_eq!(
            rig.themes
                .window_theme(MAIN_WINDOW_LABEL)
                .unwrap()
                .effective,
            UiTheme::Light
        );
    }

    /// The shell loads the config in the builder and opens writing only in setup (`lib.rs`); in between, the background and first theme chosen at window creation ([`EffectiveThemes::effective_for`]) must already answer with the on-disk `theme.default`, or boot windows would paint a color that is not the global theme.
    #[test]
    fn a_persisted_theme_default_is_answered_right_after_load_before_writes_open() {
        let config = Config::new("persisted");
        config.service().set(THEME_DEFAULT, "light").unwrap();
        let settings = Arc::new(SettingsService::load_from_dir(&config.0));
        let layout = LayoutState::new();
        layout.0.lock().unwrap().create_window(POPUP).unwrap();
        let themes = EffectiveThemes::new(settings, layout);

        for window in [MAIN_WINDOW_LABEL, POPUP, UNMODELED] {
            assert_eq!(themes.effective_for(window), UiTheme::Light, "{window}");
        }
    }

    #[test]
    fn clearing_a_windows_theme_falls_back_to_the_global() {
        let rig = rig("clear");
        rig.settings.set(THEME_DEFAULT, "light").unwrap();
        let set = rig
            .themes
            .set_window_theme(POPUP, Some(UiTheme::EInk))
            .unwrap();
        assert_eq!(set.effective, UiTheme::EInk);

        let cleared = rig.themes.set_window_theme(POPUP, None).unwrap();

        assert_eq!(
            cleared,
            WindowTheme {
                own: None,
                effective: UiTheme::Light
            }
        );
        assert_eq!(rig.themes.window_theme(POPUP), Ok(cleared));
    }

    /// The window theme is held in the window entry of `ViewManager` and only the window-attribute revision goes up (the layout `version` stays).
    #[test]
    fn a_window_theme_writes_the_model_and_bumps_only_the_attrs_revision() {
        let rig = rig("route");
        let (version, attrs_rev) = {
            let mgr = rig.layout.0.lock().unwrap();
            (mgr.version, mgr.attrs_rev())
        };

        rig.themes
            .set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::EInk))
            .unwrap();
        {
            let mgr = rig.layout.0.lock().unwrap();
            assert_eq!(
                mgr.window_attrs(MAIN_WINDOW_LABEL).unwrap().theme,
                Some(UiTheme::EInk)
            );
            assert_eq!(mgr.attrs_rev(), attrs_rev + 1);
            assert_eq!(
                mgr.version, version,
                "a window theme does not bump the layout version"
            );
        }
    }

    #[test]
    fn an_unknown_window_is_refused_on_read_and_write_but_still_answers_the_boot_pull() {
        let rig = rig("unknown");
        let ghost = "slot-popup-9";
        let attrs_rev = rig.layout.0.lock().unwrap().attrs_rev();

        assert_eq!(
            rig.themes.window_theme(ghost),
            Err(ThemeError::UnknownWindow(ghost.to_string()))
        );
        assert_eq!(
            rig.themes.set_window_theme(ghost, Some(UiTheme::Light)),
            Err(ThemeError::UnknownWindow(ghost.to_string()))
        );
        assert_eq!(rig.layout.0.lock().unwrap().attrs_rev(), attrs_rev);
        assert_eq!(rig.themes.for_window(ghost).theme, DEFAULT_THEME.as_wire());
    }

    /// A window created before it enters the model (the popout of runtime restore acceptance) pulls · is pushed · is painted with its own theme decided in advance —
    /// if drawn once with the global value, a color that is not that window's theme would be seen.
    #[test]
    fn a_staged_window_answers_its_own_theme_until_the_model_knows_it() {
        let rig = rig("staged");
        rig.themes.stage(UNMODELED, Some(UiTheme::Light));

        assert_eq!(rig.themes.effective_for(UNMODELED), UiTheme::Light);
        assert_eq!(rig.themes.for_window(UNMODELED).theme, "light");
        let windows = Recording::new(&[MAIN_WINDOW_LABEL, UNMODELED]);
        rig.themes.push_effective_themes(&windows).expect("push");
        assert_eq!(
            windows.take(),
            sent(&[(MAIN_WINDOW_LABEL, "dark"), (UNMODELED, "light")])
        );
        assert_eq!(
            rig.themes.window_theme(UNMODELED),
            Err(ThemeError::UnknownWindow(UNMODELED.to_string())),
            "reading a window theme looks only at the model"
        );

        // If the model knows that window, the model's value wins.
        rig.layout
            .0
            .lock()
            .unwrap()
            .create_window(UNMODELED)
            .unwrap();
        assert_eq!(rig.themes.effective_for(UNMODELED), DEFAULT_THEME);

        rig.themes.unstage(UNMODELED);
        rig.layout
            .0
            .lock()
            .unwrap()
            .close_window(UNMODELED)
            .unwrap();
        assert_eq!(
            rig.themes.effective_for(UNMODELED),
            DEFAULT_THEME,
            "after withdrawal, the global value"
        );
    }

    #[test]
    fn staging_no_theme_leaves_the_window_on_the_global() {
        let rig = rig("staged-none");
        rig.settings.set(THEME_DEFAULT, "e-ink").unwrap();
        rig.themes.stage(UNMODELED, Some(UiTheme::Light));
        rig.themes.stage(UNMODELED, None);
        assert_eq!(rig.themes.effective_for(UNMODELED), UiTheme::EInk);
    }

    #[test]
    fn a_push_sends_each_live_window_its_own_effective_theme() {
        let rig = rig("push");
        rig.settings.set(THEME_DEFAULT, "light").unwrap();
        rig.themes
            .set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::EInk))
            .unwrap();
        rig.themes
            .set_window_theme(POPUP, Some(UiTheme::Dark))
            .unwrap();
        // A window not in the model receives the global value.
        let windows = Recording::new(&[MAIN_WINDOW_LABEL, POPUP, UNMODELED]);

        rig.themes.push_effective_themes(&windows).expect("push");

        assert_eq!(
            windows.take(),
            sent(&[
                (MAIN_WINDOW_LABEL, "e-ink"),
                (POPUP, "dark"),
                (UNMODELED, "light"),
            ])
        );
    }

    /// ★If `theme.default` is changed and pushed again, only windows with no own theme actually change★ — the push reads the config value at that time
    /// (it does not cache).
    #[test]
    fn a_theme_default_change_moves_only_the_windows_without_their_own_theme() {
        let rig = rig("default");
        rig.themes
            .set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::EInk))
            .unwrap();
        let windows = Recording::new(&[MAIN_WINDOW_LABEL, POPUP, UNMODELED]);
        rig.themes.push_effective_themes(&windows).expect("push");
        assert_eq!(
            windows.take(),
            sent(&[
                (MAIN_WINDOW_LABEL, "e-ink"),
                (POPUP, "dark"),
                (UNMODELED, "dark")
            ])
        );

        rig.settings.set(THEME_DEFAULT, "light").unwrap();
        rig.themes.push_effective_themes(&windows).expect("push");

        assert_eq!(
            windows.take(),
            sent(&[
                (MAIN_WINDOW_LABEL, "e-ink"),
                (POPUP, "light"),
                (UNMODELED, "light")
            ])
        );
    }

    /// The boot pull and the push give the same value per window — if only one side saw a different source,
    /// that window would get a different theme from the boot and from the push.
    #[test]
    fn a_boot_pull_and_a_push_agree_on_each_window() {
        let rig = rig("agree");
        rig.settings.set(THEME_DEFAULT, "e-ink").unwrap();
        rig.themes
            .set_window_theme(POPUP, Some(UiTheme::Light))
            .unwrap();
        let windows = Recording::new(&[MAIN_WINDOW_LABEL, POPUP, UNMODELED]);

        rig.themes.push_effective_themes(&windows).expect("push");
        for (label, theme) in windows.take() {
            assert_eq!(rig.themes.for_window(&label).theme, theme, "{label}");
        }
    }

    #[test]
    fn the_control_pushes_after_a_write_and_not_after_a_refused_one() {
        let rig = rig("control");
        let windows = Arc::new(Recording::new(&[MAIN_WINDOW_LABEL, POPUP]));
        let control = ThemeControl::new(Arc::new(rig.themes), windows.clone());

        let written = control.set(POPUP, Some(UiTheme::Light)).unwrap();

        assert_eq!(written.own, Some(UiTheme::Light));
        assert_eq!(
            windows.take(),
            sent(&[(MAIN_WINDOW_LABEL, "dark"), (POPUP, "light")])
        );
        assert!(control.set("slot-popup-9", Some(UiTheme::Light)).is_err());
        assert!(windows.take().is_empty(), "no push after a refused write");
        assert_eq!(control.get(POPUP).unwrap(), written);
    }

    /// ★If even one window could not be sent to, the push is not a success★ — the command's answer is decided by `ThemeControl::set` with this value (ADR-0166 decision 6 amendment). And it does not stop there either.
    #[test]
    fn a_window_that_did_not_receive_it_fails_the_push_after_every_window_was_tried() {
        let rig = rig("refuse");
        let windows = Recording::refusing(&[MAIN_WINDOW_LABEL, POPUP, UNMODELED], &[POPUP]);

        let refused = rig
            .themes
            .push_effective_themes(&windows)
            .expect_err("it fails if even one window cannot receive");

        assert_eq!(
            refused,
            Undelivered::Refused {
                tried: 3,
                failed: vec![FailedWindow {
                    label: POPUP.to_string(),
                    reason: CLOSED.to_string(),
                }],
            }
        );
        assert_eq!(windows.take().len(), 3, "stopped at the failing window");
    }

    #[test]
    fn a_push_with_no_live_windows_is_a_failure() {
        let mut calls = 0usize;
        let outcome = deliver_per_window(&[], |_label, _payload| {
            calls += 1;
            Ok(())
        });

        assert_eq!(
            outcome,
            Err(Undelivered::NoWindows),
            "answered success for an empty list"
        );
        assert_eq!(calls, 0);
    }

    /// The value carries all windows that did not receive it, and the text spells out reasons for only the first [`MAX_FAILED_DETAILS`] windows and lumps the rest into a count.
    #[test]
    fn failed_windows_are_counted_in_full_but_described_in_bounded_numbers() {
        let targets: Vec<(String, UiTheme)> = (0..10)
            .map(|n| (format!("slot-popup-{n}"), UiTheme::Dark))
            .collect();

        let refused = deliver_per_window(&targets, |_label, _payload| Err("x".to_string()))
            .expect_err("none received it");

        assert_eq!(refused.failed().len(), 10);
        let message = refused.to_string();
        assert!(message.starts_with("10/10 "), "{message}");
        assert_eq!(
            message.matches(": x").count(),
            MAX_FAILED_DETAILS,
            "{message}"
        );
        assert!(message.ends_with(" · and 7 more"), "{message}");
    }

    /// ★If that window did not receive it, it is not a success★ (ADR-0166 decision 6) — still the written value stays, and it tries even the other windows.
    #[test]
    fn a_write_the_target_window_did_not_receive_is_an_error_but_stays_written() {
        let rig = rig("target");
        let windows = Arc::new(Recording::refusing(
            &[MAIN_WINDOW_LABEL, POPUP, UNMODELED],
            &[POPUP],
        ));
        let control = ThemeControl::new(Arc::new(rig.themes), windows.clone());

        let refused = control.set(POPUP, Some(UiTheme::Light));

        assert_eq!(
            refused,
            Err(ThemeError::Undelivered {
                window: POPUP.to_string(),
                reason: CLOSED.to_string(),
                others_failed: 0,
            })
        );
        assert_eq!(windows.take().len(), 3, "tries the other windows too");
        assert_eq!(
            control.get(POPUP),
            Ok(WindowTheme {
                own: Some(UiTheme::Light),
                effective: UiTheme::Light
            }),
            "the written value stays even when sending failed"
        );
    }

    /// If only other windows did not receive it, that write is a success — the push leaves that failure in the log.
    #[test]
    fn a_write_only_other_windows_did_not_receive_is_a_success() {
        let rig = rig("others");
        let windows = Arc::new(Recording::refusing(
            &[MAIN_WINDOW_LABEL, POPUP, UNMODELED],
            &[MAIN_WINDOW_LABEL, UNMODELED],
        ));
        let control = ThemeControl::new(Arc::new(rig.themes), windows.clone());

        let written = control.set(POPUP, Some(UiTheme::EInk));

        assert_eq!(
            written,
            Ok(WindowTheme {
                own: Some(UiTheme::EInk),
                effective: UiTheme::EInk
            })
        );
        assert_eq!(windows.take().len(), 3);
    }

    /// That window's reason is spelled out and the failures of other windows are attached only as a count.
    #[test]
    fn an_undelivered_write_names_its_window_and_counts_the_others() {
        let rig = rig("message");
        let windows = Arc::new(Recording::refusing(
            &[MAIN_WINDOW_LABEL, POPUP, UNMODELED],
            &[MAIN_WINDOW_LABEL, POPUP, UNMODELED],
        ));
        let control = ThemeControl::new(Arc::new(rig.themes), windows);

        let message = control
            .set(POPUP, Some(UiTheme::Light))
            .expect_err("that window did not receive it")
            .to_string();

        assert!(message.starts_with(POPUP), "{message}");
        assert!(message.contains(CLOSED), "{message}");
        assert!(
            message.ends_with("2 other windows did not receive it either"),
            "{message}"
        );
    }

    /// When sending, hold only the gate — if sending while holding the model lock, it is a deadlock the moment the receiving side reads the layout.
    #[test]
    fn a_push_sends_with_the_model_lock_free() {
        struct Probe {
            layout: LayoutState,
            free: Mutex<Vec<bool>>,
        }

        impl ThemeWindows for Probe {
            fn labels(&self) -> Vec<String> {
                vec![MAIN_WINDOW_LABEL.to_string(), POPUP.to_string()]
            }

            fn send(&self, _label: &str, _payload: UiSettingsPayload) -> Result<(), String> {
                let free = self.layout.0.try_lock().is_ok();
                self.free.lock().unwrap().push(free);
                Ok(())
            }
        }

        let rig = rig("locks");
        let probe = Probe {
            layout: rig.layout.clone(),
            free: Mutex::default(),
        };

        rig.themes.push_effective_themes(&probe).expect("push");

        assert_eq!(*probe.free.lock().unwrap(), [true, true]);
    }
}
