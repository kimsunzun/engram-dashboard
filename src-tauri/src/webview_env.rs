//! Webview environment — the data folder and browser arguments ([`WebviewEnv`]) that the WebView windows of this shell process use (TRD S21-storage §4).
//!
//! - ★Windows with the same data folder must have the same environment options★ — a window that violates this gets `Ok` from `build()` but no OS window appears
//!   (the ghost window of ADR-0054). So all code that creates windows passes through [`WebviewEnv::finish`], and that value comes from the
//!   one-per-shell instance. The window the config declares (main) is no exception — Tauri is left not to create it
//!   (`"create": false` in `tauri.conf.json`) and boot step 8 creates it with `from_config` and hands it to this finishing step
//!   (`state::placement`).
//! - ★The finishing step decides the four environment options (folder · browser arguments · extensions · scrollbar style)★ — these four are what
//!   `create_environment` of wry 0.55.1 `webview2` puts into the environment. Three are always pinned to the same values. Only the folder cannot be cleared by the builder,
//!   so when it falls back to the default folder, the `dataDirectory` written by the config window declaration remains — so this module's test looks at the config.
//! - ★The folder is decided once per process★ — if `<데이터 루트>\webview` is writable that folder, and if not no folder is passed,
//!   falling back to Tauri's default folder (`LocalData/<identifier>`) with a warn (it does not stop — TRD §6 D8). It is not asked again per window:
//!   if the answer changed between windows, the windows of one process would get different folders. The reason when the whole data root is unusable
//!   shows inside the window after it comes up, as a daemon connection failure (ADR-0134 decision 4 · ADR-0135).
//! - ★The folder that is checked and the folder that is passed are the same absolute path★ — the data root may be a relative path (`ENGRAM_DATA_DIR` is
//!   used as is). What WebView2 resolves a relative path against was not checked — if a relative path is passed, the checked folder and the
//!   folder in use may diverge.
//! - ★The window background and first theme ([`Backdrop`]) are also attached by the finishing step★ — it is not an environment option but a value that differs per window (that window's effective theme),
//!   but since all code that creates windows passes through here, it is taken as an argument and attached without omission.
//! - ★The OS frame (title bar · buttons) is also lifted by the finishing step★ — on an OS where the app draws the title bar · window buttons (platform
//!   `drawn_buttons` of `window::caption_style`) it is `decorations(false)` · `shadow(true)` (TRD S26-window-chrome §8-1).
//!   It is in one place, not lifted per window — and it is not written in the window declaration of `tauri.conf.json` either (this module's test looks at the config).
// ADR-0054

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engram_dashboard_platform::window::{self as os, CaptionStyle};
use tauri::webview::ScrollBarStyle;
use tauri::window::Color;
use tauri::{Manager, Runtime, WebviewWindow, WebviewWindowBuilder};

use crate::discovery::DataLayout;
use crate::theme::{EffectiveThemes, UiTheme, DEFAULT_THEME};

/// The WebView2 browser arguments — this value is the only source. The config window also receives this value, so it is not written in `tauri.conf.json`
/// (if written, this module's test catches it — config JSON cannot carry comments, so the reason is put here).
///
/// ★Do not change it to the wry default (which even turns off `msSmartScreenProtection`)★ — the invariant is not 「표준값」 (the standard value) but 「같은 폴더
/// 안에서 서로 같다」 (the same among themselves within the same folder) (the alternative ADR-0054 rejected).
pub(crate) const WEBVIEW2_BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI --autoplay-policy=no-user-gesture-required";

type WriteCheck = fn(&Path) -> Result<(), String>;

/// The window · webview background and the page's first theme painted when creating a window — what is seen before the theme the shell decided arrives over IPC. The window background is
/// a color erased when the window is shown (`crate::first_paint` — the window is shown after first paint, but may be shown earlier by the time limit), and if the webview background is
/// not painted, WebView2's default white background shows through before the page is painted. If the first theme is not embedded, the page is drawn once with the dark default
/// (`src/theme/bootTheme.ts`).
///
/// When newly building a transparent window, revive the branch that does not paint — if painted, tauri-runtime-wry 2.11.3 fills the bottom of a transparent window with that color
/// (`draw_surface` in `window/windows.rs` — it discards alpha) and the window becomes opaque (source reading).
pub(crate) enum Backdrop {
    /// Paint that theme's page background color ([`UiTheme::page_background`]) and embed `data-theme` with an initialization
    /// script that runs before the page script — if the theme that window `label` last painted in this webview ([`THEME_MEMORY_PREFIX`]) exists
    /// use that value, otherwise `theme`.
    Theme { label: String, theme: UiTheme },
}

impl Backdrop {
    /// Paint with that window's effective theme — for a window not yet in the model it is the window's theme decided in advance (`EffectiveThemes::stage`), and if none,
    /// the global theme. If there is no theme state it is the default theme (the shell registers it in the builder so this is never reached).
    pub(crate) fn for_window<R: Runtime, M: Manager<R>>(manager: &M, label: &str) -> Self {
        let theme = manager
            .try_state::<Arc<EffectiveThemes>>()
            .map_or(DEFAULT_THEME, |themes| themes.effective_for(label));
        Backdrop::Theme {
            label: label.to_string(),
            theme,
        }
    }
}

/// The head of the key under which the theme a window last painted is written into that webview's `sessionStorage` — the window label is appended. The frontend
/// writes it whenever it attaches a theme (`src/theme/themeMemory.ts`), and the initialization script reads it first on a reload (F5 · HMR full reload)
/// — that script is frozen to the theme at window creation, so reloading a window whose theme changed afterward would draw once with the old theme.
///
/// ★It must have the same spelling as `THEME_KEY_PREFIX` of `src/theme/themeMemory.ts`★ — if they diverge, memory is not found without an error
/// (this module's test reads that file and puts them side by side). ★Do not change it to `localStorage`★ — it persists across runs, but popout labels are
/// new each run (the run marker · the in-run sequence number is not reused — `commands::popout::popup_label`), so one
/// key per popout would pile up forever. The chance that the next run gets the same label is about 2⁻³², so inheriting someone else's theme is not a reason.
/// `sessionStorage` is separate per top-level browsing context by the HTML standard and survives reloads — whether a new WebView2 window
/// starts with an empty storage was not measured (even if not, labels are not reused within a run, so a new window does not
/// read someone else's key).
pub(crate) const THEME_MEMORY_PREFIX: &str = "engram.theme.";

/// Repaint the background of a window that is up with that theme — called per window when pushing the theme (`commands::settings`).
///
/// Painting on the tao 0.35.3 side only writes the color and redraws (`InvalidateRect` + `UpdateWindow`) — it does not touch window flags, so even calling it on a window waiting
/// hidden (a spare window — `commands::spare`) does not make that window visible (source reading).
pub(crate) fn repaint<R: Runtime>(window: &WebviewWindow<R>, theme: UiTheme) {
    // The background is a color seen only before the page covers it, so it is no reason to make delivery (the theme notification) a failure — it is only left in the log.
    if let Err(e) = window.set_background_color(Some(color(theme))) {
        tracing::debug!(
            module = "webview_env",
            label = window.label(),
            "창 바탕을 다시 칠하지 못했다: {e}"
        );
    }
}

fn color(theme: UiTheme) -> Color {
    let (r, g, b) = theme.page_background();
    Color(r, g, b, 255)
}

// The initialization script runs again on every reload — if the remembered theme (`THEME_MEMORY_PREFIX`) is one of the three spellings, that value, otherwise (first
//   load · storage unreadable) it embeds the theme at window creation.
// The key is put in as a JSON string — the label is made by the shell, but so the script does not break even if quotes · backslashes are mixed in.
// Whether `document.documentElement` already exists at the moment the document is born was not checked in WebView2 — if not, it embeds when the root element
//   attaches (observer callback = a microtask, so earlier than page scripts · first paint).
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

/// The webview environment of this process — one per shell (shared with `Arc` · managed in the builder).
///
/// ★Do not attach `Clone`★ — if the decision field splits into two, each copy decides separately and windows may receive different folders.
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

    /// The data folder to pass to windows (absolute path) — `None` = do not pass (Tauri's default folder). On the first call it does a write check once,
    /// and after that returns the same answer (even if called concurrently, the check happens once).
    pub(crate) fn chosen_dir(&self) -> Option<&Path> {
        self.chosen
            .get_or_init(|| choose(&self.dir, self.check))
            .as_deref()
    }

    /// Attach the webview environment options (folder · browser arguments · extensions · scrollbar style) and the background · first theme (`backdrop`) to the window builder and turn off the drop
    /// handler — all code that creates windows passes through before `build()`. On an OS where the app draws window buttons, lift the OS frame and
    /// keep the shadow (module header). Values attached earlier to the builder (including what the config window received with `from_config`) are overwritten — the folder is the exception (it is not
    /// attached on fallback). The initialization script is not overwritten and is added after what was attached earlier.
    ///
    /// The folder is passed regardless of OS — whether that value is used is up to Tauri's platform implementation (CLAUDE.md 「플랫폼 중립」).
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

    // Keep the shadow · rounded corners · 1 px border on a borderless window (Windows 11 — the look is QA S10). tao puts an invisible border for resizing
    //   around this window (tao 0.35.3 `WM_NCCALCSIZE` · `calculate_insets_for_dpi`).
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
        // If left on, wry replaces the webview child window's drop target (`IDropTarget`) with its own and does not pass it on to the webview's (wry
        //   0.55.1 `src/webview2/drag_drop.rs`), so HTML5 drag and drop inside the document dies entirely — the agent-tree move
        //   (react-dnd HTML5 backend of react-arborist) and dragging text into the input box run on that event. All that is lost is Tauri's
        //   file drag-and-drop event, and nothing uses it. In exchange, the webview receives drops of outside files · URLs and navigates there, so
        //   `fileDropGuard.ts` blocks those drops.
        // TRD S21-slot-docking edition 6.3 §14 G8
        .no_drop_handler();
    let builder = match dir {
        Some(dir) => builder.data_dir(dir.to_path_buf()),
        None => builder,
    };
    let Backdrop::Theme { label, theme } = backdrop;
    let builder = builder
        .background(color(theme))
        .init_script(theme_script(&label, theme));
    // An OS where the app does not draw buttons (macOS's share — none for now) leaves the OS frame as is (TRD S26-window-chrome §7-11).
    // ADR-0303 — Chrome-style title bar
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
            Err(format!("절대 경로로 바꾸지 못했다: {e}")),
        ),
    };
    match checked {
        Ok(()) => {
            tracing::info!(module = "webview_env", dir = %dir.display(), "웹뷰 데이터 폴더 결정");
            Some(dir)
        }
        Err(reason) => {
            // ADR-0283
            tracing::warn!(
                module = "webview_env",
                dir = %dir.display(),
                %reason,
                "웹뷰 데이터 폴더에 쓸 수 없어 Tauri 기본 폴더로 창을 만든다"
            );
            None
        }
    }
}

/// Does not create the folder (if it was created to try, it is undone) — the folder is created by Tauri when it creates the window.
fn fs_check(dir: &Path) -> Result<(), String> {
    crate::discovery::check_data_dir_writable(dir).map_err(webview_reason)
}

// Take out only the failure reason — that error's text (「쓰기 가능한 위치에 압축을 풀어 주세요」, unzip to a writable location) is a remedy written with the data root in mind and does not
//   suit the webview folder. The path is carried by the `dir` field of the log.
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
            choose(Path::new("R/webview"), |_| Err("원인".to_owned())),
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
        assert_eq!(seen, Some(chosen), "확인한 폴더를 그대로 넘긴다");
    }

    #[test]
    fn a_dir_that_cannot_be_made_absolute_falls_back_without_a_check() {
        assert_eq!(choose(Path::new(""), |_| panic!("확인하지 않는다")), None);
    }

    #[test]
    fn the_choice_is_made_once_and_every_window_gets_it() {
        static CHECKS: AtomicUsize = AtomicUsize::new(0);
        fn counting_fail(_: &Path) -> Result<(), String> {
            CHECKS.fetch_add(1, Ordering::SeqCst);
            Err("원인".to_owned())
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
            reason: "접근이 거부되었다(시험)".into(),
        });
        assert_eq!(reason, "접근이 거부되었다(시험)");
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
        }
    }

    fn painted_dark(label: &str) -> [Attached; 2] {
        [
            Attached::Background(color(UiTheme::Dark)),
            Attached::InitScript(theme_script(label, UiTheme::Dark)),
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
            "기본 폴더로 물러나도 드롭 처리기는 끈다"
        );
    }

    /// If the app draws window buttons, lift the OS frame and keep the shadow — an OS that does not draw them (macOS's share — none for now) leaves the OS frame as is
    /// (TRD S26-window-chrome §7-11 · §8-1).
    #[test]
    fn the_os_frame_is_dropped_only_where_the_app_draws_the_caption_buttons() {
        let drawn = attach(Recorder::default(), None, dark("slot-popup-3"), DRAWN).0;
        assert!(
            drawn.ends_with(&frameless()),
            "그림자는 틀을 걷은 뒤에 남긴다: {drawn:?}"
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

    /// The OS frame is decided in one place, the finishing step — if the config window declaration wrote `decorations` · `shadow`, there would be two sources with the finishing step (TRD
    /// S26-window-chrome §8-1). Config JSON cannot carry comments, so the reason is put here.
    #[test]
    fn config_windows_leave_the_os_frame_to_finish() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        for value in conf["app"]["windows"].as_array().unwrap() {
            for key in ["decorations", "shadow"] {
                assert!(value.get(key).is_none(), "창 {} 의 {key}", value["label"]);
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
            painted.extend(frameless());
            let backdrop = Backdrop::Theme {
                label: "slot-popup-3".to_owned(),
                theme,
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
                "뿌리 요소가 있을 때 · 붙을 때 둘 다 그 값을 박는다: {script}"
            );
        }
    }

    /// On reload, first read the theme that window last painted, and if there is none · an unknown spelling · or the storage is unreadable, it is the theme at
    /// creation.
    #[test]
    fn the_theme_script_reads_the_windows_remembered_theme_first() {
        let script = theme_script("slot-popup-3", UiTheme::Light);
        let read = r#"try{var s=sessionStorage.getItem("engram.theme.slot-popup-3");if(["dark","light","e-ink"].indexOf(s)>=0)t=s;}catch(_){}"#;
        assert!(script.contains(read), "{script}");
        let baked = script.find("var t='light';").expect("만들 때의 테마");
        let first_write = script.find(".dataset.theme=t;").expect("박기");
        let memory = script.find(read).unwrap();
        assert!(
            baked < memory && memory < first_write,
            "만들 때의 테마 → 기억 → 박기 차례: {script}"
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
            "src/theme/themeMemory.ts 의 THEME_KEY_PREFIX"
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

    // For the folder field this test is the only wall — if the config window declaration wrote `dataDirectory`, the finishing step could not clear that value when it falls back to the default folder,
    //   so only the config window would get a different folder. The other three are overwritten by the finishing step and do not diverge, but so that the written value is not read
    //   as different from the real one (one source), it demands the defaults for them too.
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
                "창 {}",
                window.label
            );
        }
    }
}
