//! Window placement — writes `Moved` and `Resized` into the model ([`record`]) and applies the saved placement to windows ([`restore_windows`] — boot
//! steps 8 and 9 create the static window (main) at that placement · TRD S21-storage §4 · §6-3 · §6-5 · the window port of runtime restore acceptance
//! [`TauriRestoreWindows`] — §6-7 ② ④).
//!
//! - ★Position = physical outer position (`outer_position` as is), size = logical inner size divided by the window's scale factor★. A logical position
//!   becomes ambiguous when monitors with different scale factors are mixed: one value points at two monitors (150% to the right of 100% — the logical x 1280..1920 band), so the window is
//!   restored on the other monitor — ★do not turn the position back into logical★. Writing the position as physical is the same as tauri-plugin-window-state
//!   (that plugin writes the size as physical too). Keeping the size logical preserves the visible size even if the scale setting changes, and it goes straight into the popout
//!   builder's size argument (logical).
//! - The monitor a window actually sits on is decided by its physical position — the monitor [`locate`] picks is used only to decide whether to discard and for the popout builder's first
//!   placement.
//! - ★Call window getters and setters before any lock (or after releasing it), and only write values inside a lock★ — no OS
//!   calls while the layout lock is held (§6-3 · `layout/apply.rs` header 「락 규율」).
//! - Does not panic — it is called from the window event handler, and a panic in the shell is turned into a shell exit right away by the hook (ADR-0292).

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use engram_dashboard_base::sync;
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, PhysicalPosition, WebviewWindow,
    WebviewWindowBuilder, Window,
};

use super::restore::RestoreWindows;
use crate::commands::popout::POPOUT_DEFAULT_SIZE;
use crate::layout::{
    LayoutState, MoveFailed, MoveStep, WindowAttrs, WindowBounds, WindowPlace, WindowPlacement,
    MAIN_WINDOW_LABEL,
};
use crate::theme::ThemeControl;
use crate::webview_env::{Backdrop, WebviewEnv};

/// One monitor — the physical-pixel rectangle the OS reported and its scale factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorArea {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub scale: f64,
}

/// The placement [`locate`] picked — the physical outer position and the scale factor of the chosen monitor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Landing {
    pub x: i32,
    pub y: i32,
    pub scale: f64,
}

/// The monitor the saved placement goes to. For each monitor, take the physical rectangle for 「창이 그 모니터에 있다면」 (if the window were on that monitor) (the saved position as is · logical size ×
/// that monitor's scale factor) and pick the monitor it overlaps most (the first enumerated if equal). `None` = it overlaps
/// no monitor (including zero-area contact) — that placement is discarded and the window opens at the default placement (§6-5 ⑧ ⑨ · user decision F8). A monitor whose scale factor
/// is not a finite positive number is treated as absent.
pub fn locate(bounds: WindowBounds, monitors: &[MonitorArea]) -> Option<Landing> {
    let (left, top) = (bounds.x(), bounds.y());
    let mut best: Option<(f64, f64)> = None;
    for m in monitors {
        if !(m.scale.is_finite() && m.scale > 0.0) {
            continue;
        }
        let (mx, my) = (f64::from(m.x), f64::from(m.y));
        let (mr, mb) = (mx + f64::from(m.w), my + f64::from(m.h));
        let w = (left + bounds.w() * m.scale).min(mr) - left.max(mx);
        let h = (top + bounds.h() * m.scale).min(mb) - top.max(my);
        if w > 0.0 && h > 0.0 {
            let area = w * h;
            if best.is_none_or(|(widest, _)| area > widest) {
                best = Some((area, m.scale));
            }
        }
    }
    best.map(|(_, scale)| Landing {
        x: to_px(left),
        y: to_px(top),
        scale,
    })
}

fn to_px(v: f64) -> i32 {
    v.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

/// Turn getter values (physical outer position · physical inner size · the window's scale factor) into a placement to record — the position stays as is, the size is divided by the scale factor (module
/// header). `None` if the scale factor is not a finite positive number or [`WindowBounds::new`] rejects it (size 0 — minimized, etc.).
pub fn saved_bounds(x: i32, y: i32, w: u32, h: u32, scale: f64) -> Option<WindowBounds> {
    if !(scale.is_finite() && scale > 0.0) {
        return None;
    }
    WindowBounds::new(
        f64::from(x),
        f64::from(y),
        f64::from(w) / scale,
        f64::from(h) / scale,
    )
}

/// Boot step 9 — open a window with `open` for each popout in the model (in label issue order). A window that could not be opened is removed from the model — the next periodic
/// save carries that. Returns the removed labels. ★Call `open` outside the lock★ (creating a window = an OS call).
///
/// ★Boot only★ — it assumes the model's popouts have no windows yet. Called at runtime, window creation fails for every popout that is up, because
/// its label already exists, so the popout is removed from the model while the OS window stays. Runtime restore (P3c1) uses a different path.
pub fn open_restored_popouts(
    layout: &LayoutState,
    mut open: impl FnMut(&str, WindowAttrs) -> Result<(), String>,
) -> Vec<String> {
    let mut popouts: Vec<(String, WindowAttrs)> = match layout.0.lock() {
        Ok(mgr) => mgr
            .windows
            .iter()
            .filter(|(label, _)| label.as_str() != MAIN_WINDOW_LABEL)
            .map(|(label, window)| (label.clone(), window.attrs))
            .collect(),
        Err(_) => {
            tracing::error!(
                module = "state",
                "레이아웃 락에 독이 들어 복원한 팝아웃 창을 열지 못했다"
            );
            return Vec::new();
        }
    };
    // Same prefix · same run marker (one issuer) + a decimal sequence number, so (length, characters) order is issue order — they pile up in the saved order.
    popouts.sort_by_key(|(label, _)| (label.len(), label.clone()));

    let failed: Vec<String> = popouts
        .into_iter()
        .filter_map(|(label, attrs)| match open(&label, attrs) {
            Ok(()) => None,
            Err(e) => {
                tracing::warn!(
                    module = "state",
                    label,
                    error = %e,
                    "복원한 팝아웃 창을 못 열어 모델에서 지운다"
                );
                Some(label)
            }
        })
        .collect();
    if !failed.is_empty() {
        match layout.0.lock() {
            Ok(mut mgr) => {
                for label in &failed {
                    let _ = mgr.drop_window_gone(label);
                }
            }
            Err(_) => tracing::error!(
                module = "state",
                ?failed,
                "레이아웃 락에 독이 들어 못 연 팝아웃을 모델에서 지우지 못했다"
            ),
        }
    }
    failed
}

/// The end of boot step 9 — open the restored popout windows ([`open_restored_popouts`] — its return value too) and **then** push the effective theme to all
/// windows once (§5-6). Since it comes after opening, new popouts are in the receiving window list too, and popouts that could not be opened are already out of the model.
/// ★The caller holds no lock★.
fn open_popouts_then_push_themes(
    layout: &LayoutState,
    open: impl FnMut(&str, WindowAttrs) -> Result<(), String>,
    themes: &ThemeControl,
) -> Vec<String> {
    let failed = open_restored_popouts(layout, open);
    themes.push();
    failed
}

/// The maximize desire to apply when that window becomes visible — one per label (true = maximize · false = original size · a later write overrides an earlier one).
///
/// - Writers = the paths that apply the placement · maximize to main (boot ⑧ [`restore_windows`] · runtime restore acceptance
///   [`TauriRestoreWindows`] — while main is hidden, until it is shown; if visible, until it is applied on the spot —
///   [`DeferredMaximize::apply_now`]) · maximized popouts created hidden (boot restore [`open_popout`] · runtime restore acceptance
///   `RestoreWindows::open_hidden`) — up to here it is always true · window-state commands on a window that is not visible (`window.maximize` ·
///   `window.unmaximize` — [`defer_maximize`] · TRD S26-window-chrome §5-2). Clearers = the path that applied it successfully · every show
///   path (the first-paint show [`show_placed`] · the no-focus show in `first_paint` · the tray show · the show that does not steal
///   focus — all pass through [`apply_deferred_maximize`]) · a window-state command on a window already at that size ([`drop_deferred_maximize`]) ·
///   runtime restore applying a non-maximized copy (main) · window destruction ([`forget_deferred_maximize`]).
/// - When a hidden window is maximized, tao emits `SW_MAXIMIZE` (which shows and activates the window) and only then hides it again with `SW_HIDE` (tao 0.35.3
///   `platform_impl/windows/window_state.rs` `apply_diff`) — on a `--hidden` boot main flashes and takes focus. Unmaximize is
///   recorded as a desire for the same reason (TRD §5-2).
/// - While a desire exists (whatever its value), that window's placement is not recorded ([`record`]) — a `Moved` · `Resized` that arrives while hidden
///   reads the normal placement the restore applied as 「최대화 아님」 (not maximized), and recording it lowers the model's maximize marker (if it finishes before the window is shown, the next
///   boot loses the maximize). So when only the desire is left, recording that window's placement and pushing `window:frame` stop.
/// - Boot always creates main and popouts hidden ([`restore_windows`]) and the desire stays until they are shown — for a visible boot, until the first-paint
///   show; for a `--hidden` boot, until the tray show. For runtime restore acceptance on a hidden main (the LLM's answer after a tray hide · a `--hidden` boot), it stays
///   until it is shown.
/// - The lock is a leaf — while holding it, make no window call · log · other lock.
#[derive(Debug, Default)]
pub struct DeferredMaximize(Mutex<Wishes>);

#[derive(Debug, Default)]
struct Wishes {
    // Goes up by one on every write — even writing the same value again (including true → false → true) gives a different revision.
    next_rev: u64,
    by_label: BTreeMap<String, Wish>,
}

// `rev` = the revision that wrote this desire. The side that applies outside the lock clears only if the revision it read is unchanged — looking at the value alone would erase a desire
//   of the same value that was written again in between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Wish {
    maximized: bool,
    rev: u64,
}

impl DeferredMaximize {
    fn wishes(&self) -> MutexGuard<'_, Wishes> {
        // The field is a single table, so there is no half-changed state — it is called from the window event handler, so it must not panic (module header).
        sync::lock(&self.0)
    }

    fn defer(&self, label: &str, maximized: bool) -> Wish {
        let mut wishes = self.wishes();
        let wish = Wish {
            maximized,
            rev: wishes.next_rev,
        };
        wishes.next_rev = wishes.next_rev.wrapping_add(1);
        wishes.by_label.insert(label.to_string(), wish);
        wish
    }

    fn wish(&self, label: &str) -> Option<Wish> {
        self.wishes().by_label.get(label).copied()
    }

    fn is_pending(&self, label: &str) -> bool {
        self.wish(label).is_some()
    }

    /// If there was one, clear it and return its value — clearing on window destruction is this too.
    fn take(&self, label: &str) -> Option<bool> {
        self.wishes()
            .by_label
            .remove(label)
            .map(|wish| wish.maximized)
    }

    // Clear only if the desire that was read is still there — so that it does not erase a desire newly written by another thread (runtime restore acceptance · window-state command)
    //   while applying.
    fn take_if_unchanged(&self, label: &str, seen: Wish) -> bool {
        let mut wishes = self.wishes();
        let unchanged = wishes.by_label.get(label) == Some(&seen);
        if unchanged {
            wishes.by_label.remove(label);
        }
        unchanged
    }

    /// If there is a desire, apply it with `set(바람)` (the desire) and clear it only if applied — `true` = applied and cleared this time (the caller
    /// records the placement). ★If it could not be applied, keep it★ — clearing would record the normal placement as 「최대화 아님」 (not maximized), erasing the model's maximize with no way to
    /// apply it again. If kept, the next show applies it again. `set` is called outside the lock — if the desire was written again in between (even with the same
    /// value), keep the new desire and return `false`.
    fn apply(&self, label: &str, set: impl FnOnce(bool) -> bool) -> bool {
        let Some(seen) = self.wish(label) else {
            return false;
        };
        set(seen.maximized) && self.take_if_unchanged(label, seen)
    }

    /// Apply maximize to a visible window now — write a true desire, call `place` (normal placement) and `maximize`, and clear it only if
    /// applied. `true` = applied. ★If it could not be applied, keep it★ — [`record`] does not record that window as 「최대화 아님」 (not maximized) (including the placement
    /// record after runtime restore acceptance) and the next show applies it again (the same reason as [`Self::apply`]). `place` · `maximize` are called outside
    /// the lock.
    /// - ★Write it before `place`★ — so that the `Moved` · `Resized` emitted by applying the placement do not record the normal placement as 「최대화 아님」
    ///   (not maximized).
    /// - ★Call `maximize` even if a show applied and cleared the desire in between★ — the position setter after that lowered the maximize (F8).
    ///   If it could not be applied, write it again for that reason.
    fn apply_now(
        &self,
        label: &str,
        place: impl FnOnce(),
        maximize: impl FnOnce() -> bool,
    ) -> bool {
        let seen = self.defer(label, true);
        place();
        let applied = maximize();
        if applied {
            self.take_if_unchanged(label, seen);
        } else {
            self.defer(label, true);
        }
        applied
    }
}

// ── Tauri side — it needs a window to run, so it is for GUI measurement ──────────────────────────────

/// Boot steps 8 and 9 — create the static window (main) that the config declares at the saved placement (maximize of a hidden main is
/// deferred until it is shown — [`DeferredMaximize`]), open the restored popout windows (if one cannot be opened, remove it from the model), and push the effective
/// theme to all windows once (§5-6). ★The caller holds no lock★. The subscription recompute that follows (⑩) is the caller's job — the model after this function ends
/// is the final window set.
///
/// - ★Call it after the run marker (⑤) (I6)★ — if the app dies while creating windows, the next boot reads it as abnormal. The user setup always comes after the boot
///   step plugins, so it holds if it is inside setup.
/// - `boot_hidden` = a `--hidden` boot — main is not shown even at first paint and waits for the tray show. Popouts are still raised so they are visible
///   at first paint, and the caller's hide withdraws that raise and keeps them hidden together with main (user decision F13 ·
///   `crate::first_paint`).
/// - `Err` = main could not be created — stop right there (no popouts · no theme push) and the caller ends the app (user decision
///   2026-10-07).
///
/// ★Boot only★ — it assumes neither static windows nor popouts exist yet (what happens when called at runtime is in
/// [`open_restored_popouts`]).
pub(crate) fn restore_windows(
    app: &AppHandle,
    env: &WebviewEnv,
    boot_hidden: bool,
    layout: &LayoutState,
    themes: &ThemeControl,
) -> Result<(), MainWindowFailed> {
    let monitors = monitors(app);
    let main = match layout.0.lock() {
        Ok(mgr) => mgr.window_attrs(MAIN_WINDOW_LABEL).ok(),
        Err(_) => {
            tracing::debug!(
                module = "state",
                "레이아웃 락에 독이 들어 main 창 자리를 입히지 않는다"
            );
            None
        }
    };
    open_boot_windows(
        static_window(main, &monitors, boot_hidden),
        |window| open_static(app, env, window),
        layout,
        |label, attrs| open_popout(app, label, attrs, &monitors),
        themes,
    )
    .map(drop)
}

/// The main window could not be created — a reason to end the app ([`restore_windows`]).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MainWindowFailed;

/// The static window (main) that boot ⑧ creates — the saved attributes laid on top of the config's declaration (TRD S21-storage §4). `at` = the saved normal placement and the
/// monitor it goes to (`None` = open with the config's placement · size). `hide` = create it hidden even if the config says it is a visible window (`--hidden`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct StaticWindow {
    at: Option<(WindowBounds, Landing)>,
    maximized: bool,
    hide: bool,
}

/// `main` = the model's main field (`None` = could not be read — open as configured) · `boot_hidden` = a `--hidden` boot. A placement that overlaps
/// no monitor is discarded ([`land`]).
fn static_window(
    main: Option<WindowAttrs>,
    monitors: &[MonitorArea],
    boot_hidden: bool,
) -> StaticWindow {
    let main = main.unwrap_or_default();
    StaticWindow {
        at: main.bounds.and_then(|bounds| {
            land(MAIN_WINDOW_LABEL, bounds, monitors, Fallback::DefaultPlace).map(|at| (bounds, at))
        }),
        maximized: main.maximized,
        hide: boot_hidden,
    }
}

/// The order of boot ⑧ ⑨ — create main (`open_static` — `Err` = could not be created) and **then** open popouts and push themes
/// ([`open_popouts_then_push_themes`] — the `Ok` value too). Since main comes first, that window is in the receiving window list too. If main cannot be
/// created, stop right there with `Err`.
fn open_boot_windows(
    main: StaticWindow,
    open_static: impl FnOnce(StaticWindow) -> Result<(), String>,
    layout: &LayoutState,
    open_popout: impl FnMut(&str, WindowAttrs) -> Result<(), String>,
    themes: &ThemeControl,
) -> Result<Vec<String>, MainWindowFailed> {
    if let Err(e) = open_static(main) {
        tracing::error!(
            module = "state",
            label = MAIN_WINDOW_LABEL,
            error = %e,
            "main 창을 만들지 못해 앱을 끝낸다"
        );
        return Err(MainWindowFailed);
    }
    Ok(open_popouts_then_push_themes(layout, open_popout, themes))
}

// Create main and apply the saved placement — the same path as runtime restore acceptance ([`place_main`] — defers the maximize of a hidden main).
fn open_static(app: &AppHandle, env: &WebviewEnv, window: StaticWindow) -> Result<(), String> {
    let StaticWindow {
        at,
        maximized,
        hide,
    } = window;
    let built = build_static(app, env, MAIN_WINDOW_LABEL, at.map(first_place), hide)?;
    // If the saved size is narrower than the minimum width, the first minimum-size report widens and pulls it in (TRD S26-window-chrome §7-7 · D14).
    crate::chrome::mark_opening(app, MAIN_WINDOW_LABEL);
    place_main(app, &built.as_ref().window(), at, maximized);
    // A visible boot shows at first paint after applying the placement (maximize deferred). A `--hidden` boot waits for the tray show.
    if !hide {
        crate::first_paint::hold_window(&built);
    }
    Ok(())
}

// Create the window from the config's declaration — `at` = the first placement to give the builder (the caller sets the exact placement after creating — the same reason as for popouts ·
//   `at` of `build_runtime_window`). The webview environment is attached by the common finishing step — a window that skips it differs in environment from the other windows.
// ADR-0054
fn build_static(
    app: &AppHandle,
    env: &WebviewEnv,
    label: &str,
    at: Option<(LogicalPosition<f64>, LogicalSize<f64>)>,
    hide: bool,
) -> Result<WebviewWindow, String> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|config| config.label == label)
        .ok_or_else(|| "설정에 그 창의 선언이 없다".to_owned())?;
    let mut builder = WebviewWindowBuilder::from_config(app, config).map_err(|e| e.to_string())?;
    if let Some((position, size)) = at {
        builder = builder
            .position(position.x, position.y)
            .inner_size(size.width, size.height);
    }
    // ★Always created hidden★ — even a visible boot shows at first paint after applying the placement (`crate::first_paint`).
    builder = builder.visible(false);
    // A window kept hidden does not take focus — if it did, right after the hidden window is created it would take the foreground from the visible window. The origin of this rule
    //   is the hidden-tree window, now removed (ADR-0225 · QA observation 2026-10-07 — visible boot 3/3, that window took main's foreground).
    //   The remaining use is the single main of a `--hidden` boot, and its benefit (not stealing another app's foreground at autostart) is
    //   [unverified — GUI]. wry 0.55.1 puts keyboard focus into a `focused` webview right after creating it, with `MoveFocus`
    //   (the end of creation in `webview2/mod.rs`) — that this is the cause is a source reading [unverified — GUI]. When shown, the window's `WM_SETFOCUS`
    //   hands focus to the webview (the parent-window subclass in the same file). tao 0.35.3 shows this window without activation
    //   (`SW_SHOWNOACTIVATE`) — the tray show then brings it to the front with `set_focus`.
    // The main of a visible boot takes focus as configured — it already took it while hidden (the `MoveFocus` above), and the first-paint show activates it.
    //   ★That show must be the activating show (`SW_SHOW`) so the deferred maximize is applied without a normal-size frame★ ([`show_placed`]).
    if hide {
        builder = builder.focused(false);
    }
    let built = env
        .finish(builder, Backdrop::for_window(app, label))
        .build()
        .map_err(|e| e.to_string())?;
    confirm_created(built.is_visible().map_err(|e| e.to_string()), || {
        tauri::webview_version().map_err(|e| e.to_string())
    })?;
    Ok(built)
}

// Even if `build()` returns `Ok`, the runtime may not have created the window — tauri-runtime-wry 2.11.3 only logs a window creation failure on the main thread (no WebView
//   runtime — including `WebviewRuntimeNotInstalled` from `create_webview`) (handling of `Message::CreateWindow`) and Tauri still
//   returns that window (QA measured 2026-10-07 — when the runtime was hidden with `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER`). That window is not in the
//   runtime window table, so the getter's reply channel is closed — `answer` = one getter call on the created window. To the reason, attach the answer of the same
//   lookup (`webview_version`) that the runtime uses to decide 「설치됨」 (installed).
fn confirm_created(
    answer: Result<bool, String>,
    webview_version: impl FnOnce() -> Result<String, String>,
) -> Result<(), String> {
    let Err(e) = answer else {
        return Ok(());
    };
    Err(match webview_version() {
        Err(runtime) => format!("WebView 런타임을 찾지 못해 창이 만들어지지 않았다: {runtime}"),
        Ok(_) => format!("창이 만들어지지 않았다 — 런타임이 그 창을 모른다: {e}"),
    })
}

/// Open one restored popout window — the saved placement (the label's default placement if it overlaps no monitor) · maximize · the first placement
/// record. The window must already be in the model (the record writes into that entry).
fn open_popout(
    app: &AppHandle,
    label: &str,
    attrs: WindowAttrs,
    monitors: &[MonitorArea],
) -> Result<(), String> {
    let saved = attrs.bounds.and_then(|bounds| {
        land(label, bounds, monitors, Fallback::DefaultPlace).map(|at| (bounds, at))
    });
    let built = crate::commands::popout::build_runtime_window(app, label, saved.map(first_place))?;
    // If the saved size is narrower than the minimum width, the first minimum-size report widens and pulls it in (TRD S26-window-chrome §7-7 · D14).
    crate::chrome::mark_opening(app, label);
    let window = built.as_ref().window();
    if let Some((bounds, at)) = saved {
        place(&window, bounds, at);
    }
    // It is a hidden window, so the maximize is deferred to the first-paint show ([`DeferredMaximize`] · [`show_placed`]).
    if attrs.maximized {
        match app.try_state::<DeferredMaximize>() {
            Some(deferred) => {
                deferred.defer(label, true);
            }
            None => tracing::warn!(
                module = "state",
                label,
                "미룬 최대화 표가 등록돼 있지 않아 그 창의 최대화를 입히지 않는다"
            ),
        }
    }
    record(&window);
    crate::first_paint::hold_window(&built);
    Ok(())
}

// The popout builder's first placement — the logical position · logical size resolved with the chosen monitor's scale factor. The exact placement is set by [`place`] after creation
//   (the `at` argument of `build_runtime_window`).
pub(crate) fn first_place(
    (bounds, at): (WindowBounds, Landing),
) -> (LogicalPosition<f64>, LogicalSize<f64>) {
    (
        LogicalPosition::new(f64::from(at.x) / at.scale, f64::from(at.y) / at.scale),
        LogicalSize::new(bounds.w(), bounds.h()),
    )
}

/// Where the window goes if the saved placement is discarded — it only distinguishes the log wording of [`land`].
#[derive(Debug, Clone, Copy)]
pub(crate) enum Fallback {
    /// A newly opened window — open at the default placement (popout = the label's cascade placement · static window at boot = the config's placement).
    DefaultPlace,
    /// An existing window (main in runtime restore acceptance) — stays at its current placement.
    StayPut,
}

/// [`locate`] plus one log line — a placement discarded because it overlaps no monitor is left in the log at info.
pub(crate) fn land(
    label: &str,
    bounds: WindowBounds,
    monitors: &[MonitorArea],
    fallback: Fallback,
) -> Option<Landing> {
    let at = locate(bounds, monitors);
    if at.is_none() {
        match fallback {
            Fallback::DefaultPlace => tracing::info!(
                module = "state",
                label,
                "저장된 창 자리가 어느 모니터에도 안 걸쳐 버렸다 — 기본 자리로 연다"
            ),
            Fallback::StayPut => tracing::info!(
                module = "state",
                label,
                "저장된 창 자리가 어느 모니터에도 안 걸쳐 버렸다 — 창은 지금 자리에 남는다"
            ),
        }
    }
    at
}

pub(crate) fn place(window: &Window, bounds: WindowBounds, at: Landing) {
    let label = window.label();
    // Position first, size after — tao resolves a logical size with the scale factor the window has at that moment (`set_inner_size`), and that scale factor
    //   changes in WM_DPICHANGED handling. Setting the size after makes it resolve with the arrival monitor's scale factor. tauri-plugin-window-state
    //   also changed to this order, because the version that set the size first made the size jump on a monitor with a different scale factor (plugins-workspace #2583).
    //   [unverified — whether WM_DPICHANGED is handled inside the position setter (including whether it reaches a hidden window). If it comes late, tao
    //   re-measures the size at that point, and on Windows 11 the OS-suggested rectangle is applied as is, so it may be
    //   off by a few px from the saved position · size — for GUI measurement]
    if let Err(e) = window.set_position(PhysicalPosition::new(at.x, at.y)) {
        tracing::warn!(module = "state", label, error = %e, "저장된 창 위치를 못 입혔다");
    }
    if let Err(e) = window.set_size(LogicalSize::new(bounds.w(), bounds.h())) {
        tracing::warn!(module = "state", label, error = %e, "저장된 창 크기를 못 입혔다");
    }
}

// Call it after applying the normal placement — tao's position setter lowers the maximize marker (F8). `false` = could not be applied.
fn maximize(window: &Window) -> bool {
    match window.maximize() {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(module = "state", label = window.label(), error = %e, "저장된 최대화를 못 입혔다");
            false
        }
    }
}

// Apply the original-size desire — `false` = could not be applied.
fn unmaximize(window: &Window) -> bool {
    match window.unmaximize() {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(module = "state", label = window.label(), error = %e, "원래 크기 바람을 못 입혔다");
            false
        }
    }
}

// ── Placement application that returns results — the docking window port's share (TRD S21-slot-docking edition 5.8 §1-2 「적용 서비스」) ──────────
//
// [`place`] only logs failures — boot · restore have nothing more to do there. Docking must know which step could not be applied, to withdraw the new window or
// answer that the move failed, so each setter returns a result. The order is the same — position → size (reason = [`place`]).

/// The two window setters — a seam cut so [`move_steps`] can be tested without a window. Position = physical outer top-left · size = logical inner (module header).
pub(crate) trait Setters {
    fn set_position(&self, x: i32, y: i32) -> Result<(), String>;
    fn set_size(&self, w: f64, h: f64) -> Result<(), String>;
}

impl Setters for Window {
    fn set_position(&self, x: i32, y: i32) -> Result<(), String> {
        Window::set_position(self, PhysicalPosition::new(x, y)).map_err(|e| e.to_string())
    }

    fn set_size(&self, w: f64, h: f64) -> Result<(), String> {
        Window::set_size(self, LogicalSize::new(w, h)).map_err(|e| e.to_string())
    }
}

/// Position → size — ★stop at the first failure★ and return that step. A step that is `None` is skipped.
pub(crate) fn move_steps(
    window: &impl Setters,
    pos: Option<(i32, i32)>,
    size: Option<(f64, f64)>,
) -> Result<(), MoveFailed> {
    if let Some((x, y)) = pos {
        window.set_position(x, y).map_err(|error| MoveFailed {
            step: MoveStep::Position,
            error,
        })?;
    }
    if let Some((w, h)) = size {
        window.set_size(w, h).map_err(|error| MoveFailed {
            step: MoveStep::Size,
            error,
        })?;
    }
    Ok(())
}

/// Only a logical size with two finite positive numbers — otherwise `None` (the caller's default size · no size step).
pub(crate) fn usable_size(size: Option<(f64, f64)>) -> Option<(f64, f64)> {
    size.filter(|&(w, h)| WindowBounds::new(0.0, 0.0, w, h).is_some())
}

/// Test the placement docking picked against the monitors — if the rectangle made of `at.pos` plus the size (`at.size` · `default_size` if unusable)
/// overlaps a monitor, that normal placement and landing ([`locate`]). `None` if there is no position or it overlaps nowhere, and for one that overlaps nothing
/// [`land`] leaves the `fallback` wording.
pub(crate) fn resolve_place(
    label: &str,
    at: &WindowPlace,
    default_size: (f64, f64),
    monitors: &[MonitorArea],
    fallback: Fallback,
) -> Option<(WindowBounds, Landing)> {
    let (x, y) = at.pos?;
    let (w, h) = usable_size(at.size).unwrap_or(default_size);
    let bounds = WindowBounds::new(f64::from(x), f64::from(y), w, h)?;
    land(label, bounds, monitors, fallback).map(|landing| (bounds, landing))
}

/// Widen and pull in (TRD S26-window-chrome §7-7 · D14) — once when the window is opened, if the width of the normal placement `bounds` is narrower than the minimum width
/// `min_w` the web reported (logical inner px), widen it to that width; and if the widened window's ★visible right edge★ goes past the right of the work area `work` (the work area of the
/// monitor the window sits on — physical px · `scale` = that window's scale factor), pull it into the work area. If the work area is
/// narrower than the window, put the visible left edge at the left end of the work area (R20). `frame` = outer width − inner width (physical px — even a window whose OS frame was removed
/// keeps an invisible border for resizing). Visible edge = inward from the outer edge by one side of that border (`frame` / 2 — tao 0.35.3 puts the same
/// thickness on left and right · `calculate_insets_for_dpi`) — if the outer edges are aligned, the visible window sits
/// that far (about 8 px at 100 %) from the work-area edge.
///
/// If there is nothing to widen, return it as is — even a placement that straddles off-screen is not moved (that is not the job of widen-and-pull). Height · vertical position are
/// left alone. If the scale factor is not a finite positive number, only widen and do not pull.
// ADR-0303 — Chrome-style title bar
pub(crate) fn fit_min_width(
    bounds: WindowBounds,
    min_w: f64,
    work: MonitorArea,
    frame: f64,
) -> WindowBounds {
    // A `min_w` that is NaN is filtered out here too.
    if !(bounds.w() < min_w) {
        return bounds;
    }
    let x = if work.scale.is_finite() && work.scale > 0.0 {
        let side = frame.max(0.0) / 2.0;
        let left = f64::from(work.x);
        let right = left + f64::from(work.w);
        // The range of outer x — visible right (x + side + inner width) ≤ work-area right · visible left (x + side) ≥ work-area left.
        //   Fit the right first and the left wins — if both cannot be met (the work area is narrow), it is the visible left end.
        let max_x = (right - side - min_w * work.scale).floor();
        let min_x = (left - side).ceil();
        bounds.x().min(max_x).max(min_x)
    } else {
        bounds.x()
    };
    WindowBounds::new(x, bounds.y(), min_w, bounds.h()).unwrap_or(bounds)
}

/// The Tauri implementation of the window port `move_window` (contract = `layout::apply::WindowHost`). If the position overlaps no monitor, that
/// step is skipped — a window that is up is not moved to the default placement.
pub(crate) fn move_window(
    app: &AppHandle,
    label: &str,
    at: &WindowPlace,
) -> Result<(), MoveFailed> {
    let Some(window) = app.get_webview_window(label) else {
        return Err(MoveFailed {
            step: MoveStep::Position,
            error: "옮길 창이 없다".to_string(),
        });
    };
    let window = window.as_ref().window();
    let size = usable_size(at.size);
    let pos = match at.pos {
        Some(_) => {
            let landing_size = size
                .or_else(|| kept_size(app, &window, label))
                .unwrap_or(POPOUT_DEFAULT_SIZE);
            resolve_place(label, at, landing_size, &monitors(app), Fallback::StayPut)
                .map(|(_, landing)| (landing.x, landing.y))
        }
        None => None,
    };
    move_steps(&window, pos, size)
}

// The size (logical) used to measure overlap for a move that did not choose a size — it is the size that window will have after the move. Measuring with the popout default size would decide that a window smaller than that
//   overlaps, and it would be placed off-screen. The model's normal-placement size comes first — the getter of a maximized window gives the maximized size, but
//   when the position setter lowers the maximize it goes back to the normal size (F8). If not in the model, the current inner size. ★The layout lock only reads the value and
//   releases it right away★ — the getter is outside it (module header).
fn kept_size(app: &AppHandle, window: &Window, label: &str) -> Option<(f64, f64)> {
    if let Some(layout) = app.try_state::<LayoutState>() {
        let modeled = sync::lock(&layout.0)
            .window_attrs(label)
            .ok()
            .and_then(|attrs| attrs.bounds);
        if let Some(bounds) = modeled {
            return Some((bounds.w(), bounds.h()));
        }
    }
    let scale = window.scale_factor().ok()?;
    let size = window.inner_size().ok()?;
    saved_bounds(0, 0, size.width, size.height, scale).map(|b| (b.w(), b.h()))
}

// Apply the normal placement and maximize to main — the contract is `RestoreWindows::place_main` (boot ⑧ takes the same path). If visibility cannot be
//   read, treat it as visible — if deferred on a visible window, the maximize does not happen until the show path is called again.
fn place_main(
    app: &AppHandle,
    window: &Window,
    at: Option<(WindowBounds, Landing)>,
    maximized: bool,
) {
    let place_at = || {
        if let Some((bounds, at)) = at {
            place(window, bounds, at);
        }
    };
    let deferred = app.try_state::<DeferredMaximize>();
    if !maximized {
        // Clear the desire first — so that the `Moved` · `Resized` emitted by the placement application below are recorded ([`record`]). Whatever value
        //   the desire written earlier had, the copy wins.
        if deferred.is_some_and(|deferred| deferred.take(MAIN_WINDOW_LABEL).is_some()) {
            tracing::debug!(
                module = "state",
                "사본의 main 이 최대화가 아니라 적어 둔 최대화 바람을 거둔다"
            );
        }
        place_at();
        // [unverified — if a hidden main is maximized, tao's unmaximize (`SW_RESTORE`) shows and hides the window (the same path as maximize —
        //   [`DeferredMaximize`]). For GUI measurement]
        if matches!(window.is_maximized(), Ok(true)) {
            if let Err(e) = window.unmaximize() {
                tracing::warn!(module = "state", error = %e, "사본대로 main 의 최대화를 풀지 못했다");
            }
        }
        return;
    }
    let hidden = matches!(window.is_visible(), Ok(false));
    match deferred {
        Some(deferred) if hidden => {
            let seen = deferred.defer(MAIN_WINDOW_LABEL, true);
            place_at();
            // If, after reading visibility, the tray 「보이기」 cut in and already applied the deferred maximize, the position setter after that
            //   lowered the maximize (F8) — for a main that became visible, apply it here and clear the deferral. If that show comes after this, the deferral stays
            //   and that side applies it. If it could not be applied, keep the deferral (the same reason as [`DeferredMaximize::apply`]).
            if matches!(window.is_visible(), Ok(true)) && maximize(window) {
                deferred.take_if_unchanged(MAIN_WINDOW_LABEL, seen);
            }
        }
        Some(deferred) => {
            deferred.apply_now(MAIN_WINDOW_LABEL, place_at, || maximize(window));
        }
        None => {
            place_at();
            maximize(window);
        }
    }
}

/// Called by the show path ★after it shows the window and also unminimizes it (`unminimize`)★ — if that window has a maximize desire, apply it (true =
/// maximize · false = original size) and record the current placement ([`DeferredMaximize`]). It fills in here the placement that was not recorded while the desire existed.
/// If it could not be applied, keep the desire and do not record ([`DeferredMaximize::apply`]). If there is no desire, no-op.
///
/// ★Do not call it before the unminimize★ — tao 0.35.3 lowers the maximize flag of a minimized window, so applying the original size does nothing, just
/// clears the desire, and the later unminimize (`SW_RESTORE`) goes back to the pre-minimize maximize and loses the desire (TRD S26-window-chrome §6-2 ⓑ).
// TRD S26-window-chrome §5-2 「모든 보이기 갈래가 미룬 최대화를 입힌다」
pub fn apply_deferred_maximize(window: &Window) {
    if window
        .app_handle()
        .try_state::<DeferredMaximize>()
        .is_some_and(|deferred| {
            deferred.apply(window.label(), |maximized| {
                if maximized {
                    maximize(window)
                } else {
                    unmaximize(window)
                }
            })
        })
    {
        record(window);
    }
}

/// Write the maximize state that a window-state command (`window.maximize` · `window.unmaximize`) on a window that is not visible wants — it
/// overrides what was written earlier. The path that shows that window applies it with [`apply_deferred_maximize`] (TRD S26-window-chrome §5-2). If the table is not
/// registered, do nothing.
pub(crate) fn defer_maximize(app: &AppHandle, label: &str, maximized: bool) {
    match app.try_state::<DeferredMaximize>() {
        Some(deferred) => {
            deferred.defer(label, maximized);
        }
        None => tracing::warn!(
            module = "state",
            label,
            "미룬 최대화 표가 등록돼 있지 않아 그 창의 최대화 바람을 적지 않는다"
        ),
    }
}

/// Clear the maximize desire that was written (no-op if none) — called by the window-state command for a window whose OS window already is or is about to be at that size (TRD
/// S26-window-chrome §5-2 「이미 그 크기면 바람을 거둔다」).
pub(crate) fn drop_deferred_maximize(app: &AppHandle, label: &str) {
    if let Some(deferred) = app.try_state::<DeferredMaximize>() {
        deferred.take(label);
    }
}

/// Show for the first time a window that was built hidden and had its placement applied — the first-paint show (`crate::first_paint`). If the maximize desire is true, show it straight as
/// maximized without passing through a normal-size frame (platform `restore_placement` — the current normal placement is set as the placement to return to) and record the placement.
/// If that show fails (including outside Windows), maximize after showing it normally — the same path as the tray show, so a normal-size
/// frame is seen once. If the desire is false, show normally and then apply the original size ([`apply_deferred_maximize`]). ★Call it on the main
/// thread★.
///
/// It shows and activates.
///
/// ★A window to be shown maximized must be one built with the focus-taking builder (`focused(true)`)★ — Tauri `show`, which shows with raw Win32 and then
/// matches the tao visibility flag, emits `SW_SHOWNOACTIVATE` for a window built without focus (tao 0.35.3 `window_state.rs`
/// `apply_diff` — the code that clears that marker only reaches the copy, so it stays · source reading). That command
/// can undo the maximize, like `SW_SHOWNORMAL` [unverified]. Every window shown by this path (main of a visible boot · restored popouts · popouts the window port built as active) was
/// built with the focus-taking builder.
pub(crate) fn show_placed(window: &Window) {
    let label = window.label();
    let app = window.app_handle();
    let deferred = app.try_state::<DeferredMaximize>();
    let seen = deferred
        .as_ref()
        .and_then(|deferred| deferred.wish(label))
        .filter(|wish| wish.maximized);
    if let Some(seen) = seen {
        match show_maximized(window) {
            Ok(()) => {
                // Match the tao visibility flag — if left false, the next flag setter hides the window and the tray hide does nothing
                //   (if no flag changed, tao 0.35.3 `apply_diff` returns early · source reading).
                if let Err(e) = window.show() {
                    tracing::warn!(module = "state", label, error = %e, "최대화로 보인 창의 보임 깃발을 맞추지 못했다");
                }
                if deferred.is_some_and(|deferred| deferred.take_if_unchanged(label, seen)) {
                    record(window);
                }
                return;
            }
            Err(e) => tracing::warn!(
                module = "state",
                label,
                "곧바로 최대화로 보이지 못해 보통으로 보인 뒤 최대화한다: {e}"
            ),
        }
    }
    if let Err(e) = window.show() {
        tracing::warn!(module = "state", label, error = %e, "숨긴 창을 보이지 못했다");
        return;
    }
    apply_deferred_maximize(window);
}

// Take the hidden window's current outer rectangle (physical screen coordinates) as the normal placement and show it maximized.
fn show_maximized(window: &Window) -> Result<(), String> {
    let at = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let normal = engram_dashboard_platform::window::PhysRect {
        x: at.x,
        y: at.y,
        width: i32::try_from(size.width).map_err(|e| e.to_string())?,
        height: i32::try_from(size.height).map_err(|e| e.to_string())?,
    };
    engram_dashboard_platform::window::restore_placement(window, normal, true, true)
        .map_err(|e| e.to_string())
}

/// The window was destroyed (`WindowEvent::Destroyed`) — clear that window's deferred maximize. A popout label is not reused, so leaving it would not apply to another
/// window, but if not cleared the set grows with every window that was created hidden and withdrawn.
pub fn forget_deferred_maximize(app: &AppHandle, label: &str) {
    if app
        .try_state::<DeferredMaximize>()
        .is_some_and(|deferred| deferred.take(label).is_some())
    {
        tracing::debug!(
            module = "state",
            label,
            "미룬 최대화를 입히기 전에 창이 소멸했다"
        );
    }
}

/// The OS window port for runtime restore acceptance — the Tauri implementation (TRD S21-storage §6-7 ② ④). It does not use [`restore_windows`] ·
/// [`open_restored_popouts`], which are boot-only — it shares only placement decision ([`locate`]) and application ([`place`]).
pub struct TauriRestoreWindows {
    app: AppHandle,
}

impl TauriRestoreWindows {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    fn window(&self, label: &str) -> Option<Window> {
        let window = self.app.get_webview_window(label);
        if window.is_none() {
            tracing::debug!(
                module = "state",
                label,
                "창이 없어 복원한 자리 · 보임을 입히지 않는다"
            );
        }
        window.map(|window| window.as_ref().window())
    }
}

impl RestoreWindows for TauriRestoreWindows {
    fn app_has_focus(&self) -> bool {
        foreground_is_ours(&self.app)
    }

    fn monitors(&self) -> Vec<MonitorArea> {
        monitors(&self.app)
    }

    fn open_hidden(
        &self,
        label: &str,
        at: Option<(WindowBounds, Landing)>,
        maximized: bool,
    ) -> Result<(), String> {
        let deferred = if maximized {
            let deferred = self.app.try_state::<DeferredMaximize>();
            match deferred {
                Some(deferred) => {
                    let seen = deferred.defer(label, true);
                    Some((deferred, seen))
                }
                None => {
                    tracing::debug!(
                        module = "state",
                        label,
                        "미룬 최대화 표가 등록돼 있지 않아 그 창의 최대화를 입히지 않는다"
                    );
                    None
                }
            }
        } else {
            None
        };
        let built = match crate::commands::popout::build_runtime_window(
            &self.app,
            label,
            at.map(first_place),
        ) {
            Ok(built) => built,
            Err(e) => {
                if let Some((deferred, _)) = deferred {
                    deferred.take(label);
                }
                return Err(e);
            }
        };
        // Whether to show is decided after putting it into the model ([`RestoreWindows::set_shown`]) — first-paint notifications that arrived before that are held.
        crate::first_paint::hold_window_unarmed(&built);
        // If the copy's size is narrower than the minimum width, the first minimum-size report widens and pulls it in (TRD S26-window-chrome §7-7 · D14).
        crate::chrome::mark_opening(&self.app, label);
        let window = built.as_ref().window();
        if let Some((bounds, at)) = at {
            place(&window, bounds, at);
            // If, after creating it, the tray 「보이기」 cut in and already applied the deferred maximize, the position setter just now lowered that maximize (F8)
            //   — apply it again here to a window that became visible (the same reason as `place_main`). If that show comes after this, the deferral stays
            //   and that side applies it.
            if maximized && matches!(window.is_visible(), Ok(true)) && maximize(&window) {
                if let Some((deferred, seen)) = &deferred {
                    deferred.take_if_unchanged(label, *seen);
                }
            }
        }
        Ok(())
    }

    fn visibility(&self, label: &str) -> Option<bool> {
        let window = self.app.get_webview_window(label)?;
        Some(window.is_visible().unwrap_or_else(|e| {
            tracing::debug!(
                module = "state",
                label,
                error = %e,
                "창의 보임 여부를 못 읽어 보이는 것으로 친다"
            );
            true
        }))
    }

    fn place_main(&self, at: Option<(WindowBounds, Landing)>, maximized: bool) {
        if let Some(window) = self.window(MAIN_WINDOW_LABEL) {
            // If the copy's main width is narrower than the minimum width, widen and pull it in (TRD S26-window-chrome §7-7 · D14 · PRD AC34 「사본 수락」).
            //   main is not rebuilt, so its page does not report again, and the OS minimum size it already has grows a narrow placement
            //   while keeping the left end fixed — so lift that minimum size before the placement, and the opening mark after the placement widens and pulls in with the value it already holds, then applies again.
            //   ★Hand off the lift job before the placement setter★ — both take effect in the order they were handed to the main-thread queue (`crate::chrome::lift_min`).
            crate::chrome::lift_min(&self.app, MAIN_WINDOW_LABEL);
            place_main(&self.app, &window, at, maximized);
            crate::chrome::mark_opening(&self.app, MAIN_WINDOW_LABEL);
        }
    }

    fn set_shown(&self, label: &str, shown: bool) {
        let Some(window) = self.window(label) else {
            return;
        };
        crate::tray::actions::with_usage_visibility(&self.app, |usage| {
            crate::tray::actions::for_each_ui_window(
                [label],
                shown,
                usage,
                |_| false,
                |_| {},
                |label| {
                    // The show comes after first paint — that show applies the deferred maximize too ([`show_placed`]).
                    if shown {
                        crate::first_paint::reveal_held(&self.app, label);
                        return true;
                    }
                    crate::first_paint::cancel_hold(&self.app, label);
                    match window.hide() {
                        Ok(()) => true,
                        Err(e) => {
                            tracing::warn!(module = "state", label, error = %e, "복원한 창을 숨기지 못했다");
                            false
                        }
                    }
                },
            );
        });
    }

    fn record_placement(&self, label: &str) {
        if let Some(window) = self.window(label) {
            record(&window);
        }
    }

    fn focus(&self, label: &str) {
        if let Some(window) = self.window(label) {
            if let Err(e) = window.set_focus() {
                tracing::debug!(module = "state", label, error = %e, "복원 뒤 창에 포커스를 주지 못했다");
            }
        }
    }

    fn destroy(&self, label: &str) -> Result<(), String> {
        crate::commands::popout::try_destroy_window(&self.app, label)
    }
}

// Does our process hold the foreground window now — only then does tao's focus-giving stand without a fake Alt (`force_window_active` fires Alt
//   only when `SetForegroundWindow` is refused — tao 0.35.3).
// ★On Windows, do not use the window's focus getter (`is_focused`)★ — wry 0.55.1 hands focus to the webview on every `WM_SETFOCUS` of the window
//   (`MoveFocus`), so the window immediately receives `WM_KILLFOCUS`, and tao 0.35.3's value (`is_active && is_focused`) is
//   `false` while the webview holds keyboard focus. tauri-runtime-wry 2.11.3 therefore makes the window's focus events from the
//   webview's GotFocus · LostFocus rather than from tao. [unverified — source reading. For GUI measurement]
// On other OSes, that getter is used as is — when the OS layer says it has no means to answer (`None`). [unverified — that branch does not run on
//   Windows] The OS branch lives in the OS-layer crate (ADR-0266 — no `#[cfg(windows)]` in the shell).
pub(crate) fn foreground_is_ours(app: &AppHandle) -> bool {
    engram_dashboard_platform::window::foreground_is_current_process().unwrap_or_else(|| {
        app.webview_windows()
            .values()
            .any(|window| matches!(window.is_focused(), Ok(true)))
    })
}

pub(crate) fn monitors(app: &AppHandle) -> Vec<MonitorArea> {
    match app.available_monitors() {
        Ok(list) => list
            .iter()
            .map(|m| MonitorArea {
                x: m.position().x,
                y: m.position().y,
                w: m.size().width,
                h: m.size().height,
                scale: m.scale_factor(),
            })
            .collect(),
        Err(e) => {
            tracing::warn!(
                module = "state",
                error = %e,
                "모니터 목록을 못 읽어 저장된 창 자리를 모두 버린다"
            );
            Vec::new()
        }
    }
}

/// One set of window getters. `None` if the minimized · maximized state cannot be read (do not record — the placement during minimize would be recorded as the normal placement).
/// ★Call it holding no lock★.
fn read(window: &Window) -> Option<WindowPlacement> {
    let label = window.label();
    let flags = window
        .is_minimized()
        .and_then(|minimized| Ok((minimized, window.is_maximized()?)));
    let (minimized, maximized) = match flags {
        Ok(flags) => flags,
        Err(e) => {
            tracing::debug!(
                module = "state",
                label,
                error = %e,
                "창의 최소화 · 최대화 여부를 못 읽어 이번 자리는 적지 않는다"
            );
            return None;
        }
    };
    let bounds = (|| {
        let scale = window.scale_factor()?;
        let at = window.outer_position()?;
        let size = window.inner_size()?;
        Ok::<_, tauri::Error>(saved_bounds(at.x, at.y, size.width, size.height, scale))
    })()
    .unwrap_or_else(|e| {
        tracing::debug!(
            module = "state",
            label,
            error = %e,
            "창 위치 · 크기를 못 읽어 이번 자리는 적지 않는다"
        );
        None
    });
    Some(WindowPlacement {
        minimized,
        maximized,
        bounds,
    })
}

/// Record one window's current placement in that label's entry of the layout model. A label that is not in the model (a late event after close · a new
/// window before it entered the model) is dropped — a popout label is not reused, so it is not wrongly recorded on another window (`PopupCounter`). A window
/// whose maximize is deferred is not recorded either ([`DeferredMaximize`]).
/// The rules (minimize · maximize) are in `observe` of [`WindowAttrs`]. If recorded, after releasing the lock, push that window's window fact (`window:frame`) only when it
/// changes (`commands::tab_drop::push_window_frame`).
pub fn record(window: &Window) {
    let label = window.label();
    let app = window.app_handle();
    if app
        .try_state::<DeferredMaximize>()
        .is_some_and(|deferred| deferred.is_pending(label))
    {
        tracing::debug!(
            module = "state",
            label,
            "최대화를 보일 때로 미뤄 둔 동안이라 이번 자리는 적지 않는다"
        );
        return;
    }
    let Some(seen) = read(window) else {
        return;
    };
    let Some(layout) = app.try_state::<LayoutState>() else {
        tracing::debug!(
            module = "state",
            label,
            "레이아웃 상태가 등록돼 있지 않아 창 자리를 적지 않는다"
        );
        return;
    };
    // Called on every event while a window is being dragged — the poisoned-lock log is kept at debug (the other writer of that lock already logs it loudly).
    let Ok(mut mgr) = layout.0.lock() else {
        tracing::debug!(
            module = "state",
            label,
            "레이아웃 락에 독이 들어 창 자리를 적지 않는다"
        );
        return;
    };
    let attrs = match mgr.observe_window_placement(label, seen) {
        Ok(()) => mgr.window_attrs(label).ok(),
        Err(e) => {
            tracing::debug!(module = "state", label, error = %e, "모델에 없는 창의 자리라 버린다");
            None
        }
    };
    // Push after releasing the layout lock — pushing window facts calls getters.
    // TRD S21-slot-docking §3-8
    drop(mgr);
    if let Some(attrs) = attrs {
        crate::commands::tab_drop::push_window_frame(window, seen.minimized, attrs);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::layout::ViewManager;
    use crate::settings::SettingsService;
    use crate::theme::{EffectiveThemes, ThemeWindows, UiSettingsPayload, UiTheme, DEFAULT_THEME};

    fn monitor(x: i32, y: i32, w: u32, h: u32, scale: f64) -> MonitorArea {
        MonitorArea { x, y, w, h, scale }
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> WindowBounds {
        WindowBounds::new(x, y, w, h).unwrap()
    }

    fn landing(x: i32, y: i32, scale: f64) -> Option<Landing> {
        Some(Landing { x, y, scale })
    }

    const PRIMARY: MonitorArea = MonitorArea {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
        scale: 1.0,
    };

    /// A 150% monitor to the right of PRIMARY — in logical x it starts at 1280, so it overlaps PRIMARY's [1280, 1920).
    const RIGHT: MonitorArea = MonitorArea {
        x: 1920,
        y: 0,
        w: 2560,
        h: 1440,
        scale: 1.5,
    };

    // ── Picking a monitor ──

    #[test]
    fn a_rect_on_a_monitor_keeps_its_physical_position() {
        assert_eq!(
            locate(rect(80.0, 60.0, 1280.0, 800.0), &[PRIMARY]),
            landing(80, 60, 1.0)
        );
        // Overlapping even in part counts as overlapping.
        assert_eq!(
            locate(rect(-1000.0, 100.0, 1280.0, 800.0), &[PRIMARY]),
            landing(-1000, 100, 1.0)
        );
    }

    #[test]
    fn a_rect_off_every_monitor_is_discarded() {
        let both = [PRIMARY, RIGHT];
        assert_eq!(locate(rect(-3000.0, 100.0, 800.0, 600.0), &both), None);
        assert_eq!(
            locate(rect(100.0, 1500.0, 800.0, 600.0), &both),
            None,
            "아래"
        );
        // If only the edges touch, the area is 0 — it does not overlap.
        assert_eq!(locate(rect(4480.0, 0.0, 800.0, 600.0), &both), None);
        assert_eq!(locate(rect(-800.0, 0.0, 800.0, 600.0), &both), None);
        assert_eq!(
            locate(rect(0.0, 0.0, 800.0, 600.0), &[]),
            None,
            "모니터 없음"
        );
    }

    #[test]
    fn the_rect_on_each_monitor_is_its_logical_size_times_that_monitors_scale() {
        // At the logical size as is it is 1800..1900 and does not reach RIGHT; scaled up with RIGHT's scale factor it is 1800..1950 and overlaps.
        assert_eq!(
            locate(rect(1800.0, 100.0, 100.0, 300.0), &[RIGHT]),
            landing(1800, 100, 1.5)
        );
        // Area is measured with the scaled-up rectangle too — PRIMARY = 220 × 300 · RIGHT = 380 × 450 (unscaled it would be 180 × 300 and PRIMARY
        //   wins). The position is the saved physical position either way.
        assert_eq!(
            locate(rect(1700.0, 100.0, 400.0, 300.0), &[PRIMARY, RIGHT]),
            landing(1700, 100, 1.5)
        );
    }

    #[test]
    fn monitors_with_an_unusable_scale_are_ignored() {
        let rect = rect(80.0, 60.0, 400.0, 300.0);
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(locate(rect, &[monitor(0, 0, 1920, 1080, scale)]), None);
        }
    }

    #[test]
    fn negative_coordinates_and_fractional_positions() {
        let left = monitor(-2560, 0, 2560, 1440, 1.5);
        let above = monitor(0, -1440, 2560, 1440, 1.25);
        let all = [PRIMARY, left, above];
        assert_eq!(
            locate(rect(-2000.0, 200.0, 800.0, 600.0), &all),
            landing(-2000, 200, 1.5)
        );
        assert_eq!(
            locate(rect(100.0, -1000.0, 800.0, 600.0), &all),
            landing(100, -1000, 1.25)
        );
        // A fractional position in a hand-edited file is rounded (halves go to the side away from 0).
        assert_eq!(
            locate(rect(10.6, -10.5, 800.0, 600.0), &[PRIMARY]),
            landing(11, -11, 1.0)
        );
    }

    // ── Getter values → placement to record ──

    #[test]
    fn the_position_stays_physical_and_the_size_is_divided_by_the_window_scale() {
        assert_eq!(
            saved_bounds(3000, 150, 1200, 900, 1.5),
            Some(rect(3000.0, 150.0, 800.0, 600.0))
        );
        assert_eq!(
            saved_bounds(-7, 0, 1001, 800, 1.25),
            Some(rect(-7.0, 0.0, 800.8, 640.0))
        );
    }

    #[test]
    fn a_zero_size_or_an_unusable_scale_gives_no_bounds() {
        assert_eq!(saved_bounds(-32000, -32000, 0, 0, 1.0), None, "최소화");
        for scale in [0.0, -2.0, f64::NAN] {
            assert_eq!(saved_bounds(0, 0, 100, 100, scale), None);
        }
    }

    // ── The recorded placement comes back to the same monitor ──

    #[test]
    fn a_window_entirely_on_either_monitor_lands_back_on_that_monitor() {
        // (physical outer position · physical inner size · that monitor) — the second and fourth would have fallen in the [1280, 1920) band if they were logical positions, pointing at both
        //   monitors.
        let cases = [
            (1000, 100, 800, 600, PRIMARY),
            (1300, 100, 400, 400, PRIMARY),
            (3000, 300, 1200, 900, RIGHT),
            (2100, 300, 1200, 900, RIGHT),
            (1920, 0, 600, 450, RIGHT),
        ];
        for (x, y, w, h, on) in cases {
            let saved = saved_bounds(x, y, w, h, on.scale).unwrap();
            for order in [[PRIMARY, RIGHT], [RIGHT, PRIMARY]] {
                assert_eq!(
                    locate(saved, &order),
                    landing(x, y, on.scale),
                    "({x}, {y}) {w}x{h} · {order:?}"
                );
            }
            let size = LogicalSize::new(saved.w(), saved.h()).to_physical::<u32>(on.scale);
            assert_eq!(
                (size.width, size.height),
                (w, h),
                "크기도 같은 물리 px 로 돌아온다"
            );
        }
    }

    // ── ⑨ Opening restored popouts ──

    fn layout_with_popouts(labels: &[&str]) -> LayoutState {
        let mut mgr = ViewManager::new();
        for label in labels {
            mgr.create_window(label).unwrap();
        }
        LayoutState(Arc::new(std::sync::Mutex::new(mgr)))
    }

    #[test]
    fn popouts_open_in_issue_order_with_their_attributes() {
        let layout = layout_with_popouts(&["slot-popup-10", "slot-popup-2"]);
        let normal = rect(-1920.0, 40.0, 720.0, 500.0);
        {
            let mut mgr = layout.0.lock().unwrap();
            for (maximized, at) in [(false, normal), (true, rect(-1928.0, -8.0, 1920.0, 1040.0))] {
                let seen = WindowPlacement {
                    minimized: false,
                    maximized,
                    bounds: Some(at),
                };
                mgr.observe_window_placement("slot-popup-2", seen).unwrap();
            }
        }

        let mut opened = Vec::new();
        let removed = open_restored_popouts(&layout, |label, attrs| {
            assert!(layout.0.try_lock().is_ok(), "락 밖에서 연다");
            opened.push((label.to_string(), attrs));
            Ok(())
        });

        assert!(removed.is_empty());
        let labels: Vec<&str> = opened.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["slot-popup-2", "slot-popup-10"]);
        assert_eq!(opened[0].1.bounds, Some(normal));
        assert!(opened[0].1.maximized);
        assert_eq!(opened[1].1, WindowAttrs::default());
    }

    #[test]
    fn a_popout_that_fails_to_open_is_removed_from_the_model() {
        let layout = layout_with_popouts(&["slot-popup-1", "slot-popup-2"]);
        let version = layout.0.lock().unwrap().version;

        let removed = open_restored_popouts(&layout, |label, _| {
            if label == "slot-popup-1" {
                Err("창 생성 실패(시험)".into())
            } else {
                Ok(())
            }
        });

        assert_eq!(removed, ["slot-popup-1"]);
        let mut mgr = layout.0.lock().unwrap();
        let mut windows = mgr.list_windows();
        windows.sort();
        assert_eq!(windows, [MAIN_WINDOW_LABEL, "slot-popup-2"]);
        assert!(
            mgr.version > version,
            "지운 것이 레이아웃 변경이라 기록기가 싣는다"
        );
        // `Destroyed` never arrives for a window that could not be opened — the number is released right away and the next new window gets it (TRD S26-window-chrome §4-1).
        assert!(mgr.closing_numbers().is_empty());
        mgr.create_window("slot-popup-3").unwrap();
        assert_eq!(mgr.windows["slot-popup-3"].number, 2);
        mgr.assert_number_invariants();
    }

    #[test]
    fn a_model_without_popouts_opens_nothing() {
        let layout = LayoutState::new();
        let removed = open_restored_popouts(&layout, |label, _| {
            panic!("열 팝아웃이 없다: {label}");
        });
        assert!(removed.is_empty());
    }

    /// The live webview list (`live`) and what was sent — the test's `open` puts the popouts it opened on the list.
    #[derive(Default)]
    struct Screen {
        live: Mutex<Vec<String>>,
        sent: Mutex<Vec<(String, String)>>,
    }

    impl ThemeWindows for Screen {
        fn labels(&self) -> Vec<String> {
            self.live.lock().unwrap().clone()
        }

        fn send(&self, label: &str, payload: UiSettingsPayload) -> Result<(), String> {
            self.sent
                .lock()
                .unwrap()
                .push((label.to_string(), payload.theme));
            Ok(())
        }
    }

    /// ★Boot restore pushes each window's effective theme★ — a window with its own theme (main · popout) gets that value, a popout without one gets the
    /// global value. A popout is pushed after it is opened, so it is in the receiving window list.
    #[test]
    fn boot_restore_pushes_each_window_its_effective_theme_after_opening_popouts() {
        let layout = layout_with_popouts(&["slot-popup-1", "slot-popup-2"]);
        {
            let mut mgr = layout.0.lock().unwrap();
            mgr.set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::Light))
                .unwrap();
            mgr.set_window_theme("slot-popup-1", Some(UiTheme::EInk))
                .unwrap();
        }
        let screen = Arc::new(Screen::default());
        screen
            .live
            .lock()
            .unwrap()
            .push(MAIN_WINDOW_LABEL.to_string());
        // Settings with write not opened — the global theme is the default, and loading does not create a folder that does not exist.
        let settings = Arc::new(SettingsService::load_from_dir(
            &std::env::temp_dir().join("engram-placement-no-settings"),
        ));
        let themes = ThemeControl::new(
            Arc::new(EffectiveThemes::new(settings, layout.clone())),
            screen.clone(),
        );

        let removed = open_popouts_then_push_themes(
            &layout,
            |label, _| {
                screen.live.lock().unwrap().push(label.to_string());
                Ok(())
            },
            &themes,
        );

        assert!(removed.is_empty());
        let expected: Vec<(String, String)> = [
            (MAIN_WINDOW_LABEL, "light"),
            ("slot-popup-1", "e-ink"),
            ("slot-popup-2", DEFAULT_THEME.as_wire()),
        ]
        .iter()
        .map(|(label, theme)| (label.to_string(), theme.to_string()))
        .collect();
        assert_eq!(*screen.sent.lock().unwrap(), expected);
    }

    // ── ⑧ Static window ──

    fn attrs(bounds: Option<WindowBounds>, maximized: bool) -> WindowAttrs {
        WindowAttrs {
            bounds,
            maximized,
            ..WindowAttrs::default()
        }
    }

    #[test]
    fn the_static_window_opens_at_its_saved_place() {
        let main = rect(1700.0, 100.0, 400.0, 300.0);
        let statics = static_window(Some(attrs(Some(main), true)), &[PRIMARY, RIGHT], false);
        assert_eq!(
            statics,
            StaticWindow {
                at: landing(1700, 100, 1.5).map(|at| (main, at)),
                maximized: true,
                hide: false,
            }
        );
    }

    #[test]
    fn a_static_window_saved_off_every_monitor_opens_at_the_config_place() {
        let off = rect(-3000.0, 100.0, 800.0, 600.0);
        let statics = static_window(Some(attrs(Some(off), false)), &[PRIMARY], false);
        assert_eq!(
            statics,
            StaticWindow {
                at: None,
                maximized: false,
                hide: false,
            }
        );
    }

    #[test]
    fn an_unread_main_opens_as_the_config_declares() {
        assert_eq!(
            static_window(None, &[PRIMARY], false),
            StaticWindow {
                at: None,
                maximized: false,
                hide: false,
            }
        );
    }

    #[test]
    fn a_hidden_boot_creates_main_hidden_with_its_maximize() {
        let saved = Some(attrs(Some(rect(80.0, 60.0, 1280.0, 800.0)), true));
        let main = static_window(saved, &[PRIMARY], true);
        assert!(
            matches!(
                main,
                StaticWindow {
                    hide: true,
                    maximized: true,
                    ..
                }
            ),
            "숨긴 채 만든다(보였다 숨지 않는다) · 최대화는 그대로 싣는다(숨은 main 에는 보일 때로 미룬다)"
        );
        let main = static_window(saved, &[PRIMARY], false);
        assert!(matches!(main, StaticWindow { hide: false, .. }));
    }

    // A window the config declares is not created by Tauri but by boot ⑧ — if Tauri were let to create it, the webview finishing step would be skipped (ADR-0054) and ⑧'s creation of the same
    //   label would fail. A window that is only declared and that ⑧ does not know is created by no one.
    #[test]
    fn every_config_window_is_left_to_the_boot_and_the_boot_knows_them_all() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        let declared: Vec<tauri::utils::config::WindowConfig> = conf["app"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|window| serde_json::from_value(window.clone()).unwrap())
            .collect();
        for window in &declared {
            assert!(
                !window.create,
                "창 {} 의 \"create\" 가 false 가 아니다",
                window.label
            );
        }
        let declared: Vec<&str> = declared
            .iter()
            .map(|window| window.label.as_str())
            .collect();
        assert_eq!(declared, [MAIN_WINDOW_LABEL]);
    }

    /// Test bench for boot ⑧ ⑨ — a model holding the popouts `popouts` · the live webview list · the theme pushed to that list.
    fn boot_rig(popouts: &[&str]) -> (LayoutState, Arc<Screen>, ThemeControl) {
        let layout = layout_with_popouts(popouts);
        let screen = Arc::new(Screen::default());
        let settings = Arc::new(SettingsService::load_from_dir(
            &std::env::temp_dir().join("engram-placement-no-settings"),
        ));
        let themes = ThemeControl::new(
            Arc::new(EffectiveThemes::new(settings, layout.clone())),
            screen.clone(),
        );
        (layout, screen, themes)
    }

    fn main_window() -> StaticWindow {
        static_window(None, &[], false)
    }

    fn pushed(screen: &Screen) -> Vec<String> {
        screen
            .sent
            .lock()
            .unwrap()
            .iter()
            .map(|(label, _)| label.clone())
            .collect()
    }

    /// ★main comes before the popouts · theme push★ — so main also receives the boot theme.
    #[test]
    fn boot_opens_main_before_popouts_and_pushes_themes_to_all() {
        let (layout, screen, themes) = boot_rig(&["slot-popup-1"]);

        let removed = open_boot_windows(
            main_window(),
            |_| {
                screen
                    .live
                    .lock()
                    .unwrap()
                    .push(MAIN_WINDOW_LABEL.to_string());
                Ok(())
            },
            &layout,
            |label, _| {
                screen.live.lock().unwrap().push(label.to_string());
                Ok(())
            },
            &themes,
        );

        assert_eq!(removed, Ok(Vec::new()));
        assert_eq!(
            *screen.live.lock().unwrap(),
            [MAIN_WINDOW_LABEL, "slot-popup-1"],
            "main 을 먼저 만든다"
        );
        assert_eq!(pushed(&screen), [MAIN_WINDOW_LABEL, "slot-popup-1"]);
    }

    /// If main cannot be created, end the app (user decision 2026-10-07) — stop right there and do not create more windows for an app about to end.
    #[test]
    fn a_main_that_fails_to_open_stops_the_boot() {
        let (layout, screen, themes) = boot_rig(&["slot-popup-1"]);

        let outcome = open_boot_windows(
            main_window(),
            |_| Err("창 생성 실패(시험)".into()),
            &layout,
            |label, _| panic!("팝아웃을 열지 않는다: {label}"),
            &themes,
        );

        assert_eq!(outcome, Err(MainWindowFailed));
        assert!(pushed(&screen).is_empty(), "테마를 밀지 않는다");
    }

    // ── Checking the created window ──

    #[test]
    fn a_window_that_answers_its_getter_was_created() {
        assert_eq!(
            confirm_created(Ok(false), || panic!("런타임을 다시 묻지 않는다")),
            Ok(())
        );
    }

    #[test]
    fn a_window_the_runtime_does_not_know_was_not_created() {
        let missing = || Err("failed to receive message from webview".to_owned());

        let without_runtime =
            confirm_created(missing(), || Err("런타임 없음(시험)".into())).unwrap_err();
        assert!(
            without_runtime.contains("런타임 없음(시험)"),
            "{without_runtime}"
        );

        let with_runtime = confirm_created(missing(), || Ok("141.0".into())).unwrap_err();
        assert!(
            with_runtime.contains("failed to receive message"),
            "{with_runtime}"
        );
    }

    // ── Deferred maximize ──

    #[test]
    fn a_deferred_maximize_is_applied_once() {
        let deferred = DeferredMaximize::default();
        assert!(!deferred.is_pending(MAIN_WINDOW_LABEL));
        assert_eq!(
            deferred.take(MAIN_WINDOW_LABEL),
            None,
            "미룬 것이 없으면 입히지 않는다"
        );

        deferred.defer(MAIN_WINDOW_LABEL, true);
        assert!(
            deferred.is_pending(MAIN_WINDOW_LABEL),
            "서 있는 동안 main 의 자리를 적지 않는다"
        );
        assert_eq!(deferred.take(MAIN_WINDOW_LABEL), Some(true));
        assert!(!deferred.is_pending(MAIN_WINDOW_LABEL));
        assert_eq!(
            deferred.take(MAIN_WINDOW_LABEL),
            None,
            "다음 보이기는 다시 최대화하지 않는다"
        );
    }

    // A desire is one per label — a value written later overrides an earlier one, and applying receives that value. An original-size desire also stops placement
    //   recording while it stands (`record` — regardless of value).
    #[test]
    fn a_later_wish_overwrites_an_earlier_one_and_is_applied_with_its_value() {
        let deferred = DeferredMaximize::default();
        deferred.defer("slot-popup-5", true);
        deferred.defer("slot-popup-5", false);
        assert!(deferred.is_pending("slot-popup-5"), "원래 크기 바람");
        let mut applied = Vec::new();
        assert!(deferred.apply("slot-popup-5", |maximized| {
            applied.push(maximized);
            true
        }));
        assert_eq!(applied, [false]);
        assert!(!deferred.is_pending("slot-popup-5"));
    }

    // If the other side writes a new desire while applying, keep that desire — the next show applies it.
    #[test]
    fn a_wish_written_while_applying_is_kept() {
        let deferred = DeferredMaximize::default();
        deferred.defer("slot-popup-5", true);
        assert!(!deferred.apply("slot-popup-5", |_| {
            deferred.defer("slot-popup-5", false);
            true
        }));
        assert_eq!(
            deferred.wish("slot-popup-5").map(|wish| wish.maximized),
            Some(false)
        );
    }

    // Even if the same value is written again while applying, that desire is new — clearing by value alone would erase it.
    #[test]
    fn a_same_value_wish_rewritten_while_applying_is_kept() {
        let deferred = DeferredMaximize::default();
        deferred.defer("slot-popup-5", true);
        assert!(!deferred.apply("slot-popup-5", |_| {
            deferred.defer("slot-popup-5", true);
            true
        }));
        assert_eq!(
            deferred.wish("slot-popup-5").map(|wish| wish.maximized),
            Some(true)
        );
    }

    // A desire that came back as true → false → true is also a new desire written during applying (ABA).
    #[test]
    fn a_wish_that_returns_to_the_applied_value_while_applying_is_kept() {
        let deferred = DeferredMaximize::default();
        deferred.defer("slot-popup-5", true);
        assert!(!deferred.apply("slot-popup-5", |_| {
            deferred.defer("slot-popup-5", false);
            deferred.defer("slot-popup-5", true);
            true
        }));
        assert!(deferred.is_pending("slot-popup-5"));
        assert!(
            deferred.apply("slot-popup-5", |maximized| maximized),
            "다음 보이기가 남은 바람을 입힌다"
        );
        assert!(!deferred.is_pending("slot-popup-5"));
    }

    #[test]
    fn a_deferred_maximize_that_fails_to_apply_stays_pending_for_the_next_show() {
        let deferred = DeferredMaximize::default();
        assert!(
            !deferred.apply("slot-popup-5", |_| panic!("미룬 것이 없으면 입히지 않는다")),
            "미룬 것이 없다"
        );

        deferred.defer("slot-popup-5", true);
        assert!(
            !deferred.apply("slot-popup-5", |_| false),
            "못 입혔으면 자리를 적지 않는다"
        );
        assert!(
            deferred.is_pending("slot-popup-5"),
            "못 입힌 미룸은 남는다 — 그동안 보통 자리를 「최대화 아님」으로 적지 않는다"
        );

        assert!(
            deferred.apply("slot-popup-5", |_| true),
            "다음 보이기가 다시 입힌다"
        );
        assert!(!deferred.is_pending("slot-popup-5"));
        assert!(
            !deferred.apply("slot-popup-5", |_| panic!("한 번만 입힌다")),
            "입힌 뒤에는 다시 입히지 않는다"
        );
    }

    #[test]
    fn a_visible_window_keeps_its_deferral_until_the_maximize_lands() {
        let deferred = DeferredMaximize::default();
        assert!(!deferred.apply_now(
            MAIN_WINDOW_LABEL,
            || assert!(
                deferred.is_pending(MAIN_WINDOW_LABEL),
                "자리를 입히는 동안 main 의 자리를 적지 않는다"
            ),
            || false,
        ));
        assert!(
            deferred.is_pending(MAIN_WINDOW_LABEL),
            "못 입혔으면 남는다 — 복원 뒤의 자리 기록이 main 을 「최대화 아님」으로 적지 않는다"
        );

        assert!(
            deferred.apply(MAIN_WINDOW_LABEL, |_| true),
            "다음 보이기가 다시 입힌다"
        );
        assert!(!deferred.is_pending(MAIN_WINDOW_LABEL));

        assert!(deferred.apply_now(MAIN_WINDOW_LABEL, || {}, || true));
        assert!(
            !deferred.is_pending(MAIN_WINDOW_LABEL),
            "입혔으면 거둔다 — main 의 자리가 다시 적힌다"
        );
    }

    #[test]
    fn a_show_that_takes_the_deferral_mid_placement_does_not_stop_the_maximize() {
        let deferred = DeferredMaximize::default();
        let mut maximized = false;
        assert!(!deferred.apply_now(
            MAIN_WINDOW_LABEL,
            // The tray 「보이기」 cut in, applied the deferred maximize and cleared it — the position setter after that lowers that maximize (F8).
            || assert!(deferred.apply(MAIN_WINDOW_LABEL, |_| true)),
            || {
                maximized = true;
                false
            },
        ));
        assert!(maximized, "거둬졌어도 입히기를 부른다");
        assert!(
            deferred.is_pending(MAIN_WINDOW_LABEL),
            "못 입혔으면 거둬졌던 미룸을 다시 세운다"
        );
    }

    #[test]
    fn deferred_maximizes_are_kept_per_window() {
        let deferred = DeferredMaximize::default();
        deferred.defer("slot-popup-3", true);
        deferred.defer("slot-popup-4", false);

        assert!(
            !deferred.is_pending(MAIN_WINDOW_LABEL),
            "다른 창은 그대로 적힌다"
        );
        assert_eq!(
            deferred.take("slot-popup-3"),
            Some(true),
            "보인 창 하나만 입힌다"
        );
        assert!(!deferred.is_pending("slot-popup-3"));
        assert!(
            deferred.is_pending("slot-popup-4"),
            "아직 숨은 창의 미룸은 남는다"
        );
        assert_eq!(
            deferred.take("slot-popup-4"),
            Some(false),
            "창 소멸도 같은 거두기다 — 거둔 뒤 표가 빈다"
        );
        assert!(deferred.wishes().by_label.is_empty());
    }

    // ── Widen and pull in (TRD S26-window-chrome §7-7 · D14) ──

    /// Work area = PRIMARY minus the 48 px taskbar below. Outer frame = invisible borders of 8 px on each of left and right.
    const WORK: MonitorArea = MonitorArea {
        x: 0,
        y: 0,
        w: 1920,
        h: 1032,
        scale: 1.0,
    };
    const FRAME: f64 = 16.0;

    #[test]
    fn a_narrow_saved_width_is_widened_to_the_min_in_place() {
        assert_eq!(
            fit_min_width(rect(300.0, 200.0, 200.0, 500.0), 286.0, WORK, FRAME),
            rect(300.0, 200.0, 286.0, 500.0),
            "넓힌 창이 작업 영역 안이면 자리는 그대로다"
        );
    }

    #[test]
    fn a_wide_enough_window_is_left_alone_even_off_the_work_area() {
        let off = rect(1800.0, 200.0, 600.0, 500.0);
        assert_eq!(fit_min_width(off, 286.0, WORK, FRAME), off);
        let exact = rect(1700.0, 200.0, 286.0, 500.0);
        assert_eq!(fit_min_width(exact, 286.0, WORK, FRAME), exact);
    }

    #[test]
    fn a_widened_window_past_the_work_area_right_edge_is_pulled_back_in() {
        // So that the visible right end = outer x + 8 + 286 = 1920, outer x = 1626 (the invisible right border of 8 px is outside the work area).
        assert_eq!(
            fit_min_width(rect(1800.0, 200.0, 120.0, 500.0), 286.0, WORK, FRAME),
            rect(1626.0, 200.0, 286.0, 500.0)
        );
    }

    #[test]
    fn the_pull_uses_the_window_scale_for_the_visible_width() {
        // 150 % — visible width = 286 × 1.5 = 429 · one-side border = 24 / 2 = 12 → fitting the visible right end 1920 + 2560 = 4480,
        //   outer x = 4480 − 12 − 429 = 4039.
        let work = monitor(1920, 0, 2560, 1392, 1.5);
        assert_eq!(
            fit_min_width(rect(4400.0, 100.0, 200.0, 400.0), 286.0, work, 24.0),
            rect(4039.0, 100.0, 286.0, 400.0)
        );
    }

    #[test]
    fn a_work_area_narrower_than_the_min_aligns_the_visible_left_edge() {
        let tiny = monitor(-640, 0, 280, 480, 1.0);
        assert_eq!(
            fit_min_width(rect(-500.0, 40.0, 120.0, 300.0), 286.0, tiny, FRAME),
            rect(-648.0, 40.0, 286.0, 300.0),
            "보이는 폭 286 > 작업 영역 280 — 보이는 왼쪽 끝(바깥 x + 8) = 작업 영역 왼쪽 끝"
        );
    }

    #[test]
    fn a_window_left_of_the_work_area_is_pulled_to_the_visible_left_edge() {
        assert_eq!(
            fit_min_width(rect(-300.0, 200.0, 120.0, 500.0), 286.0, WORK, FRAME),
            rect(-8.0, 200.0, 286.0, 500.0),
            "넓힐 때는 왼쪽 밖 자리도 보이는 왼쪽 끝 = 작업 영역 왼쪽 끝으로 당긴다"
        );
    }

    #[test]
    fn an_odd_frame_keeps_the_visible_edges_inside_the_work_area() {
        // One-side border = 8.5 — the right range rounds down and the left range rounds up, so half a px does not go outside the work area.
        assert_eq!(
            fit_min_width(rect(1800.0, 200.0, 120.0, 500.0), 286.0, WORK, 17.0),
            rect(1625.0, 200.0, 286.0, 500.0)
        );
        let tiny = monitor(-640, 0, 280, 480, 1.0);
        assert_eq!(
            fit_min_width(rect(-500.0, 40.0, 120.0, 300.0), 286.0, tiny, 17.0),
            rect(-648.0, 40.0, 286.0, 300.0)
        );
    }

    #[test]
    fn an_unusable_scale_widens_without_pulling() {
        let broken = monitor(0, 0, 1920, 1032, f64::NAN);
        assert_eq!(
            fit_min_width(rect(1800.0, 200.0, 120.0, 500.0), 286.0, broken, FRAME),
            rect(1800.0, 200.0, 286.0, 500.0)
        );
    }

    // ── Placement application that returns results (the docking window port) ──

    /// Setter fake — records the steps called, and the steps in `fail` fail.
    #[derive(Default)]
    struct FakeSetters {
        calls: Mutex<Vec<String>>,
        fail: Vec<MoveStep>,
    }

    impl FakeSetters {
        fn failing(fail: &[MoveStep]) -> Self {
            FakeSetters {
                fail: fail.to_vec(),
                ..FakeSetters::default()
            }
        }

        fn call(&self, step: MoveStep, what: String) -> Result<(), String> {
            self.calls.lock().unwrap().push(what);
            if self.fail.contains(&step) {
                return Err(format!("{step:?} 실패(시험)"));
            }
            Ok(())
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Setters for FakeSetters {
        fn set_position(&self, x: i32, y: i32) -> Result<(), String> {
            self.call(MoveStep::Position, format!("position {x},{y}"))
        }

        fn set_size(&self, w: f64, h: f64) -> Result<(), String> {
            self.call(MoveStep::Size, format!("size {w}x{h}"))
        }
    }

    #[test]
    fn moving_sets_the_position_before_the_size() {
        let window = FakeSetters::default();
        assert_eq!(
            move_steps(&window, Some((10, -20)), Some((300.0, 200.5))),
            Ok(())
        );
        assert_eq!(window.calls(), ["position 10,-20", "size 300x200.5"]);
    }

    #[test]
    fn moving_stops_at_the_first_failed_step() {
        let window = FakeSetters::failing(&[MoveStep::Position]);
        let err = move_steps(&window, Some((10, 20)), Some((300.0, 200.0))).unwrap_err();
        assert_eq!(err.step, MoveStep::Position);
        assert!(err.error.contains("Position"), "{err:?}");
        assert_eq!(window.calls(), ["position 10,20"], "크기는 안 불린다");

        let window = FakeSetters::failing(&[MoveStep::Size]);
        let err = move_steps(&window, Some((10, 20)), Some((300.0, 200.0))).unwrap_err();
        assert_eq!(err.step, MoveStep::Size);
        assert_eq!(window.calls().len(), 2);
    }

    #[test]
    fn moving_skips_a_step_without_a_value() {
        let window = FakeSetters::failing(&[MoveStep::Position]);
        assert_eq!(move_steps(&window, None, Some((300.0, 200.0))), Ok(()));
        assert_eq!(window.calls(), ["size 300x200"]);

        let window = FakeSetters::default();
        assert_eq!(move_steps(&window, None, None), Ok(()));
        assert!(window.calls().is_empty());
    }

    #[test]
    fn only_finite_positive_sizes_are_usable() {
        assert_eq!(usable_size(Some((300.0, 200.0))), Some((300.0, 200.0)));
        for bad in [
            (0.0, 200.0),
            (300.0, -1.0),
            (f64::NAN, 200.0),
            (300.0, f64::INFINITY),
        ] {
            assert_eq!(usable_size(Some(bad)), None, "{bad:?}");
        }
        assert_eq!(usable_size(None), None);
    }

    #[test]
    fn a_dock_place_on_a_monitor_lands_with_its_size_or_the_default() {
        let at = WindowPlace {
            pos: Some((1700, 100)),
            size: Some((400.0, 300.0)),
        };
        assert_eq!(
            resolve_place(
                "slot-popup-1",
                &at,
                (720.0, 500.0),
                &[PRIMARY, RIGHT],
                Fallback::DefaultPlace
            ),
            Some((
                rect(1700.0, 100.0, 400.0, 300.0),
                Landing {
                    x: 1700,
                    y: 100,
                    scale: 1.5
                }
            ))
        );
        for size in [None, Some((0.0, 300.0))] {
            let at = WindowPlace {
                pos: Some((80, 60)),
                size,
            };
            assert_eq!(
                resolve_place(
                    "slot-popup-1",
                    &at,
                    (720.0, 500.0),
                    &[PRIMARY],
                    Fallback::DefaultPlace
                ),
                Some((
                    rect(80.0, 60.0, 720.0, 500.0),
                    Landing {
                        x: 80,
                        y: 60,
                        scale: 1.0
                    }
                )),
                "{size:?} — 기본 크기"
            );
        }
    }

    // A move that did not choose a size measures overlap with the size the window will carry (`kept_size` of `move_window`) — measuring with the popout default size
    //   would decide that a window smaller than that overlaps, and it would be placed off-screen.
    #[test]
    fn a_small_window_left_of_the_monitor_does_not_land_though_the_default_size_would() {
        let at = WindowPlace {
            pos: Some((-650, 100)),
            size: None,
        };
        let land_with =
            |size| resolve_place("slot-popup-1", &at, size, &[PRIMARY], Fallback::StayPut);
        assert_eq!(land_with((300.0, 200.0)), None, "창 크기로 재면 화면 밖");
        assert!(
            land_with((720.0, 500.0)).is_some(),
            "전제: 기본 크기로 재면 걸친다"
        );
    }

    #[test]
    fn a_dock_place_off_every_monitor_or_without_a_position_does_not_land() {
        let off = WindowPlace {
            pos: Some((-3000, 100)),
            size: Some((400.0, 300.0)),
        };
        let unplaced = WindowPlace {
            pos: None,
            size: Some((400.0, 300.0)),
        };
        for at in [off, unplaced] {
            for fallback in [Fallback::DefaultPlace, Fallback::StayPut] {
                assert_eq!(
                    resolve_place(
                        "slot-popup-1",
                        &at,
                        (720.0, 500.0),
                        &[PRIMARY, RIGHT],
                        fallback
                    ),
                    None,
                    "{at:?}"
                );
            }
        }
    }
}
