//! Webview environment — the data folder and browser arguments ([`WebviewEnv`]) the WebView windows of this shell process use (TRD S21-storage §4).
//! - Windows with the same data folder must have the same environment options — a violating window gets `Ok` from `build()` but no OS window appears (the ghost window of ADR-0054). So all window-creating code passes through [`WebviewEnv::finish`], whose value comes from the one-per-shell instance. The config-declared window (main) is no exception: Tauri is left not to create it (`"create": false` in `tauri.conf.json`) and boot step 8 creates it with `from_config` and hands it to this finishing step (`state::placement`).
//! - The finishing step decides the four environment options (folder · browser arguments · extensions · scrollbar style) — the four wry 0.55.1 `webview2` `create_environment` puts into the environment. Three are always pinned to the same values; only the folder cannot be cleared by the builder, so on fallback to the default folder the config window declaration's `dataDirectory` would remain — hence this module's test looks at the config.
//! - The folder is decided once per process: `<data root>\webview` if writable, else no folder is passed and Tauri's default (`LocalData/<identifier>`) is used with a warn (not a stop — TRD §6 D8). It is not asked per window — a changed answer between windows would give one process different folders. When the whole data root is unusable the reason shows inside the window after it comes up, as a daemon connection failure (ADR-0134 decision 4 · ADR-0135).
//! - The checked folder and the passed folder are the same absolute path: the data root may be relative (`ENGRAM_DATA_DIR` is used as is), and what WebView2 resolves a relative path against was not checked — a relative path could diverge the checked and used folders.
//! - The window background, first theme and first UI language ([`Backdrop`]) are attached by the finishing step too — not an environment option but a per-window value (that window's effective theme); since all window-creating code passes here, it is taken as an argument and attached without omission.
//! - The OS frame (title bar · buttons) is lifted by the finishing step too: on an OS where the app draws the title bar · window buttons (platform `drawn_buttons` of `window::caption_style`) it is `decorations(false)` · `shadow(true)` (TRD S26-window-chrome §8-1) — in one place, not per window, and not in the window declaration of `tauri.conf.json` (this module's test looks at the config).
// ADR-0054

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engram_dashboard_platform::window::{self as os, CaptionStyle};
use tauri::webview::ScrollBarStyle;
use tauri::window::Color;
use tauri::{Manager, Runtime, WebviewWindow, WebviewWindowBuilder};

use crate::discovery::DataLayout;
use crate::theme::{EffectiveThemes, UiTheme, DEFAULT_THEME};
use crate::ui_text::Lang;

/// The WebView2 browser arguments — the only source. The config window receives this value too, so it is not written in `tauri.conf.json` (if written, this module's test catches it; config JSON cannot carry comments, so the reason is here).
/// Do not change it to the wry default (which even turns off `msSmartScreenProtection`): the invariant is not "the standard value" but "the same among themselves within the same folder" (the alternative ADR-0054 rejected).
pub(crate) const WEBVIEW2_BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI --autoplay-policy=no-user-gesture-required";

type WriteCheck = fn(&Path) -> Result<(), String>;

/// The window · webview background and the page's first theme painted when creating a window — what is seen before the shell's decided theme arrives over IPC. The window background is a color erased when the window is shown (`crate::first_paint` — shown after first paint, but possibly earlier by the time limit), and if the webview background is not painted WebView2's default white shows through before the page paints. Without the embedded first theme, the page draws once with the dark default (`src/theme/bootTheme.ts`).
/// When newly building a transparent window, revive the branch that does not paint: painted, tauri-runtime-wry 2.11.3 fills the bottom of a transparent window with that color (`draw_surface` in `window/windows.rs` — it discards alpha) and the window becomes opaque (source reading).
pub(crate) enum Backdrop {
    /// Paint that theme's page background color ([`UiTheme::page_background`]) and embed `data-theme` with an initialization script running before the page script — using the theme window `label` last painted in this webview ([`THEME_MEMORY_PREFIX`]) if present, else `theme`. Likewise `<html lang>` — the language that window last applied ([`LANG_MEMORY_PREFIX`]) if `en` or `ko`, else `lang`.
    Theme {
        label: String,
        theme: UiTheme,
        lang: Lang,
    },
}

impl Backdrop {
    /// Paint with that window's effective theme — for a window not yet in the model the window theme decided in advance (`EffectiveThemes::stage`), else the global theme; with no theme state the default theme (the shell registers it in the builder, so never reached). The language is the effective `ui.language` now.
    pub(crate) fn for_window<R: Runtime, M: Manager<R>>(manager: &M, label: &str) -> Self {
        let theme = manager
            .try_state::<Arc<EffectiveThemes>>()
            .map_or(DEFAULT_THEME, |themes| themes.effective_for(label));
        Backdrop::Theme {
            label: label.to_string(),
            theme,
            lang: crate::ui_language::current(manager),
        }
    }
}

/// Head of the key under which the theme a window last painted is written into that webview's `sessionStorage` (window label appended). The frontend writes it whenever it attaches a theme (`src/theme/themeMemory.ts`) and the initialization script reads it first on a reload (F5 · HMR full reload) — that script is frozen to the theme at window creation, so reloading a window whose theme changed afterward would draw once with the old theme.
/// Must have the same spelling as `THEME_KEY_PREFIX` of `src/theme/themeMemory.ts` — if they diverge memory is silently not found (this module's test reads that file and compares). Do not change it to `localStorage`: it persists across runs, but popout labels are new each run (run marker · in-run sequence number not reused — `commands::popout::popup_label`), so one key per popout would pile up forever; the ≈2⁻³² chance of the next run reusing a label is no reason to inherit someone else's theme. `sessionStorage` is separate per top-level browsing context by the HTML standard and survives reloads — [unverified — GUI] whether a new WebView2 window starts with empty storage (even if not, labels are not reused within a run, so a new window does not read someone else's key).
pub(crate) const THEME_MEMORY_PREFIX: &str = "engram.theme.";

/// Head of the key under which the UI language a window last applied is written into that webview's `sessionStorage` (window label appended). The frontend writes it after applying a language (`src/i18n/langMemory.ts`) and the initialization script reads it first, so a reload does not paint once in the creation-time language. Same storage choice and reasons as [`THEME_MEMORY_PREFIX`]; must have the same spelling as `LANG_KEY_PREFIX` of `src/i18n/langMemory.ts` (this module's test compares them).
// ADR-0373
pub(crate) const LANG_MEMORY_PREFIX: &str = "engram.lang.";

/// Repaint the background of a window that is up with that theme — called per window when pushing the theme (`commands::settings`). Painting on the tao 0.35.3 side only writes the color and redraws (`InvalidateRect` + `UpdateWindow`) without touching window flags, so calling it on a window waiting hidden (a spare window — `commands::spare`) does not make it visible (source reading).
pub(crate) fn repaint<R: Runtime>(window: &WebviewWindow<R>, theme: UiTheme) {
    // The background is a color seen only before the page covers it, so it is no reason to make delivery (the theme notification) a failure — it is only left in the log.
    if let Err(e) = window.set_background_color(Some(color(theme))) {
        tracing::debug!(
            module = "webview_env",
            label = window.label(),
            "could not repaint the window background: {e}"
        );
    }
}

fn color(theme: UiTheme) -> Color {
    let (r, g, b) = theme.page_background();
    Color(r, g, b, 255)
}

// The initialization script runs again on every reload — if the remembered theme (`THEME_MEMORY_PREFIX`) is one of the three spellings, that value; otherwise (first load · storage unreadable) the creation-time theme. The key is a JSON string so the script survives quotes · backslashes in the label (shell-made, but still). Whether `document.documentElement` already exists when the document is born was not checked in WebView2 — if not, it embeds when the root element attaches (observer callback = microtask, earlier than page scripts · first paint).
fn theme_script(label: &str, theme: UiTheme) -> String {
    let key = serde_json::Value::from(format!("{THEME_MEMORY_PREFIX}{label}"));
    let names = serde_json::Value::from(UiTheme::ALL.map(UiTheme::as_wire).to_vec());
    format!(
        "(function(){{var t='{theme}';try{{var s=sessionStorage.getItem({key});if({names}.indexOf(s)>=0)t=s;}}catch(_){{}}\
         var r=document.documentElement;if(r){{r.dataset.theme=t;return;}}\
         new MutationObserver(function(_,o){{var e=document.documentElement;if(e){{e.dataset.theme=t;o.disconnect();}}}})\
         .observe(document,{{childList:true}});}})();",
        theme = theme.as_wire(),
    )
}

// The `<html lang>` counterpart of `theme_script` — same shape, same reasons; only `en` · `ko` are taken from memory.
// ADR-0373
fn lang_script(label: &str, lang: Lang) -> String {
    let key = serde_json::Value::from(format!("{LANG_MEMORY_PREFIX}{label}"));
    let codes = serde_json::Value::from(Lang::ALL.map(Lang::code).to_vec());
    format!(
        "(function(){{var l='{lang}';try{{var s=sessionStorage.getItem({key});if({codes}.indexOf(s)>=0)l=s;}}catch(_){{}}\
         var r=document.documentElement;if(r){{r.lang=l;return;}}\
         new MutationObserver(function(_,o){{var e=document.documentElement;if(e){{e.lang=l;o.disconnect();}}}})\
         .observe(document,{{childList:true}});}})();",
        lang = lang.code(),
    )
}

/// The webview environment of this process — one per shell (shared with `Arc` · managed in the builder). Do not attach `Clone`: split decision fields would each decide separately and windows could receive different folders.
pub(crate) struct WebviewEnv {
    dir: PathBuf,
    check: WriteCheck,
    chosen: OnceLock<Option<PathBuf>>,
}

impl WebviewEnv {
    pub(crate) fn resolve() -> Self {
        Self::new(DataLayout::resolve().webview_dir(), fs_check)
    }

    fn new(dir: PathBuf, check: WriteCheck) -> Self {
        Self {
            dir,
            check,
            chosen: OnceLock::new(),
        }
    }

    /// The data folder to pass to windows (absolute path) — `None` = do not pass (Tauri's default folder). The first call does a write check once; after that it returns the same answer (even concurrently, the check happens once).
    pub(crate) fn chosen_dir(&self) -> Option<&Path> {
        self.chosen
            .get_or_init(|| choose(&self.dir, self.check))
            .as_deref()
    }

    /// Attach the webview environment options (folder · browser arguments · extensions · scrollbar style) and the background · first theme (`backdrop`) to the window builder and turn off the drop handler — all window-creating code passes before `build()`. Where the app draws window buttons it lifts the OS frame and keeps the shadow (module header). Values attached earlier to the builder (including what the config window got via `from_config`) are overwritten — except the folder (not attached on fallback). The initialization script is not overwritten but added after earlier ones.
    // hangul-ok-begin
    /// The folder is passed regardless of OS — whether that value is used is up to Tauri's platform implementation (CLAUDE.md 「플랫폼 중립」).
    // hangul-ok-end
    pub(crate) fn finish<'a, R: Runtime, M: Manager<R>>(
        &self,
        builder: WebviewWindowBuilder<'a, R, M>,
        backdrop: Backdrop,
    ) -> WebviewWindowBuilder<'a, R, M> {
        attach(builder, self.chosen_dir(), backdrop, os::caption_style())
    }
}

// What the finishing step attaches — a real builder has no way to read back what was attached, so the test uses a fake builder to see what was attached.
trait EnvOptions: Sized {
    fn browser_args(self, args: &str) -> Self;
    fn extensions(self, enabled: bool) -> Self;
    fn scroll_bars(self, style: ScrollBarStyle) -> Self;
    fn no_drop_handler(self) -> Self;
    fn data_dir(self, dir: PathBuf) -> Self;
    fn background(self, color: Color) -> Self;
    fn init_script(self, script: String) -> Self;
    fn decorated(self, on: bool) -> Self;
    fn shadowed(self, on: bool) -> Self;
}

impl<'a, R: Runtime, M: Manager<R>> EnvOptions for WebviewWindowBuilder<'a, R, M> {
    fn browser_args(self, args: &str) -> Self {
        self.additional_browser_args(args)
    }

    fn extensions(self, enabled: bool) -> Self {
        self.browser_extensions_enabled(enabled)
    }

    fn scroll_bars(self, style: ScrollBarStyle) -> Self {
        self.scroll_bar_style(style)
    }

    fn no_drop_handler(self) -> Self {
        self.disable_drag_drop_handler()
    }

    fn data_dir(self, dir: PathBuf) -> Self {
        self.data_directory(dir)
    }

    // Paint both the window layer (tao) and the webview layer (WebView2 `DefaultBackgroundColor`).
    fn background(self, color: Color) -> Self {
        self.background_color(color)
    }

    // Runs only in the main frame (tauri 2.11.3 — for sub-frames too it is `initialization_script_for_all_frames`).
    fn init_script(self, script: String) -> Self {
        self.initialization_script(script)
    }

    fn decorated(self, on: bool) -> Self {
        self.decorations(on)
    }

    // Keep the shadow · rounded corners · 1 px border on a borderless window (Windows 11 — the look is QA S10). tao puts an invisible resize border around it (tao 0.35.3 `WM_NCCALCSIZE` · `calculate_insets_for_dpi`).
    fn shadowed(self, on: bool) -> Self {
        self.shadow(on)
    }
}

fn attach<B: EnvOptions>(
    builder: B,
    dir: Option<&Path>,
    backdrop: Backdrop,
    caption: CaptionStyle,
) -> B {
    let builder = builder
        .browser_args(WEBVIEW2_BROWSER_ARGS)
        .extensions(false)
        .scroll_bars(ScrollBarStyle::Default)
        // Left on, wry replaces the webview child window's drop target (`IDropTarget`) with its own and does not pass it to the webview's (wry 0.55.1 `src/webview2/drag_drop.rs`), killing HTML5 drag and drop inside the document — the agent-tree move (react-arborist's react-dnd HTML5 backend) and dragging text into the input box run on that event. All that is lost is Tauri's file drag-and-drop event, which nothing uses. In exchange the webview receives drops of outside files · URLs and navigates there, so `fileDropGuard.ts` blocks them.
        // TRD S21-slot-docking edition 6.3 §14 G8
        .no_drop_handler();
    let builder = match dir {
        Some(dir) => builder.data_dir(dir.to_path_buf()),
        None => builder,
    };
    let Backdrop::Theme { label, theme, lang } = backdrop;
    let builder = builder
        .background(color(theme))
        .init_script(theme_script(&label, theme))
        .init_script(lang_script(&label, lang));
    // An OS where the app does not draw buttons (macOS's share — none for now) leaves the OS frame as is (TRD S26-window-chrome §7-11).
    // ADR-0303
    if caption.drawn_buttons {
        builder.decorated(false).shadowed(true)
    } else {
        builder
    }
}

// Check after converting to an absolute path (module header) — if it cannot be converted, it falls back the same as an unusable folder.
fn choose(dir: &Path, check: impl FnOnce(&Path) -> Result<(), String>) -> Option<PathBuf> {
    let (dir, checked) = match std::path::absolute(dir) {
        Ok(absolute) => {
            let checked = check(&absolute);
            (absolute, checked)
        }
        Err(e) => (
            dir.to_path_buf(),
            Err(format!("could not convert to an absolute path: {e}")),
        ),
    };
    match checked {
        Ok(()) => {
            tracing::info!(module = "webview_env", dir = %dir.display(), "webview data folder decided");
            Some(dir)
        }
        Err(reason) => {
            // ADR-0283
            tracing::warn!(
                module = "webview_env",
                dir = %dir.display(),
                %reason,
                "cannot write to the webview data folder; building the window with Tauri's default folder"
            );
            None
        }
    }
}

/// Does not create the folder (if it was created to try, it is undone) — the folder is created by Tauri when it creates the window.
fn fs_check(dir: &Path) -> Result<(), String> {
    crate::discovery::check_data_dir_writable(dir).map_err(webview_reason)
}

// Take out only the failure reason — that error's text (「extract to a writable location」) is a remedy for the data root and does not suit the webview folder. The path is carried by the log's `dir` field.
fn webview_reason(e: crate::discovery::DiscoveryError) -> String {
    match e {
        crate::discovery::DiscoveryError::DataDirUnwritable { reason, .. } => reason,
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tauri::utils::config::{ScrollBarStyle, WindowConfig};

    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "engram-webview-env-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ── Decision ──

    #[test]
    fn a_writable_dir_is_chosen() {
        let dir = std::env::temp_dir().join("webview");
        let mut seen = None;
        let chosen = choose(&dir, |dir| {
            seen = Some(dir.to_path_buf());
            Ok(())
        });
        assert_eq!(chosen, Some(dir.clone()));
        assert_eq!(seen, Some(dir));
    }

    #[test]
    fn an_unwritable_dir_falls_back_to_the_default() {
        assert_eq!(
            choose(Path::new("R/webview"), |_| Err("cause".to_owned())),
            None
        );
    }

    #[test]
    fn a_relative_dir_is_checked_and_handed_over_as_the_same_absolute_path() {
        let mut seen = None;
        let chosen = choose(Path::new("R/webview"), |dir| {
            seen = Some(dir.to_path_buf());
            Ok(())
        })
        .unwrap();
        assert!(chosen.is_absolute(), "{}", chosen.display());
        assert!(chosen.ends_with("R/webview"), "{}", chosen.display());
        assert_eq!(seen, Some(chosen), "passes the checked folder as is");
    }

    #[test]
    fn a_dir_that_cannot_be_made_absolute_falls_back_without_a_check() {
        assert_eq!(choose(Path::new(""), |_| panic!("does not check")), None);
    }

    #[test]
    fn the_choice_is_made_once_and_every_window_gets_it() {
        static CHECKS: AtomicUsize = AtomicUsize::new(0);
        fn counting_fail(_: &Path) -> Result<(), String> {
            CHECKS.fetch_add(1, Ordering::SeqCst);
            Err("cause".to_owned())
        }
        let env = WebviewEnv::new(PathBuf::from("R/webview"), counting_fail);
        let answers: Vec<_> = (0..3)
            .map(|_| env.chosen_dir().map(Path::to_path_buf))
            .collect();
        assert_eq!(answers, vec![None, None, None]);
        assert_eq!(CHECKS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn concurrent_first_calls_check_once() {
        static CHECKS: AtomicUsize = AtomicUsize::new(0);
        fn counting_ok(_: &Path) -> Result<(), String> {
            CHECKS.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(20));
            Ok(())
        }
        let dir = std::env::temp_dir().join("webview");
        let env = WebviewEnv::new(dir.clone(), counting_ok);
        let answers: Vec<_> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..4)
                .map(|_| s.spawn(|| env.chosen_dir().map(Path::to_path_buf)))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(answers.iter().all(|a| a.as_deref() == Some(dir.as_path())));
        assert_eq!(CHECKS.load(Ordering::SeqCst), 1);
    }

    // ── Production check (discovery) as is ──

    #[test]
    fn real_check_chooses_an_existing_dir() {
        let root = scratch("ok");
        let dir = root.join("webview");
        std::fs::create_dir(&dir).unwrap();
        assert_eq!(
            WebviewEnv::new(dir.clone(), fs_check).chosen_dir(),
            Some(dir.as_path())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_fallback_reason_is_the_cause_without_the_data_root_advice() {
        let reason = webview_reason(crate::discovery::DiscoveryError::DataDirUnwritable {
            path: "R/webview".into(),
            reason: "access denied (test)".into(),
        });
        assert_eq!(reason, "access denied (test)");
    }

    #[test]
    fn real_check_falls_back_when_a_file_sits_in_the_folder_spot() {
        let root = scratch("file");
        let dir = root.join("webview");
        std::fs::write(&dir, b"not a folder").unwrap();
        assert_eq!(WebviewEnv::new(dir, fs_check).chosen_dir(), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── What the finishing step attaches ──

    #[derive(Debug, PartialEq)]
    enum Attached {
        BrowserArgs(String),
        Extensions(bool),
        // The runtime-side `ScrollBarStyle` has no `PartialEq`, so compare by Debug spelling.
        ScrollBars(String),
        NoDropHandler,
        DataDir(PathBuf),
        Background(Color),
        InitScript(String),
        Decorations(bool),
        Shadow(bool),
    }

    #[derive(Default)]
    struct Recorder(Vec<Attached>);

    impl EnvOptions for Recorder {
        fn browser_args(mut self, args: &str) -> Self {
            self.0.push(Attached::BrowserArgs(args.to_owned()));
            self
        }

        fn extensions(mut self, enabled: bool) -> Self {
            self.0.push(Attached::Extensions(enabled));
            self
        }

        fn scroll_bars(mut self, style: tauri::webview::ScrollBarStyle) -> Self {
            self.0.push(Attached::ScrollBars(format!("{style:?}")));
            self
        }

        fn no_drop_handler(mut self) -> Self {
            self.0.push(Attached::NoDropHandler);
            self
        }

        fn data_dir(mut self, dir: PathBuf) -> Self {
            self.0.push(Attached::DataDir(dir));
            self
        }

        fn background(mut self, color: Color) -> Self {
            self.0.push(Attached::Background(color));
            self
        }

        fn init_script(mut self, script: String) -> Self {
            self.0.push(Attached::InitScript(script));
            self
        }

        fn decorated(mut self, on: bool) -> Self {
            self.0.push(Attached::Decorations(on));
            self
        }

        fn shadowed(mut self, on: bool) -> Self {
            self.0.push(Attached::Shadow(on));
            self
        }
    }

    /// An OS where the app draws window buttons (every OS for now — platform `caption_style`).
    const DRAWN: CaptionStyle = CaptionStyle {
        drawn_buttons: true,
        leading_inset: 0.0,
    };

    /// Lift the OS frame and keep the shadow (TRD S26-window-chrome §8-1).
    fn frameless() -> [Attached; 2] {
        [Attached::Decorations(false), Attached::Shadow(true)]
    }

    fn common() -> Vec<Attached> {
        vec![
            Attached::BrowserArgs(WEBVIEW2_BROWSER_ARGS.to_owned()),
            Attached::Extensions(false),
            Attached::ScrollBars("Default".to_owned()),
            Attached::NoDropHandler,
        ]
    }

    fn dark(label: &str) -> Backdrop {
        Backdrop::Theme {
            label: label.to_owned(),
            theme: UiTheme::Dark,
            lang: Lang::En,
        }
    }

    fn painted_dark(label: &str) -> [Attached; 3] {
        [
            Attached::Background(color(UiTheme::Dark)),
            Attached::InitScript(theme_script(label, UiTheme::Dark)),
            Attached::InitScript(lang_script(label, Lang::En)),
        ]
    }

    #[test]
    fn every_window_gets_the_environment_options_and_no_drop_handler() {
        let dir = std::env::temp_dir().join("webview");
        let mut with_dir = common();
        with_dir.push(Attached::DataDir(dir.clone()));
        with_dir.extend(painted_dark("main"));
        with_dir.extend(frameless());
        assert_eq!(
            attach(Recorder::default(), Some(&dir), dark("main"), DRAWN).0,
            with_dir
        );
        let mut fallback = common();
        fallback.extend(painted_dark("main"));
        fallback.extend(frameless());
        assert_eq!(
            attach(Recorder::default(), None, dark("main"), DRAWN).0,
            fallback,
            "even when falling back to the default folder, the drop handler is turned off"
        );
    }

    /// If the app draws window buttons, lift the OS frame and keep the shadow; an OS that does not (macOS's share — none for now) leaves it as is (TRD S26-window-chrome §7-11 · §8-1).
    #[test]
    fn the_os_frame_is_dropped_only_where_the_app_draws_the_caption_buttons() {
        let drawn = attach(Recorder::default(), None, dark("slot-popup-3"), DRAWN).0;
        assert!(
            drawn.ends_with(&frameless()),
            "the shadow is left after the frame is removed: {drawn:?}"
        );

        let native = CaptionStyle {
            drawn_buttons: false,
            leading_inset: 72.0,
        };
        let kept = attach(Recorder::default(), None, dark("slot-popup-3"), native).0;
        assert!(
            !kept
                .iter()
                .any(|a| matches!(a, Attached::Decorations(_) | Attached::Shadow(_))),
            "{kept:?}"
        );
    }

    /// The OS frame is decided in one place, the finishing step — a config window declaration writing `decorations` · `shadow` would make two sources (TRD S26-window-chrome §8-1). Config JSON cannot carry comments, so the reason is here.
    #[test]
    fn config_windows_leave_the_os_frame_to_finish() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        for value in conf["app"]["windows"].as_array().unwrap() {
            for key in ["decorations", "shadow"] {
                assert!(
                    value.get(key).is_none(),
                    "window {} key {key}",
                    value["label"]
                );
            }
        }
    }

    #[test]
    fn a_themed_window_is_painted_with_the_themes_background_and_boots_on_that_theme() {
        for (theme, expected, wire) in [
            (UiTheme::Dark, Color(0x0a, 0x0a, 0x0a, 255), "dark"),
            (UiTheme::Light, Color(0xf5, 0xf5, 0xf5, 255), "light"),
            (UiTheme::EInk, Color(0xff, 0xff, 0xff, 255), "e-ink"),
        ] {
            let mut painted = common();
            painted.push(Attached::Background(expected));
            painted.push(Attached::InitScript(theme_script("slot-popup-3", theme)));
            painted.push(Attached::InitScript(lang_script("slot-popup-3", Lang::Ko)));
            painted.extend(frameless());
            let backdrop = Backdrop::Theme {
                label: "slot-popup-3".to_owned(),
                theme,
                lang: Lang::Ko,
            };
            assert_eq!(
                attach(Recorder::default(), None, backdrop, DRAWN).0,
                painted,
                "{theme:?}"
            );

            let script = theme_script("slot-popup-3", theme);
            assert!(script.contains(&format!("var t='{wire}';")), "{script}");
            assert_eq!(
                script.matches(".dataset.theme=t;").count(),
                2,
                "both when the root element exists and when it attaches, the value is baked in: {script}"
            );
        }
    }

    /// On reload, first read the theme that window last painted; if none · an unknown spelling · or the storage is unreadable, the creation-time theme.
    #[test]
    fn the_theme_script_reads_the_windows_remembered_theme_first() {
        let script = theme_script("slot-popup-3", UiTheme::Light);
        let read = r#"try{var s=sessionStorage.getItem("engram.theme.slot-popup-3");if(["dark","light","e-ink"].indexOf(s)>=0)t=s;}catch(_){}"#;
        assert!(script.contains(read), "{script}");
        let baked = script
            .find("var t='light';")
            .expect("the theme at creation");
        let first_write = script.find(".dataset.theme=t;").expect("baking in");
        let memory = script.find(read).unwrap();
        assert!(
            baked < memory && memory < first_write,
            "order: the theme at creation → remember → bake in: {script}"
        );
    }

    #[test]
    fn the_theme_script_quotes_the_label_as_a_json_string() {
        let script = theme_script(r#"a"b\c"#, UiTheme::Dark);
        assert!(
            script.contains(r#"sessionStorage.getItem("engram.theme.a\"b\\c")"#),
            "{script}"
        );
    }

    /// The key the frontend writes and the key the initialization script reads have the same head — if they diverge, memory is not found on reload.
    #[test]
    fn the_memory_prefix_is_the_one_the_frontend_writes() {
        let ts = include_str!("../../src/theme/themeMemory.ts");
        assert!(
            ts.contains(&format!(
                "export const THEME_KEY_PREFIX = '{THEME_MEMORY_PREFIX}'"
            )),
            "THEME_KEY_PREFIX in src/theme/themeMemory.ts"
        );
    }

    #[test]
    fn the_lang_script_plants_the_language_at_creation() {
        for lang in Lang::ALL {
            let script = lang_script("slot-popup-3", lang);
            assert!(
                script.contains(&format!("var l='{}';", lang.code())),
                "{script}"
            );
            assert_eq!(
                script.matches(".lang=l;").count(),
                2,
                "both when the root element exists and when it attaches: {script}"
            );
        }
    }

    /// The per-window memory beats the setting the window was created with — but only an `en` · `ko` value; an unreadable storage keeps the setting.
    #[test]
    fn the_lang_script_reads_the_windows_remembered_language_first() {
        let script = lang_script("slot-popup-3", Lang::Ko);
        let read = r#"try{var s=sessionStorage.getItem("engram.lang.slot-popup-3");if(["en","ko"].indexOf(s)>=0)l=s;}catch(_){}"#;
        assert!(script.contains(read), "{script}");
        let baked = script
            .find("var l='ko';")
            .expect("the language at creation");
        let first_write = script.find(".lang=l;").expect("planting");
        let memory = script.find(read).unwrap();
        assert!(
            baked < memory && memory < first_write,
            "creation language → memory → planting: {script}"
        );
    }

    #[test]
    fn the_lang_script_quotes_the_label_as_a_json_string() {
        let script = lang_script(r#"a"b\c"#, Lang::En);
        assert!(
            script.contains(r#"sessionStorage.getItem("engram.lang.a\"b\\c")"#),
            "{script}"
        );
    }

    /// The key the frontend writes and the key the initialization script reads have the same head.
    #[test]
    fn the_lang_memory_prefix_is_the_one_the_frontend_writes() {
        let ts = include_str!("../../src/i18n/langMemory.ts");
        assert!(
            ts.contains(&format!(
                "export const LANG_KEY_PREFIX = '{LANG_MEMORY_PREFIX}'"
            )),
            "src/i18n/langMemory.ts LANG_KEY_PREFIX"
        );
    }

    // ── Environment options of the config window ──

    fn environment_options(
        window: &WindowConfig,
    ) -> (Option<&str>, Option<&Path>, bool, &ScrollBarStyle) {
        (
            window.additional_browser_args.as_deref(),
            window.data_directory.as_deref(),
            window.browser_extensions_enabled,
            &window.scroll_bar_style,
        )
    }

    // For the folder field this test is the only wall: a config window declaration writing `dataDirectory` could not be cleared by the finishing step when falling back to the default folder, so only the config window would get a different folder. The other three are overwritten by the finishing step and cannot diverge, but defaults are demanded for them too so the written value is never read as different from the real one (one source).
    #[test]
    fn config_windows_leave_every_environment_option_at_the_default() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let defaults = WindowConfig::default();
        for value in conf["app"]["windows"].as_array().unwrap() {
            let window: WindowConfig = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(
                environment_options(&window),
                environment_options(&defaults),
                "window {}",
                window.label
            );
        }
    }
}
