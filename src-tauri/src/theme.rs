//! Window theme — the value ([`UiTheme`]) · the global theme ([`global_theme`] — config `theme.default`) · **the effective theme per window**
//! ([`EffectiveThemes`]) · per-window delivery ([`deliver_per_window`]) · the read·write·push handle ([`ThemeControl`]).
//!
//! - **The home of the window theme is the screen-state model** — the window entry of `ViewManager` (`WindowAttrs::theme`). It lives and
//!   dies with the window, and the recorder carries it in `state.json` (TRD S21-storage §6-3).
//! - ★The effective value is computed in only one place, [`EffectiveThemes`]★ — the boot pull (`get_ui_settings`) · the push
//!   (`ui:settings-updated`) · `window.getTheme` see the same source. If one side saw a different source, that window would get
//!   a different theme from the boot and from the push.
//! - ★Lock order = theme gate › {settings-state lock · `ViewManager` lock · out-of-model window theme table}★ — only the gate is outer, and the other
//!   three are each taken briefly one at a time and released (never taken overlapping — TRD §6-3). The side that changes the theme finishes saving, releases all locks, and then calls
//!   the push.
//! - The sending side is injected by the caller ([`ThemeWindows`]) — Tauri is not brought into this module.
// ADR-0265

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use engram_dashboard_base::sync;

use crate::layout::LayoutState;
use crate::settings::{SettingsService, THEME_DEFAULT};

/// The value used when the value of config `theme.default` does not read as a [`UiTheme`] — if the table's choices and the spelling of [`UiTheme::as_wire`] are
/// the same, it is never reached (this module's test puts the two side by side).
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

/// The config's global theme. If the config value is not a [`UiTheme`] spelling, it is [`DEFAULT_THEME`] (a spot never reached —
/// the doc of [`DEFAULT_THEME`]).
pub fn global_theme(settings: &SettingsService) -> UiTheme {
    settings
        .effective(THEME_DEFAULT)
        .as_deref()
        .and_then(UiTheme::from_wire)
        .unwrap_or(DEFAULT_THEME)
}

/// One window's theme — `own` = the value set only for that window (`None` = not set · follows the global) · `effective` = the value painted on
/// that window.
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
    #[error("window 없음: {0} — 창 label 은 window.list 가 준다")]
    UnknownWindow(String),
    /// Only the write is emitted — a read reads the single theme value even from a poisoned model (harmless even for a half-changed model).
    #[error("레이아웃 락에 독이 들어 창 테마를 못 썼다")]
    Poisoned,
    /// Written but **the targeted window itself** did not receive the notification ([`ThemeControl::set`]) — the written value stays (it is not rolled back).
    /// `reason` = that window's failure reason · `others_failed` = the number of other windows that did not receive it in the same push.
    #[error(
        "{window} 창 테마는 정했지만(window.getTheme 이 새 값을 답한다) 그 창에 못 보냈다: {reason}{}",
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
        n => format!(" — 다른 창 {n}개도 못 받았다"),
    }
}

/// The value going out to the frontend — the boot query (`get_ui_settings`) and the push (`ui:settings-updated`) use the **same shape**.
///
/// The field name is a wire contract — the frontend reads only `theme` (`src/theme/uiSettings.ts`).
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
///
/// ★[`EffectiveThemes::push_effective_themes`] calls it while holding the theme gate★ — if the implementation calls that function again, it is
/// a deadlock.
pub trait ThemeWindows: Send + Sync {
    /// All labels of the webview windows alive now.
    fn labels(&self) -> Vec<String>;
    /// Send only to that one window.
    fn send(&self, label: &str, payload: UiSettingsPayload) -> Result<(), String>;
}

/// The cap on the number of windows spelled out in the failure text — so the text · log does not grow with the number of windows when there are many.
const MAX_FAILED_DETAILS: usize = 3;

/// One window that did not receive the notification — `reason` = the reason [`ThemeWindows::send`] gave, as is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedWindow {
    pub label: String,
    pub reason: String,
}

/// The failure of [`deliver_per_window`] — 「알림이 안 닿았다」 (the notification did not arrive) (ADR-0166 decision 6).
///
/// ★It carries the windows that did not receive it as values, not as text★ — whether the window [`ThemeControl::set`] targeted is among them is decided here.
/// The human-readable text (`Display` — the first [`MAX_FAILED_DETAILS`] windows with reasons · the rest lumped into a count) is made only at the edge (one log
/// line · a command error).
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
            Undelivered::NoWindows => return f.write_str("알림을 받을 창이 하나도 없다"),
            Undelivered::Refused { tried, failed } => (tried, failed),
        };
        write!(f, "{}/{tried} 창에 못 보냈다 — ", failed.len())?;
        for (i, window) in failed.iter().take(MAX_FAILED_DETAILS).enumerate() {
            if i > 0 {
                f.write_str(" · ")?;
            }
            write!(f, "{}: {}", window.label, window.reason)?;
        }
        if failed.len() > MAX_FAILED_DETAILS {
            write!(f, " · 외 {}개", failed.len() - MAX_FAILED_DETAILS)?;
        }
        Ok(())
    }
}

/// Send **that window's value** to each window — `targets` = (window label, that window's effective theme), and the sending side is injected by the caller.
///
/// ★Do not scatter one envelope over all windows★: since the value may differ per window, it must be sent by pointing at the destination. The receiving side must also
/// subscribe by its own label — Tauri wakes **every** listener registered as `Any` **regardless of the filter**, and the default target of JS
/// `listen()` is that `Any` (`view_commands::TauriViewDispatch` carries the same pair).
///
/// `Err` is the two branches of [`Undelivered`]. ★Even if one window did not receive it, it tries all the remaining windows and only then answers with failure★ — if it stopped
/// at one dead window, the other windows would remain at the old value, and since that answer is a failure, **no one would know which windows were updated.**
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

/// The single place that computes the **effective theme = that window's theme ?? config `theme.default`** per window (TRD S21-storage §5-6) — the path that writes
/// the window theme is this one too.
///
/// ★Keep only one per shell★ — the gate lock that keeps the push order is inside it, so with two instances two pushes cannot see each other.
/// The config · model must also be the one instance per shell — if given a different model, a write and a push would see different window themes.
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

    /// Decide in advance the own theme of window `window`, which is not yet in the model — if `own` = `None`, nothing is decided (it follows the global).
    /// While the model does not know that window, the boot pull · push · window background ([`Self::effective_for`]) use this value. Once the model
    /// knows that window, the model's value wins.
    ///
    /// Used by runtime restore acceptance — it creates the popout window before putting it into the model (TRD S21-storage §6-7 ②), so without this that
    /// window would be painted with the global theme and pull the first value. When done, withdraw it with [`Self::unstage`].
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

    /// One window's theme — the answer of `window.getTheme`. It does not take the gate (it does not push, so there is nothing to reverse the order among
    /// windows).
    pub fn window_theme(&self, window: &str) -> Result<WindowTheme, ThemeError> {
        let own = self.own_theme(window)?;
        Ok(WindowTheme::resolve(own, global_theme(&self.settings)))
    }

    /// Boot pull — answers one window and pushes nothing. ★It also answers for an unknown window★ — a window that loaded its script before entering the model (a popout being
    /// created) must also receive its first value. For such a window it is the value decided by [`Self::stage`], and if none, the global value.
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

    /// Write one window's theme — `None` = erase that window's theme (it follows the global). ★It does not push★ — pushing is
    /// [`ThemeControl::set`]. `attrs_rev` goes up, not `version` (only when it changed).
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

    /// Recompute the effective value of all windows and push per window. `Err` = that of [`deliver_per_window`] — the warn log is also
    /// left here (the caller does not log it again).
    ///
    /// ★If the read and the notification are split, the old value wins★: while A reads "light" and stops before notifying, the value
    /// changes to "e-ink", and B reads it and pushes; then A wakes up and its notification sets the windows back to "light". So the read-to-notify is
    /// serialized with the gate lock — last notification = last read. The writing side calls this function **after** finishing the save, so the last push
    /// sees the last write.
    ///
    /// ★Under the gate, take the settings-state lock · window list · `ViewManager` lock · out-of-model window theme table one at a time briefly and release each, then
    /// send★ — never hold any two overlapping, and when sending hold only the gate. The caller holds no lock.
    // ADR-0265
    pub fn push_effective_themes(&self, windows: &dyn ThemeWindows) -> Result<(), Undelivered> {
        // It keeps running even if the lock is poisoned (panic while held) — all this lock guards is the order, so there is no state to break afterward.
        // If we unwrapped here, a single panic would permanently block every later push.
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
                    "창별 테마 알림을 못 보냈다: {undelivered}"
                );
            },
        )
    }

    // The read is done even from a poisoned model — it is a single theme value, so harmless even for a half-changed model (the same judgment as `LiveSource::revision` of
    //   `state::boot_plugin`).
    fn own_theme(&self, window: &str) -> Result<Option<UiTheme>, ThemeError> {
        let mgr = sync::lock(&self.layout.0);
        mgr.window_attrs(window)
            .map(|attrs| attrs.theme)
            .map_err(|_| ThemeError::UnknownWindow(window.to_string()))
    }
}

/// The handle that reads · writes · pushes the theme — it bundles the shell's one [`EffectiveThemes`] and the window-side port. The command table · settings notification ·
/// restore coordinator · boot hold the same one.
///
/// ★A lost notification does not roll back the write★ — the value is already in the model (and, in a run with saving on, in `state.json` through the recorder), and windows catch up with the next
/// pull · push. The only thing where the answer differs is the one targeted window of [`Self::set`], and other failures end in the log.
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
    ///
    /// [`ThemeError::Undelivered`] = written but **that window** did not receive it (the written value stays). If only other windows did not receive it, it is a success.
    /// It is also a success if that window is not in the live webview list ([`ThemeWindows::labels`]) — a window not yet up
    /// pulls this value when it comes up ([`EffectiveThemes::for_window`]).
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
        // The failure was logged by the push — the callers of this path (boot · restore acceptance · the `theme.default` notification) do not
        //   distinguish their answer by delivery (type doc).
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
    const CLOSED: &str = "창이 이미 닫혔다";

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
                .unwrap_or_else(|| panic!("{selector} 블록에 --bg 가 없다"))
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
            .expect("표에 있는 키");
        let choices = schema[0].choices.clone().expect("choice 키");
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

    /// The shell loads the config in the builder and opens writing only in setup (`lib.rs`) — the background · first theme boot picks when creating windows
    /// ([`EffectiveThemes::effective_for`] — `webview_env::Backdrop::for_window`) must already answer with the on-disk
    /// `theme.default` in between. If it answered with the default, the windows boot opened would be painted with a color that is not the global theme.
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
            assert_eq!(mgr.version, version, "창 테마는 레이아웃 번호를 안 올린다");
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
        rig.themes.push_effective_themes(&windows).expect("밀기");
        assert_eq!(
            windows.take(),
            sent(&[(MAIN_WINDOW_LABEL, "dark"), (UNMODELED, "light")])
        );
        assert_eq!(
            rig.themes.window_theme(UNMODELED),
            Err(ThemeError::UnknownWindow(UNMODELED.to_string())),
            "창 테마 읽기는 모델만 본다"
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
            "거둔 뒤에는 전역 값"
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

        rig.themes.push_effective_themes(&windows).expect("밀기");

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
        rig.themes.push_effective_themes(&windows).expect("밀기");
        assert_eq!(
            windows.take(),
            sent(&[
                (MAIN_WINDOW_LABEL, "e-ink"),
                (POPUP, "dark"),
                (UNMODELED, "dark")
            ])
        );

        rig.settings.set(THEME_DEFAULT, "light").unwrap();
        rig.themes.push_effective_themes(&windows).expect("밀기");

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

        rig.themes.push_effective_themes(&windows).expect("밀기");
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
        assert!(windows.take().is_empty(), "거절된 쓰기 뒤에는 밀지 않는다");
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
            .expect_err("한 창이라도 못 받으면 실패다");

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
        assert_eq!(windows.take().len(), 3, "실패한 창에서 멈췄다");
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
            "빈 명단에 성공으로 답했다"
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
            .expect_err("전부 못 받았다");

        assert_eq!(refused.failed().len(), 10);
        let message = refused.to_string();
        assert!(message.starts_with("10/10 "), "{message}");
        assert_eq!(
            message.matches(": x").count(),
            MAX_FAILED_DETAILS,
            "{message}"
        );
        assert!(message.ends_with(" · 외 7개"), "{message}");
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
        assert_eq!(windows.take().len(), 3, "다른 창에도 시도한다");
        assert_eq!(
            control.get(POPUP),
            Ok(WindowTheme {
                own: Some(UiTheme::Light),
                effective: UiTheme::Light
            }),
            "못 보냈어도 쓴 값은 남는다"
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
            .expect_err("그 창이 못 받았다")
            .to_string();

        assert!(message.starts_with(POPUP), "{message}");
        assert!(message.contains(CLOSED), "{message}");
        assert!(message.ends_with("다른 창 2개도 못 받았다"), "{message}");
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

        rig.themes.push_effective_themes(&probe).expect("밀기");

        assert_eq!(*probe.free.lock().unwrap(), [true, true]);
    }
}
